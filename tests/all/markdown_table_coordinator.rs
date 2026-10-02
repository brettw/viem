use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key, Mode, TableSelectionExtent};
use viem_core::document::{
    Encoding, Format, HistoryNavigationRequest, SemanticInlineStyle, TableEditIntent,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
const SOURCE: &str = "| A | B | C |\n| - | - | - |\n| x | y | z |\n";
fn editor(source: &str) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 300.0);
    (core, view)
}
fn select(core: &mut Editor, view: ViewId, anchor: (usize, usize), active: (usize, usize)) {
    let table = core.document().projection().tables()[0].id;
    core.select_table_cells(
        view,
        core.document().id(),
        core.document().revision(),
        Some(TableSelectionExtent {
            table,
            anchor_row: anchor.0,
            anchor_column: anchor.1,
            active_row: active.0,
            active_column: active.1,
        }),
    )
    .unwrap();
}
#[test]
fn rectangle_formatting_does_not_touch_intervening_unselected_cells() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (0, 0), (1, 0));
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style: SemanticInlineStyle::Emphasis,
            enabled: true,
        },
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(core.document().source_bytes()).unwrap(),
        SOURCE.replace(" A ", " *A* ").replace(" x ", " *x* ")
    );
    assert!(core.table_selection(view).unwrap().is_some());
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetStrikethrough {
            expected,
            enabled: true,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "A\nB\nC\nx\ny\nz");
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert!(source.contains("| B | C |"));
    assert!(source.contains("| y | z |"));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}

#[test]
fn rectangle_ime_cancellation_and_commit_are_atomic_and_use_anchor() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (1, 1), (0, 0));
    for commit in [false, true] {
        let range = core.list_selection_identity(view).unwrap().range();
        let target = CompositionTarget::at_offsets(core.document(), range).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3))),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
        core.handle(
            view,
            CoreEvent::Composition(if commit {
                CompositionEvent::Commit
            } else {
                CompositionEvent::Cancel
            }),
        )
        .unwrap();
        if !commit {
            assert!(core.table_selection(view).unwrap().is_some());
        }
    }
    assert_eq!(core.document().text(), "\n\nC\n\n猫\nz");
    assert!(core.table_selection(view).unwrap().is_none());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    core.handle(view, CoreEvent::Input(InputEvent::text("!")))
        .unwrap();
    assert_eq!(core.document().text(), "\n\nC\n\n猫!\nz");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "\n\nC\n\n猫\nz");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}

#[test]
fn table_insertion_and_following_typing_have_separate_undo_units() {
    let (mut core, view) = editor("");
    core.handle(
        view,
        CoreEvent::TableEdit {
            document: core.document().id(),
            revision: core.document().revision(),
            intent: TableEditIntent::Insert {
                range: 0..0,
                columns: 2,
                body_rows: 1,
            },
        },
    )
    .unwrap();
    let inserted = core.document().source_bytes();
    core.handle(view, CoreEvent::Input(InputEvent::text("Heading")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), inserted);
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"");
}

#[test]
fn source_switch_retires_cell_rectangle_without_changing_source() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (0, 0), (1, 0));
    core.handle(
        view,
        CoreEvent::SetMarkdownSource {
            document: core.document().id(),
            revision: core.document().revision(),
            source: true,
        },
    )
    .unwrap();
    assert!(core.table_selection(view).unwrap().is_none());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
    assert!(core
        .list_selection_identity(view)
        .unwrap()
        .range()
        .is_empty());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert!(core.table_selection(view).unwrap().is_none());
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}

#[test]
fn stale_rectangle_style_does_not_mutate_after_another_view_edits() {
    let (mut core, view) = editor(SOURCE);
    let second = core.add_view(MockTextMeasurementProvider::new(), 500., 300.);
    select(&mut core, view, (0, 0), (1, 0));
    let expected = core.list_selection_identity(view).unwrap();
    let table = core.document().projection().tables()[0].id;
    core.handle(
        second,
        CoreEvent::TableEdit {
            document: core.document().id(),
            revision: core.document().revision(),
            intent: TableEditIntent::InsertRow {
                table,
                row: 1,
                after: true,
            },
        },
    )
    .unwrap();
    let bytes = core.document().source_bytes();
    let revision = core.document().revision();
    assert!(core
        .handle(
            view,
            CoreEvent::SetStrikethrough {
                expected,
                enabled: true
            }
        )
        .is_err());
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), bytes);
}

