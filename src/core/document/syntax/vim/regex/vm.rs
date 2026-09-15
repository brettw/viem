//! Resumable execution for Vim's non-regular atoms. Every bytecode instruction,
//! assertion candidate and backreference scalar comparison consumes fuel. The
//! explicit alternative stack has a reported, enforced memory ceiling.
use super::{
    look_matches, VimKeyword, VimRegexLimits, VimRegexMatch, VimRegexProgress, SLOTS, UNSET,
};
use regex_automata::util::look::Look;
use regex_syntax::hir::{Class, Hir, HirKind};

const LOOPS: usize = 32;
const MAX_THREADS: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(super) enum AssertionKind {
    Ahead(bool),
    Behind {
        positive: bool,
        limit: Option<usize>,
    },
    Atomic,
}
#[derive(Clone, Debug)]
pub(super) enum Special {
    Assertion(AssertionKind),
    Backreference(usize),
    ControlClass {
        negate: bool,
    },
    Position {
        line: bool,
        comparison: std::cmp::Ordering,
        value: usize,
    },
}
#[derive(Clone, Debug)]
enum Instruction {
    Accept,
    Character(Vec<(u32, u32)>, usize),
    ControlClass(Vec<(u32, u32)>, bool, usize),
    Look(Look, usize),
    Position {
        line: bool,
        comparison: std::cmp::Ordering,
        value: usize,
        next: usize,
    },
    Split(usize, usize),
    Save(usize, usize),
    Guard(usize, usize),
    Assert {
        start: usize,
        kind: AssertionKind,
        maximum_bytes: Option<usize>,
        next: usize,
    },
    Backreference(usize, usize),
}
#[derive(Clone, Debug)]
pub(super) struct Program {
    keyword: VimKeyword,
    code: Vec<Instruction>,
    start: usize,
    groups: usize,
    ignore_case: bool,
    multiline: bool,
    pub(super) minimum_chars: usize,
}

