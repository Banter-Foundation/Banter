#!/usr/bin/env bash
# Build Apple Silicon client + server release artifacts.
# The client zip ALWAYS contains Banter.app (never a bare Mach-O).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION_LABEL="${1:-version-1-test}"
OUT_DIR="${2:-$ROOT/dist/macos}"

mkdir -p "$OUT_DIR"
cd "$ROOT"

cargo build -p banter --release
cargo build -p banter-server --release

CLIENT_BIN="$ROOT/target/release/banter"
SERVER_BIN="$ROOT/target/release/banter-server"

if [[ ! -f "$CLIENT_BIN" || ! -f "$SERVER_BIN" ]]; then
  echo "error: expected release binaries under target/release/" >&2
  exit 1
fi

# Package Banter.app + zip (ad-hoc signed when codesign exists).
"$ROOT/scripts/package-macos-app.sh" "$CLIENT_BIN" "$OUT_DIR" "$VERSION_LABEL"

cp "$SERVER_BIN" "$OUT_DIR/banter-server-macos-arm"
chmod +x "$OUT_DIR/banter-server-macos-arm"
if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$OUT_DIR/banter-server-macos-arm"
fi

echo "macOS release artifacts in $OUT_DIR:"
ls -la "$OUT_DIR/Banter.app" \
  "$OUT_DIR/Banter-macos-arm-${VERSION_LABEL}.zip" \
  "$OUT_DIR/banter-server-macos-arm"
