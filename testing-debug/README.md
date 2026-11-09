# banter-stream-debug

Linux terminal (no UI) Banter streamer for capture / encode debugging.

1. Prompts for Banter **username** and **password**
2. Signs in against `BANTER_BASE` (default `http://127.0.0.1:3970`)
3. Joins **server id 1**, voice **channel id 2**
4. Starts Go Live (Wayland portal, or primary monitor on X11)

## Run the prebuilt binary

From this folder (on Linux):

```bash
./banter-stream-debug
```

Optional:

```bash
BANTER_BASE=http://127.0.0.1:3970 ./banter-stream-debug
```

Ctrl+C stops. On Wayland you will get the system ScreenCast picker.

## Rebuild

From the Banter repo root:

```bash
cargo build -p banter-stream-debug --release
cp -f target/release/banter-stream-debug testing-debug/banter-stream-debug
```

This crate is intentionally Linux-only.
