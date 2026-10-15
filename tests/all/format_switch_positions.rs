use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, DocumentMode, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn switch(core: &mut Editor, view: ViewId, target: Format) {
    core.handle(
        view,
        CoreEvent::SetDocumentMode {
            document: core.document().id(),
            revision: core.document().revision(),
            mode: match target {
                Format::PlainText => DocumentMode::PlainText,
                Format::Code => DocumentMode::Code("rust".into()),
                _ => DocumentMode::Markdown,
            },
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
        Format::PlainText,
        Format::Code,
        Format::Markdown,
    ] {
        for to in [
            Format::PlainText,
            Format::Code,
            Format::Markdown,
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
        Format::PlainText,
        Format::Code,
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

#[test]
fn compact_switch_maps_match_splice_policies_ranges_and_composition() {
    use viem_core::document::{Association, DeletionRecovery, ModelRequest, PositionMap, Splice};
    let bytes = b"# title\n\nA **bold** and `code` tail.\n\n> quote\n\nend".to_vec();
    for from in [Format::Markdown] {
        let mut document = Document::from_bytes(bytes.clone(), Encoding::Utf8, from).unwrap();
        let before = document.projection().clone();
        let before_points = (0..=document.text().len()).map(|offset| document.text_point(offset).unwrap())
            .collect::<Vec<_>>();
        let old_revision = document.revision();
        let prepared = document.prepare_model_request(ModelRequest::SetViewFormat {
            document: document.id(), revision: old_revision, target: Format::Code,
        }).unwrap();
        let splices = prepared.summary().formatted_splices().to_vec();
        let compact = prepared.text_position_map().clone();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), bytes);
        let oracle = PositionMap::for_text(document.id(), old_revision, document.revision(),
            before.text(), document.text(), splices).unwrap();
        for policy in [DeletionRecovery::PreferFollowingThenPreceding,
            DeletionRecovery::PreferPrecedingThenFollowing, DeletionRecovery::Unresolvable] {
            for association in [Association::BeforeInsertion, Association::AfterInsertion] {
                for offset in 0..=before.text().len() {
                    let point = before_points[offset];
                    assert_eq!(compact.map_text_point(point, association, BoundaryAffinity::Downstream, policy),
                        oracle.map_text_point(point, association, BoundaryAffinity::Downstream, policy));
                }
                let compact_reverse = compact.inverted().unwrap();
                let oracle_reverse = oracle.inverted().unwrap();
                for offset in 0..=document.text().len() {
                    let point = document.text_point(offset).unwrap();
                    assert_eq!(compact_reverse.map_text_point(point, association, BoundaryAffinity::Downstream, policy),
                        oracle_reverse.map_text_point(point, association, BoundaryAffinity::Downstream, policy));
                }
            }
        }
        for start in 0..before.text().len() {
            for end in start..=before.text().len() {
                let start = before_points[start];
                let end = before_points[end];
                let range = viem_core::document::TextRange::new(start, end).unwrap();
                assert_eq!(compact.map_text_range(range), oracle.map_text_range(range));
            }
        }
        let text = document.text().to_owned();
        let next_revision = viem_core::document::Revision(document.revision().0 + 1);
        let edit = PositionMap::for_text(document.id(), document.revision(), next_revision,
            &text, &format!("x{text}"), vec![Splice::new(0..0, 1).unwrap()]).unwrap();
        let composed = compact.then(&edit).unwrap();
        let expected = oracle.then(&edit).unwrap();
        for offset in 0..=before.text().len() {
            let point = before_points[offset];
            assert_eq!(composed.map_text_point(point, Association::AfterInsertion, BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding), expected.map_text_point(point,
                Association::AfterInsertion, BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding));
        }
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), bytes);
        assert_eq!(document.text(), before.text());
        document.try_redo().unwrap();
        assert_eq!(document.source_bytes(), bytes);
        assert_eq!(document.text(), text);
    }
}

#[test]
fn source_boundary_queries_match_reopened_projection_after_lazy_suffix_shifts() {
    use viem_core::document::measure_document_work;
    for format in [Format::PlainText, Format::Markdown] {
        let source = "Paragraph **bold** with café and tail.\n\n".repeat(700);
        let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        for step in 0..6 {
            let text = document.text().to_owned();
            let at = text.match_indices("with café").nth([2, 351, 698][step % 3]).unwrap().0;
            let (result, work) = measure_document_work(|| document.replace(at..at, "x"));
            result.unwrap();
            assert_eq!(work.full_projection_candidates, 0, "{format:?}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{format:?}: {work:?}");
            assert!(work.source_decoded_bytes < 4096, "{format:?}: {work:?}");
            let saved = document.source_bytes();
            let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(document.text(), reopened.text());
            for index in 0..2048 {
                let source_at = index * 104729 % (saved.len() + 1);
                for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                    let query = |document: &Document| document.projection()
                        .map_source_boundary(document.revision(), source_at, affinity)
                        .map(|result| (result.source_offset, result.formatted_offset, result.affinity, result.relation));
                    assert_eq!(query(&document), query(&reopened), "{format:?} edit {step}, source {source_at}");
                }
            }
        }
    }
}
