//! Publisher PeerConnection. Voice offers to the SFU. A direct call offers if
//! we placed it, and answers if we accepted it. Encoded media sits in a bounded
//! queue; a full queue drops the frame and counts as congestion.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use banter_server::{
    mids_by_track, sample_bytes, simulcast_extensions, video_rtp_duration_ms, ClientSignal,
    LayerId, MediaKind, MediaPlace, ServerEvent, CAMERA_SSRC_Q, H264_PT, OPUS_PT, SCREEN_SSRC_Q,
    STREAM_AUDIO_SSRC, USER_AUDIO_SSRC,
};
use bytes::Bytes;
use rtc::rtp::codec::h264::H264Packet;
use rtc::rtp::packetizer::Depacketizer;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceCandidateInit, RTCIceServer, RTCSessionDescription, StatsSelector,
};
use webrtc::peer_connection::RTCStatsReportEntry;

use crate::logic::resample_mono_i16;
use crate::profiler;

use super::encode::{EncoderPick, LayerEncoder, OpusListen, OpusTrack, OPUS_FRAME};
use super::qos::SenderQos;

const FLAG_DEAFENED: u32 = 2;

pub struct RawPicture {
    pub width: u32,
    pub height: u32,
    pub i420: Arc<[u8]>,
    pub key: bool,
}

pub struct PictureSlot {
    picture: Mutex<Option<RawPicture>>,
    generation: AtomicU64,
}

