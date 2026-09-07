# atlas-bot PC (P2)

Tauri 2 + TypeScript `bot_client` for the Computer Hub Bot-Relay loop.

See [`docs/P2-runbook.md`](../../docs/P2-runbook.md).

Quick:

```bash
# Hub
cargo run -p atlas-bot-hub

# Frontend (browser / preview)
cd clients/pc
npm install && npm run build && npm run preview

# Optional native window (needs WebKitGTK on Linux)
npm run tauri -- dev
```
