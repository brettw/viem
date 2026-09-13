//! The versioned Viem pattern language. Thompson transitions come from the
//! Unicode regex compiler; this bounded VM supplies semantic hard-line
//! assertions instead of conflating literal LF content with document breaks.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

use crate::document::{DocumentId, HardLineInfo, HardLineSnapshot, Revision};
use regex_automata::{
    nfa::thompson::{self, State, NFA},
    util::{look::Look, primitives::StateID, syntax},
    PatternID,
};

pub const DIALECT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RegexLimits {
    pub pattern_bytes: usize,
    pub program_bytes: usize,
    pub captures: usize,
    pub work: u64,
}
impl Default for RegexLimits {
    fn default() -> Self {
        Self {
            pattern_bytes: 16_384,
            program_bytes: 8 * 1024 * 1024,
            captures: 64,
            work: 100_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegexError {
    InvalidRegex(String),
    UnsupportedRegexAtom(String),
    UnsupportedReplacementAtom(String),
    RegexResourceLimit(&'static str),
    RegexMatchSplitsGraphemeCluster(Range<usize>),
    StaleProjection,
    Cancelled,
}
impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegex(error) => write!(f, "InvalidRegex: {error}"),
            Self::UnsupportedRegexAtom(atom) => write!(f, "UnsupportedRegexAtom: {atom:?}"),
            Self::UnsupportedReplacementAtom(atom) => {
                write!(f, "UnsupportedReplacementAtom: {atom:?}")
            }
            Self::RegexResourceLimit(limit) => write!(f, "RegexResourceLimit: {limit}"),
            Self::RegexMatchSplitsGraphemeCluster(range) => write!(
                f,
                "RegexMatchSplitsGraphemeCluster: {}..{}",
                range.start, range.end
            ),
            Self::StaleProjection => {
                f.write_str("StaleProjection: search result belongs to another snapshot")
            }
            Self::Cancelled => f.write_str("Cancelled: search was cancelled"),
        }
    }
}
impl std::error::Error for RegexError {}

/// One budget is shared by an entire operation, including repeats and all
/// substitution matches. Cancellation can be requested without editor locks.
pub struct RegexWork {
    remaining: u64,
    cancelled: Arc<AtomicBool>,
}
impl RegexWork {
    pub fn new(limits: RegexLimits) -> Self {
        Self {
            remaining: limits.work,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancellation_handle(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }
    fn tick(&mut self) -> Result<(), RegexError> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(RegexError::Cancelled);
        }
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(RegexError::RegexResourceLimit("search work"))?;
        Ok(())
    }
}

/// Results are ordinal ranges only within this explicitly named snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegexMatch {
    document: DocumentId,
    revision: Revision,
    captures: Vec<Option<Range<usize>>>,
}
impl RegexMatch {
    pub fn range(&self) -> Range<usize> {
        self.captures[0].clone().expect("capture zero is complete")
    }
    pub fn capture(&self, index: usize) -> Option<Range<usize>> {
        self.captures.get(index).cloned().flatten()
    }
    pub fn validate(&self, snapshot: &HardLineSnapshot) -> Result<(), RegexError> {
        if self.document != snapshot.document() || self.revision != snapshot.revision() {
            return Err(RegexError::StaleProjection);
        }
        Ok(())
    }
    pub fn validate_edit(&self, snapshot: &HardLineSnapshot) -> Result<(), RegexError> {
        self.validate(snapshot)?;
        let range = self.range();
        if !snapshot.is_grapheme_boundary(range.start) || !snapshot.is_grapheme_boundary(range.end)
        {
            return Err(RegexError::RegexMatchSplitsGraphemeCluster(range));
        }
        Ok(())
    }
}

