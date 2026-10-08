//! Controller publication after a verified native link edit.
use super::*;

impl CommandInterpreter {
    /// Retire the consumed selection and begin a fresh typing undo unit when
    /// the user came from native/Select selection or Insert/Replace. Vim Visual
    /// edits return to Normal. No input is replayed at this native boundary.
    pub(crate) fn finish_native_link_edit(
        &mut self,
        document: &mut Document,
        caret: usize,
    ) -> bool {
        let target_mode = if self.is_text_selection() {
            Mode::Insert
        } else {
            match self.mode {
                Mode::Insert => Mode::Insert,
                Mode::Replace => Mode::Replace,
                _ => Mode::Normal,
            }
        };
        self.retire_typing_context();
        self.table_cells = None;
        self.table_tab_selection = false;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.visual_to_line_end = false;
        self.visual_source_anchor = None;
        self.visual_block_rebind_error = None;
        self.selection_behavior = SelectionBehavior::Visual;
        self.selection_exclusive = false;
        self.selection_return_mode = Mode::Normal;
        self.select_delete_register = None;
        self.select_register_pending = false;
        self.pointer_word_origin = None;
        self.pointer_character_origin = None;
        self.insert_session = None;
        self.visual_block_insert = None;
        self.insert_normal_once = None;
        self.ctrl_o_just_started = false;
        self.generated_indent = None;
        self.restored_indent = None;
        self.clear_pending();
        self.cursor = caret;
        self.position_revision = Some(document.revision());
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.physical_cursor = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.mode = target_mode;
        match target_mode {
            Mode::Insert | Mode::Replace => {
                let placement = if target_mode == Mode::Replace {
                    InsertPlacement::Replace
                } else {
                    InsertPlacement::Before
                };
                self.enter_insert(document, placement, 1);
                true
            }
            _ => {
                self.cursor = normalize_normal_cursor_document(
                    document,
                    &document.hard_line_snapshot(),
                    caret,
                );
                false
            }
        }
    }
}

impl CommandInterpreter {
    pub(crate) fn select_inline_image_from_pointer(
        &mut self,
        document: &Document,
        range: std::ops::Range<usize>,
    ) -> Result<(), DocumentError> {
        self.select_inline_image(document, range.clone())?;
        self.pointer_character_origin = Some(range);
        Ok(())
    }

    pub(crate) fn select_inline_image(
        &mut self,
        document: &Document,
        range: std::ops::Range<usize>,
    ) -> Result<(), DocumentError> {
        document.text_point(range.start)?;
        document.text_point(range.end)?;
        let return_mode = self.plain_pointer_mode();
        self.set_cursor_from_pointer(document, range.start, BoundaryAffinity::Downstream, false);
        self.mode = Mode::VisualCharacter;
        self.visual_anchor = Some(range.end);
        self.cursor = range.start;
        self.selection_exclusive = true;
        self.selection_behavior = SelectionBehavior::Native;
        self.selection_return_mode = return_mode;
        self.visual_to_line_end = false;
        Ok(())
    }
}

impl CommandInterpreter {
    /// Only navigation entered while typing selects a newly reached image.
    /// Mode entry, typing, undo and native selection collapse keep their own
    /// semantics, including an insertion boundary immediately before an image.
    pub(crate) fn is_inline_image_navigation(&self, event: &InputEvent) -> bool {
        if !matches!(self.mode, Mode::Insert | Mode::Replace) || self.literal_input_pending() {
            return false;
        }
        match event {
            InputEvent::Key(Key::ModifiedNavigation { modifiers, .. }) => modifiers & 1 == 0,
            InputEvent::Key(key) => {
                NavigationKey::from_key(self.normalized_input_key(*key)).is_some()
            }
            _ => false,
        }
    }

    pub(crate) fn select_image_after_navigation(
        &mut self,
        document: &mut Document,
        before: &Self,
        revision: Revision,
        navigation: bool,
        output: &mut CommandOutput,
    ) -> Result<(), DocumentError> {
        if !navigation
            || output.status != CommandStatus::Complete
            || !output.cursor_moved
            || self.cursor == before.cursor
            || document.revision() != revision
            || !document.format().is_wysiwyg()
            || !matches!(self.mode, Mode::Insert | Mode::Replace)
        {
            return Ok(());
        }
        let Some(image) = document.image_snapshot_at(document.text_point(self.cursor)?)? else {
            return Ok(());
        };
        // The motion already committed the preceding typing unit and opened
        // an empty continuation. Retire that continuation without Escape-time
        // count replay or source cleanup; selection replacement starts its own.
        document.end_edit_group();
        self.insert_session = None;
        self.select_inline_image(document, image.range)?;
        output.mode_changed = true;
        Ok(())
    }
}
