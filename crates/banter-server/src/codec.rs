use crate::error::BanterError;
use crate::model::{MediaFrame, MediaKind, MediaPlace};

/// Client to server: version, kind, payload. The server decides the room.
pub fn encode_client_media(kind: MediaKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + payload.len());
    out.push(1);
    out.push(kind.code());
    out.extend_from_slice(payload);
    out
}

pub fn decode_client_media(frame: &[u8]) -> Result<(MediaKind, Vec<u8>), BanterError> {
    if frame.len() < 2 || frame[0] != 1 {
        return Err(BanterError::BadFrame);
    }
    let kind = MediaKind::from_code(frame[1]).ok_or(BanterError::BadFrame)?;
    Ok((kind, frame[2..].to_vec()))
}

/// Server to client: version, kind, place, sender, payload.
pub fn encode_media_frame(kind: MediaKind, place: MediaPlace, sender_id: i64, payload: &[u8]) -> Vec<u8> {
    let (place_code, place_id) = place.code();
    let mut out = Vec::with_capacity(19 + payload.len());
    out.push(1);
    out.push(kind.code());
    out.push(place_code);
    out.extend_from_slice(&place_id.to_le_bytes());
    out.extend_from_slice(&sender_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_media_frame(frame: &[u8]) -> Result<MediaFrame, BanterError> {
    if frame.len() < 19 || frame[0] != 1 {
        return Err(BanterError::BadFrame);
    }
    let kind = MediaKind::from_code(frame[1]).ok_or(BanterError::BadFrame)?;
    let mut place_id = [0_u8; 8];
    place_id.copy_from_slice(&frame[3..11]);
    let mut sender = [0_u8; 8];
    sender.copy_from_slice(&frame[11..19]);
    let place = MediaPlace::from_parts(frame[2], i64::from_le_bytes(place_id)).ok_or(BanterError::BadFrame)?;
    Ok(MediaFrame {
        kind,
        place,
        sender_id: i64::from_le_bytes(sender),
        payload: frame[19..].to_vec(),
    })
}