impl Program {
    pub(super) fn compile(
        hir: &Hir,
        specials: &[Special],
        groups: usize,
        ignore_case: bool,
        multiline: bool,
        limits: VimRegexLimits,
        keyword: &VimKeyword,
    ) -> Result<Self, String> {
        let mut compiler = Compiler {
            code: Vec::new(),
            range_bytes: 0,
            loops: 0,
            specials,
            limits,
        };
        compiler.emit(Instruction::Accept)?;
        let start = compiler.node(hir, 0)?;
        Ok(Self {
            keyword: keyword.clone(),
            code: compiler.code,
            start,
            groups,
            ignore_case,
            multiline,
            minimum_chars: extent(hir, specials).0,
        })
    }
    pub(super) fn memory_usage(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.code.capacity() * std::mem::size_of::<Instruction>()
            + self
                .code
                .iter()
                .map(|i| match i {
                    Instruction::Character(r, _) | Instruction::ControlClass(r, _, _) => {
                        r.capacity() * 8
                    }
                    _ => 0,
                })
                .sum::<usize>()
    }
    pub(super) fn continuation_bytes(&self) -> usize {
        MAX_THREADS * (std::mem::size_of::<Thread>() + std::mem::size_of::<Frame>())
    }
    pub(super) fn start(&self, at: usize) -> Continuation {
        let mut slots = [UNSET; SLOTS];
        slots[0] = at;
        Continuation {
            frames: vec![Frame::new(Thread::new(self.start, at, slots), None, 0)],
            alternatives: Vec::new(),
            retained: 1,
            result: None,
        }
    }
    pub(super) fn resume(
        &self,
        c: &mut Continuation,
        len: usize,
        byte: &impl Fn(usize) -> Option<u8>,
        position: &impl Fn(usize) -> Option<(usize, usize)>,
        fuel: &mut usize,
        cancel: &mut dyn FnMut() -> bool,
        inspected_end: &mut usize,
    ) -> VimRegexProgress {
        if let Some(result) = &c.result {
            return result.clone();
        }
        let mut ticks = 0usize;
        while *fuel > 0 {
            if ticks & 63 == 0 && cancel() {
                return VimRegexProgress::Pending;
            }
            ticks += 1;
            *fuel -= 1;
            if c.retained >= MAX_THREADS {
                let error = VimRegexProgress::Failed(
                    "Vim regular-expression workspace budget exceeded".into(),
                );
                c.frames.clear();
                c.alternatives.clear();
                c.result = Some(error.clone());
                return error;
            }
            let frame = c.frames.last_mut().expect("active regex frame");
            if let Some(call) = &mut frame.call {
                if call.preparing {
                    // Discover the permitted lookbehind window one scalar per
                    // instruction, then try candidates in Vim's document order.
                    if !call.previous_candidate(byte) {
                        call.preparing = false;
                        frame.current =
                            Some(Thread::new(call.start, call.candidate, call.parent.slots));
                    }
                    continue;
                }
            }
            let Some(mut thread) = frame.current.take().or_else(|| {
                (c.alternatives.len() > frame.alternative_base)
                    .then(|| c.alternatives.pop().unwrap())
            }) else {
                if let Some(result) = self.return_frame(c, None, byte) {
                    return result;
                }
                continue;
            };
            match &self.code[thread.ip] {
                Instruction::Accept => {
                    if frame.call.as_ref().is_some_and(|call| {
                        matches!(call.kind, AssertionKind::Behind { .. })
                            && thread.position != call.origin
                    }) {
                        c.retained -= 1;
                    } else if let Some(result) = self.return_frame(c, Some(thread), byte) {
                        return result;
                    }
                }
                Instruction::Character(ranges, next) => {
                    *inspected_end =
                        (*inspected_end).max(thread.position.saturating_add(4).min(len));
                    if let Some((ch, width)) = scalar(thread.position, byte) {
                        let ch = ch as u32;
                        let index = ranges.partition_point(|range| range.1 < ch);
                        if ranges.get(index).is_some_and(|range| range.0 <= ch) {
                            thread.position += width;
                            thread.ip = *next;
                            frame.current = Some(thread);
                            continue;
                        }
                    }
                    c.retained -= 1;
                }
                Instruction::ControlClass(ranges, negate, next) => {
                    *inspected_end =
                        (*inspected_end).max(thread.position.saturating_add(4).min(len));
                    if let Some((ch, width)) = scalar(thread.position, byte) {
                        let member = |ch| {
                            let index = ranges.partition_point(|range| range.1 < ch);
                            ranges.get(index).is_some_and(|range| range.0 <= ch)
                        };
                        let source_newline = ch == '\n'
                            && position(thread.position).is_none_or(|(line, _)| line != 0);
                        let contains = if matches!(ch, '\0' | '\n') {
                            member(0) || member(10)
                        } else {
                            member(ch as u32)
                        };
                        if !source_newline && contains != *negate {
                            thread.position += width;
                            thread.ip = *next;
                            frame.current = Some(thread);
                            continue;
                        }
                    }
                    c.retained -= 1;
                }
                Instruction::Look(look, next) => {
                    *inspected_end =
                        (*inspected_end).max(thread.position.saturating_add(4).min(len));
                    if look_matches(*look, thread.position, len, byte, &self.keyword) {
                        thread.ip = *next;
                        frame.current = Some(thread);
                    } else {
                        c.retained -= 1;
                    }
                }
                Instruction::Position {
                    line,
                    comparison,
                    value,
                    next,
                } => {
                    let Some((source_line, column)) = position(thread.position) else {
                        let error = VimRegexProgress::Failed(
                            "Vim source-position context is unavailable or invalid".into(),
                        );
                        c.frames.clear();
                        c.alternatives.clear();
                        c.result = Some(error.clone());
                        return error;
                    };
                    let actual = if *line { source_line } else { column };
                    // Vim's string-match APIs have no buffer line context;
                    // their lookup uses line zero, which never satisfies a
                    // line assertion, including a '<' or '>' comparison.
                    if !(*line && source_line == 0) && actual.cmp(value) == *comparison {
                        thread.ip = *next;
                        frame.current = Some(thread);
                    } else {
                        c.retained -= 1;
                    }
                }
                Instruction::Split(first, second) => {
                    let mut other = thread.clone();
                    other.ip = *second;
                    c.alternatives.push(other);
                    c.retained += 1;
                    thread.ip = *first;
                    frame.current = Some(thread);
                }
                Instruction::Save(slot, next) => {
                    thread.slots[*slot] = thread.position;
                    thread.ip = *next;
                    frame.current = Some(thread);
                }
                Instruction::Guard(index, next) => {
                    if thread.loops[*index] == thread.position {
                        c.retained -= 1;
                    } else {
                        thread.loops[*index] = thread.position;
                        thread.ip = *next;
                        frame.current = Some(thread);
                    }
                }
                Instruction::Assert {
                    start,
                    kind,
                    maximum_bytes,
                    next,
                } => {
                    let call = Call {
                        kind: *kind,
                        start: *start,
                        next: *next,
                        origin: thread.position,
                        candidate: thread.position,
                        segment_end: thread.position,
                        crossed_line: false,
                        allow_previous_line: self.multiline,
                        preparing: matches!(kind, AssertionKind::Behind { .. }),
                        maximum_bytes: *maximum_bytes,
                        parent: thread,
                    };
                    let child = Thread::new(*start, call.origin, call.parent.slots);
                    c.frames
                        .push(Frame::new(child, Some(call), c.alternatives.len()));
                    c.retained += 1;
                }
                Instruction::Backreference(group, next) => {
                    let start = thread.slots[group * 2];
                    let end = thread.slots[group * 2 + 1];
                    // Vim matches an unmatched optional capture as an empty string.
                    if start == UNSET || end == UNSET || start + thread.reference >= end {
                        thread.reference = 0;
                        thread.ip = *next;
                        frame.current = Some(thread);
                        continue;
                    }
                    *inspected_end =
                        (*inspected_end).max(thread.position.saturating_add(4).min(len));
                    let original = scalar(start + thread.reference, byte);
                    let current = scalar(thread.position, byte);
                    if let (Some((a, aw)), Some((b, bw))) = (original, current) {
                        if a == b || self.ignore_case && case_equal(a, b) {
                            thread.reference += aw;
                            thread.position += bw;
                            frame.current = Some(thread);
                            continue;
                        }
                    }
                    c.retained -= 1;
                }
            }
        }
        VimRegexProgress::Pending
    }

