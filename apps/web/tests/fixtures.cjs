const { chromium } = require('playwright');
const crypto = require('node:crypto');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const assert = require('node:assert/strict');

const output = path.join(__dirname, 'artifacts');
const childA = { SYS_GUID: 'child-a', NAME: 'Alice', SURNAME: 'Review', GRADE: { NAME: '9A' } };
const childB = { SYS_GUID: 'child-b', NAME: 'Boris', SURNAME: 'Review', GRADE: { NAME: '5B' } };

function encodedGuid(guid) {
  const key = Buffer.from([31, 23, 19, 50, 40, 23, 19, 10, 0, 0, 0, 0, 0, 0, 0, 0]);
  const cipher = crypto.createCipheriv('aes-128-ecb', key, null);
  return Buffer.concat([cipher.update(guid), cipher.final()]).toString('base64');
}

async function start() {
  await fs.mkdir(output, { recursive: true });
  const root = path.resolve(__dirname, '../web');
  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, 'http://localhost');
      const filename = path.resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
      if (!filename.startsWith(root + path.sep)) throw new Error('Invalid path');
      const body = await fs.readFile(filename);
      const type = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' }[path.extname(filename)];
      res.writeHead(200, { 'Content-Type': type || 'application/octet-stream' });
      res.end(body);
    } catch {
      res.writeHead(404); res.end();
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const host = `http://127.0.0.1:${server.address().port}`;
  let browser;
  try {
    browser = await chromium.launch({
      ...(process.env.TEST_CHROMIUM_EXECUTABLE ? { executablePath: process.env.TEST_CHROMIUM_EXECUTABLE } : {}),
      headless: true,
      args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
    });
  } catch (error) {
    await new Promise(resolve => server.close(resolve));
    throw error;
  }
  return { browser, host, close: async () => { await browser.close(); await new Promise(resolve => server.close(resolve)); } };
}

async function makePage(env, { parent = false, context, navigate = true } = {}) {
  context ||= await env.browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  const requests = [], errors = [];
  const state = { holdB: false, heldB: false, releaseB: null, failNextInit: false, redirect: false };
  page.on('pageerror', e => {
    // winit uses this exception to enter its browser event loop.
    if (!e.message.startsWith('Using exceptions for control flow')) errors.push(e.message);
  });
  page.on('dialog', dialog => dialog.accept('test-session'));
  await page.route('**/*', async route => {
    const req = route.request(), url = new URL(req.url());
    const localApi = url.origin === env.host && url.pathname.startsWith('/api/');
    if (url.origin === env.host && !localApi) return route.continue();
    if (!localApi && url.hostname !== 'mp2.obr57.ru' && url.hostname !== 'collector.invalid') return route.abort();
    const headers = { 'content-type': 'application/json', 'access-control-allow-origin': '*', 'access-control-allow-headers': '*' };
    if (req.method() === 'OPTIONS') return route.fulfill({ status: 204, headers });
    const data = JSON.parse(req.postData() || '{}');
    requests.push({ url: req.url(), data });
    let body = { success: true, data: [] };
    if (state.redirect) return route.fulfill({ status: 307, headers: { location: 'https://collector.invalid/leak' } });
    if (url.pathname.endsWith('/journals/login')) {
      body.data = { NAME: 'Test', SURNAME: 'Parent', SCHOOLS: [{
        ROLES: parent ? ['parents'] : ['student'], SCHOOL: { NAME: 'Review school' },
        ...(parent ? { PARENT: { NAME: 'Test', SURNAME: 'Parent' }, USER_PARTICIPANTS: [childA, childB] } : { PARTICIPANT: childA }),
      }] };
    } else if (url.pathname.endsWith('/session/initsession')) {
      if (state.failNextInit) { state.failNextInit = false; return route.fulfill({ status: 503, body: '' }); }
      const isB = data.sysguid === encodedGuid('child-b');
      if (isB && state.holdB) { state.heldB = true; await new Promise(resolve => { state.releaseB = resolve; }); }
      body = { status: 'ok', key: isB ? 'test-key-b' : 'test-key-a' };
    } else if (url.pathname.endsWith('/journals/allperiods')) {
      body.data = [{ NAME: 'Первая четверть', BEGIN_DATE: '01.09.2026', DATE_TO: '31.12.2026' }];
    } else if (url.pathname.endsWith('/journals/marksbyperiod')) {
      body.data = [{ SUBJECT_NAME: data.guid === 'child-b' ? 'Boris Algebra' : 'Alice Algebra', MARKS: [
        { VALUE: 5, WEIGHT: 10, SHORT_NAME: '5', LONG_NAME: 'Test', DATE: '02.11.2026' },
        { VALUE: 2, WEIGHT: 1, SHORT_NAME: '2', LONG_NAME: 'Homework', DATE: '01.11.2026' },
      ] }];
    } else if (url.pathname.endsWith('/journals/diaryday')) {
      body.data = [{ LESSON_NUMBER: 1, SUBJECT_NAME: 'Algebra', HOMEWORK: 'Exercise 1', LESSON_TIME_BEGIN: '08:30', LESSON_TIME_END: '09:15', MARKS: [{ SHORT_NAME: '5', VALUE: 5 }] }];
    } else if (url.pathname.endsWith('/journals/periodmarks')) {
      const isB = data.guid === 'child-b';
      body.data = [{ NAME: isB ? 'Boris finals' : 'Alice finals', PERIODS: [{ NAME: 'Первая четверть', MARK: { VALUE: isB ? 3 : 5, SHORT_NAME: isB ? '3' : '5' } }] }];
    }
    return route.fulfill({ status: 200, headers, body: JSON.stringify(body) });
  });
  const x = { context, page, requests, errors, state };
  if (navigate) { await page.goto(env.host + '/?proxy=/api'); await ready(page); }
  return x;
}

async function ready(page) {
  await page.waitForFunction(() => window.__appBooted);
  // The HTML canvas exists before async keystore initialization. winit sets
  // its backing size only after the real application window has been created.
  await page.waitForFunction(() => document.querySelector('canvas').width >= window.innerWidth);
  await page.waitForTimeout(250);
}
async function login(x, parent = false) {
  await x.page.mouse.click(720, 568);
  if (parent) { await x.page.waitForTimeout(700); await x.page.mouse.click(720, 430); }
  await x.page.waitForFunction(() => localStorage.getItem('b57.f..session') !== null);
  await x.page.waitForTimeout(500);
}
async function screenshot(x, name) { await x.page.screenshot({ path: path.join(output, name + '.png') }); }
function checkErrors(x) { assert.deepEqual(x.errors, []); }
async function waitFor(predicate) {
  const deadline = Date.now() + 10000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('Expected mock API request did not arrive');
    await new Promise(resolve => setTimeout(resolve, 25));
  }
}
module.exports = { start, makePage, ready, login, screenshot, checkErrors, waitFor };
