use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BlockKind, BoundaryAffinity, Encoding, FileFormat, Format, ListStyle, ModelRequest,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, key: Key) {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| {
            panic!(
                "{key:?}: {error:?} from {:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
    if let Some(command) = outcome.command {
        assert!(
            matches!(
                command.status,
                viem_core::command::CommandStatus::Complete
                    | viem_core::command::CommandStatus::Pending
            ),
            "{key:?}: {:?}",
            command.status
        );
    }
}

#[test]
fn enter_updates_the_following_source_numbers_in_both_views() {
    for format in [Format::Markdown] {
        for (source, expected) in [
            (
                "1. first\n2. second\n3. third",
                "1. first\n2. added\n3. second\n4. third",
            ),
            (
                "3) first\n1) second\n\nUnrelated\n\n7. last\n1. keep",
                "3) first\n4) added\n5) second\n\nUnrelated\n\n7. last\n1. keep",
            ),
            (
                "- parent\n\n  8) first\n  9) second",
                "- parent\n\n  8) first\n  9) added\n  10) second",
            ),
            (
                "8. first\n9. second\n   continued\n   - child\n10. third\n    keep",
                "8. first\n9. added\n10. second\n    continued\n    - child\n11. third\n    keep",
            ),
            (
                "8. first\n9.\tsecond\n\tcontinued\n\t- child",
                "8. first\n9. added\n10.\tsecond\n\tcontinued\n\t- child",
            ),
            (
                "> 8. first\n> 9. second\n>    continued\n>    - child",
                "> 8. first\n> 9. added\n> 10. second\n>     continued\n>     - child",
            ),
        ] {
            let mut core = Core::new(open(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            let other = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            core.handle(
                other,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: core.document().text().find("second").unwrap() + 2,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let at = core.document().text().find("first").unwrap() + 4;
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
            key(&mut core, view, Key::Char('a'));
            key(&mut core, view, Key::Enter);
            core.handle(view, CoreEvent::Input(InputEvent::text("added")))
                .unwrap();
            key(&mut core, view, Key::Escape);
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?}"
            );
            assert_eq!(
                core.command_state(other).unwrap().cursor(),
                core.document().text().find("second").unwrap() + 2
            );
            let saved = core.document().source_bytes();
            for target in [Format::Markdown] {
                let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, target).unwrap();
                assert_eq!(reopened.source_bytes(), saved);
            }
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}

#[test]
fn toggling_items_splits_and_rejoins_numbered_runs() {
    for format in [Format::Markdown] {
        let source = "1. first\n2. middle\n3. third\n4. fourth";
        let mut doc = open(source, format);
        let at = doc.text().find("middle").unwrap();
        doc.set_list_style(at..at, None).unwrap();
        let split = b"1. first\n\nmiddle\n\n1. third\n2. fourth";
        assert_eq!(doc.source_bytes(), split, "{format:?}");
        let at = doc.text().find("middle").unwrap();
        doc.set_list_style(at..at, Some(ListStyle::Numbered))
            .unwrap();
        let joined = b"1. first\n\n2. middle\n\n3. third\n4. fourth";
        assert_eq!(doc.source_bytes(), joined, "{format:?}");
        assert_eq!(doc.projection().list_structure().lists.len(), 1);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), split);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), split);
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), joined);
    }
}

#[test]
fn removing_the_first_item_preserves_the_lists_authored_start() {
    for format in [Format::Markdown] {
        let mut doc = open("3. first\n4. second\n5. third", format);
        doc.set_list_style(0..0, None).unwrap();
        assert_eq!(doc.source_bytes(), b"first\n\n3. second\n4. third");
        assert!(matches!(
            doc.projection().blocks()[0].kind,
            BlockKind::Paragraph
        ));
    }
}

