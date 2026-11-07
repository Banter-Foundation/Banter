//! GLFW window, OpenGL 3.3, and the Dear ImGui docking branch.
//!
//! User Settings and Server Settings open in a second shared-context GLFW window so
//! the main Banter shell stays usable. Closing that window clears settings state on
//! the main app. Only one of User / Server Settings is hosted at a time.
//!
//! Banter Settings uses its **own Dear ImGui context** (dear-imgui-rs
//! [`SuspendedContext`], activated for that pass). Sharing one ImGui context across
//! two OS windows with two NewFrame passes left Settings drawing but dead to input
//! after BA-84/BA-85 (#47/#48): sibling Focus/mouse clearing, WantCapture, and
//! Windows shared-GLFW focus lies all poison a single IO state. Each surface now
//! feeds only its own window into its own context.
//!
//! dear-imgui-rs allows one active Context at a time — the main context is suspended
//! while Settings runs, then reactivated.
//!
//! Shared GLFW children also need `is_shared` cleared before Drop or the OS window
//! leaks as a zombie and looks like it "will not close".

use std::rc::Rc;
use std::time::Duration;

use dear_imgui_glow::GlowRenderer;
use dear_imgui_rs::{ClipboardBackend, ConfigFlags, Context, Key, MouseButton, SuspendedContext};
use glfw::Context as _;
use glow::HasContext;

use crate::logic::{
    clamp_window_size, either_key_held, load_json, pick_display_refresh_hz, ui_swap_needs_update,
    DisplayRefresh, VideoPrefs,
};
use crate::server_settings::{
    settings_os_window_default_size, settings_os_window_title, SettingsOsKind,
};
use crate::stream_watch::{stream_popout_default_size, STREAM_POPOUT_WINDOW_TITLE};
use crate::titlebar::{dragged_window_pos, TitleBarAction, TitleBarChrome};
use crate::ui::App;

