//! Simplified Markdown inside Code comments.
//!
//! Only character syntax is recognized: emphasis, strong, strikethrough,
//! links and code spans. A backtick fence is presented as a code span that
//! crosses comment lines; it has no block layout. Block structure never
//! applies, so a comment leader cannot begin a list, heading or quote.
//!
//! Each comment's prose is assembled into a virtual text with a segment map.
//! Leaders, block middles and closers never enter that text, so recognized
//! spans may continue across lines without covering comment markers. The
//! bounded lexical comment context identifies comments; recognition never
//! depends on syntax highlighting and never changes source.
use super::comment_context::{CommentContextStatus, NativeCommentKind};
use super::format_options::{CommentProfile, CommentStart};
use super::HardLineSnapshot;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use std::{collections::BTreeMap, ops::Range};

/// Lines inspected beyond the requested range to find a comment's extent.
const MAX_CONTEXT_LINES: usize = 512;
/// Requested lines beyond this are not recognized in one call.
pub(crate) const MAX_REQUEST_LINES: usize = 2048;
/// A comment with more prose than this is presented without Markdown.
const MAX_COMMENT_BYTES: usize = 64 * 1024;
/// Code regions per call; later regions keep their comment presentation.
pub(crate) const MAX_CODE_REGIONS: usize = 64;
/// Role spans per call.
const MAX_ROLES: usize = 16 * 1024;

/// Buffer-owned `commentmarkdown`: the application default plus an optional
/// buffer override shared by every view. It is presentation configuration and
/// never enters source or document undo. The application default is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommentMarkdownSetting {
    default: bool,
    local: Option<bool>,
}

impl Default for CommentMarkdownSetting {
    fn default() -> Self {
        Self { default: true, local: None }
    }
}

impl CommentMarkdownSetting {
    pub fn effective(&self) -> bool {
        self.local.unwrap_or(self.default)
    }
    pub fn default_value(&self) -> bool {
        self.default
    }
    pub fn local_override(&self) -> Option<bool> {
        self.local
    }
    pub fn set_default(&mut self, enabled: bool) {
        self.default = enabled;
    }
    pub fn set_local(&mut self, enabled: Option<bool>) {
        self.local = enabled;
    }
}

/// A character treatment layered over a comment's own syntax style.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CommentMarkdownRole {
    Strong,
    Emphasis,
    Strikethrough,
    LinkLabel,
    LinkUrl,
    /// Code text; its own language highlighting is layered above this.
    Code,
    /// Emphasis markers, link punctuation, backticks and fence languages.
    Delimiter,
}

