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

use std::borrow::Cow;

use banter_server::LayerId;
use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, QpRange,
    RateControlMode, UsageType,
};
use openh264::formats::{YUVSlices, YUVSource};
use openh264::OpenH264API;
use std::sync::OnceLock;

/// Process-wide hardware encoder probe. ffmpeg / MF enumeration is expensive; cold
/// start and stream-settings saves used to re-run it every time.
static HARDWARE_PROBE: OnceLock<Vec<&'static str>> = OnceLock::new();

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
        let mut encoder =
            opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|err| err.to_string())?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(USER_AUDIO_BPS))
            .map_err(|err| err.to_string())?;
        // Complexity 5 is a common interactive trade-off (default 10 burns CPU for little delay win).
        encoder.set_complexity(5).map_err(|err| err.to_string())?;
        encoder
            .set_inband_fec(true)
            .map_err(|err| err.to_string())?;
        encoder.set_dtx(true).map_err(|err| err.to_string())?;
        Ok(Self {
            encoder,
            full_bps: USER_AUDIO_BPS,
        })
    }

    pub fn stream_audio() -> Result<Self, String> {
        let mut encoder =
            opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Audio)
                .map_err(|err| err.to_string())?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(STREAM_AUDIO_BPS))
            .map_err(|err| err.to_string())?;
        encoder.set_complexity(5).map_err(|err| err.to_string())?;
        encoder.set_dtx(true).map_err(|err| err.to_string())?;
        Ok(Self {
            encoder,
            full_bps: STREAM_AUDIO_BPS,
        })
    }

    /// `ratio` is 1, 0.5, or 0. Zero means the ladder cut the track.
    pub fn set_ratio(&mut self, ratio: f32) -> Result<(), String> {
        let bps = if ratio <= 0.0 {
            6_000
        } else {
            (self.full_bps as f32 * ratio).round() as i32
        };
        self.encoder
            .set_bitrate(opus::Bitrate::Bits(bps.max(6_000)))
            .map_err(|err| err.to_string())
    }

    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String> {
        let mut packet = vec![0u8; 4000];
        let size = self
            .encoder
            .encode(pcm, &mut packet)
            .map_err(|err| err.to_string())?;
        packet.truncate(size);
        Ok(packet)
    }
}

pub struct OpusListen {
    decoder: opus::Decoder,
}

impl OpusListen {
    pub fn new() -> Result<Self, String> {
        let decoder =
            opus::Decoder::new(OPUS_RATE, opus::Channels::Mono).map_err(|err| err.to_string())?;
        Ok(Self { decoder })
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>, String> {
        let mut pcm = vec![0i16; OPUS_FRAME * 3];
        let count = self
            .decoder
            .decode(packet, &mut pcm, false)
            .map_err(|err| err.to_string())?;
        pcm.truncate(count);
        Ok(pcm)
    }
}

pub trait VideoEncoder: Send {
    fn backend(&self) -> Backend;
    fn encode(
        &mut self,
        i420: &[u8],
        width: u32,
        height: u32,
        force_key: bool,
    ) -> Result<EncodedAu, String>;
}

struct SoftwareEncoder {
    encoder: Encoder,
}

impl SoftwareEncoder {
    fn open(screen: bool, fps: f32, bps: u32, keyframe_ms: u64) -> Result<Self, String> {
        Ok(Self {
            encoder: build_openh264(screen, fps, bps, keyframe_ms)?,
        })
    }
}

impl VideoEncoder for SoftwareEncoder {
    fn backend(&self) -> Backend {
        Backend::Software
    }

    fn encode(
        &mut self,
        i420: &[u8],
        width: u32,
        height: u32,
        force_key: bool,
    ) -> Result<EncodedAu, String> {
        if force_key {
            self.encoder.force_intra_frame();
        }
        let width = width as usize;
        let height = height as usize;
        let y_size = width * height;
        let uv = y_size / 4;
        if i420.len() < y_size + uv * 2 {
            return Err("openh264 picture is short".into());
        }
        let yuv = YUVSlices::new(
            (&i420[..y_size], &i420[y_size..y_size + uv], &i420[y_size + uv..y_size + uv * 2]),
            (width, height),
            (width, width / 2, width / 2),
        );
        let stream = self.encoder.encode(&yuv).map_err(|err| err.to_string())?;
        let mut bytes = stream.to_vec();
        mark_constrained_baseline(&mut bytes);
        Ok(EncodedAu {
            keyframe: access_unit_is_key(&bytes),
            bytes,
        })
    }
}

/// Deferred OpenH264 open — built only if hardware fails or emits an unusable AU.
#[derive(Clone, Copy)]
struct SoftParams {
    screen: bool,
    fps: f32,
    bps: u32,
    keyframe_ms: u64,
}

/// Picks a hardware encoder when the probe finds one, and replaces it with
/// OpenH264 if the first access unit is not Constrained Baseline.
pub struct LayerEncoder {
    inner: Box<dyn VideoEncoder>,
    /// Soft params kept until a hardware AU proves unusable; avoids dual-open on
    /// every layer bring-up / resolution cutover (eager-init audit #7).
    soft: Option<SoftParams>,
}

/// How [`LayerEncoder::open_with`] picks hardware vs OpenH264.
///
/// Specific backends (`Nvenc` / `Vaapi` / `MediaFoundation`) only open that
/// encoder. Prefer listing choices from [`list_available_encoders`] so the UI
/// never offers a backend this process cannot see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderPick {
    Auto,
    /// First available hardware encoder (NVENC → VA-API on Linux, MF on Windows).
    Hardware,
    Software,
    /// Linux NVIDIA NVENC. On other OSes open falls back to software.
    Nvenc,
    /// Linux VA-API. On other OSes open falls back to software.
    Vaapi,
    MediaFoundation,
}

