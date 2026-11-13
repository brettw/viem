//! Native link edits share verified document transactions and controller history.
use super::*;
use crate::document::LinkEditIntent;
impl<P: TextMeasurementProvider> Core<P> {
    /// Explicit links resolve against current source, including unsaved edits.
    /// Physical-line fragments center the recovered caret using the same row
    /// bounds, reserved margins and scroll limits as viewport alignment.
    pub fn navigate_to_link_fragment(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        fragment: &str,
    ) -> Result<Option<CoreOutcome>, CoreError> {
        self.focus_view(view);
        if !self.views.contains_key(&view) {
            return Err(CoreError::UnknownView(view));
        }
        if document != self.document.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if revision != self.document.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }
            .into());
        }
        let found = match self.document.find_link_fragment(fragment)? {
            None if self.document.format().is_code()
                && Document::link_fragment_source_line(fragment).is_none()
                && self.reflow_language().as_deref() == Some("markdown") =>
            {
                self.document.find_markdown_source_fragment(fragment)?
            }
            found => found,
        };
        let Some(offset) = found else {
            return Ok(None);
        };
        let origin = self.views[&view].commands.cursor();
        let mut outcome = self.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )?;
        self.views.get_mut(&view).unwrap().commands.record_native_jump(&self.document, origin);
        if Document::link_fragment_source_line(fragment).is_some() {
            outcome.layout_changed |= self.center_navigation_caret(view, offset);
        }
        Ok(Some(outcome))
    }

    /// Explicit navigation centers the caret row within the shared scroll extent.
    /// Disposable geometry failures retain the already completed cursor jump.
    fn center_navigation_caret(&mut self, view: ViewId, offset: usize) -> bool {
        let original_top = self.views[&view].layout.viewport_top();
        let centered = (|| -> Result<(), CoreError> {
            // A newly materialized region can refine its absolute row heights.
            // Re-evaluate the placement locally, with a finite retry budget.
            for _ in 0..3 {
                let target = &self.views[&view];
                let snapshot = target.layout.snapshot().ok_or(LayoutError::NoRows)?;
                let position = target.commands.visual_position().unwrap_or(
                    crate::command::layout_motion::VisualPosition {
                        text_offset: offset,
                        affinity: BoundaryAffinity::Downstream,
                    },
                );
                let geometry = snapshot
                    .logical_endpoint_geometry(position.text_offset, position.affinity)
                    .or_else(|_| {
                        snapshot.logical_endpoint_geometry(
                            position.text_offset,
                            if position.affinity == BoundaryAffinity::Downstream {
                                BoundaryAffinity::Upstream
                            } else {
                                BoundaryAffinity::Downstream
                            },
                        )
                    })?;
                let row = &snapshot.rows[geometry.row_index];
                let top = snapshot.centered_row_viewport_top(row, target.layout.height());
                let left = target.layout.viewport_left();
                let before = target.layout.viewport_top();
                self.materialize_requested_viewport(view, left, top)?;
                if (self.views[&view].layout.viewport_top() - before).abs() < 0.01 {
                    break;
                }
            }
            Ok(())
        })();
        // Cursor placement has succeeded. A failed disposable refinement
        // must still publish that valid navigation to native views.
        if let Err(error) = centered {
            self.record_presentation_error(view, error);
        }
        self.views[&view].layout.viewport_top() != original_top
    }

    pub fn exit_link_typing(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
    ) -> Result<CoreOutcome, CoreError> {
        self.focus_view(view);
        if expected.kind() != LogicalSelectionKind::None
            || self.list_selection_identity(view)? != expected
        {
            return Err(CoreError::StaleLogicalSelection);
        }
        let mut candidate = self.views[&view].commands.clone();
        candidate.set_typing_link_disabled(&self.document)?;
        self.publish_pending_typing(view, candidate)
    }

    pub fn edit_link(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
        intent: LinkEditIntent,
    ) -> Result<CoreOutcome, CoreError> {
        self.focus_view(view);
        if self.list_selection_identity(view)? != expected {
            return Err(CoreError::StaleLogicalSelection);
        }
        let insert = matches!(&intent, LinkEditIntent::Insert { .. });
        let normal_insert = insert
            && expected.kind() == LogicalSelectionKind::None
            && self.views[&view].commands.mode() == Mode::Normal;
        let prepare = if self.views[&view].commands.typing_link_disabled() {
            match intent {
                LinkEditIntent::Insert {
                    range,
                    text,
                    destination,
                } if range.is_empty()
                    && self.document.can_exit_link_typing(
                        range.start,
                        self.views[&view].commands.insertion_boundary_affinity(),
                    ) =>
                {
                    self.document.prepare_new_link_at_unlinked_caret(
                        range,
                        text,
                        destination,
                        self.views[&view].commands.insertion_boundary_affinity(),
                        self.views[&view].commands.typing_named_style(),
                        self.views[&view].commands.typing_properties(),
                    )
                }
                intent => self.document.prepare_link_edit(
                    expected.document(),
                    expected.revision(),
                    intent,
                ),
            }
        } else {
            self.document
                .prepare_link_edit(expected.document(), expected.revision(), intent)
        };
        let (preflight, caret) = prepare.map_err(command_model_transaction_error)?;
        let typing_caret = if insert {
            self.document
                .prepared_link_typing_caret(&preflight, caret)?
        } else {
            None
        };
        if preflight.is_no_op() {
            return Ok(CoreOutcome::default());
        }
        let map = self.commit_native_edit(view, preflight, |core| {
            let commands = &mut core.views.get_mut(&view).expect("prepared view").commands;
            let mut resumes_typing = commands.finish_native_link_edit(&mut core.document, caret);
            if normal_insert {
                resumes_typing |= commands.begin_native_character_typing(&mut core.document);
            }
            if insert && resumes_typing {
                if let Some(typing_caret) = typing_caret {
                    commands.set_native_link_typing_caret(&core.document, typing_caret);
                }
            }
            resumes_typing
        })?;
        self.finish_native_change(
            view,
            map,
            ImmediateLayoutIntent::PreserveViewportAndRevealCaret,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{MockTextMeasurementProvider, RegionalLayoutCacheLimits};

    #[test]
    fn failed_link_centering_publishes_the_revealed_cursor_and_retains_valid_geometry() {
        let source = "line\n".repeat(200);
        let mut core = Core::new(Document::new(source.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 180.);
        let layout = &core.views[&view].layout;
        let snapshot = layout.snapshot().unwrap();
        let row = snapshot.rows.last().unwrap();
        let offset = row.text_range.start;
        let fragment = format!("L{}", row.hard_line_index + 1);
        let top = row.reveal_bounds().end - layout.height();
        let layout_revision = snapshot.revision;
        assert!(row.hard_line_index < 199);
        core.views
            .get_mut(&view)
            .unwrap()
            .layout
            .set_viewport_top(top)
            .unwrap();
        assert_eq!(
            viewport_extension_needed(&core.views[&view]),
            (false, false)
        );
        // Eviction preserves the exact displayed snapshot. Only extending the
        // centered viewport will now require new provider work.
        core.views
            .get_mut(&view)
            .unwrap()
            .layout
            .set_regional_cache_limits(RegionalLayoutCacheLimits {
                max_hard_lines: 0,
                max_visual_rows: 0,
                max_estimated_bytes: 0,
            });
        core.views.get_mut(&view).unwrap().engine.clear_caches();
        let provider = core.views.get_mut(&view).unwrap().engine.provider_mut();
        let calls = provider.batch_calls();
        provider.fail_next_batches(2, "linked centering failed");
        let result = core
            .navigate_to_link_fragment(
                view,
                core.document.id(),
                core.document.revision(),
                &fragment,
            )
            .unwrap()
            .unwrap();
        assert!(result.layout_changed && !result.document_changed);
        assert_eq!(core.command_state(view).unwrap().cursor(), offset);
        assert_eq!(
            core.views[&view].engine.provider().batch_calls(),
            calls + 2,
            "only the centering refinement attempted shaping and its bounded fallback"
        );
        let layout = core.layout(view).unwrap();
        assert!(layout.last_error().is_some());
        assert_eq!(layout.viewport_top(), top);
        assert_eq!(layout.snapshot().unwrap().revision, layout_revision);
        assert_eq!(core.document.source_bytes(), source.as_bytes());
        assert!(!core.document.history_status().is_dirty);
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn edit_reference(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
        intent: crate::document::ReferenceRequest,
    ) -> Result<CoreOutcome, CoreError> {
        self.focus_view(view);
        if self.list_selection_identity(view)? != expected {
            return Err(CoreError::StaleLogicalSelection);
        }
        let selected = expected.range();
        if !matches!(expected.kind(), LogicalSelectionKind::None | LogicalSelectionKind::Character | LogicalSelectionKind::Line) {
            return Err(CoreError::StaleLogicalSelection);
        }
        if let crate::document::ReferenceRequest::Insert { offset, end, .. } = &intent {
            if selected != (*offset..*end) { return Err(CoreError::StaleLogicalSelection); }
        } else {
            let reference = self.document.reference_context_at(self.document.text_point(intent.offset())?)?
                .ok_or(DocumentError::UnsupportedFormatting)?;
            if selected.start < reference.start || selected.end > reference.end { return Err(CoreError::StaleLogicalSelection); }
        }
        if matches!(&intent, crate::document::ReferenceRequest::GoToDefinition {..} | crate::document::ReferenceRequest::GoToUse {..}) {
            let offset = self.document.reference_jump_target(&intent)?;
            let origin = self.views[&view].commands.cursor();
            let mut outcome = self.handle(view, CoreEvent::PlaceCursor {
                document_revision: self.document.revision(), text_offset: offset,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            })?;
            self.views.get_mut(&view).unwrap().commands.record_native_jump(&self.document, origin);
            if matches!(&intent, crate::document::ReferenceRequest::GoToDefinition {..}) {
                outcome.layout_changed |= self.center_navigation_caret(view, offset);
            }
            return Ok(outcome);
        }
        let (preflight, caret) = self
            .document
            .prepare_reference_edit(expected.document(), expected.revision(), &intent)
            .map_err(command_model_transaction_error)?;
        if preflight.is_no_op() {
            return Ok(CoreOutcome::default());
        }
        let map = self.commit_native_edit(view, preflight, |core| {
            core.views
                .get_mut(&view)
                .expect("prepared view")
                .commands
                .finish_native_link_edit(&mut core.document, caret)
        })?;
        self.finish_native_change(
            view,
            map,
            ImmediateLayoutIntent::PreserveViewportAndRevealCaret,
        )
    }
}

/// The link popup's target for the current selection, shared by its export
/// and by formatting-action availability.
pub struct LinkContext {
    pub selection: LogicalSelectionIdentity,
    pub can_insert: bool,
    pub linked: bool,
    pub can_exit_link: bool,
    pub can_remove_selection: bool,
    /// An inline link around the selection or caret, editable before the
    /// document's read-only flag is applied.
    pub link: Option<crate::document::LinkSnapshot>,
    /// A reference link, considered only where there is no inline link.
    pub reference: Option<crate::document::ReferenceContext>,
}

impl LinkContext {
    pub fn is_linear(&self) -> bool {
        matches!(self.selection.kind(),
            LogicalSelectionKind::None | LogicalSelectionKind::Character | LogicalSelectionKind::Line)
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn link_context(&self, view_id: ViewId) -> Result<LinkContext, CoreError> {
        self.focus_view(view_id);
        let selection = self.list_selection_identity(view_id)?;
        let commands = self.command_state(view_id).ok_or(CoreError::UnknownView(view_id))?;
        let document = &self.document;
        let range = selection.range();
        let cursor = commands.cursor();
        let affinity = if commands.mode() == Mode::Normal {
            BoundaryAffinity::Downstream
        } else {
            commands.insertion_boundary_affinity()
        };
        let sample = if range.is_empty() && affinity == BoundaryAffinity::Upstream && cursor > 0 {
            document.hard_line_snapshot().previous_grapheme_boundary(cursor).unwrap_or(cursor)
        } else if range.is_empty() {
            cursor
        } else {
            range.start
        };
        let linked = document.projection().style_spans_for_region(&(sample..sample.saturating_add(1)))
            .iter()
            .any(|span| span.application == StyleApplication::Automatic("Link".into())
                && span.range.contains(&sample)
                && (range.is_empty() || range.end <= span.range.end));
        let link = document
            .link_at_boundary(
                if range.is_empty() { cursor } else { range.start },
                if range.is_empty() { affinity } else { BoundaryAffinity::Downstream },
            )?
            .filter(|link| range.is_empty() || (link.range.start <= range.start && range.end <= link.range.end));
        // An empty-label link has no text; its popup sits at the caret.
        let link = match link {
            None if range.is_empty() => document.empty_link_at(cursor)?,
            link => link,
        };
        let reference = if link.is_none() {
            document.reference_context_at(document.text_point(sample)?)?
                .filter(|reference| range.is_empty() || range.start >= reference.start && range.end <= reference.end)
        } else {
            None
        };
        let linear = matches!(selection.kind(),
            LogicalSelectionKind::None | LogicalSelectionKind::Character | LogicalSelectionKind::Line);
        let typing_link_disabled = commands.typing_link_disabled();
        Ok(LinkContext {
            can_insert: linear && (document.can_insert_link(range.clone()) || range.is_empty() && typing_link_disabled
                && commands.typing_named_style().is_none_or(|style| !document.character_style_is_code(style))
                && document.can_exit_link_typing(cursor, affinity)),
            linked: linked && (!range.is_empty() || !typing_link_disabled),
            can_exit_link: range.is_empty() && !typing_link_disabled && document.can_exit_link_typing(cursor, affinity),
            can_remove_selection: linear && document.can_remove_link_selection(range),
            link,
            reference,
            selection,
        })
    }
}
