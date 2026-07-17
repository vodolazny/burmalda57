# 📚 Электронный дневник obr57 (burmalda57)

Неофициальный Android-клиент электронного дневника Орловской области ([obr57.ru](https://obr57.ru/gis)).
Написан на **Rust + [Slint](https://slint.dev)** (рендер FemtoVG/OpenGL), нативная сборка через `cargo-ndk` + Gradle.

![platform](https://img.shields.io/badge/platform-Android%208.0%2B-brightgreen)
![arch](https://img.shields.io/badge/arch-arm64--v8a-blue)
![license](https://img.shields.io/badge/license-Apache--2.0-lightgrey)

> ⚠️ **Неофициальный проект.** Не связан с Департаментом образования Орловской области, порталом Госуслуг/ЕСИА или разработчиками официального дневника. Используется на свой страх и риск.
> ⚠️ **БЕТА** Приложение еще сырое, возможны сбои и ошибки.
---

## ✨ Возможности

- 📊 Оценки по предметам: периоды, веса, средние баллы
- 📈 Графики успеваемости
- 📅 Дневник, расписание и домашние задания
- 🎓 Итоговые оценки за периоды
- 🗓️ События
- 👤 Профиль: ФИО, школа, класс, аватар
- 🔔 Пуш-уведомления о новых оценках (фоновая проверка раз в 30 минут)

---

## 🔒 Приватность и безопасность

### «А если я не доверяю Госуслугам / стороннему приложению?»

Это честный вопрос, поэтому коротко о том, как всё устроено:

- **Приложение полностью открытое (open source).** Весь код в этом репозитории — можно прочитать, проверить и собрать самому (см. [Сборка из исходников](#-сборка-из-исходников)). Если не доверяешь готовому APK — не ставь его, а собери бинарь сам из этих же исходников.
- **Пароль от Госуслуг приложение не видит и не хранит.** Вход через **ЕСИА** открывается в системном WebView **на официальном портале ЕСИА**(в web-view открывается точно та же страница что и в оригинальном приложении) — логин и пароль ты вводишь на настоящей странице госуслуг, а не в полях приложения.
- **Хранится только результат входа** — сессионная кука и `apikey`, причём в зашифрованном виде. Используется envelope-шифрование: данные шифруются ключом AES-256-GCM, а сам ключ лежит в **аппаратном хранилище Android Keystore** (StrongBox/TEE) и физически не покидает устройство.
- **Данные уходят только на официальные серверы** регионального дневника (`mp2.obr57.ru`). Никаких сторонних бэкендов у приложения нет.
- **Единственное внешнее исключение — Sentry** (сбор отчётов о падениях приложения). Он не собирает твои оценки/логины, только технические данные о крашах. Его можно полностью выключить, собрав приложение без него — см. [Отключить Sentry](#-отключить-sentry).
- Никакой рекламы, трекеров и аналитики поведения.

---

## 🛠️ Технологии

| Слой | Что используется |
|------|------------------|
| UI | Slint 1.x (рендер `renderer-femtovg`, OpenGL) |
| Логика/сеть/крипта | Rust (`cdylib`) |
| Платформа | `android-activity` (NativeActivity) |
| HTTP/TLS | `reqwest` + `rustls` (crypto-провайдер `ring`) |
| Async | `tokio` |
| Крашлитика | `sentry` (транспорт `ureq`) |
| Kotlin-хелперы | Android Keystore, уведомления, выбор аватара, вход через ЕСИА |

---

## 📦 Сборка из исходников

### Требования

- **Rust** (stable) + Android-таргет:
  ```bash
  rustup target add aarch64-linux-android
  ```
- **cargo-ndk**:
  ```bash
  cargo install cargo-ndk
  ```
- **Android SDK** — platform `android-34`, build-tools `34.0.0`
- **Android NDK** — r29 (`29.0.14206865`)
- **JDK 17**
- Устройство или эмулятор **arm64-v8a**, Android **8.0+** (minSdk 26)

### Переменные окружения

```bash
export ANDROID_HOME=/opt/android-sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/29.0.14206865
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk
```

> Нативную библиотеку `.so` Gradle собирает автоматически: перед упаковкой APK запускается задача `cargoNdkBuild` (см. `app/build.gradle`), которая дёргает `cargo-ndk`. Отдельно ничего вызывать не нужно.

### Debug-сборка

```bash
./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

### Release-сборка (с подписью)

1. Создай свой keystore (**один раз, храни и бэкапь надёжно** — тем же ключом придётся подписывать все обновления):
   ```bash
   keytool -genkeypair -v \
     -keystore my-release.jks \
     -alias my-alias \
     -keyalg RSA -keysize 4096 -validity 10000
   ```
2. Создай в корне проекта `keystore.properties`:
   ```properties
   storeFile=../my-release.jks
   storePassword=ПАРОЛЬ_KEYSTORE
   keyAlias=my-alias
   keyPassword=ПАРОЛЬ_КЛЮЧА
   ```
3. Собери:
   ```bash
   ./gradlew :app:assembleRelease
   ```
   Готовый файл: `app/build/outputs/apk/release/app-release.apk`
4. Проверь подпись:
   ```bash
   $ANDROID_HOME/build-tools/34.0.0/apksigner verify --print-certs \
     app/build/outputs/apk/release/app-release.apk
   ```

---

## 🧯 Отключить Sentry

Если не хочешь, чтобы приложение отправляло отчёты о падениях:

1. В `rust/Cargo.toml` убери зависимость `sentry` (и её фичи).
2. В `rust/src/lib.rs` удали инициализацию Sentry (`sentry::init(...)` и связанный `_guard`).
3. Пересобери — приложение будет работать полностью офлайн от какой-либо телеметрии.

---

## 🗂️ Структура проекта

```
.
├── app/                      # Android-обёртка (Gradle)
│   ├── build.gradle          # сборка + задача cargoNdkBuild + подпись
│   ├── proguard-rules.pro    # keep-правила для JNI-классов
│   └── src/main/
│       ├── AndroidManifest.xml
│       ├── java/ru/burmalda/journal/   # Kotlin-хелперы (Keystore, уведомления, ЕСИА, аватар)
│       └── res/                        # иконки
├── rust/                     # ядро на Rust
│   ├── Cargo.toml
│   ├── build.rs
│   ├── src/                  # логика: login, marks, diary, crypto, notify, net, ...
│   └── ui/app.slint          # интерфейс на Slint
├── build.gradle
└── settings.gradle
```

---

## 📄 Лицензия

[Apache License 2.0](LICENSE).

Товарные знаки, названия и данные принадлежат их владельцам. Проект создан в образовательных целях как удобный клиент к уже существующему сервису.

(мне это для олимпиад и конкурсов, так что пишите отзывы пожалуйста)
