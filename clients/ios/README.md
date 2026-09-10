# Atlas Bot — iOS (P4)

Swift + SwiftUI + URLSessionWebSocket bot_client.

**Requires macOS + Xcode** to build/run (no Linux CI runner for iOS in P4).

See [docs/P4-runbook.md](../../docs/P4-runbook.md).

```bash
open AtlasBot.xcodeproj
# or
xcodebuild -scheme AtlasBot -destination 'platform=iOS Simulator,name=iPhone 16' build test
```

Generated protocol: `GeneratedProtocol` → `third_party/xai-tool-protocol/generated/swift` (symlink).

## I2.2 Mobile ticket (M1)

- Login uses **ASWebAuthenticationSession + PKCE S256**. **No WebView** primary path.
- Redirect: `atlasbot://auth/callback` (Info.plist URL Types + `callbackURLScheme=atlasbot`).
- `client_id`: `atlas-bot-ios` (register on IdP next to PC loopback).
- TokenStore: Keychain (`afterFirstUnlockThisDeviceOnly`); Logout clears.
- `HubClient` optional `Authorization: Bearer` via `URLRequest`; `nil` = today's no-token (`dev`) behavior.
- Prefer `id_token` as Hub bearer (`pickHubBearer`).
- Linux CI: shared OIDC/PKCE unit tests require macOS/`xcodebuild`; logic is documented for handtest.

See [docs/i2-login-runbook.md](../../docs/i2-login-runbook.md) § Mobile (I2.2).
