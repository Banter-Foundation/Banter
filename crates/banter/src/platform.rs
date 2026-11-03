//! GLFW window, OpenGL 3.3, and the Dear ImGui docking branch.

use std::rc::Rc;
use std::time::Duration;

use glfw::Context as _;
use dear_imgui_glow::GlowRenderer;
use dear_imgui_rs::{ClipboardBackend, ConfigFlags, Context, Key, MouseButton};
use glow::HasContext;

use crate::logic::either_key_held;
use crate::ui::App;

pub(crate) fn run() -> Result<(), String> {
    ensure_server()?;
    let mut glfw = glfw::init(glfw::fail_on_errors).map_err(|err| format!("GLFW init failed: {err}"))?;
    glfw.window_hint(glfw::WindowHint::ContextVersion(3, 3));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(glfw::OpenGlProfileHint::Core));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
    let (mut window, events) = glfw
        .create_window(1280, 800, "Banter", glfw::WindowMode::Windowed)
        .ok_or_else(|| "GLFW could not open a window. Banter needs a display.".to_string())?;
    window.make_current();
    window.set_all_polling(true);
    glfw.set_swap_interval(glfw::SwapInterval::Sync(1));

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
    let ini = banter_server::default_data_dir().join("banter-dock.ini");
    imgui.set_ini_filename(Some(ini)).map_err(|err| err.to_string())?;
    let bold_font = install_font(&mut imgui);
    let mut renderer = GlowRenderer::with_shared_context(Rc::clone(&gl), &mut imgui).map_err(|err| err.to_string())?;

    let base = format!("http://{}", banter_server::DEFAULT_ADDR);
    let mut app = App::open(base, banter_server::default_data_dir());
    app.set_bold_font(bold_font);
    let mut last_time = glfw.get_time();

    while !window.should_close() {
        glfw.poll_events();
        let (width, height) = window.get_size();
        let (framebuffer_width, framebuffer_height) = window.get_framebuffer_size();
        if width <= 0 || height <= 0 {
            window.swap_buffers();
            continue;
        }
        let now = glfw.get_time();
        let delta = (now - last_time).max(1.0 / 1000.0);
        last_time = now;
        {
            let io = imgui.io_mut();
            io.set_display_size([width as f32, height as f32]);
            io.set_delta_time(delta as f32);
            let scale_x = framebuffer_width as f32 / width as f32;
            let scale_y = framebuffer_height as f32 / height as f32;
            io.set_display_framebuffer_scale([scale_x, scale_y]);
            let (cursor_x, cursor_y) = window.get_cursor_pos();
            io.add_mouse_pos_event([cursor_x as f32, cursor_y as f32]);
            for (_, event) in glfw::flush_messages(&events) {
                feed_event(io, &mut app, event);
            }
            // Dear ImGui only treats Ctrl+C and Ctrl+V as shortcuts when ModCtrl is sent.
            // The physical Ctrl key alone does not count.
            let held = |key| window.get_key(key) == glfw::Action::Press;
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
        }
        app.set_v_held(window.get_key(glfw::Key::V) == glfw::Action::Press);
        app.prepare(&mut imgui);
        unsafe {
            gl.clear_color(0.08, 0.08, 0.10, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let ui = imgui.frame();
        app.draw(&ui);
        let pending = imgui.render(renderer.renderer_consumer().map_err(|err| err.to_string())?);
        renderer.render(pending).map_err(|err| err.to_string())?;
        window.swap_buffers();
    }
    Ok(())
}

fn ensure_server() -> Result<(), String> {
    let addr = banter_server::DEFAULT_ADDR;
    if std::net::TcpStream::connect_timeout(
        &addr.parse().map_err(|err: std::net::AddrParseError| err.to_string())?,
        Duration::from_millis(200),
    )
    .is_ok()
    {
        return Ok(());
    }
    let data = banter_server::default_data_dir();
    std::fs::create_dir_all(&data).map_err(|err| err.to_string())?;
    std::thread::spawn(move || {
        if let Err(err) = banter_server::serve(addr, &data) {
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

struct GlfwClipboard;

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
    // shell looked like 8 px text. Segoe UI at 16 px is the body size; bold is for
    // category labels at 12 px.
    let atlas = imgui.font_atlas();
    atlas.clear_fonts();
    let body = load_face(atlas, r"C:\Windows\Fonts\segoeui.ttf", 16.0);
    let bold = load_face(atlas, r"C:\Windows\Fonts\segoeuib.ttf", 16.0);
    if body.is_none() && bold.is_none() {
        let _ = load_face(atlas, r"C:\Windows\Fonts\segoeui.ttf", 16.0);
    }
    bold.or(body)
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