impl CommentMarkdownRole {
    /// The Code stylesheet name supplying this role's sparse declarations.
    pub fn style_name(self) -> &'static str {
        match self {
            Self::Strong => "Markup.strong",
            Self::Emphasis => "Markup.italic",
            Self::Strikethrough => "Markup.strikethrough",
            Self::LinkLabel => "Markup.link.label",
            Self::LinkUrl => "Markup.link.url",
            Self::Code => "Markup.raw",
            Self::Delimiter => "Markup.delimiter",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentRoleSpan {
    pub range: Range<usize>,
    pub role: CommentMarkdownRole,
}

/// Code text inside a comment, which may cross comment lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentCodeRegion {
    /// Document ranges of the code text, in order and each within one hard
    /// line. An empty segment is a blank code line.
    pub segments: Vec<Range<usize>>,
    /// A fence's declared language; `None` uses the document's language.
    pub language: Option<String>,
    /// The segments' text joined by LF.
    pub text: String,
}

impl CommentCodeRegion {
    /// Document ranges covered by a range of `text`. LF joiners have none.
    pub fn document_ranges(&self, range: Range<usize>) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut at = 0usize;
        for segment in &self.segments {
            if at >= range.end {
                break;
            }
            let end = at + segment.len();
            let start = range.start.max(at);
            let stop = range.end.min(end);
            if start < stop {
                ranges.push(segment.start + start - at..segment.start + stop - at);
            }
            at = end + 1;
        }
        ranges
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommentMarkdown {
    /// Role spans in cascade order: an enclosing construct precedes the
    /// constructs nested inside it, and delimiters follow their construct.
    pub roles: Vec<CommentRoleSpan>,
    pub code: Vec<CommentCodeRegion>,
    /// Hard lines inspected, including the complete extent of every comment
    /// that intersects the request when it fits the context bound.
    pub lines: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Owner {
    /// Full-line comments sharing indentation and leader spelling.
    Lines { indent: String, leader: String },
    /// A line comment after code. It never joins its neighbours.
    Trailing,
    /// One native block comment, identified by its opener location.
    Block((usize, usize)),
    /// A line comment continued from the previous line by a backslash.
    Continued,
}

#[derive(Clone, Debug)]
struct Piece {
    line: usize,
    owner: Owner,
    /// Document range of this line's prose, after leaders and middles and
    /// before any closer.
    body: Range<usize>,
    text: String,
    closes: bool,
}

struct Recognizer<'a> {
    snapshot: &'a HardLineSnapshot,
    language: Option<&'a str>,
    profile: &'a CommentProfile,
    pieces: BTreeMap<usize, Option<Vec<Piece>>>,
    middles: BTreeMap<(usize, usize), Option<String>>,
}

impl super::Document {
    /// Recognize simplified comment Markdown in this Code revision using the
    /// buffer's effective `comments` profile and canonical language. Unknown
    /// or unsupported lexical context yields no Markdown.
    pub(crate) fn recognize_comment_markdown(
        &self,
        options: &super::ReflowOptions,
        lines: Range<usize>,
    ) -> CommentMarkdown {
        if !self.format().is_code() {
            return CommentMarkdown::default();
        }
        let Ok(profile) = CommentProfile::parse(options.comments()) else {
            return CommentMarkdown::default();
        };
        recognize(&self.hard_line_snapshot(), options.language(), &profile, lines)
    }
}

/// Recognize simplified Markdown in the comments intersecting `lines`.
pub fn recognize(
    snapshot: &HardLineSnapshot,
    language: Option<&str>,
    profile: &CommentProfile,
    lines: Range<usize>,
) -> CommentMarkdown {
    let count = snapshot.line_count();
    let start = lines.start.min(count);
    let end = lines.end.min(count).min(start.saturating_add(MAX_REQUEST_LINES));
    if start >= end {
        return CommentMarkdown { lines: start..start, ..Default::default() };
    }
    let mut recognizer = Recognizer {
        snapshot,
        language,
        profile,
        pieces: BTreeMap::new(),
        middles: BTreeMap::new(),
    };
    let first = recognizer.extend_backward(start);
    let last = recognizer.extend_forward(end, count);
    let mut groups: Vec<Vec<Piece>> = Vec::new();
    let mut open: Option<usize> = None;
    for line in first..last {
        let Some(pieces) = recognizer.pieces(line).cloned() else {
            open = None;
            continue;
        };
        if pieces.is_empty() {
            open = None;
        }
        for piece in pieces {
            let continues = open.is_some_and(|index| continues(&groups[index], &piece));
            match open.filter(|_| continues) {
                Some(index) => groups[index].push(piece),
                None => {
                    groups.push(vec![piece]);
                    open = Some(groups.len() - 1);
                }
            }
        }
    }
    let mut output = CommentMarkdown { lines: first..last, ..Default::default() };
    for group in groups {
        if group.iter().any(|piece| (start..end).contains(&piece.line)) {
            present(&group, &mut output);
        }
    }
    output
}

fn continues(group: &[Piece], next: &Piece) -> bool {
    let previous = group.last().expect("groups are nonempty");
    if previous.closes {
        return false;
    }
    match (&previous.owner, &next.owner) {
        (Owner::Block(a), Owner::Block(b)) => a == b,
        (Owner::Lines { .. } | Owner::Continued, Owner::Continued) => previous.line + 1 == next.line,
        (Owner::Lines { .. }, Owner::Lines { .. }) => {
            group[0].owner == next.owner && previous.line + 1 == next.line
        }
        _ => false,
    }
}

impl Recognizer<'_> {
    fn pieces(&mut self, line: usize) -> Option<&Vec<Piece>> {
        if !self.pieces.contains_key(&line) {
            let pieces = self.read_pieces(line);
            self.pieces.insert(line, pieces);
        }
        self.pieces.get(&line).and_then(Option::as_ref)
    }