pub(crate) fn run() -> Result<(), String> {
    let data_dir = banter_server::default_data_dir();
    let prefs = crate::load_json::<crate::DevServerPrefs>(&data_dir.join("dev-server.json"))
        .unwrap_or_default();
    let has_saved_session = crate::load_token(&data_dir).is_some();
    let base = crate::startup_api_base(&prefs);
    let local_available = match crate::startup_local_policy(&prefs, has_saved_session) {
        crate::StartupLocalPolicy::SkipLocalUseRemote => false,
        crate::StartupLocalPolicy::StartLocal => ensure_local_server().is_ok(),
        crate::StartupLocalPolicy::PreferRunningLocalElseAskRemote => local_server_listening(),
    };
    let ask_remote =
        crate::startup_should_prompt_remote(&prefs, local_available, has_saved_session);
    let startup_video = load_video_prefs(&data_dir);
    let (start_w, start_h) = clamp_window_size(startup_video.width, startup_video.height);

    let mut glfw = glfw::init(glfw_error).map_err(|err| format!("GLFW init failed: {err}"))?;
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    // Client-drawn Mac-style caption (BA-105). Native OS chrome is replaced in-app.
    glfw.window_hint(glfw::WindowHint::Decorated(false));
    let (mut window, events) = glfw
        .create_window(
            start_w as u32,
            start_h as u32,
            "Banter",
            glfw::WindowMode::Windowed,
        )
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
    imgui
        .set_ini_filename(Some(ini))
        .map_err(|err| err.to_string())?;
    let fonts = install_ui_fonts(&mut imgui);
    let mut renderer = GlowRenderer::with_shared_context(Rc::clone(&gl), &mut imgui)
        .map_err(|err| err.to_string())?;

    let mut app = App::open(base, data_dir);
    if ask_remote {
        app.prompt_remote_server(
            "Local server is not available. Enter a remote host, or start the local server and choose Local.",
        );
    }
    app.set_bold_font(fonts.bold);
    app.set_semibold_font(fonts.semibold);
    app.set_italic_font(fonts.italic);
    app.set_bold_italic_font(fonts.bold_italic);
    let mut last_time = glfw.get_time();
    // (vsync, refresh_hz) last pushed to GLFW. Reapplied when the window moves
    // to another monitor or the active mode's Hz changes.
    let mut applied_swap: Option<(bool, u32)> = None;
    let mut vsync_on = startup_video.vsync;
    let mut settings_host: Option<SettingsHost> = None;
    let mut stream_popout_host: Option<StreamPopoutHost> = None;

    while !window.should_close() {
        glfw.poll_events();
        if let Some(video) = app.take_video_apply() {
            vsync_on = video.vsync;
            apply_video(&mut glfw, &mut window, &video, &mut windowed);
            // Force a refresh-aware swap pass after Display settings change.
            applied_swap = None;
        }
        sync_ui_swap_interval(&mut glfw, &mut window, vsync_on, &mut applied_swap);
        imgui = sync_settings_window(
            &mut glfw,
            &mut window,
            &mut settings_host,
            &mut app,
            imgui,
            &gl,
        )?;
        imgui = sync_stream_popout_window(
            &mut glfw,
            &mut window,
            &mut stream_popout_host,
            &mut app,
            imgui,
            &gl,
        )?;

        let main_events: Vec<_> = glfw::flush_messages(&events)
            .map(|(_, event)| event)
            .collect();
        let settings_events: Vec<_> = settings_host
            .as_ref()
            .map(|host| {
                glfw::flush_messages(&host.events)
                    .map(|(_, event)| event)
                    .collect()
            })
            .unwrap_or_default();
        let stream_events: Vec<_> = stream_popout_host
            .as_ref()
            .map(|host| {
                glfw::flush_messages(&host.events)
                    .map(|(_, event)| event)
                    .collect()
            })
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
        let stream_focused = stream_popout_host
            .as_ref()
            .map(|host| host.window.is_focused())
            .unwrap_or(false);
        // Main keeps pointer only when no secondary Banter OS window is focused.
        let main_receive =
            main_surface_receives_input(window.is_focused(), settings_focused || stream_focused);

        {
            let _prepare = crate::profiler::scope_section(crate::profiler::Section::Prepare);
            app.prepare(&mut imgui);
        }

        if let Some(host) = settings_host.as_mut() {
            // dear-imgui-rs: only one Context may be active. Suspend main, run Settings, restore.
            let suspended_main = imgui.suspend().map_err(|err| err.to_string())?;
            let settings_result =
                draw_settings_surface(host, &gl, &mut window, &settings_events, &mut app, delta);
            imgui = suspended_main.activate().map_err(|err| err.to_string())?;
            settings_result?;
        }

        if let Some(host) = stream_popout_host.as_mut() {
            let suspended_main = imgui.suspend().map_err(|err| err.to_string())?;
            let stream_result =
                draw_stream_popout_surface(host, &gl, &mut window, &stream_events, &mut app, delta);
            imgui = suspended_main.activate().map_err(|err| err.to_string())?;
            stream_result?;
        }

        draw_main_surface(
            &mut imgui,
            &mut renderer,
            &gl,
            &mut window,
            &main_events,
            &mut app,
            width,
            height,
            framebuffer_width,
            framebuffer_height,
            delta,
            main_receive,
        )?;
        apply_title_bar_chrome(&glfw, &mut window, app.title_bar_mut());
        if let Some(host) = settings_host.as_mut() {
            apply_title_bar_chrome(&glfw, &mut host.window, app.settings_title_bar_mut());
        }
        if let Some(host) = stream_popout_host.as_mut() {
            apply_title_bar_chrome(&glfw, &mut host.window, app.stream_title_bar_mut());
        }
    }
    Ok(())
}

