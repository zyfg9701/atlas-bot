# S1 one-click stack recipe: probe agent → call existing start-cli-stack.ps1 →
# healthz (backend=cli) → optional PC → print Connect next steps.
# Does NOT rewrite start-cli-stack; does NOT cargo run; not atlas-desktop-stack.
#
# Usage:
#   .\scripts\start-atlas.ps1
#   .\scripts\start-atlas.ps1 -SkipPc
#   $env:ATLAS_SKIP_PC='1'; .\scripts\start-atlas.ps1
#   $env:ATLAS_AGENT_CLI='C:\path\to\agent.exe'; .\scripts\start-atlas.ps1
# Double-click: start-atlas.cmd (ExecutionPolicy Bypass)
#
# See docs/packaging-skeleton.md §S1 and dist\README-INSTALL.md

param(
  [switch]$SkipPc,
  [switch]$Help
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path
$BinDir = if (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('ATLAS_BOT_BIN_DIR'))) {
  $env:ATLAS_BOT_BIN_DIR
} else {
  Join-Path $Root 'bin'
}

function Get-EnvOrDefault([string]$Name, [string]$Default) {
  $v = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($v)) { return $Default }
  return $v
}

function Test-EnvTruthy([string]$Name) {
  $v = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($v)) { return $false }
  $t = $v.Trim().ToLowerInvariant()
  return ($t -eq '1' -or $t -eq 'true' -or $t -eq 'yes')
}

if ($Help) {
  Write-Host @'
Usage: start-atlas.ps1 [-SkipPc] [-Help]

S1 one-click recipe (install/unpack root):
  1. Probe ATLAS_AGENT_CLI or PATH "agent"
  2. Call existing scripts\start-cli-stack.ps1 (binaries only — no cargo run)
  3. Wait for gateway + Hub healthz with backend=cli
  4. Launch pc\atlas-bot-pc.exe (unless -SkipPc / ATLAS_SKIP_PC)
  5. Print Connect next steps (ws://…/ws)

Not this knife: signing, updater, store, C1-real, GA2/YOLO,
  atlas-desktop-stack (VNC/D1). Still need a local agent; one-click ≠ bundling it.
'@
  exit 0
}

if (Test-EnvTruthy 'ATLAS_SKIP_PC') { $SkipPc = $true }

$StartPs1 = Join-Path $Root 'scripts\start-cli-stack.ps1'
$PcExe = Join-Path $Root 'pc\atlas-bot-pc.exe'
if (-not (Test-Path -LiteralPath $PcExe)) {
  $alt = Join-Path $Root 'pc\atlas-bot-pc'
  if (Test-Path -LiteralPath $alt) { $PcExe = $alt }
}

$CLI = Get-EnvOrDefault 'ATLAS_AGENT_CLI' 'agent'
$GW_BIND = Get-EnvOrDefault 'ATLAS_GATEWAY_HTTP_BIND' '127.0.0.1:8787'
$HUB_BIND = Get-EnvOrDefault 'ATLAS_HUB_BIND' '127.0.0.1:7700'
$BACKEND = Get-EnvOrDefault 'ATLAS_GATEWAY_BACKEND' 'cli'
$PID_DIR = Get-EnvOrDefault 'ATLAS_CLI_STACK_PID_DIR' (Join-Path $Root '.cli-stack-pids')
$LOG_DIR = Get-EnvOrDefault 'ATLAS_CLI_STACK_LOG_DIR' (Join-Path $Root '.cli-stack-logs')

function Resolve-AgentCli([string]$c) {
  $looksLikePath = ($c -match '[\\/]') -or ($c -match '^[A-Za-z]:')
  if ($looksLikePath) {
    $candidates = @($c)
    foreach ($ext in @('.cmd', '.exe', '.bat')) {
      if (-not ($c.ToLowerInvariant().EndsWith($ext))) {
        $candidates += ($c + $ext)
      }
    }
    foreach ($cand in $candidates) {
      try {
        $resolved = Resolve-Path -LiteralPath $cand -ErrorAction Stop
        if (Test-Path -LiteralPath $resolved.Path -PathType Leaf) {
          return $resolved.Path
        }
      } catch { }
    }
    return $null
  }
  $cmd = Get-Command -Name $c -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandType -ne 'Alias' } |
    Select-Object -First 1
  if ($null -ne $cmd) {
    if ($cmd.Source) { return $cmd.Source }
    if ($cmd.Path) { return $cmd.Path }
  }
  foreach ($ext in @('.exe', '.cmd', '.bat')) {
    $cmd = Get-Command -Name ($c + $ext) -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($null -ne $cmd) {
      if ($cmd.Source) { return $cmd.Source }
      if ($cmd.Path) { return $cmd.Path }
    }
  }
  return $null
}

