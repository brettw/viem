//! Host file I/O continuations. Policy and edits remain portable.
use super::*;

impl<P: TextMeasurementProvider> Core<P> {
    pub fn finish_sourced_line(&mut self, view: ViewId) {
        if let Some(target) = self.views.get_mut(&view) {
            target.commands.finish_sourced_line();
        }
    }

    pub fn queue_sourced_line(
        &mut self,
        view: ViewId,
        text: &str,
        depth: u32,
    ) -> Result<bool, String> {
        let target = self
            .views
            .get_mut(&view)
            .ok_or("source view is no longer available")?;
        target.commands.install_buffer_state(&self.buffer_commands);
        target.commands.queue_sourced_line(text, depth)
    }

    pub(super) fn complete_read_file(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        after: usize,
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
            ModelRequest::ApplyFragmentEdits {
                document,
                revision,
                edits: vec![edit],
            },
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
        self.ensure_command_layout(view, ImmediateLayoutIntent::RevealCaret)?;
        Ok(outcome)
    }
}
