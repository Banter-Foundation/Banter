//! Background shell reads. The UI thread only applies a snapshot that is already
//! in memory, so a slow GET cannot freeze input or the next present.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use banter_server::{
    Account, CategoryLayout, Channel, ChannelKind, ChatTarget, DirectChat, Friend, FriendRequest,
    Mention, Message, Presence, ServerRail, ServerSummary, VoiceRoster,
};

use crate::api::{ApiError, Client, MemberView};

/// How long the signed-in shell waits between fallback list refreshes.
pub const SHELL_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Avatar image downloads per shell job. Matches the old UI-thread cap.
pub const AVATAR_FETCH_LIMIT: usize = 4;

/// True when a signed-in shell should start another background poll.
/// False while a job is already running, and false until two seconds have elapsed.
pub fn shell_poll_due(last_poll: Instant, now: Instant, in_flight: bool) -> bool {
    !in_flight && now.saturating_duration_since(last_poll) >= SHELL_POLL_INTERVAL
}

/// A finished snapshot is applied only when it was requested for the generation
/// the UI is still on.
pub fn snapshot_applies(snapshot_generation: u64, current_generation: u64) -> bool {
    snapshot_generation == current_generation
}

/// The first `AVATAR_FETCH_LIMIT` entries. The rest wait for a later job.
pub fn avatar_fetch_batch<T>(wanted: &[T]) -> &[T] {
    let end = wanted.len().min(AVATAR_FETCH_LIMIT);
    &wanted[..end]
}

/// First text channel in layout order, or the first channel of any kind.
pub fn first_shell_channel(layout: &[CategoryLayout]) -> Option<&Channel> {
    layout
        .iter()
        .flat_map(|category| category.channels.iter())
        .find(|channel| channel.kind == ChannelKind::Text)
        .or_else(|| {
            layout
                .iter()
                .flat_map(|category| category.channels.iter())
                .next()
        })
}

/// One background shell job at a time. Extra requests collapse into a single follow-up.
pub struct ShellGate {
    in_flight: bool,
    pending: bool,
}

impl ShellGate {
    pub fn new() -> Self {
        Self {
            in_flight: false,
            pending: false,
        }
    }

    /// Starts the job when idle. While one is running, remembers that another must follow.
    pub fn begin(&mut self) -> bool {
        if self.in_flight {
            self.pending = true;
            false
        } else {
            self.in_flight = true;
            true
        }
    }

    /// Marks the in-flight job finished. True when a collapsed follow-up should start.
    pub fn finish(&mut self) -> bool {
        self.in_flight = false;
        if self.pending {
            self.pending = false;
            true
        } else {
            false
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending = false;
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }
}

/// What the next job should download. Flags merge when several UI actions queue
/// before the worker is free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FetchKind {
    pub lists: bool,
    pub place: bool,
    pub messages: bool,
    pub pick_first_text: bool,
}

impl FetchKind {
    pub(crate) fn refresh() -> Self {
        Self {
            lists: true,
            place: true,
            messages: true,
            pick_first_text: false,
        }
    }

    pub(crate) fn messages() -> Self {
        Self {
            lists: false,
            place: false,
            messages: true,
            pick_first_text: false,
        }
    }

    pub(crate) fn place() -> Self {
        Self {
            lists: false,
            place: true,
            messages: true,
            pick_first_text: false,
        }
    }

    pub(crate) fn open_server() -> Self {
        Self {
            lists: false,
            place: true,
            messages: true,
            pick_first_text: true,
        }
    }

