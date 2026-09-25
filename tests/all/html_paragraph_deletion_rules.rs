use viem_core::command::{InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 300.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}, source={:?}", core.document().source_bytes()));
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for character in text.chars() {
        key(core, view, Key::Char(character));
    }
}

fn place(core: &mut Editor, view: ViewId, at: usize, extend_selection: bool) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection,
        },
    )
    .unwrap();
}

fn assert_saved_and_history(core: &mut Editor, view: ViewId, original: &str, expected: &str) {
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    let reopened = Document::from_bytes(
        core.document().source_bytes(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    assert_eq!(reopened.text(), core.document().text());
    assert_eq!(reopened.projection().blocks().len(), core.document().projection().blocks().len());
    key(core, view, Key::Escape);
    key(core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    key(core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
}

#[test]
fn deleting_only_paragraph_text_keeps_its_owner_and_unselected_blank_paragraphs() {
    let original = "<div data-keep='before'></div><p></p><p data-keep='edited'><b>word</b></p><!--keep--><p></p><div data-keep='after'></div>";
    let expected = "<div data-keep='before'></div><p></p><p data-keep='edited'></p><!--keep--><p></p><div data-keep='after'></div>";
    for deletion in [Key::Backspace, Key::Delete] {
        for reverse in [false, true] {
            let (mut core, view) = editor(original);
            assert_eq!(core.document().text(), "\nword\n");
            key(&mut core, view, Key::Char('i'));
            let endpoints = if reverse { [5, 1] } else { [1, 5] };
            place(&mut core, view, endpoints[0], false);
            place(&mut core, view, endpoints[1], true);
            key(&mut core, view, deletion);
            assert_eq!(core.document().text(), "\n\n");
            assert_saved_and_history(&mut core, view, original, expected);
        }
    }
}

#[test]
fn deleting_the_last_grapheme_keeps_the_only_paragraph() {
    let original = "<p data-keep='paragraph'><b>e\u{301}</b></p>";
    let expected = "<p data-keep='paragraph'></p>";
    for deletion in [Key::Backspace, Key::Delete] {
        let (mut core, view) = editor(original);
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, if deletion == Key::Backspace { 3 } else { 0 }, false);
        key(&mut core, view, deletion);
        assert_eq!(core.document().text(), "");
        assert_saved_and_history(&mut core, view, original, expected);
    }
}

#[test]
fn boundary_deletion_merges_only_the_two_paragraphs_it_touches() {
    for (original, separator, expected, text) in [
        ("<p data-first='yes'>a</p><!--keep--><p>b</p><p></p>", 1, "<p data-first='yes'>a<!--keep-->b</p><p></p>", "ab\n"),
        ("<p data-first='yes'></p><p></p><p>tail</p>", 0, "<p data-first='yes'></p><p>tail</p>", "\ntail"),
        ("<p>head</p><p data-first='yes'></p><p></p>", 5, "<p>head</p><p data-first='yes'></p>", "head\n"),
        ("<p>head</p><div data-keep='yes'></div><p></p>", 4, "<p>head</p><div data-keep='yes'></div>", "head"),
        ("<p>a</p><div style='color:red'><!--keep--><section></section></div><p>b</p><p>tail</p>", 1, "<p>ab</p><div style='color:red'><!--keep--><section></section></div><p>tail</p>", "ab\ntail"),
        ("<p><b>a</b></p><div style='color:red'></div><p><i>b</i></p>", 1, "<p><b>a</b><i>b</i></p><div style='color:red'></div>", "ab"),
        ("<p>a</p><p>b</p><div data-keep='tail'></div>", 1, "<p>ab</p><div data-keep='tail'></div>", "ab"),
    ] {
        for deletion in [Key::Backspace, Key::Delete] {
            let (mut core, view) = editor(original);
            let original_text = core.document().text().to_owned();
            let retained_styles = original_text.char_indices().filter(|&(at, _)| at != separator)
                .map(|(at, _)| DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false).unwrap())
                .collect::<Vec<_>>();
            key(&mut core, view, Key::Char('i'));
            place(&mut core, view, separator + usize::from(deletion == Key::Backspace), false);
            key(&mut core, view, deletion);
            assert_eq!(core.document().text(), text, "{original} {deletion:?}");
            for ((at, _), expected_style) in text.char_indices().zip(retained_styles) {
                assert_eq!(DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false).unwrap(), expected_style);
            }
            assert_saved_and_history(&mut core, view, original, expected);
        }
    }
}

#[test]
fn whole_paragraph_deletion_preserves_empty_neighbors_and_unrelated_containers() {
    let original = "<p></p><div data-keep='yes'></div><p>word</p><!--keep--><p></p><p>tail</p>";
    let expected = "<p></p><div data-keep='yes'></div><!--keep--><p></p><p>tail</p>";
    for deletion in ["dd", "Vd"] {
        let (mut core, view) = editor(original);
        assert_eq!(core.document().text(), "\nword\n\ntail");
        place(&mut core, view, 1, false);
        keys(&mut core, view, deletion);
        assert_eq!(core.document().text(), "\n\ntail");
        assert_saved_and_history(&mut core, view, original, expected);
    }
}

#[test]
fn explicit_select_all_deletion_leaves_one_empty_paragraph_and_preserves_metadata() {
    for (original, expected) in [
        ("<p>one</p><p></p><ol><li>two</li></ol>", "<p></p>"),
        ("<div></div><p>one</p><p>two</p>", "<div></div><p></p>"),
        ("<!DOCTYPE html><html><head><title>Keep</title></head><body><!--before--><p>one</p><p></p><ul><li>two</li></ul><!--after--></body></html>", "<!DOCTYPE html><html><head><title>Keep</title></head><body><!--before--><p></p><!--after--></body></html>"),
    ] {
        for deletion in [Key::Backspace, Key::Delete] {
            let (mut core, view) = editor(original);
            core.handle(
                view,
                CoreEvent::SelectAll {
                    document: core.document().id(),
                    revision: core.document().revision(),
                },
            )
            .unwrap();
            key(&mut core, view, deletion);
            assert_eq!(core.document().text(), "");
            assert_eq!(core.document().projection().blocks().len(), 1);
            assert_saved_and_history(&mut core, view, original, expected);
        }
    }
}

#[test]
fn explicit_all_line_deletion_leaves_one_empty_paragraph() {
    for original in [
        "<p>one</p><p></p><ol><li>two</li></ol>",
        "<p></p><p></p>",
        "<ul><li>one</li></ul>",
    ] {
        for deletion in ["99dd", "VGd"] {
            let (mut core, view) = editor(original);
            keys(&mut core, view, deletion);
            assert_eq!(core.document().text(), "");
            assert_eq!(core.document().projection().blocks().len(), 1);
            assert_saved_and_history(&mut core, view, original, "<p></p>");
        }
    }
}

#[test]
fn native_full_range_deletion_clears_paragraph_and_list_owners() {
    for (original, expected) in [
        ("<p>one</p><ol><li>two</li></ol>", "<p></p>"),
        ("<blockquote><h2>one</h2><p>two</p></blockquote>", "<p></p>"),
        ("<div data-keep='empty'></div><h1>one</h1><!--keep--><p>two</p>", "<div data-keep='empty'></div><p></p><!--keep-->"),
    ] {
        for deletion in [Key::Backspace, Key::Delete] {
            for reverse in [false, true] {
                let (mut core, view) = editor(original);
                key(&mut core, view, Key::Char('i'));
                let end = core.document().text().len();
                let points = if reverse { [end, 0] } else { [0, end] };
                place(&mut core, view, points[0], false);
                place(&mut core, view, points[1], true);
                key(&mut core, view, deletion);
                assert_eq!(core.document().text(), "");
                assert_eq!(core.document().projection().blocks().len(), 1);
                assert_eq!(core.document().projection().blocks()[0].style.0, "Paragraph");
                assert_saved_and_history(&mut core, view, original, expected);
            }
        }
    }
}

#[test]
fn removing_list_treatment_preserves_multiple_empty_item_paragraphs() {
    let original = "<ul><li><p></p><p></p></li></ul>";
    let expected = "<p></p><p></p>";
    let mut document =
        Document::from_bytes(original.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "\n");
    document.set_list_style(0..0, None).unwrap();
    assert_eq!(document.text(), "\n");
    assert_eq!(document.source_bytes(), expected.as_bytes());
    let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), "\n");
    assert_eq!(reopened.projection().blocks().len(), 2);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
}

