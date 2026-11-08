//! Microphone, speakers, camera, and screen capture.
//!
//! Device failures stay visible. Nothing here invents a frame when a device is missing.
//! A stream change is captured on a side thread and cut in only after that frame is encoded.

mod encode;
mod ice_cache;
#[cfg(windows)]
mod loopback;
#[cfg(target_os = "linux")]
mod loopback_linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod mf;
mod qos;
mod rtc;
mod screen_lag;
mod screen_pipeline;
#[cfg(target_os = "linux")]
mod kmsgrab;
#[cfg(target_os = "linux")]
mod screencast;

pub use encode::{
    hardware_probe, layer_target_bps, list_available_encoders, list_available_encoders_preferring,
    openh264_qp_bounds, AvailableEncoder, EncoderPick,
};
pub use ice_cache::{clear as clear_ice_config, store as store_ice_config};
pub use rtc::{playback_cap_samples, EncodeSettings};

/// Max PCM samples retained before Opus encode / capture drop (~200 ms @ 48 kHz).
///
/// Was a full second (or two) of backlog, which turned a stall into delayed audio
/// instead of a short drop.
pub fn pcm_backlog_samples() -> usize {
    9_600
}

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use banter_server::{
    should_send_camera, should_send_screen, should_send_stream_audio, ClientSignal, LayerId,
    MediaKind, MediaPlace, ServerEvent, VoiceFlags,
};

use crate::logic::{
    automatic_gain, cancel_echo, fade_gain, frame_rms, mic_opus_frame, mix_equal_power,
    resample_mono_i16, scale_i16, stamp_cursor, suppress_noise, CameraChoice, CaptureKind,
    MixSource, AUDIO_FRAME_SAMPLES,
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
    /// Latest local camera preview as RGBA, tagged with the device index it came from.
    local_camera: Arc<Mutex<Option<(u32, u32, u32, Vec<u8>)>>>,
    local_screen: Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>,
    camera_index: Arc<AtomicU32>,
    camera_preview: Arc<AtomicBool>,
    stream_edge: Arc<AtomicU32>,
    stream_interval: Arc<AtomicU64>,
    stream_target: Arc<Mutex<Option<StreamPick>>>,
    /// Bumped whenever the stream target changes so the capture thread can clear a sticky open failure.
    stream_epoch: Arc<AtomicU64>,
    capture_failed: Arc<AtomicBool>,
    engine: Arc<Engine>,
    remote_pictures: Arc<Mutex<Vec<rtc::RemotePicture>>>,
    session: rtc::Rtc,
    /// Latest-wins producer for the screen convert worker (wake on shutdown).
    screen_convert: Arc<screen_pipeline::ScreenConvertTx>,
    threads: Vec<JoinHandle<()>>,
}

pub struct VideoFrame {
    pub sender: Option<i64>,
    pub kind: MediaKind,
    pub jpeg: Vec<u8>,
    pub rgba: Option<(u32, u32, Vec<u8>)>,
    pub layer: LayerId,
}

