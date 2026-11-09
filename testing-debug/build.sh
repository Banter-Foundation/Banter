#!/usr/bin/env bash
# Rebuild the Linux release binary into this folder.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p banter-stream-debug --release
cp -f target/release/banter-stream-debug testing-debug/banter-stream-debug
strip testing-debug/banter-stream-debug || true
ls -lh testing-debug/banter-stream-debug
