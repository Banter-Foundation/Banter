//! Contract tests for compose autosuggest / local n-gram prediction (BA-103).
//!
//! These tests lock the predictor API before the UI wiring. Do not weaken them.

use banter::suggest::{
    apply_suggestion, active_token, suggest, ActiveToken, MemberHint, SuggestKind, SuggestQuery,
    SuggestionEngine, WEIGHT_DRAFT, WEIGHT_OWN, WEIGHT_PEER,
};

fn engine_with(bodies: &[&str]) -> SuggestionEngine {
    let mut engine = SuggestionEngine::new();
    engine.ingest(bodies.iter().copied());
    engine
}

#[test]
fn mention_prefix_filters_members() {
    let engine = SuggestionEngine::new();
    let members = [
        MemberHint {
            username: "alice".into(),
            display_name: "Alice".into(),
        },
        MemberHint {
            username: "bob".into(),
            display_name: "Bobby".into(),
        },
        MemberHint {
            username: "alicia".into(),
            display_name: "Alicia".into(),
        },
    ];
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "hey @ali",
            members: &members,
            emotes: &[],
            limit: 8,
        },
    );
    assert!(hits.iter().all(|s| s.kind == SuggestKind::Mention));
    let names: Vec<_> = hits.iter().map(|s| s.insert.as_str()).collect();
    assert!(names.contains(&"@alice"));
    assert!(names.contains(&"@alicia"));
    assert!(!names.iter().any(|n| *n == "@bob"));
}

#[test]
fn emote_suggestions_only_when_names_provided() {
    let engine = SuggestionEngine::new();
    let empty = suggest(
        &engine,
        &SuggestQuery {
            composer: "nice :par",
            members: &[],
            emotes: &[],
            limit: 8,
        },
    );
    assert!(
        empty.iter().all(|s| s.kind != SuggestKind::Emote),
        "no emote hook → no emote chips"
    );

    let emotes = ["party_parrot".into(), "wave".into(), "partycat".into()];
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "nice :par",
            members: &[],
            emotes: &emotes,
            limit: 8,
        },
    );
    assert!(hits.iter().all(|s| s.kind == SuggestKind::Emote));
    let inserts: Vec<_> = hits.iter().map(|s| s.insert.as_str()).collect();
    assert!(inserts.contains(&":party_parrot:"));
    assert!(inserts.contains(&":partycat:"));
    assert!(!inserts.iter().any(|n| *n == ":wave:"));
}

#[test]
fn ngram_next_word_from_history() {
    let engine = engine_with(&[
        "that sounds good to me",
        "sounds good thanks",
        "it sounds good",
    ]);
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "sounds ",
            members: &[],
            emotes: &[],
            limit: 8,
        },
    );
    assert!(
        hits.iter().any(|s| {
            s.kind == SuggestKind::Phrase
                && (s.insert == "good" || s.insert.starts_with("good "))
        }),
        "expected next-word 'good' from history, got {hits:?}"
    );
}

#[test]
fn static_phrase_prefix_completion() {
    let engine = SuggestionEngine::new();
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "thank",
            members: &[],
            emotes: &[],
            limit: 8,
        },
    );
    assert!(
        hits.iter().any(|s| {
            s.kind == SuggestKind::Phrase
                && (s.insert.starts_with("thank") || s.label.to_lowercase().contains("thank"))
        }),
        "static phrases should seed empty history, got {hits:?}"
    );
}

#[test]
fn apply_mention_replaces_partial_token() {
    let mut composer = String::from("hi @ali");
    let suggestion = banter::suggest::Suggestion {
        kind: SuggestKind::Mention,
        label: "alice".into(),
        insert: "@alice".into(),
        ghost: "ce".into(),
    };
    apply_suggestion(&mut composer, &suggestion);
    assert_eq!(composer, "hi @alice ");
}

#[test]
fn apply_phrase_completes_partial_word() {
    let mut composer = String::from("hel");
    let suggestion = banter::suggest::Suggestion {
        kind: SuggestKind::Phrase,
        label: "hello".into(),
        insert: "hello".into(),
        ghost: "lo".into(),
    };
    apply_suggestion(&mut composer, &suggestion);
    assert_eq!(composer, "hello ");
}

#[test]
fn apply_phrase_appends_next_word_after_space() {
    let mut composer = String::from("sounds ");
    let suggestion = banter::suggest::Suggestion {
        kind: SuggestKind::Phrase,
        label: "good".into(),
        insert: "good".into(),
        ghost: "good".into(),
    };
    apply_suggestion(&mut composer, &suggestion);
    assert_eq!(composer, "sounds good ");
}

#[test]
fn apply_emote_closes_shortcode() {
    let mut composer = String::from("lol :par");
    let suggestion = banter::suggest::Suggestion {
        kind: SuggestKind::Emote,
        label: ":party_parrot:".into(),
        insert: ":party_parrot:".into(),
        ghost: "ty_parrot:".into(),
    };
    apply_suggestion(&mut composer, &suggestion);
    assert_eq!(composer, "lol :party_parrot: ");
}

#[test]
fn active_token_detects_mention_emote_and_word() {
    assert_eq!(
        active_token("hello @al"),
        ActiveToken::Mention {
            start: 6,
            prefix: "al".into(),
        }
    );
    assert_eq!(
        active_token("hi :wav"),
        ActiveToken::Emote {
            start: 3,
            prefix: "wav".into(),
        }
    );
    assert_eq!(
        active_token("sounds "),
        ActiveToken::Word {
            start: 7,
            prefix: String::new(),
        }
    );
    assert_eq!(
        active_token("hel"),
        ActiveToken::Word {
            start: 0,
            prefix: "hel".into(),
        }
    );
}

#[test]
fn dismiss_stays_until_composer_changes() {
    use banter::suggest::DismissState;
    let mut dismiss = DismissState::default();
    dismiss.dismiss("thank");
    assert!(dismiss.is_dismissed("thank"));
    assert!(!dismiss.is_dismissed("thanks"));
    dismiss.clear_if_changed("thanks");
    assert!(!dismiss.is_dismissed("thanks"));
}

#[test]
fn own_messages_outweigh_peers_for_next_word() {
    let mut engine = SuggestionEngine::new();
    // Peers repeat "sounds fine" more times than the user said "sounds good",
    // but OWN weight must still prefer the user's phrasing.
    for _ in 0..3 {
        engine.ingest_weighted("sounds fine thanks", WEIGHT_PEER);
    }
    engine.ingest_weighted("sounds good thanks", WEIGHT_OWN);
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "sounds ",
            members: &[],
            emotes: &[],
            limit: 8,
        },
    );
    let good = hits.iter().position(|s| s.insert == "good");
    let fine = hits.iter().position(|s| s.insert == "fine");
    assert!(
        good.is_some(),
        "expected own next-word 'good' in {hits:?}"
    );
    assert!(
        fine.map(|i| good.unwrap() < i).unwrap_or(true),
        "own 'good' should rank above peer 'fine', got {hits:?}"
    );
}

#[test]
fn draft_text_trains_next_word() {
    let mut engine = SuggestionEngine::new();
    engine.ingest_weighted("ship it tonight", WEIGHT_DRAFT);
    let hits = suggest(
        &engine,
        &SuggestQuery {
            composer: "ship ",
            members: &[],
            emotes: &[],
            limit: 8,
        },
    );
    assert!(
        hits.iter().any(|s| s.kind == SuggestKind::Phrase && s.insert == "it"),
        "draft should teach next-word 'it', got {hits:?}"
    );
}
