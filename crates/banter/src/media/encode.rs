//! Opus and H.264 Constrained Baseline.
//!
//! Hardware is probed and used when it emits that profile. OpenH264 is the
//! software path on both operating systems. A hardware encoder that answers
//! with a different profile is dropped for that layer.

#[cfg(target_os = "linux")]
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "linux")]
use std::thread::{self, JoinHandle};
#[cfg(target_os = "linux")]
use std::time::Duration;

use banter_server::LayerId;
use openh264::OpenH264API;
use openh264::decoder::Decoder;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, Profile, UsageType};
use openh264::formats::{YUVBuffer, YUVSource};

pub const OPUS_RATE: u32 = 48_000;
pub const OPUS_FRAME: usize = 960;
const USER_AUDIO_BPS: i32 = 32_000;
const STREAM_AUDIO_BPS: i32 = 64_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Software,
    #[cfg(target_os = "linux")]
    Nvenc,
    #[cfg(target_os = "linux")]
    Vaapi,
    #[cfg_attr(not(windows), allow(dead_code))]
    MediaFoundation,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Software => "openh264",
            #[cfg(target_os = "linux")]
            Self::Nvenc => "nvenc",
            #[cfg(target_os = "linux")]
            Self::Vaapi => "vaapi",
            Self::MediaFoundation => "media foundation",
        }
    }
}

pub struct EncodedAu {
    pub bytes: Vec<u8>,
    pub keyframe: bool,
}

pub struct OpusTrack {
    encoder: opus::Encoder,
    full_bps: i32,
}

impl OpusTrack {
    pub fn user_audio() -> Result<Self, String> {
        let mut encoder = opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Voip).map_err(|err| err.to_string())?;
        encoder.set_bitrate(opus::Bitrate::Bits(USER_AUDIO_BPS)).map_err(|err| err.to_string())?;
        encoder.set_inband_fec(true).map_err(|err| err.to_string())?;
        encoder.set_dtx(true).map_err(|err| err.to_string())?;
        Ok(Self { encoder, full_bps: USER_AUDIO_BPS })
    }

    pub fn stream_audio() -> Result<Self, String> {
        let mut encoder = opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Audio).map_err(|err| err.to_string())?;
        encoder.set_bitrate(opus::Bitrate::Bits(STREAM_AUDIO_BPS)).map_err(|err| err.to_string())?;
        encoder.set_dtx(true).map_err(|err| err.to_string())?;
        Ok(Self { encoder, full_bps: STREAM_AUDIO_BPS })
    }

    /// `ratio` is 1, 0.5, or 0. Zero means the ladder cut the track.
    pub fn set_ratio(&mut self, ratio: f32) -> Result<(), String> {
        let bps = if ratio <= 0.0 { 6_000 } else { (self.full_bps as f32 * ratio).round() as i32 };
        self.encoder.set_bitrate(opus::Bitrate::Bits(bps.max(6_000))).map_err(|err| err.to_string())
    }

    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String> {
        let mut packet = vec![0u8; 4000];
        let size = self.encoder.encode(pcm, &mut packet).map_err(|err| err.to_string())?;
        packet.truncate(size);
        Ok(packet)
    }
}

pub struct OpusListen {
    decoder: opus::Decoder,
}

impl OpusListen {
    pub fn new() -> Result<Self, String> {
        let decoder = opus::Decoder::new(OPUS_RATE, opus::Channels::Mono).map_err(|err| err.to_string())?;
        Ok(Self { decoder })
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>, String> {
        let mut pcm = vec![0i16; OPUS_FRAME * 3];
        let count = self.decoder.decode(packet, &mut pcm, false).map_err(|err| err.to_string())?;
        pcm.truncate(count);
        Ok(pcm)
    }
}

pub trait VideoEncoder: Send {
    fn backend(&self) -> Backend;
    fn encode(&mut self, i420: &[u8], width: u32, height: u32, force_key: bool) -> Result<EncodedAu, String>;
}

struct SoftwareEncoder {
    encoder: Encoder,
}

impl SoftwareEncoder {
    fn open(screen: bool, fps: f32, bps: u32) -> Result<Self, String> {
        Ok(Self {
            encoder: build_openh264(screen, fps, bps)?,
        })
    }
}

impl VideoEncoder for SoftwareEncoder {
    fn backend(&self) -> Backend {
        Backend::Software
    }