impl PictureSlot {
    pub fn new() -> Self {
        Self {
            picture: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    pub fn store(&self, picture: RawPicture) {
        *self.picture.lock().expect("picture") = Some(picture);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Clones the slot handle only when a newer picture has been stored.
    /// The I420 plane is `Arc`, so Full-layer encode can borrow without a copy.
    fn load_since(&self, seen: u64) -> Option<(u64, RawPicture)> {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 || generation == seen {
            return None;
        }
        let picture = self.picture.lock().expect("picture").clone()?;
        Some((self.generation.load(Ordering::Acquire), picture))
    }
}

/// Latest-wins DMA-BUF frame for Linux VA-API hardware encode (M4).
#[cfg(target_os = "linux")]
pub struct HwPictureSlot {
    frame: Mutex<Option<(super::dmabuf::OwnedDmaBufFrame, u32)>>,
    generation: AtomicU64,
}

#[cfg(target_os = "linux")]
impl HwPictureSlot {
    pub fn new() -> Self {
        Self {
            frame: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    pub fn store(&self, frame: super::dmabuf::OwnedDmaBufFrame, edge: u32) {
        *self.frame.lock().expect("hw picture") = Some((frame, edge));
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn clear(&self) {
        *self.frame.lock().expect("hw picture") = None;
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn take_since(
        &self,
        seen: u64,
    ) -> Option<(u64, super::dmabuf::OwnedDmaBufFrame, u32)> {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 || generation == seen {
            return None;
        }
        let (frame, edge) = self.frame.lock().expect("hw picture").take()?;
        Some((self.generation.load(Ordering::Acquire), frame, edge))
    }
}

#[cfg(target_os = "linux")]
fn screen_hw_slot() -> &'static HwPictureSlot {
    use std::sync::OnceLock;
    static SLOT: OnceLock<HwPictureSlot> = OnceLock::new();
    SLOT.get_or_init(HwPictureSlot::new)
}

#[cfg(target_os = "linux")]
pub fn store_screen_dmabuf(frame: super::dmabuf::OwnedDmaBufFrame, edge: u32) {
    screen_hw_slot().store(frame, edge);
}

#[cfg(target_os = "linux")]
pub fn clear_screen_dmabuf() {
    screen_hw_slot().clear();
}

impl Clone for RawPicture {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            i420: Arc::clone(&self.i420),
            key: self.key,
        }
    }
}

pub struct RemotePicture {
    pub sender: i64,
    pub kind: MediaKind,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub layer: LayerId,
}

/// Live encode knobs from Voice & Video / Stream options (OBS-like).
#[derive(Clone, Copy, Debug)]
pub struct EncodeSettings {
    pub full_bps: u32,
    pub pick: EncoderPick,
    pub keyframe_ms: u64,
    pub fps: u32,
    pub cbr: bool,
    pub pref_label: &'static str,
    pub rate_label: &'static str,
}

impl Default for EncodeSettings {
    fn default() -> Self {
        Self {
            full_bps: 2_500_000,
            pick: EncoderPick::Auto,
            keyframe_ms: 1_000,
            fps: 30,
            cbr: true,
            pref_label: "auto",
            rate_label: "cbr",
        }
    }
}

struct Control {
    wanted: Mutex<HashMap<MediaKind, Option<Vec<LayerId>>>>,
    peer_cap: Mutex<HashMap<MediaKind, LayerId>>,
    subscribers: AtomicU32,
    topology: AtomicU8,
    ladder: Mutex<String>,
    backend: Mutex<String>,
    user_bytes: AtomicU64,
    stream_bytes: AtomicU64,
    camera_bytes: AtomicU64,
    screen_bytes: AtomicU64,
    keyframes: AtomicU64,
    encode_ms: AtomicU32,
    congested_drops: AtomicU32,
    full_bps: AtomicU32,
    encoder_pick: AtomicU8,
    keyframe_ms: AtomicU64,
    encode_fps: AtomicU32,
    rate_cbr: AtomicBool,
    encode_gen: AtomicU64,
    encode_w: AtomicU32,
    encode_h: AtomicU32,
    publishing_screen: AtomicBool,
    publishing_camera: AtomicBool,
    encoder_pref: Mutex<String>,
    rate_control: Mutex<String>,
    /// Set when WantedLayers gains a subscriber so the next AU is an IDR for late joiners.
    force_key: AtomicBool,
    /// Last WebRTC sender sample used by `SenderQos::observe` (f32 bits / ms / f32 bits).
    bwe_loss: AtomicU32,
    bwe_rtt_ms: AtomicU32,
    bwe_available_ratio: AtomicU32,
    /// Cumulative outbound `pli_count` from the last stats poll; rise → force_key.
    last_pli: AtomicU32,
}

impl Control {
    fn new() -> Self {
        Self {
            wanted: Mutex::new(HashMap::new()),
            peer_cap: Mutex::new(HashMap::new()),
            subscribers: AtomicU32::new(0),
            topology: AtomicU8::new(0),
            ladder: Mutex::new("nominal".into()),
            backend: Mutex::new(initial_backend()),
            user_bytes: AtomicU64::new(0),
            stream_bytes: AtomicU64::new(0),
            camera_bytes: AtomicU64::new(0),
            screen_bytes: AtomicU64::new(0),
            keyframes: AtomicU64::new(0),
            encode_ms: AtomicU32::new(0),
            congested_drops: AtomicU32::new(0),
            full_bps: AtomicU32::new(2_500_000),
            encoder_pick: AtomicU8::new(0),
            keyframe_ms: AtomicU64::new(1_000),
            encode_fps: AtomicU32::new(30),
            rate_cbr: AtomicBool::new(true),
            encode_gen: AtomicU64::new(0),
            encode_w: AtomicU32::new(0),
            encode_h: AtomicU32::new(0),
            publishing_screen: AtomicBool::new(false),
            publishing_camera: AtomicBool::new(false),
            encoder_pref: Mutex::new("auto".into()),
            rate_control: Mutex::new("cbr".into()),
            force_key: AtomicBool::new(false),
            bwe_loss: AtomicU32::new(0),
            bwe_rtt_ms: AtomicU32::new(30),
            bwe_available_ratio: AtomicU32::new(1.0f32.to_bits()),
            last_pli: AtomicU32::new(0),
        }
    }

    fn apply_encode(&self, settings: EncodeSettings) {
        let pick = settings.pick.as_u8();
        let mut changed = false;
        if self.full_bps.swap(settings.full_bps.max(30_000), Ordering::Relaxed) != settings.full_bps
        {
            changed = true;
        }
        if self.encoder_pick.swap(pick, Ordering::Relaxed) != pick {
            changed = true;
        }
        let key_ms = settings.keyframe_ms.clamp(500, 10_000);
        if self.keyframe_ms.swap(key_ms, Ordering::Relaxed) != key_ms {
            changed = true;
        }
        let fps = settings.fps.clamp(1, 144);
        if self.encode_fps.swap(fps, Ordering::Relaxed) != fps {
            changed = true;
        }
        // CBR/VBR now reaches Linux HW ffmpeg args; reopen layers when it flips.
        if self.rate_cbr.swap(settings.cbr, Ordering::Relaxed) != settings.cbr {
            changed = true;
        }
        {
            let mut pref = self.encoder_pref.lock().expect("encoder pref");
            if *pref != settings.pref_label {
                *pref = settings.pref_label.into();
                changed = true;
            }
        }
        {
            let mut rate = self.rate_control.lock().expect("rate control");
            if *rate != settings.rate_label {
                *rate = settings.rate_label.into();
            }
        }
        if changed {
            self.encode_gen.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn encoder_pick(&self) -> EncoderPick {
        EncoderPick::from_u8(self.encoder_pick.load(Ordering::Relaxed))
    }

    fn layer_bps(&self, layer: LayerId) -> u32 {
        super::encode::layer_target_bps(self.full_bps.load(Ordering::Relaxed), layer)
    }
}

enum Cmd {
    Session {
        place: Option<MediaPlace>,
        me: i64,
        offerer: bool,
        /// True for DM DirectCall and guild opt-in P2P mesh. Must be part of the
        /// session key: offerer often stays the same when flipping P2P↔SFU (lower
        /// user id, or 2→3 people), and a stuck PeerConnection leaves streams dead.
        mesh: bool,
    },
    Event(ServerEvent),
    Stop,
}

enum Outgoing {
    Audio {
        kind: MediaKind,
        bytes: Vec<u8>,
    },
    Video {
        kind: MediaKind,
        layer: LayerId,
        bytes: Vec<u8>,
    },
}

pub struct Rtc {
    cmd: UnboundedSender<Cmd>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    control: Arc<Control>,
    /// Last session handed to the network thread. The UI calls this every frame.
    /// Tuple is (place, me, offerer, mesh).
    session: Mutex<Option<(Option<MediaPlace>, i64, bool, bool)>>,
}

impl Rtc {
    pub fn start(
        signals: Sender<ClientSignal>,
        user_pcm: Arc<Mutex<VecDeque<i16>>>,
        stream_pcm: Arc<Mutex<VecDeque<i16>>>,
        camera: Arc<PictureSlot>,
        screen: Arc<PictureSlot>,
        playback: Arc<Mutex<VecDeque<f32>>>,
        remote: Arc<Mutex<Vec<RemotePicture>>>,
        output_rate: Arc<AtomicU32>,
        flags: Arc<AtomicU32>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        // Voice and video do not share a queue. A full video queue used to
        // drop Opus, and H.264 on the voice thread held the next mic packet
        // until the screen frame finished.
        let (audio_tx, audio_rx) = tokio::sync::mpsc::channel(64);
        let (video_tx, video_rx) = tokio::sync::mpsc::channel(16);
        let control = Arc::new(Control::new());
        let qos = Arc::new(Mutex::new(SenderQos::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let audio = thread::Builder::new()
            .name("banter-audio".into())
            .spawn({
                let control = Arc::clone(&control);
                let qos = Arc::clone(&qos);
                let stop = Arc::clone(&stop);
                move || audio_encode_loop(stop, control, qos, audio_tx, user_pcm, stream_pcm)
            })
            .expect("audio encode thread");
        let video = thread::Builder::new()
            .name("banter-video".into())
            .spawn({
                let control = Arc::clone(&control);
                let qos = Arc::clone(&qos);
                let stop = Arc::clone(&stop);
                move || video_encode_loop(stop, control, qos, video_tx, camera, screen)
            })
            .expect("video encode thread");
        let net = thread::Builder::new()
            .name("banter-rtc".into())
            .spawn({
                let control = Arc::clone(&control);
                move || {
                    let runtime = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            eprintln!("webrtc runtime: {err}");
                            return;
                        }
                    };
                    runtime.block_on(run(
                        cmd_rx,
                        audio_rx,
                        video_rx,
                        signals,
                        control,
                        playback,
                        remote,
                        output_rate,
                        flags,
                    ));
                }
            })
            .expect("rtc thread");
        Self {
            cmd: cmd_tx,
            stop,
            control: Arc::clone(&control),
            threads: vec![audio, video, net],
            session: Mutex::new(None),
        }
    }

    pub fn apply_encode(&self, settings: EncodeSettings) {
        self.control.apply_encode(settings);
    }

    pub fn note_publishing(&self, camera: bool, screen: bool) {
        self.control
            .publishing_camera
            .store(camera, Ordering::Relaxed);
        self.control
            .publishing_screen
            .store(screen, Ordering::Relaxed);
    }

    pub fn set_session(&self, place: Option<MediaPlace>, me: i64, offerer: bool, mesh: bool) {
        let next = (place, me, offerer, mesh);
        let mut current = self.session.lock().expect("rtc session");
        if *current == Some(next) {
            return;
        }
        *current = Some(next);
        // Drop SFU WantedLayers / P2P caps from the prior topology so encode does not
        // stay gated on an empty dynacast set after a mesh flip.
        self.control.wanted.lock().expect("wanted").clear();
        self.control.peer_cap.lock().expect("cap").clear();
        self.control.subscribers.store(0, Ordering::Relaxed);
        self.control.last_pli.store(0, Ordering::Relaxed);
        self.control.bwe_loss.store(0, Ordering::Relaxed);
        self.control.bwe_rtt_ms.store(30, Ordering::Relaxed);
        self.control
            .bwe_available_ratio
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        self.control.force_key.store(true, Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::Session {
            place,
            me,
            offerer,
            mesh,
        });
    }

    pub fn ingest(&self, event: ServerEvent) {
        let _ = self.cmd.send(Cmd::Event(event));
    }
}

impl Drop for Rtc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::Stop);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn audio_encode_loop(
    stop: Arc<AtomicBool>,
    control: Arc<Control>,
    qos: Arc<Mutex<SenderQos>>,
    audio_tx: tokio::sync::mpsc::Sender<Outgoing>,
    user_pcm: Arc<Mutex<VecDeque<i16>>>,
    stream_pcm: Arc<Mutex<VecDeque<i16>>>,
) {
    let mut user = OpusTrack::user_audio().ok();
    let mut stream = OpusTrack::stream_audio().ok();
    // Stream Opus held until screen grab→publish lag elapses (≤5 s). User audio
    // stays interactive and is never delayed here.
    let mut stream_delay: VecDeque<(Instant, Vec<i16>)> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        let mut sent = false;
        if let Some(encoder) = user.as_mut() {
            if let Some(frame) = take_frame(&user_pcm) {
                sent = true;
                if let Ok(packet) = encoder.encode(&frame) {
                    control
                        .user_bytes
                        .fetch_add(packet.len() as u64, Ordering::Relaxed);
                    let _ = audio_tx.try_send(Outgoing::Audio {
                        kind: MediaKind::UserAudio,
                        bytes: packet,
                    });
                }
            }
        }
        let ratio = qos.lock().expect("qos").targets().stream_audio;
        let stream_wanted = wanted_of(&control, MediaKind::StreamAudio);
        let publishing_screen = control.publishing_screen.load(Ordering::Relaxed);
        if !publishing_screen {
            super::screen_lag::clear_pipeline_lag();
            super::capture_metrics::clear();
        }
        if ratio > 0.0
            && stream_wanted
                .map(|layers| !layers.is_empty())
                .unwrap_or(true)
        {
            if let Some(encoder) = stream.as_mut() {
                let _ = encoder.set_ratio(ratio);
                let lag = Duration::from_millis(super::screen_lag::pipeline_lag_ms());
                while let Some(frame) = take_frame(&stream_pcm) {
                    stream_delay.push_back((Instant::now() + lag, frame));
                }
                // Cap hold at ~5 s of Opus frames (50 frames/s × 5 × 960 samples).
                let max_frames = (super::screen_lag::STREAM_AUDIO_MAX_LAG_MS as usize / 20).max(1);
                while stream_delay.len() > max_frames {
                    stream_delay.pop_front();
                }
                let now = Instant::now();
                while stream_delay
                    .front()
                    .is_some_and(|(ready, _)| *ready <= now)
                {
                    let Some((_, frame)) = stream_delay.pop_front() else {
                        break;
                    };
                    sent = true;
                    if let Ok(packet) = encoder.encode(&frame) {
                        control
                            .stream_bytes
                            .fetch_add(packet.len() as u64, Ordering::Relaxed);
                        let _ = audio_tx.try_send(Outgoing::Audio {
                            kind: MediaKind::StreamAudio,
                            bytes: packet,
                        });
                    }
                }
            }
        } else {
            stream_delay.clear();
            while take_frame(&stream_pcm).is_some() {}
        }
        if !sent {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn video_encode_loop(
    stop: Arc<AtomicBool>,
    control: Arc<Control>,
    qos: Arc<Mutex<SenderQos>>,
    video_tx: tokio::sync::mpsc::Sender<Outgoing>,
    camera: Arc<PictureSlot>,
    screen: Arc<PictureSlot>,
) {
    let mut camera_enc: HashMap<LayerId, (u32, u32, LayerEncoder)> = HashMap::new();
    let mut screen_enc: HashMap<LayerId, (u32, u32, LayerEncoder)> = HashMap::new();
    let mut camera_gen = 0u64;
    let mut screen_gen = 0u64;
    let mut encode_cfg_gen = 0u64;
    let mut last_keyframe = Instant::now();
    let mut ticks = 0u32;
    let mut screen_idle_ticks = 0u32;
    let mut reported = Instant::now();
    let mut user_mark = 0u64;
    let mut stream_mark = 0u64;
    let mut camera_mark = 0u64;
    let mut screen_mark = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let encode_started = Instant::now();
        let (camera_layers, screen_layers) = {
            let qos = qos.lock().expect("qos");
            let camera = if control.publishing_camera.load(Ordering::Relaxed) {
                highest_first(qos.active_layers(
                    MediaKind::Camera,
                    wanted_of(&control, MediaKind::Camera).as_deref(),
                    peer_cap(&control, MediaKind::Camera),
                ))
            } else {
                Vec::new()
            };
            let screen = if control.publishing_screen.load(Ordering::Relaxed) {
                highest_first(qos.active_layers(
                    MediaKind::Screen,
                    wanted_of(&control, MediaKind::Screen).as_deref(),
                    peer_cap(&control, MediaKind::Screen),
                ))
            } else {
                Vec::new()
            };
            (camera, screen)
        };
        // Drop idle encoders so the next publish starts on a fresh IDR instead of
        // P-frames that reference a GOP remotes never saw (camera toggle / Go Live restart).
        if camera_layers.is_empty() {
            camera_enc.clear();
        }
        if screen_layers.is_empty() {
            screen_enc.clear();
            screen_idle_ticks = screen_idle_ticks.saturating_add(1);
        } else {
            screen_idle_ticks = 0;
        }
        let cfg_gen = control.encode_gen.load(Ordering::Relaxed);
        if cfg_gen != encode_cfg_gen {
            camera_enc.clear();
            screen_enc.clear();
            encode_cfg_gen = cfg_gen;
            last_keyframe = Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now);
        }
        let key_ms = control.keyframe_ms.load(Ordering::Relaxed).max(500);
        // Periodic keys come from encoder GOP / IntraFramePeriod. Only arm force_key
        // here when the GOP is overdue *and* we are on software (HW reopen is reserved
        // for join/PLI/mesh via control.force_key). Overdue soft keys keep OpenH264
        // honest if IntraFramePeriod drifts.
        let periodic_soft = last_keyframe.elapsed() >= Duration::from_millis(key_ms);
        let mut keyed = false;
        keyed |= encode_video(
            &control,
            &video_tx,
            &camera,
            &mut camera_enc,
            &mut camera_gen,
            MediaKind::Camera,
            false,
            &camera_layers,
            periodic_soft,
        );
        keyed |= encode_video(
            &control,
            &video_tx,
            &screen,
            &mut screen_enc,
            &mut screen_gen,
            MediaKind::Screen,
            true,
            &screen_layers,
            periodic_soft,
        );
        if keyed {
            last_keyframe = Instant::now();
        }
        let ms = encode_started.elapsed().as_secs_f32() * 1000.0;
        control.encode_ms.store(ms.to_bits(), Ordering::Relaxed);
        ticks = ticks.wrapping_add(1);
        let mut qos = qos.lock().expect("qos");
        if ticks % 25 == 0 {
            let drops = control.congested_drops.swap(0, Ordering::Relaxed);
            // ~0.5s with no screen layers and no queue pressure: treat Go Live as stopped and
            // clear ScreenStop so a quick restart is not held down by the old climb.
            if screen_idle_ticks >= 100 && drops == 0 && !qos.targets().screen_published {
                qos.reset_ladder();
            } else {
                let loss = f32::from_bits(control.bwe_loss.load(Ordering::Relaxed))
                    .max(if drops > 0 { 0.08 } else { 0.0 });
                let rtt = control.bwe_rtt_ms.load(Ordering::Relaxed).max(1);
                let available =
                    f32::from_bits(control.bwe_available_ratio.load(Ordering::Relaxed)).max(0.0);
                let _ = qos.observe(loss, rtt, available);
            }
            *control.ladder.lock().expect("ladder") = qos.step().label().to_string();
        }
        if reported.elapsed() >= Duration::from_secs(1) {
            publish_qos(
                &control,
                &qos,
                &mut user_mark,
                &mut stream_mark,
                &mut camera_mark,
                &mut screen_mark,
            );
            reported = Instant::now();
        }
        drop(qos);
        // 2 ms floor keeps the encode thread snappy at 60–144 FPS without busy-spinning.
        let pause = Duration::from_millis(2).saturating_sub(encode_started.elapsed());
        if !pause.is_zero() {
            thread::sleep(pause);
        }
    }
}

/// Full picture goes out before the smaller simulcast layers are encoded.
fn highest_first(mut layers: Vec<LayerId>) -> Vec<LayerId> {
    layers.sort_by_key(|layer| std::cmp::Reverse(layer.rank()));
    layers
}

fn publish_qos(
    control: &Control,
    qos: &SenderQos,
    user_mark: &mut u64,
    stream_mark: &mut u64,
    camera_mark: &mut u64,
    screen_mark: &mut u64,
) {
    let user = control.user_bytes.load(Ordering::Relaxed);
    let stream = control.stream_bytes.load(Ordering::Relaxed);
    let camera = control.camera_bytes.load(Ordering::Relaxed);
    let screen = control.screen_bytes.load(Ordering::Relaxed);
    let topology = match control.topology.load(Ordering::Relaxed) {
        1 => "SFU",
        2 => "P2P",
        _ => "idle",
    };
    profiler::set_qos(profiler::QosView {
        topology: topology.into(),
        ladder: control.ladder.lock().expect("ladder").clone(),
        subscribers: control.subscribers.load(Ordering::Relaxed),
        layers: layer_label(control),
        backend: control.backend.lock().expect("backend").clone(),
        loss: qos.loss(),
        rtt_ms: qos.rtt_ms(),
        jitter_ms: qos.jitter_ms(),
        user_kbps: ((user.saturating_sub(*user_mark)) * 8 / 1000) as u32,
        stream_kbps: ((stream.saturating_sub(*stream_mark)) * 8 / 1000) as u32,
        camera_kbps: ((camera.saturating_sub(*camera_mark)) * 8 / 1000) as u32,
        screen_kbps: ((screen.saturating_sub(*screen_mark)) * 8 / 1000) as u32,
        publishing_screen: control.publishing_screen.load(Ordering::Relaxed),
        publishing_camera: control.publishing_camera.load(Ordering::Relaxed),
        target_kbps: control.full_bps.load(Ordering::Relaxed) / 1000,
        encode_width: control.encode_w.load(Ordering::Relaxed),
        encode_height: control.encode_h.load(Ordering::Relaxed),
        encode_fps: control.encode_fps.load(Ordering::Relaxed),
        encoder_pref: control.encoder_pref.lock().expect("encoder pref").clone(),
        rate_control: control.rate_control.lock().expect("rate control").clone(),
        keyframe_secs: (control.keyframe_ms.load(Ordering::Relaxed) / 1000).max(1) as u32,
        remote_stream: String::new(),
        keyframes: control.keyframes.load(Ordering::Relaxed) as u32,
        encode_ms: f32::from_bits(control.encode_ms.load(Ordering::Relaxed)),
        decode_ms: 0.0,
        step: qos.step().label().into(),
        capture_path: if control.publishing_screen.load(Ordering::Relaxed) {
            super::capture_metrics::summary_line()
        } else {
            String::new()
        },
    });
    *user_mark = user;
    *stream_mark = stream;
    *camera_mark = camera;
    *screen_mark = screen;
}

fn initial_backend() -> String {
    let found = super::encode::hardware_probe();
    if found.is_empty() {
        "openh264".into()
    } else {
        format!("openh264 ({} present)", found.join(", "))
    }
}

fn layer_label(control: &Control) -> String {
    let wanted = control.wanted.lock().expect("wanted");
    let mut parts = Vec::new();
    for kind in [MediaKind::Camera, MediaKind::Screen] {
        if let Some(Some(layers)) = wanted.get(&kind) {
            let names: Vec<_> = layers.iter().filter_map(|layer| layer.rid()).collect();
            if !names.is_empty() {
                parts.push(format!("{kind:?} {}", names.join(",")));
            }
        }
    }
    if parts.is_empty() {
        // Dynacast idle / P2P: encode peer-cap only (Full by default), not q+h+f.
        "cap-only".into()
    } else {
        parts.join("  ")
    }
}

fn offer_video(tx: &tokio::sync::mpsc::Sender<Outgoing>, control: &Control, packet: Outgoing) {
    if tx.try_send(packet).is_ok() {
        return;
    }
    // The voice queue is separate. A full video queue drops this picture and
    // counts as congestion. It does not discard an Opus packet.
    control.congested_drops.fetch_add(1, Ordering::Relaxed);
}

fn take_frame(slot: &Mutex<VecDeque<i16>>) -> Option<Vec<i16>> {
    let mut queue = slot.lock().expect("pcm");
    if queue.len() < OPUS_FRAME {
        return None;
    }
    Some(queue.drain(..OPUS_FRAME).collect())
}

fn wanted_of(control: &Control, kind: MediaKind) -> Option<Vec<LayerId>> {
    control
        .wanted
        .lock()
        .expect("wanted")
        .get(&kind)
        .cloned()
        .flatten()
}

fn peer_cap(control: &Control, kind: MediaKind) -> LayerId {
    control
        .peer_cap
        .lock()
        .expect("cap")
        .get(&kind)
        .copied()
        .unwrap_or(LayerId::Full)
}

/// Whether this encode tick may consume `control.force_key`.
///
/// Mesh flips / WantedLayers arm an IDR before camera or Go Live is publishing.
/// Taking the flag while `layers` is empty burned the only force-key and left the
/// first real AU mid-GOP for remotes (BAN-33).
fn can_take_join_key(layers: &[LayerId]) -> bool {
    !layers.is_empty()
}

/// Try VA-API DMA-BUF encode for screen. Returns `Some(keyed)` when a HW frame
/// was consumed (success or sticky-fail after attempt). `None` means fall through
/// to the CPU `PictureSlot` path.
#[cfg(target_os = "linux")]
fn encode_screen_dmabuf(
    control: &Control,
    tx: &tokio::sync::mpsc::Sender<Outgoing>,
    layers: &[LayerId],
) -> Option<bool> {
    if !super::vaapi_dmabuf::available() {
        return None;
    }
    // Separate from PictureSlot `seen` so MemFd and DMA-BUF gens do not collide.
    static HW_SEEN: AtomicU64 = AtomicU64::new(0);
    let seen = HW_SEEN.load(Ordering::Relaxed);
    let Some((generation, frame, edge)) = screen_hw_slot().take_since(seen) else {
        return None;
    };
    if generation == seen {
        return None;
    }
    HW_SEEN.store(generation, Ordering::Relaxed);
    let (full_w, full_h) = super::encode::even_fit(frame.width, frame.height, edge.max(2));
    if full_w < 128 || full_h < 128 {
        return Some(false);
    }
    let fps = control.encode_fps.load(Ordering::Relaxed).max(1) as f32;
    let keyframe_ms = control.keyframe_ms.load(Ordering::Relaxed).max(500);
    let join_key = control.force_key.swap(false, Ordering::Relaxed);
    let mut specs: Vec<(LayerId, u32, u32, u32)> = Vec::new();
    for &layer in layers {
        let div = match layer {
            LayerId::Full => 1u32,
            LayerId::Half => 2,
            LayerId::Quarter => 4,
            LayerId::Off => continue,
        };
        let w = (full_w / div) & !1;
        let h = (full_h / div) & !1;
        if w < 128 || h < 128 {
            continue;
        }
        specs.push((layer, w, h, control.layer_bps(layer)));
    }
    if specs.is_empty() {
        return Some(false);
    }
    let layer_dims: Vec<(u32, u32, u32)> = specs.iter().map(|(_, w, h, b)| (*w, *h, *b)).collect();
    let started = Instant::now();
    match super::vaapi_dmabuf::encode_owned_frame(&frame, &layer_dims, fps, keyframe_ms, join_key)
    {
        Ok(aus) => {
            *control.backend.lock().expect("backend") = "vaapi-dmabuf".into();
            control.encode_w.store(full_w, Ordering::Relaxed);
            control.encode_h.store(full_h, Ordering::Relaxed);
            let encode_elapsed = started.elapsed();
            let e2e = Duration::from_millis(super::screen_lag::pipeline_lag_ms())
                .saturating_add(encode_elapsed);
            super::capture_metrics::note_encoded(encode_elapsed, Some(e2e));
            profiler::record_media("screen_encode", encode_elapsed, aus.iter().map(|a| a.bytes.len()).sum());
            let mut sent_key = false;
            for ((layer, _, _, _), au) in specs.into_iter().zip(aus.into_iter()) {
                if au.keyframe {
                    control.keyframes.fetch_add(1, Ordering::Relaxed);
                    sent_key = true;
                }
                control
                    .screen_bytes
                    .fetch_add(au.bytes.len() as u64, Ordering::Relaxed);
                offer_video(
                    tx,
                    control,
                    Outgoing::Video {
                        kind: MediaKind::Screen,
                        layer,
                        bytes: au.bytes,
                    },
                );
            }
            Some(sent_key)
        }
        Err(_) => {
            // Sticky disable already logged; CPU MemFd path will take over next frames.
            Some(false)
        }
    }
}

fn encode_video(
    control: &Control,
    tx: &tokio::sync::mpsc::Sender<Outgoing>,
    slot: &PictureSlot,
    encoders: &mut HashMap<LayerId, (u32, u32, LayerEncoder)>,
    seen: &mut u64,
    kind: MediaKind,
    screen: bool,
    layers: &[LayerId],
    periodic_soft: bool,
) -> bool {
    if !can_take_join_key(layers) {
        return false;
    }
    #[cfg(target_os = "linux")]
    if screen {
        if let Some(sent) = encode_screen_dmabuf(control, tx, layers) {
            return sent;
        }
    }
    let Some((generation, picture)) = slot.load_since(*seen) else {
        return false;
    };
    if generation == *seen {
        return false;
    }
    *seen = generation;
    let fps = control.encode_fps.load(Ordering::Relaxed).max(1) as f32;
    let keyframe_ms = control.keyframe_ms.load(Ordering::Relaxed).max(500);
    let pick = control.encoder_pick();
    let join_key = control.force_key.swap(false, Ordering::Relaxed);
    let mut sent_key = false;
    for &layer in layers {
        let Some((scaled, width, height)) =
            super::encode::scale_i420(&picture.i420, picture.width, picture.height, layer)
        else {
            continue;
        };
        let bps = control.layer_bps(layer);
        let cbr = control.rate_cbr.load(Ordering::Relaxed);
        let entry = encoders.entry(layer).or_insert_with(|| {
            let encoder = match LayerEncoder::open_with(
                screen,
                width,
                height,
                fps,
                bps,
                keyframe_ms,
                pick,
                cbr,
            ) {
                Ok(encoder) => encoder,
                Err(_) => LayerEncoder::software(screen, fps, bps, keyframe_ms).expect("openh264"),
            };
            *control.backend.lock().expect("backend") = encoder.backend().label().into();
            (width, height, encoder)
        });
        if entry.0 != width || entry.1 != height {
            if let Ok(encoder) = LayerEncoder::open_with(
                screen,
                width,
                height,
                fps,
                bps,
                keyframe_ms,
                pick,
                cbr,
            ) {
                *entry = (width, height, encoder);
                *control.backend.lock().expect("backend") = entry.2.backend().label().into();
            }
        }
        entry.0 = width;
        entry.1 = height;
        if screen && layer == LayerId::Full {
            control.encode_w.store(width, Ordering::Relaxed);
            control.encode_h.store(height, Ordering::Relaxed);
        }
        let started = Instant::now();
        // Hard keys (join/PLI/mesh/size picture.key) always force. Periodic soft keys
        // only force OpenH264 — HW relies on `-g` / CodecAPI+GOP to avoid reopen storms.
        let soft_key =
            periodic_soft && matches!(entry.2.backend(), super::encode::Backend::Software);
        let want_key = soft_key || picture.key || join_key;
        if let Ok(au) = entry.2.encode(&scaled, width, height, want_key) {
            // Refresh after encode: LayerEncoder may have fallen back to OpenH264.
            *control.backend.lock().expect("backend") = entry.2.backend().label().into();
            let encode_elapsed = started.elapsed();
            profiler::record_media(
                if screen {
                    "screen_encode"
                } else {
                    "camera_encode"
                },
                encode_elapsed,
                au.bytes.len(),
            );
            if screen {
                let e2e = Duration::from_millis(super::screen_lag::pipeline_lag_ms())
                    .saturating_add(encode_elapsed);
                super::capture_metrics::note_encoded(encode_elapsed, Some(e2e));
            }
            if au.keyframe {
                control.keyframes.fetch_add(1, Ordering::Relaxed);
                sent_key = true;
            }
            let counter = if screen {
                &control.screen_bytes
            } else {
                &control.camera_bytes
            };
            counter.fetch_add(au.bytes.len() as u64, Ordering::Relaxed);
            offer_video(
                tx,
                control,
                Outgoing::Video {
                    kind,
                    layer,
                    bytes: au.bytes,
                },
            );
        }
    }
    sent_key
}

async fn run(
    mut cmds: UnboundedReceiver<Cmd>,
    mut audio_rx: tokio::sync::mpsc::Receiver<Outgoing>,
    mut video_rx: tokio::sync::mpsc::Receiver<Outgoing>,
    signals: Sender<ClientSignal>,
    control: Arc<Control>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    remote: Arc<Mutex<Vec<RemotePicture>>>,
    output_rate: Arc<AtomicU32>,
    flags: Arc<AtomicU32>,
) {
    let mut carry = None;
    let mut retry: Option<(MediaPlace, i64, bool, bool)> = None;
    let mut logged = false;
    loop {
        let (place, me, offerer, mesh) = if let Some(again) = retry.take() {
            again
        } else {
            loop {
                let cmd = if let Some(cmd) = carry.take() {
                    Some(cmd)
                } else {
                    cmds.recv().await
                };
                match cmd {
                    Some(Cmd::Stop) | None => return,
                    Some(Cmd::Event(event)) => apply_control(&control, &event),
                    Some(Cmd::Session { place: None, .. }) => {
                        control.topology.store(0, Ordering::Relaxed)
                    }
                    Some(Cmd::Session {
                        place: Some(place),
                        me,
                        offerer,
                        mesh,
                    }) => {
                        // Mesh (DM / guild P2P) reports as P2P; otherwise SFU.
                        control.topology.store(
                            if mesh || matches!(place, MediaPlace::DirectCall(_)) {
                                2
                            } else {
                                1
                            },
                            Ordering::Relaxed,
                        );
                        break (place, me, offerer, mesh);
                    }
                }
            }
        };
        match run_call(
            place,
            me,
            offerer,
            mesh,
            &mut cmds,
            &mut audio_rx,
            &mut video_rx,
            &signals,
            &control,
            Arc::clone(&playback),
            Arc::clone(&remote),
            Arc::clone(&output_rate),
            Arc::clone(&flags),
        )
        .await
        {
            Ok(Some(Cmd::Stop)) | Ok(None) => return,
            Ok(Some(cmd)) => {
                logged = false;
                carry = Some(cmd);
            }
            Err(err) => {
                if !logged {
                    eprintln!("webrtc: {err}");
                    logged = true;
                }
                // A failed bind used to restart on the next frame and leak a socket each time.
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {
                        retry = Some((place, me, offerer, mesh));
                    }
                    cmd = cmds.recv() => {
                        carry = cmd;
                    }
                }
            }
        }
    }
}

async fn run_call(
    place: MediaPlace,
    me: i64,
    offerer: bool,
    mesh: bool,
    cmds: &mut UnboundedReceiver<Cmd>,
    audio_rx: &mut tokio::sync::mpsc::Receiver<Outgoing>,
    video_rx: &mut tokio::sync::mpsc::Receiver<Outgoing>,
    signals: &Sender<ClientSignal>,
    control: &Arc<Control>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    remote: Arc<Mutex<Vec<RemotePicture>>>,
    output_rate: Arc<AtomicU32>,
    flags: Arc<AtomicU32>,
) -> Result<Option<Cmd>, String> {
    let (ice_tx, mut ice_rx) = tokio::sync::mpsc::unbounded_channel();
    let peer = Arc::new(AtomicI64::new(0));
    let mut media = crate_media_engine()?;
    let registry = webrtc::peer_connection::register_default_interceptors(
        rtc::interceptor::Registry::new(),
        &mut media,
    )
    .map_err(|err| err.to_string())?;
    let handler = Arc::new(Handler {
        ice_tx,
        playback,
        remote,
        output_rate,
        flags,
        peer: Arc::clone(&peer),
        me,
    });
    let pc = PeerConnectionBuilder::new()
        .with_configuration(
            RTCConfigurationBuilder::default()
                .with_ice_servers(ice_servers())
                .build(),
        )
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(handler)
        .with_udp_addrs(vec!["0.0.0.0:0".to_string(), "127.0.0.1:0".to_string()])
        .build()
        .await
        .map_err(|err| err.to_string())?;
    let tracks = banter_server::attach_publish_tracks(&pc).await?;
    let mut remote_ready = false;
    let mut pending_ice = Vec::new();
    let mut mids = HashMap::new();
    let mut stats_tick = tokio::time::interval(Duration::from_millis(500));
    stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    if offerer {
        let offer = pc.create_offer(None).await.map_err(|err| err.to_string())?;
        pc.set_local_description(offer.clone())
            .await
            .map_err(|err| err.to_string())?;
        mids.extend(mids_by_track(&offer.sdp));
        let _ = signals.send(ClientSignal::RtcOffer {
            place,
            sdp: offer.sdp,
        });
    }
    loop {
        tokio::select! {
            _ = stats_tick.tick() => {
                sample_sender_stats(&pc, control).await;
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else {
                    let _ = pc.close().await;
                    return Ok(None);
                };
                match cmd {
                    Cmd::Stop => {
                        let _ = pc.close().await;
                        return Ok(Some(Cmd::Stop));
                    }
                    Cmd::Session {
                        place: next,
                        me: next_me,
                        offerer: next_offerer,
                        mesh: next_mesh,
                    } => {
                        if next == Some(place)
                            && next_me == me
                            && next_offerer == offerer
                            && next_mesh == mesh
                        {
                            continue;
                        }
                        let _ = pc.close().await;
                        return Ok(Some(Cmd::Session {
                            place: next,
                            me: next_me,
                            offerer: next_offerer,
                            mesh: next_mesh,
                        }));
                    }
                    Cmd::Event(event) => {
                        apply_control(control, &event);
                        match event {
                            ServerEvent::RtcOffer { sdp, from, .. } => {
                                if from != 0 { peer.store(from, Ordering::Relaxed); }
                                mids.extend(mids_by_track(&sdp));
                                let offer = RTCSessionDescription::offer(sdp).map_err(|err| err.to_string())?;
                                pc.set_remote_description(offer).await.map_err(|err| err.to_string())?;
                                remote_ready = true;
                                let answer = pc.create_answer(None).await.map_err(|err| err.to_string())?;
                                pc.set_local_description(answer.clone()).await.map_err(|err| err.to_string())?;
                                mids.extend(mids_by_track(&answer.sdp));
                                let _ = signals.send(ClientSignal::RtcAnswer { place, sdp: answer.sdp });
                                for candidate in pending_ice.drain(..) {
                                    let _ = pc.add_ice_candidate(candidate).await;
                                }
                            }
                            ServerEvent::RtcAnswer { sdp, from, .. } => {
                                if from != 0 { peer.store(from, Ordering::Relaxed); }
                                mids.extend(mids_by_track(&sdp));
                                let answer = RTCSessionDescription::answer(sdp).map_err(|err| err.to_string())?;
                                pc.set_remote_description(answer).await.map_err(|err| err.to_string())?;
                                remote_ready = true;
                                for candidate in pending_ice.drain(..) {
                                    let _ = pc.add_ice_candidate(candidate).await;
                                }
                            }
                            ServerEvent::RtcIce { candidate, sdp_mid, sdp_mline_index, from, .. } => {
                                if from != 0 { peer.store(from, Ordering::Relaxed); }
                                let init = RTCIceCandidateInit { candidate, sdp_mid, sdp_mline_index, ..Default::default() };
                                if remote_ready {
                                    let _ = pc.add_ice_candidate(init).await;
                                } else {
                                    pending_ice.push(init);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            packet = audio_rx.recv() => {
                let Some(packet) = packet else { break };
                write_packet(&tracks, packet, &mids, control).await;
            }
            packet = video_rx.recv() => {
                let Some(packet) = packet else { break };
                write_packet(&tracks, packet, &mids, control).await;
            }
            ice = ice_rx.recv() => {
                let Some(ice) = ice else { continue };
                let _ = signals.send(ClientSignal::RtcIce {
                    place,
                    candidate: ice.candidate,
                    sdp_mid: ice.sdp_mid,
                    sdp_mline_index: ice.sdp_mline_index,
                });
            }
        }
    }
    let _ = pc.close().await;
    Ok(None)
}

/// Pull TWCC / ICE BWE + remote RR loss/RTT + outbound PLI into Control for the encode ladder.
async fn sample_sender_stats(pc: &impl PeerConnection, control: &Control) {
    let report = pc.get_stats(Instant::now(), StatsSelector::None).await;
    let mut rtt_ms = control.bwe_rtt_ms.load(Ordering::Relaxed).max(1);
    let mut available_bps = 0.0f64;
    let mut loss = 0.0f64;
    let mut pli = 0u32;
    for entry in report.iter() {
        match entry {
            RTCStatsReportEntry::IceCandidatePair(pair) => {
                if pair.available_outgoing_bitrate > available_bps {
                    available_bps = pair.available_outgoing_bitrate;
                }
                if pair.current_round_trip_time > 0.0 {
                    rtt_ms = (pair.current_round_trip_time * 1000.0).round() as u32;
                }
            }
            RTCStatsReportEntry::RemoteInboundRtp(remote) => {
                if remote.fraction_lost > loss {
                    loss = remote.fraction_lost;
                }
                if remote.round_trip_time > 0.0 {
                    rtt_ms = (remote.round_trip_time * 1000.0).round() as u32;
                }
            }
            RTCStatsReportEntry::OutboundRtp(outbound) => {
                pli = pli.saturating_add(outbound.pli_count);
            }
            _ => {}
        }
    }
    let target = control.full_bps.load(Ordering::Relaxed).max(30_000) as f64;
    let ratio = if available_bps > 0.0 {
        (available_bps / target) as f32
    } else {
        // No TWCC estimate yet — stay clear of the <0.70 congestion branch.
        1.0
    };
    control
        .bwe_loss
        .store((loss as f32).clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    control.bwe_rtt_ms.store(rtt_ms.max(1), Ordering::Relaxed);
    control
        .bwe_available_ratio
        .store(ratio.max(0.0).to_bits(), Ordering::Relaxed);
    let prev = control.last_pli.swap(pli, Ordering::Relaxed);
    if pli > prev {
        // SFU PLI on subscribe/loss — arm the same late-join IDR path as WantedLayers.
        control.force_key.store(true, Ordering::Relaxed);
    }
}

fn apply_control(control: &Control, event: &ServerEvent) {
    match event {
        ServerEvent::WantedLayers {
            track,
            layers,
            subscribers,
            ..
        } => {
            let prev = control.subscribers.load(Ordering::Relaxed);
            control
                .wanted
                .lock()
                .expect("wanted")
                .insert(*track, Some(layers.clone()));
            control.subscribers.store(*subscribers, Ordering::Relaxed);
            if *subscribers > prev {
                control.force_key.store(true, Ordering::Relaxed);
            }
        }
        ServerEvent::SubscribeLayer {
            publisher: _,
            track,
            max_layer,
            ..
        } => {
            let prev = control
                .peer_cap
                .lock()
                .expect("cap")
                .insert(*track, *max_layer);
            // P2P never sends WantedLayers; a new/changed peer cap must force an IDR so
            // late watchers (or Go Live after the call is up) can decode.
            if prev != Some(*max_layer) {
                control.force_key.store(true, Ordering::Relaxed);
            }
        }
        _ => {}
    }
}

async fn write_packet(
    tracks: &banter_server::PublishTracks,
    packet: Outgoing,
    mids: &HashMap<String, String>,
    control: &Control,
) {
    match packet {
        Outgoing::Audio { kind, bytes } => {
            let (track, ssrc) = if kind == MediaKind::StreamAudio {
                (&tracks.stream_audio, STREAM_AUDIO_SSRC)
            } else {
                (&tracks.user_audio, USER_AUDIO_SSRC)
            };
            let sample = sample_bytes(bytes, 20);
            let _ = track.write_sample(ssrc, OPUS_PT, &sample, &[]).await;
        }
        Outgoing::Video { kind, layer, bytes } => {
            let Some(rid) = layer.rid() else {
                return;
            };
            let track_id = if kind == MediaKind::Screen {
                "screen"
            } else {
                "camera"
            };
            let Some(mid) = mids.get(track_id) else {
                return;
            };
            let (track, base) = if kind == MediaKind::Screen {
                (&tracks.screen, SCREEN_SSRC_Q)
            } else {
                (&tracks.camera, CAMERA_SSRC_Q)
            };
            let ssrc = match layer {
                LayerId::Quarter => base,
                LayerId::Half => base + 1,
                LayerId::Full => base + 2,
                LayerId::Off => return,
            };
            let fps = control.encode_fps.load(Ordering::Relaxed);
            let sample = sample_bytes(bytes, video_rtp_duration_ms(fps));
            let extensions = simulcast_extensions(mid, rid);
            let _ = track
                .write_sample(ssrc, H264_PT, &sample, &extensions)
                .await;
        }
    }
}

fn ice_servers() -> Vec<RTCIceServer> {
    // Prefer the signed-in host’s `/v1/ice` snapshot so off-LAN desktops get TURN
    // without local BANTER_TURN* env. Fall back to process env (embedded / CI).
    let from_host = super::ice_cache::get();
    let stun = from_host
        .as_ref()
        .map(|cfg| cfg.stun.clone())
        .unwrap_or_else(banter_server::stun_url);
    let turn = from_host
        .as_ref()
        .and_then(|cfg| cfg.turn.clone())
        .map(|t| (t.urls, t.username, t.credential))
        .or_else(banter_server::turn_server);

    let mut servers = Vec::new();
    if !stun.is_empty() {
        servers.push(RTCIceServer {
            urls: vec![stun],
            ..Default::default()
        });
    }
    if let Some((url, username, credential)) = turn {
        if !username.is_empty() && !credential.is_empty() {
            servers.push(RTCIceServer {
                urls: vec![url],
                username,
                credential,
            });
        }
    }
    servers
}

fn crate_media_engine(
) -> Result<rtc::peer_connection::configuration::media_engine::MediaEngine, String> {
    use rtc::peer_connection::configuration::media_engine::{
        MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
    };
    use rtc::rtp_transceiver::rtp_sender::{
        RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability,
        RtpCodecKind,
    };
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48_000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1;usedtx=1".into(),
                    rtcp_feedback: vec![RTCPFeedback {
                        typ: "transport-cc".into(),
                        parameter: String::new(),
                    }],
                },
                payload_type: OPUS_PT,
                ..Default::default()
            },
            RtpCodecKind::Audio,
        )
        .map_err(|err| err.to_string())?;
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90_000,
                    channels: 0,
                    sdp_fmtp_line: banter_server::H264_FMTP.to_owned(),
                    rtcp_feedback: vec![
                        RTCPFeedback {
                            typ: "nack".into(),
                            parameter: String::new(),
                        },
                        RTCPFeedback {
                            typ: "nack".into(),
                            parameter: "pli".into(),
                        },
                        RTCPFeedback {
                            typ: "goog-remb".into(),
                            parameter: String::new(),
                        },
                        RTCPFeedback {
                            typ: "transport-cc".into(),
                            parameter: String::new(),
                        },
                    ],
                },
                payload_type: H264_PT,
                ..Default::default()
            },
            RtpCodecKind::Video,
        )
        .map_err(|err| err.to_string())?;
    for kind in [RtpCodecKind::Audio, RtpCodecKind::Video] {
        for uri in [
            "urn:ietf:params:rtp-hdrext:sdes:mid",
            "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id",
            "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
            "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
        ] {
            media
                .register_header_extension(
                    RTCRtpHeaderExtensionCapability {
                        uri: uri.to_owned(),
                    },
                    kind,
                    None,
                )
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(media)
}

struct Handler {
    ice_tx: UnboundedSender<RTCIceCandidateInit>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    remote: Arc<Mutex<Vec<RemotePicture>>>,
    output_rate: Arc<AtomicU32>,
    flags: Arc<AtomicU32>,
    peer: Arc<AtomicI64>,
    me: i64,
}

#[async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {
        if let Ok(init) = event.candidate.to_json() {
            let _ = self.ice_tx.send(init);
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let playback = Arc::clone(&self.playback);
        let remote = Arc::clone(&self.remote);
        let output_rate = Arc::clone(&self.output_rate);
        let flags = Arc::clone(&self.flags);
        let peer = Arc::clone(&self.peer);
        let me = self.me;
        tokio::spawn(async move {
            let id = track.track_id().await;
            let (parsed_sender, kind, layer) = parse_track(&id, peer.load(Ordering::Relaxed));
            let mut opus = OpusListen::new().ok();
            let mut video = super::encode::PictureDecoder::open().ok();
            let mut h264 = H264Packet::default();
            let mut access = Vec::new();
            while let Some(event) = track.poll().await {
                match event {
                    TrackRemoteEvent::OnRtpPacket(packet) => {
                        let sender =
                            remote_picture_sender(&id, parsed_sender, peer.load(Ordering::Relaxed));
                        if sender == me && sender != 0 {
                            continue;
                        }
                        if kind.is_audio() {
                            if flags.load(Ordering::Relaxed) & FLAG_DEAFENED != 0 {
                                continue;
                            }
                            if let Some(decoder) = opus.as_mut() {
                                if let Ok(pcm) = decoder.decode(&packet.payload) {
                                    push_playback(&playback, &output_rate, &pcm);
                                }
                            }
                        } else if let Some(decoder) = video.as_mut() {
                            let payload = Bytes::copy_from_slice(&packet.payload);
                            if let Ok(nal) = h264.depacketize(&payload) {
                                let _ = append_access_nal(&mut access, &nal, ACCESS_UNIT_CAP);
                            }
                            if packet.header.marker && !access.is_empty() {
                                let started = std::time::Instant::now();
                                if let Ok(Some((width, height, rgba))) =
                                    decoder.decode_rgba(&access)
                                {
                                    crate::profiler::note_decode(
                                        started.elapsed().as_secs_f32() * 1000.0,
                                    );
                                    let kind_label = match kind {
                                        MediaKind::Screen => "screen",
                                        MediaKind::Camera => "camera",
                                        _ => "video",
                                    };
                                    crate::profiler::note_remote_stream(format!(
                                        "user {sender} {kind_label} {width}x{height} layer {}",
                                        layer.rid().unwrap_or("-")
                                    ));
                                    let mut frames = remote.lock().expect("remote video");
                                    frames.push(RemotePicture {
                                        sender,
                                        kind,
                                        width,
                                        height,
                                        rgba,
                                        layer,
                                    });
                                    while frames.len() > 8 {
                                        frames.remove(0);
                                    }
                                }
                                access.clear();
                            }
                        }
                    }
                    TrackRemoteEvent::OnEnded => break,
                    _ => {}
                }
            }
        });
    }
}

/// Who owns a remote picture. SFU forward track ids embed the publisher. P2P tracks
/// are named only `camera` / `screen`, so the live peer id must win once signaling sets it —
/// otherwise Go Live tiles are keyed under sender `0` and never show for the other user.
fn remote_picture_sender(track_id: &str, parsed_sender: i64, peer: i64) -> i64 {
    if track_id.starts_with("fwd-") {
        parsed_sender
    } else if peer != 0 {
        peer
    } else {
        parsed_sender
    }
}

fn parse_track(id: &str, peer: i64) -> (i64, MediaKind, LayerId) {
    if let Some(rest) = id.strip_prefix("fwd-") {
        return parse_fwd_track(rest, peer);
    }
    (peer, kind_of(id), LayerId::Full)
}

/// `fwd-{publisher}-{kind}-{rid}` where `kind` may contain underscores (`user_audio`).
fn parse_fwd_track(rest: &str, peer: i64) -> (i64, MediaKind, LayerId) {
    let Some((publisher, after)) = rest.split_once('-') else {
        return (peer, MediaKind::UserAudio, LayerId::Full);
    };
    let sender = publisher.parse().unwrap_or(peer);
    if let Some((kind_token, rid)) = after.rsplit_once('-') {
        if LayerId::from_rid(rid).is_some() {
            return (
                sender,
                kind_of(kind_token),
                LayerId::from_rid(rid).unwrap_or(LayerId::Full),
            );
        }
    }
    (sender, kind_of(after), LayerId::Full)
}

fn kind_of(id: &str) -> MediaKind {
    match id {
        "stream_audio" => MediaKind::StreamAudio,
        "user_audio" => MediaKind::UserAudio,
        "camera" => MediaKind::Camera,
        "screen" => MediaKind::Screen,
        _ => MediaKind::UserAudio,
    }
}

/// Max samples held in the speaker playout deque (~60 ms at `rate`).
///
/// Was 1 s, then 200 ms, then 80 ms. Interactive VC prefers ~60 ms of jitter
/// margin; the 960-sample floor keeps one 20 ms Opus frame at low device rates.
pub fn playback_cap_samples(rate: u32) -> usize {
    let rate = rate.max(1) as usize;
    ((rate * 3) / 50).max(960)
}

/// Hard ceiling on one H.264 access-unit reassembly buffer. Missing marker bits
/// or a corrupt stream must not retain every NAL for the length of the call.
const ACCESS_UNIT_CAP: usize = 4 * 1024 * 1024;

/// Append one depacketized NAL. Returns false (and clears) when the buffer
/// would exceed `cap`, so the next keyframe can resync without unbounded growth.
fn append_access_nal(access: &mut Vec<u8>, nal: &[u8], cap: usize) -> bool {
    if access.len().saturating_add(nal.len()) > cap {
        access.clear();
        return false;
    }
    access.extend_from_slice(nal);
    true
}

fn push_playback(playback: &Mutex<VecDeque<f32>>, output_rate: &AtomicU32, pcm: &[i16]) {
    let rate = output_rate.load(Ordering::Relaxed).max(1);
    let samples = resample_mono_i16(pcm, 48_000, rate);
    let mut queue = playback.lock().expect("playback");
    for sample in samples {
        queue.push_back(sample as f32 / i16::MAX as f32);
    }
    let cap = playback_cap_samples(rate);
    while queue.len() > cap {
        queue.pop_front();
    }
}

#[cfg(test)]
mod receive_tests {
    use super::*;

    #[test]
    fn append_access_nal_caps_missing_marker_growth() {
        let mut access = Vec::new();
        assert!(append_access_nal(&mut access, &[1, 2, 3, 4], 8));
        assert_eq!(access.len(), 4);
        // Next NAL would push past the cap — clear instead of growing forever.
        assert!(!append_access_nal(&mut access, &[5, 6, 7, 8, 9], 8));
        assert!(access.is_empty());
    }

    #[test]
    fn join_key_waits_until_a_layer_is_publishing() {
        assert!(!can_take_join_key(&[]));
        assert!(can_take_join_key(&[LayerId::Full]));
        assert!(can_take_join_key(&[LayerId::Half, LayerId::Quarter]));
    }

    #[test]
    fn fwd_screen_track_names_carry_publisher_and_layer() {
        let (sender, kind, layer) = parse_track("fwd-42-screen-f", 0);
        assert_eq!(sender, 42);
        assert_eq!(kind, MediaKind::Screen);
        assert_eq!(layer, LayerId::Full);
        let (sender, kind, layer) = parse_track("fwd-9-screen-h", 0);
        assert_eq!(sender, 9);
        assert_eq!(kind, MediaKind::Screen);
        assert_eq!(layer, LayerId::Half);
    }

    #[test]
    fn fwd_camera_track_names_carry_publisher_and_layer() {
        let (sender, kind, layer) = parse_track("fwd-42-camera-h", 0);
        assert_eq!(sender, 42);
        assert_eq!(kind, MediaKind::Camera);
        assert_eq!(layer, LayerId::Half);
    }

    #[test]
    fn fwd_stream_audio_keeps_underscore_kind() {
        let (sender, kind, layer) = parse_track("fwd-7-stream_audio-f", 0);
        assert_eq!(sender, 7);
        assert_eq!(kind, MediaKind::StreamAudio);
        assert_eq!(layer, LayerId::Full);
    }

    #[test]
    fn p2p_screen_pictures_follow_the_live_peer_id() {
        assert_eq!(remote_picture_sender("screen", 0, 99), 99);
        assert_eq!(remote_picture_sender("screen", 0, 0), 0);
        assert_eq!(remote_picture_sender("fwd-5-screen-f", 5, 99), 5);
    }

    #[test]
    fn p2p_camera_pictures_follow_the_live_peer_id() {
        assert_eq!(remote_picture_sender("camera", 0, 99), 99);
        assert_eq!(remote_picture_sender("camera", 0, 0), 0);
        assert_eq!(remote_picture_sender("fwd-5-camera-f", 5, 99), 5);
    }
}
