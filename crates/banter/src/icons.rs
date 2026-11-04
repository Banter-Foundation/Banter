//! Lucide icons vendored from lucide-static 0.544.0 (ISC).
//! Strokes are tinted here so they stay visible on the dark window.

pub fn rasterize_lucide(name: &str, size: u32, tint: [u8; 3]) -> Result<Vec<u8>, String> {
    if size == 0 || size > 512 {
        return Err("icon size is out of range".into());
    }
    let svg = lucide_svg(name).ok_or_else(|| format!("unknown icon {name}"))?;
    let colored = svg.replace(
        "currentColor",
        &format!("#{:02x}{:02x}{:02x}", tint[0], tint[1], tint[2]),
    );
    let tree = resvg::usvg::Tree::from_str(&colored, &resvg::usvg::Options::default()).map_err(|err| err.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or_else(|| "could not allocate the icon".to_string())?;
    let width = tree.size().width().max(1.0);
    let scale = size as f32 / width;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
        } else {
            pixel[0] = tint[0];
            pixel[1] = tint[1];
            pixel[2] = tint[2];
        }
    }
    if rgba.len() != (size as usize) * (size as usize) * 4 {
        return Err("icon raster size is wrong".into());
    }
    Ok(rgba)
}

fn lucide_svg(name: &str) -> Option<&'static str> {
    Some(match name {
        "check" => include_str!("../assets/lucide/check.svg"),
        "chevron-down" => include_str!("../assets/lucide/chevron-down.svg"),
        "chevron-right" => include_str!("../assets/lucide/chevron-right.svg"),
        "chevron-up" => include_str!("../assets/lucide/chevron-up.svg"),
        "circle" => include_str!("../assets/lucide/circle.svg"),
        "circle-plus" => include_str!("../assets/lucide/circle-plus.svg"),
        "ellipsis" => include_str!("../assets/lucide/ellipsis.svg"),
        "crown" => include_str!("../assets/lucide/crown.svg"),
        "file-text" => include_str!("../assets/lucide/file-text.svg"),
        "folder" => include_str!("../assets/lucide/folder.svg"),
        "folder-plus" => include_str!("../assets/lucide/folder-plus.svg"),
        "hash" => include_str!("../assets/lucide/hash.svg"),
        "headphone-off" => include_str!("../assets/lucide/headphone-off.svg"),
        "headphones" => include_str!("../assets/lucide/headphones.svg"),
        "image" => include_str!("../assets/lucide/image.svg"),
        "log-out" => include_str!("../assets/lucide/log-out.svg"),
        "message-circle" => include_str!("../assets/lucide/message-circle.svg"),
        "mic" => include_str!("../assets/lucide/mic.svg"),
        "mic-off" => include_str!("../assets/lucide/mic-off.svg"),
        "monitor" => include_str!("../assets/lucide/monitor.svg"),
        "monitor-off" => include_str!("../assets/lucide/monitor-off.svg"),
        "mouse-pointer-2" => include_str!("../assets/lucide/mouse-pointer-2.svg"),
        "paperclip" => include_str!("../assets/lucide/paperclip.svg"),
        "pencil" => include_str!("../assets/lucide/pencil.svg"),
        "phone" => include_str!("../assets/lucide/phone.svg"),
        "phone-off" => include_str!("../assets/lucide/phone-off.svg"),
        "plus" => include_str!("../assets/lucide/plus.svg"),
        "reply" => include_str!("../assets/lucide/reply.svg"),
        "search" => include_str!("../assets/lucide/search.svg"),
        "settings" => include_str!("../assets/lucide/settings.svg"),
        "signal" => include_str!("../assets/lucide/signal.svg"),
        "square" => include_str!("../assets/lucide/square.svg"),
        "sticky-note" => include_str!("../assets/lucide/sticky-note.svg"),
        "trash-2" => include_str!("../assets/lucide/trash-2.svg"),
        "type" => include_str!("../assets/lucide/type.svg"),
        "user" => include_str!("../assets/lucide/user.svg"),
        "user-plus" => include_str!("../assets/lucide/user-plus.svg"),
        "users" => include_str!("../assets/lucide/users.svg"),
        "video" => include_str!("../assets/lucide/video.svg"),
        "video-off" => include_str!("../assets/lucide/video-off.svg"),
        "volume-2" => include_str!("../assets/lucide/volume-2.svg"),
        "x" => include_str!("../assets/lucide/x.svg"),
        _ => return None,
    })
}
