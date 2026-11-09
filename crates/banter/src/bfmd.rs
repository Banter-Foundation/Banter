//! BFMD (Better Formatted Markdown) — Banter chat text formatting.
//!
//! Spec: project store `docs/bfmd.md` (Sickle-adapted). This is a style-command
//! language, not CommonMark. Messages store BFMD source in `body`; plain text
//! with no command groups is valid BFMD and round-trips unchanged.

use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveTime, TimeZone, Timelike};

/// Default body size in points when `{s}` is unset (theme body).
pub const DEFAULT_SIZE_PT: f32 = 16.0;

/// Default inline emote height in CSS px when `{e}` is unset (Carbon parity).
pub const DEFAULT_EMOTE_PX: f32 = 22.0;

/// Inclusive clamp for `{e}` emote height (plan: 16–128).
pub const EMOTE_PX_MIN: f32 = 16.0;
pub const EMOTE_PX_MAX: f32 = 128.0;

/// Discord-compatible timestamp display style (`t` / `T` / `d` / `D` / `f` / `F` / `R`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TimestampStyle {
    /// `t` — short time
    ShortTime,
    /// `T` — long time
    LongTime,
    /// `d` — short date
    ShortDate,
    /// `D` — long date
    LongDate,
    /// `f` — short date + short time (Discord default)
    #[default]
    ShortDateTime,
    /// `F` — full date + short time
    LongDateTime,
    /// `R` — relative to now
    Relative,
}

impl TimestampStyle {
    pub fn from_flag(c: char) -> Option<Self> {
        match c {
            't' => Some(Self::ShortTime),
            'T' => Some(Self::LongTime),
            'd' => Some(Self::ShortDate),
            'D' => Some(Self::LongDate),
            'f' => Some(Self::ShortDateTime),
            'F' => Some(Self::LongDateTime),
            'R' => Some(Self::Relative),
            _ => None,
        }
    }

    pub fn flag(self) -> char {
        match self {
            Self::ShortTime => 't',
            Self::LongTime => 'T',
            Self::ShortDate => 'd',
            Self::LongDate => 'D',
            Self::ShortDateTime => 'f',
            Self::LongDateTime => 'F',
            Self::Relative => 'R',
        }
    }
}

/// Inline timestamp object (absolute instant + display style). Not a paint slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub unix_secs: i64,
    pub style: TimestampStyle,
}

/// One styled text span produced by the parser.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// When set, this run is an inline timestamp object. `text` is empty; paint
    /// formats via [`format_timestamp`]. Canonical serialize is `<t:unix:style>`.
    pub timestamp: Option<Timestamp>,
}

/// Active style map for a run (empty slots = document / theme defaults).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub size: Option<f32>,
    /// Inline emote height in px (`{e}`); independent from text `{s}`.
    pub emote_size: Option<f32>,
    pub color: Option<Rgba>,
    pub background: Option<Rgba>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub font: Option<String>,
    pub align: Option<Align>,
    pub heading: Option<u8>,
    pub hr: bool,
}

impl Style {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn size_or_default(&self) -> f32 {
        self.size.unwrap_or(DEFAULT_SIZE_PT)
    }

