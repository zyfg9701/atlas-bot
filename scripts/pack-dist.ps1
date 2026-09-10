# Pack release Hub+gateway[+cli] and PC shell into dist\ (P1 skeleton).
# Only collects --release / tauri build outputs (never debug).
# Usage: .\scripts\pack-dist.ps1 [-SkipPc] [-SkipCli]

param(
  [switch]$SkipPc,
  [switch]$SkipCli
)

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path
Set-Location -LiteralPath $Root

$Dist = Join-Path $Root 'dist'
$Template = Join-Path $Root 'packaging\templates'

Write-Host "==> pack-dist (Windows) → $Dist"

foreach ($sub in @('bin', 'pc', 'scripts')) {
  $p = Join-Path $Dist $sub
  if (Test-Path -LiteralPath $p) {
    Remove-Item -LiteralPath $p -Recurse -Force
  }
  New-Item -ItemType Directory -Force -Path $p | Out-Null
}

$packages = @('atlas-bot-hub', 'atlas-bot-gateway')
if (-not $SkipCli) { $packages += 'atlas-bot-cli' }

$cargoArgs = @('build', '--release')
foreach ($pkg in $packages) { $cargoArgs += @('-p', $pkg) }
Write-Host "==> cargo $($cargoArgs -join ' ')"
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

foreach ($name in $packages) {
  $src = Join-Path $Root "target\release\$name.exe"
  if (-not (Test-Path -LiteralPath $src)) {
    Write-Host "error: missing release binary: $src" -ForegroundColor Red
    exit 1
  }
  Copy-Item -LiteralPath $src -Destination (Join-Path $Dist 'bin') -Force
  Write-Host "    collected bin\$name.exe"
}

Copy-Item -LiteralPath (Join-Path $Template 'README-INSTALL.md') -Destination (Join-Path $Dist 'README-INSTALL.md') -Force
Copy-Item -LiteralPath (Join-Path $Template 'scripts\start-cli-stack.sh') -Destination (Join-Path $Dist 'scripts') -Force
Copy-Item -LiteralPath (Join-Path $Template 'scripts\start-cli-stack.ps1') -Destination (Join-Path $Dist 'scripts') -Force

if (-not $SkipPc) {
  Write-Host '==> PC: tauri build --no-bundle (unsigned; NOT store / NOT MSI/NSIS)'
  $pcDir = Join-Path $Root 'clients\pc'
  if (-not (Test-Path -LiteralPath (Join-Path $pcDir 'node_modules'))) {
    Push-Location -LiteralPath $pcDir
    try { npm ci } finally { Pop-Location }
  }
  Push-Location -LiteralPath $pcDir
  try {
    npm run tauri -- build --no-bundle --ci
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  } finally { Pop-Location }

  $pcBin = Join-Path $Root 'clients\pc\src-tauri\target\release\atlas-bot-pc.exe'
  if (-not (Test-Path -LiteralPath $pcBin)) {
    Write-Host "error: PC release binary not found: $pcBin" -ForegroundColor Red
    exit 1
  }
  Copy-Item -LiteralPath $pcBin -Destination (Join-Path $Dist 'pc') -Force
  Write-Host '    collected pc\atlas-bot-pc.exe'
} else {
  Write-Host '==> skip PC (-SkipPc)'
  Set-Content -LiteralPath (Join-Path $Dist 'pc\PLACEHOLDER.txt') -Encoding utf8 -Value @"
PC binary not packed on this run (-SkipPc or missing deps).
On a full pack host: npm run tauri -- build --no-bundle  → dist\pc\atlas-bot-pc.exe
"@
}

Write-Host ''
Write-Host 'Pack complete. Tree:'
Get-ChildItem -LiteralPath $Dist -Recurse -File | ForEach-Object { $_.FullName.Substring($Root.Length + 1) }
Write-Host ''
Write-Host 'Next: .\scripts\install-local.ps1   OR   .\dist\scripts\start-cli-stack.ps1'
