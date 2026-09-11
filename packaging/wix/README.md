# packaging/wix — T2·W / MSI1 (WiX v4)

| Item | Value |
|------|--------|
| **WiX version** | **v4** only — build with `wix build` (not v3 `candle`/`light`) |
| **ProductName** | `atlas-bot` (ARP display name) |
| **UpgradeCode** | `DAB90D5E-C3DB-405C-9024-769247FFC85F` (**fixed** — never regenerate) |
| **InstallScope** | `perUser` → `%LOCALAPPDATA%\atlas-bot` |
| **Shortcuts** | WiX `Shortcut` elements in `Shortcuts.wxs` (not `install-shortcuts.ps1`) |
| **Desktop** | Feature `DesktopShortcuts` Level=2 → **default off** |
| **MajorUpgrade** | yes (`Schedule=afterInstallInitialize`) |
| **Payload** | `bin/` (hub+gateway+cli) · `pc/` · `scripts/` · `README-INSTALL.md` — **no** external `agent` |

## Build (Windows + WiX v4)

```powershell
# 1) Pack release tree first
.\scripts\pack-dist.ps1

# 2) Build MSI (fails clearly on non-Windows / missing WiX / incomplete dist)
.\scripts\build-msi.ps1
# → artifacts\atlas-bot-<ver>-windows-x64.msi
```

Version for the **filename**: `-Version` / `ATLAS_BOT_VERSION` / `git describe` / Cargo hub / `0.0.0` (same family as `archive-dist`).

MSI **ProductVersion** (numeric X.Y.Z): `-ProductVersion` / numeric `ATLAS_BOT_VERSION` / Cargo hub `0.1.0`.

Install WiX v4: https://wixtoolset.org/ docs — `dotnet tool install --global wix` then `wix --version` (4.x).

## Linux / CI

Linux CI **does not** build MSI. `build-msi.ps1` exits non-zero on non-Windows with a pointer here. Validate `.wxs` XML + script presence on Linux; produce the real `.msi` on a Windows host (**MSI-P\* 「Win 机待补」** until then).

## Coexistence

Zip path (`archive-dist` → `artifacts/*-windows-x64.zip`) **remains**. Prefer one install path day-to-day (MSI **or** zip+`install-local`). MSI uninstall does **not** guarantee removal of files later unpacked manually onto the same folder.

## Not in scope

MS Store · Authenticode / EV · clearing SmartScreen · Program Files default · forced admin · packaging `agent` · changing `bot.*` / start-cli-stack semantics · deleting T1 zip.