    fn encode(&mut self, i420: &[u8], width: u32, height: u32, force_key: bool) -> Result<EncodedAu, String> {
        if force_key {
            self.encoder.force_intra_frame();
        }
        let yuv = YUVBuffer::from_vec(i420.to_vec(), width as usize, height as usize);
        let stream = self.encoder.encode(&yuv).map_err(|err| err.to_string())?;
        let mut bytes = stream.to_vec();
        mark_constrained_baseline(&mut bytes);
        Ok(EncodedAu {
            keyframe: access_unit_is_key(&bytes),
            bytes,
        })
    }
}

/// Picks a hardware encoder when the probe finds one, and replaces it with
/// OpenH264 if the first access unit is not Constrained Baseline.
pub struct LayerEncoder {
    inner: Box<dyn VideoEncoder>,
    fallback: Option<SoftwareEncoder>,
}

impl LayerEncoder {
    pub fn open(screen: bool, width: u32, height: u32, fps: f32, bps: u32) -> Result<Self, String> {
        let soft = SoftwareEncoder::open(screen, fps, bps)?;
        let hw = try_hardware(screen, width, height, fps, bps);
        if let Some(inner) = hw {
            Ok(Self { inner, fallback: Some(soft) })
        } else {
            Ok(Self { inner: Box::new(soft), fallback: None })
        }
    }

    pub fn software(screen: bool, fps: f32, bps: u32) -> Result<Self, String> {
        Ok(Self {
            inner: Box::new(SoftwareEncoder::open(screen, fps, bps)?),
            fallback: None,
        })
    }

    pub fn backend(&self) -> Backend {
        self.inner.backend()
    }

    pub fn encode(&mut self, i420: &[u8], width: u32, height: u32, force_key: bool) -> Result<EncodedAu, String> {
        match self.inner.encode(i420, width, height, force_key) {
            Ok(au) if is_constrained_baseline(&au.bytes) || self.fallback.is_none() => Ok(au),
            Ok(_) | Err(_) => {
                if let Some(soft) = self.fallback.take() {
                    self.inner = Box::new(soft);
                    self.inner.encode(i420, width, height, true)
                } else {
                    self.inner.encode(i420, width, height, force_key)
                }
            }
        }
    }
}

pub struct VideoListen {
    decoder: Decoder,
}

impl VideoListen {
    pub fn software() -> Result<Self, String> {
        let decoder = Decoder::with_api_config(OpenH264API::from_source(), Default::default()).map_err(|err| err.to_string())?;
        Ok(Self { decoder })
    }

    pub fn decode_rgba(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        let Some(yuv) = self.decoder.decode(annexb).map_err(|err| err.to_string())? else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();
        let mut rgba = vec![0u8; width * height * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some((width as u32, height as u32, rgba)))
    }
}

/// Names the hardware encoders this process can see. Empty means OpenH264 only.
pub fn hardware_probe() -> Vec<&'static str> {
    let mut found = Vec::new();
    #[cfg(target_os = "linux")]
    {
        if ffmpeg_has_encoder("h264_nvenc") && nvidia_encode_present() {
            found.push("nvenc");
        }
        if ffmpeg_has_encoder("h264_vaapi") && std::path::Path::new("/dev/dri/renderD128").exists() {
            found.push("vaapi");
        }
    }
    #[cfg(windows)]
    {
        if super::mf::hardware_present() {
            found.push("media foundation");
        }
    }
    found
}

pub fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut y = vec![0u8; width * height];
    let mut u = vec![0u8; (width / 2) * (height / 2)];
    let mut v = vec![0u8; (width / 2) * (height / 2)];
    for row in 0..height {
        for col in 0..width {
            let index = (row * width + col) * 4;
            let (yy, uu, vv) = yuv_of(rgba[index], rgba[index + 1], rgba[index + 2]);
            y[row * width + col] = yy;
            if row % 2 == 0 && col % 2 == 0 {
                let uv = (row / 2) * (width / 2) + col / 2;
                u[uv] = uu;
                v[uv] = vv;
            }
        }
    }
    y.extend(u);
    y.extend(v);
    y
}

