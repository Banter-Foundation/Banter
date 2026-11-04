//! Banter-owned profile badges for Discord-parity badge rows.
//!
//! Catalog entries are Banter art descriptors (ids, labels, colors, shapes) —
//! not Discord assets. Nothing is seeded onto accounts; users equip from this list.

use serde::{Deserialize, Serialize};

/// Visual shape hint for client procedural badge icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeShape {
    Circle,
    Hex,
    Shield,
    Gem,
    Leaf,
    Eye,
}

/// One selectable / displayable badge in the Banter catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeItem {
    pub id: String,
    pub label: String,
    pub color: u32,
    pub shape: BadgeShape,
}

/// Full Banter badge catalog shown on profiles.
pub fn badges_catalog() -> Vec<BadgeItem> {
    vec![
        badge("banter-leaf", "Banter Leaf", 0x4caf7a, BadgeShape::Leaf),
        badge("night-eye", "Night Eye", 0xe91e8c, BadgeShape::Eye),
        badge("frost-gem", "Frost Gem", 0x9b59d0, BadgeShape::Gem),
        badge("ember-shield", "Ember Shield", 0xff8a5c, BadgeShape::Shield),
        badge("aurora-hex", "Aurora Hex", 0x5ad1a4, BadgeShape::Hex),
        badge("owl-circle", "Owl Mark", 0xb8d4ff, BadgeShape::Circle),
        badge("hytl-sword", "HYTL", 0x7dffb3, BadgeShape::Shield),
        badge("spark-star", "Spark", 0xf0c14a, BadgeShape::Circle),
    ]
}

fn badge(id: &str, label: &str, color: u32, shape: BadgeShape) -> BadgeItem {
    BadgeItem {
        id: id.to_string(),
        label: label.to_string(),
        color,
        shape,
    }
}

pub fn badge_exists(id: &str) -> bool {
    badges_catalog().iter().any(|item| item.id == id)
}

pub fn badge_item(id: &str) -> Option<BadgeItem> {
    badges_catalog().into_iter().find(|item| item.id == id)
}

pub const MAX_EQUIPPED_BADGES: usize = 12;
