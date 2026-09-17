//! Cooperative matching against one immutable snapshot. Yielding preserves
//! Thompson threads, including greedy matches crossing a viewport or batch.
use super::*;

pub struct RegexScanner {
    snapshot: HardLineSnapshot,
    regex: Arc<CompiledRegex>,
    work: RegexWork,
    current: Threads,
    next: Threads,
    found: Option<Vec<Option<usize>>>,
    at: usize,
    end: usize,
    last_start: usize,
    previous_nonempty_end: Option<usize>,
    done: bool,
}

impl RegexScanner {
    /// Find non-overlapping matches whose start is at most `last_start`.
    /// Assertions and greedy tails still inspect the complete document.
    pub fn new(
        snapshot: HardLineSnapshot,
        regex: Arc<CompiledRegex>,
        last_start: usize,
        limits: RegexLimits,
    ) -> Self {
        let states = regex.nfa.states().len();
        let last_start = last_start.min(snapshot.text_length());
        let end = snapshot.text_length();
        Self {
            snapshot,
            regex,
            work: RegexWork::new(limits),
            current: Threads::new(states),
            next: Threads::new(states),
            found: None,
            at: 0,
            end,
            last_start,
            previous_nonempty_end: None,
            done: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.done
    }

    /// Restrict where a match may begin without changing assertion context.
    pub fn within(mut self, range: Range<usize>) -> Result<Self, RegexError> {
        let mut input = InputCursor::new(&self.snapshot);
        if range.start > range.end
            || range.end > self.snapshot.text_length()
            || !input.is_char_boundary(range.start)
            || !input.is_char_boundary(range.end)
        {
            return Err(RegexError::StaleProjection);
        }
        self.at = range.start;
        self.end = range.end;
        self.last_start = self.last_start.min(range.end);
        self.done = self.at > self.last_start;
        Ok(self)
    }

    /// A bounded number of input steps per host turn. The existing program
    /// and work limits additionally bound transitions within each step.
    pub fn advance(&mut self, max_steps: usize) -> Result<Vec<RegexMatch>, RegexError> {
        if self.done {
            return Ok(Vec::new());
        }
        let result = self.advance_inner(max_steps);
        if result.is_err() {
            self.done = true;
        }
        result
    }

    fn advance_inner(&mut self, max_steps: usize) -> Result<Vec<RegexMatch>, RegexError> {
        const BATCH_WORK: u64 = 100_000;
        let initial_work = self.work.remaining;
        let mut input = InputCursor::new(&self.snapshot);
        let end = self.end;
        let mut output = Vec::new();
        for _ in 0..max_steps {
            // Yield at a complete byte transition, preserving all VM threads.
            // A complex pattern must not multiply a host turn's input quota
            // into the entire operation's resource budget.
            if initial_work - self.work.remaining >= BATCH_WORK {
                break;
            }
            self.work.tick()?;
            if self.found.is_none() && self.at <= self.last_start && input.is_char_boundary(self.at)
            {
                self.regex.closure(
                    &mut self.current,
                    self.regex.nfa.start_anchored(),
                    vec![None; self.regex.nfa.group_info().slot_len()],
                    &mut input,
                    self.at,
                    &mut self.work,
                )?;
            }
            let byte = (self.at < end)
                .then(|| input.byte(self.at).expect("scan byte before document end"));
            for thread in &self.current.active {
                self.work.tick()?;
                let target = match self.regex.nfa.state(thread.state) {
                    State::Match { .. } => {
                        self.found = Some(thread.slots.clone());
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
                    self.regex.closure(
                        &mut self.next,
                        target,
                        thread.slots.clone(),
                        &mut input,
                        self.at + 1,
                        &mut self.work,
                    )?;
                }
            }
            if self.found.is_some() && (self.next.active.is_empty() || self.at == end) {
                let slots = self.found.take().unwrap();
                let matched = RegexMatch {
                    document: self.snapshot.document(),
                    revision: self.snapshot.revision(),
                    captures: slots
                        .chunks_exact(2)
                        .map(|pair| match (pair[0], pair[1]) {
                            (Some(start), Some(end)) => Some(start..end),
                            _ => None,
                        })
                        .collect(),
                };
                let range = matched.range();
                self.current.clear();
                self.next.clear();
                if range.is_empty() {
                    if self.previous_nonempty_end != Some(range.end) {
                        output.push(matched);
                    }
                    if let Some(next) = self.snapshot.next_grapheme_boundary(range.end) {
                        self.at = next;
                    } else {
                        self.done = true;
                        break;
                    }
                } else {
                    self.previous_nonempty_end = Some(range.end);
                    self.at = range.end;
                    output.push(matched);
                }
                if self.at > self.last_start {
                    self.done = true;
                    break;
                }
                continue;
            }
            if self.at == end || (self.at >= self.last_start && self.next.active.is_empty()) {
                self.done = true;
                break;
            }
            std::mem::swap(&mut self.current, &mut self.next);
            self.next.clear();
            self.at += 1;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn batches_preserve_priority_captures_and_unbounded_multiline_context() {
        for text in [
            "",
            "ababa\nfoo",
            "a\u{301}!\u{301} foo",
            "start\nfoo\nend foo",
        ] {
            let document = Document::new(text);
            let snapshot = document.hard_line_snapshot();
            for pattern in [
                r"\<",
                r"\>",
                r"a*",
                r"a+?",
                r"(a|ab)(b?)",
                r"(?s:start.*end)|foo",
                r"(?s:.*)",
                r"foo",
            ] {
                let limits = RegexLimits::default();
                let regex = CompiledRegex::compile(pattern, false, limits).unwrap();
                let expected = regex
                    .find_all(
                        &RegexInput::new(&snapshot),
                        0..text.len(),
                        &mut RegexWork::new(limits),
                    )
                    .unwrap();
                for steps in [1, 2, 7, 1024] {
                    let mut scanner =
                        RegexScanner::new(snapshot.clone(), regex.clone(), text.len(), limits);
                    let mut matches = Vec::new();
                    while !scanner.is_complete() {
                        matches.extend(scanner.advance(steps).unwrap());
                    }
                    assert_eq!(matches, expected, "{pattern} in {text:?}, steps {steps}");
                }
            }
        }
    }

    #[test]
    fn viewport_limit_does_not_truncate_a_greedy_match() {
        let document = Document::new("prefix\ninside\nsuffix ignored");
        let snapshot = document.hard_line_snapshot();
        let limits = RegexLimits::default();
        let regex = CompiledRegex::compile("(?s:prefix.*suffix)|ignored", false, limits).unwrap();
        let mut scanner = RegexScanner::new(snapshot, regex, 9, limits);
        let mut matches = Vec::new();
        while !scanner.is_complete() {
            matches.extend(scanner.advance(2).unwrap());
        }
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].range(), 0..20);
    }

    #[test]
    fn large_document_scan_yields_without_flattening_and_reports_limits() {
        let mut document = Document::new(format!("{}!", "line\n".repeat(200_000)));
        document.delete(1_000_000..1_000_001).unwrap();
        let limits = RegexLimits::default();
        let regex = CompiledRegex::compile("missing", false, limits).unwrap();
        let mut scanner = RegexScanner::new(
            document.hard_line_snapshot(),
            regex.clone(),
            900_000,
            limits,
        );
        assert!(scanner.advance(512).unwrap().is_empty());
        assert!(!scanner.is_complete());
        assert!(!document.projection().compatibility_text_is_materialized());
        let mut limited = RegexScanner::new(
            document.hard_line_snapshot(),
            regex,
            900_000,
            RegexLimits { work: 1, ..limits },
        );
        assert!(matches!(
            limited.advance(512),
            Err(RegexError::RegexResourceLimit(_))
        ));
        assert!(limited.is_complete());
    }

    #[test]
    fn addressed_scan_preserves_external_assertions_and_truncates_consumption() {
        let document = Document::new("x word tail");
        let snapshot = document.hard_line_snapshot();
        let limits = RegexLimits::default();
        for range in [2..6, 3..5, 2..2] {
            for pattern in [r"\<\w+\>", r"\b", r".*", r"\w+"] {
                let regex = CompiledRegex::compile(pattern, false, limits).unwrap();
                let expected = regex
                    .find_all(
                        &RegexInput::new(&snapshot),
                        range.clone(),
                        &mut RegexWork::new(limits),
                    )
                    .unwrap();
                let mut scanner = RegexScanner::new(snapshot.clone(), regex, range.end, limits)
                    .within(range.clone())
                    .unwrap();
                let mut matches = Vec::new();
                while !scanner.is_complete() {
                    matches.extend(scanner.advance(1).unwrap());
                }
                assert_eq!(matches, expected, "{pattern}, {range:?}");
            }
        }
    }

    #[test]
    fn transition_budget_yields_even_with_an_unbounded_input_quota() {
        let document = Document::new("a".repeat(1_000_000));
        let limits = RegexLimits::default();
        let regex = CompiledRegex::compile("a*X", false, limits).unwrap();
        let mut scanner =
            RegexScanner::new(document.hard_line_snapshot(), regex, 1_000_000, limits);
        assert!(scanner.advance(usize::MAX).unwrap().is_empty());
        assert!(!scanner.is_complete());
        assert!(scanner.at < 100_000);
    }
}
