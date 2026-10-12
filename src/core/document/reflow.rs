//! Hard-line reflow for the `gq`/`gw` operators.
//!
//! The formatter is portable text policy: it operates on literal hard lines of
//! Text and Code, recognizes ordinary paragraphs, list items, and declarative
//! comment leaders, and produces minimal whitespace/leader patches. It never
//! flattens the whole document, evaluates a language runtime, or depends on
//! syntax coverage, styles, fonts, or layout.
use super::{Format, HardLineSnapshot, TextEdit};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// The application default for `textwidth`.
pub const DEFAULT_TEXT_WIDTH: u32 = 80;
/// Tabs advance to the next multiple of this column count.
#[cfg(test)]
const TAB_STOP: usize = 8;
/// Bound on optional body-decoration lookahead; never used to prove lexical context.
const STYLE_LOOKAHEAD_LINES: usize = 512;

/// Buffer-owned `textwidth` state: an inherited application default plus an
/// optional explicit local override.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextWidthSetting {
    default: u32,
    local: Option<u32>,
}

impl Default for TextWidthSetting {
    fn default() -> Self {
        Self::new(DEFAULT_TEXT_WIDTH)
    }
}

impl TextWidthSetting {
    pub const fn new(default: u32) -> Self {
        Self {
            default,
            local: None,
        }
    }

    /// The width used by reflow: the local override when set, else the default.
    pub fn effective(&self) -> u32 {
        self.local.unwrap_or(self.default)
    }

    pub fn default_width(&self) -> u32 {
        self.default
    }

    pub fn local_override(&self) -> Option<u32> {
        self.local
    }

    /// Changing the application default keeps an explicit override intact.
    pub fn set_default(&mut self, default: u32) {
        self.default = default;
    }

    /// `None` returns the buffer to inheriting the application default.
    pub fn set_local(&mut self, local: Option<u32>) {
        self.local = local;
    }
}

use super::format_options::{BlockCommentSyntax, CommentProfile, CommentStart, ListPattern, ReflowOptions};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflowError {
    /// Markdown source and formatted views need format-aware semantics.
    UnsupportedFormat(Format),
    InvalidLineRange {
        start: usize,
        end: usize,
        line_count: usize,
    },
    InvalidTextWidth,
    InvalidOptions(String),
    UnknownCommentContext,
    /// The snapshot could not be read at a validated line.
    Unreadable,
}

impl std::fmt::Display for ReflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => {
                write!(
                    formatter,
                    "reflow is not supported for {format:?} documents"
                )
            }
            Self::InvalidLineRange {
                start,
                end,
                line_count,
            } => write!(
                formatter,
                "hard lines {start}..{end} are outside the {line_count} document lines"
            ),
            Self::InvalidTextWidth => write!(formatter, "textwidth must be positive"),
            Self::InvalidOptions(error) => write!(formatter, "{error}"),
            Self::UnknownCommentContext => write!(formatter, "comment context is not available for these lines"),
            Self::Unreadable => write!(formatter, "the formatted snapshot could not be read"),
        }
    }
}

/// One reflow of a half-open hard-line range in an exact snapshot.
pub struct ReflowRequest<'a> {
    pub snapshot: &'a HardLineSnapshot,
    pub format: Format,
    pub lines: Range<usize>,
    pub text_width: u32,
    pub tabstop: u32,
    pub options: &'a ReflowOptions,
    pub autoindent: bool,
}

#[cfg(test)]
fn end_column(start: usize, text: &str) -> usize {
    super::indentation::end_column(start, text, TAB_STOP)
}
fn columns(start: usize, text: &str, tabstop: usize) -> usize {
    super::indentation::end_column(start, text, tabstop)
}

fn is_indent_grapheme(grapheme: &str) -> bool {
    grapheme == " " || grapheme == "\t"
}

fn is_gap_space(character: char) -> bool {
    matches!(character, ' ' | '\t')
        || (character.is_whitespace()
            && !matches!(
                character,
                '\r' | '\n'
                    | '\u{0B}'
                    | '\u{0C}'
                    | '\u{85}'
                    | '\u{A0}'
                    | '\u{2007}'
                    | '\u{202F}'
                    | '\u{2028}'
                    | '\u{2029}'
            ))
}

/// Inter-word whitespace is a single-scalar space grapheme; a space carrying
/// a combining mark stays attached to its cluster.
fn is_gap_grapheme(grapheme: &str) -> bool {
    let mut characters = grapheme.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => is_gap_space(character),
        _ => false,
    }
}

fn leading_len(text: &str, predicate: impl Fn(&str) -> bool) -> usize {
    text.graphemes(true)
        .take_while(|grapheme| predicate(grapheme))
        .map(str::len)
        .sum()
}

