//! Emote Library helpers: size caps, GIF/video duration gates (BA-83).

/// Hard max duration for both classic GIF and video GIF encodings (Owl).
pub const GIF_MAX_DURATION_MS: i64 = 15_000;

/// Emote still/animated image upload cap (Carbon parity).
pub const MAX_EMOTE_BYTES: usize = 256 * 1024;

/// Sticker upload cap (Carbon parity).
pub const MAX_STICKER_BYTES: usize = 2 * 1024 * 1024;

/// Classic GIF byte cap (Carbon parity). Video GIFs use [`MAX_VIDEO_GIF_BYTES`].
pub const MAX_GIF_BYTES: usize = 2 * 1024 * 1024;

/// Starting video GIF byte cap (plan: 4MB).
pub const MAX_VIDEO_GIF_BYTES: usize = 4 * 1024 * 1024;

/// One-loop duration of a classic GIF from Graphic Control Extension delays.
///
/// Delay is in centiseconds. Missing or `0` delay counts as **10 cs (100 ms)**.
/// Netscape loop count is ignored (infinite loop ≠ infinite duration).
/// Static / single-frame with no GCE → `0`.
pub fn classic_gif_duration_ms(bytes: &[u8]) -> Result<i64, ()> {
    if bytes.len() < 6 || (&bytes[0..6] != b"GIF89a" && &bytes[0..6] != b"GIF87a") {
        return Err(());
    }
    let mut i = 6;
    // Logical Screen Descriptor
    if bytes.len() < i + 7 {
        return Err(());
    }
    let packed = bytes[i + 4];
    i += 7;
    if packed & 0x80 != 0 {
        let gct_size = 3 * (1usize << ((packed & 0x07) + 1));
        if bytes.len() < i + gct_size {
            return Err(());
        }
        i += gct_size;
    }

    let mut total_cs: i64 = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x3B => break, // trailer
            0x21 => {
                // Extension
                if i + 2 >= bytes.len() {
                    return Err(());
                }
                let label = bytes[i + 1];
                i += 2;
                if label == 0xF9 {
                    // Graphic Control Extension — next block size then 4 bytes
                    if i >= bytes.len() || bytes[i] != 4 || i + 5 >= bytes.len() {
                        return Err(());
                    }
                    let delay = u16::from_le_bytes([bytes[i + 2], bytes[i + 3]]) as i64;
                    total_cs += if delay == 0 { 10 } else { delay };
                    i += 5; // size + 4 data
                    // block terminator
                    if i >= bytes.len() || bytes[i] != 0 {
                        return Err(());
                    }
                    i += 1;
                } else {
                    // Skip data sub-blocks
                    while i < bytes.len() {
                        let sz = bytes[i] as usize;
                        i += 1;
                        if sz == 0 {
                            break;
                        }
                        if bytes.len() < i + sz {
                            return Err(());
                        }
                        i += sz;
                    }
                }
            }
            0x2C => {
                // Image descriptor
                if bytes.len() < i + 10 {
                    return Err(());
                }
                let packed = bytes[i + 9];
                i += 10;
                if packed & 0x80 != 0 {
                    let lct_size = 3 * (1usize << ((packed & 0x07) + 1));
                    if bytes.len() < i + lct_size {
                        return Err(());
                    }
                    i += lct_size;
                }
                // LZW min code size + sub-blocks
                if i >= bytes.len() {
                    return Err(());
                }
                i += 1;
                while i < bytes.len() {
                    let sz = bytes[i] as usize;
                    i += 1;
                    if sz == 0 {
                        break;
                    }
                    if bytes.len() < i + sz {
                        return Err(());
                    }
                    i += sz;
                }
            }
            _ => return Err(()),
        }
    }
    Ok(total_cs * 10)
}

/// Duration in ms from an MP4/WebM-ish container. Currently reads MP4 `mvhd`.
pub fn video_container_duration_ms(bytes: &[u8]) -> Result<i64, ()> {
    if let Ok(ms) = mp4_mvhd_duration_ms(bytes) {
        return Ok(ms);
    }
    Err(())
}

fn mp4_mvhd_duration_ms(bytes: &[u8]) -> Result<i64, ()> {
    let mut i = 0usize;
    while i + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        if size < 8 || i + size > bytes.len() {
            return Err(());
        }
        let kind = &bytes[i + 4..i + 8];
        let payload = &bytes[i + 8..i + size];
        if kind == b"moov" || kind == b"trak" || kind == b"mdia" {
            if let Ok(ms) = mp4_mvhd_duration_ms(payload) {
                return Ok(ms);
            }
        } else if kind == b"mvhd" {
            return parse_mvhd(payload);
        }
        i += size;
    }
    Err(())
}

fn parse_mvhd(body: &[u8]) -> Result<i64, ()> {
    if body.is_empty() {
        return Err(());
    }
    let version = body[0];
    if version == 0 {
        if body.len() < 20 {
            return Err(());
        }
        let timescale = u32::from_be_bytes(body[12..16].try_into().unwrap());
        let duration = u32::from_be_bytes(body[16..20].try_into().unwrap());
        if timescale == 0 {
            return Err(());
        }
        Ok((duration as i64) * 1000 / timescale as i64)
    } else if version == 1 {
        if body.len() < 32 {
            return Err(());
        }
        let timescale = u32::from_be_bytes(body[20..24].try_into().unwrap());
        let duration = u64::from_be_bytes(body[24..32].try_into().unwrap());
        if timescale == 0 {
            return Err(());
        }
        Ok((duration as i64) * 1000 / timescale as i64)
    } else {
        Err(())
    }
}

pub fn max_bytes_for_kind(kind: crate::model::EmoteAssetKind, encoding: Option<crate::model::EmoteGifEncoding>) -> usize {
    use crate::model::{EmoteAssetKind, EmoteGifEncoding};
    match kind {
        EmoteAssetKind::Emote => MAX_EMOTE_BYTES,
        EmoteAssetKind::Sticker => MAX_STICKER_BYTES,
        EmoteAssetKind::Gif => match encoding {
            Some(EmoteGifEncoding::Video) => MAX_VIDEO_GIF_BYTES,
            _ => MAX_GIF_BYTES,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_header_required() {
        assert!(classic_gif_duration_ms(b"not a gif").is_err());
    }
}
