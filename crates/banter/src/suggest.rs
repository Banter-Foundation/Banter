//! Local compose autosuggest: mentions, optional emotes, and n-gram phrase prediction.
//!
//! No cloud LLM. History bodies + a small static phrase list train a lightweight
//! bigram/trigram model for next-word / prefix completions (BA-103).
//! The signed-in user's own messages and composer draft are ingested at higher
//! weights than peers (`WEIGHT_OWN` / `WEIGHT_DRAFT` / `WEIGHT_PEER`).

use std::collections::HashMap;

/// Common short chat openers / closers so an empty channel still has something to try.
const STATIC_PHRASES: &[&str] = &[
    "hello",
    "hello everyone",
    "hey",
    "hi",
    "good morning",
    "good night",
    "thanks",
    "thank you",
    "sounds good",
    "on my way",
    "be right back",
    "brb",
    "gtg",
    "lol",
    "lmao",
    "omg",
    "wait what",
    "i think",
    "i mean",
    "let me",
    "can you",
    "do you",
    "what do you think",
];

const MAX_HISTORY_MESSAGES: usize = 200;
const MAX_NGRAM_KEYS: usize = 4_000;

/// Weight for other people's messages in the open channel/DM.
pub const WEIGHT_PEER: u32 = 1;
/// Weight for the signed-in user's own messages (channel history and session memory).
pub const WEIGHT_OWN: u32 = 4;
/// Weight for the current composer draft (in-progress text).
pub const WEIGHT_DRAFT: u32 = 3;
/// Cap on remembered own bodies from other channels visited this session.
pub const MAX_OWN_MEMORY: usize = 100;

/// Member row the UI already has for the open server / DM peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberHint {
    pub username: String,
    pub display_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestKind {
    Mention,
    Emote,
    Phrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: SuggestKind,
    /// Chip label shown in the UI.
    pub label: String,
    /// Token written into the composer on accept (without the trailing space `apply` adds).
    pub insert: String,
    /// Suffix after the already-typed prefix (ghost text).
    pub ghost: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveToken {
    Mention { start: usize, prefix: String },
    Emote { start: usize, prefix: String },
    Word { start: usize, prefix: String },
}

/// Inputs for one suggest pass. Emote names are optional — empty means no emote chips.
pub struct SuggestQuery<'a> {
    pub composer: &'a str,
    pub members: &'a [MemberHint],
    pub emotes: &'a [String],
    pub limit: usize,
}

/// Escape-dismiss until the draft string changes.
#[derive(Clone, Debug, Default)]
pub struct DismissState {
    dismissed_for: Option<String>,
}

impl DismissState {
    pub fn dismiss(&mut self, composer: &str) {
        self.dismissed_for = Some(composer.to_string());
    }

    pub fn is_dismissed(&self, composer: &str) -> bool {
        self.dismissed_for.as_deref() == Some(composer)
    }

    pub fn clear_if_changed(&mut self, composer: &str) {
        if self.dismissed_for.as_deref() != Some(composer) {
            self.dismissed_for = None;
        }
    }

    pub fn clear(&mut self) {
        self.dismissed_for = None;
    }
}

/// Bigram + trigram counts over lowercased word tokens.
#[derive(Clone, Debug, Default)]
pub struct SuggestionEngine {
    /// word → count
    unigram: HashMap<String, u32>,
    /// "a b" → next-word counts
    bigram: HashMap<String, HashMap<String, u32>>,
    /// "a b c" keyed as "a b" → next counts already in bigram; trigram uses "a b" → word
    trigram: HashMap<String, HashMap<String, u32>>,
    /// Full phrases for prefix match (static + rare multi-word history snippets).
    phrases: Vec<String>,
}

impl SuggestionEngine {
    pub fn new() -> Self {
        let mut engine = Self::default();
        for phrase in STATIC_PHRASES {
            engine.add_phrase(phrase);
            engine.ingest_text(phrase, WEIGHT_PEER);
        }
        engine
    }

    /// Rebuild counts from channel/DM message bodies at peer weight (newest or oldest order both fine).
    pub fn ingest(&mut self, bodies: impl IntoIterator<Item = impl AsRef<str>>) {
        self.ingest_weighted_many(bodies.into_iter().map(|b| (b, WEIGHT_PEER)));
    }

    /// Ingest one body with an explicit weight (own history should use [`WEIGHT_OWN`]).
    pub fn ingest_weighted(&mut self, text: impl AsRef<str>, weight: u32) {
        if weight == 0 {
            return;
        }
        self.ingest_text(text.as_ref(), weight);
    }

    /// Ingest many `(body, weight)` pairs, capped at [`MAX_HISTORY_MESSAGES`] samples.
    pub fn ingest_weighted_many(
        &mut self,
        samples: impl IntoIterator<Item = (impl AsRef<str>, u32)>,
    ) {
        let mut n = 0;
        for (body, weight) in samples {
            if n >= MAX_HISTORY_MESSAGES {
                break;
            }
            if weight == 0 {
                continue;
            }
            self.ingest_text(body.as_ref(), weight);
            n += 1;
        }
    }