fn trailing_gap_len(text: &str) -> usize {
    text.graphemes(true)
        .rev()
        .take_while(|grapheme| is_gap_grapheme(grapheme))
        .map(str::len)
        .sum()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PrefixKey {
    /// Indentation is compared through inner columns, so a hanging list
    /// continuation can follow its item line.
    Plain,
    LineComment(String, String),
    Block(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Parsed {
    key: PrefixKey,
    base_end: usize,
    inner_end: usize,
    inner_columns: usize,
    /// Hanging continuation columns of a recognized list marker.
    hanging: Option<usize>,
    word_start: usize,
    body_end: usize,
    /// End of a same-line block-comment closer following the body.
    suffix_end: Option<usize>,
    opener: bool,
    closes: bool,
    owner: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Kind {
    /// Blank or blank-comment line: a separator that stays byte-identical.
    Separator,
    /// A boundary that stays byte-identical.
    Fixed,
    Text(Parsed),
}

fn parse_text(text: &str, key: PrefixKey, base_end: usize, opener: bool, tabstop: usize) -> Option<Parsed> {
    let inner_len = leading_len(&text[base_end..], is_indent_grapheme);
    let inner_end = base_end + inner_len;
    let base_column = columns(0, &text[..base_end], tabstop);
    let inner_column = columns(base_column, &text[base_end..inner_end], tabstop);
    parse_body(
        text,
        key,
        base_end,
        inner_end,
        inner_column - base_column,
        inner_column,
        opener,
        tabstop,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_body(
    text: &str,
    key: PrefixKey,
    base_end: usize,
    inner_end: usize,
    inner_columns: usize,
    inner_column: usize,
    opener: bool,
    tabstop: usize,
) -> Option<Parsed> {
    let body_end = text.len() - trailing_gap_len(&text[inner_end..]);
    if body_end <= inner_end {
        return None;
    }
    let _ = (inner_column, tabstop);
    let word_start = inner_end;
    let hanging = None;
    Some(Parsed {
        key,
        base_end,
        inner_end,
        inner_columns,
        hanging,
        word_start,
        body_end,
        suffix_end: None,
        opener,
        closes: false,
        owner: None,
    })
}

/// Byte length of a recognized list marker plus its following whitespace.
fn list_marker(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let marker_len = if matches!(bytes.first(), Some(b'-' | b'+' | b'*')) {
        1
    } else {
        let digits = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 || digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    let whitespace = leading_len(&body[marker_len..], is_indent_grapheme);
    (whitespace > 0).then_some(marker_len + whitespace)
}

fn is_delimiter_only(text: &str, middle: &str) -> bool {
    text.trim_matches(|character: char| character == ' ' || character == '\t')
        .split(middle)
        .all(str::is_empty)
}

struct Classifier<'a> {
    profile: Option<&'a CommentProfile>,
    in_block: Option<usize>,
    tabstop: usize,
}

impl Classifier<'_> {
    fn classify(&mut self, text: &str) -> Kind {
        if text.trim().is_empty() {
            return Kind::Separator;
        }
        let indent_len = leading_len(text, is_indent_grapheme);
        let indent = &text[..indent_len];
        let rest = &text[indent_len..];
        let Some(profile) = self.profile else {
            return plain(text, self.tabstop);
        };
        if let Some(index) = self.in_block {
            return self.classify_block_interior(text, indent_len, rest, &profile.blocks[index], index);
        }
        if let Some((length, leader)) = profile.line_prefix(rest) {
            let base_end = indent_len + length;
            let mut parsed = parse_text(text, PrefixKey::LineComment(indent.to_owned(), rest[..length].to_owned()), base_end, false, self.tabstop);
            if leader.first {
                if let Some(parsed) = parsed.as_mut() {
                    parsed.hanging = Some(columns(0, &text[indent_len..parsed.inner_end], self.tabstop));
                    parsed.inner_columns = columns(0, indent, self.tabstop);
                    parsed.key = PrefixKey::Plain;
                    parsed.inner_end = indent_len;
                }
            }
            return parsed.map_or(Kind::Separator, Kind::Text);
        }
        for (block_index, block) in profile.blocks.iter().enumerate() {
            if block.open_matches(rest) {
                let base_end = indent_len + block.open.len();
                let after = &text[base_end..];
                return match block.find_close(after) {
                    Some(close) => {
                        let close_end = base_end + close + block.close.len();
                        if !text[close_end..].trim().is_empty()
                            || is_delimiter_only(&after[..close], &block.middle)
                        {
                            return Kind::Fixed;
                        }
                        let mut parsed = match parse_text(
                            &text[..base_end + close],
                            PrefixKey::Block(block_index),
                            base_end,
                            true,
                            self.tabstop,
                        ) {
                            Some(parsed) => parsed,
                            None => return Kind::Fixed,
                        };
                        parsed.suffix_end = Some(close_end);
                        parsed.closes = true;
                        Kind::Text(parsed)
                    }
                    None => {
                        self.in_block = Some(block_index);
                        if is_delimiter_only(after, &block.middle) {
                            return Kind::Fixed;
                        }
                        parse_text(text, PrefixKey::Block(block_index), base_end, true, self.tabstop)
                            .map_or(Kind::Fixed, Kind::Text)
                    }
                };
            }
        }
        plain(text, self.tabstop)
    }

    fn classify_block_interior(
        &mut self,
        text: &str,
        _indent_len: usize,
        rest: &str,
        block: &BlockCommentSyntax,
        block_index: usize,
    ) -> Kind {
        let trimmed =
            rest.trim_end_matches(|character: char| character == ' ' || character == '\t');
        if trimmed.ends_with(&block.close)
            && is_delimiter_only(&trimmed[..trimmed.len() - block.close.len()], &block.middle)
        {
            self.in_block = None;
            return Kind::Fixed;
        }
        if !block.middle_is_whitespace() && rest.starts_with(&block.middle) && !block.middle_matches(rest) {
            return Kind::Fixed;
        }
        let base_end = if !block.middle_is_whitespace() && !rest.starts_with(&block.close) {
            block.middle_body_start(text).unwrap_or(0)
        } else { 0 };
        let inner_len = leading_len(&text[base_end..], is_indent_grapheme);
        let inner_end = base_end + inner_len;
        let base_column = columns(0, &text[..base_end], self.tabstop);
        let inner_column = columns(base_column, &text[base_end..inner_end], self.tabstop);
        let inner_columns = inner_column - base_column;
        let after = &text[inner_end..];
        match block.find_close(after) {
            Some(close) => {
                let close_end = inner_end + close + block.close.len();
                self.in_block = None;
                if !text[close_end..].trim().is_empty() {
                    return Kind::Fixed;
                }
                let Some(mut parsed) = parse_body(
                    &text[..inner_end + close],
                    PrefixKey::Block(block_index),
                    base_end,
                    inner_end,
                    inner_columns,
                    inner_column,
                    false,
                    self.tabstop,
                ) else {
                    return Kind::Fixed;
                };
                parsed.suffix_end = Some(close_end);
                parsed.closes = true;
                Kind::Text(parsed)
            }
            None => parse_body(
                text,
                PrefixKey::Block(block_index),
                base_end,
                inner_end,
                inner_columns,
                inner_column,
                false,
                self.tabstop,
            )
            .map_or(Kind::Separator, Kind::Text),
        }
    }
}

fn plain(text: &str, tabstop: usize) -> Kind {
    parse_text(text, PrefixKey::Plain, 0, false, tabstop).map_or(Kind::Separator, Kind::Text)
}

struct Line {
    start: usize,
    text: String,
    parsed: Parsed,
}

struct Paragraph {
    lines: Vec<Line>,
}

impl Paragraph {
    fn first(&self) -> &Parsed {
        &self.lines[0].parsed
    }

    fn accepts(&self, next: &Parsed, second_indent: bool, trailing_white: bool) -> bool {
        let first = self.first();
        let last = &self.lines[self.lines.len() - 1].parsed;
        let same_kind = next.key == first.key || (second_indent && matches!((&next.key, &first.key),
            (PrefixKey::LineComment(_, next), PrefixKey::LineComment(_, first)) if next == first));
        if last.closes || next.hanging.is_some() || !same_kind || next.owner != first.owner
            || (trailing_white && !self.lines.last().unwrap().text.ends_with([' ', '\t'])) {
            return false;
        }
        // The first body row establishes a block opener's authored decoration
        // and indentation, including unstarred/whitespace-middle conventions.
        let body_indent = second_indent || first.opener;
        (body_indent && self.lines.len() == 1)
            || (body_indent && self.lines.get(1).is_some_and(|line| line.parsed.inner_columns == next.inner_columns))
            || next.inner_columns == first.inner_columns
            || first
                .hanging
                .is_some_and(|hanging| next.inner_columns == first.inner_columns + hanging)
    }
}

fn read_line(snapshot: &HardLineSnapshot, index: usize) -> Option<(usize, String)> {
    let info = snapshot.line(index)?;
    let range = info.content_range();
    let text = snapshot.slice_utf8(range.clone()).ok()?;
    Some((range.start, text))
}

/// A formatting region is distinct from its native lexical classification.
#[derive(Clone, Debug)]
enum FormattingRegion {
    Plain,
    Fixed,
    Unknown,
    Line { at: usize, end: usize },
    Block { index: usize, origin: (usize, usize), opener: Option<Range<usize>>, closer: Option<Range<usize>> },
}

/// Literal custom blocks have no language grammar of their own. Establish their
/// state from the beginning, never from a guessed lookback boundary. Both work
/// and allocations are bounded. Native blocks use the incremental lexer instead.
struct CustomContext {
    next_line: usize,
    active: Option<(usize, usize, usize)>,
    remaining: usize,
}
impl Default for CustomContext {
    fn default() -> Self { Self { next_line: 0, active: None, remaining: 256 * 1024 } }
}
fn custom_block_before(cache: &mut CustomContext, snapshot: &HardLineSnapshot, line: usize, profile: &CommentProfile,
    language: Option<&str>, compatibility: bool) -> Result<Option<(usize, usize, usize)>, ()> {
    let custom = |index: usize| compatibility || !super::comment_context::supports_native_block(language, &profile.blocks[index].open);
    if !profile.blocks.iter().enumerate().any(|(index, _)| custom(index)) { return Ok(None); }
    if line > 4096 { return Err(()); }
    if line < cache.next_line { *cache = CustomContext::default(); }
    for previous in cache.next_line..line {
        let info = snapshot.line(previous).ok_or(())?;
        cache.remaining = cache.remaining.checked_sub(info.content_range().len()).ok_or(())?;
        let (_, text) = read_line(snapshot, previous).ok_or(())?;
        cache.next_line = previous + 1;
        let mut at = 0;
        if let Some((index, _, _)) = cache.active {
            let block = &profile.blocks[index];
            if let Some(close) = block.find_close(&text) {
                cache.active = None;
                at = close + block.close.len();
            } else { continue; }
        }
        let context = (!compatibility).then(|| snapshot.comment_context(language, previous));
        if context.as_ref().is_some_and(|context| context.is_unknown()) { return Err(()); }
        while at < text.len() {
            if context.as_ref().is_none_or(|context| context.is_code_at(at)) {
                match profile.match_start(&text[at..]) {
                    Some(CommentStart::Line { leader, .. }) if compatibility || !super::comment_context::supports_native_line(language, &leader.text) => break,
                    Some(CommentStart::Block { index, syntax }) if custom(index) => {
                        if let Some(close) = syntax.find_close(&text[at + syntax.open.len()..]) {
                            at += syntax.open.len() + close + syntax.close.len();
                            continue;
                        }
                        cache.active = Some((index, previous, at));
                        break;
                    }
                    _ => {}
                }
            }
            at += text[at..].chars().next().unwrap().len_utf8();
        }
    }
    Ok(cache.active)
}

fn formatting_region(custom: &mut CustomContext, snapshot: &HardLineSnapshot, line: usize, text: &str,
    profile: &CommentProfile, language: Option<&str>, boundary: Option<usize>) -> FormattingRegion {
    use super::comment_context::{CommentContextStatus, NativeCommentKind};
    if profile.line_leaders.is_empty() && profile.blocks.is_empty() { return FormattingRegion::Plain; }
    if text.len() > 256 * 1024 { return FormattingRegion::Unknown; }
    let context = snapshot.comment_context(language, line);
    let compatibility = context.status == CommentContextStatus::Compatibility;
    if context.is_unknown() { return FormattingRegion::Unknown; }
    match custom_block_before(custom, snapshot, line, profile, language, compatibility) {
        Err(()) => return FormattingRegion::Unknown,
        Ok(Some((index, origin_line, origin_at))) => {
            let block = &profile.blocks[index];
            return FormattingRegion::Block { index, origin: (origin_line, origin_at), opener: None,
                closer: block.find_close(text).map(|at| at..at + block.close.len()) };
        }
        Ok(None) => {}
    }
    // Native strings protect every conflicting manual leader. In compatibility
    // mode the configured literals intentionally supply the entire recognition.
    let mut skip_until = 0;
    let mut last_closed = None;
    for (at, _) in text.char_indices() {
        if at < skip_until { continue; }
        if boundary.is_some_and(|cursor| at > cursor) { break; }
        if !compatibility && !context.is_code_at(at) { continue; }
        match profile.match_start(&text[at..]) {
            Some(CommentStart::Line { end, leader }) if compatibility || !super::comment_context::supports_native_line(language, &leader.text) => return FormattingRegion::Line { at, end: at + end },
            Some(CommentStart::Block { index, syntax }) => {
                // Native grammar always wins over a non-native block spelling.
                if !compatibility && super::comment_context::supports_native_block(language, &syntax.open) { continue; }
                let end = at + syntax.open.len();
                let closer = syntax.find_close(&text[end..]).map(|close| end + close..end + close + syntax.close.len());
                if let Some(close) = &closer {
                    if boundary.is_some_and(|cursor| close.end <= cursor) {
                        skip_until = close.end;
                        last_closed = Some(FormattingRegion::Block { index, origin: (line, at), opener: Some(at..end), closer });
                        continue;
                    }
                }
                return FormattingRegion::Block { index, origin: (line, at), opener: Some(at..end), closer };
            }
            _ => {}
        }
    }
    if !compatibility && !context.comments.is_empty() {
        let comment = if let Some(boundary) = boundary {
            let at = context.comment_at(boundary).or_else(|| context.comments.last().filter(|span|
                span.kind == NativeCommentKind::Block && span.closer.as_ref().is_some_and(|close|
                    close.end <= boundary && text[close.end..boundary].trim().is_empty())));
            let Some(comment) = at else { return FormattingRegion::Fixed; };
            comment
        } else {
            // Whitespace around a nested owner is not a second prose body.
            // Two substantive owners on one line remain a reflow boundary.
            let mut significant = context.comments.iter().filter(|span|
                span.opener.is_some() || span.closer.is_some() || !text[span.body.clone()].trim().is_empty());
            let comment = significant.next().unwrap_or(&context.comments[0]);
            if significant.next().is_some() { return FormattingRegion::Fixed; }
            comment
        };
        if comment.kind == NativeCommentKind::Line {
            let at = comment.range.start;
            return match profile.match_start(&text[at..]) {
                Some(CommentStart::Line { end, .. }) => FormattingRegion::Line { at, end: at + end },
                _ => FormattingRegion::Fixed,
            };
        }
        let Some(origin) = comment.block_start else { return FormattingRegion::Fixed; };
        let Some((_, source)) = read_line(snapshot, origin.0) else { return FormattingRegion::Unknown; };
        let Some(CommentStart::Block { index, syntax }) = profile.match_start(&source[origin.1..]) else { return FormattingRegion::Fixed; };
        if let Some(close) = comment.expected_closer.as_deref() {
            let native_open_in = |text: &str| text.char_indices().any(|(at, _)|
                super::comment_context::supports_native_block(language, &text[at..]));
            if syntax.close.find(close) != syntax.close.len().checked_sub(close.len())
                || syntax.open.contains(close) || syntax.middle.contains(close)
                || native_open_in(&syntax.middle) || native_open_in(&syntax.close)
            { return FormattingRegion::Fixed; }
        }
        let closer = if let Some(close) = &comment.closer {
            let Some(start) = close.end.checked_sub(syntax.close.len()) else { return FormattingRegion::Fixed; };
            if !text.is_char_boundary(start) || syntax.find_close(&text[start..]) != Some(0) {
                return FormattingRegion::Fixed;
            }
            Some(start..close.end)
        } else { None };
        let opener = if origin.0 == line { Some(origin.1..origin.1 + syntax.open.len()) } else { None };
        return FormattingRegion::Block { index, origin, opener, closer };
    }
    if let Some(closed) = last_closed { return closed; }
    if !context.protected.is_empty() { FormattingRegion::Fixed } else { FormattingRegion::Plain }
}

/// The existing middle-line prefix convention after an opener, looked up in
/// the local comment context. `None` when the block ends without one.
fn middle_convention(
    snapshot: &HardLineSnapshot,
    after: usize,
    block: &BlockCommentSyntax,
    language: Option<&str>,
    owner: Option<(usize, usize)>,
) -> Option<String> {
    for index in after..after.saturating_add(STYLE_LOOKAHEAD_LINES) {
        if super::comment_context::supports_native_block(language, &block.open) {
            let context = snapshot.comment_context(language, index);
            if context.is_unknown() || !context.comments.iter().any(|span| span.block_start == owner) { return None; }
            if context.comments.iter().any(|span| span.block_start != owner) { continue; }
        }
        let (_, text) = read_line(snapshot, index)?;
        let indent_len = leading_len(&text, is_indent_grapheme);
        let rest = &text[indent_len..];
        if block.find_close(rest).is_some() {
            return None;
        }
        if rest.is_empty() {
            continue;
        }
        if block.middle_matches(rest) {
            let base_end = indent_len + block.middle.len();
            let inner = leading_len(&text[base_end..], is_indent_grapheme);
            let inner = if inner == 0 {
                " ".to_owned()
            } else {
                text[base_end..base_end + inner].to_owned()
            };
            return Some(format!("{}{inner}", &text[..base_end]));
        }
        // A spelling rejected by `b` is not evidence of an unstarred body.
        // Keep the configured continuation when the next row is `*pointer`.
        if rest.starts_with(&block.middle) {
            return None;
        }
        return Some(text[..indent_len].to_owned());
    }
    None
}

/// A continuation carries its originating block's closing rule rather than
/// trying to infer that identity from an ambiguous generated middle marker.
#[derive(Clone, Debug)]
pub(crate) struct CommentContinuation {
    pub prefix: String,
    pub block: Option<usize>,
    pub trailing: bool,
    pub closing_prefix: Option<String>,
}

pub(crate) fn comment_continuation_prefix(
    snapshot: &HardLineSnapshot, line: usize, before_cursor: &str,
    profile: &CommentProfile, language: Option<&str>, above: bool, tabstop: usize,
) -> Option<CommentContinuation> {
    let (_, full_text) = read_line(snapshot, line)?;
    let region = formatting_region(&mut CustomContext::default(), snapshot, line, &full_text, profile, language, Some(before_cursor.len()));
    let indent_len = leading_len(before_cursor, is_indent_grapheme);
    match region {
        FormattingRegion::Line { at, end } => {
            if before_cursor.len() < end { return None; }
            let Some(CommentStart::Line { leader, .. }) = profile.match_start(&before_cursor[at..]) else { return None; };
            let trailing = at > indent_len;
            if above && (leader.no_above || leader.first || trailing) { return None; }
            let gap = leading_len(&before_cursor[end..], is_indent_grapheme);
            let prefix = if leader.first {
                " ".repeat(columns(0, &before_cursor[..end + gap], tabstop))
            } else {
                format!("{}{}{}", if trailing { " ".repeat(columns(0, &before_cursor[..at], tabstop)) } else { before_cursor[..at].to_owned() },
                    &before_cursor[at..end], if gap == 0 { " " } else { &before_cursor[end..end + gap] })
            };
            Some(CommentContinuation { prefix, block: None, trailing, closing_prefix: None })
        }
        FormattingRegion::Block { index, origin, opener, closer } => {
            let block = &profile.blocks[index];
            let (_, original) = read_line(snapshot, origin.0)?;
            let original_indent = &original[..origin.1];
            let original_columns = columns(0, original_indent, tabstop);
            let indent = &before_cursor[..indent_len];
            let rest = &before_cursor[indent_len..];
            if let Some(open) = &opener {
                if above || before_cursor.len() < open.end { return None; }
            }
            if closer.as_ref().is_some_and(|close| before_cursor.len() >= close.end) && !above {
                return Some(CommentContinuation { prefix: original[..leading_len(&original, is_indent_grapheme)].to_owned(), block: None, trailing: false, closing_prefix: None });
            }
            if above && if closer.as_ref().is_some_and(|close| close.start == indent_len) { block.end_no_above } else { block.middle_no_above } { return None; }
            let prefix = if let Some(open) = opener {
                let after = &before_cursor[open.end..];
                let gap = leading_len(after, is_indent_grapheme);
                let base = if original_indent.trim().is_empty() { original_indent.to_owned() } else { " ".repeat(original_columns) };
                (if closer.is_none() { middle_convention(snapshot, line + 1, block, language, Some(origin)) } else { None }).unwrap_or_else(||
                    if gap < after.len() { block.body_middle_prefix(&base, &after[..gap], tabstop) } else { block.middle_prefix(&base, tabstop) })
            } else if above && closer.as_ref().is_some_and(|close| close.start == indent_len) {
                let width = columns(0, indent, tabstop);
                let aligned = width.saturating_add_signed(block.end_offset as isize);
                let gap = columns(0, &block.close, tabstop).saturating_sub(columns(0, &block.middle, tabstop))
                    .saturating_sub(block.end_offset.max(0) as usize).max(1);
                format!("{}{}{}", " ".repeat(aligned), block.middle, " ".repeat(gap))
            } else if block.middle_is_whitespace() {
                indent.to_owned()
            } else if block.middle_matches(rest) {
                let end = indent_len + block.middle.len();
                let gap = leading_len(&before_cursor[end..], is_indent_grapheme);
                format!("{}{}", &before_cursor[..end], if gap == 0 { " " } else { &before_cursor[end..end + gap] })
            } else { indent.to_owned() };
            let closing_prefix = if block.automatic_end && closer.is_none() && (block.middle_is_whitespace() || prefix.trim_end_matches([' ', '\t']).ends_with(&block.middle)) {
                let middle_start = if block.middle_is_whitespace() {
                    columns(0, &prefix, tabstop).saturating_sub(columns(0, &block.middle, tabstop))
                } else { columns(0, &prefix[..leading_len(&prefix, is_indent_grapheme)], tabstop) };
                // Vim generated closing replaces the middle at its existing column;
                // end offsets govern opening above a closer, not this replacement.
                let closing_column = middle_start;
                Some(format!("{}{}", " ".repeat(closing_column), block.close))
            } else { None };
            Some(CommentContinuation { prefix, block: Some(index), trailing: false, closing_prefix })
        }
        _ => None,
    }
}

/// One join request shares its bounded custom-block scan across every pair of
/// original lines. Snapshot, language and profile remain fixed for the request.
pub(crate) struct CommentJoinContext<'a> {
    snapshot: &'a HardLineSnapshot,
    profile: &'a CommentProfile,
    language: Option<&'a str>,
    custom: CustomContext,
}

impl<'a> CommentJoinContext<'a> {
    pub(crate) fn new(snapshot: &'a HardLineSnapshot, profile: &'a CommentProfile, language: Option<&'a str>) -> Self {
        Self { snapshot, profile, language, custom: CustomContext::default() }
    }

    /// Strip a leader only when both original lines have compatible owners.
    pub(crate) fn prefix(&mut self, previous_line: usize, previous: &str, next: &str) -> usize {
        let Self { snapshot, profile, language, custom } = self;
        let indent = leading_len(next, is_indent_grapheme);
        let a = formatting_region(custom, snapshot, previous_line, previous, profile, *language, Some(previous.len()));
        let b = formatting_region(custom, snapshot, previous_line + 1, next, profile, *language, None);
        match (a, b) {
            (FormattingRegion::Line { .. }, FormattingRegion::Line { at: next_at, end: next_end }) if next_at == indent => {
                let Some(CommentStart::Line { leader, .. }) = profile.match_start(&next[next_at..]) else { return indent; };
                if !leader.first {
                    next_end + leading_len(&next[next_end..], is_indent_grapheme)
                } else { indent }
            }
            (FormattingRegion::Block { index, origin, opener, closer: None }, FormattingRegion::Block { index: next_index, origin: next_origin, opener: None, closer, .. })
                if index == next_index && origin == next_origin && !closer.as_ref().is_some_and(|range| range.start == indent) => {
                    let block = &profile.blocks[index];
                    if opener.is_none() && block.middle_body_start(previous).is_none() { return indent; }
                    block.middle_body_start(next).map_or(indent, |end| end + leading_len(&next[end..], is_indent_grapheme))
                }
            _ => indent,
        }
    }
}

fn continuation_prefix(
    paragraph: &Paragraph,
    snapshot: &HardLineSnapshot,
    first_index: usize,
    block: Option<&BlockCommentSyntax>,
    second_indent: bool,
    tabstop: usize,
    language: Option<&str>,
) -> String {
    if let Some(second) = paragraph.lines.get(1).filter(|_| (second_indent && paragraph.first().hanging.is_none()) || paragraph.first().opener) {
        return second.text[..second.parsed.word_start].to_owned();
    }
    let line = &paragraph.lines[0];
    let parsed = &line.parsed;
    let hanging = " ".repeat(parsed.hanging.unwrap_or(0));
    let base = if parsed.opener {
        let block = block.expect("an opener line implies block syntax");
        let indent_len = leading_len(&line.text, is_indent_grapheme);
        let inner = &line.text[parsed.base_end..parsed.inner_end];
        let fallback = || block.body_middle_prefix(&line.text[..indent_len], inner, tabstop);
        if parsed.closes {
            fallback()
        } else {
            middle_convention(snapshot, first_index + 1, block, language, parsed.owner).unwrap_or_else(fallback)
        }
    } else {
        line.text[..parsed.inner_end].to_owned()
    };
    format!("{base}{hanging}")
}

struct Word {
    range: Range<usize>,
    line: usize,
}

fn words_of(paragraph: &Paragraph) -> Vec<Word> {
    let mut words = Vec::new();
    for (line_index, line) in paragraph.lines.iter().enumerate() {
        let parsed = &line.parsed;
        let body = &line.text[parsed.word_start..parsed.body_end];
        let mut at = parsed.word_start;
        let mut current: Option<usize> = None;
        for grapheme in body.graphemes(true) {
            if is_gap_grapheme(grapheme) {
                if let Some(start) = current.take() {
                    words.push(Word {
                        range: line.start + start..line.start + at,
                        line: line_index,
                    });
                }
            } else if current.is_none() {
                current = Some(at);
            }
            at += grapheme.len();
        }
        if let Some(start) = current {
            words.push(Word {
                range: line.start + start..line.start + at,
                line: line_index,
            });
        }
    }
    words
}

fn format_paragraph(
    paragraph: &Paragraph,
    continuation: &str,
    text_width: usize,
    tabstop: usize,
    edits: &mut Vec<TextEdit>,
    trailing_white: bool,
    keep_single_letter: bool,
) {
    let words = words_of(paragraph);
    let Some(first_word) = words.first() else {
        return;
    };
    let first_line = &paragraph.lines[0];
    debug_assert_eq!(first_word.line, 0, "a text line has a body word");
    let last_line = &paragraph.lines[paragraph.lines.len() - 1];
    let suffix = last_line
        .parsed
        .suffix_end
        .map(|end| last_line.text[last_line.parsed.body_end..end].to_owned())
        .unwrap_or_default();
    let continuation_columns = columns(0, continuation, tabstop);
    let mut column = columns(0, &first_line.text[..first_line.parsed.word_start], tabstop);
    let word_text = |word: &Word| -> &str {
        let line = &paragraph.lines[word.line];
        &line.text[word.range.start - line.start..word.range.end - line.start]
    };
    column = columns(column, word_text(first_word), tabstop);
    for index in 1..words.len() {
        let previous = &words[index - 1];
        let word = &words[index];
        let gap_range = previous.range.end..word.range.start;
        let original_gap = if previous.line == word.line {
            let line = &paragraph.lines[word.line];
            line.text[gap_range.start - line.start..gap_range.end - line.start].to_owned()
        } else {
            let mut gap = String::new();
            for (line_index, line) in paragraph.lines.iter().enumerate() {
                if line_index == previous.line {
                    gap.push_str(&line.text[previous.range.end - line.start..]);
                } else if line_index > previous.line && line_index < word.line {
                    gap.push('\n');
                    gap.push_str(&line.text);
                } else if line_index == word.line {
                    gap.push('\n');
                    gap.push_str(&line.text[..word.range.start - line.start]);
                }
            }
            gap
        };
        let joined_gap = if previous.line == word.line {
            original_gap.as_str()
        } else {
            " "
        };
        let is_last = index + 1 == words.len();
        let after_gap = columns(column, joined_gap, tabstop);
        let mut end = columns(after_gap, word_text(word), tabstop);
        if is_last {
            end = columns(end, &suffix, tabstop);
        } else if keep_single_letter && word_text(word).graphemes(true).count() == 1
            && word_text(previous).graphemes(true).count() != 1 {
            // A run of one-letter words belongs with its following word. Only
            // scan at the start of each run, keeping the formatter linear.
            let mut following = index + 1;
            loop {
                let before = &words[following - 1];
                let next = &words[following];
                let gap = if before.line == next.line {
                    let line = &paragraph.lines[next.line];
                    &line.text[before.range.end - line.start..next.range.start - line.start]
                } else { " " };
                end = columns(columns(end, gap, tabstop), word_text(next), tabstop);
                if following + 1 == words.len() {
                    end = columns(end, &suffix, tabstop);
                    break;
                }
                if word_text(next).graphemes(true).count() != 1 { break; }
                following += 1;
            }
        }
        let replacement = if end > text_width
            && !(keep_single_letter && word_text(previous).graphemes(true).count() == 1) {
            column = columns(continuation_columns, word_text(word), tabstop);
            format!("{}\n{continuation}", if trailing_white { " " } else { "" })
        } else {
            column = columns(after_gap, word_text(word), tabstop);
            joined_gap.to_owned()
        };
        if replacement != original_gap {
            edits.push(TextEdit::new(gap_range, replacement));
        }
    }
}

/// Compute the minimal patches that reflow every ordinary text paragraph and
/// recognized comment paragraph within the requested hard lines. An identical
/// result yields no edits. Lines outside the range are read only as bounded
/// comment context and are never edited.
pub fn reflow_edits(request: &ReflowRequest<'_>) -> Result<Vec<TextEdit>, ReflowError> {
    if !request.format.is_literal() {
        return Err(ReflowError::UnsupportedFormat(request.format));
    }
    if request.text_width == 0 || request.tabstop == 0 {
        return Err(ReflowError::InvalidTextWidth);
    }
    let text_width =
        usize::try_from(request.text_width).map_err(|_| ReflowError::InvalidTextWidth)?;
    let line_count = request.snapshot.line_count();
    let lines = request.lines.clone();
    if lines.start > lines.end || lines.end > line_count {
        return Err(ReflowError::InvalidLineRange {
            start: lines.start,
            end: lines.end,
            line_count,
        });
    }
    let code = request.format.is_code();
    if code { super::format_options::normalize_format_options(request.options.flags()).map_err(ReflowError::InvalidOptions)?; }
    let configured_profile = CommentProfile::parse(request.options.comments()).map_err(ReflowError::InvalidOptions)?;
    let profile = (code && request.options.has('q') && !request.options.comments().is_empty()).then_some(&configured_profile);
    let second_indent = code && request.options.has('2') && request.autoindent;
    let trailing_white = code && request.options.has('w');
    let keep_single_letter = code && request.options.has('1');
    let list_pattern = (code && request.options.has('n') && request.autoindent)
        .then(|| ListPattern::compile(request.options.list_pattern())).transpose().map_err(ReflowError::InvalidOptions)?;
    let mut list_fuel = 8_000_000;
    let mut classifier = Classifier {
        profile, tabstop: request.tabstop as usize,
        in_block: None,
    };
    let mut edits = Vec::new();
    let mut current: Option<(usize, Paragraph)> = None;
    let flush = |current: &mut Option<(usize, Paragraph)>, edits: &mut Vec<TextEdit>| {
        if let Some((first_index, paragraph)) = current.take() {
            let block = match paragraph.first().key { PrefixKey::Block(index) => profile.and_then(|p| p.blocks.get(index)), _ => None };
            let continuation = continuation_prefix(&paragraph, request.snapshot, first_index, block, second_indent || !code, request.tabstop as usize, request.options.language());
            format_paragraph(&paragraph, &continuation, text_width, request.tabstop as usize, edits, trailing_white, keep_single_letter);
        }
    };
    let mut custom = CustomContext::default();
    for index in lines {
        let (start, text) = read_line(request.snapshot, index).ok_or(ReflowError::Unreadable)?;
        let region = profile.map(|profile| formatting_region(&mut custom, request.snapshot, index, &text, profile, request.options.language(), None));
        let owner = match &region { Some(FormattingRegion::Block { origin, .. }) => Some(*origin), _ => None };
        let kind = match region {
            Some(FormattingRegion::Unknown) => return Err(ReflowError::UnknownCommentContext),
            Some(FormattingRegion::Fixed) => Kind::Fixed,
            Some(FormattingRegion::Plain) => plain(&text, request.tabstop as usize),
            Some(FormattingRegion::Line { at, .. }) if at != leading_len(&text, is_indent_grapheme) => Kind::Fixed,
            Some(FormattingRegion::Block { index: block, opener, closer, origin }) => {
                if closer.as_ref().is_some_and(|range| !text[range.clone()].ends_with(&configured_profile.blocks[block].close) || configured_profile.blocks[block].find_close(&text[range.start..]) != Some(0)) {
                    Kind::Fixed
                } else if opener.as_ref().is_some_and(|range| range.start != leading_len(&text, is_indent_grapheme)) {
                    Kind::Fixed
                } else {
                    classifier.in_block = (origin.0 != index).then_some(block);
                    classifier.classify(&text)
                }
            }
            _ => { classifier.in_block = None; classifier.classify(&text) }
        };
        match kind {
            Kind::Separator | Kind::Fixed => flush(&mut current, &mut edits),
            Kind::Text(mut parsed) => {
                parsed.owner = owner;
                if parsed.hanging.is_none() {
                    let marker_end = if let Some(pattern) = &list_pattern {
                        pattern.prefix_len(&text[parsed.base_end..parsed.body_end], &mut list_fuel)
                            .map_err(ReflowError::InvalidOptions)?.map(|length| parsed.base_end + length)
                    } else if !code { list_marker(&text[parsed.inner_end..parsed.body_end]).map(|length| parsed.inner_end + length) }
                    else { None };
                    if let Some(end) = marker_end.filter(|end| *end >= parsed.inner_end) {
                        parsed.word_start = end;
                        parsed.hanging = Some(columns(0, &text[..parsed.word_start], request.tabstop as usize) - columns(0, &text[..parsed.inner_end], request.tabstop as usize));
                    }
                }
                let line = Line {
                    start,
                    text,
                    parsed,
                };
                match current.as_mut() {
                    Some((_, paragraph)) if paragraph.accepts(&line.parsed, second_indent, trailing_white) => {
                        paragraph.lines.push(line);
                    }
                    _ => {
                        flush(&mut current, &mut edits);
                        current = Some((index, Paragraph { lines: vec![line] }));
                    }
                }
            }
        }
    }
    flush(&mut current, &mut edits);
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, FileFormat};

    fn reflow_document(
        document: &mut Document,
        lines: Range<usize>,
        width: u32,
        language: Option<&str>,
    ) {
        let snapshot = document.hard_line_snapshot();
        let mut options = ReflowOptions { format_options: Some("qjn2".into()), format_list_pattern: Some(r"^\s*\%([-+*]\|\d\+[.)]\)\s\+".into()), ..Default::default() };
        options.set_language(language);
        let edits = reflow_edits(&ReflowRequest {
            snapshot: &snapshot,
            format: document.format(),
            lines,
            text_width: width,
            tabstop: 8,
            options: &options,
            autoindent: true,
        })
        .unwrap();
        if !edits.is_empty() {
            document.apply_edits(edits).unwrap();
        }
    }

    fn text(input: &str, width: u32) -> String {
        let mut document = Document::new(input);
        let count = document.line_count();
        reflow_document(&mut document, 0..count, width, None);
        document.text().to_owned()
    }

    fn code(input: &str, width: u32, language: &str) -> String {
        let mut document =
            Document::from_bytes(input.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let count = document.line_count();
        reflow_document(&mut document, 0..count, width, Some(language));
        document.text().to_owned()
    }

    #[test]
    fn configured_tabstop_controls_reflow_without_font_geometry() {
        let mut document = Document::from_bytes(b"\talpha beta gamma".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let edits = reflow_edits(&ReflowRequest { snapshot: &snapshot, format: Format::Code,
            lines: 0..1, text_width: 13, tabstop: 2, options: &crate::document::ReflowOptions::default(), autoindent: true }).unwrap();
        document.apply_edits(edits).unwrap();
        assert_eq!(document.text(), "\talpha beta\n\tgamma");
        let mut document = Document::from_bytes(b"\talpha beta gamma".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let edits = reflow_edits(&ReflowRequest { snapshot: &snapshot, format: Format::Code,
            lines: 0..1, text_width: 13, tabstop: 8, options: &crate::document::ReflowOptions::default(), autoindent: true }).unwrap();
        document.apply_edits(edits).unwrap();
        assert_eq!(document.text(), "\talpha\n\tbeta\n\tgamma");
    }

    #[test]
    fn column_policy_uses_tab_stops_and_unicode_widths() {
        assert_eq!(end_column(0, "\t"), 8);
        assert_eq!(end_column(3, "\t"), 8);
        assert_eq!(end_column(8, "\t"), 16);
        assert_eq!(end_column(0, "漢字"), 4);
        assert_eq!(end_column(0, "e\u{301}"), 1);
        assert_eq!(end_column(0, "abc"), 3);
    }

    #[test]
    fn spec_example_joins_and_wraps_a_line_comment_paragraph() {
        assert_eq!(
            code(
                "// One paragraph split across\n// several short lines.\n",
                40,
                "cpp"
            ),
            "// One paragraph split across several\n// short lines.\n"
        );
    }

    #[test]
    fn plain_paragraphs_join_wrap_and_keep_separators_and_indentation() {
        assert_eq!(
            text("aaa bbb\nccc\n\n  ddd eee fff ggg\n  hhh\n", 12),
            "aaa bbb ccc\n\n  ddd eee\n  fff ggg\n  hhh\n"
        );
        assert_eq!(
            text("one two three four five six", 9),
            "one two\nthree\nfour five\nsix"
        );
        assert_eq!(text("exactly10 word", 10), "exactly10\nword");
        assert_eq!(text("fits here", 9), "fits here");
    }

    #[test]
    fn identical_results_yield_no_edits_and_keep_bytes() {
        let mut document = Document::new("short line\n\n  indented\n");
        let revision = document.revision();
        let bytes = document.source_bytes();
        reflow_document(&mut document, 0..3, 80, None);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.source_bytes(), bytes);
        assert!(!document.undo());
    }

    #[test]
    fn interior_whitespace_survives_unless_it_becomes_a_break() {
        assert_eq!(text("a  b c", 80), "a  b c");
        assert_eq!(text("aaaa  bbbb", 6), "aaaa\nbbbb");
        assert_eq!(text("trailing   \nnext", 80), "trailing next");
        assert_eq!(text("keep tail   ", 80), "keep tail   ");
    }

    #[test]
    fn differing_indentation_separates_paragraphs() {
        assert_eq!(text("aaa\n  bbb\naaa\n", 80), "aaa\n  bbb\naaa\n");
        assert_eq!(text("\tone\n\ttwo\n", 80), "\tone two\n");
    }

    #[test]
    fn list_items_keep_markers_and_hanging_indentation() {
        assert_eq!(
            text(
                "- first item is fairly long here\n- second\n  continues\n",
                20
            ),
            "- first item is\n  fairly long here\n- second continues\n"
        );
        assert_eq!(
            text(
                "1. alpha beta gamma delta\n2) epsilon\n* star item words\n+ plus\n",
                14
            ),
            "1. alpha beta\n   gamma delta\n2) epsilon\n* star item\n  words\n+ plus\n"
        );
        assert_eq!(
            text(
                "10. numbered wide marker text\n    hanging more words\n",
                18
            ),
            "10. numbered wide\n    marker text\n    hanging more\n    words\n"
        );
        assert_eq!(text("-\nnot a list\n", 80), "- not a list\n");
        assert_eq!(text("- a\nb\n", 80), "- a b\n");
    }

    #[test]
    fn unbreakable_tokens_overflow_without_empty_lines() {
        assert_eq!(
            text(
                "see https://example.com/a/very/long/path/that/overflows now",
                20
            ),
            "see\nhttps://example.com/a/very/long/path/that/overflows\nnow"
        );
        assert_eq!(
            text("        prefix exhausts", 8),
            "        prefix\n        exhausts"
        );
        assert_eq!(text("ab cd", 1), "ab\ncd");
    }

    #[test]
    fn tabs_and_unicode_widths_count_columns_not_bytes() {
        assert_eq!(text("\tab cd ef", 13), "\tab cd\n\tef");
        assert_eq!(text("\tab cd ef", 12), "\tab\n\tcd\n\tef");
        assert_eq!(text("漢字 漢字 漢字", 9), "漢字 漢字\n漢字");
        assert_eq!(
            text("e\u{301}e\u{301} e\u{301}e\u{301} x", 5),
            "e\u{301}e\u{301} e\u{301}e\u{301}\nx"
        );
        assert_eq!(text("a \u{301}b c", 80), "a \u{301}b c");
        assert_eq!(text("aaa \u{301}bb cc", 6), "aaa \u{301}bb\ncc");
    }

    #[test]
    fn partial_spans_format_only_their_lines() {
        let mut document = Document::new("aaa\nbbb\nccc\nddd\n");
        reflow_document(&mut document, 1..3, 80, None);
        assert_eq!(document.text(), "aaa\nbbb ccc\nddd\n");
    }

    #[test]
    fn line_endings_and_final_terminators_are_preserved() {
        let mut document = Document::from_bytes_with_file_format(
            b"aaa bbb\r\nccc\r\nddd\r\n".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Dos,
        )
        .unwrap();
        let count = document.line_count();
        reflow_document(&mut document, 0..count, 7, None);
        assert_eq!(document.source_bytes(), b"aaa bbb\r\nccc ddd\r\n");
        let mut mixed = Document::from_bytes_with_file_format(
            b"one\ntwo\nthree\r\nfour\n".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        reflow_document(&mut mixed, 0..2, 80, None);
        assert_eq!(mixed.source_bytes(), b"one two\nthree\r\nfour\n");
        let mut unterminated = Document::new("aaa\nbbb");
        reflow_document(&mut unterminated, 0..2, 80, None);
        assert_eq!(unterminated.source_bytes(), b"aaa bbb");
    }

    #[test]
    fn all_line_leaders_form_separate_paragraphs() {
        assert_eq!(
            code("// a\n// b\n/// c\n/// d\n//! e\n//! f\n// g\n", 80, "c"),
            "// a b\n/// c d\n//! e f\n// g\n"
        );
        // Unlike the legacy fixture's explicit `2`, the default keeps a
        // change in comment indentation as a paragraph boundary.
        let mut document = Document::from_bytes(b"    // in\n    // dented\n// top\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let mut options = ReflowOptions::default();
        options.set_language(Some("cpp"));
        let edits = reflow_edits(&ReflowRequest { snapshot: &snapshot, format: Format::Code,
            lines: 0..3, text_width: 80, tabstop: 8, options: &options, autoindent: true }).unwrap();
        document.apply_edits(edits).unwrap();
        assert_eq!(document.text(), "    // in dented\n// top\n");
        assert_eq!(
            code("//\n// blank above\n//\n// and below\n", 80, "cpp"),
            "//\n// blank above\n//\n// and below\n"
        );
        assert_eq!(code("//aaaa bbbb\n", 8, "cpp"), "//aaaa\n//bbbb\n");
    }

    #[test]
    fn code_lines_and_trailing_comments_are_boundaries() {
        assert_eq!(
            code("int x; // trailing\nint y; // trailing\n", 80, "c"),
            "int x; // trailing\nint y; // trailing\n"
        );
        assert_eq!(
            code("a = \"//\";\nb = 1;\n", 80, "c"),
            "a = \"//\";\nb = 1;\n"
        );
        assert_eq!(code("int a;\nint b;\n", 80, "c"), "int a; int b;\n");
        assert_eq!(
            code("int a; /* c */\nint b;\n", 80, "c"),
            "int a; /* c */\nint b;\n"
        );
    }

    #[test]
    fn block_comments_keep_delimiters_and_middle_conventions() {
        assert_eq!(
            code("/*\n * one two\n * three\n */\n", 80, "c"),
            "/*\n * one two three\n */\n"
        );
        assert_eq!(
            code("/* one two three four five */\n", 14, "c"),
            "/* one two\n * three four\n * five */\n"
        );
        assert_eq!(
            code("  /* alpha beta\n   * gamma delta epsilon */\n", 16, "cpp"),
            "  /* alpha beta\n   * gamma delta\n   * epsilon */\n"
        );
        assert_eq!(
            code("/*\n   plain one\n   plain two\n*/\n", 80, "c"),
            "/*\n   plain one plain two\n*/\n"
        );
        assert_eq!(
            code(
                "/* first paragraph\n *\n * second paragraph\n */\n",
                80,
                "c"
            ),
            "/* first paragraph\n *\n * second paragraph\n */\n"
        );
        assert_eq!(
            code("/* opener text\n * more */\n", 80, "c"),
            "/* opener text more */\n"
        );
        assert_eq!(
            code("/**/\n/* */\n/**\n **/\n", 80, "c"),
            "/**/\n/* */\n/**\n **/\n"
        );
        assert_eq!(code("/* a */\n/* b */\n", 80, "c"), "/* a */\n/* b */\n");
        assert_eq!(
            code("/* long opener words here\n *\n */\n", 12, "c"),
            "/* long\n * opener\n * words\n * here\n *\n */\n"
        );
    }

    #[test]
    fn block_context_outside_the_span_is_honored() {
        let mut document = Document::from_bytes(
            b"/*\n * one\n * two\n */\nint x;\nint y;\n".to_vec(),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap();
        reflow_document(&mut document, 1..3, 80, Some("c"));
        assert_eq!(document.text(), "/*\n * one two\n */\nint x;\nint y;\n");
        let mut leading = Document::from_bytes(
            b"s = \"/*\"; // x\n * not comment\n * lines\n".to_vec(),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap();
        reflow_document(&mut leading, 1..3, 80, Some("c"));
        assert_eq!(leading.text(), "s = \"/*\"; // x\n * not comment\n * lines\n");
    }

    #[test]
    fn leading_asterisks_outside_block_context_are_ordinary_text() {
        assert_eq!(
            code("* not a\n* comment\n", 80, "c"),
            "* not a\n* comment\n"
        );
        assert_eq!(code(" *x\n *y\n", 80, "c"), " *x *y\n");
    }

    #[test]
    fn lists_inside_comments_keep_hanging_indentation() {
        assert_eq!(
            code(
                "// - item one two three\n//   continued\n// - item two\n",
                16,
                "cpp"
            ),
            "// - item one\n//   two three\n//   continued\n// - item two\n"
        );
        assert_eq!(
            code("/*\n * - alpha beta gamma\n */\n", 12, "c"),
            "/*\n * - alpha\n *   beta\n *   gamma\n */\n"
        );
    }

    #[test]
    fn unsupported_formats_and_widths_are_rejected_without_edits() {
        let document = Document::new("text");
        let snapshot = document.hard_line_snapshot();
        for format in [
            Format::Markdown,
            Format::MarkdownSource,
        ] {
            assert_eq!(
                reflow_edits(&ReflowRequest {
                    snapshot: &snapshot,
                    format,
                    lines: 0..1,
                    text_width: 80,
            tabstop: 8,
                    options: &crate::document::ReflowOptions::default(), autoindent: true,
                })
                .unwrap_err(),
                ReflowError::UnsupportedFormat(format)
            );
        }
        assert_eq!(
            reflow_edits(&ReflowRequest {
                snapshot: &snapshot,
                format: Format::PlainText,
                lines: 0..1,
                text_width: 0,
            tabstop: 8,
                options: &crate::document::ReflowOptions::default(), autoindent: true,
            })
            .unwrap_err(),
            ReflowError::InvalidTextWidth
        );
        assert!(matches!(
            reflow_edits(&ReflowRequest {
                snapshot: &snapshot,
                format: Format::PlainText,
                lines: 0..5,
                text_width: 80,
            tabstop: 8,
                options: &crate::document::ReflowOptions::default(), autoindent: true,
            }),
            Err(ReflowError::InvalidLineRange { .. })
        ));
    }

    #[test]
    fn every_c_family_language_recognizes_its_comment_leaders() {
        for language in ["c", "cpp", "objc", "rust", "swift", "c_sharp", "javascript", "typescript", "tsx", "go", "java", "php"] {

            assert_eq!(
                code("// alpha beta\n// gamma\nx = 1;\n", 20, language),
                "// alpha beta gamma\nx = 1;\n",
                "{language}"
            );
            assert_eq!(
                code("/* alpha beta\n * gamma */\n", 80, language),
                "/* alpha beta gamma */\n",
                "{language}"
            );
        }
        // Rust's inner and outer documentation leaders stay distinct.
        assert_eq!(
            code(
                "/// outer doc\n/// text\n//! inner doc\n//! text\n",
                80,
                "rust"
            ),
            "/// outer doc text\n//! inner doc text\n"
        );
    }

    #[test]
    fn nonmatching_leaders_reflow_as_ordinary_code() {
        for language in ["python", "bash", "vim", "lua", "sql", "ruby", "unknown"] {

            assert_eq!(
                code("// a\n// b\n", 80, language),
                "// a // b\n",
                "{language}"
            );
        }
    }
    #[test]
    fn native_literals_and_trailing_comments_are_preserved_during_reflow() {
        for (language, source) in [
            ("python", "value = \"\"\"\n# alpha beta gamma\n# delta epsilon\n\"\"\"\n"),
            ("cpp", "value = R\"tag(\n// alpha beta gamma\n// delta epsilon\n)tag\";\n"),
            ("lua", "value = [=[\n-- alpha beta gamma\n-- delta epsilon\n]=]\n"),
            ("bash", "cat <<'EOF'\n# alpha beta gamma\n# delta epsilon\nEOF\n"),
            ("yaml", "value: |\n  # alpha beta gamma\n  # delta epsilon\n"),
            ("python", "x = 1 # alpha beta gamma\nx = 2 # delta epsilon\n"),
        ] {
            assert_eq!(code(source, 12, language), source, "{language}");
        }
    }

    #[test]
    fn nested_native_owners_do_not_merge_or_lose_delimiters() {
        let source = "/* outer alpha\n /* inner beta\n  * gamma */\n * outer delta\n */\n";
        let expected = "/* outer alpha\n /* inner beta gamma */\n * outer delta\n */\n";
        assert_eq!(code(source, 80, "rust"), expected);
        assert_eq!(code("/* a /* b */ c */\n", 5, "rust"), "/* a /* b */ c */\n");
    }

    #[test]
    fn manual_profiles_preserve_native_comments_they_exclude() {
        let mut document = Document::from_bytes(b"// native alpha\n// native beta\n# custom alpha\n# custom beta\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut options = ReflowOptions::default();
        options.set_language(Some("cpp"));
        options.comments = Some(":#".into());
        let edits = reflow_edits(&ReflowRequest { snapshot: &document.hard_line_snapshot(),
            format: Format::Code, lines: 0..4, text_width: 80, tabstop: 8, options: &options, autoindent: true }).unwrap();
        document.apply_edits(edits).unwrap();
        assert_eq!(document.text(), "// native alpha\n// native beta\n# custom alpha custom beta\n");
    }

    #[test]
    fn cross_kind_order_and_custom_block_context_are_shared_with_reflow() {
        let mut document = Document::from_bytes(b"#{\n # alpha beta\n # gamma\n#}\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut options = ReflowOptions::default();
        options.set_language(Some("cpp"));
        options.comments = Some("s1:#{,mb:#,ex:#},:#".into());
        let edits = reflow_edits(&ReflowRequest { snapshot: &document.hard_line_snapshot(),
            format: Format::Code, lines: 1..3, text_width: 80, tabstop: 8, options: &options, autoindent: true }).unwrap();
        document.apply_edits(edits).unwrap();
        assert_eq!(document.text(), "#{\n # alpha beta gamma\n#}\n");
    }

    #[test]
    fn counted_custom_joins_reuse_one_bounded_prefix_scan() {
        let before = "code\n".repeat(1024);
        let body = format!(" # {}", "word ".repeat(20));
        let source = format!("{before}#{{\n{}#}}\n{}", format!("{body}\n").repeat(1024), "unrelated\n".repeat(100_000));
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let mut options = ReflowOptions::default();
        options.comments = Some("s1:#{,mb:#,ex:#}".into());
        let profile = options.profile();
        for language in [Some("cpp"), Some("unknown")] {
            let mut context = CommentJoinContext::new(&snapshot, &profile, language);
            for line in 1025..2048 {
                assert_eq!(context.prefix(line, &body, &body), 3);
            }
            // Counted joins consume each preceding row once, independent of the
            // number of pairs and of the much larger untouched suffix.
            assert_eq!(context.custom.next_line, 2048);
            assert_eq!(context.custom.remaining, 256 * 1024 - 1024 * 4 - 2 - 1023 * body.len());
        }
    }

    #[test]
    fn unsupported_context_rejects_all_prepared_reflow_and_empty_profile_bypasses_it() {
        let source = "// alpha\n// beta\nconst re = /[/*]/;\n// gamma\n// delta\n";
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut options = ReflowOptions::default();
        options.set_language(Some("javascript"));
        let snapshot = document.hard_line_snapshot();
        let request = |options| reflow_edits(&ReflowRequest { snapshot: &snapshot,
            format: Format::Code, lines: 0..5, text_width: 80, tabstop: 8, options, autoindent: true });
        assert_eq!(request(&options).unwrap_err(), ReflowError::UnknownCommentContext);
        assert_eq!(document.source_bytes(), source.as_bytes());
        let empty = ReflowOptions { comments: Some(String::new()), ..options.clone() };
        assert!(request(&empty).is_ok());
    }

    #[test]
    fn text_width_setting_inherits_until_overridden() {
        let mut setting = TextWidthSetting::default();
        assert_eq!(setting.effective(), 80);
        setting.set_default(72);
        assert_eq!(setting.effective(), 72);
        setting.set_local(Some(60));
        setting.set_default(100);
        assert_eq!(setting.effective(), 60);
        assert_eq!(setting.local_override(), Some(60));
        setting.set_local(None);
        assert_eq!(setting.effective(), 100);
    }
}
