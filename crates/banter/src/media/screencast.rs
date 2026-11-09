//! xdg-desktop-portal ScreenCast. The compositor sends a PipeWire view stream.
//!
//! The stream callback only copies the newest frame. Scaling happens later, on
//! the screen thread, when that frame is published. A pixel walk here stalls
//! the compositor and makes the share late.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::capture_metrics;

use pipewire::{
    channel,
    context::ContextRc,
    keys,
    main_loop::MainLoopRc,
    properties,
    spa::{
        buffer::{ChunkFlags, DataType},
        param::{
            format::{FormatProperties, MediaSubtype, MediaType},
            format_utils,
            video::{VideoFormat, VideoInfoRaw},
            ParamType,
        },
        pod::{self, serialize::PodSerializer, Pod},
        utils::{Direction, Fraction, Rectangle, SpaTypes},
    },
    stream::{StreamFlags, StreamRc, StreamState},
};
use zbus::blocking::Proxy;
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, OwnedValue, Type, Value};

use super::encode::{self, even_fit, PackedOrder};

struct Latest {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Unscaled copy of one PipeWire buffer. The convert thread scales it.
///
/// `bytes` may come from a bounded recycle pool so the PipeWire callback avoids
/// a fresh heap allocation every frame (plan §3.1). Drop returns the buffer.
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    pub format: VideoFormat,
    pub bytes: Vec<u8>,
    recycle: Option<Arc<Mutex<Vec<Vec<u8>>>>>,
}

impl RawFrame {
    /// Owned CPU frame with no recycle pool (kmsgrab / tests).
    pub fn owned(width: u32, height: u32, stride: i32, format: VideoFormat, bytes: Vec<u8>) -> Self {
        Self {
            width,
            height,
            stride,
            format,
            bytes,
            recycle: None,
        }
    }
}

impl Drop for RawFrame {
    fn drop(&mut self) {
        let Some(pool) = self.recycle.take() else {
            return;
        };
        let mut buf = std::mem::take(&mut self.bytes);
        if buf.capacity() == 0 {
            return;
        }
        buf.clear();
        if let Ok(mut free) = pool.lock() {
            // Bound the pool so a format change cannot grow memory forever.
            if free.len() < 4 {
                free.push(buf);
            }
        };
    }
}

fn take_pooled(pool: &Arc<Mutex<Vec<Vec<u8>>>>, need: usize) -> Vec<u8> {
    if let Ok(mut free) = pool.lock() {
        while let Some(mut buf) = free.pop() {
            if buf.capacity() >= need {
                buf.clear();
                return buf;
            }
        }
    }
    Vec::with_capacity(need)
}

/// Convert a pending Cast frame to encode-sized I420 (optional preview).
pub fn encode_raw_frame(
    raw: &RawFrame,
    edge: u32,
    want_preview: bool,
) -> Option<(u32, u32, Vec<u8>, Option<(u32, u32, Vec<u8>)>)> {
    sample_encode_frame(
        &raw.bytes,
        raw.width,
        raw.height,
        raw.stride,
        raw.format,
        edge,
        want_preview,
    )
}

struct StreamData {
    format: VideoInfoRaw,
    logged: bool,
    /// Reusable CPU slots for MemFd/MemPtr copies (not DMA-BUF import).
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Last observed spa_data type name for the format log line.
    data_type: &'static str,
}

pub struct CastStream {
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<String>>>,
    quit: Option<channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// Closed explicitly on drop so a second Go Live can create a new portal session.
    _portal: Portal,
}

