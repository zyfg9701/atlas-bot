# WeCom ticket checklist (W1 handtest)

Offline mock path (preferred for CI / local):

1. `cargo run -q -p atlas-bot-auth-client --bin mock-wecom` — note base URL
2. Hub: `ATLAS_AUTH_MODE=static` + `ATLAS_AUTH_JWT_SECRET=…` + `ATLAS_WECOM_CORP_ID/AGENT_ID/SECRET` + `ATLAS_WECOM_API_BASE=<mock>`
3. CLI: `ATLAS_I2_NO_BROWSER=1 ATLAS_TICKET_PROVIDER=wecom ATLAS_WECOM_AUTHORIZE_BASE=<mock> ATLAS_HUB_HTTP=http://127.0.0.1:7700 atlas-bot-cli login --provider wecom`
4. `atlas-bot-cli status` → `hello_ack.user_id=wecom:<corpId>:<userid>`
5. Switch back: `ATLAS_TICKET_PROVIDER=oidc` / `login --provider oidc` — I2 path unchanged
6. `ATLAS_AUTH_MODE=dev` without ticket still connects

Automated: `cargo test -p atlas-bot-cli --test wecom_smoke -- --nocapture --test-threads=1` → `SMOKE_OK wecom…`

Yellow-tag live WeCom (optional, not DoD): register redirect, set live `ATLAS_WECOM_API_BASE`, scan, confirm hello `sub`.

See `docs/i2-login-runbook.md` § WeCom.
