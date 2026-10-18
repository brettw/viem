//! Host file I/O continuations. Policy and edits remain portable.
use super::*;

impl<P: TextMeasurementProvider> Core<P> {
    /// Stage a command line as if typed after `:`; dispatching Enter runs it.
    pub fn stage_command_line(&mut self, view: ViewId, text: &str) -> Result<(), String> {
        let target = self.views.get_mut(&view).ok_or("command view is no longer available")?;
        target.commands.install_buffer_state(&self.buffer_commands);
        target.commands.stage_command_line(text)
    }

    pub(super) fn complete_read_file(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        after: crate::command::ex_execute::ExInsertionPoint,
        bytes: &[u8],
    ) -> Result<CoreOutcome, CoreError> {
        if document != self.document.id() {
            return Err(CoreError::Document(DocumentError::WrongDocument));
        }
        if revision != self.document.revision() {
            return Err(CoreError::Document(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }));
        }
        let edit = crate::command::ex_files::read_file_edit(&self.document, after, bytes);
        let (edit, cursor) = match edit {
            Ok(Some(edit)) => edit,
            Ok(None) => {
                return Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                })
            }
            Err(message) => {
                return Ok(CoreOutcome {
                    command: Some(CommandOutput {
                        status: CommandStatus::Error(message),
                        cursor_moved: false,
                        document_changed: false,
                        mode_changed: false,
                        history_navigation: false,
                        ex_outcome: None,
                        clipboard_writes: Vec::new(),
                        revealed_search_match: None,
                    }),
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                })
            }
        };
        let before = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        let outcome = self.apply_native_model_request(
            view,
            edit,
        )?;
        if !outcome.document_changed {
            return Ok(outcome);
        }
        self.views
            .get_mut(&view)
            .expect("validated view")
            .commands
            .set_cursor(&self.document, cursor);
        let after = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before, after),
        )?;
        if let Err(error) = self.ensure_command_layout(view, ImmediateLayoutIntent::RevealCaret, false) {
            self.record_presentation_error(view, error);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ex_execute::ExInsertionPoint;
    use crate::layout::{MeasurementError, MockTextMeasurementProvider};

    #[test]
    fn read_commit_survives_persistent_presentation_failure_and_remains_undoable() {
        let mut core = Core::new(Document::new("tail"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
        let document = core.document.id();
        let revision = core.document.revision();
        let calls = core.views[&view].engine.provider().batch_calls();
        // Both the native transaction refresh and the final read-cursor
        // refresh exhaust default-font recovery. A one-shot failure would
        // recover before the old final ensure_command_layout propagation.
        core.views.get_mut(&view).unwrap().engine.provider_mut()
            .fail_next_batches(64, "persistent read presentation failure");
        let outcome = core.handle(view, CoreEvent::ReadFile {
            document, revision, after: ExInsertionPoint { offset: 0, source: true },
            bytes: b"  imported\n".to_vec(),
        }).unwrap();
        assert!(outcome.document_changed);
        assert_eq!(core.document.source_bytes(), b"  imported\ntail");
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);
        assert!(core.views[&view].engine.provider().batch_calls() >= calls + 4,
            "the failure must persist through both refresh attempts");
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().document_revision, revision);
        assert!(matches!(core.layout(view).unwrap().last_error(),
            Some(LayoutError::Measurement(MeasurementError::Provider(message)))
                if message == "persistent read presentation failure"));
        core.views.get_mut(&view).unwrap().engine.provider_mut().fail_next_batches(0, "");
        core.handle(view, CoreEvent::Input(InputEvent::Key(crate::command::Key::Char('u')))).unwrap();
        assert_eq!(core.document.source_bytes(), b"tail");
    }
}