    fn add_phrase(&mut self, phrase: &str) {
        let trimmed = phrase.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_ascii_lowercase();
        if !self.phrases.iter().any(|p| p == &lower) {
            self.phrases.push(lower);
        }
    }

    fn ingest_text(&mut self, text: &str, weight: u32) {
        let weight = weight.max(1);
        let tokens = tokenize(text);
        if tokens.is_empty() {
            return;
        }
        if tokens.len() >= 2 && tokens.len() <= 6 {
            self.add_phrase(&tokens.join(" "));
        }
        for w in &tokens {
            *self.unigram.entry(w.clone()).or_insert(0) += weight;
        }
        for i in 0..tokens.len().saturating_sub(1) {
            let key = tokens[i].clone();
            let next = tokens[i + 1].clone();
            *self
                .bigram
                .entry(key)
                .or_default()
                .entry(next)
                .or_insert(0) += weight;
        }
        for i in 0..tokens.len().saturating_sub(2) {
            let key = format!("{} {}", tokens[i], tokens[i + 1]);
            let next = tokens[i + 2].clone();
            *self
                .trigram
                .entry(key)
                .or_default()
                .entry(next)
                .or_insert(0) += weight;
        }
        self.trim_maps();
    }

    fn trim_maps(&mut self) {
        if self.bigram.len() > MAX_NGRAM_KEYS {
            // Drop rare singleton keys; keep it bounded for long channels.
            self.bigram.retain(|_, next| next.values().any(|c| *c > 1));
        }
        if self.trigram.len() > MAX_NGRAM_KEYS {
            self.trigram.retain(|_, next| next.values().any(|c| *c > 1));
        }
    }
}

/// Scan the composer for the token under the caret (end of string for v1).
pub fn active_token(composer: &str) -> ActiveToken {
    let bytes = composer.as_bytes();
    if bytes.is_empty() {
        return ActiveToken::Word {
            start: 0,
            prefix: String::new(),
        };
    }

    // Trailing spaces → empty word token at end (next-word mode).
    if bytes.last() == Some(&b' ') {
        return ActiveToken::Word {
            start: composer.len(),
            prefix: String::new(),
        };
    }

    let start = composer
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let token = &composer[start..];

    if let Some(rest) = token.strip_prefix('@') {
        if !rest.contains('@') && !rest.contains(':') {
            return ActiveToken::Mention {
                start,
                prefix: rest.to_string(),
            };
        }
    }
    if let Some(rest) = token.strip_prefix(':') {
        if !rest.contains(':') {
            return ActiveToken::Emote {
                start,
                prefix: rest.to_string(),
            };
        }
    }
    ActiveToken::Word {
        start,
        prefix: token.to_string(),
    }
}

/// Ranked suggestions for the current composer state.
pub fn suggest(engine: &SuggestionEngine, query: &SuggestQuery<'_>) -> Vec<Suggestion> {
    let limit = query.limit.max(1).min(12);
    match active_token(query.composer) {
        ActiveToken::Mention { prefix, .. } => suggest_mentions(query.members, &prefix, limit),
        ActiveToken::Emote { prefix, .. } => {
            if query.emotes.is_empty() {
                // No emote library hook — fall through to phrase only when `:` has no names.
                // Product choice: keep `:` as emote-intent; return empty rather than phrase noise.
                Vec::new()
            } else {
                suggest_emotes(query.emotes, &prefix, limit)
            }
        }
        ActiveToken::Word { prefix, .. } => suggest_phrases(engine, query.composer, &prefix, limit),
    }
}

fn suggest_mentions(members: &[MemberHint], prefix: &str, limit: usize) -> Vec<Suggestion> {
    let needle = prefix.to_ascii_lowercase();
    let mut hits: Vec<Suggestion> = members
        .iter()
        .filter(|m| {
            m.username.to_ascii_lowercase().starts_with(&needle)
                || m.display_name.to_ascii_lowercase().starts_with(&needle)
        })
        .map(|m| {
            let insert = format!("@{}", m.username);
            let ghost = ghost_after(&insert, &format!("@{prefix}"));
            Suggestion {
                kind: SuggestKind::Mention,
                label: if m.display_name.is_empty() || m.display_name == m.username {
                    format!("@{}", m.username)
                } else {
                    format!("@{} · {}", m.username, m.display_name)
                },
                insert,
                ghost,
            }
        })
        .collect();
    hits.sort_by(|a, b| a.insert.cmp(&b.insert));
    hits.truncate(limit);
    hits
}

fn suggest_emotes(emotes: &[String], prefix: &str, limit: usize) -> Vec<Suggestion> {
    let needle = prefix.to_ascii_lowercase();
    let mut hits: Vec<Suggestion> = emotes
        .iter()
        .filter(|name| name.to_ascii_lowercase().starts_with(&needle))
        .map(|name| {
            let insert = format!(":{name}:");
            let typed = format!(":{prefix}");
            let ghost = ghost_after(&insert, &typed);
            Suggestion {
                kind: SuggestKind::Emote,
                label: insert.clone(),
                insert,
                ghost,
            }
        })
        .collect();
    hits.sort_by(|a, b| a.insert.cmp(&b.insert));
    hits.truncate(limit);
    hits
}