/// A search borrows persistent text and line indexes. Its workspace is bounded
/// by the regex program, not by the document's byte or hard-line count.
pub struct RegexInput<'a> {
    snapshot: &'a HardLineSnapshot,
    cursor: RefCell<InputCursor<'a>>,
}
impl<'a> RegexInput<'a> {
    pub fn new(snapshot: &'a HardLineSnapshot) -> Self {
        Self {
            snapshot,
            cursor: RefCell::new(InputCursor::new(snapshot)),
        }
    }
    pub fn snapshot(&self) -> &HardLineSnapshot {
        self.snapshot
    }
    pub fn hard_break_offsets(&self, range: Range<usize>) -> impl Iterator<Item = usize> + '_ {
        let mut lines = LineCursor::new(self.snapshot);
        // Most replacement captures contain no break. Avoid copying a batch
        // of unrelated hard lines just to establish that common case.
        let mut first_line = self.snapshot.line_at_offset(range.start).ok();
        let mut at = range.start;
        std::iter::from_fn(move || {
            while at < range.end {
                let separator = if let Some(line) = first_line.take() {
                    line.separator_range()?
                } else {
                    lines.at(at)?.separator_range()?
                };
                at = separator.end;
                if separator.start >= range.end {
                    return None;
                }
                if separator.start >= range.start {
                    return Some(separator.start);
                }
            }
            None
        })
    }
}

const LINE_BATCH: usize = 128;
struct LineCursor<'a> {
    snapshot: &'a HardLineSnapshot,
    batch: Vec<HardLineInfo>,
}
impl<'a> LineCursor<'a> {
    fn new(snapshot: &'a HardLineSnapshot) -> Self {
        Self {
            snapshot,
            batch: Vec::new(),
        }
    }
    fn at(&mut self, at: usize) -> Option<&HardLineInfo> {
        let index = self
            .batch
            .partition_point(|line| line.content_range().end < at);
        if index < self.batch.len() && self.batch[index].content_range().start <= at {
            return self.batch.get(index);
        }
        let index = self.snapshot.line_at_offset(at).ok()?.index();
        self.batch = self
            .snapshot
            .lines(index..(index + LINE_BATCH).min(self.snapshot.line_count()))
            .ok()?;
        self.batch.first()
    }
}

