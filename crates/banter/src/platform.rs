//! GLFW window, OpenGL 3.3, and the Dear ImGui docking branch.
//!
//! User Settings opens in a second shared-context GLFW window so the main Banter
//! shell stays usable. Closing that window clears settings state on the main app.

use std::rc::Rc;
use std::time::Duration;

use glfw::Context as _;
use dear_imgui_glow::GlowRenderer;
use dear_imgui_rs::{ClipboardBackend, ConfigFlags, Context, Key, MouseButton};
use glow::HasContext;

use crate::logic::{
    DisplayRefresh, VideoPrefs, clamp_window_size, either_key_held, load_json, pick_display_refresh_hz,
    ui_swap_needs_update,
};
use crate::settings::{settings_window_default_size, SETTINGS_WINDOW_TITLE};
use crate::ui::App;

pub(crate) fn run() -> Result<(), String> {
    let data_dir = banter_server::default_data_dir();
    let prefs = crate::load_json::<crate::DevServerPrefs>(&data_dir.join("dev-server.json")).unwrap_or_default();
    let has_saved_session = crate::load_token(&data_dir).is_some();
    let base = crate::startup_api_base(&prefs);
    let local_available = match crate::startup_local_policy(&prefs, has_saved_session) {
        crate::StartupLocalPolicy::SkipLocalUseRemote => false,
        crate::StartupLocalPolicy::StartLocal => ensure_local_server().is_ok(),
        crate::StartupLocalPolicy::PreferRunningLocalElseAskRemote => local_server_listening(),
    };
    let ask_remote = crate::startup_should_prompt_remote(&prefs, local_available, has_saved_session);
    let startup_video = load_video_prefs(&data_dir);
    let (start_w, start_h) = clamp_window_size(startup_video.width, startup_video.height);

    let mut glfw = glfw::init(glfw_error).map_err(|err| format!("GLFW init failed: {err}"))?;
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    let (mut window, events) = glfw
        .create_window(start_w as u32, start_h as u32, "Banter", glfw::WindowMode::Windowed)
        .ok_or_else(|| "GLFW could not open a window. Banter needs a display.".to_string())?;
    window.make_current();
    window.set_all_polling(true);

    let mut windowed = WindowedPlace {
        x: 100,
        y: 100,
        width: start_w,
        height: start_h,
    };
    if let Some((pos_x, pos_y)) = window_origin(&glfw, &window) {
        windowed.x = pos_x;
        windowed.y = pos_y;
    }
    apply_video(&mut glfw, &mut window, &startup_video, &mut windowed);

    let gl = Rc::new(unsafe {
        glow::Context::from_loader_function(|name| {
            window
                .get_proc_address(name)
                .map(|address| address as *const _)
                .unwrap_or(std::ptr::null())
        })
    });
    let mut imgui = Context::create();
    let flags = imgui.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
    imgui.io_mut().set_config_flags(flags);
    imgui.set_clipboard_backend(GlfwClipboard);
    let ini = data_dir.join("banter-dock.ini");
    imgui.set_ini_filename(Some(ini)).map_err(|err| err.to_string())?;
    let bold_font = install_font(&mut imgui);
    let semibold_font = load_first_face(imgui.font_atlas(), &semibold_font_paths(), 16.0).or(bold_font);
    let mut renderer = GlowRenderer::with_shared_context(Rc::clone(&gl), &mut imgui).map_err(|err| err.to_string())?;

    let mut app = App::open(base, data_dir);
    if ask_remote {
        app.prompt_remote_server(
            "Local server is not available. Enter a remote host, or start the local server and choose Local.",
        );
    }
    app.set_bold_font(bold_font);
    app.set_semibold_font(semibold_font);
    let mut last_time = glfw.get_time();
    // (vsync, refresh_hz) last pushed to GLFW. Reapplied when the window moves
    // to another monitor or the active mode's Hz changes.
    let mut applied_swap: Option<(bool, u32)> = None;
    let mut vsync_on = startup_video.vsync;
    let mut settings_host: Option<SettingsHost> = None;

    while !window.should_close() {
        glfw.poll_events();
        if let Some(video) = app.take_video_apply() {
            vsync_on = video.vsync;
            apply_video(&mut glfw, &mut window, &video, &mut windowed);
            // Force a refresh-aware swap pass after Display settings change.
            applied_swap = None;
        }
        sync_ui_swap_interval(&mut glfw, &mut window, vsync_on, &mut applied_swap);
        sync_settings_window(&mut glfw, &mut window, &mut settings_host, &mut app)?;

        let main_events: Vec<_> = glfw::flush_messages(&events).map(|(_, event)| event).collect();
        let settings_events: Vec<_> = settings_host
            .as_ref()
            .map(|host| glfw::flush_messages(&host.events).map(|(_, event)| event).collect())
            .unwrap_or_default();

        let (width, height) = window.get_size();
        let (framebuffer_width, framebuffer_height) = window.get_framebuffer_size();
        if width <= 0 || height <= 0 {
            window.swap_buffers();
            continue;
        }
        let now = glfw.get_time();
        let delta = (now - last_time).max(1.0 / 1000.0);
        last_time = now;
        crate::profiler::frame(delta);

        let settings_focused = settings_host
            .as_ref()
            .map(|host| host.window.is_focused())
            .unwrap_or(false);
        let main_focused = window.is_focused() && !settings_focused;

        {
            let _prepare = crate::profiler::scope_section(crate::profiler::Section::Prepare);
            app.prepare(&mut imgui);
        }

        // --- Main Banter window ---
        feed_frame_io(
            &mut imgui,
            &window,
            &main_events,
            &mut app,
            width,
            height,
            framebuffer_width,
            framebuffer_height,
            delta,
            main_focused,
        );
        app.set_v_held(window.get_key(glfw::Key::V) == glfw::Action::Press);
        unsafe {
            gl.clear_color(0.08, 0.08, 0.10, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        {
            let _draw = crate::profiler::scope_section(crate::profiler::Section::Draw);
            let ui = imgui.frame();
            app.draw_main(&ui);
        }
        {
            let _gl = crate::profiler::scope_section(crate::profiler::Section::Gl);
            let pending = imgui.render(renderer.renderer_consumer().map_err(|err| err.to_string())?);
            renderer.render(pending).map_err(|err| err.to_string())?;
        }
        window.swap_buffers();

        // --- Dedicated Banter Settings window (shared GL context) ---
        if let Some(host) = settings_host.as_mut() {
            let (sw, sh) = host.window.get_size();
            let (fbw, fbh) = host.window.get_framebuffer_size();
            if sw > 0 && sh > 0 {
                host.window.make_current();
                feed_frame_io(
                    &mut imgui,
                    &host.window,
                    &settings_events,
                    &mut app,
                    sw,
                    sh,
                    fbw,
                    fbh,
                    delta,
                    settings_focused || !main_focused,
                );
                unsafe {
                    gl.clear_color(0.08, 0.08, 0.10, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
                {
                    let ui = imgui.frame();
                    app.draw_settings_window(&ui);
                }
                {
                    let pending = imgui.render(renderer.renderer_consumer().map_err(|err| err.to_string())?);
                    renderer.render(pending).map_err(|err| err.to_string())?;
                }
                host.window.swap_buffers();
                window.make_current();
            }
        }
    }
    Ok(())
}

struct SettingsHost {
    window: glfw::PWindow,
    events: glfw::GlfwReceiver<(f64, glfw::WindowEvent)>,
}

fn sync_settings_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
    host: &mut Option<SettingsHost>,
    app: &mut App,
) -> Result<(), String> {
    let want = app.settings_os_window_desired();
    if want {
        if host.is_none() {
            *host = Some(create_settings_window(glfw, main)?);
        }
        if let Some(settings) = host.as_mut() {
            if app.take_settings_window_focus() {
                settings.window.focus();
            }
            if settings.window.should_close() {
                app.notify_settings_window_closed();
                *host = None;
                main.focus();
            }
        }
    } else if host.is_some() {
        *host = None;
        main.focus();
    }
    Ok(())
}

fn create_settings_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
) -> Result<SettingsHost, String> {
    let (width, height) = settings_window_default_size();
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Visible(true));
    glfw.window_hint(glfw::WindowHint::Focused(true));
    let (mut window, events) = main
        .create_shared(width as u32, height as u32, SETTINGS_WINDOW_TITLE, glfw::WindowMode::Windowed)
        .ok_or_else(|| "GLFW could not open the Banter Settings window.".to_string())?;
    window.set_all_polling(true);
    if let Some((x, y)) = window_origin(glfw, main) {
        window.set_pos(x + 48, y + 48);
    }
    window.focus();
    // Main window stays current for prepare/texture work until the settings pass.
    main.make_current();
    Ok(SettingsHost { window, events })
}

fn feed_frame_io(
    imgui: &mut Context,
    window: &glfw::Window,
    events: &[glfw::WindowEvent],
    app: &mut App,
    width: i32,
    height: i32,
    framebuffer_width: i32,
    framebuffer_height: i32,
    delta: f64,
    receive_pointer: bool,
) {
    let io = imgui.io_mut();
    io.set_display_size([width as f32, height as f32]);
    io.set_delta_time(delta as f32);
    let scale_x = framebuffer_width as f32 / width as f32;
    let scale_y = framebuffer_height as f32 / height as f32;
    io.set_display_framebuffer_scale([scale_x, scale_y]);
    if receive_pointer {
        let (cursor_x, cursor_y) = window.get_cursor_pos();
        io.add_mouse_pos_event([cursor_x as f32, cursor_y as f32]);
    } else {
        // Dear ImGui treats this as "no mouse" so the unfocused window does not steal clicks.
        io.add_mouse_pos_event([f32::MIN, f32::MIN]);
    }
    for event in events {
        match event {
            glfw::WindowEvent::Focus(focused) => io.add_focus_event(*focused),
            glfw::WindowEvent::CursorPos(_, _) if receive_pointer => app.note_input(),
            glfw::WindowEvent::MouseButton(_, _, _)
            | glfw::WindowEvent::Scroll(_, _)
            | glfw::WindowEvent::Key(_, _, _, _)
            | glfw::WindowEvent::Char(_)
                if receive_pointer =>
            {
                feed_event(io, app, event.clone());
            }
            _ => {}
        }
    }
    let held = |key| window.get_key(key) == glfw::Action::Press;
    if receive_pointer {
        io.add_key_event(
            Key::ModCtrl,
            either_key_held(held(glfw::Key::LeftControl), held(glfw::Key::RightControl)),
        );
        io.add_key_event(
            Key::ModShift,
            either_key_held(held(glfw::Key::LeftShift), held(glfw::Key::RightShift)),
        );
        io.add_key_event(Key::ModAlt, either_key_held(held(glfw::Key::LeftAlt), held(glfw::Key::RightAlt)));
        io.add_key_event(
            Key::ModSuper,
            either_key_held(held(glfw::Key::LeftSuper), held(glfw::Key::RightSuper)),
        );
    } else {
        io.add_key_event(Key::ModCtrl, false);
        io.add_key_event(Key::ModShift, false);
        io.add_key_event(Key::ModAlt, false);
        io.add_key_event(Key::ModSuper, false);
    }
}

struct WindowedPlace {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn load_video_prefs(data_dir: &std::path::Path) -> VideoPrefs {
    let mut video = load_json::<VideoPrefs>(&data_dir.join("video.json")).unwrap_or_default();
    let (width, height) = clamp_window_size(video.width, video.height);
    video.width = width;
    video.height = height;
    video
}

fn apply_video(
    glfw: &mut glfw::Glfw,
    window: &mut glfw::Window,
    video: &VideoPrefs,
    windowed: &mut WindowedPlace,
) {
    let (width, height) = clamp_window_size(video.width, video.height);
    let currently_fullscreen = window.with_window_mode(|mode| matches!(mode, glfw::WindowMode::FullScreen(_)));
    if video.fullscreen {
        if !currently_fullscreen {
            if let Some((x, y)) = window_origin(glfw, window) {
                windowed.x = x;
                windowed.y = y;
            }
            let (w, h) = window.get_size();
            windowed.width = w;
            windowed.height = h;
        }
        glfw.with_primary_monitor(|_, monitor| {
            if let Some(monitor) = monitor {
                if let Some(mode) = monitor.get_video_mode() {
                    window.set_monitor(
                        glfw::WindowMode::FullScreen(monitor),
                        0,
                        0,
                        mode.width,
                        mode.height,
                        Some(mode.refresh_rate),
                    );
                }
            }
        });
    } else {
        if currently_fullscreen {
            window.set_monitor(
                glfw::WindowMode::Windowed,
                windowed.x,
                windowed.y,
                width as u32,
                height as u32,
                None,
            );
        } else {
            let (current_w, current_h) = window.get_size();
            if current_w != width || current_h != height {
                window.set_size(width, height);
            }
        }
        windowed.width = width;
        windowed.height = height;
    }
    // Swap interval is owned by `sync_ui_swap_interval`, which follows the
    // current display's refresh rate after this apply (and when the window moves).
}

/// Match the UI present rate to the monitor that owns the window.
///
/// With VSync on, GLFW `Sync(1)` waits for that monitor's vertical blank. The interval is
/// reapplied whenever the detected refresh rate changes (moving between monitors, or the
/// OS changing the active mode). With VSync off, presents stay uncapped.
fn sync_ui_swap_interval(
    glfw: &mut glfw::Glfw,
    window: &mut glfw::Window,
    vsync: bool,
    applied: &mut Option<(bool, u32)>,
) {
    let refresh_hz = query_ui_refresh_hz(glfw, window);
    if !ui_swap_needs_update(*applied, vsync, refresh_hz) {
        return;
    }
    glfw.set_swap_interval(if vsync {
        glfw::SwapInterval::Sync(1)
    } else {
        glfw::SwapInterval::None
    });
    *applied = Some((vsync, refresh_hz));
}

fn query_ui_refresh_hz(glfw: &mut glfw::Glfw, window: &mut glfw::Window) -> u32 {
    let fullscreen_hz = glfw.with_window_monitor(window, |_, monitor| {
        monitor
            .and_then(|monitor| monitor.get_video_mode())
            .map(|mode| mode.refresh_rate)
    });
    let displays = connected_display_refreshes(glfw);
    let (center_x, center_y) = window_center_for_refresh(glfw, window);
    pick_display_refresh_hz(center_x, center_y, fullscreen_hz, &displays)
}

fn connected_display_refreshes(glfw: &mut glfw::Glfw) -> Vec<DisplayRefresh> {
    glfw.with_connected_monitors(|_, monitors| {
        monitors
            .iter()
            .filter_map(|monitor| {
                let mode = monitor.get_video_mode()?;
                let (x, y) = monitor.get_pos();
                Some(DisplayRefresh {
                    x,
                    y,
                    width: mode.width as i32,
                    height: mode.height as i32,
                    refresh_hz: mode.refresh_rate,
                })
            })
            .collect()
    })
}

/// Window center in screen coordinates. Wayland does not expose position, so use the
/// primary monitor center and let [`pick_display_refresh_hz`] fall through to that display.
fn window_center_for_refresh(glfw: &mut glfw::Glfw, window: &glfw::Window) -> (i32, i32) {
    if glfw.get_platform() == glfw::Platform::Wayland {
        return glfw.with_primary_monitor(|_, monitor| {
            let Some(monitor) = monitor else {
                return (0, 0);
            };
            let Some(mode) = monitor.get_video_mode() else {
                return (0, 0);
            };
            let (x, y) = monitor.get_pos();
            (x + mode.width as i32 / 2, y + mode.height as i32 / 2)
        });
    }
    let (x, y) = window.get_pos();
    let (width, height) = window.get_size();
    (x + width / 2, y + height / 2)
}

/// Wayland compositors place windows themselves. `glfwGetWindowPos` reports that as an error,
/// and the default GLFW callback turns the error into a panic.
fn window_origin(glfw: &glfw::Glfw, window: &glfw::Window) -> Option<(i32, i32)> {
    if glfw.get_platform() == glfw::Platform::Wayland {
        None
    } else {
        Some(window.get_pos())
    }
}

fn glfw_error(error: glfw::Error, description: String) {
    if error == glfw::Error::FormatUnavailable || wayland_position_unavailable(&description) {
        return;
    }
    panic!("GLFW Error: {description}");
}

fn wayland_position_unavailable(description: &str) -> bool {
    description.contains("does not provide the window position")
}

/// True when something already accepts TCP on the loopback Banter address.
pub(crate) fn local_server_listening() -> bool {
    let Ok(addr) = banter_server::DEFAULT_ADDR.parse() else {
        return false;
    };
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Start the loopback server when nothing is listening on `DEFAULT_ADDR`.
/// Used when the user is already logged in on Local, or when they select Local in settings.
pub(crate) fn ensure_local_server() -> Result<(), String> {
    let addr = banter_server::DEFAULT_ADDR;
    if local_server_listening() {
        return Ok(());
    }
    let data = banter_server::default_data_dir();
    let database_url = banter_server::database_url();
    std::fs::create_dir_all(&data).map_err(|err| err.to_string())?;
    std::thread::spawn(move || {
        if let Err(err) = banter_server::serve(addr, &data, &database_url) {
            eprintln!("Banter server: {err}");
        }
    });
    let health = format!("http://{addr}/v1/health");
    for _ in 0..50 {
        if ureq::get(&health).call().is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the local server did not start".into())
}

pub(crate) struct GlfwClipboard;

pub(crate) fn copy_to_clipboard(value: &str) {
    GlfwClipboard.set(value);
}

impl ClipboardBackend for GlfwClipboard {
    fn get(&mut self) -> Option<String> {
        unsafe {
            let text = glfw::ffi::glfwGetClipboardString(std::ptr::null_mut());
            if text.is_null() {
                None
            } else {
                std::ffi::CStr::from_ptr(text).to_str().ok().map(str::to_string)
            }
        }
    }

    fn set(&mut self, value: &str) {
        let Ok(text) = std::ffi::CString::new(value) else { return };
        unsafe { glfw::ffi::glfwSetClipboardString(std::ptr::null_mut(), text.as_ptr()) }
    }
}

fn install_font(imgui: &mut Context) -> Option<dear_imgui_rs::fonts::FontId> {
    // The default bitmap font is about 13 px and was still the active font, so the
    // shell looked like 8 px text. Body text is 16 px; bold is for category labels.
    // Windows uses Segoe UI. Linux prefers Liberation Sans, then Noto Sans, then DejaVu.
    let atlas = imgui.font_atlas();
    atlas.clear_fonts();
    let body = load_first_face(atlas, &body_font_paths(), 16.0);
    let bold = load_first_face(atlas, &bold_font_paths(), 16.0);
    bold.or(body)
}

fn body_font_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        return vec![r"C:\Windows\Fonts\segoeui.ttf"];
    }
    #[cfg(target_os = "macos")]
    {
        return vec![
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
        ];
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        vec![
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
    }
}

fn bold_font_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        return vec![r"C:\Windows\Fonts\segoeuib.ttf"];
    }
    #[cfg(target_os = "macos")]
    {
        return vec![
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/Library/Fonts/Arial Bold.ttf",
        ];
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        vec![
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ]
    }
}

fn semibold_font_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        return vec![r"C:\Windows\Fonts\seguisb.ttf"];
    }
    #[cfg(target_os = "macos")]
    {
        return bold_font_paths();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let mut paths = vec![
            "/usr/share/fonts/noto/NotoSans-Medium.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Medium.ttf",
        ];
        paths.extend(bold_font_paths());
        paths
    }
}

