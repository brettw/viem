use evim_core::command::{CommandStatus, InputEvent, Key};
use evim_core::document::{BoundaryAffinity, Document, Encoding, Format, SourceArtifactDigest};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, ViewId};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let description = format!(
        "{event:?} at {} in {:?}",
        core.command_state(view).unwrap().cursor(),
        String::from_utf8_lossy(&core.document().source_bytes())
    );
    let update = core
        .handle(view, CoreEvent::Input(event))
        .unwrap_or_else(|error| panic!("{description}: {error:?}"));
    if let Some(command) = update.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{description}: {:?}",
            command.status
        );
    }
}

fn fixture(source: &str, at: usize) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(html(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
    input(&mut core, view, InputEvent::key('i'));
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: if at == core.document().text().len()
                || core.document().text()[at..].starts_with('\n')
            {
                BoundaryAffinity::Upstream
            } else {
                BoundaryAffinity::Downstream
            },
            extend_selection: false,
        },
    )
    .unwrap();
    (core, view)
}

fn assert_reopens(document: &Document, expected: &str) {
    assert_eq!(document.text(), expected);
    let source = String::from_utf8(document.source_bytes()).unwrap();
    let reopened = html(&source);
    assert_eq!(reopened.text(), expected, "{source}");
    assert_eq!(
        document.projection().hard_line_count(),
        reopened.projection().hard_line_count(),
        "{source}"
    );
    // A local edit may split adjacent style spans without changing their
    // meaning. Compare the applications at each character, not tree packing.
    for (at, _) in expected.char_indices() {
        let active = |document: &Document| {
            document
                .projection()
                .style_spans()
                .iter()
                .filter(|span| span.range.contains(&at))
                .map(|span| span.application.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(active(document), active(&reopened), "{source} at {at}");
    }
}

fn type_text(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    edit_input(core, view, InputEvent::text(text));
}

fn edit_input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    input(core, view, event);
    let document = core.document();
    let after = document.source_bytes();
    let details = document
        .history_node_details(document.history_status().current.node)
        .unwrap();
    let transaction = details.transactions.last().unwrap();
    assert_eq!(transaction.before_revision(), revision);
    assert_eq!(transaction.after_revision(), document.revision());
    let mut old_end = 0;
    let mut new_end = 0;
    for patch in transaction.source_patches() {
        let range = patch.range();
        let unchanged = range.start - old_end;
        assert_eq!(
            &before[old_end..range.start],
            &after[new_end..new_end + unchanged],
            "source bytes outside the declared patch changed"
        );
        new_end += unchanged;
        assert_eq!(
            patch.replacement_digest(),
            SourceArtifactDigest::from_bytes(&after[new_end..new_end + patch.replacement_len()])
        );
        new_end += patch.replacement_len();
        old_end = range.end;
    }
    assert_eq!(&before[old_end..], &after[new_end..]);
    assert!(
        transaction
            .source_patches()
            .iter()
            .map(|patch| patch.range().len())
            .sum::<usize>()
            < 128,
        "typing changed more than its local source neighborhood: {transaction:?}"
    );
}

fn assert_undo_redo(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, original: &str) {
    input(core, view, InputEvent::Key(Key::Escape));
    let source = core.document().source_bytes();
    let text = core.document().text().to_owned();
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), source);
    assert_reopens(core.document(), &text);
}

fn typed_space_case(body: &str, at: usize, expected: &str, nbsp_count: usize) {
    let original = format!("<!--before-->{body}<!--after-->");
    let (mut core, view) = fixture(&original, at);
    type_text(&mut core, view, " ");
    assert_reopens(core.document(), expected);
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        at + if nbsp_count == 0 { 1 } else { 2 },
        "{body}"
    );
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert_eq!(source.matches("&nbsp;").count(), nbsp_count, "{source}");
    assert!(!source.contains("white-space: pre-wrap"), "{source}");
    assert!(source.starts_with("<!--before-->"), "{source}");
    assert!(source.ends_with("<!--after-->"), "{source}");
    assert_undo_redo(&mut core, view, &original);
}

macro_rules! space_case {
    ($name:ident, $source:expr, $at:expr, $expected:expr, $nbsp:expr) => {
        #[test]
        fn $name() {
            typed_space_case($source, $at, $expected, $nbsp);
        }
    };
}

