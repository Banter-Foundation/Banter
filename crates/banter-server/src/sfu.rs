//! Voice SFU. RTP is forwarded. Nothing here decodes or transcodes.
//!
//! Each subscriber has a bounded queue. Video is dropped before audio. A layer with
//! no subscribers is paused (dynacast) until the next subscribe, which requests a keyframe.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::{LayerId, MediaKind, deliverable_layer};

pub const VIDEO_QUEUE_CAP: usize = 64;
pub const AUDIO_QUEUE_CAP: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackKey {
    pub publisher: i64,
    pub track: MediaKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forwarded {
    pub subscriber: i64,
    pub publisher: i64,
    pub track: MediaKind,
    pub layer: LayerId,
    pub bytes: usize,
    pub keyframe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyframeRequest {
    pub publisher: i64,
    pub track: MediaKind,
    pub layer: LayerId,
}

#[derive(Clone, Debug)]
struct Sub {
    max_layer: LayerId,
}

#[derive(Debug)]
struct Queue {
    items: VecDeque<Forwarded>,
}

impl Queue {
    fn new() -> Self {
        Self { items: VecDeque::new() }
    }

    fn push(&mut self, item: Forwarded, drops: &mut u64) {
        self.items.push_back(item);
        loop {
            let cap = if self.items.iter().any(|queued| queued.track.is_video()) {
                VIDEO_QUEUE_CAP
            } else {
                AUDIO_QUEUE_CAP
            };
            if self.items.len() <= cap {
                break;
            }
            if let Some(index) = self.items.iter().position(|queued| queued.track.is_video()) {
                self.items.remove(index);
            } else {
                self.items.pop_front();
            }
            *drops = drops.saturating_add(1);
        }
    }

    fn pop(&mut self) -> Option<Forwarded> {
        if let Some(index) = self.items.iter().position(|queued| queued.track.is_audio()) {
            return self.items.remove(index);
        }
        self.items.pop_front()
    }
}

/// Per-subscriber forward table. `decoded` stays zero: the SFU never looks inside a payload.
#[derive(Debug)]
pub struct ForwardTable {
    subs: HashMap<(i64, TrackKey), Sub>,
    /// Layers the publisher is currently encoding. Missing means all three, until dynacast says otherwise.
    publishing: HashMap<TrackKey, HashSet<LayerId>>,
    queues: HashMap<i64, Queue>,
    egress: HashMap<i64, u64>,
    drops: u64,
    decoded: u64,
    keyframes: Vec<KeyframeRequest>,
}

impl ForwardTable {
    pub fn new() -> Self {
        Self {
            subs: HashMap::new(),
            publishing: HashMap::new(),
            queues: HashMap::new(),
            egress: HashMap::new(),
            drops: 0,
            decoded: 0,
            keyframes: Vec::new(),
        }
    }

    pub fn decoded_packets(&self) -> u64 {
        self.decoded
    }

    pub fn drops(&self) -> u64 {
        self.drops
    }

    pub fn egress(&self, subscriber: i64) -> u64 {
        self.egress.get(&subscriber).copied().unwrap_or(0)
    }

    pub fn take_keyframes(&mut self) -> Vec<KeyframeRequest> {
        std::mem::take(&mut self.keyframes)
    }

    pub fn subscribe(&mut self, subscriber: i64, publisher: i64, track: MediaKind, max_layer: LayerId) {
        let key = TrackKey { publisher, track };
        let previous = self.subs.get(&(subscriber, key)).map(|sub| sub.max_layer);
        self.subs.insert((subscriber, key), Sub { max_layer });
        let delivered = self.chosen(subscriber, key);
        if previous != Some(max_layer) && delivered != LayerId::Off && track.is_video() {
            self.keyframes.push(KeyframeRequest {
                publisher,
                track,
                layer: delivered,
            });
        }
        self.refresh_dynacast(key, true);
    }

    pub fn unsubscribe_user(&mut self, subscriber: i64) {
        let keys: Vec<TrackKey> = self
            .subs
            .keys()
            .filter(|(user, _)| *user == subscriber)
            .map(|(_, key)| *key)
            .collect();
        self.subs.retain(|(user, _), _| *user != subscriber);
        self.queues.remove(&subscriber);
        for key in keys {
            self.refresh_dynacast(key, false);
        }
    }

    pub fn note_layer(&mut self, publisher: i64, track: MediaKind, layer: LayerId) {
        if layer == LayerId::Off {
            return;
        }
        let key = TrackKey { publisher, track };
        self.publishing.entry(key).or_default().insert(layer);
    }

    pub fn set_publishing(&mut self, publisher: i64, track: MediaKind, layers: impl IntoIterator<Item = LayerId>) {
        let key = TrackKey { publisher, track };
        let set = layers.into_iter().filter(|layer| *layer != LayerId::Off).collect();
        self.publishing.insert(key, set);
    }

    /// Layers with at least one subscriber. The publisher pauses every other spatial layer.
    pub fn wanted_layers(&self, publisher: i64, track: MediaKind) -> Vec<LayerId> {
        let key = TrackKey { publisher, track };
        if !track.is_video() {
            let any = self.subs.iter().any(|((_, existing), sub)| *existing == key && sub.max_layer != LayerId::Off);
            return if any { vec![LayerId::Full] } else { Vec::new() };
        }
        let mut ranks = HashSet::new();
        for ((_, existing), sub) in &self.subs {
            if *existing == key && sub.max_layer != LayerId::Off {
                ranks.insert(sub.max_layer);
            }
        }
        let mut layers: Vec<LayerId> = ranks.into_iter().collect();
        layers.sort_by_key(|layer| layer.rank());
        layers
    }

    pub fn subscriber_count(&self, publisher: i64, track: MediaKind) -> u32 {
        let key = TrackKey { publisher, track };
        self.subs
            .iter()
            .filter(|((_, existing), sub)| *existing == key && sub.max_layer != LayerId::Off)
            .count() as u32
    }

    /// Push one RTP payload. Returns the packets released to subscribers on this call.
    /// The payload is not inspected. `bytes` is the forwarded size.
    pub fn push(&mut self, publisher: i64, track: MediaKind, layer: LayerId, bytes: usize, keyframe: bool) -> Vec<Forwarded> {
        let key = TrackKey { publisher, track };
        let subscribers: Vec<(i64, LayerId)> = self
            .subs
            .iter()
            .filter(|((_, existing), _)| *existing == key)
            .map(|((subscriber, _), sub)| {
                let chosen = deliverable_layer(sub.max_layer, LayerId::Full, &self.published_set(key));
                (*subscriber, chosen)
            })
            .filter(|(_, chosen)| *chosen == layer)
            .collect();
        let mut released = Vec::new();
        for (subscriber, chosen) in subscribers {
            let item = Forwarded {
                subscriber,
                publisher,
                track,
                layer: chosen,
                bytes,
                keyframe,
            };
            let queue = self.queues.entry(subscriber).or_insert_with(Queue::new);
            queue.push(item, &mut self.drops);
            if let Some(out) = queue.pop() {
                *self.egress.entry(subscriber).or_insert(0) += out.bytes as u64;
                released.push(out);
            }
        }
        released
    }

    /// Enqueue without draining. Overload drops the oldest video packet before audio.
    pub fn buffer(&mut self, publisher: i64, track: MediaKind, layer: LayerId, bytes: usize) {
        let key = TrackKey { publisher, track };
        let subscribers: Vec<i64> = self
            .subs
            .iter()
            .filter(|((_, existing), sub)| {
                *existing == key && deliverable_layer(sub.max_layer, LayerId::Full, &self.published_set(key)) == layer
            })
            .map(|((subscriber, _), _)| *subscriber)
            .collect();
        for subscriber in subscribers {
            let item = Forwarded {
                subscriber,
                publisher,
                track,
                layer,
                bytes,
                keyframe: false,
            };
            self.queues.entry(subscriber).or_insert_with(Queue::new).push(item, &mut self.drops);
        }
    }

    pub fn queued(&self, subscriber: i64) -> usize {
        self.queues.get(&subscriber).map(|queue| queue.items.len()).unwrap_or(0)
    }

    /// Drain one queued packet. Tests that only care about byte totals use `egress`.
    pub fn pop_one(&mut self, subscriber: i64) -> Option<Forwarded> {
        self.queues.get_mut(&subscriber)?.pop()
    }

    fn chosen(&self, subscriber: i64, key: TrackKey) -> LayerId {
        let Some(sub) = self.subs.get(&(subscriber, key)) else {
            return LayerId::Off;
        };
        deliverable_layer(sub.max_layer, LayerId::Full, &self.published_set(key))
    }

    fn published_set(&self, key: TrackKey) -> Vec<LayerId> {
        self.publishing.get(&key).map(|set| set.iter().copied().collect()).unwrap_or_default()
    }

    fn refresh_dynacast(&mut self, key: TrackKey, request_keyframe: bool) {
        if !key.track.is_video() {
            return;
        }
        let wanted = self.wanted_layers(key.publisher, key.track);
        let wanted_set: HashSet<LayerId> = wanted.iter().copied().collect();
        let was = self.publishing.get(&key).cloned().unwrap_or_default();
        if request_keyframe {
            for layer in &wanted_set {
                if !was.contains(layer) {
                    self.keyframes.push(KeyframeRequest {
                        publisher: key.publisher,
                        track: key.track,
                        layer: *layer,
                    });
                }
            }
        }
        self.publishing.insert(key, wanted_set);
    }
}

impl Default for ForwardTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Eight synthetic publishers, mixed slider settings. Egress follows the selected layer.
pub fn bench_eight_publishers(packets: usize) -> BenchReport {
    let mut table = ForwardTable::new();
    let full = 1200usize;
    let half = 400usize;
    let quarter = 120usize;
    let audio = 80usize;
    for publisher in 1..=8 {
        table.set_publishing(publisher, MediaKind::Screen, [LayerId::Full, LayerId::Half, LayerId::Quarter]);
        table.set_publishing(publisher, MediaKind::UserAudio, [LayerId::Full]);
    }
    // Subscriber 100 wants full from everyone. Subscriber 200 wants quarter.
    // Subscriber 300 wants half from the even publishers and off from the odd ones.
    for publisher in 1..=8 {
        table.subscribe(100, publisher, MediaKind::Screen, LayerId::Full);
        table.subscribe(100, publisher, MediaKind::UserAudio, LayerId::Full);
        table.subscribe(200, publisher, MediaKind::Screen, LayerId::Quarter);
        table.subscribe(200, publisher, MediaKind::UserAudio, LayerId::Full);
        let screen = if publisher % 2 == 0 { LayerId::Half } else { LayerId::Off };
        table.subscribe(300, publisher, MediaKind::Screen, screen);
        table.subscribe(300, publisher, MediaKind::UserAudio, LayerId::Full);
    }
    for _ in 0..packets {
        for publisher in 1..=8 {
            table.push(publisher, MediaKind::UserAudio, LayerId::Full, audio, false);
            table.push(publisher, MediaKind::Screen, LayerId::Full, full, false);
            table.push(publisher, MediaKind::Screen, LayerId::Half, half, false);
            table.push(publisher, MediaKind::Screen, LayerId::Quarter, quarter, false);
        }
    }
    // Nobody wanted a layer the table decoded.
    let full_wanted = table.wanted_layers(1, MediaKind::Screen);
    BenchReport {
        decoded: table.decoded_packets(),
        egress_full: table.egress(100),
        egress_quarter: table.egress(200),
        egress_mixed: table.egress(300),
        publisher_layers: full_wanted,
        drops: table.drops(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchReport {
    pub decoded: u64,
    pub egress_full: u64,
    pub egress_quarter: u64,
    pub egress_mixed: u64,
    pub publisher_layers: Vec<LayerId>,
    pub drops: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_subscribers_get_different_layers() {
        let mut table = ForwardTable::new();
        table.set_publishing(7, MediaKind::Screen, [LayerId::Full, LayerId::Half, LayerId::Quarter]);
        table.subscribe(1, 7, MediaKind::Screen, LayerId::Full);
        table.subscribe(2, 7, MediaKind::Screen, LayerId::Quarter);
        for _ in 0..10 {
            table.push(7, MediaKind::Screen, LayerId::Full, 1000, false);
            table.push(7, MediaKind::Screen, LayerId::Half, 400, false);
            table.push(7, MediaKind::Screen, LayerId::Quarter, 100, false);
        }
        assert_eq!(table.egress(1), 10_000);
        assert_eq!(table.egress(2), 1_000);
        assert_eq!(table.decoded_packets(), 0);
    }

    #[test]
    fn dynacast_pauses_the_top_layer_until_someone_subscribes() {
        let mut table = ForwardTable::new();
        table.subscribe(1, 4, MediaKind::Screen, LayerId::Quarter);
        let wanted = table.wanted_layers(4, MediaKind::Screen);
        assert!(!wanted.contains(&LayerId::Full));
        assert!(wanted.contains(&LayerId::Quarter));
        table.subscribe(2, 4, MediaKind::Screen, LayerId::Full);
        let wanted = table.wanted_layers(4, MediaKind::Screen);
        assert!(wanted.contains(&LayerId::Full));
        let keys = table.take_keyframes();
        assert!(keys.iter().any(|key| key.layer == LayerId::Full && key.publisher == 4));
    }

    #[test]
    fn video_queue_drops_oldest_instead_of_growing() {
        let mut table = ForwardTable::new();
        table.set_publishing(2, MediaKind::Camera, [LayerId::Full]);
        table.subscribe(1, 2, MediaKind::Camera, LayerId::Full);
        for _ in 0..(VIDEO_QUEUE_CAP + 40) {
            table.buffer(2, MediaKind::Camera, LayerId::Full, 10);
        }
        assert_eq!(table.queued(1), VIDEO_QUEUE_CAP);
        assert_eq!(table.drops(), 40);
        table.subscribe(1, 2, MediaKind::UserAudio, LayerId::Full);
        table.set_publishing(2, MediaKind::UserAudio, [LayerId::Full]);
        let before = table.drops();
        table.buffer(2, MediaKind::UserAudio, LayerId::Full, 4);
        assert!(table.drops() > before, "audio is kept by dropping a video packet");
        assert!(table.queued(1) <= VIDEO_QUEUE_CAP);
        let drained = table.pop_one(1).expect("audio is forwarded first");
        assert_eq!(drained.track, MediaKind::UserAudio);
    }

    #[test]
    fn eight_publishers_do_not_decode_and_egress_tracks_the_layer() {
        let report = bench_eight_publishers(20);
        assert_eq!(report.decoded, 0);
        let publishers = 8u64;
        let packets = 20u64;
        let audio = publishers * packets * 80;
        assert_eq!(report.egress_full, audio + publishers * packets * 1200);
        assert_eq!(report.egress_quarter, audio + publishers * packets * 120);
        // Even publishers (4) at half, odd publishers contribute audio only.
        let mixed_video = 4 * packets * 400;
        assert_eq!(report.egress_mixed, audio + mixed_video);
        assert!(report.publisher_layers.contains(&LayerId::Full));
    }

    #[test]
    fn layer_switch_asks_for_a_keyframe() {
        let mut table = ForwardTable::new();
        table.subscribe(1, 9, MediaKind::Screen, LayerId::Quarter);
        table.take_keyframes();
        table.subscribe(1, 9, MediaKind::Screen, LayerId::Full);
        let keys = table.take_keyframes();
        assert!(keys.iter().any(|key| key.layer == LayerId::Full));
    }
}
