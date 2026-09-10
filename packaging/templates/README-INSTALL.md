# atlas-bot · local install (P1 skeleton + T1 installer thickening)

> **Not** a store listing. **Not** code-signed / notarized. **Not** WeCom ticket flow. **Not** MSI (T2 deferred).
> Yellow SmartScreen / Gatekeeper prompts are **expected** — you can still run (Windows: More info → Run anyway; macOS: Right-click → Open).
> Does **not** change `bot.*` wire. Does **not** ship the external Cursor/Atlas `agent` binary.

This tree is produced by `scripts/pack-dist.sh` / `scripts/pack-dist.ps1`, archived by `scripts/archive-dist.*`, or copied by `install-local`.

```text
dist/   (or ~/atlas-bot / %LOCALAPPDATA%\atlas-bot after install)
  bin/                 # atlas-bot-hub, atlas-bot-gateway[, atlas-bot-cli]
  pc/                  # unsigned Tauri desktop shell (executable)
  scripts/             # start-cli-stack.* + install-shortcuts.ps1 + install-desktop-entry.sh
  README-INSTALL.md    # this file
```

Portable archives (T1):

```text
artifacts/atlas-bot-<ver>-windows-x64.zip
artifacts/atlas-bot-<ver>-linux-x64.tar.gz
# optional: macos-*.tar.gz (same layout; still not .app / not notarized)
```

Version in the filename comes from `--version` / `-Version` / `ATLAS_BOT_VERSION`, else `git describe --tags --always`, else Cargo `atlas-bot-hub` version.

Development (repo + Rust toolchain) still uses `scripts/dev-cli-stack.*` + `cargo run`.
Distribution uses **this** tree (`bin/*` only — **never** `cargo run`). See [`docs/packaging-skeleton.md`](../docs/packaging-skeleton.md).

---

## Recommended paths

| Path | When |
|------|------|
| **A. Repo pack → install-local** | You have a clone + toolchain; writes user home + Start Menu / `.desktop` |
| **B. Unpack archive → entry script** | You only have the zip/tar; run `install-shortcuts.ps1` / `install-desktop-entry.sh` from the unpacked tree |

Both keep the same `bin/` + `start-cli-stack` contract.

---

## Unix

### 1. Pack + archive (from repo clone with toolchain)

```bash
./scripts/pack-dist.sh
./scripts/archive-dist.sh
# → artifacts/atlas-bot-<ver>-linux-x64.tar.gz
```

### 2a. Install to user home (no admin) — Path A

```bash
./scripts/install-local.sh
# default: ~/atlas-bot/  (override: ATLAS_BOT_HOME=...)
# writes ~/.local/share/applications/atlas-bot-pc.desktop (+ Start CLI Stack)
# skip entries: ATLAS_BOT_SKIP_DESKTOP=1
```

### 2b. Unpack archive only — Path B

```bash
tar xzf artifacts/atlas-bot-*-linux-x64.tar.gz
./atlas-bot/scripts/install-desktop-entry.sh "$PWD/atlas-bot"
```

### 3. Start CLI stack (requires `agent` on PATH or `ATLAS_AGENT_CLI`)

```bash
~/atlas-bot/scripts/start-cli-stack.sh
# or app menu: atlas-bot Start CLI Stack
# mock bypass (dev tree only):
# ATLAS_AGENT_CLI=/path/to/repo/tools/mock-cli/mock-atlas-agent-cli.sh ~/atlas-bot/scripts/start-cli-stack.sh
```

Probe failure → **non-zero exit** (no silent Hub stub). Expect healthz `backend=cli`.

### 4. Open PC

```bash
~/atlas-bot/pc/atlas-bot-pc
# or app menu: atlas-bot PC
```

Connect → `ws://127.0.0.1:7700/ws` → select agent → Send.

**One-liner (Unix):** `./scripts/pack-dist.sh && ./scripts/archive-dist.sh && ./scripts/install-local.sh && ~/atlas-bot/scripts/start-cli-stack.sh` → open PC / menu.

---

## Windows

### 1. Pack + archive

```powershell
.\scripts\pack-dist.ps1
.\scripts\archive-dist.ps1
# → artifacts\atlas-bot-<ver>-windows-x64.zip
```

### 2a. Install (user profile, no admin) — Path A

```powershell
.\scripts\install-local.ps1
# default: %LOCALAPPDATA%\atlas-bot\  (override: $env:ATLAS_BOT_HOME=...)
# Start Menu: %APPDATA%\Microsoft\Windows\Start Menu\Programs\atlas-bot\
#   • atlas-bot PC
#   • atlas-bot Start CLI Stack
# Desktop optional: .\scripts\install-local.ps1 -Desktop
# skip entries: -SkipShortcuts
```

### 2b. Unpack archive only — Path B

```powershell
Expand-Archive .\artifacts\atlas-bot-*-windows-x64.zip -DestinationPath .
.\atlas-bot\scripts\install-shortcuts.ps1 -InstallRoot (Resolve-Path .\atlas-bot)
# Desktop too: ... -Desktop
```

### 3. Start

```powershell
# Start Menu → atlas-bot → atlas-bot Start CLI Stack
# or:
& "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1"
# streaming (optional):
& "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1" -Stream
```

### 4. Open PC

```powershell
# Start Menu → atlas-bot → atlas-bot PC
# unsigned — SmartScreen may warn; More info → Run anyway
& "$env:LOCALAPPDATA\atlas-bot\pc\atlas-bot-pc.exe"
```

**One-liner (Win):** `.\scripts\pack-dist.ps1; .\scripts\archive-dist.ps1; .\scripts\install-local.ps1; & "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1"` → Start Menu PC.

Shortcuts set **WorkingDirectory** to the install root (spaces-safe). Default = **Start Menu only** (no Desktop clutter); pass `-Desktop` to also write Desktop `.lnk`.

---

## Hand-test checklist

| ID | Step | Expect |
|----|------|--------|
| **D-P1** | pack → inspect `dist/` | `bin/` hub+gateway; `scripts/`+README; `pc/` launchable |
| **D-P2** | `install-local` | user home; no admin; entries written |
| **D-P3** | `start-cli-stack` | healthz `backend=cli`; missing agent → non-zero |
| **D-P4** | open PC → Connect → Send | non-empty reply |
| **T-P1** | archive zip → install/shortcuts | Start Menu PC + Start Stack visible |
| **T-P2** | Start Stack shortcut | healthz `backend=cli` |
| **T-P3** | PC shortcut | shell starts; Connect → Send |
| **T-P4** | archive tar.gz → desktop-entry | tree OK; `.desktop` written |
| **T-P5** | start + PC (script or `.desktop`) | healthz `backend=cli`; PC launchable |

---

## Out of scope (T1)

MSI/NSIS/DEB/RPM/store (T2 MSI deferred), code signing / notarization, auto-update, packaging external `agent`, WeCom tickets, mobile store packages, changing `bot.*`, redoing P1 dist/start contract.