space_case!(space_in_empty_block, "<p></p>", 0, "\u{a0}", 1);
space_case!(space_at_block_start, "<p>AB</p>", 0, "\u{a0}AB", 1);
space_case!(space_inside_block, "<p>AB</p>", 1, "A B", 0);
space_case!(space_at_block_end, "<p>AB</p>", 2, "AB\u{a0}", 1);
space_case!(
    space_inside_leading_span,
    "<p><span>AB</span></p>",
    0,
    "\u{a0}AB",
    1
);
space_case!(
    space_inside_trailing_span,
    "<p><span>AB</span></p>",
    2,
    "AB\u{a0}",
    1
);
space_case!(
    space_at_inline_start,
    "<p>A<span>B</span>C</p>",
    1,
    "A BC",
    0
);
space_case!(space_at_inline_end, "<p>A<span>B</span>C</p>", 2, "AB C", 0);
space_case!(
    space_before_existing_inline_space,
    "<p>A <span>B</span> C</p>",
    1,
    "A\u{a0} B C",
    1
);
space_case!(
    space_after_existing_inline_space,
    "<p>A <span>B</span> C</p>",
    2,
    "A \u{a0}B C",
    1
);
space_case!(
    space_before_run_across_empty_span,
    "<p>A <span></span> B</p>",
    1,
    "A\u{a0} B",
    1
);
space_case!(
    space_after_run_across_empty_span,
    "<p>A <span></span> B</p>",
    2,
    "A \u{a0}B",
    1
);
space_case!(
    space_before_run_across_whitespace_span,
    "<p>A\t<span>\n</span> B</p>",
    1,
    "A\u{a0} B",
    1
);
space_case!(
    space_after_run_across_whitespace_span,
    "<p>A\t<span>\n</span> B</p>",
    2,
    "A \u{a0}B",
    1
);
space_case!(
    space_before_run_across_comment,
    "<p>A <!--keep--> B</p>",
    1,
    "A\u{a0} B",
    1
);
space_case!(
    space_after_run_across_comment,
    "<p>A <!--keep--> B</p>",
    2,
    "A \u{a0}B",
    1
);
space_case!(
    space_after_leading_source_trivia,
    "<p> \t<span>AB</span>\n </p>",
    0,
    "\u{a0}AB",
    1
);
space_case!(
    space_before_trailing_source_trivia,
    "<p> \t<span>AB</span>\n </p>",
    2,
    "AB\u{a0}",
    1
);
space_case!(space_before_hard_break, "<p>A<br>B</p>", 1, "A\u{a0}\nB", 1);
space_case!(space_after_hard_break, "<p>A<br>B</p>", 2, "A\n\u{a0}B", 1);
space_case!(
    space_before_leading_hard_break,
    "<p><br>A</p>",
    0,
    "\u{a0}\nA",
    1
);
space_case!(
    space_at_list_item_start,
    "<ul><li>AB</li></ul>",
    0,
    "\u{a0}AB",
    1
);
space_case!(
    space_in_nowrap,
    "<p style='white-space:nowrap'>AB</p>",
    0,
    "\u{a0}AB",
    1
);
space_case!(
    space_after_pre_line_newline,
    "<p style='white-space:pre-line'>A\nB</p>",
    2,
    "A\n\u{a0}B",
    1
);
space_case!(
    space_in_code_outside_pre,
    "<p><code>AB</code></p>",
    0,
    "\u{a0}AB",
    1
);
space_case!(
    space_in_normal_override_inside_pre,
    "<pre><span style='white-space:normal'>AB</span></pre>",
    1,
    "A B",
    0
);

#[test]
fn repeated_typed_spaces_preserve_each_space_with_the_minimum_nbsp_count() {
    for (initial, at, count, minimum_nbsp) in [
        ("<p>AB</p>", 1, 2, 1),
        ("<p>AB</p>", 1, 3, 1),
        ("<p>AB</p>", 1, 4, 2),
        ("<p>AB</p>", 0, 2, 1),
        ("<p>AB</p>", 0, 3, 2),
        ("<p>AB</p>", 2, 2, 1),
        ("<p>AB</p>", 2, 3, 2),
        ("<p></p>", 0, 1, 1),
        ("<p></p>", 0, 2, 2),
        ("<p></p>", 0, 3, 2),
        ("<p></p>", 0, 4, 3),
    ] {
        for scalar_input in [false, true] {
            let (mut core, view) = fixture(initial, at);
            let before = core.document().text().to_owned();
            if scalar_input {
                for _ in 0..count {
                    type_text(&mut core, view, " ");
                    assert_reopens(core.document(), core.document().text());
                }
            } else {
                type_text(&mut core, view, &" ".repeat(count));
            }
            let actual = core.document().text();
            let mut visible = before;
            visible.insert_str(at, &" ".repeat(count));
            assert_eq!(
                actual.replace('\u{a0}', " "),
                visible,
                "{initial} count={count} scalar_input={scalar_input}"
            );
            assert_eq!(
                actual.chars().filter(|&ch| ch == '\u{a0}').count(),
                minimum_nbsp,
                "{initial} count={count} scalar_input={scalar_input}"
            );
            assert_reopens(core.document(), actual);
            let source = String::from_utf8(core.document().source_bytes()).unwrap();
            assert_eq!(source.matches("&nbsp;").count(), minimum_nbsp, "{source}");
            assert!(!source.contains("<span"), "{source}");
            assert_undo_redo(&mut core, view, initial);
        }
    }
}