function Test-HttpOk([string]$Url) {
  try {
    $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
    return ($r.StatusCode -ge 200 -and $r.StatusCode -lt 300)
  } catch {
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
      & curl.exe -sf $Url 1>$null 2>$null
      return ($LASTEXITCODE -eq 0)
    }
    return $false
  }
}

function Get-HealthzBody([string]$Bind) {
  $url = "http://${Bind}/healthz"
  try {
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 2
    return [string]$r.Content
  } catch {
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
      $out = & curl.exe -sf $url 2>$null
      if ($LASTEXITCODE -eq 0) { return [string]$out }
    }
    return ''
  }
}

function Test-GatewayBackendCli {
  $body = Get-HealthzBody $GW_BIND
  if ([string]::IsNullOrWhiteSpace($body)) { return $false }
  if ($body -match '"backend"') {
    return ($body -match '"backend"\s*:\s*"cli"')
  }
  return $true
}

Write-Host '==> atlas-bot S1 one-click (stack+PC recipe)'
Write-Host "    ROOT=$Root"
Write-Host '    (not atlas-desktop-stack / not signing / not updater / not store)'
Write-Host ''

if (-not (Test-Path -LiteralPath $StartPs1)) {
  Write-Host "error: missing start-cli-stack script: $StartPs1" -ForegroundColor Red
  Write-Host 'This recipe must call the existing start-cli-stack (not reinvent it).' -ForegroundColor Red
  Write-Host 'Unpack/install an atlas-bot dist tree first. See README-INSTALL.md' -ForegroundColor Red
  exit 1
}

$gwBin = Join-Path $BinDir 'atlas-bot-gateway.exe'
$hubBin = Join-Path $BinDir 'atlas-bot-hub.exe'
if (-not (Test-Path -LiteralPath $gwBin)) { $gwBin = Join-Path $BinDir 'atlas-bot-gateway' }
if (-not (Test-Path -LiteralPath $hubBin)) { $hubBin = Join-Path $BinDir 'atlas-bot-hub' }
if (-not (Test-Path -LiteralPath $gwBin)) {
  Write-Host "error: missing binary: $gwBin" -ForegroundColor Red
  Write-Host 'Run pack-dist / install-local, or unpack the archive so bin\ exists.' -ForegroundColor Red
  Write-Host 'See README-INSTALL.md' -ForegroundColor Red
  exit 1
}
if (-not (Test-Path -LiteralPath $hubBin)) {
  Write-Host "error: missing binary: $hubBin" -ForegroundColor Red
  exit 1
}

$cliResolved = Resolve-AgentCli $CLI
if ($null -eq $cliResolved) {
  Write-Host "error: ATLAS_AGENT_CLI not found: $CLI" -ForegroundColor Red
  Write-Host ''
  Write-Host 'Install and login the Cursor/Atlas agent CLI (default name: agent), then re-run.' -ForegroundColor Red
  Write-Host 'One-click does NOT bundle the agent runtime.' -ForegroundColor Red
  Write-Host ''
  Write-Host '  $env:ATLAS_AGENT_CLI = ''C:\path\to\agent.exe''' -ForegroundColor Yellow
  Write-Host '  .\scripts\start-atlas.ps1 -SkipPc' -ForegroundColor Yellow
  Write-Host ''
  Write-Host 'See docs\cli-primary-runbook.md and README-INSTALL.md' -ForegroundColor Red
  exit 1
}

$env:ATLAS_AGENT_CLI = $cliResolved
$env:ATLAS_GATEWAY_BACKEND = $BACKEND
$env:ATLAS_GATEWAY_HTTP_BIND = $GW_BIND
$env:ATLAS_HUB_BIND = $HUB_BIND
Write-Host "    ATLAS_AGENT_CLI=$cliResolved"
Write-Host "    SkipPc=$SkipPc"

$stackProc = $null
$recipeLog = Join-Path $LOG_DIR 'start-atlas-stack.log'

