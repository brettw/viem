//! Fuel-metered anchored Thompson execution for the native Vim regular profile.
//! Every NFA state visit is interruptible; input is borrowed a leaf at a time.
use super::super::SyntaxInputSnapshot;
use regex_automata::{
    nfa::thompson::{State, NFA},
    util::{look::Look, primitives::StateID},
};
use std::sync::Arc;

#[path = "regex/keyword.rs"]
mod keyword;
#[cfg(test)]
#[path = "regex/tests.rs"]
mod tests;
#[path = "regex/translate.rs"]
mod translate;
#[path = "regex/vm.rs"]
mod vm;
pub use keyword::VimKeyword;

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
            pattern_bytes: 16 * 1024,
            nfa_bytes: 1024 * 1024,
            states: 8192,
        }
    }
}
#[derive(Clone, Debug)]
pub struct VimPattern {
    keyword: VimKeyword,
    ignore_case: bool,
    start_anchor: Option<regex_syntax::hir::Look>,
    nfa: Option<Arc<NFA>>,
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
            + self.best.as_ref().map_or(0, |best| {
                best.captures.capacity() * std::mem::size_of::<Option<std::ops::Range<usize>>>()
            })
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
        Self::compile_with_keyword(source, ignore_case, limits, &VimKeyword::default())
    }
    pub fn compile_with_keyword(
        source: &str,
        ignore_case: bool,
        limits: VimRegexLimits,
        keyword: &VimKeyword,
    ) -> Result<Self, String> {
        if source.len() > limits.pattern_bytes {
            return Err("pattern byte budget exceeded".into());
        }
        let translation = translate::translate(source, keyword)?;
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
        // Scalar classes can be compact in the VM but expand into many UTF-8
        // byte states (e.g. a Unicode keyword class repeated 33 times). Try the
        // linear NFA first, then use the same bounded VM when representation
        // limits make the byte program too large. Never retain an unused NFA.
        let regular = translation.specials.is_empty();
        let nfa = if regular {
            match NFA::compiler()
                .configure(NFA::config().nfa_size_limit(Some(limits.nfa_bytes)))
                .build_from_hir(&hir)
            {
                Ok(nfa) if nfa.states().len() <= limits.states => Some(Arc::new(nfa)),
                Ok(_) => None,
                Err(error) if error.size_limit().is_some() => None,
                Err(error) => {
                    return Err(format!("unsupported/invalid Vim regular pattern: {error}"))
                }
            }
        } else {
            None
        };
        if nfa
            .as_ref()
            .is_some_and(|nfa| nfa.group_info().slot_len() > SLOTS)
        {
            return Err("capture slot budget exceeded".into());
        }
        let advanced = if nfa.is_none() {
            Some(Arc::new(vm::Program::compile(
                &hir,
                &translation.specials,
                translation.groups,
                ignore_case,
                translation.multiline,
                limits,
                keyword,
            )?))
        } else {
            None
        };
        let start_anchor = if regular {
            let looks = hir.properties().look_set_prefix();
            [
                regex_syntax::hir::Look::Start,
                regex_syntax::hir::Look::StartLF,
            ]
            .into_iter()
            .find(|look| looks.contains(*look))
        } else {
            // Synthetic assertion bodies do not have their ordinary HIR
            // semantics: a negative lookahead may invert an apparent anchor.
            None
        };
        let minimum_chars = if translation.ends.is_empty() && translation.starts.is_empty() {
            advanced
                .as_ref()
                .map_or_else(|| minimum_chars(&hir), |vm| vm.minimum_chars)
        } else {
            0
        };
        if nfa.as_ref().map_or(0, |nfa| nfa.memory_usage())
            + advanced.as_ref().map_or(0, |vm| vm.memory_usage())
            > limits.nfa_bytes
        {
            return Err("Vim regex program byte budget exceeded".into());
        }
        let eol = hir
            .properties()
            .look_set()
            .contains(regex_syntax::hir::Look::EndLF);
        Ok(Self {
            keyword: keyword.clone(),
            ignore_case,
            start_anchor,
            nfa,
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
    /// Syntax keyword options are program-wide even when declared after rules.
    /// Finalization rebinds them while retaining each rule's case declaration.
    pub fn rebind_keyword(
        &self,
        keyword: &VimKeyword,
        limits: VimRegexLimits,
    ) -> Result<Self, String> {
        if &self.keyword == keyword {
            Ok(self.clone())
        } else {
            Self::compile_with_keyword(&self.source, self.ignore_case, limits, keyword)
        }
    }
    pub fn memory_usage(&self) -> usize {
        self.nfa.as_ref().map_or(0, |nfa| nfa.memory_usage())
            + self.source.len()
            + self.advanced.as_ref().map_or(0, |vm| vm.memory_usage())
    }
    pub fn continuation_bytes(&self) -> usize {
        self.advanced.as_ref().map_or_else(
            || {
                self.nfa.as_ref().expect("NFA program").states().len()
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
                    state: self.nfa.as_ref().expect("NFA program").start_anchored(),
                    slots: [UNSET; SLOTS],
                }]
            },
            next: Vec::new(),
            seen: if self.advanced.is_some() {
                Vec::new()
            } else {
                vec![UNSET; self.nfa.as_ref().expect("NFA program").states().len()]
            },
            union: None,
            setup: if self.advanced.is_some() {
                0
            } else {
                self.nfa.as_ref().expect("NFA program").states().len()
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
            &|at| {
                let row = input.text_tree().hard_line_at_byte(at).ok()?;
                let start = input.text_tree().hard_line_start(row).ok()?;
                Some((row + 1, at - start + 1))
            },
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
        position: &impl Fn(usize) -> Option<(usize, usize)>,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> VimRegexProgress {
        if let (Some(program), Some(continuation)) = (&self.advanced, &mut c.advanced) {
            let mut result = program.resume(
                continuation,
                len,
                &byte,
                position,
                fuel,
                cancel,
                &mut c.inspected_end,
            );
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
        let nfa = self.nfa.as_ref().expect("NFA program without VM");
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
                let State::Union { alternates } = nfa.state(thread.state) else {
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
                match nfa.state(thread.state) {
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
                        if look_matches(*look, c.position, len, &byte, &self.keyword) {
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
                        let captures = thread.slots[..nfa.group_info().slot_len()]
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
    /// Like Vim's matchstr(), string inputs have no buffer-line identity and
    /// count byte columns from string start. Snapshot execution uses source rows.
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
        self.find_text_with_control(text, 0, fuel, cancel)
            .map(|found| found.is_some())
    }
    /// Run exactly one anchored attempt at an explicit UTF-8 boundary. Syntax
    /// group-name expansion supplies ^...$ patterns, so searching other starts
    /// would repeatedly initialize the same NFA for impossible candidates.
    pub fn matches_text_at_with_control(
        &self,
        text: &str,
        start: usize,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> Result<bool, String> {
        if text.len() > 256 * 1024 {
            return Err("Vim predicate text budget exceeded".into());
        }
        if !text.is_char_boundary(start) {
            return Err("Vim predicate start is not a valid UTF-8 boundary".into());
        }
        match self.resume_reader(
            &mut self.start(start),
            text.len(),
            |at| text.as_bytes().get(at).copied(),
            &|at| (at <= text.len()).then_some((0, at + 1)),
            fuel,
            cancel,
        ) {
            VimRegexProgress::Complete(found) => Ok(found.is_some()),
            VimRegexProgress::Failed(error) => Err(error),
            VimRegexProgress::Pending => {
                Err("Vim predicate cancelled or instruction budget exceeded".into())
            }
        }
    }
    /// Find a match starting at or after a checked UTF-8 boundary. Candidate
    /// starts share one caller-owned instruction budget, including assertions.
    pub fn find_text_with_control(
        &self,
        text: &str,
        start: usize,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
    ) -> Result<Option<VimRegexMatch>, String> {
        if text.len() > 256 * 1024 {
            return Err("Vim predicate text budget exceeded".into());
        }
        if !text.is_char_boundary(start) {
            return Err("Vim predicate start is not a valid UTF-8 boundary".into());
        }
        for at in text[start..]
            .char_indices()
            .map(|(at, _)| start + at)
            .chain(std::iter::once(text.len()))
        {
            if let Some(anchor) = self.start_anchor {
                if at != 0 {
                    if *fuel == 0 || cancel() {
                        return Err("Vim predicate cancelled or instruction budget exceeded".into());
                    }
                    *fuel -= 1;
                    if anchor == regex_syntax::hir::Look::Start {
                        return Ok(None);
                    }
                    if text.as_bytes()[at - 1] != b'\n' {
                        continue;
                    }
                }
            }
            match self.resume_reader(
                &mut self.start(at),
                text.len(),
                |at| text.as_bytes().get(at).copied(),
                &|at| (at <= text.len()).then_some((0, at + 1)),
                fuel,
                cancel,
            ) {
                VimRegexProgress::Pending => {
                    return Err("Vim predicate cancelled or instruction budget exceeded".into())
                }
                VimRegexProgress::Failed(error) => return Err(error),
                VimRegexProgress::Complete(Some(found)) => return Ok(Some(found)),
                VimRegexProgress::Complete(None) => {}
            }
        }
        Ok(None)
    }
}

fn word_at(
    at: usize,
    len: usize,
    byte: &impl Fn(usize) -> Option<u8>,
    keyword: &VimKeyword,
) -> bool {
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
        .is_some_and(|c| keyword.contains(c))
}
fn look_matches(
    look: Look,
    at: usize,
    len: usize,
    byte: &impl Fn(usize) -> Option<u8>,
    keyword: &VimKeyword,
) -> bool {
    let mut before = at.saturating_sub(1);
    while before > 0 && byte(before).is_some_and(|b| b & 0xc0 == 0x80) {
        before -= 1;
    }
    let left = at > 0 && word_at(before, len, byte, keyword);
    let right = word_at(at, len, byte, keyword);
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
