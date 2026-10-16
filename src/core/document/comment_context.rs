//! Bounded lexical comment context, independent of formatting options and highlighting.
//!
//! Coordinates are UTF-8 offsets within one hard line. Checkpoints are attached
//! to an immutable projection and only an unchanged prefix may cross an edit.
use super::{DocumentId, HardLineSnapshot, Revision};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
};

const SCAN_BYTES: usize = 512 * 1024;
const SCAN_LINES: usize = 4096;
const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_PARTS: usize = 256;
const CHECKPOINT_STRIDE: usize = 64;
const MAX_CHECKPOINTS: usize = 256;
const MAX_RESULTS: usize = 32;
const MAX_LANGUAGES: usize = 4;
const MAX_REVISIONS: usize = 4;
const MAX_NESTING: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommentContextStatus {
    Known,
    Unknown,
    Compatibility,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCommentKind {
    Line,
    Block,
}

/// One native comment segment. Nested blocks have their own identity and depth;
/// the enclosing comment resumes with its original identity after they close.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCommentSpan {
    pub kind: NativeCommentKind,
    pub range: Range<usize>,
    pub body: Range<usize>,
    pub opener: Option<Range<usize>>,
    pub closer: Option<Range<usize>>,
    pub expected_closer: Option<String>,
    pub block_start: Option<(usize, usize)>,
    pub depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentLineContext {
    pub status: CommentContextStatus,
    pub comments: Vec<NativeCommentSpan>,
    pub protected: Vec<Range<usize>>,
    line_length: usize,
    protected_to_line_end: bool,
}
impl CommentLineContext {
    fn empty(status: CommentContextStatus, line_length: usize) -> Self {
        Self {
            status,
            comments: Vec::new(),
            protected: Vec::new(),
            line_length,
            protected_to_line_end: false,
        }
    }
    pub fn is_unknown(&self) -> bool {
        self.status == CommentContextStatus::Unknown
    }
    pub fn is_compatibility(&self) -> bool {
        self.status == CommentContextStatus::Compatibility
    }
    /// A caret at an unfinished comment's line end remains inside it; a caret
    /// after a closing delimiter does not. Opening delimiter bytes are included.
    pub fn comment_at(&self, byte: usize) -> Option<&NativeCommentSpan> {
        if self.status != CommentContextStatus::Known {
            return None;
        }
        self.comments.iter().rev().find(|span| {
            span.range.contains(&byte)
                || (byte == self.line_length && span.range.end == byte && span.closer.is_none())
        })
    }
    pub fn is_code_at(&self, byte: usize) -> bool {
        self.status == CommentContextStatus::Known
            && byte <= self.line_length
            && self.comment_at(byte).is_none()
            && !self.protected.iter().any(|range| {
                range.contains(&byte)
                    || (byte == self.line_length && range.end == byte && self.protected_to_line_end)
            })
    }
    pub fn has_native_comment(&self) -> bool {
        !self.comments.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BlockState {
    open: String,
    close: String,
    nested: bool,
    starts: Vec<(usize, usize)>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct StringState {
    close: String,
    escape: bool,
    doubled: bool,
    multiline: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct HereDoc {
    delimiter: String,
    tabs: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Default)]
struct State {
    block: Option<BlockState>,
    string: Option<StringState>,
    heredocs: Vec<HereDoc>,
    yaml_scalar: Option<(usize, Option<usize>)>,
    vim9: bool,
    unknown: bool,
    continued_line_comment: bool,
    xml_in_tag: bool,
}
#[derive(Clone, Debug, Default)]
struct LanguageCache {
    checkpoints: BTreeMap<usize, State>,
    results: BTreeMap<usize, CommentLineContext>,
}
#[derive(Debug, Default)]
pub(super) struct CommentContextCache {
    identity: Option<(DocumentId, Revision)>,
    languages: BTreeMap<String, LanguageCache>,
}
/// All descendants of a projection share one finite arena. Undo history holds
/// lightweight handles, not another full lexical cache for every retained edit.
#[derive(Debug, Default)]
struct CacheArena {
    next: u64,
    slots: BTreeMap<u64, CommentContextCache>,
}
#[derive(Clone, Debug)]
pub(super) struct SharedCommentContextCache {
    arena: Arc<Mutex<CacheArena>>,
    slot: u64,
}
pub(super) fn fresh_cache() -> SharedCommentContextCache {
    SharedCommentContextCache {
        arena: Arc::new(Mutex::new(CacheArena::default())),
        slot: 0,
    }
}

/// Reuse only states established strictly before the first changed hard line.
/// The caller has verified the unchanged source prefix as part of its splice.
pub(super) fn prefix_cache(
    previous: &SharedCommentContextCache,
    before: usize,
) -> SharedCommentContextCache {
    let Ok(mut arena) = previous.arena.lock() else {
        return fresh_cache();
    };
    let Some(slot) = arena.next.checked_add(1) else {
        return fresh_cache();
    };
    let mut next = CommentContextCache::default();
    if let Some(cache) = arena.slots.get(&previous.slot) {
        for (language, source) in &cache.languages {
            next.languages.insert(
                language.clone(),
                LanguageCache {
                    checkpoints: source
                        .checkpoints
                        .range(..=before)
                        .map(|(line, state)| (*line, state.clone()))
                        .collect(),
                    results: source
                        .results
                        .range(..before)
                        .map(|(line, value)| (*line, value.clone()))
                        .collect(),
                },
            );
        }
    }
    arena.next = slot;
    while arena.slots.len() >= MAX_REVISIONS {
        arena.slots.pop_first();
    }
    arena.slots.insert(slot, next);
    SharedCommentContextCache {
        arena: previous.arena.clone(),
        slot,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Family {
    C,
    Rust,
    Go,
    Java,
    Js,
    CSharp,
    Swift,
    Kotlin,
    Scala,
    Groovy,
    Dart,
    Proto,
    Css,
    Scss,
    Json,
    JsonComments,
    Python,
    Hash,
    Toml,
    Graphql,
    Hcl,
    Lua,
    Shell,
    Yaml,
    Sql,
    Haskell,
    Vim,
    Cmake,
    Ini,
    Tex,
    Xml,
    PowerShell,
}
fn family(language: &str) -> Option<Family> {
    Some(match language {
        "c" | "cpp" | "objc" => Family::C,
        "rust" => Family::Rust,
        "go" => Family::Go,
        "java" => Family::Java,
        "javascript" | "typescript" | "tsx" => Family::Js,
        "c_sharp" => Family::CSharp,
        "swift" => Family::Swift,
        "kotlin" => Family::Kotlin,
        "scala" => Family::Scala,
        "groovy" => Family::Groovy,
        "dart" => Family::Dart,
        "proto" => Family::Proto,
        "css" => Family::Css,
        "scss" => Family::Scss,
        "json" => Family::Json,
        "jsonc" | "json5" => Family::JsonComments,
        "python" => Family::Python,
        "r" => Family::Hash,
        "toml" => Family::Toml,
        "graphql" => Family::Graphql,
        "hcl" | "terraform" => Family::Hcl,
        "lua" => Family::Lua,
        "bash" | "sh" | "zsh" | "ksh" | "dash" | "mksh" => Family::Shell,
        "yaml" => Family::Yaml,
        "sql" => Family::Sql,
        "haskell" => Family::Haskell,
        "vim" => Family::Vim,
        "cmake" => Family::Cmake,
        "dosini" => Family::Ini,
        "tex" | "plaintex" => Family::Tex,
        "xml" => Family::Xml,
        "ps1" => Family::PowerShell,
        _ => return None,
    })
}
/// Whether this language has a lexical provider. Unsupported constructs within
/// a provider produce Unknown, never literal-leader compatibility.
pub fn has_comment_context_provider(language: Option<&str>) -> bool {
    language
        .is_some_and(|value| family(&super::syntax::detection::canonical_language(value)).is_some())
}

/// Native opener recognition for distinguishing custom formatting blocks from
/// language blocks. This describes syntax; it never changes the active profile.
pub fn supports_native_block(language: Option<&str>, opener: &str) -> bool {
    let Some(provider) =
        language.and_then(|v| family(&super::syntax::detection::canonical_language(v)))
    else {
        return false;
    };
    match provider {
        Family::Lua => opener
            .strip_prefix("--")
            .is_some_and(|v| long_bracket(v, 0).is_some_and(|(end, _)| end == v.len())),
        Family::Cmake => opener
            .strip_prefix('#')
            .is_some_and(|v| long_bracket(v, 0).is_some_and(|(end, _)| end == v.len())),
        Family::Haskell => opener.starts_with("{-"),
        Family::Xml => opener == "<!--",
        Family::PowerShell => opener == "<#",
        Family::C
        | Family::Rust
        | Family::Go
        | Family::Java
        | Family::Js
        | Family::CSharp
        | Family::Swift
        | Family::Kotlin
        | Family::Scala
        | Family::Groovy
        | Family::Dart
        | Family::Proto
        | Family::Css
        | Family::Scss
        | Family::JsonComments
        | Family::Hcl
        | Family::Sql => opener.starts_with("/*"),
        _ => false,
    }
}

/// Leaders that spell native line-comment syntax must be validated by the
/// lexical provider even when their occurrence is in otherwise ordinary code.
/// For example, `#` in `$#` is not promoted to a custom shell comment.
pub fn supports_native_line(language: Option<&str>, leader: &str) -> bool {
    let Some(provider) =
        language.and_then(|v| family(&super::syntax::detection::canonical_language(v)))
    else {
        return false;
    };
    match provider {
        Family::C
        | Family::Rust
        | Family::Go
        | Family::Java
        | Family::Js
        | Family::CSharp
        | Family::Swift
        | Family::Kotlin
        | Family::Scala
        | Family::Groovy
        | Family::Dart
        | Family::Proto
        | Family::Scss
        | Family::JsonComments => leader.starts_with("//"),
        Family::Hcl => leader.starts_with('#') || leader.starts_with("//"),
        Family::Python
        | Family::Hash
        | Family::Toml
        | Family::Graphql
        | Family::Shell
        | Family::Yaml
        | Family::Cmake
        | Family::PowerShell => leader.starts_with('#'),
        Family::Lua | Family::Haskell | Family::Sql => leader.starts_with("--"),
        Family::Vim => leader.starts_with('"') || leader.starts_with('#'),
        Family::Ini => leader.starts_with(';') || leader.starts_with('#'),
        Family::Tex => leader.starts_with('%'),
        _ => false,
    }
}

impl HardLineSnapshot {
    pub fn comment_context(&self, language: Option<&str>, line: usize) -> CommentLineContext {
        let Some(info) = self.line(line) else {
            return CommentLineContext::empty(CommentContextStatus::Unknown, 0);
        };
        let length = info.content_range().len();
        let language = language
            .map(super::syntax::detection::canonical_language)
            .unwrap_or_default();
        let Some(provider) = family(&language) else {
            return CommentLineContext::empty(CommentContextStatus::Compatibility, length);
        };
        let identity = (self.document(), self.revision());
        let (start, mut state) = {
            let Ok(mut arena) = self.comment_context_cache.arena.lock() else {
                return CommentLineContext::empty(CommentContextStatus::Unknown, length);
            };
            if !arena.slots.contains_key(&self.comment_context_cache.slot) {
                while arena.slots.len() >= MAX_REVISIONS {
                    arena.slots.pop_first();
                }
                arena.slots.insert(
                    self.comment_context_cache.slot,
                    CommentContextCache::default(),
                );
            }
            let cache = arena
                .slots
                .get_mut(&self.comment_context_cache.slot)
                .unwrap();
            if cache.identity != Some(identity) {
                if cache.identity.is_some() {
                    cache.languages.clear();
                }
                cache.identity = Some(identity);
            }
            if cache.languages.len() >= MAX_LANGUAGES && !cache.languages.contains_key(&language) {
                cache.languages.pop_first();
            }
            let language_cache = cache.languages.entry(language.clone()).or_default();
            if let Some(result) = language_cache.results.get(&line) {
                return result.clone();
            }
            language_cache
                .checkpoints
                .range(..=line)
                .next_back()
                .map(|(at, state)| (*at, state.clone()))
                .unwrap_or_default()
        };
        let mut checkpoints = Vec::new();
        let mut results = Vec::new();
        let mut bytes = 0usize;
        let mut answer = CommentLineContext::empty(CommentContextStatus::Unknown, length);
        for at in start..=line {
            if at - start >= SCAN_LINES {
                break;
            }
            let Some(info) = self.line(at) else {
                break;
            };
            let range = info.content_range();
            if range.len() > MAX_LINE_BYTES || bytes.saturating_add(range.len()) > SCAN_BYTES {
                break;
            }
            let Ok(text) = self.slice_utf8(range) else {
                break;
            };
            bytes += text.len();
            super::work_statistics::record(|stats| stats.comment_context_lexed_lines += 1);
            let result = lex_line(provider, &text, at, &mut state);
            if (at + 1) % CHECKPOINT_STRIDE == 0 || at == line || state.unknown {
                checkpoints.push((at + 1, state.clone()));
            }
            if at == line {
                answer = result.clone();
            }
            // Keep recent context, not a context vector proportional to the file.
            if at + MAX_RESULTS >= line {
                results.push((at, result));
            }
            if state.unknown {
                answer = CommentLineContext::empty(CommentContextStatus::Unknown, length);
                break;
            }
        }
        if let Ok(mut arena) = self.comment_context_cache.arena.lock() {
            if let Some(cache) = arena.slots.get_mut(&self.comment_context_cache.slot) {
                if cache.identity == Some(identity) {
                    if let Some(target) = cache.languages.get_mut(&language) {
                        target.checkpoints.extend(checkpoints);
                        target.results.extend(results);
                        // Keep the entries nearest this query. Evicting by
                        // position alone would discard a revisited early
                        // line's new checkpoint at once, so a scan from the
                        // top would restart from the first line every time.
                        trim_farthest(&mut target.checkpoints, MAX_CHECKPOINTS, line);
                        trim_farthest(&mut target.results, MAX_RESULTS, line);
                    }
                }
            }
        }
        answer
    }
}

fn trim_farthest<T>(entries: &mut BTreeMap<usize, T>, limit: usize, line: usize) {
    while entries.len() > limit {
        let (Some(first), Some(last)) = (entries.keys().next().copied(), entries.keys().next_back().copied()) else {
            return;
        };
        if line.abs_diff(first) >= line.abs_diff(last) {
            entries.pop_first();
        } else {
            entries.pop_last();
        }
    }
}

fn indent(text: &str) -> usize {
    text.bytes()
        .take_while(|c| matches!(c, b' ' | b'\t'))
        .count()
}
fn boundary(text: &str, at: usize) -> bool {
    if at == 0 {
        return true;
    }
    let bytes = text.as_bytes();
    let escaped = bytes[..at - 1]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1;
    !escaped
        && (bytes[at - 1].is_ascii_whitespace()
            || matches!(
                bytes[at - 1],
                b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>'
            ))
}
fn long_bracket(text: &str, at: usize) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut end = at + 1;
    while bytes.get(end) == Some(&b'=') && end - at < 256 {
        end += 1;
    }
    (bytes.get(end) == Some(&b'[')).then(|| (end + 1, format!("]{}]", "=".repeat(end - at - 1))))
}
fn unknown(state: &mut State, length: usize) -> CommentLineContext {
    state.unknown = true;
    CommentLineContext::empty(CommentContextStatus::Unknown, length)
}
fn line_comment(result: &mut CommentLineContext, at: usize, width: usize) {
    result.comments.push(NativeCommentSpan {
        kind: NativeCommentKind::Line,
        range: at..result.line_length,
        body: at + width..result.line_length,
        opener: Some(at..at + width),
        closer: None,
        expected_closer: None,
        block_start: None,
        depth: 0,
    });
}
fn start_block(state: &mut State, open: &str, close: &str, nested: bool, line: usize, at: usize) {
    state.block = Some(BlockState {
        open: open.into(),
        close: close.into(),
        nested,
        starts: vec![(line, at)],
    });
}
fn string(close: &str, escape: bool, doubled: bool, multiline: bool) -> StringState {
    StringState {
        close: close.into(),
        escape,
        doubled,
        multiline,
    }
}

fn c_header_operand(text: &str) -> Result<Option<Range<usize>>, ()> {
    let trimmed = text.trim_start();
    let directive = trimmed
        .strip_prefix('#')
        .or_else(|| trimmed.strip_prefix("%:"));
    let (value, preprocessing) = if let Some(directive) = directive {
        (directive.trim_start(), true)
    } else {
        let value = trimmed
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(trimmed);
        (value, false)
    };
    if preprocessing && value.starts_with("/*") {
        return Err(());
    }
    let word_end = value
        .bytes()
        .take_while(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        .count();
    let word = &value[..word_end];
    if !(preprocessing && matches!(word, "include" | "include_next" | "import" | "embed")
        || !preprocessing && word == "import")
    {
        return Ok(None);
    }
    let operand = value[word_end..].trim_start();
    if operand.starts_with("/*") {
        return Err(());
    }
    let close = match operand.as_bytes().first() {
        Some(b'<') => '>',
        Some(b'"') => '"',
        _ => return Ok(None),
    };
    let start = text.len() - operand.len();
    let end = operand[1..].find(close).ok_or(())? + 2;
    Ok(Some(start..start + end))
}

fn lex_line(provider: Family, text: &str, line: usize, state: &mut State) -> CommentLineContext {
    let mut result = CommentLineContext::empty(CommentContextStatus::Known, text.len());
    if state.unknown {
        return unknown(state, text.len());
    }
    let bytes = text.as_bytes();
    if provider == Family::Java && text.contains("\\u")
        || provider == Family::C && text.contains("??/")
    {
        return unknown(state, text.len());
    }
    if state.continued_line_comment {
        result.comments.push(NativeCommentSpan {
            kind: NativeCommentKind::Line,
            range: 0..text.len(),
            body: 0..text.len(),
            opener: None,
            closer: None,
            expected_closer: None,
            block_start: None,
            depth: 0,
        });
        state.continued_line_comment = text.ends_with('\\');
        return result;
    }
    if let Some(here) = state.heredocs.first() {
        let candidate = if here.tabs {
            text.trim_start_matches('\t')
        } else {
            text
        };
        result.protected.push(0..text.len());
        result.protected_to_line_end = true;
        if candidate == here.delimiter {
            state.heredocs.remove(0);
        }
        return result;
    }
    if let Some((base, required)) = state.yaml_scalar {
        let amount = indent(text);
        if text.trim().is_empty() {
            result.protected.push(0..text.len());
            result.protected_to_line_end = true;
            return result;
        }
        let required = required.unwrap_or(amount);
        if amount > base && amount >= required {
            state.yaml_scalar = Some((base, Some(required)));
            result.protected.push(0..text.len());
            result.protected_to_line_end = true;
            return result;
        }
        state.yaml_scalar = None;
    }
    if provider == Family::Vim && text.trim_start().starts_with("vim9script") {
        state.vim9 = true;
    }
    let mut at = 0usize;
    if provider == Family::C && state.block.is_none() && state.string.is_none() {
        match c_header_operand(text) {
            Ok(Some(range)) => {
                at = range.end;
                result.protected.push(range);
            }
            Ok(None) => {}
            Err(()) => return unknown(state, text.len()),
        }
    }
    let mut block_open = None;
    let mut string_start = state.string.as_ref().map(|_| 0usize);
    // A first-line shebang is executable metadata, never an editable comment.
    if line == 0
        && text.starts_with("#!")
        && matches!(
            provider,
            Family::Python | Family::Shell | Family::Lua | Family::Hash | Family::Js
        )
    {
        result.protected.push(0..text.len());
        return result;
    }
    while at < text.len()
        || state.block.is_some() && at == 0
        || state.string.is_some() && string_start.is_some()
    {
        if result.comments.len() + result.protected.len() >= MAX_PARTS {
            return unknown(state, text.len());
        }
        if let Some(block) = &mut state.block {
            let segment_start = at;
            let opener = block_open.take();
            let body_start = opener.as_ref().map_or(at, |r: &Range<usize>| r.end);
            at = body_start;
            let depth = block.starts.len();
            let identity = block.starts.last().copied();
            let mut closer = None;
            while at < text.len() {
                if text[at..].starts_with(&block.close) {
                    closer = Some(at..at + block.close.len());
                    break;
                }
                if block.nested && text[at..].starts_with(&block.open) {
                    break;
                }
                at += text[at..].chars().next().unwrap().len_utf8();
            }
            let body_end = at;
            let end = closer.as_ref().map_or(at, |range| range.end);
            result.comments.push(NativeCommentSpan {
                kind: NativeCommentKind::Block,
                range: segment_start..end,
                body: body_start..body_end,
                opener,
                closer: closer.clone(),
                expected_closer: Some(block.close.clone()),
                block_start: identity,
                depth,
            });
            if let Some(closer) = closer {
                at = closer.end;
                block.starts.pop();
                if block.starts.is_empty() {
                    state.block = None;
                }
            } else if at < text.len() {
                if block.starts.len() >= MAX_NESTING {
                    return unknown(state, text.len());
                }
                block.starts.push((line, at));
                block_open = Some(at..at + block.open.len());
            } else {
                break;
            }
            continue;
        }
        if let Some(current) = &state.string {
            let begin = string_start.take().unwrap_or(at);
            let mut closed = false;
            while at < text.len() {
                if provider == Family::Sql && bytes[at] == b'\\' {
                    return unknown(state, text.len());
                }
                if (matches!(
                    provider,
                    Family::Kotlin | Family::Groovy | Family::Dart | Family::Hcl | Family::Js
                ) && text[at..].starts_with("${"))
                    || (provider == Family::Shell
                        && current.close == "\""
                        && (text[at..].starts_with("$(") || bytes[at] == b'`'))
                    || (provider == Family::PowerShell
                        && current.close == "\""
                        && text[at..].starts_with("$("))
                    || (provider == Family::Swift && text[at..].starts_with("\\("))
                    || (provider == Family::Scss && text[at..].starts_with("#{"))
                {
                    return unknown(state, text.len());
                }
                if current.escape && bytes[at] == b'\\'
                    || provider == Family::PowerShell && current.close == "\"" && bytes[at] == b'`'
                {
                    at += 1;
                    if at < text.len() {
                        at += text[at..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                if text[at..].starts_with(&current.close) {
                    at += current.close.len();
                    if current.doubled && text[at..].starts_with(&current.close) {
                        at += current.close.len();
                        continue;
                    }
                    closed = true;
                    break;
                }
                at += text[at..].chars().next().unwrap().len_utf8();
            }
            result.protected.push(begin..at);
            if closed {
                state.string = None;
            } else {
                result.protected_to_line_end = true;
                if !current.multiline {
                    // Backslash continuation is supported only for languages
                    // where it is part of the lexical string grammar.
                    if !(current.escape
                        && text.bytes().rev().take_while(|byte| *byte == b'\\').count() % 2 == 1
                        && matches!(provider, Family::Python | Family::Js))
                    {
                        state.unknown = true;
                    }
                }
                break;
            }
            continue;
        }
        if at >= text.len() {
            break;
        }
        let rest = &text[at..];
        if provider == Family::Xml {
            if bytes[at] == b'<' && !rest.starts_with("<!--") {
                state.xml_in_tag = true;
            }
            if bytes[at] == b'>' {
                state.xml_in_tag = false;
            }
            if rest.starts_with("<?") || rest.starts_with("<![CDATA[") {
                let (width, close) = if rest.starts_with("<?") {
                    (2, "?>")
                } else {
                    (9, "]]>")
                };
                string_start = Some(at);
                at += width;
                state.string = Some(string(close, false, false, true));
                state.xml_in_tag = false;
                continue;
            }
        }
        // Explicit conservative boundaries: these forms need a grammar-aware
        // provider before comment assistance can safely operate after them.
        if (provider == Family::Js
            && (rest.starts_with('/') && !rest.starts_with("//") && !rest.starts_with("/*")
                || rest.starts_with('<')))
            || (provider == Family::Groovy
                && rest.starts_with('/')
                && !rest.starts_with("//")
                && !rest.starts_with("/*"))
            || (provider == Family::Swift
                && (rest.starts_with('#')
                    || rest.starts_with('/') && !rest.starts_with("//") && !rest.starts_with("/*")))
            || (provider == Family::Scala
                && (rest.starts_with("s\"")
                    || rest.starts_with("f\"")
                    || rest.starts_with("raw\"")
                    || rest.starts_with('<')))
            || (provider == Family::CSharp
                && (rest.starts_with("$\"")
                    || rest.starts_with("$@\"")
                    || rest.starts_with("@$\"")))
            || (provider == Family::Scss && rest.starts_with("#{"))
            || (provider == Family::Hcl && rest.starts_with("<<"))
            || (provider == Family::PowerShell
                && (rest.starts_with("@\"") || rest.starts_with("@'")))
            || (provider == Family::Shell
                && (rest.starts_with("$(")
                    || rest.starts_with('`')
                    || rest.starts_with("((")
                    || rest.starts_with("[[")
                    || rest.starts_with("<<<")))
            || (provider == Family::Haskell && rest.starts_with("{-#"))
            || (provider == Family::Haskell
                && rest.starts_with("--")
                && rest[2..].chars().next().is_some_and(|ch| !ch.is_ascii()))
            || (provider == Family::Java && rest.starts_with("\\u"))
            || (provider == Family::Xml && rest.starts_with("<!DOCTYPE"))
            || (provider == Family::Sql
                && (rest.starts_with('[')
                    || rest.starts_with('$')
                        && rest[1..].find('$').is_some_and(|n| {
                            n < 64
                                && rest[1..n + 1]
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        })))
            || (matches!(provider, Family::Css | Family::Scss)
                && rest
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("url(")))
            || (provider == Family::Tex
                && (rest.starts_with("\\verb")
                    || rest.starts_with("\\catcode")
                    || [
                        "verbatim",
                        "Verbatim",
                        "lstlisting",
                        "minted",
                        "alltt",
                        "comment",
                    ]
                    .iter()
                    .any(|name| {
                        rest.strip_prefix("\\begin{")
                            .is_some_and(|tail| tail.starts_with(name))
                    })))
            || (provider == Family::Vim && rest.starts_with("=<<"))
            || (provider == Family::C
                && (rest.starts_with("__has_include") || rest.starts_with("__has_embed")))
            || (provider == Family::PowerShell
                && (rest.starts_with("--%")
                    || rest.starts_with('#') && at > 0 && !bytes[at - 1].is_ascii_whitespace()))
        {
            return unknown(state, text.len());
        }
        if provider == Family::Python {
            let prefix = rest
                .bytes()
                .take_while(|b| {
                    matches!(
                        b,
                        b'r' | b'R' | b'f' | b'F' | b't' | b'T' | b'b' | b'B' | b'u' | b'U'
                    )
                })
                .count();
            if prefix > 0
                && prefix <= 3
                && rest[..prefix].contains(['f', 'F', 't', 'T'])
                && rest
                    .as_bytes()
                    .get(prefix)
                    .is_some_and(|b| matches!(b, b'\"' | b'\''))
            {
                return unknown(state, text.len());
            }
        }
        if provider == Family::Shell && rest.starts_with("$'") {
            string_start = Some(at);
            at += 2;
            state.string = Some(string("'", true, false, true));
            continue;
        }
        if provider == Family::C && bytes[at] == b'\\' && at + 1 == text.len() {
            return unknown(state, text.len());
        }
        // C-family raw strings and Rust raw strings have dynamic closing delimiters.
        if provider == Family::C && rest.starts_with("R\"") {
            let Some(paren) = rest[2..].find('(').map(|n| n + 2) else {
                return unknown(state, text.len());
            };
            let delimiter = &rest[2..paren];
            if delimiter.len() > 16
                || delimiter
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, ')' | '\\'))
            {
                return unknown(state, text.len());
            }
            string_start = Some(at);
            at += paren + 1;
            state.string = Some(string(&format!("){delimiter}\""), false, false, true));
            continue;
        }
        if provider == Family::Rust
            && (rest.starts_with('r') || rest.starts_with("br") || rest.starts_with("cr"))
        {
            let start = if rest.starts_with('r') { 1 } else { 2 };
            let count = rest[start..].bytes().take_while(|b| *b == b'#').count();
            if count > 255 {
                return unknown(state, text.len());
            }
            if rest.as_bytes().get(start + count) == Some(&b'"') {
                string_start = Some(at);
                at += start + count + 1;
                state.string = Some(string(
                    &format!("\"{}", "#".repeat(count)),
                    false,
                    false,
                    true,
                ));
                continue;
            }
        }
        if matches!(provider, Family::Lua | Family::Cmake) {
            let is_comment = if provider == Family::Lua {
                rest.starts_with("--")
            } else {
                rest.starts_with('#')
            };
            let lead = if provider == Family::Lua { 2 } else { 1 };
            let bracket_at = at + if is_comment { lead } else { 0 };
            if bytes.get(bracket_at) == Some(&b'[')
                && bytes[bracket_at + 1..]
                    .iter()
                    .take(256)
                    .take_while(|byte| **byte == b'=')
                    .count()
                    >= 256
            {
                return unknown(state, text.len());
            }
            if let Some((end, close)) = long_bracket(text, bracket_at) {
                if is_comment {
                    let open = &text[at..end];
                    start_block(state, open, &close, false, line, at);
                    block_open = Some(at..end);
                } else {
                    string_start = Some(at);
                    state.string = Some(string(&close, false, false, true));
                    at = end;
                }
                continue;
            }
            if is_comment {
                line_comment(&mut result, at, lead);
                break;
            }
        }
        if provider == Family::Shell && rest.starts_with("<<") {
            let mut end = at + 2;
            let tabs = bytes.get(end) == Some(&b'-');
            if tabs {
                end += 1;
            }
            while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
                end += 1;
            }
            let token_start = end;
            let mut delimiter = String::new();
            let mut quote = None;
            while end < text.len() {
                let ch = text[end..].chars().next().unwrap();
                if let Some(q) = quote {
                    if ch == q {
                        quote = None;
                    } else if ch == '\\' {
                        return unknown(state, text.len());
                    } else {
                        delimiter.push(ch);
                    }
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                } else if ch.is_whitespace() || ";|&<>".contains(ch) {
                    break;
                } else if ch == '\\' {
                    end += 1;
                    if end == text.len() {
                        return unknown(state, text.len());
                    }
                    let c = text[end..].chars().next().unwrap();
                    delimiter.push(c);
                    end += c.len_utf8();
                    continue;
                } else {
                    delimiter.push(ch);
                }
                end += ch.len_utf8();
            }
            if quote.is_some()
                || delimiter.is_empty()
                || delimiter.len() > 256
                || state.heredocs.len() >= 16
            {
                return unknown(state, text.len());
            }
            state.heredocs.push(HereDoc { delimiter, tabs });
            result.protected.push(token_start..end);
            at = end;
            continue;
        }
        if provider == Family::Shell && rest.starts_with("${") {
            let Some(end) = rest.find('}') else {
                return unknown(state, text.len());
            };
            if rest[2..end].contains(['{', '\'', '"', '`']) || rest[2..end].contains("$(") {
                return unknown(state, text.len());
            }
            result.protected.push(at..at + end + 1);
            at += end + 1;
            continue;
        }
        let c_style = matches!(
            provider,
            Family::C
                | Family::Rust
                | Family::Go
                | Family::Java
                | Family::Js
                | Family::CSharp
                | Family::Swift
                | Family::Kotlin
                | Family::Scala
                | Family::Groovy
                | Family::Dart
                | Family::Proto
                | Family::Css
                | Family::Scss
                | Family::JsonComments
                | Family::Hcl
        );
        let block = if c_style && rest.starts_with("/*") {
            Some((
                "/*",
                "*/",
                matches!(
                    provider,
                    Family::Rust | Family::Swift | Family::Kotlin | Family::Scala | Family::Dart
                ),
            ))
        } else if provider == Family::Haskell && rest.starts_with("{-") {
            Some(("{-", "-}", true))
        } else if provider == Family::Sql && rest.starts_with("/*") {
            Some(("/*", "*/", true))
        } else if provider == Family::Xml && rest.starts_with("<!--") {
            Some(("<!--", "-->", false))
        } else if provider == Family::PowerShell && rest.starts_with("<#") {
            Some(("<#", "#>", false))
        } else {
            None
        };
        if let Some((open, close, nested)) = block {
            start_block(state, open, close, nested, line, at);
            block_open = Some(at..at + open.len());
            continue;
        }
        let hash = matches!(
            provider,
            Family::Python
                | Family::Hash
                | Family::Toml
                | Family::Graphql
                | Family::Hcl
                | Family::Shell
                | Family::Yaml
                | Family::PowerShell
        );
        let is_line = if c_style && provider != Family::Css && rest.starts_with("//") {
            Some(2)
        } else if hash
            && rest.starts_with('#')
            && (provider != Family::Shell || boundary(text, at))
            && (provider != Family::Yaml || at == 0 || bytes[at - 1].is_ascii_whitespace())
        {
            Some(1)
        } else if provider == Family::Sql && rest.starts_with("--") {
            Some(2)
        } else if provider == Family::Haskell
            && rest.starts_with("--")
            && rest[2..]
                .chars()
                .next()
                .is_none_or(|c| !"!#$%&*+./<=>?@\\^|-~:".contains(c))
        {
            Some(2)
        } else if provider == Family::Ini && matches!(bytes[at], b';' | b'#') && at == indent(text)
        {
            Some(1)
        } else if provider == Family::Tex && bytes[at] == b'%' {
            Some(1)
        } else if provider == Family::Vim
            && at == indent(text)
            && ((!state.vim9 && bytes[at] == b'"') || (state.vim9 && bytes[at] == b'#'))
        {
            Some(1)
        } else {
            None
        };
        if let Some(width) = is_line {
            line_comment(&mut result, at, width);
            state.continued_line_comment = provider == Family::C && text.ends_with('\\');
            break;
        }
        if provider == Family::Vim
            && ((!state.vim9 && bytes[at] == b'"') || (state.vim9 && bytes[at] == b'#'))
        {
            return unknown(state, text.len());
        }
        if provider == Family::Yaml
            && matches!(bytes[at], b'|' | b'>')
            && (at == 0 || bytes[at - 1].is_ascii_whitespace())
        {
            let suffix = rest[1..].split('#').next().unwrap().trim();
            if suffix.chars().all(|c| matches!(c, '+' | '-' | '1'..='9')) && suffix.len() <= 2 {
                let explicit = suffix
                    .chars()
                    .find_map(|c| c.to_digit(10))
                    .map(|n| indent(text) + n as usize);
                state.yaml_scalar = Some((indent(text), explicit));
            } else {
                return unknown(state, text.len());
            }
        }
        if provider == Family::PowerShell && bytes[at] == b'`' {
            at += 1;
            if at < text.len() {
                at += text[at..].chars().next().unwrap().len_utf8();
            }
            continue;
        }
        if matches!(
            provider,
            Family::Shell | Family::Tex | Family::Css | Family::Scss
        ) && bytes[at] == b'\\'
        {
            if provider == Family::Shell && at + 1 == text.len() {
                return unknown(state, text.len());
            }
            at += 1;
            if at < text.len() {
                at += text[at..].chars().next().unwrap().len_utf8();
            }
            continue;
        }
        if provider == Family::Tex {
            at += text[at..].chars().next().unwrap().len_utf8();
            continue;
        }
        if provider == Family::CSharp && rest.starts_with("@\"") {
            string_start = Some(at);
            at += 2;
            state.string = Some(string("\"", false, true, true));
            continue;
        }
        if provider == Family::Hash && (rest.starts_with("r\"") || rest.starts_with("R\"")) {
            return unknown(state, text.len());
        }
        if provider == Family::Dart && (rest.starts_with("r\"") || rest.starts_with("r'")) {
            let quotes = if rest[1..].starts_with("\"\"\"") || rest[1..].starts_with("'''") {
                3
            } else {
                1
            };
            string_start = Some(at);
            at += 1 + quotes;
            state.string = Some(string(&rest[1..1 + quotes], false, false, quotes == 3));
            continue;
        }
        let triple = (rest.starts_with("\"\"\"") || rest.starts_with("'''"))
            && matches!(
                provider,
                Family::Python
                    | Family::Toml
                    | Family::Java
                    | Family::Kotlin
                    | Family::Scala
                    | Family::Groovy
                    | Family::Dart
                    | Family::Swift
                    | Family::CSharp
                    | Family::Graphql
            );
        if triple {
            // C# raw-string delimiter length and indentation semantics are not
            // inferred from an ordinary triple quote.
            if provider == Family::CSharp {
                return unknown(state, text.len());
            }
            let close = &rest[..3];
            string_start = Some(at);
            at += 3;
            state.string = Some(string(
                close,
                !(provider == Family::Toml && close == "'''"
                    || matches!(provider, Family::Kotlin | Family::Scala)),
                false,
                true,
            ));
            continue;
        }
        if matches!(bytes[at], b'"' | b'\'')
            || (bytes[at] == b'`'
                && matches!(
                    provider,
                    Family::Go | Family::Js | Family::Hash | Family::Sql
                ))
        {
            let quote = bytes[at] as char;
            if quote == '\'' && matches!(provider, Family::Rust | Family::Haskell) {
                // Lifetimes and identifier primes are code. A char literal has
                // exactly one scalar (or an escape) followed by a closing quote.
                let tail = &rest[1..];
                let end = if tail.starts_with('\\') {
                    tail[1..].find('\'').map(|n| n + 1)
                } else {
                    tail.chars().next().map(|c| c.len_utf8())
                };
                if end.is_none_or(|end| tail.as_bytes().get(end) != Some(&b'\'')) {
                    at += 1;
                    continue;
                }
            }
            if matches!(provider, Family::Json | Family::Graphql) && quote != '"' {
                at += 1;
                continue;
            }
            if provider == Family::Xml && !state.xml_in_tag {
                at += 1;
                continue;
            }
            if provider == Family::Yaml
                && at > 0
                && !bytes[at - 1].is_ascii_whitespace()
                && !b"[{,:?-".contains(&bytes[at - 1])
            {
                at += 1;
                continue;
            }
            if provider == Family::Js && quote == '`' && rest.contains("${") {
                return unknown(state, text.len());
            }
            let doubled = matches!(
                provider,
                Family::Sql | Family::Yaml | Family::Vim | Family::PowerShell
            );
            let escape = !matches!(
                provider,
                Family::Sql | Family::Yaml | Family::Vim | Family::PowerShell
            ) && !(quote == '`' && provider == Family::Go)
                && !(quote == '\'' && matches!(provider, Family::Shell | Family::Toml));
            let escape = escape || (provider == Family::Yaml && quote == '"');
            let multiline = matches!(
                provider,
                Family::Shell
                    | Family::Yaml
                    | Family::Sql
                    | Family::Xml
                    | Family::PowerShell
                    | Family::Rust
            ) || (quote == '`' && matches!(provider, Family::Go | Family::Js));
            string_start = Some(at);
            at += 1;
            state.string = Some(string(&quote.to_string(), escape, doubled, multiline));
            continue;
        }
        at += text[at..].chars().next().unwrap().len_utf8();
    }
    if state.unknown
        || provider == Family::C && text.ends_with('\\') && !state.continued_line_comment
        || provider == Family::Shell && !state.heredocs.is_empty() && state.string.is_some()
    {
        return unknown(state, text.len());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    fn document(text: &str) -> Document {
        Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap()
    }
    fn context(language: &str, text: &str, line: usize) -> CommentLineContext {
        document(text)
            .hard_line_snapshot()
            .comment_context(Some(language), line)
    }
    fn assert_comment(language: &str, text: &str, line: usize, start: usize, opener: &str) {
        let result = context(language, text, line);
        assert_eq!(
            result.status,
            CommentContextStatus::Known,
            "{language}: {text}"
        );
        let span = result
            .comment_at(start)
            .unwrap_or_else(|| panic!("{language}: {result:?}: {text}"));
        assert_eq!(
            span.opener,
            Some(start..start + opener.len()),
            "{language}: {text}"
        );
    }
    #[test]
    fn revisiting_earlier_lines_after_the_cache_fills_resumes_nearby() {
        let lines = MAX_CHECKPOINTS * 8;
        let snapshot = document(&"let a = 1; // note\n".repeat(lines)).hard_line_snapshot();
        for line in (0..lines).rev().step_by(4) {
            snapshot.comment_context(Some("rust"), line);
        }
        let (_, work) = crate::document::measure_document_work(|| {
            for line in 0..lines {
                assert_eq!(snapshot.comment_context(Some("rust"), line).comments.len(), 1);
            }
        });
        assert!(work.comment_context_lexed_lines <= lines + CHECKPOINT_STRIDE, "{}", work.comment_context_lexed_lines);
    }
    #[test]
    fn native_forms_and_string_protection_across_families() {
        for (language, text, start, opener) in [
            ("cpp", "\"// string\"; // comment", 13, "//"),
            ("rust", "r###\"/* string */\"###; // comment", 23, "//"),
            ("go", "`// string`; // comment", 13, "//"),
            ("java", "\"// string\"; // comment", 13, "//"),
            ("kotlin", "\"// string\"; // comment", 13, "//"),
            ("scala", "\"// string\"; // comment", 13, "//"),
            ("dart", "'// string'; // comment", 13, "//"),
            ("proto", "'// string'; // comment", 13, "//"),
            ("css", "\"/* string */\" /* comment */", 15, "/*"),
            ("scss", "'// string'; // comment", 13, "//"),
            ("jsonc", "\"// string\": 1, // comment", 16, "//"),
            ("python", "'# string' # comment", 11, "#"),
            ("r", "`# string` # comment", 11, "#"),
            ("toml", "value = '# string' # comment", 19, "#"),
            ("graphql", "\"# string\" # comment", 11, "#"),
            ("terraform", "\"# string\" # comment", 11, "#"),
            ("sql", "'-- string' -- comment", 12, "--"),
            ("haskell", "\"-- string\" -- comment", 12, "--"),
            ("dosini", "  ; comment", 2, ";"),
            ("ps1", "'# string' # comment", 11, "#"),
            ("xml", "<a v='<!-- string -->'><!-- comment -->", 23, "<!--"),
        ] {
            assert_comment(language, text, 0, start, opener);
        }
        let c = context("cpp", "\"// literal\"", 0);
        assert!(!c.is_code_at(2));
        assert!(c.is_code_at(12));
        assert!(c.comments.is_empty());
        assert_eq!(
            context("unknown", "// literal", 0).status,
            CommentContextStatus::Compatibility
        );
        assert_eq!(
            document("// x")
                .hard_line_snapshot()
                .comment_context(None, 0)
                .status,
            CommentContextStatus::Compatibility
        );
    }
    #[test]
    fn multiline_literals_and_dynamic_delimiters_are_protected() {
        for (language, text, line) in [
            ("python", "value = '''\n# not a comment\n'''\n# comment", 1),
            (
                "cpp",
                "auto s = R\"tag(\n// not a comment\n)tag\";\n// comment",
                1,
            ),
            ("rust", "let s = r##\"\n/* not a comment */\n\"##;", 1),
            ("java", "var s = \"\"\"\n// not a comment\n\"\"\";", 1),
            ("go", "s := `\n// not a comment\n`", 1),
            ("lua", "s = [==[\n-- not a comment\n]==]", 1),
            ("cmake", "set(a [=[\n# not a comment\n]=])", 1),
            ("xml", "<?processing\n<!-- not a comment -->\n?>", 1),
            ("xml", "<![CDATA[\n<!-- not a comment -->\n]]>", 1),
            ("toml", "s = '''\n# not a comment\n'''", 1),
            ("yaml", "a: |\n  # not a comment\n# comment", 1),
            ("yaml", "a: 'multi\n# not a comment\nline'", 1),
            ("bash", "cat <<'EOF'\n# not a comment\nEOF\n# comment", 1),
            (
                "sh",
                "cat <<-A <<B\n\t# not a comment\n\tA\n# still literal\nB\n# comment",
                3,
            ),
        ] {
            let result = context(language, text, line);
            assert_eq!(
                result.status,
                CommentContextStatus::Known,
                "{language}: {result:?}"
            );
            assert!(result.comments.is_empty(), "{language}: {result:?}");
            assert!(!result.is_code_at(0), "{language}: {result:?}");
        }
        assert_comment(
            "lua",
            "--[==[ native\nbody\n]==]\n-- comment",
            0,
            0,
            "--[==[",
        );
        assert_comment("cmake", "#[=[ native\nbody\n]=]\n# comment", 0, 0, "#[=[");
        assert_comment("yaml", "a: |\n  # not a comment\n# comment", 2, 0, "#");
        assert_comment(
            "bash",
            "cat <<'EOF'\n# not a comment\nEOF\n# comment",
            3,
            0,
            "#",
        );
    }
    #[test]
    fn nested_blocks_keep_inner_identity_and_exact_delimiters() {
        for language in ["rust", "swift", "kotlin", "scala", "dart"] {
            let result = context(language, "/* outer\ninner /* nested */ tail\n*/ code", 1);
            assert_eq!(result.comments.len(), 3, "{language}: {result:?}");
            assert_eq!(result.comments[0].block_start, Some((0, 0)));
            assert_eq!(result.comments[1].block_start, Some((1, 6)));
            assert_eq!(result.comments[1].depth, 2);
            assert_eq!(result.comments[1].opener, Some(6..8));
            assert_eq!(result.comments[1].body, 8..16);
            assert_eq!(result.comments[1].closer, Some(16..18));
            assert_eq!(result.comments[2].block_start, Some((0, 0)));
            let closed = context(language, "/* outer\n*/ code", 1);
            assert!(closed.comment_at(1).is_some());
            assert!(closed.comment_at(2).is_none());
            assert!(closed.is_code_at(3));
        }
        let hs = context("haskell", "{- a {- b -} c -}", 0);
        assert_eq!(hs.comments.len(), 3);
        assert_eq!(hs.comments[1].block_start, Some((0, 5)));
    }
    #[test]
    fn lexical_conflicts_shell_expansions_and_dialects() {
        assert_comment("bash", "echo $# ${#x} word#suffix;# valid", 0, 26, "#");
        assert_comment("yaml", "value: don't # comment", 0, 13, "#");
        assert_comment("vim", "\" legacy comment", 0, 0, "\"");
        assert_comment("vim", "vim9script\n# comment", 1, 0, "#");
        assert!(context("css", "// ordinary text", 0).comments.is_empty());
        assert_comment("scss", "// comment", 0, 0, "//");
        assert!(context("python", "#!/usr/bin/python", 0)
            .comments
            .is_empty());
        assert_comment("tex", "\\% escaped % comment", 0, 11, "%");
        assert!(context("cpp", "// comment\\\ncontinued", 1)
            .comment_at(0)
            .is_some());
        assert!(context("rust", "let c = '\\''; // comment", 0)
            .comment_at(14)
            .is_some());
        for text in [
            "#include <path//header> // actual",
            "%:include <path/*header> // actual",
            "# include \"path//header\" // actual",
            "export import <path//header>; // actual",
        ] {
            let value = context("cpp", text, 0);
            assert_eq!(value.status, CommentContextStatus::Known);
            assert_eq!(value.comments.len(), 1);
            assert_eq!(&text[value.comments[0].body.clone()], " actual");
        }
        for text in [r"echo word\ #not-comment", r"echo word\;#not-comment"] {
            let value = context("bash", text, 0);
            assert_eq!(value.status, CommentContextStatus::Known);
            assert!(value.comments.is_empty());
        }
        for (language, text) in [
            ("dart", r###"r"\" + " // inside"; // actual"###),
            ("kotlin", r###"val s = """a\"""; // actual"###),
            ("scala", r###"val s = """a\"""; // actual"###),
        ] {
            let value = context(language, text, 0);
            assert_eq!(
                value.status,
                CommentContextStatus::Known,
                "{language}: {value:?}"
            );
            assert_eq!(value.comments.len(), 1, "{language}: {value:?}");
            assert_eq!(&text[value.comments[0].body.clone()], " actual");
        }
    }
    #[test]
    fn unsupported_or_ambiguous_forms_never_degrade_to_code() {
        for (language, text, line) in [
            ("javascript", "const re = /a[//]b/;\n// later", 1),
            ("javascript", "const s = `first\n${\"// nested\"}`;", 1),
            ("python", "x = f\"{\"# nested\"}\"", 0),
            ("python", "x = rf'''{stuff}'''", 0),
            ("python", "x = t\"{\"# nested\"}\"", 0),
            ("python", "x = rt'''{stuff}'''", 0),
            ("java", "\"\\u0022 // not safely scanned\"", 0),
            ("cpp", "code\\\n// later", 1),
            ("cpp", "/*\n*\\\n/ code", 2),
            ("cpp", "#include <incomplete", 0),
            ("cpp", "#include /* before */ <path//header>", 0),
            ("cpp", "#if __has_include(<path//header>)", 0),
            ("vim", "let lines =<< END\n\"not a comment\nEND", 1),
            (
                "vim",
                "vim9script\nvar lines =<< END\n#not a comment\nEND",
                2,
            ),
            ("c_sharp", "$\"{\"// nested\"}\"", 0),
            ("kotlin", "\"${\"// nested\"}\"", 0),
            ("swift", "\"\\(\"// nested\")\"", 0),
            ("scala", "s\"// interpolated\"", 0),
            ("groovy", "/slashy/", 0),
            ("scss", "a { x: #{y}; }", 0),
            ("hcl", "x = <<EOF", 0),
            ("ps1", "@\"\n# literal\n\"@", 1),
            ("sh", "$(echo \"#\")\n# later", 1),
            ("bash", "echo word\\\n#not-comment", 1),
            ("haskell", "{-# LANGUAGE X #-}", 0),
            ("haskell", "--⊕ operator", 0),
            ("xml", "<!DOCTYPE root [<!-- declaration -->]>", 0),
            ("sql", "$tag$ -- literal $tag$", 0),
            ("sql", "[/* identifier */]", 0),
            ("css", "url(foo/*literal)", 0),
            ("tex", "\\verb|%|", 0),
            ("tex", "\\begin{minted}{c}\n% literal\n\\end{minted}", 1),
        ] {
            assert!(
                context(language, text, line).is_unknown(),
                "{language}: {text}"
            );
        }
        assert!(context("cpp", "\"unterminated\n// text", 1).is_unknown());
    }
    #[test]
    fn long_context_budget_warms_without_guessing_or_flattening() {
        let text = format!("/*\n{}tail\n*/", "body\n".repeat(6000));
        let doc = document(&text);
        let snapshot = doc.hard_line_snapshot();
        assert!(snapshot.comment_context(Some("cpp"), 6001).is_unknown());
        let result = snapshot.comment_context(Some("cpp"), 6001);
        assert_eq!(result.status, CommentContextStatus::Known);
        assert_eq!(result.comments[0].block_start, Some((0, 0)));
        assert_eq!(result.comments[0].body, 0..4);
        let giant = document(&format!("{}\n// after", "x".repeat(MAX_LINE_BYTES + 1)));
        for _ in 0..3 {
            assert!(giant
                .hard_line_snapshot()
                .comment_context(Some("cpp"), 1)
                .is_unknown());
        }
        for language in ["lua", "cmake"] {
            let marker = if language == "lua" { "--" } else { "#" };
            let text = format!(
                "value = [{}[\n{marker} not a comment\n]{}]",
                "=".repeat(300),
                "=".repeat(300)
            );
            assert!(context(language, &text, 1).is_unknown());
        }
    }
    #[test]
    fn edited_prefix_invalidates_suffix_and_preserves_historical_snapshot() {
        let mut doc = document("/* open\nbody\n*/\n// tail");
        let old = doc.hard_line_snapshot();
        let before = old.comment_context(Some("cpp"), 1);
        assert_eq!(before.comments[0].block_start, Some((0, 0)));
        doc.replace(0..2, "  ").unwrap();
        let next = doc.hard_line_snapshot();
        assert!(next.comment_context(Some("cpp"), 1).comments.is_empty());
        assert_eq!(old.comment_context(Some("cpp"), 1), before);
        assert_eq!(
            next.comment_context(Some("cpp"), 3),
            context("cpp", doc.text(), 3)
        );
        assert!(next.comment_context(Some("python"), 3).comments.is_empty());
    }
    #[test]
    fn local_edit_reuses_only_proven_prefix_checkpoints() {
        let text = format!("/*\n{}body\n*/", "line\n".repeat(5000));
        let mut doc = document(&text);
        let old = doc.hard_line_snapshot();
        assert!(old.comment_context(Some("rust"), 5001).is_unknown());
        assert!(!old.comment_context(Some("rust"), 5001).is_unknown());
        let range = old.line(5001).unwrap().content_range();
        doc.replace(range, "changed").unwrap();
        let new = doc.hard_line_snapshot();
        // Starting fresh would exhaust the budget; the retained prefix is enough.
        assert!(!new.comment_context(Some("rust"), 5001).is_unknown());
        let changed = new.comment_context(Some("rust"), 5001);
        assert_eq!(changed.comments[0].body, 0..7);
        let fresh = document(doc.text());
        let fresh = fresh.hard_line_snapshot();
        fresh.comment_context(Some("rust"), 5001);
        assert_eq!(changed, fresh.comment_context(Some("rust"), 5001));
    }
    #[test]
    fn retained_history_shares_a_finite_revision_arena() {
        let mut doc = document("/* original */\n// body");
        let initial = doc.hard_line_snapshot();
        let expected = initial.comment_context(Some("cpp"), 0);
        let mut history = vec![initial.clone()];
        for _ in 0..MAX_REVISIONS * 3 {
            doc.insert(3, "x").unwrap();
            let snapshot = doc.hard_line_snapshot();
            snapshot.comment_context(Some("cpp"), 0);
            history.push(snapshot);
        }
        let arena = initial.comment_context_cache.arena.lock().unwrap();
        assert!(arena.slots.len() <= MAX_REVISIONS);
        assert!(history.iter().all(|snapshot| Arc::ptr_eq(
            &snapshot.comment_context_cache.arena,
            &initial.comment_context_cache.arena
        )));
        drop(arena);
        assert_eq!(initial.comment_context(Some("cpp"), 0), expected);
        assert!(
            initial
                .comment_context_cache
                .arena
                .lock()
                .unwrap()
                .slots
                .len()
                <= MAX_REVISIONS
        );
    }
    #[test]
    fn cache_eviction_and_budget_do_not_turn_unknown_into_ordinary_code() {
        let doc = document(&format!("'''\n{}end", "body\n".repeat(10000)));
        let snapshot = doc.hard_line_snapshot();
        for line in (0..10000).step_by(64) {
            snapshot.comment_context(Some("python"), line);
        }
        assert!(snapshot
            .comment_context(Some("python"), 9999)
            .comments
            .is_empty());
        for language in ["rust", "sql", "swift", "toml"] {
            snapshot.comment_context(Some(language), 0);
        }
        assert!(snapshot.comment_context(Some("python"), 9999).is_unknown());
        let unknown = context("ruby", "# comment", 0);
        assert_eq!(unknown.status, CommentContextStatus::Compatibility);
    }
}
