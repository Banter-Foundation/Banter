//! Built-in Banter cosmetics for Discord-parity profile customization.
//!
//! These are Banter-owned catalog entries (ids, labels, colors). They are not
//! Discord assets. Users pick from this list; nothing is seeded onto accounts.

use serde::{Deserialize, Serialize};

/// Which editor slot a cosmetic fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticKind {
    Nameplate,
    AvatarDecoration,
    DisplayNameStyle,
    ProfileEffect,
    ProfileFrame,
}

/// One selectable cosmetic in the catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CosmeticItem {
    pub id: String,
    pub kind: CosmeticKind,
    pub label: String,
    /// Primary accent as 0xRRGGBB.
    pub color: u32,
}

/// Full Banter cosmetics catalog for the profile editor.
pub fn cosmetics_catalog() -> Vec<CosmeticItem> {
    vec![
        item("forest-mist", CosmeticKind::Nameplate, "Forest Mist", 0x2d4a3e),
        item("midnight-ember", CosmeticKind::Nameplate, "Midnight Ember", 0x3a2218),
        item("arctic-glow", CosmeticKind::Nameplate, "Arctic Glow", 0x1a3344),
        item("leaf-ring", CosmeticKind::AvatarDecoration, "Leaf Ring", 0x4caf7a),
        item("spark-orbit", CosmeticKind::AvatarDecoration, "Spark Orbit", 0xf0c14a),
        item("soft-halo", CosmeticKind::AvatarDecoration, "Soft Halo", 0xb8d4ff),
        item("mint-glow", CosmeticKind::DisplayNameStyle, "Mint Glow", 0x7dffb3),
        item("ember-fade", CosmeticKind::DisplayNameStyle, "Ember Fade", 0xff8a5c),
        item("ice-sheen", CosmeticKind::DisplayNameStyle, "Ice Sheen", 0x9ed7ff),
        item("aurora-wash", CosmeticKind::ProfileEffect, "Aurora Wash", 0x5ad1a4),
        item("soft-particles", CosmeticKind::ProfileEffect, "Soft Particles", 0xc9b6ff),
        item("thick-jade", CosmeticKind::ProfileFrame, "Thick Jade", 0x2f6b4f),
        item("thin-gold", CosmeticKind::ProfileFrame, "Thin Gold", 0xd4b45a),
    ]
}

fn item(id: &str, kind: CosmeticKind, label: &str, color: u32) -> CosmeticItem {
    CosmeticItem {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        color,
    }
}

/// Default profile theme (deep charcoal + forest accent), Banter-owned.
pub const DEFAULT_THEME_PRIMARY: u32 = 0x2d4a3e;
pub const DEFAULT_THEME_SECONDARY: u32 = 0x111214;

pub fn cosmetic_exists(kind: CosmeticKind, id: &str) -> bool {
    cosmetics_catalog()
        .iter()
        .any(|item| item.kind == kind && item.id == id)
}

pub fn display_name_style_color(id: Option<&str>) -> Option<u32> {
    let id = id?;
    cosmetics_catalog()
        .into_iter()
        .find(|item| item.kind == CosmeticKind::DisplayNameStyle && item.id == id)
        .map(|item| item.color)
}
