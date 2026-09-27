//! Incremental, snapshot-bound word discovery for manual completion.
//!
//! Neither construction nor polling flattens a document or a hard line. Unicode
//! segmentation itself is resumable: even a single enormous grapheme or word
//! yields between bounded UTF-8 chunks. Dropping a search cancels all its work.

use super::{DocumentId, HardLineSnapshot, PositionError, Revision, TextPoint};
use std::collections::HashSet;
use std::ops::Range;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

const CHUNK_BYTES: usize = 1024;
pub const MAX_COMPLETION_WORD_BYTES: usize = 16 * 1024;
const MAX_CANDIDATES: usize = 4096;
const MAX_CANDIDATE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WordCompletionDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordCompletionPrefix {
    /// Snapshot-local range in the search's explicitly identified snapshot.
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WordCompletionBatch {
    /// Present once, in the batch that finishes discovering the prefix.
    pub prefix: Option<WordCompletionPrefix>,
    /// New, unique words, in the requested directional document order.
    pub candidates: Vec<String>,
    pub complete: bool,
    /// A word/prefix or result-memory limit was reached. An oversized candidate
    /// is skipped; an oversized prefix or a full result budget ends the search.
    pub truncated: bool,
    /// Charged UTF-8 copy/segmentation work. Never exceeds the supplied budget.
    pub bytes_scanned: usize,
}

#[derive(Clone, Debug)]
enum Phase {
    Validate,
    Prefix,
    Scan { wrapped: bool },
    Complete,
}

#[derive(Clone, Debug)]
enum CopyPurpose {
    Prefix,
    Candidate,
}

#[derive(Clone, Debug)]
struct PendingCopy {
    range: Range<usize>,
    text: String,
    purpose: CopyPurpose,
}

/// Disposable query state. All retained ordinals name this immutable snapshot;
/// callers must validate document/revision and their own session identity before
/// applying a result to a live view.
#[derive(Clone, Debug)]
pub struct WordCompletionSearch {
    snapshot: HardLineSnapshot,
    caret: usize,
    direction: WordCompletionDirection,
    phase: Phase,
    navigator: GraphemeStream,
    prefix_start: usize,
    prefix: Option<WordCompletionPrefix>,
    token: Option<Range<usize>>,
    skip_token: bool,
    pending_copy: Option<PendingCopy>,
    seen: HashSet<String>,
    candidate_bytes: usize,
    truncated: bool,
}

impl WordCompletionSearch {
    pub fn begin(
        snapshot: HardLineSnapshot,
        caret: TextPoint,
        direction: WordCompletionDirection,
    ) -> Result<Self, PositionError> {
        if caret.document() != snapshot.document() {
            return Err(PositionError::WrongDocument {
                expected: snapshot.document(),
                actual: caret.document(),
            });
        }
        if caret.revision() != snapshot.revision() {
            return Err(PositionError::WrongSnapshot {
                expected: snapshot.revision(),
                actual: caret.revision(),
            });
        }
        if caret.offset() > snapshot.text_length()
            || (!snapshot.byte_chunk_at(caret.offset()).is_empty()
                && snapshot.byte_chunk_at(caret.offset())[0] & 0xc0 == 0x80)
        {
            return Err(PositionError::InvalidUnicodeBoundary {
                offset: caret.offset(),
            });
        }
        let at = caret.offset();
        let mut navigator = GraphemeStream::new(
            at,
            snapshot.text_length(),
            WordCompletionDirection::Backward,
        );
        // Boundary validation needs the following scalar, even though the
        // subsequent prefix walk travels backward.
        if at < snapshot.text_length() {
            navigator.load = Some(Load::After(at));
        }
        Ok(Self {
            navigator,
            snapshot,
            caret: at,
            direction,
            phase: Phase::Validate,
            prefix_start: at,
            prefix: None,
            token: None,
            skip_token: false,
            pending_copy: None,
            seen: HashSet::new(),
            candidate_bytes: 0,
            truncated: false,
        })
    }

    pub fn document(&self) -> DocumentId {
        self.snapshot.document()
    }
    pub fn revision(&self) -> Revision {
        self.snapshot.revision()
    }
    pub fn prefix(&self) -> Option<&WordCompletionPrefix> {
        self.prefix.as_ref()
    }
    pub fn is_complete(&self) -> bool {
        matches!(self.phase, Phase::Complete)
    }

    /// Advance by at most `byte_budget` charged work units. A budget of at
    /// least four permits progress across every UTF-8 scalar. Small budgets
    /// may yield before copying a scalar; zero performs no work.
    pub fn advance(&mut self, byte_budget: usize) -> WordCompletionBatch {
        let mut remaining = byte_budget;
        let mut batch = WordCompletionBatch::default();
        while remaining > 0 && !self.is_complete() {
            if self.pending_copy.is_some() {
                if !self.copy_pending(&mut remaining, &mut batch) {
                    break;
                }
                continue;
            }
            match self.phase {
                Phase::Validate => match self.navigator.validate(&self.snapshot, &mut remaining) {
                    StreamResult::Pending => break,
                    StreamResult::Ready(true) => self.phase = Phase::Prefix,
                    StreamResult::Ready(false) => self.phase = Phase::Complete,
                },
                Phase::Prefix => {
                    let range = match self.navigator.next(&self.snapshot, &mut remaining) {
                        StreamResult::Pending => break,
                        StreamResult::Ready(range) => range,
                    };
                    if let Some(range) = range.filter(|range| self.is_keyword(range.start)) {
                        self.prefix_start = range.start;
                        if self.caret - self.prefix_start > MAX_COMPLETION_WORD_BYTES {
                            self.truncated = true;
                            self.phase = Phase::Complete;
                        }
                    } else {
                        self.pending_copy = Some(PendingCopy {
                            range: self.prefix_start..self.caret,
                            text: String::new(),
                            purpose: CopyPurpose::Prefix,
                        });
                    }
                }
                Phase::Scan { wrapped } => {
                    let end = match (self.direction, wrapped) {
                        (WordCompletionDirection::Forward, false) => self.snapshot.text_length(),
                        (WordCompletionDirection::Forward, true) => self.prefix_start,
                        (WordCompletionDirection::Backward, false) => 0,
                        (WordCompletionDirection::Backward, true) => self.caret,
                    };
                    if self.navigator.position == end {
                        // At the backward wrap endpoint, a pending token is the
                        // suffix of the occurrence being completed.
                        if self.direction == WordCompletionDirection::Backward && wrapped {
                            self.token = None;
                        } else {
                            self.finish_token();
                        }
                        if wrapped {
                            // A final pending candidate must be copied before
                            // marking the search complete.
                            if self.pending_copy.is_none() {
                                self.phase = Phase::Complete;
                            }
                        } else {
                            self.phase = Phase::Scan { wrapped: true };
                            let start = match self.direction {
                                WordCompletionDirection::Forward => 0,
                                WordCompletionDirection::Backward => self.snapshot.text_length(),
                            };
                            self.navigator = GraphemeStream::new(
                                start,
                                self.snapshot.text_length(),
                                self.direction,
                            );
                            self.skip_token = false;
                        }
                        continue;
                    }
                    let range = match self.navigator.next(&self.snapshot, &mut remaining) {
                        StreamResult::Pending => break,
                        StreamResult::Ready(Some(range)) => range,
                        StreamResult::Ready(None) => {
                            self.phase = Phase::Complete;
                            continue;
                        }
                    };
                    if self.is_keyword(range.start) {
                        match &mut self.token {
                            Some(token) => {
                                token.start = token.start.min(range.start);
                                token.end = token.end.max(range.end);
                            }
                            None => self.token = Some(range),
                        }
                    } else {
                        self.finish_token();
                        self.skip_token = false;
                    }
                }
                Phase::Complete => break,
            }
        }
        batch.complete = self.is_complete();
        batch.truncated = self.truncated;
        batch.bytes_scanned = byte_budget - remaining;
        batch
    }

    fn is_keyword(&self, at: usize) -> bool {
        // Rope leaves always begin/end at scalar boundaries. The first scalar
        // of an extended grapheme determines the shared Vim keyword class.
        let chunk = self.snapshot.byte_chunk_at(at);
        let scalar_len = match chunk.first().copied() {
            Some(0..=0x7f) => 1,
            Some(0xc0..=0xdf) => 2,
            Some(0xe0..=0xef) => 3,
            Some(_) => 4,
            None => return false,
        };
        std::str::from_utf8(&chunk[..scalar_len])
            .ok()
            .and_then(|chunk| chunk.chars().next())
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
    }

    fn finish_token(&mut self) {
        let Some(range) = self.token.take() else {
            return;
        };
        if self.skip_token {
            return;
        }
        if range.len() > MAX_COMPLETION_WORD_BYTES {
            self.truncated = true;
            return;
        }
        if range.len()
            < self
                .prefix
                .as_ref()
                .expect("scanning follows prefix")
                .text
                .len()
        {
            return;
        }
        self.pending_copy = Some(PendingCopy {
            range,
            text: String::new(),
            purpose: CopyPurpose::Candidate,
        });
    }

    fn copy_pending(&mut self, remaining: &mut usize, batch: &mut WordCompletionBatch) -> bool {
        let pending = self.pending_copy.as_mut().expect("pending copy exists");
        let start = pending.range.start + pending.text.len();
        if start < pending.range.end {
            let end = scalar_floor(
                &self.snapshot,
                (start + (*remaining).min(CHUNK_BYTES)).min(pending.range.end),
            );
            if end == start {
                return false;
            }
            pending.text.push_str(
                &self
                    .snapshot
                    .slice_utf8(start..end)
                    .expect("validated scalar chunk"),
            );
            *remaining -= end - start;
            if end < pending.range.end {
                return true;
            }
        }
        let pending = self.pending_copy.take().expect("copy completed");
        match pending.purpose {
            CopyPurpose::Prefix => {
                let prefix = WordCompletionPrefix {
                    range: pending.range,
                    text: pending.text,
                };
                batch.prefix = Some(prefix.clone());
                self.prefix = Some(prefix);
                self.phase = Phase::Scan { wrapped: false };
                let start = match self.direction {
                    WordCompletionDirection::Forward => self.caret,
                    WordCompletionDirection::Backward => self.prefix_start,
                };
                self.navigator =
                    GraphemeStream::new(start, self.snapshot.text_length(), self.direction);
                self.skip_token = self.direction == WordCompletionDirection::Forward;
            }
            CopyPurpose::Candidate => {
                let word = pending.text;
                let prefix = &self.prefix.as_ref().expect("candidate follows prefix").text;
                if word != *prefix && word.starts_with(prefix) && !self.seen.contains(&word) {
                    if self.seen.len() == MAX_CANDIDATES
                        || self.candidate_bytes + word.len() > MAX_CANDIDATE_BYTES
                    {
                        self.truncated = true;
                        self.phase = Phase::Complete;
                    } else {
                        self.candidate_bytes += word.len();
                        self.seen.insert(word.clone());
                        batch.candidates.push(word);
                    }
                }
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
enum Load {
    Before(usize),
    After(usize),
    Context(usize),
}

#[derive(Clone, Debug)]
struct GraphemeStream {
    cursor: GraphemeCursor,
    position: usize,
    direction: WordCompletionDirection,
    chunk: String,
    chunk_start: usize,
    load: Option<Load>,
}

enum StreamResult<T> {
    Pending,
    Ready(T),
}

impl GraphemeStream {
    fn new(at: usize, len: usize, direction: WordCompletionDirection) -> Self {
        Self {
            cursor: GraphemeCursor::new(at, len, true),
            position: at,
            direction,
            chunk: String::new(),
            chunk_start: at,
            load: if len == 0 {
                None
            } else if at == len || direction == WordCompletionDirection::Backward {
                Some(if at == 0 {
                    Load::After(0)
                } else {
                    Load::Before(at)
                })
            } else {
                Some(Load::After(at))
            },
        }
    }

    fn load_chunk(&mut self, snapshot: &HardLineSnapshot, remaining: &mut usize) -> bool {
        let Some(load) = self.load else {
            return true;
        };
        let size = (*remaining).min(CHUNK_BYTES);
        let range = match load {
            Load::After(start) => {
                start..scalar_floor(snapshot, (start + size).min(snapshot.text_length()))
            }
            Load::Before(end) | Load::Context(end) => {
                scalar_ceil(snapshot, end.saturating_sub(size))..end
            }
        };
        if range.is_empty() {
            return false;
        }
        let chunk = snapshot
            .slice_utf8(range.clone())
            .expect("scalar-aligned stream chunk");
        *remaining -= range.len();
        if matches!(load, Load::Context(_)) {
            self.cursor.provide_context(&chunk, range.start);
        } else {
            self.chunk_start = range.start;
            self.chunk = chunk;
        }
        self.load = None;
        true
    }

    fn incomplete(&mut self, result: GraphemeIncomplete) {
        self.load = Some(match result {
            GraphemeIncomplete::PreContext(end) => Load::Context(end),
            GraphemeIncomplete::PrevChunk => Load::Before(self.chunk_start),
            GraphemeIncomplete::NextChunk => Load::After(self.chunk_start + self.chunk.len()),
            GraphemeIncomplete::InvalidOffset => unreachable!("cursor and chunks share a snapshot"),
        });
    }

    fn validate(
        &mut self,
        snapshot: &HardLineSnapshot,
        remaining: &mut usize,
    ) -> StreamResult<bool> {
        loop {
            if *remaining == 0 || !self.load_chunk(snapshot, remaining) {
                return StreamResult::Pending;
            }
            if *remaining == 0 {
                return StreamResult::Pending;
            }
            *remaining -= 1;
            match self.cursor.is_boundary(&self.chunk, self.chunk_start) {
                Ok(valid) => return StreamResult::Ready(valid),
                Err(error) => self.incomplete(error),
            }
        }
    }

    fn next(
        &mut self,
        snapshot: &HardLineSnapshot,
        remaining: &mut usize,
    ) -> StreamResult<Option<Range<usize>>> {
        loop {
            if *remaining == 0 || !self.load_chunk(snapshot, remaining) {
                return StreamResult::Pending;
            }
            if *remaining == 0 {
                return StreamResult::Pending;
            }
            *remaining -= 1;
            let result = match self.direction {
                WordCompletionDirection::Forward => {
                    self.cursor.next_boundary(&self.chunk, self.chunk_start)
                }
                WordCompletionDirection::Backward => {
                    self.cursor.prev_boundary(&self.chunk, self.chunk_start)
                }
            };
            match result {
                Ok(Some(next)) => {
                    let range = self.position.min(next)..self.position.max(next);
                    self.position = next;
                    return StreamResult::Ready(Some(range));
                }
                Ok(None) => return StreamResult::Ready(None),
                Err(error) => self.incomplete(error),
            }
        }
    }
}

fn scalar_floor(snapshot: &HardLineSnapshot, mut at: usize) -> usize {
    while at > 0 && at < snapshot.text_length() && snapshot.byte_chunk_at(at)[0] & 0xc0 == 0x80 {
        at -= 1;
    }
    at
}

fn scalar_ceil(snapshot: &HardLineSnapshot, mut at: usize) -> usize {
    while at < snapshot.text_length() && snapshot.byte_chunk_at(at)[0] & 0xc0 == 0x80 {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    fn collect(
        document: &Document,
        at: usize,
        direction: WordCompletionDirection,
        budget: usize,
    ) -> (WordCompletionPrefix, Vec<String>, bool) {
        let mut search = WordCompletionSearch::begin(
            document.hard_line_snapshot(),
            document.text_point(at).unwrap(),
            direction,
        )
        .unwrap();
        let mut words = Vec::new();
        for _ in 0..1_000_000 {
            let batch = search.advance(budget);
            assert!(batch.bytes_scanned <= budget);
            words.extend(batch.candidates);
            if batch.complete {
                return (search.prefix().unwrap().clone(), words, batch.truncated);
            }
        }
        panic!("completion search did not terminate");
    }

    #[test]
    fn directional_search_wraps_deduplicates_and_excludes_the_current_occurrence() {
        let document = Document::new("alpha alpine alchemy al almanac alpha");
        let at = "alpha alpine alchemy al".len();
        let (prefix, forward, _) = collect(&document, at, WordCompletionDirection::Forward, 17);
        assert_eq!(
            prefix,
            WordCompletionPrefix {
                range: at - 2..at,
                text: "al".into()
            }
        );
        assert_eq!(forward, ["almanac", "alpha", "alpine", "alchemy"]);
        let (_, backward, _) = collect(&document, at, WordCompletionDirection::Backward, 17);
        assert_eq!(backward, ["alchemy", "alpine", "alpha", "almanac"]);
    }

    #[test]
    fn mid_word_search_does_not_offer_its_own_suffix_or_consume_it() {
        let document = Document::new("alpine alchemist alpha");
        for direction in [
            WordCompletionDirection::Forward,
            WordCompletionDirection::Backward,
        ] {
            let (prefix, words, _) = collect(&document, 9, direction, 23);
            assert_eq!(prefix.range, 7..9);
            assert!(!words
                .iter()
                .any(|word| word == "alchemist" || word == "chemist"));
            assert_eq!(words.len(), 2);
        }
    }

    #[test]
    fn unicode_words_cross_rope_and_style_boundaries() {
        let document = Document::from_bytes(
            b"caf<b>&#233;</b>teria cafe&#769;ine\n\ncaf".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let at = document.projection().text_tree().byte_len();
        let (_, words, _) = collect(&document, at, WordCompletionDirection::Backward, 9);
        assert_eq!(words, ["cafe\u{301}ine", "caféteria"]);
        assert!(!words.iter().any(|word| word.contains("&#")));

        let document = Document::new(format!("{} café_東京 cafe\u{301}ine ca", ".".repeat(4092)));
        let at = document.projection().text_tree().byte_len();
        let (_, words, _) = collect(&document, at, WordCompletionDirection::Backward, 7);
        assert_eq!(words, ["cafe\u{301}ine", "café_東京"]);
    }

    #[test]
    fn prefix_and_candidate_graphemes_cross_chunk_and_rope_boundaries() {
        let word = format!("a{}bc", "\u{301}".repeat(600));
        let prefix = format!("a{}", "\u{301}".repeat(600));
        let document = Document::new(format!("{}{} {}", " ".repeat(4000), word, prefix));
        let at = document.projection().text_tree().byte_len();
        for direction in [
            WordCompletionDirection::Forward,
            WordCompletionDirection::Backward,
        ] {
            let (found, words, _) = collect(&document, at, direction, 13);
            assert_eq!(found.text, prefix);
            assert_eq!(words, [word.clone()]);
        }
    }

    #[test]
    fn candidate_count_and_aggregate_memory_are_bounded() {
        let document = Document::new(format!(
            "a {}",
            (0..MAX_CANDIDATES + 10)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let (_, words, truncated) = collect(&document, 1, WordCompletionDirection::Forward, 8192);
        assert_eq!(words.len(), MAX_CANDIDATES);
        assert!(truncated);
        let document = Document::new(format!(
            "a {}",
            (0..100)
                .map(|index| format!("a{index}_{}", "a".repeat(MAX_COMPLETION_WORD_BYTES - 8)))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let (_, words, truncated) = collect(&document, 1, WordCompletionDirection::Forward, 8192);
        assert!(words.iter().map(String::len).sum::<usize>() <= MAX_CANDIDATE_BYTES);
        assert!(words.len() < 100);
        assert!(truncated);
    }

    #[test]
    fn empty_prefix_and_document_are_supported() {
        let document = Document::new("one two ");
        let (prefix, words, _) = collect(&document, 8, WordCompletionDirection::Backward, 32);
        assert_eq!(prefix.text, "");
        assert_eq!(words, ["two", "one"]);
        let document = Document::new("");
        let (_, words, _) = collect(&document, 0, WordCompletionDirection::Forward, 8);
        assert!(words.is_empty());
    }

    #[test]
    fn snapshot_identity_is_checked_and_a_search_keeps_its_original_snapshot() {
        let mut document = Document::new("alphabet al");
        let point = document.text_point(11).unwrap();
        let snapshot = document.hard_line_snapshot();
        document.insert(0, "other ").unwrap();
        assert!(matches!(
            WordCompletionSearch::begin(
                document.hard_line_snapshot(),
                point,
                WordCompletionDirection::Forward
            ),
            Err(PositionError::WrongSnapshot { .. })
        ));
        let other = Document::new("alphabet al");
        assert!(matches!(
            WordCompletionSearch::begin(
                other.hard_line_snapshot(),
                point,
                WordCompletionDirection::Forward
            ),
            Err(PositionError::WrongDocument { .. })
        ));
        let mut search =
            WordCompletionSearch::begin(snapshot, point, WordCompletionDirection::Backward)
                .unwrap();
        let mut words = Vec::new();
        while !search.is_complete() {
            words.extend(search.advance(31).candidates);
        }
        assert_eq!(words, ["alphabet"]);
    }

    #[test]
    fn large_document_and_giant_line_queries_yield_without_flattening() {
        let document = Document::new(format!("al {} alphabet", "x ".repeat(500_000)));
        assert!(!document.projection().compatibility_text_is_materialized());
        let mut search = WordCompletionSearch::begin(
            document.hard_line_snapshot(),
            document.text_point(2).unwrap(),
            WordCompletionDirection::Forward,
        )
        .unwrap();
        let batch = search.advance(128);
        assert!(!batch.complete);
        assert!(batch.bytes_scanned <= 128);
        for _ in 0..4 {
            if search.prefix().is_some() {
                break;
            }
            let batch = search.advance(128);
            assert!(!batch.complete);
            assert!(batch.bytes_scanned <= 128);
        }
        assert_eq!(search.prefix().unwrap().text, "al");
        assert!(!document.projection().compatibility_text_is_materialized());
        drop(search); // Cancellation releases the exact query snapshot.

        let document = Document::new(format!("al {} alphabet", "\u{301}".repeat(100_000)));
        let mut search = WordCompletionSearch::begin(
            document.hard_line_snapshot(),
            document.text_point(2).unwrap(),
            WordCompletionDirection::Forward,
        )
        .unwrap();
        for _ in 0..10 {
            let batch = search.advance(128);
            assert!(!batch.complete);
            assert!(batch.bytes_scanned <= 128);
        }
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn oversized_prefix_and_candidates_remain_bounded() {
        let document = Document::new("a".repeat(MAX_COMPLETION_WORD_BYTES + 1));
        let at = document.projection().text_tree().byte_len();
        let mut search = WordCompletionSearch::begin(
            document.hard_line_snapshot(),
            document.text_point(at).unwrap(),
            WordCompletionDirection::Backward,
        )
        .unwrap();
        assert!(search.advance(64).prefix.is_none());
        while !search.is_complete() {
            let batch = search.advance(64);
            assert!(batch.bytes_scanned <= 64);
            if batch.complete {
                assert!(batch.truncated);
            }
        }
        assert!(search.prefix().is_none());
        let document = Document::new(format!(
            "al {} alpha",
            "a".repeat(MAX_COMPLETION_WORD_BYTES + 1)
        ));
        let (_, words, truncated) = collect(&document, 2, WordCompletionDirection::Forward, 128);
        assert_eq!(words, ["alpha"]);
        assert!(truncated);
    }
}
