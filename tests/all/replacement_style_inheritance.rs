use std::ops::Range;
use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn fixture(source: &str) -> (Editor, ViewId) {
    fixture_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
}
fn fixture_format(source: Vec<u8>, encoding: Encoding, format: Format) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes(source, encoding, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
    (core, view)
}

#[test]
fn replacing_all_inline_styled_text_retains_its_style() {
    for source in [
        "**bold**",
        "[**bold**](https://example.test/)",
    ] {
        let (mut core, view) = fixture(source);
        key(&mut core, view, Key::SelectAll);
        text(&mut core, view, "X");
        text(&mut core, view, "Y");
        assert_eq!(core.document().text(), "XY");
        assert!(
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
                .unwrap()
                .bold
        );
        assert_eq!(
            core.document()
                .link_at(core.document().text_point(1).unwrap())
                .unwrap()
                .as_deref(),
            source.contains("](").then_some("https://example.test/")
        );
    }
}

#[test]
fn vim_change_and_dot_resolve_the_style_at_each_new_replacement() {
    let (mut core, view) = fixture("**bold** regular");
    text(&mut core, view, "cw");
    text(&mut core, view, "X");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('w'));
    key(&mut core, view, Key::Char('.'));
    assert_eq!(core.document().text(), "X X");
    assert!(
        DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
            .unwrap()
            .bold
    );
    assert!(
        !DocumentLayoutStyles::semantic_character_at(core.document().projection(), 2, false)
            .unwrap()
            .bold
    );
}

#[test]
fn inherited_context_survives_delete_until_typing_but_not_cursor_motion() {
    for motion in [false, true] {
        let (mut core, view) = fixture("**bold** regular");
        select(&mut core, view, 0..5, false);
        key(&mut core, view, Key::Backspace);
        if motion {
            key(&mut core, view, Key::Right);
        }
        text(&mut core, view, "X");
        let at = if motion { 1 } else { 0 };
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false)
                .unwrap()
                .bold,
            !motion
        );
    }
}

#[test]
fn utf16_replacement_keeps_non_ascii_link_attributes_and_undo_bytes() {
    let source = "pré [**bold**](https://example.test/café) tail";
    let bytes = source
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let (mut core, view) = fixture_format(bytes.clone(), Encoding::Utf16Le, Format::Markdown);
    select(&mut core, view, 5..12, true);
    text(&mut core, view, "猫");
    text(&mut core, view, "é");
    assert_eq!(
        core.document()
            .link_at(core.document().text_point(5).unwrap())
            .unwrap()
            .as_deref(),
        Some("https://example.test/café")
    );
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), bytes);
}

#[test]
fn utf16_markdown_link_replacement_keeps_formatted_caret_and_destination() {
    let source = "pré [link](https://example.test/café) tail";
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = source
            .encode_utf16()
            .flat_map(|unit| match encoding {
                Encoding::Utf16Le => unit.to_le_bytes(),
                _ => unit.to_be_bytes(),
            })
            .collect::<Vec<_>>();
        let (mut core, view) = fixture_format(bytes.clone(), encoding, Format::Markdown);
        select(&mut core, view, 5..12, false);
        text(&mut core, view, "猫");
        text(&mut core, view, "é");
        assert_eq!(core.document().text(), "pré 猫éil");
        assert_eq!(core.command_state(view).unwrap().cursor(), 10);
        for at in [5, 8] {
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(at).unwrap())
                    .unwrap()
                    .as_deref(),
                Some("https://example.test/café")
            );
        }
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), bytes);
    }
}

#[test]
fn markdown_replace_use_first_character_bold() {
    for (format, source) in [

        (Format::Markdown, "plain **bold** tail"),
    ] {
        for (range, bold) in [
            (2..12, false),
            (6..12, true),
            (6..10, true),
            (7..9, true),
            (2..10, false),
        ] {
            let (mut core, view) =
                fixture_format(source.as_bytes().to_vec(), Encoding::Utf8, format);
            select(&mut core, view, range.clone(), true);
            text(&mut core, view, "X");
            text(&mut core, view, "Y");
            for at in range.start..range.start + 2 {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        core.document().projection(),
                        at,
                        false
                    )
                    .unwrap()
                    .bold,
                    bold,
                    "{format:?} range={range:?}: {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
            }
        }
    }
}

#[test]
fn every_small_styled_selection_replaces_and_undoes_without_losing_caret_boundaries() {
    for source in [
        "A\n\nB",
        "\n\nA\\\nB",
        "A**B**C",
        "A**B** C",
    ] {
        let (core, _) = fixture(source);
        let original = core.document().text().to_owned();
        let boundaries = original
            .char_indices()
            .map(|(at, _)| at)
            .chain([original.len()])
            .collect::<Vec<_>>();
        for (index, &start) in boundaries.iter().enumerate() {
            for &end in &boundaries[index + 1..] {
                let (mut core, view) = fixture(source);
                key(&mut core, view, Key::Char('i'));
                select(&mut core, view, start..end, true);
                text(&mut core, view, "X");
                text(&mut core, view, "Y");
                assert_eq!(
                    core.document().text().replace('\u{a0}', " "),
                    format!("{}XY{}", &original[..start], &original[end..]),
                    "{source} {start}..{end}"
                );
                key(&mut core, view, Key::Escape);
                key(&mut core, view, Key::Char('u'));
                assert_eq!(
                    core.document().source_bytes(),
                    source.as_bytes(),
                    "{source} {start}..{end}"
                );
            }
        }
    }
}

