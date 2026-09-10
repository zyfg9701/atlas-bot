# L1b: Resolve ATLAS_AGENT_CLI (default: agent) and optional non-interactive version/help smoke.
# Does NOT replace scripts/dev-cli-stack.ps1. Does NOT auto-login, write tokens, or redirect to mock.
#
# Usage:
#   .\scripts\probe-agent-cli.ps1
#   .\scripts\probe-agent-cli.ps1 -Cli 'C:\path\to\agent.exe'
#   $env:ATLAS_AGENT_CLI='agent'; .\scripts\probe-agent-cli.ps1
#
# Exit codes:
#   0  found (version/help smoke optional; unsupported → found-only)
#   1  not found
#
# Login state is NOT claimed here — confirm with PC Send after dev-cli-stack.ps1.

param(
  [string]$Cli = ''
)

$ErrorActionPreference = 'Stop'

function Resolve-AgentCli([string]$c) {
  if ([string]::IsNullOrWhiteSpace($c)) { return $null }

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

  # where.exe (PATH) first when available
  $whereCmd = Get-Command -Name 'where.exe' -ErrorAction SilentlyContinue
  if ($null -ne $whereCmd) {
    try {
      $whereOut = & where.exe $c 2>$null
      if ($LASTEXITCODE -eq 0 -and $whereOut) {
        $first = ($whereOut | Select-Object -First 1)
        if ($first -and (Test-Path -LiteralPath $first -PathType Leaf)) {
          return $first
        }
      }
    } catch { }
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

function Test-VersionOrHelpSmoke([string]$resolvedPath) {
  # Non-interactive only. Never claim login. Unsupported flags → found-only.
  $flags = @('--version', '-version', '--help', '-help', '-h')
  foreach ($flag in $flags) {
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    try {
      $p = Start-Process -FilePath $resolvedPath -ArgumentList @($flag) `
        -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr
      $code = $p.ExitCode
      $stdout = ''
      $stderr = ''
      try { $stdout = (Get-Content -LiteralPath $tmpOut -Raw -ErrorAction SilentlyContinue) } catch { }
      try { $stderr = (Get-Content -LiteralPath $tmpErr -Raw -ErrorAction SilentlyContinue) } catch { }
      $combined = (($stdout + "`n" + $stderr) -replace '\s+', ' ').Trim()
      # Heuristic: exit 0, or short help/version-looking output without clear "unknown option"
      $looksUnknown = $combined -match '(?i)unknown (option|flag|argument)|unrecognized|invalid option'
      if ($code -eq 0 -and -not $looksUnknown) {
        $snippet = if ($combined.Length -gt 120) { $combined.Substring(0, 120) + '…' } else { $combined }
        return @{ Ok = $true; Flag = $flag; ExitCode = $code; Snippet = $snippet }
      }
      if ($code -eq 0) { continue }
      # Some CLIs print version to stdout and exit 0 only for --version; keep trying
    } catch {
      # Try next flag
    } finally {
      Remove-Item -LiteralPath $tmpOut, $tmpErr -Force -ErrorAction SilentlyContinue
    }
  }
  return @{ Ok = $false; Flag = $null; ExitCode = $null; Snippet = $null }
}

# Resolve input: -Cli > ATLAS_AGENT_CLI > default "agent"
$requested = $Cli
if ([string]::IsNullOrWhiteSpace($requested)) {
  $requested = [Environment]::GetEnvironmentVariable('ATLAS_AGENT_CLI')
}
if ([string]::IsNullOrWhiteSpace($requested)) {
  $requested = 'agent'
}

Write-Host '==> probe-agent-cli (L1b)'
Write-Host "    requested: $requested"
Write-Host '    note: this probe NEVER claims logged-in; Send after stack is the login check'
Write-Host '    note: will NOT auto-login, write tokens, or silently point at mock'

$resolved = Resolve-AgentCli $requested
if (-not $resolved) {
  Write-Host "    found: false" -ForegroundColor Red
  Write-Host "    resolved: (none)"
  Write-Host "    exit: 1"
  Write-Host ''
  Write-Host 'Next:'
  Write-Host '  1. Install Cursor/Atlas agent CLI and ensure PATH, or set:'
  Write-Host "       `$env:ATLAS_AGENT_CLI='C:\path\to\agent.exe'"
  Write-Host '  2. Re-run: .\scripts\probe-agent-cli.ps1'
  Write-Host '  3. Or temporary mock (CI / no live agent):'
  Write-Host "       `$env:ATLAS_AGENT_CLI=`"`$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd`"; .\scripts\dev-cli-stack.ps1"
  Write-Host 'See docs/live-agent-handtest-checklist.md'
  exit 1
}

Write-Host "    found: true" -ForegroundColor Green
Write-Host "    resolved: $resolved"

$smoke = Test-VersionOrHelpSmoke $resolved
if ($smoke.Ok) {
  Write-Host "    smoke: ok (flag=$($smoke.Flag), exit=$($smoke.ExitCode))"
  if ($smoke.Snippet) {
    Write-Host "    smoke snippet: $($smoke.Snippet)"
  }
  Write-Host '    login: UNKNOWN (probe does not check auth — use PC Send)'
} else {
  Write-Host '    smoke: found-only (version/help flags unsupported or non-zero)'
  Write-Host '    login: UNKNOWN (probe does not check auth — use PC Send)'
}

Write-Host "    exit: 0"
Write-Host ''
Write-Host 'Next:'
Write-Host '  Text stack:    .\scripts\dev-cli-stack.ps1'
Write-Host '  Stream stack:  .\scripts\dev-cli-stack.ps1 -Stream'
Write-Host '  Live checklist: docs/live-agent-handtest-checklist.md'
Write-Host '  Clear mock env before live: Remove-Item Env:ATLAS_AGENT_CLI,Env:MOCK_CLI_STREAM,Env:MOCK_CLI_SLEEP_MS -ErrorAction SilentlyContinue'
exit 0
