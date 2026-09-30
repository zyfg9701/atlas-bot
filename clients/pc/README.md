# atlas-bot PC

Tauri 2 + TypeScript `bot_client` for Computer Hub Bot-Relay.

**U2 UI：** 冷启动 **入口** 二选一。`#/chat` 是纯 Chat（Connect / 选 agent / 对话 / Send）。`#/debug` 保留 U1 整页能力（调试手风琴默认展开、Gateway HTTP、G1/C1、VNC/upload、Raw Log）。同一会话切换 hash **不断开** WebSocket。  
**U4：** Chat 与 Debug 共用顶栏下的 Hub WS + 可选 Bearer（`localStorage` `atlas-pc-hub-ws` / `atlas-pc-hub-bearer`）。默认仍是 `ws://127.0.0.1:7700/ws`。Gateway HTTP `8787` 不在这条连接条里。  
能力只藏不砍。说明见 [`docs/pc-user-guide.md`](../../docs/pc-user-guide.md)；手测 [`docs/pc-ui-u4-checklist.md`](../../docs/pc-ui-u4-checklist.md)（U2：[`docs/pc-ui-u2-checklist.md`](../../docs/pc-ui-u2-checklist.md)，U1：[`docs/pc-ui-u1-checklist.md`](../../docs/pc-ui-u1-checklist.md)）。

## Dev

```bash
npm i
npm run tauri dev   # recommended (Bearer on WS via connect_ws)
npm run dev         # vite only — browser WS cannot set Authorization
npm run test:unit   # hubClient + authClient + pcMode
```

Hub default: `ws://127.0.0.1:7700/ws` (`ATLAS_AUTH_MODE=dev` = no login).

Routes: `#/` gate (unless `localStorage atlas-pc-mode` is `chat` or `debug`), `#/chat`, `#/debug`. Deep link overwrites the stored mode. `?debug=1` opens `#/debug` once and is then removed from the query string. `atlas-pc-debug` only remembers whether the debug accordion is open (`0` closed; missing or `1` open) — it does not choose the page. Chat hides Login / Gateway HTTP / G1 / C1 / 「更多 / 调试」; Hub WS and optional Bearer stay on the shared strip under Connect (`atlas-pc-hub-ws`, `atlas-pc-hub-bearer`). Use the **调试** link for Login. Browser `npm run dev` and Tauri share this hash shell (browser WS still cannot set `Authorization`).
