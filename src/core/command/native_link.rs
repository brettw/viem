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
