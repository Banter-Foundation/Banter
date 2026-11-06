//! UI sound effects for voice, mute, stream, and call ringtone.
//!
//! Clips are embedded Ogg Vorbis under `assets/sfx/`. Playback uses a dedicated
//! cpal output stream so local feedback still works while deafened and before a
//! voice media session is up.
//!
//! The DM ringtone plays once (no loop). Call UI times out when
//! [`ringtone_duration`] elapses while still ringing.

use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use lewton::inside_ogg::OggStreamReader;

/// One-shot Banter UI sounds. Ringtone is controlled separately (plays once while ringing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent {
    Join,
    Leave,
    Mute,
    Unmute,
    StartStream,
    StopStream,
}

impl SoundEvent {
    /// Asset filename under `crates/banter/assets/sfx/`.
    pub fn asset_file(self) -> &'static str {
        match self {
            Self::Join => "join_call.ogg",
            Self::Leave => "leave_call.ogg",
            Self::Mute => "mute.ogg",
            Self::Unmute => "unmute.ogg",
            Self::StartStream => "start_stream.ogg",
            Self::StopStream => "stop_stream.ogg",
        }
    }

    pub fn all() -> &'static [SoundEvent] {
        &[
            Self::Join,
            Self::Leave,
            Self::Mute,
            Self::Unmute,
            Self::StartStream,
            Self::StopStream,
        ]
    }
}

/// Embedded ringtone asset name (plays once while a DM call is ringing).
pub const RINGTONE_ASSET: &str = "BanterRingtone.ogg";

/// Wall-clock length of [`RINGTONE_ASSET`]. Used to time out unanswered calls.
pub fn ringtone_duration() -> Duration {
    match decode_ogg(asset_bytes(RINGTONE_ASSET)) {
        Ok(clip) if clip.rate > 0 && !clip.samples.is_empty() => {
            let secs = clip.samples.len() as f64 / clip.rate as f64;
            Duration::from_secs_f64(secs.max(0.1))
        }
        _ => Duration::from_secs(70),
    }
}

fn asset_bytes(name: &str) -> &'static [u8] {
    match name {
        "join_call.ogg" => include_bytes!("../assets/sfx/join_call.ogg"),
        "leave_call.ogg" => include_bytes!("../assets/sfx/leave_call.ogg"),
        "mute.ogg" => include_bytes!("../assets/sfx/mute.ogg"),
        "unmute.ogg" => include_bytes!("../assets/sfx/unmute.ogg"),
        "start_stream.ogg" => include_bytes!("../assets/sfx/start_stream.ogg"),
        "stop_stream.ogg" => include_bytes!("../assets/sfx/stop_stream.ogg"),
        "BanterRingtone.ogg" => include_bytes!("../assets/sfx/BanterRingtone.ogg"),
        _ => &[],
    }
}

#[derive(Clone)]
struct Clip {
    rate: u32,
    /// Mono PCM in -1.0..1.0.
    samples: Arc<[f32]>,
}

fn decode_ogg(bytes: &[u8]) -> Result<Clip, String> {
    if bytes.is_empty() {
        return Err("empty audio asset".into());
    }
    let mut reader = OggStreamReader::new(Cursor::new(bytes)).map_err(|err| err.to_string())?;
    let channels = reader.ident_hdr.audio_channels.max(1) as usize;
    let rate = reader.ident_hdr.audio_sample_rate;
    if rate == 0 {
        return Err("ogg sample rate is zero".into());
    }
    let mut interleaved = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|err| err.to_string())?
    {
        interleaved.extend(packet);
    }
    let frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in interleaved.chunks(channels) {
        let sum: f32 = frame.iter().map(|sample| *sample as f32 / 32768.0).sum();
        mono.push((sum / channels as f32).clamp(-1.0, 1.0));
    }
    Ok(Clip {
        rate,
        samples: Arc::from(mono.into_boxed_slice()),
    })
}

fn resample_mono(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 || from_rate == to_rate {
        return input.to_vec();
    }
    let out_len = (input.len() as u64) * (to_rate as u64) / (from_rate as u64);
    let mut out = Vec::with_capacity(out_len as usize);
    for index in 0..out_len {
        let source = (index as f64) * (from_rate as f64) / (to_rate as f64);
        let left = source.floor() as usize;
        let fraction = (source - left as f64) as f32;
        let a = input[left.min(input.len() - 1)];
        let b = input[(left + 1).min(input.len() - 1)];
        out.push(a + (b - a) * fraction);
    }
    out
}

enum Command {
    Play(SoundEvent),
    Ring(bool),
    Config { device: String, volume: f32 },
    Shutdown,
}

/// Background UI sound player. Failures are silent so missing speakers never block the UI.
pub struct Sfx {
    tx: Sender<Command>,
    _worker: Option<JoinHandle<()>>,
}

