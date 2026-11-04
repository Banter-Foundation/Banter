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
    sample_bytes, ClientSignal, LayerId, MediaKind, MediaPlace, ServerEvent, CAMERA_SSRC_Q, H264_PT, OPUS_PT, SCREEN_SSRC_Q,
    STREAM_AUDIO_SSRC, USER_AUDIO_SSRC,
};
use bytes::Bytes;
use rtc::rtp::codec::h264::H264Packet;
use rtc::rtp::packetizer::Depacketizer;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceServer, RTCSessionDescription};

use crate::logic::resample_mono_i16;
use crate::profiler;

use super::encode::{LayerEncoder, OpusListen, OpusTrack, OPUS_FRAME};
use super::qos::SenderQos;

const FLAG_DEAFENED: u32 = 2;

pub struct RawPicture {
    pub width: u32,
    pub height: u32,
    pub i420: Vec<u8>,
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

    /// Copies the picture only when a newer one has been stored.
    fn load_since(&self, seen: u64) -> Option<(u64, RawPicture)> {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 || generation == seen {
            return None;
        }
        let picture = self.picture.lock().expect("picture").clone()?;
        Some((self.generation.load(Ordering::Acquire), picture))
    }
}

impl Clone for RawPicture {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            i420: self.i420.clone(),
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
        }
    }
}

enum Cmd {
    Session { place: Option<MediaPlace>, me: i64, offerer: bool },
    Event(ServerEvent),
    Stop,
}

enum Outgoing {
    Audio { kind: MediaKind, bytes: Vec<u8> },
    Video { kind: MediaKind, layer: LayerId, bytes: Vec<u8> },
}

