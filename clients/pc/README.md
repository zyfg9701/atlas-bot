# atlas-bot PC

Tauri 2 + TypeScript `bot_client` for Computer Hub Bot-Relay.

**U1 UI：** 默认一屏 **Chat**（Connect / 选 agent / 对话）；Cold path、协议细按钮、VNC、upload、完整 Log 在页底 **「更多 / 调试」**。  
能力只藏不砍。说明见 [`docs/pc-user-guide.md`](../../docs/pc-user-guide.md)；手测 [`docs/pc-ui-u1-checklist.md`](../../docs/pc-ui-u1-checklist.md)。

## Dev

```bash
npm i
npm run tauri dev   # recommended (Bearer on WS via connect_ws)
npm run dev         # vite only — browser WS cannot set Authorization
npm run test:unit   # hubClient + authClient
```

Hub default: `ws://127.0.0.1:7700/ws` (`ATLAS_AUTH_MODE=dev` = no login).

Debug panel: open「更多 / 调试」, or `?debug=1`, or `localStorage atlas-pc-debug=1`.
