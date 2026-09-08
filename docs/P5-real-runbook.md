# P5 real runbook — VNC proxy + disk attachments

Extends the stub MVP in [P5-runbook.md](./P5-runbook.md). Wire shapes are unchanged:
`bot.vncDescriptor` → `{ vncUrl, expiresHint }`; upload/attach → `{ path }` (+ `uploadId`).

## Mode switches

| Env | Values | Default | Notes |
|-----|--------|---------|-------|
| `ATLAS_VNC_MODE` | `stub` \| `proxy` | **stub** | CI stays stub |
| `ATLAS_VNC_UPSTREAM` | `host:port` | unset | Required for real proxy mint; without it, proxy **degrades to stub page** |
| `ATLAS_VNC_STUB_BASE` | URL | `http://127.0.0.1:8787` | Public base used when minting `vncUrl` |
| `ATLAS_ATTACH_MODE` | `memory` \| `disk` | **memory** | CI/default for `p5_smoke`; **disk** is the real default below |
| `ATLAS_ATTACH_ROOT` | path | `./data/attachments` | Disk mode root |
| `ATLAS_ATTACH_TTL_SECS` | seconds | `86400` (24h) | Swept on upload/attach; expired → `attachment_not_found` |

Hub embeds `InMemoryGateway` and reads these env vars on startup (no Hub code change required).

## Smoke

```bash
# Stub + memory regression (unchanged)
cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture
# expect: SMOKE_OK p5 vnc+upload ...

# Disk + VNC proxy mock
cargo test -p atlas-bot-hub --test p5r_smoke -- --nocapture
# expect: SMOKE_OK p5r attach-disk ...
#         SMOKE_OK p5r vnc-proxy ...
#         SMOKE_OK p5r vnc-proxy degrade-to-stub ...
```

## Start (real defaults)

```bash
# Disk attachments + VNC proxy (degrades to stub until upstream is set)
export ATLAS_ATTACH_MODE=disk
export ATLAS_ATTACH_ROOT=./data/attachments
export ATLAS_VNC_MODE=proxy
# optional: export ATLAS_VNC_UPSTREAM=127.0.0.1:5900
RUST_LOG=info cargo run -p atlas-bot-hub
# ws://127.0.0.1:7700/ws
# gateway HTTP (loopback): http://127.0.0.1:8787/
```

With upstream set, `bot.vncDescriptor` mints `http://127.0.0.1:8787/vnc/{token}/`
(short TTL ≈ 5 minutes). The built-in page is a **proxy mock** documenting the
upstream — enough for Evidence. Full noVNC + websockify is **not** vendored.

### Optional: full noVNC via Docker

If you need a browser RFB client against a real `:5900`:

```bash
# Example only — pin/tag as you prefer; not required for P5 实装 DoD
docker run --rm --network host \
  -e VNC_SERVER=127.0.0.1:5900 \
  theasp/novnc:latest
# Point ATLAS_VNC_STUB_BASE / operator docs at the noVNC URL, or put a reverse
# proxy in front of gateway `/vnc/{token}/` → websockify. Do not vendor the
# noVNC tree into this repo unless NOTICE is updated.
```

## Attachments (disk)

- Upload writes `{ATLAS_ATTACH_ROOT}/{uploadId}/{filename}` plus `meta.json`
  (`uploadId`, `path`, `filename`, `bytes`, `createdAt`, `state=ready`, `agentId`).
- Restart of gateway reloads `meta.json` → `attachUpload` still works until TTL.
- On upload/attach, expired entries are swept and treated as `attachment_not_found`.
- Limits unchanged: args JSON > 3 MiB → `args_too_large`; object > 25 MiB → `attachment_too_large`.

## Security

- Bind gateway HTTP to **loopback** only (`127.0.0.1:8787` / `ATLAS_GATEWAY_HTTP_BIND`).
- VNC tokens are short-lived; expired/unknown → HTTP `410` / `403`.
- Disk root may contain user uploads — keep permissions tight; do not expose
  `ATLAS_ATTACH_ROOT` on a public volume without auth (out of scope: IdP).

## What this is / is not

| Is | Is not |
|----|--------|
| Disk + TTL + restart-safe metadata | Multi-region attachment cluster |
| Tokenized VNC URL + mock Evidence | Vendored full noVNC tree / RFB cluster |
| Stub degrade when no upstream | Claiming a real desktop without upstream |
| Closed-set reject reasons preserved | `readAttachment*` family / IdP / groups |

## Known limitations (checklist §6)

- **VNC:** proxy Evidence is the built-in mock HTML when upstream is set; full
  noVNC is a Docker/operator option, not embedded. Without `ATLAS_VNC_UPSTREAM`,
  proxy mode degrades to the same stub page as today.
- **Attachments:** TTL swept on upload/attach (lazy); disk mode persists across
  gateway restart via `meta.json`. Memory mode keeps in-process map (CI).
- **`readAttachment*`:** not in scope for P5 实装.
