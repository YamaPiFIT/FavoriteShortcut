<#
.SYNOPSIS
    お気に入りショートカット（Rust 版）の配布用ビルド。

.DESCRIPTION
    cargo で検証（テスト）と最適化ビルドを行い、build\rust\FavoriteShortcutRs.exe を作ります。
    あわせてポータブル版 ZIP（build\FavoriteShortcutRs-portable.zip）も作成します。

    Rust（stable）と Visual Studio Build Tools（C++ によるデスクトップ開発）が必要です。
    依存ライブラリは Cargo.lock に書かれた版をそのまま使います（--locked）。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-rust.ps1
#>
[CmdletBinding()]
param(
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$rustDir = Join-Path $root 'rust'
$buildDir = Join-Path $root 'build'
$outDir = Join-Path $buildDir 'rust'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo が見つかりません。https://www.rust-lang.org/tools/install から Rust をインストールしてください。'
}

Write-Host ''
Write-Host '=== お気に入りショートカット（Rust 版）ビルド ===' -ForegroundColor Cyan
Write-Host "ルート  : $root"
& cargo --version
Write-Host ''

Push-Location $rustDir
try {
    # --- テスト ---------------------------------------------------------
    if (-not $SkipTests) {
        Write-Host '[1/3] 検証（cargo test）を実行します...' -ForegroundColor Yellow
        # cargo は進み具合を標準エラーに出す。パイプに流すと Windows PowerShell 5.1 ではエラー扱いになるので、そのまま表示する
        & cargo test --locked
        if ($LASTEXITCODE -ne 0) {
            throw '検証に失敗しました。ビルドを中止します。'
        }
        Write-Host ''
    }
    else {
        Write-Host '[1/3] 検証はスキップされました。' -ForegroundColor DarkGray
    }

    # --- EXE ------------------------------------------------------------
    Write-Host '[2/3] EXE を作成します（cargo build --release）...' -ForegroundColor Yellow
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) {
        throw 'ビルドに失敗しました。'
    }
}
finally {
    Pop-Location
}

$exe = Join-Path $rustDir 'target\release\FavoriteShortcutRs.exe'
if (-not (Test-Path $exe)) { throw "$exe が生成されませんでした。" }

if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
Copy-Item $exe $outDir -Force
$outExe = Join-Path $outDir 'FavoriteShortcutRs.exe'

$sizeMb = [math]::Round((Get-Item $outExe).Length / 1MB, 1)
Write-Host ("  -> {0}  ({1} MB)" -f $outExe, $sizeMb) -ForegroundColor Green

# --- ポータブル版 ZIP ---------------------------------------------------

Write-Host ''
Write-Host '[3/3] ポータブル版 ZIP を作成します...' -ForegroundColor Yellow

$stage = Join-Path $buildDir '_portable_rust'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item $outExe $stage -Force
# C# 版の ZIP と同じフォルダに展開しても上書きし合わないよう、説明とライセンス表記は名前を分ける
Copy-Item (Join-Path $rustDir 'README.md') (Join-Path $stage 'README-Rust.md') -Force
Copy-Item (Join-Path $rustDir 'THIRD-PARTY-LICENSES.md') (Join-Path $stage 'THIRD-PARTY-LICENSES-Rust.md') -Force
Copy-Item (Join-Path $root 'LICENSE') $stage -Force

$zipPath = Join-Path $buildDir 'FavoriteShortcutRs-portable.zip'
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
Remove-Item $stage -Recurse -Force

$zipMb = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
Write-Host ("  -> {0}  ({1} MB)" -f $zipPath, $zipMb) -ForegroundColor Green

Write-Host ''
Write-Host '完了しました。' -ForegroundColor Cyan
Write-Host 'FavoriteShortcutRs.exe を FavoriteShortcut.exe と同じフォルダに置くと、同じデータを使います。'