pub struct Rtc {
    cmd: UnboundedSender<Cmd>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// Last session handed to the network thread. The UI calls this every frame.
    session: Mutex<Option<(Option<MediaPlace>, i64, bool)>>,
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
        // Audio shares this queue with video. A tiny buffer dropped Opus under
        // camera/screen load and made voice crackle when it worked at all.
        let (media_tx, media_rx) = tokio::sync::mpsc::channel(64);
        let control = Arc::new(Control::new());
        let stop = Arc::new(AtomicBool::new(false));
        let encode = thread::Builder::new()
            .name("banter-encode".into())
            .spawn({
                let control = Arc::clone(&control);
                let stop = Arc::clone(&stop);
                move || encode_loop(stop, control, media_tx, user_pcm, stream_pcm, camera, screen)
            })
            .expect("encode thread");
        let net = thread::Builder::new()
            .name("banter-rtc".into())
            .spawn({
                let control = Arc::clone(&control);
                move || {
                    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            eprintln!("webrtc runtime: {err}");
                            return;
                        }
                    };
                    runtime.block_on(run(cmd_rx, media_rx, signals, control, playback, remote, output_rate, flags));
                }
            })
            .expect("rtc thread");
        Self {
            cmd: cmd_tx,
            stop,
            threads: vec![encode, net],
            session: Mutex::new(None),
        }
    }

    pub fn set_session(&self, place: Option<MediaPlace>, me: i64, offerer: bool) {
        let next = (place, me, offerer);
        let mut current = self.session.lock().expect("rtc session");
        if *current == Some(next) {
            return;
        }
        *current = Some(next);
        let _ = self.cmd.send(Cmd::Session { place, me, offerer });
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

fn encode_loop(
    stop: Arc<AtomicBool>,
    control: Arc<Control>,
    media_tx: tokio::sync::mpsc::Sender<Outgoing>,
    user_pcm: Arc<Mutex<VecDeque<i16>>>,
    stream_pcm: Arc<Mutex<VecDeque<i16>>>,
    camera: Arc<PictureSlot>,
    screen: Arc<PictureSlot>,
) {
    let mut qos = SenderQos::new();
    let mut user = OpusTrack::user_audio().ok();
    let mut stream = OpusTrack::stream_audio().ok();
    let mut camera_enc: HashMap<LayerId, (u32, u32, LayerEncoder)> = HashMap::new();
    let mut screen_enc: HashMap<LayerId, (u32, u32, LayerEncoder)> = HashMap::new();
    let mut camera_gen = 0u64;
    let mut screen_gen = 0u64;
    let mut ticks = 0u32;
    let mut reported = Instant::now();
    let mut user_mark = 0u64;
    let mut stream_mark = 0u64;
    let mut camera_mark = 0u64;
    let mut screen_mark = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let encode_started = Instant::now();
        if let Some(encoder) = user.as_mut() {
            if let Some(frame) = take_frame(&user_pcm) {
                if let Ok(packet) = encoder.encode(&frame) {
                    control.user_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                    offer(&media_tx, &control, Outgoing::Audio { kind: MediaKind::UserAudio, bytes: packet });
                }
            }
        }
        let ratio = qos.targets().stream_audio;
        let stream_wanted = wanted_of(&control, MediaKind::StreamAudio);
        if ratio > 0.0 && stream_wanted.map(|layers| !layers.is_empty()).unwrap_or(true) {
            if let Some(encoder) = stream.as_mut() {
                let _ = encoder.set_ratio(ratio);
                if let Some(frame) = take_frame(&stream_pcm) {
                    if let Ok(packet) = encoder.encode(&frame) {
                        control.stream_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                        offer(&media_tx, &control, Outgoing::Audio { kind: MediaKind::StreamAudio, bytes: packet });
                    }
                }
            }
        }
        encode_video(&control, &qos, &media_tx, &camera, &mut camera_enc, &mut camera_gen, MediaKind::Camera, false);
        encode_video(&control, &qos, &media_tx, &screen, &mut screen_enc, &mut screen_gen, MediaKind::Screen, true);
        let ms = encode_started.elapsed().as_secs_f32() * 1000.0;
        control.encode_ms.store(ms.to_bits(), Ordering::Relaxed);
        ticks = ticks.wrapping_add(1);
        if ticks % 25 == 0 {
            let drops = control.congested_drops.swap(0, Ordering::Relaxed);
            let targets = qos.observe(if drops > 0 { 0.08 } else { 0.0 }, 30, 1.0);
            *control.ladder.lock().expect("ladder") = qos.step().label().to_string();
            let _ = targets;
        }
        if reported.elapsed() >= Duration::from_secs(1) {
            publish_qos(&control, &qos, &mut user_mark, &mut stream_mark, &mut camera_mark, &mut screen_mark);
            reported = Instant::now();
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn publish_qos(control: &Control, qos: &SenderQos, user_mark: &mut u64, stream_mark: &mut u64, camera_mark: &mut u64, screen_mark: &mut u64) {
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
        keyframes: control.keyframes.load(Ordering::Relaxed) as u32,
        encode_ms: f32::from_bits(control.encode_ms.load(Ordering::Relaxed)),
        decode_ms: 0.0,
        step: qos.step().label().into(),
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
    if parts.is_empty() { "all".into() } else { parts.join("  ") }
}

fn offer(tx: &tokio::sync::mpsc::Sender<Outgoing>, control: &Control, packet: Outgoing) {
    if tx.try_send(packet).is_ok() {
        return;
    }
    // Prefer dropping video over Opus when the send path is congested.
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
    control.wanted.lock().expect("wanted").get(&kind).cloned().flatten()
}

fn peer_cap(control: &Control, kind: MediaKind) -> LayerId {
    control.peer_cap.lock().expect("cap").get(&kind).copied().unwrap_or(LayerId::Full)
}

fn encode_video(
    control: &Control,
    qos: &SenderQos,
    tx: &tokio::sync::mpsc::Sender<Outgoing>,
    slot: &PictureSlot,
    encoders: &mut HashMap<LayerId, (u32, u32, LayerEncoder)>,
    seen: &mut u64,
    kind: MediaKind,
    screen: bool,
) {
    let Some((generation, picture)) = slot.load_since(*seen) else { return };
    if generation == *seen {
        return;
    }
    *seen = generation;
    let layers = qos.active_layers(kind, wanted_of(control, kind).as_deref(), peer_cap(control, kind));
    for layer in layers {
        let Some((scaled, width, height)) = super::encode::scale_i420(&picture.i420, picture.width, picture.height, layer) else {
            continue;
        };
        let entry = encoders.entry(layer).or_insert_with(|| {
            let bps = match layer {
                LayerId::Full => 2_500_000,
                LayerId::Half => 600_000,
                LayerId::Quarter => 150_000,
                LayerId::Off => 0,
            };
            let encoder = match LayerEncoder::open(screen, width, height, 30.0, bps) {
                Ok(encoder) => encoder,
                Err(_) => LayerEncoder::software(screen, 30.0, bps).expect("openh264"),
            };
            *control.backend.lock().expect("backend") = encoder.backend().label().into();
            (width, height, encoder)
        });
        if entry.0 != width || entry.1 != height {
            let bps = match layer {
                LayerId::Full => 2_500_000,
                LayerId::Half => 600_000,
                _ => 150_000,
            };
            if let Ok(encoder) = LayerEncoder::open(screen, width, height, 30.0, bps) {
                *entry = (width, height, encoder);
            }
        }
        entry.0 = width;
        entry.1 = height;
        let started = Instant::now();
        if let Ok(au) = entry.2.encode(&scaled, width, height, picture.key) {
            profiler::record_media(if screen { "screen_encode" } else { "camera_encode" }, started.elapsed(), au.bytes.len());
            if au.keyframe {
                control.keyframes.fetch_add(1, Ordering::Relaxed);
            }
            let counter = if screen { &control.screen_bytes } else { &control.camera_bytes };
            counter.fetch_add(au.bytes.len() as u64, Ordering::Relaxed);
            offer(tx, control, Outgoing::Video { kind, layer, bytes: au.bytes });
        }
    }
}

async fn run(
    mut cmds: UnboundedReceiver<Cmd>,
    mut media_rx: tokio::sync::mpsc::Receiver<Outgoing>,
    signals: Sender<ClientSignal>,
    control: Arc<Control>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    remote: Arc<Mutex<Vec<RemotePicture>>>,
    output_rate: Arc<AtomicU32>,
    flags: Arc<AtomicU32>,
) {
    let mut carry = None;
    let mut retry: Option<(MediaPlace, i64, bool)> = None;
    let mut logged = false;
    loop {
        let (place, me, offerer) = if let Some(again) = retry.take() {
            again
        } else {
            loop {
                let cmd = if let Some(cmd) = carry.take() { Some(cmd) } else { cmds.recv().await };
                match cmd {
                    Some(Cmd::Stop) | None => return,
                    Some(Cmd::Event(event)) => apply_control(&control, &event),
                    Some(Cmd::Session { place: None, .. }) => control.topology.store(0, Ordering::Relaxed),
                    Some(Cmd::Session { place: Some(place), me, offerer }) => {
                        control.topology.store(match place {
                            MediaPlace::VoiceChannel(_) => 1,
                            MediaPlace::DirectCall(_) => 2,
                        }, Ordering::Relaxed);
                        break (place, me, offerer);
                    }
                }
            }
        };
        match run_call(place, me, offerer, &mut cmds, &mut media_rx, &signals, &control, Arc::clone(&playback), Arc::clone(&remote), Arc::clone(&output_rate), Arc::clone(&flags)).await {
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
                        retry = Some((place, me, offerer));
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
    cmds: &mut UnboundedReceiver<Cmd>,
    media_rx: &mut tokio::sync::mpsc::Receiver<Outgoing>,
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
    let registry = webrtc::peer_connection::register_default_interceptors(rtc::interceptor::Registry::new(), &mut media).map_err(|err| err.to_string())?;
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
        .with_configuration(RTCConfigurationBuilder::default().with_ice_servers(ice_servers()).build())
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
    if offerer {
        let offer = pc.create_offer(None).await.map_err(|err| err.to_string())?;
        pc.set_local_description(offer.clone()).await.map_err(|err| err.to_string())?;
        let _ = signals.send(ClientSignal::RtcOffer { place, sdp: offer.sdp });
    }
    loop {
        tokio::select! {
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
                    Cmd::Session { place: next, me: next_me, offerer: next_offerer } => {
                        if next == Some(place) && next_me == me && next_offerer == offerer {
                            continue;
                        }
                        let _ = pc.close().await;
                        return Ok(Some(Cmd::Session { place: next, me: next_me, offerer: next_offerer }));
                    }
                    Cmd::Event(event) => {
                        apply_control(control, &event);
                        match event {
                            ServerEvent::RtcOffer { sdp, from, .. } => {
                                if from != 0 { peer.store(from, Ordering::Relaxed); }
                                let offer = RTCSessionDescription::offer(sdp).map_err(|err| err.to_string())?;
                                pc.set_remote_description(offer).await.map_err(|err| err.to_string())?;
                                remote_ready = true;
                                let answer = pc.create_answer(None).await.map_err(|err| err.to_string())?;
                                pc.set_local_description(answer.clone()).await.map_err(|err| err.to_string())?;
                                let _ = signals.send(ClientSignal::RtcAnswer { place, sdp: answer.sdp });
                                for candidate in pending_ice.drain(..) {
                                    let _ = pc.add_ice_candidate(candidate).await;
                                }
                            }
                            ServerEvent::RtcAnswer { sdp, from, .. } => {
                                if from != 0 { peer.store(from, Ordering::Relaxed); }
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
            packet = media_rx.recv() => {
                let Some(packet) = packet else { break };
                write_packet(&tracks, packet).await;
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

fn apply_control(control: &Control, event: &ServerEvent) {
    match event {
        ServerEvent::WantedLayers { track, layers, subscribers, .. } => {
            control.wanted.lock().expect("wanted").insert(*track, Some(layers.clone()));
            control.subscribers.store(*subscribers, Ordering::Relaxed);
        }
        ServerEvent::SubscribeLayer { publisher: _, track, max_layer, .. } => {
            control.peer_cap.lock().expect("cap").insert(*track, *max_layer);
        }
        _ => {}
    }
}

async fn write_packet(tracks: &banter_server::PublishTracks, packet: Outgoing) {
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
            let sample = sample_bytes(bytes, 33);
            let _ = track.write_sample(ssrc, H264_PT, &sample, &[]).await;
        }
    }
}

fn ice_servers() -> Vec<RTCIceServer> {
    let mut servers = Vec::new();
    let stun = banter_server::stun_url();
    if !stun.is_empty() {
        servers.push(RTCIceServer { urls: vec![stun], ..Default::default() });
    }
    if let Some((url, username, credential)) = banter_server::turn_server() {
        if !username.is_empty() && !credential.is_empty() {
            servers.push(RTCIceServer { urls: vec![url], username, credential });
        }
    }
    servers
}

fn crate_media_engine() -> Result<rtc::peer_connection::configuration::media_engine::MediaEngine, String> {
    use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine};
    use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RTCRtpHeaderExtensionCapability, RtpCodecKind};
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48_000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1;usedtx=1".into(),
                    rtcp_feedback: vec![],
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
                    rtcp_feedback: vec![],
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
                .register_header_extension(RTCRtpHeaderExtensionCapability { uri: uri.to_owned() }, kind, None)
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
            let (sender, kind, layer) = parse_track(&id, peer.load(Ordering::Relaxed));
            if sender == me && sender != 0 {
                return;
            }
            let mut opus = OpusListen::new().ok();
            let mut video = super::encode::PictureDecoder::open().ok();
            let mut h264 = H264Packet::default();
            let mut access = Vec::new();
            while let Some(event) = track.poll().await {
                match event {
                    TrackRemoteEvent::OnRtpPacket(packet) => {
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
                                access.extend_from_slice(&nal);
                            }
                            if packet.header.marker && !access.is_empty() {
                                let started = std::time::Instant::now();
                                if let Ok(Some((width, height, rgba))) = decoder.decode_rgba(&access) {
                                    crate::profiler::note_decode(started.elapsed().as_secs_f32() * 1000.0);
                                    let mut frames = remote.lock().expect("remote video");
                                    frames.push(RemotePicture { sender, kind, width, height, rgba, layer });
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

fn parse_track(id: &str, peer: i64) -> (i64, MediaKind, LayerId) {
    if let Some(rest) = id.strip_prefix("fwd-") {
        let mut parts = rest.split('-');
        let sender = parts.next().and_then(|value| value.parse().ok()).unwrap_or(peer);
        let kind = kind_of(parts.next().unwrap_or("user_audio"));
        let layer = parts.next().and_then(LayerId::from_rid).unwrap_or(LayerId::Full);
        return (sender, kind, layer);
    }
    (peer, kind_of(id), LayerId::Full)
}

fn kind_of(id: &str) -> MediaKind {
    match id {
        "stream_audio" => MediaKind::StreamAudio,
        "camera" => MediaKind::Camera,
        "screen" => MediaKind::Screen,
        _ => MediaKind::UserAudio,
    }
}

fn push_playback(playback: &Mutex<VecDeque<f32>>, output_rate: &AtomicU32, pcm: &[i16]) {
    let rate = output_rate.load(Ordering::Relaxed).max(1);
    let samples = resample_mono_i16(pcm, 48_000, rate);
    let mut queue = playback.lock().expect("playback");
    for sample in samples {
        queue.push_back(sample as f32 / i16::MAX as f32);
    }
    // Keep about 200 ms of jitter margin. A full second of backlog made late
    // bursts sound mushy and then underrun when the queue was drained from the front.
    let cap = (rate as usize / 5).max(960);
    while queue.len() > cap {
        queue.pop_front();
    }
}