#[test]
fn restarting_a_split_run_adjusts_continuation_indentation_when_numbers_shrink() {
    for format in [Format::Markdown] {
        let source = "8. first\n9. middle\n10. third\n    continued\n    - child";
        let mut doc = open(source, format);
        let at = doc.text().find("middle").unwrap();
        doc.set_list_style(at..at, None).unwrap();
        assert_eq!(
            doc.source_bytes(),
            b"8. first\n\nmiddle\n\n1. third\n   continued\n   - child"
        );
        let reopened = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), doc.text());
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn numbering_preserves_source_encoding_and_line_endings() {
    let encode = |text: &str, encoding| match encoding {
        Encoding::Utf16Le => [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        Encoding::Utf16Be => [
            vec![0xfe, 0xff],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
        Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
        Encoding::Utf8 => [vec![0xef, 0xbb, 0xbf], text.as_bytes().to_vec()].concat(),
    };
    for format in [Format::Markdown] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for (ending, file_format) in [
                ("\n", FileFormat::Unix),
                ("\r\n", FileFormat::Dos),
                ("\r", FileFormat::Mac),
            ] {
                let source =
                    format!("8) café{ending}9) middle{ending}10) tail{ending}    continued");
                let original = encode(&source, encoding);
                let mut doc = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    format,
                    file_format,
                )
                .unwrap();
                let at = doc.text().find("middle").unwrap();
                doc.set_list_style(at..at, None).unwrap();
                let expected = format!(
                    "8) café{ending}{ending}middle{ending}{ending}1) tail{ending}   continued"
                );
                assert_eq!(
                    doc.source_bytes(),
                    encode(&expected, encoding),
                    "{format:?} {encoding:?} {file_format:?}"
                );
                let reopened = Document::from_bytes_with_file_format(
                    doc.source_bytes(),
                    encoding,
                    format,
                    file_format,
                )
                .unwrap();
                assert_eq!(reopened.text(), doc.text());
                assert!(doc.undo());
                assert_eq!(doc.source_bytes(), original);
                assert!(doc.redo());
                assert_eq!(doc.source_bytes(), encode(&expected, encoding));
            }
        }
    }
}

#[test]
fn nesting_updates_both_runs_and_preserves_the_parent_start() {
    for format in [Format::Markdown] {
        let source = "3. first\n4. middle\n5. last";
        let mut doc = open(source, format);
        for (unindent, expected) in [(false, "3. first\n   1. middle\n4. last"), (true, source)] {
            let at = doc.text().find("middle").unwrap();
            doc.apply_model_request(ModelRequest::IndentList {
                document: doc.id(),
                revision: doc.revision(),
                range: at..at,
                unindent,
            })
            .unwrap();
            assert_eq!(doc.source_bytes(), expected.as_bytes(), "{format:?}");
        }
        assert!(doc.undo());
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn exiting_an_empty_numbered_item_restarts_only_following_siblings() {
    for format in [Format::Markdown] {
        for (source, ordinal, expected) in [
            (
                "1. first\n2.\n3. last",
                2,
                "1. first\n\nPlain\n\n1. last",
            ),
            (
                "8. first\n9.\n10. last\n11. tail",
                9,
                "8. first\n\nPlain\n\n1. last\n2. tail",
            ),
            (
                "8. first\n   3. child\n   4. other child\n9.\n10. last",
                9,
                "8. first\n   3. child\n   4. other child\n\nPlain\n\n1. last",
            ),
            (
                "8. first\n9.\n10. last\n    3. child\n    4. other child\n11. tail",
                9,
                "8. first\n\nPlain\n\n1. last\n   3. child\n   4. other child\n2. tail",
            ),
        ] {
            let mut core = Core::new(open(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            let at = core.document().projection().blocks().iter().find(|block| {
                matches!(block.kind, BlockKind::ListItem { ordinal: value, level: 0, .. } if value == ordinal)
            }).unwrap().range.start;
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            }).unwrap();
            key(&mut core, view, Key::Char('A'));
            key(&mut core, view, Key::Enter);
            // Renumber before prose arrives: parser-owned blank separators
            // must not erase the explicit exit-list intention.
            let exited = core.document().source_bytes();
            assert!(String::from_utf8_lossy(&exited).contains("\n1. last"), "{format:?}: {:?}", String::from_utf8_lossy(&exited));
            core.handle(view, CoreEvent::Input(InputEvent::text("Plain"))).unwrap();
            key(&mut core, view, Key::Escape);
            let saved = core.document().source_bytes();
            assert_eq!(saved, expected.as_bytes(), "{format:?}: {source:?}");
            let reopened = open(expected, format);
            assert_eq!(core.document().text(), reopened.text());
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}

#[test]
fn deleting_an_empty_numbered_item_continues_the_surviving_run() {
    for format in [Format::Markdown] {
        for (source, ordinal, expected) in [
            ("1. first\n2.\n3. last", 2, "1. first\n2. last"),
            ("8. first\n9.\n10. last", 9, "8. first\n9. last"),
        ] {
            let mut core = Core::new(open(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            let at = core.document().projection().blocks().iter().find(|block| {
                matches!(block.kind, BlockKind::ListItem { ordinal: value, level: 0, .. } if value == ordinal)
            }).unwrap().range.start;
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            }).unwrap();
            key(&mut core, view, Key::Char('d'));
            key(&mut core, view, Key::Char('d'));
            assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{format:?}: {source:?}");
            let reopened = open(expected, format);
            assert_eq!(core.document().text(), reopened.text());
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), expected.as_bytes());
        }
    }
}
