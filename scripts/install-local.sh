#!/usr/bin/env bash
# Install a packed dist/ tree into the user home (no admin / no Program Files).
# Usage:
#   ./scripts/install-local.sh [path-to-dist]
#   ATLAS_BOT_HOME=~/my-atlas ./scripts/install-local.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/dist}"
if [[ ! -d "$SRC" ]]; then
  echo "error: dist source not found: $SRC" >&2
  echo "Run ./scripts/pack-dist.sh first, or pass an explicit dist path." >&2
  exit 1
fi
if [[ ! -f "$SRC/README-INSTALL.md" && ! -d "$SRC/bin" ]]; then
  echo "error: $SRC does not look like an atlas-bot dist tree" >&2
  exit 1
fi

DEST="${ATLAS_BOT_HOME:-$HOME/atlas-bot}"
echo "==> install-local → $DEST"
mkdir -p "$DEST"

# rsync-like copy preserving structure; quote-safe for spaces
copy_tree() {
  local from="$1" to="$2"
  mkdir -p "$to"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete --exclude '.cli-stack-pids' --exclude '.cli-stack-logs' "$from"/ "$to"/
  else
    (cd "$from" && tar cf - --exclude='.cli-stack-pids' --exclude='.cli-stack-logs' .) | (cd "$to" && tar xf -)
  fi
}

copy_tree "$SRC" "$DEST"

if [[ -f "$DEST/scripts/start-cli-stack.sh" ]]; then
  chmod +x "$DEST/scripts/start-cli-stack.sh"
fi
if [[ -d "$DEST/bin" ]]; then
  chmod +x "$DEST/bin/"* 2>/dev/null || true
fi
if [[ -d "$DEST/pc" ]]; then
  chmod +x "$DEST/pc/"* 2>/dev/null || true
fi

LOCAL_BIN="${HOME}/.local/bin"
if mkdir -p "$LOCAL_BIN" 2>/dev/null; then
  for b in atlas-bot-hub atlas-bot-gateway atlas-bot-cli; do
    if [[ -x "$DEST/bin/$b" ]]; then
      ln -sfn "$DEST/bin/$b" "$LOCAL_BIN/$b" 2>/dev/null && \
        echo "    symlink $LOCAL_BIN/$b → $DEST/bin/$b" || \
        echo "    (skip symlink $b — permission or existing file)"
    fi
  done
fi

echo
echo "Installed to: $DEST"
echo "Next steps:"
echo "  1. Start stack:  \"$DEST/scripts/start-cli-stack.sh\""
echo "     (requires agent on PATH, or ATLAS_AGENT_CLI=... ; probe fail = non-zero)"
echo "  2. Open PC:      \"$DEST/pc/atlas-bot-pc\"   # or platform app dir under pc/"
echo "  3. Connect → ws://127.0.0.1:7700/ws → select agent → Send"
echo
echo "Unsigned / yellow prompt is OK (SmartScreen / Gatekeeper). Not a store package."
echo "See: $DEST/README-INSTALL.md"