fn load_first_face(
    atlas: &dear_imgui_rs::fonts::FontAtlas,
    paths: &[&str],
    size: f32,
) -> Option<dear_imgui_rs::fonts::FontId> {
    paths.iter().find_map(|path| load_face(atlas, path, size))
}

fn load_face(atlas: &dear_imgui_rs::fonts::FontAtlas, path: &str, size: f32) -> Option<dear_imgui_rs::fonts::FontId> {
    let bytes = std::fs::read(path).ok()?;
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    Some(atlas.add_font(&[unsafe { dear_imgui_rs::FontSource::ttf_data_with_size(leaked, size) }]))
}

fn feed_event(io: &mut dear_imgui_rs::Io, app: &mut App, event: glfw::WindowEvent) {
    match event {
        glfw::WindowEvent::Key(key, _, action, _) => {
            app.note_input();
            if let Some(mapped) = map_key(key) {
                let down = action != glfw::Action::Release;
                io.add_key_event(mapped, down);
            }
        }
        glfw::WindowEvent::Char(character) => {
            app.note_input();
            io.add_input_character(character);
        }
        glfw::WindowEvent::MouseButton(button, action, _) => {
            app.note_input();
            if let Some(mapped) = map_mouse(button) {
                io.add_mouse_button_event(mapped, action != glfw::Action::Release);
            }
        }
        glfw::WindowEvent::CursorPos(_, _) => app.note_input(),
        glfw::WindowEvent::Scroll(x, y) => {
            app.note_input();
            io.add_mouse_wheel_event([x as f32, y as f32]);
        }
        glfw::WindowEvent::Focus(focused) => io.add_focus_event(focused),
        _ => {}
    }
}

