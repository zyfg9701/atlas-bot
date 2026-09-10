# atlas-bot · local install (P1 packaging skeleton)

> **Not** a store listing. **Not** code-signed / notarized. **Not** WeCom ticket flow.
> Yellow SmartScreen / Gatekeeper prompts are **expected** — you can still run (Windows: More info → Run anyway; macOS: Right-click → Open).
> Does **not** change `bot.*` wire. Does **not** ship the external Cursor/Atlas `agent` binary.

This tree is produced by `scripts/pack-dist.sh` / `scripts/pack-dist.ps1` (or copied by `install-local`).

```text
dist/   (or ~/atlas-bot / %LOCALAPPDATA%\atlas-bot after install)
  bin/                 # atlas-bot-hub, atlas-bot-gateway[, atlas-bot-cli]
  pc/                  # unsigned Tauri desktop shell (executable / directory)
  scripts/             # start-cli-stack.sh|.ps1  (binaries only — no cargo run)
  README-INSTALL.md    # this file
```

Development (repo + Rust toolchain) still uses `scripts/dev-cli-stack.*` + `cargo run`.
Distribution uses **this** tree. See [`docs/packaging-skeleton.md`](../docs/packaging-skeleton.md) and [`docs/cli-primary-runbook.md`](../docs/cli-primary-runbook.md).

---

## Unix

### 1. Pack (from repo clone with toolchain)

```bash
./scripts/pack-dist.sh
# tree: dist/bin dist/pc dist/scripts dist/README-INSTALL.md
```

### 2. Install to user home (no admin)

```bash
./scripts/install-local.sh
# default: ~/atlas-bot/  (override: ATLAS_BOT_HOME=...)
# optional symlink into ~/.local/bin (best-effort)
```

### 3. Start CLI stack (requires `agent` on PATH or `ATLAS_AGENT_CLI`)

```bash
~/atlas-bot/scripts/start-cli-stack.sh
# mock bypass (dev tree only):
# ATLAS_AGENT_CLI=/path/to/repo/tools/mock-cli/mock-atlas-agent-cli.sh ~/atlas-bot/scripts/start-cli-stack.sh
```

Probe failure → **non-zero exit** (no silent Hub stub). Expect healthz `backend=cli`.

### 4. Open PC

```bash
# Linux: run the collected binary under pc/
~/atlas-bot/pc/atlas-bot-pc
# or open the app directory copied into pc/ (platform-specific)
```

Connect → `ws://127.0.0.1:7700/ws` → select agent → Send.

---

## Windows

### 1. Pack

```powershell
.\scripts\pack-dist.ps1
```

### 2. Install (user profile, no admin)

```powershell
.\scripts\install-local.ps1
# default: %LOCALAPPDATA%\atlas-bot\  (override: $env:ATLAS_BOT_HOME=...)
# optional user PATH append for bin\ (best-effort; failure does not block)
```

### 3. Start

```powershell
& "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1"
# streaming (optional, WS1 parity):
& "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1" -Stream
```

### 4. Open PC

```powershell
# unsigned — SmartScreen may warn; More info → Run anyway
& "$env:LOCALAPPDATA\atlas-bot\pc\atlas-bot-pc.exe"
```

---

## Hand-test checklist (D-P*)

| ID | Step | Expect |
|----|------|--------|
| **D-P1** | pack → inspect `dist/` | `bin/` has hub+gateway; `scripts/`+README; `pc/` has launchable |
| **D-P2** | clean dir → `install-local` | user home layout; no admin; prints next steps |
| **D-P3** | `start-cli-stack` | stack up; healthz `backend=cli`; missing agent → non-zero |
| **D-P4** | open PC → Connect → Send | non-empty reply (note stub/cli/mock) |

Unix evidence: run pack+install on a Linux box with Rust/Node.  
Win evidence: same steps on a Windows host (hand test; CI matrix **not** required for P1).

---

## Out of scope (P1)

MSI/NSIS/DEB/RPM/store, code signing / notarization, auto-update, packaging external `agent`, WeCom tickets, mobile store packages, changing `bot.*`.
