//! `%`, `[(`, `[{`, `])`, `]}` and the block text objects follow Vim 9.2's
//! `findmatchlimit()` quote and backslash rules. Expectations come from Vim
//! 9.1.1752, whose function is identical, run with `-u NONE`; positions are
//! 1-based lines and byte columns.
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

/// Run `command` from `start` and return the cursor and the resulting text.
fn run(text: &str, start: (usize, usize), cpoptions: &str, command: &str) -> ((usize, usize), String) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 2000.0, 400.0);
    let mut input = |keys: &str| {
        for character in keys.chars() {
            let key = if character == '\n' { Key::Enter } else { Key::Char(character) };
            let outcome = core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            let status = outcome.command.unwrap().status;
            assert!(!matches!(status, CommandStatus::Unsupported(_)), "{keys:?}: {status:?}");
        }
    };
    if !cpoptions.is_empty() {
        input(&format!(":set cpo+={cpoptions}\n"));
    }
    input(&format!("{}G{}|{command}", start.0, start.1));
    let cursor = core.command_state(view).unwrap().cursor();
    let text = core.document().text().to_owned();
    let line_start = text[..cursor].rfind('\n').map_or(0, |at| at + 1);
    ((text[..cursor].matches('\n').count() + 1, cursor - line_start + 1), text)
}

fn assert_cursor(text: &str, start: (usize, usize), cpoptions: &str, command: &str, expected: (usize, usize)) {
    let (cursor, after) = run(text, start, cpoptions, command);
    assert_eq!(cursor, expected, "{command} from {start:?} in {text:?} with cpo+={cpoptions}");
    assert_eq!(after, text);
}

#[test]
fn percent_skips_delimiters_in_double_quotes_unless_the_start_is_quoted() {
    assert_cursor("foo(\"(\", x)", (1, 4), "", "%", (1, 11));
    assert_cursor("x = f(\")\") + g(\"(\")", (1, 6), "", "%", (1, 10));
    assert_cursor("x = f(\")\") + g(\"(\")", (1, 10), "", "%", (1, 6));
    // Quote parity is relative to the start, so a quoted start matches inside.
    assert_cursor("\"a ( b ) c\"", (1, 4), "", "%", (1, 8));
    // An escaped quote neither opens nor closes; an even run of backslashes
    // leaves the next quote unescaped.
    assert_cursor("a \"b\\\\\"c\" ( x )", (1, 12), "", "%", (1, 11));
    // An uneven number of quotes leaves the line's quotes ignored.
    assert_cursor("say \"hi ( x )", (1, 9), "", "%", (1, 13));
    // Lines without a continuation reset the quote state.
    assert_cursor("f(\n\")\"\n)\n", (1, 2), "", "%", (3, 1));
    assert_cursor("f(\n\")\"\n)\n", (3, 1), "", "%", (1, 2));
}

#[test]
fn percent_skips_character_literals_and_honors_backslash_escapes() {
    assert_cursor("f('(', x)", (1, 2), "", "%", (1, 9));
    assert_cursor("f('(', x)", (1, 9), "", "%", (1, 2));
    assert_cursor("f('\\(', x)", (1, 2), "", "%", (1, 10));
    assert_cursor("f('\\(', x)", (1, 10), "", "%", (1, 2));
    assert_cursor("if (c == '\"') { f(\")\"); }", (1, 4), "", "%", (1, 13));
    assert_cursor("if (c == '\"') { f(\")\"); }", (1, 15), "", "%", (1, 25));
    assert_cursor("a ( \\) )", (1, 3), "", "%", (1, 8));
    // An escaped start matches only an escaped partner.
    assert_cursor("\\( ( \\)", (1, 2), "", "%", (1, 7));
}

#[test]
fn cpoptions_percent_and_m_restore_vi_matching() {
    // With `%` quotes are ordinary text, so the quoted `(` leaves `(` unmatched.
    assert_cursor("foo(\"(\", x)", (1, 4), "%", "%", (1, 4));
    // With `M` backslashes are ignored.
    assert_cursor("a ( \\) )", (1, 3), "M", "%", (1, 6));
}

#[test]
fn backslash_continued_lines_carry_quote_state() {
    let text = "s = \"a ( \\\nb ) \" )\nend";
    assert_cursor(text, (1, 8), "", "%", (2, 3));
    assert_cursor(text, (2, 3), "", "%", (1, 8));
    assert_cursor(text, (2, 7), "", "%", (2, 7));
    let text = "x ( \"a \\\n b\" )";
    assert_cursor(text, (1, 3), "", "%", (2, 5));
    assert_cursor(text, (2, 5), "", "%", (1, 3));
    let text = "p ( \"q \\\n ( \" ) )";
    assert_cursor(text, (1, 3), "", "%", (2, 6));
    assert_cursor(text, (2, 2), "", "%", (2, 6));
    assert_cursor(text, (2, 8), "", "%", (2, 8));
}

#[test]
fn unmatched_delimiter_motions_skip_quoted_delimiters() {
    assert_cursor("f(a, \")\", b)", (1, 11), "", "[(", (1, 2));
    assert_cursor("f(a, \")\", b)", (1, 3), "", "])", (1, 12));
    assert_cursor("if (c == '\"') { f(\")\"); }", (1, 22), "", "[{", (1, 15));
    assert_cursor("{ x = \"}\"; }", (1, 3), "", "]}", (1, 12));
    assert_cursor("{\n  s = \"{\";\n  t\n}", (3, 3), "", "[{", (1, 1));
    assert_cursor("f(a \\) b)", (1, 4), "", "])", (1, 9));
}

#[test]
fn block_text_objects_find_the_opening_without_quotes_and_the_close_with_them() {
    let (cursor, text) = run("(a \")\" b)", (1, 2), "", "di(");
    assert_eq!((cursor, text.as_str()), ((1, 2), "()"));
    // From `b` the quoted `)` closes the only `(` before it, so nothing encloses `b`.
    let (cursor, text) = run("(a \")\" b)", (1, 8), "", "di(");
    assert_eq!((cursor, text.as_str()), ((1, 8), "(a \")\" b)"));
    let (_, text) = run("{ x = \"}\"; }", (1, 3), "", "di{");
    assert_eq!(text, "{}");
    let (_, text) = run("f(\")\", x) + y", (1, 4), "", "da(");
    assert_eq!(text, "f + y");
    // Vim's opening search ignores quotes, so it chooses the quoted brace,
    // whose partner search then skips the unquoted `}` and fails.
    let (cursor, text) = run("{ s = \"{\"; t }", (1, 12), "", "di{");
    assert_eq!((cursor, text.as_str()), ((1, 12), "{ s = \"{\"; t }"));
}