impl CastStream {
    /// Opens the system share dialog and starts the PipeWire stream.
    /// Blocks until the dialog is confirmed or dismissed.
    pub fn open(max_edge: u32, fps: u32) -> Result<Self, String> {
        let mut cast = Portal::connect()?;
        let session = cast.create_session()?;
        cast.select_sources(&session)?;
        let started = cast.start(&session)?;
        let node = started
            .streams
            .as_ref()
            .and_then(|streams| streams.first())
            .map(|stream| stream.0)
            .ok_or_else(|| "screen share was cancelled".to_string())?;
        let fd = cast.pipewire_fd(&session)?;
        cast.attach_session(session);
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Mutex::new(None));
        let (quit_tx, quit_rx) = channel::channel();
        let latest_thread = Arc::clone(&latest);
        let stop_thread = Arc::clone(&stop);
        let fault_thread = Arc::clone(&fault);
        let thread = thread::Builder::new()
            .name("banter-screencast".into())
            .spawn(move || {
                stream_loop(
                    fd,
                    node,
                    max_edge,
                    fps.max(1),
                    latest_thread,
                    stop_thread,
                    fault_thread,
                    quit_rx,
                )
            })
            .map_err(|err| err.to_string())?;
        Ok(Self {
            latest,
            stop,
            fault,
            quit: Some(quit_tx),
            thread: Some(thread),
            _portal: cast,
        })
    }

    /// True once the compositor has delivered a buffer that has not been taken.
    pub fn has_frame(&self) -> bool {
        self.latest.lock().expect("screencast frame").is_some()
    }

    /// Takes the newest raw frame and scales it to RGBA at `edge` on this thread.
    #[allow(dead_code)] // kept for tests / preview-only callers; Go Live uses the convert pipeline.
    pub fn take_picture(&self, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
        let raw = self.take_raw()?;
        let frame = sample_frame(
            &raw.bytes, raw.width, raw.height, raw.stride, raw.format, edge,
        )?;
        Some((frame.width, frame.height, frame.rgba))
    }

    /// Move the newest unscaled PipeWire buffer off the portal slot (no pixel walk).
    /// The screen convert thread turns this into I420 via [`encode_raw_frame`].
    pub fn take_raw(&self) -> Option<RawFrame> {
        self.latest.lock().expect("screencast frame").take()
    }

    /// One-pass packed→I420 (optional small RGBA preview). Avoids RGBA intermediate
    /// then another `fit_rgba`+`rgba_to_i420` on the screen thread (~100 ms at Source).
    #[allow(dead_code)]
    pub fn take_encode(
        &self,
        edge: u32,
        want_preview: bool,
    ) -> Option<(u32, u32, Vec<u8>, Option<(u32, u32, Vec<u8>)>)> {
        let raw = self.take_raw()?;
        encode_raw_frame(&raw, edge, want_preview)
    }

    pub fn take_fault(&self) -> Option<String> {
        self.fault.lock().expect("screencast fault").take()
    }
}

impl Drop for CastStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(quit) = self.quit.take() {
            let _ = quit.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn stream_loop(
    fd: OwnedFd,
    node: u32,
    max_edge: u32,
    fps: u32,
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<String>>>,
    quit_rx: channel::Receiver<()>,
) {
    pipewire::init();
    let edge = capture_edge(max_edge);
    let main_loop = match MainLoopRc::new(None) {
        Ok(loop_) => loop_,
        Err(err) => {
            fail(&fault, format!("screencast loop: {err}"));
            return;
        }
    };
    let context = match ContextRc::new(&main_loop, None) {
        Ok(context) => context,
        Err(err) => {
            fail(&fault, format!("screencast context: {err}"));
            return;
        }
    };
    let core = match context.connect_fd_rc(fd, None) {
        Ok(core) => core,
        Err(err) => {
            fail(&fault, format!("screencast connect: {err}"));
            return;
        }
    };
    let stream = match StreamRc::new(
        core,
        "banter-screencast",
        properties::properties! {
            *keys::MEDIA_TYPE => "Video",
            *keys::MEDIA_CATEGORY => "Capture",
            *keys::MEDIA_ROLE => "Screen",
        },
    ) {
        Ok(stream) => stream,
        Err(err) => {
            fail(&fault, format!("screencast stream: {err}"));
            return;
        }
    };

    let cancel = main_loop.clone();
    let _quit = quit_rx.attach(main_loop.loop_(), move |_| cancel.quit());

    let data = StreamData {
        format: VideoInfoRaw::new(),
        logged: false,
        pool: Arc::new(Mutex::new(Vec::with_capacity(4))),
        data_type: "unknown",
    };
    let callback_loop = main_loop.clone();
    let fault_state = Arc::clone(&fault);
    let stop_loop = Arc::clone(&stop);
    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(move |_, _, _, new| {
            if let StreamState::Error(err) = new {
                let detail = if err.is_empty() {
                    "screen share stream failed".to_string()
                } else {
                    format!("screen share stream failed: {err}")
                };
                fail(&fault_state, detail);
            }
        })
        .param_changed(|_, data, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            if data.format.parse(param).is_err() {
                return;
            }
            // Re-log on renegotiation so format/size changes are visible in the journal.
            data.logged = false;
        })
        .process(move |stream, data| {
            let callback_started = Instant::now();
            if stop_loop.load(Ordering::Relaxed) {
                callback_loop.quit();
                return;
            }
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let plane_count = datas.len();
            let Some(chunk_src) = datas.first_mut() else {
                return;
            };
            let data_type = spa_data_type_name(chunk_src.type_());
            data.data_type = data_type;
            let (offset, chunk_size, stride, corrupted) = {
                let chunk = chunk_src.chunk();
                (
                    chunk.offset() as usize,
                    chunk.size() as usize,
                    chunk.stride(),
                    chunk.flags().contains(ChunkFlags::CORRUPTED),
                )
            };
            if corrupted {
                return;
            }
            if !data.logged {
                data.logged = true;
                let size = data.format.size();
                let fmt = data.format.format();
                eprintln!(
                    "banter: screencast format {} ({}) {}x{} stride={} type={} planes={}",
                    format_name(fmt),
                    fmt.0,
                    size.width,
                    size.height,
                    stride,
                    data_type,
                    plane_count
                );
            }
            let copy_started = Instant::now();
            // DMA-BUF EGL import is Milestone 2+. Prefer an existing CPU mapping; never
            // mmap a DMA-BUF as a linear pixel array (tiled/compressed modifiers).
            let frame = if let Some(bytes) = chunk_src.data() {
                data.data_type = "MemPtr";
                copy_frame(
                    bytes,
                    offset,
                    chunk_size,
                    stride,
                    &data.format,
                    &data.pool,
                )
            } else if chunk_src.type_() == DataType::DmaBuf {
                None
            } else if let Some(mapped) = map_memfd(chunk_src) {
                data.data_type = "MemFd";
                copy_frame(
                    mapped.bytes(),
                    offset,
                    chunk_size,
                    stride,
                    &data.format,
                    &data.pool,
                )
            } else {
                None
            };
            let copy_elapsed = copy_started.elapsed();
            if let Some(frame) = frame {
                let bytes = frame.bytes.len() as u64;
                let mut slot = latest.lock().expect("screencast frame");
                let dropped = slot.is_some();
                *slot = Some(frame);
                drop(slot);
                capture_metrics::note_received(callback_started.elapsed(), copy_elapsed, bytes, dropped);
            }
        })
        .register();
    let listener = match _listener {
        Ok(listener) => listener,
        Err(err) => {
            fail(&fault, format!("screencast listen: {err}"));
            return;
        }
    };