/// Apply traffic-light actions and caption drag from the ImGui pass.
fn apply_title_bar_chrome(
    glfw: &glfw::Glfw,
    window: &mut glfw::Window,
    chrome: &mut TitleBarChrome,
) {
    for action in chrome.take_actions() {
        match action {
            TitleBarAction::Close => window.set_should_close(true),
            TitleBarAction::Minimize => window.iconify(),
            TitleBarAction::ToggleMaximize => {
                if window.is_maximized() {
                    window.restore();
                } else {
                    window.maximize();
                }
            }
        }
    }
    if let Some((grab_x, grab_y)) = chrome.drag_grab {
        if let Some((wx, wy)) = window_origin(glfw, window) {
            let (cx, cy) = window.get_cursor_pos();
            let (nx, ny) = dragged_window_pos(
                wx as f32,
                wy as f32,
                cx as f32,
                cy as f32,
                grab_x,
                grab_y,
            );
            window.set_pos(nx.round() as i32, ny.round() as i32);
        }
    }
    if chrome.drag_grab.is_some()
        && window.get_mouse_button(glfw::MouseButton::Button1) != glfw::Action::Press
    {
        chrome.end_drag();
    }
}

fn draw_main_surface(
    imgui: &mut Context,
    renderer: &mut GlowRenderer,
    gl: &glow::Context,
    window: &mut glfw::PWindow,
    main_events: &[glfw::WindowEvent],
    app: &mut App,
    width: i32,
    height: i32,
    framebuffer_width: i32,
    framebuffer_height: i32,
    delta: f64,
    receive_pointer: bool,
) -> Result<(), String> {
    window.make_current();
    unsafe {
        gl.viewport(0, 0, framebuffer_width, framebuffer_height);
    }
    feed_frame_io(
        imgui,
        window,
        main_events,
        app,
        width,
        height,
        framebuffer_width,
        framebuffer_height,
        delta,
        receive_pointer,
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
        let pending = imgui.render(
            renderer
                .renderer_consumer()
                .map_err(|err| err.to_string())?,
        );
        renderer.render(pending).map_err(|err| err.to_string())?;
    }
    window.swap_buffers();
    Ok(())
}

