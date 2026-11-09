//! Capture → convert pipeline for screen share.
//!
//! The screen loop only grabs / drains sources. A dedicated convert thread waits
//! on the latest job, turns it into I420, and publishes into [`PictureSlot`].
//! Hand-off is **latest-wins** (single slot): if convert is slow, intermediate
//! frames are dropped instead of queuing latency — same interactive contract as
//! PipeWire / DXGI already use.
//!
//! Stages:
//! 1. capture thread (`screen_loop`) — OS / PipeWire / DXGI grab
//! 2. convert thread (`convert_loop`) — scale + BT.601 / Cast encode
//! 3. encode thread (`banter-video`) — already separate in [`super::rtc`]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use super::capture_metrics;
use super::encode;
use super::rtc::{self, PictureSlot};
use super::Engine;

#[cfg(target_os = "linux")]
use super::screencast::{self, RawFrame};

/// Work waiting for the convert thread. Overwritten while convert is busy.
enum ConvertJob {
    /// Still / DXGI Record path: already-RGBA buffer from the capture thread.
    Rgba {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        edge: u32,
        want_preview: bool,
        grabbed_at: Instant,
        generation: u64,
    },
    /// Wayland Cast: unscaled PipeWire copy; convert walks pixels here.
    #[cfg(target_os = "linux")]
    Cast {
        raw: RawFrame,
        edge: u32,
        want_preview: bool,
        grabbed_at: Instant,
        generation: u64,
    },
}

struct Inbox {
    job: Mutex<Option<ConvertJob>>,
    ready: Condvar,
}

