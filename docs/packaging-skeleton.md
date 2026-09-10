# Packaging / install skeleton (P1)

> Date: 2026-09-10 · baseline `main`@`294e664`  
> Goal: unify `dist/` + cargo/tauri collection + install/start scripts for Hub+gateway(+cli)+PC.  
> **Still not:** store listing, code signing / notarization, WeCom tickets, MSI/NSIS/DEB/RPM, auto-update, packaging external `agent`, `bot.*` changes.

Acceptance: knowledge-handoff `feitian-packaging-install-skeleton-acceptance.md` · feasibility `pangu-packaging-install-skeleton-feasibility.md`.

---

## Layout

```text
dist/                          # produced by scripts/pack-dist.* (gitignored binaries)
  bin/                         # atlas-bot-hub, atlas-bot-gateway, atlas-bot-cli (default)
  pc/                          # unsigned Tauri executable (atlas-bot-pc[.exe])
  scripts/                     # start-cli-stack.sh|.ps1
  README-INSTALL.md

packaging/templates/           # sources copied into dist/ by pack
  README-INSTALL.md
  scripts/start-cli-stack.*

scripts/pack-dist.sh|.ps1      # cargo --release + tauri build --no-bundle → dist/
scripts/install-local.sh|.ps1  # copy dist → user home (no admin)
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
- `--no-bundle` / no store targets: **never** MSI, NSIS, MS Store, Mac App Store for P1
- `--skip-pc` / `-SkipPc` if WebKit/GTK missing on the pack host
- `--skip-cli` / `-SkipCli` optional (default **includes** `atlas-bot-cli`)

### Tauri `bundle`

`clients/pc/src-tauri/tauri.conf.json` keeps `bundle.active=false` historically; pack always passes `--no-bundle` so even if `active` is flipped later, P1 still collects the **raw executable only**. Do **not** enable store channels.

### PC artifact shape (platform)

| Platform | `dist/pc/` contents (P1) |
|----------|--------------------------|
| Linux | single executable `atlas-bot-pc` |
| Windows | single executable `atlas-bot-pc.exe` |
| macOS | single executable `atlas-bot-pc` (not `.app` / not notarized) |

Unsigned: Windows SmartScreen / macOS Gatekeeper yellow is OK — see `dist/README-INSTALL.md` (Yellow / Gatekeeper section).

---

## Known limits (§10 filled)

| Item | Decision |
|------|----------|
| `dist/` path | Repo-root `dist/`; **gitignore** `dist/bin/`, `dist/pc/` binaries & generated tree contents except we do **not** commit packed binaries. Templates live under `packaging/templates/`; pack regenerates `dist/`. |
| PC shape | Single unsigned executable per platform (see table); not `.app`/MSI/AppImage for P1. |
| `atlas-bot-cli` | **Default included** in `dist/bin/` (omit with `--skip-cli`). |
| start vs `dev-cli-stack` | **Independent** dist scripts (no cargo); contract table above prevents drift. |
| Cross-platform CI artifacts | **Deferred** (P1 = local/manual pack). |
| Yellow copy | `packaging/templates/README-INSTALL.md` / `dist/README-INSTALL.md` intro + Windows/macOS open notes. |

---

## One-liners (after pack)

Unix:

```bash
./scripts/pack-dist.sh && ./scripts/install-local.sh && ~/atlas-bot/scripts/start-cli-stack.sh
```

Windows:

```powershell
.\scripts\pack-dist.ps1; .\scripts\install-local.ps1; & "$env:LOCALAPPDATA\atlas-bot\scripts\start-cli-stack.ps1"
```

Then open `…/pc/atlas-bot-pc[.exe]` → Connect → Send.

---

## Hand-test (D-P*)

| ID | Unix | Win |
|----|------|-----|
| D-P1 pack tree | this slice: run pack on Linux if toolchain allows | hand host |
| D-P2 install-local | user home, no admin | `%LOCALAPPDATA%\atlas-bot` |
| D-P3 start | healthz `backend=cli`; no agent → non-zero | same + optional `-Stream` |
| D-P4 PC Connect/Send | note backend (cli/mock) | same; SmartScreen OK |

---


## Hand-test evidence notes (this slice)

### Unix (Linux pack host · 2026-09-10)

| ID | Result |
|----|--------|
| **D-P1** | `./scripts/pack-dist.sh --skip-pc` → `dist/bin/{hub,gateway,cli}` + `dist/scripts/start-cli-stack.*` + `README-INSTALL.md`. Full PC tauri build **blocked** on this box (no `pkg-config` / WebKit GTK sys deps) — use `--skip-pc` or a desktop Linux/macOS/Win host with Tauri deps for `dist/pc/atlas-bot-pc`. |
| **D-P2** | `ATLAS_BOT_HOME=/tmp/… ./scripts/install-local.sh` → user dir layout; no admin; prints next steps; optional `~/.local/bin` symlinks. |
| **D-P3** | Missing agent → exit 1. Mock: `ATLAS_AGENT_CLI=…/mock-atlas-agent-cli.sh` start → healthz `backend=cli`,`agent_cli_found=true`. |
| **D-P4** | PC Connect/Send **deferred** on this pack host (no PC binary). |

### Windows (hand test · document for host machine)

| ID | Steps |
|----|--------|
| **D-P1** | `.\scripts\pack-dist.ps1` on a Win machine with Rust + Node + WebView2; expect `dist\bin\*.exe` + `dist\pc\atlas-bot-pc.exe`. |
| **D-P2** | `.\scripts\install-local.ps1` → `%LOCALAPPDATA%\atlas-bot\` (or `$env:ATLAS_BOT_HOME`); no admin. |
| **D-P3** | `.\scripts\start-cli-stack.ps1` probe fail non-zero; with agent or mock `.cmd` → healthz `backend=cli`; optional `-Stream`. |
| **D-P4** | Run `pc\atlas-bot-pc.exe` (SmartScreen → More info → Run anyway) → Connect → Send. |

## Related

- Install how-to: `dist/README-INSTALL.md` (from templates)
- Dev runbook: [`cli-primary-runbook.md`](./cli-primary-runbook.md) (dev = cargo; dist = this doc)
- Dev scripts: `scripts/dev-cli-stack.sh|.ps1` (kept)
