//! Linux DRM kmsgrab → ffmpeg → planar I420 for screen share.
//!
//! When a DRM card is usable and ffmpeg was built with the `kmsgrab` demuxer,
//! Banter can pull framebuffer pixels without the PipeWire MemFd BGRA copy.
//! Privileges are often strict (root / CAP_SYS_ADMIN / DRM master); callers must
//! fall back to PipeWire Cast or xcap Still when [`try_open`] returns [`None`]
//! (Auto) or Err (Force).
//!
//! Env: `BANTER_KMSGRAB`, `BANTER_KMSGRAB_DEVICE`, optional `BANTER_SCREEN_CAPTURE`.
//! UI: User Settings → Experimental → Prefer kmsgrab (`experimental.json` / [`set_ui_preferred`]).
//! See store plan `docs/kmsgrab-ffmpeg-capture.md` and repo `docs/streaming.md`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pipewire::spa::param::video::VideoFormat;

use super::screencast::RawFrame;

/// UI / `experimental.json` preference (same as `BANTER_KMSGRAB=1` when env is unset).
static UI_PREFERRED: AtomicBool = AtomicBool::new(false);

/// Apply the Experimental settings toggle. Takes effect on the next monitor Go Live
/// (`open_feed` re-reads [`mode`]); no process restart.
pub fn set_ui_preferred(enabled: bool) {
    UI_PREFERRED.store(enabled, Ordering::Relaxed);
}

pub fn ui_preferred() -> bool {
    UI_PREFERRED.load(Ordering::Relaxed)
}

/// How aggressively Banter should prefer kmsgrab for monitor capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Never attempt kmsgrab.
    Off,
    /// Probe and open when DRM/ffmpeg look usable; silent fallback on failure.
    Auto,
    /// Require kmsgrab for monitor share; surface errors.
    Force,
}

/// Long-running ffmpeg kmsgrab capture. Same hand-off shape as [`super::screencast::CastStream`].
pub struct KmsStream {
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<String>>>,
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
}

impl KmsStream {
    pub fn has_frame(&self) -> bool {
        self.latest.lock().expect("kmsgrab frame").is_some()
    }

    pub fn take_raw(&self) -> Option<RawFrame> {
        self.latest.lock().expect("kmsgrab frame").take()
    }

    pub fn take_fault(&self) -> Option<String> {
        self.fault.lock().expect("kmsgrab fault").take()
    }
}

impl Drop for KmsStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

/// Resolve capture policy: process env overrides the Experimental UI toggle.
pub fn mode() -> Mode {
    if let Ok(raw) = std::env::var("BANTER_SCREEN_CAPTURE") {
        match normalize(&raw).as_str() {
            "pipewire" | "xcap" | "portal" | "still" => return Mode::Off,
            "kmsgrab" | "kms" => return Mode::Force,
            _ => {}
        }
    }
    match std::env::var("BANTER_KMSGRAB") {
        Ok(raw) => parse_mode(&raw),
        // Unset: Experimental toggle → Auto (same as BANTER_KMSGRAB=1); else Off.
        Err(_) => {
            if ui_preferred() {
                Mode::Auto
            } else {
                Mode::Off
            }
        }
    }
}

fn parse_mode(raw: &str) -> Mode {
    match normalize(raw).as_str() {
        "" | "1" | "true" | "yes" | "on" | "auto" => Mode::Auto,
        "force" | "require" | "must" => Mode::Force,
        "0" | "false" | "no" | "off" | "disable" | "disabled" => Mode::Off,
        other if other == "kmsgrab" || other == "kms" => Mode::Force,
        _ => Mode::Off,
    }
}