    fn extend_backward(&mut self, start: usize) -> usize {
        let mut first = start;
        let floor = start.saturating_sub(MAX_CONTEXT_LINES);
        while first > floor {
            let Some(head) = self.pieces(first).and_then(|pieces| pieces.first()).cloned() else {
                break;
            };
            match head.owner {
                Owner::Block(origin) if origin.0 < first => {
                    first = origin.0.max(floor);
                    break;
                }
                Owner::Lines { .. } | Owner::Continued => {
                    let previous = self
                        .pieces(first - 1)
                        .and_then(|pieces| pieces.last())
                        .cloned();
                    if !previous.is_some_and(|previous| continues(&[previous], &head)) {
                        break;
                    }
                    first -= 1;
                }
                _ => break,
            }
        }
        first
    }

    fn extend_forward(&mut self, end: usize, count: usize) -> usize {
        let mut last = end;
        let ceiling = end.saturating_add(MAX_CONTEXT_LINES).min(count);
        while last < ceiling {
            let Some(tail) = self.pieces(last - 1).and_then(|pieces| pieces.last()).cloned() else {
                break;
            };
            let Some(next) = self.pieces(last).and_then(|pieces| pieces.first()).cloned() else {
                break;
            };
            if !continues(&[tail], &next) {
                break;
            }
            last += 1;
        }
        last
    }

    fn read_pieces(&mut self, line: usize) -> Option<Vec<Piece>> {
        let context = self.snapshot.comment_context(self.language, line);
        if context.status != CommentContextStatus::Known {
            return None;
        }
        if context.comments.is_empty() {
            return Some(Vec::new());
        }
        let range = self.snapshot.line(line)?.content_range();
        let text = self.snapshot.slice_utf8(range.clone()).ok()?;
        let mut pieces = Vec::new();
        for span in &context.comments {
            // Nested native comments keep their enclosing comment's prose.
            if span.depth > 1 {
                continue;
            }
            let (owner, body, closes) = match span.kind {
                NativeCommentKind::Line => match span.opener.clone() {
                    None => (Owner::Continued, span.body.clone(), false),
                    Some(opener) => {
                        // A configured leader such as `///` extends the native
                        // `//`; a profile that excludes it leaves the native one.
                        let leader_end = match self.profile.match_start(&text[opener.start..]) {
                            Some(CommentStart::Line { end, .. }) => {
                                (opener.start + end).clamp(span.body.start, span.range.end)
                            }
                            _ => span.body.start,
                        };
                        let indent = &text[..span.range.start];
                        if indent.trim_matches([' ', '\t']).is_empty() {
                            let leader = text[opener.start..leader_end].to_owned();
                            (
                                Owner::Lines { indent: indent.to_owned(), leader },
                                leader_end..span.range.end,
                                false,
                            )
                        } else {
                            (Owner::Trailing, leader_end..span.range.end, true)
                        }
                    }
                },
                NativeCommentKind::Block => {
                    let Some(origin) = span.block_start else {
                        continue;
                    };
                    let mut body = span.body.clone();
                    if let Some(opener) = span.opener.clone() {
                        // `/**` and `/*!` documentation markers are not prose.
                        if &text[opener] == "/*" {
                            let markers = text[body.clone()]
                                .bytes()
                                .take_while(|byte| matches!(byte, b'*' | b'!'))
                                .count();
                            body.start += markers;
                        }
                    } else if let Some(middle) = self.block_middle(origin) {
                        body.start = strip_middle(&text, body.clone(), &middle);
                    }
                    (Owner::Block(origin), body, span.closer.is_some())
                }
            };
            pieces.push(Piece {
                line,
                owner,
                body: range.start + body.start..range.start + body.end,
                text: text[body].to_owned(),
                closes,
            });
        }
        Some(pieces)
    }

