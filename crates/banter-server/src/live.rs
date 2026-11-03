use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc::UnboundedSender;

use crate::error::BanterError;
use crate::model::*;
use crate::store::Store;

#[derive(Clone, Debug)]
pub(crate) enum Outbound {
    Text(String),
    Binary(Vec<u8>),
}

struct UserLive {
    connected: bool,
    last_activity: i64,
    presence: Presence,
    flags: VoiceFlags,
    voice_channel: Option<i64>,
}

struct LiveInner {
    users: HashMap<i64, UserLive>,
    calls: HashMap<i64, CallSnapshot>,
    sockets: HashMap<i64, Vec<(u64, UnboundedSender<Outbound>)>>,
    next_socket: u64,
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
                next_socket: 1,
            }),
        }
    }

    pub fn connect(&self, user_id: i64, now: i64) {
        let mut inner = self.lock();
        let user = entry(&mut inner, user_id);
        user.connected = true;
        user.last_activity = now;
        user.presence = Presence::Online;
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
        self.lock()
            .users
            .get(&user_id)
            .map(|user| user.presence)
            .unwrap_or(Presence::Offline)
    }

    pub fn disconnect(&self, user_id: i64) {
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
            ended_calls = take_calls(&mut inner, user_id);
        }
        for channel_id in channels {
            self.broadcast_voice(channel_id, &[user_id]);
        }
        for (dm_id, peer) in ended_calls {
            self.send_event(&[user_id, peer], &ServerEvent::CallEnded { dm_id });
        }
    }

    pub fn join_server_voice(&self, store: &Store, user_id: i64, channel_id: i64) -> Result<(), BanterError> {
        let channel = store.channel_access(user_id, channel_id)?;
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
                self.broadcast_voice(previous, &[user_id]);
            }
        }
        self.broadcast_voice(channel_id, &[]);
        for (dm_id, peer) in ended {
            self.send_event(&[user_id, peer], &ServerEvent::CallEnded { dm_id });
        }
        Ok(())
    }

    pub fn leave_voice(&self, user_id: i64) {
        let previous = {
            let mut inner = self.lock();
            let Some(user) = inner.users.get_mut(&user_id) else {
                return;
            };
            user.voice_channel.take()
        };
        if let Some(channel_id) = previous {
            self.broadcast_voice(channel_id, &[user_id]);
        }
    }

    pub fn set_flags(&self, user_id: i64, flags: VoiceFlags) {
        let channel = {
            let mut inner = self.lock();
            let user = entry(&mut inner, user_id);
            user.flags = flags;
            user.voice_channel
        };
        if let Some(channel_id) = channel {
            self.broadcast_voice(channel_id, &[]);
        }
    }

    pub fn voice_channel_members(&self, channel_id: i64) -> Vec<VoiceMember> {
        let inner = self.lock();
        members_of(&inner, channel_id)
    }

    pub fn start_dm_call(&self, store: &Store, user_id: i64, dm_id: i64) -> Result<CallSnapshot, BanterError> {
        let (_me, peer) = store.dm_participants(user_id, dm_id)?;
        let snapshot = {
            let mut inner = self.lock();
            if let Some(existing) = inner.calls.get(&dm_id).cloned() {
                if existing.caller == user_id || existing.callee == user_id {
                    if existing.phase == CallPhase::Ringing && existing.callee == user_id {
                        let mut active = existing;
                        active.phase = CallPhase::Active;
                        inner.calls.insert(dm_id, active.clone());
                        if let Some(user) = inner.users.get_mut(&user_id) {
                            user.voice_channel = None;
                        }
                        active
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
                snapshot
            }
        };
        self.send_event(&[snapshot.caller, snapshot.callee], &ServerEvent::Call { call: snapshot.clone() });
        Ok(snapshot)
    }

    pub fn accept_dm_call(&self, _store: &Store, user_id: i64, dm_id: i64) -> Result<CallSnapshot, BanterError> {
        let snapshot = {
            let mut inner = self.lock();
            let Some(call) = inner.calls.get(&dm_id).cloned() else {
                return Err(BanterError::NotInCall);
            };
            if call.callee != user_id {
                return Err(BanterError::NotInCall);
            }
            let mut call = call;
            call.phase = CallPhase::Active;
            inner.calls.insert(dm_id, call.clone());
            if let Some(user) = inner.users.get_mut(&user_id) {
                user.voice_channel = None;
            }
            call
        };
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

    pub fn route_media(
        &self,
        sender: i64,
        place: MediaPlace,
        kind: MediaKind,
        payload: &[u8],
    ) -> Result<Vec<RoutedMedia>, BanterError> {
        let inner = self.lock();
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
        let send = match kind {
            MediaKind::Audio => should_send_audio(&sender_state.flags),
            MediaKind::Camera => should_send_camera(&sender_state.flags),
            MediaKind::Screen => should_send_screen(&sender_state.flags),
        };
        if !send {
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
            if kind == MediaKind::Audio && !should_receive_audio(&flags) {
                continue;
            }
            routed.push(RoutedMedia {
                user_id,
                kind,
                bytes: payload.to_vec(),
            });
        }
        Ok(routed)
    }

    pub(crate) fn register_socket(&self, user_id: i64, tx: UnboundedSender<Outbound>) -> u64 {
        let mut inner = self.lock();
        let id = inner.next_socket;
        inner.next_socket += 1;
        inner.sockets.entry(user_id).or_default().push((id, tx));
        id
    }

    pub(crate) fn unregister_socket(&self, user_id: i64, socket_id: u64) {
        let mut inner = self.lock();
        if let Some(list) = inner.sockets.get_mut(&user_id) {
            list.retain(|(id, _)| *id != socket_id);
        }
    }

    pub fn send_event(&self, user_ids: &[i64], event: &ServerEvent) {
        let Ok(text) = serde_json::to_string(event) else {
            return;
        };
        let inner = self.lock();
        for user_id in user_ids {
            if let Some(list) = inner.sockets.get(user_id) {
                for (_, tx) in list {
                    let _ = tx.send(Outbound::Text(text.clone()));
                }
            }
        }
    }

    pub(crate) fn send_binary(&self, user_id: i64, bytes: Vec<u8>) {
        let inner = self.lock();
        if let Some(list) = inner.sockets.get(&user_id) {
            for (_, tx) in list {
                let _ = tx.send(Outbound::Binary(bytes.clone()));
            }
        }
    }

    fn broadcast_voice(&self, channel_id: i64, also: &[i64]) {
        let members = {
            let inner = self.lock();
            members_of(&inner, channel_id)
        };
        let mut ids: Vec<i64> = members.iter().map(|member| member.user_id).collect();
        ids.extend_from_slice(also);
        self.send_event(&ids, &ServerEvent::Voice { channel_id, members });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LiveInner> {
        self.inner.lock().expect("live lock")
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
        flags: VoiceFlags::default(),
        voice_channel: None,
    })
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
