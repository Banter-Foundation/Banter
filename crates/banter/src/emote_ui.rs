//! Composer Emote Library picker helpers (BA-83).

use banter_server::{EmoteAsset, EmoteAssetKind, EmotePickerSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmotePickerTab {
    Emote,
    Sticker,
    Gif,
}

impl EmotePickerTab {
    pub fn kind(self) -> EmoteAssetKind {
        match self {
            Self::Emote => EmoteAssetKind::Emote,
            Self::Sticker => EmoteAssetKind::Sticker,
            Self::Gif => EmoteAssetKind::Gif,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Emote => "Emotes",
            Self::Sticker => "Stickers",
            Self::Gif => "GIFs",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EmotePickerState {
    pub open: Option<EmotePickerTab>,
    pub source: EmotePickerSourceState,
    pub query: String,
    pub items: Vec<EmoteAsset>,
    pub error: String,
    /// Sticker (Model A, at most one) and/or GIF assets waiting to send with the next message.
    pub pending: Vec<EmoteAsset>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmotePickerSourceState {
    Server,
    Subscribed,
}

impl Default for EmotePickerSourceState {
    fn default() -> Self {
        Self::Server
    }
}

impl EmotePickerSourceState {
    pub fn api(self) -> EmotePickerSource {
        match self {
            Self::Server => EmotePickerSource::Server,
            Self::Subscribed => EmotePickerSource::Subscribed,
        }
    }

}

impl EmotePickerState {
    pub fn queue_media(&mut self, asset: EmoteAsset) {
        match asset.kind {
            EmoteAssetKind::Emote => {}
            EmoteAssetKind::Sticker => {
                // Model A: one sticker; clear GIFs so the message stays sticker-primary.
                self.pending.clear();
                self.pending.push(asset);
            }
            EmoteAssetKind::Gif => {
                self.pending.retain(|a| a.kind != EmoteAssetKind::Sticker);
                if !self.pending.iter().any(|a| a.id == asset.id) {
                    self.pending.push(asset);
                }
            }
        }
    }

    pub fn pending_ids(&self) -> Vec<i64> {
        self.pending.iter().map(|a| a.id).collect()
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }
}
