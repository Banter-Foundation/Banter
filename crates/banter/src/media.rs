//! Microphone, speakers, camera, and screen capture.
//!
//! Device failures stay visible. Nothing here invents a frame when a device is missing.
//! A stream change is captured on a side thread and cut in only after that frame is encoded.

#[cfg(windows)]
mod loopback;

use std::collections::{HashMap, VecDeque};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use banter_server::{MediaKind, VoiceFlags, encode_client_media, should_send_audio, should_send_camera, should_send_screen};

use crate::logic::{
    AUDIO_FRAME_SAMPLES, CameraChoice, CaptureKind, MixSource, PrewarmClock, StreamJob, automatic_gain, cancel_echo,
    encode_jpeg_rgba, encode_jpeg_rgba_quality, fade_gain, frame_rms, mix_equal_power,
    pack_picture, resample_mono_i16, scale_i16, stamp_cursor, suppress_noise,
};

const FLAG_MUTED: u32 = 1;
const FLAG_DEAFENED: u32 = 2;
const FLAG_PTT: u32 = 4;
const FLAG_TRANSMIT: u32 = 8;
const FLAG_CAMERA: u32 = 16;
const FLAG_SCREEN: u32 = 32;

fn pack(flags: VoiceFlags) -> u32 {
    let mut bits = 0;
    if flags.muted {
        bits |= FLAG_MUTED;
    }
    if flags.deafened {
        bits |= FLAG_DEAFENED;
    }
    if flags.push_to_talk {
        bits |= FLAG_PTT;
    }
    if flags.transmitting {
        bits |= FLAG_TRANSMIT;
    }
    if flags.camera {
        bits |= FLAG_CAMERA;
    }
    if flags.screen {
        bits |= FLAG_SCREEN;
    }
    bits
}

fn unpack(bits: u32) -> VoiceFlags {
    VoiceFlags {
        muted: bits & FLAG_MUTED != 0,
        deafened: bits & FLAG_DEAFENED != 0,
        push_to_talk: bits & FLAG_PTT != 0,
        transmitting: bits & FLAG_TRANSMIT != 0,
        camera: bits & FLAG_CAMERA != 0,
        screen: bits & FLAG_SCREEN != 0,
    }
}

struct Engine {
    input_rms: AtomicU32,
    input_volume: AtomicU32,
    output_volume: AtomicU32,
    sensitivity: AtomicU32,
    noise: AtomicBool,
    echo: AtomicBool,
    agc: AtomicBool,
    mic_test: AtomicBool,
    show_cursor: AtomicBool,
    show_preview: AtomicBool,
    quality: AtomicU32,
    input_name: Mutex<String>,
    output_name: Mutex<String>,
    sources: Mutex<Vec<MixSource>>,
    source_pcm: Arc<Mutex<HashMap<String, VecDeque<i16>>>>,
    last_output: Mutex<Vec<i16>>,
    epoch: AtomicU64,
}

impl Engine {
    fn new() -> Self {
        Self {
            input_rms: AtomicU32::new(0),
            input_volume: AtomicU32::new(1f32.to_bits()),
            output_volume: AtomicU32::new(1f32.to_bits()),
            sensitivity: AtomicU32::new(crate::logic::OPEN_MIC_RMS.to_bits()),
            noise: AtomicBool::new(true),
            echo: AtomicBool::new(true),
            agc: AtomicBool::new(false),
            mic_test: AtomicBool::new(false),
            show_cursor: AtomicBool::new(true),
            show_preview: AtomicBool::new(true),
            quality: AtomicU32::new(75),
            input_name: Mutex::new(String::new()),
            output_name: Mutex::new(String::new()),
            sources: Mutex::new(Vec::new()),
            source_pcm: Arc::new(Mutex::new(HashMap::new())),
            last_output: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(1),
        }
    }
}