/// One selectable encoder for Voice & Video / Stream options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvailableEncoder {
    pub pick: EncoderPick,
    pub key: &'static str,
    pub label: &'static str,
    /// False when the preference is kept for display but the probe no longer finds it.
    pub available: bool,
}

impl EncoderPick {
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Hardware => "hardware",
            Self::Software => "software",
            Self::Nvenc => "nvenc",
            Self::Vaapi => "vaapi",
            Self::MediaFoundation => "mediafoundation",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (prefer hardware)",
            Self::Hardware => "Any hardware",
            Self::Software => "Software (OpenH264)",
            Self::Nvenc => "NVIDIA NVENC",
            Self::Vaapi => "VA-API",
            Self::MediaFoundation => "Media Foundation",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "hardware" => Self::Hardware,
            "software" => Self::Software,
            "nvenc" => Self::Nvenc,
            "vaapi" => Self::Vaapi,
            "mediafoundation" | "media foundation" => Self::MediaFoundation,
            _ => Self::Auto,
        }
    }

    pub(crate) fn as_u8(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Hardware => 1,
            Self::Software => 2,
            Self::Nvenc => 3,
            Self::Vaapi => 4,
            Self::MediaFoundation => 5,
        }
    }

    pub(crate) fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Hardware,
            2 => Self::Software,
            3 => Self::Nvenc,
            4 => Self::Vaapi,
            5 => Self::MediaFoundation,
            _ => Self::Auto,
        }
    }
}

impl LayerEncoder {
    #[allow(dead_code)]
    pub fn open(
        screen: bool,
        width: u32,
        height: u32,
        fps: f32,
        bps: u32,
        keyframe_ms: u64,
    ) -> Result<Self, String> {
        Self::open_with(
            screen,
            width,
            height,
            fps,
            bps,
            keyframe_ms,
            EncoderPick::Auto,
            true,
        )
    }

    pub fn open_with(
        screen: bool,
        width: u32,
        height: u32,
        fps: f32,
        bps: u32,
        keyframe_ms: u64,
        pick: EncoderPick,
        cbr: bool,
    ) -> Result<Self, String> {
        match pick {
            EncoderPick::Software => Self::software(screen, fps, bps, keyframe_ms),
            EncoderPick::Auto | EncoderPick::Hardware => Self::with_optional_hardware(
                screen, width, height, fps, bps, keyframe_ms, None, cbr,
            ),
            EncoderPick::Nvenc => Self::with_optional_hardware(
                screen, width, height, fps, bps, keyframe_ms, Some(HardwareKind::Nvenc), cbr,
            ),
            EncoderPick::Vaapi => Self::with_optional_hardware(
                screen, width, height, fps, bps, keyframe_ms, Some(HardwareKind::Vaapi), cbr,
            ),
            EncoderPick::MediaFoundation => Self::with_optional_hardware(
                screen,
                width,
                height,
                fps,
                bps,
                keyframe_ms,
                Some(HardwareKind::MediaFoundation),
                cbr,
            ),
        }
    }

    fn with_optional_hardware(
        screen: bool,
        width: u32,
        height: u32,
        fps: f32,
        bps: u32,
        keyframe_ms: u64,
        only: Option<HardwareKind>,
        cbr: bool,
    ) -> Result<Self, String> {
        // HW-first: do not construct OpenH264 until hardware is missing or
        // the first usable AU fails the Constrained Baseline / Annex-B check.
        if let Some(inner) = try_hardware(screen, width, height, fps, bps, keyframe_ms, only, cbr)
        {
            Ok(Self {
                inner,
                soft: Some(SoftParams {
                    screen,
                    fps,
                    bps,
                    keyframe_ms,
                }),
            })
        } else {
            Self::software(screen, fps, bps, keyframe_ms)
        }
    }

