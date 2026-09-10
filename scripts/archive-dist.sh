#!/usr/bin/env bash
# Archive an already-packed dist/ tree into a portable zip/tar.gz (T1).
# Does not rebuild binaries — run pack-dist first.
# Usage:
#   ./scripts/archive-dist.sh [--dist DIR] [--out DIR] [--version VER] [--platform PLATFORM]
# Platform auto-detect: linux-x64 | macos-x64 | macos-arm64 | windows-x64 (when run under MSYS)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
OUT="$ROOT/artifacts"
VERSION=""
PLATFORM=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dist) DIST="${2:?}"; shift 2 ;;
    --out) OUT="${2:?}"; shift 2 ;;
    --version) VERSION="${2:?}"; shift 2 ;;
    --platform) PLATFORM="${2:?}"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [--dist DIR] [--out DIR] [--version VER] [--platform PLATFORM]"
      echo "Archives existing dist/ (bin/ pc/ scripts/ README-INSTALL.md). No cargo rebuild."
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if [[ ! -d "$DIST" ]]; then
  echo "error: dist not found: $DIST" >&2
  echo "Run ./scripts/pack-dist.sh first." >&2
  exit 1
fi
if [[ ! -f "$DIST/README-INSTALL.md" && ! -d "$DIST/bin" ]]; then
  echo "error: $DIST does not look like an atlas-bot dist tree" >&2
  exit 1
fi

resolve_version() {
  if [[ -n "$VERSION" ]]; then
    echo "$VERSION"
    return
  fi
  if [[ -n "${ATLAS_BOT_VERSION:-}" ]]; then
    echo "$ATLAS_BOT_VERSION"
    return
  fi
  if command -v git >/dev/null 2>&1 && git -C "$ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    local desc
    desc="$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null || true)"
    if [[ -n "$desc" ]]; then
      # sanitize for filenames
      echo "$desc" | tr '/:' '--'
      return
    fi
  fi
  local cargo_ver=""
  if [[ -f "$ROOT/crates/atlas-bot-hub/Cargo.toml" ]]; then
    cargo_ver="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$ROOT/crates/atlas-bot-hub/Cargo.toml" | head -1)"
  fi
  if [[ -n "$cargo_ver" ]]; then
    echo "$cargo_ver"
    return
  fi
  echo "0.0.0"
}

resolve_platform() {
  if [[ -n "$PLATFORM" ]]; then
    echo "$PLATFORM"
    return
  fi
  local uname_s uname_m
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"
  case "$uname_s" in
    Linux)
      case "$uname_m" in
        x86_64|amd64) echo "linux-x64" ;;
        aarch64|arm64) echo "linux-arm64" ;;
        *) echo "linux-${uname_m}" ;;
      esac
      ;;
    Darwin)
      case "$uname_m" in
        x86_64) echo "macos-x64" ;;
        arm64) echo "macos-arm64" ;;
        *) echo "macos-${uname_m}" ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*)
      echo "windows-x64"
      ;;
    *)
      echo "unknown-${uname_s}-${uname_m}" | tr '[:upper:]' '[:lower:]'
      ;;
  esac
}

VER="$(resolve_version)"
PLAT="$(resolve_platform)"
mkdir -p "$OUT"

# Staging: clean copy without runtime state
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/atlas-bot-archive.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

mkdir -p "$STAGE/atlas-bot"
if command -v rsync >/dev/null 2>&1; then
  rsync -a \
    --exclude '.cli-stack-pids' \
    --exclude '.cli-stack-logs' \
    --exclude '*.pdb' \
    --exclude 'PLACEHOLDER.txt' \
    "$DIST"/ "$STAGE/atlas-bot"/
else
  (cd "$DIST" && tar cf - \
    --exclude='.cli-stack-pids' \
    --exclude='.cli-stack-logs' \
    --exclude='*.pdb' \
    .) | (cd "$STAGE/atlas-bot" && tar xf -)
  rm -f "$STAGE/atlas-bot/pc/PLACEHOLDER.txt" 2>/dev/null || true
fi

# Ensure entry helpers are present when packing from a fresh dist that already has them;
# if missing (older dist), copy from repo scripts/ when available.
for helper in install-desktop-entry.sh install-shortcuts.ps1; do
  if [[ ! -f "$STAGE/atlas-bot/scripts/$helper" && -f "$ROOT/scripts/$helper" ]]; then
    mkdir -p "$STAGE/atlas-bot/scripts"
    cp -f "$ROOT/scripts/$helper" "$STAGE/atlas-bot/scripts/$helper"
  fi
done
if [[ -f "$STAGE/atlas-bot/scripts/install-desktop-entry.sh" ]]; then
  chmod +x "$STAGE/atlas-bot/scripts/install-desktop-entry.sh"
fi
if [[ -f "$STAGE/atlas-bot/scripts/start-cli-stack.sh" ]]; then
  chmod +x "$STAGE/atlas-bot/scripts/start-cli-stack.sh"
fi
chmod +x "$STAGE/atlas-bot/bin/"* 2>/dev/null || true
chmod +x "$STAGE/atlas-bot/pc/"* 2>/dev/null || true

case "$PLAT" in
  windows-*)
    NAME="atlas-bot-${VER}-${PLAT}.zip"
    OUT_PATH="$OUT/$NAME"
    rm -f "$OUT_PATH"
    if command -v zip >/dev/null 2>&1; then
      (cd "$STAGE" && zip -r -q "$OUT_PATH" atlas-bot)
    else
      # Fallback: tar.gz named .zip is wrong — use Python zipfile
      python3 - <<PY
import zipfile, os
from pathlib import Path
root = Path("$STAGE") / "atlas-bot"
out = Path("$OUT_PATH")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
    for p in root.rglob("*"):
        if p.is_file():
            zf.write(p, Path("atlas-bot") / p.relative_to(root))
print("wrote", out)
PY
    fi
    ;;
  *)
    NAME="atlas-bot-${VER}-${PLAT}.tar.gz"
    OUT_PATH="$OUT/$NAME"
    rm -f "$OUT_PATH"
    tar -C "$STAGE" -czf "$OUT_PATH" atlas-bot
    ;;
esac

echo "==> archive-dist"
echo "    version:  $VER  (override: --version / ATLAS_BOT_VERSION; else git describe; else Cargo hub)"
echo "    platform: $PLAT"
echo "    source:   $DIST"
echo "    output:   $OUT_PATH"
ls -lh "$OUT_PATH"
echo
echo "Unpack then write entries:"
echo "  # Unix: tar xzf $NAME && ./atlas-bot/scripts/install-desktop-entry.sh \"\$PWD/atlas-bot\""
echo "  # Win:  expand zip, then .\\atlas-bot\\scripts\\install-shortcuts.ps1 -InstallRoot <path>"