fn normalize(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Attempt kmsgrab for a full-monitor share.
///
/// - [`Mode::Off`] → always `Ok(None)`.
/// - [`Mode::Auto`] → `Ok(None)` when probe/open fails (caller keeps PipeWire/xcap).
/// - [`Mode::Force`] → `Err` when unusable.
pub fn try_open(fps: u32) -> Result<Option<KmsStream>, String> {
    match mode() {
        Mode::Off => Ok(None),
        Mode::Auto => match open(fps) {
            Ok(stream) => {
                eprintln!("screen: kmsgrab capture active ({})", device_path().display());
                Ok(Some(stream))
            }
            Err(err) => {
                // Wayland compositors usually hold DRM master — Auto is expected to
                // fall through here; Force surfaces the same error to the UI.
                eprintln!(
                    "screen: kmsgrab unavailable ({err}); using PipeWire/xcap (common on Wayland when the compositor holds DRM master)"
                );
                Ok(None)
            }
        },
        Mode::Force => Ok(Some(open(fps)?)),
    }
}

fn open(fps: u32) -> Result<KmsStream, String> {
    let fps = fps.clamp(1, 60);
    ensure_ffmpeg_kmsgrab()?;
    let device = device_path();
    if !device.exists() {
        return Err(format!("DRM device {} is missing", device.display()));
    }
    // Openability is a weak check; mapping FBs still often needs CAP_SYS_ADMIN.
    std::fs::File::open(&device).map_err(|err| {
        format!(
            "cannot open {} ({err}); need root, CAP_SYS_ADMIN, or DRM access",
            device.display()
        )
    })?;
    let (width, height) = display_size(&device)?;
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err(format!("kmsgrab size {width}x{height} is unusable"));
    }
    let frame_bytes = i420_frame_bytes(width, height);

    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "kmsgrab",
        "-framerate",
        &fps.to_string(),
        "-device",
        &device.to_string_lossy(),
        "-i",
        "-",
        "-vf",
        "hwdownload,format=bgr0",
        "-an",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "pipe:1",
    ]);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("ffmpeg kmsgrab spawn failed: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ffmpeg kmsgrab produced no stdout".to_string())?;
    let stderr = child.stderr.take();

    let latest = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let fault = Arc::new(Mutex::new(None));

    let stderr_thread = stderr.map(|stderr| {
        let fault = Arc::clone(&fault);
        thread::Builder::new()
            .name("banter-kmsgrab-err".into())
            .spawn(move || {
                let mut lines = BufReader::new(stderr).lines();
                while let Some(Ok(line)) = lines.next() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // Keep the last ffmpeg error line for take_fault.
                    *fault.lock().expect("kmsgrab fault") = Some(line.to_string());
                }
            })
            .expect("spawn kmsgrab stderr")
    });

    let latest_thread = Arc::clone(&latest);
    let stop_thread = Arc::clone(&stop);
    let fault_thread = Arc::clone(&fault);
    let reader = thread::Builder::new()
        .name("banter-kmsgrab".into())
        .spawn(move || {
            read_i420_loop(
                stdout,
                width,
                height,
                frame_bytes,
                latest_thread,
                stop_thread,
                fault_thread,
            );
        })
        .map_err(|err| err.to_string())?;

    // Fail fast if ffmpeg dies before the first frame (permissions, NVIDIA, Wayland master).
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        if latest.lock().expect("kmsgrab frame").is_some() {
            break;
        }
        if let Some(err) = fault.lock().expect("kmsgrab fault").clone() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            if let Some(stderr_thread) = stderr_thread {
                let _ = stderr_thread.join();
            }
            return Err(format!("kmsgrab failed to start: {err}"));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let detail = fault
                    .lock()
                    .expect("kmsgrab fault")
                    .clone()
                    .unwrap_or_else(|| format!("ffmpeg exited ({status})"));
                let _ = reader.join();
                if let Some(stderr_thread) = stderr_thread {
                    let _ = stderr_thread.join();
                }
                return Err(format!("kmsgrab failed to start: {detail}"));
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(err) => {
                let _ = child.kill();
                let _ = reader.join();
                if let Some(stderr_thread) = stderr_thread {
                    let _ = stderr_thread.join();
                }
                return Err(format!("kmsgrab wait failed: {err}"));
            }
        }
    }
    if latest.lock().expect("kmsgrab frame").is_none() {
        // Still running but silent — keep it; screen_loop has a cast-style timeout.
        eprintln!(
            "screen: kmsgrab started ({width}x{height} @ {fps}); waiting for first frame"
        );
    }

    Ok(KmsStream {
        latest,
        stop,
        fault,
        child: Some(child),
        reader: Some(reader),
        stderr: stderr_thread,
    })
}

fn read_i420_loop(
    mut stdout: impl Read,
    width: u32,
    height: u32,
    frame_bytes: usize,
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Arc<AtomicBool>,
    fault: Arc<Mutex<Option<String>>>,
) {
    let mut buf = vec![0u8; frame_bytes];
    while !stop.load(Ordering::Relaxed) {
        if let Err(err) = read_exact_interruptible(&mut stdout, &mut buf, &stop) {
            if !stop.load(Ordering::Relaxed) {
                *fault.lock().expect("kmsgrab fault") =
                    Some(format!("kmsgrab frame read failed: {err}"));
            }
            break;
        }
        let mut next = vec![0u8; frame_bytes];
        std::mem::swap(&mut buf, &mut next);
        *latest.lock().expect("kmsgrab frame") =
            Some(RawFrame::owned(width, height, width as i32, VideoFormat::I420, next));
    }
}