    pub fn software(screen: bool, fps: f32, bps: u32, keyframe_ms: u64) -> Result<Self, String> {
        Ok(Self {
            inner: Box::new(SoftwareEncoder::open(screen, fps, bps, keyframe_ms)?),
            soft: None,
        })
    }

    pub fn backend(&self) -> Backend {
        self.inner.backend()
    }

    pub fn encode(
        &mut self,
        i420: &[u8],
        width: u32,
        height: u32,
        force_key: bool,
    ) -> Result<EncodedAu, String> {
        match self.inner.encode(i420, width, height, force_key) {
            // Delta AUs omit SPS. Only reject hardware when the AU is not Annex-B
            // (some Windows MFTs emit length-prefixed AVCC) or carries an SPS that
            // is not Constrained Baseline. Treating “no SPS” alone as failure made
            // every P-frame fall back to OpenH264 (BAN-30); keeping non-Annex-B
            // bytes after that change broke Windows remotes (BAN-35).
            Ok(au) if self.soft.is_none() || !hardware_au_unusable(&au.bytes) => Ok(au),
            Ok(_) | Err(_) => {
                if let Some(params) = self.soft.take() {
                    self.inner = Box::new(SoftwareEncoder::open(
                        params.screen,
                        params.fps,
                        params.bps,
                        params.keyframe_ms,
                    )?);
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
        let decoder = Decoder::with_api_config(OpenH264API::from_source(), Default::default())
            .map_err(|err| err.to_string())?;
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

/// Which hardware path [`try_hardware`] should open. `None` means first available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HardwareKind {
    Nvenc,
    Vaapi,
    MediaFoundation,
}

/// Names the hardware encoders this process can see. Empty means OpenH264 only.
///
/// Result is cached for the process lifetime (eager-init audit #6).
pub fn hardware_probe() -> Vec<&'static str> {
    HARDWARE_PROBE
        .get_or_init(|| {
            list_available_encoders()
                .into_iter()
                .filter(|entry| {
                    entry.available
                        && !matches!(
                            entry.pick,
                            EncoderPick::Auto | EncoderPick::Hardware | EncoderPick::Software
                        )
                })
                .map(|entry| entry.key)
                .collect()
        })
        .clone()
}

/// Simulcast layer target bitrate from the Full-layer budget.
///
/// Half/Quarter get more than a pure pixel-area share so dynacast mid/thumbnail
/// layers stay readable without raising Full.
pub fn layer_target_bps(full: u32, layer: LayerId) -> u32 {
    let full = full.max(30_000);
    match layer {
        LayerId::Full => full,
        // Half is ~1/4 of the pixels; ~1/3 of the bits keeps the mid layer sharp.
        LayerId::Half => (full / 3).max(100_000),
        // Quarter is ~1/16 of the pixels; ~1/10 keeps thumbnails usable.
        LayerId::Quarter => (full / 10).max(50_000),
        LayerId::Off => 0,
    }
}

/// OpenH264 QP floor/ceiling for interactive VC (min..=max, 0=best, 51=hardest).
///
/// Camera raises min so bits are not wasted on imperceptible detail. Screen keeps
/// a lower max so UI text does not mush when rate control is stressed.
pub fn openh264_qp_bounds(screen: bool) -> (u8, u8) {
    if screen {
        (12, 38)
    } else {
        (16, 42)
    }
}

fn nvenc_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        ffmpeg_has_encoder("h264_nvenc") && nvidia_encode_present()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn vaapi_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        ffmpeg_has_encoder("h264_vaapi") && std::path::Path::new("/dev/dri/renderD128").exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn media_foundation_present() -> bool {
    #[cfg(windows)]
    {
        super::mf::hardware_present()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

static AVAILABLE_ENCODERS: OnceLock<Vec<AvailableEncoder>> = OnceLock::new();

/// Scan this process for selectable H.264 encoders (Auto / Software always; HW when present).
///
/// Specific backends that are not installed are omitted so the UI cannot offer a missing encoder.
/// Pass `prefer` when reloading a saved key that is no longer present — it is appended disabled.
pub fn list_available_encoders() -> Vec<AvailableEncoder> {
    AVAILABLE_ENCODERS
        .get_or_init(|| list_available_encoders_uncached(None))
        .clone()
}

pub fn list_available_encoders_preferring(prefer: Option<&str>) -> Vec<AvailableEncoder> {
    match prefer {
        None => list_available_encoders(),
        Some(_) => list_available_encoders_uncached(prefer),
    }
}

fn list_available_encoders_uncached(prefer: Option<&str>) -> Vec<AvailableEncoder> {
    let mut out = vec![
        AvailableEncoder {
            pick: EncoderPick::Auto,
            key: EncoderPick::Auto.key(),
            label: EncoderPick::Auto.label(),
            available: true,
        },
        AvailableEncoder {
            pick: EncoderPick::Software,
            key: EncoderPick::Software.key(),
            label: EncoderPick::Software.label(),
            available: true,
        },
    ];
    let nvenc = nvenc_present();
    let vaapi = vaapi_present();
    let mf = media_foundation_present();
    if nvenc || vaapi || mf {
        out.push(AvailableEncoder {
            pick: EncoderPick::Hardware,
            key: EncoderPick::Hardware.key(),
            label: EncoderPick::Hardware.label(),
            available: true,
        });
    }
    if nvenc {
        out.push(AvailableEncoder {
            pick: EncoderPick::Nvenc,
            key: EncoderPick::Nvenc.key(),
            label: EncoderPick::Nvenc.label(),
            available: true,
        });
    }
    if vaapi {
        out.push(AvailableEncoder {
            pick: EncoderPick::Vaapi,
            key: EncoderPick::Vaapi.key(),
            label: EncoderPick::Vaapi.label(),
            available: true,
        });
    }
    if mf {
        out.push(AvailableEncoder {
            pick: EncoderPick::MediaFoundation,
            key: EncoderPick::MediaFoundation.key(),
            label: EncoderPick::MediaFoundation.label(),
            available: true,
        });
    }
    if let Some(key) = prefer {
        let pick = EncoderPick::from_key(key);
        if !matches!(
            pick,
            EncoderPick::Auto | EncoderPick::Software | EncoderPick::Hardware
        ) && !out.iter().any(|entry| entry.pick == pick)
        {
            out.push(AvailableEncoder {
                pick,
                key: pick.key(),
                label: pick.label(),
                available: false,
            });
        }
    }
    out
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
    let need = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
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

pub fn scale_i420<'a>(
    src: &'a [u8],
    width: u32,
    height: u32,
    layer: LayerId,
) -> Option<(Cow<'a, [u8]>, u32, u32)> {
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
        return Some((Cow::Borrowed(src), width, height));
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
    Some((Cow::Owned(y), dw, dh))
}

pub fn is_constrained_baseline(annexb: &[u8]) -> bool {
    nals(annexb)
        .any(|nal| nal.len() >= 4 && nal[0] & 0x1f == 7 && nal[1] == 66 && nal[2] & 0x40 != 0)
}

/// True when this AU carries an SPS that is *not* Constrained Baseline.
///
/// P-frames usually omit SPS. Treating “no SPS” as a profile failure made every
/// hardware session fall back to OpenH264 after the first keyframe, so the GPU
/// sat idle even though the probe had found VA-API / NVENC.
pub fn sps_rejects_constrained_baseline(annexb: &[u8]) -> bool {
    for nal in nals(annexb) {
        if nal.len() >= 4 && nal[0] & 0x1f == 7 {
            return !(nal[1] == 66 && nal[2] & 0x40 != 0);
        }
    }
    false
}

/// True when a hardware AU must not stay on the GPU path.
///
/// WebRTC packetization expects Annex-B start codes. BAN-30 stopped rejecting
/// SPS-less P-frames (correct for VA-API), but that also kept length-prefixed
/// Media Foundation samples that `nals` cannot parse — remotes then saw garbage
/// until the publisher fell back or restarted (BAN-35).
pub fn hardware_au_unusable(bytes: &[u8]) -> bool {
    if bytes.is_empty() || nals(bytes).next().is_none() {
        return true;
    }
    sps_rejects_constrained_baseline(bytes)
}

pub fn access_unit_is_key(annexb: &[u8]) -> bool {
    nals(annexb).any(|nal| !nal.is_empty() && nal[0] & 0x1f == 5)
}

fn build_openh264(screen: bool, fps: f32, bps: u32, keyframe_ms: u64) -> Result<Encoder, String> {
    let usage = if screen {
        UsageType::ScreenContentRealTime
    } else {
        UsageType::CameraVideoRealTime
    };
    let gop = gop_frames(fps, keyframe_ms);
    let (qp_min, qp_max) = openh264_qp_bounds(screen);
    // Bitrate RC + single thread + Low complexity favor interactive delay over archive quality
    // (BA-97 / PR #85). QP floors improve quality-per-bitrate without raising complexity.
    // ScreenContentRealTime rejects AdaptiveQuant / BackgroundDetection (harmless ParamValidation
    // warnings) — turn those off for screen so the encoder stays on the validated path.
    let mut config = EncoderConfig::new()
        .bitrate(BitRate::from_bps(bps.max(30_000)))
        .max_frame_rate(FrameRate::from_hz(fps.max(1.0)))
        .profile(Profile::Baseline)
        .usage_type(usage)
        .rate_control_mode(RateControlMode::Bitrate)
        .complexity(Complexity::Low)
        .num_threads(1)
        .long_term_reference(false)
        .qp(QpRange::new(qp_min, qp_max))
        .intra_frame_period(IntraFramePeriod::from_num_frames(gop));
    if screen {
        // Keep scene-change detect on (OpenH264 asks for it). AQ / background
        // detection are ParamValidation rejects for ScreenContentRealTime.
        config = config
            .adaptive_quantization(false)
            .background_detection(false);
    }
    Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|err| err.to_string())
}

/// Frames between IDRs from the stream keyframe interval and encode FPS.
pub fn gop_frames(fps: f32, keyframe_ms: u64) -> u32 {
    let fps = fps.max(1.0);
    let ms = keyframe_ms.clamp(500, 10_000) as f32;
    ((fps * ms) / 1000.0).round().max(1.0) as u32
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
            while end + 3 < bytes.len()
                && !bytes[end..].starts_with(&[0, 0, 1])
                && !bytes[end..].starts_with(&[0, 0, 0, 1])
            {
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
    if bytes.starts_with(&[0, 0, 0, 1]) {
        4
    } else {
        3
    }
}

fn yuv_of(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (
        y.clamp(0, 255) as u8,
        u.clamp(0, 255) as u8,
        v.clamp(0, 255) as u8,
    )
}

pub(crate) fn even_fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (2, 2);
    }
    let edge = if max_edge == 0 {
        width.max(height)
    } else {
        max_edge
    };
    let (mut dw, mut dh) = if width <= edge && height <= edge {
        (width, height)
    } else {
        let ratio = (edge as f64 / width as f64).min(edge as f64 / height as f64);
        (
            (width as f64 * ratio).round() as u32,
            (height as f64 * ratio).round() as u32,
        )
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

fn try_hardware(
    screen: bool,
    width: u32,
    height: u32,
    fps: f32,
    bps: u32,
    keyframe_ms: u64,
    only: Option<HardwareKind>,
    cbr: bool,
) -> Option<Box<dyn VideoEncoder>> {
    let want_nvenc = matches!(only, None | Some(HardwareKind::Nvenc));
    let want_vaapi = matches!(only, None | Some(HardwareKind::Vaapi));
    let want_mf = matches!(only, None | Some(HardwareKind::MediaFoundation));
    #[cfg(target_os = "linux")]
    {
        if want_nvenc && nvenc_present() {
            if let Ok(encoder) = FfmpegEncoder::open(
                "h264_nvenc",
                Backend::Nvenc,
                screen,
                width,
                height,
                fps,
                bps,
                keyframe_ms,
                cbr,
            ) {
                return Some(Box::new(encoder));
            }
            if matches!(only, Some(HardwareKind::Nvenc)) {
                return None;
            }
        }
        // AMD VA-API H.264 encode rejects sizes below 128×128 (see vainfo /
        // ffmpeg). Opening smaller layers (e.g. Quarter on a short edge) fails
        // and must not block Full/Half from using the GPU.
        if want_vaapi && vaapi_present() && width >= 128 && height >= 128 {
            if let Ok(encoder) = FfmpegEncoder::open(
                "h264_vaapi",
                Backend::Vaapi,
                screen,
                width,
                height,
                fps,
                bps,
                keyframe_ms,
                cbr,
            ) {
                return Some(Box::new(encoder));
            }
            if matches!(only, Some(HardwareKind::Vaapi)) {
                return None;
            }
        }
        let _ = want_mf;
    }
    #[cfg(windows)]
    {
        let _ = (screen, keyframe_ms, cbr, want_nvenc, want_vaapi);
        if want_mf {
            if let Ok(encoder) = super::mf::open(width, height, fps, bps) {
                return Some(Box::new(encoder));
            }
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (screen, width, height, fps, bps, keyframe_ms, cbr, want_nvenc, want_vaapi, want_mf);
    }
    None
}

/// Linux NVENC/VAAPI path via ffmpeg. Windows uses Media Foundation instead.
#[cfg(target_os = "linux")]
fn ffmpeg_has_encoder(name: &str) -> bool {
    let Ok(output) = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains(name)
}

#[cfg(target_os = "linux")]
fn nvidia_encode_present() -> bool {
    std::path::Path::new("/usr/lib/libnvidia-encode.so").exists()
        || std::path::Path::new("/usr/lib64/libnvidia-encode.so").exists()
        || Command::new("ldconfig")
            .arg("-p")
            .output()
            .ok()
            .is_some_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains("libnvidia-encode")
            })
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
    keyframe_ms: u64,
    screen: bool,
    cbr: bool,
    name: &'static str,
}

#[cfg(target_os = "linux")]
impl FfmpegEncoder {
    fn open(
        name: &'static str,
        backend: Backend,
        screen: bool,
        width: u32,
        height: u32,
        fps: f32,
        bps: u32,
        keyframe_ms: u64,
        cbr: bool,
    ) -> Result<Self, String> {
        let gop = gop_frames(fps, keyframe_ms);
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
            // async_depth defaults to 2, which holds the first AU until a second
            // frame is submitted. Realtime WebRTC can't wait that long — Banter
            // timed out and fell back to OpenH264 after every keyframe open.
            command.args([
                "-vaapi_device",
                "/dev/dri/renderD128",
                "-vf",
                "format=nv12,hwupload",
                "-async_depth",
                "1",
            ]);
        }
        command.args([
            "-c:v",
            name,
            "-bf",
            "0",
            "-g",
            &gop.to_string(),
            "-b:v",
            &bps.to_string(),
        ]);
        if name == "h264_nvenc" {
            // forced-idr: when a key is requested (reopen path), emit a true IDR.
            // p1 + ll + zerolatency keep interactive delay; CBR/VBR follows stream prefs.
            command.args([
                "-profile:v",
                "baseline",
                "-preset",
                "p1",
                "-tune",
                "ll",
                "-rc",
                if cbr { "cbr" } else { "vbr" },
                "-zerolatency",
                "1",
                "-forced-idr",
                "1",
            ]);
        } else {
            // VA-API constrained baseline + explicit RC (was bitrate-label only).
            command.args([
                "-profile:v",
                "constrained_baseline",
                "-rc_mode",
                if cbr { "CBR" } else { "VBR" },
            ]);
        }
        command.args(["-f", "h264", "pipe:1"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "ffmpeg produced no picture".to_string())?;
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
                        Ok(n) => captured
                            .lock()
                            .expect("ffmpeg bytes")
                            .extend_from_slice(&buf[..n]),
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
            keyframe_ms,
            screen,
            cbr,
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

    fn encode(
        &mut self,
        i420: &[u8],
        width: u32,
        height: u32,
        force_key: bool,
    ) -> Result<EncodedAu, String> {
        // Size change always reopens. force_key also reopens: the rawvideo pipe has
        // no mid-stream force-IDR API (unlike MF CodecAPI on Windows). Ignoring
        // force_key broke late-join / mesh recovery (BAN-33). Periodic keys come from
        // `-g` (gop_frames) so the encode loop only arms force_key for join/PLI/
        // topology — not every keyframe_ms tick. async_depth=1 + flush keep the first
        // AU from timing out the way BAN-30 saw before those fixes.
        if force_key || width != self.width || height != self.height {
            let replacement = FfmpegEncoder::open(
                self.name,
                self.backend,
                self.screen,
                width,
                height,
                self.fps,
                self.bps,
                self.keyframe_ms,
                self.cbr,
            )?;
            *self = replacement;
        }
        use std::io::Write;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "encoder is closed".to_string())?;
        let before = self.captured.lock().expect("ffmpeg bytes").len();
        stdin.write_all(i420).map_err(|err| err.to_string())?;
        stdin.flush().map_err(|err| err.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_millis(800);
        let mut flushed = false;
        loop {
            let bytes = self.captured.lock().expect("ffmpeg bytes");
            if bytes.len() > before && access_unit_is_key(&bytes[before..])
                || bytes.len() > before + 32 && nals(&bytes[before..]).count() >= 2
            {
                let au = bytes[before..].to_vec();
                drop(bytes);
                return Ok(EncodedAu {
                    keyframe: access_unit_is_key(&au),
                    bytes: au,
                });
            }
            drop(bytes);
            // Some VA-API stacks still hold the first AU until a second write.
            // Duplicate the frame once to flush, then keep waiting.
            if !flushed && std::time::Instant::now() + Duration::from_millis(400) > deadline {
                let stdin = self
                    .stdin
                    .as_mut()
                    .ok_or_else(|| "encoder is closed".to_string())?;
                stdin.write_all(i420).map_err(|err| err.to_string())?;
                stdin.flush().map_err(|err| err.to_string())?;
                flushed = true;
            }
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
    /// OpenH264 is deferred while hardware is present (eager-init audit #8).
    software: Option<VideoListen>,
}

impl PictureDecoder {
    pub fn open() -> Result<Self, String> {
        let hardware = try_hardware_decoder();
        Ok(Self {
            // Only pre-build OpenH264 when no HW decoder answered.
            software: if hardware.is_none() {
                Some(VideoListen::software()?)
            } else {
                None
            },
            hardware,
        })
    }

    fn ensure_software(&mut self) -> Result<&mut VideoListen, String> {
        if self.software.is_none() {
            self.software = Some(VideoListen::software()?);
        }
        self.software
            .as_mut()
            .ok_or_else(|| "openh264 decoder missing".into())
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
                self.ensure_software()?.decode_rgba(annexb)
            }
            Decoded::Software => self.ensure_software()?.decode_rgba(annexb),
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
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "hardware decoder produced no picture".to_string())?;
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
                        Ok(n) => captured
                            .lock()
                            .expect("decoder bytes")
                            .extend_from_slice(&buf[..n]),
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            captured,
            reader: Some(reader),
            failed: false,
            warmed: false,
        })
    }

    fn decode(&mut self, annexb: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
        if self.failed {
            return Err("hardware decoder already failed".into());
        }
        use std::io::Write;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "decoder is closed".to_string())?;
        stdin
            .write_all(annexb)
            .and_then(|_| stdin.write_all(&[0, 0, 0, 1, 9, 0x10]))
            .and_then(|_| stdin.flush())
            .map_err(|err| {
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
    let start = bytes
        .windows(3)
        .position(|mark| mark == b"P6\n" || mark == b"P6 ")?;
    let header = bytes.get(start..)?;
    let rest = header.strip_prefix(b"P6").and_then(|rest| {
        rest.strip_prefix(b"\n")
            .or_else(|| rest.strip_prefix(b"\r\n"))
    })?;
    let (dims, after_dims) = split_line(rest)?;
    let mut dims = dims.split_whitespace();
    let width: u32 = dims.next()?.parse().ok()?;
    let height: u32 = dims.next()?.parse().ok()?;
    let (_, pixels) = split_line(after_dims)?;
    let need = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(3);
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
    let Ok(output) = Command::new("ffmpeg")
        .args(["-hide_banner", &format!("-{list}")])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_encoders_always_include_auto_and_software() {
        let list = list_available_encoders();
        assert!(list.iter().any(|e| e.pick == EncoderPick::Auto && e.available));
        assert!(list
            .iter()
            .any(|e| e.pick == EncoderPick::Software && e.available));
        assert!(
            list.iter()
                .filter(|e| {
                    matches!(
                        e.pick,
                        EncoderPick::Nvenc | EncoderPick::Vaapi | EncoderPick::MediaFoundation
                    )
                })
                .all(|e| e.available),
            "specific backends in the list must be present"
        );
    }

    #[test]
    fn preferring_missing_backend_keeps_disabled_chip() {
        let list = list_available_encoders_preferring(Some("nvenc"));
        let nvenc = list.iter().find(|e| e.pick == EncoderPick::Nvenc);
        if nvenc_present() {
            assert!(nvenc.is_some_and(|e| e.available));
        } else {
            assert!(nvenc.is_some_and(|e| !e.available));
        }
    }

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
        let mut encoder = LayerEncoder::software(false, 30.0, 400_000, 2_000).expect("encoder");
        let rgba = vec![40u8, 80, 120, 255].repeat(16 * 16);
        let i420 = rgba_to_i420(&rgba, 16, 16);
        let au = encoder.encode(&i420, 16, 16, true).expect("encode");
        assert!(
            is_constrained_baseline(&au.bytes),
            "sps was not constrained baseline"
        );
        assert!(au.keyframe);
        let delta = encoder.encode(&i420, 16, 16, false).expect("delta");
        assert!(!delta.bytes.is_empty());
        let again = encoder.encode(&i420, 16, 16, true).expect("forced intra");
        assert!(
            again.keyframe,
            "a requested keyframe has to stay an intra frame without rebuilding the encoder"
        );
        let mut decoder = VideoListen::software().expect("decoder");
        let picture = decoder
            .decode_rgba(&au.bytes)
            .expect("decode")
            .expect("frame");
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

    #[test]
    fn full_layer_borrows_without_copying() {
        let rgba = vec![10u8, 20, 30, 255].repeat(16 * 16);
        let i420 = rgba_to_i420(&rgba, 16, 16);
        let (scaled, width, height) = scale_i420(&i420, 16, 16, LayerId::Full).expect("full");
        assert_eq!((width, height), (16, 16));
        assert!(matches!(scaled, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn gop_tracks_keyframe_interval_and_fps() {
        assert_eq!(gop_frames(30.0, 2_000), 60);
        assert_eq!(gop_frames(60.0, 1_000), 60);
        assert_eq!(gop_frames(30.0, 500), 15);
    }

    #[test]
    fn p_frame_without_sps_does_not_reject_hardware() {
        // Annex-B P-slice only (NAL type 1) — what VA-API emits on deltas.
        let p_au = [0u8, 0, 0, 1, 0x01, 0x9a, 0x10, 0x00];
        assert!(!is_constrained_baseline(&p_au));
        assert!(
            !sps_rejects_constrained_baseline(&p_au),
            "missing SPS must not trigger OpenH264 fallback"
        );
        assert!(
            !hardware_au_unusable(&p_au),
            "Annex-B delta without SPS must stay on hardware"
        );
    }

    #[test]
    fn high_profile_sps_rejects_hardware() {
        // NAL type 7, profile_idc=100 (High), constraint_set1_flag clear.
        let high = [0u8, 0, 0, 1, 0x67, 100, 0x0c, 0x1f];
        assert!(!is_constrained_baseline(&high));
        assert!(sps_rejects_constrained_baseline(&high));
    }

    #[test]
    fn constrained_baseline_sps_keeps_hardware() {
        let cb = [0u8, 0, 0, 1, 0x67, 66, 0x40, 0x15];
        assert!(is_constrained_baseline(&cb));
        assert!(!sps_rejects_constrained_baseline(&cb));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn vaapi_key_then_delta_stays_on_hardware() {
        if hardware_probe().is_empty() {
            return;
        }
        let width = 1280u32;
        let height = 720u32;
        let mut encoder = match LayerEncoder::open_with(
            true,
            width,
            height,
            30.0,
            2_000_000,
            2_000,
            EncoderPick::Hardware,
            true,
        ) {
            Ok(encoder) if matches!(encoder.backend(), Backend::Vaapi | Backend::Nvenc) => encoder,
            _ => return,
        };
        let rgba = vec![40u8, 80, 120, 255].repeat((width * height) as usize);
        let i420 = rgba_to_i420(&rgba, width, height);
        let key = encoder.encode(&i420, width, height, true).expect("key");
        assert!(
            is_constrained_baseline(&key.bytes),
            "first AU must be Constrained Baseline"
        );
        assert!(matches!(
            encoder.backend(),
            Backend::Vaapi | Backend::Nvenc
        ));
        let delta = encoder.encode(&i420, width, height, false).expect("delta");
        assert!(!delta.bytes.is_empty());
        assert!(
            matches!(encoder.backend(), Backend::Vaapi | Backend::Nvenc),
            "delta without SPS must not fall back to OpenH264; got {}",
            encoder.backend().label()
        );
        // Mid-stream force_key must reopen for a real IDR (late-join PLI / mesh flip).
        // async_depth=1 keeps this from timing out back to OpenH264 (BAN-33).
        let forced = encoder
            .encode(&i420, width, height, true)
            .expect("force_key after delta");
        assert!(
            forced.keyframe || access_unit_is_key(&forced.bytes),
            "force_key must emit an IDR so remotes can decode after PLI / topology flip"
        );
        assert!(
            matches!(encoder.backend(), Backend::Vaapi | Backend::Nvenc),
            "force_key reopen must stay on hardware; got {}",
            encoder.backend().label()
        );
    }

    #[test]
    fn software_force_key_emits_idr() {
        let width = 320u32;
        let height = 180u32;
        let mut encoder =
            LayerEncoder::software(true, 30.0, 500_000, 2_000).expect("openh264");
        let rgba = vec![10u8, 20, 30, 255].repeat((width * height) as usize);
        let i420 = rgba_to_i420(&rgba, width, height);
        let _key = encoder.encode(&i420, width, height, true).expect("key");
        let _delta = encoder.encode(&i420, width, height, false).expect("delta");
        let forced = encoder.encode(&i420, width, height, true).expect("force");
        assert!(
            forced.keyframe || access_unit_is_key(&forced.bytes),
            "OpenH264 force_key must still produce an IDR"
        );
    }

    #[test]
    fn annexb_delta_stays_usable_without_sps() {
        // Annex-B P-slice only — Linux VA-API / Windows MF deltas after BAN-30.
        let p_au = [0u8, 0, 0, 1, 0x01, 0x9a, 0x10, 0x00];
        assert!(
            !hardware_au_unusable(&p_au),
            "SPS-less Annex-B delta must stay on hardware"
        );
    }

    #[test]
    fn length_prefixed_avcc_is_unusable() {
        // Length-prefixed CB SPS without start codes — shape some Windows MFTs emit.
        // BAN-30's "no SPS → keep" must not accept this; remotes expect Annex-B.
        let nal = [0x67u8, 66, 0x40, 0x15];
        let mut avcc = (nal.len() as u32).to_be_bytes().to_vec();
        avcc.extend_from_slice(&nal);
        assert!(
            !sps_rejects_constrained_baseline(&avcc),
            "fixture: no Annex-B SPS so sps_rejects alone would keep it"
        );
        assert!(
            hardware_au_unusable(&avcc),
            "non-Annex-B hardware AU must fall back to OpenH264"
        );
        assert!(hardware_au_unusable(&[]), "empty AU is unusable");
    }
}
