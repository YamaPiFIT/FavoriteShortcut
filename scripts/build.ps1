<#
.SYNOPSIS
    お気に入りショートカット の配布用ビルド。

.DESCRIPTION
    dotnet publish で単一ファイルの EXE を作り、build\ に出力します。
    あわせてポータブル版 ZIP も作成します。

.PARAMETER Mode
    SelfContained       .NET ランタイム同梱。配布先に .NET のインストールが不要（既定）
    FrameworkDependent  .NET 8 デスクトップランタイムが必要。サイズは 1/20 程度
    Both                両方

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1
    powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -Mode Both
#>
[CmdletBinding()]
param(
    [ValidateSet('SelfContained', 'FrameworkDependent', 'Both')]
    [string]$Mode = 'SelfContained',

    [string]$Runtime = 'win-x64',

    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$project = Join-Path $root 'src\FavoriteShortcut\FavoriteShortcut.csproj'
$buildDir = Join-Path $root 'build'

Write-Host ''
Write-Host '=== お気に入りショートカット ビルド ===' -ForegroundColor Cyan
Write-Host "ルート  : $root"
Write-Host "モード  : $Mode"
Write-Host "ランタイム: $Runtime"
Write-Host ''

# --- テスト -------------------------------------------------------------

if (-not $SkipTests) {
    Write-Host '[1/3] 検証プログラムを実行します...' -ForegroundColor Yellow
    & dotnet run --project (Join-Path $root 'tests\FavoriteShortcut.Tests') -c Release --nologo
    if ($LASTEXITCODE -ne 0) {
        throw '検証に失敗しました。ビルドを中止します。'
    }
    Write-Host ''
}
else {
    Write-Host '[1/3] 検証はスキップされました。' -ForegroundColor DarkGray
}

# --- 発行 ---------------------------------------------------------------

function Publish-App {
    param(
        [string]$Name,
        [bool]$SelfContained
    )

    $outDir = Join-Path $buildDir $Name
    if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null

    # $args は自動変数なので別名を使う
    $publishArgs = @(
        'publish', $project,
        '-c', 'Release',
        '-r', $Runtime,
        '-o', $outDir,
        '--nologo',
        "-p:PublishSingleFile=true",
        "-p:IncludeNativeLibrariesForSelfExtract=true",
        "-p:DebugType=none",
        "--self-contained", $(if ($SelfContained) { 'true' } else { 'false' })
    )

    if ($SelfContained) {
        # 圧縮と ReadyToRun は自己完結版でのみ指定できる
        # （圧縮でサイズが約 160MB → 約 70MB、ReadyToRun で起動が速くなる）
        $publishArgs += '-p:EnableCompressionInSingleFile=true'
        $publishArgs += '-p:PublishReadyToRun=true'
    }

    # dotnet の出力が戻り値に混ざらないよう、明示的に画面へ流す
    & dotnet @publishArgs | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "$Name の発行に失敗しました。" }

    $exe = Join-Path $outDir 'FavoriteShortcut.exe'
    if (-not (Test-Path $exe)) { throw "$exe が生成されませんでした。" }

    $sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host ("  -> {0}  ({1} MB)" -f $exe, $sizeMb) -ForegroundColor Green
    return $outDir
}

Write-Host '[2/3] EXE を発行します...' -ForegroundColor Yellow

$selfContainedDir = $null
$frameworkDir = $null

if ($Mode -eq 'SelfContained' -or $Mode -eq 'Both') {
    Write-Host '  自己完結版 (.NET 同梱)'
    $selfContainedDir = Publish-App -Name 'self-contained' -SelfContained $true
}

if ($Mode -eq 'FrameworkDependent' -or $Mode -eq 'Both') {
    Write-Host '  ランタイム依存版'
    $frameworkDir = Publish-App -Name 'framework-dependent' -SelfContained $false
}

# --- ポータブル版 ZIP ---------------------------------------------------

Write-Host ''
Write-Host '[3/3] ポータブル版 ZIP を作成します...' -ForegroundColor Yellow

$portableSource = if ($selfContainedDir) { $selfContainedDir } else { $frameworkDir }
$stage = Join-Path $buildDir '_portable'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item (Join-Path $portableSource '*') $stage -Recurse -Force

# データは EXE と同じ場所の Data フォルダに作られるので、
# この ZIP を展開したフォルダごとコピーすれば別PCへそのまま持ち運べる。

$readme = Join-Path $root 'README.md'
if (Test-Path $readme) { Copy-Item $readme $stage -Force }

$zipPath = Join-Path $buildDir 'FavoriteShortcut-portable.zip'
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
Remove-Item $stage -Recurse -Force

$zipMb = [math]::Round((Get-Item $zipPath).Length / 1MB, 1)
Write-Host ("  -> {0}  ({1} MB)" -f $zipPath, $zipMb) -ForegroundColor Green

Write-Host ''
Write-Host '完了しました。' -ForegroundColor Cyan
Get-ChildItem $buildDir | Format-Table Name, LastWriteTime -AutoSize
Write-Host 'インストーラーを作る場合は、Inno Setup 6 をインストールしたうえで次を実行してください:'
Write-Host '  & "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\FavoriteShortcut.iss' -ForegroundColor DarkGray
