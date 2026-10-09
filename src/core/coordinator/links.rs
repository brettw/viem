//! Native link edits share verified document transactions and controller history.
use super::*;
use crate::document::LinkEditIntent;
impl<P: TextMeasurementProvider> Core<P> {
    pub fn exit_link_typing(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
    ) -> Result<CoreOutcome, CoreError> {
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
            return Ok(CoreOutcome {
                command: None,
                document_changed: false,
                position_map: None,
                layout_changed: false,
                composition_changes: Vec::new(),
            });
        }
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
        let resumes_typing = self
            .views
            .get_mut(&view)
            .expect("prepared view")
            .commands
            .finish_native_link_edit(&mut self.document, caret);
        let resumes_typing = if normal_insert {
            self.views
                .get_mut(&view)
                .expect("prepared view")
                .commands
                .begin_native_character_typing(&mut self.document)
                || resumes_typing
        } else {
            resumes_typing
        };
        if insert && resumes_typing {
            if let Some(typing_caret) = typing_caret {
                self.views
                    .get_mut(&view)
                    .expect("prepared view")
                    .commands
                    .set_native_link_typing_caret(&self.document, typing_caret);
            }
        }
        let after = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before, after.clone()),
        )?;
        if resumes_typing {
            self.edit_group_owner = Some(view);
            self.edit_group_restoration = Some(OpenGroupRestoration {
                generation: self.document.edit_group_generation(),
                parent: self.document.history_status().current,
                before: after,
            });
        }
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
}
