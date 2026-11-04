Проверки запускают настоящую полную WASM-сборку в Chromium. API подставляется
в Playwright; запросы с настоящими учётными данными не выполняются. WebCrypto,
IndexedDB, Web Locks, localStorage и Slint работают без подмены.

Из корня репозитория:

```sh
cargo test --locked -p burmalda57-core
./apps/web/build_web.sh
cd apps/web/tests
npm ci
npx playwright install chromium
npm test
```

Сервер статики стартует автоматически на свободном порту localhost.
Для системного Chromium можно задать `TEST_CHROMIUM_EXECUTABLE`.
Снимки сценариев сохраняются в `artifacts/`.

Проверяются внешние прокси и перенаправления, поздний ответ после выхода,
итоговые оценки после смены ребёнка, повтор выбора после сетевой ошибки и
первичная инициализация ключей в двух вкладках и сохранность данных при
временной ошибке WebCrypto.
