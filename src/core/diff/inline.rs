//! Changed spans within a pair of corresponding lines.
//!
//! Lines are compared as Unicode word-boundary tokens, so whole words change
//! together; a changed token pair is then narrowed by its common grapheme
//! prefix and suffix, so `  foo` against `    foo` marks only the added
//! spaces. Spans are byte ranges within each line, on grapheme boundaries.

use super::sequence::{diff, Run, WorkBudget};
use std::collections::HashMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Longest line, in bytes, that gets inline spans. Longer lines still show as
/// changed lines; only their within-line detail is omitted.
pub const MAX_INLINE_LINE_BYTES: usize = 16 * 1024;
const INLINE_WORK_UNITS: u64 = 200_000;

/// Changed byte ranges within one base line and its corresponding new line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InlineSpans {
    pub base: Vec<Range<usize>>,
    pub new: Vec<Range<usize>>,
}

/// Tokens of `line` with their byte ranges.
fn tokens(line: &str) -> Vec<(usize, &str)> {
    line.split_word_bound_indices().collect()
}

/// A cheap similarity in `0..=1`: the Dice coefficient of the two lines'
/// character bigrams, ignoring whitespace. Used to decide which lines of a
/// changed region correspond. Lines differing only in whitespace score 1.
pub fn similarity(base: &str, new: &str) -> f32 {
    if base.len() > MAX_INLINE_LINE_BYTES || new.len() > MAX_INLINE_LINE_BYTES {
        return 0.0;
    }
    fn bigrams(line: &str) -> Vec<(char, char)> {
        let characters: Vec<char> = line.chars().filter(|character| !character.is_whitespace()).collect();
        match characters.len() {
            0 => Vec::new(),
            // A single character still needs one comparable unit.
            1 => vec![(characters[0], '\0')],
            _ => characters.windows(2).map(|pair| (pair[0], pair[1])).collect(),
        }
    }
    let base_bigrams = bigrams(base);
    let new_bigrams = bigrams(new);
    if base_bigrams.is_empty() && new_bigrams.is_empty() {
        return 1.0;
    }
    let mut counts: HashMap<(char, char), usize> = HashMap::new();
    for bigram in &base_bigrams {
        *counts.entry(*bigram).or_default() += 1;
    }
    let mut common = 0usize;
    for bigram in &new_bigrams {
        if let Some(count) = counts.get_mut(bigram).filter(|count| **count > 0) {
            *count -= 1;
            common += 1;
        }
    }
    (2 * common) as f32 / (base_bigrams.len() + new_bigrams.len()) as f32
}

/// The changed spans between two corresponding lines.
pub fn inline_spans(base: &str, new: &str) -> InlineSpans {
    let mut spans = InlineSpans::default();
    if base == new || base.len() > MAX_INLINE_LINE_BYTES || new.len() > MAX_INLINE_LINE_BYTES {
        return spans;
    }
    let base_tokens = tokens(base);
    let new_tokens = tokens(new);
    let mut table: HashMap<&str, u32> = HashMap::new();
    let mut base_ids = Vec::with_capacity(base_tokens.len());
    for (_, token) in &base_tokens {
        let next = table.len() as u32;
        base_ids.push(*table.entry(token).or_insert(next));
    }
    let mut new_ids = Vec::with_capacity(new_tokens.len());
    for (_, token) in &new_tokens {
        let next = table.len() as u32;
        new_ids.push(*table.entry(token).or_insert(next));
    }
    let runs = diff(&base_ids, &new_ids, &mut WorkBudget::new(INLINE_WORK_UNITS));
    let token_end = |list: &[(usize, &str)], index: usize, line: &str| {
        list.get(index).map_or(line.len(), |(offset, _)| *offset)
    };
    let (mut i, mut j) = (0usize, 0usize);
    for run in runs {
        match run {
            Run::Equal(length) => {
                i += length;
                j += length;
            }
            Run::Change { base: removed, new: added } => {
                let base_range = token_end(&base_tokens, i, base)..token_end(&base_tokens, i + removed, base);
                let new_range = token_end(&new_tokens, j, new)..token_end(&new_tokens, j + added, new);
                let (base_range, new_range) = narrow(base, base_range, new, new_range);
                if !base_range.is_empty() {
                    push_span(&mut spans.base, base_range);
                }
                if !new_range.is_empty() {
                    push_span(&mut spans.new, new_range);
                }
                i += removed;
                j += added;
            }
        }
    }
    spans
}

/// Trim the grapheme prefix and suffix two replaced ranges have in common.
fn narrow(base: &str, base_range: Range<usize>, new: &str, new_range: Range<usize>) -> (Range<usize>, Range<usize>) {
    let base_text = &base[base_range.clone()];
    let new_text = &new[new_range.clone()];
    let mut prefix = 0;
    for ((offset, a), (_, b)) in base_text.grapheme_indices(true).zip(new_text.grapheme_indices(true)) {
        if a != b {
            break;
        }
        prefix = offset + a.len();
    }
    let base_rest = &base_text[prefix..];
    let new_rest = &new_text[prefix..];
    let mut suffix = 0;
    for (a, b) in base_rest.graphemes(true).rev().zip(new_rest.graphemes(true).rev()) {
        if a != b {
            break;
        }
        suffix += a.len();
    }
    (
        base_range.start + prefix..base_range.end - suffix,
        new_range.start + prefix..new_range.end - suffix,
    )
}

fn push_span(spans: &mut Vec<Range<usize>>, range: Range<usize>) {
    match spans.last_mut() {
        Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
        _ => spans.push(range),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_marks_only_the_added_whitespace() {
        let spans = inline_spans("foo();", "  foo();");
        assert_eq!(spans.new, vec![0..2]);
        assert!(spans.base.is_empty());
        let deeper = inline_spans("  foo();", "    foo();");
        assert_eq!(deeper.new, vec![2..4]);
        assert!(deeper.base.is_empty());
    }

    #[test]
    fn a_replaced_word_is_marked_on_both_sides() {
        let spans = inline_spans("the quick fox", "the slow fox");
        assert_eq!(spans.base, vec![4..9]);
        assert_eq!(spans.new, vec![4..8]);
    }

    #[test]
    fn spans_stay_on_grapheme_boundaries() {
        let spans = inline_spans("cafe\u{301} ok", "cafe ok");
        // The combining accent belongs to the final grapheme of "café", so
        // the whole "é" changes rather than only its accent.
        assert_eq!(spans.base, vec![3..6]);
        assert_eq!(spans.new, vec![3..4]);
        for range in spans.base {
            assert!("cafe\u{301} ok".is_char_boundary(range.start));
        }
    }

    #[test]
    fn identical_and_oversized_lines_have_no_spans() {
        assert_eq!(inline_spans("same", "same"), InlineSpans::default());
        let long = "x".repeat(MAX_INLINE_LINE_BYTES + 1);
        assert_eq!(inline_spans(&long, "x"), InlineSpans::default());
    }

    #[test]
    fn similarity_prefers_lines_that_share_words() {
        assert!(similarity("let value = compute(a);", "let value = compute(b);") > 0.6);
        assert!(similarity("let value = compute(a);", "return;") < 0.3);
        assert!(similarity("beta", "betax") > 0.8);
        assert_eq!(similarity("    ", "\t"), 1.0);
        assert_eq!(similarity("a  b", "a b"), 1.0);
    }
}
