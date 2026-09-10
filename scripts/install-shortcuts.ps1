# Write Start Menu (default) and optional Desktop shortcuts for an atlas-bot install root (T1).
# No admin. WorkingDirectory = install root (spaces-safe).
# Usage:
#   .\scripts\install-shortcuts.ps1 [-InstallRoot <path>] [-Desktop]
#   # From an unpacked archive:
#   .\atlas-bot\scripts\install-shortcuts.ps1 -InstallRoot (Resolve-Path .\atlas-bot)
#
# Defaults: Start Menu only (%APPDATA%\Microsoft\Windows\Start Menu\Programs\atlas-bot\).
# Pass -Desktop to also write Desktop .lnk files.

param(
  [string]$InstallRoot = '',
  [switch]$Desktop
)

$ErrorActionPreference = 'Stop'

function Resolve-InstallRoot {
  param([string]$Hint)
  if (-not [string]::IsNullOrWhiteSpace($Hint)) {
    return (Resolve-Path -LiteralPath $Hint).Path
  }
  $envHome = [Environment]::GetEnvironmentVariable('ATLAS_BOT_HOME')
  if (-not [string]::IsNullOrWhiteSpace($envHome) -and (Test-Path -LiteralPath $envHome)) {
    return (Resolve-Path -LiteralPath $envHome).Path
  }
  # If this script lives under <root>\scripts\, use parent
  $here = Split-Path -Parent $MyInvocation.MyCommand.Path
  $parent = Split-Path -Parent $here
  $pcCand = Join-Path $parent 'pc\atlas-bot-pc.exe'
  $binCand = Join-Path $parent 'bin'
  if ((Test-Path -LiteralPath $pcCand) -or (Test-Path -LiteralPath $binCand)) {
    return (Resolve-Path -LiteralPath $parent).Path
  }
  $fallback = Join-Path $env:LOCALAPPDATA 'atlas-bot'
  if (Test-Path -LiteralPath $fallback) {
    return (Resolve-Path -LiteralPath $fallback).Path
  }
  throw "Cannot resolve install root. Pass -InstallRoot or set ATLAS_BOT_HOME."
}

# Re-bind MyInvocation for nested function — compute script dir at top level
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
  $envHome = [Environment]::GetEnvironmentVariable('ATLAS_BOT_HOME')
  if (-not [string]::IsNullOrWhiteSpace($envHome) -and (Test-Path -LiteralPath $envHome)) {
    $InstallRoot = (Resolve-Path -LiteralPath $envHome).Path
  } else {
    $parent = Split-Path -Parent $ScriptDir
    $pcCand = Join-Path $parent 'pc\atlas-bot-pc.exe'
    $binCand = Join-Path $parent 'bin'
    if ((Test-Path -LiteralPath $pcCand) -or (Test-Path -LiteralPath $binCand)) {
      $InstallRoot = (Resolve-Path -LiteralPath $parent).Path
    } else {
      $fallback = Join-Path $env:LOCALAPPDATA 'atlas-bot'
      if (Test-Path -LiteralPath $fallback) {
        $InstallRoot = (Resolve-Path -LiteralPath $fallback).Path
      } else {
        Write-Host 'error: Cannot resolve install root. Pass -InstallRoot or set ATLAS_BOT_HOME.' -ForegroundColor Red
        exit 1
      }
    }
  }
} else {
  $InstallRoot = (Resolve-Path -LiteralPath $InstallRoot).Path
}

$PcExe = Join-Path $InstallRoot 'pc\atlas-bot-pc.exe'
$StartPs1 = Join-Path $InstallRoot 'scripts\start-cli-stack.ps1'

if (-not (Test-Path -LiteralPath $PcExe)) {
  Write-Host "warning: PC executable missing: $PcExe (shortcut still written)" -ForegroundColor Yellow
}
if (-not (Test-Path -LiteralPath $StartPs1)) {
  Write-Host "warning: start script missing: $StartPs1 (shortcut still written)" -ForegroundColor Yellow
}

$Wsh = New-Object -ComObject WScript.Shell

function New-AtlasShortcut {
  param(
    [Parameter(Mandatory = $true)][string]$LnkPath,
    [Parameter(Mandatory = $true)][string]$TargetPath,
    [Parameter(Mandatory = $true)][string]$WorkingDirectory,
    [string]$Arguments = '',
    [string]$Description = '',
    [string]$IconLocation = ''
  )
  $dir = Split-Path -Parent $LnkPath
  if (-not (Test-Path -LiteralPath $dir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
  }
  $sc = $Wsh.CreateShortcut($LnkPath)
  $sc.TargetPath = $TargetPath
  if (-not [string]::IsNullOrWhiteSpace($Arguments)) {
    $sc.Arguments = $Arguments
  }
  $sc.WorkingDirectory = $WorkingDirectory
  if (-not [string]::IsNullOrWhiteSpace($Description)) {
    $sc.Description = $Description
  }
  if (-not [string]::IsNullOrWhiteSpace($IconLocation)) {
    $sc.IconLocation = $IconLocation
  }
  $sc.Save()
  Write-Host "    wrote $LnkPath"
}

$Programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\atlas-bot'
New-Item -ItemType Directory -Force -Path $Programs | Out-Null

# PC shortcut → exe directly; WorkingDirectory = install root
New-AtlasShortcut `
  -LnkPath (Join-Path $Programs 'atlas-bot PC.lnk') `
  -TargetPath $PcExe `
  -WorkingDirectory $InstallRoot `
  -Description 'atlas-bot PC shell (unsigned)' `
  -IconLocation $(if (Test-Path -LiteralPath $PcExe) { "$PcExe,0" } else { '' })

# Start CLI Stack → powershell -NoProfile -ExecutionPolicy Bypass -File "<path>"
# Quote File path for spaces
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $psExe)) {
  $psExe = 'powershell.exe'
}
# Arguments must quote the script path when it contains spaces
$startArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$StartPs1`""
New-AtlasShortcut `
  -LnkPath (Join-Path $Programs 'atlas-bot Start CLI Stack.lnk') `
  -TargetPath $psExe `
  -Arguments $startArgs `
  -WorkingDirectory $InstallRoot `
  -Description 'Start atlas-bot Hub+gateway (cli backend; binaries only — no cargo run)'

if ($Desktop) {
  $DesktopDir = [Environment]::GetFolderPath('Desktop')
  New-AtlasShortcut `
    -LnkPath (Join-Path $DesktopDir 'atlas-bot PC.lnk') `
    -TargetPath $PcExe `
    -WorkingDirectory $InstallRoot `
    -Description 'atlas-bot PC shell (unsigned)' `
    -IconLocation $(if (Test-Path -LiteralPath $PcExe) { "$PcExe,0" } else { '' })
  New-AtlasShortcut `
    -LnkPath (Join-Path $DesktopDir 'atlas-bot Start CLI Stack.lnk') `
    -TargetPath $psExe `
    -Arguments $startArgs `
    -WorkingDirectory $InstallRoot `
    -Description 'Start atlas-bot Hub+gateway (cli backend)'
  Write-Host '    Desktop shortcuts enabled (-Desktop)'
} else {
  Write-Host '    Desktop skipped (default Start Menu only; pass -Desktop to enable)'
}

Write-Host ''
Write-Host "Install root: $InstallRoot"
Write-Host "Start Menu:   $Programs"
Write-Host 'Next: click "atlas-bot Start CLI Stack", then "atlas-bot PC" → Connect → Send'
Write-Host 'Unsigned / SmartScreen yellow is OK. Not a store package. T2 MSI deferred.'