#[test]
fn scalar_typing_compacts_only_the_generated_space_that_no_longer_needs_protection() {
    let original = "<!--keep--><p></p><!--tail-->";
    let (mut core, view) = fixture(original, 0);
    for (typed, text, source_body) in [
        ("A", "A", "A"),
        (" ", "A\u{a0}", "A&nbsp;"),
        ("B", "A B", "A B"),
        (" ", "A B\u{a0}", "A B&nbsp;"),
        ("C", "A B C", "A B C"),
    ] {
        type_text(&mut core, view, typed);
        assert_reopens(core.document(), text);
        assert_eq!(core.command_state(view).unwrap().cursor(), text.len());
        assert_eq!(
            core.document().source_bytes(),
            format!("<!--keep--><p>{source_body}</p><!--tail-->").as_bytes()
        );
    }
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn authored_and_explicit_nbsp_are_not_changed_into_breakable_spaces() {
    for spelling in ["&nbsp;", "&#160;", "&#xA0;", "\u{a0}"] {
        let original = format!("<p>A{spelling}</p><!--keep-->");
        let (mut core, view) = fixture(&original, "A\u{a0}".len());
        type_text(&mut core, view, "B");
        assert_reopens(core.document(), "A\u{a0}B");
        assert_eq!(
            core.document().source_bytes(),
            format!("<p>A{spelling}B</p><!--keep-->").as_bytes()
        );
        assert_undo_redo(&mut core, view, &original);
    }

    let original = "<p>A</p><!--keep-->";
    let (mut core, view) = fixture(original, 1);
    type_text(&mut core, view, "\u{a0}");
    let protected = core.document().source_bytes();
    type_text(&mut core, view, "B");
    assert_reopens(core.document(), "A\u{a0}B");
    let source = String::from_utf8(protected).unwrap();
    assert_eq!(
        core.document().source_bytes(),
        source.replace("</p>", "B</p>").as_bytes()
    );
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn non_ascii_spacing_does_not_trigger_ascii_whitespace_collapse() {
    for whitespace in [
        '\u{a0}', '\u{2002}', '\u{2003}', '\u{2009}', '\u{202f}', '\u{3000}',
    ] {
        let original = format!("<p>A{whitespace}B</p>");
        for at in [1, 1 + whitespace.len_utf8()] {
            let (mut core, view) = fixture(&original, at);
            type_text(&mut core, view, " ");
            let mut expected = format!("A{whitespace}B");
            expected.insert(at, ' ');
            assert_reopens(core.document(), &expected);
            let mut expected_source = original.clone();
            expected_source.insert("<p>".len() + at, ' ');
            assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
            assert_undo_redo(&mut core, view, &original);
        }
    }
}

#[test]
fn typed_tabs_use_html_space_semantics_in_collapsing_contexts() {
    for (original, at, typed, expected) in [
        ("<p>AB</p>", 1, "\t", "A B"),
        ("<p>AB</p>", 0, "\t", "\u{a0}AB"),
        ("<p>AB</p>", 2, "\t", "AB\u{a0}"),
        ("<p>AB</p>", 1, "\r", "A B"),
        ("<p>AB</p>", 0, "\r", "\u{a0}AB"),
    ] {
        let (mut core, view) = fixture(original, at);
        type_text(&mut core, view, typed);
        assert_reopens(core.document(), expected);
        assert_undo_redo(&mut core, view, original);
    }
}

#[test]
fn whitespace_preserving_elements_keep_literal_spaces_and_tabs() {
    for (opening, closing) in [
        ("<pre>", "</pre>"),
        ("<pre><code>", "</code></pre>"),
        ("<p style='white-space:pre'>", "</p>"),
        ("<p style='white-space:pre-wrap'>", "</p>"),
        ("<p style='white-space:break-spaces'>", "</p>"),
        ("<p><span style='white-space:pre-wrap'>", "</span></p>"),
    ] {
        for at in [0, 1, 2] {
            let original = format!("{opening}AB{closing}<!--keep-->");
            let (mut core, view) = fixture(&original, at);
            type_text(&mut core, view, " \t  ");
            let mut expected = "AB".to_owned();
            expected.insert_str(at, " \t  ");
            assert_reopens(core.document(), &expected);
            let mut source = original.clone();
            source.insert_str(opening.len() + at, " \t  ");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_undo_redo(&mut core, view, &original);
        }
    }
}

#[test]
fn typed_newlines_create_hard_breaks_and_spaces_beside_them_are_protected() {
    let original = "<p></p><!--keep-->";
    let (mut core, view) = fixture(original, 0);
    type_text(&mut core, view, "A \n B");
    assert_reopens(core.document(), "A\u{a0}\n\u{a0}B");
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert_eq!(source.matches("&nbsp;").count(), 2, "{source}");
    assert!(!source.contains("white-space"), "{source}");
    assert!(source.ends_with("<!--keep-->"), "{source}");
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn tab_and_return_keys_share_the_html_typing_policy() {
    let original = "<p>A</p><!--keep-->";
    let (mut core, view) = fixture(original, 1);
    for (key, expected) in [
        (Key::Tab, "A\u{a0}"),
        (Key::Enter, "A\u{a0}\n"),
        (Key::Tab, "A\u{a0}\n\u{a0}"),
    ] {
        edit_input(&mut core, view, InputEvent::Key(key));
        assert_reopens(core.document(), expected);
        assert_eq!(core.command_state(view).unwrap().cursor(), expected.len());
    }
    type_text(&mut core, view, "B");
    assert_reopens(core.document(), "A\u{a0}\n\u{a0}B");
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert_eq!(source.matches("&nbsp;").count(), 2, "{source}");
    assert!(!source.contains("white-space"), "{source}");
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn typing_in_an_empty_styled_span_removes_only_newly_exposed_source_whitespace() {
    for trivia in [" ".to_owned(), " \t\r\n".repeat(2_500)] {
        let original = format!("<p>A <b data-keep='x'></b>{trivia}B</p><!--tail-->");
        let (mut core, view) = fixture(&original, 2);
        // This supporting deletion may be as large as the collapsed run it
        // necessarily removes, so use the normal input helper, not its small
        // patch-size assertion for the short source fixtures.
        input(&mut core, view, InputEvent::text("X"));
        assert_eq!(
            core.document().source_bytes(),
            b"<p>A <b data-keep='x'>X</b>B</p><!--tail-->"
        );
        assert_reopens(core.document(), "A XB");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        assert_undo_redo(&mut core, view, &original);
    }
}

#[test]
fn protective_space_spelling_and_compaction_preserve_the_original_encoding() {
    fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
        match encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            Encoding::Latin1 => text
                .chars()
                .map(|ch| u8::try_from(ch as u32).unwrap())
                .collect(),
        }
    }
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        let original = encoded("<p></p><!--keep-->", encoding);
        let mut core =
            Core::new(Document::from_bytes(original.clone(), encoding, Format::Html).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
        input(&mut core, view, InputEvent::key('i'));
        for (typed, expected_text, expected_source) in [
            ("A", "A", "<p>A</p><!--keep-->"),
            (" ", "A\u{a0}", "<p>A&nbsp;</p><!--keep-->"),
            ("é", "A é", "<p>A é</p><!--keep-->"),
        ] {
            input(&mut core, view, InputEvent::text(typed));
            assert_eq!(
                core.document().source_bytes(),
                encoded(expected_source, encoding)
            );
            assert_eq!(core.document().text(), expected_text);
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                expected_text.len()
            );
            let reopened =
                Document::from_bytes(core.document().source_bytes(), encoding, Format::Html)
                    .unwrap();
            assert_eq!(reopened.text(), expected_text);
        }
        input(&mut core, view, InputEvent::Key(Key::Escape));
        let final_source = core.document().source_bytes();
        input(&mut core, view, InputEvent::key('u'));
        assert_eq!(core.document().source_bytes(), original);
        input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
        assert_eq!(core.document().source_bytes(), final_source);
    }
}