impl Media {
    pub fn start(signals: Sender<ClientSignal>) -> Self {
        let flags = Arc::new(AtomicU32::new(0));
        let in_place = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let playback = Arc::new(Mutex::new(VecDeque::new()));
        let captured = Arc::new(Mutex::new(VecDeque::new()));
        let user_pcm = Arc::new(Mutex::new(VecDeque::new()));
        let stream_pcm = Arc::new(Mutex::new(VecDeque::new()));
        let camera_picture = Arc::new(rtc::PictureSlot::new());
        let screen_picture = Arc::new(rtc::PictureSlot::new());
        let remote_pictures = Arc::new(Mutex::new(Vec::new()));
        let input_rate = Arc::new(AtomicU32::new(48_000));
        let local_camera = Arc::new(Mutex::new(None));
        let local_screen = Arc::new(Mutex::new(None));
        let camera_index = Arc::new(AtomicU32::new(0));
        let camera_preview = Arc::new(AtomicBool::new(false));
        let stream_edge = Arc::new(AtomicU32::new(1280));
        let stream_interval = Arc::new(AtomicU64::new(33));
        let stream_target = Arc::new(Mutex::new(None));
        let stream_epoch = Arc::new(AtomicU64::new(0));
        let capture_failed = Arc::new(AtomicBool::new(false));
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
            move || {
                device_loop(
                    stop,
                    engine,
                    captured,
                    input_rate,
                    playback,
                    flags,
                    output_rate,
                    errors,
                )
            }
        });

        let audio = thread::spawn({
            let flags = Arc::clone(&flags);
            let in_place = Arc::clone(&in_place);
            let stop = Arc::clone(&stop);
            let captured = Arc::clone(&captured);
            let input_rate = Arc::clone(&input_rate);
            let playback = Arc::clone(&playback);
            let engine = Arc::clone(&engine);
            let user_pcm = Arc::clone(&user_pcm);
            let stream_pcm = Arc::clone(&stream_pcm);
            move || {
                audio_loop(
                    flags, in_place, stop, captured, input_rate, playback, engine, user_pcm,
                    stream_pcm,
                )
            }
        });
        let camera = thread::spawn({
            let flags = Arc::clone(&flags);
            let in_place = Arc::clone(&in_place);
            let stop = Arc::clone(&stop);
            let local_camera = Arc::clone(&local_camera);
            let errors = Arc::clone(&errors);
            let camera_index = Arc::clone(&camera_index);
            let camera_preview = Arc::clone(&camera_preview);
            let camera_picture = Arc::clone(&camera_picture);
            move || {
                camera_loop(
                    flags,
                    in_place,
                    stop,
                    local_camera,
                    errors,
                    camera_index,
                    camera_preview,
                    camera_picture,
                )
            }
        });
        let (screen_convert, convert_thread) = screen_pipeline::start_convert_thread(
            Arc::clone(&stop),
            Arc::clone(&local_screen),
            Arc::clone(&engine),
            Arc::clone(&screen_picture),
        );
        let screen_convert = Arc::new(screen_convert);
        let screen = thread::Builder::new()
            .name("banter-screen-capture".into())
            .spawn({
                let flags = Arc::clone(&flags);
                let in_place = Arc::clone(&in_place);
                let stop = Arc::clone(&stop);
                let local_screen = Arc::clone(&local_screen);
                let errors = Arc::clone(&errors);
                let stream_edge = Arc::clone(&stream_edge);
                let stream_interval = Arc::clone(&stream_interval);
                let stream_target = Arc::clone(&stream_target);
                let stream_epoch = Arc::clone(&stream_epoch);
                let capture_failed = Arc::clone(&capture_failed);
                let engine = Arc::clone(&engine);
                let screen_convert = Arc::clone(&screen_convert);
                move || {
                    screen_loop(
                        flags,
                        in_place,
                        stop,
                        local_screen,
                        errors,
                        stream_edge,
                        stream_interval,
                        stream_target,
                        stream_epoch,
                        capture_failed,
                        engine,
                        screen_convert,
                    )
                }
            })
            .expect("spawn screen capture");

        let session = rtc::Rtc::start(
            signals,
            user_pcm,
            stream_pcm,
            camera_picture,
            screen_picture,
            Arc::clone(&playback),
            Arc::clone(&remote_pictures),
            Arc::clone(&output_rate),
            Arc::clone(&flags),
        );
        Self {
            flags,
            in_place,
            stop,
            errors,
            playback,
            local_camera,
            local_screen,
            camera_index,
            camera_preview,
            stream_edge,
            stream_interval,
            stream_target,
            stream_epoch,
            capture_failed,
            engine,
            remote_pictures,
            session,
            screen_convert,
            threads: vec![devices, audio, camera, screen, convert_thread],
        }
    }

    pub fn set_session(&self, place: Option<MediaPlace>, me: i64, offerer: bool, mesh: bool) {
        self.session.set_session(place, me, offerer, mesh);
    }

    pub fn ingest(&self, event: ServerEvent) {
        self.session.ingest(event);
    }

    pub fn set_camera_choice(&self, index: u32, preview: bool) {
        self.camera_index.store(index, Ordering::Relaxed);
        self.camera_preview.store(preview, Ordering::Relaxed);
    }

    pub fn set_stream_choice(&self, target: Option<StreamPick>, max_edge: u32, interval_ms: u64) {
        {
            let mut slot = self.stream_target.lock().expect("stream target");
            if *slot != target {
                self.stream_epoch.fetch_add(1, Ordering::Relaxed);
            }
            *slot = target;
        }
        self.stream_edge.store(max_edge, Ordering::Relaxed);
        self.stream_interval
            .store(interval_ms.max(1), Ordering::Relaxed);
    }

    pub fn apply_encode(&self, settings: EncodeSettings) {
        self.session.apply_encode(settings);
    }

    pub fn note_publishing(&self, camera: bool, screen: bool) {
        self.session.note_publishing(camera, screen);
    }

    /// The recorder could not start (the share dialog was dismissed, or the source is gone).
    /// Cleared on read so the UI stops the share once.
    pub fn take_capture_failure(&self) -> bool {
        self.capture_failed.swap(false, Ordering::Relaxed)
    }

    pub fn apply_engine(&self, update: &EngineUpdate) {
        store_f32(
            &self.engine.input_volume,
            update.input_volume.clamp(0.0, 2.0),
        );
        store_f32(
            &self.engine.output_volume,
            update.output_volume.clamp(0.0, 2.0),
        );
        store_f32(&self.engine.sensitivity, update.sensitivity.max(0.0));
        self.engine
            .noise
            .store(update.noise_suppression, Ordering::Relaxed);
        self.engine
            .echo
            .store(update.echo_cancellation, Ordering::Relaxed);
        self.engine.agc.store(update.auto_gain, Ordering::Relaxed);
        self.engine
            .show_cursor
            .store(update.show_cursor, Ordering::Relaxed);
        self.engine
            .show_preview
            .store(update.show_preview, Ordering::Relaxed);
        let changed = {
            let mut input = self.engine.input_name.lock().expect("input device");
            let mut output = self.engine.output_name.lock().expect("output device");
            let mut sources = self.engine.sources.lock().expect("stream sources");
            let changed = *input != update.input_device
                || *output != update.output_device
                || *sources != update.sources;
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

    pub fn take_video(&self) -> Vec<VideoFrame> {
        let mut frames = Vec::new();
        let chosen = self.camera_index.load(Ordering::Relaxed);
        if let Some((width, height, rgba)) =
            current_camera_frame(self.local_camera.lock().expect("camera").take(), chosen)
        {
            frames.push(VideoFrame {
                sender: None,
                kind: MediaKind::Camera,
                jpeg: Vec::new(),
                rgba: Some((width, height, rgba)),
                layer: LayerId::Full,
            });
        }
        if let Some((width, height, rgba)) = self.local_screen.lock().expect("screen").take() {
            frames.push(VideoFrame {
                sender: None,
                kind: MediaKind::Screen,
                jpeg: Vec::new(),
                rgba: Some((width, height, rgba)),
                layer: LayerId::Full,
            });
        }
        for picture in self
            .remote_pictures
            .lock()
            .expect("remote pictures")
            .drain(..)
        {
            frames.push(VideoFrame {
                sender: Some(picture.sender),
                kind: picture.kind,
                jpeg: Vec::new(),
                rgba: Some((picture.width, picture.height, picture.rgba)),
                layer: picture.layer,
            });
        }
        frames
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.screen_convert.wake();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn push_mono(data: &[i16], channels: u16, store: &Mutex<VecDeque<i16>>) {
    let channels = channels.max(1) as usize;
    let mut queue = store.lock().expect("capture");
    for frame in data.chunks(channels) {
        let sum: i32 = frame.iter().map(|sample| *sample as i32).sum();
        queue.push_back((sum / channels as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    let cap = pcm_backlog_samples();
    while queue.len() > cap {
        queue.pop_front();
    }
}

fn pick_device(input: bool, name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let missing = if input {
        "no microphone is available"
    } else {
        "no speakers are available"
    };
    if name.is_empty() {
        return if input {
            host.default_input_device()
                .ok_or_else(|| missing.to_string())
        } else {
            host.default_output_device()
                .ok_or_else(|| missing.to_string())
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
        .map(|devices| {
            devices
                .filter_map(|device| device.name().ok())
                .filter(|name| !name.trim().is_empty())
                .collect()
        })
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
        .map(|devices| {
            devices
                .filter_map(|device| device.name().ok())
                .filter(|name| !name.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn app_audio_sources() -> Vec<(u32, String)> {
    let Ok(windows) = xcap::Window::all() else {
        return Vec::new();
    };
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
    let Ok(monitors) = xcap::Monitor::all() else {
        return (0, 0);
    };
    let Some(monitor) = monitors
        .iter()
        .find(|monitor| monitor.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
    else {
        return (0, 0);
    };
    (
        monitor.height().unwrap_or(0),
        monitor.frequency().unwrap_or(0.0).round() as u32,
    )
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
    #[cfg(target_os = "linux")]
    let mut loops: Vec<loopback_linux::Loop> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        let current = engine.epoch.load(Ordering::Relaxed);
        if current != applied {
            streams.clear();
            #[cfg(any(windows, target_os = "linux"))]
            loops.clear();
            let input_name = engine.input_name.lock().expect("input device").clone();
            let output_name = engine.output_name.lock().expect("output device").clone();
            match open_input(&input_name, Arc::clone(&captured), Arc::clone(&input_rate)) {
                Ok(stream) => {
                    streams.push(stream);
                    device_recovered(&errors, "microphone");
                }
                Err(err) => {
                    #[cfg(target_os = "macos")]
                    let err = format!("{err}; {}", macos::privacy_hint("microphone"));
                    device_failed(&errors, "microphone", &err);
                }
            }
            match open_output(
                &output_name,
                Arc::clone(&playback),
                Arc::clone(&flags),
                Arc::clone(&engine),
                Arc::clone(&output_rate),
            ) {
                Ok(stream) => {
                    streams.push(stream);
                    device_recovered(&errors, "speakers");
                }
                Err(err) => device_failed(&errors, "speakers", &err),
            }
            let sources = engine.sources.lock().expect("stream sources").clone();
            for source in sources.into_iter().filter(|source| source.enabled) {
                if let Some(name) = source.id.strip_prefix("mic:") {
                    match open_input_queue(name, source.id.clone(), Arc::clone(&engine.source_pcm))
                    {
                        Ok(stream) => streams.push(stream),
                        Err(err) => remember_loopback_error(&errors, &source.name, &err),
                    }
                } else if let Some(name) = source.id.strip_prefix("loop:") {
                    #[cfg(windows)]
                    match loopback::start_render(
                        source.id.clone(),
                        name.to_string(),
                        Arc::clone(&engine.source_pcm),
                    ) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_loopback_error(&errors, &source.name, &err),
                    }
                    #[cfg(target_os = "linux")]
                    match loopback_linux::start_render(
                        source.id.clone(),
                        name.to_string(),
                        Arc::clone(&engine.source_pcm),
                    ) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_loopback_error(&errors, &source.name, &err),
                    }
                    #[cfg(not(any(windows, target_os = "linux")))]
                    {
                        let _ = name;
                        remember_loopback_error(
                            &errors,
                            &source.name,
                            "speaker loopback is not available on this system",
                        );
                    }
                } else if let Some(pid) = source
                    .id
                    .strip_prefix("app:")
                    .and_then(|pid| pid.parse::<u32>().ok())
                {
                    #[cfg(windows)]
                    match loopback::start_process(
                        source.id.clone(),
                        pid,
                        Arc::clone(&engine.source_pcm),
                    ) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_loopback_error(&errors, &source.name, &err),
                    }
                    #[cfg(target_os = "linux")]
                    match loopback_linux::start_process(
                        source.id.clone(),
                        pid,
                        Arc::clone(&engine.source_pcm),
                    ) {
                        Ok(handle) => loops.push(handle),
                        Err(err) => remember_loopback_error(&errors, &source.name, &err),
                    }
                    #[cfg(not(any(windows, target_os = "linux")))]
                    {
                        let _ = pid;
                        remember_loopback_error(
                            &errors,
                            &source.name,
                            "app audio is not available on this system",
                        );
                    }
                }
            }
            applied = current;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn open_input(
    name: &str,
    store: Arc<Mutex<VecDeque<i16>>>,
    rate: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let device = pick_device(true, name)?;
    let config = device
        .default_input_config()
        .map_err(|err| err.to_string())?;
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

fn open_input_queue(
    name: &str,
    id: String,
    store: Arc<Mutex<HashMap<String, VecDeque<i16>>>>,
) -> Result<cpal::Stream, String> {
    let device = pick_device(true, name)?;
    let config = device
        .default_input_config()
        .map_err(|err| err.to_string())?;
    let rate = config.sample_rate().0.max(1);
    let channels = config.channels();
    let stream_config = config.config();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let stream = build_input(
        &device,
        config.sample_format(),
        &stream_config,
        channels,
        Arc::clone(&queue),
    )?;
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
                while bucket.len() > pcm_backlog_samples() {
                    bucket.pop_front();
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
    Ok(stream)
}

fn build_input(
    device: &cpal::Device,
    format: cpal::SampleFormat,
    stream_config: &cpal::StreamConfig,
    channels: u16,
    store: Arc<Mutex<VecDeque<i16>>>,
) -> Result<cpal::Stream, String> {
    let stream = match format {
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                stream_config,
                move |data: &[i16], _| push_mono(data, channels, &store),
                |_| {},
                None,
            )
            .map_err(|err| err.to_string())?,
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                stream_config,
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

fn open_output(
    name: &str,
    playback: Arc<Mutex<VecDeque<f32>>>,
    flags: Arc<AtomicU32>,
    engine: Arc<Engine>,
    rate_slot: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let device = pick_device(false, name)?;
    let config = device
        .default_output_config()
        .map_err(|err| err.to_string())?;
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
                    move |output: &mut [f32], _| {
                        fill_output(output, channels, &playback, &flags, &engine)
                    },
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
                            let sample = if deafened {
                                0.0
                            } else {
                                queue.pop_front().unwrap_or(0.0) * volume
                            };
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

fn fill_output(
    output: &mut [f32],
    channels: usize,
    playback: &Mutex<VecDeque<f32>>,
    flags: &AtomicU32,
    engine: &Engine,
) {
    let deafened = unpack(flags.load(Ordering::Relaxed)).deafened;
    let volume = load_f32(&engine.output_volume);
    let mut queue = playback.lock().expect("playback");
    let mut played = Vec::with_capacity(output.len() / channels.max(1));
    for frame in output.chunks_mut(channels) {
        let sample = if deafened {
            0.0
        } else {
            queue.pop_front().unwrap_or(0.0) * volume
        };
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
    user_pcm: Arc<Mutex<VecDeque<i16>>>,
    stream_pcm: Arc<Mutex<VecDeque<i16>>>,
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
        samples = suppress_noise(
            &samples,
            engine.noise.load(Ordering::Relaxed),
            sensitivity * 0.5,
        );
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
        let stream = mix_stream_sources(&engine, &mut fades);
        if in_place.load(Ordering::Relaxed) && should_send_stream_audio(&state) {
            push_pcm(&stream_pcm, &stream);
        }
        if !in_place.load(Ordering::Relaxed) {
            continue;
        }
        // Do not gate open-mic by RMS here. Quiet frames still feed Opus so DTX
        // keeps a stable timeline; mute / PTT still stop the track via mic_opus_frame.
        if let Some(frame) = mic_opus_frame(&state, &samples) {
            push_pcm(&user_pcm, &frame);
        }
    }
}

fn push_pcm(slot: &Mutex<VecDeque<i16>>, samples: &[i16]) {
    let mut queue = slot.lock().expect("opus pcm");
    queue.extend(samples);
    while queue.len() > pcm_backlog_samples() {
        queue.pop_front();
    }
}

fn mix_stream_sources(engine: &Engine, fades: &mut HashMap<String, f32>) -> Vec<i16> {
    let sources = engine.sources.lock().expect("stream sources").clone();
    let mut mixed = vec![0i16; AUDIO_FRAME_SAMPLES];
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
            let Some(queue) = map.get_mut(&source.id) else {
                continue;
            };
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
        let sample = left.get(index).copied().unwrap_or(0) as i32
            + right.get(index).copied().unwrap_or(0) as i32;
        out.push(sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    out
}

fn device_failed(errors: &Mutex<Vec<String>>, device: &str, raw: &str) {
    eprintln!("{device}: {raw}");
    let mut list = errors.lock().expect("media errors");
    let message = crate::settings::media_device_notice(device, raw);
    crate::settings::remember_device_error(&mut list, device, &message);
}

fn device_recovered(errors: &Mutex<Vec<String>>, device: &str) {
    crate::settings::forget_device_errors(&mut errors.lock().expect("media errors"), device);
}

fn remember_error(errors: &Mutex<Vec<String>>, message: String) {
    let mut list = errors.lock().expect("media errors");
    if !list.iter().any(|existing| existing == &message) {
        eprintln!("banter: {message}");
        list.push(message);
    }
}

fn remember_loopback_error(errors: &Mutex<Vec<String>>, label: &str, raw: &str) {
    eprintln!("{label}: {raw}");
    remember_error(
        errors,
        format!(
            "{label}: {}",
            crate::settings::media_wasapi_notice(label, raw)
        ),
    );
}

fn remember_screen_error(errors: &Mutex<Vec<String>>, raw: &str) {
    eprintln!("screen: {raw}");
    remember_error(
        errors,
        format!("screen: {}", crate::settings::media_screen_notice(raw)),
    );
}

fn camera_loop(
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    local: Arc<Mutex<Option<(u32, u32, u32, Vec<u8>)>>>,
    errors: Arc<Mutex<Vec<String>>>,
    camera_index: Arc<AtomicU32>,
    camera_preview: Arc<AtomicBool>,
    pictures: Arc<rtc::PictureSlot>,
) {
    let mut camera: Option<nokhwa::Camera> = None;
    let mut opened: Option<u32> = None;
    let mut reported = false;
    // Same contract as screen: first frame (and every size change) must be a keyframe so
    // remotes can decode. A forever `key: false` leaves viewers on an empty bitstream.
    let mut keyed: Option<(u32, u32)> = None;
    while !stop.load(Ordering::Relaxed) {
        let state = unpack(flags.load(Ordering::Relaxed));
        let send = in_place.load(Ordering::Relaxed) && should_send_camera(&state);
        let preview = camera_preview.load(Ordering::Relaxed);
        if !send && !preview {
            camera = None;
            opened = None;
            reported = false;
            keyed = None;
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let mut choice = CameraChoice::new();
        choice.select(camera_index.load(Ordering::Relaxed));
        if choice.needs_reopen(opened) {
            camera = None;
            opened = None;
            reported = false;
            keyed = None;
            *local.lock().expect("camera") = None;
            device_recovered(&errors, "camera");
        }
        if camera.is_none() {
            match open_camera(choice.device_index()) {
                Ok(device) => {
                    camera = Some(device);
                    opened = Some(choice.device_index());
                    keyed = None;
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
        let Some(device) = camera.as_mut() else {
            continue;
        };
        let frame_started = Instant::now();
        match capture_camera(device) {
            Ok((preview_w, preview_h, preview_rgba, width, height, i420)) => {
                if reported {
                    device_recovered(&errors, "camera");
                    reported = false;
                }
                if send && width >= 2 && height >= 2 && !i420.is_empty() {
                    let key = picture_needs_key(&mut keyed, width, height);
                    pictures.store(rtc::RawPicture {
                        width,
                        height,
                        i420: std::sync::Arc::<[u8]>::from(i420),
                        key,
                    });
                } else if !send {
                    keyed = None;
                }
                *local.lock().expect("camera") =
                    Some((choice.device_index(), preview_w, preview_h, preview_rgba));
            }
            // Reported once per failure run; a failing camera fails every frame, about 30 times a second.
            Err(err) if !reported => {
                device_failed(&errors, "camera", &format!("frame: {err}"));
                reported = true;
            }
            Err(_) => {}
        }
        // Pace ~30 FPS from the start of the grab, not after convert — a slow frame
        // must not stack another fixed 33 ms of sleep on top (that is what made OBS /
        // MJPEG cameras feel like ~10–15 FPS).
        const FRAME_BUDGET: Duration = Duration::from_millis(33);
        if let Some(rest) = FRAME_BUDGET.checked_sub(frame_started.elapsed()) {
            thread::sleep(rest);
        }
    }
}

/// True for the first picture at a size, so the encoder emits an IDR. Matching size → delta.
fn picture_needs_key(keyed: &mut Option<(u32, u32)>, width: u32, height: u32) -> bool {
    let key = *keyed != Some((width, height));
    *keyed = Some((width, height));
    key
}

/// High bit marks a nokhwa `CameraIndex::String` device (OBS / some AVFoundation ids).
const STRING_CAMERA_FLAG: u32 = 0x8000_0000;

fn string_camera_id(key: &str) -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    STRING_CAMERA_FLAG | (hasher.finish() as u32 & !STRING_CAMERA_FLAG)
}

fn string_camera_map() -> &'static Mutex<HashMap<u32, String>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<u32, String>>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_string_camera(id: u32, key: String) {
    string_camera_map()
        .lock()
        .expect("string cameras")
        .insert(id, key);
}

fn string_camera_key(id: u32) -> Option<String> {
    string_camera_map()
        .lock()
        .expect("string cameras")
        .get(&id)
        .cloned()
}

/// Real camera names from the operating system. An empty list means none are available.
///
/// Includes OBS Virtual Camera / other virtual devices when the OS exposes them. Virtual
/// names are sorted to the top so they are easy to pick. String-backed nokhwa indices
/// (some macOS / driver paths) are kept via a stable high-bit id.
pub(crate) fn camera_names() -> Vec<(u32, String)> {
    use nokhwa::utils::{ApiBackend, CameraIndex};
    nokhwa::nokhwa_initialize(|_| {});
    let Ok(list) = nokhwa::query(ApiBackend::Auto) else {
        return Vec::new();
    };
    let mut out: Vec<(u32, String)> = list
        .into_iter()
        .filter_map(|info| {
            let human = info.human_name();
            let (index, fallback) = match info.index() {
                CameraIndex::Index(index) => (*index, format!("Camera {index}")),
                CameraIndex::String(key) => {
                    let id = string_camera_id(key);
                    remember_string_camera(id, key.clone());
                    (id, key.clone())
                }
            };
            let name = if human.trim().is_empty() {
                fallback
            } else {
                human
            };
            Some((index, name))
        })
        .collect();
    out.sort_by(|(_, a), (_, b)| {
        is_virtual_camera_name(b)
            .cmp(&is_virtual_camera_name(a))
            .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
    });
    out
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

/// True for OBS Virtual Camera / v4l2loopback-style devices. Prefer raw YUYV over MJPEG there.
pub fn is_virtual_camera_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("obs")
        || lower.contains("virtual")
        || lower.contains("v4l2loopback")
        || lower.contains("unity capture")
        || lower.contains("iriun")
}

/// Picks ~720p capture. Modes at or above 720p beat anything smaller when both exist; then nearest
/// resolution wins; at the same size, FPS near 30 then MJPEG (or raw when `prefer_mjpeg` is false —
/// OBS Virtual Camera stays on YUYV/NV12 instead of a CPU JPEG decode).
pub fn pick_preferred_camera_mode(
    modes: &[CameraModeChoice],
    prefer_mjpeg: bool,
) -> Option<CameraModeChoice> {
    const TARGET_FPS: u32 = 30;
    let target_w = preferred_camera_width();
    let target_h = preferred_camera_height();
    modes.iter().copied().min_by_key(|mode| {
        let undersized = (mode.width < target_w || mode.height < target_h) as u64;
        let dx = mode.width as i64 - target_w as i64;
        let dy = mode.height as i64 - target_h as i64;
        let distance = (dx * dx + dy * dy) as u64;
        let fps_delta = (mode.fps as i64 - TARGET_FPS as i64).unsigned_abs() as u64;
        let mjpeg_rank = if mode.mjpeg == prefer_mjpeg { 0u64 } else { 1 };
        (undersized, distance, fps_delta, mjpeg_rank)
    })
}

fn camera_index_for(id: u32) -> nokhwa::utils::CameraIndex {
    use nokhwa::utils::CameraIndex;
    if id & STRING_CAMERA_FLAG != 0 {
        if let Some(key) = string_camera_key(id) {
            return CameraIndex::String(key);
        }
    }
    CameraIndex::Index(id)
}

fn camera_display_name(id: u32) -> String {
    camera_names()
        .into_iter()
        .find(|(index, _)| *index == id)
        .map(|(_, name)| name)
        .unwrap_or_default()
}

fn open_camera(index: u32) -> Result<nokhwa::Camera, String> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        CameraFormat, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    };
    static CAMERA_DENIED: AtomicBool = AtomicBool::new(false);
    nokhwa::nokhwa_initialize(|granted| {
        if !granted {
            CAMERA_DENIED.store(true, Ordering::Relaxed);
            eprintln!("camera permission was denied");
        }
    });
    if CAMERA_DENIED.load(Ordering::Relaxed) {
        #[cfg(target_os = "macos")]
        {
            return Err(macos::privacy_hint("camera"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Err("camera permission was denied".into());
        }
    }
    let camera_index = camera_index_for(index);
    let prefer_mjpeg = !is_virtual_camera_name(&camera_display_name(index));
    let first_format = if prefer_mjpeg {
        FrameFormat::MJPEG
    } else {
        FrameFormat::YUYV
    };
    let second_format = if prefer_mjpeg {
        FrameFormat::YUYV
    } else {
        FrameFormat::MJPEG
    };
    // Open with a flexible request so we can list modes, then switch to ~720p (or nearest).
    let mut camera = nokhwa::Camera::new(
        camera_index.clone(),
        RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(preferred_camera_width(), preferred_camera_height()),
            first_format,
            30,
        ))),
    )
    .or_else(|_| {
        nokhwa::Camera::new(
            camera_index.clone(),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(preferred_camera_width(), preferred_camera_height()),
                second_format,
                30,
            ))),
        )
    })
    .or_else(|_| {
        nokhwa::Camera::new(
            camera_index,
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
        if let Some(pick) = pick_preferred_camera_mode(&modes, prefer_mjpeg) {
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
    let image = frame
        .decode_image::<RgbFormat>()
        .map_err(|err| err.to_string())?;
    Ok((image.width(), image.height(), image.into_raw()))
}

/// Shrink a camera RGBA frame for the local preview tile. Not a wire codec — outbound video is I420 → H.264.
pub fn camera_preview_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(u32, u32, Vec<u8>), String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "frame is too large".to_string())?;
    if rgba.len() != expected {
        return Err("rgba size does not match the frame".into());
    }
    let (preview, out_w, out_h) = encode::fit_rgba(rgba, width, height, 640);
    Ok((out_w, out_h, preview))
}

/// True when a Constrained Baseline hardware H.264 encoder is visible. OpenH264 is always the fallback.
pub fn hardware_encoder_available() -> bool {
    !encode::hardware_probe().is_empty()
}

/// Hardware backend keys this process can see (`nvenc`, `vaapi`, `mediafoundation`).
pub fn hardware_encoder_names() -> Vec<&'static str> {
    encode::hardware_probe()
}

/// The local preview slot holds the device index with each frame, so a frame captured just before the user
/// picked another camera is never shown as the new camera.
pub fn current_camera_frame(
    slot: Option<(u32, u32, u32, Vec<u8>)>,
    chosen_device: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    match slot {
        Some((device, width, height, rgba)) if device == chosen_device => {
            Some((width, height, rgba))
        }
        _ => None,
    }
}

fn capture_camera(
    camera: &mut nokhwa::Camera,
) -> Result<(u32, u32, Vec<u8>, u32, u32, Vec<u8>), String> {
    let started = Instant::now();
    let frame = camera.frame().map_err(|err| err.to_string())?;
    let (width, height, rgb) = decode_camera_frame(&frame)?;
    // Stay on packed RGB → I420 (SIMD). The old path expanded every pixel to RGBA
    // then converted, which stacked lag on OBS Virtual Camera / MJPEG webcams.
    let (i420, out_w, out_h) = if width >= 2 && height >= 2 {
        encode::fit_i420_rgb(&rgb, width, height, 1280)
    } else {
        (Vec::new(), 0, 0)
    };
    let (preview, preview_w, preview_h) = encode::fit_rgb_preview(&rgb, width, height, 640);
    if preview_w < 2 || preview_h < 2 {
        return Err("camera preview scale failed".into());
    }
    crate::profiler::record_media("camera_capture", started.elapsed(), preview.len());
    Ok((preview_w, preview_h, preview, out_w, out_h, i420))
}

/// What the capture thread should record. `Portal` opens the system ScreenCast dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamPick {
    Portal,
    Source(CaptureKind, u32),
}

/// Linux Wayland has a native ScreenCast dialog. Everywhere else the Banter picker chooses a source.
pub fn native_screen_picker() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    session.eq_ignore_ascii_case("wayland") || display.to_lowercase().contains("wayland")
}

enum Grabber {
    Monitor { monitor: xcap::Monitor },
    Window { window: xcap::Window },
}

enum Feed {
    Record {
        recorder: xcap::VideoRecorder,
        frames: mpsc::Receiver<xcap::Frame>,
    },
    Still {
        grabber: Grabber,
    },
    #[cfg(target_os = "linux")]
    Cast(screencast::CastStream),
    /// DRM kmsgrab → ffmpeg I420 (Linux monitors only; see `kmsgrab`).
    #[cfg(target_os = "linux")]
    Kms(kmsgrab::KmsStream),
}

impl Drop for Feed {
    fn drop(&mut self) {
        if let Feed::Record { recorder, .. } = self {
            let _ = recorder.stop();
        }
    }
}

fn screen_loop(
    flags: Arc<AtomicU32>,
    in_place: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    local: Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>,
    errors: Arc<Mutex<Vec<String>>>,
    stream_edge: Arc<AtomicU32>,
    stream_interval: Arc<AtomicU64>,
    stream_target: Arc<Mutex<Option<StreamPick>>>,
    stream_epoch: Arc<AtomicU64>,
    capture_failed: Arc<AtomicBool>,
    engine: Arc<Engine>,
    convert: Arc<screen_pipeline::ScreenConvertTx>,
) {
    let mut feed: Option<(StreamPick, Feed)> = None;
    let mut latest: Option<(u32, u32, Vec<u8>)> = None;
    let mut fresh = false;
    let mut next_encode = Instant::now();
    let mut failed: Option<StreamPick> = None;
    let mut seen_epoch = stream_epoch.load(Ordering::Relaxed);
    // After a feed is dropped, give the OS / portal a beat before reopening the same source.
    let mut settle_until: Option<Instant> = None;
    // Only the Linux Cast feed reads this; Record/Still never need a cast timeout.
    #[cfg(target_os = "linux")]
    let mut cast_since: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        let epoch = stream_epoch.load(Ordering::Relaxed);
        if epoch != seen_epoch {
            // End → Go Live (or a source change) must be allowed to reopen even if the last
            // open failed for the same StreamPick while the UI still showed live.
            failed = None;
            seen_epoch = epoch;
            convert.reset();
        }
        let state = unpack(flags.load(Ordering::Relaxed));
        let enabled = in_place.load(Ordering::Relaxed) && should_send_screen(&state);
        let pick = if enabled {
            stream_target.lock().expect("stream target").clone()
        } else {
            None
        };
        let Some(pick) = pick else {
            if feed.is_some() {
                settle_until = Some(Instant::now() + Duration::from_millis(150));
            }
            feed = None;
            latest = None;
            fresh = false;
            failed = None;
            convert.reset();
            *local.lock().expect("screen") = None;
            #[cfg(target_os = "linux")]
            {
                cast_since = None;
            }
            thread::sleep(Duration::from_millis(40));
            continue;
        };
        if let Some(until) = settle_until {
            if Instant::now() < until {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            settle_until = None;
        }
        let edge = stream_edge.load(Ordering::Relaxed);
        let interval = stream_interval.load(Ordering::Relaxed).max(1);
        if feed.as_ref().is_none_or(|(current, _)| *current != pick) {
            let had_feed = feed.is_some();
            feed = None;
            latest = None;
            fresh = false;
            convert.reset();
            #[cfg(target_os = "linux")]
            {
                cast_since = None;
            }
            if had_feed {
                // xcap / PipeWire need the previous recorder gone before the next start.
                thread::sleep(Duration::from_millis(100));
            }
            if failed == Some(pick) {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            match open_feed(pick, &stream_target, &stop, edge, interval) {
                Ok(Some(opened)) => {
                    failed = None;
                    #[cfg(target_os = "linux")]
                    {
                        cast_since = Some(Instant::now());
                    }
                    feed = Some((pick, opened));
                    next_encode = Instant::now();
                    convert.reset();
                }
                Ok(None) => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(err) => {
                    remember_screen_error(&errors, &err);
                    failed = Some(pick);
                    capture_failed.store(true, Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
        }
        match feed.as_mut() {
            Some((_, Feed::Record { frames, .. })) => {
                while let Ok(frame) = frames.try_recv() {
                    let need = (frame.width as usize)
                        .saturating_mul(frame.height as usize)
                        .saturating_mul(4);
                    if frame.width >= 2 && frame.height >= 2 && frame.raw.len() >= need {
                        latest = Some((frame.width, frame.height, frame.raw));
                        fresh = true;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Some((_, Feed::Cast(cast))) => {
                if let Some(err) = cast.take_fault() {
                    remember_screen_error(&errors, &err);
                    capture_failed.store(true, Ordering::Relaxed);
                    failed = Some(pick);
                    feed = None;
                    cast_since = None;
                    convert.reset();
                    continue;
                }
                // Leave the raw frame in the portal slot until publish time.
                // Scaling on the convert thread — never on the PipeWire callback.
                if cast.has_frame() {
                    cast_since = None;
                } else if latest.is_none()
                    && cast_since.is_some_and(|started| started.elapsed() > Duration::from_secs(3))
                {
                    remember_screen_error(&errors, "the desktop sent no picture");
                    capture_failed.store(true, Ordering::Relaxed);
                    failed = Some(pick);
                    feed = None;
                    cast_since = None;
                    convert.reset();
                    continue;
                }
            }
            #[cfg(target_os = "linux")]
            Some((_, Feed::Kms(kms))) => {
                if let Some(err) = kms.take_fault() {
                    remember_screen_error(&errors, &err);
                    capture_failed.store(true, Ordering::Relaxed);
                    failed = Some(pick);
                    feed = None;
                    cast_since = None;
                    convert.reset();
                    continue;
                }
                if kms.has_frame() {
                    cast_since = None;
                } else if latest.is_none()
                    && cast_since.is_some_and(|started| started.elapsed() > Duration::from_secs(3))
                {
                    remember_screen_error(&errors, "kmsgrab sent no picture");
                    capture_failed.store(true, Ordering::Relaxed);
                    failed = Some(pick);
                    feed = None;
                    cast_since = None;
                    convert.reset();
                    continue;
                }
            }
            _ => {}
        }
        if Instant::now() < next_encode {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let show_cursor = engine.show_cursor.load(Ordering::Relaxed);
        let want_preview = engine.show_preview.load(Ordering::Relaxed);
        let Some((_, current)) = feed.as_ref() else {
            continue;
        };
        match current {
            Feed::Still { grabber } => {
                next_encode = Instant::now() + Duration::from_millis(interval);
                let grabbed_at = Instant::now();
                match capture_rgba(grabber, show_cursor) {
                    Ok((width, height, rgba)) => convert.push_rgba(
                        width,
                        height,
                        rgba,
                        edge,
                        want_preview,
                        grabbed_at,
                    ),
                    Err(err) => remember_screen_error(&errors, &err),
                }
            }
            #[cfg(target_os = "linux")]
            Feed::Cast(cast) => {
                let grabbed_at = Instant::now();
                let Some(raw) = cast.take_raw() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                next_encode = Instant::now() + Duration::from_millis(interval);
                convert.push_cast(raw, edge, want_preview, grabbed_at);
            }
            #[cfg(target_os = "linux")]
            Feed::Kms(kms) => {
                let grabbed_at = Instant::now();
                let Some(raw) = kms.take_raw() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                next_encode = Instant::now() + Duration::from_millis(interval);
                // I420 from ffmpeg — same convert path as PipeWire Cast.
                convert.push_cast(raw, edge, want_preview, grabbed_at);
            }
            Feed::Record { .. } => {
                if !fresh {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                fresh = false;
                next_encode = Instant::now() + Duration::from_millis(interval);
                let Some((width, height, rgba)) = latest.take() else {
                    continue;
                };
                // Keep a copy only if DXGI delivers another frame before convert finishes;
                // latest-wins convert slot already drops stale jobs.
                convert.push_rgba(
                    width,
                    height,
                    rgba,
                    edge,
                    want_preview,
                    Instant::now(),
                );
            }
        }
    }
}

fn open_feed(
    pick: StreamPick,
    stream_target: &Mutex<Option<StreamPick>>,
    stop: &AtomicBool,
    edge: u32,
    interval_ms: u64,
) -> Result<Option<Feed>, String> {
    #[cfg(target_os = "macos")]
    macos::ensure_screen_recording()?;
    let fps = (1000 / interval_ms.max(1)).clamp(1, 60) as u32;
    // Linux monitors only: optional DRM kmsgrab → ffmpeg I420 before PipeWire/xcap.
    // Window picks stay on Still — kmsgrab is a CRTC/plane, not a single window.
    #[cfg(target_os = "linux")]
    if matches!(
        pick,
        StreamPick::Portal | StreamPick::Source(CaptureKind::Screen, _)
    ) {
        if let Some(kms) = kmsgrab::try_open(fps)? {
            let current = stream_target.lock().expect("stream target").clone();
            if stop.load(Ordering::Relaxed) || current != Some(pick) {
                return Ok(None);
            }
            return Ok(Some(Feed::Kms(kms)));
        }
    }
    #[cfg(target_os = "linux")]
    if native_screen_picker() {
        let cast = screencast::CastStream::open(edge, fps)?;
        let current = stream_target.lock().expect("stream target").clone();
        if stop.load(Ordering::Relaxed) || current != Some(pick) {
            return Ok(None);
        }
        return Ok(Some(Feed::Cast(cast)));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (edge, interval_ms, fps);
    }
    match pick {
        StreamPick::Portal => {
            let monitors = xcap::Monitor::all().map_err(|err| err.to_string())?;
            let monitor = monitors
                .into_iter()
                .next()
                .ok_or_else(|| "no screen".to_string())?;
            start_recorder(pick, monitor, stream_target, stop)
        }
        StreamPick::Source(CaptureKind::Screen, id) => {
            let monitors = xcap::Monitor::all().map_err(|err| err.to_string())?;
            let monitor = monitors
                .into_iter()
                .find(|monitor| monitor.id().ok() == Some(id))
                .ok_or_else(|| "that screen is gone".to_string())?;
            // X11 has no ScreenCast session. A paced grab is the capture; Wayland and Windows use a recorder.
            if cfg!(target_os = "linux") && !native_screen_picker() {
                Ok(Some(Feed::Still {
                    grabber: Grabber::Monitor { monitor },
                }))
            } else {
                start_recorder(pick, monitor, stream_target, stop)
            }
        }
        StreamPick::Source(CaptureKind::Window, id) => Ok(Some(Feed::Still {
            grabber: open_grabber(CaptureKind::Window, id)?,
        })),
    }
}

fn start_recorder(
    pick: StreamPick,
    monitor: xcap::Monitor,
    stream_target: &Mutex<Option<StreamPick>>,
    stop: &AtomicBool,
) -> Result<Option<Feed>, String> {
    // On Wayland this blocks in the system ScreenCast dialog, then delivers a PipeWire stream.
    // On Windows, DXGI Desktop Duplication (`video_recorder`) often returns E_INVALIDARG
    // (0x80070057) when the D3D device is not created on the output's adapter — same class
    // as xcap#264. Fall back to paced GDI `capture_image` (already used for window share)
    // so Go Live still works; Linux Cast / X11 paths never take this Windows branch.
    match monitor.video_recorder() {
        Ok((recorder, frames)) => {
            let current = stream_target.lock().expect("stream target").clone();
            if stop.load(Ordering::Relaxed) || current != Some(pick) {
                let _ = recorder.stop();
                return Ok(None);
            }
            recorder.start().map_err(|err| err.to_string())?;
            Ok(Some(Feed::Record { recorder, frames }))
        }
        Err(err) => {
            if crate::settings::prefer_still_screen_after_recorder_error() {
                eprintln!(
                    "screen: video_recorder failed ({err}); falling back to paced capture_image"
                );
                Ok(Some(Feed::Still {
                    grabber: Grabber::Monitor { monitor },
                }))
            } else {
                Err(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod picture_key_tests {
    use super::picture_needs_key;

    #[test]
    fn first_frame_and_size_change_are_keyframes() {
        let mut keyed = None;
        assert!(picture_needs_key(&mut keyed, 1280, 720));
        assert!(!picture_needs_key(&mut keyed, 1280, 720));
        assert!(picture_needs_key(&mut keyed, 640, 360));
        assert!(!picture_needs_key(&mut keyed, 640, 360));
        keyed = None;
        assert!(
            picture_needs_key(&mut keyed, 640, 360),
            "reopen after stop must key again"
        );
    }
}

fn capture_rgba(grabber: &Grabber, show_cursor: bool) -> Result<(u32, u32, Vec<u8>), String> {
    let image = match grabber {
        Grabber::Monitor { monitor, .. } => {
            monitor.capture_image().map_err(|err| err.to_string())?
        }
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
    Ok((width, height, rgba))
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
///
/// One display for the process. Opening it on every frame prints
/// "Authorization required, but no authorization protocol specified" and leaks a connection.
#[cfg(target_os = "linux")]
fn cursor_point_x11() -> Option<(i32, i32)> {
    use std::os::raw::{c_int, c_uint, c_ulong, c_void};

    #[link(name = "X11")]
    unsafe extern "C" {
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
        let display = x11_display();
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
        if ok == 0 {
            None
        } else {
            Some((root_x, root_y))
        }
    }
}

#[cfg(target_os = "linux")]
fn x11_display() -> *mut std::os::raw::c_void {
    use std::os::raw::{c_char, c_void};
    use std::sync::OnceLock;
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    }
    struct Display(*mut c_void);
    unsafe impl Send for Display {}
    unsafe impl Sync for Display {}
    static DISPLAY: OnceLock<Display> = OnceLock::new();
    DISPLAY
        .get_or_init(|| unsafe { Display(XOpenDisplay(std::ptr::null())) })
        .0
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
