#!/usr/bin/env bash
# Pack release Hub+gateway[+cli] and PC shell into dist/ (P1 skeleton).
# Only collects --release / tauri build outputs (never debug).
# Usage: ./scripts/pack-dist.sh [--skip-pc] [--skip-cli]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SKIP_PC=0
SKIP_CLI=0
for arg in "$@"; do
  case "$arg" in
    --skip-pc) SKIP_PC=1 ;;
    --skip-cli) SKIP_CLI=1 ;;
    -h|--help)
      echo "Usage: $0 [--skip-pc] [--skip-cli]"
      exit 0
      ;;
    *)
      echo "unknown arg: $arg" >&2
      exit 2
      ;;
  esac
done

DIST="$ROOT/dist"
TEMPLATE="$ROOT/packaging/templates"
EXE_SUFFIX=""
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE_SUFFIX=".exe" ;;
esac

echo "==> pack-dist (Unix) → $DIST"
rm -rf "$DIST/bin" "$DIST/pc" "$DIST/scripts"
mkdir -p "$DIST/bin" "$DIST/pc" "$DIST/scripts"

PACKAGES=(atlas-bot-hub atlas-bot-gateway)
if [[ "$SKIP_CLI" -eq 0 ]]; then
  PACKAGES+=(atlas-bot-cli)
fi

echo "==> cargo build --release: ${PACKAGES[*]}"
cargo build --release $(printf -- '-p %s ' "${PACKAGES[@]}")

for name in "${PACKAGES[@]}"; do
  src="$ROOT/target/release/${name}${EXE_SUFFIX}"
  if [[ ! -f "$src" ]]; then
    echo "error: missing release binary: $src" >&2
    exit 1
  fi
  cp -f "$src" "$DIST/bin/"
  chmod +x "$DIST/bin/${name}${EXE_SUFFIX}" 2>/dev/null || true
  echo "    collected bin/${name}${EXE_SUFFIX}"
done

# Templates (start scripts + README)
cp -f "$TEMPLATE/README-INSTALL.md" "$DIST/README-INSTALL.md"
cp -f "$TEMPLATE/scripts/start-cli-stack.sh" "$DIST/scripts/start-cli-stack.sh"
cp -f "$TEMPLATE/scripts/start-cli-stack.ps1" "$DIST/scripts/start-cli-stack.ps1"
chmod +x "$DIST/scripts/start-cli-stack.sh"
# T1 entry helpers (also kept under scripts/ for repo-side install-local)
for helper in install-desktop-entry.sh install-shortcuts.ps1; do
  if [[ -f "$ROOT/scripts/$helper" ]]; then
    cp -f "$ROOT/scripts/$helper" "$DIST/scripts/$helper"
  elif [[ -f "$TEMPLATE/scripts/$helper" ]]; then
    cp -f "$TEMPLATE/scripts/$helper" "$DIST/scripts/$helper"
  fi
done
if [[ -f "$DIST/scripts/install-desktop-entry.sh" ]]; then
  chmod +x "$DIST/scripts/install-desktop-entry.sh"
fi

if [[ "$SKIP_PC" -eq 0 ]]; then
  echo "==> PC: tauri build --no-bundle (unsigned executable; NOT store / NOT MSI)"
  if [[ ! -d "$ROOT/clients/pc/node_modules" ]]; then
    (cd "$ROOT/clients/pc" && npm ci)
  fi
  # --no-bundle: collect raw release binary only; never open MS Store / Mac App Store
  (cd "$ROOT/clients/pc" && npm run tauri -- build --no-bundle --ci)
  PC_BIN_DIR="$ROOT/clients/pc/src-tauri/target/release"
  PC_NAME="atlas-bot-pc${EXE_SUFFIX}"
  if [[ -f "$PC_BIN_DIR/$PC_NAME" ]]; then
    cp -f "$PC_BIN_DIR/$PC_NAME" "$DIST/pc/"
    chmod +x "$DIST/pc/$PC_NAME" 2>/dev/null || true
    echo "    collected pc/$PC_NAME"
  else
    echo "error: PC release binary not found at $PC_BIN_DIR/$PC_NAME" >&2
    echo "hint: re-run with --skip-pc if WebKit/GTK deps missing; document in hand-test." >&2
    exit 1
  fi
  # Copy sidecars if present (linux .so deps not required for skeleton)
  if [[ -d "$PC_BIN_DIR/bundle" ]]; then
    echo "    note: tauri bundle/ present but ignored (P1 = --no-bundle executable only)"
  fi
else
  echo "==> skip PC (--skip-pc); writing pc/PLACEHOLDER.txt"
  cat > "$DIST/pc/PLACEHOLDER.txt" <<'EOP'
PC binary not packed on this run (--skip-pc or missing deps).
On a full pack host: npm run tauri -- build --no-bundle  → dist/pc/atlas-bot-pc[.exe]
EOP
fi

echo
echo "Pack complete. Tree:"
find "$DIST" -maxdepth 3 \( -type f -o -type d \) | sort
echo
echo "Next: ./scripts/archive-dist.sh   # portable tar.gz/zip from dist/"
echo "      ./scripts/install-local.sh  # copy + .desktop entries"
echo "      OR run dist/scripts/start-cli-stack.sh from this tree"