impl Inbox {
    fn new() -> Self {
        Self {
            job: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn push(&self, job: ConvertJob) {
        let mut slot = self.job.lock().expect("screen convert inbox");
        let dropped = slot.is_some();
        *slot = Some(job);
        if dropped {
            // Latest-wins: a pending convert job was replaced before the worker took it.
            capture_metrics::note_stale_drop();
        }
        self.ready.notify_one();
    }

    fn clear(&self) {
        let mut slot = self.job.lock().expect("screen convert inbox");
        *slot = None;
    }

    fn take_wait(&self, stop: &AtomicBool) -> Option<ConvertJob> {
        let mut slot = self.job.lock().expect("screen convert inbox");
        loop {
            if let Some(job) = slot.take() {
                return Some(job);
            }
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            // Timed wait so Media::drop can join without a separate wake path.
            let (guard, _) = self
                .ready
                .wait_timeout(slot, std::time::Duration::from_millis(50))
                .expect("screen convert inbox");
            slot = guard;
        }
    }

    fn wake(&self) {
        self.ready.notify_one();
    }
}

/// Producer side used by `screen_loop`.
pub struct ScreenConvertTx {
    inbox: Arc<Inbox>,
    generation: Arc<AtomicU64>,
}

impl ScreenConvertTx {
    /// Drop any pending job and invalidate in-flight work after a feed reset.
    pub fn reset(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.inbox.clear();
    }

    pub fn push_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        edge: u32,
        want_preview: bool,
        grabbed_at: Instant,
    ) {
        self.inbox.push(ConvertJob::Rgba {
            width,
            height,
            rgba,
            edge,
            want_preview,
            grabbed_at,
            generation: self.generation.load(Ordering::Relaxed),
        });
    }

    #[cfg(target_os = "linux")]
    pub fn push_cast(
        &self,
        raw: RawFrame,
        edge: u32,
        want_preview: bool,
        grabbed_at: Instant,
    ) {
        self.inbox.push(ConvertJob::Cast {
            raw,
            edge,
            want_preview,
            grabbed_at,
            generation: self.generation.load(Ordering::Relaxed),
        });
    }

    /// Unblock the convert worker (e.g. on shutdown).
    pub fn wake(&self) {
        self.inbox.wake();
    }
}

/// Spawn the convert worker. Stops when `stop` is set and the inbox is drained.
pub fn start_convert_thread(
    stop: Arc<AtomicBool>,
    local: Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>,
    engine: Arc<Engine>,
    pictures: Arc<PictureSlot>,
) -> (ScreenConvertTx, JoinHandle<()>) {
    let inbox = Arc::new(Inbox::new());
    let generation = Arc::new(AtomicU64::new(0));
    let tx = ScreenConvertTx {
        inbox: Arc::clone(&inbox),
        generation: Arc::clone(&generation),
    };
    let handle = thread::Builder::new()
        .name("banter-screen-convert".into())
        .spawn(move || {
            convert_loop(stop, inbox, generation, local, engine, pictures);
        })
        .expect("spawn screen convert");
    (tx, handle)
}

fn convert_loop(
    stop: Arc<AtomicBool>,
    inbox: Arc<Inbox>,
    generation: Arc<AtomicU64>,
    local: Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>,
    engine: Arc<Engine>,
    pictures: Arc<PictureSlot>,
) {
    let mut keyed: Option<(u32, u32)> = None;
    let mut seen_gen = generation.load(Ordering::Relaxed);
    while let Some(job) = inbox.take_wait(&stop) {
        let job_gen = job_generation(&job);
        let live_gen = generation.load(Ordering::Relaxed);
        if job_gen != live_gen {
            // Feed was reset while this frame sat in the slot — drop it.
            if live_gen != seen_gen {
                keyed = None;
                seen_gen = live_gen;
            }
            capture_metrics::note_stale_drop();
            continue;
        }
        if live_gen != seen_gen {
            keyed = None;
            seen_gen = live_gen;
        }
        let convert_started = Instant::now();
        match job {
            ConvertJob::Rgba {
                width,
                height,
                rgba,
                edge,
                want_preview,
                grabbed_at,
                ..
            } => {
                publish_rgba(
                    &pictures,
                    &local,
                    &engine,
                    &mut keyed,
                    &rgba,
                    width,
                    height,
                    edge,
                    want_preview,
                    grabbed_at,
                );
            }
            #[cfg(target_os = "linux")]
            ConvertJob::Cast {
                raw,
                edge,
                want_preview,
                grabbed_at,
                ..
            } => {
                // Skip preview pixel work when the local tile is hidden.
                let want_preview =
                    want_preview && engine.show_preview.load(Ordering::Relaxed);
                let Some((width, height, i420, preview)) =
                    screencast::encode_raw_frame(&raw, edge, want_preview)
                else {
                    continue;
                };
                publish_i420(
                    &pictures,
                    &local,
                    &engine,
                    &mut keyed,
                    i420,
                    width,
                    height,
                    if want_preview { preview } else { None },
                    grabbed_at,
                );
            }
        }
        capture_metrics::note_converted(convert_started.elapsed());
        if stop.load(Ordering::Relaxed) && inbox.job.lock().expect("inbox").is_none() {
            break;
        }
    }
}

fn job_generation(job: &ConvertJob) -> u64 {
    match job {
        ConvertJob::Rgba { generation, .. } => *generation,
        #[cfg(target_os = "linux")]
        ConvertJob::Cast { generation, .. } => *generation,
    }
}

fn publish_rgba(
    pictures: &PictureSlot,
    local: &Mutex<Option<(u32, u32, Vec<u8>)>>,
    engine: &Engine,
    keyed: &mut Option<(u32, u32)>,
    rgba: &[u8],
    width: u32,
    height: u32,
    edge: u32,
    want_preview: bool,
    grabbed_at: Instant,
) {
    let (out_w, out_h) = encode::even_fit(width, height, edge);
    if out_w < 2 || out_h < 2 {
        return;
    }
    let i420 = encode::scale_rgba_to_i420(rgba, width, height, out_w, out_h);
    if i420.is_empty() {
        return;
    }
    // Preview only when the UI asked for it — never scale twice for a hidden tile.
    let preview = if want_preview && engine.show_preview.load(Ordering::Relaxed) {
        let (small, preview_w, preview_h) =
            encode::fit_rgba(rgba, width, height, 1280.min(edge.max(2)));
        if preview_w >= 2 && preview_h >= 2 {
            Some((preview_w, preview_h, small))
        } else {
            None
        }
    } else {
        None
    };
    publish_i420(
        pictures,
        local,
        engine,
        keyed,
        i420,
        out_w,
        out_h,
        preview,
        grabbed_at,
    );
}

fn publish_i420(
    pictures: &PictureSlot,
    local: &Mutex<Option<(u32, u32, Vec<u8>)>>,
    engine: &Engine,
    keyed: &mut Option<(u32, u32)>,
    i420: Vec<u8>,
    width: u32,
    height: u32,
    preview: Option<(u32, u32, Vec<u8>)>,
    grabbed_at: Instant,
) {
    if width < 2 || height < 2 || i420.is_empty() {
        return;
    }
    let key = picture_needs_key(keyed, width, height);
    if engine.show_preview.load(Ordering::Relaxed) {
        *local.lock().expect("screen") = preview;
    } else {
        *local.lock().expect("screen") = None;
    }
    pictures.store(rtc::RawPicture {
        width,
        height,
        i420: std::sync::Arc::<[u8]>::from(i420),
        key,
    });
    let elapsed = grabbed_at.elapsed();
    super::screen_lag::note_pipeline_lag_ms(elapsed.as_millis() as u64);
    crate::profiler::record_media(
        "screen_capture",
        elapsed,
        (width * height) as usize,
    );
}

fn picture_needs_key(keyed: &mut Option<(u32, u32)>, width: u32, height: u32) -> bool {
    if *keyed == Some((width, height)) {
        false
    } else {
        *keyed = Some((width, height));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn inbox_overwrites_pending_job() {
        let inbox = Inbox::new();
        let t0 = Instant::now();
        inbox.push(ConvertJob::Rgba {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            edge: 2,
            want_preview: false,
            grabbed_at: t0,
            generation: 0,
        });
        inbox.push(ConvertJob::Rgba {
            width: 4,
            height: 4,
            rgba: vec![1; 64],
            edge: 4,
            want_preview: false,
            grabbed_at: t0,
            generation: 0,
        });
        let stop = AtomicBool::new(true);
        // With stop already true, take_wait still returns the pending job first.
        let job = inbox.take_wait(&stop).expect("job");
        match job {
            ConvertJob::Rgba { width, height, .. } => {
                assert_eq!((width, height), (4, 4));
            }
            #[cfg(target_os = "linux")]
            ConvertJob::Cast { .. } => panic!("expected rgba"),
        }
        assert!(inbox.take_wait(&AtomicBool::new(true)).is_none());
    }
}
