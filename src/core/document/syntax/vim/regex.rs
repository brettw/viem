//! Fuel-metered anchored Thompson execution for the native Vim regular profile.
//! Every NFA state visit is interruptible; input is borrowed a leaf at a time.
use super::super::SyntaxInputSnapshot;
use regex_automata::{
    nfa::thompson::{State, NFA},
    util::{look::Look, primitives::StateID, syntax},
};
use std::sync::Arc;

#[cfg(test)]
#[path = "regex/tests.rs"]
mod tests;
#[path = "regex/translate.rs"]
mod translate;
#[path = "regex/vm.rs"]
mod vm;

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
    advanced: Option<Arc<vm::Program>>,
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
    advanced: Option<vm::Continuation>,
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
impl VimRegexContinuation {
    pub(super) fn retained_bytes(&self) -> usize {
        self.stack.capacity() * std::mem::size_of::<Thread>()
            + self.next.capacity() * std::mem::size_of::<Thread>()
            + self.seen.capacity() * std::mem::size_of::<usize>()
            + self.advanced.as_ref().map_or(0, |vm| vm.retained_bytes())
            + self.best.as_ref().map_or(0, |best| best.captures.capacity()
                * std::mem::size_of::<Option<std::ops::Range<usize>>>())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VimRegexProgress {
    Pending,
    Failed(String),
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
        let translation = translate::translate(source)?;
        let translated = &translation.regex;
        let ignore_case = translation.case.unwrap_or(ignore_case);
        if translation.groups * 2 + 2 > SLOTS {
            return Err("capture slot budget exceeded".into());
        }
        let hir = regex_syntax::ParserBuilder::new()
            .multi_line(true)
            .case_insensitive(ignore_case)
            .build()
            .parse(translated)
            .map_err(|e| format!("unsupported/invalid Vim regular pattern: {e}"))?;
        let advanced = if translation.specials.is_empty() {
            None
        } else {
            Some(Arc::new(vm::Program::compile(
                &hir,
                &translation.specials,
                translation.groups,
                ignore_case,
                translation.multiline,
                limits,
            )?))
        };
        let minimum_chars = if translation.ends.is_empty() && translation.starts.is_empty() {
            advanced
                .as_ref()
                .map_or_else(|| minimum_chars(&hir), |vm| vm.minimum_chars)
        } else {
            0
        };
        let nfa = NFA::compiler()
            .configure(NFA::config().nfa_size_limit(Some(limits.nfa_bytes)))
            .syntax(
                syntax::Config::new()
                    .multi_line(true)
                    .case_insensitive(ignore_case),
            )
            .build(&translated)
            .map_err(|e| format!("unsupported/invalid Vim regular pattern: {e}"))?;
        if nfa.states().len() > limits.states {
            return Err("pattern state budget exceeded".into());
        }
        if advanced.is_none() && nfa.group_info().slot_len() > SLOTS {
            return Err("capture slot budget exceeded".into());
        }
        if nfa.memory_usage() + advanced.as_ref().map_or(0, |vm| vm.memory_usage())
            > limits.nfa_bytes
        {
            return Err("combined Vim regex program byte budget exceeded".into());
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
            advanced,
            external_groups: translation.external,
            multiline: translation.multiline,
            source: source.into(),
            start_marker: translation.starts,
            end_marker: translation.ends,
            eol,
            minimum_chars,
        })
    }
    pub fn memory_usage(&self) -> usize {
        self.nfa.memory_usage()
            + self.source.len()
            + self.advanced.as_ref().map_or(0, |vm| vm.memory_usage())
    }
    pub fn continuation_bytes(&self) -> usize {
        self.advanced.as_ref().map_or_else(
            || {
                self.nfa.states().len()
                    * (4 * std::mem::size_of::<Thread>() + std::mem::size_of::<usize>())
            },
            |vm| vm.continuation_bytes(),
        )
    }
    pub fn start(&self, at: usize) -> VimRegexContinuation {
        VimRegexContinuation {
            advanced: self.advanced.as_ref().map(|vm| vm.start(at)),
            position: at,
            stack: if self.advanced.is_some() {
                Vec::new()
            } else {
                vec![Thread {
                    state: self.nfa.start_anchored(),
                    slots: [UNSET; SLOTS],
                }]
            },
            next: Vec::new(),
            seen: if self.advanced.is_some() {
                Vec::new()
            } else {
                vec![UNSET; self.nfa.states().len()]
            },
            union: None,
            setup: if self.advanced.is_some() {
                0
            } else {
                self.nfa.states().len()
            },
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
        if let (Some(program), Some(continuation)) = (&self.advanced, &mut c.advanced) {
            let mut result =
                program.resume(continuation, len, &byte, fuel, cancel, &mut c.inspected_end);
            if let VimRegexProgress::Complete(Some(found)) = &mut result {
                let marker = |groups: &[usize], fallback| {
                    groups
                        .iter()
                        .filter_map(|id| {
                            found
                                .captures
                                .get(*id)
                                .and_then(|range| range.as_ref())
                                .map(|range| range.start)
                        })
                        .max()
                        .unwrap_or(fallback)
                };
                found.start = marker(&self.start_marker, found.start);
                found.end = marker(&self.end_marker, found.end);
            }
            return result;
        }
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
                VimRegexProgress::Failed(error) => return Err(error),
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
    if spec.ends_with('\\') {
        spec.pop();
    }
    let lazy = spec.starts_with('-');
    let spec = spec.strip_prefix('-').unwrap_or(&spec);
    if !spec.chars().all(|c| c.is_ascii_digit() || c == ',') {
        return Err("unsupported repetition".into());
    }
    let mut out = if spec.is_empty() {
        "*".into()
    } else {
        if spec.starts_with(',') {
            format!("{{0{spec}}}")
        } else {
            format!("{{{spec}}}")
        }
    };
    if lazy {
        out.push('?');
    }
    Ok((out, i))
}
