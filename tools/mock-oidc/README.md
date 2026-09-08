# mock-oidc (I2 CI / local)

Offline mock OIDC Authorization Code + PKCE IdP.

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-oidc
```

Endpoints: `/authorize`, `/token`, `/jwks.json`, `/.well-known/openid-configuration`.

Tokens: HS256 JWT (`id_token` + JWT `access_token`) signed with a known secret;
JWKS is oct/`HS256` compatible with Hub `ATLAS_OIDC_JWKS_JSON`.

See `docs/i2-login-runbook.md`.