impl Sfx {
    pub fn open() -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("banter-sfx".into())
            .spawn(move || run_worker(rx))
            .ok();
        Self {
            tx,
            _worker: worker,
        }
    }

    pub fn play(&self, event: SoundEvent) {
        let _ = self.tx.send(Command::Play(event));
    }

    pub fn set_ringing(&self, ringing: bool) {
        let _ = self.tx.send(Command::Ring(ringing));
    }

    pub fn configure(&self, output_device: &str, output_volume: f32) {
        let _ = self.tx.send(Command::Config {
            device: output_device.to_string(),
            volume: output_volume.clamp(0.0, 2.0),
        });
    }
}

impl Drop for Sfx {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(handle) = self._worker.take() {
            let _ = handle.join();
        }
    }
}

struct Catalog {
    events: [Clip; 6],
    ringtone: Clip,
}

impl Catalog {
    fn load() -> Result<Self, String> {
        Ok(Self {
            events: [
                decode_ogg(asset_bytes(SoundEvent::Join.asset_file()))?,
                decode_ogg(asset_bytes(SoundEvent::Leave.asset_file()))?,
                decode_ogg(asset_bytes(SoundEvent::Mute.asset_file()))?,
                decode_ogg(asset_bytes(SoundEvent::Unmute.asset_file()))?,
                decode_ogg(asset_bytes(SoundEvent::StartStream.asset_file()))?,
                decode_ogg(asset_bytes(SoundEvent::StopStream.asset_file()))?,
            ],
            ringtone: decode_ogg(asset_bytes(RINGTONE_ASSET))?,
        })
    }

    fn clip(&self, event: SoundEvent) -> &Clip {
        &self.events[event as usize]
    }
}

struct Mixer {
    oneshots: VecDeque<f32>,
    ringtone: Option<Clip>,
    ringtone_pos: usize,
    ringing: bool,
    volume: f32,
}

impl Mixer {
    fn new() -> Self {
        Self {
            oneshots: VecDeque::new(),
            ringtone: None,
            ringtone_pos: 0,
            ringing: false,
            volume: 1.0,
        }
    }

    fn push_clip(&mut self, clip: &Clip, device_rate: u32) {
        let samples = resample_mono(&clip.samples, clip.rate, device_rate.max(1));
        // Cap queue so overlapping spam cannot grow without bound.
        let cap = device_rate.max(1) as usize * 4;
        while self.oneshots.len() > cap {
            self.oneshots.pop_front();
        }
        self.oneshots.extend(samples);
    }

    fn set_ringtone(&mut self, clip: &Clip, device_rate: u32) {
        let samples = resample_mono(&clip.samples, clip.rate, device_rate.max(1));
        self.ringtone = Some(Clip {
            rate: device_rate.max(1),
            samples: Arc::from(samples.into_boxed_slice()),
        });
        self.ringtone_pos = 0;
    }

    fn next_sample(&mut self) -> f32 {
        let mut sample = self.oneshots.pop_front().unwrap_or(0.0);
        if self.ringing {
            if let Some(ring) = &self.ringtone {
                if self.ringtone_pos < ring.samples.len() {
                    sample += ring.samples[self.ringtone_pos];
                    self.ringtone_pos += 1;
                } else {
                    // One play-through finished; do not loop.
                    self.ringing = false;
                }
            }
        }
        (sample * self.volume).clamp(-1.0, 1.0)
    }
}

fn pick_output(name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if name.is_empty() {
        return host
            .default_output_device()
            .ok_or_else(|| "no speakers are available".to_string());
    }
    host.output_devices()
        .map_err(|err| err.to_string())?
        .find(|device| device.name().ok().as_deref() == Some(name))
        .ok_or_else(|| format!("{name} is not available"))
}

