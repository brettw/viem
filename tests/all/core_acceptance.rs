//! Black-box acceptance tests for the portable Viem core.
//!
//! These tests intentionally use only public APIs. They exercise the seams
//! between source preservation, projections, command interpretation, history,
//! and frontend-supplied text measurement.

use viem_core::command::{CommandStatus, InputEvent, Key, Mode, RegisterKind};
use viem_core::document::{BlockKind, DocumentError, SemanticInlineStyle, StyleApplication};
use viem_core::layout::{BoundaryAffinity, LayoutError, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn encode_source(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => {
            let mut bytes = vec![0xef, 0xbb, 0xbf];
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::Latin1 => text
            .chars()
            .map(|character| {
                let value = u32::from(character);
                assert!(value <= 0xff, "test fixture is not Latin-1");
                value as u8
            })
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xff, 0xfe];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = vec![0xfe, 0xff];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            bytes
        }
    }
}

fn key(character: char) -> CoreEvent {
    CoreEvent::Input(InputEvent::Key(Key::Char(character)))
}

fn special(key: Key) -> CoreEvent {
    CoreEvent::Input(InputEvent::Key(key))
}

fn text(value: &str) -> CoreEvent {
    CoreEvent::Input(InputEvent::Text(value.to_owned()))
}

#[test]
fn text_pipelines_preserve_no_op_bytes_and_patch_locally_in_every_encoding() {
    let cases = [
        (
            Format::PlainText,
            "café\r\nsecond line",
            "café\nsecond line",
        ),
        (
            Format::Markdown,
            "# café\r\n\r\nThis is **bold**.",
            "café\nThis is bold.",
        ),
    ];

    for (format, source_text, formatted_text) in cases {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let original = encode_source(source_text, encoding);
            let mut document = Document::from_bytes(original.clone(), encoding, format)
                .unwrap_or_else(|error| panic!("failed to open {format:?}/{encoding:?}: {error}"));

            // Reading the authoritative serialization is the core's no-op
            // save path: it must not regenerate bytes from formatted text.
            assert_eq!(
                document.source_bytes(),
                original,
                "no-op serialization changed {format:?}/{encoding:?}"
            );
            assert_eq!(document.text(), formatted_text);

            let start = document.text().find('é').expect("fixture contains é");
            let end = start + 'é'.len_utf8();
            document.replace(start..end, "è").unwrap_or_else(|error| {
                panic!("failed local edit for {format:?}/{encoding:?}: {error}")
            });

            let expected_source = source_text.replacen('é', "è", 1);
            assert_eq!(
                document.source_bytes(),
                encode_source(&expected_source, encoding),
                "local edit disturbed source syntax or byte order for {format:?}/{encoding:?}"
            );

            if format == Format::Markdown {
                assert_eq!(
                    document.projection().blocks()[0].kind,
                    BlockKind::Heading(1)
                );
                assert!(document.projection().style_spans().iter().any(|span| {
                    span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)
                }));
            }
        }
    }
}

#[test]
fn controller_composes_operator_insert_register_and_undo_redo_sequences() {
    let mut core = Core::new(Document::new("one two three"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 120.0);

    let pending = core.handle(view, key('d')).unwrap();
    assert_eq!(pending.command.unwrap().status, CommandStatus::Pending);
    let deletion = core.handle(view, key('w')).unwrap();
    assert!(deletion.document_changed);
    assert_eq!(core.document().text(), "two three");
    let deleted = core
        .command_state(view)
        .unwrap()
        .register('"')
        .expect("delete populates the unnamed register");
    assert_eq!(deleted.text, "one ");
    assert_eq!(deleted.kind, RegisterKind::Characterwise);

    core.handle(view, key('i')).unwrap();
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    core.handle(view, text("ONE ")).unwrap();
    core.handle(view, special(Key::Escape)).unwrap();
    assert_eq!(core.document().text(), "ONE two three");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);

    core.handle(view, key('u')).unwrap();
    assert_eq!(core.document().text(), "two three");
    core.handle(view, special(Key::Ctrl('r'))).unwrap();
    assert_eq!(core.document().text(), "ONE two three");
}

#[test]
fn grapheme_clusters_are_atomic_for_document_and_controller_edits() {
    let combining = "a\u{301}";
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
    let mut document = Document::new(format!("{combining}{family}z"));

    assert_eq!(
        document.delete(1..combining.len()),
        Err(DocumentError::NotGraphemeBoundary(1))
    );
    let first_boundary = document.next_grapheme_boundary(0).unwrap();
    assert_eq!(first_boundary, combining.len());
    document.delete(0..first_boundary).unwrap();
    assert_eq!(document.text(), format!("{family}z"));

    let mut core = Core::new(Document::new(format!("{family}z")));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 100.0);
    core.handle(view, key('x')).unwrap();
    assert_eq!(core.document().text(), "z");
}

#[test]
fn views_have_independent_widths_and_refresh_after_shared_edits() {
    let mut core = Core::new(Document::new(
        "one two three four five six seven eight nine ten",
    ));
    let narrow = core.add_view(MockTextMeasurementProvider::new(), 55.0, 120.0);
    let wide = core.add_view(MockTextMeasurementProvider::new(), 1_000.0, 120.0);

    let narrow_rows = core.layout(narrow).unwrap().snapshot().unwrap().rows.len();
    let wide_rows = core.layout(wide).unwrap().snapshot().unwrap().rows.len();
    assert!(narrow_rows > wide_rows);
    assert_eq!(wide_rows, 1);
    assert_ne!(
        core.layout(narrow).unwrap().usable_width(),
        core.layout(wide).unwrap().usable_width()
    );

    let stale_layout_revision = core.layout(wide).unwrap().snapshot().unwrap().revision;
    let stale_caret = core
        .layout(wide)
        .unwrap()
        .snapshot()
        .unwrap()
        .caret_point(0, BoundaryAffinity::Downstream)
        .unwrap();

    core.handle(narrow, key('i')).unwrap();
    core.handle(narrow, text("X")).unwrap();
    core.handle(narrow, special(Key::Escape)).unwrap();
    assert_eq!(
        core.layout(wide)
            .unwrap()
            .snapshot()
            .unwrap()
            .document_revision,
        core.document().revision(),
        "a shared edit boundedly rematerializes every attached visible view"
    );

    let refreshed = core.handle(wide, key('l')).unwrap();
    assert!(refreshed.layout_changed);
    let current = core.layout(wide).unwrap().snapshot().unwrap();
    assert_eq!(current.document_revision, core.document().revision());
    assert_ne!(current.revision, stale_layout_revision);
    assert!(matches!(
        current.caret_geometry(stale_caret),
        Err(LayoutError::WrongDocumentRevision | LayoutError::StaleLayout { .. })
    ));

    let wide_revision = current.revision;
    core.handle(
        narrow,
        CoreEvent::Resize {
            width: 90.0,
            height: 120.0,
        },
    )
    .unwrap();
    assert_eq!(
        core.layout(wide).unwrap().snapshot().unwrap().revision,
        wide_revision,
        "resizing one view must not invalidate another view"
    );
}