fn suggest_phrases(
    engine: &SuggestionEngine,
    composer: &str,
    prefix: &str,
    limit: usize,
) -> Vec<Suggestion> {
    let mut scored: HashMap<String, (u32, Suggestion)> = HashMap::new();

    let words = tokenize(composer.trim_end());
    let prefix_lower = prefix.to_ascii_lowercase();

    if prefix_lower.is_empty() {
        // Next-word / next-phrase after a completed word.
        if let Some(nexts) = words.last().and_then(|w| engine.bigram.get(w)) {
            for (word, count) in nexts {
                push_phrase(
                    &mut scored,
                    word,
                    word,
                    word,
                    *count * 4,
                    &prefix_lower,
                );
            }
        }
        if words.len() >= 2 {
            let key = format!("{} {}", words[words.len() - 2], words[words.len() - 1]);
            if let Some(nexts) = engine.trigram.get(&key) {
                for (word, count) in nexts {
                    push_phrase(
                        &mut scored,
                        word,
                        word,
                        word,
                        *count * 6,
                        &prefix_lower,
                    );
                }
            }
        }
        // Multi-word static / learned phrases that continue after the trailing context.
        let context = words.join(" ");
        for phrase in &engine.phrases {
            if let Some(rest) = phrase.strip_prefix(&context) {
                let rest = rest.trim_start();
                if rest.is_empty() {
                    continue;
                }
                let first = rest.split_whitespace().next().unwrap_or(rest);
                push_phrase(&mut scored, first, first, first, 3, &prefix_lower);
            }
        }
    } else {
        // Prefix-complete the current word from unigrams + phrase heads.
        for (word, count) in &engine.unigram {
            if word.starts_with(&prefix_lower) && word != &prefix_lower {
                let ghost = word[prefix_lower.len()..].to_string();
                push_phrase(
                    &mut scored,
                    word,
                    word,
                    &ghost,
                    *count * 2,
                    &prefix_lower,
                );
            }
        }
        for phrase in &engine.phrases {
            if phrase.starts_with(&prefix_lower) && phrase != &prefix_lower {
                let ghost = phrase[prefix_lower.len()..].to_string();
                push_phrase(
                    &mut scored,
                    phrase,
                    phrase,
                    &ghost,
                    5,
                    &prefix_lower,
                );
            }
        }
        // Also: last full word + current prefix → next from bigram filtered by prefix.
        if words.len() >= 2 {
            let prev = &words[words.len() - 2];
            if let Some(nexts) = engine.bigram.get(prev) {
                for (word, count) in nexts {
                    if word.starts_with(&prefix_lower) {
                        let ghost = word[prefix_lower.len()..].to_string();
                        push_phrase(
                            &mut scored,
                            word,
                            word,
                            &ghost,
                            *count * 5,
                            &prefix_lower,
                        );
                    }
                }
            }
        }
    }

    let mut hits: Vec<(u32, Suggestion)> = scored.into_values().collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.insert.cmp(&b.1.insert)));
    hits.into_iter()
        .take(limit)
        .map(|(_, s)| s)
        .collect()
}

fn push_phrase(
    scored: &mut HashMap<String, (u32, Suggestion)>,
    key: &str,
    insert: &str,
    ghost: &str,
    weight: u32,
    _prefix: &str,
) {
    let suggestion = Suggestion {
        kind: SuggestKind::Phrase,
        label: insert.to_string(),
        insert: insert.to_string(),
        ghost: ghost.to_string(),
    };
    scored
        .entry(key.to_string())
        .and_modify(|(w, s)| {
            *w += weight;
            if suggestion.ghost.len() < s.ghost.len() {
                *s = suggestion.clone();
            }
        })
        .or_insert((weight, suggestion));
}

fn ghost_after(full: &str, typed: &str) -> String {
    let full_l = full.to_ascii_lowercase();
    let typed_l = typed.to_ascii_lowercase();
    if full_l.starts_with(&typed_l) {
        full[typed.len().min(full.len())..].to_string()
    } else {
        full.to_string()
    }
}

/// Replace the active token (or append a next word) and ensure a trailing space.
pub fn apply_suggestion(composer: &mut String, suggestion: &Suggestion) {
    let token = active_token(composer);
    let start = match &token {
        ActiveToken::Mention { start, .. }
        | ActiveToken::Emote { start, .. }
        | ActiveToken::Word { start, .. } => *start,
    };
    composer.truncate(start);
    composer.push_str(&suggestion.insert);
    if !composer.ends_with(' ') {
        composer.push(' ');
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '\'' || c == '_' {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn tokenize_splits_words() {
        assert_eq!(tokenize("Sounds Good!"), vec!["sounds", "good"]);
    }
}
