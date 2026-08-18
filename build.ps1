#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Сборка burmalda57 (Android-клиент дневника obr57) на Windows.

.PARAMETER Debug_
    Собрать debug APK (по умолчанию).

.PARAMETER Release
    Собрать release APK (требует keystore.properties в корне проекта).

.PARAMETER Yes
    Не спрашивать подтверждение на установку зависимостей.

.EXAMPLE
    ./build.ps1
    ./build.ps1 -Release
    ./build.ps1 -Debug_ -Yes
#>

param(
    [switch]$Debug_,
    [switch]$Release,
    [switch]$Yes
)

$ErrorActionPreference = "Continue"

# ---------- требуемые версии ----------
$RequiredNdk          = "29.0.14206865"
$RequiredSdkPlatform  = "android-34"
$RequiredBuildTools   = "34.0.0"
$RequiredJdkMajor     = "17"
$AndroidTarget        = "aarch64-linux-android"

# ---------- вывод ----------
function Info($msg)  { Write-Host "[i] $msg" -ForegroundColor Cyan }
function Ok($msg)    { Write-Host "[ok] $msg" -ForegroundColor Green }
function Warn($msg)  { Write-Host "[!] $msg" -ForegroundColor Yellow }
function Err($msg)   { Write-Host "[x] $msg" -ForegroundColor Red }
function Die($msg)   { Err $msg; exit 1 }

function Confirm($prompt) {
    if ($Yes) { return $true }
    $reply = Read-Host "? $prompt [y/N]"
    return $reply -match '^(?i:y|yes|д|да)$'
}

# ---------- разбор флагов ----------
if ($Release -and $Debug_) {
    Die "Нельзя одновременно указать -Debug_ и -Release."
}
$BuildType = if ($Release) { "release" } else { "debug" }

# ---------- определение пакетного менеджера ----------
$PkgManager = $null
if (Get-Command winget -ErrorAction SilentlyContinue) {
    $PkgManager = "winget"
} elseif (Get-Command choco -ErrorAction SilentlyContinue) {
    $PkgManager = "choco"
} else {
    Warn "Не найден ни winget, ни choco. Автоустановка зависимостей будет недоступна — придётся ставить вручную."
}
if ($PkgManager) { Info "Пакетный менеджер: $PkgManager" }

function Install-Pkg {
    param(
        [string]$Description,
        [string]$WingetId,
        [string]$ChocoId
    )
    Warn "Не найдено: $Description"
    if (-not (Confirm "Установить '$Description' через $PkgManager?")) {
        Die "Без '$Description' сборка невозможна. Прервано пользователем."
    }
    if ($PkgManager -eq "winget") {
        winget install --id $WingetId -e --accept-source-agreements --accept-package-agreements
    } elseif ($PkgManager -eq "choco") {
        choco install $ChocoId -y
    } else {
        Die "Автоустановка недоступна. Установи '$Description' вручную."
    }
}

# ---------- проверки инструментов ----------

function Check-Rust {
    $rustc = Get-Command rustc -ErrorAction SilentlyContinue
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $rustc -or -not $cargo) {
        Warn "Rust toolchain не найден."
        if (Confirm "Установить Rust через rustup-init?") {
            if ($PkgManager -eq "winget") {
                winget install --id Rustlang.Rustup -e
            } else {
                Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile "$env:TEMP\rustup-init.exe"
                & "$env:TEMP\rustup-init.exe" -y
            }
            $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
        } else {
            Die "Rust обязателен для сборки."
        }
    }
    Ok "Rust: $(rustc --version)"

    $targets = rustup target list --installed 2>$null
    if ($targets -notmatch [regex]::Escape($AndroidTarget)) {
        Warn "Android target ($AndroidTarget) не установлен."
        if (Confirm "Выполнить 'rustup target add $AndroidTarget'?") {
            rustup target add $AndroidTarget
        } else {
            Die "Без target $AndroidTarget сборка невозможна."
        }
    }
    Ok "Rust target $AndroidTarget установлен"
}

function Find-VsWhere {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) { return $vswhere }
    return $null
}

