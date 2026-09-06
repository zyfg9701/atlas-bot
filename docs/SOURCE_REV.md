# Upstream pin

| Item | Value |
|------|-------|
| SOURCE_REV | `a549186d9d39311f2d3ee4208db62af8c65aa476` |
| Upstream tree | `E:\work\mygit\architechure\grok-build` |
| Vendored crates | `crates/common/xai-tool-protocol`, `crates/common/xai-tool-types` (dependency) |
| License (package) | Apache-2.0 (per crate `Cargo.toml`) |

`Cargo.toml` in `third_party/*` was adjusted to path/crates.io deps so the empty atlas-bot workspace compiles without the full grok-build workspace. Wire types/fixtures/tests are unchanged from the pin.

**License audit:** package metadata says Apache-2.0; full dependency/file NOTICE review still yellow — no Hub/gateway business until green.

## Target repo

Primary: https://github.com/zyfg9701/atlas-bot
Earlier Eric9701/atlas-bot mirror may exist from P0 push; this tree is canonical going forward.
