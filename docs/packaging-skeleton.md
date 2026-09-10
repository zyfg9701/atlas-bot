# Packaging / install skeleton (P1) + installer thickening (T1)

> Date: 2026-09-10 · baseline `main`@`fd837c7`  
> Goal: unify `dist/` + cargo/tauri collection + install/start scripts for Hub+gateway(+cli)+PC; **T1** adds portable zip/tar.gz archives + Win Start Menu shortcuts + Unix `.desktop` entries.  
> **Still not:** store listing, code signing / notarization, WeCom tickets, MSI/NSIS/DEB/RPM (**T2 MSI deferred**), auto-update, packaging external `agent`, `bot.*` changes.

Acceptance: knowledge-handoff `feitian-installer-thickening-acceptance.md` · feasibility `pangu-installer-thickening-feasibility.md`  
(P1 prior: `feitian-packaging-install-skeleton-acceptance.md`)

---

## Layout

```text
dist/                          # produced by scripts/pack-dist.* (gitignored binaries)
  bin/                         # atlas-bot-hub, atlas-bot-gateway, atlas-bot-cli (default)
  pc/                          # unsigned Tauri executable (atlas-bot-pc[.exe])
  scripts/                     # start-cli-stack.* + install-shortcuts.ps1 + install-desktop-entry.sh
  README-INSTALL.md

artifacts/                     # produced by scripts/archive-dist.* (gitignored)
  atlas-bot-<ver>-windows-x64.zip
  atlas-bot-<ver>-linux-x64.tar.gz
  # optional macos-*.tar.gz

packaging/templates/           # sources copied into dist/ by pack
  README-INSTALL.md
  scripts/start-cli-stack.*

scripts/pack-dist.sh|.ps1      # cargo --release + tauri build --no-bundle → dist/
scripts/archive-dist.sh|.ps1   # T1: zip/tar.gz from existing dist/ (no rebuild)
scripts/install-local.sh|.ps1  # copy dist → user home (no admin) + entries
scripts/install-shortcuts.ps1  # T1: Win Start Menu (+ optional Desktop)
scripts/install-desktop-entry.sh # T1: ~/.local/share/applications/*.desktop
scripts/dev-cli-stack.sh|.ps1  # DEV path (cargo run) — unchanged
```

Install defaults:

