//! Serial publication of portable completion state and its disposable preview.

use super::*;
use crate::command::completion::{CompletionPresentation, CompletionSession};

fn completion_followup_failure(error: CoreError) -> CoreOutcome {
    let message = match error {
        CoreError::Document(error) => error.to_string(),
        _ => {
            "The completion was accepted, but the following input could not be applied.".to_owned()
        }
    };
    CoreOutcome {
        command: Some(crate::command::completion::failed_completion_terminator(
            message,
        )),
        document_changed: false,
        position_map: None,
        layout_changed: false,
        composition_changes: Vec::new(),
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    // Completion is also Insert input even before a candidate is accepted.
    // Share the coordinator's ordinary empty/open-unit ownership transition
    // so native Save can finalize the active view's selected completion.
    pub(super) fn prepare_input_edit_group(&mut self, view_id: ViewId) {
        if self.replay_undo_floor.is_some() || self.edit_group_owner == Some(view_id) {
            return;
        }
        if self.edit_group_owner.take().is_some() {
            self.document.close_edit_group();
            self.edit_group_restoration = None;
        }
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| matches!(view.commands.mode(), Mode::Insert | Mode::Replace))
        {
            self.document.begin_edit_group();
            self.edit_group_owner = Some(view_id);
        }
    }
    pub fn completion_presentation(
        &self,
        view_id: ViewId,
    ) -> Result<Option<&CompletionPresentation>, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        Ok(view
            .completion
            .as_ref()
            .filter(|session| session.is_current(&self.document, &view.commands))
            .map(|session| &session.presentation))
    }

    /// Cooperative background work: each host callback does one bounded Rust
    /// search slice and returns. No worker has mutable document or UI access.
    pub fn poll_completion(&mut self, view_id: ViewId) -> Result<bool, CoreError> {
        let had_session = self
            .views
            .get(&view_id)
            .is_some_and(|view| view.completion.is_some());
        self.retire_stale_completion(view_id)?;
        self.refresh_completion_preview_policy(view_id)?;
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let Some(session) = view.completion.as_mut() else {
            return Ok(had_session);
        };
        let previous_selection = session.presentation.selected_index;
        let changed = session
            .advance(&self.document)
            .map_err(CoreError::Completion)?;
        let preview_changed = previous_selection != session.presentation.selected_index;
        if preview_changed {
            view.composition_layout = None;
            if session.has_inline_preview() {
                // Logical discovery must also work headlessly. A shaper error
                // is a presentation diagnostic, not a failed completion query.
                if let Err(error) = self.materialize_composition_layout(view_id, true) {
                    self.record_presentation_error(view_id, error);
                }
            }
        }
        Ok(changed)
    }

    pub(super) fn retire_stale_completion(&mut self, view_id: ViewId) -> Result<(), CoreError> {
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        if view
            .completion
            .as_ref()
            .is_some_and(|session| !session.is_current(&self.document, &view.commands))
        {
            view.completion = None;
            view.composition_layout = None;
        }
        Ok(())
    }

    pub(super) fn discard_completion(&mut self, view_id: ViewId) {
        if let Some(view) = self.views.get_mut(&view_id) {
            if view.completion.take().is_some() {
                view.composition_layout = None;
            }
        }
    }

    pub(super) fn handle_completion_input(
        &mut self,
        view_id: ViewId,
        input: &InputEvent,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> Result<Option<CoreOutcome>, CoreError> {
        self.retire_stale_completion(view_id)?;
        let direction = self.views[&view_id].commands.completion_direction(input);
        if let Some(direction) = direction {
            let view = self
                .views
                .get_mut(&view_id)
                .expect("validated completion view");
            if view.composition.is_some() {
                return Ok(None);
            }
            if let Some(session) = &mut view.completion {
                session.navigate(direction);
            } else {
                view.completion = Some(
                    CompletionSession::begin(&self.document, view.commands.cursor(), direction)
                        .map_err(CoreError::Completion)?,
                );
            }
            self.prepare_input_edit_group(view_id);
            self.poll_completion(view_id)?;
            return Ok(Some(CoreOutcome {
                command: self.views[&view_id]
                    .completion
                    .as_ref()
                    .map(CompletionSession::output),
                document_changed: false,
                position_map: None,
                layout_changed: true,
                composition_changes: Vec::new(),
            }));
        }
        if self.views[&view_id].completion.is_none() {
            return Ok(None);
        }
        let accepted = self.accept_completion(view_id)?;
        let event = clipboard.map_or_else(
            || CoreEvent::Input(input.clone()),
            |clipboard| CoreEvent::InputWithClipboard {
                input: input.clone(),
                clipboard: clipboard.clone(),
            },
        );
        let mut accumulated = CoreOutcomeAccumulator::new(accepted);
        // Acceptance precedes the key's ordinary action. If that action
        // fails, publish its diagnostic together with the already committed
        // completion/map rather than returning an error that hides the edit.
        // Finish layout retries for the still-uncommitted terminator before
        // merging, so an outer retry never drops acceptance's position map.
        let continued = self
            .handle_with_layout(view_id, event)
            .unwrap_or_else(completion_followup_failure);
        accumulated.merge(continued)?;
        Ok(Some(accumulated.finish()))
    }

    /// Only the chosen suffix becomes ordinary authored input. This preserves
    /// the typed prefix's styles, the Insert undo unit, counts, dot, and the
    /// last-insert register. Macro recording sees the resolved text, never the
    /// timing-dependent candidate navigation or intermediate previews.
    pub fn accept_completion(&mut self, view_id: ViewId) -> Result<CoreOutcome, CoreError> {
        self.retire_stale_completion(view_id)?;
        let baseline = self
            .views
            .get(&view_id)
            .and_then(|view| viewport::composition_caret_baseline(&self.document, view));
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let Some(session) = view.completion.take() else {
            return Ok(CoreOutcome {
                command: None,
                document_changed: false,
                position_map: None,
                layout_changed: false,
                composition_changes: Vec::new(),
            });
        };
        let previous_layout = view.composition_layout.take();
        let suffix = session.accepted_suffix().to_owned();
        if suffix.is_empty() {
            return Ok(CoreOutcome {
                command: Some(session.output()),
                document_changed: false,
                position_map: None,
                layout_changed: true,
                composition_changes: Vec::new(),
            });
        }
        match self.handle(view_id, CoreEvent::Input(InputEvent::Text(suffix))) {
            Ok(mut outcome) => {
                if let Some(baseline) = baseline {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("accepted completion retains view");
                    if let Ok(point) = self.document.text_point(view.commands.cursor()) {
                        if let Ok(anchor) = self.document.text_anchor(
                            point,
                            Association::BeforeInsertion,
                            view.commands.boundary_affinity(),
                            DeletionRecovery::PreferFollowingThenPreceding,
                        ) {
                            view.viewport_anchor = Some(ViewportTextAnchor {
                                anchor,
                                offset_from_reference: -baseline,
                                reference: ViewportAnchorReference::Baseline,
                            });
                            if let Err(error) = self.materialize_immediate_viewport(
                                view_id,
                                ImmediateLayoutIntent::PreserveViewportAndRevealCaret,
                            ) {
                                self.record_presentation_error(view_id, error);
                            }
                        }
                    }
                }
                outcome.layout_changed = true;
                Ok(outcome)
            }
            Err(error) => {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("completion acceptance retains view");
                view.completion = Some(session);
                view.composition_layout = previous_layout;
                Err(error)
            }
        }
    }

    pub(super) fn begin_composition_after_completion(
        &mut self,
        view_id: ViewId,
        target: crate::command::composition::CompositionTarget,
    ) -> Result<CoreOutcome, CoreError> {
        if target.document_id() != self.document.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if target.revision() != self.document.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: target.revision(),
            }
            .into());
        }
        let range = target.replacement_range();
        let start = self.document.text_point(range.start)?;
        let end = self.document.text_point(range.end)?;
        let accepted = self.accept_completion(view_id)?;
        let continued = (|| -> Result<CoreOutcome, CoreError> {
            let range = if let Some(map) = &accepted.position_map {
                let map_point = |point, association| -> Result<usize, CoreError> {
                    match map.map_text_point(
                        point,
                        association,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )? {
                        MappingOutcome::Exact(point)
                        | MappingOutcome::Moved(point)
                        | MappingOutcome::CollapsedByDeletion(point)
                        | MappingOutcome::RecoveredFromProvenance(point) => Ok(point.offset()),
                        _ => Err(CoreError::Composition(
                            CompositionError::UnresolvableCommitCaret,
                        )),
                    }
                };
                let start = map_point(start, Association::AfterInsertion)?;
                let end = map_point(
                    end,
                    if range.is_empty() {
                        Association::AfterInsertion
                    } else {
                        Association::BeforeInsertion
                    },
                )?;
                start..end
            } else {
                range
            };
            let target =
                crate::command::composition::CompositionTarget::at_offsets(&self.document, range)?;
            self.handle(
                view_id,
                CoreEvent::Composition(CompositionEvent::Begin(target)),
            )
        })()
        .unwrap_or_else(completion_followup_failure);
        let mut accumulated = CoreOutcomeAccumulator::new(accepted);
        accumulated.merge(continued)?;
        Ok(accumulated.finish())
    }
}
