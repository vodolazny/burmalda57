#!/usr/bin/env bash
#
# build.sh — сборка burmalda57 (Android-клиент дневника obr57)
#
# Использование:
#   ./build.sh --debug     # debug-сборка (по умолчанию)
#   ./build.sh --release   # release-сборка (требует keystore.properties)
#   ./build.sh --help
#
set -euo pipefail

# ---------- цвета ----------
if [ -t 1 ]; then
    C_RESET='\033[0m'; C_RED='\033[31m'; C_GREEN='\033[32m'
    C_YELLOW='\033[33m'; C_BLUE='\033[34m'; C_BOLD='\033[1m'
else
    C_RESET=''; C_RED=''; C_GREEN=''; C_YELLOW=''; C_BLUE=''; C_BOLD=''
fi

info()  { echo -e "${C_BLUE}[i]${C_RESET} $*"; }
ok()    { echo -e "${C_GREEN}[ok]${C_RESET} $*"; }
warn()  { echo -e "${C_YELLOW}[!]${C_RESET} $*"; }
err()   { echo -e "${C_RED}[x]${C_RESET} $*" >&2; }
die()   { err "$*"; exit 1; }

confirm() {
    # confirm "вопрос" -> 0 если да
    local prompt="$1"
    local reply
    read -r -p "$(echo -e "${C_YELLOW}?${C_RESET} ${prompt} [y/N] ")" reply
    [[ "$reply" =~ ^([yY]|[yY][eE][sS]|д|да)$ ]]
}

# ---------- параметры ----------
BUILD_TYPE="debug"
REQUIRED_NDK="29.0.14206865"
REQUIRED_SDK_PLATFORM="android-34"
REQUIRED_BUILD_TOOLS="34.0.0"
REQUIRED_JDK_MAJOR="17"
ANDROID_TARGET="aarch64-linux-android"

usage() {
    cat <<EOF
Использование: $0 [--debug|--release] [--yes]

  --debug      собрать debug APK (по умолчанию)
  --release    собрать release APK (нужен keystore.properties в корне проекта)
  --yes        не спрашивать подтверждение на установку зависимостей
  --help       показать эту справку
EOF
}

AUTO_YES=0
for arg in "$@"; do
    case "$arg" in
        --debug) BUILD_TYPE="debug" ;;
        --release) BUILD_TYPE="release" ;;
        --yes|-y) AUTO_YES=1 ;;
        --help|-h) usage; exit 0 ;;
        *) die "Неизвестный флаг: $arg (см. --help)" ;;
    esac
done

confirm_or_auto() {
    [ "$AUTO_YES" -eq 1 ] && return 0
    confirm "$1"
}

# ---------- определение дистрибутива ----------
detect_distro() {
    if [ "$(uname -s)" != "Linux" ]; then
        echo "unsupported"
        return
    fi
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        case "$ID" in
            ubuntu|debian|linuxmint|pop) echo "debian" ;;
            fedora|rhel|centos|rocky|almalinux) echo "fedora" ;;
            arch|manjaro|endeavouros) echo "arch" ;;
            opensuse*|suse) echo "suse" ;;
            gentoo) echo "gentoo" ;;
            *) echo "unknown" ;;
        esac
    else
        echo "unknown"
    fi
}

DISTRO=$(detect_distro)
info "Обнаружена платформа: ${C_BOLD}${DISTRO}${C_RESET}"

pkg_install() {
    # pkg_install "человекочитаемое имя" pkg1 pkg2 ...
    local desc="$1"; shift
    local pkgs=("$@")
    warn "Не найдено: ${desc}"
    if ! confirm_or_auto "Установить (${pkgs[*]}) через системный пакетный менеджер?"; then
        die "Без ${desc} сборка невозможна. Прервано пользователем."
    fi
    case "$DISTRO" in
        debian) sudo apt-get update && sudo apt-get install -y "${pkgs[@]}" ;;
        fedora) sudo dnf install -y "${pkgs[@]}" ;;
        arch)   sudo pacman -S --noconfirm "${pkgs[@]}" ;;
        suse)   sudo zypper install -y "${pkgs[@]}" ;;
        gentoo) sudo emerge --ask=n "${pkgs[@]}" ;;
        macos)  brew install "${pkgs[@]}" ;;
        *) die "Автоустановка не поддерживается на этой платформе. Установи вручную: ${pkgs[*]}" ;;
    esac
}

