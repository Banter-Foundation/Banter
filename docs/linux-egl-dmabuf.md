# Linux EGL DMA-BUF import proof (M3)

Stacked on DMA-BUF negotiate (`docs/linux-dmabuf-negotiate.md`). Proves `EGL_EXT_image_dma_buf_import` (+ modifiers when present) against PipeWire DMA-BUF frames. Encode still uses MemFd/MemPtr until VA-API hardware frames (M4).

## What landed

- GBM `/dev/dri/renderD*` (or surfaceless Mesa) EGL display + GLES2 context
- Per-frame `eglCreateImageKHR` / `eglDestroyImageKHR` while PipeWire FDs are valid
- Sticky disable on init/import failure (no per-frame spam); MemFd encode path unchanged
- `/debugnow` tokens: `egl_ok=` / `egl_fail=` alongside `dmabuf=` / `mem=` / `mod=`
- Journal: `banter: EGL DMA-BUF ready …`, `import ok …`, or `init/import failed …`
- `BANTER_DMABUF=0` still advertises MemFd-only

## Deferred / follow-up

- Keep EGLImage / GL textures across frames (optional GL path)
- **M4 VA-API hardware-frame encode:** `docs/linux-vaapi-dmabuf.md`

## Measure

Same matrix as M0–M2. Watch for:

```text
banter: EGL DMA-BUF ready (EGL …, modifiers=…)
banter: EGL DMA-BUF import ok (…x… … mod=0x… planes=…)
```

and `/debugnow` `egl_ok` rising while Go Live runs with DMA-BUF. Headless CI without a GPU is fine — init fails once and MemFd fallback remains.