#[test]
fn obsolete_table_width_workers_cannot_replace_current_geometry() {
    use viem_core::layout::{compute_layout_job, LayoutEngine, LayoutExecutionContext};
    for format in [Format::Markdown, Format::MarkdownSource] {
        for change in [
            CoreEvent::Resize {
                width: 420.,
                height: 180.,
            },
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(200.),
            },
            CoreEvent::SetScale(1.25),
        ] {
            let source = "| A | B |\n| - | - |\n".to_owned()
                + &"| short | value |\n".repeat(2_000)
                + "| last | offscreen maximum width |";
            let mut core = Core::new(
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
            );
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
            let request = core.prepare_view_table_refinement(view).unwrap().unwrap();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate =
                compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            core.handle(view, change).unwrap();
            let current = core.layout(view).unwrap().snapshot().unwrap();
            let revision = current.revision;
            let rows = current.rows.clone();
            let cursor = core.command_state(view).unwrap().cursor();
            assert!(!core.install_view_table_refinement(view, candidate).unwrap());
            let current = core.layout(view).unwrap().snapshot().unwrap();
            assert_eq!(current.revision, revision);
            assert!(std::sync::Arc::ptr_eq(&current.rows, &rows));
            assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert!(
                core.prepare_view_table_refinement(view).unwrap().is_some(),
                "obsolete work must not prevent further discovery"
            );
        }
    }
}

#[test]
fn deleting_selected_cells_in_another_view_retires_the_rectangle() {
    let (mut core, view) = editor(SOURCE);
    let second = core.add_view(MockTextMeasurementProvider::new(), 500., 300.);
    select(&mut core, view, (1, 0), (1, 1));
    let table = core.document().projection().tables()[0].id;
    core.handle(
        second,
        CoreEvent::TableEdit {
            document: core.document().id(),
            revision: core.document().revision(),
            intent: TableEditIntent::DeleteRow { table, row: 1 },
        },
    )
    .unwrap();
    assert!(core.table_selection(view).unwrap().is_none());
    assert!(core
        .list_selection_identity(view)
        .unwrap()
        .range()
        .is_empty());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.document().text(), "A\nB\nC");
}