    let pod = match format_pod(edge, fps) {
        Ok(pod) => pod,
        Err(err) => {
            fail(&fault, err);
            return;
        }
    };
    let Some(pod) = Pod::from_bytes(&pod) else {
        fail(&fault, "screencast format was rejected".into());
        return;
    };
    let mut params = [pod];
    if let Err(err) = stream.connect(
        Direction::Input,
        Some(node),
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut params,
    ) {
        fail(&fault, format!("screencast start: {err}"));
        return;
    }
    let started = Instant::now();
    main_loop.run();
    drop(listener);
    drop(stream);
    if !stop.load(Ordering::Relaxed) && started.elapsed() < Duration::from_millis(200) {
        fail(
            &fault,
            "screencast ended before the first frame".to_string(),
        );
    }
}

fn fail(fault: &Mutex<Option<String>>, message: String) {
    eprintln!("banter: {message}");
    let mut slot = fault.lock().expect("screencast fault");
    if slot.is_none() {
        *slot = Some(message);
    }
}

/// PipeWire leaves the pixel pointer empty until the consumer maps a memfd.
struct Mmap {
    ptr: *mut libc::c_void,
    len: usize,
}

impl Mmap {
    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.cast(), self.len) }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

fn spa_data_type_name(kind: DataType) -> &'static str {
    if kind == DataType::MemPtr {
        "MemPtr"
    } else if kind == DataType::MemFd {
        "MemFd"
    } else if kind == DataType::DmaBuf {
        "DmaBuf"
    } else {
        "other"
    }
}

fn copy_frame(
    bytes: &[u8],
    offset: usize,
    chunk_size: usize,
    stride: i32,
    format: &VideoInfoRaw,
    pool: &Arc<Mutex<Vec<Vec<u8>>>>,
) -> Option<RawFrame> {
    let size = if chunk_size == 0 {
        bytes.len().saturating_sub(offset)
    } else {
        chunk_size
    };
    let end = offset.saturating_add(size).min(bytes.len());
    if end <= offset {
        return None;
    }
    let dim = format.size();
    if dim.width < 2 || dim.height < 2 {
        return None;
    }
    let need = end - offset;
    let mut buf = take_pooled(pool, need);
    buf.clear();
    buf.extend_from_slice(&bytes[offset..end]);
    Some(RawFrame {
        width: dim.width,
        height: dim.height,
        stride,
        format: format.format(),
        bytes: buf,
        recycle: Some(Arc::clone(pool)),
    })
}

