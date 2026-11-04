//! Voice-channel PeerConnection. The server answers, forwards RTP, and does not decode.
//!
//! Direct calls never reach this module. Their offers are relayed by `Live`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use async_trait::async_trait;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine};
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RTCRtpHeaderExtensionCapability,
    RtpCodecKind,
};
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceCandidateInit,
    RTCIceServer, RTCSessionDescription, register_default_interceptors,
};

use crate::codec::{stun_url, turn_server, H264_FMTP};
use crate::live::Live;
use crate::model::{LayerId, MediaKind, MediaPlace, ServerEvent};

const RID_EXT: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
const MID_EXT: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
const REPAIR_EXT: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
const TWCC_EXT: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

enum Cmd {
    Ice(RTCIceCandidateInit),
    Answer(String),
    Pli { rid: String },
    BindPeer(i64),
    Close,
}

struct Slot {
    tx: tokio::sync::mpsc::UnboundedSender<Cmd>,
}

fn slots() -> &'static Mutex<HashMap<i64, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<i64, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// ICE that arrived before `accept_offer` created the slot. Voice channels lost
/// early trickle candidates and then never completed DTLS to the SFU.
fn pending_ice() -> &'static Mutex<HashMap<i64, Vec<RTCIceCandidateInit>>> {
    static PENDING: OnceLock<Mutex<HashMap<i64, Vec<RTCIceCandidateInit>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Outbound RTP tracks already negotiated (or being negotiated) toward a subscriber.
fn forwards() -> &'static Mutex<Vec<Forward>> {
    static FORWARDS: OnceLock<Mutex<Vec<Forward>>> = OnceLock::new();
    FORWARDS.get_or_init(|| Mutex::new(Vec::new()))
}

struct Forward {
    subscriber: i64,
    publisher: i64,
    kind: MediaKind,
    rid: String,
    track: Arc<TrackLocalStaticRTP>,
}

fn ice_servers() -> Vec<RTCIceServer> {
    let mut servers = Vec::new();
    let stun = stun_url();
    if !stun.is_empty() {
        servers.push(RTCIceServer {
            urls: vec![stun],
            ..Default::default()
        });
    }
    if let Some((url, username, credential)) = turn_server() {
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

fn h264_codec() -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: H264_FMTP.to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 102,
        ..Default::default()
    }
}

fn opus_codec() -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48_000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1;usedtx=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 111,
        ..Default::default()
    }
}