    pub fn emote_size_or_default(&self) -> f32 {
        self.emote_size.unwrap_or(DEFAULT_EMOTE_PX)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_imgui(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseResult {
    pub runs: Vec<Run>,
    pub diagnostics: Vec<Diagnostic>,
    pub version: u32,
}

/// Parse BFMD source into styled runs. Never refuses; recovers per §8.
pub fn parse(source: &str) -> ParseResult {
    let source = strip_bom(source);
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut style = Style::default();
    let mut runs: Vec<Run> = Vec::new();
    let mut text = String::new();
    let mut diagnostics = Vec::new();
    let mut version = 1u32;
    let mut saw_version = false;
    let mut first_group = true;

    let flush = |text: &mut String, style: &Style, runs: &mut Vec<Run>| {
        if text.is_empty() {
            return;
        }
        runs.push(Run {
            text: std::mem::take(text),
            style: style.clone(),
            timestamp: None,
        });
    };

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            let (ch, next, diag) = parse_text_escape(bytes, i);
            if let Some(d) = diag {
                diagnostics.push(d);
            }
            text.push(ch);
            i = next;
            continue;
        }
        if c == b'<' && group_starts_at(bytes, i) {
            flush(&mut text, &style, &mut runs);
            match parse_group(bytes, i, &mut style, &mut diagnostics) {
                Ok((next, cmds)) => {
                    if first_group {
                        for cmd in &cmds {
                            if let ParsedCommand::Set { name, value } = cmd {
                                if name == "v" {
                                    if let Some(Value::Number(n)) = value {
                                        version = n.floor() as u32;
                                        if version != 1 {
                                            diagnostics.push(Diagnostic {
                                                message: format!("unknown BFMD version {version}"),
                                                index: i,
                                            });
                                        }
                                        saw_version = true;
                                    }
                                }
                            }
                        }
                        let _ = saw_version;
                        first_group = false;
                    } else {
                        first_group = false;
                    }
                    apply_commands(&cmds, &mut style, &mut diagnostics, i);
                    i = next;
                }
                Err(next) => {
                    // Unclosed group: treat from `<` as literal (§8).
                    diagnostics.push(Diagnostic {
                        message: "unclosed BFMD group".into(),
                        index: i,
                    });
                    text.push('<');
                    i = i + 1;
                    let _ = next;
                }
            }
            continue;
        }
        // Discord `<t:unix[:style]>` — only when `<` is not a BFMD `{` group.
        if c == b'<' {
            if let Some((next, ts)) = try_parse_discord_timestamp(bytes, i) {
                flush(&mut text, &style, &mut runs);
                runs.push(Run {
                    text: String::new(),
                    style: style.clone(),
                    timestamp: Some(ts),
                });
                i = next;
                first_group = false;
                continue;
            }
        }
        // Easy plain-text `@t[...]` (Owl): clear marker, no ordinary-sentence hits.
        if c == b'@' {
            if let Some((next, ts)) = try_parse_easy_timestamp(bytes, i) {
                flush(&mut text, &style, &mut runs);
                runs.push(Run {
                    text: String::new(),
                    style: style.clone(),
                    timestamp: Some(ts),
                });
                i = next;
                first_group = false;
                continue;
            }
        }
        // Literal `<` or any other byte as UTF-8 char.
        let ch = source[i..].chars().next().unwrap();
        text.push(ch);
        i += ch.len_utf8();
        first_group = false;
    }
    flush(&mut text, &style, &mut runs);
    ParseResult {
        runs,
        diagnostics,
        version,
    }
}

/// Plain visible text (commands stripped). Timestamps become formatted display.
pub fn plain_text(source: &str) -> String {
    let now = Local::now().timestamp();
    parse(source)
        .runs
        .into_iter()
        .map(|r| {
            if let Some(ts) = r.timestamp {
                format_timestamp(&ts, now)
            } else {
                r.text
            }
        })
        .collect()
}

/// Render a timestamp for the reader's locale/timezone.
///
/// Absolute styles ignore `now_unix_secs`. Relative (`R`) uses it.
pub fn format_timestamp(ts: &Timestamp, now_unix_secs: i64) -> String {
    if ts.style == TimestampStyle::Relative {
        return format_relative(ts.unix_secs, now_unix_secs);
    }
    let Some(utc) = DateTime::from_timestamp(ts.unix_secs, 0) else {
        return format!("<t:{}:{}>", ts.unix_secs, ts.style.flag());
    };
    let local = utc.with_timezone(&Local);
    match ts.style {
        TimestampStyle::ShortTime => local.format("%I:%M %p").to_string(),
        TimestampStyle::LongTime => local.format("%I:%M:%S %p").to_string(),
        TimestampStyle::ShortDate => local.format("%m/%d/%Y").to_string(),
        TimestampStyle::LongDate => local.format("%B %d, %Y").to_string(),
        TimestampStyle::ShortDateTime => local.format("%B %d, %Y %I:%M %p").to_string(),
        TimestampStyle::LongDateTime => local.format("%A, %B %d, %Y %I:%M %p").to_string(),
        TimestampStyle::Relative => unreachable!(),
    }
}

fn format_relative(unix_secs: i64, now_unix_secs: i64) -> String {
    let delta = unix_secs.saturating_sub(now_unix_secs);
    let past = delta < 0;
    let secs = delta.unsigned_abs();
    let (n, unit) = if secs < 45 {
        return if past {
            "a few seconds ago".into()
        } else if secs < 5 {
            "now".into()
        } else {
            "in a few seconds".into()
        };
    } else if secs < 90 {
        (1u64, "minute")
    } else if secs < 45 * 60 {
        (secs / 60, "minute")
    } else if secs < 90 * 60 {
        (1, "hour")
    } else if secs < 36 * 3600 {
        (secs / 3600, "hour")
    } else if secs < 60 * 86400 {
        // up to ~2 months use days
        let days = secs / 86400;
        if days < 2 {
            (1, "day")
        } else {
            (days, "day")
        }
    } else if secs < 365 * 86400 {
        let months = secs / (30 * 86400);
        if months < 2 {
            (1, "month")
        } else {
            (months.max(1), "month")
        }
    } else {
        let years = secs / (365 * 86400);
        if years < 2 {
            (1, "year")
        } else {
            (years, "year")
        }
    };
    let noun = if n == 1 {
        unit.to_string()
    } else {
        format!("{unit}s")
    };
    let amount = if n == 1 {
        format!("a {noun}")
    } else {
        format!("{n} {noun}")
    };
    if past {
        format!("{amount} ago")
    } else {
        format!("in {amount}")
    }
}

/// Find stable emote tokens `:{e:ASSET_ID}:` in plain/BFMD text.
///
/// Returns `(byte_start, byte_len, asset_id)` in source order. Malformed or
/// name-only shortcodes are ignored (they stay literal text).
pub fn scan_emote_tokens(text: &str) -> Vec<(usize, usize, i64)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i + 5 < bytes.len() {
        if bytes[i] == b':' && bytes.get(i + 1) == Some(&b'{') && bytes.get(i + 2) == Some(&b'e')
            && bytes.get(i + 3) == Some(&b':')
        {
            let mut j = i + 4;
            let mut id: i64 = 0;
            let mut digits = 0usize;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                id = id
                    .saturating_mul(10)
                    .saturating_add((bytes[j] - b'0') as i64);
                digits += 1;
                j += 1;
            }
            if digits > 0 && bytes.get(j) == Some(&b'}') && bytes.get(j + 1) == Some(&b':') {
                let len = (j + 2) - i;
                out.push((i, len, id));
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Format a stored emote token for message body / composer insert.
pub fn emote_token(asset_id: i64) -> String {
    format!(":{{e:{asset_id}}}:")
}

/// Serialize runs to canonical BFMD (§9). Empty / default-only → plain text.
/// Timestamp runs always emit Discord `<t:unix:style>` (easy `@t[...]` normalizes).
pub fn serialize(runs: &[Run]) -> String {
    let mut out = String::new();
    let mut current = Style::default();
    for run in runs {
        if run.text.is_empty() && run.timestamp.is_none() {
            continue;
        }
        if run.style != current {
            out.push_str(&diff_group(&current, &run.style));
            current = run.style.clone();
        }
        if let Some(ts) = run.timestamp {
            out.push_str(&format!("<t:{}:{}>", ts.unix_secs, ts.style.flag()));
        } else {
            escape_text_into(&run.text, &mut out);
        }
    }
    out
}

/// Apply or clear a boolean flag over a byte range of visible plain text mapped
/// onto BFMD source. Operates on the run model then re-serializes.
pub fn apply_flag(
    source: &str,
    plain_start: usize,
    plain_end: usize,
    flag: Flag,
    on: bool,
) -> String {
    let parsed = parse(source);
    let mut runs = parsed.runs;
    if runs.is_empty() {
        return source.to_string();
    }
    let (start, end) = ordered_range(plain_start, plain_end, plain_len(&runs));
    if start == end && !on {
        return serialize(&runs);
    }
    runs = split_runs_at_plain(&runs, start);
    runs = split_runs_at_plain(&runs, end);
    let mut offset = 0usize;
    for run in &mut runs {
        let len = run_plain_len(run);
        let run_start = offset;
        let run_end = offset + len;
        offset = run_end;
        if run_end <= start || run_start >= end {
            continue;
        }
        set_flag(&mut run.style, flag, on);
    }
    // Compact adjacent runs with identical style.
    serialize(&compact_runs(runs))
}

/// Toggle a flag over the selected plain range. If the range is empty, toggles
/// the style that would apply at the caret (inserts an empty styled run marker
/// by wrapping nothing — instead flips trailing open state via a zero-width
/// change at the caret for subsequent typing helpers).
pub fn toggle_flag(source: &str, plain_start: usize, plain_end: usize, flag: Flag) -> String {
    let parsed = parse(source);
    let runs = &parsed.runs;
    let (start, end) = ordered_range(plain_start, plain_end, plain_len(runs));
    if start < end {
        let on = !range_all_flag(runs, start, end, flag);
        return apply_flag(source, start, end, flag, on);
    }
    // Empty selection: toggle open style from caret to end of buffer (Discord-ish
    // "following text" when the rest of the draft is one run after caret).
    let total = plain_len(runs);
    if total == 0 {
        // Empty composer: seed a zero-length concept by returning a group that
        // will style the next typed characters once the UI appends text. We emit
        // an open group only; writers may leave it open (EOF closes).
        return open_flag_prefix(flag, true);
    }
    let on = !flag_at(
        runs,
        start.saturating_sub(1).min(total.saturating_sub(1)),
        flag,
    );
    apply_flag(source, start, total, flag, on)
}

/// Prefix emitted for an empty draft when a style button is pressed.
pub fn open_flag_prefix(flag: Flag, on: bool) -> String {
    if !on {
        return String::new();
    }
    match flag {
        Flag::Bold => "<{b}>".into(),
        Flag::Italic => "<{i}>".into(),
        Flag::Underline => "<{u}>".into(),
        Flag::Strike => "<{k}>".into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Bold,
    Italic,
    Underline,
    Strike,
}

fn set_flag(style: &mut Style, flag: Flag, on: bool) {
    match flag {
        Flag::Bold => style.bold = on,
        Flag::Italic => style.italic = on,
        Flag::Underline => style.underline = on,
        Flag::Strike => style.strike = on,
    }
}

fn flag_value(style: &Style, flag: Flag) -> bool {
    match flag {
        Flag::Bold => style.bold,
        Flag::Italic => style.italic,
        Flag::Underline => style.underline,
        Flag::Strike => style.strike,
    }
}

fn ordered_range(a: usize, b: usize, max: usize) -> (usize, usize) {
    let a = a.min(max);
    let b = b.min(max);
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn range_all_flag(runs: &[Run], start: usize, end: usize, flag: Flag) -> bool {
    let mut offset = 0usize;
    let mut any = false;
    for run in runs {
        let len = run_plain_len(run);
        let run_start = offset;
        let run_end = offset + len;
        offset = run_end;
        if run_end <= start || run_start >= end || len == 0 {
            continue;
        }
        any = true;
        if !flag_value(&run.style, flag) {
            return false;
        }
    }
    any
}

fn flag_at(runs: &[Run], plain_index: usize, flag: Flag) -> bool {
    let mut offset = 0usize;
    for run in runs {
        let len = run_plain_len(run);
        if plain_index < offset + len {
            return flag_value(&run.style, flag);
        }
        offset += len;
    }
    runs.last()
        .map(|r| flag_value(&r.style, flag))
        .unwrap_or(false)
}

fn split_runs_at_plain(runs: &[Run], at: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for run in runs {
        let len = run_plain_len(run);
        let run_start = offset;
        let run_end = offset + len;
        // Timestamp objects are atomic — never split mid-token.
        if run.timestamp.is_none() && at > run_start && at < run_end {
            let local = at - run_start;
            out.push(Run {
                text: run.text[..local].to_string(),
                style: run.style.clone(),
                timestamp: None,
            });
            out.push(Run {
                text: run.text[local..].to_string(),
                style: run.style.clone(),
                timestamp: None,
            });
        } else {
            out.push(run.clone());
        }
        offset = run_end;
    }
    out
}

fn compact_runs(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for run in runs {
        if run.text.is_empty() && run.timestamp.is_none() {
            continue;
        }
        // Timestamp objects never merge with neighbors.
        if run.timestamp.is_some() {
            out.push(run);
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.timestamp.is_none() && last.style == run.style {
                last.text.push_str(&run.text);
                continue;
            }
        }
        out.push(run);
    }
    out
}

fn run_plain_len(run: &Run) -> usize {
    if let Some(ts) = run.timestamp {
        // Match `plain_text` display length (absolute/relative at "now").
        format_timestamp(&ts, Local::now().timestamp()).len()
    } else {
        run.text.len()
    }
}

fn plain_len(runs: &[Run]) -> usize {
    runs.iter().map(run_plain_len).sum()
}

fn strip_bom(source: &str) -> &str {
    source.strip_prefix('\u{FEFF}').unwrap_or(source)
}

fn group_starts_at(bytes: &[u8], i: usize) -> bool {
    if bytes.get(i) != Some(&b'<') {
        return false;
    }
    let mut j = i + 1;
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    bytes.get(j) == Some(&b'{')
}

/// Discord `<t:UNIX>` / `<t:UNIX:STYLE>`. STYLE ∈ t T d D f F R. Default style `f`.
fn try_parse_discord_timestamp(bytes: &[u8], i: usize) -> Option<(usize, Timestamp)> {
    if bytes.get(i) != Some(&b'<')
        || bytes.get(i + 1) != Some(&b't')
        || bytes.get(i + 2) != Some(&b':')
    {
        return None;
    }
    let mut j = i + 3;
    let neg = if bytes.get(j) == Some(&b'-') {
        j += 1;
        true
    } else {
        false
    };
    if j >= bytes.len() || !bytes[j].is_ascii_digit() {
        return None;
    }
    let mut unix: i64 = 0;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        unix = unix
            .checked_mul(10)?
            .checked_add((bytes[j] - b'0') as i64)?;
        j += 1;
    }
    if neg {
        unix = -unix;
    }
    let style = if bytes.get(j) == Some(&b':') {
        j += 1;
        let flag = *bytes.get(j)? as char;
        j += 1;
        TimestampStyle::from_flag(flag)?
    } else {
        TimestampStyle::ShortDateTime
    };
    if bytes.get(j) != Some(&b'>') {
        return None;
    }
    Some((
        j + 1,
        Timestamp {
            unix_secs: unix,
            style,
        },
    ))
}

/// Easy plain-text `@t[datetime]` (Owl). See `docs/bfmd.md` §6.2.
///
/// Marker is exactly `@t[` (no space). Invalid interiors stay literal.
fn try_parse_easy_timestamp(bytes: &[u8], i: usize) -> Option<(usize, Timestamp)> {
    if bytes.get(i) != Some(&b'@')
        || bytes.get(i + 1) != Some(&b't')
        || bytes.get(i + 2) != Some(&b'[')
    {
        return None;
    }
    let mut j = i + 3;
    let start = j;
    while j < bytes.len() && bytes[j] != b']' {
        j += 1;
    }
    if j >= bytes.len() {
        return None; // unclosed — leave `@t[` as literal progressively
    }
    let inner = std::str::from_utf8(&bytes[start..j]).ok()?;
    let ts = parse_easy_timestamp_inner(inner)?;
    Some((j + 1, ts))
}

fn parse_easy_timestamp_inner(inner: &str) -> Option<Timestamp> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    // Optional trailing style flag: last whitespace-separated single known letter.
    let (datetime_part, style) = {
        let mut parts = inner.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 2 {
            let last = *parts.last().unwrap();
            if last.len() == 1 {
                if let Some(style) = TimestampStyle::from_flag(last.chars().next().unwrap()) {
                    parts.pop();
                    (parts.join(" "), style)
                } else {
                    (inner.to_string(), TimestampStyle::ShortDateTime)
                }
            } else {
                (inner.to_string(), TimestampStyle::ShortDateTime)
            }
        } else {
            (inner.to_string(), TimestampStyle::ShortDateTime)
        }
    };
    let unix = parse_easy_datetime(&datetime_part)?;
    Some(Timestamp {
        unix_secs: unix,
        style,
    })
}

fn parse_easy_datetime(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Split date and rest. Accept `YYYY-MM-DDTHH:MM...` or `YYYY-MM-DD HH:MM...`.
    let (date_str, time_and_tz) = if let Some(pos) = raw.find('T') {
        (&raw[..pos], &raw[pos + 1..])
    } else {
        let mut sp = raw.splitn(2, char::is_whitespace);
        let d = sp.next()?;
        let rest = sp.next()?;
        (d, rest.trim())
    };
    let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;

    // Offset: trailing Z / +HH:MM / -HH:MM / +HHMM / -HHMM
    let (time_part, offset) = split_time_and_offset(time_and_tz)?;
    let (naive_time, _had_ampm) = parse_easy_time(&time_part)?;
    let naive = date.and_time(naive_time);

    let unix = if let Some(off) = offset {
        off.from_local_datetime(&naive).single()?.timestamp()
    } else {
        // No zone → writer's local timezone at parse time.
        Local.from_local_datetime(&naive).single()?.timestamp()
    };
    Some(unix)
}

fn split_time_and_offset(s: &str) -> Option<(String, Option<FixedOffset>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Zulu
    if let Some(rest) = s.strip_suffix('Z').or_else(|| s.strip_suffix('z')) {
        return Some((rest.trim().to_string(), Some(FixedOffset::east_opt(0)?)));
    }
    // Find last + or - that looks like a timezone (not part of time)
    // Time looks like `HH:MM` or `H:MM AM`; offset at end: `+00:00` / `-0530`
    for (idx, ch) in s.char_indices().rev() {
        if ch != '+' && ch != '-' {
            continue;
        }
        // Must not be the start of the string's time (first char of time_part)
        if idx == 0 {
            continue;
        }
        let before = s[..idx].trim_end();
        let after = &s[idx..];
        if let Some(off) = parse_offset_token(after) {
            // Ensure `before` still has a time (digit)
            if before.chars().any(|c| c.is_ascii_digit()) {
                return Some((before.to_string(), Some(off)));
            }
        }
    }
    Some((s.to_string(), None))
}

fn parse_offset_token(tok: &str) -> Option<FixedOffset> {
    let tok = tok.trim();
    let (sign, rest) = match tok.chars().next()? {
        '+' => (1i32, &tok[1..]),
        '-' => (-1i32, &tok[1..]),
        _ => return None,
    };
    let rest = rest.trim();
    let (hh, mm) = if rest.len() == 5 && rest.as_bytes().get(2) == Some(&b':') {
        (rest[..2].parse::<i32>().ok()?, rest[3..].parse::<i32>().ok()?)
    } else if rest.len() == 4 && rest.bytes().all(|b| b.is_ascii_digit()) {
        (rest[..2].parse::<i32>().ok()?, rest[2..].parse::<i32>().ok()?)
    } else if rest.len() == 2 && rest.bytes().all(|b| b.is_ascii_digit()) {
        (rest.parse::<i32>().ok()?, 0)
    } else {
        return None;
    };
    if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) {
        return None;
    }
    FixedOffset::east_opt(sign * (hh * 3600 + mm * 60))
}

fn parse_easy_time(raw: &str) -> Option<(NaiveTime, bool)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    let (core, is_pm) = if let Some(rest) = lower.strip_suffix("a.m.") {
        (rest.trim(), Some(false))
    } else if let Some(rest) = lower.strip_suffix("p.m.") {
        (rest.trim(), Some(true))
    } else if let Some(rest) = lower.strip_suffix("am") {
        (rest.trim(), Some(false))
    } else if let Some(rest) = lower.strip_suffix("pm") {
        (rest.trim(), Some(true))
    } else {
        (lower.as_str(), None)
    };

    let naive = NaiveTime::parse_from_str(core, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(core, "%H:%M"))
        .ok()?;

    if let Some(pm) = is_pm {
        let mut hour = naive.hour();
        if hour > 12 {
            return None;
        }
        if pm {
            if hour != 12 {
                hour += 12;
            }
        } else if hour == 12 {
            hour = 0;
        }
        let t = NaiveTime::from_hms_opt(hour, naive.minute(), naive.second())?;
        Some((t, true))
    } else {
        Some((naive, false))
    }
}

fn parse_text_escape(bytes: &[u8], i: usize) -> (char, usize, Option<Diagnostic>) {
    // i points at '\'
    if i + 1 >= bytes.len() {
        return ('\\', i + 1, None);
    }
    let next = bytes[i + 1];
    let known = matches!(next, b'<' | b'{' | b'>' | b'}' | b'\\' | b'n');
    if next == b'n' {
        return ('\n', i + 2, None);
    }
    let ch = next as char;
    let diag = if known {
        None
    } else {
        Some(Diagnostic {
            message: format!("unknown escape \\{ch}"),
            index: i,
        })
    };
    (ch, i + 2, diag)
}

#[derive(Clone, Debug)]
enum ParsedCommand {
    Set { name: String, value: Option<Value> },
    Close { name: String },
    CloseAll,
}

#[derive(Clone, Debug)]
enum Value {
    Number(f32),
    Bool(bool),
    Hex(Rgba),
    Ident(String),
    Quoted(String),
}

fn parse_group(
    bytes: &[u8],
    start: usize,
    _style: &mut Style,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(usize, Vec<ParsedCommand>), usize> {
    // start at '<'
    let mut i = start + 1;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let mut cmds = Vec::new();
    loop {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b',') {
            // Leading / trailing commas ignored (§8).
            i += 1;
        }
        if i >= bytes.len() {
            return Err(start);
        }
        if bytes[i] == b'>' {
            return Ok((i + 1, cmds));
        }
        // Nested `<{` inside a group: close here (§8).
        if bytes[i] == b'<' && group_starts_at(bytes, i) {
            diagnostics.push(Diagnostic {
                message: "nested BFMD group".into(),
                index: i,
            });
            return Ok((i, cmds));
        }
        if bytes[i] != b'{' {
            diagnostics.push(Diagnostic {
                message: "expected command in BFMD group".into(),
                index: i,
            });
            // Skip until `>` or nested `<` or EOF.
            while i < bytes.len()
                && bytes[i] != b'>'
                && !(bytes[i] == b'<' && group_starts_at(bytes, i))
            {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'>' {
                return Ok((i + 1, cmds));
            }
            if i < bytes.len() {
                return Ok((i, cmds));
            }
            return Err(start);
        }
        match parse_command(bytes, i, diagnostics) {
            Ok((next, cmd)) => {
                cmds.push(cmd);
                i = next;
            }
            Err(next) => {
                i = next;
            }
        }
    }
}

fn parse_command(
    bytes: &[u8],
    start: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(usize, ParsedCommand), usize> {
    // start at '{'
    let mut i = start + 1;
    if i >= bytes.len() {
        diagnostics.push(Diagnostic {
            message: "unclosed command".into(),
            index: start,
        });
        return Err(start + 1);
    }
    if bytes[i] == b'}' {
        // `{}` empty — no-op (§8).
        return Ok((
            i + 1,
            ParsedCommand::Set {
                name: String::new(),
                value: None,
            },
        ));
    }
    if bytes[i] == b'!' {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'}' || bytes[i] == b'*') {
            if bytes[i] == b'*' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'}' {
                return Ok((i + 1, ParsedCommand::CloseAll));
            }
        }
        let name_start = i;
        while i < bytes.len() && is_name_char(bytes[i], i == name_start) {
            i += 1;
        }
        if name_start == i {
            diagnostics.push(Diagnostic {
                message: "bad close command".into(),
                index: start,
            });
            // skip to }
            while i < bytes.len() && bytes[i] != b'}' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            return Err(i);
        }
        let name = String::from_utf8_lossy(&bytes[name_start..i]).into_owned();
        if i < bytes.len() && bytes[i] == b'}' {
            return Ok((i + 1, ParsedCommand::Close { name }));
        }
        diagnostics.push(Diagnostic {
            message: "unclosed close command".into(),
            index: start,
        });
        return Err(i);
    }
    let name_start = i;
    while i < bytes.len() && is_name_char(bytes[i], i == name_start) {
        i += 1;
    }
    if name_start == i {
        diagnostics.push(Diagnostic {
            message: "bad set command".into(),
            index: start,
        });
        while i < bytes.len() && bytes[i] != b'}' {
            i += 1;
        }
        if i < bytes.len() {
            i += 1;
        }
        return Err(i);
    }
    let name = String::from_utf8_lossy(&bytes[name_start..i]).into_owned();
    let value = if i < bytes.len() && (bytes[i] == b':' || bytes[i] == b'=') {
        i += 1;
        match parse_value(bytes, i, diagnostics) {
            Ok((next, v)) => {
                i = next;
                Some(v)
            }
            Err(next) => {
                i = next;
                None
            }
        }
    } else {
        None
    };
    if i < bytes.len() && bytes[i] == b'}' {
        // Distinguish empty `{}` — name would be empty; already handled.
        // `<{}>`: name empty after `{` then `}` — handled above as CloseAll wrongly.
        // Fix: `{` then `}` was CloseAll — for `<{}>` apply as no-op.
        return Ok((i + 1, ParsedCommand::Set { name, value }));
    }
    diagnostics.push(Diagnostic {
        message: "unclosed set command".into(),
        index: start,
    });
    Err(i)
}

fn is_name_char(b: u8, first: bool) -> bool {
    if first {
        b.is_ascii_lowercase()
    } else {
        b.is_ascii_lowercase() || b.is_ascii_digit()
    }
}

fn parse_value(
    bytes: &[u8],
    start: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(usize, Value), usize> {
    let mut i = start;
    if i >= bytes.len() {
        return Err(i);
    }
    if bytes[i] == b'"' {
        i += 1;
        let mut s = String::new();
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                if i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\\') {
                    s.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
            }
            if bytes[i] == b'"' {
                return Ok((i + 1, Value::Quoted(s)));
            }
            s.push(bytes[i] as char);
            i += 1;
        }
        diagnostics.push(Diagnostic {
            message: "unclosed quoted value".into(),
            index: start,
        });
        return Err(i);
    }
    // Hex 0x or #
    if bytes[i] == b'#'
        || (bytes[i] == b'0'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X'))
    {
        return parse_hex(bytes, i, diagnostics);
    }
    // Number
    if bytes[i].is_ascii_digit() {
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'.' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
        }
        let raw = std::str::from_utf8(&bytes[i..j]).unwrap_or("");
        if let Ok(n) = raw.parse::<f32>() {
            return Ok((j, Value::Number(n)));
        }
        diagnostics.push(Diagnostic {
            message: "bad number".into(),
            index: i,
        });
        return Err(j);
    }
    // Ident / bool
    if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-' {
        let mut j = i;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
        {
            j += 1;
        }
        let raw = std::str::from_utf8(&bytes[i..j]).unwrap_or("");
        if let Some(b) = parse_bool(raw) {
            return Ok((j, Value::Bool(b)));
        }
        return Ok((j, Value::Ident(raw.to_string())));
    }
    diagnostics.push(Diagnostic {
        message: "bad value".into(),
        index: i,
    });
    Err(i)
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "t" | "true" | "1" => Some(true),
        "f" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_hex(
    bytes: &[u8],
    start: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(usize, Value), usize> {
    let mut i = start;
    let digits = if bytes[i] == b'#' {
        i += 1;
        &bytes[i..]
    } else {
        i += 2; // 0x
        &bytes[i..]
    };
    let mut n = 0usize;
    while n < digits.len() && digits[n].is_ascii_hexdigit() {
        n += 1;
    }
    if n != 6 && n != 8 {
        diagnostics.push(Diagnostic {
            message: "bad hex color".into(),
            index: start,
        });
        return Err(i + n);
    }
    let hex = std::str::from_utf8(&digits[..n]).unwrap_or("");
    let rgba = match n {
        6 => {
            let v = u32::from_str_radix(hex, 16).unwrap_or(0);
            Rgba::rgb(
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            )
        }
        _ => {
            let v = u32::from_str_radix(hex, 16).unwrap_or(0);
            Rgba {
                r: ((v >> 24) & 0xff) as u8,
                g: ((v >> 16) & 0xff) as u8,
                b: ((v >> 8) & 0xff) as u8,
                a: (v & 0xff) as u8,
            }
        }
    };
    Ok((i + n, Value::Hex(rgba)))
}

fn apply_commands(
    cmds: &[ParsedCommand],
    style: &mut Style,
    diagnostics: &mut Vec<Diagnostic>,
    index: usize,
) {
    for cmd in cmds {
        match cmd {
            ParsedCommand::CloseAll => {
                *style = Style::default();
            }
            ParsedCommand::Close { name } => close_slot(style, name),
            ParsedCommand::Set { name, value } => {
                if name.is_empty() {
                    continue;
                }
                // Unknown names: skip + diagnostic (§6). Bad values: same.
                if !KNOWN_NAMES.contains(&name.as_str()) {
                    diagnostics.push(Diagnostic {
                        message: format!("unknown BFMD command {{{name}}}"),
                        index,
                    });
                    continue;
                }
                if !apply_set(style, name, value.as_ref()) {
                    diagnostics.push(Diagnostic {
                        message: format!("bad BFMD value for {{{name}}}"),
                        index,
                    });
                }
            }
        }
    }
}

const KNOWN_NAMES: &[&str] = &["v", "s", "e", "c", "bg", "b", "i", "u", "k", "f", "a", "h", "hr"];

fn close_slot(style: &mut Style, name: &str) {
    match name {
        "s" => style.size = None,
        "e" => style.emote_size = None,
        "c" => style.color = None,
        "bg" => style.background = None,
        "b" => style.bold = false,
        "i" => style.italic = false,
        "u" => style.underline = false,
        "k" => style.strike = false,
        "f" => style.font = None,
        "a" => style.align = None,
        "h" => style.heading = None,
        "hr" => style.hr = false,
        "v" => {}
        _ => {}
    }
}

fn apply_set(style: &mut Style, name: &str, value: Option<&Value>) -> bool {
    match name {
        "v" => true, // handled at parse for version; ignore as style slot
        "s" => match value {
            Some(Value::Number(n)) if *n > 0.0 => {
                style.size = Some(*n);
                true
            }
            _ => false,
        },
        "e" => match value {
            Some(Value::Number(n)) if *n > 0.0 => {
                style.emote_size = Some(n.clamp(EMOTE_PX_MIN, EMOTE_PX_MAX));
                true
            }
            _ => false,
        },
        "c" => match value {
            Some(Value::Hex(c)) => {
                style.color = Some(*c);
                true
            }
            _ => false,
        },
        "bg" => match value {
            Some(Value::Hex(c)) => {
                style.background = Some(*c);
                true
            }
            _ => false,
        },
        "b" | "i" | "u" | "k" | "hr" => {
            let on = match value {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => *n != 0.0,
                Some(Value::Ident(s)) => parse_bool(s).unwrap_or(true),
                _ => return false,
            };
            match name {
                "b" => {
                    if on {
                        style.bold = true;
                    } else {
                        style.bold = false;
                    }
                }
                "i" => style.italic = on,
                "u" => style.underline = on,
                "k" => style.strike = on,
                "hr" => style.hr = on,
                _ => {}
            }
            true
        }
        "f" => match value {
            Some(Value::Ident(s)) | Some(Value::Quoted(s)) => {
                style.font = Some(s.clone());
                true
            }
            _ => false,
        },
        "a" => match value {
            Some(Value::Ident(s)) => {
                style.align = match s.as_str() {
                    "l" => Some(Align::Left),
                    "c" => Some(Align::Center),
                    "r" => Some(Align::Right),
                    "j" => Some(Align::Justify),
                    _ => return false,
                };
                true
            }
            _ => false,
        },
        "h" => match value {
            Some(Value::Number(n)) => {
                let level = *n as u8;
                if (1..=6).contains(&level) {
                    style.heading = Some(level);
                    true
                } else {
                    false
                }
            }
            Some(Value::Ident(s)) => {
                if let Ok(level) = s.parse::<u8>() {
                    if (1..=6).contains(&level) {
                        style.heading = Some(level);
                        return true;
                    }
                }
                false
            }
            _ => false,
        },
        _ => false, // unknown name — skip
    }
}

fn diff_group(from: &Style, to: &Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    // Closes first for slots that went to default, then sets.
    if from.size.is_some() && to.size.is_none() {
        parts.push("{!s}".into());
    }
    if from.emote_size.is_some() && to.emote_size.is_none() {
        parts.push("{!e}".into());
    }
    if from.color.is_some() && to.color.is_none() {
        parts.push("{!c}".into());
    }
    if from.background.is_some() && to.background.is_none() {
        parts.push("{!bg}".into());
    }
    if from.bold && !to.bold {
        parts.push("{!b}".into());
    }
    if from.italic && !to.italic {
        parts.push("{!i}".into());
    }
    if from.underline && !to.underline {
        parts.push("{!u}".into());
    }
    if from.strike && !to.strike {
        parts.push("{!k}".into());
    }
    if from.font.is_some() && to.font.is_none() {
        parts.push("{!f}".into());
    }
    if from.align.is_some() && to.align.is_none() {
        parts.push("{!a}".into());
    }
    if from.heading.is_some() && to.heading.is_none() {
        parts.push("{!h}".into());
    }
    if from.hr && !to.hr {
        parts.push("{!hr}".into());
    }

    if to.size != from.size {
        if let Some(s) = to.size {
            parts.push(format!("{{s:{s}}}"));
        }
    }
    if to.emote_size != from.emote_size {
        if let Some(e) = to.emote_size {
            parts.push(format!("{{e:{e}}}"));
        }
    }
    if to.color != from.color {
        if let Some(c) = to.color {
            parts.push(format!("{{c={}}}", format_hex(c)));
        }
    }
    if to.background != from.background {
        if let Some(c) = to.background {
            parts.push(format!("{{bg={}}}", format_hex(c)));
        }
    }
    if to.bold && !from.bold {
        parts.push("{b:t}".into());
    }
    if to.italic && !from.italic {
        parts.push("{i:t}".into());
    }
    if to.underline && !from.underline {
        parts.push("{u:t}".into());
    }
    if to.strike && !from.strike {
        parts.push("{k:t}".into());
    }
    if to.font != from.font {
        if let Some(f) = &to.font {
            if f.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                parts.push(format!("{{f:{f}}}"));
            } else {
                parts.push(format!(
                    "{{f:\"{}\"}}",
                    f.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
        }
    }
    if to.align != from.align {
        if let Some(a) = to.align {
            let letter = match a {
                Align::Left => "l",
                Align::Center => "c",
                Align::Right => "r",
                Align::Justify => "j",
            };
            parts.push(format!("{{a:{letter}}}"));
        }
    }
    if to.heading != from.heading {
        if let Some(h) = to.heading {
            parts.push(format!("{{h:{h}}}"));
        }
    }
    if to.hr && !from.hr {
        parts.push("{hr:t}".into());
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("<{}>", parts.join(","))
    }
}

fn format_hex(c: Rgba) -> String {
    if c.a == 255 {
        format!("0x{:02X}{:02X}{:02X}", c.r, c.g, c.b)
    } else {
        format!("0x{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a)
    }
}

fn escape_text_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("\\<"),
            '{' => out.push_str("\\{"),
            '>' => out.push_str("\\>"),
            '}' => out.push_str("\\}"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
}

/// Map plain-text (visible) caret indices to... used by composer when InputText
/// edits the visible plain form. Rebuild BFMD after a plain edit by re-parsing
/// is lossy; Banter keeps BFMD in the composer buffer for v1 (commands visible
/// only when styles are applied via toolbar).
pub fn map_plain_selection_to_source_hint(source: &str) -> (String, usize) {
    let plain = plain_text(source);
    let len = plain.len();
    (plain, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles_of(source: &str) -> Vec<(String, Style)> {
        parse(source)
            .runs
            .into_iter()
            .map(|r| (r.text, r.style))
            .collect()
    }

    #[test]
    fn example_a_bold_remains_after_close_s_c() {
        let src = "<{s:36},{c=0xFFFFFF},{b:t}> Text Goes Here <{!s},{!c}> This is still bold.";
        let runs = parse(src).runs;
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, " Text Goes Here ");
        assert_eq!(runs[0].style.size, Some(36.0));
        assert_eq!(runs[0].style.color, Some(Rgba::rgb(255, 255, 255)));
        assert!(runs[0].style.bold);
        assert_eq!(runs[1].text, " This is still bold.");
        assert_eq!(runs[1].style.size, None);
        assert_eq!(runs[1].style.color, None);
        assert!(runs[1].style.bold);
    }

    #[test]
    fn example_b_size_replace_no_stack() {
        let src = "<{s:36}> Text is size 36 <{s:42}> This is size 42.";
        let runs = parse(src).runs;
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].style.size, Some(36.0));
        assert_eq!(runs[1].style.size, Some(42.0));
    }

    #[test]
    fn example_c_eof_closes_without_trailing_group() {
        let runs = parse("<{b:t},{i:t}>hello").runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hello");
        assert!(runs[0].style.bold && runs[0].style.italic);
    }

    #[test]
    fn example_d_escaped_angle_is_literal() {
        let runs = parse(r"use \<{s:12}> as text").runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "use <{s:12}> as text");
        assert!(runs[0].style.is_default());
    }

    #[test]
    fn close_size_after_replace_restores_default() {
        let src = "<{s:36}>a<{s:42}>b<{!s}>c";
        let runs = parse(src).runs;
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].style.size, Some(36.0));
        assert_eq!(runs[1].style.size, Some(42.0));
        assert_eq!(runs[2].style.size, None);
        assert_eq!(runs[2].text, "c");
    }