fn map_memfd(data: &pipewire::spa::buffer::Data) -> Option<Mmap> {
    if data.type_() != DataType::MemFd {
        return None;
    }
    let raw = data.as_raw();
    let len = raw.maxsize as usize;
    let fd = data.fd();
    if fd < 0 || len == 0 {
        return None;
    }
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            raw.mapoffset as i64,
        )
    };
    if ptr == libc::MAP_FAILED {
        return None;
    }
    Some(Mmap { ptr, len })
}

fn capture_edge(max_edge: u32) -> u32 {
    let edge = if max_edge == 0 { 1920 } else { max_edge };
    (edge.clamp(640, 1920)) & !1
}

fn format_pod(edge: u32, fps: u32) -> Result<Vec<u8>, String> {
    let height = (edge as u64 * 9 / 16).max(2) as u32 & !1;
    let obj = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        // Prefer planar I420 / NV12 when the compositor offers them — skips a full
        // BT.601 packed→YUV walk (OBS PipeWire path also prefers DMA/planar).
        // Packed BGRA/RGBA remain the common fallback on KWin/Mutter.
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::I420,
            VideoFormat::I420,
            VideoFormat::NV12,
            VideoFormat::BGRA,
            VideoFormat::BGRx,
            VideoFormat::RGBA,
            VideoFormat::RGBx,
            VideoFormat::RGB,
            VideoFormat::BGR
        ),
        pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: edge,
                height
            },
            Rectangle {
                width: 2,
                height: 2
            },
            Rectangle {
                width: 4096,
                height: 4096
            }
        ),
        // KWin offers 0/1 here, meaning the compositor picks the rate. A minimum of 1 rejects that offer.
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: fps, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: fps.max(1),
                denom: 1
            }
        ),
    );
    let (cursor, _) =
        PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &pod::Value::Object(obj))
            .map_err(|err| err.to_string())?;
    Ok(cursor.into_inner())
}

fn sample_frame(
    src: &[u8],
    width: u32,
    height: u32,
    stride: i32,
    format: VideoFormat,
    max_edge: u32,
) -> Option<Latest> {
    let (bpp, kind) = pixel_kind(format)?;
    let stride = if stride > 0 {
        stride as usize
    } else {
        width as usize * bpp
    };
    if src.len()
        < stride
            .saturating_mul(height as usize)
            .max(width as usize * bpp)
        && kind != Pixel::I420
        && kind != Pixel::Nv12
    {
        if src.len() < width as usize * bpp * height as usize {
            return None;
        }
    }
    let (dw, dh) = even_fit(width, height, max_edge);
    if dw < 2 || dh < 2 {
        return None;
    }
    let mut rgba = vec![0u8; (dw as usize) * (dh as usize) * 4];
    match kind {
        Pixel::I420 => sample_i420(src, width, height, dw, dh, &mut rgba)?,
        Pixel::Nv12 => {
            let i420 = scale_nv12_to_i420(src, width, height, stride, dw, dh)?;
            sample_i420(&i420, dw, dh, dw, dh, &mut rgba)?;
        }
        Pixel::Packed { swap_rb } => {
            if stride < width as usize * bpp {
                return None;
            }
            for row in 0..dh {
                let src_row = (row as u64 * height as u64 / dh as u64) as usize;
                let row_bytes = src.get(src_row * stride..)?;
                for col in 0..dw {
                    let src_col = (col as u64 * width as u64 / dw as u64) as usize;
                    let pixel = row_bytes.get(src_col * bpp..)?.get(..bpp)?;
                    let at = ((row * dw + col) * 4) as usize;
                    write_pixel(&mut rgba[at..at + 4], pixel, bpp, swap_rb);
                }
            }
        }
    }
    Some(Latest {
        width: dw,
        height: dh,
        rgba,
    })
}

