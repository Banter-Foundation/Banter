use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc::UnboundedSender;

use crate::error::BanterError;
use crate::model::*;
use crate::sfu::ForwardTable;
use crate::store::Store;

#[derive(Clone, Debug)]
pub(crate) enum Outbound {
    Text(String),
}

struct UserLive {
    connected: bool,
    last_activity: i64,
    presence: Presence,
    custom_status: Option<String>,
    flags: VoiceFlags,
    voice_channel: Option<i64>,
}

struct LiveInner {
    users: HashMap<i64, UserLive>,
    calls: HashMap<i64, CallSnapshot>,
    sockets: HashMap<i64, Vec<(u64, UnboundedSender<Outbound>)>>,
    /// Bot sockets and the intents they may receive.
    bot_sockets: HashMap<u64, Vec<String>>,
    next_socket: u64,
    forward: ForwardTable,
}

pub struct Live {
    inner: Mutex<LiveInner>,
}

impl Live {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LiveInner {
                users: HashMap::new(),
                calls: HashMap::new(),
                sockets: HashMap::new(),
                bot_sockets: HashMap::new(),
                next_socket: 1,
                forward: ForwardTable::new(),
            }),
        }
    }

    pub fn connect(&self, user_id: i64, now: i64, custom_status: Option<String>) {
        let mut inner = self.lock();
        let user = entry(&mut inner, user_id);
        user.connected = true;
        user.last_activity = now;
        user.presence = Presence::Online;
        user.custom_status = custom_status;
    }

    pub fn set_custom_status(&self, user_id: i64, custom_status: Option<String>) {
        let mut inner = self.lock();
        let user = entry(&mut inner, user_id);
        user.custom_status = custom_status;
    }

    pub fn note_activity(&self, user_id: i64, now: i64) {
        let mut inner = self.lock();
        let Some(user) = inner.users.get_mut(&user_id) else {
            return;
        };
        if !user.connected {
            return;
        }
        user.last_activity = now;
        user.presence = Presence::Online;
    }

    pub fn refresh(&self, now: i64) -> Vec<(i64, Presence)> {
        let mut inner = self.lock();
        let mut changed = Vec::new();
        for (user_id, user) in inner.users.iter_mut() {
            if !user.connected || user.presence == Presence::Offline {
                continue;
            }
            if now.saturating_sub(user.last_activity) >= IDLE_AFTER_MS && user.presence != Presence::Idle {
                user.presence = Presence::Idle;
                changed.push((*user_id, Presence::Idle));
            }
        }
        changed
    }

    pub fn presence(&self, user_id: i64) -> Presence {
        self.presence_snapshot(user_id).state
    }

    pub fn presence_snapshot(&self, user_id: i64) -> PresenceSnapshot {
        self.lock()
            .users
            .get(&user_id)
            .map(|user| PresenceSnapshot::with_state(user.presence, user.custom_status.clone()))
            .unwrap_or_else(PresenceSnapshot::offline)
    }

    pub fn custom_status(&self, user_id: i64) -> Option<String> {
        self.presence_snapshot(user_id).custom_status
    }

    /// Drop the socket and voice seat. Returns voice channels the user just left so the caller can publish the roster.
    pub fn disconnect(&self, user_id: i64) -> Vec<i64> {
        crate::room::close_user(user_id);
        let channels;
        let ended_calls;
        {
            let mut inner = self.lock();
            channels = inner
                .users
                .get(&user_id)
                .and_then(|user| user.voice_channel)
                .into_iter()
                .collect::<Vec<_>>();
            if let Some(user) = inner.users.get_mut(&user_id) {
                user.connected = false;
                user.presence = Presence::Offline;
                user.voice_channel = None;
            }
            inner.sockets.remove(&user_id);
            inner.forward.unsubscribe_user(user_id);
            ended_calls = take_calls(&mut inner, user_id);
        }
        for (dm_id, peer) in ended_calls {
            self.send_event(&[user_id, peer], &ServerEvent::CallEnded { dm_id });
        }
        channels
    }

    pub async fn join_server_voice(&self, store: &Store, user_id: i64, channel_id: i64) -> Result<(), BanterError> {
        let channel = store.channel_access(user_id, channel_id).await?;
        if channel.kind != ChannelKind::Voice {
            return Err(BanterError::NotVoiceChannel);
        }
        let previous;
        let ended;
        {
            let mut inner = self.lock();
            let user = entry(&mut inner, user_id);
            previous = user.voice_channel;
            user.voice_channel = Some(channel_id);
            ended = take_calls(&mut inner, user_id);
            if let Some(user) = inner.users.get_mut(&user_id) {
                user.voice_channel = Some(channel_id);
            }
        }
        if let Some(previous) = previous {
            if previous != channel_id {
                self.publish_voice(store, user_id, previous).await;
            }
        }
        self.publish_voice(store, user_id, channel_id).await;
        for (dm_id, peer) in ended {
            self.send_event(&[user_id, peer], &ServerEvent::CallEnded { dm_id });
        }
        Ok(())
    }

    /// Test-only membership. Real joins go through `join_server_voice`, which checks the channel.
    #[cfg(test)]
    pub(crate) fn seed_voice_member(&self, user_id: i64, channel_id: i64) {
        let mut inner = self.lock();
        let user = entry(&mut inner, user_id);
        user.connected = true;
        user.voice_channel = Some(channel_id);
    }

    /// Leave the current voice channel. The channel id is the one that needs a roster update.
    pub fn leave_voice(&self, user_id: i64) -> Option<i64> {
        crate::room::close_user(user_id);
        let mut inner = self.lock();
        let Some(user) = inner.users.get_mut(&user_id) else {
            return None;
        };
        user.voice_channel.take()
    }

    /// Returns the voice channel whose roster changed, when the user is in one.
    pub fn set_flags(&self, user_id: i64, flags: VoiceFlags) -> Option<i64> {
        let mut inner = self.lock();
        let user = entry(&mut inner, user_id);
        user.flags = crate::model::sanitize_voice_flags(flags);
        user.voice_channel
    }

    pub fn voice_channel_members(&self, channel_id: i64) -> Vec<VoiceMember> {
        let inner = self.lock();
        members_of(&inner, channel_id)
    }

    pub async fn start_dm_call(&self, store: &Store, user_id: i64, dm_id: i64) -> Result<CallSnapshot, BanterError> {
        let (_me, peer) = store.dm_participants(user_id, dm_id).await?;
        let (snapshot, left_voice) = {
            let mut inner = self.lock();
            let left_voice = inner.users.get(&user_id).and_then(|user| user.voice_channel);
            if let Some(existing) = inner.calls.get(&dm_id).cloned() {
                if existing.caller == user_id || existing.callee == user_id {
                    if existing.phase == CallPhase::Ringing && existing.callee == user_id {
                        let mut active = existing;
                        active.phase = CallPhase::Active;
                        inner.calls.insert(dm_id, active.clone());
                        if let Some(user) = inner.users.get_mut(&user_id) {
                            user.voice_channel = None;
                        }
                        (active, left_voice)
                    } else {
                        return Ok(existing);
                    }
                } else {
                    return Err(BanterError::NotInCall);
                }
            } else {
                if let Some(user) = inner.users.get_mut(&user_id) {
                    user.voice_channel = None;
                } else {
                    entry(&mut inner, user_id).voice_channel = None;
                }
                let snapshot = CallSnapshot {
                    dm_id,
                    phase: CallPhase::Ringing,
                    caller: user_id,
                    callee: peer,
                };
                inner.calls.insert(dm_id, snapshot.clone());
                (snapshot, left_voice)
            }
        };
        if let Some(channel_id) = left_voice {
            self.publish_voice(store, user_id, channel_id).await;
        }
        self.send_event(&[snapshot.caller, snapshot.callee], &ServerEvent::Call { call: snapshot.clone() });
        Ok(snapshot)
    }

    pub async fn accept_dm_call(&self, store: &Store, user_id: i64, dm_id: i64) -> Result<CallSnapshot, BanterError> {
        let (snapshot, left_voice) = {
            let mut inner = self.lock();
            let Some(call) = inner.calls.get(&dm_id).cloned() else {
                return Err(BanterError::NotInCall);
            };
            if call.callee != user_id {
                return Err(BanterError::NotInCall);
            }
            let left_voice = inner.users.get(&user_id).and_then(|user| user.voice_channel);
            let mut call = call;
            call.phase = CallPhase::Active;
            inner.calls.insert(dm_id, call.clone());
            if let Some(user) = inner.users.get_mut(&user_id) {
                user.voice_channel = None;
            }
            (call, left_voice)
        };
        if let Some(channel_id) = left_voice {
            self.publish_voice(store, user_id, channel_id).await;
        }
        self.send_event(&[snapshot.caller, snapshot.callee], &ServerEvent::Call { call: snapshot.clone() });
        Ok(snapshot)
    }

    pub fn decline_dm_call(&self, user_id: i64, dm_id: i64) -> Result<(), BanterError> {
        let call = {
            let mut inner = self.lock();
            let Some(call) = inner.calls.get(&dm_id).cloned() else {
                return Err(BanterError::NotInCall);
            };
            if call.phase != CallPhase::Ringing || (user_id != call.caller && user_id != call.callee) {
                return Err(BanterError::NotInCall);
            }
            inner.calls.remove(&dm_id);
            call
        };
        self.send_event(&[call.caller, call.callee], &ServerEvent::CallEnded { dm_id });
        Ok(())
    }

    pub fn end_dm_call(&self, user_id: i64, dm_id: i64) -> Result<(), BanterError> {
        let call = {
            let mut inner = self.lock();
            let Some(call) = inner.calls.get(&dm_id).cloned() else {
                return Err(BanterError::NotInCall);
            };
            if user_id != call.caller && user_id != call.callee {
                return Err(BanterError::NotInCall);
            }
            inner.calls.remove(&dm_id);
            call
        };
        self.send_event(&[call.caller, call.callee], &ServerEvent::CallEnded { dm_id });
        Ok(())
    }

    pub fn call(&self, dm_id: i64) -> Option<CallSnapshot> {
        self.lock().calls.get(&dm_id).cloned()
    }

    pub fn current_place(&self, user_id: i64) -> Option<MediaPlace> {
        let inner = self.lock();
        if let Some(channel) = inner.users.get(&user_id).and_then(|user| user.voice_channel) {
            return Some(MediaPlace::VoiceChannel(channel));
        }
        inner.calls.iter().find_map(|(dm_id, call)| {
            if call.phase == CallPhase::Active && (call.caller == user_id || call.callee == user_id) {
                Some(MediaPlace::DirectCall(*dm_id))
            } else {
                None
            }
        })
    }

    /// Who may receive this track. Empty when the sender is in the room but gated off, or the call is still ringing.
    ///
    /// Voice media is not copied here. The SFU forwards RTP for these subscribers.
    pub fn media_subscribers(&self, sender: i64, place: MediaPlace, kind: MediaKind) -> Result<Vec<i64>, BanterError> {
        let inner = self.lock();
        subscribers_of(&inner, sender, place, kind)
    }

    pub fn note_layer(&self, publisher: i64, track: MediaKind, layer: LayerId) {
        self.lock().forward.note_layer(publisher, track, layer);
    }

    /// Record the packet in the forward table and return the subscribers who should receive it.
    pub fn admit_rtp(&self, channel_id: i64, publisher: i64, kind: MediaKind, layer: LayerId, bytes: usize, keyframe: bool) -> Vec<i64> {
        let mut inner = self.lock();
        let allowed = subscribers_of(&inner, publisher, MediaPlace::VoiceChannel(channel_id), kind).unwrap_or_default();
        inner
            .forward
            .push(publisher, kind, layer, bytes, keyframe)
            .into_iter()
            .map(|item| item.subscriber)
            .filter(|subscriber| allowed.contains(subscriber))
            .collect()
    }

    pub fn subscribe_voice_pair(&self, left: i64, right: i64) {
        if left == right {
            return;
        }
        let mut inner = self.lock();
        for kind in [MediaKind::UserAudio, MediaKind::StreamAudio, MediaKind::Camera, MediaKind::Screen] {
            inner.forward.subscribe(left, right, kind, LayerId::Full);
            inner.forward.subscribe(right, left, kind, LayerId::Full);
        }
    }

    pub fn take_keyframes(&self) -> Vec<crate::sfu::KeyframeRequest> {
        self.lock().forward.take_keyframes()
    }

    pub fn handle_signal(&self, user_id: i64, signal: ClientSignal) -> Result<(), BanterError> {
        if self.bot_user(user_id) && matches!(
            signal,
            ClientSignal::RtcOffer { .. }
                | ClientSignal::RtcAnswer { .. }
                | ClientSignal::RtcIce { .. }
                | ClientSignal::SubscribeLayer { .. }
        ) {
            return Ok(());
        }
        let prepared = {
            let mut inner = self.lock();
            prepare_signal(&mut inner, user_id, signal)?
        };
        for (recipients, event) in prepared {
            self.send_event(&recipients, &event);
        }
        Ok(())
    }

    pub(crate) fn register_socket(&self, user_id: i64, tx: UnboundedSender<Outbound>) -> u64 {
        let mut inner = self.lock();
        let id = inner.next_socket;
        inner.next_socket += 1;
        inner.sockets.entry(user_id).or_default().push((id, tx));
        id
    }

    pub(crate) fn mark_bot_socket(&self, socket_id: u64, intents: Vec<String>) {
        self.lock().bot_sockets.insert(socket_id, intents);
    }

    pub(crate) fn send_socket(&self, user_id: i64, socket_id: u64, event: &ServerEvent) {
        let Ok(text) = serde_json::to_string(event) else {
            return;
        };
        let inner = self.lock();
        if let Some(list) = inner.sockets.get(&user_id) {
            for (id, tx) in list {
                if *id == socket_id {
                    let _ = tx.send(Outbound::Text(text));
                    break;
                }
            }
        }
    }

    pub(crate) fn unregister_socket(&self, user_id: i64, socket_id: u64) {
        let mut inner = self.lock();
        if let Some(list) = inner.sockets.get_mut(&user_id) {
            list.retain(|(id, _)| *id != socket_id);
        }
        inner.bot_sockets.remove(&socket_id);
    }

    pub fn send_event(&self, user_ids: &[i64], event: &ServerEvent) {
        let Ok(text) = serde_json::to_string(event) else {
            return;
        };
        let inner = self.lock();
        for user_id in user_ids {
            if let Some(list) = inner.sockets.get(user_id) {
                for (socket_id, tx) in list {
                    if bot_allows(&inner.bot_sockets, *socket_id, event) {
                        let _ = tx.send(Outbound::Text(text.clone()));
                    }
                }
            }
        }
    }

    /// Everyone who should see this channel's roster: the server's members, plus anyone still seated in it.
    pub async fn voice_watcher_ids(&self, store: &Store, actor: i64, channel_id: i64) -> Vec<i64> {
        let members = self.voice_channel_members(channel_id);
        let mut ids = match store.channel_access(actor, channel_id).await {
            Ok(channel) => store.member_user_ids(channel.server_id).await.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        for member in &members {
            if !ids.contains(&member.user_id) {
                ids.push(member.user_id);
            }
        }
        if !ids.contains(&actor) {
            ids.push(actor);
        }
        ids
    }

    /// Push the current roster to every member of the channel's server.
    pub async fn publish_voice(&self, store: &Store, actor: i64, channel_id: i64) {
        let members = self.voice_channel_members(channel_id);
        let ids = self.voice_watcher_ids(store, actor, channel_id).await;
        if ids.is_empty() {
            return;
        }
        self.send_event(&ids, &ServerEvent::Voice { channel_id, members });
    }

    fn bot_user(&self, user_id: i64) -> bool {
        let inner = self.lock();
        inner.sockets.get(&user_id).is_some_and(|list| list.iter().any(|(id, _)| inner.bot_sockets.contains_key(id)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LiveInner> {
        self.inner.lock().expect("live lock")
    }

    pub(crate) fn counts(&self) -> LiveCounts {
        let inner = self.lock();
        let mut online = 0;
        let mut idle = 0;
        let mut voice_members = 0;
        let mut channels = std::collections::HashSet::new();
        for user in inner.users.values() {
            if user.connected && user.presence == Presence::Online {
                online += 1;
            }
            if user.connected && user.presence == Presence::Idle {
                idle += 1;
            }
            if let Some(channel_id) = user.voice_channel {
                voice_members += 1;
                channels.insert(channel_id);
            }
        }
        let sockets = inner.sockets.values().map(|list| list.len() as i64).sum();
        LiveCounts {
            online,
            idle,
            sockets,
            voice_members,
            voice_channels: channels.len() as i64,
            calls: inner.calls.len() as i64,
        }
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

fn entry(inner: &mut LiveInner, user_id: i64) -> &mut UserLive {
    inner.users.entry(user_id).or_insert_with(|| UserLive {
        connected: false,
        last_activity: 0,
        presence: Presence::Offline,
        custom_status: None,
        flags: VoiceFlags::default(),
        voice_channel: None,
    })
}

fn bot_allows(bots: &HashMap<u64, Vec<String>>, socket_id: u64, event: &ServerEvent) -> bool {
    let Some(intents) = bots.get(&socket_id) else {
        return true;
    };
    match crate::bots::event_gate(event) {
        crate::bots::EventGate::Deliver => true,
        crate::bots::EventGate::Drop => false,
        crate::bots::EventGate::Need(intent) => intents.iter().any(|item| item == intent),
    }
}

fn members_of(inner: &LiveInner, channel_id: i64) -> Vec<VoiceMember> {
    let mut members: Vec<VoiceMember> = inner
        .users
        .iter()
        .filter(|(_, user)| user.voice_channel == Some(channel_id))
        .map(|(user_id, user)| VoiceMember {
            user_id: *user_id,
            flags: user.flags,
        })
        .collect();
    members.sort_by_key(|member| member.user_id);
    members
}

fn calls_involving(calls: &HashMap<i64, CallSnapshot>, user_id: i64) -> Vec<i64> {
    calls
        .iter()
        .filter(|(_, call)| call.caller == user_id || call.callee == user_id)
        .map(|(dm_id, _)| *dm_id)
        .collect()
}

fn subscribers_of(inner: &LiveInner, sender: i64, place: MediaPlace, kind: MediaKind) -> Result<Vec<i64>, BanterError> {
    let Some(sender_state) = inner.users.get(&sender) else {
        return Err(BanterError::NotFound);
    };
    let allowed = match place {
        MediaPlace::VoiceChannel(channel_id) => sender_state.voice_channel == Some(channel_id),
        MediaPlace::DirectCall(dm_id) => match inner.calls.get(&dm_id) {
            Some(call) if call.caller == sender || call.callee == sender => {
                if call.phase != CallPhase::Active {
                    return Ok(Vec::new());
                }
                true
            }
            _ => false,
        },
    };
    if !allowed {
        return Err(BanterError::NotFound);
    }
    if !kind.publish_allowed(&sender_state.flags) {
        return Ok(Vec::new());
    }
    let recipients = match place {
        MediaPlace::VoiceChannel(channel_id) => inner
            .users
            .iter()
            .filter(|(user_id, user)| **user_id != sender && user.voice_channel == Some(channel_id))
            .map(|(user_id, user)| (*user_id, user.flags))
            .collect::<Vec<_>>(),
        MediaPlace::DirectCall(dm_id) => {
            let call = inner.calls.get(&dm_id).ok_or(BanterError::NotFound)?;
            let other = if call.caller == sender { call.callee } else { call.caller };
            let flags = inner.users.get(&other).map(|user| user.flags).unwrap_or_default();
            vec![(other, flags)]
        }
    };
    let mut routed = Vec::new();
    for (user_id, flags) in recipients {
        if kind.is_audio() && !should_receive_audio(&flags) {
            continue;
        }
        routed.push(user_id);
    }
    Ok(routed)
}

fn prepare_signal(inner: &mut LiveInner, user_id: i64, signal: ClientSignal) -> Result<Vec<(Vec<i64>, ServerEvent)>, BanterError> {
    match signal {
        ClientSignal::Activity | ClientSignal::BoardOp { .. } => Ok(Vec::new()),
        ClientSignal::RtcOffer { place, sdp } => relay_p2p(inner, user_id, place, ServerEvent::RtcOffer { place, from: user_id, sdp }),
        ClientSignal::RtcAnswer { place, sdp } => relay_p2p(inner, user_id, place, ServerEvent::RtcAnswer { place, from: user_id, sdp }),
        ClientSignal::RtcIce {
            place,
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => relay_p2p(
            inner,
            user_id,
            place,
            ServerEvent::RtcIce {
                place,
                from: user_id,
                candidate,
                sdp_mid,
                sdp_mline_index,
            },
        ),
        ClientSignal::SubscribeLayer {
            place,
            publisher,
            track,
            max_layer,
        } => {
            ensure_in_place(inner, user_id, place)?;
            match place {
                MediaPlace::VoiceChannel(_) => {
                    inner.forward.subscribe(user_id, publisher, track, max_layer);
                    let layers = inner.forward.wanted_layers(publisher, track);
                    let subscribers = inner.forward.subscriber_count(publisher, track);
                    Ok(vec![(
                        vec![publisher],
                        ServerEvent::WantedLayers {
                            place,
                            track,
                            layers,
                            subscribers,
                        },
                    )])
                }
                MediaPlace::DirectCall(_) => {
                    let peer = other_party(inner, user_id, place)?;
                    Ok(vec![(
                        vec![peer],
                        ServerEvent::SubscribeLayer {
                            place,
                            subscriber: user_id,
                            publisher,
                            track,
                            max_layer,
                        },
                    )])
                }
            }
        }
    }
}

pub(crate) struct LiveCounts {
    pub online: i64,
    pub idle: i64,
    pub sockets: i64,
    pub voice_members: i64,
    pub voice_channels: i64,
    pub calls: i64,
}

/// Voice signaling stops at the SFU. Direct calls are relayed and the server does not terminate media.
fn relay_p2p(inner: &LiveInner, user_id: i64, place: MediaPlace, event: ServerEvent) -> Result<Vec<(Vec<i64>, ServerEvent)>, BanterError> {
    ensure_in_place(inner, user_id, place)?;
    match place {
        MediaPlace::VoiceChannel(_) => Ok(Vec::new()),
        MediaPlace::DirectCall(_) => {
            let peer = other_party(inner, user_id, place)?;
            Ok(vec![(vec![peer], event)])
        }
    }
}

fn ensure_in_place(inner: &LiveInner, user_id: i64, place: MediaPlace) -> Result<(), BanterError> {
    match place {
        MediaPlace::VoiceChannel(channel_id) => {
            let joined = inner.users.get(&user_id).and_then(|user| user.voice_channel) == Some(channel_id);
            if joined { Ok(()) } else { Err(BanterError::NotFound) }
        }
        MediaPlace::DirectCall(dm_id) => match inner.calls.get(&dm_id) {
            Some(call) if call.phase == CallPhase::Active && (call.caller == user_id || call.callee == user_id) => Ok(()),
            Some(_) => Err(BanterError::NotInCall),
            None => Err(BanterError::NotInCall),
        },
    }
}

fn other_party(inner: &LiveInner, user_id: i64, place: MediaPlace) -> Result<i64, BanterError> {
    match place {
        MediaPlace::VoiceChannel(_) => Err(BanterError::NotFound),
        MediaPlace::DirectCall(dm_id) => {
            let call = inner.calls.get(&dm_id).ok_or(BanterError::NotInCall)?;
            if call.caller == user_id {
                Ok(call.callee)
            } else if call.callee == user_id {
                Ok(call.caller)
            } else {
                Err(BanterError::NotInCall)
            }
        }
    }
}

#[cfg(test)]
mod counts_tests {
    use super::*;

    #[test]
    fn counts_online_members_and_voice_channels() {
        let live = Live::new();
        live.connect(1, 0, None);
        live.seed_voice_member(1, 9);
        live.connect(2, 0, None);
        let counts = live.counts();
        assert_eq!(counts.online, 2);
        assert_eq!(counts.idle, 0);
        assert_eq!(counts.voice_members, 1);
        assert_eq!(counts.voice_channels, 1);
        assert_eq!(counts.calls, 0);
        assert_eq!(counts.sockets, 0);
    }
}

fn take_calls(inner: &mut LiveInner, user_id: i64) -> Vec<(i64, i64)> {
    let ids = calls_involving(&inner.calls, user_id);
    let mut ended = Vec::new();
    for dm_id in ids {
        if let Some(call) = inner.calls.remove(&dm_id) {
            let peer = if call.caller == user_id { call.callee } else { call.caller };
            ended.push((dm_id, peer));
        }
    }
    ended
}