if [ "$(uname -s)" = "Darwin" ]; then
    DISTRO="macos"
    command -v brew >/dev/null 2>&1 || die "Нужен Homebrew (https://brew.sh) для автоустановки зависимостей на macOS."
fi

# ---------- проверки инструментов ----------

check_build_essentials() {
    if command -v cc >/dev/null 2>&1 && command -v make >/dev/null 2>&1; then
        ok "Базовый toolchain сборки (cc, make) найден"
        return
    fi

    if [ "$DISTRO" = "macos" ]; then
        warn "На macOS нужны Xcode Command Line Tools (нет cc/make)."
        if confirm_or_auto "Выполнить 'xcode-select --install'?"; then
            xcode-select --install
            die "Установка запущена в отдельном окне — доведи её до конца и перезапусти скрипт."
        else
            die "Xcode Command Line Tools обязательны для сборки."
        fi
    fi

    pkg_install "базовый toolchain сборки (gcc/cc, make и т.п. — нужен cargo для линковки и сборки нативных зависимостей)" \
        $( [ "$DISTRO" = "debian" ] && echo "build-essential" ) \
        $( [ "$DISTRO" = "fedora" ] && echo "@development-tools" ) \
        $( [ "$DISTRO" = "arch" ] && echo "base-devel" ) \
        $( [ "$DISTRO" = "suse" ] && echo "-t pattern devel_basis" ) \
        $( [ "$DISTRO" = "gentoo" ] && echo "sys-devel/gcc sys-devel/make" )

    command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 || die "Toolchain сборки всё ещё не найден после установки — проверь вручную."
    ok "Базовый toolchain сборки установлен"
}

check_rust() {
    if ! command -v rustc >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
        warn "Rust toolchain не найден."
        if confirm_or_auto "Установить Rust через rustup (https://rustup.rs)?"; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source "$HOME/.cargo/env"
        else
            die "Rust обязателен для сборки."
        fi
    fi
    ok "Rust: $(rustc --version)"

    if ! rustup target list --installed 2>/dev/null | grep -q "$ANDROID_TARGET"; then
        warn "Android target ($ANDROID_TARGET) не установлен."
        if confirm_or_auto "Выполнить 'rustup target add $ANDROID_TARGET'?"; then
            rustup target add "$ANDROID_TARGET"
        else
            die "Без target $ANDROID_TARGET сборка невозможна."
        fi
    fi
    ok "Rust target $ANDROID_TARGET установлен"
}

check_cargo_ndk() {
    if ! command -v cargo-ndk >/dev/null 2>&1; then
        warn "cargo-ndk не найден."
        if confirm_or_auto "Выполнить 'cargo install cargo-ndk'?"; then
            cargo install cargo-ndk
        else
            die "cargo-ndk обязателен для сборки."
        fi
    fi
    ok "cargo-ndk установлен"
}

check_jdk() {
    if ! command -v java >/dev/null 2>&1; then
        pkg_install "JDK ${REQUIRED_JDK_MAJOR}" \
            $( [ "$DISTRO" = "debian" ] && echo "openjdk-${REQUIRED_JDK_MAJOR}-jdk" ) \
            $( [ "$DISTRO" = "fedora" ] && echo "java-${REQUIRED_JDK_MAJOR}-openjdk-devel" ) \
            $( [ "$DISTRO" = "arch" ] && echo "jdk${REQUIRED_JDK_MAJOR}-openjdk" ) \
            $( [ "$DISTRO" = "gentoo" ] && echo "virtual/jdk:${REQUIRED_JDK_MAJOR}" ) \
            $( [ "$DISTRO" = "macos" ] && echo "openjdk@${REQUIRED_JDK_MAJOR}" )
    fi
    local jver
    jver=$(java -version 2>&1 | head -n1 | grep -oE '"[0-9]+' | tr -d '"' || echo "0")
    if [ "$jver" != "$REQUIRED_JDK_MAJOR" ]; then
        warn "Найдена Java версии ${jver}, а требуется ${REQUIRED_JDK_MAJOR}. Убедись, что JAVA_HOME указывает на нужную версию."
    else
        ok "JDK ${REQUIRED_JDK_MAJOR} найден"
    fi
}