#[test]
fn cancelled_table_width_request_does_not_block_its_replacement() {
    use viem_core::layout::{compute_layout_job, LayoutEngine, LayoutExecutionContext};
    let source = "| A | B |\n| - | - |\n".to_owned() + &"| row | value |\n".repeat(2_000);
    let (mut core, view) = editor(&source);
    let first = core.prepare_view_table_refinement(view).unwrap().unwrap();
    first.cancellation_token().cancel();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(6.),
        },
    )
    .unwrap();
    let top = core.layout(view).unwrap().viewport_top();
    let next = core.prepare_view_table_refinement(view).unwrap().unwrap();
    assert_ne!(first.job_id(), next.job_id());
    assert!(!next.cancellation_token().is_cancelled());
    let candidate = compute_layout_job(
        &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
        &next,
        LayoutExecutionContext::WorkerPool,
    )
    .unwrap();
    assert!(core.install_view_table_refinement(view, candidate).unwrap());
    assert_eq!(core.layout(view).unwrap().viewport_top(), top);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn end_motion_in_a_giant_table_cell_reveals_exact_caret_geometry() {
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{compute_layout_job, LayoutEngine, LayoutExecutionContext};
    for format in [Format::Markdown, Format::MarkdownSource] {
        for warm in [false, true] {
            let source = format!(
                "# Long cell interaction test\n\n| Text | Neighbor |\n| --- | --- |\n| {} | still a separate cell |\n\nAfter the table.\n",
                "readable text ".repeat(40_000)
            );
            let document = Document::from_bytes(
                source.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::MarkdownSource,
            )
            .unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
            if format == Format::Markdown {
                core.handle(
                    view,
                    CoreEvent::SetMarkdownSource {
                        document: core.document().id(),
                        revision: core.document().revision(),
                        source: false,
                    },
                )
                .unwrap();
            }
            let table = &core.document().projection().tables()[0];
            let range = if format == Format::Markdown {
                table.rows[1].cells[0].range.clone()
            } else {
                table.source_rows[2].range.clone()
            };
            if warm {
                let mut iterations = 0;
                while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
                    iterations += 1;
                    assert!(iterations < 40);
                    let candidate = compute_layout_job(
                        &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
                        &request,
                        LayoutExecutionContext::WorkerPool,
                    )
                    .unwrap();
                    assert!(core.install_view_table_refinement(view, candidate).unwrap());
                }
            }
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: range.start + 3,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('$'))))
                .unwrap();
            let commands = core.command_state(view).unwrap();
            assert_eq!(commands.cursor(), range.end - 1, "{format:?}, warm={warm}");
            let layout = core.layout(view).unwrap();
            let geometry = layout
                .snapshot()
                .unwrap()
                .logical_endpoint_geometry(commands.cursor(), commands.boundary_affinity())
                .unwrap();
            assert!(
                geometry.rect.x >= layout.viewport_left(),
                "{format:?}, warm={warm}"
            );
            assert!(
                geometry.rect.x <= layout.viewport_left() + layout.width(),
                "{format:?}, warm={warm}"
            );
            assert!(
                layout.viewport_left() > 1_000.,
                "{format:?}, warm={warm}: far caret was not revealed"
            );
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            for type_text in [false, true] {
                if !type_text && !warm {
                    continue;
                }
                if type_text {
                    let pending =
                        core.prepare_view_table_refinement(view)
                            .unwrap()
                            .map(|request| {
                                compute_layout_job(
                                    &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
                                    &request,
                                    LayoutExecutionContext::WorkerPool,
                                )
                                .unwrap()
                            });
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a'))))
                        .unwrap();
                    let before_resize = core.layout(view).unwrap().snapshot().unwrap().revision;
                    let resized = core
                        .handle(
                            view,
                            CoreEvent::Resize {
                                width: 600.,
                                height: 300.,
                            },
                        )
                        .unwrap();
                    assert!(!resized.layout_changed);
                    assert_eq!(
                        core.layout(view).unwrap().snapshot().unwrap().revision,
                        before_resize
                    );
                    {
                        let command = core.command_state(view).unwrap();
                        core.layout(view)
                            .unwrap()
                            .snapshot()
                            .unwrap()
                            .logical_endpoint_geometry(
                                command.cursor(),
                                command.boundary_affinity(),
                            )
                            .expect("same-size native layout after a must retain the exact caret");
                    }
                    core.handle(view, CoreEvent::Input(InputEvent::text("Z")))
                        .unwrap();
                    core.handle(
                        view,
                        CoreEvent::Resize {
                            width: 600.,
                            height: 300.,
                        },
                    )
                    .unwrap();
                    for width in [450., 800., 600.] {
                        core.handle(
                            view,
                            CoreEvent::Resize {
                                width,
                                height: 300.,
                            },
                        )
                        .unwrap();
                        let layout = core.layout(view).unwrap();
                        let commands = core.command_state(view).unwrap();
                        let caret = layout
                            .snapshot()
                            .unwrap()
                            .logical_endpoint_geometry(
                                commands.cursor(),
                                commands.boundary_affinity(),
                            )
                            .unwrap();
                        assert!(
                            caret.rect.x >= layout.viewport_left()
                                && caret.rect.x <= layout.viewport_left() + layout.width(),
                            "resizing lost caret: {format:?}, warm={warm}, width={width}"
                        );
                    }
                    if let Some(candidate) = pending {
                        assert!(!core.install_view_table_refinement(view, candidate).unwrap());
                    }
                    assert!(core.document().text().contains('Z'));
                    let layout = core.layout(view).unwrap();
                    let command = core.command_state(view).unwrap();
                    let geometry = layout
                        .snapshot()
                        .unwrap()
                        .logical_endpoint_geometry(command.cursor(), command.boundary_affinity())
                        .unwrap();
                    assert!(
                        geometry.rect.x >= layout.viewport_left()
                            && geometry.rect.x <= layout.viewport_left() + layout.width(),
                        "typing lost the caret: {format:?}, warm={warm}, caret={}, viewport={}..{}, cursor={}, visual={:?}, error={:?}", geometry.rect.x, layout.viewport_left(), layout.viewport_left()+layout.width(), command.cursor(), command.visual_position(), layout.last_error()
                    );
                }
                let mut refinements = 0;
                while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
                    refinements += 1;
                    assert!(
                        refinements < 40,
                        "caret-focused refinement did not converge: {format:?}, warm={warm}"
                    );
                    let candidate = compute_layout_job(
                        &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
                        &request,
                        LayoutExecutionContext::WorkerPool,
                    )
                    .unwrap();
                    assert!(core.install_view_table_refinement(view, candidate).unwrap());
                    let layout = core.layout(view).unwrap();
                    let command = core.command_state(view).unwrap();
                    let geometry = layout
                        .snapshot()
                        .unwrap()
                        .logical_endpoint_geometry(command.cursor(), command.boundary_affinity())
                        .unwrap();
                    assert!(
                    geometry.rect.x >= layout.viewport_left()
                        && geometry.rect.x <= layout.viewport_left() + layout.width(),
                    "refinement lost the caret: {format:?}, warm={warm}, iteration={refinements}"
                );
                }
            }
        }
    }
}