fn draw_settings_surface(
    host: &mut SettingsHost,
    gl: &glow::Context,
    main: &mut glfw::PWindow,
    settings_events: &[glfw::WindowEvent],
    app: &mut App,
    delta: f64,
) -> Result<(), String> {
    let (sw, sh) = host.window.get_size();
    let (fbw, fbh) = host.window.get_framebuffer_size();
    if sw <= 0 || sh <= 0 {
        return Ok(());
    }
    host.window.make_current();
    unsafe {
        gl.viewport(0, 0, fbw, fbh);
    }

    let kind = host.kind;
    let SettingsHost {
        window,
        imgui,
        renderer,
        fonts,
        ..
    } = host;
    let fonts = UiFonts {
        bold: fonts.bold,
        semibold: fonts.semibold,
        italic: fonts.italic,
        bold_italic: fonts.bold_italic,
    };

    // Main ImGui context is already suspended by the caller. Activate Settings for this pass.
    imgui
        .try_with_active(|imgui| -> Result<(), String> {
            // Own-window IO only — never share mouse/focus with the main ImGui context.
            feed_frame_io(
                imgui,
                window,
                settings_events,
                app,
                sw,
                sh,
                fbw,
                fbh,
                delta,
                /* receive_pointer */ true,
            );
            app.begin_settings_surface(fonts.bold, fonts.semibold, fonts.italic, fonts.bold_italic);
            let prepare_draw = (|| -> Result<(), String> {
                app.prepare_settings(imgui);
                unsafe {
                    gl.clear_color(0.08, 0.08, 0.10, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
                {
                    let ui = imgui.frame();
                    match kind {
                        SettingsOsKind::User => app.draw_settings_window(&ui),
                        SettingsOsKind::Server => app.draw_server_settings_window(&ui),
                    }
                }
                let pending = imgui.render(
                    renderer
                        .renderer_consumer()
                        .map_err(|err| err.to_string())?,
                );
                renderer.render(pending).map_err(|err| err.to_string())?;
                Ok(())
            })();
            app.end_settings_surface();
            prepare_draw
        })
        .map_err(|err| err.to_string())?;

    window.swap_buffers();
    main.make_current();
    let (mfbw, mfbh) = main.get_framebuffer_size();
    unsafe {
        gl.viewport(0, 0, mfbw, mfbh);
    }
    Ok(())
}

struct UiFonts {
    bold: Option<dear_imgui_rs::fonts::FontId>,
    semibold: Option<dear_imgui_rs::fonts::FontId>,
    italic: Option<dear_imgui_rs::fonts::FontId>,
    bold_italic: Option<dear_imgui_rs::fonts::FontId>,
}

struct SettingsHost {
    window: glfw::PWindow,
    events: glfw::GlfwReceiver<(f64, glfw::WindowEvent)>,
    imgui: SuspendedContext,
    renderer: GlowRenderer,
    fonts: UiFonts,
    /// User vs Server Settings — drives the OS title and which App draw path runs.
    kind: SettingsOsKind,
}

struct StreamPopoutHost {
    window: glfw::PWindow,
    events: glfw::GlfwReceiver<(f64, glfw::WindowEvent)>,
    imgui: SuspendedContext,
    renderer: GlowRenderer,
    fonts: UiFonts,
}

fn sync_settings_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
    host: &mut Option<SettingsHost>,
    app: &mut App,
    mut main_imgui: Context,
    gl: &Rc<glow::Context>,
) -> Result<Context, String> {
    let want = app.settings_os_kind();
    if let Some(kind) = want {
        if host.is_none() {
            let (imgui, created) = create_settings_window(glfw, main, main_imgui, gl, kind)?;
            main_imgui = imgui;
            *host = Some(created);
        }
        if let Some(settings) = host.as_mut() {
            if settings.kind != kind {
                settings.kind = kind;
                settings
                    .window
                    .set_title(settings_os_window_title(kind));
            }
            if app.take_settings_window_focus() {
                settings.window.focus();
            }
            if settings.window.should_close() {
                app.notify_settings_window_closed();
                if let Some(closed) = host.take() {
                    main_imgui = destroy_settings_host(closed, main, main_imgui, app)?;
                }
                main.focus();
            }
        }
    } else if let Some(closed) = host.take() {
        main_imgui = destroy_settings_host(closed, main, main_imgui, app)?;
        main.focus();
    }
    Ok(main_imgui)
}

/// glfw-rs marks `create_shared` windows with `is_shared = true` and then **skips**
/// `glfwDestroyWindow` on Drop (it still frees the event callback box, leaving a live
/// OS window with a dangling user pointer). Banter Settings must clear the flag so the
/// shared child is actually destroyed when User Settings closes.
fn prepare_shared_settings_window_for_destroy(window: &mut glfw::Window) {
    window.is_shared = false;
}

fn destroy_settings_host(
    host: SettingsHost,
    main: &mut glfw::PWindow,
    main_imgui: Context,
    app: &mut App,
) -> Result<Context, String> {
    let SettingsHost {
        mut window,
        events: _,
        imgui,
        renderer,
        fonts: _,
        kind: _,
    } = host;

    // Release Settings-context textures / renderer while that context is active.
    let suspended_main = main_imgui.suspend().map_err(|err| err.to_string())?;
    let mut active = imgui.activate().map_err(|err| err.to_string())?;
    app.release_settings_gpu(&mut active);
    drop(renderer);
    drop(active);
    let main_imgui = suspended_main.activate().map_err(|err| err.to_string())?;

    // GLFW requires a different context current when destroying a share-group member.
    main.make_current();
    window.hide();
    prepare_shared_settings_window_for_destroy(&mut window);
    drop(window);
    Ok(main_imgui)
}

fn create_settings_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
    main_imgui: Context,
    gl: &Rc<glow::Context>,
    kind: SettingsOsKind,
) -> Result<(Context, SettingsHost), String> {
    let (width, height) = settings_os_window_default_size(kind);
    let title = settings_os_window_title(kind);
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Decorated(false));
    glfw.window_hint(glfw::WindowHint::Visible(true));
    glfw.window_hint(glfw::WindowHint::Focused(true));
    let (mut window, events) = main
        .create_shared(
            width as u32,
            height as u32,
            title,
            glfw::WindowMode::Windowed,
        )
        .ok_or_else(|| "GLFW could not open the Banter Settings window.".to_string())?;
    window.set_all_polling(true);
    if let Some((x, y)) = window_origin(glfw, main) {
        window.set_pos(x + 48, y + 48);
    }
    window.focus();
    window.make_current();

    // Build a dedicated ImGui context while main is suspended (one active Context at a time).
    let suspended_main = main_imgui.suspend().map_err(|err| err.to_string())?;
    let mut settings_imgui = Context::create();
    settings_imgui.set_clipboard_backend(GlfwClipboard);
    let fonts = install_ui_fonts(&mut settings_imgui);
    let renderer = GlowRenderer::with_shared_context(Rc::clone(gl), &mut settings_imgui)
        .map_err(|err| err.to_string())?;
    let settings_imgui = settings_imgui.suspend().map_err(|err| err.to_string())?;
    let main_imgui = suspended_main.activate().map_err(|err| err.to_string())?;

    // Main window stays current for prepare/texture work until the settings pass.
    main.make_current();
    Ok((
        main_imgui,
        SettingsHost {
            window,
            events,
            imgui: settings_imgui,
            renderer,
            fonts,
            kind,
        },
    ))
}

