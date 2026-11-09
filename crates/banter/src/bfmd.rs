//! BFMD (Better Formatted Markdown) — Banter chat text formatting.
//!
//! Spec: project store `docs/bfmd.md` (Sickle-adapted). This is a style-command
//! language, not CommonMark. Messages store BFMD source in `body`; plain text
//! with no command groups is valid BFMD and round-trips unchanged.

/// Default body size in points when `{s}` is unset (theme body).
pub const DEFAULT_SIZE_PT: f32 = 16.0;

/// Default inline emote height in CSS px when `{e}` is unset (Carbon parity).
pub const DEFAULT_EMOTE_PX: f32 = 22.0;

/// Inclusive clamp for `{e}` emote height (plan: 16–128).
pub const EMOTE_PX_MIN: f32 = 16.0;
pub const EMOTE_PX_MAX: f32 = 128.0;

/// Inclusive clamp for `{ld}` list nesting depth.
pub const LIST_DEPTH_MIN: u8 = 1;
pub const LIST_DEPTH_MAX: u8 = 8;

/// List kind for the `{lk}` slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// Bulleted item (`{lk:u}`). Markdown `-`/`*`/`+` flat markers also map here.
    Unordered,
    /// Numbered item (`{lk:o}`). BFMD-only — Markdown `1.` is not a list.
    Ordered,
}

/// One styled text span produced by the parser.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
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
    /// `{lk:u}` / `{lk:o}` — list item kind while set.
    pub list_kind: Option<ListKind>,
    /// `{ld:N}` — nesting depth 1..=8. When `list_kind` is set and this is `None`, depth is 1.
    pub list_depth: Option<u8>,
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

    /// Effective list depth when `list_kind` is set; otherwise `0` (not a list item).
    pub fn list_depth_or_default(&self) -> u8 {
        if self.list_kind.is_none() {
            return 0;
        }
        self.list_depth
            .unwrap_or(LIST_DEPTH_MIN)
            .clamp(LIST_DEPTH_MIN, LIST_DEPTH_MAX)
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
        // Literal `<` or any other byte as UTF-8 char.
        let ch = source[i..].chars().next().unwrap();
        text.push(ch);
        i += ch.len_utf8();
        first_group = false;
    }
    flush(&mut text, &style, &mut runs);
    let runs = apply_markdown_flat_bullets(runs);
    ParseResult {
        runs,
        diagnostics,
        version,
    }
}

/// Flat Markdown bullets only (`-`/`*`/`+` at column 0). Never numbers, never indent-nesting.
///
/// When a line already has BFMD `{lk}` set, markers stay literal. Numbered Markdown (`1. `)
/// is never promoted to a list — use `{lk:o}` instead.
fn apply_markdown_flat_bullets(runs: Vec<Run>) -> Vec<Run> {
    if runs.is_empty() {
        return runs;
    }
    let mut out: Vec<Run> = Vec::new();
    let mut at_line_start = true;

    let flush = |buf: &mut String, style: &Style, out: &mut Vec<Run>| {
        if buf.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut() {
            if last.style == *style {
                last.text.push_str(buf);
                buf.clear();
                return;
            }
        }
        out.push(Run {
            text: std::mem::take(buf),
            style: style.clone(),
        });
    };

    for run in runs {
        let base = run.style;
        let chars: Vec<char> = run.text.chars().collect();
        let mut i = 0usize;
        let mut buf = String::new();
        let mut buf_style = base.clone();

        while i < chars.len() {
            if at_line_start && base.list_kind.is_none() {
                if i + 1 < chars.len()
                    && matches!(chars[i], '-' | '*' | '+')
                    && chars[i + 1] == ' '
                {
                    flush(&mut buf, &buf_style, &mut out);
                    i += 2;
                    let mut line_style = base.clone();
                    line_style.list_kind = Some(ListKind::Unordered);
                    line_style.list_depth = None;
                    buf_style = line_style;
                    at_line_start = false;
                    continue;
                }
            }
            let ch = chars[i];
            i += 1;
            if ch == '\n' {
                buf.push(ch);
                flush(&mut buf, &buf_style, &mut out);
                buf_style = base.clone();
                at_line_start = true;
                continue;
            }
            at_line_start = false;
            buf.push(ch);
        }
        flush(&mut buf, &buf_style, &mut out);
    }
    compact_runs(out)
}

/// Plain visible text (commands stripped). Useful for search previews / mentions.
pub fn plain_text(source: &str) -> String {
    parse(source).runs.into_iter().map(|r| r.text).collect()
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
pub fn serialize(runs: &[Run]) -> String {
    let mut out = String::new();
    let mut current = Style::default();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        if run.style != current {
            out.push_str(&diff_group(&current, &run.style));
            current = run.style.clone();
        }
        escape_text_into(&run.text, &mut out);
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
        let len = run.text.len();
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

fn plain_len(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.len()).sum()
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
        let len = run.text.len();
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
        let len = run.text.len();
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
        let len = run.text.len();
        let run_start = offset;
        let run_end = offset + len;
        if at > run_start && at < run_end {
            let local = at - run_start;
            out.push(Run {
                text: run.text[..local].to_string(),
                style: run.style.clone(),
            });
            out.push(Run {
                text: run.text[local..].to_string(),
                style: run.style.clone(),
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
        if run.text.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.style == run.style {
                last.text.push_str(&run.text);
                continue;
            }
        }
        out.push(run);
    }
    out
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

const KNOWN_NAMES: &[&str] = &[
    "v", "s", "e", "c", "bg", "b", "i", "u", "k", "f", "a", "h", "hr", "lk", "ld",
];

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
        // Closing list kind also drops depth — depth alone is not a list item.
        "lk" => {
            style.list_kind = None;
            style.list_depth = None;
        }
        "ld" => style.list_depth = None,
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
        "lk" => match value {
            Some(Value::Ident(s)) => match s.as_str() {
                "u" => {
                    style.list_kind = Some(ListKind::Unordered);
                    true
                }
                "o" => {
                    style.list_kind = Some(ListKind::Ordered);
                    true
                }
                _ => false,
            },
            _ => false,
        },
        "ld" => match value {
            Some(Value::Number(n)) => {
                let depth = *n as u8;
                if (LIST_DEPTH_MIN..=LIST_DEPTH_MAX).contains(&depth) {
                    style.list_depth = Some(depth);
                    true
                } else {
                    false
                }
            }
            Some(Value::Ident(s)) => {
                if let Ok(depth) = s.parse::<u8>() {
                    if (LIST_DEPTH_MIN..=LIST_DEPTH_MAX).contains(&depth) {
                        style.list_depth = Some(depth);
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
    if from.list_kind.is_some() && to.list_kind.is_none() {
        parts.push("{!lk}".into());
    }
    if from.list_depth.is_some() && to.list_depth.is_none() {
        // `{!lk}` already clears depth; only emit `{!ld}` when kind remains.
        if to.list_kind.is_some() {
            parts.push("{!ld}".into());
        }
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
    if to.list_kind != from.list_kind {
        if let Some(kind) = to.list_kind {
            let letter = match kind {
                ListKind::Unordered => "u",
                ListKind::Ordered => "o",
            };
            parts.push(format!("{{lk:{letter}}}"));
        }
    }
    if to.list_depth != from.list_depth {
        if let Some(d) = to.list_depth {
            parts.push(format!("{{ld:{d}}}"));
        }
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