fn media_engine() -> Result<MediaEngine, String> {
    let mut media = MediaEngine::default();
    media
        .register_codec(opus_codec(), RtpCodecKind::Audio)
        .map_err(|err| err.to_string())?;
    media
        .register_codec(h264_codec(), RtpCodecKind::Video)
        .map_err(|err| err.to_string())?;
    for kind in [RtpCodecKind::Audio, RtpCodecKind::Video] {
        for uri in [MID_EXT, RID_EXT, REPAIR_EXT, TWCC_EXT] {
            media
                .register_header_extension(RTCRtpHeaderExtensionCapability { uri: uri.to_owned() }, kind, None)
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(media)
}

fn encoding(codec: &RTCRtpCodecParameters, ssrc: u32, rid: &str, scale: f64, bitrate: u32) -> RTCRtpEncodingParameters {
    RTCRtpEncodingParameters {
        rtp_coding_parameters: RTCRtpCodingParameters {
            rid: rid.to_owned(),
            ssrc: Some(ssrc),
            ..Default::default()
        },
        codec: codec.rtp_codec.clone(),
        active: true,
        max_bitrate: bitrate,
        scale_resolution_down_by: Some(scale),
        ..Default::default()
    }
}

fn video_encodings(base: u32) -> Vec<RTCRtpEncodingParameters> {
    let codec = h264_codec();
    vec![
        encoding(&codec, base, "q", 4.0, 150_000),
        encoding(&codec, base + 1, "h", 2.0, 600_000),
        encoding(&codec, base + 2, "f", 1.0, 2_500_000),
    ]
}

fn audio_encoding(ssrc: u32) -> Vec<RTCRtpEncodingParameters> {
    vec![encoding(&opus_codec(), ssrc, "", 1.0, 32_000)]
}

fn stream_track(kind: RtpCodecKind, id: &str, encodings: Vec<RTCRtpEncodingParameters>) -> MediaStreamTrack {
    MediaStreamTrack::new("banter".into(), id.into(), id.into(), kind, encodings)
}

struct Handler {
    live: Arc<Live>,
    user_id: i64,
    channel_id: i64,
    remote: Arc<Mutex<Vec<Arc<dyn TrackRemote>>>>,
}

#[async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: webrtc::peer_connection::RTCPeerConnectionIceEvent) {
        let Ok(init) = event.candidate.to_json() else {
            return;
        };
        self.live.send_event(
            &[self.user_id],
            &ServerEvent::RtcIce {
                place: MediaPlace::VoiceChannel(self.channel_id),
                from: 0,
                candidate: init.candidate,
                sdp_mid: init.sdp_mid,
                sdp_mline_index: init.sdp_mline_index,
            },
        );
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        self.remote.lock().expect("remote tracks").push(Arc::clone(&track));
        let live = Arc::clone(&self.live);
        let user_id = self.user_id;
        let channel_id = self.channel_id;
        tokio::spawn(async move {
            let id = track.track_id().await;
            let kind = match id.as_str() {
                "stream_audio" => MediaKind::StreamAudio,
                "camera" => MediaKind::Camera,
                "screen" => MediaKind::Screen,
                _ => MediaKind::UserAudio,
            };
            let mut layers: HashMap<u32, String> = HashMap::new();
            while let Some(event) = track.poll().await {
                match event {
                    TrackRemoteEvent::OnOpen(init) => {
                        layers.insert(init.ssrc, init.rid.unwrap_or_default());
                    }
                    TrackRemoteEvent::OnRtpPacket(packet) => {
                        let mapped = layers.get(&packet.header.ssrc).map(String::as_str);
                        let Some(layer) = inbound_rtp_layer(kind, mapped) else {
                            continue;
                        };
                        let rid = mapped.unwrap_or("").to_string();
                        live.note_layer(user_id, kind, layer);
                        let subscribers = live.admit_rtp(channel_id, user_id, kind, layer, packet.payload.len(), rtp_is_key(&packet.payload));
                        let targets: Vec<Arc<TrackLocalStaticRTP>> = {
                            let table = forwards().lock().expect("forwards");
                            table
                                .iter()
                                .filter(|row| {
                                    row.publisher == user_id
                                        && row.kind == kind
                                        && (kind.is_audio() || row.rid == rid)
                                        && subscribers.contains(&row.subscriber)
                                })
                                .map(|row| Arc::clone(&row.track))
                                .collect()
                        };
                        for target in targets {
                            if let Err(err) = target.write_rtp(packet.clone()).await {
                                let _ = err;
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

pub fn accept_offer(live: Arc<Live>, user_id: i64, channel_id: i64, sdp: String) {
    close_user(user_id);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    slots().lock().expect("sfu slots").insert(user_id, Slot { tx: tx.clone() });
    if let Some(early) = pending_ice().lock().expect("pending ice").remove(&user_id) {
        for candidate in early {
            let _ = tx.send(Cmd::Ice(candidate));
        }
    }
    let peers: Vec<i64> = live
        .voice_channel_members(channel_id)
        .into_iter()
        .map(|member| member.user_id)
        .filter(|id| *id != user_id)
        .collect();
    for peer in peers {
        live.subscribe_voice_pair(user_id, peer);
        let _ = tx.send(Cmd::BindPeer(peer));
        if let Some(slot) = slots().lock().expect("sfu slots").get(&peer) {
            let _ = slot.tx.send(Cmd::BindPeer(user_id));
        }
    }
    tokio::spawn(async move {
        if let Err(err) = run_session(live, user_id, channel_id, sdp, rx).await {
            eprintln!("voice sfu: {err}");
        }
        slots().lock().expect("sfu slots").remove(&user_id);
        pending_ice().lock().expect("pending ice").remove(&user_id);
        forwards().lock().expect("forwards").retain(|row| row.subscriber != user_id && row.publisher != user_id);
    });
}

pub fn accept_answer(user_id: i64, sdp: String) {
    if let Some(slot) = slots().lock().expect("sfu slots").get(&user_id) {
        let _ = slot.tx.send(Cmd::Answer(sdp));
    }
}

pub fn accept_ice(user_id: i64, candidate: String, sdp_mid: Option<String>, sdp_mline_index: Option<u16>) {
    let init = RTCIceCandidateInit {
        candidate,
        sdp_mid,
        sdp_mline_index,
        ..Default::default()
    };
    if let Some(slot) = slots().lock().expect("sfu slots").get(&user_id) {
        let _ = slot.tx.send(Cmd::Ice(init));
        return;
    }
    let mut pending = pending_ice().lock().expect("pending ice");
    let list = pending.entry(user_id).or_default();
    if list.len() < 64 {
        list.push(init);
    }
}

pub fn request_pli(publisher: i64, layer: LayerId) {
    let Some(rid) = layer.rid().map(str::to_string) else {
        return;
    };
    if let Some(slot) = slots().lock().expect("sfu slots").get(&publisher) {
        let _ = slot.tx.send(Cmd::Pli { rid });
    }
}

pub fn close_user(user_id: i64) {
    pending_ice().lock().expect("pending ice").remove(&user_id);
    if let Some(slot) = slots().lock().expect("sfu slots").remove(&user_id) {
        let _ = slot.tx.send(Cmd::Close);
    }
}

async fn run_session(
    live: Arc<Live>,
    user_id: i64,
    channel_id: i64,
    offer_sdp: String,
    mut cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
) -> Result<(), String> {
    let mut media = media_engine()?;
    let registry = register_default_interceptors(rtc::interceptor::Registry::new(), &mut media).map_err(|err| err.to_string())?;
    let remote = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(Handler {
        live: Arc::clone(&live),
        user_id,
        channel_id,
        remote: Arc::clone(&remote),
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
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .build()
        .await
        .map_err(|err| err.to_string())?;
    let offer = RTCSessionDescription::offer(offer_sdp).map_err(|err| err.to_string())?;
    pc.set_remote_description(offer).await.map_err(|err| err.to_string())?;
    let answer = pc.create_answer(None).await.map_err(|err| err.to_string())?;
    pc.set_local_description(answer.clone()).await.map_err(|err| err.to_string())?;
    live.send_event(
        &[user_id],
        &ServerEvent::RtcAnswer {
            place: MediaPlace::VoiceChannel(channel_id),
            from: 0,
            sdp: answer.sdp,
        },
    );
    let mut pending_ice = Vec::new();
    let mut have_remote = true;
    loop {
        let Some(cmd) = cmds.recv().await else {
            break;
        };
        match cmd {
            Cmd::Close => break,
            Cmd::Ice(candidate) => {
                if have_remote {
                    let _ = pc.add_ice_candidate(candidate).await;
                } else {
                    pending_ice.push(candidate);
                }
            }
            Cmd::Answer(sdp) => {
                let answer = RTCSessionDescription::answer(sdp).map_err(|err| err.to_string())?;
                pc.set_remote_description(answer).await.map_err(|err| err.to_string())?;
                have_remote = true;
                for candidate in pending_ice.drain(..) {
                    let _ = pc.add_ice_candidate(candidate).await;
                }
            }
            Cmd::BindPeer(peer) => {
                if attach_forward(&pc, user_id, peer).await {
                    if let Ok(offer) = pc.create_offer(None).await {
                        if pc.set_local_description(offer.clone()).await.is_ok() {
                            live.send_event(
                                &[user_id],
                                &ServerEvent::RtcOffer {
                                    place: MediaPlace::VoiceChannel(channel_id),
                                    from: 0,
                                    sdp: offer.sdp,
                                },
                            );
                        }
                    }
                }
            }
            Cmd::Pli { rid } => {
                let tracks = remote.lock().expect("remote tracks").clone();
                for track in tracks {
                    let ssrc = track.ssrcs().await.into_iter().next().unwrap_or(0);
                    let _ = rid;
                    let _ = track
                        .write_rtcp(vec![Box::new(PictureLossIndication { sender_ssrc: 0, media_ssrc: ssrc })])
                        .await;
                }
            }
        }
    }
    let _ = pc.close().await;
    Ok(())
}

fn next_ssrc() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0x8100_0001);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn rtp_is_key(payload: &[u8]) -> bool {
    if payload.is_empty() {
        return false;
    }
    let nal = payload[0] & 0x1f;
    if nal == 5 || nal == 7 {
        return true;
    }
    nal == 28 && payload.len() > 1 && matches!(payload[1] & 0x1f, 5 | 7)
}

fn kind_token(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::UserAudio => "user_audio",
        MediaKind::StreamAudio => "stream_audio",
        MediaKind::Camera => "camera",
        MediaKind::Screen => "screen",
    }
}

/// Layer for one inbound RTP packet on a known track.
///
/// Audio never waits on an OnOpen SSRC→RID map. That map often missed the mic
/// SSRC (or arrived late), so voice-channel Opus was dropped while 1:1 P2P still
/// worked. Video still needs a known RID.
pub fn inbound_rtp_layer(kind: MediaKind, mapped_rid: Option<&str>) -> Option<LayerId> {
    if kind.is_audio() {
        return Some(LayerId::Full);
    }
    mapped_rid.and_then(LayerId::from_rid)
}

async fn attach_forward(pc: &impl PeerConnection, subscriber: i64, publisher: i64) -> bool {
    let layers = [
        (MediaKind::UserAudio, ""),
        (MediaKind::StreamAudio, ""),
        (MediaKind::Camera, "q"),
        (MediaKind::Camera, "h"),
        (MediaKind::Camera, "f"),
        (MediaKind::Screen, "q"),
        (MediaKind::Screen, "h"),
        (MediaKind::Screen, "f"),
    ];
    let mut added = false;
    for (kind, rid) in layers {
        let exists = forwards().lock().expect("forwards").iter().any(|row| {
            row.subscriber == subscriber && row.publisher == publisher && row.kind == kind && row.rid == rid
        });
        if exists {
            continue;
        }
        let ssrc = next_ssrc();
        let id = format!("fwd-{publisher}-{}-{}", kind_token(kind), if rid.is_empty() { "f" } else { rid });
        let track_kind = if kind.is_audio() { RtpCodecKind::Audio } else { RtpCodecKind::Video };
        let codec = if kind.is_audio() { opus_codec() } else { h264_codec() };
        let scale = match rid {
            "h" => 2.0,
            "q" => 4.0,
            _ => 1.0,
        };
        let bitrate = if kind.is_audio() {
            32_000
        } else {
            match rid {
                "q" => 150_000,
                "h" => 600_000,
                _ => 2_500_000,
            }
        };
        let local = Arc::new(TrackLocalStaticRTP::new(stream_track(
            track_kind,
            &id,
            vec![encoding(&codec, ssrc, rid, scale, bitrate)],
        )));
        if pc.add_track(Arc::clone(&local) as Arc<dyn TrackLocal>).await.is_err() {
            continue;
        }
        forwards().lock().expect("forwards").push(Forward {
            subscriber,
            publisher,
            kind,
            rid: rid.to_string(),
            track: local,
        });
        added = true;
    }
    added
}

/// The SDP a publisher offers. Tests lock the profile and the `q`/`h`/`f` RIDs.
struct Quiet;

#[async_trait]
impl PeerConnectionEventHandler for Quiet {}

pub struct PublishTracks {
    pub user_audio: Arc<TrackLocalStaticSample>,
    pub stream_audio: Arc<TrackLocalStaticSample>,
    pub camera: Arc<TrackLocalStaticSample>,
    pub screen: Arc<TrackLocalStaticSample>,
}

pub const USER_AUDIO_SSRC: u32 = 0x1001;
pub const STREAM_AUDIO_SSRC: u32 = 0x1002;
pub const CAMERA_SSRC_Q: u32 = 0x2000;
pub const SCREEN_SSRC_Q: u32 = 0x3000;
pub const OPUS_PT: u8 = 111;
pub const H264_PT: u8 = 102;

pub async fn attach_publish_tracks(pc: &impl PeerConnection) -> Result<PublishTracks, String> {
    let opus = opus_codec();
    let h264 = h264_codec();
    let user_audio = add_sample(pc, stream_track(RtpCodecKind::Audio, "user_audio", audio_encoding(USER_AUDIO_SSRC)), &opus).await?;
    let stream_audio = add_sample(pc, stream_track(RtpCodecKind::Audio, "stream_audio", audio_encoding(STREAM_AUDIO_SSRC)), &opus).await?;
    let camera = add_sample(pc, stream_track(RtpCodecKind::Video, "camera", video_encodings(CAMERA_SSRC_Q)), &h264).await?;
    let screen = add_sample(pc, stream_track(RtpCodecKind::Video, "screen", video_encodings(SCREEN_SSRC_Q)), &h264).await?;
    Ok(PublishTracks {
        user_audio,
        stream_audio,
        camera,
        screen,
    })
}

pub async fn publish_offer_sdp() -> Result<String, String> {
    let mut media = media_engine()?;
    let registry = register_default_interceptors(rtc::interceptor::Registry::new(), &mut media).map_err(|err| err.to_string())?;
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::default().with_ice_servers(ice_servers()).build())
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(Quiet))
        .with_udp_addrs(vec!["127.0.0.1:0".to_string()])
        .build()
        .await
        .map_err(|err| err.to_string())?;
    attach_publish_tracks(&pc).await?;
    let offer = pc.create_offer(None).await.map_err(|err| err.to_string())?;
    pc.set_local_description(offer.clone()).await.map_err(|err| err.to_string())?;
    let _ = pc.close().await;
    Ok(offer.sdp)
}

async fn add_sample(pc: &impl PeerConnection, track: MediaStreamTrack, _codec: &RTCRtpCodecParameters) -> Result<Arc<TrackLocalStaticSample>, String> {
    let sample = Arc::new(TrackLocalStaticSample::new(Instant::now(), track).map_err(|err| err.to_string())?);
    pc.add_track(Arc::clone(&sample) as Arc<dyn TrackLocal>)
        .await
        .map_err(|err| err.to_string())?;
    Ok(sample)
}

/// Used by the client crate through the same SDP tokens. Keeping the constructor reachable
/// avoids a second offer shape.
pub fn sample_bytes(payload: Vec<u8>, duration_ms: u64) -> rtc::media::Sample {
    rtc::media::Sample {
        data: bytes::Bytes::from(payload),
        timestamp: Instant::now(),
        duration: std::time::Duration::from_millis(duration_ms),
        ..rtc::media::Sample::new(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::sdp_matches_contract;

    #[tokio::test]
    async fn publish_offer_locks_profile_and_rids() {
        let sdp = publish_offer_sdp().await.expect("offer");
        assert!(sdp_matches_contract(&sdp), "{sdp}");
    }
}
