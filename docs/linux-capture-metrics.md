# Linux screen-capture metrics (Arch / Bazzite Wayland)

Banter records per-stage capture counters while Go Live is active. Use them to baseline FPS before larger DMA-BUF / EGL work.

## Where to look

- **`/debugnow`** (and F3 profiler QoS): a `capture …` line while screen is publishing.
- Journal / stderr: one negotiated format line per PipeWire renegotiation, e.g.  
  `banter: screencast format BGRA (12) 2560x1440 stride=10240 type=MemFd planes=1`

Fields on the capture line:

| Token | Meaning |
| --- | --- |
| `capture A/B fps` | Rolling PipeWire arrival FPS / convert-submit FPS |
| `drop N%` | Latest-wins drops (portal slot or convert inbox overwritten) |
| `cb p95` | PipeWire callback wall time |
| `copy p95` | CPU MemFd/MemPtr copy into pooled buffer |
| `convert p95` | Scale + I420 convert |
| `encode p95` | H.264 encode for a screen layer |
| `e2e p95` | Approximate grab→publish lag + encode |
| `copied` | Cumulative CPU bytes copied this Go Live |

## Baseline matrix

Same machine, same encoder settings, ≥30 s each:

| Test | Resolution | Target FPS |
| --- | ---: | ---: |
| A | 1920×1080 | 60 |
| B | 2560×1440 | 60 |
| C | 2560×1440 | 120 (if compositor allows) |
| D | 3840×2160 | 60 |

Record: captured FPS, submit FPS, drop %, CPU, GPU, and the p95 stages above. Also note the negotiated format/type from the journal (do not assume I420/NV12).

### Arch

```bash
echo "$XDG_SESSION_TYPE"   # expect wayland
vainfo
pw-cli info 0
```

### Bazzite

Prefer a toolbox/dev environment for builds. Same diagnostics (`vainfo`, `pw-cli`, session type).

## What this PR changed vs deferred

**Landed (M0 + low-risk M1):**

- Atomic capture metrics + p50/p95 rings
- Pooled CPU frame buffers for PipeWire MemFd/MemPtr copies
- Drop counting on latest-wins overwrite
- Richer format/type/stride/plane logging
- Skip preview conversion when the local preview is hidden
- Callback stays copy-only (convert on `banter-screen-convert`)

**Follow-ups:**

- M2 DMA-BUF negotiate/parse: `docs/linux-dmabuf-negotiate.md`
- M3 EGL import proof: `docs/linux-egl-dmabuf.md`
- M4: VA-API hardware frames (still deferred)

Windows DXGI/GDI and macOS paths are unchanged aside from shared convert timing counters when those feeds publish.