fn map_mouse(button: glfw::MouseButton) -> Option<MouseButton> {
    match button {
        glfw::MouseButton::Button1 => Some(MouseButton::Left),
        glfw::MouseButton::Button2 => Some(MouseButton::Right),
        glfw::MouseButton::Button3 => Some(MouseButton::Middle),
        _ => None,
    }
}

fn map_key(key: glfw::Key) -> Option<Key> {
    Some(match key {
        glfw::Key::Tab => Key::Tab,
        glfw::Key::Left => Key::LeftArrow,
        glfw::Key::Right => Key::RightArrow,
        glfw::Key::Up => Key::UpArrow,
        glfw::Key::Down => Key::DownArrow,
        glfw::Key::PageUp => Key::PageUp,
        glfw::Key::PageDown => Key::PageDown,
        glfw::Key::Home => Key::Home,
        glfw::Key::End => Key::End,
        glfw::Key::Insert => Key::Insert,
        glfw::Key::Delete => Key::Delete,
        glfw::Key::Backspace => Key::Backspace,
        glfw::Key::Space => Key::Space,
        glfw::Key::Enter | glfw::Key::KpEnter => Key::Enter,
        glfw::Key::Escape => Key::Escape,
        glfw::Key::LeftControl => Key::LeftCtrl,
        glfw::Key::LeftShift => Key::LeftShift,
        glfw::Key::LeftAlt => Key::LeftAlt,
        glfw::Key::LeftSuper => Key::LeftSuper,
        glfw::Key::RightControl => Key::RightCtrl,
        glfw::Key::RightShift => Key::RightShift,
        glfw::Key::RightAlt => Key::RightAlt,
        glfw::Key::RightSuper => Key::RightSuper,
        glfw::Key::Num0 => Key::Key0,
        glfw::Key::Num1 => Key::Key1,
        glfw::Key::Num2 => Key::Key2,
        glfw::Key::Num3 => Key::Key3,
        glfw::Key::Num4 => Key::Key4,
        glfw::Key::Num5 => Key::Key5,
        glfw::Key::Num6 => Key::Key6,
        glfw::Key::Num7 => Key::Key7,
        glfw::Key::Num8 => Key::Key8,
        glfw::Key::Num9 => Key::Key9,
        glfw::Key::A => Key::A,
        glfw::Key::B => Key::B,
        glfw::Key::C => Key::C,
        glfw::Key::D => Key::D,
        glfw::Key::E => Key::E,
        glfw::Key::F => Key::F,
        glfw::Key::G => Key::G,
        glfw::Key::H => Key::H,
        glfw::Key::I => Key::I,
        glfw::Key::J => Key::J,
        glfw::Key::K => Key::K,
        glfw::Key::L => Key::L,
        glfw::Key::M => Key::M,
        glfw::Key::N => Key::N,
        glfw::Key::O => Key::O,
        glfw::Key::P => Key::P,
        glfw::Key::Q => Key::Q,
        glfw::Key::R => Key::R,
        glfw::Key::S => Key::S,
        glfw::Key::T => Key::T,
        glfw::Key::U => Key::U,
        glfw::Key::V => Key::V,
        glfw::Key::W => Key::W,
        glfw::Key::X => Key::X,
        glfw::Key::Y => Key::Y,
        glfw::Key::Z => Key::Z,
        glfw::Key::F1 => Key::F1,
        glfw::Key::F2 => Key::F2,
        glfw::Key::F3 => Key::F3,
        glfw::Key::F4 => Key::F4,
        glfw::Key::F5 => Key::F5,
        glfw::Key::F6 => Key::F6,
        glfw::Key::F7 => Key::F7,
        glfw::Key::F8 => Key::F8,
        glfw::Key::F9 => Key::F9,
        glfw::Key::F10 => Key::F10,
        glfw::Key::F11 => Key::F11,
        glfw::Key::F12 => Key::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::wayland_position_unavailable;

    #[test]
    fn wayland_window_position_is_not_fatal() {
        assert!(wayland_position_unavailable(
            "Wayland: The platform does not provide the window position"
        ));
        assert!(!wayland_position_unavailable("GLFW: an OpenGL context could not be created"));
    }
}