/// Scale + convert PipeWire packed/I420 buffers straight to planar I420.
fn sample_encode_frame(
    src: &[u8],
    width: u32,
    height: u32,
    stride: i32,
    format: VideoFormat,
    max_edge: u32,
    want_preview: bool,
) -> Option<(u32, u32, Vec<u8>, Option<(u32, u32, Vec<u8>)>)> {
    let (bpp, kind) = pixel_kind(format)?;
    let stride = if stride > 0 {
        stride as usize
    } else {
        width as usize * bpp
    };
    let (dw, dh) = even_fit(width, height, max_edge);
    if dw < 2 || dh < 2 {
        return None;
    }
    let preview_edge = 1280u32.min(max_edge.max(2));
    let (pw, ph) = even_fit(width, height, preview_edge);
    match kind {
        Pixel::I420 => {
            // Already planar — NN-scale I420 directly when size changes.
            let i420 = scale_i420_planes(src, width, height, dw, dh)?;
            let preview = if want_preview && pw >= 2 && ph >= 2 {
                let mut rgba = vec![0u8; (pw as usize) * (ph as usize) * 4];
                sample_i420(src, width, height, pw, ph, &mut rgba)?;
                Some((pw, ph, rgba))
            } else {
                None
            };
            Some((dw, dh, i420, preview))
        }
        Pixel::Nv12 => {
            let i420 = scale_nv12_to_i420(src, width, height, stride, dw, dh)?;
            let preview = if want_preview && pw >= 2 && ph >= 2 {
                // Preview from the converted I420 (small); avoids a second NV12 walk.
                let mut rgba = vec![0u8; (pw as usize) * (ph as usize) * 4];
                sample_i420(&i420, dw, dh, pw, ph, &mut rgba)?;
                Some((pw, ph, rgba))
            } else {
                None
            };
            Some((dw, dh, i420, preview))
        }
        Pixel::Packed { swap_rb } => {
            if stride < width as usize * bpp {
                return None;
            }
            if src.len() < stride.saturating_mul(height as usize).max(width as usize * bpp)
                && src.len() < width as usize * bpp * height as usize
            {
                return None;
            }
            let order = match (bpp, swap_rb) {
                (4, true) => PackedOrder::Bgra,
                (4, false) => PackedOrder::Rgba,
                (3, true) => PackedOrder::Bgr,
                (3, false) => PackedOrder::Rgb,
                _ => return None,
            };
            let i420 = encode::scale_packed_to_i420(
                src, width, height, stride, order, dw, dh,
            )?;
            let preview = if want_preview && pw >= 2 && ph >= 2 {
                let mut rgba = vec![0u8; (pw as usize) * (ph as usize) * 4];
                for row in 0..ph {
                    let src_row = (row as u64 * height as u64 / ph as u64) as usize;
                    let row_bytes = src.get(src_row * stride..)?;
                    for col in 0..pw {
                        let src_col = (col as u64 * width as u64 / pw as u64) as usize;
                        let pixel = row_bytes.get(src_col * bpp..)?.get(..bpp)?;
                        let at = ((row * pw + col) * 4) as usize;
                        write_pixel(&mut rgba[at..at + 4], pixel, bpp, swap_rb);
                    }
                }
                Some((pw, ph, rgba))
            } else {
                None
            };
            Some((dw, dh, i420, preview))
        }
    }
}

