# Install a packed dist\ tree into the user profile (no admin / no Program Files).
# T1: also writes Start Menu shortcuts (default); pass -Desktop for Desktop .lnk too.
# Usage:
#   .\scripts\install-local.ps1 [[-]DistPath <path>] [-Desktop] [-SkipShortcuts]
#   $env:ATLAS_BOT_HOME='D:\atlas-bot'; .\scripts\install-local.ps1

param(
  [Parameter(Position = 0)]
  [string]$DistPath = '',
  [switch]$Desktop,
  [switch]$SkipShortcuts
)

$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path

if ([string]::IsNullOrWhiteSpace($DistPath)) {
  $DistPath = Join-Path $Root 'dist'
}
if (-not (Test-Path -LiteralPath $DistPath -PathType Container)) {
  Write-Host "error: dist source not found: $DistPath" -ForegroundColor Red
  Write-Host 'Run .\scripts\pack-dist.ps1 first, or pass -DistPath.' -ForegroundColor Red
  exit 1
}

$hasReadme = Test-Path -LiteralPath (Join-Path $DistPath 'README-INSTALL.md')
$hasBin = Test-Path -LiteralPath (Join-Path $DistPath 'bin')
if (-not $hasReadme -and -not $hasBin) {
  Write-Host "error: $DistPath does not look like an atlas-bot dist tree" -ForegroundColor Red
  exit 1
}

$Dest = [Environment]::GetEnvironmentVariable('ATLAS_BOT_HOME')
if ([string]::IsNullOrWhiteSpace($Dest)) {
  $Dest = Join-Path $env:LOCALAPPDATA 'atlas-bot'
}

Write-Host "==> install-local → $Dest"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

# Robocopy is quote/space-safe; exclude runtime state dirs
$xd = @('.cli-stack-pids', '.cli-stack-logs')
$rcArgs = @($DistPath, $Dest, '/E', '/NFL', '/NDL', '/NJH', '/NJS', '/nc', '/ns', '/np', '/XD') + $xd
& robocopy.exe @rcArgs | Out-Null
if ($LASTEXITCODE -ge 8) {
  Write-Host "error: robocopy failed with code $LASTEXITCODE" -ForegroundColor Red
  exit 1
}

# Ensure T1 shortcut helper is present in the install tree
foreach ($helper in @('install-shortcuts.ps1', 'install-desktop-entry.sh')) {
  $destHelper = Join-Path $Dest "scripts\$helper"
  $srcHelper = Join-Path $Root "scripts\$helper"
  if (-not (Test-Path -LiteralPath $destHelper) -and (Test-Path -LiteralPath $srcHelper)) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destHelper) | Out-Null
    Copy-Item -LiteralPath $srcHelper -Destination $destHelper -Force
  }
}

# Optional: append Dest\bin to user PATH (best-effort)
$binDir = Join-Path $Dest 'bin'
try {
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if ($null -eq $userPath) { $userPath = '' }
  $parts = @($userPath -split ';' | Where-Object { $_ -and $_.Trim() })
  $already = $false
  foreach ($p in $parts) {
    if ($p.TrimEnd('\').ToLowerInvariant() -eq $binDir.TrimEnd('\').ToLowerInvariant()) { $already = $true; break }
  }
  if (-not $already) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $binDir } else { "$userPath;$binDir" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "    appended user PATH: $binDir"
  } else {
    Write-Host '    user PATH already contains bin\'
  }
} catch {
  Write-Host "    (skip PATH append: $($_.Exception.Message))" -ForegroundColor Yellow
}

# T1: Start Menu shortcuts (default); -Desktop optional
if (-not $SkipShortcuts) {
  $sc = Join-Path $Dest 'scripts\install-shortcuts.ps1'
  if (-not (Test-Path -LiteralPath $sc)) {
    $sc = Join-Path $Root 'scripts\install-shortcuts.ps1'
  }
  if (Test-Path -LiteralPath $sc) {
    Write-Host '==> writing Start Menu shortcuts'
    $scArgs = @{ InstallRoot = $Dest }
    if ($Desktop) { $scArgs['Desktop'] = $true }
    & $sc @scArgs
  } else {
    Write-Host '    (install-shortcuts.ps1 missing — skipped)' -ForegroundColor Yellow
  }
} else {
  Write-Host '==> skip shortcuts (-SkipShortcuts)'
}

Write-Host ''
Write-Host "Installed to: $Dest"
Write-Host 'Next steps:'
$startPs1 = Join-Path $Dest 'scripts\start-cli-stack.ps1'
$pcExe = Join-Path $Dest 'pc\atlas-bot-pc.exe'
Write-Host "  1. Start stack:  Start Menu → atlas-bot → atlas-bot Start CLI Stack"
Write-Host "     or: & '$startPs1'"
Write-Host '     (requires agent on PATH, or $env:ATLAS_AGENT_CLI=... ; probe fail = non-zero)'
Write-Host "     Optional streaming: & '$startPs1' -Stream"
Write-Host "  2. Open PC:      Start Menu → atlas-bot → atlas-bot PC"
Write-Host "     or: & '$pcExe'"
Write-Host '  3. Connect → ws://127.0.0.1:7700/ws → select agent → Send'
Write-Host ''
Write-Host 'Unsigned / SmartScreen yellow is OK (More info → Run anyway). Not a store package.'
Write-Host 'T2 MSI: scripts/build-msi.ps1 (WiX v4). Desktop shortcuts: re-run install-shortcuts.ps1 -Desktop'
Write-Host "See: $(Join-Path $Dest 'README-INSTALL.md')"
