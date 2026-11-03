# Сборка веб-версии (wasm)
param([switch]$DemoOnly)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$workspace = Split-Path (Split-Path $root -Parent) -Parent

$buildArgs = @()
if ($DemoOnly) { $buildArgs += '--features', 'demo-only' }

# 1. Сборка wasm-библиотеки
Push-Location "$root"
cargo build --release --target wasm32-unknown-unknown -p burmalda57-web @buildArgs
Pop-Location

# 2. Генерация JS-обвязки (wasm-bindgen CLI)
#    Ставится один раз: cargo install wasm-bindgen-cli
wasm-bindgen `
    "$workspace\target\wasm32-unknown-unknown\release\burmalda57.wasm" `
    --out-dir "$root\web\pkg" `
    --target web --remove-name-section

# 3. Дополнительная оптимизация размера (wasm-opt из Binaryen), если установлена
if (Get-Command wasm-opt -ErrorAction SilentlyContinue) {
    Write-Host "==> Оптимизация размера WASM (wasm-opt -Oz)..."
    wasm-opt --all-features -Oz --strip-debug -o "$root\web\pkg\burmalda57_bg.wasm" "$root\web\pkg\burmalda57_bg.wasm"
} else {
    Write-Host "==> Подсказка: wasm-opt не найден, пропускаем post-оптимизацию"
}

Write-Host ""
Write-Host "Готово. Для локального запуска (нужен http-сервер, file:// не работает):"
Write-Host "  cd $root\web"
Write-Host "  python -m http.server 8080"
Write-Host "и открой http://localhost:8080"
