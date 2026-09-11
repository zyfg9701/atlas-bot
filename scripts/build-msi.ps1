# T2·W / MSI1: build a per-user Windows MSI from an existing dist\ tree (WiX v4).
# Does not rebuild binaries — run pack-dist.ps1 first.
# Does NOT run on Linux/macOS (explicit failure).
#
# Usage:
#   .\scripts\pack-dist.ps1
#   .\scripts\build-msi.ps1 [-DistPath <path>] [-OutDir <path>] [-Version <ver>] [-ProductVersion <x.y.z>]
#
# Output:
#   artifacts\atlas-bot-<ver>-windows-x64.msi
#
# WiX: v4 only (`wix build`). UpgradeCode fixed in packaging\wix\Product.wxs.
# Shortcuts: WiX Shortcut elements (not install-shortcuts.ps1).
# See: packaging\wix\README.md · docs\packaging-skeleton.md § T2 MSI

param(
  [string]$DistPath = '',
  [string]$OutDir = '',
  [string]$Version = '',
  [string]$ProductVersion = ''
)

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path

# --- Platform gate (hard fail on non-Windows) ---------------------------------
$isWindows = $false
try {
  if ($PSVersionTable.PSEdition -eq 'Desktop') { $isWindows = $true }
  elseif (Get-Variable -Name IsWindows -ErrorAction SilentlyContinue) { $isWindows = [bool]$IsWindows }
  elseif ($env:OS -like '*Windows*') { $isWindows = $true }
} catch { }

if (-not $isWindows) {
  Write-Host 'error: build-msi.ps1 only runs on Windows with WiX Toolset v4.' -ForegroundColor Red
  Write-Host '  This host is not Windows — MSI is not produced here (Linux CI does not build MSI).' -ForegroundColor Red
  Write-Host '  See packaging/wix/README.md and docs/packaging-skeleton.md § T2 MSI.' -ForegroundColor Yellow
  Write-Host '  On a Windows machine: install WiX v4 (`dotnet tool install --global wix`),' -ForegroundColor Yellow
  Write-Host '  run .\scripts\pack-dist.ps1 then .\scripts\build-msi.ps1.' -ForegroundColor Yellow
  exit 1
}

if ([string]::IsNullOrWhiteSpace($DistPath)) {
  $DistPath = Join-Path $Root 'dist'
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
  $OutDir = Join-Path $Root 'artifacts'
}

$WixDir = Join-Path $Root 'packaging\wix'
foreach ($req in @('Product.wxs', 'Components.wxs', 'Shortcuts.wxs')) {
  $p = Join-Path $WixDir $req
  if (-not (Test-Path -LiteralPath $p)) {
    Write-Host "error: missing WiX source: $p" -ForegroundColor Red
    exit 1
  }
}

# --- dist prerequisites -------------------------------------------------------
if (-not (Test-Path -LiteralPath $DistPath -PathType Container)) {
  Write-Host "error: dist not found: $DistPath" -ForegroundColor Red
  Write-Host 'Run .\scripts\pack-dist.ps1 first (MSI consumes existing dist\; does not cargo/tauri build).' -ForegroundColor Red
  exit 1
}

$required = @(
  @{ Rel = 'bin\atlas-bot-hub.exe';      Label = 'Hub' },
  @{ Rel = 'bin\atlas-bot-gateway.exe';  Label = 'gateway' },
  @{ Rel = 'bin\atlas-bot-cli.exe';      Label = 'cli (default pack includes cli)' },
  @{ Rel = 'pc\atlas-bot-pc.exe';        Label = 'PC shell (no PC-only MSI)' },
  @{ Rel = 'scripts\start-cli-stack.ps1'; Label = 'start-cli-stack.ps1' },
  @{ Rel = 'README-INSTALL.md';          Label = 'README-INSTALL.md' }
)
$missing = @()
foreach ($r in $required) {
  $full = Join-Path $DistPath $r.Rel
  if (-not (Test-Path -LiteralPath $full)) {
    $missing += "  missing $($r.Rel) ($($r.Label))"
  }
}
if ($missing.Count -gt 0) {
  Write-Host 'error: dist\ is incomplete for MSI (need Hub+gateway+cli+PC+scripts+README):' -ForegroundColor Red
  $missing | ForEach-Object { Write-Host $_ -ForegroundColor Red }
  Write-Host 'Re-run .\scripts\pack-dist.ps1 without -SkipPc / -SkipCli.' -ForegroundColor Yellow
  exit 1
}

# --- WiX v4 toolchain ---------------------------------------------------------
$wixCmd = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wixCmd) {
  Write-Host 'error: WiX v4 CLI not found on PATH (`wix`).' -ForegroundColor Red
  Write-Host '  Install: dotnet tool install --global wix' -ForegroundColor Yellow
  Write-Host '  Then verify: wix --version  (expect 4.x)' -ForegroundColor Yellow
  Write-Host '  Docs: packaging/wix/README.md' -ForegroundColor Yellow
  exit 1
}
$wixVerText = & wix --version 2>&1 | Out-String
$wixVerText = $wixVerText.Trim()
if ($wixVerText -notmatch '(^|\s)4\.') {
  Write-Host ("error: WiX v4 required (got: $wixVerText). This slice pins v4 wix build only.") -ForegroundColor Red
  Write-Host '  Do not use v3 candle/light. See packaging/wix/README.md' -ForegroundColor Yellow
  exit 1
}

