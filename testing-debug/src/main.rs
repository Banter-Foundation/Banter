//! Linux-only terminal Banter streamer: login → voice channel → Go Live.
//!
//! Hard-wired for Adam's capture/encode debug loop: server id 1, channel id 2.
//! No GLFW / ImGui — uses the same Media + Realtime stack as the desktop client.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("banter-stream-debug is Linux-only");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    use std::thread;
    use std::time::{Duration, Instant};

    use banter::{
        clear_ice_config, default_stream_fps, default_stream_resolution, store_ice_config,
        stream_interval_ms, stream_max_edge, voice_rtc_mesh, voice_rtc_session_ready,
        websocket_url, CaptureKind, Client, EncodeSettings, EncoderPick, EngineUpdate, Media,
        Realtime, StreamPick,
    };
    use banter_server::{MediaPlace, ServerEvent, VoiceFlags, DEFAULT_ADDR};

    const SERVER_ID: i64 = 1;
    const CHANNEL_ID: i64 = 2;

    println!("banter-stream-debug (Linux, no UI)");
    println!("target: server {SERVER_ID}, voice channel {CHANNEL_ID}");

    let base = std::env::var("BANTER_BASE").unwrap_or_else(|_| format!("http://{DEFAULT_ADDR}"));
    println!("API base: {base} (override with BANTER_BASE)");

    let username = prompt("Banter username: ")?;
    let password = read_password()?;
    if username.trim().is_empty() || password.is_empty() {
        return Err("username and password are required".into());
    }

    let mut client = Client::new(&base);
    let session = client
        .login(username.trim(), &password)
        .map_err(|err| err.to_string())?;
    println!(
        "signed in as {} (user id {})",
        session.account.username, session.account.id
    );

    let health = client.health().map_err(|err| err.to_string())?;
    println!(
        "server {} protocol {}",
        health.version.as_deref().unwrap_or("?"),
        health.protocol.map(|p| p.to_string()).unwrap_or_else(|| "?".into())
    );

    let layout = client
        .layout(SERVER_ID)
        .map_err(|err| format!("server {SERVER_ID} layout: {err}"))?;
    let channel = layout
        .iter()
        .flat_map(|category| category.channels.iter())
        .find(|channel| channel.id == CHANNEL_ID)
        .ok_or_else(|| {
            format!("channel {CHANNEL_ID} not found on server {SERVER_ID} (check ids / membership)")
        })?;
    if !channel.kind.is_voice() {
        return Err(format!(
            "channel {CHANNEL_ID} is {:?}, need a voice channel",
            channel.kind
        ));
    }
    println!("found voice channel `{}` on server {SERVER_ID}", channel.name);

    match client.ice_config() {
        Ok(cfg) => store_ice_config(cfg),
        Err(err) => {
            eprintln!("warning: ICE config unavailable ({err}); using local defaults");
            clear_ice_config();
        }
    }

    let token = client
        .token()
        .ok_or_else(|| "login did not return a token".to_string())?
        .to_string();
    let realtime = Realtime::connect(websocket_url(&base, &token));
    let media = Media::start(realtime.signal_sender());

    // Wait briefly for the websocket handshake.
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < ready_deadline && !realtime.is_connected() {
        if let Some(err) = realtime.take_error() {
            return Err(err);
        }
        thread::sleep(Duration::from_millis(40));
    }
    if !realtime.is_connected() {
        return Err("websocket did not connect".into());
    }
    println!("realtime connected");

    let mut members = client
        .join_voice(CHANNEL_ID)
        .map_err(|err| err.to_string())?;
    println!(
        "joined voice channel {CHANNEL_ID} ({} member(s))",
        members.len()
    );

    let pick = choose_stream_pick()?;
    match pick {
        StreamPick::Portal => println!("capture: Wayland portal (compositor picker)"),
        StreamPick::Source(CaptureKind::Screen, id) => println!("capture: monitor id {id}"),
        StreamPick::Source(CaptureKind::Window, id) => println!("capture: window id {id}"),
    }

    let resolution = default_stream_resolution();
    let fps = default_stream_fps();
    // Full-layer target bitrate. Default 2500 kbps; set BANTER_BITRATE_KBPS=10000 for 10 Mbps.
    let bitrate_kbps = std::env::var("BANTER_BITRATE_KBPS")
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .filter(|kbps| *kbps > 0)
        .unwrap_or(2_500);
    let full_bps = bitrate_kbps.saturating_mul(1_000);
    println!("encode bitrate: {bitrate_kbps} kbps ({full_bps} bps)");
    let encode = EncodeSettings {
        full_bps,
        pick: EncoderPick::Auto,
        keyframe_ms: 1_000,
        fps: 30,
        cbr: true,
        pref_label: "auto",
        rate_label: "cbr",
    };
    let engine = EngineUpdate {
        input_device: String::new(),
        output_device: String::new(),
        input_volume: 1.0,
        output_volume: 1.0,
        sensitivity: banter::OPEN_MIC_RMS,
        noise_suppression: true,
        echo_cancellation: true,
        auto_gain: false,
        show_cursor: true,
        show_preview: false,
        sources: Vec::new(),
    };

    let me = session.account.id;
    let mut sent_flags = VoiceFlags {
        muted: true,
        ..VoiceFlags::default()
    };
    let flags = VoiceFlags {
        muted: false,
        deafened: false,
        push_to_talk: false,
        transmitting: true,
        camera: false,
        screen: true,
    };

    println!("Go Live started — Ctrl+C to stop");
    println!("status lines every 5s (errors / capture failure)");

    let mut last_status = Instant::now();
    loop {
        if let Some(err) = realtime.take_error() {
            return Err(format!("realtime dropped: {err}"));
        }
        for event in realtime.take_events() {
            media.ingest(event.clone());
            if let ServerEvent::Voice {
                channel_id,
                members: roster,
            } = &event
            {
                if *channel_id == CHANNEL_ID {
                    members = roster.clone();
                }
            }
        }

        let in_place = true;
        let place = Some(MediaPlace::VoiceChannel(CHANNEL_ID));
        let offerer = webrtc_offerer_guild_sfu(me, &members);
        let mesh = voice_rtc_mesh(false, false);
        let rtc_ready = voice_rtc_session_ready(in_place, false);

        media.set_stream_choice(
            Some(pick),
            stream_max_edge(resolution),
            stream_interval_ms(fps),
        );
        media.apply_encode(encode);
        media.note_publishing(flags.camera, flags.screen);
        media.apply_engine(&engine);
        media.set_mic_test(false);
        media.set_state(flags, in_place);
        media.set_session(place.filter(|_| rtc_ready), me, offerer, mesh);

        if flags != sent_flags {
            match client.set_flags(flags) {
                Ok(()) => sent_flags = flags,
                Err(err) => eprintln!("voice flags: {err}"),
            }
        }

        if media.take_capture_failure() {
            eprintln!("capture failed (portal dismissed or source gone) — retrying pick");
            // Keep trying the same pick; portal will re-prompt if needed.
        }

        let errors = media.errors();
        if last_status.elapsed() >= Duration::from_secs(5) {
            last_status = Instant::now();
            if errors.is_empty() {
                println!(
                    "streaming… members={} screen={} offerer={}",
                    members.len(),
                    flags.screen,
                    offerer
                );
            } else {
                for err in &errors {
                    eprintln!("media: {err}");
                }
            }
        }

        // Drain local preview frames so the convert queue does not stick.
        let _ = media.take_video();
        thread::sleep(Duration::from_millis(16));
    }
}