$alreadyUp = (Test-HttpOk "http://${GW_BIND}/healthz") -and (Test-HttpOk "http://${HUB_BIND}/healthz") -and (Test-GatewayBackendCli)
if ($alreadyUp) {
  Write-Host '==> stack already healthy (gateway+Hub, backend=cli); reusing'
} else {
  Write-Host '==> starting stack via existing start-cli-stack.ps1 (separate process)…'
  if (-not (Test-Path -LiteralPath $LOG_DIR)) {
    New-Item -ItemType Directory -Force -Path $LOG_DIR | Out-Null
  }
  if (-not (Test-Path -LiteralPath $PID_DIR)) {
    New-Item -ItemType Directory -Force -Path $PID_DIR | Out-Null
  }

  $psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  if (-not (Test-Path -LiteralPath $psExe)) { $psExe = 'powershell.exe' }
  $args = "-NoProfile -ExecutionPolicy Bypass -File `"$StartPs1`""
  $stackProc = Start-Process -FilePath $psExe -ArgumentList $args -WorkingDirectory $Root -PassThru -WindowStyle Minimized
  Write-Host "    start-cli-stack pid=$($stackProc.Id)"

  Write-Host -NoNewline '    waiting for gateway /healthz (backend=cli)'
  $ok = $false
  for ($i = 1; $i -le 90; $i++) {
    if ($stackProc.HasExited) {
      Write-Host ''
      Write-Host 'error: start-cli-stack exited early.' -ForegroundColor Red
      Write-Host 'Hint: missing agent/bin usually prints in that window. See README-INSTALL.md' -ForegroundColor Red
      exit 1
    }
    if (Test-HttpOk "http://${GW_BIND}/healthz") {
      Write-Host ' ok'
      $ok = $true
      break
    }
    Write-Host -NoNewline '.'
    Start-Sleep -Milliseconds 500
  }
  if (-not $ok) {
    Write-Host ''
    Write-Host "error: gateway healthz timeout; see $LOG_DIR\gateway.log" -ForegroundColor Red
    exit 1
  }
  if (-not (Test-GatewayBackendCli)) {
    $body = Get-HealthzBody $GW_BIND
    Write-Host "error: gateway healthz backend is not cli: $body" -ForegroundColor Red
    Write-Host 'Expected ATLAS_GATEWAY_BACKEND=cli. Refusing to pretend the stack is ready.' -ForegroundColor Red
    exit 1
  }
  Write-Host '    gateway healthz backend=cli'

  Write-Host -NoNewline '    waiting for Hub /healthz'
  $ok = $false
  for ($i = 1; $i -le 90; $i++) {
    if ($stackProc.HasExited) {
      Write-Host ''
      Write-Host 'error: start-cli-stack exited early while waiting for Hub.' -ForegroundColor Red
      exit 1
    }
    if (Test-HttpOk "http://${HUB_BIND}/healthz") {
      Write-Host ' ok'
      $ok = $true
      break
    }
    Write-Host -NoNewline '.'
    Start-Sleep -Milliseconds 500
  }
  if (-not $ok) {
    Write-Host ''
    Write-Host "error: Hub healthz timeout; see $LOG_DIR\hub.log" -ForegroundColor Red
    exit 1
  }
}

if ($SkipPc) {
  Write-Host '==> SkipPc: not launching PC'
} else {
  if (-not (Test-Path -LiteralPath $PcExe)) {
    Write-Host "warning: PC binary missing: $PcExe" -ForegroundColor Yellow
    Write-Host '         Stack is up; open PC when available, or re-pack without -SkipPc.' -ForegroundColor Yellow
    Write-Host '         Tip: .\scripts\start-atlas.ps1 -SkipPc to suppress in CI.' -ForegroundColor Yellow
  } else {
    Write-Host "==> launching PC: $PcExe"
    try {
      $pcProc = Start-Process -FilePath $PcExe -WorkingDirectory $Root -PassThru
      Write-Host "    PC launch requested (pid=$($pcProc.Id))"
    } catch {
      Write-Host "error: failed to launch PC: $PcExe" -ForegroundColor Red
      Write-Host "  $($_.Exception.Message)" -ForegroundColor Red
      Write-Host 'Stack is still running. Diagnose display, or use -SkipPc.' -ForegroundColor Red
      exit 1
    }
  }
}

Write-Host ''
Write-Host 'Ready (S1 one-click).'
Write-Host "  Connect:  ws://${HUB_BIND}/ws"
Write-Host "  Gateway:  http://${GW_BIND}/healthz   (expect backend=cli)"
Write-Host "  Hub:      http://${HUB_BIND}/healthz"
Write-Host '  Next:     PC → Connect → select agent → Send'
Write-Host "  Logs:     $LOG_DIR\gateway.log  $LOG_DIR\hub.log"
if ($null -ne $stackProc -and -not $stackProc.HasExited) {
  Write-Host "  Stack pid: $($stackProc.Id) (start-cli-stack window; close it or kill PIDs under $PID_DIR)"
}
$gwPidFile = Join-Path $PID_DIR 'gateway.pid'
$hubPidFile = Join-Path $PID_DIR 'hub.pid'
Write-Host "  Stop:     Stop-Process -Id (Get-Content '$gwPidFile'), (Get-Content '$hubPidFile') -ErrorAction SilentlyContinue"
Write-Host ''
Write-Host 'Still need local agent. Unsigned / SmartScreen yellow OK. Not store / not updater / not atlas-desktop-stack.'
exit 0