fn scale_i420_planes(src: &[u8], width: u32, height: u32, dw: u32, dh: u32) -> Option<Vec<u8>> {
    let sw = width as usize;
    let sh = height as usize;
    let dw = dw as usize;
    let dh = dh as usize;
    let y_len = sw * sh;
    let uv_w = sw / 2;
    let uv_h = sh / 2;
    let uv_len = uv_w * uv_h;
    if src.len() < y_len + uv_len * 2 || uv_w == 0 || uv_h == 0 {
        return None;
    }
    if dw == sw && dh == sh {
        return Some(src[..y_len + uv_len * 2].to_vec());
    }
    let y = &src[..y_len];
    let u = &src[y_len..y_len + uv_len];
    let v = &src[y_len + uv_len..y_len + uv_len * 2];
    let out_y_len = dw * dh;
    let out_uv_w = dw / 2;
    let out_uv_len = out_uv_w * (dh / 2);
    let mut out = vec![0u8; out_y_len + out_uv_len * 2];
    for row in 0..dh {
        let src_row = (row as u64 * sh as u64 / dh as u64) as usize;
        for col in 0..dw {
            let src_col = (col as u64 * sw as u64 / dw as u64) as usize;
            out[row * dw + col] = y[src_row * sw + src_col];
        }
    }
    for row in 0..(dh / 2) {
        let src_row = (row as u64 * uv_h as u64 / (dh / 2) as u64) as usize;
        for col in 0..out_uv_w {
            let src_col = (col as u64 * uv_w as u64 / out_uv_w as u64) as usize;
            let src = src_row * uv_w + src_col;
            let dst = out_y_len + row * out_uv_w + col;
            out[dst] = u[src];
            out[dst + out_uv_len] = v[src];
        }
    }
    Some(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pixel {
    Packed { swap_rb: bool },
    I420,
    Nv12,
}

fn format_name(format: VideoFormat) -> &'static str {
    if format == VideoFormat::I420 {
        "I420"
    } else if format == VideoFormat::NV12 {
        "NV12"
    } else if format == VideoFormat::BGRA {
        "BGRA"
    } else if format == VideoFormat::BGRx {
        "BGRx"
    } else if format == VideoFormat::RGBA {
        "RGBA"
    } else if format == VideoFormat::RGBx {
        "RGBx"
    } else if format == VideoFormat::RGB {
        "RGB"
    } else if format == VideoFormat::BGR {
        "BGR"
    } else {
        "unknown"
    }
}

fn pixel_kind(format: VideoFormat) -> Option<(usize, Pixel)> {
    if format == VideoFormat::RGB || format == VideoFormat::BGR {
        return Some((
            3,
            Pixel::Packed {
                swap_rb: format == VideoFormat::BGR,
            },
        ));
    }
    if format == VideoFormat::RGBA
        || format == VideoFormat::RGBx
        || format == VideoFormat::BGRA
        || format == VideoFormat::BGRx
    {
        return Some((
            4,
            Pixel::Packed {
                swap_rb: format == VideoFormat::BGRA || format == VideoFormat::BGRx,
            },
        ));
    }
    if format == VideoFormat::I420 {
        return Some((1, Pixel::I420));
    }
    if format == VideoFormat::NV12 {
        return Some((1, Pixel::Nv12));
    }
    None
}

/// Semi-planar NV12 → planar I420 with optional NN scale (OBS-style prefer YUV).
fn scale_nv12_to_i420(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    dw: u32,
    dh: u32,
) -> Option<Vec<u8>> {
    let sw = width as usize;
    let sh = height as usize;
    let dw = dw as usize;
    let dh = dh as usize;
    if sw < 2 || sh < 2 || dw < 2 || dh < 2 || (sw & 1) != 0 || (sh & 1) != 0 {
        return None;
    }
    let y_stride = stride.max(sw);
    let y_plane_size = y_stride.saturating_mul(sh);
    let uv_stride = y_stride;
    let uv_plane_size = uv_stride.saturating_mul(sh / 2);
    if src.len() < y_plane_size + uv_plane_size {
        return None;
    }
    let y = &src[..y_plane_size];
    let uv = &src[y_plane_size..y_plane_size + uv_plane_size];
    let out_y_len = dw * dh;
    let out_uv_w = dw / 2;
    let out_uv_len = out_uv_w * (dh / 2);
    let mut out = vec![0u8; out_y_len + out_uv_len * 2];
    for row in 0..dh {
        let src_row = (row as u64 * sh as u64 / dh as u64) as usize;
        let row_base = src_row * y_stride;
        for col in 0..dw {
            let src_col = (col as u64 * sw as u64 / dw as u64) as usize;
            out[row * dw + col] = *y.get(row_base + src_col)?;
        }
    }
    for row in 0..(dh / 2) {
        let src_row = (row as u64 * (sh / 2) as u64 / (dh / 2) as u64) as usize;
        let row_base = src_row * uv_stride;
        for col in 0..out_uv_w {
            let src_col = (col as u64 * (sw / 2) as u64 / out_uv_w as u64) as usize;
            let at = row_base + src_col * 2;
            let dst = out_y_len + row * out_uv_w + col;
            out[dst] = *uv.get(at)?;
            out[dst + out_uv_len] = *uv.get(at + 1)?;
        }
    }
    Some(out)
}

fn write_pixel(dst: &mut [u8], src: &[u8], bpp: usize, swap_rb: bool) {
    if bpp == 3 {
        if swap_rb {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
        } else {
            dst[0] = src[0];
            dst[1] = src[1];
            dst[2] = src[2];
        }
        dst[3] = 255;
        return;
    }
    if swap_rb {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
    } else {
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
    }
    dst[3] = 255;
}

fn sample_i420(
    src: &[u8],
    width: u32,
    height: u32,
    dw: u32,
    dh: u32,
    rgba: &mut [u8],
) -> Option<()> {
    let width = width as usize;
    let height = height as usize;
    let y_len = width * height;
    let uv_w = width / 2;
    let uv_h = height / 2;
    let uv_len = uv_w * uv_h;
    if src.len() < y_len + uv_len * 2 || uv_w == 0 || uv_h == 0 {
        return None;
    }
    let y = &src[..y_len];
    let u = &src[y_len..y_len + uv_len];
    let v = &src[y_len + uv_len..y_len + uv_len * 2];
    for row in 0..dh {
        let src_row = (row as u64 * height as u64 / dh as u64) as usize;
        for col in 0..dw {
            let src_col = (col as u64 * width as u64 / dw as u64) as usize;
            let uv = (src_row / 2) * uv_w + src_col / 2;
            let (r, g, b) = yuv_rgb(y[src_row * width + src_col], u[uv], v[uv]);
            let at = ((row * dw + col) * 4) as usize;
            rgba[at] = r;
            rgba[at + 1] = g;
            rgba[at + 2] = b;
            rgba[at + 3] = 255;
        }
    }
    Some(())
}

fn yuv_rgb(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let y = y as i32;
    let u = u as i32 - 128;
    let v = v as i32 - 128;
    let r = (y + (359 * v) / 256).clamp(0, 255) as u8;
    let g = (y - (88 * u + 183 * v) / 256).clamp(0, 255) as u8;
    let b = (y + (454 * u) / 256).clamp(0, 255) as u8;
    (r, g, b)
}

fn token() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let tick = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = Instant::now().elapsed().as_nanos();
    format!("banter{tick:x}{nanos:x}")
}