#[allow(dead_code)]
pub fn rgb_to_i420(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut y = vec![0u8; width * height];
    let mut u = vec![0u8; (width / 2) * (height / 2)];
    let mut v = vec![0u8; (width / 2) * (height / 2)];
    for row in 0..height {
        for col in 0..width {
            let index = (row * width + col) * 3;
            let (yy, uu, vv) = yuv_of(rgb[index], rgb[index + 1], rgb[index + 2]);
            y[row * width + col] = yy;
            if row % 2 == 0 && col % 2 == 0 {
                let uv = (row / 2) * (width / 2) + col / 2;
                u[uv] = uu;
                v[uv] = vv;
            }
        }
    }
    y.extend(u);
    y.extend(v);
    y
}

/// Nearest-neighbor scale onto an even size that fits inside `max_edge`.
/// `max_edge == 0` keeps the source size, rounded down to even.
pub fn fit_rgba(rgba: &[u8], width: u32, height: u32, max_edge: u32) -> (Vec<u8>, u32, u32) {
    let (dw, dh) = even_fit(width, height, max_edge);
    let need = (width as usize).saturating_mul(height as usize).saturating_mul(4);
    if dw < 2 || dh < 2 || rgba.len() < need {
        return (Vec::new(), 0, 0);
    }
    if dw == width && dh == height {
        return (rgba[..need].to_vec(), width, height);
    }
    let mut scaled = vec![0u8; (dw as usize) * (dh as usize) * 4];
    for row in 0..dh {
        let src_row = (row as u64 * height as u64 / dh as u64) as u32;
        for col in 0..dw {
            let src_col = (col as u64 * width as u64 / dw as u64) as u32;
            let from = ((src_row * width + src_col) * 4) as usize;
            let to = ((row * dw + col) * 4) as usize;
            scaled[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    (scaled, dw, dh)
}

/// Nearest-neighbor scale onto an even size that fits inside `max_edge`.
pub fn fit_i420_rgba(rgba: &[u8], width: u32, height: u32, max_edge: u32) -> (Vec<u8>, u32, u32) {
    let (scaled, dw, dh) = fit_rgba(rgba, width, height, max_edge);
    if dw < 2 || dh < 2 {
        return (Vec::new(), 0, 0);
    }
    (rgba_to_i420(&scaled, dw, dh), dw, dh)
}

pub fn scale_i420(src: &[u8], width: u32, height: u32, layer: LayerId) -> Option<(Vec<u8>, u32, u32)> {
    let div = match layer {
        LayerId::Full => 1,
        LayerId::Half => 2,
        LayerId::Quarter => 4,
        LayerId::Off => return None,
    };
    let dw = (width / div) & !1;
    let dh = (height / div) & !1;
    if dw < 2 || dh < 2 {
        return None;
    }
    if div == 1 {
        return Some((src.to_vec(), width, height));
    }
    let (y_plane, uv_w, uv_h) = planes(src, width, height)?;
    let dw_u = dw / 2;
    let dh_u = dh / 2;
    let mut y = vec![0u8; (dw * dh) as usize];
    let mut u = vec![0u8; (dw_u * dh_u) as usize];
    let mut v = vec![0u8; (dw_u * dh_u) as usize];
    for row in 0..dh {
        let src_row = row * div;
        for col in 0..dw {
            y[(row * dw + col) as usize] = y_plane.0[(src_row * width + col * div) as usize];
        }
    }
    for row in 0..dh_u {
        let src_row = row * div;
        for col in 0..dw_u {
            let src = (src_row * uv_w + col * div) as usize;
            u[(row * dw_u + col) as usize] = y_plane.1[src];
            v[(row * dw_u + col) as usize] = y_plane.2[src];
        }
    }
    let _ = uv_h;
    y.extend(u);
    y.extend(v);
    Some((y, dw, dh))
}

pub fn is_constrained_baseline(annexb: &[u8]) -> bool {
    nals(annexb).any(|nal| nal.len() >= 4 && nal[0] & 0x1f == 7 && nal[1] == 66 && nal[2] & 0x40 != 0)
}

pub fn access_unit_is_key(annexb: &[u8]) -> bool {
    nals(annexb).any(|nal| !nal.is_empty() && nal[0] & 0x1f == 5)
}

fn build_openh264(screen: bool, fps: f32, bps: u32) -> Result<Encoder, String> {
    let usage = if screen { UsageType::ScreenContentRealTime } else { UsageType::CameraVideoRealTime };
    let config = EncoderConfig::new()
        .bitrate(BitRate::from_bps(bps.max(30_000)))
        .max_frame_rate(FrameRate::from_hz(fps.max(1.0)))
        .profile(Profile::Baseline)
        .usage_type(usage);
    Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|err| err.to_string())
}

/// OpenH264 baseline is CAVLC with no B frames. Set constraint_set1 so the SPS
/// says Constrained Baseline, which is the profile both operating systems decode.
fn mark_constrained_baseline(annexb: &mut [u8]) {
    let bytes = annexb.to_vec();
    let mut offset = 0;
    while let Some((start, nal)) = next_nal(&bytes, offset) {
        let nal_at = start + nal_header_skip(&bytes[start..]);
        if nal.len() >= 4 && nal[0] & 0x1f == 7 && nal[1] == 66 {
            let flag = nal_at + 2;
            if flag < annexb.len() {
                annexb[flag] |= 0x40;
            }
        }
        offset = nal_at + nal.len();
    }
}

fn nals(annexb: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let (start, nal) = next_nal(annexb, offset)?;
        offset = start + nal_header_skip(&annexb[start..]) + nal.len();
        Some(nal)
    })
}