/// Each ImGui context sets AppFocused from its own window's `receive_pointer`.
fn imgui_focus_for_surface(receive_pointer: bool) -> bool {
    receive_pointer
}

/// Main shell suppresses pointer while a secondary Banter OS window (Settings or Stream) is
/// focused so overlapping content-relative cursor coords do not ghost-hover the shell.
fn main_surface_receives_input(main_focused: bool, secondary_focused: bool) -> bool {
    main_focused && !secondary_focused
}

fn sync_stream_popout_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
    host: &mut Option<StreamPopoutHost>,
    app: &mut App,
    mut main_imgui: Context,
    gl: &Rc<glow::Context>,
) -> Result<Context, String> {
    let want = app.stream_popout_os_window_desired();
    if want {
        if host.is_none() {
            let (imgui, created) = create_stream_popout_window(glfw, main, main_imgui, gl)?;
            main_imgui = imgui;
            *host = Some(created);
        }
        if let Some(stream) = host.as_mut() {
            if stream.window.should_close() {
                app.notify_stream_popout_window_closed();
                if let Some(closed) = host.take() {
                    main_imgui = destroy_stream_popout_host(closed, main, main_imgui, app)?;
                }
                main.focus();
            }
        }
    } else if let Some(closed) = host.take() {
        main_imgui = destroy_stream_popout_host(closed, main, main_imgui, app)?;
        // Keep focus on main unless Settings still wants it.
        if app.settings_os_kind().is_none() {
            main.focus();
        }
    }
    Ok(main_imgui)
}

fn destroy_stream_popout_host(
    host: StreamPopoutHost,
    main: &mut glfw::PWindow,
    main_imgui: Context,
    app: &mut App,
) -> Result<Context, String> {
    let StreamPopoutHost {
        mut window,
        events: _,
        imgui,
        renderer,
        fonts: _,
    } = host;

    let suspended_main = main_imgui.suspend().map_err(|err| err.to_string())?;
    let mut active = imgui.activate().map_err(|err| err.to_string())?;
    app.release_stream_popout_gpu(&mut active);
    drop(renderer);
    drop(active);
    let main_imgui = suspended_main.activate().map_err(|err| err.to_string())?;

    main.make_current();
    window.hide();
    prepare_shared_settings_window_for_destroy(&mut window);
    drop(window);
    Ok(main_imgui)
}

