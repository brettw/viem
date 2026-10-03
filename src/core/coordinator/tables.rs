//! Publication of source-backed table intentions and portable cell selections.
use super::*;
use crate::command::TableSelectionExtent;
use crate::document::TableEditIntent;

impl<P: TextMeasurementProvider> Core<P> {
    /// Capture one bounded visible-table width refinement. The native scheduler
    /// computes this immutable request off the buffer's serial owner, then asks
    /// again after installation until every visible table has exact widths.
    pub fn prepare_view_table_refinement(
        &mut self,
        view_id: ViewId,
    ) -> Result<Option<crate::layout::LayoutJobRequest>, CoreError> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        // Native cancellation can complete without a candidate to install.
        // Retire that request here so the next visible refinement can start.
        if view
            .active_layout_work
            .as_ref()
            .is_some_and(|work| work.cancellation.is_cancelled())
        {
            view.active_layout_work = None;
        }
        if view.composition.is_some() || view.active_layout_work.is_some() {
            return Ok(None);
        }
        let requirements = inspect_layout_provider(&view.engine);
        let Some(snapshot) =
            current_snapshot_for_layout(&self.document, &view.layout, requirements)
        else {
            return Ok(None);
        };
        if !snapshot.has_provisional_table_widths() {
            return Ok(None);
        }
        let region = LayoutJobRegion::Viewport(ViewportLayoutRegion::new(
            snapshot.coverage.hard_lines(),
            view.layout.viewport_top(),
            view.layout.height().max(f32::EPSILON),
        )?);
        self.prepare_view_layout_job(
            view_id,
            LayoutJobPriority::Background,
            region,
            LayoutCancellationToken::new(),
        )
        .map(Some)
    }

    /// Install only the currently requested table refinement. A moved viewport,
    /// new input, or changed resource identity discards the obsolete result.
    pub fn install_view_table_refinement(
        &mut self,
        view_id: ViewId,
        candidate: LayoutJobCandidate,
    ) -> Result<bool, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let job = candidate.job_id();
        if !view
            .active_layout_work
            .as_ref()
            .is_some_and(|active| active.job_id == job)
        {
            return Ok(false);
        }
        let LayoutJobRegion::Viewport(region) = candidate.requested_region() else {
            return Ok(false);
        };
        if region.viewport_top() != view.layout.viewport_top()
            || region.viewport_height() != view.layout.height()
        {
            self.views
                .get_mut(&view_id)
                .expect("validated view")
                .active_layout_work = None;
            return Ok(false);
        }
        let caret_anchor = capture_caret_baseline_anchor(&self.document, view);
        let reveal_horizontal_caret = caret_anchor.is_some_and(|anchor| {
            view.layout
                .snapshot()
                .and_then(|snapshot| viewport::anchor_geometry(snapshot, anchor).ok())
                .is_some_and(|geometry| {
                    geometry.rect.x >= view.layout.viewport_left()
                        && geometry.rect.x <= view.layout.viewport_left() + view.layout.width()
                })
        });
        let anchor = caret_anchor.or(view.viewport_anchor);
        let result = self.install_view_layout_job(view_id, candidate);
        let view = self.views.get_mut(&view_id).expect("validated view");
        if view
            .active_layout_work
            .as_ref()
            .is_some_and(|active| active.job_id == job)
        {
            view.active_layout_work = None;
        }
        match result {
            Ok(_) => {
                view.viewport_anchor = anchor;
                if let Some(top) = anchor.and_then(|anchor| {
                    view.layout.snapshot().and_then(|snapshot| {
                        viewport::anchor_geometry(snapshot, anchor)
                            .ok()
                            .map(|geometry| anchor.viewport_top(&snapshot.rows[geometry.row_index]))
                    })
                }) {
                    if let Err(error) = view.layout.set_viewport_top(top) {
                        view.layout.record_error(error);
                    }
                }
                // Width discovery must not undo a manual vertical scroll that
                // leaves the caret row partly clipped. Only retain horizontal
                // visibility of a caret that was visible before refinement.
                if reveal_horizontal_caret {
                    if let Some(geometry) = caret_anchor.and_then(|anchor| {
                        view.layout
                            .snapshot()
                            .and_then(|snapshot| viewport::anchor_geometry(snapshot, anchor).ok())
                    }) {
                        let left = view.layout.reveal_viewport_left(
                            geometry.rect.x
                                ..geometry.rect.x
                                    + geometry.rect.width.max(crate::layout::CARET_REVEAL_WIDTH),
                            geometry.rect.x,
                        );
                        if let Err(error) = view.layout.set_viewport_left(left) {
                            view.layout.record_error(error);
                        }
                    }
                }
                update_viewport_anchor(&self.document, view);
                Ok(true)
            }
            Err(CoreError::LayoutInstall(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn reject_table_block_formatting(
        &self,
        selection: &LogicalSelectionIdentity,
    ) -> Result<(), CoreError> {
        let range = selection.range();
        if selection.kind() == LogicalSelectionKind::Cells
            || self.document.projection().range_intersects_table(&range)
        {
            return Err(DocumentError::UnsupportedTableEdit(
                "Table cells retain their structural paragraph style.",
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn table_named_styles(
        &self,
        view: ViewId,
    ) -> Result<Option<crate::document::SelectedNamedStyles>, CoreError> {
        let Some(selection) = self.table_selection(view)? else {
            return Ok(None);
        };
        let table = self
            .document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == selection.table)
            .expect("validated selection");
        let mut combined: Option<crate::document::SelectedNamedStyles> = None;
        for row in selection.rows() {
            for column in selection.columns() {
                let range = table.rows[row].cells[column].range.clone();
                let next = self
                    .document
                    .projection()
                    .selected_named_styles(range, BoundaryAffinity::Downstream);
                if let Some(value) = &mut combined {
                    value.paragraph_mixed |=
                        next.paragraph_mixed || value.paragraph != next.paragraph;
                    value.character_mixed |=
                        next.character_mixed || value.character != next.character;
                    value.has_bullets |= next.has_bullets;
                    value.has_numbering |= next.has_numbering;
                    value.has_non_list |= next.has_non_list;
                    value.has_quotes |= next.has_quotes;
                    value.has_non_quote |= next.has_non_quote;
                    value.has_table |= next.has_table;
                    value.has_code_block |= next.has_code_block;
                } else {
                    combined = Some(next);
                }
            }
        }
        Ok(combined)
    }

    pub(super) fn table_strikethrough_state(
        &self,
        view: ViewId,
    ) -> Result<Option<SemanticStyleState>, CoreError> {
        let Some(selection) = self.table_selection(view)? else {
            return Ok(None);
        };
        let table = self
            .document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == selection.table)
            .expect("validated selection");
        let mut on = false;
        let mut off = false;
        for row in selection.rows() {
            for column in selection.columns() {
                let range = table.rows[row].cells[column].range.clone();
                if range.is_empty() {
                    continue;
                }
                let resolved =
                    DocumentLayoutStyles::resolve_region(self.document.projection(), range.clone())
                        .map_err(LayoutError::from)?;
                match ranged_boolean_style_state(
                    self.document.projection().text_tree(),
                    range,
                    resolved.default_paint.strikethrough,
                    resolved
                        .paint_runs
                        .iter()
                        .map(|run| (run.text_range.clone(), run.paint.strikethrough)),
                ) {
                    SemanticStyleState::Off => off = true,
                    SemanticStyleState::On => on = true,
                    SemanticStyleState::Mixed => {
                        on = true;
                        off = true;
                    }
                }
                if on && off {
                    return Ok(Some(SemanticStyleState::Mixed));
                }
            }
        }
        Ok(Some(if on {
            SemanticStyleState::On
        } else {
            SemanticStyleState::Off
        }))
    }

    pub fn table_selection(&self, view: ViewId) -> Result<Option<TableSelectionExtent>, CoreError> {
        Ok(self
            .views
            .get(&view)
            .ok_or(CoreError::UnknownView(view))?
            .commands
            .table_selection(&self.document))
    }

    pub fn select_table_cells(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        extent: Option<TableSelectionExtent>,
    ) -> Result<CoreOutcome, CoreError> {
        if document != self.document.id() || revision != self.document.revision() {
            return Err(CoreError::StaleLogicalSelection);
        }
        let mut commands = self
            .views
            .get(&view)
            .ok_or(CoreError::UnknownView(view))?
            .commands
            .clone();
        if let Some(extent) = extent {
            commands.select_table_cells(&self.document, &extent)?;
        } else {
            let cursor = commands.cursor();
            commands.set_cursor_from_pointer(
                &self.document,
                cursor,
                commands.boundary_affinity(),
                false,
            );
        }
        self.finalize_style_edit_group()?;
        self.finalize_open_edit_group(view)?;
        self.views.get_mut(&view).expect("validated view").commands = commands;
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            layout_changed: false,
            composition_changes: Vec::new(),
        })
    }

    pub(super) fn apply_table_edit(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        intent: TableEditIntent,
    ) -> Result<CoreOutcome, CoreError> {
        if document != self.document.id() || revision != self.document.revision() {
            return Err(CoreError::StaleLogicalSelection);
        }
        self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        let preserve_selection = matches!(
            intent,
            TableEditIntent::SetCellsSemanticStyle { .. }
                | TableEditIntent::SetCellsStrikethrough { .. }
                | TableEditIntent::AssignCellsNamedStyle { .. }
        );
        let enter_insert = matches!(
            intent,
            TableEditIntent::Insert { .. }
                | TableEditIntent::InsertRow { .. }
                | TableEditIntent::InsertColumn { .. }
                | TableEditIntent::ReplaceCells { .. }
        );
        let (preflight, caret) = self
            .document
            .prepare_table_edit(document, revision, intent)
            .map_err(command_model_transaction_error)?;
        if preflight.is_no_op() {
            return Ok(CoreOutcome {
                command: None,
                document_changed: false,
                position_map: None,
                layout_changed: false,
                composition_changes: Vec::new(),
            });
        }
        self.document.prepared_text_point(&preflight, caret)?;
        let map = preflight.text_position_map().clone();
        let before = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        let mapped =
            self.prepare_mapped_commands(&map, CommandInterpreter::capture_position_anchors)?;
        self.finalize_open_edit_group(view)?;
        let prepared = self
            .document
            .rebind_prepared_after_group_close(preflight)
            .map_err(command_model_transaction_error)?;
        self.document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        for (id, commands) in mapped {
            self.views.get_mut(&id).expect("prepared view").commands = commands;
        }
        if !preserve_selection {
            self.views
                .get_mut(&view)
                .expect("prepared view")
                .commands
                .finish_native_table_edit(&mut self.document, caret, enter_insert);
            if enter_insert {
                self.edit_group_owner = Some(view);
            }
        }
        let after = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before, after),
        )?;
        let composition_changes = self.refresh_views_after_native_change(view, &map)?;
        if let Err(error) =
            self.materialize_immediate_viewport(view, ImmediateLayoutIntent::RevealCaret)
        {
            self.record_presentation_error(view, error);
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: true,
            position_map: Some(map),
            layout_changed: true,
            composition_changes,
        })
    }

    pub(super) fn table_logical_selection(
        &self,
        view: ViewId,
    ) -> Result<Option<LogicalSelectionIdentity>, CoreError> {
        let Some(extent) = self.table_selection(view)? else {
            return Ok(None);
        };
        let table = self
            .document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == extent.table)
            .expect("validated selection");
        let anchor = table.rows[extent.anchor_row].cells[extent.anchor_column]
            .range
            .start;
        let active = table.rows[extent.active_row].cells[extent.active_column]
            .range
            .start;
        let first = table.rows[extent.rows().start].cells[extent.columns().start]
            .range
            .start;
        let last = table.rows[extent.rows().end - 1].cells[extent.columns().end - 1]
            .range
            .end;
        Ok(Some(LogicalSelectionIdentity {
            view,
            document: self.document.id(),
            revision: self.document.revision(),
            kind: LogicalSelectionKind::Cells,
            anchor,
            active,
            active_affinity: BoundaryAffinity::Downstream,
            range: first..last,
        }))
    }

    pub(super) fn table_semantic_style_presentation(
        &self,
        view: ViewId,
        style: SemanticInlineStyle,
    ) -> Result<Option<SemanticStylePresentation>, CoreError> {
        let Some(extent) = self.table_selection(view)? else {
            return Ok(None);
        };
        let table = self
            .document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == extent.table)
            .expect("validated selection");
        let mut on = false;
        let mut off = false;
        for row in extent.rows() {
            for column in extent.columns() {
                let range = table.rows[row].cells[column].range.clone();
                if range.is_empty() {
                    continue;
                }
                let resolved =
                    DocumentLayoutStyles::resolve_region(self.document.projection(), range.clone())
                        .map_err(LayoutError::from)?;
                let property = |value: &crate::layout::ResolvedTextStyle| match style {
                    SemanticInlineStyle::Strong => value.relative_bold,
                    SemanticInlineStyle::Emphasis => {
                        value.slant != crate::document::FontSlant::Upright
                    }
                    _ => false,
                };
                let state = if style == SemanticInlineStyle::Code {
                    ranged_boolean_style_state(
                        self.document.projection().text_tree(),
                        range.clone(),
                        false,
                        self.document
                            .projection()
                            .style_spans_for_region(&range)
                            .into_iter()
                            .filter(|span| {
                                span.application
                                    == StyleApplication::Semantic(SemanticInlineStyle::Code)
                            })
                            .map(|span| (span.range, true)),
                    )
                } else {
                    ranged_boolean_style_state(
                        self.document.projection().text_tree(),
                        range,
                        property(&resolved.default_shaping_style),
                        resolved
                            .shaping_runs
                            .iter()
                            .map(|run| (run.text_range.clone(), property(&run.style))),
                    )
                };
                match state {
                    SemanticStyleState::Off => off = true,
                    SemanticStyleState::On => on = true,
                    SemanticStyleState::Mixed => {
                        on = true;
                        off = true;
                    }
                }
                if on && off {
                    return Ok(Some(SemanticStylePresentation {
                        selection_kind: LogicalSelectionKind::Cells,
                        selection: self.table_logical_selection(view)?,
                        state: SemanticStyleState::Mixed,
                        can_set: true,
                        can_clear: true,
                    }));
                }
            }
        }
        let state = if on {
            SemanticStyleState::On
        } else {
            SemanticStyleState::Off
        };
        Ok(Some(SemanticStylePresentation {
            selection_kind: LogicalSelectionKind::Cells,
            selection: self.table_logical_selection(view)?,
            state,
            can_set: true,
            can_clear: true,
        }))
    }
}