fn next_nal(bytes: &[u8], mut offset: usize) -> Option<(usize, &[u8])> {
    while offset + 3 < bytes.len() {
        let start = if bytes[offset..].starts_with(&[0, 0, 1]) {
            Some(offset)
        } else if bytes[offset..].starts_with(&[0, 0, 0, 1]) {
            Some(offset)
        } else {
            None
        };
        if let Some(start) = start {
            let header = nal_header_skip(&bytes[start..]);
            let nal_start = start + header;
            let mut end = nal_start + 1;
            while end + 3 < bytes.len() && !bytes[end..].starts_with(&[0, 0, 1]) && !bytes[end..].starts_with(&[0, 0, 0, 1]) {
                end += 1;
            }
            if end + 3 >= bytes.len() {
                end = bytes.len();
            }
            return Some((start, &bytes[nal_start..end]));
        }
        offset += 1;
    }
    None
}

fn nal_header_skip(bytes: &[u8]) -> usize {
    if bytes.starts_with(&[0, 0, 0, 1]) { 4 } else { 3 }
}

fn yuv_of(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (y.clamp(0, 255) as u8, u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

pub(crate) fn even_fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (2, 2);
    }
    let edge = if max_edge == 0 { width.max(height) } else { max_edge };
    let (mut dw, mut dh) = if width <= edge && height <= edge {
        (width, height)
    } else {
        let ratio = (edge as f64 / width as f64).min(edge as f64 / height as f64);
        ((width as f64 * ratio).round() as u32, (height as f64 * ratio).round() as u32)
    };
    dw = (dw.max(2)) & !1;
    dh = (dh.max(2)) & !1;
    (dw, dh)
}

fn planes(src: &[u8], width: u32, height: u32) -> Option<((&[u8], &[u8], &[u8]), u32, u32)> {
    let y_len = (width * height) as usize;
    let uv_w = width / 2;
    let uv_h = height / 2;
    let uv_len = (uv_w * uv_h) as usize;
    if src.len() < y_len + uv_len * 2 {
        return None;
    }
    let y = &src[..y_len];
    let u = &src[y_len..y_len + uv_len];
    let v = &src[y_len + uv_len..y_len + uv_len * 2];
    Some(((y, u, v), uv_w, uv_h))
}

