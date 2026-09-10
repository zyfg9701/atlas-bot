# Start the recommended CLI primary stack: gateway(backend=cli) + Hub(GATEWAY_URL).
# Windows PowerShell 5.1+ / PowerShell 7+. Semantic parity with scripts/dev-cli-stack.sh.
#
# Usage:
#   .\scripts\dev-cli-stack.ps1
#   powershell -ExecutionPolicy Bypass -File .\scripts\dev-cli-stack.ps1
#   $env:ATLAS_AGENT_CLI='.\tools\mock-cli\mock-atlas-agent-cli.cmd'; .\scripts\dev-cli-stack.ps1
#   # WS1·B streaming one-liner (multi-process B1 mid-turn):
#   $env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1 -Stream
#   # Or: $env:ATLAS_CLI_STACK_STREAM='1'; ...\dev-cli-stack.ps1
#
# Env overrides (same names as Unix):
#   ATLAS_AGENT_CLI          default: agent
#   ATLAS_GATEWAY_HTTP_BIND  default: 127.0.0.1:8787
#   ATLAS_HUB_BIND           default: 127.0.0.1:7700
#   ATLAS_GATEWAY_BACKEND    default: cli
#   ATLAS_AGENT_CLI_STREAM   CS1 opt-in; -Stream / ATLAS_CLI_STACK_STREAM=1 sets to 1 if unset
#   ATLAS_HUB_EVENT_URL      CS1b B1 POST URL; -Stream defaults http://127.0.0.1:7701/internal/runtime-hint if unset
#   ATLAS_HUB_EVENT_TOKEN    B1 shared token (explicit wins; do not dual with B2)
#   ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK  B1 loopback without token; -Stream sets 1 if token+flag both unset
#   ATLAS_HUB_EVENT_BIND     Hub ingest listen (Hub default 127.0.0.1:7701)
#   MOCK_CLI_STREAM          mock NDJSON deltas; -Stream + mock .cmd sets 1 if unset
#   MOCK_CLI_SLEEP_MS        mock sleep between delta and result (cmd timeout is second-granularity)
#   ATLAS_CLI_STACK_STREAM   1/true/yes — same as -Stream (explicit env alternative)
#
# Priority: explicit user env for STREAM / EVENT_URL / MOCK / TOKEN / INSECURE always wins
# (-Stream only fills unset defaults). Without -Stream / STACK_STREAM: text stack (W1 unchanged).
# Multi-process mid-turn = B1 only; do NOT also open B2 spawn_turn_bridge.

param(
  [switch]$Stream
)

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $ScriptDir '..')).Path
Set-Location -LiteralPath $Root

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

function Test-EnvSet([string]$Name) {
  $v = [Environment]::GetEnvironmentVariable($Name)
  return -not [string]::IsNullOrWhiteSpace($v)
}

$CLI = Get-EnvOrDefault 'ATLAS_AGENT_CLI' 'agent'
$GW_BIND = Get-EnvOrDefault 'ATLAS_GATEWAY_HTTP_BIND' '127.0.0.1:8787'
$HUB_BIND = Get-EnvOrDefault 'ATLAS_HUB_BIND' '127.0.0.1:7700'
$BACKEND = Get-EnvOrDefault 'ATLAS_GATEWAY_BACKEND' 'cli'
$PID_DIR = Get-EnvOrDefault 'ATLAS_CLI_STACK_PID_DIR' (Join-Path $Root '.cli-stack-pids')
$LOG_DIR = Get-EnvOrDefault 'ATLAS_CLI_STACK_LOG_DIR' (Join-Path $Root '.cli-stack-logs')

