# mock-wecom (W1 CI / local)

Offline mock WeCom OAuth (authorize → code → gettoken / getuserinfo).

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-wecom
```

Endpoints: `/authorize`, `/cgi-bin/gettoken`, `/cgi-bin/user/getuserinfo`.

Point Hub `ATLAS_WECOM_API_BASE` at the printed base URL. **Never** hits `qyapi.weixin.qq.com`.

See `docs/i2-login-runbook.md` § WeCom.