fn try_hardware(screen: bool, width: u32, height: u32, fps: f32, bps: u32) -> Option<Box<dyn VideoEncoder>> {
    #[cfg(target_os = "linux")]
    {
        if ffmpeg_has_encoder("h264_nvenc") && nvidia_encode_present() {
            if let Ok(encoder) = FfmpegEncoder::open("h264_nvenc", Backend::Nvenc, screen, width, height, fps, bps) {
                return Some(Box::new(encoder));
            }
        }
        if ffmpeg_has_encoder("h264_vaapi") && std::path::Path::new("/dev/dri/renderD128").exists() {
            if let Ok(encoder) = FfmpegEncoder::open("h264_vaapi", Backend::Vaapi, screen, width, height, fps, bps) {
                return Some(Box::new(encoder));
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = screen;
        if let Ok(encoder) = super::mf::open(width, height, fps, bps) {
            return Some(Box::new(encoder));
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (screen, width, height, fps, bps);
    }
    None
}

/// Linux NVENC/VAAPI path via ffmpeg. Windows uses Media Foundation instead.
#[cfg(target_os = "linux")]
fn ffmpeg_has_encoder(name: &str) -> bool {
    let Ok(output) = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains(name)
}

#[cfg(target_os = "linux")]
fn nvidia_encode_present() -> bool {
    std::path::Path::new("/usr/lib/libnvidia-encode.so").exists()
        || std::path::Path::new("/usr/lib64/libnvidia-encode.so").exists()
        || Command::new("ldconfig").arg("-p").output().ok().is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains("libnvidia-encode"))
}

#[cfg(target_os = "linux")]
struct FfmpegEncoder {
    backend: Backend,
    child: Child,
    stdin: Option<ChildStdin>,
    captured: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
    fps: f32,
    bps: u32,
    screen: bool,
    name: &'static str,
}

#[cfg(target_os = "linux")]
impl FfmpegEncoder {
    fn open(name: &'static str, backend: Backend, screen: bool, width: u32, height: u32, fps: f32, bps: u32) -> Result<Self, String> {
        let mut command = Command::new("ffmpeg");
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-s",
            &format!("{width}x{height}"),
            "-r",
            &format!("{}", fps.max(1.0) as u32),
            "-i",
            "pipe:0",
        ]);
        if name == "h264_vaapi" {
            command.args(["-vaapi_device", "/dev/dri/renderD128", "-vf", "format=nv12,hwupload"]);
        }
        command.args(["-c:v", name, "-bf", "0", "-g", &format!("{}", fps.max(1.0) as u32), "-b:v", &bps.to_string()]);
        if name == "h264_nvenc" {
            command.args(["-profile:v", "baseline", "-preset", "p1", "-tune", "ll"]);
        } else {
            command.args(["-profile:v", "constrained_baseline"]);
        }
        command.args(["-f", "h264", "pipe:1"]);
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| "ffmpeg produced no picture".to_string())?;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let reader = thread::spawn({
            let captured = Arc::clone(&captured);
            move || {
                use std::io::Read;
                let mut stdout = stdout;
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => captured.lock().expect("ffmpeg bytes").extend_from_slice(&buf[..n]),
                    }
                }
            }
        });
        Ok(Self {
            backend,
            child,
            stdin,
            captured,
            reader: Some(reader),
            width,
            height,
            fps,
            bps,
            screen,
            name,
        })
    }
}

#[cfg(target_os = "linux")]
impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(target_os = "linux")]
impl VideoEncoder for FfmpegEncoder {
    fn backend(&self) -> Backend {
        self.backend
    }

