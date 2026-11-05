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

use pipewire::{
    channel, context::ContextRc, keys, main_loop::MainLoopRc, properties,
    spa::{
        buffer::{ChunkFlags, DataType},
        param::{
            ParamType,
            format::{FormatProperties, MediaSubtype, MediaType},
            format_utils,
            video::{VideoFormat, VideoInfoRaw},
        },
        pod::{self, Pod, serialize::PodSerializer},
        utils::{Direction, Fraction, Rectangle, SpaTypes},
    },
    stream::{StreamFlags, StreamRc, StreamState},
};
use zbus::blocking::Proxy;
use zbus::zvariant::{DeserializeDict, OwnedObjectPath, OwnedValue, Type, Value};

use super::encode::even_fit;

struct Latest {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Unscaled copy of one PipeWire buffer. The screen thread scales it.
struct RawFrame {
    width: u32,
    height: u32,
    stride: i32,
    format: VideoFormat,
    bytes: Vec<u8>,
}

struct StreamData {
    format: VideoInfoRaw,
    logged: bool,
}

pub struct CastStream {
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<String>>>,
    quit: Option<channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// The portal closes the cast when this bus connection drops.
    _portal: Portal,
}

impl CastStream {
    /// Opens the system share dialog and starts the PipeWire stream.
    /// Blocks until the dialog is confirmed or dismissed.
    pub fn open(max_edge: u32, fps: u32) -> Result<Self, String> {
        let cast = Portal::connect()?;
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
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(Mutex::new(None));
        let (quit_tx, quit_rx) = channel::channel();
        let latest_thread = Arc::clone(&latest);
        let stop_thread = Arc::clone(&stop);
        let fault_thread = Arc::clone(&fault);
        let thread = thread::Builder::new()
            .name("banter-screencast".into())
            .spawn(move || stream_loop(fd, node, max_edge, fps.max(1), latest_thread, stop_thread, fault_thread, quit_rx))
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

    /// Takes the newest raw frame and scales it to `edge` on this thread.
    pub fn take_picture(&self, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
        let raw = self.latest.lock().expect("screencast frame").take()?;
        let frame = sample_frame(&raw.bytes, raw.width, raw.height, raw.stride, raw.format, edge)?;
        Some((frame.width, frame.height, frame.rgba))
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

    let data = StreamData { format: VideoInfoRaw::new(), logged: false };
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
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else { return };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            if data.format.parse(param).is_err() {
                return;
            }
            if !data.logged {
                data.logged = true;
                let size = data.format.size();
                eprintln!("banter: screencast format {} {}x{}", data.format.format().0, size.width, size.height);
            }
        })
        .process(move |stream, data| {
            if stop_loop.load(Ordering::Relaxed) {
                callback_loop.quit();
                return;
            }
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            let Some(chunk_src) = datas.first_mut() else { return };
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
            let frame = if let Some(bytes) = chunk_src.data() {
                copy_frame(bytes, offset, chunk_size, stride, &data.format)
            } else if let Some(mapped) = map_memfd(chunk_src) {
                copy_frame(mapped.bytes(), offset, chunk_size, stride, &data.format)
            } else {
                None
            };
            if let Some(frame) = frame {
                *latest.lock().expect("screencast frame") = Some(frame);
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
    if let Err(err) = stream.connect(Direction::Input, Some(node), StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS, &mut params) {
        fail(&fault, format!("screencast start: {err}"));
        return;
    }
    let started = Instant::now();
    main_loop.run();
    drop(listener);
    drop(stream);
    if !stop.load(Ordering::Relaxed) && started.elapsed() < Duration::from_millis(200) {
        fail(&fault, "screencast ended before the first frame".to_string());
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

fn copy_frame(bytes: &[u8], offset: usize, chunk_size: usize, stride: i32, format: &VideoInfoRaw) -> Option<RawFrame> {
    let size = if chunk_size == 0 { bytes.len().saturating_sub(offset) } else { chunk_size };
    let end = offset.saturating_add(size).min(bytes.len());
    if end <= offset {
        return None;
    }
    let dim = format.size();
    if dim.width < 2 || dim.height < 2 {
        return None;
    }
    Some(RawFrame {
        width: dim.width,
        height: dim.height,
        stride,
        format: format.format(),
        bytes: bytes[offset..end].to_vec(),
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
    let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd, raw.mapoffset as i64) };
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
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRA,
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
            Rectangle { width: edge, height },
            Rectangle { width: 2, height: 2 },
            Rectangle { width: 4096, height: 4096 }
        ),
        // KWin offers 0/1 here, meaning the compositor picks the rate. A minimum of 1 rejects that offer.
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: fps, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: fps.max(1), denom: 1 }
        ),
    );
    let (cursor, _) = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &pod::Value::Object(obj)).map_err(|err| err.to_string())?;
    Ok(cursor.into_inner())
}

fn sample_frame(src: &[u8], width: u32, height: u32, stride: i32, format: VideoFormat, max_edge: u32) -> Option<Latest> {
    let (bpp, kind) = pixel_kind(format)?;
    let stride = if stride > 0 { stride as usize } else { width as usize * bpp };
    if src.len() < stride.saturating_mul(height as usize).max(width as usize * bpp) && kind != Pixel::I420 {
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
    Some(Latest { width: dw, height: dh, rgba })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pixel {
    Packed { swap_rb: bool },
    I420,
}

fn pixel_kind(format: VideoFormat) -> Option<(usize, Pixel)> {
    if format == VideoFormat::RGB || format == VideoFormat::BGR {
        return Some((3, Pixel::Packed { swap_rb: format == VideoFormat::BGR }));
    }
    if format == VideoFormat::RGBA || format == VideoFormat::RGBx || format == VideoFormat::BGRA || format == VideoFormat::BGRx {
        return Some((4, Pixel::Packed { swap_rb: format == VideoFormat::BGRA || format == VideoFormat::BGRx }));
    }
    if format == VideoFormat::I420 {
        return Some((1, Pixel::I420));
    }
    None
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

fn sample_i420(src: &[u8], width: u32, height: u32, dw: u32, dh: u32, rgba: &mut [u8]) -> Option<()> {
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
}

#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
struct StartResponse {
    streams: Option<Vec<(u32, HashMap<String, OwnedValue>)>>,
}

impl Portal {
    fn connect() -> Result<Self, String> {
        let conn = zbus::blocking::Connection::session().map_err(|err| err.to_string())?;
        let proxy = Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.ScreenCast")
            .map_err(|err| err.to_string())?;
        Ok(Self { proxy })
    }

    fn create_session(&self) -> Result<OwnedObjectPath, String> {
        let session_token = token();
        let handle = token();
        let request = portal_request(self.proxy.connection(), &handle)?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(handle.as_str()));
        options.insert("session_handle_token", Value::from(session_token.as_str()));
        let incoming = request.receive_signal("Response").map_err(|err| err.to_string())?;
        self.proxy.call_method("CreateSession", &(options)).map_err(|err| err.to_string())?;
        let results: HashMap<String, OwnedValue> = read_response(incoming)?;
        let session_handle = dbus_string(results.get("session_handle")).ok_or_else(|| "screen cast did not return a session".to_string())?;
        let unique = self.proxy.connection().unique_name().ok_or_else(|| "no dbus name".to_string())?;
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
        let incoming = request.receive_signal("Response").map_err(|err| err.to_string())?;
        self.proxy.call_method("SelectSources", &(session, options)).map_err(|err| err.to_string())?;
        let _: HashMap<String, OwnedValue> = read_response(incoming)?;
        Ok(())
    }

    fn start(&self, session: &OwnedObjectPath) -> Result<StartResponse, String> {
        let handle = token();
        let request = portal_request(self.proxy.connection(), &handle)?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(handle.as_str()));
        let incoming = request.receive_signal("Response").map_err(|err| err.to_string())?;
        self.proxy.call_method("Start", &(session, "", options)).map_err(|err| err.to_string())?;
        read_response(incoming)
    }

    fn pipewire_fd(&self, session: &OwnedObjectPath) -> Result<OwnedFd, String> {
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let fd: zbus::zvariant::OwnedFd = self.proxy.call("OpenPipeWireRemote", &(session, options)).map_err(|err| err.to_string())?;
        Ok(OwnedFd::from(fd))
    }
}

/// A portal dict value is either a string or an object path, depending on the desktop.
fn dbus_string(value: Option<&OwnedValue>) -> Option<String> {
    let value = value?;
    <&str>::try_from(value).ok().map(str::to_string).or_else(|| <&zbus::zvariant::ObjectPath>::try_from(value).ok().map(|path| path.to_string()))
}

fn portal_request(conn: &zbus::blocking::Connection, handle: &str) -> Result<Proxy<'static>, String> {
    let unique = conn.unique_name().ok_or_else(|| "no dbus name".to_string())?;
    let unique = unique.trim_start_matches(':').replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{handle}");
    Proxy::new(conn, "org.freedesktop.portal.Desktop", path, "org.freedesktop.portal.Request").map_err(|err| err.to_string())
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
}

fn read_response<T>(mut incoming: zbus::blocking::proxy::SignalIterator<'_>) -> Result<T, String>
where
    T: for<'de> serde::Deserialize<'de> + Type,
{
    let message = incoming.next().ok_or_else(|| "screen share dialog closed".to_string())?;
    let (code, body): (u32, T) = message.body().deserialize().map_err(|err| err.to_string())?;
    match code {
        0 => Ok(body),
        1 => Err("cancelled".into()),
        other => Err(format!("screen cast failed ({other})")),
    }
}
