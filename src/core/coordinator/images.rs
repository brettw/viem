//! Native link edits share verified document transactions and controller history.
use super::*;
use crate::document::ImageEditIntent;
impl<P: TextMeasurementProvider> Core<P> {
    pub fn edit_image(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
        intent: ImageEditIntent,
    ) -> Result<CoreOutcome, CoreError> {
        if self.list_selection_identity(view)? != expected {
            return Err(CoreError::StaleLogicalSelection);
        }
        let (preflight, caret) = self
            .document
            .prepare_image_edit(expected.document(), expected.revision(), intent)
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

impl<P: TextMeasurementProvider> Core<P> {
    /// Select an atomic image without replaying pointer or Vim input. The
    /// projection identity is checked before closing a typing undo group.
    pub fn select_image(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        offset: usize,
    ) -> Result<CoreOutcome, CoreError> {
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
        self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        if !self.document.format().is_wysiwyg() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let image = self
            .document
            .image_snapshot_at(self.document.text_point(offset)?)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        self.document.text_point(image.range.start)?;
        self.document.text_point(image.range.end)?;
        let composition_changes = if self.views[&view].composition.is_some() {
            self.handle_composition_event(view, CompositionEvent::Cancel)?
                .composition_changes
        } else {
            Vec::new()
        };
        self.finalize_open_edit_group(view)?;
        self.views
            .get_mut(&view)
            .expect("validated view")
            .commands
            .select_inline_image_from_pointer(&self.document, image.range)?;
        if let Err(error) =
            self.materialize_immediate_viewport(view, ImmediateLayoutIntent::RevealCaret)
        {
            self.record_presentation_error(view, error);
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            layout_changed: true,
            composition_changes,
        })
    }
}
