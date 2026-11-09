# Linux DMA-BUF negotiation (M2)

Stacked on capture metrics (`docs/linux-capture-metrics.md`). Implements PipeWire DMA-BUF **negotiation + plane parsing + metrics**, with MemFd/MemPtr remaining the encode path until EGL import.

## What landed

- Dual `SPA_PARAM_EnumFormat`: MemFd (no modifier) first, then DMA-BUF with `VIDEO_modifier` (`INVALID` + `LINEAR`, `MANDATORY|DONT_FIXATE`)
- `SPA_PARAM_Buffers` `dataType` updated on Format fixation (DmaBuf+Mem when modifier present; Mem-only otherwise)
- Per-plane DMA-BUF parse (`fd`, offset, stride, size, flags) + `/debugnow` counters (`dmabuf=` / `mem=` / `mod=`)
- Journal line includes `modifier=` / `has_mod=`
- One-shot warning when a DMA-BUF frame arrives before EGL import is wired
- `BANTER_DMABUF=0` advertises MemFd-only (escape hatch)

## Deferred (next stacked PRs)

- **M3:** landed separately — `docs/linux-egl-dmabuf.md` (import proof + metrics; no encode yet)
- **M4:** VA-API `AVHWFramesContext` / hardware-frame encode without GPU→CPU readback
- Consuming DMA-BUF into the convert/encode path (today MemFd still produces the I420)

## Measure

Same matrix as M0/M1. Watch for:

```text
banter: screencast format … type=MemFd|DmaBuf … modifier=0x… has_mod=…
```

and `/debugnow` `mem=` / `dmabuf=` counts. If the compositor only offers DMA-BUF and encode stalls, set `BANTER_DMABUF=0` and report the journal line.