struct Portal {
    proxy: Proxy<'static>,
    session: Option<OwnedObjectPath>,
}

impl Drop for Portal {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        // Close before the bus connection drops so the compositor releases the cast
        // for the next Go Live instead of racing a stale session.
        let path = session.as_str().to_string();
        {
            let Ok(session_proxy) = Proxy::new(
                self.proxy.connection(),
                "org.freedesktop.portal.Desktop",
                path.as_str(),
                "org.freedesktop.portal.Session",
            ) else {
                return;
            };
            let _ = session_proxy.call_method("Close", &());
        }
    }
}

#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
struct StartResponse {
    streams: Option<Vec<(u32, HashMap<String, OwnedValue>)>>,
}

impl Portal {
    fn connect() -> Result<Self, String> {
        let conn = zbus::blocking::Connection::session().map_err(|err| err.to_string())?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.ScreenCast",
        )
        .map_err(|err| err.to_string())?;
        Ok(Self {
            proxy,
            session: None,
        })
    }

    fn attach_session(&mut self, session: OwnedObjectPath) {
        self.session = Some(session);
    }

    fn create_session(&self) -> Result<OwnedObjectPath, String> {
        let session_token = token();
        let handle = token();
        let request = portal_request(self.proxy.connection(), &handle)?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(handle.as_str()));
        options.insert("session_handle_token", Value::from(session_token.as_str()));
        let incoming = request
            .receive_signal("Response")
            .map_err(|err| err.to_string())?;
        self.proxy
            .call_method("CreateSession", &(options))
            .map_err(|err| err.to_string())?;
        let results: HashMap<String, OwnedValue> = read_response(incoming)?;
        let session_handle = dbus_string(results.get("session_handle"))
            .ok_or_else(|| "screen cast did not return a session".to_string())?;
        let unique = self
            .proxy
            .connection()
            .unique_name()
            .ok_or_else(|| "no dbus name".to_string())?;
        let unique = unique.trim_start_matches(':').replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/session/{unique}/{session_token}");
        if path != session_handle {
            return Err("screen cast session mismatch".into());
        }
        OwnedObjectPath::try_from(path).map_err(|err| err.to_string())
    }

    fn select_sources(&self, session: &OwnedObjectPath) -> Result<(), String> {
        let handle = token();
        let request = portal_request(self.proxy.connection(), &handle)?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(handle.as_str()));
        options.insert("types", Value::from(3_u32));
        options.insert("multiple", Value::from(false));
        options.insert("cursor_mode", Value::from(2_u32));
        options.insert("persist_mode", Value::from(0_u32));
        let incoming = request
            .receive_signal("Response")
            .map_err(|err| err.to_string())?;
        self.proxy
            .call_method("SelectSources", &(session, options))
            .map_err(|err| err.to_string())?;
        let _: HashMap<String, OwnedValue> = read_response(incoming)?;
        Ok(())
    }

    fn start(&self, session: &OwnedObjectPath) -> Result<StartResponse, String> {
        let handle = token();
        let request = portal_request(self.proxy.connection(), &handle)?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(handle.as_str()));
        let incoming = request
            .receive_signal("Response")
            .map_err(|err| err.to_string())?;
        self.proxy
            .call_method("Start", &(session, "", options))
            .map_err(|err| err.to_string())?;
        read_response(incoming)
    }

    fn pipewire_fd(&self, session: &OwnedObjectPath) -> Result<OwnedFd, String> {
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let fd: zbus::zvariant::OwnedFd = self
            .proxy
            .call("OpenPipeWireRemote", &(session, options))
            .map_err(|err| err.to_string())?;
        Ok(OwnedFd::from(fd))
    }
}