#[cfg(target_os = "linux")]
fn prompt(label: &str) -> Result<String, String> {
    use std::io::{self, Write};
    print!("{label}");
    io::stdout()
        .flush()
        .map_err(|err| format!("stdout: {err}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| format!("stdin: {err}"))?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(target_os = "linux")]
fn read_password() -> Result<String, String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Banter password: ")
            .map_err(|err| format!("couldn't read password: {err}"))
    } else {
        // Piped / non-TTY (CI, scripts): read a plain line.
        eprint!("Banter password: ");
        prompt("")
    }
}

/// Guild SFU: every client offers (matches desktop when P2P guild video is off).
#[cfg(target_os = "linux")]
fn webrtc_offerer_guild_sfu(_me: i64, _members: &[banter_server::VoiceMember]) -> bool {
    true
}

#[cfg(target_os = "linux")]
fn choose_stream_pick() -> Result<banter::StreamPick, String> {
    use banter::{native_screen_picker, CaptureKind, StreamPick};

    if native_screen_picker() {
        return Ok(StreamPick::Portal);
    }
    let monitors = xcap::Monitor::all().map_err(|err| err.to_string())?;
    let primary = monitors.iter().find(|m| m.is_primary().unwrap_or(false));
    let monitor = primary.or_else(|| monitors.first()).ok_or_else(|| {
        "no monitors found for X11/xcap capture (try Wayland portal or set DISPLAY)".to_string()
    })?;
    let id = monitor
        .id()
        .map_err(|err| format!("monitor id: {err}"))?;
    Ok(StreamPick::Source(CaptureKind::Screen, id))
}