bootstrap_android_sdk() {
    local default_home="$HOME/Android/Sdk"
    warn "ANDROID_HOME не задан или указывает на несуществующую директорию."
    if ! confirm_or_auto "Скачать Android SDK cmdline-tools с нуля в $default_home?"; then
        die "Установи Android SDK вручную и экспортируй:
  export ANDROID_HOME=/путь/до/sdk"
    fi

    command -v unzip >/dev/null 2>&1 || pkg_install "unzip" \
        $( [ "$DISTRO" = "debian" ] && echo "unzip" ) \
        $( [ "$DISTRO" = "fedora" ] && echo "unzip" ) \
        $( [ "$DISTRO" = "arch" ] && echo "unzip" ) \
        $( [ "$DISTRO" = "gentoo" ] && echo "app-arch/unzip" ) \
        $( [ "$DISTRO" = "macos" ] && echo "unzip" )
    command -v curl >/dev/null 2>&1 || pkg_install "curl" \
        $( [ "$DISTRO" = "debian" ] && echo "curl" ) \
        $( [ "$DISTRO" = "fedora" ] && echo "curl" ) \
        $( [ "$DISTRO" = "arch" ] && echo "curl" ) \
        $( [ "$DISTRO" = "gentoo" ] && echo "net-misc/curl" ) \
        $( [ "$DISTRO" = "macos" ] && echo "curl" )

    local platform_tag="linux"
    [ "$DISTRO" = "macos" ] && platform_tag="mac"

    info "Ищу актуальную ссылку на cmdline-tools ($platform_tag)..."
    local tools_url
    tools_url=$(curl -fsSL https://developer.android.com/studio \
        | grep -oE "https://dl\.google\.com/android/repository/commandlinetools-${platform_tag}-[0-9]+_latest\.zip" \
        | head -n1)
    [ -n "$tools_url" ] || die "Не удалось найти ссылку на cmdline-tools автоматически. Скачай вручную с https://developer.android.com/studio#command-tools"

    info "Качаю $tools_url ..."
    local tmp_zip
    tmp_zip=$(mktemp --suffix=.zip 2>/dev/null || mktemp)
    curl -fL --progress-bar -o "$tmp_zip" "$tools_url" || die "Не удалось скачать cmdline-tools."

    mkdir -p "$default_home/cmdline-tools"
    local tmp_extract
    tmp_extract=$(mktemp -d)
    unzip -q "$tmp_zip" -d "$tmp_extract" || die "Не удалось распаковать cmdline-tools."
    rm -rf "$default_home/cmdline-tools/latest"
    mv "$tmp_extract/cmdline-tools" "$default_home/cmdline-tools/latest"
    rm -rf "$tmp_zip" "$tmp_extract"

    export ANDROID_HOME="$default_home"
    ok "cmdline-tools установлены в $ANDROID_HOME"
    warn "Добавь в свой ~/.bashrc или ~/.zshrc, чтобы не повторять это каждый раз:
  export ANDROID_HOME=\"$default_home\"
  export PATH=\"\$PATH:\$ANDROID_HOME/cmdline-tools/latest/bin:\$ANDROID_HOME/platform-tools\""

    info "Принимаю лицензии SDK..."
    yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null 2>&1 || true
}