/// A portal dict value is either a string or an object path, depending on the desktop.
fn dbus_string(value: Option<&OwnedValue>) -> Option<String> {
    let value = value?;
    <&str>::try_from(value)
        .ok()
        .map(str::to_string)
        .or_else(|| {
            <&zbus::zvariant::ObjectPath>::try_from(value)
                .ok()
                .map(|path| path.to_string())
        })
}

fn portal_request(
    conn: &zbus::blocking::Connection,
    handle: &str,
) -> Result<Proxy<'static>, String> {
    let unique = conn
        .unique_name()
        .ok_or_else(|| "no dbus name".to_string())?;
    let unique = unique.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{handle}");
    Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        path,
        "org.freedesktop.portal.Request",
    )
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgrx_frame_is_scaled_and_swapped() {
        let mut src = Vec::new();
        for index in 0..4u8 {
            src.extend_from_slice(&[index, 10, 200, 0]);
        }
        let frame = sample_frame(&src, 2, 2, 8, VideoFormat::BGRx, 2).expect("frame");
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(&frame.rgba[..4], &[200, 10, 0, 255]);
    }

    #[test]
    fn bgrx_encode_path_emits_i420_without_rgba_middle() {
        let mut src = Vec::new();
        for index in 0..16u8 {
            src.extend_from_slice(&[index, 20, 180, 0]);
        }
        let (w, h, i420, preview) =
            sample_encode_frame(&src, 4, 4, 16, VideoFormat::BGRx, 4, true).expect("encode");
        assert_eq!((w, h), (4, 4));
        assert_eq!(i420.len(), 4 * 4 + 2 * (2 * 2));
        let (pw, ph, rgba) = preview.expect("preview");
        assert_eq!((pw, ph), (4, 4));
        assert_eq!(rgba.len(), 4 * 4 * 4);
        // BGRX → R from blue channel (180).
        assert_eq!(rgba[0], 180);
    }

    #[test]
    fn format_name_labels_common_spa_ids() {
        assert_eq!(format_name(VideoFormat::BGRA), "BGRA");
        assert_eq!(format_name(VideoFormat::NV12), "NV12");
        assert_eq!(format_name(VideoFormat::I420), "I420");
        // SPA raw.h: BGRA is enum value 12 (UNKNOWN=0 … BGRA=12).
        assert_eq!(VideoFormat::BGRA.0, 12);
    }

    #[test]
    fn nv12_encode_path_splits_to_planar_i420() {
        // 4x4 NV12: Y plane then interleaved UV.
        let mut src = vec![40u8; 4 * 4];
        src.extend_from_slice(&[128, 140, 128, 140, 128, 140, 128, 140]);
        let (w, h, i420, preview) =
            sample_encode_frame(&src, 4, 4, 4, VideoFormat::NV12, 4, true).expect("nv12");
        assert_eq!((w, h), (4, 4));
        assert_eq!(i420.len(), 4 * 4 + 2 * (2 * 2));
        assert_eq!(&i420[..16], &[40u8; 16]);
        assert_eq!(&i420[16..20], &[128, 128, 128, 128]);
        assert_eq!(&i420[20..24], &[140, 140, 140, 140]);
        assert!(preview.is_some());
    }
}

fn read_response<T>(mut incoming: zbus::blocking::proxy::SignalIterator<'_>) -> Result<T, String>
where
    T: for<'de> serde::Deserialize<'de> + Type,
{
    let message = incoming
        .next()
        .ok_or_else(|| "screen share dialog closed".to_string())?;
    let (code, body): (u32, T) = message
        .body()
        .deserialize()
        .map_err(|err| err.to_string())?;
    match code {
        0 => Ok(body),
        1 => Err("cancelled".into()),
        other => Err(format!("screen cast failed ({other})")),
    }
}