    /// The decorative middle of the block opened at `origin`, from the
    /// configured profile, or `*` for an unconfigured C-style block.
    fn block_middle(&mut self, origin: (usize, usize)) -> Option<String> {
        if let Some(middle) = self.middles.get(&origin) {
            return middle.clone();
        }
        let middle = (|| {
            let range = self.snapshot.line(origin.0)?.content_range();
            let text = self.snapshot.slice_utf8(range).ok()?;
            let rest = text.get(origin.1..)?;
            match self.profile.match_start(rest) {
                Some(CommentStart::Block { syntax, .. }) => {
                    (!syntax.middle_is_whitespace()).then(|| syntax.middle.clone())
                }
                _ => rest.starts_with("/*").then(|| "*".to_owned()),
            }
        })();
        self.middles.insert(origin, middle.clone());
        middle
    }
}

/// Skip a block middle and its indentation. A middle must be followed by a
/// blank or end the body, so `**bold**` at a line start remains prose.
fn strip_middle(text: &str, body: Range<usize>, middle: &str) -> usize {
    let rest = &text[body.clone()];
    let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let after = &rest[indent..];
    match after.strip_prefix(middle) {
        Some(tail) if tail.is_empty() || tail.starts_with([' ', '\t']) => {
            body.start + indent + middle.len()
        }
        _ => body.start,
    }
}

enum Row<'a> {
    /// Prose available to inline recognition.
    Prose(&'a Piece),
    /// A paragraph boundary: a fence row or a gap in a block comment.
    Break,
}

struct Fence {
    ticks: usize,
    indent: usize,
    language: Option<String>,
    segments: Vec<Range<usize>>,
    text: String,
}

fn present(group: &[Piece], output: &mut CommentMarkdown) {
    if group.iter().map(|piece| piece.text.len()).sum::<usize>() > MAX_COMMENT_BYTES {
        return;
    }
    let mut rows = Vec::with_capacity(group.len());
    let mut fence: Option<Fence> = None;
    for (index, piece) in group.iter().enumerate() {
        if index > 0 && piece.line > group[index - 1].line + 1 {
            rows.push(Row::Break);
        }
        // A line holding only a block closer contributes no prose or code.
        if piece.closes
            && matches!(piece.owner, Owner::Block(_))
            && piece.text.trim_matches([' ', '\t']).is_empty()
        {
            continue;
        }
        if let Some(open) = &mut fence {
            if let Some(ticks) = fence_close(&piece.text, open.ticks) {
                let closed = fence.take().expect("open fence");
                let at = piece.body.start;
                push_role(output, at + ticks.start..at + ticks.end, CommentMarkdownRole::Delimiter);
                finish_fence(closed, output);
            } else {
                let indent = piece
                    .text
                    .bytes()
                    .take(open.indent)
                    .take_while(|byte| matches!(byte, b' ' | b'\t'))
                    .count();
                if !open.segments.is_empty() {
                    open.text.push('\n');
                }
                open.text.push_str(&piece.text[indent..]);
                open.segments.push(piece.body.start + indent..piece.body.end);
            }
            rows.push(Row::Break);
            continue;
        }
        if let Some((indent, ticks, info)) = fence_open(&piece.text) {
            let at = piece.body.start;
            push_role(output, at + indent..at + indent + ticks, CommentMarkdownRole::Delimiter);
            if !info.is_empty() {
                push_role(output, at + info.start..at + info.end, CommentMarkdownRole::Delimiter);
            }
            let language = piece.text[info]
                .split([' ', '\t', ','])
                .next()
                .filter(|word| !word.is_empty())
                .map(super::syntax::detection::canonical_language);
            fence = Some(Fence { ticks, indent, language, segments: Vec::new(), text: String::new() });
            rows.push(Row::Break);
            continue;
        }
        rows.push(Row::Prose(piece));
    }
    // The end of the enclosing comment implicitly closes a fence.
    if let Some(open) = fence {
        finish_fence(open, output);
    }
    present_inline(&rows, output);
}

fn finish_fence(mut fence: Fence, output: &mut CommentMarkdown) {
    // Blank comment lines before an implicit close are not code.
    while fence.segments.last().is_some_and(|segment| segment.is_empty()) {
        fence.segments.pop();
        fence.text.truncate(fence.text.rfind('\n').unwrap_or(0));
    }
    for segment in fence.segments.iter().filter(|segment| !segment.is_empty()) {
        push_role(output, segment.clone(), CommentMarkdownRole::Code);
    }
    if output.code.len() < MAX_CODE_REGIONS && fence.segments.iter().any(|s| !s.is_empty()) {
        output.code.push(CommentCodeRegion {
            segments: fence.segments,
            language: fence.language,
            text: fence.text,
        });
    }
}

/// A line opening a backtick fence: indentation, three or more backticks,
/// then an optional info string without backticks.
fn fence_open(text: &str) -> Option<(usize, usize, Range<usize>)> {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let ticks = text[indent..].bytes().take_while(|byte| *byte == b'`').count();
    if ticks < 3 {
        return None;
    }
    let after = indent + ticks;
    if text[after..].contains('`') {
        return None;
    }
    let info_start = after + (text[after..].len() - text[after..].trim_start_matches([' ', '\t']).len());
    let info_end = info_start + text[info_start..].trim_end_matches([' ', '\t']).len();
    Some((indent, ticks, info_start..info_end))
}

/// The backticks of a line closing a fence of `ticks` backticks.
fn fence_close(text: &str, ticks: usize) -> Option<Range<usize>> {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let count = text[indent..].bytes().take_while(|byte| *byte == b'`').count();
    (count >= ticks && text[indent + count..].trim_matches([' ', '\t']).is_empty())
        .then_some(indent..indent + count)
}

fn push_role(output: &mut CommentMarkdown, range: Range<usize>, role: CommentMarkdownRole) {
    if !range.is_empty() && output.roles.len() < MAX_ROLES {
        output.roles.push(CommentRoleSpan { range, role });
    }
}

/// Virtual prose: each nonblank row is prefixed by `x ` so no row can begin
/// Markdown block structure. The prefix and the LF joiners map to nothing.
struct Prose {
    text: String,
    /// (virtual start, length, document start) for each row's prose.
    map: Vec<(usize, usize, usize)>,
}

impl Prose {
    /// Each row's part of a virtual range, as (virtual, document) ranges.
    fn pieces(&self, range: Range<usize>) -> Vec<(Range<usize>, Range<usize>)> {
        let first = self.map.partition_point(|(start, length, _)| start + length <= range.start);
        let mut pieces = Vec::new();
        for &(start, length, document) in &self.map[first..] {
            if start >= range.end {
                break;
            }
            let from = range.start.max(start);
            let to = range.end.min(start + length);
            if from < to {
                pieces.push((from..to, document + from - start..document + to - start));
            }
        }
        pieces
    }

