#!/usr/bin/env bash
# Wrap a macOS Banter binary as Banter.app so TCC (Mic / Camera / Screen Recording)
# can attribute permissions to a stable bundle id.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="${1:-}"
OUT_DIR="${2:-$ROOT/dist/macos}"
VERSION_LABEL="${3:-version-1-test}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  echo "usage: $0 <banter-binary> [out-dir] [version-label]" >&2
  exit 1
fi

APP="$OUT_DIR/Banter.app"
CONTENTS="$APP/Contents"
MACOS_DIR="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

rm -rf "$APP"
mkdir -p "$MACOS_DIR" "$RESOURCES"
cp "$BINARY" "$MACOS_DIR/banter"
chmod +x "$MACOS_DIR/banter"
cp "$ROOT/crates/banter/macos/Info.plist" "$CONTENTS/Info.plist"

# Ad-hoc sign so Launch Services / TCC can track the bundle across launches.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP"
fi

(
  cd "$OUT_DIR"
  zip -ry "Banter-macos-arm-${VERSION_LABEL}.zip" Banter.app
)

echo "Wrote $OUT_DIR/Banter-macos-arm-${VERSION_LABEL}.zip"
file "$MACOS_DIR/banter"