function Resolve-AgentCli([string]$c) {
  # Absolute/relative path (slash, backslash, .\ ..\ or drive letter) → existing file
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

  # Bare name → Get-Command (resolves agent.exe / agent.cmd / agent.bat on PATH)
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

$CLI_RESOLVED = Resolve-AgentCli $CLI
if (-not $CLI_RESOLVED) {
  $mockFull = Join-Path $Root 'tools\mock-cli\mock-atlas-agent-cli.cmd'
  Write-Host @"
error: ATLAS_AGENT_CLI not found: $CLI

Install and login the Cursor/Atlas agent CLI (default binary name: agent), then re-run.
This script will NOT silently fall back to the Hub InMemory stub.

For CI / local without a real agent, override with the mock CLI:
  `$env:ATLAS_AGENT_CLI='$mockFull'; .\scripts\dev-cli-stack.ps1

Streaming (WS1·B):
  `$env:ATLAS_AGENT_CLI='$mockFull'; .\scripts\dev-cli-stack.ps1 -Stream

See docs/cli-primary-runbook.md
"@ -ForegroundColor Red
  exit 1
}

$env:ATLAS_AGENT_CLI = $CLI_RESOLVED
$env:ATLAS_GATEWAY_BACKEND = $BACKEND
$env:ATLAS_GATEWAY_HTTP_BIND = $GW_BIND
$env:ATLAS_HUB_BIND = $HUB_BIND
$env:ATLAS_GATEWAY_URL = "http://${GW_BIND}"

# WS1·B: -Stream switch and/or ATLAS_CLI_STACK_STREAM=1
$wantStream = $Stream.IsPresent -or (Test-EnvTruthy 'ATLAS_CLI_STACK_STREAM')
if ($wantStream) {
  if (-not (Test-EnvSet 'ATLAS_AGENT_CLI_STREAM')) {
    $env:ATLAS_AGENT_CLI_STREAM = '1'
  }
  if (-not (Test-EnvSet 'ATLAS_HUB_EVENT_URL')) {
    $env:ATLAS_HUB_EVENT_URL = 'http://127.0.0.1:7701/internal/runtime-hint'
  }
  # Hub B1 auth: explicit token OR insecure loopback (explicit either wins; else default insecure)
  if (-not (Test-EnvSet 'ATLAS_HUB_EVENT_TOKEN') -and -not (Test-EnvSet 'ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK')) {
    $env:ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK = '1'
  }
  $cliLeaf = [System.IO.Path]::GetFileName($CLI_RESOLVED)
  if ($cliLeaf -ieq 'mock-atlas-agent-cli.cmd') {
    if (-not (Test-EnvSet 'MOCK_CLI_STREAM')) {
      $env:MOCK_CLI_STREAM = '1'
    }
  }
}

New-Item -ItemType Directory -Force -Path $PID_DIR | Out-Null
New-Item -ItemType Directory -Force -Path $LOG_DIR | Out-Null
$GW_PID_FILE = Join-Path $PID_DIR 'gateway.pid'
$HUB_PID_FILE = Join-Path $PID_DIR 'hub.pid'
$GW_LOG = Join-Path $LOG_DIR 'gateway.log'
$HUB_LOG = Join-Path $LOG_DIR 'hub.log'

$script:GwProcess = $null
$script:HubProcess = $null
$script:Cleaned = $false

function Stop-StackChildren {
  if ($script:Cleaned) { return }
  $script:Cleaned = $true
  foreach ($pair in @(
      @{ Proc = $script:GwProcess; File = $GW_PID_FILE },
      @{ Proc = $script:HubProcess; File = $HUB_PID_FILE }
    )) {
    $pidVal = $null
    if ($pair.Proc -and -not $pair.Proc.HasExited) {
      $pidVal = $pair.Proc.Id
    } elseif (Test-Path -LiteralPath $pair.File) {
      $raw = Get-Content -LiteralPath $pair.File -ErrorAction SilentlyContinue | Select-Object -First 1
      if ($raw) { $pidVal = $raw.Trim() }
    }
    if ($pidVal) {
      try { Stop-Process -Id ([int]$pidVal) -Force -ErrorAction SilentlyContinue } catch { }
    }
    if (Test-Path -LiteralPath $pair.File) {
      Remove-Item -LiteralPath $pair.File -Force -ErrorAction SilentlyContinue
    }
  }
}

# Ctrl-C: let PowerShell stop the script; `finally` below always runs Stop-StackChildren.
# (Do NOT CancelKeyPress — that would swallow Ctrl-C and leave children running.)

Write-Host '==> CLI primary stack'
Write-Host "    ATLAS_AGENT_CLI=$env:ATLAS_AGENT_CLI"
Write-Host "    ATLAS_GATEWAY_BACKEND=$env:ATLAS_GATEWAY_BACKEND"
Write-Host "    gateway bind: $GW_BIND"
Write-Host "    Hub WS:       ws://${HUB_BIND}/ws"
Write-Host "    GATEWAY_URL:  $env:ATLAS_GATEWAY_URL"
# Streaming / B1 / mock (always print — unset shown as empty)
$streamDisp = if (Test-EnvSet 'ATLAS_AGENT_CLI_STREAM') { $env:ATLAS_AGENT_CLI_STREAM } else { '(unset=text)' }
Write-Host "    ATLAS_AGENT_CLI_STREAM=$streamDisp"
$eventUrlDisp = if (Test-EnvSet 'ATLAS_HUB_EVENT_URL') { $env:ATLAS_HUB_EVENT_URL } else { '(unset)' }
Write-Host "    ATLAS_HUB_EVENT_URL=$eventUrlDisp"
if (Test-EnvSet 'ATLAS_HUB_EVENT_TOKEN') {
  Write-Host "    ATLAS_HUB_EVENT_TOKEN=(set)"
}
if (Test-EnvSet 'ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK') {
  Write-Host "    ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=$env:ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK"
}
$mockStreamDisp = if (Test-EnvSet 'MOCK_CLI_STREAM') { $env:MOCK_CLI_STREAM } else { '(unset)' }
Write-Host "    MOCK_CLI_STREAM=$mockStreamDisp"
$mockSleepDisp = if (Test-EnvSet 'MOCK_CLI_SLEEP_MS') { $env:MOCK_CLI_SLEEP_MS } else { '(unset)' }
Write-Host "    MOCK_CLI_SLEEP_MS=$mockSleepDisp"
if ($wantStream) {
  Write-Host '    mode: Stream (-Stream / ATLAS_CLI_STACK_STREAM) — multi-process B1 only (no B2 dual)'
} else {
  Write-Host '    mode: text (pass -Stream or ATLAS_CLI_STACK_STREAM=1 for streaming)'
}
Write-Host ''

function Start-CargoPackage {
  param(
    [Parameter(Mandatory)][string]$Package,
    [Parameter(Mandatory)][string]$LogPath,
    [hashtable]$ExtraEnv = @{}
  )
  $saved = @{}
  foreach ($k in $ExtraEnv.Keys) {
    $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process')
    [Environment]::SetEnvironmentVariable($k, [string]$ExtraEnv[$k], 'Process')
  }
  # stdout + stderr cannot share one Redirect* path on Windows; use sidecar .err.log
  $errPath = $LogPath -replace '\.log$', '.err.log'
  if ($errPath -eq $LogPath) { $errPath = "$LogPath.err" }
  try {
    # Quote-safe: WorkingDirectory / Redirect* are .NET strings (spaces OK, e.g. E:\GitHub\...)
    # ArgumentList as single string is more reliable on Windows PowerShell 5.1
    $p = Start-Process -FilePath 'cargo' `
      -ArgumentList "run -p $Package --quiet" `
      -WorkingDirectory $Root `
      -RedirectStandardOutput $LogPath `
      -RedirectStandardError $errPath `
      -PassThru `
      -WindowStyle Hidden `
      -NoNewWindow
    return $p
  } finally {
    foreach ($k in $saved.Keys) {
      [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process')
    }
  }
}

function Test-HttpOk([string]$Url) {
  try {
    $resp = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
    return ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 300)
  } catch {
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
      & curl.exe -sf $Url 1>$null 2>$null
      return ($LASTEXITCODE -eq 0)
    }
    return $false
  }
}

function Test-PidAlive([System.Diagnostics.Process]$Proc, [string]$PidFile) {
  if ($Proc -and -not $Proc.HasExited) { return $true }
  if (Test-Path -LiteralPath $PidFile) {
    $raw = Get-Content -LiteralPath $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($raw) {
      $p = Get-Process -Id ([int]$raw.Trim()) -ErrorAction SilentlyContinue
      if ($p) { return $true }
    }
  }
  return $false
}

function Show-LogTail([string]$LogPath) {
  $errPath = $LogPath -replace '\.log$', '.err.log'
  if ($errPath -eq $LogPath) { $errPath = "$LogPath.err" }
  foreach ($f in @($LogPath, $errPath)) {
    if (Test-Path -LiteralPath $f) {
      Write-Host "---- $f (tail) ----"
      Get-Content -LiteralPath $f -Tail 40 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
    }
  }
}

function Wait-Healthz {
  param(
    [string]$Bind,
    [System.Diagnostics.Process]$Proc,
    [string]$PidFile,
    [string]$LogPath,
    [string]$Label
  )
  Write-Host -NoNewline "    waiting for $Label /healthz"
  $url = "http://${Bind}/healthz"
  for ($i = 1; $i -le 60; $i++) {
    if (Test-HttpOk $url) {
      Write-Host ' ok'
      return
    }
    if (-not (Test-PidAlive -Proc $Proc -PidFile $PidFile)) {
      Write-Host ''
      Write-Host "error: $Label exited early; last log:" -ForegroundColor Red
      Show-LogTail $LogPath
      Stop-StackChildren
      exit 1
    }
    Write-Host -NoNewline '.'
    Start-Sleep -Milliseconds 500
  }
  Write-Host ''
  Write-Host "error: $Label healthz timeout; log: $LogPath" -ForegroundColor Red
  Show-LogTail $LogPath
  Stop-StackChildren
  exit 1
}

try {
  Write-Host "==> starting atlas-bot-gateway (backend=$BACKEND) …"
  if (Test-Path -LiteralPath $GW_LOG) { Remove-Item -LiteralPath $GW_LOG -Force -ErrorAction SilentlyContinue }
  $script:GwProcess = Start-CargoPackage -Package 'atlas-bot-gateway' -LogPath $GW_LOG
  Set-Content -LiteralPath $GW_PID_FILE -Value $script:GwProcess.Id -Encoding ascii

  Wait-Healthz -Bind $GW_BIND -Proc $script:GwProcess -PidFile $GW_PID_FILE -LogPath $GW_LOG -Label 'gateway'

  try {
    $hz = Invoke-WebRequest -Uri "http://${GW_BIND}/healthz" -UseBasicParsing -TimeoutSec 2
    Write-Host "    healthz: $($hz.Content)"
  } catch { }

  Write-Host "==> starting atlas-bot-hub (ATLAS_GATEWAY_URL=$env:ATLAS_GATEWAY_URL) …"
  if (Test-Path -LiteralPath $HUB_LOG) { Remove-Item -LiteralPath $HUB_LOG -Force -ErrorAction SilentlyContinue }
  # Disable embedded gateway HTTP on Hub (remote gateway owns :8787) — same as sh
  $script:HubProcess = Start-CargoPackage -Package 'atlas-bot-hub' -LogPath $HUB_LOG -ExtraEnv @{
    'ATLAS_GATEWAY_HTTP_BIND' = 'off'
  }
  Set-Content -LiteralPath $HUB_PID_FILE -Value $script:HubProcess.Id -Encoding ascii

  Wait-Healthz -Bind $HUB_BIND -Proc $script:HubProcess -PidFile $HUB_PID_FILE -LogPath $HUB_LOG -Label 'Hub'

  Write-Host ''
  Write-Host 'Stack ready.'
  Write-Host "  Invoke-WebRequest http://${GW_BIND}/healthz   # or: curl.exe -s http://${GW_BIND}/healthz"
  Write-Host "  Invoke-WebRequest http://${GW_BIND}/stats"
  Write-Host "  PC: Connect → ws://${HUB_BIND}/ws → select agent → Send"
  if ($wantStream -or (Test-EnvSet 'ATLAS_HUB_EVENT_URL')) {
    Write-Host '  Streaming: PC Events ≥1× hub:assistant_delta → hub:turn_finished (B1 ingest; no B2 dual)'
  }
  Write-Host "  Logs: $GW_LOG  $HUB_LOG"
  Write-Host "  Stop: Ctrl-C (cleanup kills both) or Stop-Process -Id (Get-Content '$GW_PID_FILE'), (Get-Content '$HUB_PID_FILE')"
  Write-Host ''
  Write-Host 'Foreground hold (Ctrl-C to stop)…'

  while ($true) {
    $gwAlive = Test-PidAlive -Proc $script:GwProcess -PidFile $GW_PID_FILE
    $hubAlive = Test-PidAlive -Proc $script:HubProcess -PidFile $HUB_PID_FILE
    if (-not $gwAlive -or -not $hubAlive) {
      Write-Host 'a child exited; shutting down' -ForegroundColor Yellow
      exit 1
    }
    Start-Sleep -Seconds 2
  }
} finally {
  Stop-StackChildren
}