    fn document_ranges(&self, range: Range<usize>) -> Vec<Range<usize>> {
        self.pieces(range).into_iter().map(|(_, document)| document).collect()
    }

    fn push(&self, output: &mut CommentMarkdown, range: Range<usize>, role: CommentMarkdownRole) {
        for range in self.document_ranges(range) {
            push_role(output, range, role);
        }
    }
}

fn present_inline(rows: &[Row<'_>], output: &mut CommentMarkdown) {
    let mut prose = Prose { text: String::new(), map: Vec::new() };
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            prose.text.push('\n');
        }
        if let Row::Prose(piece) = row {
            // Leading blanks are comment margin; they never begin structure.
            let body = piece.text.trim_start_matches([' ', '\t']);
            if body.trim_end_matches([' ', '\t']).is_empty() {
                continue;
            }
            let margin = piece.text.len() - body.len();
            prose.text.push_str("x ");
            prose.map.push((prose.text.len(), body.len(), piece.body.start + margin));
            prose.text.push_str(body);
        }
    }
    if prose.map.is_empty() {
        return;
    }
    let text = prose.text.as_str();
    let mut link_depth = 0usize;
    for (event, range) in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        match event {
            Event::Start(Tag::Strong) => delimited(&prose, output, range, 2, CommentMarkdownRole::Strong),
            Event::Start(Tag::Emphasis) => delimited(&prose, output, range, 1, CommentMarkdownRole::Emphasis),
            Event::Start(Tag::Strikethrough) => {
                let width = text[range.clone()].bytes().take_while(|byte| *byte == b'~').count().min(2);
                delimited(&prose, output, range, width, CommentMarkdownRole::Strikethrough);
            }
            Event::Start(Tag::Link { link_type, .. }) => {
                link_depth += 1;
                link(&prose, output, range, link_type);
            }
            Event::End(pulldown_cmark::TagEnd::Link) => link_depth = link_depth.saturating_sub(1),
            Event::Code(_) => code_span(&prose, output, range),
            Event::Text(_) if link_depth == 0 => bare_urls(&prose, output, range),
            _ => {}
        }
    }
}

