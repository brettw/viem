//! Semantic menu intentions shared by native hosts. No Vim grammar is synthesized.
use super::*;
use NativeActionKind::*;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NativeActionKind {
    SelectWord = 1,
    SelectSentence,
    SelectParagraph,
    SelectLine,
    SelectVisualRow,
    SelectBlock,
    Find,
    FindReplace,
    FindNext,
    FindPrevious,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAction {
    pub kind: NativeActionKind,
    pub origin: SelectionOrigin,
    pub return_mode: Option<Mode>,
}
impl NativeActionKind {
    pub fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            1 => Self::SelectWord,
            2 => Self::SelectSentence,
            3 => Self::SelectParagraph,
            4 => Self::SelectLine,
            5 => Self::SelectVisualRow,
            6 => Self::SelectBlock,
            7 => Self::Find,
            8 => Self::FindReplace,
            9 => Self::FindNext,
            10 => Self::FindPrevious,
            _ => return None,
        })
    }
    pub(super) fn needs_layout(self) -> bool {
        matches!(
            self,
            Self::SelectLine | Self::SelectVisualRow | Self::SelectBlock
        )
    }
}
impl CommandInterpreter {
    pub fn native_action_available(&self, kind: NativeActionKind) -> bool {
        !self.command_input_pending()
            && self.mode != Mode::CommandLine
            && (!matches!(
                kind,
                NativeActionKind::FindNext | NativeActionKind::FindPrevious
            ) || self.last_search.is_some())
    }
    pub(super) fn native_action(
        &mut self,
        document: &mut Document,
        action: NativeAction,
        context: Option<&mut LayoutCommandContext<'_>>,
    ) -> Result<CommandOutput, DocumentError> {
        if !self.native_action_available(action.kind) {
            return Ok(CommandOutput::unsupported(
                "This action is unavailable while command input is pending or no search is set",
            ));
        }
        if action.kind.needs_layout() && context.is_none() {
            return Ok(layout_required("native selection"));
        }
        let before = self.clone();
        let checkpoint = document.begin_command_checkpoint();
        let result = self.native_action_inner(document, action, context);
        if result.as_ref().is_ok_and(|output| {
            matches!(
                output.status,
                CommandStatus::Complete | CommandStatus::Pending
            )
        }) {
            document.commit_command_checkpoint(checkpoint);
        } else {
            document.rollback_command_checkpoint(checkpoint);
            *self = before;
        }
        result
    }
    fn native_action_inner(
        &mut self,
        document: &mut Document,
        action: NativeAction,
        mut context: Option<&mut LayoutCommandContext<'_>>,
    ) -> Result<CommandOutput, DocumentError> {
        let before = self.clone();
        self.record_event(&InputEvent::Key(Key::NativeAction(action)));
        let cursor = self.cursor;
        let return_mode = action.return_mode.unwrap_or_else(|| {
            if self.is_native_selection() {
                self.selection_return_mode
            } else {
                self.mode
            }
        });
        let return_mode = match return_mode {
            Mode::Insert | Mode::Replace => return_mode,
            _ => Mode::Normal,
        };
        let mut output = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            // A native action ends typing without expanding an unfinished counted
            // insertion. Keep the insertion boundary for selection and search.
            if let Some(session) = self.insert_session.as_mut() {
                session.entry_count = 1;
            }
            let result = self.finish_insert(document)?;
            self.cursor = cursor;
            result
        } else {
            CommandOutput::complete()
        };
        if output.status != CommandStatus::Complete {
            return Ok(output);
        }
        self.clear_pending();
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.mode = Mode::Normal;
        self.selection_behavior = SelectionBehavior::Visual;
        self.selection_exclusive = false;
        let next = match action.kind {
            SelectWord | SelectSentence | SelectParagraph => self.select_visual_text_object(
                document,
                TextObjectScope::Inner,
                match action.kind {
                    SelectWord => 'w',
                    SelectSentence => 's',
                    _ => 'p',
                },
                1,
            ),
            SelectLine => self.enter_visual(document, Mode::VisualLine, 1, false),
            SelectVisualRow => {
                let context = context.as_deref_mut().unwrap();
                let mut result = self.move_to_visual_row_edge(document, context, RowEdge::Start);
                if result.status == CommandStatus::Complete {
                    result.merge(self.enter_visual(document, Mode::VisualCharacter, 1, false));
                    result.merge(self.move_to_counted_visual_row_end(document, context, 1));
                }
                result
            }
            SelectBlock => self.enter_visual_block(document, context.as_deref_mut().unwrap()),
            Find => {
                self.enter_search(SearchDirection::Forward, 1);
                CommandOutput::complete()
            }
            FindReplace => {
                self.enter_command_line(CommandLineKind::Ex);
                self.command_line_state
                    .as_mut()
                    .unwrap()
                    .buffer
                    .insert("%s/");
                CommandOutput::complete()
            }
            FindNext | FindPrevious => self.repeat_search(document, action.kind == FindPrevious, 1),
        };
        if !matches!(
            next.status,
            CommandStatus::Complete | CommandStatus::Pending
        ) {
            // Failed queries never replace an existing selection or mode.
            *self = before;
            return Ok(next);
        }
        output.merge(next);
        if matches!(
            self.mode,
            Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.set_native_selection_origin(document, action.origin, return_mode);
        }
        output.mode_changed = self.mode != before.mode;
        output.cursor_moved |= self.cursor != cursor;
        Ok(output)
    }
}