fn create_stream_popout_window(
    glfw: &mut glfw::Glfw,
    main: &mut glfw::PWindow,
    main_imgui: Context,
    gl: &Rc<glow::Context>,
) -> Result<(Context, StreamPopoutHost), String> {
    let (width, height) = stream_popout_default_size();
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    glfw.window_hint(glfw::WindowHint::Decorated(false));
    glfw.window_hint(glfw::WindowHint::Visible(true));
    glfw.window_hint(glfw::WindowHint::Focused(true));
    let (mut window, events) = main
        .create_shared(
            width as u32,
            height as u32,
            STREAM_POPOUT_WINDOW_TITLE,
            glfw::WindowMode::Windowed,
        )
        .ok_or_else(|| "GLFW could not open the Banter Stream window.".to_string())?;
    window.set_all_polling(true);
    if let Some((x, y)) = window_origin(glfw, main) {
        window.set_pos(x + 72, y + 72);
    }
    window.focus();
    window.make_current();

    let suspended_main = main_imgui.suspend().map_err(|err| err.to_string())?;
    let mut stream_imgui = Context::create();
    stream_imgui.set_clipboard_backend(GlfwClipboard);
    let fonts = install_ui_fonts(&mut stream_imgui);
    let renderer = GlowRenderer::with_shared_context(Rc::clone(gl), &mut stream_imgui)
        .map_err(|err| err.to_string())?;
    let stream_imgui = stream_imgui.suspend().map_err(|err| err.to_string())?;
    let main_imgui = suspended_main.activate().map_err(|err| err.to_string())?;

    main.make_current();
    Ok((
        main_imgui,
        StreamPopoutHost {
            window,
            events,
            imgui: stream_imgui,
            renderer,
            fonts,
        },
    ))
}

