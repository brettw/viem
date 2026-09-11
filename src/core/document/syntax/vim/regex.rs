//! Fuel-metered anchored Thompson execution for the native Vim regular profile.
//! Every NFA state visit is interruptible; input is borrowed a leaf at a time.
use super::super::SyntaxInputSnapshot;
use regex_automata::{
    nfa::thompson::{State, NFA},
    util::{look::Look, primitives::StateID, syntax},
};
use std::sync::Arc;

const SLOTS: usize = 64;
const UNSET: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
pub struct VimRegexLimits {
    pub pattern_bytes: usize,
    pub nfa_bytes: usize,
    pub states: usize,
}
impl Default for VimRegexLimits {
    fn default() -> Self {
        Self {
            pattern_bytes: 8192,
            nfa_bytes: 1024 * 1024,
            states: 8192,
        }
    }
}
#[derive(Clone, Debug)]
pub struct VimPattern {
    nfa: Arc<NFA>,
    pub(super) external_groups: Vec<usize>,
    pub(super) multiline: bool,
    pub(super) source: Arc<str>,
    pub(super) eol: bool,
    pub(super) minimum_chars: usize,
    start_marker: Vec<usize>,
    end_marker: Vec<usize>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimRegexMatch {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<Option<std::ops::Range<usize>>>,
}
#[derive(Clone, Debug)]
struct Thread {
    state: StateID,
    slots: [usize; SLOTS],
}
#[derive(Clone, Debug)]
pub struct VimRegexContinuation {
    position: usize,
    stack: Vec<Thread>,
    next: Vec<Thread>,
    seen: Vec<usize>,
    union: Option<(Thread, usize)>,
    setup: usize,
    best: Option<VimRegexMatch>,
    done: bool,
    pub inspected_end: usize,
    pub instructions: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VimRegexProgress {
    Pending,
    Complete(Option<VimRegexMatch>),
}

impl VimPattern {
    /// Neovim 0.11 query.lua adds very-magic to patterns of at least two bytes
    /// unless the pattern already begins with an explicit magic-mode switch.
    pub fn compile_neovim_query(source: &str, limits: VimRegexLimits) -> Result<Self, String> {
        if source.len() < 2
            || ["\\v", "\\m", "\\M", "\\V"]
                .iter()
                .any(|mode| source.starts_with(mode))
        {
            Self::compile(source, false, limits)
        } else {
            Self::compile(&format!("\\v{source}"), false, limits)
        }
    }
    pub fn compile(
        source: &str,
        ignore_case: bool,
        limits: VimRegexLimits,
    ) -> Result<Self, String> {
        if source.len() > limits.pattern_bytes {
            return Err("pattern byte budget exceeded".into());
        }
        let (translated, external_groups, multiline, start_marker, end_marker, case_override) =
            translate(source)?;
        let minimum_chars = if end_marker.is_empty() && start_marker.is_empty() {
            regex_syntax::Parser::new()
                .parse(&translated)
                .map(|hir| minimum_chars(&hir))
                .unwrap_or(0)
        } else {
            0
        };
        let nfa = NFA::compiler()
            .configure(NFA::config().nfa_size_limit(Some(limits.nfa_bytes)))
            .syntax(
                syntax::Config::new()
                    .multi_line(true)
                    .case_insensitive(case_override.unwrap_or(ignore_case)),
            )
            .build(&translated)
            .map_err(|e| format!("unsupported/invalid Vim regular pattern: {e}"))?;
        if nfa.states().len() > limits.states {
            return Err("pattern state budget exceeded".into());
        }
        if nfa.group_info().slot_len() > SLOTS {
            return Err("capture slot budget exceeded".into());
        }
        let eol = nfa.states().iter().any(|s| {
            matches!(
                s,
                State::Look {
                    look: Look::EndLF,
                    ..
                }
            )
        });
        Ok(Self {
            nfa: Arc::new(nfa),
            external_groups,
            multiline,
            source: source.into(),
            start_marker,
            end_marker,
            eol,
            minimum_chars,
        })
    }
    pub fn memory_usage(&self) -> usize {
        self.nfa.memory_usage() + self.source.len()
    }
    pub fn continuation_bytes(&self) -> usize {
        self.nfa.states().len() * (4 * std::mem::size_of::<Thread>() + std::mem::size_of::<usize>())
    }
    pub fn start(&self, at: usize) -> VimRegexContinuation {
        VimRegexContinuation {
            position: at,
            stack: vec![Thread {
                state: self.nfa.start_anchored(),
                slots: [UNSET; SLOTS],
            }],
            next: Vec::new(),
            seen: vec![UNSET; self.nfa.states().len()],
            union: None,
            setup: self.nfa.states().len(),
            best: None,
            done: false,
            inspected_end: at,
            instructions: 0,
        }
    }
    pub fn resume(
        &self,
        c: &mut VimRegexContinuation,
        input: &SyntaxInputSnapshot,
        fuel: &mut usize,
    ) -> VimRegexProgress {
        self.resume_with_control(c, input, fuel, &mut || false)
    }
    pub fn resume_with_control(
        &self,
        c: &mut VimRegexContinuation,
        input: &SyntaxInputSnapshot,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> VimRegexProgress {
        let before = *fuel;
        let result = self.resume_reader(
            c,
            input.byte_len(),
            |at| input.chunk_at(at).first().copied(),
            fuel,
            cancel,
        );
        c.instructions = c.instructions.saturating_add(before - *fuel);
        result
    }
    fn resume_reader(
        &self,
        c: &mut VimRegexContinuation,
        len: usize,
        byte: impl Fn(usize) -> Option<u8>,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> VimRegexProgress {
        if c.done {
            return VimRegexProgress::Complete(c.best.clone());
        }
        if cancel() {
            return VimRegexProgress::Pending;
        }
        let setup = c.setup.min(*fuel);
        *fuel -= setup;
        c.setup -= setup;
        if c.setup > 0 {
            return VimRegexProgress::Pending;
        }
        let mut ticks = 0usize;
        loop {
            if ticks & 63 == 0 && cancel() {
                return VimRegexProgress::Pending;
            }
            ticks += 1;
            if *fuel == 0 {
                return VimRegexProgress::Pending;
            }
            *fuel -= 1;
            if let Some((thread, count)) = c.union.take() {
                let State::Union { alternates } = self.nfa.state(thread.state) else {
                    unreachable!()
                };
                let mut branch = thread.clone();
                branch.state = alternates[count - 1];
                c.stack.push(branch);
                if count > 1 {
                    c.union = Some((thread, count - 1));
                }
                continue;
            }
            if let Some(mut thread) = c.stack.pop() {
                let id = thread.state.as_usize();
                if c.seen[id] == c.position {
                    continue;
                }
                c.seen[id] = c.position;
                match self.nfa.state(thread.state) {
                    State::ByteRange { trans } => {
                        c.inspected_end =
                            c.inspected_end.max(c.position.saturating_add(1).min(len));
                        if byte(c.position).is_some_and(|b| trans.matches_byte(b)) {
                            thread.state = trans.next;
                            c.next.push(thread);
                        }
                    }
                    State::Sparse(trans) => {
                        c.inspected_end =
                            c.inspected_end.max(c.position.saturating_add(1).min(len));
                        if let Some(next) = byte(c.position).and_then(|b| trans.matches_byte(b)) {
                            thread.state = next;
                            c.next.push(thread);
                        }
                    }
                    State::Dense(trans) => {
                        c.inspected_end =
                            c.inspected_end.max(c.position.saturating_add(1).min(len));
                        if let Some(next) = byte(c.position).and_then(|b| trans.matches_byte(b)) {
                            thread.state = next;
                            c.next.push(thread);
                        }
                    }
                    State::Look { look, next } => {
                        c.inspected_end =
                            c.inspected_end.max(c.position.saturating_add(4).min(len));
                        if look_matches(*look, c.position, len, &byte) {
                            thread.state = *next;
                            c.stack.push(thread);
                        }
                    }
                    State::Union { alternates } => {
                        if !alternates.is_empty() {
                            c.union = Some((thread, alternates.len()));
                        }
                    }
                    State::BinaryUnion { alt1, alt2 } => {
                        let mut second = thread.clone();
                        second.state = *alt2;
                        c.stack.push(second);
                        thread.state = *alt1;
                        c.stack.push(thread);
                    }
                    State::Capture { next, slot, .. } => {
                        thread.slots[slot.as_usize()] = c.position;
                        thread.state = *next;
                        c.stack.push(thread);
                    }
                    State::Match { .. } => {
                        let captures = thread.slots[..self.nfa.group_info().slot_len()]
                            .chunks_exact(2)
                            .map(|p| (p[0] != UNSET && p[1] != UNSET).then_some(p[0]..p[1]))
                            .collect();
                        let marker = |groups: &[usize], fallback| {
                            groups
                                .iter()
                                .map(|g| thread.slots[g * 2])
                                .filter(|p| *p != UNSET)
                                .max()
                                .unwrap_or(fallback)
                        };
                        c.best = Some(VimRegexMatch {
                            start: marker(&self.start_marker, thread.slots[0]),
                            end: marker(&self.end_marker, thread.slots[1]),
                            captures,
                        });
                        // Earlier consuming threads may still produce the preferred greedy
                        // match; lower-priority alternatives must not replace this match.
                        c.stack.clear();
                    }
                    State::Fail => {}
                }
            } else if c.next.is_empty() || c.position >= len {
                c.done = true;
                return VimRegexProgress::Complete(c.best.clone());
            } else {
                c.position += 1;
                c.stack = std::mem::take(&mut c.next);
                c.stack.reverse();
            }
        }
    }
    /// Bounded convenience for a query predicate's already-bounded capture text.
    /// Returns a budget error rather than interpreting incomplete work as false.
    pub fn is_match_text(&self, text: &str, mut fuel: usize) -> Result<bool, String> {
        self.is_match_text_with_fuel(text, &mut fuel)
    }
    pub fn is_match_text_with_fuel(&self, text: &str, fuel: &mut usize) -> Result<bool, String> {
        self.is_match_text_with_control(text, fuel, &mut || false)
    }
    pub fn is_match_text_with_control(
        &self,
        text: &str,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> Result<bool, String> {
        if text.len() > 256 * 1024 {
            return Err("Vim predicate text budget exceeded".into());
        }
        for at in text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
        {
            match self.resume_reader(
                &mut self.start(at),
                text.len(),
                |at| text.as_bytes().get(at).copied(),
                fuel,
                cancel,
            ) {
                VimRegexProgress::Pending => {
                    return Err("Vim predicate cancelled or instruction budget exceeded".into())
                }
                VimRegexProgress::Complete(Some(_)) => return Ok(true),
                VimRegexProgress::Complete(None) => {}
            }
        }
        Ok(false)
    }
}

fn word_at(at: usize, len: usize, byte: &impl Fn(usize) -> Option<u8>) -> bool {
    if at >= len {
        return false;
    }
    let mut bytes = [0; 4];
    let Some(first) = byte(at) else {
        return false;
    };
    bytes[0] = first;
    let count = if first < 128 {
        1
    } else if first < 224 {
        2
    } else if first < 240 {
        3
    } else {
        4
    };
    for (i, b) in bytes.iter_mut().enumerate().take(count).skip(1) {
        *b = byte(at + i).unwrap_or(0);
    }
    std::str::from_utf8(&bytes[..count])
        .ok()
        .and_then(|s| s.chars().next())
        .is_some_and(is_keyword)
}
pub(super) fn is_keyword(c: char) -> bool {
    c == '_' || c.is_alphabetic() || c.is_ascii_digit() || ('\u{c0}'..='\u{ff}').contains(&c)
}
fn look_matches(look: Look, at: usize, len: usize, byte: &impl Fn(usize) -> Option<u8>) -> bool {
    let mut before = at.saturating_sub(1);
    while before > 0 && byte(before).is_some_and(|b| b & 0xc0 == 0x80) {
        before -= 1;
    }
    let left = at > 0 && word_at(before, len, byte);
    let right = word_at(at, len, byte);
    match look {
        Look::Start => at == 0,
        Look::End => at == len,
        Look::StartLF => at == 0 || byte(at - 1) == Some(b'\n'),
        Look::EndLF => at == len || byte(at) == Some(b'\n'),
        Look::WordAscii | Look::WordUnicode => left != right,
        Look::WordAsciiNegate | Look::WordUnicodeNegate => left == right,
        Look::WordStartAscii | Look::WordStartUnicode => !left && right,
        Look::WordEndAscii | Look::WordEndUnicode => left && !right,
        Look::WordStartHalfAscii | Look::WordStartHalfUnicode => !left,
        Look::WordEndHalfAscii | Look::WordEndHalfUnicode => !right,
        Look::StartCRLF | Look::EndCRLF => false, // never emitted by this compiler
    }
}

/// Translate only the explicitly supported regular subset. No replacement of
/// unsupported Vim operators with a different regular expression is permitted.
fn translate(
    source: &str,
) -> Result<
    (
        String,
        Vec<usize>,
        bool,
        Vec<usize>,
        Vec<usize>,
        Option<bool>,
    ),
    String,
> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut external = Vec::new();
    let mut groups = 0usize;
    let mut i = 0;
    let mut branch_start = true;
    let mut multiline = false;
    let mut start_marker = Vec::new();
    let mut end_marker = Vec::new();
    let mut very = false;
    let mut case_override = None;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c == '[' {
            branch_start = false;
            out.push('[');
            if chars.get(i) == Some(&'^') {
                out.push('^');
                out.push_str("\\n");
                i += 1;
            }
            let mut first = true;
            let mut closed = false;
            while i < chars.len() {
                let c = chars[i];
                i += 1;
                if c == '-' && chars.get(i) == Some(&'-') {
                    return Err(
                        "ambiguous double-dash character class is outside native profile v1".into(),
                    );
                }
                if c == ']' && !first {
                    out.push(c);
                    closed = true;
                    break;
                }
                if c == '\\' {
                    let escaped = *chars.get(i).ok_or("unterminated class escape")?;
                    i += 1;
                    if !matches!(escaped, '\\' | ']' | '^' | '-' | 't' | 'r' | 'n') {
                        return Err(format!("unsupported Vim class escape: \\{escaped}"));
                    }
                    out.push('\\');
                    out.push(escaped);
                    multiline |= escaped == 'n';
                } else {
                    if c == '&' || c == '~' {
                        out.push('\\');
                    }
                    out.push(c);
                }
                first = false;
            }
            if !closed {
                return Err("unterminated Vim character class".into());
            }
            continue;
        }
        if c != '\\' {
            if c == '~' {
                return Err(
                    "substitution-dependent Vim atoms are outside native profile v1".into(),
                );
            }
            if c == '\n' {
                multiline = true;
            }
            if very {
                match c {
                    '%' if chars.get(i) == Some(&'(') => {
                        out.push_str("(?:");
                        i += 1;
                        branch_start = true;
                        continue;
                    }
                    '<' | '>' => {
                        out.push_str(if c == '<' { "\\b{start}" } else { "\\b{end}" });
                        branch_start = false;
                        continue;
                    }
                    '=' => {
                        out.push('?');
                        continue;
                    }
                    '@' | '&' | '%' | '~' => {
                        return Err(format!("unsupported very-magic Vim operator: {c}"))
                    }
                    '(' => groups += 1,
                    '{' => {
                        let (spec, tail) = repetition(&chars, i)?;
                        i = tail;
                        out.push_str(&spec);
                        continue;
                    }
                    '?' if i >= 2 && matches!(chars[i - 2], '*' | '+' | '?' | '}') => {
                        return Err("Vim lazy repetition uses {-}, not a second quantifier".into())
                    }
                    _ => {}
                }
            }
            let end_branch = i == chars.len()
                || chars.get(i) == Some(&'\\') && matches!(chars.get(i + 1), Some('|' | ')' | 'n'))
                || very && matches!(chars.get(i), Some('|' | ')'));
            if !very && matches!(c, '+' | '?' | '(' | ')' | '|' | '{' | '}')
                || c == '^' && !branch_start
                || c == '$' && !end_branch
                || c == '*' && branch_start
            {
                out.push('\\');
            }
            branch_start = very && matches!(c, '(' | '|') || c == '\n';
            out.push(c);
            continue;
        }
        let c = *chars.get(i).ok_or("trailing Vim escape")?;
        i += 1;
        if very && !c.is_alphanumeric() {
            out.push_str(&regex::escape(&c.to_string()));
            branch_start = false;
            continue;
        }
        match c {
            'm' => very = false,
            'v' => very = true,
            'M' | 'V' => return Err("nomagic modes are outside native profile v1".into()),
            '+' | '?' | '|' | '(' | ')' => {
                if c == '(' {
                    groups += 1;
                }
                out.push(c);
            }
            '=' => out.push('?'),
            '<' => out.push_str("\\b{start}"),
            '>' => out.push_str("\\b{end}"),
            's' => out.push_str("[ \\t]"),
            'S' => out.push_str("[^ \\t\\n]"),
            'd' => out.push_str("[0-9]"),
            'D' => out.push_str("[^0-9\\n]"),
            'w' => out.push_str("[A-Za-z0-9_]"),
            'W' => out.push_str("[^A-Za-z0-9_\\n]"),
            'h' => out.push_str("[A-Za-z_]"),
            'H' => out.push_str("[^A-Za-z_\\n]"),
            'a' => out.push_str("[A-Za-z]"),
            'A' => out.push_str("[^A-Za-z\\n]"),
            'x' => out.push_str("[A-Fa-f0-9]"),
            'o' => out.push_str("[0-7]"),
            'k' | 'i' => out.push_str("[_0-9\\p{L}\\x{c0}-\\x{ff}]"),
            'K' | 'I' => out.push_str("[_\\p{L}\\x{c0}-\\x{ff}]"),
            't' | 'r' | 'n' => {
                out.push('\\');
                out.push(c);
                multiline |= c == 'n';
            }
            'c' => case_override = Some(true),
            'C' => case_override = Some(false),
            '%' if chars.get(i) == Some(&'(') => {
                out.push_str("(?:");
                i += 1;
            }
            'z' if chars.get(i) == Some(&'(') => {
                groups += 1;
                external.push(groups);
                out.push('(');
                i += 1;
            }
            'z' if matches!(chars.get(i), Some('s' | 'e')) => {
                groups += 1;
                if chars[i] == 's' {
                    start_marker.push(groups);
                } else {
                    end_marker.push(groups);
                }
                out.push_str("()");
                i += 1;
            }
            '{' => {
                let (spec, tail) = repetition(&chars, i)?;
                i = tail;
                out.push_str(&spec);
            }
            '_' => return Err("multiline character classes are outside native profile v1".into()),
            '@' | '&' | '1'..='9' | 'z' | '%' => {
                return Err(format!("unsupported Vim assertion/backreference: \\{c}"))
            }
            c if !c.is_alphanumeric() => {
                out.push_str(&regex::escape(&c.to_string()));
            }
            _ => return Err(format!("unsupported Vim escape: \\{c}")),
        }
        if c != 'm' && c != 'v' && c != 'c' && c != 'C' {
            branch_start = c == '|'
                || c == '('
                || c == '%'
                || c == 'z' && chars.get(i.wrapping_sub(1)) == Some(&'(')
                || c == 'n';
        }
    }
    Ok((
        out,
        external,
        multiline,
        start_marker,
        end_marker,
        case_override,
    ))
}
fn minimum_chars(hir: &regex_syntax::hir::Hir) -> usize {
    use regex_syntax::hir::HirKind;
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => 0,
        HirKind::Literal(l) => std::str::from_utf8(&l.0).map_or(0, |s| s.chars().count()),
        HirKind::Class(_) => 1,
        HirKind::Repetition(r) => minimum_chars(&r.sub).saturating_mul(r.min as usize),
        HirKind::Capture(c) => minimum_chars(&c.sub),
        HirKind::Concat(v) => v.iter().map(minimum_chars).fold(0, usize::saturating_add),
        HirKind::Alternation(v) => v.iter().map(minimum_chars).min().unwrap_or(0),
    }
}
fn repetition(chars: &[char], mut i: usize) -> Result<(String, usize), String> {
    let mut spec = String::new();
    while i < chars.len() && chars[i] != '}' {
        spec.push(chars[i]);
        i += 1;
    }
    if i == chars.len() {
        return Err("unterminated repetition".into());
    }
    i += 1;
    let lazy = spec.starts_with('-');
    let spec = spec.strip_prefix('-').unwrap_or(&spec);
    if !spec.chars().all(|c| c.is_ascii_digit() || c == ',') {
        return Err("unsupported repetition".into());
    }
    let mut out = if spec.is_empty() {
        "*".into()
    } else {
        format!("{{{spec}}}")
    };
    if lazy {
        out.push('?');
    }
    Ok((out, i))
}
