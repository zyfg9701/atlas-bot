# Archive an already-packed dist\ tree into a portable zip (T1).
# Does not rebuild binaries — run pack-dist first.
# Usage:
#   .\scripts\archive-dist.ps1 [-DistPath <path>] [-OutDir <path>] [-Version <ver>] [-Platform <plat>]

param(
  [string]$DistPath = '',
  [string]$OutDir = '',
  [string]$Version = '',
  [string]$Platform = 'windows-x64'
)

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path

if ([string]::IsNullOrWhiteSpace($DistPath)) {
  $DistPath = Join-Path $Root 'dist'
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
  $OutDir = Join-Path $Root 'artifacts'
}

if (-not (Test-Path -LiteralPath $DistPath -PathType Container)) {
  Write-Host "error: dist not found: $DistPath" -ForegroundColor Red
  Write-Host 'Run .\scripts\pack-dist.ps1 first.' -ForegroundColor Red
  exit 1
}
$hasReadme = Test-Path -LiteralPath (Join-Path $DistPath 'README-INSTALL.md')
$hasBin = Test-Path -LiteralPath (Join-Path $DistPath 'bin')
if (-not $hasReadme -and -not $hasBin) {
  Write-Host "error: $DistPath does not look like an atlas-bot dist tree" -ForegroundColor Red
  exit 1
}

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

$Ver = Resolve-AtlasVersion -Override $Version
if ([string]::IsNullOrWhiteSpace($Platform)) { $Platform = 'windows-x64' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$StageRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("atlas-bot-archive-" + [guid]::NewGuid().ToString('N'))
$Stage = Join-Path $StageRoot 'atlas-bot'
try {
  New-Item -ItemType Directory -Force -Path $Stage | Out-Null
  $xd = @('.cli-stack-pids', '.cli-stack-logs')
  $rcArgs = @($DistPath, $Stage, '/E', '/NFL', '/NDL', '/NJH', '/NJS', '/nc', '/ns', '/np', '/XD') + $xd
  & robocopy.exe @rcArgs | Out-Null
  if ($LASTEXITCODE -ge 8) {
    Write-Host "error: robocopy failed with code $LASTEXITCODE" -ForegroundColor Red
    exit 1
  }
  # Drop placeholder / pdbs from archive
  $ph = Join-Path $Stage 'pc\PLACEHOLDER.txt'
  if (Test-Path -LiteralPath $ph) { Remove-Item -LiteralPath $ph -Force }
  Get-ChildItem -LiteralPath $Stage -Recurse -Filter '*.pdb' -ErrorAction SilentlyContinue |
    ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }

  foreach ($helper in @('install-shortcuts.ps1', 'install-desktop-entry.sh')) {
    $destHelper = Join-Path $Stage "scripts\$helper"
    $srcHelper = Join-Path $Root "scripts\$helper"
    if (-not (Test-Path -LiteralPath $destHelper) -and (Test-Path -LiteralPath $srcHelper)) {
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destHelper) | Out-Null
      Copy-Item -LiteralPath $srcHelper -Destination $destHelper -Force
    }
  }

  $Name = "atlas-bot-$Ver-$Platform.zip"
  $OutPath = Join-Path $OutDir $Name
  if (Test-Path -LiteralPath $OutPath) { Remove-Item -LiteralPath $OutPath -Force }

  # Compress-Archive paths with spaces are OK when using -LiteralPath / array of items
  Compress-Archive -Path (Join-Path $StageRoot 'atlas-bot') -DestinationPath $OutPath -CompressionLevel Optimal

  Write-Host '==> archive-dist'
  Write-Host "    version:  $Ver  (override: -Version / ATLAS_BOT_VERSION; else git describe; else Cargo hub)"
  Write-Host "    platform: $Platform"
  Write-Host "    source:   $DistPath"
  Write-Host "    output:   $OutPath"
  Get-Item -LiteralPath $OutPath | Format-List FullName, Length
  Write-Host ''
  Write-Host 'Unpack then write Start Menu entries:'
  Write-Host "  Expand-Archive $Name; .\atlas-bot\scripts\install-shortcuts.ps1 -InstallRoot (Resolve-Path .\atlas-bot)"
} finally {
  if (Test-Path -LiteralPath $StageRoot) {
    Remove-Item -LiteralPath $StageRoot -Recurse -Force -ErrorAction SilentlyContinue
  }
}
