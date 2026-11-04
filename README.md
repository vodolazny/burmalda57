# burmalda57

Неофициальный клиент электронного дневника Орловской области
([obr57.ru](https://obr57.ru/gis)) для **Android**, **iOS** и **web**.

[![Android APK](https://img.shields.io/badge/Android-APK-3DDC84?logo=android&logoColor=white)](https://github.com/vodolazny/burmalda57test/releases/latest)
[![iOS IPA](https://img.shields.io/badge/iOS-IPA-000000?logo=apple&logoColor=white)](https://github.com/vodolazny/burmalda57test/releases/tag/ios-latest)
[![Web](https://img.shields.io/badge/Web-demo-2563EB?logo=googlechrome&logoColor=white)](https://burmalda57.redhatgcc.ru/)
![license](https://img.shields.io/badge/license-Apache--2.0-lightgrey)

Написан на Rust и [Slint](https://slint.dev). Мобильные приложения используют
общий код, веб — общее ядро и отдельный desktop-интерфейс.

> ⚠️ **Неофициальный проект.** Не связан с Департаментом образования Орловской
> области, Госуслугами/ЕСИА или разработчиками официального дневника.
> Используется на свой страх и риск. Android и iOS-версии находятся в **бете**.

**Новости и обратная связь:** [Telegram](https://t.me/burmalda57_dev) ·
[Issues](https://github.com/vodolazny/burmalda57test/issues)

---

## Скачать

| Платформа | Что получить | Требования | Подробнее |
|---|---|---|---|
| **Android** | `burmalda57-X.Y.Z.apk` из [Releases](https://github.com/vodolazny/burmalda57test/releases/latest) | Android 8.0+, arm64-v8a | [apps/android](apps/android/README.md) |
| **iOS** | неподписанный `.ipa` из релиза [ios-latest](https://github.com/vodolazny/burmalda57test/releases/tag/ios-latest) — всегда свежая сборка из `main` | iOS 15+; при установке нужна переподпись | [apps/ios](apps/ios/README.md) |
| **Web** | [демо в браузере](https://burmalda57.redhatgcc.ru/) — без входа; полная версия — на своём сервере | современный браузер с WebGL | [apps/web](apps/web/README.md) |

**iOS:** `.ipa` не подписан, его нужно переподписать своим Apple ID через
AltStore, Sideloadly или TrollStore. Пошаговая инструкция и частые проблемы —
в [apps/ios/README.md](apps/ios/README.md).

---

## Возможности

- дневник, расписание и домашние задания;
- оценки по периодам, веса и средний балл, графики успеваемости, итоговые оценки;
- личные события в расписании и календарь;
- родительский режим с выбором ребёнка;
- офлайн-кеш и вход без повторной авторизации;
- светлая и тёмная тема;
- демо-режим без входа.

Заметки к урокам и скрытие предметов доступны также в вебе.
Аватар профиля — в мобильных приложениях. Уведомления о новых оценках и проверка
обновлений есть на Android и iOS, Material You — только на Android. Ограничения iOS — в
[apps/ios/README.md](apps/ios/README.md#️-ограничения-ios).

---

## Приватность

- **Пароль от Госуслуг приложение не видит и не хранит.** На Android и iOS вход
  идёт через WebView на официальной странице ЕСИА. В вебе вы входите на
  настоящей странице Госуслуг и вставляете в приложение только значение
  сессионной куки `X1_SSO`.
- **Локальные данные зашифрованы** (AES-256-GCM). Ключ шифрования данных
  заворачивается ключом платформы: Android Keystore, iOS Keychain или
  неизвлекаемый ключ WebCrypto в браузере. При выходе из аккаунта сессия и
  кеши удаляются.
- **Данные уходят только на серверы дневника** (`mp2.obr57.ru`). Рекламы,
  трекеров и поведенческой аналитики нет.
- **Единственное исключение — Sentry** в мобильных сборках: технические отчёты
  о падениях без персональных данных. 
- **Веб с прокси:** запросы идут через ваш сервер, а значит прокси видит весь
  трафик, включая сессионные данные. Разворачивайте **собственный** прокси и не
  входите через чужой.

Подробнее — в README каждого приложения.

---

## Веб-версия

Веб-клиент — это wasm-приложение: всё работает в браузере, серверной части у
него нет. Но браузер не может обратиться к `mp2.obr57.ru` напрямую: сервер
дневника не отдаёт CORS-заголовки, а часть запросов блокируется по IP. Поэтому
есть два режима.

| | Демо | Полная версия |
|---|---|---|
| Вход через Госуслуги | нет | да |
| Данные | тестовые | настоящие |
| Где работает | [GitHub Pages](https://burmalda57.redhatgcc.ru/), любой статический хостинг, локально | только за **своим прокси** на сервере |
| Сборка | `./apps/web/build_web.sh --demo-only` | `./apps/web/build_web.sh` |

- **Демо** собирается автоматически и публикуется на GitHub Pages при каждом
  push в `main`.
- **Для полной версии нужен прокси.** Простой Python-сервер
  (`python -m http.server`) только раздаёт статические файлы: этого хватает,
  чтобы посмотреть демо, но запросы к API через него не пойдут. Полную версию
  разворачивают на сервере с nginx или Caddy, которые отдают сборку и
  проксируют `/api/` на `mp2.obr57.ru`. Страница открывается с параметром
  `?proxy=/api`. Прокси должен находиться на том же origin, что и сайт.

Готовые конфиги Caddy и nginx, порядок развёртывания и разбор типичных ошибок —
в разделе
[«Полная версия: свой сервер с прокси»](apps/web/README.md#-полная-версия-свой-сервер-с-прокси)
в `apps/web/README.md`.

---

## Архитектура

```
apps/android ─┐
              ├──→ crates/mobile ──→ crates/core
apps/ios ─────┘          │
                         └──────→ crates/ui

apps/web ───────────────────────→ crates/core
```

| Часть | Что делает |
|---|---|
| `crates/core` | домен, API-запросы, парсеры, криптография, кеш, локальные хранилища, уведомления, обновления; без Slint, `reqwest` и `tokio` |
| `crates/ui` | общий Slint-UI для Android и iOS |
| `crates/mobile` | общее состояние Android/iOS: колбэки, сеть, `Platform`, адаптеры `core` → Slint |
| `apps/android` | Gradle/Kotlin и тонкая Rust-оболочка: JNI, Keystore, WorkManager, Material You |
| `apps/ios` | Rust-бинарник и Objective-C-мост: WKWebView, Keychain, PHPicker, BGTask, safe area |
| `apps/web` | wasm-клиент: `fetch`, WebCrypto, localStorage и собственный desktop-Slint-UI |
| `vendor/` | патченный Slint Android backend |

Бизнес-логика и мобильный UI существуют в одном экземпляре.

### Что где менять

| Задача | Где |
|---|---|
| Логика, API, парсинг, кеш, крипта | `crates/core/src/` |
| Экраны и компоненты мобильного UI | `crates/ui/ui/` |
| Состояние, колбэки, маппинг в Slint, сеть | `crates/mobile/src/` |
| JNI, Kotlin, Keystore, WorkManager | `apps/android/` |
| ObjC-мост, Keychain, BGTask, safe area | `apps/ios/` |
| `fetch`, WebCrypto, localStorage, web-UI | `apps/web/` |
| CI и релизы | `.github/workflows/` |

---

## Сборка

Собирайте конкретный пакет, а не весь workspace: Slint backend и renderer
подключаются оболочками приложений, и `cargo build --workspace` смешает их.

| Платформа | Требования | Команда |
|---|---|---|
| Android | JDK 17, SDK 34, NDK 29.0.14206865, `cargo-ndk` | `cd apps/android && ./build.sh --debug` |
| iOS | macOS, Xcode, `cmake`, `ninja`, таргет `aarch64-apple-ios` | `cargo build --release --target aarch64-apple-ios -p burmalda57-ios --bin burmalda57-ios --features ios-app` |
| Web | таргет `wasm32-unknown-unknown`, `wasm-bindgen-cli` 0.2.130 | `./apps/web/build_web.sh` (полная) или `--demo-only` |

- **Android:** скрипт `build.sh` (Linux/macOS) и `build.ps1` (Windows) сами
  ставят недостающие зависимости; release требует `keystore.properties`.
- **iOS:** на выходе неподписанный `.ipa`, упаковка описана в
  [apps/ios/README.md](apps/ios/README.md).
- **Web:** результат лежит в `apps/web/web/` (`index.html` и `pkg/`).

---

## Релизы и CI

[![Android](https://github.com/vodolazny/burmalda57test/actions/workflows/android.yml/badge.svg)](https://github.com/vodolazny/burmalda57test/actions/workflows/android.yml)
[![iOS](https://github.com/vodolazny/burmalda57test/actions/workflows/ios.yml/badge.svg)](https://github.com/vodolazny/burmalda57test/actions/workflows/ios.yml)
[![Web](https://github.com/vodolazny/burmalda57test/actions/workflows/web.yml/badge.svg)](https://github.com/vodolazny/burmalda57test/actions/workflows/web.yml)

| Платформа | Что запускает | Результат |
|---|---|---|
| Android | тег `android-vX.Y.Z` | подписанный `burmalda57-X.Y.Z.apk` в Releases с пометкой *latest* |
| iOS | push в `main` (после smoke-теста и сборки IPA) | единственный релиз `ios-latest` с неподписанным `.ipa` |
| Web | push в `main` | демо-сборка на GitHub Pages (источник Pages — «GitHub Actions») |

Дополнительно push в `main` и pull request'ы собирают debug APK в артефактах
Actions.

- Пометка *latest* закреплена за Android: на неё опираются ссылка «скачать APK»
  и проверка обновлений в Android-приложении. Поэтому `ios-latest` публикуется с `make_latest: false`.
- Версия тега Android должна совпадать с `versionName` и версией Rust-крейта;
  `versionCode` увеличивается вручную.
- Проверка обновлений в iOS-приложении запрашивает релиз `ios-latest` и берёт
  версию из его названия (`iOS latest build X.Y.Z (unsigned)`). Баннер
  появится, когда версия в `apps/ios/rust/Cargo.toml` станет выше установленной.
  Android запрашивает `/releases/latest` и берёт версию из тега
  `android-vX.Y.Z`, поэтому пометка *latest* должна оставаться у Android-релиза.
- Тег `ios-latest` создаётся заранее: `GITHUB_TOKEN` не может создавать и
  двигать теги на коммитах, меняющих workflow.

Workflow: [android.yml](.github/workflows/android.yml),
[ios.yml](.github/workflows/ios.yml), [web.yml](.github/workflows/web.yml).

---

## Лицензия

Apache-2.0, см. [LICENSE](LICENSE).

Товарные знаки, названия и данные принадлежат их владельцам. Проект создан в
образовательных целях как удобный клиент к уже существующему сервису.