| OS | Default home |
|----|--------------|
| Unix | `~/atlas-bot/` or `$ATLAS_BOT_HOME` |
| Windows | `%LOCALAPPDATA%\atlas-bot\` or `$env:ATLAS_BOT_HOME` |

---

## Dev vs dist

| Path | How to start stack | Binaries |
|------|--------------------|----------|
| **Development** | `scripts/dev-cli-stack.sh|.ps1` | `cargo run -p …` inside repo |
| **Distribution** | `dist/scripts/start-cli-stack.*` (or after install) | `dist/bin/*` only — **never** `cargo run` |

Contract parity (same env names, probe fail → non-zero, `ATLAS_GATEWAY_BACKEND=cli`, Hub `ATLAS_GATEWAY_URL`, Hub `ATLAS_GATEWAY_HTTP_BIND=off`, healthz/logs/PIDs, Win optional `-Stream`):

See checklist table in §start below. Implementation is an **independent script** (not a thin wrapper over `dev-cli-stack`), to guarantee no accidental `cargo run`. Drift prevention = this doc + matching probe/start behavior.

Mock bypass remains **dev-tree only** (`tools/mock-cli/…`); default dist does **not** ship mock or ACP adapter.

---

## §start contract checklist (dev ↔ dist)

| Behavior | `dev-cli-stack.*` | `dist/.../start-cli-stack.*` |
|----------|-------------------|------------------------------|
| Resolve `ATLAS_AGENT_CLI` / PATH `agent` | yes | yes |
| Missing agent → exit ≠ 0, no silent stub | yes | yes |
| Start gateway with `BACKEND=cli` | `cargo run -p atlas-bot-gateway` | `$ROOT/bin/atlas-bot-gateway` |
| Start Hub with `GATEWAY_URL` + `HTTP_BIND=off` | `cargo run -p atlas-bot-hub` | `$ROOT/bin/atlas-bot-hub` |
| Wait healthz gateway + Hub | yes | yes |
| PID + log dirs | `.cli-stack-pids` / `.cli-stack-logs` | same under dist/install root |
| Win `-Stream` / `ATLAS_CLI_STACK_STREAM` | yes | yes |
| Unix STREAM default | unset (explicit) | unset (explicit) |

---

## Pack notes

- `cargo build -p atlas-bot-hub -p atlas-bot-gateway -p atlas-bot-cli --release` → `dist/bin/`
- PC: `npm run tauri -- build --no-bundle` → copy `clients/pc/src-tauri/target/release/atlas-bot-pc[.exe]` → `dist/pc/`
- `--no-bundle` / no store targets: **never** MSI, NSIS, MS Store, Mac App Store for P1/T1
- `--skip-pc` / `-SkipPc` if WebKit/GTK missing on the pack host
- `--skip-cli` / `-SkipCli` optional (default **includes** `atlas-bot-cli`)

### Tauri `bundle`

`clients/pc/src-tauri/tauri.conf.json` keeps `bundle.active=false` historically; pack always passes `--no-bundle` so even if `active` is flipped later, P1/T1 still collects the **raw executable only**. Do **not** enable store channels. **T2 MSI** (optional, deferred) must whitelist `targets` to `msi` only.

### PC artifact shape (platform)

| Platform | `dist/pc/` contents (P1) |
|----------|--------------------------|
| Linux | single executable `atlas-bot-pc` |
| Windows | single executable `atlas-bot-pc.exe` |
| macOS | single executable `atlas-bot-pc` (not `.app` / not notarized) |

Unsigned: Windows SmartScreen / macOS Gatekeeper yellow is OK — see `dist/README-INSTALL.md` (Yellow / Gatekeeper section).

---

## T1 · Archive + launch entries

### Archive

```bash
./scripts/archive-dist.sh [--dist DIR] [--out DIR] [--version VER] [--platform PLATFORM]
```

```powershell
.\scripts\archive-dist.ps1 [-DistPath <path>] [-OutDir <path>] [-Version <ver>] [-Platform windows-x64]
```

- Consumes **existing** `dist/` (does not rebuild / no `cargo run`)
- Output under `artifacts/` (gitignored): `atlas-bot-<ver>-<platform>.zip|.tar.gz`
- Archive root folder: `atlas-bot/{bin,pc,scripts,README-INSTALL.md}`
- Excludes: `.cli-stack-*`, `*.pdb`, `pc/PLACEHOLDER.txt`, debug, ACP adapter, mock-oidc

### Win shortcuts

- Default: **Start Menu only** → `%APPDATA%\Microsoft\Windows\Start Menu\Programs\atlas-bot\`
  - `atlas-bot PC.lnk` → `pc\atlas-bot-pc.exe`
  - `atlas-bot Start CLI Stack.lnk` → `powershell -NoProfile -ExecutionPolicy Bypass -File …\scripts\start-cli-stack.ps1`
- `WorkingDirectory` = install root (spaces-safe, quoted `-File` path)
- Desktop: opt-in via `install-local.ps1 -Desktop` or `install-shortcuts.ps1 -Desktop`
- Hook: `install-local.ps1` calls `install-shortcuts.ps1`; archive also carries the helper for unpack-only

### Unix `.desktop`

- Writes `~/.local/share/applications/atlas-bot-pc.desktop`
- Also writes `atlas-bot-start-cli-stack.desktop` (Terminal=true) — Start stack is a **second `.desktop`**, not README-only
- `Exec=` / `Path=` pin install root (desktop-entry `\s` escaping for spaces)
- `update-desktop-database` best-effort (failure ignored)
- Hook: `install-local.sh` calls `install-desktop-entry.sh` (skip with `ATLAS_BOT_SKIP_DESKTOP=1`)

---

## Known limits (§9 filled — T1)

| Item | Decision |
|------|----------|
| Archive filename / version | `atlas-bot-<ver>-<platform>.zip\|tar.gz`. Version: `--version`/`-Version`/`ATLAS_BOT_VERSION` → else `git describe --tags --always --dirty` → else Cargo `atlas-bot-hub` version → else `0.0.0`. |
| Win default entries | **Start Menu only**; Desktop via `-Desktop` switch on `install-local.ps1` / `install-shortcuts.ps1`. |
| Unix Start stack | **Second `.desktop`** (`atlas-bot-start-cli-stack.desktop`) + script one-liner in README. |
| Entry hook | Both: `install-local.*` calls helpers at end; helpers also live in `dist/scripts/` (and archives) for unpack-only Path B. |
| Cross-platform CI artifact | **Deferred** (this slice = local/manual pack+archive). |
| macOS tar | **Optional same layout** (`macos-x64` / `macos-arm64`); still not `.app` / not notarized. Auto-detected by `archive-dist.sh` on Darwin. |
| `dist/` path | Repo-root `dist/`; gitignore binaries. Templates under `packaging/templates/`. |
| PC shape | Single unsigned executable per platform; not `.app`/MSI/AppImage for T1. |
| `atlas-bot-cli` | **Default included** in `dist/bin/` (omit with `--skip-cli`). |
| start vs `dev-cli-stack` | **Independent** dist scripts (no cargo); contract table above. |
| Yellow copy | README-INSTALL intro + Windows/macOS open notes; T1 does **not** promise to clear SmartScreen / Gatekeeper. |
| MSI | **T2 deferred** — not required for T1 DoD. |

---

## One-liners

### After pack (P1 path)

Unix:

```bash
./scripts/pack-dist.sh && ./scripts/install-local.sh && ~/atlas-bot/scripts/start-cli-stack.sh
```

Windows:

```powershell
.\scripts\pack-dist.ps1; .\scripts\install-local.ps1; & "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1"
```

### T1 archive → entry → start → PC

Unix:

```bash
./scripts/pack-dist.sh && ./scripts/archive-dist.sh && \
  tar xzf artifacts/atlas-bot-*-linux-*.tar.gz && \
  ./atlas-bot/scripts/install-desktop-entry.sh "$PWD/atlas-bot" && \
  "$PWD/atlas-bot/scripts/start-cli-stack.sh"
# then: "$PWD/atlas-bot/pc/atlas-bot-pc"   # or app menu
```

Windows:

```powershell
.\scripts\pack-dist.ps1; .\scripts\archive-dist.ps1
Expand-Archive .\artifacts\atlas-bot-*-windows-x64.zip -DestinationPath .
.\atlas-bot\scripts\install-shortcuts.ps1 -InstallRoot (Resolve-Path .\atlas-bot)
& ".\atlas-bot\scripts\start-cli-stack.ps1"
# then Start Menu → atlas-bot PC   (or & ".\atlas-bot\pc\atlas-bot-pc.exe")
```

---

## Hand-test (D-P* + T-P*)

| ID | Unix | Win |
|----|------|-----|
| D-P1 pack tree | pack on Linux if toolchain allows | hand host |
| D-P2 install-local | user home + `.desktop` | `%LOCALAPPDATA%\atlas-bot` + Start Menu |
| D-P3 start | healthz `backend=cli`; no agent → non-zero | same + optional `-Stream` |
| D-P4 PC Connect/Send | note backend (cli/mock) | same; SmartScreen OK |
| T-P1 archive → entries | tar.gz + `.desktop` | zip + Start Menu |
| T-P2 Start Stack | script or `.desktop` | Start Menu shortcut |
| T-P3 PC | binary or `.desktop` | Start Menu shortcut |

---

## Hand-test evidence notes

### Unix (Linux pack host · 2026-09-10)

| ID | Result |
|----|--------|
| **D-P1** | `./scripts/pack-dist.sh --skip-pc` → `dist/bin/{hub,gateway,cli}` + `dist/scripts/start-cli-stack.*` + helpers + `README-INSTALL.md`. Full PC tauri build **blocked** on this box (no WebKit GTK) — use `--skip-pc` or a desktop host for `dist/pc/atlas-bot-pc`. |
| **D-P2** | `ATLAS_BOT_HOME=/tmp/… ./scripts/install-local.sh` → user dir + `.desktop` under `~/.local/share/applications/`. |
| **D-P3** | Missing agent → exit 1. Mock: `ATLAS_AGENT_CLI=…/mock-atlas-agent-cli.sh` start → healthz `backend=cli`. |
| **D-P4** | PC Connect/Send **deferred** on this pack host when `--skip-pc`. |
| **T-P4** | `./scripts/archive-dist.sh` → `artifacts/atlas-bot-<ver>-linux-x64.tar.gz`; unpack + `install-desktop-entry.sh` writes both `.desktop` files (code-reviewable). |
| **T-P5** | start via script; PC `.desktop` content reviewable when PC binary absent. |

### Windows (hand test · document for host machine)

| ID | Steps |
|----|--------|
| **D-P1** | `.\scripts\pack-dist.ps1` → `dist\bin\*.exe` + `dist\pc\atlas-bot-pc.exe`. |
| **D-P2** | `.\scripts\install-local.ps1` → Start Menu `Programs\atlas-bot\`. |
| **D-P3** | Start Stack shortcut / script → healthz `backend=cli`; optional `-Stream`. |
| **D-P4** | PC shortcut (SmartScreen → Run anyway) → Connect → Send. |
| **T-P1–T-P3** | `.\scripts\archive-dist.ps1` → expand → `install-shortcuts.ps1` → Start Menu PC + Start Stack. |

---

## Related

- Install how-to: `dist/README-INSTALL.md` (from templates)
- Dev runbook: [`cli-primary-runbook.md`](./cli-primary-runbook.md) (dev = cargo; dist = this doc)
- Dev scripts: `scripts/dev-cli-stack.sh|.ps1` (kept)
