use viem_core::command::{CommandInterpreter, InputEvent};
use viem_core::{Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn visible_list_boundaries_accept_typing_and_line_deletion() {
    let mut failures = Vec::new();
    for source in [
        "<ul><li>First</li><li>Second</li></ul>",
        "<ol><li><p>First</p></li><li><p>Second</p></li></ol>",
        "<ul><li><b>First</b><i></i></li><li></li></ul>",
        "<ul><li></li><li><b>Second</b></li></ul>",
        "<ul><li><p></p></li><li><p>Second</p></li></ul>",
        "<ol><li>First<br></li><li>Second</li></ol>",
        "<ol><li>A&fjlig;</li><li>B</li></ol>",
        "<ul><li>First</li></ul><p></p>",
    ] {
        let original = html(source);
        for at in 0..=original.text().len() {
            let mut document = html(source);
            if let Err(error) = document.replace(at..at, "X") {
                failures.push(format!("insert {source:?} at {at}: {error:?}"));
            }
        }
        for line in 0..original.text().split('\n').count() {
            let mut document = html(source);
            let mut commands = CommandInterpreter::new();
            let keys = format!("{}Gdd", line + 1);
            for key in keys.chars() {
                if let Err(error) = commands.handle(&mut document, InputEvent::key(key)) {
                    failures.push(format!("{keys}: {source:?}: {error:?}"));
                    break;
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn partial_entity_edits_preserve_the_unselected_text_and_surrounding_syntax() {
    let source = "<p class='keep'><b>&fjlig;</b><!--keep--><i>tail</i></p>";
    for (range, replacement, expected) in [
        (1..1, "X", "fXjtail"),
        (1..2, "", "ftail"),
        (0..1, "X", "Xjtail"),
    ] {
        let mut document = html(source);
        document
            .replace(range, replacement)
            .unwrap_or_else(|error| panic!("{error:?} {:?}", document.projection().provenance()));
        assert_eq!(document.text(), expected);
        assert!(String::from_utf8(document.source_bytes())
            .unwrap()
            .contains("</b><!--keep--><i>tail</i></p>"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn keyboard_typing_at_entity_interior_uses_the_same_minimal_source_edit() {
    use viem_core::command::Key;
    use viem_core::document::{BoundaryAffinity, SemanticInlineStyle};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for input in ["X", " "] {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            for clear_bold in [false, true] {
                let source = "<ol><li><b>&fjlig;</b></li></ol><!--keep-->";
                let mut core = Core::new(html(source));
                let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
                core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                    .unwrap();
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: 1,
                        affinity,
                        extend_selection: false,
                    },
                )
                .unwrap();
                if clear_bold {
                    core.handle(
                        view,
                        CoreEvent::SetSelectionSemanticStyle {
                            expected: core.list_selection_identity(view).unwrap(),
                            style: SemanticInlineStyle::Strong,
                            enabled: false,
                        },
                    )
                    .unwrap();
                }
                core.handle(view, CoreEvent::Input(InputEvent::text(input)))
                    .unwrap_or_else(|error| {
                        panic!("{input:?} {affinity:?} clear_bold={clear_bold}: {error:?}")
                    });
                assert_eq!(core.document().text(), format!("f{input}j"));
                assert_eq!(core.command_state(view).unwrap().cursor(), 2);
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn entity_rewrite_retains_original_encoding_and_untouched_source_bytes() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        let encode = |text: &str| -> Vec<u8> {
            match encoding {
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => text.as_bytes().to_vec(),
            }
        };
        let before = encode("<p data-keep='yes'>&fjlig;</p><!--keep-->");
        let mut document = Document::from_bytes(before.clone(), encoding, Format::Html).unwrap();
        document.replace(1..1, "X").unwrap();
        assert_eq!(document.text(), "fXj");
        assert_eq!(
            document.source_bytes(),
            encode("<p data-keep='yes'>fXj</p><!--keep-->")
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before);
    }
}
