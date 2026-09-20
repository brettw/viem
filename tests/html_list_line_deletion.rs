use viem_core::command::{InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn document(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn native_final_list_row_deletion_keeps_valid_history_and_an_editable_empty_item() {
    for (source, prefix, suffix) in [
        (
            "<ul><li>Now ist the time </li><li>For all good men</li></ul>",
            "<ul><li>Now ist the time </li><li>",
            "</li></ul>",
        ),
        (
            "<ol start='4'><li style='color:red'>first</li><li value='9'><b>last</b></li></ol><!--keep-->",
            "<ol start='4'><li style='color:red'>first</li><li value='9'>",
            "</li></ol><!--keep-->",
        ),
    ] {
        for return_mode in [Mode::Normal, Mode::Insert] {
            for deletion in [Key::Delete, Key::Backspace] {
                let original = document(source).text().to_owned();
                let start = original.find('\n').unwrap() + 1;
                for at in [start, original.len() - 1] {
                    let mut core = Core::new(document(source));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 200.);
                    core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset: at,
                            affinity: BoundaryAffinity::Downstream,
                            extend_selection: false,
                        },
                    )
                    .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('V')))
                        .unwrap();
                    core.set_selection_origin(view, SelectionOrigin::Mouse, return_mode)
                        .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::Key(deletion)))
                        .unwrap_or_else(|error| panic!("{source}, {at}, {return_mode:?}, {deletion:?}: {error:?}"));
                    assert_eq!(core.document().text(), &original[..start]);
                    assert_eq!(core.command_state(view).unwrap().cursor(), start);
                    let deleted = core.document().source_bytes();
                    assert!(deleted.starts_with(prefix.as_bytes()));
                    assert!(deleted.ends_with(suffix.as_bytes()));
                    assert_eq!(document(std::str::from_utf8(&deleted).unwrap()).text(), core.document().text());
                    core.handle(view, CoreEvent::Input(InputEvent::text("X")))
                        .unwrap();
                    assert_eq!(core.document().text(), format!("{}X", &original[..start]));
                    let changed = core.document().source_bytes();
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                        .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), changed);
                }
            }
        }
    }
}

#[test]
fn linewise_deletion_preserves_empty_neighbors_and_consumes_only_selected_owners() {
    for (source, range, expected) in [
        (
            "<p>a</p><p></p><p>b</p><p></p>",
            0..2,
            "<p></p><p>b</p><p></p>",
        ),
        ("<p>a</p><p></p><p>b</p><p></p>", 2..5, "<p>a</p><p></p>"),
        ("<pre>a\nb\n</pre><p>c</p>", 0..5, "<p>c</p>"),
        (
            "<blockquote><blockquote><p>a</p></blockquote></blockquote><p>b</p>",
            0..2,
            "<p>b</p>",
        ),
        (
            "<ul><li><blockquote><p>a</p><p>b</p></blockquote></li><li>c</li></ul>",
            0..4,
            "<ul><li>c</li></ul>",
        ),
    ] {
        let mut document = document(source);
        document
            .delete_lines(range.clone())
            .unwrap_or_else(|error| panic!("{source} {range:?}: {error:?}"));
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(projected_text(expected), document.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

fn projected_text(source: &str) -> String {
    document(source).text().to_owned()
}

#[test]
fn deleting_all_lines_clears_selected_paragraph_treatments() {
    for source in [
        "<blockquote><p>a</p><p>b</p></blockquote>",
        "<blockquote style='color:red'><h2>a</h2><p>b</p></blockquote>",
        "<div data-keep='yes'><pre>a\nb</pre><p>c</p></div>",
    ] {
        let mut document = document(source);
        document.delete_lines(0..document.text().len()).unwrap();
        assert_eq!(document.text(), "");
        assert_eq!(document.source_bytes(), b"<p></p>");
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        for after in [document.projection(), reopened.projection()] {
            assert_eq!(after.blocks().len(), 1, "{source}");
            assert_eq!(after.blocks()[0].style.0, "Paragraph", "{source}");
            assert_eq!(after.blocks()[0].direct_default_character, Default::default(), "{source}");
            assert_eq!(after.blocks()[0].direct_paragraph, Default::default(), "{source}");
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn deleting_only_paragraph_text_retains_its_owner_and_style() {
    for source in [
        "<blockquote><p>a</p></blockquote>",
        "<blockquote style='color:red'><h2>a</h2></blockquote>",
        "<div data-keep='yes'><pre>a</pre></div>",
    ] {
        let mut document = document(source);
        let before = document.projection().blocks()[0].clone();
        document.replace(0..document.text().len(), "").unwrap();
        assert_eq!(document.text(), "");
        assert_eq!(document.source_bytes(), source.replacen(">a<", "><", 1).as_bytes());
        let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        for after in [document.projection(), reopened.projection()] {
            assert_eq!(after.blocks()[0].style, before.style, "{source}");
            assert_eq!(after.blocks()[0].direct_default_character, before.direct_default_character, "{source}");
            assert_eq!(after.blocks()[0].direct_paragraph, before.direct_paragraph, "{source}");
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