    fn return_frame(
        &self,
        c: &mut Continuation,
        found: Option<Thread>,
        byte: &impl Fn(usize) -> Option<u8>,
    ) -> Option<VimRegexProgress> {
        let mut frame = c.frames.pop().unwrap();
        c.retained -= c.alternatives.len() - frame.alternative_base + usize::from(found.is_some());
        c.alternatives.truncate(frame.alternative_base);
        let Some(mut call) = frame.call.take() else {
            let result = VimRegexProgress::Complete(found.map(|mut thread| {
                thread.slots[1] = thread.position;
                VimRegexMatch {
                    start: thread.slots[0],
                    end: thread.position,
                    captures: thread.slots[..(self.groups + 1) * 2]
                        .chunks_exact(2)
                        .map(|pair| {
                            (pair[0] != UNSET && pair[1] != UNSET).then_some(pair[0]..pair[1])
                        })
                        .collect(),
                }
            }));
            c.result = Some(result.clone());
            return Some(result);
        };
        if found.is_none()
            && matches!(call.kind, AssertionKind::Behind { .. })
            && call.next_candidate(byte)
        {
            let child = Thread::new(call.start, call.candidate, call.parent.slots);
            c.frames
                .push(Frame::new(child, Some(call), c.alternatives.len()));
            c.retained += 1;
            return None;
        }
        let positive = match call.kind {
            AssertionKind::Ahead(p) | AssertionKind::Behind { positive: p, .. } => p,
            AssertionKind::Atomic => true,
        };
        if found.is_some() == positive {
            if let Some(found) = found {
                call.parent.slots = found.slots;
                if matches!(call.kind, AssertionKind::Atomic) {
                    call.parent.position = found.position;
                }
            }
            call.parent.ip = call.next;
            c.frames.last_mut().unwrap().current = Some(call.parent);
        } else {
            c.retained -= 1;
        }
        None
    }
}

