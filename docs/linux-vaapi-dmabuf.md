# Linux VA-API DMA-BUF encode (M4)

Stacked on EGL import proof (`docs/linux-egl-dmabuf.md`). Feeds PipeWire DMA-BUF into VA-API: PRIME import → VideoProc CSC/scale to NV12 → `VAEntrypointEncSlice` H.264. Skips Banter’s CPU BGRA→I420 convert on the Go Live screen path when this path sticks.

## What landed

- Own DMA-BUF plane FDs (`dup`) in the PipeWire callback so buffers can requeue
- Prefer DMA-BUF EnumFormat ahead of MemFd when `/dev/dri/renderD*` exists (override with `BANTER_VAAPI_DMABUF=0` or `BANTER_DMABUF=0`)
- VA-API session: DRM display, optional VPP, per-simulcast-layer encode contexts
- Convert thread hand-off is near-zero cost; `/debugnow` shows `va_ok=` / `va_fail=`
- Sticky disable on init/import/encode failure — MemFd/CPU + existing ffmpeg VA-API/`openh264` remain
- Windows DXGI/GDI and macOS paths unchanged

## Escape hatches

| Env | Effect |
| --- | --- |
| `BANTER_VAAPI_DMABUF=0` | Do not prefer / use the M4 path (`available()` false) |
| `BANTER_DMABUF=0` | Advertise MemFd-only (M2 escape; no DMA-BUF EnumFormat) |

## Deferred

- Packed SPS/PPS / richer rate-control tuning parity with ffmpeg `h264_vaapi`
- Local preview from DMA-BUF without CPU convert
- NVENC CUDA import of the same DMA-BUF
- Multi-GPU capture vs encode device pinning

## Measure

Same matrix as M0–M3. Expect `convert p95` near zero and `va_ok` rising while Go Live runs with DMA-BUF + VA-API. If the path sticky-fails, journal shows the reason and MemFd encode resumes after renegotiation / `BANTER_DMABUF=0`.