fn draw_stream_popout_surface(
    host: &mut StreamPopoutHost,
    gl: &glow::Context,
    main: &mut glfw::PWindow,
    stream_events: &[glfw::WindowEvent],
    app: &mut App,
    delta: f64,
) -> Result<(), String> {
    let (sw, sh) = host.window.get_size();
    let (fbw, fbh) = host.window.get_framebuffer_size();
    if sw <= 0 || sh <= 0 {
        return Ok(());
    }
    host.window.make_current();
    unsafe {
        gl.viewport(0, 0, fbw, fbh);
    }

    let StreamPopoutHost {
        window,
        imgui,
        renderer,
        fonts,
        ..
    } = host;
    let fonts = UiFonts {
        bold: fonts.bold,
        semibold: fonts.semibold,
        italic: fonts.italic,
        bold_italic: fonts.bold_italic,
    };

    imgui
        .try_with_active(|imgui| -> Result<(), String> {
            feed_frame_io(
                imgui,
                window,
                stream_events,
                app,
                sw,
                sh,
                fbw,
                fbh,
                delta,
                /* receive_pointer */ true,
            );
            app.begin_stream_popout_surface(
                fonts.bold,
                fonts.semibold,
                fonts.italic,
                fonts.bold_italic,
            );
            let prepare_draw = (|| -> Result<(), String> {
                app.prepare_stream_popout(imgui);
                unsafe {
                    gl.clear_color(0.12, 0.12, 0.14, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }
                {
                    let ui = imgui.frame();
                    app.draw_stream_popout_window(&ui);
                }
                let pending = imgui.render(
                    renderer
                        .renderer_consumer()
                        .map_err(|err| err.to_string())?,
                );
                renderer.render(pending).map_err(|err| err.to_string())?;
                Ok(())
            })();
            app.end_stream_popout_surface();
            prepare_draw
        })
        .map_err(|err| err.to_string())?;

    window.swap_buffers();
    main.make_current();
    let (mfbw, mfbh) = main.get_framebuffer_size();
    unsafe {
        gl.viewport(0, 0, mfbw, mfbh);
    }
    Ok(())
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
    // Per-context focus from this window's receive flag (not GLFW Focus transition events).
    io.add_focus_event(imgui_focus_for_surface(receive_pointer));
    if receive_pointer {
        let (cursor_x, cursor_y) = window.get_cursor_pos();
        io.add_mouse_pos_event([cursor_x as f32, cursor_y as f32]);
    } else {
        // Dear ImGui treats this as "no mouse" so the unfocused window does not steal clicks.
        io.add_mouse_pos_event([f32::MIN, f32::MIN]);
    }
    for event in events {
        match event {
            glfw::WindowEvent::Focus(_) => {}
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
    // Poll buttons each pass so a missed queue edge still reaches ImGui while the button is held.
    let press = |button| window.get_mouse_button(button) == glfw::Action::Press;
    if receive_pointer {
        io.add_mouse_button_event(MouseButton::Left, press(glfw::MouseButton::Button1));
        io.add_mouse_button_event(MouseButton::Right, press(glfw::MouseButton::Button2));
        io.add_mouse_button_event(MouseButton::Middle, press(glfw::MouseButton::Button3));
    } else {
        io.add_mouse_button_event(MouseButton::Left, false);
        io.add_mouse_button_event(MouseButton::Right, false);
        io.add_mouse_button_event(MouseButton::Middle, false);
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
        io.add_key_event(
            Key::ModAlt,
            either_key_held(held(glfw::Key::LeftAlt), held(glfw::Key::RightAlt)),
        );
        io.add_key_event(
            Key::ModSuper,
            either_key_held(held(glfw::Key::LeftSuper), held(glfw::Key::RightSuper)),
        );
        // Poll Escape so Settings close works even when the Key event was coalesced away
        // (CursorPos can flood the glfw-rs channel) while GLFW still holds the key state.
        io.add_key_event(Key::Escape, held(glfw::Key::Escape));
    } else {
        io.add_key_event(Key::ModCtrl, false);
        io.add_key_event(Key::ModShift, false);
        io.add_key_event(Key::ModAlt, false);
        io.add_key_event(Key::ModSuper, false);
        io.add_key_event(Key::Escape, false);
    }
}

fn install_ui_fonts(imgui: &mut Context) -> UiFonts {
    let bold = install_font(imgui);
    let semibold = load_first_face(imgui.font_atlas(), &semibold_font_paths(), 16.0).or(bold);
    let italic = load_first_face(imgui.font_atlas(), &italic_font_paths(), 16.0);
    let bold_italic = load_first_face(imgui.font_atlas(), &bold_italic_font_paths(), 16.0).or(bold);
    UiFonts {
        bold,
        semibold,
        italic,
        bold_italic,
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
    let currently_fullscreen =
        window.with_window_mode(|mode| matches!(mode, glfw::WindowMode::FullScreen(_)));
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
                std::ffi::CStr::from_ptr(text)
                    .to_str()
                    .ok()
                    .map(str::to_string)
            }
        }
    }

    fn set(&mut self, value: &str) {
        let Ok(text) = std::ffi::CString::new(value) else {
            return;
        };
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

fn italic_font_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        return vec![r"C:\Windows\Fonts\segoeuii.ttf"];
    }
    #[cfg(target_os = "macos")]
    {
        return vec![
            "/System/Library/Fonts/Supplemental/Arial Italic.ttf",
            "/Library/Fonts/Arial Italic.ttf",
        ];
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        vec![
            "/usr/share/fonts/liberation/LiberationSans-Italic.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
            "/usr/share/fonts/noto/NotoSans-Italic.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Italic.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Oblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
        ]
    }
}

fn bold_italic_font_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        return vec![r"C:\Windows\Fonts\segoeuiz.ttf"];
    }
    #[cfg(target_os = "macos")]
    {
        return vec![
            "/System/Library/Fonts/Supplemental/Arial Bold Italic.ttf",
            "/Library/Fonts/Arial Bold Italic.ttf",
        ];
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        vec![
            "/usr/share/fonts/liberation/LiberationSans-BoldItalic.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-BoldItalic.ttf",
            "/usr/share/fonts/noto/NotoSans-BoldItalic.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-BoldItalic.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-BoldOblique.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-BoldOblique.ttf",
        ]
    }
}

fn load_first_face(
    atlas: &dear_imgui_rs::fonts::FontAtlas,
    paths: &[&str],
    size: f32,
) -> Option<dear_imgui_rs::fonts::FontId> {
    paths.iter().find_map(|path| load_face(atlas, path, size))
}

