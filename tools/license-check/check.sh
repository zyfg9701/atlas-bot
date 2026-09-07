#!/usr/bin/env bash
# Fail if any cargo package license matches GPL/AGPL (case-insensitive).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if ! command -v cargo-license >/dev/null 2>&1; then
  echo "Installing cargo-license..."
  cargo install cargo-license --locked 2>/dev/null || cargo install cargo-license
fi

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

cargo license --json >"$TMP"

python3 - "$TMP" <<'PY'
import json, re, sys
path = sys.argv[1]
gpl_re = re.compile(r"(?i)(?<!\w)(agpl|gpl)(?!\w)")
bad = []
with open(path, encoding="utf-8") as f:
    data = json.load(f)
for row in data:
    lic = row.get("license") or row.get("license_file") or ""
    name = row.get("name") or "?"
    ver = row.get("version") or "?"
    if not lic:
        continue
    if gpl_re.search(lic):
        bad.append(f"{name}@{ver}: {lic}")
if bad:
    print("ERROR: GPL/AGPL license detected:")
    for b in bad:
        print(" ", b)
    sys.exit(1)
print(f"OK: scanned {len(data)} packages; no GPL/AGPL licenses.")
PY