check_android_sdk() {
    if [ -z "${ANDROID_HOME:-}" ] || [ ! -d "${ANDROID_HOME:-/nonexistent}" ]; then
        bootstrap_android_sdk
    fi
    ok "ANDROID_HOME: $ANDROID_HOME"

    if [ ! -d "$ANDROID_HOME/platforms/$REQUIRED_SDK_PLATFORM" ]; then
        warn "Platform $REQUIRED_SDK_PLATFORM не найдена в SDK."
        if confirm_or_auto "Установить через sdkmanager?"; then
            "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "platforms;${REQUIRED_SDK_PLATFORM}"
        else
            die "Нужна platform $REQUIRED_SDK_PLATFORM."
        fi
    fi
    ok "Platform $REQUIRED_SDK_PLATFORM найдена"

    if [ ! -d "$ANDROID_HOME/build-tools/$REQUIRED_BUILD_TOOLS" ]; then
        warn "Build-tools $REQUIRED_BUILD_TOOLS не найдены в SDK."
        if confirm_or_auto "Установить через sdkmanager?"; then
            "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "build-tools;${REQUIRED_BUILD_TOOLS}"
        else
            die "Нужны build-tools $REQUIRED_BUILD_TOOLS."
        fi
    fi
    ok "Build-tools $REQUIRED_BUILD_TOOLS найдены"

    if [ ! -d "$ANDROID_HOME/platform-tools" ]; then
        warn "platform-tools (adb) не найдены в SDK."
        if confirm_or_auto "Установить через sdkmanager?"; then
            "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "platform-tools"
            export PATH="$PATH:$ANDROID_HOME/platform-tools"
        else
            warn "Без platform-tools установка APK через adb в конце будет недоступна."
        fi
    else
        export PATH="$PATH:$ANDROID_HOME/platform-tools"
    fi

    if [ -z "${ANDROID_NDK_HOME:-}" ] || [ ! -d "${ANDROID_NDK_HOME:-/nonexistent}" ]; then
        local expected="$ANDROID_HOME/ndk/$REQUIRED_NDK"
        if [ -d "$expected" ]; then
            export ANDROID_NDK_HOME="$expected"
            ok "ANDROID_NDK_HOME найден автоматически: $ANDROID_NDK_HOME"
        else
            warn "NDK r$REQUIRED_NDK не найден по пути $expected."
            if confirm_or_auto "Установить NDK $REQUIRED_NDK через sdkmanager?"; then
                "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "ndk;${REQUIRED_NDK}"
                export ANDROID_NDK_HOME="$expected"
            else
                die "ANDROID_NDK_HOME обязателен для сборки."
            fi
        fi
    fi
    ok "ANDROID_NDK_HOME: $ANDROID_NDK_HOME"
}

check_adb() {
    if ! command -v adb >/dev/null 2>&1; then
        warn "adb не найден в PATH (обычно idет в составе platform-tools Android SDK)."
        return 1
    fi
    return 0
}

# ---------- запуск проверок ----------
info "Проверка зависимостей..."
check_build_essentials
check_rust
check_cargo_ndk
check_jdk
check_android_sdk
ok "Все зависимости на месте."

# ---------- сборка ----------
cd "$(dirname "$0")"

if [ "$BUILD_TYPE" = "release" ]; then
    if [ ! -f "keystore.properties" ]; then
        die "Для release-сборки нужен файл keystore.properties в корне проекта (см. README, раздел «Release-сборка»)."
    fi
    info "Собираю RELEASE APK..."
    ./gradlew :app:assembleRelease
    APK_PATH="app/build/outputs/apk/release/app-release.apk"
else
    info "Собираю DEBUG APK..."
    ./gradlew :app:assembleDebug
    APK_PATH="app/build/outputs/apk/debug/app-debug.apk"
fi

if [ -f "$APK_PATH" ]; then
    ok "Готово: $APK_PATH"
else
    die "Сборка завершилась, но APK не найден по ожидаемому пути ($APK_PATH)."
fi

# ---------- установка на устройство ----------
if check_adb; then
    if adb get-state >/dev/null 2>&1; then
        DEVICE_INFO=$(adb devices | sed -n '2p' | awk '{print $1}')
        if confirm "Обнаружено устройство ($DEVICE_INFO) по ADB. Установить APK сейчас?"; then
            adb install -r "$APK_PATH"
            ok "APK установлен на устройство."
        else
            info "Установка пропущена. APK лежит здесь: $APK_PATH"
        fi
    else
        if confirm "Устройство по ADB не обнаружено. Подключить устройство и повторить проверку?"; then
            info "Жду устройство (adb wait-for-device, Ctrl+C для отмены)..."
            adb wait-for-device
            if confirm "Устройство появилось. Установить APK?"; then
                adb install -r "$APK_PATH"
                ok "APK установлен на устройство."
            fi
        else
            info "Установка пропущена. APK лежит здесь: $APK_PATH"
        fi
    fi
else
    warn "adb недоступен — установка пропущена. APK лежит здесь: $APK_PATH"
fi

ok "Готово."
