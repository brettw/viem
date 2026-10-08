use super::*;
use crate::document::FormattedTextTree;

fn input(text: &str, revision: u64) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 572,
            revision,
            generation: 1,
        },
        FormattedTextTree::try_from_text(text).unwrap(),
    )
}

fn session(source: &str) -> VimSession {
    VimSession::new(VimProgram::compile("oneline.vim", source, VimLoadLimits::default()).unwrap())
}

fn finish(session: &mut VimSession, input: &SyntaxInputSnapshot, range: Range<usize>) -> VimResult {
    // A single instruction exposes setup/context work which incorrectly resets
    // its progress whenever the scanner yields.
    for _ in 0..100_000 {
        let result = session.highlight(
            input,
            range.clone(),
            VimBudget {
                instructions: 1,
                allow_provisional: false,
                ..Default::default()
            },
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.stats.instructions <= 1);
        if !result.stats.yielded {
            assert_eq!(result.coverage, Coverage::Exact);
            return result;
        }
    }
    panic!("oneline scan did not make bounded progress");
}

fn names(result: &VimResult, len: usize) -> Vec<&str> {
    let mut names = vec![""; len];
    for run in &result.runs {
        names[run.range.clone()].fill(&run.name.0);
    }
    names
}

#[test]
fn oneline_compares_end_of_start_with_start_of_end() {
    // :syn-oneline permits both delimiters to contain a newline. The end of
    // the opening delimiter and the start of the closing one share a line.
    let mut session = session(r"syn region Region start=/A\nB/ end=/Z\nQ/ oneline");
    let text = "A\nBxZ\nQ!";
    let input = input(text, 1);
    let result = finish(&mut session, &input, 0..text.len());
    assert_eq!(
        names(&result, text.len()),
        ["Region", "Region", "Region", "Region", "Region", "Region", "Region", ""]
    );
}

#[test]
fn oneline_start_ending_at_line_boundary_uses_the_next_line() {
    let mut session = session(r"syn region Region start=/A\n/ end=/Z/ oneline");
    let text = "A\nxZ!";
    let input = input(text, 1);
    let result = finish(&mut session, &input, 0..text.len());
    assert_eq!(
        names(&result, text.len()),
        ["Region", "Region", "Region", "Region", ""]
    );
}

#[test]
fn oneline_skip_cannot_move_end_search_to_a_later_line() {
    let mut session = session(r"syn region Region start=/</ skip=/X\nY/ end=/>/ oneline");
    let text = "<X\nY>";
    let input = input(text, 1);
    let result = finish(&mut session, &input, 0..text.len());
    assert!(result.runs.is_empty());

    let mut session = self::session(r"syn region Region start=/</ skip=/\\>/ end=/>/ oneline");
    let text = r"<x\>y>!";
    let input = self::input(text, 1);
    let result = finish(&mut session, &input, 0..text.len());
    assert_eq!(
        names(&result, text.len()),
        ["Region", "Region", "Region", "Region", "Region", "Region", ""]
    );
}

#[test]
fn oneline_multiline_end_edit_invalidates_an_already_highlighted_prefix() {
    let mut session = session(r"syn region Region start=/A\nB/ end=/Z\nQ/ oneline");
    let before = input("A\nBxZ\nQ!", 1);
    let result = finish(&mut session, &before, 0..4);
    assert!(!result.runs.is_empty());
    let after = input("A\nBxZ\nR!", 2);
    session
        .apply_edit(before.identity(), after.identity(), 6..7, 7)
        .unwrap();
    let result = finish(&mut session, &after, 0..4);
    assert!(result.runs.is_empty());
}

#[test]
fn oneline_leading_context_does_not_reuse_a_delimiter_behind_the_start() {
    let mut session = session(r"syn region Region start=/AB/ end=/A/lc=2 oneline");
    let input = input("ABx", 1);
    let result = finish(&mut session, &input, 0..3);
    assert!(result.runs.is_empty());
}

#[test]
fn oneline_anchor_preflight_preserves_unicode_leading_context() {
    let mut session = session("syn region Region start=/é/ end=/^éx/lc=1 oneline");
    let text = "éx\n éx";
    let input = input(text, 1);
    let result = finish(&mut session, &input, 0..text.len());
    assert_eq!(
        names(&result, text.len()),
        ["Region", "Region", "Region", "", "", "", "", ""]
    );
}

#[test]
fn leading_context_future_start_preserves_intervening_matches() {
    for text in ["abc", "aéc"] {
        let mut session = session(&format!(
            "syn match Earlier /[bé]/\nsyn match Later /{text}/lc=2\n"
        ));
        let input = input(text, 1);
        let result = finish(&mut session, &input, 0..text.len());
        let names = names(&result, text.len());
        assert_eq!(names[0], "");
        assert!(names[1..text.len() - 1]
            .iter()
            .all(|name| *name == "Earlier"));
        // Vim's legacy lc rewinds bytes (rounding back out of a UTF-8
        // continuation), even though the implicit ms advances characters.
        assert_eq!(
            names[text.len() - 1],
            if text.is_ascii() { "Later" } else { "" }
        );
    }
}

#[test]
fn leading_context_byte_rewind_aligns_to_a_unicode_boundary() {
    let mut session = session("syn match Later /éw/lc=1");
    let input = input("éw", 1);
    let result = finish(&mut session, &input, 0..input.byte_len());
    assert_eq!(names(&result, input.byte_len()), ["", "", "Later"]);
}