    #[test]
    fn close_all_clears_bold_and_size() {
        let runs = parse("<{s:20},{b:t}>x<{!}>y").runs;
        assert_eq!(runs.len(), 2);
        assert!(runs[0].style.bold);
        assert_eq!(runs[0].style.size, Some(20.0));
        assert!(runs[1].style.is_default());
        assert_eq!(runs[1].text, "y");
    }

    #[test]
    fn unknown_command_ignored_bold_still_applies() {
        let runs = parse("<{z:1},{b:t}>hi").runs;
        assert_eq!(runs.len(), 1);
        assert!(runs[0].style.bold);
        assert_eq!(runs[0].text, "hi");
    }

    #[test]
    fn angle_not_followed_by_brace_is_literal() {
        let runs = parse("a < b").runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "a < b");
    }

    #[test]
    fn hex_0x_and_hash_same_rgb() {
        let a = parse("<{c=0xFFFFFF}>x").runs;
        let b = parse("<{c:#FFFFFF}>x").runs;
        assert_eq!(a[0].style.color, b[0].style.color);
        assert_eq!(a[0].style.color, Some(Rgba::rgb(255, 255, 255)));
    }

    #[test]
    fn hr_flag_then_clear() {
        let runs = parse("<{hr}>\n<{!hr}>after").runs;
        assert!(runs.len() >= 2);
        assert!(runs[0].style.hr);
        assert_eq!(runs[0].text, "\n");
        let last = runs.last().unwrap();
        assert!(!last.style.hr);
        assert_eq!(last.text, "after");
    }

    #[test]
    fn plain_text_round_trips() {
        let src = "hello world";
        let runs = parse(src).runs;
        assert_eq!(serialize(&runs), src);
    }

    #[test]
    fn toggle_bold_on_selection() {
        let out = toggle_flag("hello world", 6, 11, Flag::Bold);
        let runs = parse(&out).runs;
        assert!(runs.iter().any(|r| r.text == "world" && r.style.bold));
        assert!(runs.iter().any(|r| r.text == "hello " && !r.style.bold));
    }

    #[test]
    fn styles_of_smoke() {
        let s = styles_of("<{b}>x");
        assert_eq!(s[0].0, "x");
        assert!(s[0].1.bold);
    }
}
