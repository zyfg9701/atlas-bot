#!/usr/bin/env bash
# Write ~/.local/share/applications/*.desktop for an atlas-bot install root (T1).
# Optional but delivered: PC + Start CLI Stack entries.
# Usage:
#   ./scripts/install-desktop-entry.sh [install-root]
#   ATLAS_BOT_HOME=~/atlas-bot ./scripts/install-desktop-entry.sh
#   # From unpacked archive:
#   ./atlas-bot/scripts/install-desktop-entry.sh "$PWD/atlas-bot"
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

resolve_root() {
  if [[ $# -ge 1 && -n "${1:-}" ]]; then
    cd "$1" && pwd
    return
  fi
  if [[ -n "${ATLAS_BOT_HOME:-}" && -d "$ATLAS_BOT_HOME" ]]; then
    cd "$ATLAS_BOT_HOME" && pwd
    return
  fi
  # Script under <root>/scripts/
  local parent
  parent="$(cd "$SCRIPT_DIR/.." && pwd)"
  if [[ -x "$parent/pc/atlas-bot-pc" || -d "$parent/bin" || -f "$parent/README-INSTALL.md" ]]; then
    echo "$parent"
    return
  fi
  if [[ -d "${HOME}/atlas-bot" ]]; then
    echo "${HOME}/atlas-bot"
    return
  fi
  echo "error: cannot resolve install root; pass path or set ATLAS_BOT_HOME" >&2
  exit 1
}

ROOT="$(resolve_root "${1:-}")"
PC_BIN="$ROOT/pc/atlas-bot-pc"
START_SH="$ROOT/scripts/start-cli-stack.sh"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$APPS"

# Escape for .desktop Exec= (spaces → \s per desktop-entry-spec, or quote via wrapper)
# Safest portable approach: use quoted path via env sh -c for Path, and Exec with escaped spaces.
desktop_escape() {
  # Escape \ and " for Exec keys; prefer paths without needing shell
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s// /\\s}"
  printf '%s' "$s"
}

ICON_KEY=""
ICON_CANDIDATES=(
  "$ROOT/icons/128x128.png"
  "$ROOT/icons/icon.png"
)
for c in "${ICON_CANDIDATES[@]}"; do
  if [[ -f "$c" ]]; then
    ICON_KEY="Icon=$(desktop_escape "$c")"
    break
  fi
done

# Best-effort: copy icon from repo Tauri icons if install tree has none and we can find them
if [[ -z "$ICON_KEY" ]]; then
  REPO_HINT="$(cd "$SCRIPT_DIR/.." && pwd)"
  for src in \
    "$REPO_HINT/clients/pc/src-tauri/icons/128x128.png" \
    "$REPO_HINT/clients/pc/src-tauri/icons/icon.png"
  do
    if [[ -f "$src" ]]; then
      mkdir -p "$ROOT/icons"
      cp -f "$src" "$ROOT/icons/$(basename "$src")"
      ICON_KEY="Icon=$(desktop_escape "$ROOT/icons/$(basename "$src")")"
      break
    fi
  done
fi

PC_EXEC="$(desktop_escape "$PC_BIN")"
START_EXEC="$(desktop_escape "$START_SH")"
PATH_KEY="$(desktop_escape "$ROOT")"

if [[ ! -x "$PC_BIN" && -f "$PC_BIN" ]]; then
  chmod +x "$PC_BIN" || true
fi
if [[ ! -x "$START_SH" && -f "$START_SH" ]]; then
  chmod +x "$START_SH" || true
fi

cat > "$APPS/atlas-bot-pc.desktop" <<EOD
[Desktop Entry]
Type=Application
Version=1.0
Name=atlas-bot PC
Comment=atlas-bot PC shell (unsigned; not a store package)
Exec=$PC_EXEC
Path=$PATH_KEY
Terminal=false
Categories=Development;Utility;
StartupNotify=true
$ICON_KEY
EOD

cat > "$APPS/atlas-bot-start-cli-stack.desktop" <<EOD
[Desktop Entry]
Type=Application
Version=1.0
Name=atlas-bot Start CLI Stack
Comment=Start atlas-bot Hub+gateway (cli backend; binaries only — no cargo run)
Exec=$START_EXEC
Path=$PATH_KEY
Terminal=true
Categories=Development;Utility;
StartupNotify=false
$ICON_KEY
EOD

echo "==> desktop entries → $APPS"
echo "    atlas-bot-pc.desktop"
echo "    atlas-bot-start-cli-stack.desktop"
echo "    Path=/WorkingDirectory: $ROOT"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPS" 2>/dev/null && \
    echo "    update-desktop-database: ok" || \
    echo "    update-desktop-database: failed (ignored)"
else
  echo "    update-desktop-database: not installed (ignored)"
fi

echo
echo "Install root: $ROOT"
echo "Next: start stack (menu or \"$START_SH\"), then open PC (menu or \"$PC_BIN\")"
echo "Unsigned / Gatekeeper yellow is OK. Not a store package. T2 MSI deferred."
