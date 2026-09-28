use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn switch(core: &mut Editor, view: ViewId, target: Format) {
    core.handle(
        view,
        CoreEvent::SetMarkdownSource {
            document: core.document().id(),
            revision: core.document().revision(),
            source: target == Format::MarkdownSource,
        },
    )
    .unwrap();
}

fn place(core: &mut Editor, view: ViewId, offset: usize) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
}

#[test]
fn conversion_preserves_insert_carets_in_repeated_unicode_content_and_history() {
    for from in [
        Format::Markdown,
        Format::MarkdownSource,
    ] {
        for to in [
            Format::Markdown,
            Format::MarkdownSource,
        ] {
            for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
                let markdown = "# Before\n\nA **café & 👩‍💻 العربية** tail.\n\n# Middle\n\nA **café & 👩‍💻 العربية** tail.\n\n# After\n\nA **café & 👩‍💻 العربية** tail.";
                let source = markdown;
                let source = source.replace('\n', "\r\n");
                let mut bytes = match encoding {
                    Encoding::Utf8 => source.as_bytes().to_vec(),
                    Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                    Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                    _ => unreachable!(),
                };
                let bom = match encoding {
                    Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
                    Encoding::Utf16Le => vec![0xff, 0xfe],
                    Encoding::Utf16Be => vec![0xfe, 0xff],
                    _ => unreachable!(),
                };
                bytes.splice(0..0, bom);
                let mut core =
                    Core::new(Document::from_bytes(bytes.clone(), encoding, from).unwrap());
                let view = core.add_view(MockTextMeasurementProvider::new(), 320., 120.);
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
                    .unwrap();
                let before = core
                    .document()
                    .text()
                    .match_indices("العربية")
                    .nth(1)
                    .unwrap()
                    .0
                    + "ال".len();
                place(&mut core, view, before);
                switch(&mut core, view, to);
                let after = core
                    .document()
                    .text()
                    .match_indices("العربية")
                    .nth(1)
                    .unwrap()
                    .0
                    + "ال".len();
                assert_eq!(
                    core.command_state(view).unwrap().cursor(),
                    after,
                    "{from:?} -> {to:?}, {encoding:?}"
                );
                assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
                core.document().text_point(after).unwrap();
                if from != to {
                    let converted = core.document().source_bytes();
                    core.handle(
                        view,
                        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
                    )
                    .unwrap();
                    assert_eq!(core.document().source_bytes(), bytes);
                    assert_eq!(core.command_state(view).unwrap().cursor(), before);
                    core.handle(
                        view,
                        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
                    )
                    .unwrap();
                    assert_eq!(core.document().source_bytes(), converted);
                    assert_eq!(core.command_state(view).unwrap().cursor(), after);
                }
            }
        }
    }
}

#[test]
fn mode_switch_keeps_each_views_visible_text_and_insertion_point_in_large_document() {
    let source = (0..600).map(|n| format!("## Heading {n:04}\n\nParagraph {n:04} **café** العربية with enough words for several wrapped rows in each view.\n\n")).collect::<String>();
    let mut core = Core::new(
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let views = [
        core.add_view(MockTextMeasurementProvider::new(), 240., 150.),
        core.add_view(MockTextMeasurementProvider::new(), 340., 180.),
    ];
    for (view, paragraph) in views.into_iter().zip([150, 450]) {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
            .unwrap();
        let offset = core
            .document()
            .text()
            .find(&format!("Paragraph {paragraph:04}"))
            .unwrap();
        place(&mut core, view, offset);
        let y = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .logical_endpoint_geometry(offset, BoundaryAffinity::Downstream)
            .unwrap()
            .rect
            .y;
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(y + 3.),
            },
        )
        .unwrap();
    }
    for format in [
        Format::MarkdownSource,
        Format::Markdown,
    ] {
        switch(&mut core, views[0], format);
        for (view, paragraph) in views.into_iter().zip([150, 450]) {
            let expected = core
                .document()
                .text()
                .find(&format!("Paragraph {paragraph:04}"))
                .unwrap();
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                expected,
                "{format:?}, {view:?}"
            );
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            assert_eq!(snapshot.document_revision, core.document().revision());
            let y = snapshot
                .logical_endpoint_geometry(expected, BoundaryAffinity::Downstream)
                .unwrap()
                .rect
                .y;
            assert!(
                (layout.viewport_top() - y - 3.).abs() < 0.1,
                "{format:?}, {view:?}: top={} expected={}",
                layout.viewport_top(),
                y + 3.
            );
            assert!(
                snapshot.coverage.hard_lines().len() < 100,
                "mode switches materialize only the visible region"
            );
        }
    }
}