#[test]
fn whole_multiline_cell_trait_state_ignores_its_hard_break() {
    use viem_core::document::BoundaryAffinity;
    use viem_core::SemanticStyleState;
    let (mut core, view) = editor("| A | B |\n| - | - |\n| first | **bold**<br>plain |\n");
    let at = core.document().projection().tables()[0].rows[1].cells[0]
        .range
        .start;
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
    core.handle(view, CoreEvent::Input(InputEvent::text("i")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Tab)))
        .unwrap();
    for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style,
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(
            core.selection_semantic_style_presentation(view, style)
                .unwrap()
                .state(),
            SemanticStyleState::On
        );
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style,
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(
            core.selection_semantic_style_presentation(view, style)
                .unwrap()
                .state(),
            SemanticStyleState::Off
        );
    }
    assert_eq!(core.document().text(), "A\nB\nfirst\nbold\nplain");
}

#[test]
fn unchanged_resize_reuses_table_geometry_but_rebuilds_changed_metrics() {
    use std::sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    };
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{
        MeasurementEnvironmentId, MeasurementError, MetricsGeneration, RenderRunPolicy,
        ShapeRequest, ShapedFragment, TextMeasurementProvider,
    };
    struct Measured {
        inner: MockTextMeasurementProvider,
        calls: Arc<AtomicUsize>,
        metrics: Arc<AtomicU64>,
    }
    impl TextMeasurementProvider for Measured {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.inner.measurement_environment_id()
        }
        fn metrics_generation(&self) -> MetricsGeneration {
            MetricsGeneration(self.metrics.load(Ordering::Relaxed))
        }
        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.inner.render_run_policy()
        }
        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            self.calls.fetch_add(requests.len(), Ordering::Relaxed);
            self.inner.set_metrics_generation(self.metrics_generation());
            self.inner.shape_batch(requests)
        }
    }
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = format!(
            "| A | B |\n| - | - |\n| {} | next |\n\nAfter",
            "readable text ".repeat(12_000)
        );
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        let table = &document.projection().tables()[0];
        let offset = if format == Format::Markdown {
            table.rows[1].cells[0].range.end - 1
        } else {
            table.source_rows[2].range.end - 1
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(AtomicU64::new(1));
        let mut core = Core::new(document);
        let view = core.add_view(
            Measured {
                inner: MockTextMeasurementProvider::new(),
                calls: calls.clone(),
                metrics: metrics.clone(),
            },
            600.,
            300.,
        );
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
        let before = calls.load(Ordering::Relaxed);
        let revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        let result = core
            .handle(
                view,
                CoreEvent::Resize {
                    width: 600.,
                    height: 300.,
                },
            )
            .unwrap();
        assert!(!result.layout_changed);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            before,
            "unchanged resize shaped text"
        );
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            revision
        );
        metrics.store(2, Ordering::Relaxed);
        let result = core
            .handle(
                view,
                CoreEvent::Resize {
                    width: 600.,
                    height: 300.,
                },
            )
            .unwrap();
        assert!(result.layout_changed);
        assert!(calls.load(Ordering::Relaxed) > before);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
        let geometry = snapshot
            .logical_endpoint_geometry(offset, BoundaryAffinity::Downstream)
            .unwrap();
        assert!(
            geometry.rect.x >= layout.viewport_left()
                && geometry.rect.x <= layout.viewport_left() + layout.width(),
            "metric refresh lost the previously visible caret: {format:?}"
        );
    }
}
