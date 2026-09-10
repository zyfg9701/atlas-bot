# I2.2 / M1 Mobile ticket — handtest checklist

Baseline: main@b61304b+ · redirect `atlasbot://auth/callback` · no WebView · no bot.* changes.

## Prep

- [ ] `cargo run -q -p atlas-bot-auth-client --bin mock-oidc` (note port / issuer)
- [ ] IdP registers `atlasbot://auth/callback` (+ PC loopback if shared)
- [ ] Hub `ATLAS_AUTH_MODE=oidc` + JWKS for E1; `dev` for E3

## Android

1. [ ] Install debug APK / run Compose app
2. [ ] Issuer = host mock (`http://10.0.2.2:<port>` on emulator)
3. [ ] **Login** → Custom Tabs → approve → callback → "ticket stored"
4. [ ] **Connect** → `hello_ack.user_id=sub`
5. [ ] **Logout** → store cleared; under oidc, Connect without ticket fails observably
6. [ ] Under Hub `dev`, Connect with no ticket still works (no regression)

## iOS

1. [ ] Simulator / device build
2. [ ] Issuer = `http://127.0.0.1:<port>`
3. [ ] **Login** → ASWebAuthenticationSession → callback → ticket in Keychain
4. [ ] **Connect** → Bearer on upgrade → `user_id=sub`
5. [ ] **Logout** clears Keychain
6. [ ] `dev` no-ticket Connect still OK

## Unit tests

- [ ] Android: `./gradlew :app:testDebugUnitTest` (PKCE / callback / Bearer)
- [ ] iOS: `xcodebuild -scheme AtlasBot test` on macOS (same)

## Out of scope (do not fail handtest)

- Mobile UI convergence · refresh productization · App/Universal Link · paste Bearer · WebView login
