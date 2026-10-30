use super::*;
use crate::document::Document;

fn line_range(document: &Document) -> (HardLineSnapshot, Range<usize>) {
    let lines = document.hard_line_snapshot();
    let range = lines.line(0).unwrap().content_range();
    (lines, range)
}

#[test]
fn quote_count_skips_escaped_and_apostrophe_quoted_quotes() {
    for (text, even) in [
        ("\"a\"", true),
        ("\"a", false),
        ("\\\"a", true),
        ("\\\\\"a", false),
        ("'\"' x", true),
        ("'\"x", false),
        ("x\"'", false),
    ] {
        let document = Document::new(text);
        let (lines, range) = line_range(&document);
        assert_eq!(count_quotes(&lines, &range, None).0, even, "{text:?}");
    }
}

#[test]
fn quote_count_reports_parity_at_the_probe_unless_a_backslash_skips_it() {
    let document = Document::new("\"a\" \"b \\(");
    let (lines, range) = line_range(&document);
    // Vim's running count starts at -1, so even quotes report true.
    assert_eq!(count_quotes(&lines, &range, Some(0)).1, Some(true));
    assert_eq!(count_quotes(&lines, &range, Some(1)).1, Some(false));
    assert_eq!(count_quotes(&lines, &range, Some(4)).1, Some(true));
    assert_eq!(count_quotes(&lines, &range, Some(8)).1, None, "the escaped byte is never examined");
    assert_eq!(count_quotes(&lines, &range, Some(range.end)).1, None);
}

#[test]
fn backslash_runs_are_counted_across_text_tree_leaves() {
    for run in [0, 1, 2, 100_001] {
        let text = format!("x{}(", "\\".repeat(run));
        let document = Document::new(&text);
        let lines = document.hard_line_snapshot();
        assert_eq!(backslashes_before(&lines, text.len() - 1, 0), run);
        assert_eq!(backslashes_before(&lines, text.len() - 1, text.len() - 1), 0);
    }
}

#[test]
fn character_literals_skip_from_either_quote_only_to_a_logical_item() {
    let document = Document::new("'(' '\\(' 'xy'");
    let (lines, range) = line_range(&document);
    assert_eq!(character_literal(&lines, 0, &range, true), Some(2));
    assert_eq!(character_literal(&lines, 2, &range, false), Some(0));
    assert_eq!(character_literal(&lines, 4, &range, true), Some(7));
    assert_eq!(character_literal(&lines, 7, &range, false), Some(4));
    assert_eq!(character_literal(&lines, 9, &range, true), None, "'xy' is not a literal");
    // A prepended format character joins the opening quote's grapheme.
    let document = Document::new("\u{600}'x'");
    let (lines, range) = line_range(&document);
    assert_eq!(character_literal(&lines, 4, &range, false), None);
}

#[test]
fn matching_reads_no_flat_text_and_reports_missing_partners() {
    let prefix = "unrelated \"quoted\" line\n".repeat(10_000);
    let text = format!("{prefix}call(\")\", (x))\n{prefix}");
    let document = Document::new(&text);
    let lines = document.hard_line_snapshot();
    let (open, quoted, close) = (prefix.len() + 4, prefix.len() + 6, prefix.len() + 13);
    assert_eq!(matching_delimiter(&lines, open, MatchRules::default()), Some(close));
    assert_eq!(matching_delimiter(&lines, close, MatchRules::default()), Some(open));
    assert_eq!(matching_delimiter(&lines, quoted, MatchRules::default()), None,
        "one quote separates the quoted `)` from every `(` before it");
    assert_eq!(matching_delimiter(&lines, open, MatchRules { vi_compatible: true, ..Default::default() }),
        Some(quoted), "without quote rules the quoted `)` closes the call");
    assert!(!document.projection().compatibility_text_is_materialized());
}
