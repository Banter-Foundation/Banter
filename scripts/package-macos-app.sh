#!/usr/bin/env bash
# Wrap a macOS Banter binary as Banter.app so:
# 1) Finder / `open` launch it as a GUI app (no Terminal window)
# 2) TCC (Mic / Camera / Screen Recording) can attribute permissions to a
#    stable bundle id (foundation.banter.desktop)
#
# Release zips MUST be produced by this script. Zipping a bare Mach-O
# (banter-macos-arm) makes macOS open Terminal.app on double-click.
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
ZIP_NAME="Banter-macos-arm-${VERSION_LABEL}.zip"
ZIP_PATH="$OUT_DIR/$ZIP_NAME"

rm -rf "$APP" "$ZIP_PATH"
mkdir -p "$MACOS_DIR" "$RESOURCES"
cp "$BINARY" "$MACOS_DIR/banter"
chmod +x "$MACOS_DIR/banter"
cp "$ROOT/crates/banter/macos/Info.plist" "$CONTENTS/Info.plist"
# Classic bundle type stamp; Launch Services treats APPL as a GUI app.
printf 'APPL????' > "$CONTENTS/PkgInfo"

# Do not set LSUIElement — Banter is a normal docked GUI app. LSUIElement=true
# would hide the Dock icon and is unrelated to preventing Terminal on launch.
# Terminal opens when Finder runs a *bare* Mach-O; the .app wrapper is the fix.

# Ad-hoc sign so Launch Services / TCC can track the bundle across launches.
# On Linux release helpers (structure-only), codesign is skipped.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP"
fi

(
  cd "$OUT_DIR"
  # -y: store symlinks as symlinks; keep Banter.app/ hierarchy (never zip -j).
  zip -ry "$ZIP_NAME" Banter.app
)

# Guardrail: refuse a zip that is only a loose Mach-O (the Terminal-on-open bug).
if ! unzip -l "$ZIP_PATH" | grep -q 'Banter.app/Contents/MacOS/banter'; then
  echo "error: $ZIP_PATH is missing Banter.app/Contents/MacOS/banter" >&2
  exit 1
fi
if unzip -l "$ZIP_PATH" | awk '{print $NF}' | grep -qx 'banter-macos-arm'; then
  echo "error: $ZIP_PATH still contains a bare banter-macos-arm entry" >&2
  exit 1
fi
if ! grep -q 'CFBundlePackageType' "$CONTENTS/Info.plist" \
  || ! grep -q '<string>APPL</string>' "$CONTENTS/Info.plist"; then
  echo "error: Info.plist must declare CFBundlePackageType APPL" >&2
  exit 1
fi
if grep -q 'LSUIElement' "$CONTENTS/Info.plist"; then
  echo "error: Info.plist must not set LSUIElement (Banter is a docked GUI app)" >&2
  exit 1
fi

echo "Wrote $ZIP_PATH (double-click Banter.app — do not ship a bare Mach-O)"
file "$MACOS_DIR/banter"
unzip -l "$ZIP_PATH"