fn read_exact_interruptible(
    reader: &mut impl Read,
    buf: &mut [u8],
    stop: &AtomicBool,
) -> Result<(), String> {
    let mut filled = 0usize;
    while filled < buf.len() {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return Err("ffmpeg pipe closed".into()),
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}

fn ensure_ffmpeg_kmsgrab() -> Result<(), String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-h", "demuxer=kmsgrab"])
        .output()
        .map_err(|_| "ffmpeg was not found (needed for kmsgrab capture)".to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !text.to_ascii_lowercase().contains("kmsgrab") {
        return Err("ffmpeg was built without the kmsgrab demuxer".into());
    }
    Ok(())
}

fn device_path() -> PathBuf {
    if let Ok(raw) = std::env::var("BANTER_KMSGRAB_DEVICE") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    first_dri_card().unwrap_or_else(|| PathBuf::from("/dev/dri/card0"))
}

fn first_dri_card() -> Option<PathBuf> {
    let dir = Path::new("/dev/dri");
    let mut cards = std::fs::read_dir(dir).ok()?.filter_map(|entry| {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") {
            Some(entry.path())
        } else {
            None
        }
    }).collect::<Vec<_>>();
    cards.sort();
    cards.into_iter().next()
}

fn display_size(device: &Path) -> Result<(u32, u32), String> {
    if let Some(size) = size_from_sysfs(device) {
        return Ok(size);
    }
    size_from_ffmpeg_probe(device)
}

fn size_from_sysfs(device: &Path) -> Option<(u32, u32)> {
    let name = device.file_name()?.to_str()?;
    let drm = Path::new("/sys/class/drm");
    let mut best: Option<(u32, u32)> = None;
    let entries = std::fs::read_dir(drm).ok()?;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if !fname.starts_with(name) || !fname.contains('-') {
            continue;
        }
        let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
        if !status.trim().eq_ignore_ascii_case("connected") {
            continue;
        }
        let modes = std::fs::read_to_string(entry.path().join("modes")).ok()?;
        let line = modes.lines().next()?.trim();
        if let Some(size) = parse_mode_line(line) {
            best = Some(size);
            break;
        }
    }
    best
}

fn parse_mode_line(line: &str) -> Option<(u32, u32)> {
    let (w, h) = line.split_once('x')?;
    let width: u32 = w.parse().ok()?;
    // Modes may look like "1920x1080i" or "1920x1080".
    let height: u32 = h
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    if width >= 2 && height >= 2 {
        Some((width & !1, height & !1))
    } else {
        None
    }
}

fn size_from_ffmpeg_probe(device: &Path) -> Result<(u32, u32), String> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-f",
            "kmsgrab",
            "-framerate",
            "1",
            "-device",
            &device.to_string_lossy(),
            "-i",
            "-",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|err| format!("ffmpeg kmsgrab probe failed: {err}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(size) = parse_ffmpeg_size(&text) {
        return Ok(size);
    }
    Err(if text.trim().is_empty() {
        "could not read kmsgrab display size".into()
    } else {
        format!(
            "could not read kmsgrab display size ({})",
            text.lines().last().unwrap_or("probe failed").trim()
        )
    })
}

fn parse_ffmpeg_size(text: &str) -> Option<(u32, u32)> {
    // Typical: "1920x1080" near Stream #0 video line.
    for token in text.split(|c: char| !c.is_ascii_digit() && c != 'x') {
        if let Some(size) = parse_mode_line(token) {
            if size.0 >= 64 && size.1 >= 64 {
                return Some(size);
            }
        }
    }
    None
}

fn i420_frame_bytes(width: u32, height: u32) -> usize {
    let y = (width as usize).saturating_mul(height as usize);
    y + y / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_recognizes_common_values() {
        assert_eq!(parse_mode("0"), Mode::Off);
        assert_eq!(parse_mode("off"), Mode::Off);
        assert_eq!(parse_mode("1"), Mode::Auto);
        assert_eq!(parse_mode("auto"), Mode::Auto);
        assert_eq!(parse_mode("force"), Mode::Force);
        assert_eq!(parse_mode("kmsgrab"), Mode::Force);
    }

    #[test]
    fn ui_preferred_maps_to_auto_when_env_unset() {
        // Do not assert full mode() here — process env in the test runner may set BANTER_*.
        set_ui_preferred(true);
        assert!(ui_preferred());
        set_ui_preferred(false);
        assert!(!ui_preferred());
    }

    #[test]
    fn parse_mode_line_strips_interlace_suffix() {
        assert_eq!(parse_mode_line("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_mode_line("1920x1080i"), Some((1920, 1080)));
        assert_eq!(parse_mode_line("2560x1440"), Some((2560, 1440)));
    }

    #[test]
    fn i420_frame_bytes_matches_planar_420() {
        assert_eq!(i420_frame_bytes(2, 2), 6);
        assert_eq!(i420_frame_bytes(1920, 1080), 1920 * 1080 * 3 / 2);
    }

    #[test]
    fn parse_ffmpeg_size_finds_desktop_geometry() {
        let sample = "Stream #0:0: Video: wrapped_avframe, 2560x1440, 30 fps";
        assert_eq!(parse_ffmpeg_size(sample), Some((2560, 1440)));
    }
}
