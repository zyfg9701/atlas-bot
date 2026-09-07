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