function Check-BuildEssentials {
    # Определяем, какой линкер нужен рантайму Rust: MSVC (cl.exe) или GNU (gcc, для -gnu таргетов)
    $hostTriple = (rustc -vV | Select-String "host:").ToString()
    $usesGnu = $hostTriple -match "gnu"

    if ($usesGnu) {
        if (Get-Command gcc -ErrorAction SilentlyContinue) {
            Ok "MinGW/gcc toolchain найден"
            return
        }
        Warn "Rust-таргет использует GNU ABI, но gcc не найден."
        Install-Pkg -Description "MinGW-w64 (gcc)" `
            -WingetId "MSYS2.MSYS2" `
            -ChocoId "mingw"
        return
    }

    # MSVC-таргет (по умолчанию для rustup на Windows) — нужен cl.exe из VS Build Tools
    $vswhere = Find-VsWhere
    $hasVcTools = $false
    if ($vswhere) {
        $vcPath = & $vswhere -latest -products * `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -property installationPath
        if ($vcPath) { $hasVcTools = $true }
    }

    if ($hasVcTools) {
        Ok "MSVC C++ build tools найдены"
        return
    }

    Warn "Не найдены MSVC C++ build tools (cl.exe) — без них cargo упадёт на этапе линковки."
    if (-not (Confirm "Установить Visual Studio Build Tools (компонент C++ x64/x86) через winget?")) {
        Die "MSVC C++ build tools обязательны для сборки (либо переключись на GNU-таргет вручную)."
    }
    if ($PkgManager -ne "winget") {
        Die "Автоустановка VS Build Tools поддержана только через winget. Установи вручную: https://visualstudio.microsoft.com/visual-cpp-build-tools/"
    }
    winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
        --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    Ok "VS Build Tools установлены (может понадобиться перезапуск терминала)"
}

function Check-CargoNdk {
    if (-not (Get-Command cargo-ndk -ErrorAction SilentlyContinue)) {
        Warn "cargo-ndk не найден."
        if (Confirm "Выполнить 'cargo install cargo-ndk'?") {
            cargo install cargo-ndk
        } else {
            Die "cargo-ndk обязателен для сборки."
        }
    }
    Ok "cargo-ndk установлен"
}

function Update-SessionEnvironment {
    # winget/choco пишут PATH и JAVA_HOME в реестр, но уже запущенный процесс
    # PowerShell их не подхватывает без перезапуска — обновляем вручную.
    $machinePath = [System.Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = @($machinePath, $userPath, $env:Path) -join ";"

    $machineJavaHome = [System.Environment]::GetEnvironmentVariable("JAVA_HOME", "Machine")
    $userJavaHome = [System.Environment]::GetEnvironmentVariable("JAVA_HOME", "User")
    if ($userJavaHome) {
        $env:JAVA_HOME = $userJavaHome
    } elseif ($machineJavaHome) {
        $env:JAVA_HOME = $machineJavaHome
    }
}

function Find-JdkFallback {
    # Если реестр не подхватил JAVA_HOME (бывает у некоторых пакетов), ищем вручную.
    $roots = @(
        "C:\Program Files\Eclipse Adoptium",
        "C:\Program Files\Java"
    )
    foreach ($root in $roots) {
        if (Test-Path $root) {
            $jdkDir = Get-ChildItem $root -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match "jdk-?$RequiredJdkMajor" } |
                Select-Object -First 1
            if ($jdkDir) { return $jdkDir.FullName }
        }
    }
    return $null
}

function Check-Jdk {
    $java = Get-Command java -ErrorAction SilentlyContinue
    if (-not $java) {
        Install-Pkg -Description "JDK $RequiredJdkMajor" `
            -WingetId "EclipseAdoptium.Temurin.${RequiredJdkMajor}.JDK" `
            -ChocoId "temurin${RequiredJdkMajor}"

        Update-SessionEnvironment
        $java = Get-Command java -ErrorAction SilentlyContinue

        if (-not $java) {
            $fallback = Find-JdkFallback
            if ($fallback) {
                $env:JAVA_HOME = $fallback
                $env:Path = "$fallback\bin;$env:Path"
                Ok "JDK найден вручную: $fallback"
            } else {
                Die "JDK установлен, но не найден ни в PATH, ни в стандартных папках. Перезапусти терминал и запусти скрипт снова — тогда PATH подхватится автоматически."
            }
        } elseif (-not $env:JAVA_HOME) {
            # java нашёлся в PATH, но JAVA_HOME не выставлен — вычисляем из пути к java.exe
            $env:JAVA_HOME = Split-Path (Split-Path $java.Source -Parent) -Parent
        }
    }

    try {
        $verOutput = & java -version 2>&1 | Select-Object -First 1
        if ($verOutput -match '"(\d+)') {
            $jver = $matches[1]
            if ($jver -ne $RequiredJdkMajor) {
                Warn "Найдена Java версии $jver, а требуется $RequiredJdkMajor. Проверь JAVA_HOME."
            } else {
                Ok "JDK $RequiredJdkMajor найден (JAVA_HOME: $env:JAVA_HOME)"
            }
        }
    } catch {
        Warn "Не удалось определить версию Java."
    }
}

function Bootstrap-AndroidSdk {
    $defaultHome = Join-Path $env:LOCALAPPDATA "Android\Sdk"
    Warn "ANDROID_HOME не задан или указывает на несуществующую директорию."
    if (-not (Confirm "Скачать Android SDK cmdline-tools с нуля в $defaultHome?")) {
        Die "Установи Android SDK вручную и задай:`n  `$env:ANDROID_HOME = 'C:\путь\до\sdk'"
    }

    Info "Ищу актуальную ссылку на cmdline-tools (Windows)..."
    $page = Invoke-WebRequest -Uri "https://developer.android.com/studio" -UseBasicParsing
    $match = [regex]::Match($page.Content, 'https://dl\.google\.com/android/repository/commandlinetools-win-[0-9]+_latest\.zip')
    if (-not $match.Success) {
        Die "Не удалось найти ссылку на cmdline-tools автоматически. Скачай вручную с https://developer.android.com/studio#command-tools"
    }
    $toolsUrl = $match.Value

    Info "Качаю $toolsUrl ..."
    $tmpZip = Join-Path $env:TEMP "cmdline-tools.zip"
    Invoke-WebRequest -Uri $toolsUrl -OutFile $tmpZip

    $tmpExtract = Join-Path $env:TEMP "cmdline-tools-extract"
    if (Test-Path $tmpExtract) { Remove-Item $tmpExtract -Recurse -Force }
    Expand-Archive -Path $tmpZip -DestinationPath $tmpExtract -Force

    $destLatest = Join-Path $defaultHome "cmdline-tools\latest"
    New-Item -ItemType Directory -Force -Path (Join-Path $defaultHome "cmdline-tools") | Out-Null
    if (Test-Path $destLatest) { Remove-Item $destLatest -Recurse -Force }
    Move-Item (Join-Path $tmpExtract "cmdline-tools") $destLatest
    Remove-Item $tmpZip -Force
    Remove-Item $tmpExtract -Recurse -Force

    $env:ANDROID_HOME = $defaultHome
    Ok "cmdline-tools установлены в $defaultHome"
    Warn "Чтобы не повторять это каждый раз, задай переменные окружения в системе (Панель управления -> Переменные среды) или добавь в профиль PowerShell:`n  `$env:ANDROID_HOME = '$defaultHome'`n  `$env:Path += `";$defaultHome\cmdline-tools\latest\bin;$defaultHome\platform-tools`""

    Info "Принимаю лицензии SDK..."
    $sdkManager = Join-Path $defaultHome "cmdline-tools\latest\bin\sdkmanager.bat"
    $licenses = "y`n" * 10
    $prevPref = $global:PSNativeCommandUseErrorActionPreference
    $global:PSNativeCommandUseErrorActionPreference = $false
    $licenses | & $sdkManager --licenses 2>&1 | Out-Null
    $global:PSNativeCommandUseErrorActionPreference = $prevPref
}

function Check-AndroidSdk {
    $androidHome = $env:ANDROID_HOME
    if (-not $androidHome -or -not (Test-Path $androidHome)) {
        Bootstrap-AndroidSdk
        $androidHome = $env:ANDROID_HOME
    }
    Ok "ANDROID_HOME: $androidHome"

    $sdkManager = Join-Path $androidHome "cmdline-tools\latest\bin\sdkmanager.bat"

    if (-not (Test-Path (Join-Path $androidHome "platforms\$RequiredSdkPlatform"))) {
        Warn "Platform $RequiredSdkPlatform не найдена в SDK."
        if (Confirm "Установить через sdkmanager?") {
            & $sdkManager "platforms;$RequiredSdkPlatform"
        } else {
            Die "Нужна platform $RequiredSdkPlatform."
        }
    }
    Ok "Platform $RequiredSdkPlatform найдена"

    if (-not (Test-Path (Join-Path $androidHome "build-tools\$RequiredBuildTools"))) {
        Warn "Build-tools $RequiredBuildTools не найдены в SDK."
        if (Confirm "Установить через sdkmanager?") {
            & $sdkManager "build-tools;$RequiredBuildTools"
        } else {
            Die "Нужны build-tools $RequiredBuildTools."
        }
    }
    Ok "Build-tools $RequiredBuildTools найдены"

    if (-not (Test-Path (Join-Path $androidHome "platform-tools"))) {
        Warn "platform-tools (adb) не найдены в SDK."
        if (Confirm "Установить через sdkmanager?") {
            & $sdkManager "platform-tools"
            $env:Path += ";$(Join-Path $androidHome 'platform-tools')"
        } else {
            Warn "Без platform-tools установка APK через adb в конце будет недоступна."
        }
    } else {
        $env:Path += ";$(Join-Path $androidHome 'platform-tools')"
    }

    $ndkHome = $env:ANDROID_NDK_HOME
    if (-not $ndkHome -or -not (Test-Path $ndkHome)) {
        $expected = Join-Path $androidHome "ndk\$RequiredNdk"
        if (Test-Path $expected) {
            $env:ANDROID_NDK_HOME = $expected
            Ok "ANDROID_NDK_HOME найден автоматически: $expected"
        } else {
            Warn "NDK r$RequiredNdk не найден по пути $expected."
            if (Confirm "Установить NDK $RequiredNdk через sdkmanager?") {
                & $sdkManager "ndk;$RequiredNdk"
                $env:ANDROID_NDK_HOME = $expected
            } else {
                Die "ANDROID_NDK_HOME обязателен для сборки."
            }
        }
    }
    Ok "ANDROID_NDK_HOME: $($env:ANDROID_NDK_HOME)"
}

function Check-Adb {
    return [bool](Get-Command adb -ErrorAction SilentlyContinue)
}

# ---------- запуск проверок ----------
Info "Проверка зависимостей..."
Check-Rust
Check-BuildEssentials
Check-CargoNdk
Check-Jdk
Check-AndroidSdk
Ok "Все зависимости на месте."

# ---------- сборка ----------
Set-Location $PSScriptRoot

if ($BuildType -eq "release") {
    if (-not (Test-Path "keystore.properties")) {
        Die "Для release-сборки нужен файл keystore.properties в корне проекта (см. README, раздел «Release-сборка»)."
    }
    Info "Собираю RELEASE APK..."
    & .\gradlew.bat :app:assembleRelease
    $ApkPath = "app\build\outputs\apk\release\app-release.apk"
} else {
    Info "Собираю DEBUG APK..."
    & .\gradlew.bat :app:assembleDebug
    $ApkPath = "app\build\outputs\apk\debug\app-debug.apk"
}

if (Test-Path $ApkPath) {
    Ok "Готово: $ApkPath"
} else {
    Die "Сборка завершилась, но APK не найден по ожидаемому пути ($ApkPath)."
}

# ---------- установка на устройство ----------
if (Check-Adb) {
    $devicesOutput = & adb devices 2>$null
    $deviceLine = ($devicesOutput -split "`n") | Select-Object -Skip 1 | Where-Object { $_.Trim() -ne "" -and $_ -match "\tdevice$" }

    if ($deviceLine) {
        $deviceId = ($deviceLine -split "\s+")[0]
        if (Confirm "Обнаружено устройство ($deviceId) по ADB. Установить APK сейчас?") {
            & adb install -r $ApkPath
            Ok "APK установлен на устройство."
        } else {
            Info "Установка пропущена. APK лежит здесь: $ApkPath"
        }
    } else {
        if (Confirm "Устройство по ADB не обнаружено. Подключить устройство и повторить проверку?") {
            Info "Жду устройство (adb wait-for-device, Ctrl+C для отмены)..."
            & adb wait-for-device
            if (Confirm "Устройство появилось. Установить APK?") {
                & adb install -r $ApkPath
                Ok "APK установлен на устройство."
            }
        } else {
            Info "Установка пропущена. APK лежит здесь: $ApkPath"
        }
    }
} else {
    Warn "adb недоступен — установка пропущена. APK лежит здесь: $ApkPath"
}

Ok "Готово."
