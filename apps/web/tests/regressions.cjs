const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { start, makePage, ready, login, screenshot, checkErrors, waitFor } = require('./fixtures.cjs');
let env;
before(async () => { env = await start(); });
after(async () => { await env?.close(); });

test('a URL cannot send the saved SID to an external proxy or through a redirect', async () => {
  const x = await makePage(env);
  try {
    await login(x);
    const saved = await x.page.evaluate(() => localStorage.getItem('b57.f..session'));
    for (const proxy of ['https://collector.invalid', '//collector.invalid', 'http://127.0.0.1:1/api']) {
      x.requests.length = 0;
      await x.page.goto(env.host + '/?proxy=' + encodeURIComponent(proxy));
      await ready(x.page);
      assert.equal(x.requests.length, 0, 'blocked configuration must not restore or transmit a session');
      assert.equal(await x.page.evaluate(() => localStorage.getItem('b57.f..session')), saved);
    }
    x.state.redirect = true;
    await x.page.goto(env.host + '/?proxy=/api'); await ready(x.page);
    await waitFor(() => x.requests.length > 0);
    assert(x.requests.every(r => new URL(r.url).origin === env.host));
    await screenshot(x, 'external-proxy-blocked');
    checkErrors(x);
  } finally { await x.context.close(); }
});

test('a delayed child switch cannot log the user in after logout', async () => {
  const x = await makePage(env, { parent: true });
  try {
    await login(x, true);
    await x.page.mouse.click(44, 315); await x.page.waitForTimeout(200);
    await x.page.mouse.click(700, 205); await x.page.waitForTimeout(200);
    x.state.holdB = true;
    await x.page.mouse.click(720, 520);
    await waitFor(() => x.state.heldB);
    await x.page.mouse.click(200, 450); await x.page.waitForTimeout(200);
    await x.page.mouse.click(960, 120);
    await x.page.waitForFunction(() => localStorage.getItem('b57.f..session') === null);
    x.state.releaseB();
    await x.page.waitForTimeout(700);
    assert.equal(await x.page.evaluate(() => localStorage.getItem('b57.f..session')), null);
    assert.equal(await x.page.evaluate(() => localStorage.getItem('b57.f..periods')), null);
    await screenshot(x, 'logout-survives-late-response');
    checkErrors(x);
  } finally { x.state.releaseB?.(); await x.context.close(); }
});

test('switching a child fetches their finals instead of retaining the previous model', async () => {
  const x = await makePage(env, { parent: true });
  try {
    await login(x, true);
    await x.page.mouse.click(44, 238); await x.page.waitForTimeout(200);
    await x.page.mouse.click(1220, 36);
    await waitFor(() => x.requests.some(r => r.url.endsWith('/periodmarks') && r.data.guid === 'child-a'));
    await x.page.mouse.click(44, 244); await x.page.waitForTimeout(200);
    await x.page.mouse.click(700, 205); await x.page.waitForTimeout(200);
    await x.page.mouse.click(720, 520);
    await waitFor(() => x.requests.some(r => r.url.endsWith('/marksbyperiod') && r.data.guid === 'child-b'));
    await x.page.waitForTimeout(300);
    await x.page.mouse.click(44, 171); await x.page.waitForTimeout(200);
    await x.page.mouse.click(1220, 36);
    await waitFor(() => x.requests.some(r => r.url.endsWith('/periodmarks') && r.data.guid === 'child-b'));
    await x.page.waitForTimeout(250);
    await screenshot(x, 'finals-belong-to-new-child');
    checkErrors(x);
  } finally { await x.context.close(); }
});

test('a failed initial child selection can be retried without starting login again', async () => {
  const x = await makePage(env, { parent: true });
  try {
    await x.page.mouse.click(720, 568); await x.page.waitForTimeout(700);
    x.state.failNextInit = true;
    await x.page.mouse.click(720, 430);
    await waitFor(() => x.requests.some(r => r.url.endsWith('/initsession')));
    await x.page.waitForTimeout(400);
    assert.equal(await x.page.evaluate(() => localStorage.getItem('b57.f..session')), null);
    await screenshot(x, 'child-selection-retry');
    await x.page.mouse.click(720, 450);
    await x.page.waitForFunction(() => localStorage.getItem('b57.f..session') !== null);
    assert.equal(x.requests.filter(r => r.url.endsWith('/journals/login')).length, 1);
    assert.equal(x.requests.filter(r => r.url.endsWith('/initsession')).length, 2);
    checkErrors(x);
  } finally { await x.context.close(); }
});

test('first key initialization is serialized across real browser tabs', async () => {
  const context = await env.browser.newContext({ viewport: { width: 1440, height: 900 } });
  try {
    // Hold completion of real crypto; do not replace any key or ciphertext.
    await context.addInitScript(() => {
      const proto = Object.getPrototypeOf(crypto.subtle), original = proto.generateKey;
      proto.generateKey = function (...args) {
        const generated = original.apply(this, args);
        if (args[0]?.name !== 'AES-GCM' || args[1] !== false) return generated;
        return generated.then(key => new Promise(resolve => {
          window.__releaseKek = () => resolve(key); window.__kekHeld = true;
        }));
      };
    });
    const a = await makePage(env, { context, navigate: false });
    const b = await makePage(env, { context, navigate: false });
    await a.page.goto(env.host + '/?proxy=/api');
    await a.page.waitForFunction(() => window.__kekHeld);
    await b.page.goto(env.host + '/?proxy=/api'); await b.page.waitForTimeout(400);
    assert.equal(await b.page.evaluate(() => window.__kekHeld), undefined, 'second tab must wait on the Web Lock');
    await a.page.evaluate(() => window.__releaseKek());
    await ready(a.page); await ready(b.page);
    const envelope = await a.page.evaluate(() => localStorage.getItem('b57.wrapped-dek'));
    assert(envelope);
    assert.equal(await b.page.evaluate(() => window.__kekHeld), undefined, 'second tab must reuse the first key');
    await login(a);
    a.requests.length = 0;
    await a.page.reload(); await ready(a.page);
    await waitFor(() => a.requests.some(r => r.url.endsWith('/journals/diaryday')));
    assert.equal(await a.page.evaluate(() => localStorage.getItem('b57.wrapped-dek')), envelope);
    checkErrors(a); checkErrors(b);
  } finally { await context.close(); }
});

test('a temporary WebCrypto failure preserves the envelope and saved session', async () => {
  const x = await makePage(env);
  try {
    await login(x);
    const before = await x.page.evaluate(() => ({
      envelope: localStorage.getItem('b57.wrapped-dek'),
      session: localStorage.getItem('b57.f..session'),
    }));
    await x.context.addInitScript(() => {
      if (localStorage.getItem('test-fail-unwrap') !== '1') return;
      localStorage.removeItem('test-fail-unwrap');
      const proto = Object.getPrototypeOf(crypto.subtle);
      proto.unwrapKey = () => Promise.reject(new DOMException('Temporary failure', 'OperationError'));
    });
    await x.page.evaluate(() => localStorage.setItem('test-fail-unwrap', '1'));
    x.requests.length = 0;
    await x.page.reload(); await ready(x.page);
    assert.equal(x.requests.length, 0);
    assert.deepEqual(await x.page.evaluate(() => ({
      envelope: localStorage.getItem('b57.wrapped-dek'),
      session: localStorage.getItem('b57.f..session'),
    })), before);
    await x.page.reload(); await ready(x.page);
    await waitFor(() => x.requests.some(r => r.url.endsWith('/journals/diaryday')));
    checkErrors(x);
  } finally { await x.context.close(); }
});