// Look assertions inspect at most the immediately adjacent Unicode scalars.
// Keep four bytes of overlap on each side of the scan position. Byte transitions
// read this bounded window directly; only refills traverse persistent tree leaves.
const LOOK_CONTEXT: usize = 4;
const SCAN_BYTES: usize = 4096;
struct InputCursor<'a> {
    snapshot: &'a HardLineSnapshot,
    bytes: [u8; SCAN_BYTES + 2 * LOOK_CONTEXT],
    start: usize,
    end: usize,
    lines: LineCursor<'a>,
    #[cfg(test)]
    chunk_reads: usize,
}
impl<'a> InputCursor<'a> {
    fn new(snapshot: &'a HardLineSnapshot) -> Self {
        Self {
            snapshot,
            bytes: [0; SCAN_BYTES + 2 * LOOK_CONTEXT],
            start: 0,
            end: 0,
            lines: LineCursor::new(snapshot),
            #[cfg(test)]
            chunk_reads: 0,
        }
    }
    fn ensure(&mut self, at: usize) {
        let length = self.snapshot.text_length();
        let before = at.saturating_sub(LOOK_CONTEXT);
        let after = at.saturating_add(LOOK_CONTEXT).min(length);
        if self.start <= before && after <= self.end {
            return;
        }
        self.start = before;
        self.end = (before + self.bytes.len()).min(length);
        let mut offset = before;
        while offset < self.end {
            let chunk = self.snapshot.byte_chunk_at(offset);
            #[cfg(test)]
            {
                self.chunk_reads += 1;
            }
            let count = chunk.len().min(self.end - offset);
            self.bytes[offset - before..offset - before + count].copy_from_slice(&chunk[..count]);
            offset += count;
        }
    }
    fn byte(&mut self, at: usize) -> Option<u8> {
        if at >= self.snapshot.text_length() {
            return None;
        }
        self.ensure(at);
        Some(self.bytes[at - self.start])
    }
    fn is_char_boundary(&mut self, at: usize) -> bool {
        at == self.snapshot.text_length() || self.byte(at).is_some_and(|byte| byte & 0xC0 != 0x80)
    }
    fn boundary_without_refill(&self, at: usize) -> bool {
        if at == self.snapshot.text_length() {
            return true;
        }
        let byte = if self.start <= at && at < self.end {
            Some(self.bytes[at - self.start])
        } else {
            self.snapshot.byte_chunk_at(at).first().copied()
        };
        byte.is_some_and(|byte| byte & 0xc0 != 0x80)
    }
    fn matches_look(&mut self, nfa: &NFA, look: Look, at: usize) -> bool {
        match look {
            Look::Start => at == 0,
            Look::End => at == self.snapshot.text_length(),
            Look::StartLF | Look::EndLF => {
                if !self.is_char_boundary(at) {
                    return false;
                }
                self.lines.at(at).is_some_and(|line| {
                    let range = line.content_range();
                    at == if look == Look::StartLF {
                        range.start
                    } else {
                        range.end
                    }
                })
            }
            _ => {
                self.ensure(at);
                nfa.look_matcher().matches(
                    look,
                    &self.bytes[..self.end - self.start],
                    at - self.start,
                )
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompiledRegex {
    nfa: NFA,
}
#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    pattern: String,
    insensitive: bool,
    limits: RegexLimits,
}
type PatternCache = VecDeque<(CacheKey, Arc<CompiledRegex>)>;
static CACHE: OnceLock<Mutex<PatternCache>> = OnceLock::new();

impl CompiledRegex {
    pub fn compile(
        pattern: &str,
        insensitive: bool,
        limits: RegexLimits,
    ) -> Result<Arc<Self>, RegexError> {
        if pattern.len() > limits.pattern_bytes {
            return Err(RegexError::RegexResourceLimit("pattern length"));
        }
        validate_pattern(pattern)?;
        let key = CacheKey {
            pattern: pattern.into(),
            insensitive,
            limits,
        };
        let cache = CACHE.get_or_init(Default::default);
        if let Some(found) = cache
            .lock()
            .expect("pattern cache lock")
            .iter()
            .find(|(old, _)| old == &key)
            .map(|(_, value)| value.clone())
        {
            return Ok(found);
        }
        let nfa = NFA::compiler()
            .configure(thompson::Config::new().nfa_size_limit(Some(limits.program_bytes)))
            .syntax(
                syntax::Config::new()
                    .unicode(true)
                    .utf8(true)
                    .multi_line(true)
                    .case_insensitive(insensitive),
            )
            .build(pattern)
            .map_err(|error| {
                if error.to_string().contains("size limit") {
                    RegexError::RegexResourceLimit("compiled program")
                } else {
                    RegexError::InvalidRegex(error.to_string())
                }
            })?;
        let slots = nfa.group_info().slot_len();
        if nfa
            .group_info()
            .group_len(PatternID::ZERO)
            .saturating_sub(1)
            > limits.captures
        {
            return Err(RegexError::RegexResourceLimit("capture count"));
        }
        let runtime_bytes = nfa
            .states()
            .len()
            .saturating_mul(slots.saturating_add(4))
            .saturating_mul(std::mem::size_of::<usize>())
            .saturating_mul(2);
        if nfa.memory_usage().saturating_add(runtime_bytes) > limits.program_bytes {
            return Err(RegexError::RegexResourceLimit(
                "compiled program and capture workspace",
            ));
        }
        let compiled = Arc::new(Self { nfa });
        let mut cache = cache.lock().expect("pattern cache lock");
        if cache.len() == 32 {
            cache.pop_front();
        }
        cache.push_back((key, compiled.clone()));
        Ok(compiled)
    }
    pub fn capture_count(&self) -> usize {
        self.nfa.group_info().group_len(PatternID::ZERO)
    }
    pub fn capture_index(&self, name: &str) -> Option<usize> {
        self.nfa.group_info().to_index(PatternID::ZERO, name)
    }

    pub fn find(
        &self,
        input: &RegexInput<'_>,
        start: usize,
        end: usize,
        work: &mut RegexWork,
    ) -> Result<Option<RegexMatch>, RegexError> {
        if start > end || end > input.snapshot.text_length() {
            return Err(RegexError::StaleProjection);
        }
        // Reuse the scan window across repeated matches. Substitution and
        // backward search must not refill a whole window for each short match.
        let mut cursor = input.cursor.borrow_mut();
        if !cursor.is_char_boundary(start) || !cursor.boundary_without_refill(end) {
            return Err(RegexError::StaleProjection);
        }
        let mut current = Threads::new(self.nfa.states().len());
        let mut next = Threads::new(self.nfa.states().len());
        let slots = vec![None; self.nfa.group_info().slot_len()];
        let mut found = None;
        for at in start..=end {
            work.tick()?;
            if found.is_none() && cursor.is_char_boundary(at) {
                self.closure(
                    &mut current,
                    self.nfa.start_anchored(),
                    slots.clone(),
                    &mut cursor,
                    at,
                    work,
                )?;
            }
            let byte = (at < end).then(|| cursor.byte(at).expect("byte before search end"));
            for thread in &current.active {
                work.tick()?;
                let target = match self.nfa.state(thread.state) {
                    State::Match { .. } => {
                        found = Some(thread.slots.clone());
                        break;
                    }
                    State::ByteRange { trans }
                        if byte.is_some_and(|byte| trans.matches_byte(byte)) =>
                    {
                        Some(trans.next)
                    }
                    State::Sparse(trans) => byte.and_then(|byte| trans.matches_byte(byte)),
                    State::Dense(trans) => byte.and_then(|byte| trans.matches_byte(byte)),
                    _ => None,
                };
                if let Some(target) = target {
                    self.closure(
                        &mut next,
                        target,
                        thread.slots.clone(),
                        &mut cursor,
                        at + 1,
                        work,
                    )?;
                }
            }
            if found.is_some() && next.active.is_empty() {
                break;
            }
            std::mem::swap(&mut current, &mut next);
            next.clear();
        }
        Ok(found.map(|slots| RegexMatch {
            document: input.snapshot.document(),
            revision: input.snapshot.revision(),
            captures: slots
                .chunks_exact(2)
                .map(|pair| match (pair[0], pair[1]) {
                    (Some(start), Some(end)) => Some(start..end),
                    _ => None,
                })
                .collect(),
        }))
    }

    pub fn find_all(
        &self,
        input: &RegexInput<'_>,
        range: Range<usize>,
        work: &mut RegexWork,
    ) -> Result<Vec<RegexMatch>, RegexError> {
        let mut start = range.start;
        let mut output = Vec::new();
        let mut previous_empty_end = None;
        while start <= range.end {
            let Some(matched) = self.find(input, start, range.end, work)? else {
                break;
            };
            let bounds = matched.range();
            // Match iteration after zero width always reaches a legal logical
            // boundary, including a semantic break adjacent to literal CR.
            if bounds.is_empty() {
                if previous_empty_end != Some(bounds.end) {
                    output.push(matched);
                }
                let Some(next) = input.snapshot.next_grapheme_boundary(bounds.end) else {
                    break;
                };
                start = next;
            } else {
                start = bounds.end;
                previous_empty_end = Some(bounds.end);
                output.push(matched);
            }
        }
        Ok(output)
    }

    fn closure(
        &self,
        threads: &mut Threads,
        state: StateID,
        slots: Vec<Option<usize>>,
        input: &mut InputCursor<'_>,
        at: usize,
        work: &mut RegexWork,
    ) -> Result<(), RegexError> {
        let mut stack = vec![Thread { state, slots }];
        while let Some(mut thread) = stack.pop() {
            work.tick()?;
            if threads.seen[thread.state.as_usize()] {
                continue;
            }
            threads.seen[thread.state.as_usize()] = true;
            threads.visited.push(thread.state.as_usize());
            match self.nfa.state(thread.state) {
                State::Fail => {}
                State::Look { look, next } => {
                    let pass = input.matches_look(&self.nfa, *look, at);
                    if pass {
                        thread.state = *next;
                        stack.push(thread);
                    }
                }
                State::Capture { next, slot, .. } => {
                    thread.slots[slot.as_usize()] = Some(at);
                    thread.state = *next;
                    stack.push(thread);
                }
                State::Union { alternates } => {
                    for state in alternates.iter().rev() {
                        stack.push(Thread {
                            state: *state,
                            slots: thread.slots.clone(),
                        });
                    }
                }
                State::BinaryUnion { alt1, alt2 } => {
                    stack.push(Thread {
                        state: *alt2,
                        slots: thread.slots.clone(),
                    });
                    thread.state = *alt1;
                    stack.push(thread);
                }
                _ => threads.active.push(thread),
            }
        }
        Ok(())
    }
}
struct Thread {
    state: StateID,
    slots: Vec<Option<usize>>,
}
struct Threads {
    seen: Vec<bool>,
    visited: Vec<usize>,
    active: Vec<Thread>,
}
impl Threads {
    fn new(states: usize) -> Self {
        Self {
            seen: vec![false; states],
            visited: Vec::new(),
            active: Vec::new(),
        }
    }
    fn clear(&mut self) {
        for state in self.visited.drain(..) {
            self.seen[state] = false;
        }
        self.active.clear();
    }
}

/// Lexical validation runs before engine parsing. Escape pairs, nested classes,
/// verbose comments and scoped flags are understood, so unsupported atoms are
/// reported in source order without false positives inside literal escapes.
pub fn validate_pattern(pattern: &str) -> Result<(), RegexError> {
    scan_pattern(pattern).map(|_| ())
}
pub fn has_smartcase_uppercase(pattern: &str) -> Result<bool, RegexError> {
    scan_pattern(pattern)
}

fn scan_pattern(pattern: &str) -> Result<bool, RegexError> {
    let mut at = 0;
    let mut class_depth = 0usize;
    let mut class_first = Vec::new();
    let mut class_negated = Vec::new();
    let mut extended = false;
    let mut stack = Vec::new();
    let mut uppercase = false;
    let bytes = pattern.as_bytes();
    while at < bytes.len() {
        let ch = pattern[at..].chars().next().unwrap();
        if extended && ch == '#' {
            at = pattern[at..]
                .find('\n')
                .map_or(bytes.len(), |end| at + end + 1);
            continue;
        }
        if extended && ch.is_whitespace() {
            at += ch.len_utf8();
            continue;
        }
        if ch == '\\' {
            if let Some(first) = class_first.last_mut() {
                *first = false;
            }
            let start = at;
            at += 1;
            let Some(escaped) = pattern[at..].chars().next() else {
                return Err(RegexError::InvalidRegex("trailing backslash".into()));
            };
            at += escaped.len_utf8();
            let allowed = match escaped {
                'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'A' | 't' | 'r' | 'n' => true,
                'b' | 'B' => !pattern[at..].starts_with('{'),
                'z' => !pattern[at..]
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, 's' | 'e' | '1'..='9' | '(')),
                'p' | 'P' | 'u' if pattern[at..].starts_with('{') => {
                    let Some(end) = pattern[at + 1..].find('}') else {
                        return Err(RegexError::InvalidRegex("unclosed Unicode escape".into()));
                    };
                    at += end + 2;
                    true
                }
                'x' => {
                    if bytes
                        .get(at..at + 2)
                        .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
                    {
                        at += 2;
                        true
                    } else {
                        false
                    }
                }
                '\\' | '.' | '*' | '[' | ']' | '^' | '$' | '/' | '-' | '#' | ' ' | '\t' | '\n' => {
                    true
                }
                _ => false,
            };
            if !allowed {
                if matches!(escaped, 'z' | '_' | '%') {
                    if let Some(next) = pattern[at..].chars().next() {
                        at += next.len_utf8();
                    }
                } else if escaped == '@' {
                    while bytes.get(at).is_some_and(|c| {
                        c.is_ascii_digit() || matches!(c, b'<' | b'>' | b'=' | b'!')
                    }) {
                        at += 1;
                    }
                }
                return Err(RegexError::UnsupportedRegexAtom(pattern[start..at].into()));
            }
            continue;
        }
        if ch == '[' {
            if class_depth != 0
                && (pattern[at..].starts_with("[.") || pattern[at..].starts_with("[="))
            {
                return Err(RegexError::UnsupportedRegexAtom(
                    pattern[at..].chars().take(2).collect(),
                ));
            }
            if pattern[at..].starts_with("[:") {
                return Err(RegexError::UnsupportedRegexAtom(
                    pattern[at..]
                        .split(']')
                        .next()
                        .unwrap_or(&pattern[at..])
                        .into(),
                ));
            }
            if let Some(first) = class_first.last_mut() {
                *first = false;
            }
            class_depth += 1;
            class_first.push(true);
            class_negated.push(false);
        } else if class_depth != 0 {
            if ch == ']' {
                if class_first.last() == Some(&true) {
                    *class_first.last_mut().unwrap() = false;
                } else {
                    class_depth -= 1;
                    class_first.pop();
                    class_negated.pop();
                }
            } else if ch == '^'
                && class_first.last() == Some(&true)
                && class_negated.last() == Some(&false)
            {
                *class_negated.last_mut().unwrap() = true;
            } else {
                *class_first.last_mut().unwrap() = false;
            }
        } else if class_depth == 0 {
            if ch == '(' {
                stack.push(extended);
                if pattern[at..].starts_with("(?") {
                    let rest = &pattern[at + 2..];
                    if rest.starts_with("P<")
                        || (rest.starts_with('<')
                            && !rest.starts_with("<=")
                            && !rest.starts_with("<!"))
                    {
                        if let Some(end) = rest.find('>') {
                            at += end + 3;
                            continue;
                        }
                    } else if rest.starts_with(':') {
                        at += 3;
                        continue;
                    } else {
                        let start = at;
                        at += 2;
                        let mut enabled = true;
                        while at < bytes.len() && !matches!(bytes[at], b')' | b':') {
                            let flag = pattern[at..].chars().next().unwrap();
                            at += flag.len_utf8();
                            match flag {
                                '-' => enabled = false,
                                'i' | 's' => {}
                                'x' => extended = enabled,
                                _ => {
                                    return Err(RegexError::UnsupportedRegexAtom(
                                        pattern[start..at].into(),
                                    ))
                                }
                            }
                        }
                        if bytes.get(at) == Some(&b')') {
                            stack.pop();
                        }
                        at = (at + 1).min(bytes.len());
                        continue;
                    }
                }
            } else if ch == ')' {
                if let Some(old) = stack.pop() {
                    extended = old;
                }
            } else if ch.is_uppercase() {
                uppercase = true;
            }
        }
        at += ch.len_utf8();
    }
    Ok(uppercase)
}

/// Literal search text must not introduce spellings reserved for Vim atoms.
pub fn escape_literal(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if matches!(ch, '(' | ')' | '+' | '?' | '{' | '}' | '|') {
            result.push_str(&format!("\\x{:02X}", ch as u32));
        } else {
            if matches!(ch, '\\' | '.' | '*' | '[' | ']' | '^' | '$' | '#') {
                result.push('\\');
            }
            result.push(ch);
        }
    }
    result
}