fn load_face(
    atlas: &dear_imgui_rs::fonts::FontAtlas,
    path: &str,
    size: f32,
) -> Option<dear_imgui_rs::fonts::FontId> {
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
    use super::{
        imgui_focus_for_surface, main_surface_receives_input,
        prepare_shared_settings_window_for_destroy, wayland_position_unavailable,
    };
    use dear_imgui_rs::{Context, SuspendedContext};

    #[test]
    fn wayland_window_position_is_not_fatal() {
        assert!(wayland_position_unavailable(
            "Wayland: The platform does not provide the window position"
        ));
        assert!(!wayland_position_unavailable(
            "GLFW: an OpenGL context could not be created"
        ));
    }

    #[test]
    fn each_surface_context_sets_imgui_focus_from_its_own_receive_pointer() {
        // BA-86: Settings and main no longer share one ImGui IO. Each context still writes
        // AppFocused from that surface's receive_pointer (GLFW Focus events are transition-only).
        assert!(
            !imgui_focus_for_surface(false),
            "unfocused surface must clear ImGui AppFocused"
        );
        assert!(
            imgui_focus_for_surface(true),
            "Banter Settings context must set ImGui AppFocused so controls and close work"
        );
    }

    #[test]
    fn main_suppresses_pointer_while_settings_os_window_is_focused() {
        // Settings has its own ImGui context and always feeds its GLFW window. Main only needs
        // to drop pointer while Settings is focused so overlapping content-relative cursor
        // coords do not ghost-hover the shell.
        assert!(
            !main_surface_receives_input(true, true),
            "main must not keep ImGui pointer while Banter Settings is focused"
        );
        assert!(
            main_surface_receives_input(true, false),
            "main keeps ImGui pointer when focused and Settings is not"
        );
        assert!(!main_surface_receives_input(false, false));
    }

    #[test]
    fn settings_imgui_context_is_independent_via_suspend_activate() {
        // dear-imgui-rs allows one active Context. Banter Settings must live as a
        // SuspendedContext and run under try_with_active while main is suspended — otherwise
        // two NewFrames on one context leave Settings drawn but dead to input (BA-84/85).
        let main = Context::create();
        let suspended_main = main.suspend_or_panic();
        let mut settings = SuspendedContext::create();
        let fed = settings
            .try_with_active(|ctx| {
                ctx.io_mut().add_focus_event(true);
                ctx.io_mut().add_mouse_pos_event([12.0, 34.0]);
                ctx.io_mut().set_display_size([640.0, 480.0]);
                Ok::<_, ()>(ctx.io().display_size())
            })
            .expect("settings context activates while main is suspended");
        assert_eq!(fed, [640.0, 480.0]);
        drop(settings);
        let main = suspended_main.activate_or_panic();
        assert_ne!(
            main.io().display_size(),
            [640.0, 480.0],
            "main context IO stays separate"
        );
        drop(main);
    }

    /// glfw-rs `create_shared` sets `Window.is_shared = true`, and `Drop` then frees the
    /// event callbacks but skips `glfwDestroyWindow`. Banter Settings must clear that flag
    /// before drop or the OS window becomes a zombie (blank/frozen) with a dangling
    /// user-pointer — close appears to do nothing.
    #[test]
    fn shared_settings_window_must_clear_is_shared_before_drop() {
        use glfw::Context as _;

        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        let Ok(mut glfw) = glfw::init(|_, _| {}) else {
            return;
        };
        glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        glfw.window_hint(glfw::WindowHint::Visible(false));
        let Some((mut main, _)) = glfw.create_window(
            64,
            64,
            "banter-settings-destroy-main",
            glfw::WindowMode::Windowed,
        ) else {
            return;
        };
        main.make_current();
        let Some((mut settings, _)) = main.create_shared(
            64,
            64,
            "banter-settings-destroy-child",
            glfw::WindowMode::Windowed,
        ) else {
            return;
        };
        assert!(
            settings.is_shared,
            "create_shared must mark the Banter Settings window shared (glfw-rs Drop skips DestroyWindow while set)"
        );
        prepare_shared_settings_window_for_destroy(&mut settings);
        assert!(
            !settings.is_shared,
            "closing Banter Settings must clear is_shared so Drop calls glfwDestroyWindow"
        );
        // Main stays current while the shared child is destroyed (GLFW requirement).
        main.make_current();
        drop(settings);
    }
}