    fn encode(&mut self, i420: &[u8], width: u32, height: u32, force_key: bool) -> Result<EncodedAu, String> {
        if force_key || width != self.width || height != self.height {
            let replacement = FfmpegEncoder::open(self.name, self.backend, self.screen, width, height, self.fps, self.bps)?;
            *self = replacement;
        }
        use std::io::Write;
        let stdin = self.stdin.as_mut().ok_or_else(|| "encoder is closed".to_string())?;
        let before = self.captured.lock().expect("ffmpeg bytes").len();
        stdin.write_all(i420).map_err(|err| err.to_string())?;
        stdin.flush().map_err(|err| err.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_millis(800);
        loop {
            let bytes = self.captured.lock().expect("ffmpeg bytes");
            if bytes.len() > before && access_unit_is_key(&bytes[before..]) || bytes.len() > before + 32 && nals(&bytes[before..]).count() >= 2 {
                let au = bytes[before..].to_vec();
                drop(bytes);
                return Ok(EncodedAu { keyframe: access_unit_is_key(&au), bytes: au });
            }
            drop(bytes);
            if std::time::Instant::now() > deadline {
                return Err("hardware encoder produced no access unit".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Receives one H.264 access unit. Hardware is used when the platform decoder
/// accepts the stream. The first failure drops that decoder for the track.
pub struct PictureDecoder {
    hardware: Option<HwDecoder>,
    software: VideoListen,
}

impl PictureDecoder {
    pub fn open() -> Result<Self, String> {
        Ok(Self { hardware: try_hardware_decoder(), software: VideoListen::software()? })
    }

    pub fn decode_rgba(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        let outcome = match self.hardware.as_mut() {
            Some(decoder) => match decoder.decode(annexb) {
                Ok(picture) => Decoded::Ready(picture),
                Err(_) => Decoded::GiveUp,
            },
            None => Decoded::Software,
        };
        match outcome {
            Decoded::Ready(picture) => Ok(picture),
            Decoded::GiveUp => {
                self.hardware = None;
                self.software.decode_rgba(annexb)
            }
            Decoded::Software => self.software.decode_rgba(annexb),
        }
    }
}

enum Decoded {
    Ready(Option<(u32, u32, Vec<u8>)>),
    GiveUp,
    Software,
}

enum HwDecoder {
    #[cfg(target_os = "linux")]
    Ffmpeg(FfmpegDecoder),
    #[cfg(windows)]
    Mf(super::mf::MfDecoder),
}

impl HwDecoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Ffmpeg(decoder) => decoder.decode(annexb),
            #[cfg(windows)]
            Self::Mf(decoder) => decoder.decode(annexb),
            #[cfg(not(any(target_os = "linux", windows)))]
            _ => Err("no hardware decoder".into()),
        }
    }
}

fn try_hardware_decoder() -> Option<HwDecoder> {
    #[cfg(target_os = "linux")]
    {
        if ffmpeg_has("decoders", "h264_cuvid") && nvidia_encode_present() {
            if let Ok(decoder) = FfmpegDecoder::open(true) {
                return Some(HwDecoder::Ffmpeg(decoder));
            }
        }
        if std::path::Path::new("/dev/dri/renderD128").exists() && ffmpeg_has("hwaccels", "vaapi") {
            if let Ok(decoder) = FfmpegDecoder::open(false) {
                return Some(HwDecoder::Ffmpeg(decoder));
            }
        }
    }
    #[cfg(windows)]
    {
        if let Ok(decoder) = super::mf::open_decoder() {
            return Some(HwDecoder::Mf(decoder));
        }
    }
    None
}

#[cfg(target_os = "linux")]
struct FfmpegDecoder {
    child: Child,
    stdin: Option<ChildStdin>,
    captured: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    failed: bool,
    warmed: bool,
}

#[cfg(target_os = "linux")]
impl FfmpegDecoder {
    fn open(nvdec: bool) -> Result<Self, String> {
        let mut command = Command::new("ffmpeg");
        command.args(["-hide_banner", "-loglevel", "error"]);
        if nvdec {
            command.args(["-hwaccel", "cuda", "-c:v", "h264_cuvid"]);
        } else {
            command.args([
                "-hwaccel",
                "vaapi",
                "-hwaccel_device",
                "/dev/dri/renderD128",
                "-hwaccel_output_format",
                "vaapi",
            ]);
        }
        command.args(["-f", "h264", "-i", "pipe:0"]);
        if !nvdec {
            command.args(["-vf", "hwdownload,format=nv12"]);
        }
        command.args(["-f", "image2pipe", "-vcodec", "ppm", "pipe:1"]);
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| "hardware decoder produced no picture".to_string())?;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let reader = thread::spawn({
            let captured = Arc::clone(&captured);
            move || {
                use std::io::Read;
                let mut stdout = stdout;
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => captured.lock().expect("decoder bytes").extend_from_slice(&buf[..n]),
                    }
                }
            }
        });
        Ok(Self { child, stdin, captured, reader: Some(reader), failed: false, warmed: false })
    }