#[test]
fn full_document_replacement_keeps_first_character_effective_traits() {
    let (mut core, view) = fixture("# heading");
    let expected =
        DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
            .unwrap();
    key(&mut core, view, Key::SelectAll);
    text(&mut core, view, "X");
    text(&mut core, view, "Y");
    let actual =
        DocumentLayoutStyles::semantic_character_at(core.document().projection(), 1, false)
            .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn ime_replacement_and_cancel_follow_the_same_first_character_rule() {
    let source = "plain [**link**](https://example.test/) tail";
    for (range, bold) in [(2..12, false), (6..12, true)] {
        for delete_first in [false, true] {
            let (mut core, view) = fixture(source);
            select(&mut core, view, range.clone(), false);
            if delete_first {
                key(&mut core, view, Key::Backspace);
            }
            let target_range = if delete_first {
                range.start..range.start
            } else {
                range.clone()
            };
            for cancel in [true, false] {
                let target =
                    CompositionTarget::at_offsets(core.document(), target_range.clone()).unwrap();
                core.handle(
                    view,
                    CoreEvent::Composition(CompositionEvent::Begin(target)),
                )
                .unwrap();
                core.handle(
                    view,
                    CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                        "猫",
                        3..3,
                    ))),
                )
                .unwrap();
                core.handle(
                    view,
                    CoreEvent::Composition(if cancel {
                        CompositionEvent::Cancel
                    } else {
                        CompositionEvent::Commit
                    }),
                )
                .unwrap();
            }
            text(&mut core, view, "Y");
            for at in [range.start, range.start + "猫".len()] {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        core.document().projection(),
                        at,
                        false
                    )
                    .unwrap()
                    .bold,
                    bold,
                    "range={range:?} delete_first={delete_first} at={at} source={:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
                assert_eq!(
                    core.document()
                        .link_at(core.document().text_point(at).unwrap())
                        .unwrap()
                        .as_deref(),
                    bold.then_some("https://example.test/")
                );
            }
        }
    }
}

#[test]
fn markdown_replacement_retains_code_and_link_identity() {
    for (source, code, link) in [
        ("plain `code` tail", true, false),
        ("plain [link](https://example.test/) tail", false, true),
    ] {
        let (mut core, view) =
            fixture_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown);
        select(&mut core, view, 6..12, false);
        text(&mut core, view, "X");
        text(&mut core, view, "Y");
        for at in 6..8 {
            assert_eq!(
                core.document()
                    .projection()
                    .style_spans()
                    .iter()
                    .any(|span| span.range.contains(&at)
                        && span.application
                            == viem_core::document::StyleApplication::Named("Code".into())),
                code
            );
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(at).unwrap())
                    .unwrap()
                    .as_deref(),
                link.then_some("https://example.test/")
            );
        }
    }
}

#[test]
fn markdown_link_replacement_uses_the_normalized_first_character() {
    let source = "plain [link](https://example.test/a?x=1&amp;y=2) tail";
    for (range, linked) in [
        (2..12, false),
        (6..12, true),
        (6..10, true),
        (7..9, true),
        (2..10, false),
    ] {
        let (mut core, view) =
            fixture_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown);
        select(&mut core, view, range.clone(), false);
        text(&mut core, view, "X");
        text(&mut core, view, "Y");
        for at in range.start..range.start + 2 {
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(at).unwrap())
                    .unwrap()
                    .as_deref(),
                linked.then_some("https://example.test/a?x=1&y=2"),
                "{range:?}: {:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            );
        }
    }
}

fn select(core: &mut Editor, view: ViewId, range: Range<usize>, reverse: bool) {
    let (anchor, active) = if reverse {
        (range.end, range.start)
    } else {
        (range.start, range.end)
    };
    for (text_offset, extend_selection) in [(anchor, false), (active, true)] {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection,
            },
        )
        .unwrap();
    }
    assert_eq!(core.list_selection_identity(view).unwrap().range(), range);
}
fn text(core: &mut Editor, view: ViewId, text: &str) {
    core.handle(view, CoreEvent::Input(InputEvent::text(text)))
        .unwrap_or_else(|error| {
            panic!(
                "{error:?}: {:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
}
fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

#[test]
fn native_replacement_inherits_first_character_in_both_selection_directions() {
    let source = "baseparagraph **bold** baseparagraph";
    for (range, bold) in [
        (10..23, false),
        (14..23, true),
        (14..18, true),
        (15..17, true),
        (10..18, false),
    ] {
        for reverse in [false, true] {
            let (mut core, view) = fixture(source);
            let original = core.document().text().to_owned();
            select(&mut core, view, range.clone(), reverse);
            text(&mut core, view, "X");
            text(&mut core, view, "Y");
            assert_eq!(
                core.document().text(),
                format!("{}XY{}", &original[..range.start], &original[range.end..])
            );
            for at in range.start..range.start + 2 {
                let actual = DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false,
                )
                .unwrap();
                assert_eq!(
                    actual.bold,
                    bold,
                    "range={range:?}, reverse={reverse}: {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
            }
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    range.start,
                    false
                )
                .unwrap()
                .bold,
                bold
            );
        }
    }
}

#[test]
fn replacement_preserves_only_the_first_characters_link_identity() {
    let source =
        "plain [**link**](https://example.test/path?x=1&amp;y=2) tail";
    for (range, linked) in [
        (2..13, false),
        (6..13, true),
        (6..10, true),
        (7..9, true),
        (2..10, false),
    ] {
        for reverse in [false, true] {
            let (mut core, view) = fixture(source);
            select(&mut core, view, range.clone(), reverse);
            text(&mut core, view, "X");
            text(&mut core, view, "Y");
            for at in range.start..range.start + 2 {
                assert_eq!(
                    core.document()
                        .link_at(core.document().text_point(at).unwrap())
                        .unwrap()
                        .as_deref(),
                    linked.then_some("https://example.test/path?x=1&y=2"),
                    "range={range:?}, reverse={reverse}: {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
            }
        }
    }
}
