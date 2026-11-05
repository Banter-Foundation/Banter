#!/usr/bin/env bash
# Smoke-test that package-macos-app.sh produces a Banter.app zip (not a bare
# Mach-O). Safe to run on Linux CI with a dummy binary.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

DUMMY="$TMP/fake-banter"
printf '#!/bin/sh\necho banter\n' > "$DUMMY"
chmod +x "$DUMMY"

"$ROOT/scripts/package-macos-app.sh" "$DUMMY" "$TMP/out" "smoke"

ZIP="$TMP/out/Banter-macos-arm-smoke.zip"
test -f "$ZIP"
unzip -l "$ZIP" | grep -q 'Banter.app/Contents/Info.plist'
unzip -l "$ZIP" | grep -q 'Banter.app/Contents/MacOS/banter'
unzip -l "$ZIP" | grep -q 'Banter.app/Contents/PkgInfo'
# Must not look like the old Terminal-opening release layout.
if unzip -l "$ZIP" | awk '{print $NF}' | grep -qx 'banter-macos-arm'; then
  echo "fail: zip still has bare banter-macos-arm" >&2
  exit 1
fi

# Extract and check plist policy (no LSUIElement; APPL package type).
mkdir -p "$TMP/extract"
unzip -q "$ZIP" -d "$TMP/extract"
PLIST="$TMP/extract/Banter.app/Contents/Info.plist"
grep -q '<string>APPL</string>' "$PLIST"
if grep -q 'LSUIElement' "$PLIST"; then
  echo "fail: LSUIElement present (would hide Dock; not the Terminal fix)" >&2
  exit 1
fi
test -x "$TMP/extract/Banter.app/Contents/MacOS/banter"
test "$(cat "$TMP/extract/Banter.app/Contents/PkgInfo")" = 'APPL????'

echo "ok: Banter.app zip layout (GUI launch path, no bare Mach-O)"