# --- Version (filename) + ProductVersion (MSI numeric) ------------------------
function Resolve-AtlasVersion {
  param([string]$Override)
  if (-not [string]::IsNullOrWhiteSpace($Override)) { return $Override }
  $envVer = [Environment]::GetEnvironmentVariable('ATLAS_BOT_VERSION')
  if (-not [string]::IsNullOrWhiteSpace($envVer)) { return $envVer }
  try {
    Push-Location -LiteralPath $Root
    $desc = & git describe --tags --always --dirty 2>$null
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($desc)) {
      return (($desc -replace '[/:]', '-').Trim())
    }
  } catch {
  } finally {
    Pop-Location
  }
  $cargoToml = Join-Path $Root 'crates\atlas-bot-hub\Cargo.toml'
  if (Test-Path -LiteralPath $cargoToml) {
    $m = Select-String -LiteralPath $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($m) { return $m.Matches[0].Groups[1].Value }
  }
  return '0.0.0'
}

function Resolve-MsiProductVersion {
  param([string]$Override, [string]$FileVer)
  if (-not [string]::IsNullOrWhiteSpace($Override)) {
    if ($Override -notmatch '^\d+\.\d+\.\d+(\.\d+)?$') {
      throw "ProductVersion must be numeric X.Y.Z[.W], got: $Override"
    }
    return $Override
  }
  if ($FileVer -match '^\d+\.\d+\.\d+(\.\d+)?$') { return $FileVer }
  $envVer = [Environment]::GetEnvironmentVariable('ATLAS_BOT_VERSION')
  if (-not [string]::IsNullOrWhiteSpace($envVer) -and $envVer -match '^\d+\.\d+\.\d+(\.\d+)?$') {
    return $envVer
  }
  $cargoToml = Join-Path $Root 'crates\atlas-bot-hub\Cargo.toml'
  if (Test-Path -LiteralPath $cargoToml) {
    $m = Select-String -LiteralPath $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($m -and $m.Matches[0].Groups[1].Value -match '^\d+\.\d+\.\d+') {
      return $m.Matches[0].Groups[1].Value
    }
  }
  return '0.1.0'
}

$Ver = Resolve-AtlasVersion -Override $Version
try {
  $ProdVer = Resolve-MsiProductVersion -Override $ProductVersion -FileVer $Ver
} catch {
  Write-Host "error: $($_.Exception.Message)" -ForegroundColor Red
  exit 1
}

# MSI ProductVersion: max 255.255.65535.65535 — clamp naive if needed
$parts = $ProdVer.Split('.')
if ([int]$parts[0] -gt 255) {
  Write-Host "error: ProductVersion major > 255: $ProdVer" -ForegroundColor Red
  exit 1
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$MsiName = "atlas-bot-$Ver-windows-x64.msi"
$OutPath = Join-Path $OutDir $MsiName
if (Test-Path -LiteralPath $OutPath) { Remove-Item -LiteralPath $OutPath -Force }

$DistBin = Join-Path $DistPath 'bin'
$DistPc = Join-Path $DistPath 'pc'
$DistScripts = Join-Path $DistPath 'scripts'
$PcExe = Join-Path $DistPc 'atlas-bot-pc.exe'

$wixArgs = @(
  'build',
  (Join-Path $WixDir 'Product.wxs'),
  (Join-Path $WixDir 'Components.wxs'),
  (Join-Path $WixDir 'Shortcuts.wxs'),
  '-d', "ProductVersion=$ProdVer",
  '-d', "PcExePath=$PcExe",
  '-b', "DistBin=$DistBin",
  '-b', "DistPc=$DistPc",
  '-b', "DistScripts=$DistScripts",
  '-b', "DistRoot=$DistPath",
  '-o', $OutPath,
  '-arch', 'x64'
)

Write-Host '==> build-msi (WiX v4)'
Write-Host "    file version:    $Ver  (-Version / ATLAS_BOT_VERSION / git describe / Cargo)"
Write-Host "    ProductVersion:  $ProdVer  (MSI numeric; -ProductVersion override)"
Write-Host "    UpgradeCode:     DAB90D5E-C3DB-405C-9024-769247FFC85F (fixed)"
Write-Host "    scope:           perUser → %LOCALAPPDATA%\atlas-bot"
Write-Host "    shortcuts:       WiX Shortcut (Start Menu on; Desktop Feature default off)"
Write-Host "    dist:            $DistPath"
Write-Host "    wix:             $wixVerText"
Write-Host "    output:          $OutPath"
Write-Host ''
Write-Host ("    wix " + ($wixArgs -join ' '))

& wix @wixArgs
if ($LASTEXITCODE -ne 0) {
  Write-Host "error: wix build failed with exit $LASTEXITCODE" -ForegroundColor Red
  exit $LASTEXITCODE
}

if (-not (Test-Path -LiteralPath $OutPath)) {
  Write-Host "error: MSI not produced at $OutPath" -ForegroundColor Red
  exit 1
}

Write-Host ''
Write-Host '==> MSI ready'
Get-Item -LiteralPath $OutPath | Format-List FullName, Length, LastWriteTime
Write-Host ("Install (no admin):  msiexec /i `"$OutPath`"")
Write-Host ("Silent:              msiexec /i `"$OutPath`" /qn")
Write-Host 'Desktop shortcuts:   msiexec /i ... ADDLOCAL=ProductFeature,DesktopShortcuts'
Write-Host 'Uninstall:           Apps & Features -> atlas-bot  (or msiexec /x ...)'
Write-Host 'Unsigned / SmartScreen yellow is OK. Not store. Zip path still available via archive-dist.'