    fn decode(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        if self.failed {
            return Err("hardware decoder already failed".into());
        }
        use std::io::Write;
        let stdin = self.stdin.as_mut().ok_or_else(|| "decoder is closed".to_string())?;
        stdin.write_all(annexb).and_then(|_| stdin.write_all(&[0, 0, 0, 1, 9, 0x10])).and_then(|_| stdin.flush()).map_err(|err| {
            self.failed = true;
            err.to_string()
        })?;
        let wait = if self.warmed { 120 } else { 800 };
        let deadline = std::time::Instant::now() + Duration::from_millis(wait);
        loop {
            if let Some(status) = self.child.try_wait().ok().flatten() {
                self.failed = true;
                return Err(format!("hardware decoder exited {status}"));
            }
            let mut bytes = self.captured.lock().expect("decoder bytes");
            if let Some(picture) = take_ppm(&mut bytes) {
                self.warmed = true;
                return Ok(Some(picture));
            }
            drop(bytes);
            if std::time::Instant::now() > deadline {
                self.failed = true;
                return Err("hardware decoder did not accept the access unit".into());
            }
            thread::sleep(Duration::from_millis(4));
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(target_os = "linux")]
fn take_ppm(bytes: &mut Vec<u8>) -> Option<(u32, u32, Vec<u8>)> {
    let start = bytes.windows(3).position(|mark| mark == b"P6\n" || mark == b"P6 ")?;
    let header = bytes.get(start..)?;
    let rest = header.strip_prefix(b"P6").and_then(|rest| rest.strip_prefix(b"\n").or_else(|| rest.strip_prefix(b"\r\n")))?;
    let (dims, after_dims) = split_line(rest)?;
    let mut dims = dims.split_whitespace();
    let width: u32 = dims.next()?.parse().ok()?;
    let height: u32 = dims.next()?.parse().ok()?;
    let (_, pixels) = split_line(after_dims)?;
    let need = (width as usize).saturating_mul(height as usize).saturating_mul(3);
    if pixels.len() < need || width == 0 || height == 0 {
        return None;
    }
    let mut rgba = Vec::with_capacity(need / 3 * 4);
    for pixel in pixels[..need].chunks_exact(3) {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    let consumed = bytes.len() - pixels.len() + need;
    bytes.drain(..consumed);
    Some((width, height, rgba))
}

#[cfg(target_os = "linux")]
fn split_line(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let end = bytes.iter().position(|byte| *byte == b'\n')?;
    let line = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    Some((line, &bytes[end + 1..]))
}

#[cfg(target_os = "linux")]
fn ffmpeg_has(list: &str, name: &str) -> bool {
    let Ok(output) = Command::new("ffmpeg").args(["-hide_banner", &format!("-{list}")]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_voice_round_trip_keeps_energy() {
        let mut encoder = OpusTrack::user_audio().expect("opus");
        let mut pcm = vec![0i16; OPUS_FRAME];
        for (index, sample) in pcm.iter_mut().enumerate() {
            *sample = ((index as f32 / 12.0).sin() * 8_000.0) as i16;
        }
        let packet = encoder.encode(&pcm).expect("encode");
        assert!(!packet.is_empty());
        let mut decoder = OpusListen::new().expect("decoder");
        let decoded = decoder.decode(&packet).expect("decode");
        let energy: i64 = decoded.iter().map(|sample| sample.abs() as i64).sum();
        assert!(energy > 1_000, "decoded energy {energy}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ppm_frame_becomes_rgba() {
        let mut bytes = b"P6\n2 1\n255\n\x01\x02\x03\x04\x05\x06".to_vec();
        let (width, height, rgba) = take_ppm(&mut bytes).expect("ppm");
        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert!(bytes.is_empty());
    }

    #[test]
    fn openh264_round_trip_is_constrained_baseline() {
        let mut encoder = LayerEncoder::software(false, 30.0, 400_000).expect("encoder");
        let rgba = vec![40u8, 80, 120, 255].repeat(16 * 16);
        let i420 = rgba_to_i420(&rgba, 16, 16);
        let au = encoder.encode(&i420, 16, 16, true).expect("encode");
        assert!(is_constrained_baseline(&au.bytes), "sps was not constrained baseline");
        assert!(au.keyframe);
        let delta = encoder.encode(&i420, 16, 16, false).expect("delta");
        assert!(!delta.bytes.is_empty());
        let again = encoder.encode(&i420, 16, 16, true).expect("forced intra");
        assert!(again.keyframe, "a requested keyframe has to stay an intra frame without rebuilding the encoder");
        let mut decoder = VideoListen::software().expect("decoder");
        let picture = decoder.decode_rgba(&au.bytes).expect("decode").expect("frame");
        assert_eq!(picture.0, 16);
        assert_eq!(picture.1, 16);
        assert_eq!(picture.2.len(), 16 * 16 * 4);
    }

    #[test]
    fn quarter_layer_is_a_quarter_of_the_pixels() {
        let rgba = vec![10u8, 20, 30, 255].repeat(32 * 32);
        let i420 = rgba_to_i420(&rgba, 32, 32);
        let (_scaled, width, height) = scale_i420(&i420, 32, 32, LayerId::Quarter).expect("scale");
        assert_eq!((width, height), (8, 8));
    }
}
