//! Native Paste ends deferred block collection before putting at its first row.

use super::*;

impl CommandInterpreter {
    pub(super) fn paste_platform_deferred_block(
        &mut self,
        document: &mut Document,
        value: &RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        if value.kind != RegisterKind::Characterwise
            || value.text.contains('\n')
            || !value.hard_break_offsets().is_empty()
        {
            return Ok(CommandOutput::unsupported(
                "Native Paste during a block insertion requires single-line characterwise text.",
            ));
        }
        let session = self
            .visual_block_insert
            .as_ref()
            .expect("native deferred paste requires a collected block insertion");
        if session.document_id != document.id() {
            return Err(DocumentError::WrongDocument);
        }
        if session.revision != document.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: document.revision(),
                actual: session.revision,
            });
        }
        if session.kind == VisualBlockInsertKind::Change {
            if let Err(output) = self.require_register_write(session.register) {
                return Ok(output);
            }
        }
        let origin = document
            .text_anchor(
                document.text_point(session.cursor_target)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .map_err(command_position_document_error)?;
        let checkpoint = self.clone();
        let flush_positions = self.capture_position_anchors(document)?;
        let before_flush = flush_positions.history_snapshot();
        let before_flush_revision = document.revision();
        let model_checkpoint = document.begin_command_checkpoint();
        let result = (|| {
            self.record_event(&InputEvent::Key(Key::PasteClipboard));
            let separate_unit = !self.replaying && self.compound_replay_depth == 0;
            if separate_unit {
                document.end_edit_group();
            }

            // Collection and paste retain distinct undo units. The outer
            // checkpoint keeps the flush tentative until the donor succeeds,
            // including source/history/register effects from a block change.
            let (flushed, map) = document
                .capture_position_maps(|document| self.finish_visual_block_insert(document));
            let mut flushed = flushed?;
            if flushed.status != CommandStatus::Complete {
                return Ok(flushed);
            }
            let mapped = map
                .map_text_anchor(origin)
                .map_err(command_position_document_error)?;
            let caret = mapped
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            document.text_point(caret)?;
            if separate_unit {
                document.end_edit_group();
            }

            // This turn creates an intermediate undo node which the outer
            // coordinator does not visit. Record its own revision-bound
            // controller state before the native donor creates the next node.
            self.position_revision = Some(document.revision());
            if !self
                .rebase_unchanged_persistent_positions(
                    document,
                    &checkpoint,
                    &flush_positions,
                    &map,
                    true,
                )
                .map_err(command_position_document_error)?
            {
                return Err(DocumentError::VerificationFailed);
            }
            let after_flush = self.capture_history_restoration(document)?;
            if document.revision() != before_flush_revision {
                document
                    .attach_history_restoration(
                        document.history_status().current.node,
                        crate::document::HistoryRestoration::new(before_flush, after_flush),
                    )
                    .map_err(|_| DocumentError::VerificationFailed)?;
            }

            // The Normal cursor used to finish collection can floor at EOF.
            // Resume at the mapped insertion boundary before the first row's
            // payload so gP inserts there, preserving the following text.
            self.cursor = caret;
            self.insert_normal_once = None;
            self.ctrl_o_just_started = false;
            self.enter_insert(document, InsertPlacement::Before, 1);
            if let Some(session) = self.insert_session.as_mut() {
                session.preserve_normal_repeat = true;
            }
            let paste_checkpoint = self.clone();
            let paste_positions = self.capture_position_anchors(document)?;
            let before_paste = paste_positions.history_snapshot();
            let before_paste_revision = document.revision();
            let recording_suppressed = self.set_macro_recording_suppressed(true);
            let (pasted, paste_map) = document
                .capture_position_maps(|document| self.paste_platform_clipboard(document, None));
            self.set_macro_recording_suppressed(recording_suppressed);
            let pasted = pasted?;
            if pasted.status != CommandStatus::Complete {
                return Ok(pasted);
            }
            if !self
                .rebase_unchanged_persistent_positions(
                    document,
                    &paste_checkpoint,
                    &paste_positions,
                    &paste_map,
                    true,
                )
                .map_err(command_position_document_error)?
            {
                return Err(DocumentError::VerificationFailed);
            }
            if document.revision() != before_paste_revision {
                let after_paste = self.capture_history_restoration(document)?;
                document
                    .attach_history_restoration(
                        document.history_status().current.node,
                        crate::document::HistoryRestoration::new(before_paste, after_paste),
                    )
                    .map_err(|_| DocumentError::VerificationFailed)?;
            }
            flushed.merge(pasted);
            self.position_revision = Some(document.revision());
            Ok(flushed)
        })();
        match result {
            Ok(mut output) if output.status == CommandStatus::Complete => {
                self.finish_clipboard_writes(&mut output);
                self.invalidate_changed_incremental_navigation();
                document.commit_command_checkpoint(model_checkpoint);
                Ok(output)
            }
            result => {
                document.rollback_command_checkpoint(model_checkpoint);
                *self = checkpoint;
                result
            }
        }
    }
}
