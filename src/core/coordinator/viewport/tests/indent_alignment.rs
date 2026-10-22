use super::*;

fn aligned_fixture(
    intervening: usize,
    height: f32,
) -> (Core<MockTextMeasurementProvider>, ViewId, usize) {
    // A following indented line makes visible layout index the zero-indent
    // barrier too. A final unindented line needs no anchor lookup and may stay
    // outside the lazy index, where conservative invalidation reaches EOF.
    let source = format!(
        "  if (a == b ||\n{}      c == d) {{\nreset\n  after reset",
        "          nested\n".repeat(intervening)
    );
    let child = source.rfind("      c").unwrap() + 6;
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    core.set_code_language(crate::document::syntax::detection::LanguageSelection::None);
    // Let the initial syntax poll publish its empty coverage before installing
    // the controlled sheet; otherwise adding the first view replaces it.
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, height);
    let mut sheet = code_style::default_sheet();
    sheet
        .insert_character_style(
            CharacterStyle {
                id: StyleId("test:large".into()),
                based_on: None,
                properties: CharacterProperties {
                    size: Some(36.0.into()),
                    ..Default::default()
                },
            },
            StyleDefinitionMetadata::generated("Large"),
        )
        .unwrap();
    sheet.revision = code_style::snapshot().revision;
    core.publish_code_presentation_runs(Arc::new(sheet), &[]);
    core.set_align_indents_to_previous_line(view, true).unwrap();
    assert_eq!(
        code_style::resolve_syntax_name(core.document.projection().style_sheet(), "Large"),
        Some(&StyleId("test:large".into())),
        "the controlled metric style must survive layout polling"
    );
    (core, view, child)
}

fn x(core: &Core<MockTextMeasurementProvider>, view: ViewId, offset: usize) -> f32 {
    core.layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .flat_map(|row| row.clusters.iter())
        .find(|cluster| cluster.text_range.start == offset)
        .unwrap()
        .x
}

#[test]
fn syntax_metrics_on_an_anchor_invalidate_distant_dependents_without_touching_source() {
    let intervening = 32;
    let (mut core, view, child) = aligned_fixture(intervening, 2400.0);
    let child_line = intervening + 1;
    let barrier_line = child_line + 1;
    assert!((x(&core, view, 6) - x(&core, view, child)).abs() < 0.001);
    let before = x(&core, view, child);
    let bytes = core.document.source_bytes();
    let revision = core.document.revision();
    let history = core.document.history_status();
    assert!(core
        .layout(view)
        .unwrap()
        .regional_hard_line_layout(child_line)
        .is_some());
    assert!(core
        .layout(view)
        .unwrap()
        .regional_hard_line_layout(barrier_line)
        .is_some());
    let sheet = Arc::new(core.document.projection().style_sheet().clone());
    core.publish_code_presentation_runs(
        sheet,
        &[SyntaxRun {
            range: 2..4,
            name: SyntaxStyleName("Large".into()),
            origin: "controlled anchor metrics".into(),
            priority: 100,
        }],
    );
    assert!(core.layout(view).unwrap().regional_hard_line_layout(child_line).is_none(),
        "the dependent's own source and syntax stayed unchanged, but its x depends on the anchor's metrics");
    assert!(
        core.layout(view)
            .unwrap()
            .regional_hard_line_layout(barrier_line)
            .is_some(),
        "a less-indented line ends the affected dependency subtree"
    );
    core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport)
        .unwrap();
    assert!((x(&core, view, 6) - x(&core, view, child)).abs() < 0.001);
    assert!(x(&core, view, child) > before);
    assert_eq!(core.document.source_bytes(), bytes);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status(), history);
}

#[test]
fn anchor_metric_invalidation_rejects_speculation_beyond_the_indexed_prefix() {
    let (mut core, view, _) = aligned_fixture(800, 60.0);
    let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
    let LayoutJobRegion::HardLines(region) = request.region() else {
        panic!("expected cache-only work")
    };
    assert!(region.range().start > 0, "speculation excludes the anchor");
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let candidate =
        compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    core.views
        .get_mut(&view)
        .unwrap()
        .layout
        .invalidate_syntax_presentation_lines(0..1);
    assert!(
        !core.install_view_prelayout(view, true, candidate).unwrap(),
        "speculative dependent geometry must validate metrics of off-region anchors"
    );
}

#[test]
fn body_edit_keeps_equal_indent_successors_cached_after_coordinator_rebase() {
    let source = format!(
        "root\n  changed body\n{}reset\n  after reset",
        "  unchanged body\n".repeat(40)
    );
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    core.set_code_language(crate::document::syntax::detection::LanguageSelection::None);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 4000.0);
    core.set_align_indents_to_previous_line(view, true).unwrap();
    let retained_line = 30;
    assert!(core
        .layout(view)
        .unwrap()
        .regional_hard_line_layout(retained_line)
        .is_some());

    // Stop at the coordinator's publication boundary, before visible layout
    // could repopulate an incorrectly discarded cache entry.
    let at = core
        .document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start
        + 4;
    let prepared = core
        .document
        .prepare_model_request(crate::document::ModelRequest::ApplyTextEdits {
            document: core.document.id(),
            revision: core.document.revision(),
            edits: vec![crate::document::TextEdit::new(at..at, "longer")],
        })
        .unwrap();
    let committed = core.document.commit_model_transaction(prepared).unwrap();
    core.rebase_viewport_anchors(committed.text_position_map())
        .unwrap();
    assert!(core
        .layout(view)
        .unwrap()
        .regional_hard_line_layout(1)
        .is_none());
    assert!(core.layout(view).unwrap().regional_cached_ranges().iter().any(|range| range.contains(&retained_line)),
        "the exact body-edit barrier must survive coordinator invalidation without a second dependency expansion");
}