#[derive(Clone, Debug)]
enum ReplacementAtom {
    Literal(String),
    HardBreak,
    Capture(usize),
}
#[derive(Clone, Debug)]
pub struct ReplacementTemplate {
    atoms: Vec<ReplacementAtom>,
}
#[derive(Clone, Debug)]
pub enum ExpandedFragment {
    Literal {
        text: String,
        break_offsets: Vec<usize>,
    },
    Capture(Range<usize>),
}
impl ReplacementTemplate {
    pub fn compile(template: &str, regex: &CompiledRegex) -> Result<Self, RegexError> {
        let mut atoms = Vec::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            let atom = match ch {
                '&' => ReplacementAtom::Capture(0),
                '~' => return Err(RegexError::UnsupportedReplacementAtom("~".into())),
                '$' if chars
                    .peek()
                    .is_some_and(|ch| ch.is_ascii_digit() || *ch == '{') =>
                {
                    return Err(RegexError::UnsupportedReplacementAtom(
                        "$ capture reference".into(),
                    ))
                }
                '\\' => match chars.next() {
                    Some(digit @ '0'..='9') => {
                        ReplacementAtom::Capture(digit as usize - '0' as usize)
                    }
                    Some('g') if chars.next() == Some('{') => {
                        let mut name = String::new();
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(ch) => name.push(ch),
                                None => {
                                    return Err(RegexError::UnsupportedReplacementAtom(
                                        "unclosed \\g{name}".into(),
                                    ))
                                }
                            }
                        }
                        ReplacementAtom::Capture(regex.capture_index(&name).ok_or_else(|| {
                            RegexError::UnsupportedReplacementAtom(format!(
                                "undeclared capture {name:?}"
                            ))
                        })?)
                    }
                    Some('r') => ReplacementAtom::HardBreak,
                    Some('n') => ReplacementAtom::Literal("\n".into()),
                    Some('t') => ReplacementAtom::Literal("\t".into()),
                    Some(ch @ ('\\' | '&')) => ReplacementAtom::Literal(ch.to_string()),
                    Some(other) => {
                        return Err(RegexError::UnsupportedReplacementAtom(format!("\\{other}")))
                    }
                    None => {
                        return Err(RegexError::UnsupportedReplacementAtom(
                            "trailing backslash".into(),
                        ))
                    }
                },
                other => ReplacementAtom::Literal(other.to_string()),
            };
            if let ReplacementAtom::Capture(index) = atom {
                if index >= regex.capture_count() {
                    return Err(RegexError::UnsupportedReplacementAtom(format!(
                        "undeclared capture {index}"
                    )));
                }
            }
            if let (Some(ReplacementAtom::Literal(previous)), ReplacementAtom::Literal(text)) =
                (atoms.last_mut(), &atom)
            {
                previous.push_str(text);
            } else {
                atoms.push(atom);
            }
        }
        Ok(Self { atoms })
    }
    pub fn expand(
        &self,
        matched: &RegexMatch,
        input: &RegexInput<'_>,
    ) -> Result<Vec<ExpandedFragment>, RegexError> {
        matched.validate(input.snapshot)?;
        let mut fragments = Vec::new();
        for atom in &self.atoms {
            match atom {
                ReplacementAtom::Literal(text) => fragments.push(ExpandedFragment::Literal {
                    text: text.clone(),
                    break_offsets: Vec::new(),
                }),
                ReplacementAtom::HardBreak => fragments.push(ExpandedFragment::Literal {
                    text: "\n".into(),
                    break_offsets: vec![0],
                }),
                ReplacementAtom::Capture(index) => {
                    if let Some(range) = matched.capture(*index) {
                        fragments.push(ExpandedFragment::Capture(range));
                    }
                }
            }
        }
        Ok(fragments)
    }
}