fn delimited(
    prose: &Prose,
    output: &mut CommentMarkdown,
    range: Range<usize>,
    width: usize,
    role: CommentMarkdownRole,
) {
    if range.len() < width * 2 {
        return;
    }
    prose.push(output, range.clone(), role);
    prose.push(output, range.start..range.start + width, CommentMarkdownRole::Delimiter);
    prose.push(output, range.end - width..range.end, CommentMarkdownRole::Delimiter);
}

fn link(prose: &Prose, output: &mut CommentMarkdown, range: Range<usize>, link_type: LinkType) {
    let text = prose.text.as_str();
    match link_type {
        LinkType::Inline => {
            let Some(label_end) = super::links::markdown_label_end(text, range.start, range.end) else {
                return;
            };
            if !text[label_end..range.end].starts_with("](") || !text[..range.end].ends_with(')') {
                return;
            }
            prose.push(output, range.start + 1..label_end, CommentMarkdownRole::LinkLabel);
            let inner_start = label_end + 2;
            let inner = &text[inner_start..range.end - 1];
            let lead = inner.len() - inner.trim_start().len();
            let destination = &inner[lead..];
            let length = if destination.starts_with('<') {
                destination.find('>').map_or(destination.len(), |end| end + 1)
            } else {
                destination.find(char::is_whitespace).unwrap_or(destination.len())
            };
            let url = inner_start + lead..inner_start + lead + length;
            prose.push(output, url, CommentMarkdownRole::LinkUrl);
            prose.push(output, range.start..range.start + 1, CommentMarkdownRole::Delimiter);
            prose.push(output, label_end..inner_start, CommentMarkdownRole::Delimiter);
            prose.push(output, range.end - 1..range.end, CommentMarkdownRole::Delimiter);
        }
        LinkType::Autolink | LinkType::Email if range.len() >= 2 => {
            prose.push(output, range.start + 1..range.end - 1, CommentMarkdownRole::LinkUrl);
            prose.push(output, range.start..range.start + 1, CommentMarkdownRole::Delimiter);
            prose.push(output, range.end - 1..range.end, CommentMarkdownRole::Delimiter);
        }
        _ => {}
    }
}

fn code_span(prose: &Prose, output: &mut CommentMarkdown, range: Range<usize>) {
    let text = &prose.text[range.clone()];
    let ticks = text.bytes().take_while(|byte| *byte == b'`').count();
    if ticks == 0 || range.len() < ticks * 2 {
        return;
    }
    let content = range.start + ticks..range.end - ticks;
    prose.push(output, range.start..content.start, CommentMarkdownRole::Delimiter);
    prose.push(output, content.end..range.end, CommentMarkdownRole::Delimiter);
    // Each piece lies in its own row, so row prefixes never enter the code.
    let pieces = prose.pieces(content);
    if pieces.is_empty() {
        return;
    }
    for (_, segment) in &pieces {
        push_role(output, segment.clone(), CommentMarkdownRole::Code);
    }
    if output.code.len() >= MAX_CODE_REGIONS {
        return;
    }
    let code = pieces
        .iter()
        .map(|(range, _)| &prose.text[range.clone()])
        .collect::<Vec<_>>()
        .join("\n");
    let segments = pieces.into_iter().map(|(_, segment)| segment).collect();
    output.code.push(CommentCodeRegion { segments, language: None, text: code });
}

