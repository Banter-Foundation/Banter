//! Embedded Banter UI sound assets decode and map to sound events.

use banter::{decode_all_assets, SoundEvent, RINGTONE_ASSET};

#[test]
fn sound_event_files_match_adam_audio_pack() {
    assert_eq!(SoundEvent::Join.asset_file(), "join_call.ogg");
    assert_eq!(SoundEvent::Leave.asset_file(), "leave_call.ogg");
    assert_eq!(SoundEvent::Mute.asset_file(), "mute.ogg");
    assert_eq!(SoundEvent::Unmute.asset_file(), "unmute.ogg");
    assert_eq!(SoundEvent::StartStream.asset_file(), "start_stream.ogg");
    assert_eq!(SoundEvent::StopStream.asset_file(), "stop_stream.ogg");
    assert_eq!(RINGTONE_ASSET, "BanterRingtone.ogg");
}

#[test]
fn every_sfx_asset_decodes_to_pcm() {
    let clips = decode_all_assets().expect("decode embedded sfx");
    assert_eq!(clips.len(), 7);
    for (name, rate, frames) in clips {
        assert!(rate >= 8_000, "{name} sample rate");
        assert!(frames > 100, "{name} should have audible length");
    }
}