fn run_worker(rx: mpsc::Receiver<Command>) {
    let Ok(catalog) = Catalog::load() else {
        // Keep draining so senders do not block if decode failed.
        while let Ok(cmd) = rx.recv() {
            if matches!(cmd, Command::Shutdown) {
                break;
            }
        }
        return;
    };

    let mixer = Arc::new(Mutex::new(Mixer::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let mut device_name = String::new();
    let mut stream: Option<cpal::Stream> = None;
    let mut device_rate = 48_000u32;

    let reopen = |device_name: &str,
                  mixer: &Arc<Mutex<Mixer>>,
                  stop: &Arc<AtomicBool>,
                  stream: &mut Option<cpal::Stream>,
                  device_rate: &mut u32,
                  catalog: &Catalog| {
        stop.store(true, Ordering::Relaxed);
        *stream = None;
        stop.store(false, Ordering::Relaxed);
        let Ok(device) = pick_output(device_name) else {
            return;
        };
        let Ok(config) = device.default_output_config() else {
            return;
        };
        let rate = config.sample_rate().0.max(1);
        *device_rate = rate;
        {
            let mut mix = mixer.lock().expect("sfx mixer");
            mix.set_ringtone(&catalog.ringtone, rate);
        }
        let channels = config.channels().max(1) as usize;
        let stream_config = config.config();
        let mix = Arc::clone(mixer);
        let halt = Arc::clone(stop);
        let built = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |output: &mut [f32], _| {
                    if halt.load(Ordering::Relaxed) {
                        output.fill(0.0);
                        return;
                    }
                    let mut mix = mix.lock().expect("sfx mixer");
                    for frame in output.chunks_mut(channels) {
                        let sample = mix.next_sample();
                        for slot in frame {
                            *slot = sample;
                        }
                    }
                },
                |_| {},
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &stream_config,
                move |output: &mut [i16], _| {
                    if halt.load(Ordering::Relaxed) {
                        for slot in output.iter_mut() {
                            *slot = 0;
                        }
                        return;
                    }
                    let mut mix = mix.lock().expect("sfx mixer");
                    for frame in output.chunks_mut(channels) {
                        let sample = mix.next_sample();
                        let value = (sample * i16::MAX as f32) as i16;
                        for slot in frame {
                            *slot = value;
                        }
                    }
                },
                |_| {},
                None,
            ),
            _ => return,
        };
        if let Ok(built) = built {
            if built.play().is_ok() {
                *stream = Some(built);
            }
        }
    };

    reopen(
        &device_name,
        &mixer,
        &stop,
        &mut stream,
        &mut device_rate,
        &catalog,
    );

    loop {
        let cmd = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(cmd) => cmd,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match cmd {
            Command::Shutdown => break,
            Command::Play(event) => {
                if stream.is_none() {
                    reopen(
                        &device_name,
                        &mixer,
                        &stop,
                        &mut stream,
                        &mut device_rate,
                        &catalog,
                    );
                }
                let clip = catalog.clip(event).clone();
                mixer
                    .lock()
                    .expect("sfx mixer")
                    .push_clip(&clip, device_rate);
            }
            Command::Ring(ringing) => {
                if stream.is_none() {
                    reopen(
                        &device_name,
                        &mixer,
                        &stop,
                        &mut stream,
                        &mut device_rate,
                        &catalog,
                    );
                }
                let mut mix = mixer.lock().expect("sfx mixer");
                if ringing {
                    // Start (or restart) a single play-through from the beginning.
                    if mix.ringtone.is_none() {
                        mix.set_ringtone(&catalog.ringtone, device_rate);
                    }
                    if !mix.ringing {
                        mix.ringtone_pos = 0;
                        mix.ringing = true;
                    }
                } else {
                    mix.ringing = false;
                    mix.ringtone_pos = 0;
                }
            }
            Command::Config { device, volume } => {
                mixer.lock().expect("sfx mixer").volume = volume;
                if device != device_name || stream.is_none() {
                    device_name = device;
                    reopen(
                        &device_name,
                        &mixer,
                        &stop,
                        &mut stream,
                        &mut device_rate,
                        &catalog,
                    );
                }
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    stream.take();
}

/// Decode every embedded clip. Used by tests to prove assets map and decode.
pub fn decode_all_assets() -> Result<Vec<(String, u32, usize)>, String> {
    let mut out = Vec::new();
    for event in SoundEvent::all() {
        let clip = decode_ogg(asset_bytes(event.asset_file()))?;
        out.push((
            event.asset_file().to_string(),
            clip.rate,
            clip.samples.len(),
        ));
    }
    let ring = decode_ogg(asset_bytes(RINGTONE_ASSET))?;
    out.push((RINGTONE_ASSET.to_string(), ring.rate, ring.samples.len()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_asset_names_match_shipped_files() {
        assert_eq!(SoundEvent::Join.asset_file(), "join_call.ogg");
        assert_eq!(SoundEvent::Leave.asset_file(), "leave_call.ogg");
        assert_eq!(SoundEvent::Mute.asset_file(), "mute.ogg");
        assert_eq!(SoundEvent::Unmute.asset_file(), "unmute.ogg");
        assert_eq!(SoundEvent::StartStream.asset_file(), "start_stream.ogg");
        assert_eq!(SoundEvent::StopStream.asset_file(), "stop_stream.ogg");
        assert_eq!(RINGTONE_ASSET, "BanterRingtone.ogg");
    }

    #[test]
    fn embedded_ogg_assets_decode() {
        let clips = decode_all_assets().expect("decode sfx assets");
        assert_eq!(clips.len(), 7);
        for (name, rate, frames) in clips {
            assert!(rate >= 8_000, "{name} rate");
            assert!(frames > 0, "{name} has samples");
        }
    }

    #[test]
    fn ringtone_duration_is_about_one_playthrough() {
        let secs = ringtone_duration().as_secs_f64();
        assert!(
            (60.0..90.0).contains(&secs),
            "expected ~70s BanterRingtone, got {secs}"
        );
    }

    #[test]
    fn ringtone_mixer_plays_once_then_stops() {
        let clip = decode_ogg(asset_bytes(RINGTONE_ASSET)).expect("ringtone");
        let short = Clip {
            rate: clip.rate,
            samples: Arc::from(clip.samples[..4800.min(clip.samples.len())].to_vec()),
        };
        let mut mix = Mixer::new();
        mix.ringtone = Some(short.clone());
        mix.ringing = true;
        mix.ringtone_pos = 0;
        for _ in 0..short.samples.len() {
            let _ = mix.next_sample();
            assert!(mix.ringing || mix.ringtone_pos >= short.samples.len());
        }
        let _ = mix.next_sample();
        assert!(!mix.ringing, "ringtone must not loop");
        assert_eq!(mix.ringtone_pos, short.samples.len());
    }
}