/// GFM extended autolinks in ordinary prose.
fn bare_urls(prose: &Prose, output: &mut CommentMarkdown, range: Range<usize>) {
    let text = prose.text.as_str();
    let mut at = range.start;
    while at < range.end {
        let tail = &text[at..range.end];
        let Some(found) = ["http://", "https://", "www."]
            .iter()
            .filter_map(|prefix| tail.find(prefix))
            .min()
        else {
            return;
        };
        let start = at + found;
        match super::markdown_syntax::autolink(text, start, range.end) {
            Some((end, _)) if end > start => {
                prose.push(output, start..end, CommentMarkdownRole::LinkUrl);
                at = end;
            }
            _ => at = start + text[start..].chars().next().map_or(1, char::len_utf8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{format_options::default_comments, Document, Encoding, Format};
    use CommentMarkdownRole::*;

    fn recognize_text(text: &str, language: &str) -> (String, CommentMarkdown) {
        let document =
            Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let profile = CommentProfile::parse(default_comments(language)).unwrap();
        let result = recognize(&snapshot, Some(language), &profile, 0..snapshot.line_count());
        (text.to_owned(), result)
    }

    fn roles(text: &str, result: &CommentMarkdown) -> Vec<(String, CommentMarkdownRole)> {
        result
            .roles
            .iter()
            .map(|span| (text[span.range.clone()].to_owned(), span.role))
            .collect()
    }

    fn has(text: &str, result: &CommentMarkdown, slice: &str, role: CommentMarkdownRole) -> bool {
        roles(text, result).iter().any(|(value, found)| value == slice && *found == role)
    }

    #[test]
    fn strong_crosses_line_comment_leaders_without_covering_them() {
        let (text, result) = recognize_text("// some **bold\n// text** here\nlet x = 1;\n", "rust");
        assert!(has(&text, &result, "**bold", Strong));
        assert!(has(&text, &result, "text**", Strong));
        assert!(has(&text, &result, "**", Delimiter));
        for span in &result.roles {
            assert!(!text[span.range.clone()].contains("//"), "{:?}", &text[span.range.clone()]);
            assert!(!text[span.range.clone()].contains('\n'));
        }
    }

    #[test]
    fn block_middles_doc_markers_and_closers_are_not_prose() {
        let source = "/**\n * Returns the **total\n * count** of _items_.\n */\nfn f() {}\n";
        let (text, result) = recognize_text(source, "rust");
        assert!(has(&text, &result, "**total", Strong));
        assert!(has(&text, &result, "count**", Strong));
        assert!(has(&text, &result, "_items_", Emphasis));
        for span in &result.roles {
            let value = &text[span.range.clone()];
            assert!(!value.contains(" * ") && !value.contains("*/") && !value.contains("/*"), "{value:?}");
        }
    }

    #[test]
    fn block_structure_never_applies() {
        let source = "// # Not a heading\n// - not a list\n// > not a quote\n//     not code\n";
        let (_, result) = recognize_text(source, "rust");
        assert!(result.roles.is_empty(), "{:?}", result.roles);
        assert!(result.code.is_empty());
    }

    #[test]
    fn paragraphs_end_at_blank_comment_lines_and_leader_changes() {
        let (_, result) = recognize_text("// **a\n//\n// b**\n", "rust");
        assert!(result.roles.is_empty());
        let (_, result) = recognize_text("/// **a\n// b**\n", "rust");
        assert!(result.roles.is_empty());
        let (_, result) = recognize_text("x(); // **a\n// b**\n", "rust");
        assert!(result.roles.is_empty());
    }

    #[test]
    fn inline_code_uses_the_document_language_and_maps_across_lines() {
        let (text, result) = recognize_text("// call `foo(\n// bar)` now\n", "rust");
        assert_eq!(result.code.len(), 1);
        let code = &result.code[0];
        assert_eq!(code.language, None);
        assert_eq!(code.text, "foo(\nbar)");
        let pieces: Vec<_> = code.segments.iter().map(|s| &text[s.clone()]).collect();
        assert_eq!(pieces, ["foo(", "bar)"]);
        assert_eq!(code.document_ranges(0..code.text.len()), code.segments);
        assert!(has(&text, &result, "`", Delimiter));
        assert!(has(&text, &result, "foo(", Code));
    }

    #[test]
    fn fences_cross_lines_and_close_explicitly_or_with_the_comment() {
        let source = "// ```python\n// def f():\n//     return 1\n// ```\n// after `x`\n";
        let (text, result) = recognize_text(source, "rust");
        assert_eq!(result.code.len(), 2);
        assert_eq!(result.code[0].language.as_deref(), Some("python"));
        assert_eq!(result.code[0].text, "def f():\n    return 1");
        assert!(has(&text, &result, "```", Delimiter));
        assert!(has(&text, &result, "python", Delimiter));
        assert_eq!(result.code[1].text, "x");

        let (text, result) = recognize_text("/* ```\n * let a = 1;\n */\nlet b = 2;\n", "rust");
        assert_eq!(result.code.len(), 1);
        assert_eq!(result.code[0].language, None);
        assert_eq!(result.code[0].text, "let a = 1;");
        let extent = result.code[0].segments[0].start..result.code[0].segments.last().unwrap().end;
        assert!(!text[extent].contains('*'));
        let (_, result) = recognize_text("// ```\n// let a = 1;\nlet b = 2;\n// `c`\n", "rust");
        assert_eq!(result.code.len(), 2);
        assert_eq!(result.code[0].text, "let a = 1;");
        assert_eq!(result.code[1].text, "c");
    }

    #[test]
    fn fence_markup_is_not_inline_emphasis() {
        let (_, result) = recognize_text("// ```\n// a *b* c\n// ```\n", "rust");
        assert!(!result.roles.iter().any(|span| span.role == Emphasis));
    }

    #[test]
    fn links_autolinks_and_bare_urls() {
        let source = "// See [the docs](https://example.com/a \"t\") or <https://x.org> and https://www.rust-lang.org.\n";
        let (text, result) = recognize_text(source, "rust");
        assert!(has(&text, &result, "the docs", LinkLabel));
        assert!(has(&text, &result, "https://example.com/a", LinkUrl));
        assert!(has(&text, &result, "https://x.org", LinkUrl));
        assert!(has(&text, &result, "https://www.rust-lang.org", LinkUrl));
        assert!(has(&text, &result, "](", Delimiter));
    }

    #[test]
    fn strings_and_code_are_never_comments() {
        let (_, result) = recognize_text("let s = \"// **no**\";\n", "rust");
        assert!(result.roles.is_empty());
        let (text, result) = recognize_text("x = 1  # **yes**\ns = '# **no**'\n", "python");
        assert!(has(&text, &result, "**yes**", Strong));
        assert_eq!(result.roles.iter().filter(|span| span.role == Strong).count(), 1);
    }

    #[test]
    fn requests_extend_to_the_whole_comment() {
        let source = "/*\n * ```\n * a\n * b\n */\n";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let profile = CommentProfile::parse(default_comments("c")).unwrap();
        let result = recognize(&snapshot, Some("c"), &profile, 3..4);
        assert_eq!(result.lines, 0..5);
        assert_eq!(result.code.len(), 1);
        assert_eq!(result.code[0].text, "a\nb");
        let source = "// **a\n// b\n// c**\n";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let profile = CommentProfile::parse(default_comments("rust")).unwrap();
        let result = recognize(&snapshot, Some("rust"), &profile, 1..2);
        assert_eq!(result.lines, 0..3);
        assert_eq!(result.roles.iter().filter(|span| span.role == Strong).count(), 3);
    }

    #[test]
    fn languages_without_a_lexical_provider_are_not_recognized() {
        let (_, result) = recognize_text("# **bold**\n", "ruby");
        assert!(result.roles.is_empty());
    }

    #[test]
    fn recognition_is_bounded_in_large_documents_and_comments() {
        // One giant line-comment run: the request inspects bounded context.
        let source = "// **x** `y`\n".repeat(20_000);
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let profile = CommentProfile::parse(default_comments("rust")).unwrap();
        let result = recognize(&snapshot, Some("rust"), &profile, 10_000..10_050);
        assert!(result.lines.len() <= 50 + 2 * MAX_CONTEXT_LINES, "{:?}", result.lines);
        assert!(result.code.len() <= MAX_CODE_REGIONS);
        assert!(result.roles.len() <= MAX_ROLES);
        // A block comment longer than the context bound degrades to its
        // inspected part rather than scanning to its opener.
        let source = format!("/*\n{} */\n", " * **x**\n".repeat(10_000));
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let snapshot = document.hard_line_snapshot();
        let profile = CommentProfile::parse(default_comments("c")).unwrap();
        let result = recognize(&snapshot, Some("c"), &profile, 9_000..9_010);
        assert!(result.lines.len() <= 10 + 2 * MAX_CONTEXT_LINES, "{:?}", result.lines);
        // Requests themselves are capped.
        let result = recognize(&snapshot, Some("c"), &profile, 0..10_002);
        assert!(result.lines.len() <= MAX_REQUEST_LINES + MAX_CONTEXT_LINES);
    }

    #[test]
    fn unicode_spans_map_to_character_boundaries() {
        let (text, result) = recognize_text("// é **ü\n// ß** `ø`\n", "rust");
        for span in &result.roles {
            assert!(text.is_char_boundary(span.range.start) && text.is_char_boundary(span.range.end));
        }
        assert!(has(&text, &result, "**ü", Strong));
        assert_eq!(result.code[0].text, "ø");
    }
}
