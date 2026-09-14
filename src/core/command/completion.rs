//! Portable manual completion policy. Search and previews never edit source.

use std::sync::atomic::{AtomicU64, Ordering};

use super::composition::{
    CompositionError, CompositionOverlay, CompositionSession, CompositionUpdate,
};
use super::{CommandInterpreter, CommandOutput, InputEvent, Key, Mode};
use crate::document::{
    Document, DocumentError, DocumentId, PositionError, Revision, WordCompletionDirection,
    WordCompletionSearch,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
pub(crate) const SEARCH_SLICE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionError {
    Search(PositionError),
    Composition(CompositionError),
    Document(DocumentError),
    IdentityExhausted,
}

impl From<CompositionError> for CompletionError {
    fn from(value: CompositionError) -> Self {
        Self::Composition(value)
    }
}
impl From<DocumentError> for CompletionError {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}

fn next_id() -> Result<u64, CompletionError> {
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            id.checked_add(1).filter(|next| *next < (1 << 63))
        })
        .map_err(|_| CompletionError::IdentityExhausted)
}

/// One backend-owned menu. Item order remains stable while search appends.
#[derive(Clone, Debug)]
pub struct CompletionPresentation {
    pub session_id: u64,
    pub generation: u64,
    pub document_id: DocumentId,
    pub revision: Revision,
    pub items: Vec<String>,
    pub selected_index: Option<usize>,
    pub searching: bool,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CompletionSession {
    pub presentation: CompletionPresentation,
    search: WordCompletionSearch,
    direction: WordCompletionDirection,
    // This offset names only the retained, exact source/projection snapshot.
    caret: usize,
    prefix: Option<String>,
    exhausted: bool,
    pending_steps: i64,
    inline_preview: bool,
    preview: CompositionSession,
}

impl CommandInterpreter {
    /// Literal-next, register operands, and pending Ctrl-G retain key ownership.
    pub(crate) fn completion_direction(
        &self,
        input: &InputEvent,
    ) -> Option<WordCompletionDirection> {
        if self.mode != Mode::Insert
            || self.register_pending
            || self.literal_input.is_some()
            || self.insert_control_g_pending()
            || self.visual_block_insert.is_some()
        {
            return None;
        }
        match input {
            InputEvent::Key(Key::Ctrl('n' | 'N')) => Some(WordCompletionDirection::Forward),
            InputEvent::Key(Key::Ctrl('p' | 'P')) => Some(WordCompletionDirection::Backward),
            _ => None,
        }
    }
}

pub(crate) fn failed_completion_terminator(message: String) -> CommandOutput {
    CommandOutput {
        status: super::CommandStatus::Error(message),
        ..CommandOutput::complete()
    }
}

impl CompletionSession {
    pub fn begin(
        document: &Document,
        caret: usize,
        direction: WordCompletionDirection,
    ) -> Result<Self, CompletionError> {
        let point = document.text_point(caret)?;
        let search = WordCompletionSearch::begin(document.hard_line_snapshot(), point, direction)
            .map_err(CompletionError::Search)?;
        let id = next_id()?;
        Ok(Self {
            presentation: CompletionPresentation {
                session_id: id,
                generation: id,
                document_id: document.id(),
                revision: document.revision(),
                items: Vec::new(),
                selected_index: None,
                searching: true,
                truncated: false,
            },
            search,
            direction,
            caret,
            prefix: None,
            exhausted: false,
            pending_steps: 1,
            inline_preview: true,
            preview: CompositionSession::begin_at_offsets(document, caret..caret)?,
        })
    }

    pub fn is_current(&self, document: &Document, commands: &CommandInterpreter) -> bool {
        self.presentation.document_id == document.id()
            && self.presentation.revision == document.revision()
            && commands.mode() == Mode::Insert
            && commands.cursor() == self.caret
    }

    pub fn navigate(&mut self, direction: WordCompletionDirection) {
        let step = if direction == self.direction { 1 } else { -1 };
        self.pending_steps = self.pending_steps.saturating_add(step);
    }

    pub fn advance(&mut self, document: &Document) -> Result<bool, CompletionError> {
        let previous_prefix_ready = self.prefix.is_some();
        let previous_selection = self.presentation.selected_index;
        let previous_searching = self.presentation.searching;
        let previous_length = self.presentation.items.len();
        let previous_truncated = self.presentation.truncated;
        if !self.exhausted {
            let batch = self.search.advance(SEARCH_SLICE_BYTES);
            if let Some(prefix) = batch.prefix {
                self.prefix = Some(prefix.text);
            }
            self.presentation.items.extend(batch.candidates);
            self.exhausted = batch.complete;
            self.presentation.truncated |= batch.truncated;
        }
        // A held key cannot turn one timer callback into an unbounded loop.
        for _ in 0..64 {
            if self.pending_steps == 0 {
                break;
            }
            let count = self.presentation.items.len();
            if count == 0 {
                if self.exhausted {
                    self.pending_steps = 0;
                }
                break;
            }
            let next = if self.pending_steps > 0 {
                match self.presentation.selected_index {
                    None => Some(0),
                    Some(index) if index + 1 < count => Some(index + 1),
                    Some(_) if !self.exhausted => break,
                    Some(_) => None,
                }
            } else {
                match self.presentation.selected_index {
                    Some(0) => None,
                    Some(index) => Some(index - 1),
                    None if !self.exhausted => break,
                    None => Some(count - 1),
                }
            };
            self.presentation.selected_index = next;
            self.pending_steps -= self.pending_steps.signum();
        }
        self.presentation.searching = !self.exhausted || self.pending_steps != 0;
        let changed = previous_selection != self.presentation.selected_index
            || previous_prefix_ready != self.prefix.is_some()
            || previous_searching != self.presentation.searching
            || previous_length != self.presentation.items.len()
            || previous_truncated != self.presentation.truncated;
        if changed {
            self.presentation.generation = next_id()?;
            if previous_selection != self.presentation.selected_index {
                let suffix = self.accepted_suffix().to_owned();
                self.preview.update(
                    document,
                    CompositionUpdate::new(suffix.clone(), suffix.len()..suffix.len()),
                )?;
                // Distinct from IME generations; old completion exports cannot
                // validate against a later session on the same source revision.
                self.preview
                    .set_preview_generation(self.presentation.generation | (1 << 63));
            }
        }
        Ok(changed)
    }

    pub fn accepted_suffix(&self) -> &str {
        self.presentation
            .selected_index
            .and_then(|index| self.presentation.items.get(index))
            .and_then(|word| word.strip_prefix(self.prefix.as_deref()?))
            .unwrap_or("")
    }

    /// This boundary belongs to the retained source snapshot. Discovery is
    /// incremental, so callers must wait instead of anchoring to the caret.
    pub(crate) fn prefix_start(&self) -> Option<usize> {
        self.search.prefix().map(|prefix| prefix.range.start)
    }

    pub(crate) fn set_inline_preview(&mut self, enabled: bool) -> bool {
        let changed = self.inline_preview != enabled;
        self.inline_preview = enabled;
        changed
    }

    pub(crate) fn has_inline_preview(&self) -> bool {
        self.inline_preview && !self.accepted_suffix().is_empty()
    }

    pub fn overlay(
        &self,
        document: &Document,
    ) -> Result<Option<CompositionOverlay>, CompositionError> {
        if !self.has_inline_preview() {
            return Ok(None);
        }
        self.preview.overlay(document).map(Some)
    }

    pub fn output(&self) -> CommandOutput {
        CommandOutput::complete()
    }
}