    pub(crate) fn any(self) -> bool {
        self.lists || self.place || self.messages || self.pick_first_text
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.lists |= other.lists;
        self.place |= other.place;
        self.messages |= other.messages;
        self.pick_first_text |= other.pick_first_text;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AvatarWant {
    pub user_id: i64,
    pub rev: i64,
}

pub(crate) struct ShellQuery {
    pub generation: u64,
    pub base: String,
    pub token: String,
    pub kind: FetchKind,
    pub server_id: Option<i64>,
    pub chat: Option<ChatTarget>,
    pub search: Option<String>,
    pub avatars: Vec<AvatarWant>,
}

pub(crate) struct AvatarFetch {
    pub user_id: i64,
    pub rev: i64,
    pub bytes: Result<Vec<u8>, ApiError>,
}

pub(crate) struct ShellSnapshot {
    pub generation: u64,
    pub account: Option<Result<Account, ApiError>>,
    pub friends: Option<Result<Vec<Friend>, ApiError>>,
    pub requests: Option<Result<(Vec<FriendRequest>, Vec<FriendRequest>), ApiError>>,
    pub servers: Option<Result<Vec<ServerSummary>, ApiError>>,
    pub rail: Option<Result<ServerRail, ApiError>>,
    pub dms: Option<Result<Vec<DirectChat>, ApiError>>,
    pub mentions: Option<Result<Vec<Mention>, ApiError>>,
    pub presence: Option<Result<Vec<(i64, Presence, Option<String>)>, ApiError>>,
    pub server_id: Option<i64>,
    pub layout: Option<Result<Vec<CategoryLayout>, ApiError>>,
    pub members: Option<Result<Vec<MemberView>, ApiError>>,
    pub voice: Option<Result<Vec<VoiceRoster>, ApiError>>,
    pub chat: Option<ChatTarget>,
    pub messages: Option<Result<Vec<Message>, ApiError>>,
    pub picked_channel: Option<Channel>,
    pub avatars: Vec<AvatarFetch>,
}

pub(crate) enum AvatarFile {
    Ready { rev: i64, bytes: Vec<u8> },
    Failed { rev: i64 },
}

pub(crate) struct ShellHost {
    worker: ShellWorker,
    gate: ShellGate,
    generation: u64,
    last_poll: Instant,
    pub queued: FetchKind,
    pub avatar_wants: Vec<AvatarWant>,
    files: std::collections::HashMap<i64, AvatarFile>,
}

impl ShellHost {
    pub(crate) fn start() -> Self {
        Self {
            worker: ShellWorker::start(),
            gate: ShellGate::new(),
            generation: 0,
            last_poll: Instant::now(),
            queued: FetchKind::default(),
            avatar_wants: Vec::new(),
            files: std::collections::HashMap::new(),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn request(&mut self, kind: FetchKind) {
        self.queued.merge(kind);
        if self.gate.in_flight() {
            let _ = self.gate.begin();
        }
    }

    /// Drop queued work so a logout or host switch does not apply the previous server.
    pub(crate) fn invalidate(&mut self) {
        self.bump();
        self.queued = FetchKind::default();
        self.avatar_wants.clear();
        self.gate.clear_pending();
        self.files.clear();
    }

    pub(crate) fn poll_due(&self, now: Instant) -> bool {
        shell_poll_due(self.last_poll, now, self.gate.in_flight())
    }

    pub(crate) fn mark_polled(&mut self, now: Instant) {
        self.last_poll = now;
    }

    pub(crate) fn in_flight(&self) -> bool {
        self.gate.in_flight()
    }

    pub(crate) fn finish_job(&mut self) -> bool {
        self.gate.finish()
    }

    pub(crate) fn begin_job(&mut self) -> bool {
        self.gate.begin()
    }

    pub(crate) fn submit(&self, query: ShellQuery) {
        self.worker.submit(query);
    }

    pub(crate) fn try_recv(&self) -> Option<ShellSnapshot> {
        self.worker.try_recv()
    }

    pub(crate) fn has_work(&self) -> bool {
        self.queued.any() || !self.avatar_wants.is_empty()
    }

    pub(crate) fn take_avatars(&mut self) -> Vec<AvatarWant> {
        self.avatar_wants
            .retain(|want| match self.files.get(&want.user_id) {
                Some(AvatarFile::Ready { rev, .. }) => *rev != want.rev,
                Some(AvatarFile::Failed { rev }) => *rev != want.rev,
                None => true,
            });
        let count = avatar_fetch_batch(&self.avatar_wants).len();
        self.avatar_wants.drain(..count).collect()
    }

    pub(crate) fn note_want(&mut self, want: AvatarWant) -> bool {
        let known = match self.files.get(&want.user_id) {
            Some(AvatarFile::Ready { rev, .. }) => *rev == want.rev,
            Some(AvatarFile::Failed { rev }) => *rev == want.rev,
            None => false,
        };
        if known
            || self
                .avatar_wants
                .iter()
                .any(|queued| queued.user_id == want.user_id && queued.rev == want.rev)
        {
            return false;
        }
        self.avatar_wants.push(want);
        true
    }

    pub(crate) fn store_avatar(
        &mut self,
        user_id: i64,
        rev: i64,
        bytes: Result<Vec<u8>, ApiError>,
    ) {
        let file = match bytes {
            Ok(bytes) => AvatarFile::Ready { rev, bytes },
            Err(_) => AvatarFile::Failed { rev },
        };
        self.files.insert(user_id, file);
    }

    pub(crate) fn avatar_file(&self, user_id: i64) -> Option<&AvatarFile> {
        self.files.get(&user_id)
    }

    pub(crate) fn forget_avatar(&mut self, user_id: i64) {
        self.files.remove(&user_id);
    }
}

struct ShellWorker {
    tx: Sender<ShellQuery>,
    rx: Receiver<ShellSnapshot>,
}

impl ShellWorker {
    fn start() -> Self {
        let (tx, cmd_rx) = mpsc::channel();
        let (snap_tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("banter-shell".into())
            .spawn(move || {
                while let Ok(query) = cmd_rx.recv() {
                    let snap = fetch_shell(&query);
                    if snap_tx.send(snap).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn shell sync");
        Self { tx, rx }
    }

    fn submit(&self, query: ShellQuery) {
        let _ = self.tx.send(query);
    }

    fn try_recv(&self) -> Option<ShellSnapshot> {
        self.rx.try_recv().ok()
    }
}

fn fetch_shell(query: &ShellQuery) -> ShellSnapshot {
    let mut client = Client::new(&query.base);
    if !query.token.is_empty() {
        client.set_token(Some(query.token.clone()));
    }
    let mut snap = ShellSnapshot {
        generation: query.generation,
        account: None,
        friends: None,
        requests: None,
        servers: None,
        rail: None,
        dms: None,
        mentions: None,
        presence: None,
        server_id: None,
        layout: None,
        members: None,
        voice: None,
        chat: None,
        messages: None,
        picked_channel: None,
        avatars: Vec::new(),
    };
    if query.kind.lists {
        snap.account = Some(client.me());
        snap.friends = Some(client.friends());
        snap.requests = Some(client.friend_requests());
        snap.servers = Some(client.servers());
        snap.rail = Some(client.server_rail());
        snap.dms = Some(client.direct_messages());
        snap.mentions = Some(client.mentions());
        snap.presence = Some(client.presence());
    }
    if query.kind.place {
        if let Some(server_id) = query.server_id {
            snap.server_id = Some(server_id);
            snap.layout = Some(client.layout(server_id));
            snap.members = Some(client.members(server_id));
            snap.voice = Some(client.server_voice(server_id));
        }
    }
    let mut chat = query.chat;
    if chat.is_none() && query.kind.pick_first_text {
        if let Some(Ok(layout)) = snap.layout.as_ref() {
            if let Some(channel) = first_shell_channel(layout).cloned() {
                snap.picked_channel = Some(channel.clone());
                if channel.kind == ChannelKind::Text {
                    chat = Some(ChatTarget::Channel(channel.id));
                }
            }
        }
    }
    if query.kind.messages {
        if let Some(target) = chat {
            snap.chat = Some(target);
            snap.messages = Some(client.messages(target, 100, query.search.as_deref()));
        }
    }
    for want in &query.avatars {
        snap.avatars.push(AvatarFetch {
            user_id: want.user_id,
            rev: want.rev,
            bytes: client.avatar(want.user_id),
        });
    }
    snap
}