#[test]
fn open_save_and_navigation_preserve_authored_empty_paragraphs_and_containers() {
    for original in [
        "<p></p><p></p>",
        "<div></div><p></p>",
        "<p></p><!--keep--><p></p><div data-keep='yes'></div>",
    ] {
        let (mut core, view) = editor(original);
        let revision = core.document().revision();
        keys(&mut core, view, "Gggi");
        key(&mut core, view, Key::Left);
        key(&mut core, view, Key::Right);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), original.as_bytes());
        assert_eq!(core.document().revision(), revision);
        assert!(!core.document().history_status().can_undo);
    }
}

#[test]
fn paragraph_deletion_retains_empty_containers_with_hidden_paragraph_markup() {
    for hidden in [
        "<template><p>hidden</p></template>",
        "<template></div><p>hidden</p></template>",
        "<template><template><h2>hidden</h2></template><p>also hidden</p></template>",
        "<template><blockquote><p>hidden</p></blockquote><br><img src='hidden'></template>",
    ] {
        let container = format!("<div data-keep='yes'>{hidden}</div>");
        let original = format!("{container}<p>word</p>");
        let expected = format!("{container}<p></p>");
        let (mut core, view) = editor(&original);
        assert_eq!(core.document().text(), "word");
        core.handle(view, CoreEvent::SelectAll {
            document: core.document().id(),
            revision: core.document().revision(),
        }).unwrap();
        key(&mut core, view, Key::Delete);
        assert_eq!(core.document().text(), "");
        assert_saved_and_history(&mut core, view, &original, &expected);

        for deletion in [Key::Backspace, Key::Delete] {
            let original = format!("<p>a</p>{container}<p>b</p>");
            let expected = format!("<p>ab</p>{container}");
            let (mut core, view) = editor(&original);
            assert_eq!(core.document().text(), "a\nb");
            key(&mut core, view, Key::Char('i'));
            place(&mut core, view, 1 + usize::from(deletion == Key::Backspace), false);
            key(&mut core, view, deletion);
            assert_eq!(core.document().text(), "ab");
            assert_saved_and_history(&mut core, view, &original, &expected);
        }
    }
}