fn store_f32(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

fn load_f32(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

/// Live voice and stream controls. Device names reopen capture only when they change.
pub struct EngineUpdate {
    pub input_device: String,
    pub output_device: String,
    pub input_volume: f32,
    pub output_volume: f32,
    pub sensitivity: f32,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    pub quality: u8,
    pub show_cursor: bool,
    pub show_preview: bool,
    pub sources: Vec<MixSource>,
}

pub struct Media {
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    errors: Arc<Mutex<Vec<String>>>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    output_rate: Arc<AtomicU32>,
    /// The latest local camera JPEG, tagged with the device index it came from.
    local_camera: Arc<Mutex<Option<(u32, Vec<u8>)>>>,
    local_screen: Arc<Mutex<Option<Vec<u8>>>>,
    remote_video: Arc<Mutex<HashMap<(i64, u8), Vec<u8>>>>,
    camera_index: Arc<AtomicU32>,
    camera_preview: Arc<AtomicBool>,
    stream_edge: Arc<AtomicU32>,
    stream_interval: Arc<AtomicU64>,
    stream_target: Arc<Mutex<Option<(CaptureKind, u32)>>>,
    engine: Arc<Engine>,
    threads: Vec<JoinHandle<()>>,
}

pub struct VideoFrame {
    pub sender: Option<i64>,
    pub kind: MediaKind,
    pub jpeg: Vec<u8>,
}

impl Media {
    pub fn start(outgoing: Sender<Vec<u8>>) -> Self {
        let flags = Arc::new(AtomicU32::new(0));
        let in_place = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let playback = Arc::new(Mutex::new(VecDeque::new()));
        let captured = Arc::new(Mutex::new(VecDeque::new()));
        let input_rate = Arc::new(AtomicU32::new(48_000));
        let local_camera = Arc::new(Mutex::new(None));
        let local_screen = Arc::new(Mutex::new(None));
        let remote_video = Arc::new(Mutex::new(HashMap::new()));
        let camera_index = Arc::new(AtomicU32::new(0));
        let camera_preview = Arc::new(AtomicBool::new(false));
        let stream_edge = Arc::new(AtomicU32::new(1280));
        let stream_interval = Arc::new(AtomicU64::new(33));
        let stream_target = Arc::new(Mutex::new(None));
        let output_rate = Arc::new(AtomicU32::new(48_000));
        let engine = Arc::new(Engine::new());
        let devices = thread::spawn({
            let stop = Arc::clone(&stop);
            let engine = Arc::clone(&engine);
            let captured = Arc::clone(&captured);
            let input_rate = Arc::clone(&input_rate);
            let playback = Arc::clone(&playback);
            let flags = Arc::clone(&flags);
            let output_rate = Arc::clone(&output_rate);
            let errors = Arc::clone(&errors);
            move || device_loop(stop, engine, captured, input_rate, playback, flags, output_rate, errors)
        });

        let audio = thread::spawn({
            let flags = Arc::clone(&flags);
            let in_place = Arc::clone(&in_place);
            let stop = Arc::clone(&stop);
            let captured = Arc::clone(&captured);
            let input_rate = Arc::clone(&input_rate);
            let playback = Arc::clone(&playback);
            let engine = Arc::clone(&engine);
            let outgoing = outgoing.clone();
            move || audio_loop(flags, in_place, stop, captured, input_rate, playback, engine, outgoing)
        });
        let camera = thread::spawn({
            let flags = Arc::clone(&flags);
            let in_place = Arc::clone(&in_place);
            let stop = Arc::clone(&stop);
            let local_camera = Arc::clone(&local_camera);
            let errors = Arc::clone(&errors);
            let camera_index = Arc::clone(&camera_index);
            let camera_preview = Arc::clone(&camera_preview);
            let outgoing = outgoing.clone();
            move || camera_loop(flags, in_place, stop, local_camera, errors, camera_index, camera_preview, outgoing)
        });
        let screen = thread::spawn({
            let flags = Arc::clone(&flags);
            let in_place = Arc::clone(&in_place);
            let stop = Arc::clone(&stop);
            let local_screen = Arc::clone(&local_screen);
            let errors = Arc::clone(&errors);
            let stream_edge = Arc::clone(&stream_edge);
            let stream_interval = Arc::clone(&stream_interval);
            let stream_target = Arc::clone(&stream_target);
            let engine = Arc::clone(&engine);
            move || screen_loop(flags, in_place, stop, local_screen, errors, stream_edge, stream_interval, stream_target, engine, outgoing)
        });

        Self {
            flags,
            in_place,
            stop,
            errors,
            playback,
            output_rate,
            local_camera,
            local_screen,
            remote_video,
            camera_index,
            camera_preview,
            stream_edge,
            stream_interval,
            stream_target,
            engine,
            threads: vec![devices, audio, camera, screen],
        }
    }

    pub fn set_camera_choice(&self, index: u32, preview: bool) {
        self.camera_index.store(index, Ordering::Relaxed);
        self.camera_preview.store(preview, Ordering::Relaxed);
    }

    pub fn set_stream_choice(&self, target: Option<(CaptureKind, u32)>, max_edge: u32, interval_ms: u64) {
        *self.stream_target.lock().expect("stream target") = target;
        self.stream_edge.store(max_edge, Ordering::Relaxed);
        self.stream_interval.store(interval_ms.max(1), Ordering::Relaxed);
    }

    pub fn apply_engine(&self, update: &EngineUpdate) {
        store_f32(&self.engine.input_volume, update.input_volume.clamp(0.0, 2.0));
        store_f32(&self.engine.output_volume, update.output_volume.clamp(0.0, 2.0));
        store_f32(&self.engine.sensitivity, update.sensitivity.max(0.0));
        self.engine.noise.store(update.noise_suppression, Ordering::Relaxed);
        self.engine.echo.store(update.echo_cancellation, Ordering::Relaxed);
        self.engine.agc.store(update.auto_gain, Ordering::Relaxed);
        self.engine.quality.store(update.quality as u32, Ordering::Relaxed);
        self.engine.show_cursor.store(update.show_cursor, Ordering::Relaxed);
        self.engine.show_preview.store(update.show_preview, Ordering::Relaxed);
        let changed = {
            let mut input = self.engine.input_name.lock().expect("input device");
            let mut output = self.engine.output_name.lock().expect("output device");
            let mut sources = self.engine.sources.lock().expect("stream sources");
            let changed = *input != update.input_device || *output != update.output_device || *sources != update.sources;
            *input = update.input_device.clone();
            *output = update.output_device.clone();
            *sources = update.sources.clone();
            changed
        };
        if changed {
            self.engine.epoch.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_mic_test(&self, enabled: bool) {
        self.engine.mic_test.store(enabled, Ordering::Relaxed);
    }

    pub fn input_rms(&self) -> f32 {
        load_f32(&self.engine.input_rms)
    }

    pub fn set_state(&self, flags: VoiceFlags, in_place: bool) {
        if flags.deafened {
            self.playback.lock().expect("playback").clear();
        }
        self.flags.store(pack(flags), Ordering::Relaxed);
        self.in_place.store(in_place, Ordering::Relaxed);
    }

    pub fn errors(&self) -> Vec<String> {
        self.errors.lock().expect("media errors").clone()
    }

    pub fn push_remote_audio(&self, pcm: &[u8]) {
        if unpack(self.flags.load(Ordering::Relaxed)).deafened {
            return;
        }
        if pcm.len() < 2 || pcm.len() % 2 != 0 {
            return;
        }
        let mut samples = Vec::with_capacity(pcm.len() / 2);
        for chunk in pcm.chunks_exact(2) {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let rate = self.output_rate.load(Ordering::Relaxed).max(1);
        let samples = resample_mono_i16(&samples, 48_000, rate);
        let mut queue = self.playback.lock().expect("playback");
        for sample in samples {
            queue.push_back(sample as f32 / i16::MAX as f32);
        }
        let cap = (rate as usize).saturating_mul(1);
        while queue.len() > cap {
            queue.pop_front();
        }
    }

    pub fn push_remote_video(&self, sender: i64, kind: MediaKind, jpeg: &[u8]) {
        self.remote_video
            .lock()
            .expect("remote video")
            .insert((sender, kind_code(kind)), jpeg.to_vec());
    }

    pub fn take_video(&self) -> Vec<VideoFrame> {
        let mut frames = Vec::new();
        let chosen = self.camera_index.load(Ordering::Relaxed);
        if let Some(jpeg) = current_camera_frame(self.local_camera.lock().expect("camera").take(), chosen) {
            frames.push(VideoFrame { sender: None, kind: MediaKind::Camera, jpeg });
        }
        if let Some(jpeg) = self.local_screen.lock().expect("screen").take() {
            frames.push(VideoFrame { sender: None, kind: MediaKind::Screen, jpeg });
        }
        for ((sender, kind), jpeg) in self.remote_video.lock().expect("remote video").drain() {
            let Some(kind) = kind_from(kind) else { continue };
            frames.push(VideoFrame { sender: Some(sender), kind, jpeg });
        }
        frames
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn kind_code(kind: MediaKind) -> u8 {
    match kind {
        MediaKind::Audio => 1,
        MediaKind::Camera => 2,
        MediaKind::Screen => 3,
    }
}

fn kind_from(code: u8) -> Option<MediaKind> {
    match code {
        2 => Some(MediaKind::Camera),
        3 => Some(MediaKind::Screen),
        _ => None,
    }
}

fn push_mono(data: &[i16], channels: u16, store: &Mutex<VecDeque<i16>>) {
    let channels = channels.max(1) as usize;
    let mut queue = store.lock().expect("capture");
    for frame in data.chunks(channels) {
        let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
        queue.push_back((sum / channels as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    let cap = 48_000 * 2;
    while queue.len() > cap {
        queue.pop_front();
    }
}

fn pick_device(input: bool, name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let missing = if input { "no microphone is available" } else { "no speakers are available" };
    if name.is_empty() {
        return if input {
            host.default_input_device().ok_or_else(|| missing.to_string())
        } else {
            host.default_output_device().ok_or_else(|| missing.to_string())
        };
    }
    let devices = if input {
        host.input_devices().map_err(|err| err.to_string())?
    } else {
        host.output_devices().map_err(|err| err.to_string())?
    };
    devices
        .into_iter()
        .find(|device| device.name().ok().as_deref() == Some(name))
        .ok_or_else(|| format!("{name} is not available"))
}

pub(crate) fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).filter(|name| !name.trim().is_empty()).collect())
        .unwrap_or_default()
}

pub(crate) fn output_device_names() -> Vec<String> {
    #[cfg(windows)]
    {
        let wasapi = loopback::render_devices();
        if !wasapi.is_empty() {
            return wasapi;
        }
    }
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).filter(|name| !name.trim().is_empty()).collect())
        .unwrap_or_default()
}

pub(crate) fn app_audio_sources() -> Vec<(u32, String)> {
    let Ok(windows) = xcap::Window::all() else { return Vec::new() };
    let mut rows = Vec::new();
    for window in windows {
        if window.is_minimized().unwrap_or(true) || window.width().unwrap_or(0) < 80 {
            continue;
        }
        let Ok(pid) = window.pid() else { continue };
        if pid == 0 || rows.iter().any(|(existing, _)| *existing == pid) {
            continue;
        }
        let app = window.app_name().unwrap_or_default();
        let title = window.title().unwrap_or_default();
        let name = if !app.trim().is_empty() { app } else { title };
        if name.trim().is_empty() {
            continue;
        }
        rows.push((pid, name));
    }
    rows.sort_by(|left, right| left.1.cmp(&right.1));
    rows
}

pub(crate) fn monitor_limits() -> (u32, u32) {
    let Ok(monitors) = xcap::Monitor::all() else { return (0, 0) };
    let Some(monitor) = monitors.iter().find(|monitor| monitor.is_primary().unwrap_or(false)).or_else(|| monitors.first()) else {
        return (0, 0);
    };
    (monitor.height().unwrap_or(0), monitor.frequency().unwrap_or(0.0).round() as u32)
}

fn device_loop(
    stop: Arc<AtomicBool>,
    engine: Arc<Engine>,
    captured: Arc<Mutex<VecDeque<i16>>>,
    input_rate: Arc<AtomicU32>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    flags: Arc<AtomicU32>,
    output_rate: Arc<AtomicU32>,
    errors: Arc<Mutex<Vec<String>>>,
) {
    let mut applied = u64::MAX;
    let mut streams: Vec<cpal::Stream> = Vec::new();
    #[cfg(windows)]
    let mut loops: Vec<loopback::Loop> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let current = engine.epoch.load(Ordering::Relaxed);
        if current != applied {
            streams.clear();
            #[cfg(windows)]
            loops.clear();
            let input_name = engine.input_name.lock().expect("input device").clone();
            let output_name = engine.output_name.lock().expect("output device").clone();
            match open_input(&input_name, Arc::clone(&captured), Arc::clone(&input_rate)) {
                Ok(stream) => {
                    streams.push(stream);
                    device_recovered(&errors, "microphone");
                }
                Err(err) => device_failed(&errors, "microphone", &err),
            }
            match open_output(&output_name, Arc::clone(&playback), Arc::clone(&flags), Arc::clone(&engine), Arc::clone(&output_rate)) {
                Ok(stream) => {
                    streams.push(stream);
                    device_recovered(&errors, "speakers");
                }
                Err(err) => device_failed(&errors, "speakers", &err),
            }
            let sources = engine.sources.lock().expect("stream sources").clone();
            for source in sources.into_iter().filter(|source| source.enabled) {
                if let Some(name) = source.id.strip_prefix("mic:") {
                    match open_input_queue(name, source.id.clone(), Arc::clone(&engine.source_pcm)) {
                        Ok(stream) => streams.push(stream),
                        Err(err) => remember_error(&errors, format!("{}: {err}", source.name)),
                    }
                } else if let Some(name) = source.id.strip_prefix("loop:") {
                    #[cfg(windows)]
                    match loopback::start_render(source.id.clone(), name.to_string(), Arc::clone(&engine.source_pcm)) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_error(&errors, format!("{}: {err}", source.name)),
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = name;
                        remember_error(&errors, format!("{}: speaker loopback is only available on Windows", source.name));
                    }
                } else if let Some(pid) = source.id.strip_prefix("app:").and_then(|pid| pid.parse::<u32>().ok()) {
                    #[cfg(windows)]
                    match loopback::start_process(source.id.clone(), pid, Arc::clone(&engine.source_pcm)) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_error(&errors, format!("{}: {err}", source.name)),
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = pid;
                        remember_error(&errors, format!("{}: app audio is only available on Windows", source.name));
                    }
                }
            }
            applied = current;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn open_input(name: &str, store: Arc<Mutex<VecDeque<i16>>>, rate: Arc<AtomicU32>) -> Result<cpal::Stream, String> {
    let device = pick_device(true, name)?;
    let config = device.default_input_config().map_err(|err| err.to_string())?;
    rate.store(config.sample_rate().0, Ordering::Relaxed);
    let channels = config.channels();
    let stream_config = config.config();
    let format = config.sample_format();
    let store_err = Arc::clone(&store);
    let stream = match format {
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[i16], _| push_mono(data, channels, &store),
                move |err| {
                    let _ = err;
                    let _ = &store_err;
                },
                None,
            )
            .map_err(|err| err.to_string())?,
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let samples: Vec<i16> = data
                        .iter()
                        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                        .collect();
                    push_mono(&samples, channels, &store);
                },
                |_| {},
                None,
            )
            .map_err(|err| err.to_string())?,
        other => return Err(format!("microphone format {other} is not supported")),
    };
    stream.play().map_err(|err| err.to_string())?;
    Ok(stream)
}

fn open_input_queue(name: &str, id: String, store: Arc<Mutex<HashMap<String, VecDeque<i16>>>>) -> Result<cpal::Stream, String> {
    let device = pick_device(true, name)?;
    let config = device.default_input_config().map_err(|err| err.to_string())?;
    let rate = config.sample_rate().0.max(1);
    let channels = config.channels();
    let stream_config = config.config();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let stream = build_input(&device, config.sample_format(), &stream_config, channels, Arc::clone(&queue))?;
    let queued = Arc::clone(&queue);
    thread::spawn(move || {
        while Arc::strong_count(&queued) > 1 {
            let raw = {
                let mut samples = queued.lock().expect("extra mic");
                if samples.len() < 480 {
                    None
                } else {
                    Some(samples.drain(..).collect::<Vec<_>>())
                }
            };
            if let Some(raw) = raw {
                let samples = resample_mono_i16(&raw, rate, 48_000);
                let mut map = store.lock().expect("source audio");
                let bucket = map.entry(id.clone()).or_default();
                bucket.extend(samples);
                while bucket.len() > 48_000 {
                    bucket.pop_front();
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
    Ok(stream)
}

fn build_input(device: &cpal::Device, format: cpal::SampleFormat, stream_config: &cpal::StreamConfig, channels: u16, store: Arc<Mutex<VecDeque<i16>>>) -> Result<cpal::Stream, String> {
    let stream = match format {
        cpal::SampleFormat::I16 => device
            .build_input_stream(stream_config, move |data: &[i16], _| push_mono(data, channels, &store), |_| {}, None)
            .map_err(|err| err.to_string())?,
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                stream_config,
                move |data: &[f32], _| {
                    let samples: Vec<i16> = data.iter().map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
                    push_mono(&samples, channels, &store);
                },
                |_| {},
                None,
            )
            .map_err(|err| err.to_string())?,
        other => return Err(format!("microphone format {other} is not supported")),
    };
    stream.play().map_err(|err| err.to_string())?;
    Ok(stream)
}

fn open_output(name: &str, playback: Arc<Mutex<VecDeque<f32>>>, flags: Arc<AtomicU32>, engine: Arc<Engine>, rate_slot: Arc<AtomicU32>) -> Result<cpal::Stream, String> {
    let device = pick_device(false, name)?;
    let config = device.default_output_config().map_err(|err| err.to_string())?;
    let rate = config.sample_rate().0;
    rate_slot.store(rate, Ordering::Relaxed);
    let channels = config.channels().max(1) as usize;
    let stream_config = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            let playback = Arc::clone(&playback);
            let flags = Arc::clone(&flags);
            let engine = Arc::clone(&engine);
            device
                .build_output_stream(
                    &stream_config,
                    move |output: &mut [f32], _| fill_output(output, channels, &playback, &flags, &engine),
                    |_| {},
                    None,
                )
                .map_err(|err| err.to_string())?
        }
        cpal::SampleFormat::I16 => {
            let engine = Arc::clone(&engine);
            device
            .build_output_stream(
                &stream_config,
                move |output: &mut [i16], _| {
                    let deafened = unpack(flags.load(Ordering::Relaxed)).deafened;
                    let volume = load_f32(&engine.output_volume);
                    let mut queue = playback.lock().expect("playback");
                    let mut played = Vec::with_capacity(output.len() / channels.max(1));
                    for frame in output.chunks_mut(channels) {
                        let sample = if deafened { 0.0 } else { queue.pop_front().unwrap_or(0.0) * volume };
                        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        played.push(value);
                        for slot in frame {
                            *slot = value;
                        }
                    }
                    *engine.last_output.lock().expect("echo reference") = played;
                },
                |_| {},
                None,
            )
            .map_err(|err| err.to_string())?
        }
        other => return Err(format!("speaker format {other} is not supported")),
    };
    stream.play().map_err(|err| err.to_string())?;
    Ok(stream)
}

fn fill_output(output: &mut [f32], channels: usize, playback: &Mutex<VecDeque<f32>>, flags: &AtomicU32, engine: &Engine) {
    let deafened = unpack(flags.load(Ordering::Relaxed)).deafened;
    let volume = load_f32(&engine.output_volume);
    let mut queue = playback.lock().expect("playback");
    let mut played = Vec::with_capacity(output.len() / channels.max(1));
    for frame in output.chunks_mut(channels) {
        let sample = if deafened { 0.0 } else { queue.pop_front().unwrap_or(0.0) * volume };
        played.push((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        for slot in frame {
            *slot = sample;
        }
    }
    *engine.last_output.lock().expect("echo reference") = played;
}

fn audio_loop(
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    captured: Arc<Mutex<VecDeque<i16>>>,
    input_rate: Arc<AtomicU32>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    engine: Arc<Engine>,
    outgoing: Sender<Vec<u8>>,
) {
    let mut fades: HashMap<String, f32> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let rate = input_rate.load(Ordering::Relaxed).max(1);
        let need = ((AUDIO_FRAME_SAMPLES as u64) * (rate as u64) / 48_000).max(1) as usize;
        let raw = {
            let mut queue = captured.lock().expect("capture");
            if queue.len() < need {
                None
            } else {
                Some(queue.drain(0..need).collect::<Vec<_>>())
            }
        };
        let Some(raw) = raw else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        let mut samples = resample_mono_i16(&raw, rate, 48_000);
        if samples.len() > AUDIO_FRAME_SAMPLES {
            samples.truncate(AUDIO_FRAME_SAMPLES);
        }
        while samples.len() < AUDIO_FRAME_SAMPLES {
            samples.push(0);
        }
        let gain = load_f32(&engine.input_volume);
        let sensitivity = load_f32(&engine.sensitivity);
        samples = scale_i16(&samples, gain);
        samples = suppress_noise(&samples, engine.noise.load(Ordering::Relaxed), sensitivity * 0.5);
        samples = automatic_gain(&samples, engine.agc.load(Ordering::Relaxed), 4_000.0);
        let reference = engine.last_output.lock().expect("echo reference").clone();
        samples = cancel_echo(&samples, &reference, engine.echo.load(Ordering::Relaxed));
        let rms = frame_rms(&samples);
        store_f32(&engine.input_rms, rms);
        if engine.mic_test.load(Ordering::Relaxed) {
            let mut queue = playback.lock().expect("playback");
            for sample in &samples {
                queue.push_back((*sample as f32 / i16::MAX as f32) * 0.45);
            }
        }
        let state = unpack(flags.load(Ordering::Relaxed));
        if state.screen {
            samples = mix_stream_sources(&samples, &engine, &mut fades);
        }
        if !in_place.load(Ordering::Relaxed) || !should_send_audio(&state) {
            continue;
        }
        if !state.push_to_talk && rms < sensitivity {
            continue;
        }
        let mut pcm = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        let frame = encode_client_media(MediaKind::Audio, &pcm);
        if outgoing.send(frame).is_err() {
            break;
        }
    }
}

fn mix_stream_sources(mic: &[i16], engine: &Engine, fades: &mut HashMap<String, f32>) -> Vec<i16> {
    let sources = engine.sources.lock().expect("stream sources").clone();
    let mut mixed = mic.to_vec();
    for source in sources {
        let target = if source.enabled { 1.0 } else { 0.0 };
        let current = fades.entry(source.id.clone()).or_insert(0.0);
        *current = fade_gain(*current, target, 20.0 / 150.0);
        let t = *current;
        if t <= 0.001 {
            continue;
        }
        let extra = {
            let mut map = engine.source_pcm.lock().expect("source audio");
            let Some(queue) = map.get_mut(&source.id) else { continue };
            let take = AUDIO_FRAME_SAMPLES.min(queue.len());
            let mut frame: Vec<i16> = queue.drain(0..take).collect();
            frame.resize(AUDIO_FRAME_SAMPLES, 0);
            frame
        };
        let scaled = scale_i16(&extra, source.volume.clamp(0.0, 2.0));
        let with = add_i16(&mixed, &scaled);
        mixed = mix_equal_power(&mixed, &with, t);
    }
    mixed
}

fn add_i16(left: &[i16], right: &[i16]) -> Vec<i16> {
    let len = left.len().max(right.len());
    let mut out = Vec::with_capacity(len);
    for index in 0..len {
        let sample = left.get(index).copied().unwrap_or(0) as i32 + right.get(index).copied().unwrap_or(0) as i32;
        out.push(sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    out
}

fn device_failed(errors: &Mutex<Vec<String>>, device: &str, raw: &str) {
    eprintln!("{device}: {raw}");
    let mut list = errors.lock().expect("media errors");
    match crate::settings::friendly_device_error(device) {
        Some(message) => crate::settings::remember_device_error(&mut list, device, message),
        None => crate::settings::remember_device_error(&mut list, device, raw),
    }
}

fn device_recovered(errors: &Mutex<Vec<String>>, device: &str) {
    crate::settings::forget_device_errors(&mut errors.lock().expect("media errors"), device);
}

fn remember_error(errors: &Mutex<Vec<String>>, message: String) {
    let mut list = errors.lock().expect("media errors");
    if !list.iter().any(|existing| existing == &message) {
        list.push(message);
    }
}

fn camera_loop(
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    local: Arc<Mutex<Option<(u32, Vec<u8>)>>>,
    errors: Arc<Mutex<Vec<String>>>,
    camera_index: Arc<AtomicU32>,
    camera_preview: Arc<AtomicBool>,
    outgoing: Sender<Vec<u8>>,
) {
    let mut camera: Option<nokhwa::Camera> = None;
    let mut opened: Option<u32> = None;
    let mut reported = false;
    while !stop.load(Ordering::Relaxed) {
        let state = unpack(flags.load(Ordering::Relaxed));
        let send = in_place.load(Ordering::Relaxed) && should_send_camera(&state);
        let preview = camera_preview.load(Ordering::Relaxed);
        if !send && !preview {
            camera = None;
            opened = None;
            reported = false;
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let mut choice = CameraChoice::new();
        choice.select(camera_index.load(Ordering::Relaxed));
        if choice.needs_reopen(opened) {
            camera = None;
            opened = None;
            reported = false;
            *local.lock().expect("camera") = None;
            device_recovered(&errors, "camera");
        }
        if camera.is_none() {
            match open_camera(choice.device_index()) {
                Ok(device) => {
                    camera = Some(device);
                    opened = Some(choice.device_index());
                }
                Err(err) => {
                    if !reported {
                        device_failed(&errors, "camera", &format!("open: {err}"));
                        reported = true;
                    }
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }
        let Some(device) = camera.as_mut() else { continue };
        match capture_camera(device) {
            Ok(jpeg) => {
                if reported {
                    device_recovered(&errors, "camera");
                    reported = false;
                }
                if send {
                    let frame = encode_client_media(MediaKind::Camera, &jpeg);
                    if outgoing.send(frame).is_err() {
                        break;
                    }
                }
                *local.lock().expect("camera") = Some((choice.device_index(), jpeg));
            }
            // Reported once per failure run; a failing camera fails every frame, about 30 times a second.
            Err(err) if !reported => {
                device_failed(&errors, "camera", &format!("frame: {err}"));
                reported = true;
            }
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(33));
    }
}

/// Real camera names from the operating system. An empty list means none are available.
pub(crate) fn camera_names() -> Vec<(u32, String)> {
    use nokhwa::utils::{ApiBackend, CameraIndex};
    nokhwa::nokhwa_initialize(|_| {});
    let Ok(list) = nokhwa::query(ApiBackend::Auto) else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|info| {
            let &CameraIndex::Index(index) = info.index() else {
                return None;
            };
            let human = info.human_name();
            let name = if human.trim().is_empty() { format!("Camera {index}") } else { human };
            Some((index, name))
        })
        .collect()
}

/// Target capture size for a webcam. Banter asks for this (or the nearest available mode) instead of
/// opening the device's highest resolution and then shrinking the frame.
pub fn preferred_camera_width() -> u32 {
    1280
}

pub fn preferred_camera_height() -> u32 {
    720
}

/// One camera mode the picker can rank. `mjpeg` is true for MJPEG (Facecam, Elgato, and most webcams).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraModeChoice {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub mjpeg: bool,
}

/// Picks ~720p capture. Modes at or above 720p beat anything smaller when both exist; then nearest
/// resolution wins; at the same size, FPS near 30 then MJPEG.
pub fn pick_preferred_camera_mode(modes: &[CameraModeChoice]) -> Option<CameraModeChoice> {
    const TARGET_FPS: u32 = 30;
    let target_w = preferred_camera_width();
    let target_h = preferred_camera_height();
    modes
        .iter()
        .copied()
        .min_by_key(|mode| {
            let undersized = (mode.width < target_w || mode.height < target_h) as u64;
            let dx = mode.width as i64 - target_w as i64;
            let dy = mode.height as i64 - target_h as i64;
            let distance = (dx * dx + dy * dy) as u64;
            let fps_delta = (mode.fps as i64 - TARGET_FPS as i64).unsigned_abs() as u64;
            let mjpeg_rank = if mode.mjpeg { 0u64 } else { 1 };
            (undersized, distance, fps_delta, mjpeg_rank)
        })
}

fn open_camera(index: u32) -> Result<nokhwa::Camera, String> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
    nokhwa::nokhwa_initialize(|granted| {
        if !granted {
            eprintln!("camera permission was denied");
        }
    });
    // Open with a flexible request so we can list modes, then switch to ~720p (or nearest).
    let mut camera = nokhwa::Camera::new(
        CameraIndex::Index(index),
        RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(preferred_camera_width(), preferred_camera_height()),
            FrameFormat::MJPEG,
            30,
        ))),
    )
    .or_else(|_| {
        nokhwa::Camera::new(
            CameraIndex::Index(index),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(preferred_camera_width(), preferred_camera_height()),
                FrameFormat::YUYV,
                30,
            ))),
        )
    })
    .or_else(|_| {
        nokhwa::Camera::new(
            CameraIndex::Index(index),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::None),
        )
    })
    .map_err(|err| err.to_string())?;
    if let Ok(formats) = camera.compatible_camera_formats() {
        let modes: Vec<CameraModeChoice> = formats
            .iter()
            .map(|format| {
                let resolution = format.resolution();
                CameraModeChoice {
                    width: resolution.width(),
                    height: resolution.height(),
                    fps: format.frame_rate(),
                    mjpeg: format.format() == FrameFormat::MJPEG,
                }
            })
            .collect();
        if let Some(pick) = pick_preferred_camera_mode(&modes) {
            if let Some(exact) = formats.iter().copied().find(|format| {
                let resolution = format.resolution();
                resolution.width() == pick.width
                    && resolution.height() == pick.height
                    && format.frame_rate() == pick.fps
                    && (format.format() == FrameFormat::MJPEG) == pick.mjpeg
            }) {
                let _ = camera.set_camera_requset(RequestedFormat::with_formats(
                    RequestedFormatType::Exact(exact),
                    &[exact.format()],
                ));
            }
        }
    }
    camera.open_stream().map_err(|err| err.to_string())?;
    Ok(camera)
}