#[derive(Clone, Debug)]
struct Thread {
    ip: usize,
    position: usize,
    slots: [usize; SLOTS],
    loops: [usize; LOOPS],
    reference: usize,
}
impl Thread {
    fn new(ip: usize, position: usize, slots: [usize; SLOTS]) -> Self {
        Self {
            ip,
            position,
            slots,
            loops: [UNSET; LOOPS],
            reference: 0,
        }
    }
}
#[derive(Clone, Debug)]
struct Call {
    kind: AssertionKind,
    start: usize,
    next: usize,
    origin: usize,
    candidate: usize,
    segment_end: usize,
    crossed_line: bool,
    allow_previous_line: bool,
    preparing: bool,
    maximum_bytes: Option<usize>,
    parent: Thread,
}
impl Call {
    fn next_candidate(&mut self, byte: &impl Fn(usize) -> Option<u8>) -> bool {
        if self.candidate >= self.origin {
            return false;
        }
        let Some((_, width)) = scalar(self.candidate, byte) else {
            return false;
        };
        self.candidate += width;
        self.candidate <= self.origin
    }
    fn previous_candidate(&mut self, byte: &impl Fn(usize) -> Option<u8>) -> bool {
        if self.candidate == 0
            || self
                .maximum_bytes
                .is_some_and(|maximum| self.origin - self.candidate >= maximum)
        {
            return false;
        }
        if let AssertionKind::Behind {
            limit: Some(limit), ..
        } = self.kind
        {
            // Vim measures the start of each backward step. A complete Unicode
            // scalar may cross the final byte of the limit.
            if self.segment_end - self.candidate >= limit {
                return false;
            }
        }
        let mut previous = self.candidate - 1;
        while previous > 0 && byte(previous).is_some_and(|b| b & 0xc0 == 0x80) {
            previous -= 1;
        }
        if byte(previous) == Some(b'\n') {
            if self.crossed_line || !self.allow_previous_line {
                return false;
            }
            self.crossed_line = true;
            self.segment_end = previous;
        }
        self.candidate = previous;
        true
    }
}
#[derive(Clone, Debug)]
struct Frame {
    current: Option<Thread>,
    alternative_base: usize,
    call: Option<Call>,
}
impl Frame {
    fn new(thread: Thread, call: Option<Call>, alternative_base: usize) -> Self {
        Self {
            current: Some(thread),
            alternative_base,
            call,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Continuation {
    frames: Vec<Frame>,
    alternatives: Vec<Thread>,
    retained: usize,
    result: Option<VimRegexProgress>,
}

impl Continuation {
    pub(super) fn retained_bytes(&self) -> usize {
        self.frames.capacity() * std::mem::size_of::<Frame>()
            + self.alternatives.capacity() * std::mem::size_of::<Thread>()
    }
}

fn case_equal(a: char, b: char) -> bool {
    // A single Unicode scalar has a small, table-bounded simple-fold closure.
    let mut class =
        regex_syntax::hir::ClassUnicode::new([regex_syntax::hir::ClassUnicodeRange::new(a, a)]);
    class.case_fold_simple();
    class
        .ranges()
        .iter()
        .any(|range| range.start() <= b && b <= range.end())
}

fn scalar(at: usize, byte: &impl Fn(usize) -> Option<u8>) -> Option<(char, usize)> {
    let first = byte(at)?;
    let width = if first < 128 {
        1
    } else if first < 224 {
        2
    } else if first < 240 {
        3
    } else {
        4
    };
    let mut bytes = [0; 4];
    bytes[0] = first;
    for (i, b) in bytes.iter_mut().enumerate().take(width).skip(1) {
        *b = byte(at + i)?;
    }
    Some((
        std::str::from_utf8(&bytes[..width]).ok()?.chars().next()?,
        width,
    ))
}

struct Compiler<'a> {
    code: Vec<Instruction>,
    range_bytes: usize,
    loops: usize,
    specials: &'a [Special],
    limits: VimRegexLimits,
}
impl Compiler<'_> {
    fn emit(&mut self, instruction: Instruction) -> Result<usize, String> {
        let capacity = if self.code.len() == self.code.capacity() {
            self.code.capacity().saturating_mul(2).max(4)
        } else {
            self.code.capacity()
        };
        let ranges = match &instruction {
            Instruction::Character(ranges, _) | Instruction::ControlClass(ranges, _, _) => {
                ranges.capacity() * std::mem::size_of::<(u32, u32)>()
            }
            _ => 0,
        };
        let bytes = std::mem::size_of::<Program>()
            + capacity * std::mem::size_of::<Instruction>()
            + self.range_bytes
            + ranges;
        if self.code.len() >= self.limits.states || bytes > self.limits.nfa_bytes {
            return Err("Vim regex bytecode budget exceeded".into());
        }
        if capacity > self.code.capacity() {
            self.code
                .try_reserve_exact(capacity - self.code.len())
                .map_err(|_| "Vim regex bytecode allocation failed")?;
        }
        self.range_bytes += ranges;
        let at = self.code.len();
        self.code.push(instruction);
        Ok(at)
    }
    fn node(&mut self, hir: &Hir, next: usize) -> Result<usize, String> {
        match hir.kind() {
            HirKind::Empty => Ok(next),
            HirKind::Literal(literal) => {
                let text =
                    std::str::from_utf8(&literal.0).map_err(|_| "non-Unicode Vim literal")?;
                let mut start = next;
                for c in text.chars().rev() {
                    start = self.emit(Instruction::Character(vec![(c as u32, c as u32)], start))?;
                }
                Ok(start)
            }
            HirKind::Class(class) => {
                let ranges = match class {
                    Class::Unicode(c) => c
                        .ranges()
                        .iter()
                        .map(|r| (r.start() as u32, r.end() as u32))
                        .collect(),
                    Class::Bytes(c) => c
                        .ranges()
                        .iter()
                        .map(|r| (r.start() as u32, r.end() as u32))
                        .collect(),
                };
                self.emit(Instruction::Character(ranges, next))
            }
            HirKind::Look(look) => {
                use regex_syntax::hir::Look as H;
                let look = match look {
                    H::Start => Look::Start,
                    H::End => Look::End,
                    H::StartLF => Look::StartLF,
                    H::EndLF => Look::EndLF,
                    H::StartCRLF => Look::StartCRLF,
                    H::EndCRLF => Look::EndCRLF,
                    H::WordAscii => Look::WordAscii,
                    H::WordAsciiNegate => Look::WordAsciiNegate,
                    H::WordUnicode => Look::WordUnicode,
                    H::WordUnicodeNegate => Look::WordUnicodeNegate,
                    H::WordStartAscii => Look::WordStartAscii,
                    H::WordEndAscii => Look::WordEndAscii,
                    H::WordStartUnicode => Look::WordStartUnicode,
                    H::WordEndUnicode => Look::WordEndUnicode,
                    H::WordStartHalfAscii => Look::WordStartHalfAscii,
                    H::WordEndHalfAscii => Look::WordEndHalfAscii,
                    H::WordStartHalfUnicode => Look::WordStartHalfUnicode,
                    H::WordEndHalfUnicode => Look::WordEndHalfUnicode,
                };
                self.emit(Instruction::Look(look, next))
            }
            HirKind::Concat(nodes) => {
                let mut start = next;
                for node in nodes.iter().rev() {
                    start = self.node(node, start)?;
                }
                Ok(start)
            }
            HirKind::Alternation(nodes) => {
                let mut start = self.node(nodes.last().ok_or("empty Vim alternative")?, next)?;
                for node in nodes[..nodes.len() - 1].iter().rev() {
                    let first = self.node(node, next)?;
                    start = self.emit(Instruction::Split(first, start))?;
                }
                Ok(start)
            }
            HirKind::Capture(capture) => {
                let name = capture
                    .name
                    .as_deref()
                    .ok_or("unnamed translated Vim capture")?;
                if let Some(id) = name.strip_prefix("vimspecial") {
                    let id = id
                        .parse::<usize>()
                        .map_err(|_| "invalid Vim instruction identity")?;
                    match self.specials.get(id).ok_or("missing Vim instruction")? {
                        Special::Assertion(kind) => {
                            let kind = *kind;
                            let maximum_bytes = extent(&capture.sub, self.specials).1;
                            let start = self.node(&capture.sub, 0)?;
                            self.emit(Instruction::Assert {
                                start,
                                kind,
                                maximum_bytes,
                                next,
                            })
                        }
                        Special::Backreference(group) => {
                            self.emit(Instruction::Backreference(*group, next))
                        }
                        Special::ControlClass { negate } => {
                            let ranges = match capture.sub.kind() {
                                HirKind::Class(Class::Unicode(class)) => class
                                    .ranges()
                                    .iter()
                                    .map(|r| (r.start() as u32, r.end() as u32))
                                    .collect(),
                                HirKind::Literal(literal) => {
                                    let text = std::str::from_utf8(&literal.0)
                                        .map_err(|_| "non-Unicode Vim control class")?;
                                    let mut chars = text.chars();
                                    let ch = chars.next().ok_or("empty Vim control class")?;
                                    if chars.next().is_some() {
                                        return Err("invalid Vim control class extent".into());
                                    }
                                    vec![(ch as u32, ch as u32)]
                                }
                                _ => return Err("invalid Vim control class".into()),
                            };
                            self.emit(Instruction::ControlClass(ranges, *negate, next))
                        }
                        Special::Position {
                            line,
                            comparison,
                            value,
                        } => self.emit(Instruction::Position {
                            line: *line,
                            comparison: *comparison,
                            value: *value,
                            next,
                        }),
                    }
                } else {
                    let id = name
                        .strip_prefix("vimcap")
                        .and_then(|id| id.parse::<usize>().ok())
                        .ok_or("invalid Vim capture identity")?;
                    let end = self.emit(Instruction::Save(id * 2 + 1, next))?;
                    let start = self.node(&capture.sub, end)?;
                    self.emit(Instruction::Save(id * 2, start))
                }
            }
            HirKind::Repetition(repetition) => {
                let mut start = next;
                if let Some(maximum) = repetition.max {
                    for _ in repetition.min..maximum {
                        let body = self.node(&repetition.sub, start)?;
                        start = self.emit(if repetition.greedy {
                            Instruction::Split(body, start)
                        } else {
                            Instruction::Split(start, body)
                        })?;
                    }
                } else {
                    let split = self.emit(Instruction::Accept)?;
                    let mut body = self.node(&repetition.sub, split)?;
                    if extent(&repetition.sub, self.specials).0 == 0 {
                        // Only nullable bodies can revisit a repetition without
                        // advancing. Consuming loops need no saved position;
                        // counting them exhausted guards on J's number syntax.
                        if self.loops >= LOOPS {
                            return Err("Vim nullable repetition guard budget exceeded".into());
                        }
                        let guard = self.loops;
                        self.loops += 1;
                        body = self.emit(Instruction::Guard(guard, body))?;
                    }
                    self.code[split] = if repetition.greedy {
                        Instruction::Split(body, next)
                    } else {
                        Instruction::Split(next, body)
                    };
                    start = split;
                }
                for _ in 0..repetition.min {
                    start = self.node(&repetition.sub, start)?;
                }
                Ok(start)
            }
        }
    }
}

/// Lower bound in Unicode scalars and upper bound in bytes of consumed input.
/// Zero-width assertions contribute no extent; atomic atoms retain their body.
fn extent(hir: &Hir, specials: &[Special]) -> (usize, Option<usize>) {
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => (0, Some(0)),
        HirKind::Literal(l) => (
            std::str::from_utf8(&l.0).map_or(0, |s| s.chars().count()),
            Some(l.0.len()),
        ),
        HirKind::Class(Class::Unicode(class)) => {
            (1, class.ranges().iter().map(|r| r.end().len_utf8()).max())
        }
        HirKind::Class(Class::Bytes(_)) => (1, Some(1)),
        HirKind::Capture(capture) => {
            if let Some(id) = capture
                .name
                .as_deref()
                .and_then(|name| name.strip_prefix("vimspecial"))
                .and_then(|id| id.parse::<usize>().ok())
            {
                match specials[id] {
                    Special::Assertion(AssertionKind::Atomic) => extent(&capture.sub, specials),
                    Special::Assertion(_) => (0, Some(0)),
                    Special::Backreference(_) => (0, None),
                    Special::Position { .. } => (0, Some(0)),
                    Special::ControlClass { .. } => (1, Some(4)),
                }
            } else {
                extent(&capture.sub, specials)
            }
        }
        HirKind::Repetition(repetition) => {
            let (minimum, maximum) = extent(&repetition.sub, specials);
            (
                minimum.saturating_mul(repetition.min as usize),
                maximum.and_then(|n| {
                    if n == 0 {
                        Some(0)
                    } else {
                        repetition
                            .max
                            .and_then(|count| n.checked_mul(count as usize))
                    }
                }),
            )
        }
        HirKind::Concat(nodes) => nodes.iter().map(|node| extent(node, specials)).fold(
            (0usize, Some(0usize)),
            |(min, max), (a, b)| {
                (
                    min.saturating_add(a),
                    max.zip(b).and_then(|(a, b)| a.checked_add(b)),
                )
            },
        ),
        HirKind::Alternation(nodes) => {
            let mut result = (usize::MAX, Some(0));
            for node in nodes {
                let (min, max) = extent(node, specials);
                result.0 = result.0.min(min);
                result.1 = result.1.zip(max).map(|(a, b)| a.max(b));
            }
            result
        }
    }
}