/// Buffer search policy, shared by every view of the same document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchOptions {
    pub ignorecase: bool,
    pub smartcase: bool,
    pub wrapscan: bool,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            ignorecase: false,
            smartcase: false,
            wrapscan: true,
        }
    }
}
impl SearchOptions {
    pub fn case_insensitive(self, pattern: &str) -> Result<bool, RegexError> {
        Ok(self.ignorecase && !(self.smartcase && has_smartcase_uppercase(pattern)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn edited_document(text: &str) -> Document {
        let mut document = Document::new(format!("{text}!"));
        document.delete(text.len()..text.len() + 1).unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
        document
    }

    #[test]
    fn tree_regex_captures_match_flat_oracle_across_windows() {
        let limits = RegexLimits::default();
        let patterns = [
            r"\A",
            r"\z",
            r"^",
            r"$",
            r"\b",
            r"\B",
            r"\b\w+\b",
            r"(é|𐐀|😀)(\w*)",
            r"(?s:é.*?終)",
            r"(?i:é)(?P<tail>\w+)",
            r"(?:xyz|終)?",
            r"[^\w\n]+",
            r"\n終$",
            r"a+?",
            r"(a*)(a?)",
        ];
        for padding in [4089, 4090, 4091, 4092, 4093, 4094, 4095, 4096] {
            let text = format!(
                "{} é𐐀😀e\u{301}_xyz\n終\n{} é終",
                " ".repeat(padding),
                "a".repeat(4101)
            );
            let document = edited_document(&text);
            let snapshot = document.hard_line_snapshot();
            let input = RegexInput::new(&snapshot);
            for pattern in patterns {
                let compiled = CompiledRegex::compile(pattern, false, limits).unwrap();
                let oracle = regex::RegexBuilder::new(pattern)
                    .multi_line(true)
                    .build()
                    .unwrap();
                for start in [0, padding, padding + 1, text.len()] {
                    let found = compiled
                        .find(&input, start, text.len(), &mut RegexWork::new(limits))
                        .unwrap();
                    let expected = oracle.captures_at(&text, start);
                    let captures = found.map(|matched| matched.captures);
                    let expected = expected.map(|captures| {
                        captures
                            .iter()
                            .map(|m| m.map(|m| m.start()..m.end()))
                            .collect::<Vec<_>>()
                    });
                    assert_eq!(
                        captures, expected,
                        "pattern {pattern:?}, padding {padding}, start {start}"
                    );
                }
            }
            assert!(!document.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn window_look_context_matches_whole_utf8_at_every_byte() {
        let text = format!("{}\r\n_é𐐀😀e\u{301} 𐐀\r\n", "x".repeat(4088));
        let document = edited_document(&text);
        let snapshot = document.hard_line_snapshot();
        let mut cursor = InputCursor::new(&snapshot);
        let nfa = NFA::new("").unwrap();
        let looks = [
            Look::Start,
            Look::End,
            Look::StartCRLF,
            Look::EndCRLF,
            Look::WordAscii,
            Look::WordAsciiNegate,
            Look::WordUnicode,
            Look::WordUnicodeNegate,
            Look::WordStartAscii,
            Look::WordEndAscii,
            Look::WordStartUnicode,
            Look::WordEndUnicode,
            Look::WordStartHalfAscii,
            Look::WordEndHalfAscii,
            Look::WordStartHalfUnicode,
            Look::WordEndHalfUnicode,
        ];
        for at in 0..=text.len() {
            for look in looks {
                assert_eq!(
                    cursor.matches_look(&nfa, look, at),
                    nfa.look_matcher().matches(look, text.as_bytes(), at),
                    "{look:?} at {at}"
                );
            }
        }
    }

    #[test]
    fn range_search_keeps_document_anchors_and_semantic_line_endings() {
        let mut document = Document::from_bytes_with_file_format(
            b"first\nliteral\rtarget\rlast!".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let end = document.projection().text_tree().byte_len();
        document.delete(end - 1..end).unwrap();
        let snapshot = document.hard_line_snapshot();
        let input = RegexInput::new(&snapshot);
        let limits = RegexLimits::default();
        for (pattern, start, end, expected) in [
            (r"^literal", 0, snapshot.text_length(), None),
            (r"^target$", 0, snapshot.text_length(), Some(14..20)),
            (r"\A", 14, 20, None),
            (r"\z", 14, 20, None),
            (r"^target$", 14, 20, Some(14..20)),
        ] {
            let regex = CompiledRegex::compile(pattern, false, limits).unwrap();
            let found = regex
                .find(&input, start, end, &mut RegexWork::new(limits))
                .unwrap();
            assert_eq!(found.map(|matched| matched.range()), expected, "{pattern}");
        }
        assert_eq!(
            input
                .hard_break_offsets(0..snapshot.text_length())
                .collect::<Vec<_>>(),
            vec![13, 20]
        );
        assert_eq!(
            input.hard_break_offsets(14..20).collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn repeated_short_matches_reuse_the_tree_scan_window() {
        let document = edited_document(&"a ".repeat(10_000));
        let snapshot = document.hard_line_snapshot();
        let input = RegexInput::new(&snapshot);
        let limits = RegexLimits::default();
        let regex = CompiledRegex::compile("a", false, limits).unwrap();
        let matches = regex
            .find_all(
                &input,
                0..snapshot.text_length(),
                &mut RegexWork::new(limits),
            )
            .unwrap();
        assert_eq!(matches.len(), 10_000);
        assert!(input.cursor.borrow().chunk_reads < 30);
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn large_search_does_not_flatten_or_look_up_each_byte() {
        let mut text = "line\n".repeat(200_000);
        text.push_str("needle終");
        let document = edited_document(&text);
        let snapshot = document.hard_line_snapshot();
        let mut cursor = InputCursor::new(&snapshot);
        for at in 0..snapshot.text_length() {
            assert!(cursor.byte(at).is_some());
        }
        assert!(
            cursor.chunk_reads < snapshot.text_length() / 1000,
            "{} tree reads",
            cursor.chunk_reads
        );
        let limits = RegexLimits::default();
        let regex = CompiledRegex::compile(r"^needle終$", false, limits).unwrap();
        let matched = regex
            .find(
                &RegexInput::new(&snapshot),
                0,
                snapshot.text_length(),
                &mut RegexWork::new(limits),
            )
            .unwrap()
            .unwrap();
        assert_eq!(matched.range(), 1_000_000..1_000_009);
        assert!(!document.projection().compatibility_text_is_materialized());
    }
}