/// Decodes one camera frame to RGB. nokhwa's own MJPEG path needs its `mjpeg` feature (mozjpeg), which Banter
/// leaves off, and without it nokhwa answers "Not available on WASM". MJPEG frames go through `image` instead.
pub fn decode_camera_frame(frame: &nokhwa::Buffer) -> Result<(u32, u32, Vec<u8>), String> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::FrameFormat;
    if frame.source_frame_format() == FrameFormat::MJPEG {
        let image = image::load_from_memory_with_format(frame.buffer(), image::ImageFormat::Jpeg)
            .map_err(|err| format!("MJPEG frame: {err}"))?
            .into_rgb8();
        return Ok((image.width(), image.height(), image.into_raw()));
    }
    let image = frame.decode_image::<RgbFormat>().map_err(|err| err.to_string())?;
    Ok((image.width(), image.height(), image.into_raw()))
}

/// The local preview slot holds the device index with each frame, so a frame captured just before the user
/// picked another camera is never shown as the new camera.
pub fn current_camera_frame(slot: Option<(u32, Vec<u8>)>, chosen_device: u32) -> Option<Vec<u8>> {
    match slot {
        Some((device, jpeg)) if device == chosen_device => Some(jpeg),
        _ => None,
    }
}

fn capture_camera(camera: &mut nokhwa::Camera) -> Result<Vec<u8>, String> {
    let frame = camera.frame().map_err(|err| err.to_string())?;
    let (width, height, rgb) = decode_camera_frame(&frame)?;
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for pixel in rgb.chunks_exact(3) {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    encode_jpeg_rgba(width, height, &rgba, 640)
}

enum Grabber {
    Monitor { monitor: xcap::Monitor },
    Window { window: xcap::Window },
}

struct WarmFrame {
    job: StreamJob,
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

fn screen_loop(
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    local: Arc<Mutex<Option<Vec<u8>>>>,
    errors: Arc<Mutex<Vec<String>>>,
    stream_edge: Arc<AtomicU32>,
    stream_interval: Arc<AtomicU64>,
    stream_target: Arc<Mutex<Option<(CaptureKind, u32)>>>,
    engine: Arc<Engine>,
    outgoing: Sender<Vec<u8>>,
) {
    let mut grabber: Option<Grabber> = None;
    let mut clock: Option<PrewarmClock> = None;
    let mut next_deadline = 0u64;
    let mut started_at = Instant::now();
    let (warm_tx, warm_rx) = mpsc::channel::<WarmFrame>();
    let mut warming: Option<StreamJob> = None;
    let mut ready_warm: Option<WarmFrame> = None;
    while !stop.load(Ordering::Relaxed) {
        let state = unpack(flags.load(Ordering::Relaxed));
        let enabled = in_place.load(Ordering::Relaxed) && should_send_screen(&state);
        let target = stream_target.lock().expect("stream target").clone();
        if !enabled || target.is_none() {
            grabber = None;
            clock = None;
            warming = None;
            ready_warm = None;
            thread::sleep(Duration::from_millis(40));
            continue;
        }
        let Some((kind, id)) = target else { continue };
        let edge = stream_edge.load(Ordering::Relaxed);
        let interval = stream_interval.load(Ordering::Relaxed).max(1);
        let quality = engine.quality.load(Ordering::Relaxed).clamp(1, 100) as u8;
        let show_cursor = engine.show_cursor.load(Ordering::Relaxed);
        let desired = StreamJob { source: source_key(kind, id), edge, interval_ms: interval, quality };
        if clock.is_none() {
            match open_grabber(kind, id) {
                Ok(found) => {
                    grabber = Some(found);
                    clock = Some(PrewarmClock::new(desired));
                    started_at = Instant::now();
                    next_deadline = 0;
                }
                Err(err) => {
                    remember_error(&errors, format!("screen: {err}"));
                    thread::sleep(Duration::from_millis(400));
                    continue;
                }
            }
        }
        let Some(clock) = clock.as_mut() else { continue };
        if desired != clock.active() && warming != Some(desired) {
            clock.request(desired);
            warming = Some(desired);
            let tx = warm_tx.clone();
            let errors = Arc::clone(&errors);
            thread::spawn(move || {
                let (warm_kind, warm_id) = source_parts(desired.source);
                match capture_encoded(warm_kind, warm_id, desired.edge, desired.quality, show_cursor) {
                    Ok((jpeg, width, height)) => {
                        let _ = tx.send(WarmFrame { job: desired, jpeg, width, height });
                    }
                    Err(err) => remember_error(&errors, format!("screen: {err}")),
                }
            });
        }
        while let Ok(warm) = warm_rx.try_recv() {
            if warming == Some(warm.job) {
                clock.prewarm_ready();
                ready_warm = Some(warm);
            }
        }
        let now_ms = started_at.elapsed().as_millis() as u64;
        if now_ms + 1 < next_deadline {
            let wait = next_deadline - now_ms;
            if wait > 3 {
                thread::sleep(Duration::from_millis(1));
            } else {
                std::hint::spin_loop();
            }
            continue;
        }
        let Some(frame) = clock.poll(now_ms) else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        next_deadline = frame.at_ms.saturating_add(frame.job.interval_ms.max(1));
        if frame.keyframe {
            if let Some(warm) = ready_warm.take() {
                if warm.job == frame.job {
                    warming = None;
                    let (warm_kind, warm_id) = source_parts(frame.job.source);
                    if let Ok(found) = open_grabber(warm_kind, warm_id) {
                        grabber = Some(found);
                    }
                    if send_picture(&outgoing, &local, &engine, true, warm.width, warm.height, &warm.jpeg).is_err() {
                        break;
                    }
                    continue;
                }
            }
        }
        let Some(found) = grabber.as_ref() else { continue };
        match capture_grabber(found, frame.job.edge, frame.job.quality, show_cursor) {
            Ok((jpeg, width, height)) => {
                if send_picture(&outgoing, &local, &engine, frame.keyframe, width, height, &jpeg).is_err() {
                    break;
                }
            }
            Err(err) => remember_error(&errors, format!("screen: {err}")),
        }
    }
}

fn source_key(kind: CaptureKind, id: u32) -> u64 {
    let tag = match kind {
        CaptureKind::Screen => 1u64,
        CaptureKind::Window => 2u64,
    };
    (tag << 32) | id as u64
}

fn source_parts(key: u64) -> (CaptureKind, u32) {
    let kind = if key >> 32 == 2 { CaptureKind::Window } else { CaptureKind::Screen };
    (kind, key as u32)
}

fn send_picture(outgoing: &Sender<Vec<u8>>, local: &Mutex<Option<Vec<u8>>>, engine: &Engine, keyframe: bool, width: u32, height: u32, jpeg: &[u8]) -> Result<(), ()> {
    if engine.show_preview.load(Ordering::Relaxed) {
        *local.lock().expect("screen") = Some(jpeg.to_vec());
    } else {
        *local.lock().expect("screen") = None;
    }
    let packet = pack_picture(keyframe, width, height, jpeg);
    outgoing.send(encode_client_media(MediaKind::Screen, &packet)).map_err(|_| ())
}

fn capture_encoded(kind: CaptureKind, id: u32, edge: u32, quality: u8, show_cursor: bool) -> Result<(Vec<u8>, u32, u32), String> {
    let grabber = open_grabber(kind, id)?;
    capture_grabber(&grabber, edge, quality, show_cursor)
}

fn capture_grabber(grabber: &Grabber, edge: u32, quality: u8, show_cursor: bool) -> Result<(Vec<u8>, u32, u32), String> {
    let image = match grabber {
        Grabber::Monitor { monitor, .. } => monitor.capture_image().map_err(|err| err.to_string())?,
        Grabber::Window { window, .. } => window.capture_image().map_err(|err| err.to_string())?,
    };
    let width = image.width();
    let height = image.height();
    let mut rgba = image.into_raw();
    if show_cursor {
        if let Some((cursor_x, cursor_y)) = cursor_point() {
            let (origin_x, origin_y, desktop_w, desktop_h) = grabber_bounds(grabber);
            if desktop_w > 0 && desktop_h > 0 {
                let x = ((cursor_x - origin_x) as f32 / desktop_w as f32 * width as f32) as i32;
                let y = ((cursor_y - origin_y) as f32 / desktop_h as f32 * height as f32) as i32;
                stamp_cursor(&mut rgba, width, height, x, y, true);
            }
        }
    }
    let jpeg = encode_jpeg_rgba_quality(width, height, &rgba, edge, quality)?;
    Ok((jpeg, fit_edge(edge, width, height).0, fit_edge(edge, width, height).1))
}

fn fit_edge(edge: u32, width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    if edge == 0 || (width <= edge && height <= edge) {
        return (width, height);
    }
    let ratio = (edge as f64 / width as f64).min(edge as f64 / height as f64);
    (((width as f64) * ratio).round().max(1.0) as u32, ((height as f64) * ratio).round().max(1.0) as u32)
}

fn grabber_bounds(grabber: &Grabber) -> (i32, i32, u32, u32) {
    match grabber {
        Grabber::Monitor { monitor, .. } => (
            monitor.x().unwrap_or(0),
            monitor.y().unwrap_or(0),
            monitor.width().unwrap_or(0),
            monitor.height().unwrap_or(0),
        ),
        Grabber::Window { window, .. } => (
            window.x().unwrap_or(0),
            window.y().unwrap_or(0),
            window.width().unwrap_or(0),
            window.height().unwrap_or(0),
        ),
    }
}

fn cursor_point() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetCursorPos(point: *mut CursorPoint) -> i32;
        }
        #[repr(C)]
        struct CursorPoint {
            x: i32,
            y: i32,
        }
        let mut point = CursorPoint { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            None
        } else {
            Some((point.x, point.y))
        }
    }
    #[cfg(target_os = "linux")]
    {
        cursor_point_x11()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// Global cursor for screen-share overlay. Works on X11 and XWayland; pure Wayland returns None.
#[cfg(target_os = "linux")]
fn cursor_point_x11() -> Option<(i32, i32)> {
    use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};

    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut c_void;
        fn XCloseDisplay(display: *mut c_void) -> c_int;
        fn XDefaultRootWindow(display: *mut c_void) -> c_ulong;
        fn XQueryPointer(
            display: *mut c_void,
            window: c_ulong,
            root_return: *mut c_ulong,
            child_return: *mut c_ulong,
            root_x: *mut c_int,
            root_y: *mut c_int,
            win_x: *mut c_int,
            win_y: *mut c_int,
            mask: *mut c_uint,
        ) -> c_int;
    }

    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return None;
        }
        let root = XDefaultRootWindow(display);
        let mut root_ret = 0;
        let mut child_ret = 0;
        let mut root_x = 0;
        let mut root_y = 0;
        let mut win_x = 0;
        let mut win_y = 0;
        let mut mask = 0;
        let ok = XQueryPointer(
            display,
            root,
            &mut root_ret,
            &mut child_ret,
            &mut root_x,
            &mut root_y,
            &mut win_x,
            &mut win_y,
            &mut mask,
        );
        let _ = XCloseDisplay(display);
        if ok == 0 {
            None
        } else {
            Some((root_x, root_y))
        }
    }
}

fn open_grabber(kind: CaptureKind, id: u32) -> Result<Grabber, String> {
    match kind {
        CaptureKind::Screen => {
            let monitors = xcap::Monitor::all().map_err(|err| err.to_string())?;
            let monitor = monitors
                .into_iter()
                .find(|monitor| monitor.id().ok() == Some(id))
                .ok_or_else(|| "that screen is gone".to_string())?;
            Ok(Grabber::Monitor { monitor })
        }
        CaptureKind::Window => {
            let windows = xcap::Window::all().map_err(|err| err.to_string())?;
            let window = windows
                .into_iter()
                .find(|window| window.id().ok() == Some(id))
                .ok_or_else(|| "that window is gone".to_string())?;
            Ok(Grabber::Window { window })
        }
    }
}
