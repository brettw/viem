//! Vim-compatible command interpretation.
//!
//! This module owns modal and pending parser state. All authoritative text and
//! history live in [`crate::document`]; offsets here are short-lived positions
//! in the document revision synchronously passed to [`CommandInterpreter::handle`].

pub mod clipboard;
pub mod composition;
pub mod ex;
pub mod ex_execute;
pub mod insert_motion;
pub mod layout_motion;
mod reflow;
pub mod regex_v1;
pub mod text_object;
pub mod visual_block;

mod command_line_completion;
mod command_line_edit;
mod filename_candidates;
mod input_assistance;
#[cfg(test)]
mod smart_quote_commands_tests;
#[cfg(test)]
mod code_performance_tests;
mod line_mode;
mod registers;
mod sort;
mod typing_style;
pub use command_line_edit::{CommandLineEditAction, CommandLineEditRequest, CommandLineSnapshot};
pub use line_mode::{LineLocation, LineMode};
mod text;

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, DocumentError, DocumentId,
    FileFormat, FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    HardLineSnapshot, HistoryNavigationRequest, HistoryRestorationSnapshot,
    MappingOutcome, ModelRequest, ModelTransactionError, PositionError, PositionMap,
    PreparedModelTransaction, Revision, TextAnchor, TextEdit, UnresolvableAnchor,
};
use crate::layout::{LayoutError, LayoutSnapshot};
use clipboard::{ClipboardCommandContext, ClipboardWriteRequest};
use ex::{parse_ex, ExAction};
use ex_execute::{
    commit_ex, prepare_ex, ExExecuteError, ExExecutionContext, ExExecutionState, ExFrontendRequest,
    ExNavigation, ExNormalRequest, ExOptionEffect, ExOptionName, ExOptionValue, ExOutcome,
    ExRegisterEffectKind, ExRegisterKind, ExRegisterReader, ExRegisterValue,
};
use insert_motion::{ctrl_u_delete_range_since, ctrl_w_delete_range};
use layout_motion::{
    align_viewport, g0, g_caret, g_dollar_for_document, gj, gk, screen_motion, viewport_line,
    LayoutMotionError, ScreenMotion, Viewport, ViewportAlignment, ViewportLine, VisualPosition,
};
use registers::{
    is_valid_register, DeletionClass, RegisterReadContext, RegisterWriteEffect, Registers,
};
pub use registers::{
    RegisterKind, RegisterReadError, RegisterValue, RegisterValueError, RegisterWriteError,
};
use text::{
    advance_graphemes, find_character, first_non_blank, grapheme_column, grapheme_range_at,
    is_grapheme_boundary, last_grapheme_on_line, last_non_blank, line_count, line_end, line_range,
    line_start, linewise_range, move_horizontal, move_paragraph, move_sentence, move_vertical,
    move_word_backward, move_word_end, move_word_end_backward, move_word_forward,
    next_grapheme_boundary, next_line_start, normalize_normal_cursor,
    normalize_normal_cursor_snapshot, nth_line_start, position_at_column,
    previous_grapheme_boundary, repeat_find_character,
};
use text_object::{resolve_text_object, TextObject, TextObjectKind, TextObjectScope};
use visual_block::{
    apply_block_edits, delete_text_edits, resolve_block_selection,
    resolve_block_selection_to_line_end, BlockSelection, ResolvedBlockSelection, VisualBlockError,
};

// Macro and `:normal` replay currently recurse through the compatibility
// interpreter. Keep a conservative shared ceiling below the native stack
// limit; a future iterative command-plan executor can raise this safely.
pub(crate) const COMPOUND_REPLAY_LIMIT: usize = 32;
/// Bounds synchronous compatibility-interpreter work in one macro command.
/// Large documents remain editable because this counts replayed input events,
/// not document content inspected by one event.
pub(crate) const MACRO_REPLAY_EVENT_LIMIT: usize = 100_000;

impl ExRegisterReader for Registers {
    fn read(&self, requested: Option<char>) -> Option<ExRegisterValue> {
        let value = self.get(requested.unwrap_or('"'))?;
        Some(register_to_ex_value(value))
    }
}

struct ContextualExRegisterReader<'a> {
    stored: &'a Registers,
    override_value: Option<(Option<char>, ExRegisterValue)>,
}

impl ExRegisterReader for ContextualExRegisterReader<'_> {
    fn read(&self, requested: Option<char>) -> Option<ExRegisterValue> {
        if let Some((override_name, value)) = &self.override_value {
            if *override_name == requested {
                return Some(value.clone());
            }
        }
        <Registers as ExRegisterReader>::read(self.stored, requested)
    }
}

fn register_to_ex_value(value: &RegisterValue) -> ExRegisterValue {
    let kind = match value.kind {
        RegisterKind::Characterwise => ExRegisterKind::Characterwise,
        RegisterKind::Linewise => ExRegisterKind::Linewise,
        // Ex has no display-space block range. Treat a block register as
        // characterwise there rather than discarding its row separators.
        RegisterKind::Blockwise => ExRegisterKind::Characterwise,
    };
    ExRegisterValue::try_new(
        value.text.clone(),
        kind,
        value.hard_break_offsets().to_vec(),
    )
    .expect("command registers carry validated semantic-break offsets")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Replace,
    VisualCharacter,
    VisualLine,
    VisualBlock,
    CommandLine,
}

/// The active `:`/search prompt. The prompt prefix is represented separately
/// from its editable contents so frontends never have to infer command kind
/// from display text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandLineKind {
    Ex,
    SearchForward,
    SearchBackward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Escape,
    Enter,
    ShiftEnter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    DocumentStart,
    DocumentEnd,
    /// Portable replayable native whole-document selection intention.
    SelectAll,
    PageUp,
    PageDown,
    Ctrl(char),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Key(Key),
    Text(String),
}

impl InputEvent {
    pub fn key(character: char) -> Self {
        Self::Key(Key::Char(character))
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

impl From<char> for InputEvent {
    fn from(value: char) -> Self {
        Self::key(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    Complete,
    Pending,
    Cancelled,
    /// The command is valid, but its exact visual destination is outside the
    /// currently materialized partial layout. The coordinator must leave the
    /// command and document state unchanged while a scheduler extends layout.
    NeedsMoreLayout(LayoutMotionError),
    SearchNotFound,
    CountError(CountError),
    Unsupported(String),
    Error(String),
    RegisterReadError(RegisterReadError),
    RegisterWriteError(RegisterWriteError),
    ExError(ExCommandError),
    VisualBlockError(VisualBlockError),
}

/// Typed failures while parsing or applying a Vim command count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountError {
    Overflow,
    ReplayEventBudgetExceeded {
        count: usize,
        events_per_iteration: usize,
        limit: usize,
    },
}

/// Typed failures produced by the Ex parser or revision-bound executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExCommandError {
    Parse(ex::ExParseError),
    Execute(ExExecuteError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub status: CommandStatus,
    pub cursor_moved: bool,
    pub document_changed: bool,
    pub mode_changed: bool,
    /// This source transition installed a retained history state rather than
    /// committing a new undo unit. The coordinator uses the stored history
    /// restoration anchors instead of treating the invoking cursor as an
    /// ordinary post-edit position.
    pub history_navigation: bool,
    /// Present after a successfully committed Ex command. This carries typed
    /// host requests plus the exact side effects committed by the controller.
    /// Internal continuations such as `:normal` are consumed before this
    /// output crosses the core coordinator boundary.
    pub ex_outcome: Option<ExOutcome>,
    /// Owned provider writes emitted by successful `+`/`*` register effects.
    /// The caller executes these only after the coordinator turn returns.
    pub clipboard_writes: Vec<ClipboardWriteRequest>,
}

/// Exact immutable inputs against which one Vim event is resolved.
///
/// This deliberately exposes document queries through an immutable reference;
/// planning cannot retain or acquire mutable model state. Layout-dependent
/// commands continue through the explicitly marked compatibility executor
/// until their viewport effects are represented in [`CommandPlan`].
#[derive(Clone, Copy)]
pub struct CommandContext<'a> {
    document: &'a Document,
    clipboard: Option<&'a ClipboardCommandContext>,
}

impl<'a> CommandContext<'a> {
    pub fn new(document: &'a Document) -> Self {
        Self {
            document,
            clipboard: None,
        }
    }

    pub fn with_clipboard(document: &'a Document, clipboard: &'a ClipboardCommandContext) -> Self {
        Self {
            document,
            clipboard: Some(clipboard),
        }
    }

    pub fn document(&self) -> &'a Document {
        self.document
    }

    pub fn clipboard(&self) -> Option<&'a ClipboardCommandContext> {
        self.clipboard
    }

    pub fn document_id(&self) -> DocumentId {
        self.document.id()
    }

    pub fn document_revision(&self) -> Revision {
        self.document.revision()
    }
}

/// Model operation emitted by the command controller.
///
/// Structured text payloads preserve the distinction between a semantic hard
/// break and a literal U+000A, which a flat `TextEdit` cannot express.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandModelRequest {
    Model(ModelRequest),
    FormattedPayload(FormattedPayloadEditRequest),
}

impl CommandModelRequest {
    pub fn document(&self) -> DocumentId {
        match self {
            Self::Model(request) => request.document(),
            Self::FormattedPayload(request) => request.document(),
        }
    }

    pub fn revision(&self) -> Revision {
        match self {
            Self::Model(request) => request.revision(),
            Self::FormattedPayload(request) => request.revision(),
        }
    }

    pub(crate) fn prepare(
        &self,
        document: &Document,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        match self {
            Self::Model(request) => document.prepare_model_request(request.clone()),
            Self::FormattedPayload(request) => {
                document.prepare_formatted_payload_request(request.clone())
            }
        }
    }
}

/// Command-layer ownership change requested at the publication boundary.
/// `End` is used by time navigation; ordinary mutations preserve the group
/// already established by their mode/coordinator context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndoGroupDirective {
    Preserve,
    End,
}

/// Typed presentation work returned after controller/model publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPresentationRequest {
    PreserveViewport,
    RevealCaret,
    Relayout,
}

/// Why a command still requires the old combined interpreter/model executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyCommandReason {
    LayoutDependent,
    CompoundOrUnmigrated,
}

/// Result of resolving an event at the command boundary.
pub enum CommandResolution {
    Planned(CommandPlan),
    Legacy(LegacyCommandReason),
}

/// Revision-bound command result. The private controller images contain all
/// success- and failure-dependent Vim state; callers can inspect only the
/// typed preconditions and effects and must ask the coordinator to publish it.
pub struct CommandPlan {
    document: DocumentId,
    revision: Revision,
    model: Option<CommandModelRequest>,
    success_controller: Box<CommandInterpreter>,
    failure_controller: Box<CommandInterpreter>,
    post_commit: PlannedPostCommit,
    output: CommandOutput,
    replay: Option<ReplayPlan>,
    undo_group: UndoGroupDirective,
    presentation: Vec<CommandPresentationRequest>,
}

impl std::fmt::Debug for CommandPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandPlan")
            .field("document", &self.document)
            .field("revision", &self.revision)
            .field("model", &self.model)
            .field("output", &self.output)
            .field("undo_group", &self.undo_group)
            .field("presentation", &self.presentation)
            .finish_non_exhaustive()
    }
}

impl CommandPlan {
    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn model_request(&self) -> Option<&CommandModelRequest> {
        self.model.as_ref()
    }

    pub fn undo_group_directive(&self) -> UndoGroupDirective {
        self.undo_group
    }

    pub fn presentation_requests(&self) -> &[CommandPresentationRequest] {
        &self.presentation
    }
}

#[derive(Clone, Debug)]
enum PlannedPostCommit {
    None,
    ReplaceJournal(Vec<ReplaceJournalEntry>),
    NormalizeNormalCursor,
    NormalizeTypingCursor,
}

/// Exact, short-lived layout state supplied by the core coordinator for one
/// input event. Commands may update `viewport`; the coordinator publishes the
/// resulting scroll position after interpretation completes.
#[derive(Debug)]
pub struct LayoutCommandContext<'a> {
    snapshot: &'a LayoutSnapshot,
    wrap: bool,
    viewport: Viewport,
}

impl<'a> LayoutCommandContext<'a> {
    pub fn new(snapshot: &'a LayoutSnapshot, wrap: bool, viewport: Viewport) -> Self {
        Self {
            snapshot,
            wrap,
            viewport,
        }
    }

    pub fn snapshot(&self) -> &'a LayoutSnapshot {
        self.snapshot
    }

    pub fn wrap(&self) -> bool {
        self.wrap
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }
}

impl CommandOutput {
    fn complete() -> Self {
        Self {
            status: CommandStatus::Complete,
            cursor_moved: false,
            document_changed: false,
            mode_changed: false,
            history_navigation: false,
            ex_outcome: None,
            clipboard_writes: Vec::new(),
        }
    }

    fn pending() -> Self {
        Self {
            status: CommandStatus::Pending,
            ..Self::complete()
        }
    }

    pub(crate) fn unsupported(command: impl Into<String>) -> Self {
        Self {
            status: CommandStatus::Unsupported(command.into()),
            ..Self::complete()
        }
    }

    pub(crate) fn count_error(error: CountError) -> Self {
        Self {
            status: CommandStatus::CountError(error),
            ..Self::complete()
        }
    }

    pub(crate) fn merge(&mut self, next: Self) {
        let Self {
            status,
            cursor_moved,
            document_changed,
            mode_changed,
            history_navigation,
            ex_outcome,
            mut clipboard_writes,
        } = next;
        self.cursor_moved |= cursor_moved;
        self.document_changed |= document_changed;
        self.mode_changed |= mode_changed;
        self.history_navigation |= history_navigation;
        self.status = status;
        if let Some(next_outcome) = ex_outcome {
            match self.ex_outcome.as_mut() {
                Some(outcome) => {
                    if let Err(error) = outcome.try_merge(next_outcome) {
                        // A successful prefix may already be authoritative.
                        // Keep it intact and report the aggregation failure
                        // without obscuring the command's changed bit.
                        self.status = CommandStatus::Error(format!(
                            "compound command output could not merge Ex effects: {error}"
                        ));
                    }
                }
                None => self.ex_outcome = Some(next_outcome),
            }
        }
        self.clipboard_writes.append(&mut clipboard_writes);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Reindent,
    ToggleCase,
    Lowercase,
    Uppercase,
    /// `gq`: hard-line reflow leaving the cursor on the last formatted line.
    Format,
    /// `gw`: hard-line reflow restoring the original text location.
    FormatKeepCursor,
}

impl Operator {
    fn doubled_key(self) -> char {
        match self {
            Self::Delete => 'd',
            Self::Change => 'c',
            Self::Yank => 'y',
            Self::Indent => '>',
            Self::Outdent => '<',
            Self::Reindent => '=',
            Self::ToggleCase => '~',
            Self::Lowercase => 'u',
            Self::Uppercase => 'U',
            Self::Format => 'q',
            Self::FormatKeepCursor => 'w',
        }
    }

    /// Operators entered through the `g` prefix, whose doubled forms are
    /// spelled both `gXX` and `gXgX`.
    fn is_g_prefixed(self) -> bool {
        matches!(
            self,
            Self::ToggleCase
                | Self::Lowercase
                | Self::Uppercase
                | Self::Format
                | Self::FormatKeepCursor
        )
    }

    fn is_format(self) -> bool {
        matches!(self, Self::Format | Self::FormatKeepCursor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingOperator {
    operator: Operator,
    operator_count: usize,
    operator_count_explicit: bool,
    motion_count: Option<usize>,
    register: Option<char>,
    g_prefix: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    None,
    G {
        count: usize,
        count_explicit: bool,
        register: Option<char>,
    },
    Z {
        count: usize,
        count_explicit: bool,
    },
    Operator(PendingOperator),
    ReplaceCharacter {
        count: usize,
        register: Option<char>,
    },
    ReplaceVisual,
    ReplaceVisualBlock,
    Find {
        forward: bool,
        till: bool,
        count: usize,
    },
    OperatorFind {
        operator: PendingOperator,
        forward: bool,
        till: bool,
    },
    OperatorMark {
        operator: PendingOperator,
        linewise: bool,
    },
    TextObject {
        operator: PendingOperator,
        scope: TextObjectScope,
    },
    VisualTextObject {
        scope: TextObjectScope,
        count: usize,
    },
    SetMark,
    JumpMark {
        linewise: bool,
    },
    MacroRecord,
    MacroPlay {
        count: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FindState {
    needle: String,
    forward: bool,
    till: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VisualMemory {
    mode: Mode,
    anchor: usize,
    active: usize,
    to_line_end: bool,
    block: Option<BlockVisualMemory>,
}

/// Revision-stable row identities and display-space edges for an active
/// Visual Block. `BlockSelection` remains the exact, disposable realization
/// for one layout snapshot; these anchors are what survive source edits and
/// reflow.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ActiveVisualBlock {
    anchor: TextAnchor,
    active: TextAnchor,
    anchor_x: f32,
    active_x: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualBlockEndpoint {
    Anchor,
    Active,
}

/// A persistent Visual Block is cancelled only when its logical endpoint can
/// no longer be resolved. The diagnostic is retained on the controller so a
/// frontend can distinguish cancellation from an ordinary mode transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisualBlockRebindError {
    WrongDocument {
        endpoint: VisualBlockEndpoint,
    },
    WrongRevision {
        endpoint: VisualBlockEndpoint,
        expected: Revision,
        actual: Revision,
    },
    AmbiguousAnchor {
        endpoint: VisualBlockEndpoint,
        candidate_count: usize,
    },
    UnresolvableAnchor {
        endpoint: VisualBlockEndpoint,
        reason: UnresolvableAnchor,
    },
    EndpointNotInLayout {
        endpoint: VisualBlockEndpoint,
        offset: usize,
        affinity: BoundaryAffinity,
    },
    InvalidSelection(VisualBlockError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VisualBlockRebindStatus {
    Inactive,
    AwaitingLayout,
    Rebound,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BlockVisualMemory {
    anchor_affinity: BoundaryAffinity,
    active_affinity: BoundaryAffinity,
    anchor_x: f32,
    active_x: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SearchDirection {
    Forward,
    Backward,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CommandLineBuffer {
    input: String,
    selection_anchor: Option<usize>,
    cursor: usize,
    history_index: Option<usize>,
    draft: String,
    completion: Option<command_line_completion::FilenameCompletion>,
}

impl CommandLineBuffer {
    fn delete_selection(&mut self) -> bool {
        let Some(anchor) = self.selection_anchor.take() else {
            return false;
        };
        let range = anchor.min(self.cursor)..anchor.max(self.cursor);
        self.input.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.detach_from_history();
        !range.is_empty()
    }
    fn insert(&mut self, text: &str) {
        if self.accept_completion_input(self.cursor..self.cursor, text) {
            return;
        }
        self.delete_selection();
        self.input.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.detach_from_history();
    }

    fn set(&mut self, text: String) {
        self.completion = None;
        self.selection_anchor = None;
        self.input = text;
        self.cursor = self.input.len();
    }

    fn detach_from_history(&mut self) {
        self.history_index = None;
        self.draft = self.input.clone();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandLineState {
    kind: CommandLineKind,
    buffer: CommandLineBuffer,
    return_mode: Mode,
    count: usize,
    operator: Option<PendingOperator>,
    visual_range_revision: Option<Revision>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertPlacement {
    Before,
    After,
    LineStart,
    LineEnd,
    OpenBelow,
    OpenAbove,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplaceJournalEntry {
    /// Byte range start in the current formatted snapshot while this entry is
    /// at the journal frontier. Earlier entries may have different UTF-8
    /// lengths than the graphemes they replaced, so this cannot be inferred
    /// from an original column.
    start: usize,
    inserted: String,
    original: Option<String>,
    source_record: Option<crate::document::RecordedReplacement>,
}

impl ReplaceJournalEntry {
    fn frontier(&self) -> Option<usize> {
        self.source_record
            .as_ref()
            .map(|record| record.after_cursor)
            .or_else(|| self.start.checked_add(self.inserted.len()))
    }
}

/// Snapshot-independent redo program for one uninterrupted Insert or Replace
/// session. Adjacent text events are coalesced while semantic editing keys
/// remain explicit so replay resolves them against its destination text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct EditSessionProgram {
    steps: Vec<EditSessionStep>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditSessionStep {
    AssistedText(String),
    TypingStyle(typing_style::TypingStyle),
    Text(RegisterValue),
    ListEnter,
    HardBreak,
    ListIndent { unindent: bool },
    Backspace,
    Delete,
    DeleteWord,
    DeleteToLineStart,
}

impl EditSessionProgram {
    fn append_text(&mut self, value: &RegisterValue) {
        if value.text.is_empty() {
            return;
        }
        if let Some(EditSessionStep::Text(payload)) = self.steps.last_mut() {
            payload.append_inserted_payload(value);
        } else {
            self.steps.push(EditSessionStep::Text(value.clone()));
        }
    }

    fn push(&mut self, step: EditSessionStep) {
        self.steps.push(step);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InsertSession {
    placement: InsertPlacement,
    /// `None` means an unsupported discontinuity made this session
    /// non-repeatable; finishing it publishes an explicit no-op instead of
    /// retaining a stale older dot command.
    repeat_program: Option<EditSessionProgram>,
    /// Actual inserted payload since the last explicit cursor discontinuity.
    /// This is independent of whether the session can be encoded as a dot
    /// repeat recipe and becomes Vim's read-only `.` register on completion.
    last_inserted: RegisterValue,
    entry_count: usize,
    /// Internal count/dot replay must update Replace restoration state without
    /// recursively appending steps or expanding the `.` register payload.
    replaying_program: bool,
    /// A successful Normal change executed through Insert/Replace Ctrl-O owns
    /// dot until this session records another edit fragment.
    preserve_normal_repeat: bool,
    unit_floor: usize,
    repeat_operator: Option<OperatorRepeat>,
    repeat_visual_operator: Option<VisualOperatorRepeat>,
    /// Character-at-a-time undo for the uninterrupted Replace-mode frontier.
    /// Operations that make these snapshot-local offsets ambiguous clear the
    /// journal explicitly rather than trying to reconstruct overwritten text
    /// from the current document.
    replace_journal: Vec<ReplaceJournalEntry>,
}

impl InsertSession {
    /// Record user input once, independently of model preparation or commit.
    /// Count/dot replay updates restoration state but must not grow its own
    /// recipe or the last-insert register, nor take ownership from Ctrl-O.
    fn record_edit(
        &mut self,
        update: impl FnOnce(Option<&mut EditSessionProgram>, &mut RegisterValue),
    ) {
        if !self.replaying_program {
            self.preserve_normal_repeat = false;
            update(self.repeat_program.as_mut(), &mut self.last_inserted);
        }
    }

    fn record_step(&mut self, step: EditSessionStep) {
        self.record_edit(|program, _| {
            if let Some(program) = program {
                program.push(step);
            }
        });
    }

    fn record_inserted(&mut self, value: &RegisterValue, step: Option<EditSessionStep>) {
        self.record_inserted_intent(value, value, step);
    }

    fn record_inserted_intent(
        &mut self,
        value: &RegisterValue,
        intent: &RegisterValue,
        step: Option<EditSessionStep>,
    ) {
        self.record_edit(|program, inserted| {
            if let Some(program) = program {
                if let Some(step) = step {
                    program.push(step);
                } else {
                    program.append_text(intent);
                }
            }
            inserted.append_inserted_payload(value);
        });
    }

    fn record_deleted(&mut self, format: crate::document::Format, removed: &str, step: EditSessionStep) {
        self.record_edit(|program, inserted| {
            if let Some(program) = program {
                program.push(step);
            }
            remove_typing_inserted_suffix(format, inserted, removed);
        });
    }
}

/// The deferred edit being collected after a Visual Block insert command.
///
/// Unlike ordinary Insert mode, the source is not changed while the payload is
/// typed. Escape commits every row through one `Document::apply_edits` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualBlockInsertKind {
    Insert,
    Append,
    Change,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VisualBlockInsertRow {
    insertion_offset: usize,
    ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VisualBlockInsertSession {
    kind: VisualBlockInsertKind,
    document_id: DocumentId,
    revision: Revision,
    rows: Vec<VisualBlockInsertRow>,
    payload: String,
    count: usize,
    register: Option<char>,
    replaced: Option<RegisterValue>,
    cursor_target: usize,
    repeat_shape: VisualBlockRepeatShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OperatorRepeat {
    operator: Operator,
    target: RepeatTarget,
    count: usize,
    register: Option<char>,
}

/// Snapshot-independent size of a Visual Character or Visual Line selection.
///
/// Vim repeats the size of a completed Visual operation from the current
/// cursor. A multiline character selection is not a flat character count: it
/// retains its line count and the selected width on its final line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisualRepeatShape {
    CharacterSingleLine {
        grapheme_count: usize,
        to_line_end: bool,
    },
    CharacterMultiLine {
        hard_line_count: usize,
        last_line_grapheme_count: usize,
        to_line_end: bool,
    },
    Line {
        hard_line_count: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VisualOperatorRepeat {
    operator: Operator,
    shape: VisualRepeatShape,
    application_count: usize,
    register: Option<char>,
}

/// Snapshot-independent display geometry for a repeated Visual Block change.
///
/// The endpoint-caret span is stored as finite IEEE bits so repeat recipes
/// remain exactly comparable without retaining an old layout snapshot. At
/// replay time both vertical endpoints and every row edge are hit-tested again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VisualBlockRepeatShape {
    visual_row_count: usize,
    endpoint_x_span_bits: u32,
    to_line_end: bool,
}

impl VisualBlockRepeatShape {
    fn endpoint_x_span(self) -> f32 {
        f32::from_bits(self.endpoint_x_span_bits)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum VisualBlockRepeatAction {
    Operator {
        operator: Operator,
        register: Option<char>,
    },
    Replace {
        replacement: RegisterValue,
    },
    Shift {
        operator: Operator,
        application_count: usize,
    },
    Join {
        insert_space: bool,
    },
    Insert {
        kind: VisualBlockInsertKind,
        payload: String,
        application_count: usize,
        register: Option<char>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VisualBlockRepeat {
    shape: VisualBlockRepeatShape,
    action: VisualBlockRepeatAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OperatorSearch {
    direction: SearchDirection,
    pattern: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OperatorTarget {
    repeat: RepeatTarget,
    count: usize,
    jump_destination: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExNormalTarget {
    pub(crate) anchor: TextAnchor,
    pub(crate) block_id: u64,
}

/// A compound command which the serial core coordinator must replay one input
/// event at a time. Keeping this out of `CommandOutput` prevents an internal
/// continuation from crossing the public controller boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReplayPlan {
    Macro(MacroReplayPlan),
    ExNormal(ExNormalReplayPlan),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MacroReplayPlan {
    pub(crate) register: char,
    pub(crate) iterations: usize,
    pub(crate) events: Vec<InputEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExNormalReplayPlan {
    pub(crate) literal: bool,
    pub(crate) events: Vec<InputEvent>,
    pub(crate) targets: Vec<Option<ExNormalTarget>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandStep {
    pub(crate) output: CommandOutput,
    pub(crate) replay: Option<ReplayPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RepeatTarget {
    ViewLine(line_mode::LineShape, usize),
    Motion(OperatorMotion),
    Lines,
    TextObject(TextObject),
    Find(FindState),
    Mark { name: char, linewise: bool },
    Search(OperatorSearch),
    Characters,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RepeatAction {
    Noop,
    DeleteForward {
        count: usize,
    },
    DeleteBackward {
        count: usize,
    },
    ReplaceCharacter {
        count: usize,
        value: RegisterValue,
    },
    Insert {
        placement: InsertPlacement,
        program: EditSessionProgram,
        count: usize,
    },
    Join {
        count: usize,
        insert_space: bool,
    },
    Operator {
        command: OperatorRepeat,
        edits: Option<EditSessionProgram>,
    },
    VisualOperator {
        command: VisualOperatorRepeat,
        edits: Option<EditSessionProgram>,
    },
    VisualReplace {
        shape: VisualRepeatShape,
        value: RegisterValue,
    },
    VisualJoin {
        shape: VisualRepeatShape,
        insert_space: bool,
    },
    VisualBlock(VisualBlockRepeat),
    Paste {
        before: bool,
        follow: bool,
        count: usize,
        register: char,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MotionKind {
    Characterwise,
    Linewise,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MotionExtent {
    range: Range<usize>,
    kind: MotionKind,
}

/// Buffer/session-owned Vim state temporarily installed into the active view
/// interpreter for one serial command dispatch.
///
/// This is a compatibility bridge while parsing and execution still live in
/// one `CommandInterpreter`: presentation and pending grammar remain per-view,
/// while registers, marks, histories, and repeat recipes have one
/// authoritative owner in the core buffer.
#[derive(Clone, Debug)]
pub(crate) struct BufferCommandState {
    registers: Registers,
    marks: BTreeMap<char, usize>,
    search_history: Vec<String>,
    ex_history: Vec<String>,
    ex_state: ExExecutionState,
    fileformats: Vec<FileFormat>,
    search_options: regex_v1::SearchOptions,
    last_search: Option<(SearchDirection, String)>,
    last_repeat: Option<RepeatAction>,
    recording: Option<(char, Vec<InputEvent>)>,
    last_macro: Option<char>,
    /// Buffer-owned `textwidth`: inherited default plus optional override.
    pub(crate) text_width: crate::document::TextWidthSetting,
}

/// Per-view Vim controller state.
#[derive(Clone, Debug)]
pub struct CommandInterpreter {
    mode: Mode,
    cursor: usize,
    position_revision: Option<Revision>,
    boundary_affinity: BoundaryAffinity,
    visual_position: Option<VisualPosition>,
    desired_x: Option<f32>,
    preferred_column: Option<usize>,
    visual_anchor: Option<usize>,
    /// Whether hard-line `$` established the moving edge of the active
    /// character/line Visual selection. Vim carries this sentinel through
    /// vertical extension and dot repeats it to the target line end.
    visual_to_line_end: bool,
    visual_block: Option<BlockSelection>,
    active_visual_block: Option<ActiveVisualBlock>,
    visual_block_rebind_error: Option<VisualBlockRebindError>,
    /// Vim's window-local CTRL-D/CTRL-U row amount. `None` means derive half
    /// of the currently visible visual rows; an explicit count replaces it.
    half_page_scroll_rows: Option<usize>,
    count: Option<usize>,
    /// Discards the remainder of one overflowing decimal token so its tail
    /// cannot accidentally become a second, executable count.
    count_overflowed: bool,
    register_pending: bool,
    requested_register: Option<char>,
    clipboard_copy_as_seen: bool,
    pending: Pending,
    registers: Registers,
    command_line_state: Option<CommandLineState>,
    search_history: Vec<String>,
    ex_history: Vec<String>,
    ex_state: ExExecutionState,
    wrap: bool,
    line_mode: LineMode,
    line_layout: Option<LayoutSnapshot>,
    physical_cursor: Option<line_mode::PhysicalCursor>,
    visual_source_anchor: Option<crate::document::SourcePoint>,
    fileformats: Vec<FileFormat>,
    search_options: regex_v1::SearchOptions,
    last_search: Option<(SearchDirection, String)>,
    last_repeat: Option<RepeatAction>,
    insert_session: Option<InsertSession>,
    typing_style: typing_style::TypingStyle,
    input_assistance: input_assistance::InputAssistance,
    visual_block_insert: Option<VisualBlockInsertSession>,
    replaying: bool,
    last_find: Option<FindState>,
    marks: BTreeMap<char, usize>,
    jumps: Vec<usize>,
    jump_index: usize,
    last_visual: Option<VisualMemory>,
    recording: Option<(char, Vec<InputEvent>)>,
    last_macro: Option<char>,
    compound_replay_depth: usize,
    /// Core-facing dispatch asks compound commands to emit a replay plan.
    /// Direct `CommandInterpreter` users retain the legacy headless executor
    /// while the coordinator remains the product execution boundary.
    plan_compound_replay: bool,
    pending_replay: Option<ReplayPlan>,
    insert_normal_once: Option<Mode>,
    ctrl_o_just_started: bool,
    /// Set for the publication turn only when Ctrl-O completion reopens the
    /// surrounding edit group after its Normal command has finished.
    reopened_group_after_insert_normal_once: bool,
    /// One-turn external register inputs. Never exported as buffer state.
    clipboard_context: ClipboardCommandContext,
    /// Effects accumulated by low-level register policy until the enclosing
    /// command output is published or rolled back.
    pending_clipboard_writes: Vec<ClipboardWriteRequest>,
    /// Buffer-owned `textwidth`, bridged through the buffer command state.
    text_width: crate::document::TextWidthSetting,
    /// Canonical language selecting the reflow comment profile.
    reflow_language: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CommandPositionAnchors {
    revision: Revision,
    cursor: TextAnchor,
    visual_anchor: Option<TextAnchor>,
    /// True when the invoking active Visual Character selection occupied one
    /// hard line. Vim retains the old byte columns for `gv` after a same-line
    /// destructive operator (clamped against the result), while cross-line
    /// and linewise endpoints follow the edit map and commonly collapse.
    active_visual_was_single_hard_line: bool,
    marks: BTreeMap<char, TextAnchor>,
    jumps: Vec<TextAnchor>,
    last_visual: Option<(TextAnchor, TextAnchor)>,
    /// Logical endpoints of the active revision-bound Visual Block. If an
    /// invoking edit remembers that block while leaving Visual mode, these
    /// are the source-revision anchors used to publish the new memory.
    visual_block_memory: Option<(TextAnchor, TextAnchor)>,
    /// Persistent row identities for an active Visual Block. These may differ
    /// from its current hit-tested corner carets after a reflow.
    active_visual_block: Option<(TextAnchor, TextAnchor)>,
    insert_unit_floor: Option<TextAnchor>,
}

impl CommandPositionAnchors {
    pub(crate) fn revision(&self) -> Revision {
        self.revision
    }

    pub(crate) fn history_snapshot(&self) -> HistoryRestorationSnapshot {
        HistoryRestorationSnapshot::new(self.cursor, self.marks.clone())
    }
}

impl Default for CommandInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandInterpreter {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            cursor: 0,
            position_revision: None,
            boundary_affinity: BoundaryAffinity::Downstream,
            visual_position: None,
            desired_x: None,
            preferred_column: None,
            visual_anchor: None,
            visual_to_line_end: false,
            visual_block: None,
            active_visual_block: None,
            visual_block_rebind_error: None,
            half_page_scroll_rows: None,
            count: None,
            count_overflowed: false,
            register_pending: false,
            requested_register: None,
            clipboard_copy_as_seen: false,
            pending: Pending::None,
            registers: Registers::default(),
            command_line_state: None,
            search_history: Vec::new(),
            ex_history: Vec::new(),
            ex_state: ExExecutionState::default(),
            wrap: false,
            line_mode: LineMode::Visual,
            line_layout: None,
            physical_cursor: None,
            visual_source_anchor: None,
            fileformats: vec![FileFormat::Unix, FileFormat::Dos],
            search_options: regex_v1::SearchOptions::default(),
            last_search: None,
            last_repeat: None,
            insert_session: None,
            typing_style: Default::default(),
            input_assistance: Default::default(),
            visual_block_insert: None,
            replaying: false,
            last_find: None,
            marks: BTreeMap::new(),
            jumps: Vec::new(),
            jump_index: 0,
            last_visual: None,
            recording: None,
            last_macro: None,
            compound_replay_depth: 0,
            plan_compound_replay: false,
            pending_replay: None,
            insert_normal_once: None,
            ctrl_o_just_started: false,
            reopened_group_after_insert_normal_once: false,
            clipboard_context: ClipboardCommandContext::default(),
            pending_clipboard_writes: Vec::new(),
            text_width: crate::document::TextWidthSetting::default(),
            reflow_language: None,
        }
    }

    pub(crate) fn export_buffer_state(&self) -> BufferCommandState {
        BufferCommandState {
            registers: self.registers.clone(),
            marks: self
                .marks
                .iter()
                .filter(|(name, _)| name.is_ascii_lowercase())
                .map(|(name, offset)| (*name, *offset))
                .collect(),
            search_history: self.search_history.clone(),
            ex_history: self.ex_history.clone(),
            ex_state: self.ex_state.clone(),
            fileformats: self.fileformats.clone(),
            search_options: self.search_options,
            last_search: self.last_search.clone(),
            last_repeat: self.last_repeat.clone(),
            recording: self.recording.clone(),
            last_macro: self.last_macro,
            text_width: self.text_width,
        }
    }

    pub(crate) fn install_buffer_state(&mut self, state: &BufferCommandState) {
        self.registers.clone_from(&state.registers);
        self.marks.clone_from(&state.marks);
        self.search_history.clone_from(&state.search_history);
        self.ex_history.clone_from(&state.ex_history);
        self.ex_state.clone_from(&state.ex_state);
        self.fileformats.clone_from(&state.fileformats);
        self.search_options = state.search_options;
        self.last_search.clone_from(&state.last_search);
        self.last_repeat.clone_from(&state.last_repeat);
        self.recording.clone_from(&state.recording);
        self.last_macro = state.last_macro;
        self.text_width = state.text_width;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Install a literal string as the buffer's forward search target without
    /// moving the cursor or entering command-line mode. Native "Use Selection
    /// for Find" uses this after the coordinator has validated the exact
    /// core-owned Visual selection. Escaping here keeps arbitrary prose (and
    /// regex metacharacters in it) literal while `n`/`N` continue through the
    /// ordinary Vim search engine.
    pub(crate) fn set_literal_search_pattern(&mut self, literal: &str) -> bool {
        if literal.is_empty() {
            return false;
        }
        self.last_search = Some((SearchDirection::Forward, regex_v1::escape_literal(literal)));
        true
    }

    pub fn position_revision(&self) -> Option<Revision> {
        self.position_revision
    }

    pub(crate) fn capture_position_anchors(
        &self,
        document: &Document,
    ) -> Result<CommandPositionAnchors, DocumentError> {
        if let Some(revision) = self.position_revision {
            if revision != document.revision() {
                return Err(DocumentError::WrongSnapshot {
                    expected: document.revision(),
                    actual: revision,
                });
            }
        }
        let make = |offset, association, affinity| {
            let point = document.text_point(offset)?;
            document
                .text_anchor(
                    point,
                    association,
                    affinity,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )
                .map_err(command_position_document_error)
        };
        let (cursor_association, visual_anchor_association) = self
            .visual_anchor
            .map(|anchor| {
                if anchor <= self.cursor {
                    (Association::BeforeInsertion, Association::AfterInsertion)
                } else {
                    (Association::AfterInsertion, Association::BeforeInsertion)
                }
            })
            .unwrap_or((Association::AfterInsertion, Association::AfterInsertion));
        let active_visual_was_single_hard_line = self.mode == Mode::VisualCharacter
            && self.visual_anchor.is_some_and(|anchor| {
                let lines = document.hard_line_snapshot();
                lines.line_at_offset(anchor).map(|line| line.index())
                    == lines.line_at_offset(self.cursor).map(|line| line.index())
            });
        let marks = self
            .marks
            .iter()
            .map(|(name, offset)| {
                make(
                    *offset,
                    Association::AfterInsertion,
                    BoundaryAffinity::Downstream,
                )
                .map(|a| (*name, a))
            })
            .collect::<Result<_, _>>()?;
        let jumps = self
            .jumps
            .iter()
            .map(|offset| {
                make(
                    *offset,
                    Association::AfterInsertion,
                    BoundaryAffinity::Downstream,
                )
            })
            .collect::<Result<_, _>>()?;
        let last_visual = self
            .last_visual
            .map(|memory| {
                let (anchor_association, active_association) = if memory.anchor <= memory.active {
                    (Association::AfterInsertion, Association::BeforeInsertion)
                } else {
                    (Association::BeforeInsertion, Association::AfterInsertion)
                };
                Ok((
                    make(
                        memory.anchor,
                        anchor_association,
                        BoundaryAffinity::Downstream,
                    )?,
                    make(
                        memory.active,
                        active_association,
                        BoundaryAffinity::Downstream,
                    )?,
                ))
            })
            .transpose()?;
        let active_visual_block = if let Some(block) = self.active_visual_block {
            for anchor in [block.anchor, block.active] {
                if anchor.document() != document.id() {
                    return Err(DocumentError::WrongDocument);
                }
                if anchor.revision() != document.revision() {
                    return Err(DocumentError::WrongSnapshot {
                        expected: document.revision(),
                        actual: anchor.revision(),
                    });
                }
                document.text_point(anchor.offset())?;
            }
            Some((block.anchor, block.active))
        } else {
            None
        };
        let visual_block_memory = self
            .visual_block_memory()
            .map(|memory| {
                let block = memory
                    .block
                    .expect("Visual Block memory carries its visual geometry");
                let (anchor_association, active_association) = if memory.anchor <= memory.active {
                    (Association::AfterInsertion, Association::BeforeInsertion)
                } else {
                    (Association::BeforeInsertion, Association::AfterInsertion)
                };
                Ok((
                    make(memory.anchor, anchor_association, block.anchor_affinity)?,
                    make(memory.active, active_association, block.active_affinity)?,
                ))
            })
            .transpose()?;
        Ok(CommandPositionAnchors {
            revision: document.revision(),
            cursor: make(self.cursor, cursor_association, self.boundary_affinity)?,
            visual_anchor: self
                .visual_anchor
                .map(|offset| {
                    make(
                        offset,
                        visual_anchor_association,
                        BoundaryAffinity::Downstream,
                    )
                })
                .transpose()?,
            active_visual_was_single_hard_line,
            marks,
            jumps,
            last_visual,
            visual_block_memory,
            active_visual_block,
            insert_unit_floor: self
                .insert_session
                .as_ref()
                .map(|session| {
                    make(
                        session.unit_floor,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                    )
                })
                .transpose()?,
        })
    }

    /// Capture a format-change caret with the text on its chosen side. A
    /// conversion exposes or hides wrappers; it does not type those wrappers
    /// at the caret. An upstream insertion caret therefore stays before new
    /// closing syntax. Ordinary editing keeps its AfterInsertion association.
    pub(crate) fn capture_format_position_anchors(
        &self,
        document: &Document,
    ) -> Result<CommandPositionAnchors, DocumentError> {
        let mut anchors = self.capture_position_anchors(document)?;
        if matches!(self.mode, Mode::Insert | Mode::Replace)
            && self.boundary_affinity == BoundaryAffinity::Upstream
        {
            anchors.cursor = document
                .text_anchor(
                    document.text_point(self.cursor)?,
                    Association::BeforeInsertion,
                    BoundaryAffinity::Upstream,
                    DeletionRecovery::PreferPrecedingThenFollowing,
                )
                .map_err(command_position_document_error)?;
        }
        Ok(anchors)
    }

    /// Install a prepared set of anchors through one checked revision map.
    /// Every result is resolved before any controller field is published.
    pub(crate) fn apply_position_map(
        &mut self,
        anchors: &CommandPositionAnchors,
        map: &PositionMap,
    ) -> Result<bool, PositionError> {
        if anchors.revision != map.source_revision()
            || self
                .position_revision
                .is_some_and(|revision| revision != anchors.revision)
        {
            return Ok(false);
        }
        let mut next = self.clone();
        let Some(cursor) = mapped_anchor(map, anchors.cursor)? else {
            return Ok(false);
        };
        next.cursor = cursor;
        next.visual_anchor = match anchors.visual_anchor {
            Some(anchor) => {
                let Some(offset) = mapped_anchor(map, anchor)? else {
                    return Ok(false);
                };
                Some(offset)
            }
            None => None,
        };
        let mut marks = BTreeMap::new();
        for (name, anchor) in &anchors.marks {
            let Some(offset) = mapped_anchor(map, *anchor)? else {
                return Ok(false);
            };
            marks.insert(*name, offset);
        }
        next.marks = marks;
        let mut jumps = Vec::with_capacity(anchors.jumps.len());
        for anchor in &anchors.jumps {
            let Some(offset) = mapped_anchor(map, *anchor)? else {
                return Ok(false);
            };
            jumps.push(offset);
        }
        next.jumps = jumps;
        next.last_visual = match anchors.last_visual {
            Some((anchor, active)) => {
                let (Some(anchor), Some(active)) =
                    (mapped_anchor(map, anchor)?, mapped_anchor(map, active)?)
                else {
                    return Ok(false);
                };
                next.last_visual.map(|mut memory| {
                    memory.anchor = anchor;
                    memory.active = active;
                    memory
                })
            }
            None => None,
        };
        if let (Some(session), Some(anchor)) =
            (next.insert_session.as_mut(), anchors.insert_unit_floor)
        {
            let Some(offset) = mapped_anchor(map, anchor)? else {
                return Ok(false);
            };
            session.unit_floor = offset;
        }
        if next.mode == Mode::VisualBlock {
            let geometry = next.visual_block_memory().and_then(|memory| memory.block);
            match (anchors.active_visual_block, geometry) {
                (Some((anchor, active)), Some(geometry)) => {
                    let anchor = map_visual_block_anchor(map, VisualBlockEndpoint::Anchor, anchor)?;
                    let active = map_visual_block_anchor(map, VisualBlockEndpoint::Active, active)?;
                    match (anchor, active) {
                        (Ok(anchor), Ok(active)) => {
                            next.active_visual_block = Some(ActiveVisualBlock {
                                anchor,
                                active,
                                anchor_x: geometry.anchor_x,
                                active_x: geometry.active_x,
                            });
                            // Exact caret identities belong to the superseded
                            // layout. The coordinator reconstructs them after
                            // installing this view's target-revision layout.
                            next.visual_block = None;
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            next.cancel_visual_block_rebind(error);
                        }
                    }
                }
                _ => next.cancel_visual_block_rebind(VisualBlockRebindError::EndpointNotInLayout {
                    endpoint: VisualBlockEndpoint::Anchor,
                    offset: next.cursor,
                    affinity: next.boundary_affinity,
                }),
            }
        }
        // A deferred block edit stores a whole revision-bound row plan, not
        // merely one anchor. An edit from another view invalidates that plan;
        // cancel it rather than trying to rebase only its endpoints.
        if next.visual_block_insert.is_some() {
            next.visual_block_insert = None;
            next.insert_session = None;
            next.mode = Mode::Normal;
            next.register_pending = false;
        }
        next.visual_position = None;
        next.desired_x = None;
        // Line undo is buffer-owned and source-backed. The invoking command
        // publishes its validated post-change slot to every view after this
        // position-map preparation, so an inactive view must not discard the
        // shared pre-publication value here.
        next.typing_style = Default::default();
        next.input_assistance.clear_tag();
        next.position_revision = Some(map.target_revision());
        *self = next;
        Ok(true)
    }

    /// Rebase persistent positions which an invoking command did not itself
    /// change while that command committed a document revision.
    ///
    /// Applying [`Self::apply_position_map`] to the invoking controller would
    /// be incorrect: it would replace the cursor and active selection chosen
    /// by the command with their pre-command locations. Marks, jump entries,
    /// and remembered Visual endpoints are different. They denote persistent
    /// document locations and still need to follow the committed edit. This
    /// method maps only values which remain byte-for-byte equal to their
    /// pre-command values. Newly assigned values are already expressed in the
    /// target revision and are left alone.
    ///
    /// Every selected anchor is resolved into a clone before anything is
    /// installed, so failure cannot partially rebase controller state.
    pub(crate) fn rebase_unchanged_persistent_positions(
        &mut self,
        document: &Document,
        before: &Self,
        anchors: &CommandPositionAnchors,
        map: &PositionMap,
        cursor_was_moved: bool,
    ) -> Result<bool, PositionError> {
        if anchors.revision != map.source_revision()
            || before
                .position_revision
                .is_some_and(|revision| revision != anchors.revision)
            || self
                .position_revision
                .is_some_and(|revision| revision != map.target_revision())
        {
            return Ok(false);
        }

        let mut next = self.clone();

        if !cursor_was_moved && self.cursor == before.cursor {
            let Some(mapped) = mapped_anchor(map, anchors.cursor)? else {
                return Ok(false);
            };
            next.cursor = mapped;
            next.boundary_affinity = anchors.cursor.affinity();
        }

        if let (Some(previous), Some(current), Some(anchor)) = (
            before.insert_session.as_ref(),
            next.insert_session.as_mut(),
            anchors.insert_unit_floor,
        ) {
            if current.unit_floor == previous.unit_floor
                && document.text_point(current.unit_floor).is_err()
            {
                let Some(mapped) = mapped_anchor(map, anchor)? else {
                    return Ok(false);
                };
                current.unit_floor = mapped;
            }
        }

        if self.visual_anchor == before.visual_anchor {
            next.visual_anchor = match anchors.visual_anchor {
                Some(anchor) => {
                    let Some(mapped) = mapped_anchor(map, anchor)? else {
                        return Ok(false);
                    };
                    Some(mapped)
                }
                None => None,
            };
        }

        for (name, anchor) in &anchors.marks {
            let Some(old_offset) = before.marks.get(name) else {
                continue;
            };
            if self.marks.get(name) != Some(old_offset) {
                continue;
            }
            let Some(mapped) = mapped_anchor(map, *anchor)? else {
                return Ok(false);
            };
            next.marks.insert(*name, mapped);
        }

        // Jump-list operations only truncate a suffix, append entries, or
        // move the current index. Thus the equal prefix is exactly the set of
        // pre-command entries that survived this command unchanged.
        let unchanged_jump_prefix = before
            .jumps
            .iter()
            .zip(&self.jumps)
            .take_while(|(old, current)| old == current)
            .count();
        for (index, anchor) in anchors.jumps.iter().take(unchanged_jump_prefix).enumerate() {
            let Some(mapped) = mapped_anchor(map, *anchor)? else {
                return Ok(false);
            };
            next.jumps[index] = mapped;
        }

        if self.last_visual == before.last_visual {
            next.last_visual = match anchors.last_visual {
                Some((anchor, active)) => {
                    let (Some(anchor), Some(active)) =
                        (mapped_anchor(map, anchor)?, mapped_anchor(map, active)?)
                    else {
                        return Ok(false);
                    };
                    self.last_visual.map(|mut memory| {
                        memory.anchor = anchor;
                        memory.active = active;
                        memory
                    })
                }
                None => None,
            };
        } else if matches!(before.mode, Mode::VisualCharacter | Mode::VisualLine)
            && before.visual_anchor.is_some()
            && self.last_visual
                == before.visual_anchor.map(|anchor| VisualMemory {
                    mode: before.mode,
                    anchor,
                    active: before.cursor,
                    to_line_end: before.visual_to_line_end,
                    block: None,
                })
        {
            // A successful linear Visual operator remembers the selection it
            // just consumed for `gv`. That memory is newly installed command
            // state, but its endpoints still name the source revision: map
            // the pre-command active selection anchors before publishing the
            // target-revision controller. Without this branch a deletion can
            // leave `last_visual` beyond the shortened document and make the
            // otherwise successful Core turn fail while capturing history.
            if let (Some(mut memory), Some(anchor)) = (self.last_visual, anchors.visual_anchor) {
                if anchors.active_visual_was_single_hard_line
                    && before.mode == Mode::VisualCharacter
                {
                    // Vim's same-line delete/change marks retain their old
                    // columns, so `gv` reselects as much of that directed
                    // shape as remains. This is intentionally not the anchor
                    // map behavior used for cross-line selections, whose
                    // deleted endpoints collapse to the surviving boundary.
                    let lines = document.hard_line_snapshot();
                    memory.anchor = normalize_normal_cursor_document(document, &lines,
                        memory.anchor.min(document.projection().text_tree().byte_len()),
                    );
                    memory.active = normalize_normal_cursor_document(document, &lines,
                        memory.active.min(document.projection().text_tree().byte_len()),
                    );
                } else {
                    let (Some(mapped_anchor), Some(active)) = (
                        mapped_anchor(map, anchor)?,
                        mapped_anchor(map, anchors.cursor)?,
                    ) else {
                        return Ok(false);
                    };
                    memory.anchor = mapped_anchor;
                    memory.active = active;
                }
                next.last_visual = Some(memory);
            }
        } else if self.last_visual == before.visual_block_memory() {
            if let (Some(mut memory), Some((anchor, active))) =
                (self.last_visual, anchors.visual_block_memory)
            {
                let (Some(anchor), Some(active)) =
                    (mapped_anchor(map, anchor)?, mapped_anchor(map, active)?)
                else {
                    return Ok(false);
                };
                memory.anchor = anchor;
                memory.active = active;
                next.last_visual = Some(memory);
            }
        }

        if self.mode == Mode::VisualBlock && self.active_visual_block == before.active_visual_block
        {
            match (anchors.active_visual_block, self.active_visual_block) {
                (Some((anchor, active)), Some(block)) => {
                    let anchor = map_visual_block_anchor(map, VisualBlockEndpoint::Anchor, anchor)?;
                    let active = map_visual_block_anchor(map, VisualBlockEndpoint::Active, active)?;
                    match (anchor, active) {
                        (Ok(anchor), Ok(active)) => {
                            next.active_visual_block = Some(ActiveVisualBlock {
                                anchor,
                                active,
                                anchor_x: block.anchor_x,
                                active_x: block.active_x,
                            });
                            next.visual_block = None;
                            next.visual_position = None;
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            next.cancel_visual_block_rebind(error);
                        }
                    }
                }
                _ => next.cancel_visual_block_rebind(VisualBlockRebindError::EndpointNotInLayout {
                    endpoint: VisualBlockEndpoint::Anchor,
                    offset: next.cursor,
                    affinity: next.boundary_affinity,
                }),
            }
        }

        *self = next;
        Ok(true)
    }

    pub(crate) fn note_document_revision(&mut self, revision: Revision) {
        self.position_revision = Some(revision);
    }

    pub(crate) fn begin_replay_frame(&mut self) -> bool {
        if self.compound_replay_depth >= COMPOUND_REPLAY_LIMIT {
            return false;
        }
        self.compound_replay_depth += 1;
        true
    }

    pub(crate) fn end_replay_frame(&mut self) {
        debug_assert!(self.compound_replay_depth > 0);
        self.compound_replay_depth = self.compound_replay_depth.saturating_sub(1);
    }

    pub(crate) fn finish_deferred_insert_normal_once(
        &mut self,
        document: &mut Document,
        output: &mut CommandOutput,
    ) {
        debug_assert_eq!(self.compound_replay_depth, 0);
        self.finish_insert_normal_once(document, output);
    }

    pub(crate) fn replay_needs_abort(&self) -> bool {
        self.mode != Mode::Normal
            || self.pending != Pending::None
            || self.register_pending
            || self.command_line_state.is_some()
            || self.visual_anchor.is_some()
            || self.visual_block.is_some()
            || self.visual_block_insert.is_some()
    }

    pub(crate) fn reopened_group_after_insert_normal_once(&self) -> bool {
        self.reopened_group_after_insert_normal_once
    }

    pub(crate) fn rejects_ex_normal_macro_recording(&self, event: &InputEvent) -> bool {
        self.mode == Mode::Normal
            && self.pending == Pending::None
            && !self.register_pending
            && *event == InputEvent::Key(Key::Char('q'))
    }

    /// Whether this input must be resolved against exact visual geometry.
    ///
    /// This preflight is deliberately read-only and follows the same state
    /// ordering as [`Self::try_handle_layout_key`]. Multi-key prefixes and
    /// logical commands can therefore advance without acquiring layout; the
    /// completed visual-row, viewport, or block command requests it on the
    /// input that actually consumes geometry.
    pub(crate) fn requires_layout_for_input(
        &self,
        document: &Document,
        event: &InputEvent,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> bool {
        if matches!(event, InputEvent::Key(Key::SelectAll)) {
            return false;
        }
        if self.mode == Mode::CommandLine {
            return matches!(event, InputEvent::Key(Key::Enter))
                && self
                    .command_line_state
                    .as_ref()
                    .is_some_and(|state| state.return_mode == Mode::VisualBlock);
        }
        if self.visual_block_insert.is_some() {
            return false;
        }
        if self.line_mode == LineMode::Visual
            && (self.mode == Mode::VisualLine
                || matches!(event,InputEvent::Key(key) if self.mode_line_key(*key)))
        {
            return true;
        }
        let InputEvent::Key(key) = event else {
            // Layout-aware dispatch has one special text path: a frontend may
            // deliver the replacement grapheme as Text after Visual Block r.
            return self.mode == Mode::VisualBlock && self.pending == Pending::ReplaceVisualBlock;
        };
        if self.mode == Mode::VisualBlock {
            return true;
        }
        if self.register_pending {
            return false;
        }
        if self.mode == Mode::Normal
            && self.count.is_some()
            && matches!(
                *key,
                Key::Char('v' | 'V') | Key::Ctrl('v' | 'V' | 'q' | 'Q')
            )
            && self
                .last_visual
                .is_some_and(|memory| memory.mode == Mode::VisualBlock)
        {
            return true;
        }
        if matches!(*key, Key::Ctrl('v' | 'V' | 'q' | 'Q'))
            && matches!(
                self.mode,
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
            )
        {
            return true;
        }

        if matches!(
            self.mode,
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
        ) {
            if let Pending::Operator(pending) = self.pending {
                if pending.g_prefix && matches!(*key, Key::Char('j' | 'k' | '0' | '^' | '$')) {
                    return true;
                }
                if !pending.g_prefix && matches!(*key, Key::Char('H' | 'M' | 'L')) {
                    return true;
                }
            }
            if let Pending::G { register, .. } = self.pending {
                if self.mode == Mode::Normal
                    && matches!(*key, Key::Char('p' | 'P'))
                    && self.register_is_blockwise_with_context(document, register, clipboard)
                {
                    return true;
                }
                if matches!(*key, Key::Char('j' | 'k' | '0' | '^' | '$')) {
                    return true;
                }
                if *key == Key::Char('v')
                    && self
                        .last_visual
                        .is_some_and(|memory| memory.mode == Mode::VisualBlock)
                {
                    return true;
                }
            }
            if matches!(self.pending, Pending::Z { .. })
                && matches!(*key, Key::Char('t' | 'z' | 'b'))
            {
                return true;
            }
        }
        if self.pending != Pending::None {
            return false;
        }

        match self.mode {
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine => match *key {
                Key::Char('p' | 'P')
                    if self.mode == Mode::Normal
                        && self.register_is_blockwise_with_context(
                            document,
                            self.requested_register,
                            clipboard,
                        ) =>
                {
                    true
                }
                Key::Char('.')
                    if self.mode == Mode::Normal
                        && (self.repeat_is_visual_block_change()
                            || self
                                .repeat_is_blockwise_paste_with_context(document, clipboard)) =>
                {
                    true
                }
                Key::Char('j') | Key::Down | Key::Char('k') | Key::Up => true,
                Key::Char('H' | 'M' | 'L')
                | Key::PageDown
                | Key::PageUp
                | Key::Ctrl(
                    'f' | 'F' | 'b' | 'B' | 'd' | 'D' | 'u' | 'U' | 'e' | 'E' | 'y' | 'Y',
                ) => true,
                _ => false,
            },
            Mode::Insert | Mode::Replace => {
                matches!(*key, Key::Up | Key::Down | Key::PageUp | Key::PageDown)
            }
            Mode::VisualBlock => true,
            Mode::CommandLine => false,
        }
    }

    /// Whether layout acquisition must retain the viewport used by this input.
    /// Caret-relative commands can reveal an unmaterialized caret, but viewport
    /// motions must resolve their target against the viewport before the command.
    /// Visual modes also acquire layout for pending grammar; those prefixes must
    /// not reveal the caret before the eventual command chooses its target.
    pub(crate) fn layout_input_preserves_viewport(&self, event: &InputEvent) -> bool {
        if self.register_pending || self.count_overflowed || self.visual_block_insert.is_some() {
            return true;
        }
        let InputEvent::Key(key) = event else {
            return false;
        };
        if *key == Key::Escape {
            return true;
        }
        match self.mode {
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock => {
                match self.pending {
                    Pending::None => match *key {
                        Key::Char('0') => self.count.is_some(),
                        Key::Char('1'..='9' | '"' | 'g' | 'z' | 'r' | 'f' | 'F' | 't' | 'T')
                        | Key::Char('i' | 'a' | 'm' | '`' | '\'' | 'q' | '@' | '/' | '?' | ':') => true,
                        Key::Char('H' | 'M' | 'L')
                        | Key::PageDown
                        | Key::PageUp
                        | Key::Ctrl(
                            'f' | 'F' | 'b' | 'B' | 'd' | 'D' | 'u' | 'U' | 'e' | 'E' | 'y' | 'Y',
                        ) => true,
                        _ => false,
                    },
                    Pending::Operator(pending) => {
                        !pending.g_prefix && matches!(*key, Key::Char('H' | 'M' | 'L'))
                    }
                    Pending::SetMark | Pending::MacroRecord | Pending::MacroPlay { .. } => true,
                    Pending::G { .. } => {
                        !matches!(
                            *key,
                            Key::Char('g' | 'e' | 'E' | '_' | '*' | '#' | 'v' | '~' | 'u' | 'U' | 'J')
                                | Key::Char('q' | 'w')
                                | Key::Char('j' | 'k' | '0' | '^' | '$')
                        ) && !(self.mode == Mode::Normal && matches!(*key, Key::Char('p' | 'P')))
                    }
                    Pending::Z { .. } => !matches!(*key, Key::Char('t' | 'z' | 'b')),
                    Pending::Find { .. } => !matches!(*key, Key::Char(_)),
                    Pending::VisualTextObject { .. } => {
                        !matches!(*key, Key::Char(key) if TextObjectKind::from_vim_key(key).is_some())
                    }
                    Pending::JumpMark { .. } => !matches!(*key, Key::Char('a'..='z')),
                    _ => false,
                }
            }
            Mode::Insert | Mode::Replace => {
                self.pending == Pending::None && matches!(*key, Key::PageUp | Key::PageDown)
            }
            Mode::CommandLine => false,
        }
    }

    pub(crate) fn set_layout_options(&mut self, wrap: bool) {
        self.wrap = wrap;
    }

    pub(crate) fn capture_history_restoration(
        &self,
        document: &Document,
    ) -> Result<HistoryRestorationSnapshot, DocumentError> {
        Ok(self.capture_position_anchors(document)?.history_snapshot())
    }

    /// Apply history-owned state to the invoking view while deliberately
    /// retaining registers, repeat/search/macro state, jumps, and view options.
    pub(crate) fn apply_history_restoration(
        &mut self,
        document: &Document,
        restoration: &HistoryRestorationSnapshot,
    ) -> Result<(), DocumentError> {
        let validate = |anchor: TextAnchor| -> Result<usize, DocumentError> {
            if anchor.document() != document.id() {
                return Err(DocumentError::WrongDocument);
            }
            if anchor.revision() != document.revision() {
                return Err(DocumentError::WrongSnapshot {
                    expected: document.revision(),
                    actual: anchor.revision(),
                });
            }
            document.text_point(anchor.offset())?;
            Ok(anchor.offset())
        };

        let cursor = validate(restoration.cursor())?;
        let marks = restoration
            .marks()
            .iter()
            .map(|(name, anchor)| validate(*anchor).map(|offset| (*name, offset)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;

        self.typing_style = Default::default();
        self.input_assistance.clear_tag();
        self.mode = Mode::Normal;
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(), cursor);
        self.boundary_affinity = restoration.cursor().affinity();
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.command_line_state = None;
        self.insert_session = None;
        self.visual_block_insert = None;
        self.insert_normal_once = None;
        self.ctrl_o_just_started = false;
        self.marks = marks;
        self.clear_pending();
        self.position_revision = Some(document.revision());
        Ok(())
    }

    pub fn boundary_affinity(&self) -> BoundaryAffinity {
        self.boundary_affinity
    }

    pub fn visual_position(&self) -> Option<VisualPosition> {
        self.visual_position
    }

    pub fn desired_x(&self) -> Option<f32> {
        self.desired_x
    }

    pub fn visual_anchor(&self) -> Option<usize> {
        self.visual_anchor
    }

    pub fn visual_block(&self) -> Option<&BlockSelection> {
        self.visual_block.as_ref()
    }

    /// Resolve the active linear Visual selection to the exact logical
    /// half-open range consumed by operators. Keeping this query beside the
    /// command implementation prevents presentation adapters from separately
    /// approximating Character- and Linewise inclusive endpoint rules.
    pub(crate) fn linear_visual_selection_range(
        &self,
        document: &Document,
    ) -> Option<Range<usize>> {
        matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine)
            .then(|| self.visual_extent(document).range)
    }

    /// Whether the active Visual Block right edge follows each row's semantic
    /// end rather than a fixed x coordinate.
    pub(crate) fn visual_block_to_line_end(&self) -> bool {
        self.mode == Mode::VisualBlock && self.visual_to_line_end
    }

    pub fn visual_block_rebind_error(&self) -> Option<&VisualBlockRebindError> {
        self.visual_block_rebind_error.as_ref()
    }

    pub(crate) fn active_visual_block_endpoint_offsets(&self) -> Option<(usize, usize)> {
        let block = self.active_visual_block?;
        (self.mode == Mode::VisualBlock).then_some((block.anchor.offset(), block.active.offset()))
    }

    /// Rebuild the disposable block geometry from revision-stable logical row
    /// anchors after an exact layout for this view has been installed.
    pub(crate) fn rebind_visual_block(
        &mut self,
        document: &Document,
        snapshot: &LayoutSnapshot,
    ) -> VisualBlockRebindStatus {
        if self.mode != Mode::VisualBlock {
            return VisualBlockRebindStatus::Inactive;
        }
        if self.visual_block.as_ref().is_some_and(|selection| {
            selection.anchor.document_id != snapshot.document_id
                || selection.anchor.document_revision != snapshot.document_revision
                || selection.anchor.layout_revision != snapshot.revision
        }) {
            self.visual_block = None;
            self.visual_position = None;
        }
        let Some(block) = self.active_visual_block else {
            // Legacy/direct-test state can still carry exact geometry only.
            // It needs no rebind while that geometry names this snapshot.
            if self.visual_block.as_ref().is_some_and(|selection| {
                selection.anchor.document_id == snapshot.document_id
                    && selection.anchor.document_revision == snapshot.document_revision
                    && selection.anchor.layout_revision == snapshot.revision
            }) {
                return VisualBlockRebindStatus::Rebound;
            }
            self.cancel_visual_block_rebind(VisualBlockRebindError::EndpointNotInLayout {
                endpoint: VisualBlockEndpoint::Anchor,
                offset: self.cursor,
                affinity: self.boundary_affinity,
            });
            return VisualBlockRebindStatus::Cancelled;
        };

        for (endpoint, anchor) in [
            (VisualBlockEndpoint::Anchor, block.anchor),
            (VisualBlockEndpoint::Active, block.active),
        ] {
            if anchor.document() != document.id() {
                self.cancel_visual_block_rebind(VisualBlockRebindError::WrongDocument { endpoint });
                return VisualBlockRebindStatus::Cancelled;
            }
            if anchor.revision() != document.revision() {
                self.cancel_visual_block_rebind(VisualBlockRebindError::WrongRevision {
                    endpoint,
                    expected: document.revision(),
                    actual: anchor.revision(),
                });
                return VisualBlockRebindStatus::Cancelled;
            }
        }

        let anchor_row =
            match visual_block_anchor_row(snapshot, VisualBlockEndpoint::Anchor, block.anchor) {
                Ok(Some(row)) => row,
                Ok(None) => return VisualBlockRebindStatus::AwaitingLayout,
                Err(error) => {
                    self.cancel_visual_block_rebind(error);
                    return VisualBlockRebindStatus::Cancelled;
                }
            };
        let active_row =
            match visual_block_anchor_row(snapshot, VisualBlockEndpoint::Active, block.active) {
                Ok(Some(row)) => row,
                Ok(None) => return VisualBlockRebindStatus::AwaitingLayout,
                Err(error) => {
                    self.cancel_visual_block_rebind(error);
                    return VisualBlockRebindStatus::Cancelled;
                }
            };
        let Some(anchor) = crate::layout::nearest_caret(&snapshot.rows[anchor_row], block.anchor_x)
        else {
            self.cancel_visual_block_rebind(VisualBlockRebindError::EndpointNotInLayout {
                endpoint: VisualBlockEndpoint::Anchor,
                offset: block.anchor.offset(),
                affinity: block.anchor.affinity(),
            });
            return VisualBlockRebindStatus::Cancelled;
        };
        let Some(active) = crate::layout::nearest_caret(&snapshot.rows[active_row], block.active_x)
        else {
            self.cancel_visual_block_rebind(VisualBlockRebindError::EndpointNotInLayout {
                endpoint: VisualBlockEndpoint::Active,
                offset: block.active.offset(),
                affinity: block.active.affinity(),
            });
            return VisualBlockRebindStatus::Cancelled;
        };
        let mut selection =
            match BlockSelection::new(anchor.point, active.point, block.anchor_x, block.active_x) {
                Ok(selection) => selection,
                Err(error) => {
                    self.cancel_visual_block_rebind(VisualBlockRebindError::InvalidSelection(
                        error,
                    ));
                    return VisualBlockRebindStatus::Cancelled;
                }
            };
        if let Err(error) = selection.update_inclusive_rectangle(snapshot) {
            self.cancel_visual_block_rebind(VisualBlockRebindError::InvalidSelection(error));
            return VisualBlockRebindStatus::Cancelled;
        }

        let active_position = VisualPosition {
            text_offset: active.point.text_offset,
            affinity: active.point.affinity,
        };
        self.visual_block = Some(selection);
        self.cursor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            snapshot,
            active_position,
        );
        self.boundary_affinity = active_position.affinity;
        self.visual_position = Some(active_position);
        self.position_revision = Some(document.revision());
        self.visual_block_rebind_error = None;
        VisualBlockRebindStatus::Rebound
    }

    fn cancel_visual_block_rebind(&mut self, error: VisualBlockRebindError) {
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.visual_block_rebind_error = Some(error);
        self.clear_pending();
    }

    /// Reports the deferred Visual Block insert/change currently collecting
    /// text. No document bytes change until Escape commits the batch.
    pub fn visual_block_insert_kind(&self) -> Option<VisualBlockInsertKind> {
        self.visual_block_insert
            .as_ref()
            .map(|session| session.kind)
    }

    pub fn visual_block_insert_payload(&self) -> Option<&str> {
        self.visual_block_insert
            .as_ref()
            .map(|session| session.payload.as_str())
    }

    pub fn command_line(&self) -> Option<&str> {
        self.command_line_state
            .as_ref()
            .map(|state| state.buffer.input.as_str())
    }

    pub fn command_line_kind(&self) -> Option<CommandLineKind> {
        self.command_line_state.as_ref().map(|state| state.kind)
    }

    pub fn command_line_cursor(&self) -> Option<usize> {
        self.command_line_state
            .as_ref()
            .map(|state| state.buffer.cursor)
    }

    pub fn wrap_option(&self) -> bool {
        self.wrap
    }

    pub fn fileformats_option(&self) -> &[FileFormat] {
        &self.fileformats
    }

    pub fn register(&self, name: char) -> Option<&RegisterValue> {
        self.registers.get(name)
    }

    /// Current stored register names in stable scalar order. Dynamic host
    /// registers (`+`/`*`) and the current artifact name (`%`) are deliberately
    /// absent; a boundary resolving them must add only the capabilities it
    /// captured for that exact turn.
    pub(crate) fn stored_register_names(&self) -> Vec<char> {
        self.registers.names()
    }

    /// Current local marks in stable name order. Offsets belong to
    /// `position_revision()` and are exposed only to the coordinator/FFI
    /// composition boundary for immutable command-result snapshots.
    pub(crate) fn mark_positions(&self) -> impl Iterator<Item = (char, usize)> + '_ {
        self.marks.iter().map(|(name, offset)| (*name, *offset))
    }

    /// Jump locations from oldest to newest plus the current list index.
    /// An empty list has no meaningful current index.
    pub(crate) fn jump_positions(&self) -> (&[usize], Option<usize>) {
        (
            &self.jumps,
            (!self.jumps.is_empty()).then_some(self.jump_index),
        )
    }

    /// Resolve a stored or special register against exact external state for
    /// one read. `+`/`*` never fall back to an internal cache and `%` never
    /// performs a lossy path conversion.
    pub fn register_with_context(
        &self,
        document: &Document,
        clipboard: &ClipboardCommandContext,
        name: char,
    ) -> Result<Option<RegisterValue>, RegisterReadError> {
        self.registers.read(
            name,
            RegisterReadContext::new(
                clipboard,
                document.artifact_binding().map(|binding| binding.path()),
            ),
        )
    }

    pub fn is_recording_macro(&self) -> bool {
        self.recording.is_some()
    }

    /// Sets the caret from a trusted view hit-test. Invalid byte or grapheme
    /// boundaries are rejected instead of being silently clamped.
    pub fn set_cursor(&mut self, document: &Document, offset: usize) -> bool {
        let lines = document.hard_line_snapshot();
        if lines.is_grapheme_boundary(offset) {
            self.typing_style = Default::default();
            self.input_assistance.clear_tag();
            self.invalidate_replace_restoration();
            self.cursor = normalize_normal_cursor_document(document, &lines, offset);
            self.position_revision = Some(document.revision());
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.visual_position = None;
            self.desired_x = None;
            self.preferred_column = None;
            true
        } else {
            false
        }
    }

    /// Places the caret from a native pointer hit-test while keeping Vim mode
    /// state authoritative in the command controller. A plain placement ends
    /// an active Visual selection but preserves Insert/Replace mode. Extending
    /// starts or updates a Visual Character selection from the pre-click
    /// cursor, matching the native shift-click/drag affordance without keeping
    /// a second frontend-owned selection.
    pub fn set_cursor_from_pointer(
        &mut self,
        document: &Document,
        offset: usize,
        affinity: BoundaryAffinity,
        extend_selection: bool,
    ) -> bool {
        let lines = document.hard_line_snapshot();
        if !lines.is_grapheme_boundary(offset) {
            return false;
        }

        self.invalidate_replace_restoration();
        self.typing_style = Default::default();
        self.input_assistance.clear_tag();
        if extend_selection {
            if !matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            ) {
                self.visual_anchor = Some(self.cursor);
            }
            self.mode = Mode::VisualCharacter;
        } else {
            if matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock | Mode::CommandLine
            ) {
                self.mode = Mode::Normal;
            }
            self.visual_anchor = None;
        }

        let (cursor, affinity) = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            (offset, affinity)
        } else {
            let cursor = normalize_normal_cursor_document(document, &lines, offset);
            // A hit-test at the trailing edge of a non-empty hard line returns
            // its end boundary with upstream affinity. Normal-mode storage
            // addresses the associated grapheme by its start boundary, so the
            // affinity must be canonicalized with the moved point. Retaining
            // upstream here would visually associate the preceding grapheme a
            // second time even though commands operate on `cursor`.
            let affinity = if cursor == offset {
                affinity
            } else {
                BoundaryAffinity::Downstream
            };
            (cursor, affinity)
        };
        self.cursor = cursor;
        self.position_revision = Some(document.revision());
        self.boundary_affinity = affinity;
        self.visual_position = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.command_line_state = None;
        self.insert_normal_once = None;
        self.ctrl_o_just_started = false;
        self.clear_pending();
        true
    }

    /// Select the complete logical document, including its final hard break.
    /// The host closes an active editing session before invoking this method.
    pub(crate) fn select_all(&mut self, document: &Document) -> Result<(), DocumentError> {
        let end = document.projection().text_tree().byte_len();
        document.text_point(0)?;
        document.text_point(end)?;
        self.set_cursor_from_pointer(document, 0, BoundaryAffinity::Downstream, false);
        self.mode = Mode::VisualCharacter;
        self.visual_anchor = Some(0);
        self.cursor = end;
        self.boundary_affinity = BoundaryAffinity::Upstream;
        self.visual_to_line_end = false;
        Ok(())
    }

    /// Publish a text commit performed by a non-keyboard core input source,
    /// such as an IME composition. Insert/Replace modes retain a boundary
    /// caret; character-shaped modes normalize it to their associated item.
    pub(crate) fn note_external_text_commit(
        &mut self,
        document: &Document,
        caret_offset: usize,
        replaced_empty_range: bool,
        inserted_text: &str,
        input_intent: &str,
    ) -> Result<(), DocumentError> {
        document.text_point(caret_offset)?;
        self.cursor = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            caret_offset
        } else {
            let lines = document.hard_line_snapshot();
            normalize_normal_cursor_document(document, &lines, caret_offset)
        };
        self.position_revision = Some(document.revision());
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.clear_pending();

        if let Some(session) = self.insert_session.as_mut() {
            session.replace_journal.clear();
            session.unit_floor = self.cursor;
            let inserted_value = external_text_register_value(document, inserted_text);
            if !inserted_value.text.is_empty() {
                session.preserve_normal_repeat = false;
            }
            if replaced_empty_range && self.mode == Mode::Insert {
                if let Some(program) = session.repeat_program.as_mut() {
                    program.append_text(&external_text_register_value(document, input_intent));
                }
                session
                    .last_inserted
                    .append_inserted_payload(&inserted_value);
            } else {
                session.repeat_program = None;
                session
                    .last_inserted
                    .append_inserted_payload(&inserted_value);
            }
        }
        Ok(())
    }

    pub fn handle(
        &mut self,
        document: &mut Document,
        event: InputEvent,
    ) -> Result<CommandOutput, DocumentError> {
        let model_checkpoint = document.begin_command_checkpoint();
        let checkpoint = self.clone();
        if matches!(
            &event,
            InputEvent::Key(
                Key::Left
                    | Key::Right
                    | Key::Up
                    | Key::Down
                    | Key::Home
                    | Key::End
                    | Key::PageUp
                    | Key::PageDown
            )
        ) {
            self.typing_style = Default::default();
            self.input_assistance.clear_tag();
        }
        let edit_group_depth = document.edit_group_depth();
        self.record_event(&event);
        match self.dispatch_event(document, event) {
            Ok(mut output) => {
                self.finish_non_layout_dispatch(&output);
                self.finish_insert_normal_once(document, &mut output);
                self.finish_explicit_register_prefix(&output);
                self.position_revision = Some(document.revision());
                self.finish_clipboard_writes(&mut output);
                document.commit_command_checkpoint(model_checkpoint);
                Ok(output)
            }
            Err(error) => {
                document.rollback_command_checkpoint(model_checkpoint);
                self.rollback_failed_event(document, checkpoint, edit_group_depth);
                Err(error)
            }
        }
    }

    /// Handle one headless command with externally captured clipboard state.
    /// The context is installed only for this event and is never retained as
    /// buffer state.
    pub fn handle_with_clipboard_context(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        clipboard: &ClipboardCommandContext,
    ) -> Result<CommandOutput, DocumentError> {
        let previous = std::mem::replace(&mut self.clipboard_context, clipboard.clone());
        let result = self.handle(document, event);
        self.clipboard_context = previous;
        result
    }

    /// Interprets one event against an exact immutable layout revision.
    /// [`Self::handle`] remains available for headless callers; commands that
    /// intrinsically require rows or viewport geometry report `Unsupported`
    /// there instead of inventing geometry.
    pub fn handle_with_layout(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        context: &mut LayoutCommandContext<'_>,
    ) -> Result<CommandOutput, DocumentError> {
        let model_checkpoint = document.begin_command_checkpoint();
        let checkpoint = self.clone();
        let edit_group_depth = document.edit_group_depth();
        let viewport = context.viewport;
        self.line_layout = Some(context.snapshot.clone());
        let result = self.handle_with_layout_inner(document, event, context);
        self.line_layout = None;
        match result {
            Ok(mut output) => {
                if matches!(output.status, CommandStatus::NeedsMoreLayout(_)) {
                    document.rollback_command_checkpoint(model_checkpoint);
                    *self = checkpoint;
                    context.viewport = viewport;
                    return Ok(output);
                }
                self.finish_explicit_register_prefix(&output);
                self.position_revision = Some(document.revision());
                self.finish_clipboard_writes(&mut output);
                document.commit_command_checkpoint(model_checkpoint);
                Ok(output)
            }
            Err(error) => {
                context.viewport = viewport;
                document.rollback_command_checkpoint(model_checkpoint);
                self.rollback_failed_event(document, checkpoint, edit_group_depth);
                Err(error)
            }
        }
    }

    /// Layout-aware counterpart of [`Self::handle_with_clipboard_context`].
    pub fn handle_with_layout_and_clipboard_context(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        layout: &mut LayoutCommandContext<'_>,
        clipboard: &ClipboardCommandContext,
    ) -> Result<CommandOutput, DocumentError> {
        let previous = std::mem::replace(&mut self.clipboard_context, clipboard.clone());
        let result = self.handle_with_layout(document, event, layout);
        self.clipboard_context = previous;
        result
    }

    /// Execute exactly one input event for the serial core coordinator. A
    /// macro or `:normal` command is returned as an internal continuation
    /// instead of recursively entering the headless interpreter.
    pub(crate) fn handle_for_core(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> Result<CommandStep, DocumentError> {
        debug_assert!(self.pending_replay.is_none());
        self.plan_compound_replay = true;
        let result = match clipboard {
            Some(clipboard) => self.handle_with_clipboard_context(document, event, clipboard),
            None => self.handle(document, event),
        };
        self.plan_compound_replay = false;
        match result {
            Ok(output) => Ok(CommandStep {
                output,
                replay: self.pending_replay.take(),
            }),
            Err(error) => {
                self.pending_replay = None;
                Err(error)
            }
        }
    }

    /// Layout-aware one-event counterpart of [`Self::handle_for_core`].
    pub(crate) fn handle_with_layout_for_core(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        layout: &mut LayoutCommandContext<'_>,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> Result<CommandStep, DocumentError> {
        debug_assert!(self.pending_replay.is_none());
        self.plan_compound_replay = true;
        let result = match clipboard {
            Some(clipboard) => {
                self.handle_with_layout_and_clipboard_context(document, event, layout, clipboard)
            }
            None => self.handle_with_layout(document, event, layout),
        };
        self.plan_compound_replay = false;
        match result {
            Ok(output) => Ok(CommandStep {
                output,
                replay: self.pending_replay.take(),
            }),
            Err(error) => {
                self.pending_replay = None;
                Err(error)
            }
        }
    }

    /// Resolve one non-layout event without mutating the authoritative
    /// document or this interpreter.
    ///
    /// Direct Insert/Replace edits, Normal `x`/`X`/Delete, one-step history
    /// navigation, and controller-only logical commands emit typed plans.
    /// Source-changing compound commands and commands which consume exact
    /// layout geometry retain the compatibility executor until all of their
    /// effects have a typed representation.
    pub fn resolve(
        &self,
        context: &CommandContext<'_>,
        event: InputEvent,
    ) -> Result<CommandResolution, DocumentError> {
        if context.document_id() != context.document().id()
            || context.document_revision() != context.document().revision()
        {
            return Err(DocumentError::WrongSnapshot {
                expected: context.document().revision(),
                actual: context.document_revision(),
            });
        }

        if matches!(&event, InputEvent::Key(Key::SelectAll)) {
            return Ok(CommandResolution::Legacy(LegacyCommandReason::CompoundOrUnmigrated));
        }
        if self.needs_input_assistance(context.document(), &event)
            || !self.typing_style.is_empty()
            || (self.mode == Mode::Replace
                && context.document().format() == crate::document::Format::Html)
            || self
                .insert_session
                .as_ref()
                .and_then(|session| session.replace_journal.last())
                .is_some_and(|entry| entry.source_record.is_some())
        {
            return Ok(CommandResolution::Legacy(
                LegacyCommandReason::CompoundOrUnmigrated,
            ));
        }
        if self.line_mode == LineMode::PhysicalSource
            && matches!(&event,InputEvent::Key(key) if (self.mode==Mode::VisualLine||self.mode_line_key(*key)||matches!(key,Key::Char('p'|'P'))))
        {
            return Ok(CommandResolution::Legacy(
                LegacyCommandReason::CompoundOrUnmigrated,
            ));
        }
        if self.requires_layout_for_input(context.document(), &event, context.clipboard()) {
            return Ok(CommandResolution::Legacy(
                LegacyCommandReason::LayoutDependent,
            ));
        }

        if let Some(plan) = self.plan_history_event(context, &event)? {
            return Ok(CommandResolution::Planned(plan));
        }
        if let Some(plan) = self.plan_normal_delete_event(context, &event)? {
            return Ok(CommandResolution::Planned(plan));
        }
        if let Some(plan) = self.plan_direct_edit_text(context, &event)? {
            return Ok(CommandResolution::Planned(plan));
        }
        if let Some(plan) = self.plan_controller_only_event(context, &event) {
            return Ok(CommandResolution::Planned(plan));
        }
        Ok(CommandResolution::Legacy(
            LegacyCommandReason::CompoundOrUnmigrated,
        ))
    }

    /// Resolve an event whose complete effect is confined to command state.
    ///
    /// [`Self::try_handle_controller_only_event`] is also the first dispatch
    /// path used by the compatibility executor. Keeping the transition in one
    /// place prevents the immutable planner and direct interpreter from
    /// acquiring subtly different count, prefix, search, or Visual semantics.
    fn plan_controller_only_event(
        &self,
        context: &CommandContext<'_>,
        event: &InputEvent,
    ) -> Option<CommandPlan> {
        // A single Normal command entered through Insert Ctrl-O can close and
        // reopen a model-owned edit group as it returns to Insert. That effect
        // needs a typed undo-group directive before it can use this planner.
        if self.insert_normal_once.is_some() || self.ctrl_o_just_started {
            return None;
        }

        let document = context.document();
        let mut next = self.clone();
        next.record_event(event);
        let mut output = next.try_handle_controller_only_event(document, event)?;
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        next.finish_clipboard_writes(&mut output);
        Some(next.non_mutating_plan(document, output))
    }

    fn plan_history_event(
        &self,
        context: &CommandContext<'_>,
        event: &InputEvent,
    ) -> Result<Option<CommandPlan>, DocumentError> {
        let redo = match event {
            InputEvent::Key(Key::Char('u'))
                if self.mode == Mode::Normal
                    && self.pending == Pending::None
                    && !self.register_pending =>
            {
                false
            }
            InputEvent::Key(Key::Ctrl('r' | 'R'))
                if self.mode == Mode::Normal
                    && self.pending == Pending::None
                    && !self.register_pending =>
            {
                true
            }
            _ => return Ok(None),
        };
        if self.compound_replay_depth > 0 || self.count.unwrap_or(1).max(1) != 1 {
            return Ok(None);
        }

        let document = context.document();
        let mut next = self.clone();
        next.record_event(event);
        if let Some(output) = next.finish_overflowed_count() {
            return Ok(Some(next.non_mutating_plan(document, output)));
        }
        next.count = None;

        let available = if redo {
            document.history_status().can_redo
        } else {
            document.history_status().can_undo
        };
        if !available {
            let message = if redo {
                "no preferred redo state is available"
            } else {
                "already at the oldest document state"
            };
            let output = CommandOutput {
                status: CommandStatus::Error(message.to_owned()),
                ..CommandOutput::complete()
            };
            next.finish_explicit_register_prefix(&output);
            return Ok(Some(next.non_mutating_plan(document, output)));
        }

        let navigation = if redo {
            HistoryNavigationRequest::Redo
        } else {
            HistoryNavigationRequest::Undo
        };
        let output = CommandOutput {
            document_changed: true,
            history_navigation: true,
            ..CommandOutput::complete()
        };
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        let failure_controller = self.failed_plan_controller(document.revision());
        Ok(Some(CommandPlan {
            document: document.id(),
            revision: document.revision(),
            model: Some(CommandModelRequest::Model(ModelRequest::NavigateHistory {
                document: document.id(),
                revision: document.revision(),
                navigation,
            })),
            success_controller: Box::new(next),
            failure_controller: Box::new(failure_controller),
            post_commit: PlannedPostCommit::None,
            output,
            replay: None,
            undo_group: UndoGroupDirective::End,
            presentation: vec![
                CommandPresentationRequest::Relayout,
                CommandPresentationRequest::RevealCaret,
            ],
        }))
    }

    fn plan_normal_delete_event(
        &self,
        context: &CommandContext<'_>,
        event: &InputEvent,
    ) -> Result<Option<CommandPlan>, DocumentError> {
        let backward = match event {
            InputEvent::Key(Key::Char('x') | Key::Delete)
                if self.mode == Mode::Normal
                    && self.pending == Pending::None
                    && !self.register_pending
                    // Returning from Insert/Replace Ctrl-O also reopens the
                    // edit group. That controller/model combination remains
                    // on the compatibility path until plans can request a
                    // typed Begin undo-group effect.
                    && self.insert_normal_once.is_none() =>
            {
                false
            }
            InputEvent::Key(Key::Char('X'))
                if self.mode == Mode::Normal
                    && self.pending == Pending::None
                    && !self.register_pending
                    && self.insert_normal_once.is_none() =>
            {
                true
            }
            _ => return Ok(None),
        };
        let document = context.document();
        let mut next = self.clone();
        next.record_event(event);
        if let Some(output) = next.finish_overflowed_count() {
            return Ok(Some(next.non_mutating_plan(document, output)));
        }
        let count = next.count.take().unwrap_or(1).max(1);
        let register = next.requested_register.take();
        let previous_clipboard = std::mem::replace(
            &mut next.clipboard_context,
            context.clipboard().cloned().unwrap_or_default(),
        );
        if let Err(output) = next.require_register_write(register) {
            next.clipboard_context = previous_clipboard;
            next.finish_explicit_register_prefix(&output);
            return Ok(Some(next.non_mutating_plan(document, output)));
        }

        let lines = document.hard_line_snapshot();
        let range = normal_delete_range(&lines, self.cursor, count, backward);
        if range.is_empty() {
            next.clipboard_context = previous_clipboard;
            let output = CommandOutput::complete();
            next.finish_explicit_register_prefix(&output);
            return Ok(Some(next.non_mutating_plan(document, output)));
        }

        let value = register_value(
            document,
            &lines,
            &MotionExtent {
                range: range.clone(),
                kind: MotionKind::Characterwise,
            },
            register,
        );
        next.delete_register(register, value, DeletionClass::Small);
        next.cursor = range.start;
        if !next.replaying {
            next.last_repeat = Some(if backward {
                RepeatAction::DeleteBackward { count }
            } else {
                RepeatAction::DeleteForward { count }
            });
        }
        let mut output = CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        };
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        next.finish_clipboard_writes(&mut output);
        next.clipboard_context = previous_clipboard;

        Ok(Some(self.edited_plan(
            document,
            next,
            CommandModelRequest::Model(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(range, "")],
            }),
            output,
            PlannedPostCommit::NormalizeNormalCursor,
        )))
    }

    fn plan_direct_edit_text(
        &self,
        context: &CommandContext<'_>,
        event: &InputEvent,
    ) -> Result<Option<CommandPlan>, DocumentError> {
        if !matches!(self.mode, Mode::Insert | Mode::Replace)
            || self.pending != Pending::None
            || self.register_pending
            || self.visual_block_insert.is_some()
        {
            return Ok(None);
        }
        let document = context.document();
        if let InputEvent::Key(key) = event {
            if let Some(request) = self.paragraph_key_request(document, *key)? {
                return self.plan_paragraph_key(document, event, *key, request).map(Some);
            }
        }
        match event {
            InputEvent::Key(Key::Backspace) => {
                return self.plan_edit_mode_backspace(document, event).map(Some)
            }
            InputEvent::Key(Key::Delete) => {
                return self.plan_edit_mode_delete(document, event).map(Some)
            }
            InputEvent::Key(Key::Ctrl('w' | 'W')) => {
                return self
                    .plan_edit_mode_delete_motion(document, event, EditSessionStep::DeleteWord)
                    .map(Some)
            }
            InputEvent::Key(Key::Ctrl('u' | 'U')) => {
                return self
                    .plan_edit_mode_delete_motion(
                        document,
                        event,
                        EditSessionStep::DeleteToLineStart,
                    )
                    .map(Some)
            }
            _ => {}
        }
        let list_enter =
            if self.mode == Mode::Insert && matches!(event, InputEvent::Key(Key::Enter)) {
                document.list_enter_edit(self.cursor)?
            } else {
                None
            };
        let value = match event {
            InputEvent::Text(input) => external_text_register_value(document, input),
            InputEvent::Key(Key::Char(character)) => {
                RegisterValue::characterwise(character.to_string())
            }
            InputEvent::Key(Key::Enter) => RegisterValue::characterwise(
                list_enter
                    .as_ref()
                    .map_or("\n", |edit| edit.replacement.as_str()),
            ),
            InputEvent::Key(Key::Tab) => RegisterValue::characterwise("\t"),
            _ => return Ok(None),
        };

        let mut next = self.clone();
        next.record_event(event);
        if matches!(event, InputEvent::Key(Key::Enter)) {
            next.invalidate_replace_restoration();
        }
        let intent = value;
        let snapshot = document.hard_line_snapshot();
        let start = list_enter.as_ref().map_or(self.cursor, |edit| edit.range.start);
        let end = if self.mode == Mode::Replace {
            replacement_payload_end(&snapshot, start, &intent)
        } else {
            list_enter.as_ref().map_or(start, |edit| edit.range.end)
        };
        let value = if matches!(event, InputEvent::Text(_) | InputEvent::Key(Key::Char(_))) {
            self.assist_typing_input_payload(document, start..end, self.insertion_boundary_affinity(), &intent)?
        } else {
            intent.clone()
        };
        let input = value.text.as_str();
        let structural_list_enter =
            list_enter.is_some() && document.format().has_structural_lists();
        if input.is_empty()
            && !structural_list_enter
            && list_enter
                .as_ref()
                .map_or(true, |edit| edit.range.is_empty())
        {
            return Ok(Some(
                next.non_mutating_plan(document, CommandOutput::complete()),
            ));
        }

        let (end, post_commit) = if self.mode == Mode::Insert {
            (
                list_enter.as_ref().map_or(start, |edit| edit.range.end),
                PlannedPostCommit::NormalizeTypingCursor,
            )
        } else {
            next.plan_replace_payload_state(document, &snapshot, &value)
        };
        let payload = FormattedTextPayload::new(
            &snapshot,
            value.text.clone(),
            value.hard_break_offsets().to_vec(),
        )
        .expect("command register payload carries validated semantic hard breaks");
        let edit = document.normalize_typing_payload(
            FormattedPayloadEdit::new(start..end, payload)
                .with_boundary_affinity(self.insertion_boundary_affinity()),
        )?;
        let typing_caret = edit.range().start + edit.payload().text().len();
        let model = if structural_list_enter {
            CommandModelRequest::Model(ModelRequest::ContinueList {
                document: document.id(),
                revision: document.revision(),
                at: self.cursor,
            })
        } else {
            CommandModelRequest::FormattedPayload(FormattedPayloadEditRequest::new(
                document.id(),
                document.revision(),
                vec![edit],
            ))
        };

        next.cursor = if structural_list_enter {
            start + input.len()
        } else {
            typing_caret
        };
        if let Some(session) = next.insert_session.as_mut() {
            session.record_inserted_intent(&value, &intent, list_enter.as_ref().map(|_| EditSessionStep::ListEnter));
        }
        let mut output = CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        };
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        next.finish_clipboard_writes(&mut output);

        Ok(Some(self.edited_plan(document, next, model, output, post_commit)))
    }

    fn paragraph_key_request(
        &self,
        document: &Document,
        key: Key,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        if matches!(key, Key::Tab | Key::BackTab) && self.mode == Mode::Insert {
            return document.list_item_indent_key_request(self.cursor, key == Key::BackTab);
        }
        if key == Key::ShiftEnter {
            return Ok(Some(ModelRequest::InsertHardBreak {
                document: document.id(), revision: document.revision(),
                at: self.cursor, affinity: self.insertion_boundary_affinity(),
            }));
        }
        if !matches!(key, Key::Enter | Key::Backspace) {
            return Ok(None);
        }
        if key == Key::Backspace && self.mode == Mode::Replace
            && self.insert_session.as_ref().and_then(|session| session.replace_journal.last())
                .is_some_and(|entry| entry.frontier() == Some(self.cursor))
        {
            return Ok(None);
        }
        document.paragraph_boundary_reset_request(self.cursor, key == Key::Enter)
    }

    fn note_paragraph_key(&mut self, key: Key) {
        self.invalidate_replace_restoration();
        if key != Key::ShiftEnter {
            self.typing_style = Default::default();
        }
        self.boundary_affinity = BoundaryAffinity::Downstream;
        if let Some(session) = self.insert_session.as_mut() {
            if key == Key::ShiftEnter {
                session.record_inserted(
                    &RegisterValue::characterwise("\n"),
                    Some(EditSessionStep::HardBreak),
                );
            } else {
                session.record_step(match key {
                    Key::Backspace => EditSessionStep::Backspace,
                    Key::Tab | Key::BackTab => EditSessionStep::ListIndent { unindent: key == Key::BackTab },
                    _ => EditSessionStep::ListEnter,
                });
            }
        }
    }

    fn plan_paragraph_key(
        &self,
        document: &Document,
        event: &InputEvent,
        key: Key,
        request: ModelRequest,
    ) -> Result<CommandPlan, DocumentError> {
        let mut next = self.clone();
        next.record_event(event);
        let Some(prepared) = Self::prepare_paragraph_key(document, key, request.clone())? else {
            return Ok(next.non_mutating_plan(document, CommandOutput::complete()));
        };
        next.cursor = if key == Key::ShiftEnter {
            prepared_break_cursor(document, &prepared, self.cursor)?
        } else {
            prepared_cursor(document, &prepared, self.cursor, Association::BeforeInsertion)?
        };
        next.note_paragraph_key(key);
        let mut output = CommandOutput { document_changed: !prepared.is_no_op(), cursor_moved: true, ..CommandOutput::complete() };
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        next.finish_clipboard_writes(&mut output);
        Ok(self.edited_plan(
            document,
            next,
            CommandModelRequest::Model(request),
            output,
            PlannedPostCommit::None,
        ))
    }

    fn apply_paragraph_key(
        &mut self,
        document: &mut Document,
        key: Key,
        request: ModelRequest,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(prepared) = Self::prepare_paragraph_key(document, key, request)? else {
            return Ok(CommandOutput::complete());
        };
        let cursor = if key == Key::ShiftEnter {
            prepared_break_cursor(document, &prepared, self.cursor)?
        } else {
            prepared_cursor(document, &prepared, self.cursor, Association::BeforeInsertion)?
        };
        let changed = !prepared.is_no_op();
        document.commit_model_transaction(prepared).map_err(command_document_error)?;
        self.cursor = cursor;
        self.note_paragraph_key(key);
        Ok(CommandOutput { document_changed: changed, cursor_moved: true, ..CommandOutput::complete() })
    }

    fn prepare_paragraph_key(
        document: &Document,
        key: Key,
        request: ModelRequest,
    ) -> Result<Option<PreparedModelTransaction>, DocumentError> {
        match document.prepare_model_request(request) {
            Ok(prepared) => Ok(Some(prepared)),
            // Reaching the list nesting limit, or an item with no suitable
            // parent/sibling, makes the structural key a harmless no-op.
            Err(ModelTransactionError::Document(DocumentError::UnsupportedFormatting))
                if matches!(key, Key::Tab | Key::BackTab) => Ok(None),
            Err(error) => Err(command_document_error(error)),
        }
    }

    fn plan_edit_mode_backspace(
        &self,
        document: &Document,
        event: &InputEvent,
    ) -> Result<CommandPlan, DocumentError> {
        let mut next = self.clone();
        next.record_event(event);
        if self.mode == Mode::Replace {
            let journal_entry = self
                .insert_session
                .as_ref()
                .and_then(|session| session.replace_journal.last())
                .filter(|entry| entry.start.checked_add(entry.inserted.len()) == Some(self.cursor))
                .cloned();
            if let Some(entry) = journal_entry {
                next.cursor = entry.start;
                if let Some(session) = next.insert_session.as_mut() {
                    session.replace_journal.pop();
                    session.record_deleted(document.format(), &entry.inserted, EditSessionStep::Backspace);
                }
                return Ok(self.planned_flat_text_edit(
                    document,
                    next,
                    entry.start..self.cursor,
                    entry.original.unwrap_or_default(),
                    true,
                ));
            }
            next.invalidate_replace_restoration();
        }
        let Some(start) = document
            .hard_line_snapshot()
            .previous_grapheme_boundary(self.cursor)
        else {
            return Ok(next.non_mutating_plan(document, CommandOutput::complete()));
        };
        let removed = document.hard_line_snapshot().slice_utf8(start..self.cursor).expect("validated backspace range");
        next.cursor = start;
        if let Some(session) = next.insert_session.as_mut() {
            session.record_deleted(document.format(), &removed, EditSessionStep::Backspace);
        }
        Ok(self.planned_flat_text_edit(document, next, start..self.cursor, "", true))
    }

    fn plan_edit_mode_delete(
        &self,
        document: &Document,
        event: &InputEvent,
    ) -> Result<CommandPlan, DocumentError> {
        let mut next = self.clone();
        next.record_event(event);
        next.invalidate_replace_restoration();
        let Some(end) = document
            .hard_line_snapshot()
            .next_grapheme_boundary(self.cursor)
        else {
            if let Some(session) = next.insert_session.as_mut() {
                session.record_step(EditSessionStep::Delete);
            }
            return Ok(next.non_mutating_plan(document, CommandOutput::complete()));
        };
        if let Some(session) = next.insert_session.as_mut() {
            session.record_step(EditSessionStep::Delete);
        }
        Ok(self.planned_flat_text_edit(document, next, self.cursor..end, "", false))
    }

    fn plan_edit_mode_delete_motion(
        &self,
        document: &Document,
        event: &InputEvent,
        step: EditSessionStep,
    ) -> Result<CommandPlan, DocumentError> {
        let mut next = self.clone();
        next.record_event(event);
        next.invalidate_replace_restoration();
        let range = match self.insert_delete_motion_range(document, &step) {
            Ok(range) => range,
            Err(output) => return Ok(next.non_mutating_plan(document, output)),
        };
        if range.is_empty() {
            return Ok(next.non_mutating_plan(document, CommandOutput::complete()));
        }
        let removed = document.hard_line_snapshot().slice_utf8(range.clone()).expect("validated deletion motion range");
        next.cursor = range.start;
        if let Some(session) = next.insert_session.as_mut() {
            session.unit_floor = session.unit_floor.min(next.cursor);
            session.record_deleted(document.format(), &removed, step);
        }
        Ok(self.planned_flat_text_edit(document, next, range, "", true))
    }

    fn planned_flat_text_edit(
        &self,
        document: &Document,
        mut next: Self,
        range: Range<usize>,
        replacement: impl Into<String>,
        cursor_moved: bool,
    ) -> CommandPlan {
        let mut output = CommandOutput {
            document_changed: true,
            cursor_moved,
            ..CommandOutput::complete()
        };
        next.finish_non_layout_dispatch(&output);
        next.finish_explicit_register_prefix(&output);
        next.finish_clipboard_writes(&mut output);
        self.edited_plan(
            document,
            next,
            CommandModelRequest::Model(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(range, replacement)],
            }),
            output,
            PlannedPostCommit::None,
        )
    }

    fn edited_plan(
        &self,
        document: &Document,
        next: Self,
        model: CommandModelRequest,
        output: CommandOutput,
        post_commit: PlannedPostCommit,
    ) -> CommandPlan {
        CommandPlan {
            document: document.id(),
            revision: document.revision(),
            model: Some(model),
            success_controller: Box::new(next),
            failure_controller: Box::new(self.failed_plan_controller(document.revision())),
            post_commit,
            output,
            replay: None,
            undo_group: UndoGroupDirective::Preserve,
            presentation: vec![
                CommandPresentationRequest::Relayout,
                CommandPresentationRequest::RevealCaret,
            ],
        }
    }

    fn replace_payload_is_journalable(&self, input: &str) -> bool {
        self.mode == Mode::Replace
            && !input.contains('\n')
            && self.insert_session.as_ref()
                .is_some_and(|session| session.placement == InsertPlacement::Replace)
    }

    fn plan_replace_payload_state(
        &mut self,
        document: &Document,
        lines: &HardLineSnapshot,
        value: &RegisterValue,
    ) -> (usize, PlannedPostCommit) {
        let input = value.text.as_str();
        let journalable = self.replace_payload_is_journalable(input);
        if !journalable {
            self.invalidate_replace_restoration();
        }
        let (end, journal_entries) = replacement_payload_targets(document, lines, self.cursor, value, journalable);
        let post_commit = if journalable {
            PlannedPostCommit::ReplaceJournal(journal_entries)
        } else {
            PlannedPostCommit::NormalizeTypingCursor
        };
        (end, post_commit)
    }

    fn failed_plan_controller(&self, revision: Revision) -> Self {
        let mut failure = self.clone();
        if self.pending != Pending::None || self.register_pending {
            failure.clear_pending();
        } else {
            failure.requested_register = None;
        }
        failure.pending_clipboard_writes.clear();
        failure.position_revision = Some(revision);
        failure
    }

    fn non_mutating_plan(self, document: &Document, output: CommandOutput) -> CommandPlan {
        let failure = self.clone();
        let presentation = if output.cursor_moved {
            vec![CommandPresentationRequest::RevealCaret]
        } else {
            Vec::new()
        };
        CommandPlan {
            document: document.id(),
            revision: document.revision(),
            model: None,
            success_controller: Box::new(self),
            failure_controller: Box::new(failure),
            post_commit: PlannedPostCommit::None,
            output,
            replay: None,
            undo_group: UndoGroupDirective::Preserve,
            presentation,
        }
    }

    fn read_register_value(
        &self,
        document: &Document,
        name: char,
    ) -> Result<Option<RegisterValue>, RegisterReadError> {
        self.register_with_context(document, &self.clipboard_context, name)
    }

    fn read_register_value_for_put(
        &self,
        document: &Document,
        name: char,
    ) -> Result<Option<RegisterValue>, RegisterReadError> {
        self.registers.read_for_put(
            name,
            RegisterReadContext::new(
                &self.clipboard_context,
                document.artifact_binding().map(|binding| binding.path()),
            ),
        )
    }

    // Returning the ready-to-publish output keeps every register failure path
    // identical at its many command sites; boxing this short-lived local error
    // would add allocation solely to reduce the enum's stack-size estimate.
    #[allow(clippy::result_large_err)]
    fn require_register_value(
        &self,
        document: &Document,
        name: char,
    ) -> Result<RegisterValue, CommandOutput> {
        match self.read_register_value_for_put(document, name) {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err(CommandOutput {
                status: CommandStatus::Error(format!("empty register {name}")),
                ..CommandOutput::complete()
            }),
            Err(error) => Err(CommandOutput {
                status: CommandStatus::RegisterReadError(error),
                ..CommandOutput::complete()
            }),
        }
    }

    fn validate_register_write(&self, requested: Option<char>) -> Result<(), RegisterWriteError> {
        self.registers
            .validate_write(requested, &self.clipboard_context)
    }

    #[allow(clippy::result_large_err)]
    fn require_register_write(&self, requested: Option<char>) -> Result<(), CommandOutput> {
        self.validate_register_write(requested)
            .map_err(|error| CommandOutput {
                status: CommandStatus::RegisterWriteError(error),
                ..CommandOutput::complete()
            })
    }

    fn collect_register_write(&mut self, effect: RegisterWriteEffect) {
        if let Some(request) = effect.clipboard() {
            self.pending_clipboard_writes.push(request);
        }
    }

    fn yank_register(&mut self, requested: Option<char>, mut value: RegisterValue) {
        if self.clipboard_copy_as_seen && requested == Some('*') {
            if let Some(fragment) = value.clipboard_fragment() {
                value = RegisterValue::from_clipboard_fragment(fragment.as_seen()).expect("captured exact clipboard selection");
            }
        }
        self.clipboard_copy_as_seen = false;
        let effect = self.registers.yank(requested, value);
        self.collect_register_write(effect);
    }

    fn delete_register(
        &mut self,
        requested: Option<char>,
        value: RegisterValue,
        class: DeletionClass,
    ) {
        let effect = self.registers.delete(requested, value, class);
        self.collect_register_write(effect);
    }

    fn finish_clipboard_writes(&mut self, output: &mut CommandOutput) {
        output
            .clipboard_writes
            .append(&mut self.pending_clipboard_writes);
    }

    fn publish_last_insert_fragment(&mut self) {
        let value = self
            .insert_session
            .as_ref()
            .map(|session| session.last_inserted.clone())
            .filter(|value| !value.text.is_empty());
        if let Some(value) = value {
            self.registers.set_last_insert(value);
        }
    }

    fn handle_with_layout_inner(
        &mut self,
        document: &mut Document,
        event: InputEvent,
        context: &mut LayoutCommandContext<'_>,
    ) -> Result<CommandOutput, DocumentError> {
        self.record_event(&event);
        if context.snapshot.document_id != document.id()
            || context.snapshot.document_revision != document.revision()
        {
            return Ok(CommandOutput {
                status: CommandStatus::Error("stale layout context".to_owned()),
                ..CommandOutput::complete()
            });
        }
        // The layout is authoritative for the current view's wrap setting.
        // Keeping this synchronized makes `:set wrap?` accurate even when the
        // host changed wrapping outside the Ex command line.
        self.wrap = context.wrap;
        if let InputEvent::Text(input) = &event {
            if self.mode == Mode::VisualBlock && self.pending == Pending::ReplaceVisualBlock {
                self.pending = Pending::None;
                let Some(grapheme) = one_extended_grapheme(input) else {
                    return Ok(CommandOutput::unsupported(
                        "visual block r expects one grapheme",
                    ));
                };
                let replacement =
                    RegisterValue::try_new(grapheme, RegisterKind::Characterwise, Vec::new())
                        .expect("pending Visual Block literal grapheme has valid break metadata");
                return self.apply_visual_block_replace(document, context, &replacement);
            }
        }
        if let InputEvent::Key(key) = event {
            if let Some(mut output) = self.try_handle_layout_key(document, key, context)? {
                self.finish_insert_normal_once(document, &mut output);
                return Ok(output);
            }
            let command_line_block_enter = key == Key::Enter
                && self.mode == Mode::CommandLine
                && self
                    .command_line_state
                    .as_ref()
                    .is_some_and(|state| state.return_mode == Mode::VisualBlock);
            let checkpoint = command_line_block_enter.then(|| self.clone());
            let mut output = self.handle_key(document, key)?;
            if command_line_block_enter
                && self.mode == Mode::VisualBlock
                && output.cursor_moved
                && output.status == CommandStatus::Complete
            {
                let position = match layout_position_for_offset(
                    context.snapshot,
                    self.cursor,
                    BoundaryAffinity::Downstream,
                ) {
                    Ok(position) => position,
                    Err(error) => {
                        *self = checkpoint.expect("block command line saved a checkpoint");
                        return Ok(layout_error(error));
                    }
                };
                let installed = self.install_visual_position(document, context.snapshot, position);
                if installed.status != CommandStatus::Complete {
                    *self = checkpoint.expect("block command line saved a checkpoint");
                    return Ok(installed);
                }
                output.merge(installed);
            }
            if !(command_line_block_enter && self.mode == Mode::VisualBlock) {
                self.finish_non_layout_dispatch(&output);
            }
            self.finish_insert_normal_once(document, &mut output);
            return Ok(output);
        }
        let mut output = self.dispatch_event(document, event)?;
        self.finish_non_layout_dispatch(&output);
        self.finish_insert_normal_once(document, &mut output);
        Ok(output)
    }

    fn rollback_failed_event(
        &mut self,
        document: &mut Document,
        checkpoint: Self,
        edit_group_depth: usize,
    ) {
        self.restore_failed_command(checkpoint);
        document.restore_edit_group_depth(edit_group_depth);
        self.position_revision = Some(document.revision());
    }

    pub(crate) fn restore_failed_command(&mut self, checkpoint: Self) {
        let cancel_pending = checkpoint.pending != Pending::None || checkpoint.register_pending;
        *self = checkpoint;
        if cancel_pending {
            self.clear_pending();
        } else {
            // A register prefix belongs to the semantic command which just
            // failed. Restoring it would silently retarget the next command.
            self.requested_register = None;
        }
    }

    fn record_event(&mut self, event: &InputEvent) {
        self.reopened_group_after_insert_normal_once = false;
        let stops_recording = self.recording.is_some()
            && self.mode == Mode::Normal
            && self.pending == Pending::None
            && *event == InputEvent::Key(Key::Char('q'));
        if self.compound_replay_depth == 0 && !stops_recording {
            if let Some((_, events)) = self.recording.as_mut() {
                events.push(event.clone());
            }
        }
    }

    fn dispatch_event(
        &mut self,
        document: &mut Document,
        event: InputEvent,
    ) -> Result<CommandOutput, DocumentError> {
        match event {
            InputEvent::Text(text) => self.handle_text(document, text),
            InputEvent::Key(key) => self.handle_key(document, key),
        }
    }

    /// Execute a command transition which is proven not to touch document
    /// state. This is shared by immutable planning and the compatibility
    /// executor so their observable parser/controller behavior stays exact.
    fn try_handle_controller_only_event(
        &mut self,
        document: &Document,
        event: &InputEvent,
    ) -> Option<CommandOutput> {
        match event {
            InputEvent::Text(input) => self.try_handle_controller_only_text(document, input),
            InputEvent::Key(key) => self.try_handle_controller_only_key(document, *key),
        }
    }

    fn try_handle_controller_only_text(
        &mut self,
        document: &Document,
        input: &str,
    ) -> Option<CommandOutput> {
        if self.mode == Mode::CommandLine {
            if let Some(state) = self.command_line_state.as_mut() {
                state.buffer.insert(input);
            }
            return Some(CommandOutput::pending());
        }

        let Pending::Find {
            forward,
            till,
            count,
        } = self.pending
        else {
            return None;
        };
        if !matches!(
            self.mode,
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
        ) {
            return None;
        }

        self.pending = Pending::None;
        let Some(grapheme) = one_extended_grapheme(input) else {
            return Some(CommandOutput::unsupported(
                "pending text command expects one grapheme",
            ));
        };
        Some(self.execute_find(
            document,
            FindState {
                needle: grapheme.to_owned(),
                forward,
                till,
            },
            count,
            true,
        ))
    }

    fn resolved_clipboard_copy_key(&self, key: Key) -> Key {
        let active = self.requested_register == Some('*') && self.pending == Pending::None
            || matches!(self.pending, Pending::Operator(PendingOperator { operator: Operator::Yank, register: Some('*'), .. }));
        if key == Key::Char('c') && active && !self.register_pending
            && matches!(self.mode, Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock) {
            Key::Char('y')
        } else { key }
    }

    fn clipboard_copy_alias_key(&mut self, key: Key) -> Key {
        let resolved = self.resolved_clipboard_copy_key(key);
        if resolved != key { self.clipboard_copy_as_seen = true; }
        resolved
    }

    fn try_handle_controller_only_key(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        let key = self.clipboard_copy_alias_key(key);
        if matches!(key, Key::DocumentStart | Key::DocumentEnd)
            && matches!(self.mode, Mode::Normal | Mode::VisualCharacter | Mode::VisualLine)
            && self.pending == Pending::None
            && !self.register_pending
        {
            return Some(self.move_to_document_edge(document, key == Key::DocumentEnd));
        }
        match self.mode {
            Mode::CommandLine => self.try_handle_controller_only_command_line_key(key),
            Mode::Normal => self.try_handle_controller_only_normal_key(document, key),
            Mode::VisualCharacter | Mode::VisualLine => {
                self.try_handle_controller_only_visual_key(document, key)
            }
            Mode::Insert | Mode::Replace | Mode::VisualBlock => None,
        }
    }

    fn try_handle_controller_only_command_line_key(&mut self, key: Key) -> Option<CommandOutput> {
        if let Some(output) = self.handle_filename_completion_key(key) {
            return Some(output);
        }
        if key == Key::Enter {
            return None;
        }
        let output = match key {
            Key::Escape => {
                self.mode = self
                    .command_line_state
                    .take()
                    .map_or(Mode::Normal, |state| state.return_mode);
                CommandOutput {
                    status: CommandStatus::Cancelled,
                    mode_changed: true,
                    ..CommandOutput::complete()
                }
            }
            Key::Backspace => {
                if let Some(state) = self.command_line_state.as_mut() {
                    command_line_backspace(&mut state.buffer);
                }
                CommandOutput::pending()
            }
            Key::Delete => {
                if let Some(state) = self.command_line_state.as_mut() {
                    command_line_delete(&mut state.buffer);
                }
                CommandOutput::pending()
            }
            Key::Left => {
                if let Some(state) = self.command_line_state.as_mut() {
                    state.buffer.selection_anchor = None;
                    state.buffer.cursor =
                        previous_grapheme_boundary(&state.buffer.input, state.buffer.cursor)
                            .unwrap_or(0);
                }
                CommandOutput::pending()
            }
            Key::Right => {
                if let Some(state) = self.command_line_state.as_mut() {
                    state.buffer.selection_anchor = None;
                    state.buffer.cursor =
                        next_grapheme_boundary(&state.buffer.input, state.buffer.cursor)
                            .unwrap_or(state.buffer.input.len());
                }
                CommandOutput::pending()
            }
            Key::Home | Key::DocumentStart | Key::Ctrl('b' | 'B') => {
                if let Some(state) = self.command_line_state.as_mut() {
                    state.buffer.selection_anchor = None;
                    state.buffer.cursor = 0;
                }
                CommandOutput::pending()
            }
            Key::End | Key::DocumentEnd | Key::Ctrl('e' | 'E') => {
                if let Some(state) = self.command_line_state.as_mut() {
                    state.buffer.selection_anchor = None;
                    state.buffer.cursor = state.buffer.input.len();
                }
                CommandOutput::pending()
            }
            Key::Up | Key::Ctrl('p' | 'P') => {
                self.navigate_command_line_history(true);
                CommandOutput::pending()
            }
            Key::Down | Key::Ctrl('n' | 'N') => {
                self.navigate_command_line_history(false);
                CommandOutput::pending()
            }
            Key::Ctrl('u' | 'U') => {
                if let Some(state) = self.command_line_state.as_mut() {
                    let cursor = state.buffer.cursor;
                    state.buffer.input.replace_range(..cursor, "");
                    state.buffer.selection_anchor = None;
                    state.buffer.cursor = 0;
                    state.buffer.detach_from_history();
                }
                CommandOutput::pending()
            }
            Key::Ctrl('w' | 'W') => {
                if let Some(state) = self.command_line_state.as_mut() {
                    command_line_delete_word(&mut state.buffer);
                }
                CommandOutput::pending()
            }
            Key::Char(character) => {
                if let Some(state) = self.command_line_state.as_mut() {
                    let mut encoded = [0; 4];
                    state.buffer.insert(character.encode_utf8(&mut encoded));
                }
                CommandOutput::pending()
            }
            _ => CommandOutput::unsupported(format!("command-line key {key:?}")),
        };
        Some(output)
    }

    fn try_handle_controller_only_normal_key(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        if key == Key::Escape {
            self.clear_pending();
            return Some(CommandOutput {
                status: CommandStatus::Cancelled,
                ..CommandOutput::complete()
            });
        }

        if self.register_pending {
            self.register_pending = false;
            return Some(match key {
                Key::Char(name) if is_valid_register(name) => {
                    self.requested_register = Some(name);
                    CommandOutput::pending()
                }
                _ => {
                    self.requested_register = None;
                    CommandOutput::unsupported("invalid register")
                }
            });
        }

        match self.pending {
            Pending::G {
                count,
                count_explicit,
                register,
            } => {
                let output = match key {
                    Key::Char('g') => self.goto_line(document, count),
                    Key::Char('e') => {
                        self.move_cursor(document, Motion::WordEndBackward(false), count)
                    }
                    Key::Char('E') => {
                        self.move_cursor(document, Motion::WordEndBackward(true), count)
                    }
                    Key::Char('_') => self.move_cursor(document, Motion::LastNonBlank, count),
                    Key::Char('*') => self.search_word_at_cursor(document, true, false, count),
                    Key::Char('#') => self.search_word_at_cursor(document, false, false, count),
                    Key::Char('v') => self.restore_visual(document),
                    Key::Char('~' | 'u' | 'U' | 'q' | 'w') => {
                        let Key::Char(command) = key else {
                            unreachable!()
                        };
                        self.requested_register = register;
                        self.start_operator(
                            match command {
                                '~' => Operator::ToggleCase,
                                'u' => Operator::Lowercase,
                                'U' => Operator::Uppercase,
                                'q' => Operator::Format,
                                'w' => Operator::FormatKeepCursor,
                                _ => unreachable!(),
                            },
                            count,
                            count_explicit,
                        );
                        CommandOutput::pending()
                    }
                    _ => return None,
                };
                if !matches!(key, Key::Char('~' | 'u' | 'U' | 'q' | 'w')) {
                    self.pending = Pending::None;
                }
                return Some(output);
            }
            Pending::Z { .. } => {
                if matches!(key, Key::Char('t' | 'z' | 'b')) {
                    return None;
                }
                self.pending = Pending::None;
                return Some(CommandOutput::unsupported(format!("z{key:?}")));
            }
            Pending::Operator(mut pending) => {
                if let Key::Char(digit @ '1'..='9') = key {
                    return Some(self.push_operator_motion_count(pending, digit));
                }
                if key == Key::Char('0') && pending.motion_count.is_some() {
                    return Some(self.push_operator_motion_count(pending, '0'));
                }
                if key == Key::Char('g') && !pending.g_prefix {
                    pending.g_prefix = true;
                    self.pending = Pending::Operator(pending);
                    return Some(CommandOutput::pending());
                }
                if !pending.g_prefix {
                    if let Key::Char(scope @ ('i' | 'a')) = key {
                        self.pending = Pending::TextObject {
                            operator: pending,
                            scope: if scope == 'i' {
                                TextObjectScope::Inner
                            } else {
                                TextObjectScope::Around
                            },
                        };
                        return Some(CommandOutput::pending());
                    }
                    if let Key::Char(command @ ('f' | 'F' | 't' | 'T')) = key {
                        self.pending = Pending::OperatorFind {
                            operator: pending,
                            forward: matches!(command, 'f' | 't'),
                            till: matches!(command, 't' | 'T'),
                        };
                        return Some(CommandOutput::pending());
                    }
                    if let Key::Char(command @ ('`' | '\'')) = key {
                        self.pending = Pending::OperatorMark {
                            operator: pending,
                            linewise: command == '\'',
                        };
                        return Some(CommandOutput::pending());
                    }
                    if let Key::Char(command @ ('/' | '?')) = key {
                        self.enter_operator_search(
                            if command == '/' {
                                SearchDirection::Forward
                            } else {
                                SearchDirection::Backward
                            },
                            pending,
                        );
                        return Some(CommandOutput {
                            mode_changed: true,
                            ..CommandOutput::pending()
                        });
                    }
                }
                return None;
            }
            Pending::Find {
                forward,
                till,
                count,
            } => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(character) => self.execute_find(
                        document,
                        FindState {
                            needle: character.to_string(),
                            forward,
                            till,
                        },
                        count,
                        true,
                    ),
                    _ => CommandOutput::unsupported("find expects text"),
                });
            }
            Pending::SetMark => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(name @ 'a'..='z') => {
                        self.marks.insert(name, self.cursor);
                        CommandOutput::complete()
                    }
                    _ => CommandOutput::unsupported("mark expects a-z"),
                });
            }
            Pending::JumpMark { linewise } => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(name @ 'a'..='z') => self.jump_to_mark(document, name, linewise),
                    _ => CommandOutput::unsupported("jump expects a-z"),
                });
            }
            Pending::MacroRecord => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(name @ 'a'..='z') => {
                        self.recording = Some((name, Vec::new()));
                        CommandOutput::complete()
                    }
                    Key::Char(name @ 'A'..='Z') => {
                        let normalized = name.to_ascii_lowercase();
                        let existing = self.registers.macro_events(normalized).unwrap_or_default();
                        self.recording = Some((normalized, existing));
                        CommandOutput::complete()
                    }
                    _ => CommandOutput::unsupported("macro register expects a-z or A-Z"),
                });
            }
            Pending::None => {}
            Pending::ReplaceCharacter { .. }
            | Pending::ReplaceVisual
            | Pending::ReplaceVisualBlock
            | Pending::OperatorFind { .. }
            | Pending::OperatorMark { .. }
            | Pending::TextObject { .. }
            | Pending::VisualTextObject { .. }
            | Pending::MacroPlay { .. } => return None,
        }

        if let Key::Char(digit @ '1'..='9') = key {
            return Some(self.push_count(digit));
        }
        if key == Key::Char('0') && self.count.is_some() {
            return Some(self.push_count('0'));
        }
        if let Some(output) = self.finish_overflowed_count() {
            return Some(output);
        }

        self.try_handle_controller_only_normal_action(document, key)
    }

    fn try_handle_controller_only_normal_action(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        let explicit_count = self.count.take();
        let count = explicit_count.unwrap_or(1).max(1);
        let output = match key {
            Key::Char('"') => {
                self.register_pending = true;
                CommandOutput::pending()
            }
            Key::Char('g') => {
                let register = self.requested_register.take();
                self.pending = Pending::G {
                    count,
                    count_explicit: explicit_count.is_some(),
                    register,
                };
                CommandOutput::pending()
            }
            Key::Char('z') => {
                self.pending = Pending::Z {
                    count,
                    count_explicit: explicit_count.is_some(),
                };
                CommandOutput::pending()
            }
            Key::Char(character @ ('d' | 'c' | 'y' | '>' | '<' | '=')) => {
                self.start_operator(
                    match character {
                        'd' => Operator::Delete,
                        'c' => Operator::Change,
                        'y' => Operator::Yank,
                        '>' => Operator::Indent,
                        '<' => Operator::Outdent,
                        '=' => Operator::Reindent,
                        _ => unreachable!(),
                    },
                    count,
                    explicit_count.is_some(),
                );
                CommandOutput::pending()
            }
            Key::Char('r') => {
                let register = self.requested_register.take();
                self.pending = Pending::ReplaceCharacter { count, register };
                CommandOutput::pending()
            }
            Key::Char(command @ ('f' | 'F' | 't' | 'T')) => {
                self.pending = Pending::Find {
                    forward: matches!(command, 'f' | 't'),
                    till: matches!(command, 't' | 'T'),
                    count,
                };
                CommandOutput::pending()
            }
            Key::Char(';') => self.repeat_find(document, false, count),
            Key::Char(',') => self.repeat_find(document, true, count),
            Key::Char('/') => {
                self.enter_search(SearchDirection::Forward, count);
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                }
            }
            Key::Char('?') => {
                self.enter_search(SearchDirection::Backward, count);
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                }
            }
            Key::Char(':') => {
                self.enter_command_line(CommandLineKind::Ex);
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                }
            }
            Key::Char('n') => self.repeat_search(document, false, count),
            Key::Char('N') => self.repeat_search(document, true, count),
            Key::Char('v') => self.enter_visual(
                document,
                Mode::VisualCharacter,
                count,
                explicit_count.is_some(),
            ),
            Key::Char('V') => {
                self.enter_visual(document, Mode::VisualLine, count, explicit_count.is_some())
            }
            Key::Char('m') => {
                self.pending = Pending::SetMark;
                CommandOutput::pending()
            }
            Key::Char('`') => {
                self.pending = Pending::JumpMark { linewise: false };
                CommandOutput::pending()
            }
            Key::Char('\'') => {
                self.pending = Pending::JumpMark { linewise: true };
                CommandOutput::pending()
            }
            Key::Char('q') => {
                if let Some((name, events)) = self.recording.take() {
                    self.registers.set_macro(name, events);
                    CommandOutput::complete()
                } else {
                    self.pending = Pending::MacroRecord;
                    CommandOutput::pending()
                }
            }
            Key::Char('@') => {
                self.pending = Pending::MacroPlay { count };
                CommandOutput::pending()
            }
            Key::Ctrl('o' | 'O') => self.navigate_jump(document, false, count),
            Key::Ctrl('i' | 'I') => self.navigate_jump(document, true, count),
            Key::Char('h') | Key::Left | Key::Backspace => {
                self.move_cursor(document, Motion::Horizontal(-1), count)
            }
            Key::Char('l') | Key::Right | Key::Char(' ') => {
                self.move_cursor(document, Motion::Horizontal(1), count)
            }
            Key::Char('j') | Key::Down => self.move_cursor(document, Motion::Vertical(1), count),
            Key::Char('k') | Key::Up => self.move_cursor(document, Motion::Vertical(-1), count),
            Key::Char('+') | Key::Enter => {
                self.move_cursor(document, Motion::LineOffsetFirstNonBlank(1), count)
            }
            Key::Char('-') => {
                self.move_cursor(document, Motion::LineOffsetFirstNonBlank(-1), count)
            }
            Key::Char('0') | Key::Home => self.move_cursor(document, Motion::LineStart, 1),
            Key::Char('^') => self.move_cursor(document, Motion::FirstNonBlank, 1),
            Key::Char('$') | Key::End => self.move_cursor(document, Motion::LineEnd, count),
            Key::Char('|') => self.move_cursor(document, Motion::Column(count), 1),
            Key::Char('w') => self.move_cursor(document, Motion::WordForward(false), count),
            Key::Char('W') => self.move_cursor(document, Motion::WordForward(true), count),
            Key::Char('e') => self.move_cursor(document, Motion::WordEnd(false), count),
            Key::Char('E') => self.move_cursor(document, Motion::WordEnd(true), count),
            Key::Char('b') => self.move_cursor(document, Motion::WordBackward(false), count),
            Key::Char('B') => self.move_cursor(document, Motion::WordBackward(true), count),
            Key::Char('(') => self.move_cursor_as_jump(document, Motion::Sentence(false), count),
            Key::Char(')') => self.move_cursor_as_jump(document, Motion::Sentence(true), count),
            Key::Char('{') => self.move_cursor_as_jump(document, Motion::Paragraph(false), count),
            Key::Char('}') => self.move_cursor_as_jump(document, Motion::Paragraph(true), count),
            Key::Char('%') => {
                if let Some(percent) = explicit_count {
                    let lines = document.hard_line_snapshot();
                    let Some(line) = percentage_line(&lines, percent) else {
                        return Some(invalid_percentage(percent));
                    };
                    self.goto_line(document, line)
                } else {
                    self.match_pair_motion(document)
                }
            }
            Key::Char('*') => self.search_word_at_cursor(document, true, true, count),
            Key::Char('#') => self.search_word_at_cursor(document, false, true, count),
            Key::Char('G') => self.goto_line(
                document,
                explicit_count.unwrap_or_else(|| document.line_count()),
            ),
            _ => {
                self.count = explicit_count;
                return None;
            }
        };
        Some(output)
    }

    fn try_handle_controller_only_visual_key(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        if key == Key::Escape && self.visual_command_is_pending() {
            self.clear_pending();
            return Some(CommandOutput {
                status: CommandStatus::Cancelled,
                ..CommandOutput::complete()
            });
        }
        if key == Key::Escape {
            self.leave_visual();
            return Some(CommandOutput {
                status: CommandStatus::Cancelled,
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        if self.register_pending {
            self.register_pending = false;
            return Some(match key {
                Key::Char(name) if is_valid_register(name) => {
                    self.requested_register = Some(name);
                    CommandOutput::pending()
                }
                _ => {
                    self.requested_register = None;
                    CommandOutput::unsupported("invalid register")
                }
            });
        }

        match self.pending {
            Pending::Find {
                forward,
                till,
                count,
            } => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(character) => self.execute_find(
                        document,
                        FindState {
                            needle: character.to_string(),
                            forward,
                            till,
                        },
                        count,
                        true,
                    ),
                    _ => CommandOutput::unsupported("find expects text"),
                });
            }
            Pending::G { count, .. } => {
                let output = match key {
                    Key::Char('e') => {
                        self.move_cursor(document, Motion::WordEndBackward(false), count)
                    }
                    Key::Char('E') => {
                        self.move_cursor(document, Motion::WordEndBackward(true), count)
                    }
                    Key::Char('g') => self.goto_line(document, count),
                    Key::Char('_') => self.move_cursor(document, Motion::LastNonBlank, count),
                    Key::Char('*') => self.search_word_at_cursor(document, true, false, count),
                    Key::Char('#') => self.search_word_at_cursor(document, false, false, count),
                    Key::Char('v') => self.exchange_visual(document),
                    _ => return None,
                };
                self.pending = Pending::None;
                return Some(output);
            }
            Pending::VisualTextObject { scope, count } => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(key) => self.select_visual_text_object(document, scope, key, count),
                    _ => CommandOutput::unsupported("text object expects a key"),
                });
            }
            Pending::JumpMark { linewise } => {
                self.pending = Pending::None;
                return Some(match key {
                    Key::Char(name @ 'a'..='z') => self.jump_to_mark(document, name, linewise),
                    _ => CommandOutput::unsupported("jump expects a-z"),
                });
            }
            Pending::None => {}
            _ => return None,
        }

        if (key == Key::Char('v') && self.mode == Mode::VisualCharacter)
            || (key == Key::Char('V') && self.mode == Mode::VisualLine)
        {
            self.leave_visual();
            return Some(CommandOutput {
                status: CommandStatus::Cancelled,
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        if let Key::Char(digit @ '1'..='9') = key {
            return Some(self.push_count(digit));
        }
        if key == Key::Char('0') && self.count.is_some() {
            return Some(self.push_count('0'));
        }
        if let Some(output) = self.finish_overflowed_count() {
            return Some(output);
        }

        self.try_handle_controller_only_visual_action(document, key)
    }

    fn try_handle_controller_only_visual_action(
        &mut self,
        document: &Document,
        key: Key,
    ) -> Option<CommandOutput> {
        let explicit_count = self.count.take();
        let count = explicit_count.unwrap_or(1).max(1);
        let output = match key {
            Key::Char(':') => self.enter_visual_ex(document),
            Key::Char('"') => {
                self.register_pending = true;
                CommandOutput::pending()
            }
            Key::Char('g') => {
                self.pending = Pending::G {
                    count,
                    count_explicit: explicit_count.is_some(),
                    register: None,
                };
                CommandOutput::pending()
            }
            Key::Char('z') => {
                self.pending = Pending::Z {
                    count,
                    count_explicit: explicit_count.is_some(),
                };
                CommandOutput::pending()
            }
            Key::Char(scope @ ('i' | 'a')) => {
                self.pending = Pending::VisualTextObject {
                    scope: if scope == 'i' {
                        TextObjectScope::Inner
                    } else {
                        TextObjectScope::Around
                    },
                    count,
                };
                CommandOutput::pending()
            }
            Key::Char(command @ ('f' | 'F' | 't' | 'T')) => {
                self.pending = Pending::Find {
                    forward: matches!(command, 'f' | 't'),
                    till: matches!(command, 't' | 'T'),
                    count,
                };
                CommandOutput::pending()
            }
            Key::Char(';') => self.repeat_find(document, false, count),
            Key::Char(',') => self.repeat_find(document, true, count),
            Key::Char('/') => {
                self.enter_search(SearchDirection::Forward, count);
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                }
            }
            Key::Char('?') => {
                self.enter_search(SearchDirection::Backward, count);
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                }
            }
            Key::Char('n') => self.repeat_search(document, false, count),
            Key::Char('N') => self.repeat_search(document, true, count),
            Key::Char('*') => self.search_word_at_cursor(document, true, true, count),
            Key::Char('#') => self.search_word_at_cursor(document, false, true, count),
            Key::Char('`') => {
                self.pending = Pending::JumpMark { linewise: false };
                CommandOutput::pending()
            }
            Key::Char('\'') => {
                self.pending = Pending::JumpMark { linewise: true };
                CommandOutput::pending()
            }
            Key::Char('o' | 'O') => {
                if let Some(anchor) = self.visual_anchor.as_mut() {
                    std::mem::swap(anchor, &mut self.cursor);
                }
                CommandOutput {
                    cursor_moved: true,
                    ..CommandOutput::complete()
                }
            }
            Key::Char('v') => {
                self.mode = Mode::VisualCharacter;
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::complete()
                }
            }
            Key::Char('V') => {
                self.mode = Mode::VisualLine;
                CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::complete()
                }
            }
            Key::Char('r') => {
                self.pending = Pending::ReplaceVisual;
                CommandOutput::pending()
            }
            Key::Char('h') | Key::Left => {
                self.move_cursor(document, Motion::Horizontal(-1), count)
            }
            Key::Char('l') | Key::Right | Key::Char(' ') => {
                self.move_cursor(document, Motion::Horizontal(1), count)
            }
            Key::Char('j') | Key::Down => self.move_cursor(document, Motion::Vertical(1), count),
            Key::Char('k') | Key::Up => self.move_cursor(document, Motion::Vertical(-1), count),
            Key::Char('+') | Key::Enter => {
                self.move_cursor(document, Motion::LineOffsetFirstNonBlank(1), count)
            }
            Key::Char('-') => {
                self.move_cursor(document, Motion::LineOffsetFirstNonBlank(-1), count)
            }
            Key::Char('0') | Key::Home => self.move_cursor(document, Motion::LineStart, 1),
            Key::Char('^') => self.move_cursor(document, Motion::FirstNonBlank, 1),
            Key::Char('$') | Key::End => self.move_cursor(document, Motion::LineEnd, count),
            Key::Char('|') => self.move_cursor(document, Motion::Column(count), 1),
            Key::Char('w') => self.move_cursor(document, Motion::WordForward(false), count),
            Key::Char('W') => self.move_cursor(document, Motion::WordForward(true), count),
            Key::Char('e') => self.move_cursor(document, Motion::WordEnd(false), count),
            Key::Char('E') => self.move_cursor(document, Motion::WordEnd(true), count),
            Key::Char('b') => self.move_cursor(document, Motion::WordBackward(false), count),
            Key::Char('B') => self.move_cursor(document, Motion::WordBackward(true), count),
            Key::Char('(') => self.move_cursor_as_jump(document, Motion::Sentence(false), count),
            Key::Char(')') => self.move_cursor_as_jump(document, Motion::Sentence(true), count),
            Key::Char('{') => self.move_cursor_as_jump(document, Motion::Paragraph(false), count),
            Key::Char('}') => self.move_cursor_as_jump(document, Motion::Paragraph(true), count),
            Key::Char('%') => {
                if let Some(percent) = explicit_count {
                    let lines = document.hard_line_snapshot();
                    let Some(line) = percentage_line(&lines, percent) else {
                        return Some(invalid_percentage(percent));
                    };
                    self.goto_line(document, line)
                } else {
                    self.match_pair_motion(document)
                }
            }
            Key::Char('G') => self.goto_line(
                document,
                explicit_count.unwrap_or_else(|| document.line_count()),
            ),
            Key::Ctrl('o' | 'O') => self.navigate_jump(document, false, count),
            Key::Ctrl('i' | 'I') => self.navigate_jump(document, true, count),
            _ => {
                self.count = explicit_count;
                return None;
            }
        };
        Some(output)
    }

    fn finish_non_layout_dispatch(&mut self, output: &CommandOutput) {
        if output.cursor_moved || output.document_changed {
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.visual_position = None;
            self.desired_x = None;
        }
    }

    /// A Normal/Visual register prefix applies to exactly one complete
    /// command, even when that command has no register operand. Keep it while
    /// a syntactically valid multi-key command is pending, then discard it at
    /// the same completion boundary as the command itself.
    fn finish_explicit_register_prefix(&mut self, output: &CommandOutput) {
        if output.status != CommandStatus::Pending && self.pending == Pending::None && !self.register_pending {
            self.clipboard_copy_as_seen = false;
        }
        if self.requested_register.is_some()
            && output.status != CommandStatus::Pending
            && self.pending == Pending::None
            && !self.register_pending
            && self.mode != Mode::CommandLine
        {
            self.requested_register = None;
        }
    }

    fn finish_insert_normal_once(&mut self, document: &mut Document, output: &mut CommandOutput) {
        if self.ctrl_o_just_started {
            self.ctrl_o_just_started = false;
            return;
        }
        // `@` and `:normal` are one Normal command for Insert Ctrl-O. Their
        // coordinator-owned child events must not return to Insert midway
        // through the replay.
        if self.compound_replay_depth > 0
            || (self.plan_compound_replay && self.pending_replay.is_some())
        {
            return;
        }
        let Some(return_mode) = self.insert_normal_once else {
            return;
        };
        if output.status == CommandStatus::Pending
            || self.pending != Pending::None
            || self.register_pending
            || self.mode == Mode::CommandLine
        {
            return;
        }
        self.insert_normal_once = None;
        if self.mode == Mode::Normal {
            self.mode = return_mode;
            document.begin_edit_group();
            self.reopened_group_after_insert_normal_once = true;
            if let Some(session) = self.insert_session.as_mut() {
                if output.document_changed && self.last_repeat.is_some() {
                    session.preserve_normal_repeat = true;
                }
                session.unit_floor = self.cursor;
            }
            output.mode_changed = true;
        }
    }

    fn try_handle_layout_key(
        &mut self,
        document: &mut Document,
        key: Key,
        context: &mut LayoutCommandContext<'_>,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        let key = self.clipboard_copy_alias_key(key);
        if let Some(output) = self.try_mode_line_key(document, key)? {
            return Ok(Some(output));
        }
        if self.mode == Mode::CommandLine {
            return Ok(None);
        }
        if self.visual_block_insert.is_some() {
            // Deferred block insertion has no live layout overlay. Its key
            // grammar is handled by the edit-mode dispatcher below.
            return Ok(None);
        }
        if self.mode == Mode::VisualBlock {
            return self
                .handle_visual_block_layout_key(document, key, context)
                .map(Some);
        }
        if self.count_overflowed && !matches!(key, Key::Char('0'..='9') | Key::Escape) {
            return Ok(self.finish_overflowed_count());
        }
        if self.register_pending {
            return Ok(None);
        }
        if self.mode == Mode::Normal
            && self.count.is_some()
            && matches!(key, Key::Char('v' | 'V') | Key::Ctrl('v' | 'V' | 'q' | 'Q'))
        {
            if let Some(memory) = self.last_visual {
                let count = self.count.take().unwrap_or(1).max(1);
                return Ok(Some(self.enter_scaled_previous_visual_with_layout(
                    document, context, memory, count,
                )));
            }
        }
        if matches!(key, Key::Ctrl('v' | 'V' | 'q' | 'Q'))
            && matches!(
                self.mode,
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
            )
        {
            let count = self.count.take().unwrap_or(1).max(1);
            let mut output = self.enter_visual_block(document, context);
            if output.status == CommandStatus::Complete && count > 1 {
                output.merge(self.move_visual_block_horizontal(document, context, count - 1, true));
            }
            return Ok(Some(output));
        }

        if matches!(
            self.mode,
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
        ) {
            if let Pending::Operator(pending) = self.pending {
                if pending.g_prefix && matches!(key, Key::Char('j' | 'k' | '0' | '^' | '$')) {
                    self.pending = Pending::None;
                    let count = match effective_operator_count(pending) {
                        Ok(count) => count,
                        Err(error) => return Ok(Some(CommandOutput::count_error(error))),
                    };
                    let extent =
                        match self.resolve_layout_operator_motion(document, context, key, count) {
                            Ok(extent) => extent,
                            Err(error) => return Ok(Some(layout_error(error))),
                        };
                    let deletion_class =
                        ordinary_deletion_class(&document.hard_line_snapshot(), &extent);
                    let output = self.apply_operator(
                        document,
                        pending.operator,
                        extent,
                        pending.register,
                        deletion_class,
                    )?;
                    if output.document_changed {
                        self.boundary_affinity = BoundaryAffinity::Downstream;
                        self.visual_position = None;
                        self.desired_x = None;
                    }
                    return Ok(Some(output));
                }
                if !pending.g_prefix && matches!(key, Key::Char('H' | 'M' | 'L')) {
                    let count = match effective_operator_count(pending) {
                        Ok(count) => count,
                        Err(error) => return Ok(Some(CommandOutput::count_error(error))),
                    };
                    let target = match key {
                        Key::Char('H') => ViewportLine::Top,
                        Key::Char('M') => ViewportLine::Middle,
                        Key::Char('L') => ViewportLine::Bottom,
                        _ => unreachable!("caller filters viewport-line motions"),
                    };
                    let (extent, destination, selected_line_count) = match self
                        .resolve_viewport_operator_motion(document, context, target, count)
                    {
                        Ok(resolved) => resolved,
                        // Keep the complete operator state intact. A
                        // bounded coordinator retry can then execute the
                        // same command once the requested layout exists;
                        // invalid/stale layout likewise cannot consume a
                        // register-prefixed operator halfway through.
                        Err(error) => return Ok(Some(layout_error(error))),
                    };
                    self.pending = Pending::None;
                    let origin = self.cursor;
                    let mut output = self.apply_operator_target_with_jump(
                        document,
                        pending.operator,
                        extent,
                        pending.register,
                        OperatorTarget {
                            repeat: RepeatTarget::Lines,
                            count: selected_line_count,
                            jump_destination: Some(destination),
                        },
                    )?;
                    output.cursor_moved |= self.cursor != origin;
                    if output.cursor_moved || output.document_changed {
                        self.boundary_affinity = BoundaryAffinity::Downstream;
                        self.visual_position = None;
                        self.desired_x = None;
                    }
                    return Ok(Some(output));
                }
            }
            if let Pending::G {
                count, register, ..
            } = self.pending
            {
                if self.mode == Mode::Normal
                    && matches!(key, Key::Char('p' | 'P'))
                    && self.register_is_blockwise(document, register)
                {
                    self.pending = Pending::None;
                    self.requested_register = register;
                    let before = key == Key::Char('P');
                    return self
                        .normal_block_paste(document, context, before, true, count)
                        .map(Some);
                }
                if key == Key::Char('v')
                    && self
                        .last_visual
                        .is_some_and(|memory| memory.mode == Mode::VisualBlock)
                {
                    let pending = self.pending;
                    self.pending = Pending::None;
                    let output = if self.mode == Mode::Normal {
                        self.restore_visual_with_layout(document, context)
                    } else {
                        self.exchange_visual_with_layout(document, context)
                    };
                    if matches!(output.status, CommandStatus::NeedsMoreLayout(_)) {
                        self.pending = pending;
                    }
                    return Ok(Some(output));
                }
                let output = match key {
                    Key::Char('j') => self.move_layout_rows(document, context, count, true),
                    Key::Char('k') => self.move_layout_rows(document, context, count, false),
                    Key::Char('0') => {
                        self.move_to_visual_row_edge(document, context, RowEdge::Start)
                    }
                    Key::Char('^') => {
                        self.move_to_visual_row_edge(document, context, RowEdge::FirstNonBlank)
                    }
                    Key::Char('$') => self.move_to_counted_visual_row_end(document, context, count),
                    _ => return Ok(None),
                };
                self.pending = Pending::None;
                return Ok(Some(output));
            }

            if let Pending::Z {
                count,
                count_explicit,
            } = self.pending
            {
                let alignment = match key {
                    Key::Char('t') => ViewportAlignment::Top,
                    Key::Char('z') => ViewportAlignment::Middle,
                    Key::Char('b') => ViewportAlignment::Bottom,
                    _ => return Ok(None),
                };
                self.pending = Pending::None;
                return Ok(Some(self.align_counted_row(
                    document,
                    context,
                    alignment,
                    count,
                    count_explicit,
                )));
            }
        }

        if self.pending != Pending::None {
            return Ok(None);
        }

        Ok(match self.mode {
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine => {
                let explicit_count = self.count;
                let count = explicit_count.unwrap_or(1).max(1);
                let output = match key {
                    Key::Char('p' | 'P')
                        if self.mode == Mode::Normal
                            && self.register_is_blockwise(document, self.requested_register) =>
                    {
                        self.count = None;
                        let before = key == Key::Char('P');
                        return self
                            .normal_block_paste(document, context, before, false, count)
                            .map(Some);
                    }
                    Key::Char('.')
                        if self.mode == Mode::Normal && self.repeat_is_visual_block_change() =>
                    {
                        // Counts supplied to a Visual repeat are consumed but
                        // intentionally ignored; the recorded block geometry
                        // and source command application count are authoritative.
                        self.count.take();
                        return self
                            .repeat_visual_block_with_layout(document, context)
                            .map(Some);
                    }
                    Key::Char('.')
                        if self.mode == Mode::Normal
                            && self.repeat_is_blockwise_paste(document) =>
                    {
                        let override_count = self.count.take();
                        return self
                            .repeat_block_paste_with_layout(document, context, override_count)
                            .map(Some);
                    }
                    Key::Char('j') | Key::Down => {
                        self.count = None;
                        self.move_layout_rows(document, context, count, true)
                    }
                    Key::Char('k') | Key::Up => {
                        self.count = None;
                        self.move_layout_rows(document, context, count, false)
                    }
                    Key::Char('H') => {
                        self.count = None;
                        self.move_to_viewport_line(document, context, ViewportLine::Top, count)
                    }
                    Key::Char('M') => {
                        self.count = None;
                        self.move_to_viewport_line(document, context, ViewportLine::Middle, count)
                    }
                    Key::Char('L') => {
                        self.count = None;
                        self.move_to_viewport_line(document, context, ViewportLine::Bottom, count)
                    }
                    Key::PageDown | Key::Ctrl('f' | 'F') => {
                        self.count = None;
                        self.execute_screen_motion(document, context, ScreenMotion::PageDown, count)
                    }
                    Key::PageUp | Key::Ctrl('b' | 'B') => {
                        self.count = None;
                        self.execute_screen_motion(document, context, ScreenMotion::PageUp, count)
                    }
                    Key::Ctrl('d' | 'D') => {
                        self.count = None;
                        let rows = self.half_page_scroll_amount(explicit_count);
                        self.execute_screen_motion(
                            document,
                            context,
                            ScreenMotion::HalfPageDown,
                            rows,
                        )
                    }
                    Key::Ctrl('u' | 'U') => {
                        self.count = None;
                        let rows = self.half_page_scroll_amount(explicit_count);
                        self.execute_screen_motion(
                            document,
                            context,
                            ScreenMotion::HalfPageUp,
                            rows,
                        )
                    }
                    Key::Ctrl('e' | 'E') => {
                        self.count = None;
                        self.execute_screen_motion(
                            document,
                            context,
                            ScreenMotion::ScrollDown,
                            count,
                        )
                    }
                    Key::Ctrl('y' | 'Y') => {
                        self.count = None;
                        self.execute_screen_motion(document, context, ScreenMotion::ScrollUp, count)
                    }
                    _ => return Ok(None),
                };
                self.requested_register = None;
                Some(output)
            }
            Mode::Insert | Mode::Replace => {
                let Some(motion) = (match key {
                    Key::Up => Some(InsertLayoutMotion::Rows(false)),
                    Key::Down => Some(InsertLayoutMotion::Rows(true)),
                    Key::PageUp => Some(InsertLayoutMotion::Screen(ScreenMotion::PageUp)),
                    Key::PageDown => Some(InsertLayoutMotion::Screen(ScreenMotion::PageDown)),
                    _ => None,
                }) else {
                    return Ok(None);
                };
                Some(self.move_in_insert_mode(document, context, motion))
            }
            Mode::VisualBlock => unreachable!("visual block handled above"),
            Mode::CommandLine => None,
        })
    }

    fn move_layout_rows(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        count: usize,
        down: bool,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let result = if down {
            gj(context.snapshot, current, count, self.desired_x)
        } else {
            gk(context.snapshot, current, count, self.desired_x)
        };
        match result {
            Ok(result) => {
                let retain_line_end = matches!(
                    self.mode,
                    Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
                ) && self.visual_to_line_end;
                let position = if retain_line_end {
                    match g_dollar_for_document(document, context.snapshot, result.position) {
                        Ok(position) => position,
                        Err(error) => return layout_error(error),
                    }
                } else {
                    result.position
                };
                let output = self.install_visual_position(document, context.snapshot, position);
                self.desired_x = if retain_line_end {
                    None
                } else {
                    Some(result.desired_x)
                };
                self.preferred_column = None;
                output
            }
            Err(error) => layout_error(error),
        }
    }

    fn resolve_layout_operator_motion(
        &self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        key: Key,
        count: usize,
    ) -> Result<MotionExtent, LayoutMotionError> {
        let origin = self.cursor.min(document.projection().text_tree().byte_len());
        let current = self.current_visual_position(context.snapshot)?;
        let destination = match key {
            Key::Char('j') => {
                let result = gj(context.snapshot, current, count, self.desired_x)?;
                normal_offset_for_visual(
                    document.text(),
                    &document.hard_line_snapshot(),
                    context.snapshot,
                    result.position,
                )
            }
            Key::Char('k') => {
                let result = gk(context.snapshot, current, count, self.desired_x)?;
                normal_offset_for_visual(
                    document.text(),
                    &document.hard_line_snapshot(),
                    context.snapshot,
                    result.position,
                )
            }
            Key::Char('0') => g0(context.snapshot, current)?.text_offset,
            Key::Char('^') => g_caret(context.snapshot, document.text(), current)?.text_offset,
            Key::Char('$') => {
                let row = if count > 1 {
                    gj(context.snapshot, current, count - 1, self.desired_x)?.position
                } else {
                    current
                };
                g_dollar_for_document(document, context.snapshot, row)?.text_offset
            }
            _ => unreachable!("caller filters visual-row motions"),
        };
        let (start, end) = if destination < origin {
            (destination, origin)
        } else {
            (origin, destination)
        };
        Ok(MotionExtent {
            range: start..end,
            kind: MotionKind::Characterwise,
        })
    }

    fn resolve_viewport_operator_motion(
        &self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        target: ViewportLine,
        count: usize,
    ) -> Result<(MotionExtent, usize, usize), LayoutMotionError> {
        let destination = viewport_line(
            context.snapshot,
            document.text(),
            context.viewport,
            target,
            count,
        )?;
        let lines = document.hard_line_snapshot();
        let origin = self.cursor.min(document.projection().text_tree().byte_len());
        let origin_line = lines
            .line_at_offset(origin)
            .map_err(|_| LayoutMotionError::TextDoesNotMatchLayout)?
            .index();
        let destination_line = lines
            .line_at_offset(destination.text_offset)
            .map_err(|_| LayoutMotionError::TextDoesNotMatchLayout)?
            .index();
        let first_line = origin_line.min(destination_line);
        let last_line = origin_line.max(destination_line);
        let first = nth_line_start(&lines, first_line.saturating_add(1));
        let last = nth_line_start(&lines, last_line.saturating_add(1));
        Ok((
            MotionExtent {
                range: first..line_range(&lines, last).end,
                kind: MotionKind::Linewise,
            },
            nth_line_start(&lines, destination_line.saturating_add(1)),
            last_line.saturating_sub(first_line).saturating_add(1),
        ))
    }

    fn enter_visual_block(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
    ) -> CommandOutput {
        let active_position = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let anchor_position = match self.visual_anchor {
            Some(anchor) if anchor != self.cursor => match layout_position_for_offset(
                context.snapshot,
                anchor,
                BoundaryAffinity::Downstream,
            ) {
                Ok(position) => position,
                Err(error) => return layout_error(error),
            },
            _ => active_position,
        };
        let anchor = match context
            .snapshot
            .caret_point(anchor_position.text_offset, anchor_position.affinity)
        {
            Ok(point) => point,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block anchor could not be resolved: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        let active = match context
            .snapshot
            .caret_point(active_position.text_offset, active_position.affinity)
        {
            Ok(point) => point,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block active caret could not be resolved: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        let anchor_x = match context.snapshot.caret_geometry(anchor) {
            Ok(geometry) => geometry.rect.x,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block anchor has no geometry: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        let active_x = match context.snapshot.caret_geometry(active) {
            Ok(geometry) => geometry.rect.x,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block active caret has no geometry: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        let mut selection = match BlockSelection::new(anchor, active, anchor_x, active_x) {
            Ok(selection) => selection,
            Err(error) => return visual_block_error(error),
        };
        if let Err(error) = selection.update_inclusive_rectangle(context.snapshot) {
            return visual_block_error(error);
        }
        let persistent = match active_visual_block_from_selection(document, &selection) {
            Ok(persistent) => persistent,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block anchors could not be captured: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        self.mode = Mode::VisualBlock;
        self.visual_to_line_end = false;
        self.visual_anchor = None;
        self.visual_block = Some(selection);
        self.active_visual_block = Some(persistent);
        self.visual_block_rebind_error = None;
        self.cursor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            context.snapshot,
            active_position,
        );
        self.boundary_affinity = active_position.affinity;
        self.visual_position = Some(active_position);
        self.clear_pending();
        CommandOutput {
            mode_changed: true,
            ..CommandOutput::complete()
        }
    }

    fn handle_visual_block_layout_key(
        &mut self,
        document: &mut Document,
        key: Key,
        context: &mut LayoutCommandContext<'_>,
    ) -> Result<CommandOutput, DocumentError> {
        let key = self.clipboard_copy_alias_key(key);
        if key == Key::Escape && self.visual_command_is_pending() {
            self.clear_pending();
            return Ok(CommandOutput {
                status: CommandStatus::Cancelled,
                ..CommandOutput::complete()
            });
        }
        if key == Key::Escape || matches!(key, Key::Ctrl('v' | 'V' | 'q' | 'Q')) {
            self.leave_visual_block();
            return Ok(CommandOutput {
                status: CommandStatus::Cancelled,
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        if self.register_pending {
            self.register_pending = false;
            return Ok(match key {
                Key::Char(name) if is_valid_register(name) => {
                    self.requested_register = Some(name);
                    CommandOutput::pending()
                }
                _ => {
                    self.requested_register = None;
                    CommandOutput::unsupported("invalid register")
                }
            });
        }
        if self.pending == Pending::ReplaceVisualBlock {
            self.pending = Pending::None;
            return match key {
                Key::Char(character) => self.apply_visual_block_replace(
                    document,
                    context,
                    &RegisterValue::characterwise(character.to_string()),
                ),
                Key::Enter => self.apply_visual_block_replace(
                    document,
                    context,
                    &RegisterValue::characterwise("\n"),
                ),
                Key::Tab => self.apply_visual_block_replace(
                    document,
                    context,
                    &RegisterValue::characterwise("\t"),
                ),
                _ => Ok(CommandOutput::unsupported("visual block r expects text")),
            };
        }
        if let Pending::Find {
            forward,
            till,
            count,
        } = self.pending
        {
            return match key {
                Key::Char(character) => Ok(self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.pending = Pending::None;
                        commands.execute_find(
                            document,
                            FindState {
                                needle: character.to_string(),
                                forward,
                                till,
                            },
                            count,
                            true,
                        )
                    },
                )),
                _ => {
                    self.pending = Pending::None;
                    Ok(CommandOutput::unsupported("find expects text"))
                }
            };
        }
        if let Pending::JumpMark { linewise } = self.pending {
            return match key {
                Key::Char(name @ 'a'..='z') => Ok(self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.pending = Pending::None;
                        commands.jump_to_mark(document, name, linewise)
                    },
                )),
                _ => {
                    self.pending = Pending::None;
                    Ok(CommandOutput::unsupported("jump expects a-z"))
                }
            };
        }
        if let Pending::G { count, .. } = self.pending {
            let pending = self.pending;
            self.pending = Pending::None;
            let output = match key {
                Key::Char('j') => self.move_layout_rows(document, context, count, true),
                Key::Char('k') => self.move_layout_rows(document, context, count, false),
                Key::Char('0') => self.move_to_visual_row_edge(document, context, RowEdge::Start),
                Key::Char('^') => {
                    self.move_to_visual_row_edge(document, context, RowEdge::FirstNonBlank)
                }
                Key::Char('$') => self.move_to_counted_visual_row_end(document, context, count),
                Key::Char('e') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.move_cursor(document, Motion::WordEndBackward(false), count)
                    },
                ),
                Key::Char('E') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.move_cursor(document, Motion::WordEndBackward(true), count)
                    },
                ),
                Key::Char('g') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| commands.goto_line(document, count),
                ),
                Key::Char('_') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.move_cursor(document, Motion::LastNonBlank, count)
                    },
                ),
                Key::Char('*') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.search_word_at_cursor(document, true, false, count)
                    },
                ),
                Key::Char('#') => self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| {
                        commands.search_word_at_cursor(document, false, false, count)
                    },
                ),
                Key::Char('~') => {
                    return self.apply_visual_block_operator(
                        document,
                        context,
                        Operator::ToggleCase,
                    )
                }
                Key::Char('u') => {
                    return self.apply_visual_block_operator(document, context, Operator::Lowercase)
                }
                Key::Char('U') => {
                    return self.apply_visual_block_operator(document, context, Operator::Uppercase)
                }
                Key::Char('q') => {
                    return self.apply_visual_block_operator(document, context, Operator::Format)
                }
                Key::Char('w') => {
                    return self.apply_visual_block_operator(
                        document,
                        context,
                        Operator::FormatKeepCursor,
                    )
                }
                Key::Char('J') => return self.visual_block_join(document, context, false),
                Key::Char('v') => self.exchange_visual_with_layout(document, context),
                _ => CommandOutput::unsupported(format!("visual block g{key:?}")),
            };
            if matches!(output.status, CommandStatus::NeedsMoreLayout(_)) {
                self.pending = pending;
            }
            return Ok(output);
        }
        if let Pending::Z {
            count,
            count_explicit,
        } = self.pending
        {
            let alignment = match key {
                Key::Char('t') => ViewportAlignment::Top,
                Key::Char('z') => ViewportAlignment::Middle,
                Key::Char('b') => ViewportAlignment::Bottom,
                _ => {
                    self.pending = Pending::None;
                    return Ok(CommandOutput::unsupported(format!("visual block z{key:?}")));
                }
            };
            let pending = self.pending;
            self.pending = Pending::None;
            let checkpoint = self.clone();
            let viewport = context.viewport;
            let mut output = if count_explicit {
                self.apply_visual_block_logical_motion(document, context, |commands, document| {
                    commands.goto_line(document, count)
                })
            } else {
                CommandOutput::complete()
            };
            if output.status == CommandStatus::Complete {
                output.merge(self.align_current_row(context, alignment));
            }
            if output.status != CommandStatus::Complete {
                *self = checkpoint;
                context.viewport = viewport;
                if matches!(output.status, CommandStatus::NeedsMoreLayout(_)) {
                    self.pending = pending;
                }
            }
            return Ok(output);
        }
        if key == Key::Char('"') {
            // Keep a count already typed before the register prefix. Vim also
            // accepts the more usual `"a3{operator}` order.
            self.register_pending = true;
            return Ok(CommandOutput::pending());
        }
        if let Key::Char(digit @ '1'..='9') = key {
            return Ok(self.push_count(digit));
        }
        if key == Key::Char('0') && self.count.is_some() {
            return Ok(self.push_count('0'));
        }
        if let Some(output) = self.finish_overflowed_count() {
            return Ok(output);
        }
        let explicit_count = self.count.take();
        let count = explicit_count.unwrap_or(1).max(1);
        match key {
            Key::Char(':') => Ok(self.enter_visual_ex(document)),
            Key::Char('g') => {
                self.pending = Pending::G {
                    count,
                    count_explicit: explicit_count.is_some(),
                    register: None,
                };
                Ok(CommandOutput::pending())
            }
            Key::Char('z') => {
                self.pending = Pending::Z {
                    count,
                    count_explicit: explicit_count.is_some(),
                };
                Ok(CommandOutput::pending())
            }
            Key::Char('f' | 'F' | 't' | 'T') => {
                let Key::Char(command) = key else {
                    unreachable!()
                };
                self.pending = Pending::Find {
                    forward: matches!(command, 'f' | 't'),
                    till: matches!(command, 't' | 'T'),
                    count,
                };
                Ok(CommandOutput::pending())
            }
            Key::Char(';') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.repeat_find(document, false, count),
            )),
            Key::Char(',') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.repeat_find(document, true, count),
            )),
            Key::Char('/') => {
                self.enter_search(SearchDirection::Forward, count);
                Ok(CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                })
            }
            Key::Char('?') => {
                self.enter_search(SearchDirection::Backward, count);
                Ok(CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                })
            }
            Key::Char('n') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.repeat_search(document, false, count),
            )),
            Key::Char('N') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.repeat_search(document, true, count),
            )),
            Key::Char('*') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.search_word_at_cursor(document, true, true, count),
            )),
            Key::Char('#') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.search_word_at_cursor(document, false, true, count),
            )),
            Key::Char('`') => {
                self.pending = Pending::JumpMark { linewise: false };
                Ok(CommandOutput::pending())
            }
            Key::Char('\'') => {
                self.pending = Pending::JumpMark { linewise: true };
                Ok(CommandOutput::pending())
            }
            Key::Char('h') | Key::Left => {
                Ok(self.move_visual_block_horizontal(document, context, count, false))
            }
            Key::Char('l') | Key::Right | Key::Char(' ') => {
                Ok(self.move_visual_block_horizontal(document, context, count, true))
            }
            Key::Char('j') | Key::Down => Ok(self.move_layout_rows(document, context, count, true)),
            Key::Char('k') | Key::Up => Ok(self.move_layout_rows(document, context, count, false)),
            Key::Char('0') | Key::Home => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_cursor(document, Motion::LineStart, 1),
            )),
            Key::DocumentStart | Key::DocumentEnd => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_to_document_edge(document, key == Key::DocumentEnd),
            )),
            Key::Char('^') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_cursor(document, Motion::FirstNonBlank, 1),
            )),
            Key::Char('$') | Key::End => {
                let output = self.apply_visual_block_logical_motion(
                    document,
                    context,
                    |commands, document| commands.move_cursor(document, Motion::LineEnd, count),
                );
                if output.status == CommandStatus::Complete {
                    self.visual_to_line_end = true;
                }
                Ok(output)
            }
            Key::Char('+') | Key::Enter => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::LineOffsetFirstNonBlank(1), count)
                },
            )),
            Key::Char('-') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::LineOffsetFirstNonBlank(-1), count)
                },
            )),
            Key::Char('|') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_cursor(document, Motion::Column(count), 1),
            )),
            Key::Char('w') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::WordForward(false), count)
                },
            )),
            Key::Char('W') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::WordForward(true), count)
                },
            )),
            Key::Char('e') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_cursor(document, Motion::WordEnd(false), count),
            )),
            Key::Char('E') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.move_cursor(document, Motion::WordEnd(true), count),
            )),
            Key::Char('b') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::WordBackward(false), count)
                },
            )),
            Key::Char('B') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor(document, Motion::WordBackward(true), count)
                },
            )),
            Key::Char('(') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor_as_jump(document, Motion::Sentence(false), count)
                },
            )),
            Key::Char(')') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor_as_jump(document, Motion::Sentence(true), count)
                },
            )),
            Key::Char('{') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor_as_jump(document, Motion::Paragraph(false), count)
                },
            )),
            Key::Char('}') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.move_cursor_as_jump(document, Motion::Paragraph(true), count)
                },
            )),
            Key::Char('%') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    if let Some(percent) = explicit_count {
                        let lines = document.hard_line_snapshot();
                        let Some(line) = percentage_line(&lines, percent) else {
                            return invalid_percentage(percent);
                        };
                        commands.goto_line(document, line)
                    } else {
                        commands.match_pair_motion(document)
                    }
                },
            )),
            Key::Char('G') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| {
                    commands.goto_line(
                        document,
                        explicit_count.unwrap_or_else(|| document.line_count()),
                    )
                },
            )),
            Key::Ctrl('o' | 'O') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.navigate_jump(document, false, count),
            )),
            Key::Ctrl('i' | 'I') => Ok(self.apply_visual_block_logical_motion(
                document,
                context,
                |commands, document| commands.navigate_jump(document, true, count),
            )),
            Key::Char('o') => Ok(self.swap_visual_block_endpoints(document, context, false)),
            Key::Char('O') => Ok(self.swap_visual_block_endpoints(document, context, true)),
            Key::Char('v') => Ok(self.transition_visual_block_to_linear(
                document,
                context.snapshot,
                Mode::VisualCharacter,
            )),
            Key::Char('V') => Ok(self.transition_visual_block_to_linear(
                document,
                context.snapshot,
                Mode::VisualLine,
            )),
            Key::Char('H') => {
                Ok(self.move_to_viewport_line(document, context, ViewportLine::Top, count))
            }
            Key::Char('M') => {
                Ok(self.move_to_viewport_line(document, context, ViewportLine::Middle, count))
            }
            Key::Char('L') => {
                Ok(self.move_to_viewport_line(document, context, ViewportLine::Bottom, count))
            }
            Key::PageDown | Key::Ctrl('f' | 'F') => {
                Ok(self.execute_screen_motion(document, context, ScreenMotion::PageDown, count))
            }
            Key::PageUp | Key::Ctrl('b' | 'B') => {
                Ok(self.execute_screen_motion(document, context, ScreenMotion::PageUp, count))
            }
            Key::Ctrl('d' | 'D') => {
                let rows = self.half_page_scroll_amount(explicit_count);
                Ok(self.execute_screen_motion(document, context, ScreenMotion::HalfPageDown, rows))
            }
            Key::Ctrl('u' | 'U') => {
                let rows = self.half_page_scroll_amount(explicit_count);
                Ok(self.execute_screen_motion(document, context, ScreenMotion::HalfPageUp, rows))
            }
            Key::Ctrl('e' | 'E') => {
                Ok(self.execute_screen_motion(document, context, ScreenMotion::ScrollDown, count))
            }
            Key::Ctrl('y' | 'Y') => {
                Ok(self.execute_screen_motion(document, context, ScreenMotion::ScrollUp, count))
            }
            Key::Char('r') => {
                self.pending = Pending::ReplaceVisualBlock;
                Ok(CommandOutput::pending())
            }
            Key::Char('y') => self.apply_visual_block_operator(document, context, Operator::Yank),
            Key::Char('d' | 'x') | Key::Backspace | Key::Delete => {
                self.apply_visual_block_operator(document, context, Operator::Delete)
            }
            Key::Char('~') => {
                self.apply_visual_block_operator(document, context, Operator::ToggleCase)
            }
            Key::Char('u') => {
                self.apply_visual_block_operator(document, context, Operator::Lowercase)
            }
            Key::Char('U') => {
                self.apply_visual_block_operator(document, context, Operator::Uppercase)
            }
            Key::Char('I') => self.begin_visual_block_insert(
                document,
                context,
                VisualBlockInsertKind::Insert,
                count,
            ),
            Key::Char('A') => self.begin_visual_block_insert(
                document,
                context,
                VisualBlockInsertKind::Append,
                count,
            ),
            Key::Char('c' | 's') => self.begin_visual_block_insert(
                document,
                context,
                VisualBlockInsertKind::Change,
                count,
            ),
            Key::Char('p') => self.visual_block_paste(document, context, false, count),
            Key::Char('P') => self.visual_block_paste(document, context, true, count),
            Key::Char('J') => self.visual_block_join(document, context, true),
            Key::Char('>') => {
                self.apply_visual_block_shift(document, context, Operator::Indent, count)
            }
            Key::Char('<') => {
                self.apply_visual_block_shift(document, context, Operator::Outdent, count)
            }
            Key::Char('=') => {
                self.apply_visual_block_shift(document, context, Operator::Reindent, count)
            }
            _ => Ok(CommandOutput::unsupported(format!(
                "visual block key {key:?}"
            ))),
        }
    }

    fn move_visual_block_horizontal(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        count: usize,
        forward: bool,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let Some(row) = context.snapshot.rows.iter().find(|row| {
            row.carets.iter().any(|caret| {
                caret.point.text_offset == current.text_offset
                    && caret.point.affinity == current.affinity
            })
        }) else {
            return layout_error(LayoutMotionError::PositionNotInLayout(current));
        };
        // A logical boundary commonly has both upstream and downstream caret
        // stops at the same x. Vim `h`/`l` counts selected character cells,
        // not those duplicate affinities, so navigate the visually ordered
        // shaped clusters and choose the affinity that associates the cursor
        // with that cluster's visual-left edge.
        let associates =
            |cluster: &crate::layout::PositionedCluster, position: VisualPosition| match position
                .affinity
            {
                BoundaryAffinity::Downstream => {
                    cluster.text_range.start <= position.text_offset
                        && position.text_offset < cluster.text_range.end
                }
                BoundaryAffinity::Upstream => {
                    cluster.text_range.start < position.text_offset
                        && position.text_offset <= cluster.text_range.end
                }
            };
        let Some(index) = row
            .clusters
            .iter()
            .position(|cluster| associates(cluster, current))
        else {
            // Empty visual rows have no character cell for horizontal motion.
            if row.clusters.is_empty() {
                return CommandOutput::complete();
            }
            return layout_error(LayoutMotionError::PositionNotInLayout(current));
        };
        let target = if forward {
            index.saturating_add(count).min(row.clusters.len() - 1)
        } else {
            index.saturating_sub(count)
        };
        let cluster = &row.clusters[target];
        let position = if cluster.bidi_level % 2 == 0 {
            VisualPosition {
                text_offset: cluster.text_range.start,
                affinity: BoundaryAffinity::Downstream,
            }
        } else {
            VisualPosition {
                text_offset: cluster.text_range.end,
                affinity: BoundaryAffinity::Upstream,
            }
        };
        self.desired_x = None;
        self.preferred_column = None;
        let output = self.install_visual_position(document, context.snapshot, position);
        if output.status == CommandStatus::Complete {
            self.visual_to_line_end = false;
        }
        output
    }

    fn swap_visual_block_endpoints(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        horizontal_only: bool,
    ) -> CommandOutput {
        let Some(selection) = self.visual_block.as_mut() else {
            return CommandOutput::unsupported("visual block selection is missing");
        };
        if horizontal_only {
            if let Err(error) = selection.swap_horizontal_corners(context.snapshot) {
                return visual_block_error(error);
            }
            self.visual_to_line_end = false;
        } else {
            std::mem::swap(&mut selection.anchor, &mut selection.active);
            std::mem::swap(&mut selection.anchor_x, &mut selection.active_x);
        }
        let position = VisualPosition {
            text_offset: selection.active.text_offset,
            affinity: selection.active.affinity,
        };
        let persistent = match active_visual_block_from_selection(document, selection) {
            Ok(persistent) => persistent,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block anchors could not be captured: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        self.cursor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            context.snapshot,
            position,
        );
        self.boundary_affinity = position.affinity;
        self.visual_position = Some(position);
        self.active_visual_block = Some(persistent);
        self.visual_block_rebind_error = None;
        CommandOutput {
            cursor_moved: true,
            ..CommandOutput::complete()
        }
    }

    fn resolved_visual_block(
        &self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
    ) -> Result<ResolvedBlockSelection, VisualBlockError> {
        let selection = self
            .visual_block
            .as_ref()
            .ok_or(VisualBlockError::EmptyLayout)?;
        if self.visual_to_line_end {
            resolve_block_selection_to_line_end(selection, context.snapshot, document.text())
        } else {
            resolve_block_selection(selection, context.snapshot, document.text())
        }
    }

    fn visual_block_repeat_shape(
        &self,
        resolved: &ResolvedBlockSelection,
    ) -> Result<VisualBlockRepeatShape, VisualBlockError> {
        let selection = self
            .visual_block
            .as_ref()
            .ok_or(VisualBlockError::EmptyLayout)?;
        // Store the distance between endpoint carets, not the inclusive outer
        // rectangle width. A one-cell block therefore has span zero and still
        // selects one wide destination cluster after fresh hit testing.
        let endpoint_x_span = (selection.active_x - selection.anchor_x).abs();
        if !endpoint_x_span.is_finite() {
            return Err(VisualBlockError::InvalidX);
        }
        Ok(VisualBlockRepeatShape {
            visual_row_count: resolved.rows.len().max(1),
            endpoint_x_span_bits: endpoint_x_span.to_bits(),
            to_line_end: self.visual_to_line_end,
        })
    }

    /// Restart the previous Visual operation from the current cursor using
    /// Vim's counted-Visual rule. The requested `v`/`V`/Ctrl-V spelling does
    /// not select the mode when a prior Visual operation exists; its recorded
    /// mode and shape do. A block's inclusive display width and visual-row
    /// height are both multiplied, then re-hit-tested in this exact layout.
    fn enter_scaled_previous_visual_with_layout(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        memory: VisualMemory,
        count: usize,
    ) -> CommandOutput {
        if memory.mode != Mode::VisualBlock {
            return self.enter_scaled_previous_visual(document, memory, count);
        }

        let checkpoint = self.clone();
        let installed = self.install_visual_memory_with_layout(document, context, memory);
        if installed.status != CommandStatus::Complete {
            *self = checkpoint;
            return installed;
        }
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => {
                *self = checkpoint;
                return visual_block_error(error);
            }
        };
        let prior_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => {
                *self = checkpoint;
                return visual_block_error(error);
            }
        };
        let prior_width = self
            .visual_block
            .as_ref()
            .map_or(0.0, |selection| selection.right_x() - selection.left_x());
        *self = checkpoint.clone();

        let count = count.max(1);
        let Some(visual_row_count) = prior_shape.visual_row_count.checked_mul(count) else {
            return CommandOutput::count_error(CountError::Overflow);
        };
        let endpoint_x_span = if prior_shape.to_line_end {
            prior_shape.endpoint_x_span()
        } else {
            let desired_width = prior_width * count as f32;
            if !desired_width.is_finite() {
                return CommandOutput::count_error(CountError::Overflow);
            }
            match self.endpoint_span_for_inclusive_block_width(context, desired_width) {
                Ok(span) => span,
                Err(output) => return output,
            }
        };
        let shape = VisualBlockRepeatShape {
            visual_row_count,
            endpoint_x_span_bits: endpoint_x_span.to_bits(),
            to_line_end: prior_shape.to_line_end,
        };
        let old_cursor = self.cursor;
        match self.install_visual_block_repeat_selection(document, context, shape) {
            Ok(()) => CommandOutput {
                cursor_moved: self.cursor != old_cursor,
                mode_changed: true,
                ..CommandOutput::complete()
            },
            Err(output) => {
                *self = checkpoint;
                output
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn endpoint_span_for_inclusive_block_width(
        &self,
        context: &LayoutCommandContext<'_>,
        desired_width: f32,
    ) -> Result<f32, CommandOutput> {
        let current = self
            .current_visual_position(context.snapshot)
            .map_err(layout_error)?;
        let anchor = context
            .snapshot
            .caret_point(current.text_offset, current.affinity)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "counted Visual Block anchor could not be resolved: {error:?}"
                )),
                ..CommandOutput::complete()
            })?;
        let anchor_x = context
            .snapshot
            .caret_geometry(anchor)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "counted Visual Block anchor has no geometry: {error:?}"
                )),
                ..CommandOutput::complete()
            })?
            .rect
            .x;
        let row = context
            .snapshot
            .rows
            .iter()
            .find(|row| row.carets.iter().any(|caret| caret.point == anchor))
            .ok_or_else(|| layout_error(LayoutMotionError::PositionNotInLayout(current)))?;

        let mut best: Option<(f32, f32)> = None;
        for caret in row
            .carets
            .iter()
            .filter(|caret| caret.x.is_finite() && caret.x >= anchor_x)
        {
            let mut selection = BlockSelection::new(anchor, caret.point, anchor_x, caret.x)
                .map_err(visual_block_error)?;
            selection
                .update_inclusive_rectangle(context.snapshot)
                .map_err(visual_block_error)?;
            let width = selection.right_x() - selection.left_x();
            let difference = (width - desired_width).abs();
            let span = caret.x - anchor_x;
            let replace = match best {
                None => true,
                Some((best_difference, best_span)) => {
                    difference < best_difference
                        || (difference == best_difference && span < best_span)
                }
            };
            if replace {
                best = Some((difference, span));
            }
        }
        best.map(|(_, span)| span)
            .ok_or_else(|| layout_error(LayoutMotionError::PositionNotInLayout(current)))
    }

    /// Materialize a recorded Visual Block shape at the current Normal cursor.
    /// Only row count and display-space intent survive in the repeat recipe;
    /// all caret identities and logical offsets come from this exact snapshot.
    #[allow(clippy::result_large_err)]
    fn install_visual_block_repeat_selection(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        shape: VisualBlockRepeatShape,
    ) -> Result<(), CommandOutput> {
        let current = self
            .current_visual_position(context.snapshot)
            .map_err(layout_error)?;
        let anchor = context
            .snapshot
            .caret_point(current.text_offset, current.affinity)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "Visual Block repeat anchor could not be resolved: {error:?}"
                )),
                ..CommandOutput::complete()
            })?;
        let anchor_x = context
            .snapshot
            .caret_geometry(anchor)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "Visual Block repeat anchor has no geometry: {error:?}"
                )),
                ..CommandOutput::complete()
            })?
            .rect
            .x;

        let target_x = anchor_x + shape.endpoint_x_span();
        if !target_x.is_finite() {
            return Err(visual_block_error(VisualBlockError::InvalidX));
        }

        let anchor_row = context
            .snapshot
            .rows
            .iter()
            .find(|row| row.carets.iter().any(|caret| caret.point == anchor))
            .ok_or_else(|| layout_error(LayoutMotionError::PositionNotInLayout(current)))?;
        let horizontal_caret = crate::layout::nearest_caret(anchor_row, target_x)
            .ok_or_else(|| layout_error(LayoutMotionError::PositionNotInLayout(current)))?;
        let horizontal_x = horizontal_caret.x;
        let mut horizontal_selection =
            BlockSelection::new(anchor, horizontal_caret.point, anchor_x, horizontal_x)
                .map_err(visual_block_error)?;
        horizontal_selection
            .update_inclusive_rectangle(context.snapshot)
            .map_err(visual_block_error)?;

        let active_position = if shape.visual_row_count > 1 {
            gj(
                context.snapshot,
                current,
                shape.visual_row_count - 1,
                Some(target_x),
            )
            .map_err(layout_error)?
            .position
        } else {
            VisualPosition {
                text_offset: horizontal_caret.point.text_offset,
                affinity: horizontal_caret.point.affinity,
            }
        };
        let active = context
            .snapshot
            .caret_point(active_position.text_offset, active_position.affinity)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "Visual Block repeat active endpoint could not be resolved: {error:?}"
                )),
                ..CommandOutput::complete()
            })?;
        let active_x = context
            .snapshot
            .caret_geometry(active)
            .map_err(|error| CommandOutput {
                status: CommandStatus::Error(format!(
                    "Visual Block repeat active endpoint has no geometry: {error:?}"
                )),
                ..CommandOutput::complete()
            })?
            .rect
            .x;
        let selection = BlockSelection::from_rectangle(
            anchor,
            active,
            anchor_x,
            active_x,
            horizontal_selection.left_x(),
            horizontal_selection.right_x(),
        )
        .map_err(visual_block_error)?;
        let persistent =
            active_visual_block_from_selection(document, &selection).map_err(|error| {
                CommandOutput {
                    status: CommandStatus::Error(format!(
                        "Visual Block repeat anchors could not be captured: {error:?}"
                    )),
                    ..CommandOutput::complete()
                }
            })?;

        self.mode = Mode::VisualBlock;
        self.visual_to_line_end = shape.to_line_end;
        self.visual_anchor = None;
        self.visual_block = Some(selection);
        self.active_visual_block = Some(persistent);
        self.visual_block_rebind_error = None;
        self.cursor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            context.snapshot,
            active_position,
        );
        self.boundary_affinity = active_position.affinity;
        self.visual_position = Some(active_position);
        self.desired_x = Some(anchor_x);
        self.preferred_column = None;
        self.clear_pending();
        Ok(())
    }

    fn begin_visual_block_insert(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        kind: VisualBlockInsertKind,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let requested_register = self.requested_register.take();
        if kind == VisualBlockInsertKind::Change {
            if let Err(output) = self.require_register_write(requested_register) {
                return Ok(output);
            }
        }
        let replaced = (kind == VisualBlockInsertKind::Change)
            .then(|| block_register_value(document, &resolved, requested_register));
        let mut rows = Vec::with_capacity(resolved.rows.len());
        for row in &resolved.rows {
            let ranges = row
                .ranges
                .iter()
                .filter(|range| !range.is_empty())
                .cloned()
                .collect::<Vec<_>>();
            if matches!(
                kind,
                VisualBlockInsertKind::Insert | VisualBlockInsertKind::Change
            ) && ranges.is_empty()
            {
                // Vim's block I/c semantics leave lines which do not extend
                // into the rectangle unchanged. A always appends at the
                // nearest legal row-end caret instead.
                continue;
            }
            let insertion_offset = match kind {
                VisualBlockInsertKind::Insert => row.visual_left.point.text_offset,
                VisualBlockInsertKind::Append => row.visual_right.point.text_offset,
                // A logical edit cannot persist a display-space side. The
                // first selected logical segment is the unambiguous insertion
                // boundary for the replacement batch.
                VisualBlockInsertKind::Change => ranges[0].start,
            };
            rows.push(VisualBlockInsertRow {
                insertion_offset,
                ranges,
            });
        }
        let cursor_target = rows.first().map_or(self.cursor, |row| row.insertion_offset);
        let count = if kind == VisualBlockInsertKind::Change {
            1
        } else {
            count
        };
        self.visual_block_insert = Some(VisualBlockInsertSession {
            kind,
            document_id: document.id(),
            revision: document.revision(),
            rows,
            payload: String::new(),
            count,
            register: requested_register,
            replaced,
            cursor_target,
            repeat_shape,
        });
        self.mode = Mode::Insert;
        self.cursor = cursor_target;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_block = None;
        self.active_visual_block = None;
        self.visual_anchor = None;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.insert_session = None;
        self.clear_pending();
        Ok(CommandOutput {
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn visual_block_paste(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        preserve_unnamed: bool,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let register_name = self.requested_register.take().unwrap_or('"');
        let register = match self.require_register_value(document, register_name) {
            Ok(register) => register,
            Err(output) => return Ok(output),
        };
        if register.text.is_empty() {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!("empty register {register_name}")),
                ..CommandOutput::complete()
            });
        }
        let replaced = block_register_value(document, &resolved, None);
        let target = resolved.rows.first().map_or(self.cursor, |row| {
            row.ranges
                .iter()
                .find(|range| !range.is_empty())
                .map_or(row.visual_left.point.text_offset, |range| range.start)
        });
        let lines = document.hard_line_snapshot();
        let mut linewise_cursor_offset = None;
        let plans = match register.kind {
            RegisterKind::Characterwise if register.hard_break_offsets().is_empty() => {
                let repeated = match checked_register_repetition(&register, count) {
                    Ok(repeated) => repeated,
                    Err(error) => return Ok(error.into_command_output()),
                };
                let payloads =
                    vec![Some(StructuredFragment::from_register(&repeated)); resolved.rows.len()];
                block_row_replacement_plans(&resolved, &payloads)
            }
            RegisterKind::Characterwise => {
                let repeated = match checked_register_repetition(&register, count) {
                    Ok(repeated) => repeated,
                    Err(error) => return Ok(error.into_command_output()),
                };
                let mut plans = block_deletion_plans(&resolved);
                plans.push(PlannedFormattedEdit::from_register(
                    target..target,
                    &repeated,
                ));
                plans
            }
            RegisterKind::Blockwise => {
                let register_rows = match checked_block_register_rows(&register, count) {
                    Ok(rows) => rows,
                    Err(error) => return Ok(error.into_command_output()),
                };
                let selected_payloads = (0..resolved.rows.len())
                    .map(|index| {
                        register_rows
                            .get(index)
                            .cloned()
                            .map(StructuredFragment::literal)
                    })
                    .collect::<Vec<_>>();
                let mut plans = block_row_replacement_plans(&resolved, &selected_payloads);
                if register_rows.len() > resolved.rows.len() {
                    let additional = &register_rows[resolved.rows.len()..];
                    let after_row = resolved
                        .rows
                        .last()
                        .map_or(0, |row| row.row_index.saturating_add(1));
                    let needed_last_row = after_row.saturating_add(additional.len() - 1);
                    if needed_last_row >= context.snapshot.rows.len() {
                        let last = resolved
                            .rows
                            .last()
                            .expect("a resolved block contains at least one row");
                        let position = VisualPosition {
                            text_offset: last.visual_left.point.text_offset,
                            affinity: last.visual_left.point.affinity,
                        };
                        if let Err(error) = gj(
                            context.snapshot,
                            position,
                            additional.len(),
                            Some(last.visual_left.x),
                        )
                        .map(|_| ())
                        {
                            return Ok(layout_error(error));
                        }
                    }
                    let available = context.snapshot.rows.len().saturating_sub(after_row);
                    let target_x = resolved.rows[0].visual_left.x;
                    for (index, row_payload) in additional.iter().take(available).enumerate() {
                        let row = &context.snapshot.rows[after_row + index];
                        let at = nearest_layout_caret_offset(row, target_x)
                            .unwrap_or(row.text_range.end);
                        if !row_payload.is_empty() {
                            plans.push(PlannedFormattedEdit::literal(at..at, row_payload.clone()));
                        }
                    }
                    if additional.len() > available {
                        plans.push(appended_block_rows_plan(
                            document.projection().text_tree().byte_len(),
                            &additional[available..],
                        ));
                    }
                }
                plans
            }
            RegisterKind::Linewise => {
                let mut repeated = match checked_register_repetition(&register, count) {
                    Ok(repeated) => repeated,
                    Err(error) => return Ok(error.into_command_output()),
                };
                let first_line = resolved.rows[0].hard_line_index;
                let last_line = resolved.rows[resolved.rows.len() - 1].hard_line_index;
                let insertion = if preserve_unnamed {
                    lines
                        .line(first_line)
                        .expect("resolved layout rows name existing hard lines")
                        .content_range()
                        .start
                } else {
                    lines
                        .line(last_line)
                        .expect("resolved layout rows name existing hard lines")
                        .linewise_range()
                        .end
                };
                let mut content_start = 0;
                if !preserve_unnamed && insertion == document.projection().text_tree().byte_len() {
                    repeated = match linewise_register_at_eof(repeated, count) {
                        Ok(repeated) => repeated,
                        Err(error) => return Ok(error.into_command_output()),
                    };
                    content_start = 1;
                }
                linewise_cursor_offset = Some(content_start);
                let mut plans = block_deletion_plans(&resolved);
                plans.push(PlannedFormattedEdit::from_register(
                    insertion..insertion,
                    &repeated,
                ));
                plans
            }
        };
        let before = document.revision();
        // One structured call is the atomicity boundary for all hit-tested
        // rows, including any semantic line breaks in the register payload.
        let mapped_target = commit_planned_formatted_edits(
            self, document,
            &lines,
            plans,
            if register.kind == RegisterKind::Linewise {
                if preserve_unnamed {
                    lines
                        .line(resolved.rows[0].hard_line_index)
                        .expect("resolved row has a hard line")
                        .content_range()
                        .start
                } else {
                    lines
                        .line(resolved.rows[resolved.rows.len() - 1].hard_line_index)
                        .expect("resolved row has a hard line")
                        .linewise_range()
                        .end
                }
            } else {
                target
            },
            Association::BeforeInsertion,
        )?;
        let changed = document.revision() != before;
        if !preserve_unnamed {
            self.delete_register(
                None,
                replaced,
                visual_block_deletion_class(resolved.rows.len()),
            );
        }
        let new_lines = document.hard_line_snapshot();
        let cursor_target = if let Some(relative) = linewise_cursor_offset {
            first_nonblank_document(document, &new_lines,
                mapped_target
                    .saturating_add(relative)
                    .min(document.projection().text_tree().byte_len()),
            )
        } else {
            mapped_target
        };
        self.cursor = normalize_normal_cursor_document(document, &new_lines, cursor_target);
        if !self.replaying {
            // Visual p repeats as a selection-shaped delete in Vim. P routes
            // that delete through the black-hole register so replay preserves
            // the unnamed register, while p keeps the ordinary delete effect.
            self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                shape: repeat_shape,
                action: VisualBlockRepeatAction::Operator {
                    operator: Operator::Delete,
                    register: preserve_unnamed.then_some('_'),
                },
            }));
        }
        self.leave_visual_block();
        Ok(CommandOutput {
            cursor_moved: true,
            document_changed: changed,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn register_is_blockwise(&self, document: &Document, requested: Option<char>) -> bool {
        self.register_is_blockwise_with_context(document, requested, None)
    }

    fn register_is_blockwise_with_context(
        &self,
        document: &Document,
        requested: Option<char>,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> bool {
        let name = requested.unwrap_or('"');
        let value = match clipboard {
            Some(clipboard) => self.register_with_context(document, clipboard, name),
            None => self.read_register_value(document, name),
        };
        value
            .ok()
            .flatten()
            .is_some_and(|value| value.kind == RegisterKind::Blockwise)
    }

    fn repeat_is_blockwise_paste(&self, document: &Document) -> bool {
        self.repeat_is_blockwise_paste_with_context(document, None)
    }

    fn repeat_is_blockwise_paste_with_context(
        &self,
        document: &Document,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> bool {
        matches!(
            self.last_repeat.as_ref(),
            Some(RepeatAction::Paste { register, .. })
                if self.register_is_blockwise_with_context(
                    document,
                    Some(*register),
                    clipboard,
                )
        )
    }

    fn repeat_is_visual_block_change(&self) -> bool {
        matches!(self.last_repeat, Some(RepeatAction::VisualBlock(_)))
    }

    fn repeat_visual_block_with_layout(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(RepeatAction::VisualBlock(repeat)) = self.last_repeat.clone() else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no previous Visual Block change".into()),
                ..CommandOutput::complete()
            });
        };
        let checkpoint = self.clone();
        if let Err(output) =
            self.install_visual_block_repeat_selection(document, context, repeat.shape)
        {
            return Ok(output);
        }

        // A supplied dot count is deliberately absent here. Vim reuses the
        // recorded Visual shape and the source command's own application
        // count; `2.` must not double either dimension.
        self.replaying = true;
        document.begin_edit_group();
        let result = (|| -> Result<CommandOutput, DocumentError> {
            match repeat.action {
                VisualBlockRepeatAction::Operator { operator, register } => {
                    self.requested_register = register;
                    self.apply_visual_block_operator(document, context, operator)
                }
                VisualBlockRepeatAction::Replace { replacement } => {
                    self.apply_visual_block_replace(document, context, &replacement)
                }
                VisualBlockRepeatAction::Shift {
                    operator,
                    application_count,
                } => self.apply_visual_block_shift(document, context, operator, application_count),
                VisualBlockRepeatAction::Join { insert_space } => {
                    self.visual_block_join(document, context, insert_space)
                }
                VisualBlockRepeatAction::Insert {
                    kind,
                    payload,
                    application_count,
                    register,
                } => {
                    self.requested_register = register;
                    let mut output =
                        self.begin_visual_block_insert(document, context, kind, application_count)?;
                    if output.status == CommandStatus::Complete {
                        self.visual_block_insert
                            .as_mut()
                            .expect("successful block-repeat insert installs a session")
                            .payload = payload;
                        output.merge(self.finish_visual_block_insert(document)?);
                    }
                    Ok(output)
                }
            }
        })();
        document.end_edit_group();
        self.replaying = false;

        match result {
            Ok(output) if output.status != CommandStatus::Complete && !output.document_changed => {
                *self = checkpoint;
                Ok(output)
            }
            Ok(mut output) => {
                // Synthetic selection state is an implementation detail of
                // dot. Preserve the user's last explicit Visual selection for
                // `gv` and do not report a phantom Normal -> Visual -> Normal
                // mode transition to the frontend.
                self.last_visual = checkpoint.last_visual;
                output.mode_changed = self.mode != checkpoint.mode;
                Ok(output)
            }
            Err(error) => Err(error),
        }
    }

    fn repeat_block_paste_with_layout(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        override_count: Option<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(RepeatAction::Paste {
            before,
            follow,
            count,
            register,
        }) = self.last_repeat.clone()
        else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no previous blockwise paste".into()),
                ..CommandOutput::complete()
            });
        };
        self.requested_register = Some(register);
        self.replaying = true;
        document.begin_edit_group();
        let result = self.normal_block_paste(
            document,
            context,
            before,
            follow,
            choose_repeat_count(override_count, count),
        );
        document.end_edit_group();
        self.replaying = false;
        result
    }

    fn normal_block_paste(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        before: bool,
        follow: bool,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let register_name = self.requested_register.take().unwrap_or('"');
        let register = match self.require_register_value(document, register_name) {
            Ok(register) => register,
            Err(output) => return Ok(output),
        };
        if register.text.is_empty() {
            return Ok(CommandOutput::complete());
        }
        debug_assert_eq!(register.kind, RegisterKind::Blockwise);
        let register_rows = match checked_block_register_rows(&register, count) {
            Ok(rows) => rows,
            Err(error) => return Ok(error.into_command_output()),
        };
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return Ok(layout_error(error)),
        };
        let Some((origin_row, origin_offset, target_x)) =
            normal_block_insertion_origin(context.snapshot, current, before)
        else {
            return Ok(layout_error(LayoutMotionError::PositionNotInLayout(
                current,
            )));
        };

        let needed_last_row = origin_row.saturating_add(register_rows.len().saturating_sub(1));
        if needed_last_row >= context.snapshot.rows.len() {
            if let Err(error) = if register_rows.len() > 1 {
                gj(
                    context.snapshot,
                    current,
                    register_rows.len() - 1,
                    Some(target_x),
                )
                .map(|_| ())
            } else {
                Ok(())
            } {
                return Ok(layout_error(error));
            }
        }
        self.requested_register = None;

        let lines = document.hard_line_snapshot();
        let mut plans = Vec::new();
        let mut insertion_boundaries = Vec::with_capacity(register_rows.len());
        let available = context.snapshot.rows.len().saturating_sub(origin_row);
        for (index, row_payload) in register_rows.iter().take(available).enumerate() {
            let at = if index == 0 {
                origin_offset
            } else {
                nearest_layout_caret_offset(&context.snapshot.rows[origin_row + index], target_x)
                    .unwrap_or(context.snapshot.rows[origin_row + index].text_range.end)
            };
            insertion_boundaries.push(at);
            if !row_payload.is_empty() {
                plans.push(PlannedFormattedEdit::literal(at..at, row_payload.clone()));
            }
        }
        if register_rows.len() > available {
            let remaining = &register_rows[available..];
            let at = document.projection().text_tree().byte_len();
            insertion_boundaries.extend(std::iter::repeat(at).take(remaining.len()));
            plans.push(appended_block_rows_plan(at, remaining));
        }

        let first_boundary = *insertion_boundaries.first().unwrap_or(&origin_offset);
        let last_boundary = *insertion_boundaries.last().unwrap_or(&origin_offset);
        let target_boundary = if follow {
            last_boundary
        } else {
            first_boundary
        };
        let association = if follow {
            Association::AfterInsertion
        } else {
            Association::BeforeInsertion
        };
        let before_revision = document.revision();
        let mapped_target =
            commit_planned_formatted_edits(self, document, &lines, plans, target_boundary, association)?;
        let changed = document.revision() != before_revision;
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            mapped_target.min(document.projection().text_tree().byte_len()),
        );
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::Paste {
                before,
                follow,
                count,
                register: register_name,
            });
        }
        Ok(CommandOutput {
            cursor_moved: true,
            document_changed: changed,
            ..CommandOutput::complete()
        })
    }

    fn apply_visual_block_shift(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        operator: Operator,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        self.requested_register = None;
        let Some(shift_width) = 4usize.checked_mul(count) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("Visual Block shift count is too large".into()),
                ..CommandOutput::complete()
            });
        };
        let mut edits = Vec::new();
        let mut target = self.cursor;
        match operator {
            Operator::Indent => {
                let padding = match checked_text_repetition("    ", count) {
                    Ok(padding) => padding,
                    Err(error) => return Ok(error.into_command_output()),
                };
                for row in resolved
                    .rows
                    .iter()
                    .filter(|row| row.ranges.iter().any(|range| !range.is_empty()))
                {
                    let at = row.visual_left.point.text_offset;
                    if edits.is_empty() {
                        target = at;
                    }
                    edits.push(TextEdit::new(at..at, padding.clone()));
                }
                edits = merge_same_boundary_insertions(edits);
            }
            Operator::Outdent => {
                for row in resolved
                    .rows
                    .iter()
                    .filter(|row| row.ranges.iter().any(|range| !range.is_empty()))
                {
                    let at = row.visual_left.point.text_offset;
                    let row_end = context.snapshot.rows[row.row_index].text_range.end;
                    let end = document.text().as_bytes()[at..row_end]
                        .iter()
                        .take(shift_width)
                        .take_while(|byte| **byte == b' ')
                        .count()
                        + at;
                    if at < end {
                        if edits.is_empty() {
                            target = at;
                        }
                        edits.push(TextEdit::new(at..end, ""));
                    }
                }
            }
            Operator::Reindent => {
                let mut hard_lines = resolved
                    .rows
                    .iter()
                    .map(|row| row.hard_line_index)
                    .collect::<Vec<_>>();
                hard_lines.sort_unstable();
                hard_lines.dedup();
                for hard_line in hard_lines {
                    let Some(start) = document.line_start(hard_line) else {
                        continue;
                    };
                    let Some(end) = document.line_end(hard_line) else {
                        continue;
                    };
                    let old = &document.text()[start..end];
                    let replacement = old.trim_start();
                    if old != replacement {
                        if edits.is_empty() {
                            target = start;
                        }
                        edits.push(TextEdit::new(start..end, replacement));
                    }
                }
            }
            _ => unreachable!("Visual Block shift accepts only >, <, and ="),
        }

        let before = document.revision();
        // Even a no-op is routed through exactly one batch preparation call.
        apply_block_edits(document, edits)?;
        let changed = document.revision() != before;
        self.cursor = if operator == Operator::Reindent {
            first_nonblank_document(document, &document.hard_line_snapshot(),
                target.min(document.projection().text_tree().byte_len()),
            )
        } else {
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                target.min(document.projection().text_tree().byte_len()),
            )
        };
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                shape: repeat_shape,
                action: VisualBlockRepeatAction::Shift {
                    operator,
                    application_count: count,
                },
            }));
        }
        self.leave_visual_block();
        Ok(CommandOutput {
            cursor_moved: true,
            document_changed: changed,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn apply_visual_block_operator(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        operator: Operator,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let target = resolved
            .range_set
            .segments
            .first()
            .map_or(self.cursor, |segment| segment.range.start);
        let register = self.requested_register.take();
        if operator.is_format() {
            // Visual Block formats each intersected hard line in full, once,
            // rather than reflowing a pixel-width rectangle.
            let (first, last) = resolved.rows.iter().fold((usize::MAX, 0), |(low, high), row| {
                (low.min(row.hard_line_index), high.max(row.hard_line_index))
            });
            if first == usize::MAX {
                return Ok(CommandOutput::complete());
            }
            let before = document.revision();
            let output = self.apply_format_operator(
                document,
                operator == Operator::FormatKeepCursor,
                first..last + 1,
            )?;
            if output.status != CommandStatus::Complete {
                return Ok(output);
            }
            if !self.replaying && document.revision() != before {
                self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                    shape: repeat_shape,
                    action: VisualBlockRepeatAction::Operator { operator, register },
                }));
            }
            self.leave_visual_block();
            return Ok(CommandOutput {
                cursor_moved: true,
                document_changed: output.document_changed,
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        if matches!(operator, Operator::Delete | Operator::Yank) {
            if let Err(output) = self.require_register_write(register) {
                return Ok(output);
            }
        }
        let selected = block_register_value(document, &resolved, register);
        let edits = match operator {
            Operator::Delete => delete_text_edits(&resolved),
            Operator::ToggleCase | Operator::Lowercase | Operator::Uppercase => resolved
                .range_set
                .segments
                .iter()
                .filter_map(|segment| {
                    let old = &document.text()[segment.range.clone()];
                    let replacement = change_case(old, operator);
                    (old != replacement).then(|| TextEdit::new(segment.range.clone(), replacement))
                })
                .collect(),
            Operator::Yank => Vec::new(),
            _ => {
                return Ok(CommandOutput::unsupported(
                    "operator has no visual block implementation",
                ));
            }
        };
        let changed = !edits.is_empty();
        if changed {
            apply_block_edits(document, edits)?;
        }
        match operator {
            Operator::Delete => self.delete_register(
                register,
                selected,
                visual_block_deletion_class(resolved.rows.len()),
            ),
            Operator::Yank => self.yank_register(register, selected),
            _ => {}
        }
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            target.min(document.projection().text_tree().byte_len()),
        );
        if !self.replaying && operator != Operator::Yank {
            self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                shape: repeat_shape,
                action: VisualBlockRepeatAction::Operator { operator, register },
            }));
        }
        self.leave_visual_block();
        Ok(CommandOutput {
            cursor_moved: true,
            document_changed: operator != Operator::Yank && changed,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn apply_visual_block_replace(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        replacement: &RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let target = resolved
            .range_set
            .segments
            .first()
            .map_or(self.cursor, |segment| segment.range.start);
        // Visual `r` is a pure replacement, not a delete followed by an
        // insertion. A syntactic register prefix is consumed but no register
        // is read, checked for writability, or updated.
        self.requested_register.take();
        let lines = document.hard_line_snapshot();
        let plans = if is_single_semantic_hard_break(replacement) {
            // Vim's Visual Block r<Enter> splits every nonempty selected row
            // once, independent of the rectangle width. Discontiguous logical
            // ranges on one bidi row still form one visual replacement.
            let payloads = resolved
                .rows
                .iter()
                .map(|row| {
                    row.ranges
                        .iter()
                        .any(|range| !range.is_empty())
                        .then(|| StructuredFragment::from_register(replacement))
                })
                .collect::<Vec<_>>();
            block_row_replacement_plans(&resolved, &payloads)
        } else {
            let mut plans = Vec::new();
            for segment in resolved
                .range_set
                .segments
                .iter()
                .filter(|segment| segment.grapheme_count > 0)
            {
                let repeated =
                    match checked_register_repetition(replacement, segment.grapheme_count) {
                        Ok(repeated) => repeated,
                        Err(error) => return Ok(error.into_command_output()),
                    };
                plans.push(PlannedFormattedEdit::from_register(
                    segment.range.clone(),
                    &repeated,
                ));
            }
            plans
        };
        let before = document.revision();
        let has_plans = !plans.is_empty();
        let mapped_target = if !has_plans {
            target
        } else {
            commit_planned_formatted_edits(
                self, document,
                &lines,
                plans,
                target,
                Association::BeforeInsertion,
            )?
        };
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            mapped_target.min(document.projection().text_tree().byte_len()),
        );
        let changed = document.revision() != before;
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                shape: repeat_shape,
                action: VisualBlockRepeatAction::Replace {
                    replacement: replacement.clone(),
                },
            }));
        }
        self.leave_visual_block();
        Ok(CommandOutput {
            cursor_moved: true,
            document_changed: changed,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn leave_visual_block(&mut self) {
        self.remember_visual_block();
        self.mode = Mode::Normal;
        self.visual_to_line_end = false;
        self.visual_block = None;
        self.active_visual_block = None;
        self.visual_anchor = None;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.clear_pending();
    }

    fn visual_block_memory(&self) -> Option<VisualMemory> {
        if let Some(selection) = self.visual_block.as_ref() {
            return Some(VisualMemory {
                mode: Mode::VisualBlock,
                anchor: selection.anchor.text_offset,
                active: selection.active.text_offset,
                to_line_end: self.visual_to_line_end,
                block: Some(BlockVisualMemory {
                    anchor_affinity: selection.anchor.affinity,
                    active_affinity: selection.active.affinity,
                    anchor_x: selection.anchor_x,
                    active_x: selection.active_x,
                }),
            });
        }
        let selection = self.active_visual_block?;
        Some(VisualMemory {
            mode: Mode::VisualBlock,
            anchor: selection.anchor.offset(),
            active: selection.active.offset(),
            to_line_end: self.visual_to_line_end,
            block: Some(BlockVisualMemory {
                anchor_affinity: selection.anchor.affinity(),
                active_affinity: selection.active.affinity(),
                anchor_x: selection.anchor_x,
                active_x: selection.active_x,
            }),
        })
    }

    fn remember_visual_block(&mut self) {
        if let Some(memory) = self.visual_block_memory() {
            self.last_visual = Some(memory);
        }
    }

    fn transition_visual_block_to_linear(
        &mut self,
        document: &Document,
        snapshot: &LayoutSnapshot,
        mode: Mode,
    ) -> CommandOutput {
        let Some(selection) = self.visual_block.as_ref() else {
            return CommandOutput::unsupported("visual block selection is missing");
        };
        let anchor_position = VisualPosition {
            text_offset: selection.anchor.text_offset,
            affinity: selection.anchor.affinity,
        };
        let active_position = VisualPosition {
            text_offset: selection.active.text_offset,
            affinity: selection.active.affinity,
        };
        let anchor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            snapshot,
            anchor_position,
        );
        let active = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            snapshot,
            active_position,
        );
        self.mode = mode;
        self.visual_to_line_end = false;
        self.visual_anchor = Some(anchor);
        self.cursor = active;
        self.boundary_affinity = active_position.affinity;
        self.visual_position = Some(active_position);
        self.visual_block = None;
        self.active_visual_block = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.clear_pending();
        CommandOutput {
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        }
    }

    fn install_visual_memory_with_layout(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        memory: VisualMemory,
    ) -> CommandOutput {
        if memory.mode != Mode::VisualBlock {
            self.mode = memory.mode;
            self.visual_to_line_end = memory.to_line_end;
            self.visual_anchor = Some(normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                memory.anchor.min(document.projection().text_tree().byte_len()),
            ));
            self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                memory.active.min(document.projection().text_tree().byte_len()),
            );
            self.visual_block = None;
            self.active_visual_block = None;
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.visual_position = None;
            self.desired_x = None;
            self.preferred_column = None;
            self.clear_pending();
            return CommandOutput {
                cursor_moved: true,
                mode_changed: true,
                ..CommandOutput::complete()
            };
        }
        let Some(block) = memory.block else {
            return CommandOutput::unsupported("saved Visual Block geometry is missing");
        };
        let anchor_position = match layout_position_for_offset(
            context.snapshot,
            memory.anchor,
            block.anchor_affinity,
        ) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let active_position = match layout_position_for_offset(
            context.snapshot,
            memory.active,
            block.active_affinity,
        ) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let anchor = context
            .snapshot
            .caret_point(anchor_position.text_offset, anchor_position.affinity)
            .expect("resolved block anchor is an exact caret");
        let active = context
            .snapshot
            .caret_point(active_position.text_offset, active_position.affinity)
            .expect("resolved block active endpoint is an exact caret");
        let mut selection =
            match BlockSelection::new(anchor, active, block.anchor_x, block.active_x) {
                Ok(selection) => selection,
                Err(error) => return visual_block_error(error),
            };
        if let Err(error) = selection.update_inclusive_rectangle(context.snapshot) {
            return visual_block_error(error);
        }
        let persistent = match active_visual_block_from_selection(document, &selection) {
            Ok(persistent) => persistent,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "visual block anchors could not be captured: {error:?}"
                    )),
                    ..CommandOutput::complete()
                };
            }
        };
        self.mode = Mode::VisualBlock;
        self.visual_to_line_end = memory.to_line_end;
        self.visual_anchor = None;
        self.visual_block = Some(selection);
        self.active_visual_block = Some(persistent);
        self.visual_block_rebind_error = None;
        self.cursor = normal_offset_for_visual(
            document.text(),
            &document.hard_line_snapshot(),
            context.snapshot,
            active_position,
        );
        self.boundary_affinity = active_position.affinity;
        self.visual_position = Some(active_position);
        self.desired_x = None;
        self.preferred_column = None;
        self.clear_pending();
        CommandOutput {
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        }
    }

    fn exchange_visual_with_layout(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
    ) -> CommandOutput {
        let Some(previous) = self.last_visual else {
            return CommandOutput {
                status: CommandStatus::Error("no previous visual selection".into()),
                ..CommandOutput::complete()
            };
        };
        let current = if self.mode == Mode::VisualBlock {
            self.visual_block_memory()
        } else {
            self.visual_anchor.map(|anchor| VisualMemory {
                mode: self.mode,
                anchor,
                active: self.cursor,
                to_line_end: self.visual_to_line_end,
                block: None,
            })
        };
        let Some(current) = current else {
            return CommandOutput::unsupported("current Visual selection is missing");
        };
        let checkpoint = self.clone();
        let output = self.install_visual_memory_with_layout(document, context, previous);
        if output.status != CommandStatus::Complete {
            *self = checkpoint;
            return output;
        }
        self.last_visual = Some(current);
        output
    }

    fn restore_visual_with_layout(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
    ) -> CommandOutput {
        let Some(memory) = self.last_visual else {
            return CommandOutput {
                status: CommandStatus::Error("no previous visual selection".into()),
                ..CommandOutput::complete()
            };
        };
        let checkpoint = self.clone();
        let output = self.install_visual_memory_with_layout(document, context, memory);
        if output.status != CommandStatus::Complete {
            *self = checkpoint;
        }
        output
    }

    fn move_to_visual_row_edge(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        edge: RowEdge,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let result = match edge {
            RowEdge::Start => g0(context.snapshot, current),
            RowEdge::FirstNonBlank => g_caret(context.snapshot, document.text(), current),
        };
        match result {
            Ok(position) => {
                self.visual_to_line_end = false;
                self.desired_x = None;
                self.preferred_column = None;
                self.install_visual_position(document, context.snapshot, position)
            }
            Err(error) => layout_error(error),
        }
    }

    fn move_to_counted_visual_row_end(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        count: usize,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        let row = if count > 1 {
            match gj(
                context.snapshot,
                current,
                count.saturating_sub(1),
                self.desired_x,
            ) {
                Ok(result) => result.position,
                Err(error) => return layout_error(error),
            }
        } else {
            current
        };
        match g_dollar_for_document(document, context.snapshot, row) {
            Ok(position) => {
                self.visual_to_line_end = false;
                self.desired_x = None;
                self.preferred_column = None;
                self.install_visual_position(document, context.snapshot, position)
            }
            Err(error) => layout_error(error),
        }
    }

    fn move_to_viewport_line(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        line: ViewportLine,
        count: usize,
    ) -> CommandOutput {
        let origin = self.cursor;
        let output = match viewport_line(
            context.snapshot,
            document.text(),
            context.viewport,
            line,
            count,
        ) {
            Ok(position) => {
                self.desired_x = None;
                self.preferred_column = None;
                self.install_visual_position(document, context.snapshot, position)
            }
            Err(error) => layout_error(error),
        };
        self.record_successful_jump(document, origin, output)
    }

    fn execute_screen_motion(
        &mut self,
        document: &Document,
        context: &mut LayoutCommandContext<'_>,
        motion: ScreenMotion,
        count: usize,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        match screen_motion(
            context.snapshot,
            current,
            context.viewport,
            motion,
            count,
            self.desired_x,
        ) {
            Ok(result) => {
                context.viewport = result.viewport;
                let output = self.install_visual_position(
                    document,
                    context.snapshot,
                    result.motion.position,
                );
                self.desired_x = Some(result.motion.desired_x);
                self.preferred_column = None;
                output
            }
            Err(error) => layout_error(error),
        }
    }

    /// Resolve Vim's window-local CTRL-D/CTRL-U amount. Zero is reserved for
    /// the layout layer's dynamic half-viewport default, so every explicit
    /// count is retained as a positive exact visual-row amount.
    fn half_page_scroll_amount(&mut self, explicit_count: Option<usize>) -> usize {
        if let Some(count) = explicit_count {
            let count = count.max(1);
            self.half_page_scroll_rows = Some(count);
            count
        } else {
            self.half_page_scroll_rows.unwrap_or(0)
        }
    }

    fn align_current_row(
        &mut self,
        context: &mut LayoutCommandContext<'_>,
        alignment: ViewportAlignment,
    ) -> CommandOutput {
        let current = match self.current_visual_position(context.snapshot) {
            Ok(position) => position,
            Err(error) => return layout_error(error),
        };
        match align_viewport(context.snapshot, current, context.viewport, alignment) {
            Ok(viewport) => {
                context.viewport = viewport;
                CommandOutput::complete()
            }
            Err(error) => layout_error(error),
        }
    }

    fn align_counted_row(
        &mut self,
        document: &Document,
        context: &mut LayoutCommandContext<'_>,
        alignment: ViewportAlignment,
        count: usize,
        count_explicit: bool,
    ) -> CommandOutput {
        let old_cursor = self.cursor;
        let old_affinity = self.boundary_affinity;
        let old_visual_position = self.visual_position;
        let old_desired_x = self.desired_x;
        let old_preferred_column = self.preferred_column;
        if count_explicit {
            let lines = document.hard_line_snapshot();
            self.cursor = first_nonblank_document(document, &lines, nth_line_start(&lines, count));
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.visual_position = None;
            self.desired_x = None;
            self.preferred_column = None;
        }
        let mut output = self.align_current_row(context, alignment);
        if output.status == CommandStatus::Complete {
            output.cursor_moved |= self.cursor != old_cursor;
        } else {
            self.cursor = old_cursor;
            self.boundary_affinity = old_affinity;
            self.visual_position = old_visual_position;
            self.desired_x = old_desired_x;
            self.preferred_column = old_preferred_column;
        }
        output
    }

    fn move_in_insert_mode(
        &mut self,
        document: &mut Document,
        context: &mut LayoutCommandContext<'_>,
        motion: InsertLayoutMotion,
    ) -> CommandOutput {
        document.end_edit_group();
        self.invalidate_replace_restoration();
        self.publish_last_insert_fragment();
        let placement = if self.mode == Mode::Replace {
            InsertPlacement::Replace
        } else {
            InsertPlacement::Before
        };
        if let Some(session) = self.insert_session.as_mut() {
            session.placement = placement;
            session.repeat_program = Some(EditSessionProgram::default());
            session.last_inserted = RegisterValue::characterwise("");
            session.preserve_normal_repeat = false;
        }
        let output = match motion {
            InsertLayoutMotion::Rows(down) => self.move_layout_rows(document, context, 1, down),
            InsertLayoutMotion::Screen(motion) => {
                self.execute_screen_motion(document, context, motion, 1)
            }
        };
        document.begin_edit_group();
        if let Some(session) = self.insert_session.as_mut() {
            session.unit_floor = self.cursor;
        }
        output
    }

    fn current_visual_position(
        &self,
        snapshot: &LayoutSnapshot,
    ) -> Result<VisualPosition, LayoutMotionError> {
        let mut candidates = Vec::with_capacity(3);
        if let Some(position) = self.visual_position {
            candidates.push(position);
        }
        candidates.push(VisualPosition {
            text_offset: self.cursor,
            affinity: self.boundary_affinity,
        });
        for affinity in [BoundaryAffinity::Downstream, BoundaryAffinity::Upstream] {
            let candidate = VisualPosition {
                text_offset: self.cursor,
                affinity,
            };
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        candidates
            .into_iter()
            .find(|position| {
                snapshot
                    .caret_point(position.text_offset, position.affinity)
                    .is_ok()
            })
            .ok_or(LayoutMotionError::PositionNotInLayout(VisualPosition {
                text_offset: self.cursor,
                affinity: self.boundary_affinity,
            }))
    }

    fn install_visual_position(
        &mut self,
        document: &Document,
        snapshot: &LayoutSnapshot,
        position: VisualPosition,
    ) -> CommandOutput {
        let old_cursor = self.cursor;
        let old_position = self.current_visual_position(snapshot).ok();
        let next_cursor = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            position.text_offset
        } else if position.affinity == BoundaryAffinity::Upstream
            && !is_empty_row_at_position(snapshot, position)
        {
            document.hard_line_snapshot().previous_grapheme_boundary(position.text_offset)
                .unwrap_or(position.text_offset)
        } else {
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                position.text_offset,
            )
        };
        let next_block = if self.mode == Mode::VisualBlock {
            let Some(mut selection) = self.visual_block.clone() else {
                return CommandOutput::unsupported("visual block selection is missing");
            };
            let point = match snapshot.caret_point(position.text_offset, position.affinity) {
                Ok(point) => point,
                Err(_) => return layout_error(LayoutMotionError::PositionNotInLayout(position)),
            };
            let geometry = match snapshot.caret_geometry(point) {
                Ok(geometry) => geometry,
                Err(_) => return layout_error(LayoutMotionError::PositionNotInLayout(position)),
            };
            selection.active = point;
            selection.active_x = geometry.rect.x;
            if let Err(error) = selection.update_inclusive_rectangle(snapshot) {
                return visual_block_error(error);
            }
            let persistent = match active_visual_block_from_selection(document, &selection) {
                Ok(persistent) => persistent,
                Err(error) => {
                    return CommandOutput {
                        status: CommandStatus::Error(format!(
                            "visual block anchors could not be captured: {error:?}"
                        )),
                        ..CommandOutput::complete()
                    };
                }
            };
            Some((selection, persistent))
        } else {
            None
        };
        self.typing_style = Default::default();
        self.boundary_affinity = position.affinity;
        self.visual_position = Some(position);
        self.cursor = next_cursor;
        if let Some((selection, persistent)) = next_block {
            self.visual_block = Some(selection);
            self.active_visual_block = Some(persistent);
            self.visual_block_rebind_error = None;
        }
        CommandOutput {
            cursor_moved: old_cursor != self.cursor || old_position != Some(position),
            ..CommandOutput::complete()
        }
    }

    fn apply_visual_block_logical_motion(
        &mut self,
        document: &Document,
        context: &LayoutCommandContext<'_>,
        action: impl FnOnce(&mut Self, &Document) -> CommandOutput,
    ) -> CommandOutput {
        let checkpoint = self.clone();
        let mut output = action(self, document);
        if output.status != CommandStatus::Complete || !output.cursor_moved {
            return output;
        }
        let position = match layout_position_for_offset(
            context.snapshot,
            self.cursor,
            BoundaryAffinity::Downstream,
        ) {
            Ok(position) => position,
            Err(error) => {
                *self = checkpoint;
                return layout_error(error);
            }
        };
        let installed = self.install_visual_position(document, context.snapshot, position);
        if installed.status != CommandStatus::Complete {
            *self = checkpoint;
            return installed;
        }
        output.merge(installed);
        output
    }

    fn handle_text(
        &mut self,
        document: &mut Document,
        input: String,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(output) = self.try_handle_controller_only_text(document, &input) {
            return Ok(output);
        }
        if self.visual_block_insert.is_some() {
            return Ok(self.collect_visual_block_payload(&input));
        }
        if matches!(
            self.pending,
            Pending::ReplaceCharacter { .. }
                | Pending::ReplaceVisual
                | Pending::Find { .. }
                | Pending::OperatorFind { .. }
        ) {
            return self.handle_pending_grapheme_text(document, &input);
        }
        match self.mode {
            Mode::Insert => self.insert_literal_text(document, &input),
            Mode::Replace => self.replace_literal_text(document, &input),
            Mode::CommandLine => {
                if let Some(state) = self.command_line_state.as_mut() {
                    state.buffer.insert(&input);
                    Ok(CommandOutput::pending())
                } else {
                    Ok(CommandOutput::unsupported("command-line input"))
                }
            }
            _ => {
                let mut result = CommandOutput::complete();
                for character in input.chars() {
                    let next = self.handle_key(document, Key::Char(character))?;
                    let stop = command_status_stops_compound(&next.status);
                    result.merge(next);
                    if stop || command_status_stops_compound(&result.status) {
                        break;
                    }
                }
                Ok(result)
            }
        }
    }

    fn handle_pending_grapheme_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let pending = self.pending;
        self.pending = Pending::None;
        let Some(grapheme) = one_extended_grapheme(input) else {
            return Ok(CommandOutput::unsupported(
                "pending text command expects one grapheme",
            ));
        };
        match pending {
            Pending::ReplaceCharacter { count, register } => {
                let value =
                    RegisterValue::try_new(grapheme, RegisterKind::Characterwise, Vec::new())
                        .expect("pending literal grapheme has valid break metadata");
                self.replace_characters(document, count, &value, register)
            }
            Pending::ReplaceVisual => {
                let value =
                    RegisterValue::try_new(grapheme, RegisterKind::Characterwise, Vec::new())
                        .expect("pending Visual literal grapheme has valid break metadata");
                self.replace_visual(document, &value)
            }
            Pending::Find {
                forward,
                till,
                count,
            } => Ok(self.execute_find(
                document,
                FindState {
                    needle: grapheme.to_owned(),
                    forward,
                    till,
                },
                count,
                true,
            )),
            Pending::OperatorFind {
                operator,
                forward,
                till,
            } => self.execute_operator_find(
                document,
                operator,
                FindState {
                    needle: grapheme.to_owned(),
                    forward,
                    till,
                },
                false,
            ),
            _ => unreachable!("caller filters pending grapheme commands"),
        }
    }

    fn handle_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        if key == Key::SelectAll {
            let mut output = if self.visual_block_insert.is_some() {
                self.finish_visual_block_insert(document)?
            } else if matches!(self.mode, Mode::Insert | Mode::Replace)
                || self.insert_session.is_some()
            {
                self.finish_insert(document)?
            } else {
                CommandOutput::complete()
            };
            if output.status != CommandStatus::Complete {
                return Ok(output);
            }
            self.select_all(document)?;
            output.mode_changed = true;
            output.cursor_moved = true;
            return Ok(output);
        }
        let key = if key == Key::ShiftEnter && !matches!(self.mode, Mode::Insert | Mode::Replace) {
            Key::Enter
        } else { key };
        let key = self.clipboard_copy_alias_key(key);
        if let Some(output) = self.try_html_assistance_key(document, key)? {
            return Ok(output);
        }
        if matches!(
            key,
            Key::Escape
                | Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
                | Key::Home
                | Key::End
                | Key::Ctrl('o')
        ) {
            self.typing_style = Default::default();
        }
        if let Some(output) = self.try_mode_line_key(document, key)? {
            return Ok(output);
        }
        if let Some(output) = self.try_handle_controller_only_key(document, key) {
            return Ok(output);
        }
        match self.mode {
            Mode::Insert | Mode::Replace => self.handle_edit_mode_key(document, key),
            Mode::VisualCharacter | Mode::VisualLine => self.handle_visual_key(document, key),
            Mode::VisualBlock => {
                if key == Key::Escape && self.visual_command_is_pending() {
                    self.clear_pending();
                    Ok(CommandOutput {
                        status: CommandStatus::Cancelled,
                        ..CommandOutput::complete()
                    })
                } else if key == Key::Escape || matches!(key, Key::Ctrl('v' | 'V' | 'q' | 'Q')) {
                    self.leave_visual_block();
                    Ok(CommandOutput {
                        status: CommandStatus::Cancelled,
                        mode_changed: true,
                        ..CommandOutput::complete()
                    })
                } else {
                    Ok(layout_required("visual block command"))
                }
            }
            Mode::CommandLine => self.submit_command_line(document),
            Mode::Normal => self.handle_normal_key(document, key),
        }
    }

    fn handle_normal_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        // Controller-only grammar and motions have already run in handle_key.
        match self.pending {
            Pending::ReplaceCharacter { count, register } => {
                self.pending = Pending::None;
                return match key {
                    Key::Char(character) => self.replace_characters(
                        document,
                        count,
                        &RegisterValue::characterwise(character.to_string()),
                        register,
                    ),
                    Key::Enter => self.replace_characters(
                        document,
                        count,
                        &RegisterValue::characterwise("\n"),
                        register,
                    ),
                    Key::Tab => self.replace_characters(
                        document,
                        count,
                        &RegisterValue::characterwise("\t"),
                        register,
                    ),
                    _ => Ok(CommandOutput::unsupported("r expects text")),
                };
            }
            Pending::G {
                count,
                register,
                ..
            } => {
                self.pending = Pending::None;
                return self.handle_g_command(document, key, count, register);
            }
            Pending::Z { .. } => {
                self.pending = Pending::None;
                return Ok(match key {
                    Key::Char('t' | 'z' | 'b') => layout_required("viewport alignment"),
                    _ => CommandOutput::unsupported(format!("z{key:?}")),
                });
            }
            Pending::Operator(operator) => {
                return self.handle_operator_key(document, key, operator);
            }
            Pending::OperatorFind {
                operator,
                forward,
                till,
            } => {
                self.pending = Pending::None;
                return match key {
                    Key::Char(character) => self.execute_operator_find(
                        document,
                        operator,
                        FindState {
                            needle: character.to_string(),
                            forward,
                            till,
                        },
                        false,
                    ),
                    _ => Ok(CommandOutput::unsupported("find expects text")),
                };
            }
            Pending::OperatorMark { operator, linewise } => {
                self.pending = Pending::None;
                return match key {
                    Key::Char(name @ 'a'..='z') => {
                        self.execute_operator_mark(document, operator, name, linewise)
                    }
                    _ => Ok(CommandOutput::unsupported("mark motion expects a-z")),
                };
            }
            Pending::TextObject { operator, scope } => {
                self.pending = Pending::None;
                return match key {
                    Key::Char(key) => {
                        self.execute_text_object_operator(document, operator, scope, key)
                    }
                    _ => Ok(CommandOutput::unsupported("text object expects a key")),
                };
            }
            Pending::MacroPlay { count } => {
                self.pending = Pending::None;
                return match key {
                    Key::Char('@') => self.play_macro(document, None, count),
                    Key::Char(name @ ('a'..='z' | 'A'..='Z')) => {
                        self.play_macro(document, Some(name.to_ascii_lowercase()), count)
                    }
                    _ => Ok(CommandOutput::unsupported(
                        "macro register expects a-z, A-Z, or @",
                    )),
                };
            }
            Pending::Find { .. }
            | Pending::SetMark
            | Pending::JumpMark { .. }
            | Pending::MacroRecord => unreachable!("controller-only pending command was dispatched"),
            Pending::VisualTextObject { .. }
            | Pending::ReplaceVisual
            | Pending::ReplaceVisualBlock
            | Pending::None => {}
        }

        if let Key::Char(digit @ '1'..='9') = key {
            return Ok(self.push_count(digit));
        }
        if key == Key::Char('0') && self.count.is_some() {
            return Ok(self.push_count('0'));
        }
        if let Some(output) = self.finish_overflowed_count() {
            return Ok(output);
        }

        if let Some(output) = self.try_handle_controller_only_normal_action(document, key) {
            return Ok(output);
        }
        let explicit_count = self.count.take();
        let count = explicit_count.unwrap_or(1).max(1);
        match key {
            Key::Char('.') => self.repeat_last_change(document, explicit_count),
            Key::Char('u') => Ok(self.history(document, false, count)),
            Key::Ctrl('r' | 'R') => Ok(self.history(document, true, count)),
            Key::Char('i') => Ok(self.enter_insert(document, InsertPlacement::Before, count)),
            Key::Char('I') => Ok(self.enter_insert(document, InsertPlacement::LineStart, count)),
            Key::Char('a') => Ok(self.enter_insert(document, InsertPlacement::After, count)),
            Key::Char('A') => Ok(self.enter_insert(document, InsertPlacement::LineEnd, count)),
            Key::Char('o') => self.open_line(document, false, count),
            Key::Char('O') => self.open_line(document, true, count),
            Key::Char('R') => Ok(self.enter_insert(document, InsertPlacement::Replace, count)),
            Key::Char('x') | Key::Delete => {
                let register = self.requested_register.take();
                self.delete_characters(document, count, register, false)
            }
            Key::Char('X') => {
                let register = self.requested_register.take();
                self.delete_characters(document, count, register, true)
            }
            Key::Char('s') => {
                let register = self.requested_register.take();
                let lines = document.hard_line_snapshot();
                let end = advance_graphemes(document.text(), self.cursor, count)
                    .min(line_end(&lines, self.cursor));
                self.apply_operator_with_repeat(
                    document,
                    Operator::Change,
                    MotionExtent {
                        range: self.cursor..end,
                        kind: MotionKind::Characterwise,
                    },
                    register,
                    RepeatTarget::Characters,
                    count,
                )
            }
            Key::Char('S') => self.change_current_lines(document, count),
            Key::Char('D') => self.operator_to_line_end(document, Operator::Delete, count),
            Key::Char('C') => self.operator_to_line_end(document, Operator::Change, count),
            Key::Char('Y') => self.yank_current_lines(document, count),
            Key::Char('p') => self.paste(document, false, count),
            Key::Char('P') => self.paste(document, true, count),
            Key::Char('J') => self.join_lines(document, count, true),
            Key::Char('~') => self.toggle_at_cursor(document, count),
            Key::Char('H' | 'M' | 'L')
            | Key::Ctrl('f' | 'F' | 'b' | 'B' | 'd' | 'D' | 'u' | 'U' | 'e' | 'E' | 'y' | 'Y')
            | Key::PageUp
            | Key::PageDown => Ok(layout_required("viewport command")),
            Key::Ctrl('v' | 'V' | 'q' | 'Q') => Ok(layout_required("visual block command")),
            Key::Ctrl(_) => {
                self.requested_register = None;
                Ok(CommandOutput::unsupported(format!("normal key {key:?}")))
            }
            Key::Char(character) => {
                self.requested_register = None;
                Ok(CommandOutput::unsupported(format!(
                    "normal command {character}"
                )))
            }
            _ => Ok(CommandOutput::unsupported(format!("normal key {key:?}"))),
        }
    }

    fn handle_g_command(
        &mut self,
        document: &mut Document,
        key: Key,
        count: usize,
        register: Option<char>,
    ) -> Result<CommandOutput, DocumentError> {
        match key {
            Key::Char('j' | 'k' | '0' | '^' | '$') => Ok(layout_required("visual-row motion")),
            Key::Char('J') => self.join_lines(document, count, false),
            Key::Char('p') => {
                self.requested_register = register;
                self.paste_after_and_follow(document, false, count)
            }
            Key::Char('P') => {
                self.requested_register = register;
                self.paste_after_and_follow(document, true, count)
            }
            _ => {
                self.requested_register = None;
                Ok(CommandOutput::unsupported(format!("g{key:?}")))
            }
        }
    }

    fn start_operator(&mut self, operator: Operator, count: usize, count_explicit: bool) {
        self.pending = Pending::Operator(PendingOperator {
            operator,
            operator_count: count,
            operator_count_explicit: count_explicit,
            motion_count: None,
            register: self.requested_register.take(),
            g_prefix: false,
        });
    }

    fn handle_operator_key(
        &mut self,
        document: &mut Document,
        key: Key,
        mut pending: PendingOperator,
    ) -> Result<CommandOutput, DocumentError> {
        if let Key::Char(digit @ '1'..='9') = key {
            return Ok(self.push_operator_motion_count(pending, digit));
        }
        if key == Key::Char('0') && pending.motion_count.is_some() {
            return Ok(self.push_operator_motion_count(pending, '0'));
        }
        if key == Key::Char('g') && !pending.g_prefix {
            pending.g_prefix = true;
            self.pending = Pending::Operator(pending);
            return Ok(CommandOutput::pending());
        }

        if !pending.g_prefix {
            if let Key::Char(scope @ ('i' | 'a')) = key {
                self.pending = Pending::TextObject {
                    operator: pending,
                    scope: if scope == 'i' {
                        TextObjectScope::Inner
                    } else {
                        TextObjectScope::Around
                    },
                };
                return Ok(CommandOutput::pending());
            }
        }

        if !pending.g_prefix {
            if let Key::Char(command @ ('f' | 'F' | 't' | 'T')) = key {
                self.pending = Pending::OperatorFind {
                    operator: pending,
                    forward: matches!(command, 'f' | 't'),
                    till: matches!(command, 't' | 'T'),
                };
                return Ok(CommandOutput::pending());
            }
            if let Key::Char(command @ (';' | ',')) = key {
                let Some(mut find) = self.last_find.clone() else {
                    self.pending = Pending::None;
                    return Ok(CommandOutput {
                        status: CommandStatus::Error("no previous character find".into()),
                        ..CommandOutput::complete()
                    });
                };
                if command == ',' {
                    find.forward = !find.forward;
                }
                self.pending = Pending::None;
                return self.execute_operator_find(document, pending, find, true);
            }
            if let Key::Char(command @ ('`' | '\'')) = key {
                self.pending = Pending::OperatorMark {
                    operator: pending,
                    linewise: command == '\'',
                };
                return Ok(CommandOutput::pending());
            }
            if let Key::Char(command @ ('/' | '?')) = key {
                self.enter_operator_search(
                    if command == '/' {
                        SearchDirection::Forward
                    } else {
                        SearchDirection::Backward
                    },
                    pending,
                );
                return Ok(CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::pending()
                });
            }
            if let Key::Char(command @ ('n' | 'N')) = key {
                self.pending = Pending::None;
                return self.execute_operator_repeat_search(document, pending, command == 'N');
            }
            if let Key::Char(command @ ('*' | '#')) = key {
                self.pending = Pending::None;
                return self.execute_operator_word_search(document, pending, command == '*', true);
            }
        } else if let Key::Char(command @ ('*' | '#')) = key {
            self.pending = Pending::None;
            return self.execute_operator_word_search(document, pending, command == '*', false);
        }

        self.pending = Pending::None;
        let has_explicit_count = pending.operator_count_explicit || pending.motion_count.is_some();
        let count = match effective_operator_count(pending) {
            Ok(count) => count,
            Err(error) => return Ok(CommandOutput::count_error(error)),
        };
        let long_case_doubled = pending.g_prefix
            && pending.operator.is_g_prefixed()
            && key == Key::Char(pending.operator.doubled_key());
        if (key == Key::Char(pending.operator.doubled_key()) && !pending.g_prefix)
            || long_case_doubled
        {
            let lines = document.hard_line_snapshot();
            let range = linewise_range(&lines, self.cursor, count);
            return self.apply_operator_with_repeat(
                document,
                pending.operator,
                MotionExtent {
                    range,
                    kind: MotionKind::Linewise,
                },
                pending.register,
                RepeatTarget::Lines,
                count,
            );
        }

        if pending.g_prefix && matches!(key, Key::Char('j' | 'k' | '0' | '^' | '$')) {
            return Ok(layout_required("visual-row operator motion"));
        }

        let motion = if pending.g_prefix {
            match key {
                Key::Char('g') => Some(OperatorMotion::FirstLine),
                Key::Char('e') => Some(OperatorMotion::WordEndBackward(false)),
                Key::Char('E') => Some(OperatorMotion::WordEndBackward(true)),
                Key::Char('_') => Some(OperatorMotion::LastNonBlank),
                _ => None,
            }
        } else if key == Key::Char('G') && has_explicit_count {
            Some(OperatorMotion::FirstLine)
        } else if key == Key::Char('%') && has_explicit_count {
            if !(1..=100).contains(&count) {
                return Ok(invalid_percentage(count));
            }
            Some(OperatorMotion::Percentage)
        } else {
            operator_motion_for_key(key)
        };
        let Some(motion) = motion else {
            return Ok(CommandOutput::unsupported("operator motion"));
        };
        let Some(extent) = self.resolve_operator_extent(document, pending.operator, motion, count)
        else {
            return Ok(CommandOutput::complete());
        };
        self.apply_operator_motion_with_jump(
            document,
            pending.operator,
            extent,
            pending.register,
            motion,
            count,
        )
    }

    fn resolve_operator_extent(
        &self,
        document: &Document,
        operator: Operator,
        motion: OperatorMotion,
        count: usize,
    ) -> Option<MotionExtent> {
        let text = document.text();
        let motion = if operator == Operator::Change
            && matches!(motion, OperatorMotion::WordForward(_))
            && grapheme_range_at(text, self.cursor)
                .is_some_and(|range| !text[range].chars().all(char::is_whitespace))
        {
            match motion {
                OperatorMotion::WordForward(big) => OperatorMotion::WordEnd(big),
                _ => unreachable!("change-word special case checked above"),
            }
        } else {
            motion
        };
        self.resolve_operator_motion(document, motion, count)
    }

    fn resolve_operator_motion(
        &self,
        document: &Document,
        motion: OperatorMotion,
        count: usize,
    ) -> Option<MotionExtent> {
        let text = document.text();
        let lines = document.hard_line_snapshot();
        let origin = self.cursor.min(text.len());
        let characterwise = |destination: usize, inclusive: bool| {
            let (start, mut end) = if destination < origin {
                (destination, origin)
            } else {
                (origin, destination)
            };
            if inclusive {
                end = advance_graphemes(text, end, 1);
            }
            MotionExtent {
                range: start..end,
                kind: MotionKind::Characterwise,
            }
        };
        match motion {
            OperatorMotion::Left => Some(characterwise(
                move_horizontal(text, &lines, origin, directional_count(count, false)),
                false,
            )),
            OperatorMotion::Right => Some(characterwise(
                move_horizontal(text, &lines, origin, directional_count(count, true)),
                false,
            )),
            OperatorMotion::Down | OperatorMotion::Up => {
                let amount = directional_count(count, motion == OperatorMotion::Down);
                let destination = move_vertical(text, &lines, origin, amount);
                let first = line_start(&lines, origin.min(destination));
                let last = line_start(&lines, origin.max(destination));
                Some(MotionExtent {
                    range: first..line_range(&lines, last).end,
                    kind: MotionKind::Linewise,
                })
            }
            OperatorMotion::LineStart => Some(characterwise(line_start(&lines, origin), false)),
            OperatorMotion::FirstNonBlank => {
                Some(characterwise(first_non_blank(text, &lines, origin), false))
            }
            OperatorMotion::LineEnd => {
                let mut destination = origin;
                for index in 0..count {
                    destination = last_grapheme_on_line(text, &lines, destination);
                    if index + 1 < count {
                        let Some(next) = next_line_start(&lines, destination) else {
                            break;
                        };
                        destination = next;
                    }
                }
                Some(characterwise(destination, true))
            }
            OperatorMotion::Column => Some(characterwise(
                position_at_column(
                    text,
                    &lines,
                    line_start(&lines, origin),
                    count.saturating_sub(1),
                ),
                false,
            )),
            OperatorMotion::WordForward(big) => Some(characterwise(
                move_word_forward(text, origin, big, count),
                false,
            )),
            OperatorMotion::WordEnd(big) => {
                Some(characterwise(move_word_end(text, origin, big, count), true))
            }
            OperatorMotion::WordBackward(big) => Some(characterwise(
                move_word_backward(text, origin, big, count),
                false,
            )),
            OperatorMotion::WordEndBackward(big) => Some(characterwise(
                move_word_end_backward(text, origin, big, count),
                true,
            )),
            OperatorMotion::LastNonBlank => {
                let mut target = origin;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, target) else {
                        break;
                    };
                    target = next;
                }
                Some(characterwise(last_non_blank(text, &lines, target), true))
            }
            OperatorMotion::Sentence(forward) => Some(characterwise(
                move_sentence(text, &lines, origin, forward, count),
                false,
            )),
            OperatorMotion::Paragraph(forward) => Some(exclusive_motion_extent(
                text,
                &lines,
                origin,
                move_paragraph(&lines, origin, forward, count),
            )),
            OperatorMotion::MatchPair => {
                matching_pair(text, &lines, origin).map(|target| characterwise(target, true))
            }
            OperatorMotion::Percentage => {
                let target = nth_line_start(&lines, percentage_line(&lines, count)?);
                let first = line_start(&lines, origin.min(target));
                let last = line_start(&lines, origin.max(target));
                Some(MotionExtent {
                    range: first..line_range(&lines, last).end,
                    kind: MotionKind::Linewise,
                })
            }
            OperatorMotion::LastLine => {
                let target = nth_line_start(&lines, line_count(&lines));
                let first = line_start(&lines, origin.min(target));
                let last = line_start(&lines, origin.max(target));
                Some(MotionExtent {
                    range: first..line_range(&lines, last).end,
                    kind: MotionKind::Linewise,
                })
            }
            OperatorMotion::FirstLine => {
                let target = nth_line_start(&lines, count);
                let first = line_start(&lines, origin.min(target));
                let last = line_start(&lines, origin.max(target));
                Some(MotionExtent {
                    range: first..line_range(&lines, last).end,
                    kind: MotionKind::Linewise,
                })
            }
        }
    }

    fn apply_operator(
        &mut self,
        document: &mut Document,
        operator: Operator,
        extent: MotionExtent,
        register: Option<char>,
        deletion_class: DeletionClass,
    ) -> Result<CommandOutput, DocumentError> {
        self.apply_operator_with_application_count(
            document,
            operator,
            extent,
            register,
            deletion_class,
            1,
        )
    }

    fn apply_operator_with_application_count(
        &mut self,
        document: &mut Document,
        operator: Operator,
        mut extent: MotionExtent,
        register: Option<char>,
        deletion_class: DeletionClass,
        application_count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        if matches!(
            operator,
            Operator::Yank | Operator::Delete | Operator::Change
        ) {
            if let Err(output) = self.require_register_write(register) {
                return Ok(output);
            }
        }
        let lines = document.hard_line_snapshot();
        document.validate_hard_line_snapshot(&lines)?;
        if operator.is_format() {
            // Reflow never reads registers and counts complete hard lines
            // without materializing the compatibility flat text.
            let covered = Self::hard_lines_covered(&lines, &extent.range);
            return self.apply_format_operator(
                document,
                operator == Operator::FormatKeepCursor,
                covered,
            );
        }
        if matches!(
            operator,
            Operator::Indent | Operator::Outdent | Operator::Reindent
        ) {
            extent.range = covered_line_range(&lines, extent.range);
            extent.kind = MotionKind::Linewise;
        }
        let full_selection = matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine)
            && extent.range.start == 0
            && extent.range.end == document.projection().text_tree().byte_len();
        if extent.range.is_empty()
            && !full_selection
            && operator != Operator::Change
            && !(operator == Operator::Delete && extent.kind == MotionKind::Linewise)
        {
            return Ok(CommandOutput::complete());
        }

        match operator {
            Operator::Yank => {
                let old_cursor = self.cursor;
                let mut value = register_value(document, &lines, &extent, register);
                if self.clipboard_copy_as_seen && value.clipboard_fragment().is_none() {
                    value.clipboard_fragment = document.clipboard_fragment(extent.range.clone()).ok();
                }
                self.yank_register(register, value);
                self.cursor =
                    yank_cursor_after_motion(document.text(), &lines, old_cursor, &extent);
                Ok(CommandOutput {
                    cursor_moved: self.cursor != old_cursor,
                    ..CommandOutput::complete()
                })
            }
            Operator::Delete | Operator::Change => {
                let value = register_value(document, &lines, &extent, register);
                let mut edit_range = extent.range.clone();
                let mut replacement = "";
                if extent.kind == MotionKind::Linewise && operator == Operator::Delete {
                    edit_range = linewise_edit_range(&lines, edit_range);
                }
                if extent.kind == MotionKind::Linewise
                    && operator == Operator::Change
                    && edit_range.end < document.projection().text_tree().byte_len()
                {
                    replacement = "\n";
                }
                if operator == Operator::Change {
                    document.begin_edit_group();
                }
                let result = if full_selection && replacement.is_empty() {
                    document.clear_document_content()
                } else if operator == Operator::Delete && extent.kind == MotionKind::Linewise
                {
                    document.delete_lines(edit_range.clone())
                } else {
                    document.replace(edit_range.clone(), replacement)
                };
                if let Err(error) = result {
                    if operator == Operator::Change {
                        document.end_edit_group();
                    }
                    return Err(error);
                }
                self.delete_register(register, value, deletion_class);
                self.cursor = edit_range.start.min(document.projection().text_tree().byte_len());
                let mut output = CommandOutput {
                    document_changed: true,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                };
                if operator == Operator::Change {
                    self.mode = Mode::Insert;
                    self.insert_session = Some(InsertSession {
                        placement: InsertPlacement::Before,
                        repeat_program: Some(EditSessionProgram::default()),
                        last_inserted: RegisterValue::characterwise(""),
                        entry_count: 1,
                        replaying_program: false,
                        preserve_normal_repeat: false,
                        unit_floor: self.cursor,
                        repeat_operator: None,
                        repeat_visual_operator: None,
                        replace_journal: Vec::new(),
                    });
                    output.mode_changed = true;
                } else {
                    self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                        self.cursor,
                    );
                }
                Ok(output)
            }
            Operator::Indent | Operator::Outdent | Operator::Reindent => {
                let edits = match indent_edits(
                    document.text(),
                    &lines,
                    &extent.range,
                    operator,
                    application_count,
                ) {
                    Ok(edits) => edits,
                    Err(error) => return Ok(error.into_command_output()),
                };
                if edits.is_empty() {
                    return Ok(CommandOutput::complete());
                }
                document.validate_hard_line_snapshot(&lines)?;
                document.apply_edits(edits)?;
                self.cursor = first_nonblank_document(document, &document.hard_line_snapshot(),
                    extent.range.start,
                );
                Ok(CommandOutput {
                    document_changed: true,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                })
            }
            Operator::Format | Operator::FormatKeepCursor => {
                unreachable!("reflow returned before register and extent policy")
            }
            Operator::ToggleCase | Operator::Lowercase | Operator::Uppercase => {
                let old = document.text()[extent.range.clone()].to_owned();
                let replacement = change_case(&old, operator);
                if old == replacement {
                    return Ok(CommandOutput::complete());
                }
                document.replace(extent.range.clone(), &replacement)?;
                self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                    extent.range.start,
                );
                Ok(CommandOutput {
                    document_changed: true,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                })
            }
        }
    }

    fn apply_operator_with_repeat(
        &mut self,
        document: &mut Document,
        operator: Operator,
        extent: MotionExtent,
        register: Option<char>,
        target: RepeatTarget,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        self.apply_operator_with_repeat_and_application_count(
            document, operator, extent, register, target, count, 1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_operator_with_repeat_and_application_count(
        &mut self,
        document: &mut Document,
        operator: Operator,
        extent: MotionExtent,
        register: Option<char>,
        target: RepeatTarget,
        count: usize,
        application_count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let deletion_class =
            deletion_class_for_target(&document.hard_line_snapshot(), &extent, &target);
        let before = document.revision();
        let output = self.apply_operator_with_application_count(
            document,
            operator,
            extent,
            register,
            deletion_class,
            application_count,
        )?;
        if document.revision() != before && !self.replaying && operator != Operator::Yank {
            let command = OperatorRepeat {
                operator,
                target,
                count,
                register,
            };
            if operator == Operator::Change {
                if let Some(session) = self.insert_session.as_mut() {
                    session.repeat_operator = Some(command);
                }
            } else {
                self.last_repeat = Some(RepeatAction::Operator {
                    command,
                    edits: None,
                });
            }
        }
        Ok(output)
    }

    fn apply_operator_motion_with_jump(
        &mut self,
        document: &mut Document,
        operator: Operator,
        extent: MotionExtent,
        register: Option<char>,
        motion: OperatorMotion,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let destination = self.operator_jump_destination(document, motion, count);
        self.apply_operator_target_with_jump(
            document,
            operator,
            extent,
            register,
            OperatorTarget {
                repeat: RepeatTarget::Motion(motion),
                count,
                jump_destination: destination,
            },
        )
    }

    fn apply_operator_target_with_jump(
        &mut self,
        document: &mut Document,
        operator: Operator,
        extent: MotionExtent,
        register: Option<char>,
        target: OperatorTarget,
    ) -> Result<CommandOutput, DocumentError> {
        let origin = self.cursor;
        let should_record = target
            .jump_destination
            .is_some_and(|destination| destination != origin);
        let before_revision = document.revision();
        let origin_anchor = should_record.then(|| {
            document.text_point(origin).ok().and_then(|point| {
                document
                    .text_anchor(
                        point,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )
                    .ok()
            })
        });
        let origin_anchor = origin_anchor.flatten();
        let (result, map) = document.capture_position_maps(|document| {
            self.apply_operator_with_repeat(
                document,
                operator,
                extent,
                register,
                target.repeat,
                target.count,
            )
        });
        let output = result?;

        let succeeded = operator == Operator::Yank || before_revision != document.revision();
        if succeeded {
            let Some(origin_anchor) = origin_anchor else {
                return Ok(output);
            };
            let mapped_origin = if before_revision == document.revision() {
                Some(origin_anchor.offset())
            } else {
                mapped_anchor(&map, origin_anchor).ok().flatten()
            };
            if let Some(mapped_origin) = mapped_origin {
                self.record_jump(document, mapped_origin, self.cursor);
            }
        }
        Ok(output)
    }

    fn operator_jump_destination(
        &self,
        document: &Document,
        motion: OperatorMotion,
        count: usize,
    ) -> Option<usize> {
        let text = document.text();
        let lines = document.hard_line_snapshot();
        let origin = self.cursor.min(text.len());
        match motion {
            OperatorMotion::Sentence(forward) => {
                Some(move_sentence(text, &lines, origin, forward, count))
            }
            OperatorMotion::Paragraph(forward) => {
                Some(move_paragraph(&lines, origin, forward, count))
            }
            OperatorMotion::MatchPair => matching_pair(text, &lines, origin),
            OperatorMotion::Percentage => {
                Some(nth_line_start(&lines, percentage_line(&lines, count)?))
            }
            OperatorMotion::LastLine => Some(nth_line_start(&lines, line_count(&lines))),
            OperatorMotion::FirstLine => Some(nth_line_start(&lines, count)),
            _ => None,
        }
    }

    fn handle_visual_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        // Selection-changing grammar/motions share the immutable dispatcher;
        // this fallback owns edits and their pending-state-specific behavior.
        match self.pending {
            Pending::G { count, .. } => {
                self.pending = Pending::None;
                return Ok(match key {
                    Key::Char('j' | 'k' | '0' | '^' | '$') => layout_required("visual-row motion"),
                    Key::Char('~') => {
                        return self.apply_visual_operator(document, Operator::ToggleCase, count)
                    }
                    Key::Char('u') => {
                        return self.apply_visual_operator(document, Operator::Lowercase, count)
                    }
                    Key::Char('U') => {
                        return self.apply_visual_operator(document, Operator::Uppercase, count)
                    }
                    Key::Char('q') => {
                        return self.apply_visual_operator(document, Operator::Format, count)
                    }
                    Key::Char('w') => {
                        return self.apply_visual_operator(
                            document,
                            Operator::FormatKeepCursor,
                            count,
                        )
                    }
                    Key::Char('J') => return self.visual_join(document, false),
                    _ => CommandOutput::unsupported(format!("visual g{key:?}")),
                });
            }
            _ => {}
        }
        if (key == Key::Char('v') && self.mode == Mode::VisualCharacter)
            || (key == Key::Char('V') && self.mode == Mode::VisualLine)
        {
            self.leave_visual();
            return Ok(CommandOutput {
                status: CommandStatus::Cancelled,
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        if matches!(self.pending, Pending::ReplaceVisual) {
            self.pending = Pending::None;
            return match key {
                Key::Char(character) => self.replace_visual(
                    document,
                    &RegisterValue::characterwise(character.to_string()),
                ),
                // Vim's characterwise and linewise Visual r<Enter> inserts a
                // literal carriage return for each selected content grapheme;
                // existing semantic hard-line separators remain separators.
                Key::Enter => self.replace_visual(
                    document,
                    &RegisterValue::try_new("\r", RegisterKind::Characterwise, Vec::new())
                        .expect("literal carriage return has valid break metadata"),
                ),
                Key::Tab => self.replace_visual(document, &RegisterValue::characterwise("\t")),
                _ => Ok(CommandOutput::unsupported("visual r expects text")),
            };
        }
        if let Key::Char(digit @ '1'..='9') = key {
            return Ok(self.push_count(digit));
        }
        if key == Key::Char('0') && self.count.is_some() {
            return Ok(self.push_count('0'));
        }
        if let Some(output) = self.finish_overflowed_count() {
            return Ok(output);
        }
        if let Some(output) = self.try_handle_controller_only_visual_action(document, key) {
            return Ok(output);
        }
        let count = self.count.take().unwrap_or(1).max(1);
        match key {
            Key::Char('y') => self.apply_visual_operator(document, Operator::Yank, count),
            Key::Char('d' | 'x') | Key::Backspace | Key::Delete => {
                self.apply_visual_operator(document, Operator::Delete, count)
            }
            Key::Char('c' | 's') => self.apply_visual_operator(document, Operator::Change, count),
            Key::Char('>') => self.apply_visual_operator(document, Operator::Indent, count),
            Key::Char('<') => self.apply_visual_operator(document, Operator::Outdent, count),
            Key::Char('=') => self.apply_visual_operator(document, Operator::Reindent, count),
            Key::Char('~') => self.apply_visual_operator(document, Operator::ToggleCase, count),
            Key::Char('u') => self.apply_visual_operator(document, Operator::Lowercase, count),
            Key::Char('U') => self.apply_visual_operator(document, Operator::Uppercase, count),
            Key::Char('p') => self.visual_paste(document, false, count),
            Key::Char('P') => self.visual_paste(document, true, count),
            Key::Char('J') => self.visual_join(document, true),
            Key::Char('H' | 'M' | 'L')
            | Key::Ctrl('f' | 'F' | 'b' | 'B' | 'd' | 'D' | 'u' | 'U' | 'e' | 'E' | 'y' | 'Y')
            | Key::PageUp
            | Key::PageDown => Ok(layout_required("viewport command")),
            _ => Ok(CommandOutput::unsupported(format!("visual key {key:?}"))),
        }
    }

    fn visual_extent(&self, document: &Document) -> MotionExtent {
        let lines = document.hard_line_snapshot();
        let anchor = self.visual_anchor.unwrap_or(self.cursor);
        if self.mode == Mode::VisualLine {
            let start = line_start(&lines, anchor.min(self.cursor));
            let last = line_start(&lines, anchor.max(self.cursor));
            MotionExtent {
                range: start..line_range(&lines, last).end,
                kind: MotionKind::Linewise,
            }
        } else {
            let start = anchor.min(self.cursor);
            let high = anchor.max(self.cursor);
            MotionExtent {
                range: start..lines.next_grapheme_boundary(high).unwrap_or(high),
                kind: MotionKind::Characterwise,
            }
        }
    }

    fn visual_repeat_shape_for(
        document: &Document,
        mode: Mode,
        anchor: usize,
        active: usize,
        to_line_end: bool,
    ) -> VisualRepeatShape {
        let lines = document.hard_line_snapshot();
        let start = anchor.min(active);
        let active_end = anchor.max(active);
        let first = lines
            .line_at_offset(start)
            .expect("a Visual endpoint resolves to one hard line");
        let last = lines
            .line_at_offset(active_end)
            .expect("a Visual endpoint resolves to one hard line");
        let hard_line_count = last.index().saturating_sub(first.index()) + 1;

        if mode == Mode::VisualLine {
            return VisualRepeatShape::Line { hard_line_count };
        }

        if first.index() == last.index() {
            let end = lines
                .next_grapheme_boundary(active_end)
                .unwrap_or(active_end)
                .min(first.content_range().end);
            return VisualRepeatShape::CharacterSingleLine {
                grapheme_count: lines
                    .grapheme_count(start..end)
                    .expect("a Visual shape has logical grapheme boundaries"),
                to_line_end,
            };
        }

        let last_content = last.content_range();
        let end = lines
            .next_grapheme_boundary(active_end)
            .unwrap_or(active_end)
            .min(last_content.end);
        VisualRepeatShape::CharacterMultiLine {
            hard_line_count,
            last_line_grapheme_count: lines
                .grapheme_count(last_content.start..end)
                .expect("a Visual shape has logical grapheme boundaries"),
            to_line_end,
        }
    }

    fn visual_repeat_shape(&self, document: &Document) -> VisualRepeatShape {
        Self::visual_repeat_shape_for(
            document,
            self.mode,
            self.visual_anchor.unwrap_or(self.cursor),
            self.cursor,
            self.visual_to_line_end,
        )
    }

    /// Bind the remembered directed Visual selection to content produced by
    /// the just-committed replacement. Unlike a delete, Visual put remembers
    /// the inserted payload, whose size and hard-line topology may differ
    /// from the consumed selection.
    fn visual_memory_for_result_range(
        document: &Document,
        prior: VisualMemory,
        range: Range<usize>,
    ) -> VisualMemory {
        debug_assert!(prior.mode != Mode::VisualBlock);
        let text = document.text();
        let lines = document.hard_line_snapshot();
        let start = range.start.min(text.len());
        let end = range.end.min(text.len()).max(start);
        let (low, high) = if start == end {
            let point = normalize_normal_cursor(text, &lines, start);
            (point, point)
        } else if prior.mode == Mode::VisualLine {
            let low = line_start(&lines, start);
            let final_item = previous_grapheme_boundary(text, end).unwrap_or(start);
            (low, line_start(&lines, final_item))
        } else {
            (
                normalize_normal_cursor(text, &lines, start),
                previous_grapheme_boundary(text, end)
                    .map(|offset| normalize_normal_cursor(text, &lines, offset))
                    .unwrap_or_else(|| normalize_normal_cursor(text, &lines, start)),
            )
        };
        let (anchor, active) = if prior.anchor <= prior.active {
            (low, high)
        } else {
            (high, low)
        };
        VisualMemory {
            anchor,
            active,
            ..prior
        }
    }

    fn resolve_visual_repeat_extent(
        &self,
        document: &Document,
        shape: VisualRepeatShape,
    ) -> MotionExtent {
        let lines = document.hard_line_snapshot();
        match shape {
            VisualRepeatShape::CharacterSingleLine {
                grapheme_count,
                to_line_end,
            } => {
                let content_end = line_end(&lines, self.cursor);
                let end = if to_line_end {
                    content_end
                } else {
                    lines
                        .advance_graphemes(self.cursor, grapheme_count)
                        .unwrap_or(content_end)
                        .min(content_end)
                };
                MotionExtent {
                    range: self.cursor..end,
                    kind: MotionKind::Characterwise,
                }
            }
            VisualRepeatShape::CharacterMultiLine {
                hard_line_count,
                last_line_grapheme_count,
                to_line_end,
            } => {
                let first_line = lines
                    .line_at_offset(self.cursor)
                    .expect("a Normal cursor resolves to one hard line")
                    .index();
                let last_line = first_line
                    .saturating_add(hard_line_count.saturating_sub(1))
                    .min(lines.line_count().saturating_sub(1));
                let last_content = lines
                    .line(last_line)
                    .expect("the repeated Visual line was clamped to the document")
                    .content_range();
                let end = if to_line_end {
                    last_content.end
                } else {
                    lines
                        .advance_graphemes(last_content.start, last_line_grapheme_count)
                        .unwrap_or(last_content.end)
                        .min(last_content.end)
                };
                MotionExtent {
                    range: self.cursor..end.max(self.cursor),
                    kind: MotionKind::Characterwise,
                }
            }
            VisualRepeatShape::Line { hard_line_count } => MotionExtent {
                range: linewise_range(&lines, self.cursor, hard_line_count),
                kind: MotionKind::Linewise,
            },
        }
    }

    fn select_visual_text_object(
        &mut self,
        document: &Document,
        scope: TextObjectScope,
        key: char,
        count: usize,
    ) -> CommandOutput {
        let Some(kind) = TextObjectKind::from_vim_key(key) else {
            return CommandOutput::unsupported(format!("unknown text object {key}"));
        };
        let range = match resolve_text_object(
            document.text(),
            &document.hard_line_snapshot(),
            self.cursor,
            TextObject { scope, kind },
            count,
        ) {
            Ok(range) if !range.is_empty() => range,
            Ok(_) => {
                return CommandOutput {
                    status: CommandStatus::Error("text object is empty".into()),
                    ..CommandOutput::complete()
                }
            }
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!(
                        "text object could not be resolved: {error:?}"
                    )),
                    ..CommandOutput::complete()
                }
            }
        };
        let old_cursor = self.cursor;
        let old_mode = self.mode;
        self.mode = Mode::VisualCharacter;
        self.visual_to_line_end = false;
        self.visual_anchor = Some(range.start);
        self.cursor = document.hard_line_snapshot().previous_grapheme_boundary(range.end).unwrap_or(range.start);
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        CommandOutput {
            cursor_moved: old_cursor != self.cursor,
            mode_changed: old_mode != self.mode,
            ..CommandOutput::complete()
        }
    }

    fn apply_visual_operator(
        &mut self,
        document: &mut Document,
        operator: Operator,
        command_count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(output) = self.apply_mode_visual_operator(document, operator, command_count)? {
            return Ok(output);
        }
        let remembered = self.visual_anchor.map(|anchor| VisualMemory {
            mode: self.mode,
            anchor,
            active: self.cursor,
            to_line_end: self.visual_to_line_end,
            block: None,
        });
        let shape = self.visual_repeat_shape(document);
        let extent = self.visual_extent(document);
        let visual_yank_target = extent.range.start;
        let cursor_before = self.cursor;
        let lines = document.hard_line_snapshot();
        let deletion_class = ordinary_deletion_class(&lines, &extent);
        let register = self.requested_register.take();
        let command = VisualOperatorRepeat {
            operator,
            shape,
            application_count: command_count,
            register,
        };
        let before = document.revision();
        let mut output = self.apply_operator_with_application_count(
            document,
            operator,
            extent,
            register,
            deletion_class,
            command_count,
        )?;
        if output.status != CommandStatus::Complete {
            return Ok(output);
        }
        if operator == Operator::Yank {
            self.cursor = visual_yank_target;
            output.cursor_moved = self.cursor != cursor_before;
        }
        if document.revision() != before && !self.replaying && operator != Operator::Yank {
            if operator == Operator::Change {
                if let Some(session) = self.insert_session.as_mut() {
                    session.repeat_visual_operator = Some(command);
                }
            } else {
                self.last_repeat = Some(RepeatAction::VisualOperator {
                    command,
                    edits: None,
                });
            }
        }
        self.last_visual = remembered;
        self.visual_anchor = None;
        self.visual_to_line_end = false;
        if operator != Operator::Change {
            self.mode = Mode::Normal;
        }
        output.mode_changed = true;
        Ok(output)
    }

    fn replace_visual(
        &mut self,
        document: &mut Document,
        replacement_value: &RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        let shape = self.visual_repeat_shape(document);
        self.remember_visual();
        let remembered = self.last_visual;
        let extent = self.visual_extent(document);
        let result_start = extent.range.start;
        let (mut output, replacement_len) =
            self.replace_visual_extent(document, extent, replacement_value)?;
        if output.document_changed {
            if let Some(remembered) = remembered {
                self.last_visual = Some(Self::visual_memory_for_result_range(
                    document,
                    remembered,
                    result_start..result_start.saturating_add(replacement_len),
                ));
            }
        }
        if output.document_changed && !self.replaying {
            self.last_repeat = Some(RepeatAction::VisualReplace {
                shape,
                value: replacement_value.clone(),
            });
        }
        self.visual_anchor = None;
        self.leave_visual();
        output.mode_changed = true;
        Ok(output)
    }

    fn replace_visual_extent(
        &mut self,
        document: &mut Document,
        extent: MotionExtent,
        replacement_value: &RegisterValue,
    ) -> Result<(CommandOutput, usize), DocumentError> {
        let lines = document.hard_line_snapshot();
        let mut replacement = String::new();
        let mut hard_break_offsets = Vec::new();
        for range in lines
            .grapheme_ranges(extent.range.clone())
            .expect("a resolved Visual extent has logical grapheme boundaries")
        {
            if is_hard_line_separator(&lines, &range) {
                hard_break_offsets.push(replacement.len());
                replacement.push_str(&document.text()[range]);
            } else {
                let base = replacement.len();
                replacement.push_str(&replacement_value.text);
                hard_break_offsets.extend(
                    replacement_value
                        .hard_break_offsets()
                        .iter()
                        .map(|offset| base + *offset),
                );
            }
        }
        let value = RegisterValue::try_new(replacement, RegisterKind::Characterwise, hard_break_offsets)
            .expect("Visual replacement constructs valid hard-break metadata");
        let value = self.assist_input_payload(
            document, extent.range.clone(), BoundaryAffinity::Downstream, &value,
        )?;
        let replacement_len = value.text.len();
        let payload = FormattedTextPayload::new(&lines, value.text.clone(), value.hard_break_offsets().to_vec())
            .expect("Visual replacement constructs valid hard-break metadata");
        let before = document.revision();
        document.replace_with_formatted_payload(extent.range.clone(), payload)?;
        self.registers.set_last_insert(inserted_input_unit(&value, &replacement_value.text));
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            extent.range.start,
        );
        Ok((
            CommandOutput {
                document_changed: document.revision() != before,
                cursor_moved: true,
                ..CommandOutput::complete()
            },
            replacement_len,
        ))
    }

    fn visual_paste(
        &mut self,
        document: &mut Document,
        preserve_unnamed: bool,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let remembered = self.visual_anchor.map(|anchor| VisualMemory {
            mode: self.mode,
            anchor,
            active: self.cursor,
            to_line_end: self.visual_to_line_end,
            block: None,
        });
        let shape = self.visual_repeat_shape(document);
        let extent = self.visual_extent(document);
        let lines = document.hard_line_snapshot();
        let register_name = self.requested_register.take().unwrap_or('"');
        let register = match self.require_register_value(document, register_name) {
            Ok(register) => register,
            Err(output) => return Ok(output),
        };
        let register = match checked_register_repetition(&register, count) {
            Ok(register) => register,
            Err(error) => return Ok(error.into_command_output()),
        };
        let replaced = register_value(document, &lines, &extent, None);
        let deletion_class = ordinary_deletion_class(&lines, &extent);
        let selection_was_linewise = extent.kind == MotionKind::Linewise;
        let register_is_linewise = register.kind == RegisterKind::Linewise;
        let selection_kept_following_line = selection_was_linewise
            && lines
                .capture(extent.range.clone())
                .expect("a resolved Visual extent is a valid payload range")
                .break_offsets()
                .last()
                .is_some_and(|offset| *offset + 1 == extent.range.len());
        let (fragment, cursor_policy) = match (selection_was_linewise, register_is_linewise) {
            (false, true) => (
                linewise_fragment_for_character_selection(&register),
                VisualPasteCursor::FirstNonBlank { relative: 1 },
            ),
            (true, _) => (
                fragment_for_linewise_selection(&register, selection_kept_following_line),
                VisualPasteCursor::FirstNonBlank { relative: 0 },
            ),
            (false, false) => (
                StructuredFragment::from_register(&register),
                if register.hard_break_offsets().is_empty() && !register.text.is_empty() {
                    VisualPasteCursor::LastInsertedGrapheme
                } else {
                    VisualPasteCursor::Start
                },
            ),
        };
        let value = if fragment.text == register.text
            && fragment.hard_break_offsets == register.hard_break_offsets()
        {
            register
        } else {
            RegisterValue::try_new(
                fragment.text, RegisterKind::Characterwise, fragment.hard_break_offsets,
            ).expect("Visual put constructs valid semantic-break offsets")
        };
        let register = self.assist_input_payload(
            document, extent.range.clone(), BoundaryAffinity::Downstream, &value,
        )?;
        let fragment = StructuredFragment::from_register(&register);
        let inserted_payload = FormattedTextPayload::new(
            &lines,
            fragment.text.clone(),
            fragment.hard_break_offsets.clone(),
        )
        .expect("Visual put carries validated semantic-break offsets");
        let edit = document.normalize_typing_payload(FormattedPayloadEdit::new(
            extent.range.clone(), inserted_payload,
        ))?;
        let mut inserted_len = edit.payload().text().len();
        let before_revision = document.revision();
        let private = register.clipboard_fragment().map(|payload|
            document.prepare_clipboard_fragment(extent.range.clone(), payload, &fragment.text)
        ).transpose().map_err(command_document_error)?.flatten();
        let prepared = match private {
            Some(prepared) => prepared,
            None => document.prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                document.id(), document.revision(), vec![edit.clone()],
            )).map_err(command_document_error)?,
        };
        let mapped_start = prepared_cursor(
            document, &prepared, extent.range.start, Association::BeforeInsertion,
        )?;
        if document.format() == crate::document::Format::MarkdownSource {
            inserted_len = prepared_payload_caret(document, &prepared, &edit)?
                .saturating_sub(mapped_start);
        }
        document.commit_model_transaction(prepared).map_err(command_document_error)?;
        self.last_visual = remembered.map(|remembered| {
            Self::visual_memory_for_result_range(
                document,
                remembered,
                mapped_start..mapped_start.saturating_add(inserted_len),
            )
        });
        if !preserve_unnamed {
            self.delete_register(None, replaced, deletion_class);
        }
        let new_lines = document.hard_line_snapshot();
        let cursor_target = match cursor_policy {
            VisualPasteCursor::Start => mapped_start,
            VisualPasteCursor::FirstNonBlank { relative } => first_nonblank_document(document, &new_lines,
                mapped_start
                    .saturating_add(relative)
                    .min(document.projection().text_tree().byte_len()),
            ),
            VisualPasteCursor::LastInsertedGrapheme => previous_grapheme_boundary(
                document.text(),
                mapped_start.saturating_add(inserted_len),
            )
            .unwrap_or(mapped_start),
        };
        self.cursor = normalize_normal_cursor_document(document, &new_lines, cursor_target);
        self.visual_anchor = None;
        self.leave_visual();
        let changed = document.revision() != before_revision;
        if changed && !self.replaying {
            // Vim implements Visual put as a put followed by deletion of the
            // selected area. Dot repeats that final, selection-shaped delete.
            // `P` uses the black-hole register so the unnamed register remains
            // intact; `p` performs an ordinary deletion.
            self.last_repeat = Some(RepeatAction::VisualOperator {
                command: VisualOperatorRepeat {
                    operator: Operator::Delete,
                    shape,
                    application_count: 1,
                    register: preserve_unnamed.then_some('_'),
                },
                edits: None,
            });
        }
        Ok(CommandOutput {
            document_changed: changed,
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn visual_join(
        &mut self,
        document: &mut Document,
        insert_space: bool,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(output) = self.mode_visual_join(document, insert_space)? {
            return Ok(output);
        }
        let shape = self.visual_repeat_shape(document);
        self.remember_visual();
        let extent = self.visual_extent(document);
        let count = hard_line_count_for_range(&document.hard_line_snapshot(), &extent.range);
        self.cursor = extent.range.start;
        self.visual_anchor = None;
        self.leave_visual();
        let before = document.revision();
        let mut output = self.join_hard_lines(document, count, count, insert_space)?;
        if document.revision() != before && !self.replaying {
            self.last_repeat = Some(RepeatAction::VisualJoin {
                shape,
                insert_space,
            });
        }
        output.mode_changed = true;
        Ok(output)
    }

    fn visual_block_join(
        &mut self,
        document: &mut Document,
        context: &LayoutCommandContext<'_>,
        insert_space: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let resolved = match self.resolved_visual_block(document, context) {
            Ok(resolved) => resolved,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let repeat_shape = match self.visual_block_repeat_shape(&resolved) {
            Ok(shape) => shape,
            Err(error) => return Ok(visual_block_error(error)),
        };
        let Some(first_line) = resolved.rows.first().map(|row| row.hard_line_index) else {
            return Ok(visual_block_error(VisualBlockError::EmptyLayout));
        };
        let last_line = resolved
            .rows
            .last()
            .map_or(first_line, |row| row.hard_line_index);
        let count = last_line.saturating_sub(first_line).saturating_add(1);
        let lines = document.hard_line_snapshot();
        self.cursor = lines
            .line(first_line)
            .expect("a resolved block row names an existing hard line")
            .content_range()
            .start;
        self.leave_visual_block();
        let before = document.revision();
        let mut output = self.join_hard_lines(document, count, count, insert_space)?;
        if document.revision() != before && !self.replaying {
            self.last_repeat = Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                shape: repeat_shape,
                action: VisualBlockRepeatAction::Join { insert_space },
            }));
        }
        output.mode_changed = true;
        Ok(output)
    }

    fn handle_edit_mode_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        if self.visual_block_insert.is_some() {
            return self.handle_visual_block_insert_key(document, key);
        }
        if matches!(key, Key::Escape | Key::Ctrl('[')) {
            return self.finish_insert(document);
        }
        if self.register_pending {
            self.register_pending = false;
            let Key::Char(name) = key else {
                return Ok(CommandOutput::unsupported("Ctrl-R expects a register"));
            };
            if !is_valid_register(name) {
                return Ok(CommandOutput::unsupported("invalid register"));
            }
            let value = match self.require_register_value(document, name) {
                Ok(value) => value,
                Err(output) => return Ok(output),
            };
            let snapshot = document.hard_line_snapshot();
            let edit_end = if self.mode == Mode::Replace {
                replacement_payload_end(&snapshot, self.cursor, &value)
            } else {
                self.cursor
            };
            let assisted = self.assist_typing_input_payload(
                document, self.cursor..edit_end, self.insertion_boundary_affinity(), &value,
            )?;
            let payload = FormattedTextPayload::new(
                &snapshot, assisted.text.clone(), assisted.hard_break_offsets().to_vec(),
            ).expect("register values carry validated semantic-break offsets");
            let request = FormattedPayloadEditRequest::new(
                document.id(),
                document.revision(),
                vec![FormattedPayloadEdit::new(
                    self.cursor..edit_end,
                    payload.clone(),
                )],
            );
            // Reverse projection is fallible. Validate the complete paste
            // before closing the surrounding typed-text unit so rejection
            // cannot introduce an otherwise invisible undo boundary.
            let private = if self.mode == Mode::Insert {
                assisted.clipboard_fragment().map(|fragment| document.prepare_clipboard_fragment(self.cursor..edit_end, fragment, &assisted.text))
                    .transpose().map_err(command_document_error)?.flatten()
            } else { None };
            if private.is_none() {
                document.prepare_formatted_payload_request(request).map_err(command_document_error)?;
            }
            self.invalidate_replace_restoration();
            document.end_edit_group();
            document.begin_edit_group();
            let output = if self.mode == Mode::Replace {
                self.replace_register_payload(document, &value)?
            } else {
                self.insert_register_payload(document, &value)?
            };
            document.end_edit_group();
            document.begin_edit_group();
            if let Some(session) = self.insert_session.as_mut() {
                session.unit_floor = self.cursor;
            }
            return Ok(output);
        }
        if key != Key::Backspace {
            if let Some(request) = self.paragraph_key_request(document, key)? {
                return self.apply_paragraph_key(document, key, request);
            }
        }
        match key {
            Key::Backspace => self.edit_mode_backspace(document),
            Key::Delete => self.edit_mode_delete(document),
            Key::Enter => {
                self.invalidate_replace_restoration();
                if self.mode == Mode::Insert {
                    if let Some(edit) = document.list_enter_edit(self.cursor)? {
                        if document.format().has_structural_lists() {
                            self.cursor = continue_list_with_cursor(document, self.cursor)?;
                        } else if edit.range.is_empty() {
                            let value = RegisterValue::characterwise(&edit.replacement);
                            let payload = FormattedTextPayload::new(
                                &document.hard_line_snapshot(),
                                &edit.replacement,
                                value.hard_break_offsets().to_vec(),
                            )
                            .expect("list Enter carries one explicit semantic break");
                            document.insert_formatted_payload(self.cursor, payload)?;
                        } else {
                            document.replace(edit.range.clone(), &edit.replacement)?;
                        }
                        if !document.format().has_structural_lists() {
                            self.cursor = edit.range.start + edit.replacement.len();
                        }
                        if let Some(session) = self.insert_session.as_mut() {
                            session.record_inserted(
                                &RegisterValue::characterwise(&edit.replacement),
                                Some(EditSessionStep::ListEnter),
                            );
                        }
                        Ok(CommandOutput {
                            document_changed: true,
                            cursor_moved: true,
                            ..CommandOutput::complete()
                        })
                    } else {
                        self.insert_text(document, "\n")
                    }
                } else {
                    self.replace_text(document, "\n")
                }
            }
            Key::Tab | Key::BackTab => {
                if self.mode == Mode::Insert {
                    self.insert_text(document, "\t")
                } else {
                    self.replace_text(document, "\t")
                }
            }
            Key::Ctrl('r' | 'R') => {
                self.register_pending = true;
                Ok(CommandOutput::pending())
            }
            Key::Ctrl('w' | 'W') => self.insert_delete_motion(document, EditSessionStep::DeleteWord),
            Key::Ctrl('u' | 'U') => self.insert_delete_motion(document, EditSessionStep::DeleteToLineStart),
            Key::Ctrl('o' | 'O') => {
                document.end_edit_group();
                self.invalidate_replace_restoration();
                self.publish_last_insert_fragment();
                let placement = if self.mode == Mode::Replace {
                    InsertPlacement::Replace
                } else {
                    InsertPlacement::Before
                };
                if let Some(session) = self.insert_session.as_mut() {
                    session.placement = placement;
                    session.repeat_program = Some(EditSessionProgram::default());
                    session.last_inserted = RegisterValue::characterwise("");
                    session.preserve_normal_repeat = false;
                }
                let return_mode = self.mode;
                self.mode = Mode::Normal;
                self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                    self.cursor,
                );
                self.insert_normal_once = Some(return_mode);
                self.ctrl_o_just_started = true;
                Ok(CommandOutput {
                    mode_changed: true,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                })
            }
            Key::Char(character) => {
                if self.mode == Mode::Insert {
                    self.insert_text(document, &character.to_string())
                } else {
                    self.replace_text(document, &character.to_string())
                }
            }
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End
            | Key::DocumentStart | Key::DocumentEnd => {
                document.end_edit_group();
                self.invalidate_replace_restoration();
                self.publish_last_insert_fragment();
                let placement = if self.mode == Mode::Replace {
                    InsertPlacement::Replace
                } else {
                    InsertPlacement::Before
                };
                if let Some(session) = self.insert_session.as_mut() {
                    session.placement = placement;
                    session.repeat_program = Some(EditSessionProgram::default());
                    session.last_inserted = RegisterValue::characterwise("");
                    session.preserve_normal_repeat = false;
                }
                let mut output = if matches!(key, Key::DocumentStart | Key::DocumentEnd) {
                    self.move_to_document_edge(document, key == Key::DocumentEnd)
                } else {
                    let motion = match key {
                    Key::Left => Motion::InsertionHorizontal(-1),
                    Key::Right => Motion::InsertionHorizontal(1),
                    Key::Up => Motion::Vertical(-1),
                    Key::Down => Motion::Vertical(1),
                    Key::Home => Motion::InsertionLineStart,
                    Key::End => Motion::InsertionLineEnd,
                    _ => unreachable!(),
                    };
                    self.move_cursor(document, motion, 1)
                };
                document.begin_edit_group();
                if let Some(session) = self.insert_session.as_mut() {
                    session.unit_floor = self.cursor;
                }
                output.status = CommandStatus::Complete;
                Ok(output)
            }
            Key::PageUp | Key::PageDown => Ok(layout_required("page motion")),
            _ => Ok(CommandOutput::unsupported(format!("edit-mode key {key:?}"))),
        }
    }

    fn handle_visual_block_insert_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        if matches!(key, Key::Escape | Key::Ctrl('[')) {
            self.register_pending = false;
            return self.finish_visual_block_insert(document);
        }
        if self.register_pending {
            self.register_pending = false;
            let Key::Char(name) = key else {
                return Ok(CommandOutput::unsupported("Ctrl-R expects a register"));
            };
            if !is_valid_register(name) {
                return Ok(CommandOutput::unsupported("invalid register"));
            }
            let value = match self.require_register_value(document, name) {
                Ok(value) => value,
                Err(output) => return Ok(output),
            };
            if value.text.contains('\n') {
                return Ok(CommandOutput::unsupported(
                    "Visual Block insertion cannot collect a multiline register",
                ));
            }
            return Ok(self.collect_visual_block_payload(&value.text));
        }

        match key {
            Key::Char(character) => Ok(self.collect_visual_block_payload(&character.to_string())),
            Key::Tab => Ok(self.collect_visual_block_payload("\t")),
            Key::Backspace => {
                let session = self
                    .visual_block_insert
                    .as_mut()
                    .expect("deferred block insert checked above");
                if let Some((start, _)) = session.payload.grapheme_indices(true).last() {
                    session.payload.truncate(start);
                }
                Ok(CommandOutput::complete())
            }
            Key::Ctrl('w' | 'W') => {
                let session = self
                    .visual_block_insert
                    .as_mut()
                    .expect("deferred block insert checked above");
                match ctrl_w_delete_range(
                    &session.payload,
                    0..session.payload.len(),
                    session.payload.len(),
                ) {
                    Ok(range) => session.payload.replace_range(range, ""),
                    Err(error) => {
                        return Ok(CommandOutput {
                            status: CommandStatus::Error(format!(
                                "Visual Block Ctrl-W motion failed: {error:?}"
                            )),
                            ..CommandOutput::complete()
                        });
                    }
                }
                Ok(CommandOutput::complete())
            }
            Key::Ctrl('u' | 'U') => {
                self.visual_block_insert
                    .as_mut()
                    .expect("deferred block insert checked above")
                    .payload
                    .clear();
                Ok(CommandOutput::complete())
            }
            Key::Ctrl('r' | 'R') => {
                self.register_pending = true;
                Ok(CommandOutput::pending())
            }
            Key::Enter | Key::ShiftEnter => Ok(CommandOutput::unsupported(
                "Visual Block insertion cannot contain a hard line break",
            )),
            Key::DocumentStart | Key::DocumentEnd | Key::BackTab
            | Key::Delete
            | Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::Ctrl(_) => Ok(CommandOutput::unsupported(
                "cursor motion is unavailable while a deferred Visual Block insertion is collected",
            )),
            Key::Escape => unreachable!("handled above"),
            Key::SelectAll => self.handle_key(document, key),
        }
    }

    fn collect_visual_block_payload(&mut self, input: &str) -> CommandOutput {
        if input.contains('\n') {
            return CommandOutput::unsupported(
                "Visual Block insertion cannot contain a hard line break",
            );
        }
        self.visual_block_insert
            .as_mut()
            .expect("payload collection requires a deferred block insert")
            .payload
            .push_str(input);
        CommandOutput::complete()
    }

    fn finish_visual_block_insert(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        let session = self
            .visual_block_insert
            .take()
            .expect("deferred block insert checked by caller");
        if session.document_id != document.id() {
            self.visual_block_insert = Some(session);
            return Err(DocumentError::WrongDocument);
        }
        if session.revision != document.revision() {
            let actual = session.revision;
            self.visual_block_insert = Some(session);
            return Err(DocumentError::WrongSnapshot {
                expected: document.revision(),
                actual,
            });
        }

        let payload = match checked_text_repetition(&session.payload, session.count) {
            Ok(payload) => payload,
            Err(error) => {
                self.visual_block_insert = Some(session);
                return Ok(error.into_command_output());
            }
        };
        let edits = match session.kind {
            VisualBlockInsertKind::Insert | VisualBlockInsertKind::Append => {
                merge_same_boundary_insertions(
                    session
                        .rows
                        .iter()
                        .map(|row| {
                            TextEdit::new(
                                row.insertion_offset..row.insertion_offset,
                                payload.clone(),
                            )
                        })
                        .collect(),
                )
            }
            VisualBlockInsertKind::Change => {
                block_session_replacement_edits(&session.rows, &payload)
            }
        };
        let edits = edits.into_iter().map(|edit| {
            let value = RegisterValue::try_new(edit.replacement, RegisterKind::Characterwise, Vec::new())
                .expect("deferred block input is literal text");
            let value = self.assist_input_payload(
                document, edit.range.clone(), BoundaryAffinity::Downstream, &value,
            )?;
            Ok(TextEdit::new(edit.range, value.text))
        }).collect::<Result<Vec<_>, DocumentError>>()?;
        let inserted = edits.iter().find(|edit| !edit.replacement.is_empty())
            .map(|edit| inserted_input_unit(
                &RegisterValue::characterwise(&edit.replacement), &session.payload,
            ));
        let before = document.revision();
        // Deferred collection commits once, here, regardless of row count.
        apply_block_edits(document, edits)?;
        let changed = document.revision() != before;
        if session.kind == VisualBlockInsertKind::Change {
            if let Some(replaced) = session.replaced {
                self.delete_register(
                    session.register,
                    replaced,
                    visual_block_deletion_class(session.rows.len()),
                );
            }
        }
        self.mode = Mode::Normal;
        self.insert_session = None;
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            session.cursor_target.min(document.projection().text_tree().byte_len()),
        );
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        if !self.replaying {
            self.last_repeat = if session.kind != VisualBlockInsertKind::Change && !changed {
                // A successful empty block I/A replaces an older dot command
                // with an explicit no-op, matching ordinary empty Insert.
                Some(RepeatAction::Noop)
            } else {
                Some(RepeatAction::VisualBlock(VisualBlockRepeat {
                    shape: session.repeat_shape,
                    action: VisualBlockRepeatAction::Insert {
                        kind: session.kind,
                        payload: session.payload.clone(),
                        application_count: session.count,
                        register: session.register,
                    },
                }))
            };
        }
        if changed {
            if let Some(inserted) = inserted {
                self.registers.set_last_insert(inserted);
            }
        }
        self.clear_pending();
        Ok(CommandOutput {
            document_changed: changed,
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn insert_delete_motion_range(
        &self,
        document: &Document,
        step: &EditSessionStep,
    ) -> Result<Range<usize>, CommandOutput> {
        let line = document
            .hard_line_snapshot()
            .line_at_offset(self.cursor)
            .expect("an edit caret resolves to one hard line")
            .content_range();
        let (name, range) = match step {
            EditSessionStep::DeleteWord => (
                "Ctrl-W",
                ctrl_w_delete_range(document.text(), line, self.cursor),
            ),
            EditSessionStep::DeleteToLineStart => {
                let floor = self.insert_session.as_ref()
                    .map_or(line.start, |session| session.unit_floor);
                ("Ctrl-U", ctrl_u_delete_range_since(document.text(), line, self.cursor, floor))
            }
            _ => unreachable!("only Insert-mode delete motions use this resolver"),
        };
        range.map_err(|error| CommandOutput {
            status: CommandStatus::Error(format!("Insert {name} motion failed: {error:?}")),
            ..CommandOutput::complete()
        })
    }

    fn insert_delete_motion(
        &mut self,
        document: &mut Document,
        step: EditSessionStep,
    ) -> Result<CommandOutput, DocumentError> {
        self.invalidate_replace_restoration();
        match self.insert_delete_motion_range(document, &step) {
            Ok(range) => self.delete_insert_mode_range(document, range, step),
            Err(output) => Ok(output),
        }
    }

    fn delete_insert_mode_range(
        &mut self,
        document: &mut Document,
        range: Range<usize>,
        step: EditSessionStep,
    ) -> Result<CommandOutput, DocumentError> {
        self.invalidate_replace_restoration();
        if range.is_empty() {
            return Ok(CommandOutput::complete());
        }
        let removed = document.text()[range.clone()].to_owned();
        self.cursor = delete_with_cursor(document, range)?;
        if let Some(session) = self.insert_session.as_mut() {
            session.unit_floor = session.unit_floor.min(self.cursor);
            session.record_deleted(document.format(), &removed, step);
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn insert_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(output) = self.try_insert_html_assistance(document, input)? {
            return Ok(output);
        }
        let value = RegisterValue::characterwise(input);
        self.insert_register_payload(document, &value)
    }

    fn insert_literal_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(output) = self.try_insert_html_assistance(document, input)? {
            return Ok(output);
        }
        let value = external_text_register_value(document, input);
        self.insert_register_payload(document, &value)
    }

    fn insert_register_payload(
        &mut self,
        document: &mut Document,
        value: &RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        let intent = value;
        let value = self.assist_typing_input_payload(
            document, self.cursor..self.cursor, self.insertion_boundary_affinity(), intent,
        )?;
        let input = value.text.as_str();
        if input.is_empty() {
            return Ok(CommandOutput::complete());
        }
        let lines = document.hard_line_snapshot();
        let payload = FormattedTextPayload::new(&lines, input, value.hard_break_offsets().to_vec())
            .expect("insert register payload has validated semantic breaks");
        let edit = document.normalize_typing_payload(
            FormattedPayloadEdit::new(self.cursor..self.cursor, payload)
                .with_boundary_affinity(self.insertion_boundary_affinity()),
        )?;
        if let Some(prepared) = value.clipboard_fragment().map(|fragment|
            document.prepare_clipboard_fragment(self.cursor..self.cursor, fragment, input)
        ).transpose().map_err(command_document_error)?.flatten() {
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            self.cursor += input.len();
        } else if !self.typing_style.is_empty() {
            self.cursor = document
                .insert_with_typing_style(
                    edit,
                    self.typing_style.named.as_ref(),
                    &self.typing_style.values,
                )
                .map_err(command_document_error)?;
        } else {
            self.cursor = commit_typing_payload(document, edit)?;
        }
        self.finish_typing_caret(document)?;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_inserted_intent(&value, intent, None);
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn finish_typing_caret(&mut self, document: &Document) -> Result<(), DocumentError> {
        let lines = document.hard_line_snapshot();
        if !lines.is_grapheme_boundary(self.cursor) {
            // The inserted suffix may join unchanged following combining
            // marks or regional indicators. The typing caret associates after
            // the resulting grapheme, not an interior UTF-8 insertion end.
            self.cursor = lines
                .next_grapheme_boundary(self.cursor)
                .ok_or(DocumentError::NotGraphemeBoundary(self.cursor))?;
        }
        Ok(())
    }

    pub(crate) fn insertion_boundary_affinity(&self) -> BoundaryAffinity {
        if self
            .insert_session
            .as_ref()
            .is_some_and(|session| session.last_inserted.text.ends_with('\n'))
        {
            // Enter creates a new paragraph context. Its first character
            // belongs to that empty paragraph; subsequent typing associates
            // with the newly inserted content on the upstream side.
            return BoundaryAffinity::Downstream;
        }
        let continuing = self.insert_session.as_ref().is_some_and(|session| {
            !session.last_inserted.text.is_empty()
                || matches!(
                    session.placement,
                    InsertPlacement::After | InsertPlacement::LineEnd
                )
        });
        if continuing {
            BoundaryAffinity::Upstream
        } else {
            self.boundary_affinity
        }
    }

    fn invalidate_replace_restoration(&mut self) {
        if let Some(session) = self.insert_session.as_mut() {
            session.replace_journal.clear();
        }
    }

    fn replace_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let value = RegisterValue::characterwise(input);
        self.replace_register_payload(document, &value)
    }

    fn replace_literal_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let value = external_text_register_value(document, input);
        self.replace_register_payload(document, &value)
    }

    fn replace_register_payload(
        &mut self,
        document: &mut Document,
        value: &RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        let intent = value;
        let previous = self.insert_session.as_ref()
            .and_then(|session| session.replace_journal.last());
        let target = previous.filter(|entry| entry.frontier() == Some(self.cursor))
            .and_then(|entry| entry.source_record.as_ref())
            .map_or(self.cursor, |record| record.next_target);
        let lines = document.hard_line_snapshot();
        let end = replacement_payload_end(&lines, target, intent);
        let value = self.assist_typing_input_payload(
            document, target..end, self.insertion_boundary_affinity(), intent,
        )?;
        let input = value.text.as_str();
        if input.is_empty() {
            return Ok(CommandOutput::complete());
        }
        let journalable = self.replace_payload_is_journalable(input);
        if journalable
            && (document.format() == crate::document::Format::Html
                || !self.typing_style.is_empty()
                || self
                    .insert_session
                    .as_ref()
                    .and_then(|session| session.replace_journal.last())
                    .is_some_and(|entry| entry.source_record.is_some()))
        {
            let previous = self
                .insert_session
                .as_ref()
                .and_then(|session| session.replace_journal.last());
            let continuing = previous.is_some_and(|entry| entry.frontier() == Some(self.cursor));
            let target = if continuing {
                previous
                    .and_then(|entry| entry.source_record.as_ref())
                    .map_or(self.cursor, |record| record.next_target)
            } else {
                self.cursor
            };
            let (prepared, records) = document
                .prepare_recorded_replacement_with_typing_style(
                    self.cursor,
                    target,
                    input,
                    self.typing_style.named.as_ref(),
                    &self.typing_style.values,
                    self.insertion_boundary_affinity(),
                )
                .map_err(command_document_error)?;
            let changed = !prepared.is_no_op();
            document
                .commit_model_transaction(prepared)
                .map_err(command_document_error)?;
            if let Some(last) = records.last() {
                self.cursor = last.after_cursor;
            }
            if let Some(session) = self.insert_session.as_mut() {
                if !continuing {
                    session.replace_journal.clear();
                }
                session
                    .replace_journal
                    .extend(records.into_iter().map(|record| ReplaceJournalEntry {
                        start: record.before_cursor,
                        inserted: record.inserted.clone(),
                        original: record.original.clone(),
                        source_record: Some(record),
                    }));
                session.record_inserted_intent(&value, intent, None);
            }
            return Ok(CommandOutput {
                document_changed: changed,
                cursor_moved: true,
                ..CommandOutput::complete()
            });
        }
        if !journalable {
            // Newlines and externally established Replace sessions need
            // richer row-aware bookkeeping. Do not let a stale one-line
            // frontier restore unrelated content after such an operation.
            self.invalidate_replace_restoration();
        }
        // Resolve the entire text event in the pre-edit snapshot, then publish
        // it as one document transaction. This prevents an unrepresentable
        // later grapheme from committing an earlier prefix of the same IME or
        // paste event.
        let start = self.cursor;
        let lines = document.hard_line_snapshot();
        let (end, journal_entries) = replacement_payload_targets(document, &lines, start, &value, journalable);
        let before = document.revision();
        let payload = FormattedTextPayload::new(&lines, input, value.hard_break_offsets().to_vec())
            .expect("replacement register payload has validated semantic breaks");
        let edit = document.normalize_typing_payload(
            FormattedPayloadEdit::new(start..end, payload)
                .with_boundary_affinity(self.insertion_boundary_affinity()),
        )?;
        if !self.typing_style.is_empty() {
            self.cursor = document
                .insert_with_typing_style(
                    edit,
                    self.typing_style.named.as_ref(),
                    &self.typing_style.values,
                )
                .map_err(command_document_error)?;
        } else {
            self.cursor = commit_typing_payload(document, edit)?;
        }
        self.finish_typing_caret(document)?;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_inserted_intent(&value, intent, None);
            if journalable {
                let continues_frontier = session.replace_journal.last().map_or(true, |entry| {
                    entry.start.checked_add(entry.inserted.len()) == Some(start)
                });
                let has_legal_boundaries = journal_entries.iter().all(|entry| {
                    is_grapheme_boundary(document.text(), entry.start)
                        && entry
                            .start
                            .checked_add(entry.inserted.len())
                            .is_some_and(|end| is_grapheme_boundary(document.text(), end))
                });
                if continues_frontier && has_legal_boundaries {
                    session.replace_journal.extend(journal_entries);
                } else {
                    session.replace_journal.clear();
                }
            }
        }
        let changed = document.revision() != before;
        Ok(CommandOutput {
            document_changed: changed,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn edit_mode_backspace(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        if self.mode == Mode::Replace {
            let journal_entry = self
                .insert_session
                .as_ref()
                .and_then(|session| session.replace_journal.last())
                .filter(|entry| entry.frontier() == Some(self.cursor))
                .cloned();
            if let Some(entry) = journal_entry {
                let before = document.revision();
                if let Some(record) = &entry.source_record {
                    document
                        .restore_recorded_replacement(&record.restoration)
                        .map_err(command_document_error)?;
                } else {
                    document.replace(
                        entry.start..self.cursor,
                        entry.original.as_deref().unwrap_or(""),
                    )?;
                }
                self.cursor = entry.start;
                if let Some(session) = self.insert_session.as_mut() {
                    session.replace_journal.pop();
                    if let Some(previous) = session
                        .replace_journal
                        .last_mut()
                        .and_then(|entry| entry.source_record.as_mut())
                    {
                        previous
                            .restoration
                            .after_newer_frontier_restored(document.revision());
                    }
                    session.record_deleted(document.format(), &entry.inserted, EditSessionStep::Backspace);
                }
                return Ok(CommandOutput {
                    document_changed: document.revision() != before,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                });
            }
            self.invalidate_replace_restoration();
        }
        if let Some(request) = self.paragraph_key_request(document, Key::Backspace)? {
            return self.apply_paragraph_key(document, Key::Backspace, request);
        }
        let Some(start) = document
            .hard_line_snapshot()
            .previous_grapheme_boundary(self.cursor)
        else {
            return Ok(CommandOutput::complete());
        };
        let removed = document.hard_line_snapshot().slice_utf8(start..self.cursor).expect("validated backspace range");
        self.cursor = delete_with_cursor(document, start..self.cursor)?;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_deleted(document.format(), &removed, EditSessionStep::Backspace);
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn edit_mode_delete(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        self.invalidate_replace_restoration();
        let Some(end) = document
            .hard_line_snapshot()
            .next_grapheme_boundary(self.cursor)
        else {
            if let Some(session) = self.insert_session.as_mut() {
                session.record_step(EditSessionStep::Delete);
            }
            return Ok(CommandOutput::complete());
        };
        self.cursor = delete_with_cursor(document, self.cursor..end)?;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_step(EditSessionStep::Delete);
        }
        Ok(CommandOutput {
            document_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn enter_insert(
        &mut self,
        document: &mut Document,
        placement: InsertPlacement,
        entry_count: usize,
    ) -> CommandOutput {
        if entry_count > isize::MAX as usize {
            return repetition_too_large(entry_count);
        }
        // A named menu choice in Normal mode targets the next typing session.
        // Keep its sparse direct overrides separate from the named identity.
        if self.typing_style.named.is_none() {
            self.typing_style = Default::default();
        }
        self.input_assistance.clear_tag();
        let lines = document.hard_line_snapshot();
        self.cursor = match placement {
            InsertPlacement::Before => self.cursor,
            InsertPlacement::After => lines
                .grapheme_range_at(self.cursor)
                .filter(|range| !is_hard_line_separator(&lines, range))
                .map_or(self.cursor, |range| range.end),
            InsertPlacement::LineStart => self
                .mode_line_insertion(document, false, true)
                .unwrap_or_else(|| first_nonblank_document(document, &lines, self.cursor)),
            InsertPlacement::LineEnd => self
                .mode_line_insertion(document, true, false)
                .unwrap_or_else(|| line_end(&lines, self.cursor)),
            InsertPlacement::Replace => self.cursor,
            InsertPlacement::OpenBelow | InsertPlacement::OpenAbove => self.cursor,
        };
        self.mode = if placement == InsertPlacement::Replace {
            Mode::Replace
        } else {
            Mode::Insert
        };
        let mut program = EditSessionProgram::default();
        if !self.typing_style.is_empty() {
            program.push(EditSessionStep::TypingStyle(self.typing_style.clone()));
        }
        self.insert_session = Some(InsertSession {
            placement,
            repeat_program: Some(program),
            last_inserted: RegisterValue::characterwise(""),
            entry_count,
            replaying_program: false,
            preserve_normal_repeat: false,
            unit_floor: self.cursor,
            repeat_operator: None,
            repeat_visual_operator: None,
            replace_journal: Vec::new(),
        });
        document.begin_edit_group();
        CommandOutput {
            mode_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        }
    }

    fn validate_edit_session_encoding(
        document: &Document,
        program: &EditSessionProgram,
    ) -> Result<(), DocumentError> {
        if document.format().is_rich_text() {
            // Rich adapters serialize otherwise unrepresentable scalars as
            // exact character references or Unicode controls. The source
            // transaction validates that escaped representation atomically.
            return Ok(());
        }
        for step in &program.steps {
            if let EditSessionStep::AssistedText(value) = step {
                document.encoding().encode_fragment(value)?;
            }
            if let EditSessionStep::Text(value) = step {
                // Encoding failure is deterministic and must be discovered
                // before an earlier semantic step can mutate the document.
                document.encoding().encode_fragment(&value.text)?;
            }
        }
        Ok(())
    }

    fn validate_edit_session_repetition(
        program: &EditSessionProgram,
        iterations: usize,
        opens_line_per_iteration: bool,
    ) -> Result<(), TextRepetitionError> {
        let steps_per_iteration = program
            .steps
            .len()
            .checked_add(usize::from(opens_line_per_iteration))
            .ok_or_else(|| TextRepetitionError::new(iterations))?;
        let total_steps = steps_per_iteration
            .checked_mul(iterations)
            .ok_or_else(|| TextRepetitionError::new(iterations))?;
        if total_steps > MACRO_REPLAY_EVENT_LIMIT {
            return Err(TextRepetitionError::new(iterations));
        }
        let text_bytes = program.steps.iter().try_fold(0usize, |total, step| {
            let length = match step {
                EditSessionStep::Text(value) => value.text.len(),
                EditSessionStep::AssistedText(value) => value.len(),
                _ => 0,
            };
            total.checked_add(length)
        });
        let bytes_per_iteration = text_bytes
            .and_then(|bytes| bytes.checked_add(usize::from(opens_line_per_iteration)))
            .ok_or_else(|| TextRepetitionError::new(iterations))?;
        let total_bytes = bytes_per_iteration
            .checked_mul(iterations)
            .ok_or_else(|| TextRepetitionError::new(iterations))?;
        if total_bytes > isize::MAX as usize {
            return Err(TextRepetitionError::new(iterations));
        }
        Ok(())
    }

    fn replay_edit_session_program(
        &mut self,
        document: &mut Document,
        program: &EditSessionProgram,
        iterations: usize,
        open_line_before_first: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let opens_lines = self.insert_session.as_ref().is_some_and(|session| {
            matches!(
                session.placement,
                InsertPlacement::OpenBelow | InsertPlacement::OpenAbove
            )
        });
        Self::validate_edit_session_encoding(document, program)?;
        if let Err(error) = Self::validate_edit_session_repetition(program, iterations, opens_lines)
        {
            return Ok(error.into_command_output());
        }
        if program.steps.is_empty() && !opens_lines {
            return Ok(CommandOutput::complete());
        }
        if let Some(session) = self.insert_session.as_mut() {
            session.replaying_program = true;
        }
        let result = (|| {
            let mut output = CommandOutput::complete();
            for iteration in 0..iterations {
                if opens_lines && (open_line_before_first || iteration > 0) {
                    if self.line_mode == LineMode::PhysicalSource {
                        let lines = document.hard_line_snapshot();
                        let payload = FormattedTextPayload::new(&lines, "\n", vec![0])
                            .expect("a repeated open-line separator is a semantic hard break");
                        document.insert_formatted_payload(self.cursor, payload)?;
                        self.cursor += 1;
                    } else {
                        let (prepared, cursor) =
                            prepare_open_line_with_cursor(document, self.cursor, self.cursor, true)?;
                        document
                            .commit_model_transaction(prepared)
                            .map_err(command_document_error)?;
                        self.cursor = cursor;
                    }
                    output.merge(CommandOutput {
                        document_changed: true,
                        cursor_moved: true,
                        ..CommandOutput::complete()
                    });
                }
                for step in &program.steps {
                    let next = match step {
                        EditSessionStep::TypingStyle(value) => {
                            self.typing_style = Default::default();
                            if let Some(named) = &value.named {
                                self.set_typing_named_style(document, named.clone())?;
                            }
                            self.set_typing_properties(document, value.values.clone())?;
                            CommandOutput::complete()
                        }
                        EditSessionStep::AssistedText(value) => {
                            self.insert_text(document, value)?
                        }
                        EditSessionStep::Text(value) if self.mode == Mode::Insert => {
                            self.insert_register_payload(document, value)?
                        }
                        EditSessionStep::Text(value) => {
                            self.replace_register_payload(document, value)?
                        }
                        EditSessionStep::Backspace => {
                            if let Some(output) =
                                self.try_html_assistance_key(document, Key::Backspace)?
                            {
                                output
                            } else {
                                self.edit_mode_backspace(document)?
                            }
                        }
                        EditSessionStep::Delete => self.edit_mode_delete(document)?,
                        EditSessionStep::DeleteWord => self.insert_delete_motion(document, EditSessionStep::DeleteWord)?,
                        EditSessionStep::DeleteToLineStart => self.insert_delete_motion(document, EditSessionStep::DeleteToLineStart)?,
                        EditSessionStep::ListEnter => {
                            self.handle_edit_mode_key(document, Key::Enter)?
                        }
                        EditSessionStep::HardBreak => {
                            self.handle_edit_mode_key(document, Key::ShiftEnter)?
                        }
                        EditSessionStep::ListIndent { unindent } => {
                            self.handle_edit_mode_key(document, if *unindent { Key::BackTab } else { Key::Tab })?
                        }
                    };
                    output.merge(next);
                }
            }
            Ok(output)
        })();
        if let Some(session) = self.insert_session.as_mut() {
            session.replaying_program = false;
        }
        result
    }

    fn finish_insert(&mut self, document: &mut Document) -> Result<CommandOutput, DocumentError> {
        self.insert_normal_once = None;
        self.ctrl_o_just_started = false;
        let mut expansion_changed = false;
        let expansion = self.insert_session.as_ref().and_then(|session| {
            session.repeat_program.as_ref().and_then(|program| {
                (session.entry_count > 1).then(|| (program.clone(), session.entry_count - 1))
            })
        });
        if let Some((program, iterations)) = expansion {
            let output = self.replay_edit_session_program(document, &program, iterations, true)?;
            if output.status != CommandStatus::Complete {
                return Ok(output);
            }
            expansion_changed = output.document_changed;
        }
        self.typing_style = Default::default();
        self.input_assistance.clear_tag();
        if let Some(session) = self.insert_session.take() {
            let last_inserted = session.last_inserted.clone();
            document.end_edit_group();
            if !self.replaying && !session.preserve_normal_repeat {
                self.last_repeat = Some(match session.repeat_program {
                    Some(program) => {
                        if let Some(command) = session.repeat_visual_operator {
                            RepeatAction::VisualOperator {
                                command,
                                edits: Some(program),
                            }
                        } else if let Some(command) = session.repeat_operator {
                            RepeatAction::Operator {
                                command,
                                edits: Some(program),
                            }
                        } else {
                            RepeatAction::Insert {
                                placement: session.placement,
                                program,
                                count: session.entry_count,
                            }
                        }
                    }
                    None => RepeatAction::Noop,
                });
            }
            if !last_inserted.text.is_empty() {
                self.registers.set_last_insert(last_inserted);
            }
        } else {
            document.end_edit_group();
        }
        self.mode = Mode::Normal;
        let lines = document.hard_line_snapshot();
        let current_line_start = line_start(&lines, self.cursor);
        if self.cursor > current_line_start {
            self.cursor = lines.previous_grapheme_boundary(self.cursor).unwrap_or(current_line_start);
        }
        self.cursor = normalize_normal_cursor_document(document, &lines, self.cursor);
        self.clear_pending();
        Ok(CommandOutput {
            mode_changed: true,
            cursor_moved: true,
            document_changed: expansion_changed,
            ..CommandOutput::complete()
        })
    }

    fn open_line(
        &mut self,
        document: &mut Document,
        above: bool,
        entry_count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        if entry_count > isize::MAX as usize {
            return Ok(repetition_too_large(entry_count));
        }
        let position = self
            .mode_line_insertion(document, !above, false)
            .unwrap_or_else(|| {
                if above {
                    line_start(&document.hard_line_snapshot(), self.cursor)
                } else {
                    line_end(&document.hard_line_snapshot(), self.cursor)
                }
            });
        document.begin_edit_group();
        if self.line_mode == LineMode::PhysicalSource {
            self.open_physical_line(document, above)?;
        } else {
            let (prepared, cursor) = prepare_open_line_with_cursor(document, position, self.cursor, !above)?;
            document
                .commit_model_transaction(prepared)
                .map_err(command_document_error)?;
            self.cursor = cursor;
        }
        // The newly opened line supplies the first insertion's context. An
        // upstream affinity retained from the old cursor (for example `$`)
        // can otherwise map a leading empty HTML line outside its paragraph.
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.mode = Mode::Insert;
        self.insert_session = Some(InsertSession {
            placement: if above {
                InsertPlacement::OpenAbove
            } else {
                InsertPlacement::OpenBelow
            },
            repeat_program: Some(EditSessionProgram::default()),
            last_inserted: RegisterValue::characterwise(""),
            entry_count,
            replaying_program: false,
            preserve_normal_repeat: false,
            unit_floor: self.cursor,
            repeat_operator: None,
            repeat_visual_operator: None,
            replace_journal: Vec::new(),
        });
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        })
    }

    fn delete_characters(
        &mut self,
        document: &mut Document,
        count: usize,
        register: Option<char>,
        backward: bool,
    ) -> Result<CommandOutput, DocumentError> {
        if let Err(output) = self.require_register_write(register) {
            return Ok(output);
        }
        let lines = document.hard_line_snapshot();
        let range = normal_delete_range(&lines, self.cursor, count, backward);
        if range.is_empty() {
            return Ok(CommandOutput::complete());
        }
        let value = register_value(
            document,
            &lines,
            &MotionExtent {
                range: range.clone(),
                kind: MotionKind::Characterwise,
            },
            register,
        );
        let cursor = delete_with_cursor(document, range)?;
        self.delete_register(register, value, DeletionClass::Small);
        self.cursor =
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), cursor);
        if !self.replaying {
            self.last_repeat = Some(if backward {
                RepeatAction::DeleteBackward { count }
            } else {
                RepeatAction::DeleteForward { count }
            });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn replace_characters(
        &mut self,
        document: &mut Document,
        count: usize,
        replacement: &RegisterValue,
        _register: Option<char>,
    ) -> Result<CommandOutput, DocumentError> {
        let start = self.cursor;
        let lines = document.hard_line_snapshot();
        let content_end = line_end(&lines, start);
        let mut end = start;
        for _ in 0..count {
            let Some(next) = lines.next_grapheme_boundary(end) else {
                return Ok(CommandOutput {
                    status: CommandStatus::Error("not enough characters on line".into()),
                    ..CommandOutput::complete()
                });
            };
            if end >= content_end || next > content_end {
                return Ok(CommandOutput {
                    status: CommandStatus::Error("not enough characters on line".into()),
                    ..CommandOutput::complete()
                });
            }
            end = next;
        }
        // Normal [count]r<Enter> deletes `count` graphemes but inserts exactly
        // one hard-line item. Other replacement operands are repeated once per
        // replaced grapheme.
        let replacement_count = if is_single_semantic_hard_break(replacement) {
            1
        } else {
            count
        };
        let repeated = match checked_register_repetition(replacement, replacement_count) {
            Ok(repeated) => repeated,
            Err(error) => return Ok(error.into_command_output()),
        };
        let repeated = self.assist_input_payload(
            document, start..end, BoundaryAffinity::Downstream, &repeated,
        )?;
        let text = repeated.text.clone();
        let lines = document.hard_line_snapshot();
        let payload =
            FormattedTextPayload::new(&lines, text.clone(), repeated.hard_break_offsets().to_vec())
                .expect("repeated replacement carries valid semantic-break offsets");
        let edit = document.normalize_typing_payload(FormattedPayloadEdit::new(start..end, payload))?;
        let insertion_end = if is_single_semantic_hard_break(replacement) {
            let inserted_bytes = edit.payload().text().len();
            let prepared = document
                .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                    document.id(), document.revision(), vec![edit],
                ))
                .map_err(command_document_error)?;
            // HTML may protect an adjacent ordinary space in the same
            // transaction. Map the replacement start, then cross only the
            // inserted break; mapping the old end would cross a protected
            // following space as well.
            let cursor = prepared_cursor(document, &prepared, start, Association::BeforeInsertion)?
                .checked_add(inserted_bytes)
                .ok_or(DocumentError::AmbiguousProjection)?;
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            cursor
        } else {
            commit_typing_payload(document, edit)?
        };
        let new_lines = document.hard_line_snapshot();
        self.cursor = if is_single_semantic_hard_break(replacement) {
            // This may be EOF when the replacement created a trailing empty
            // hard line. EOF is that line's sole legal Normal-mode cursor.
            insertion_end
        } else {
            new_lines
                .previous_grapheme_boundary(insertion_end)
                .unwrap_or(start)
        };
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::ReplaceCharacter {
                count,
                value: replacement.clone(),
            });
            self.registers.set_last_insert(inserted_input_unit(&repeated, &replacement.text));
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn paste(
        &mut self,
        document: &mut Document,
        before: bool,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        self.paste_impl(document, before, count, false)
    }

    fn paste_impl(
        &mut self,
        document: &mut Document,
        before: bool,
        count: usize,
        follow: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let name = self.requested_register.take().unwrap_or('"');
        let value = match self.require_register_value(document, name) {
            Ok(value) => value,
            Err(output) => return Ok(output),
        };
        if value.text.is_empty() {
            return Ok(CommandOutput::complete());
        }
        if self.line_mode == LineMode::PhysicalSource && value.kind == RegisterKind::Linewise {
            return self.paste_physical_lines(document, before, count, follow, name, value);
        }
        let lines = document.hard_line_snapshot();
        let mut repeated = match checked_register_repetition(&value, count) {
            Ok(repeated) => repeated,
            Err(error) => return Ok(error.into_command_output()),
        };
        let mut content_start_offset = 0;
        let position = match value.kind {
            RegisterKind::Characterwise if before => self.cursor,
            RegisterKind::Characterwise => grapheme_range_at(document.text(), self.cursor)
                .map_or(self.cursor, |range| range.end),
            RegisterKind::Linewise if before => line_start(&lines, self.cursor),
            RegisterKind::Linewise => {
                let at = line_range(&lines, self.cursor).end;
                if at == document.projection().text_tree().byte_len() {
                    repeated = match linewise_register_at_eof(repeated, count) {
                        Ok(repeated) => repeated,
                        Err(error) => return Ok(error.into_command_output()),
                    };
                    content_start_offset = 1;
                }
                at
            }
            RegisterKind::Blockwise => {
                return Ok(CommandOutput::unsupported(
                    "Normal-mode blockwise paste requires display-space placement",
                ));
            }
        };
        repeated = self.assist_input_payload(
            document, position..position, BoundaryAffinity::Downstream, &repeated,
        )?;
        let payload = FormattedTextPayload::new(
            &lines,
            repeated.text.clone(),
            repeated.hard_break_offsets().to_vec(),
        )
        .expect("register values carry validated semantic-break offsets");
        let edit = document.normalize_typing_payload(FormattedPayloadEdit::new(
            position..position,
            payload,
        ))?;
        let insertion_end = if let Some(prepared) = repeated.clipboard_fragment().map(|fragment|
            document.prepare_clipboard_fragment(position..position, fragment, &repeated.text)
        ).transpose().map_err(command_document_error)?.flatten() {
            let cursor = prepared_payload_caret(document, &prepared, &edit)?;
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            cursor
        } else {
            commit_typing_payload(document, edit)?
        };
        let target = if follow {
            insertion_end
        } else {
            match value.kind {
                RegisterKind::Characterwise => {
                    document.hard_line_snapshot().previous_grapheme_boundary(insertion_end).unwrap_or(position)
                }
                RegisterKind::Linewise => first_nonblank_document(document, &document.hard_line_snapshot(),
                    position + content_start_offset,
                ),
                RegisterKind::Blockwise => unreachable!("blockwise paste returned above"),
            }
        };
        self.cursor =
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), target);
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::Paste {
                before,
                follow,
                count,
                register: name,
            });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn operator_to_line_end(
        &mut self,
        document: &mut Document,
        operator: Operator,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let extent = self
            .resolve_operator_motion(document, OperatorMotion::LineEnd, count)
            .unwrap();
        let register = self.requested_register.take();
        self.apply_operator_with_repeat(
            document,
            operator,
            extent,
            register,
            RepeatTarget::Motion(OperatorMotion::LineEnd),
            count,
        )
    }

    fn change_current_lines(
        &mut self,
        document: &mut Document,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let extent = MotionExtent {
            range: linewise_range(&document.hard_line_snapshot(), self.cursor, count),
            kind: MotionKind::Linewise,
        };
        let register = self.requested_register.take();
        self.apply_operator_with_repeat(
            document,
            Operator::Change,
            extent,
            register,
            RepeatTarget::Lines,
            count,
        )
    }

    fn yank_current_lines(
        &mut self,
        document: &mut Document,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let extent = MotionExtent {
            range: linewise_range(&document.hard_line_snapshot(), self.cursor, count),
            kind: MotionKind::Linewise,
        };
        let register = self.requested_register.take();
        self.apply_operator(
            document,
            Operator::Yank,
            extent,
            register,
            DeletionClass::Large,
        )
    }

    fn toggle_at_cursor(
        &mut self,
        document: &mut Document,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let lines = document.hard_line_snapshot();
        let end = advance_graphemes(document.text(), self.cursor, count)
            .min(line_end(&lines, self.cursor));
        if self.cursor == end {
            return Ok(CommandOutput::complete());
        }
        let start = self.cursor;
        let replacement = change_case(&document.text()[start..end], Operator::ToggleCase);
        let changed = replacement != document.text()[start..end];
        if changed {
            document.replace(start..end, &replacement)?;
            if !self.replaying {
                self.last_repeat = Some(RepeatAction::Operator {
                    command: OperatorRepeat {
                        operator: Operator::ToggleCase,
                        target: RepeatTarget::Characters,
                        count,
                        register: None,
                    },
                    edits: None,
                });
            }
        }
        let replacement_end = start + replacement.len();
        let lines = document.hard_line_snapshot();
        self.cursor = if replacement_end < line_end(&lines, start) {
            replacement_end
        } else {
            last_grapheme_on_line(document.text(), &lines, start)
        };
        Ok(CommandOutput {
            document_changed: changed,
            cursor_moved: self.cursor != start,
            ..CommandOutput::complete()
        })
    }

    fn join_lines(
        &mut self,
        document: &mut Document,
        count: usize,
        insert_space: bool,
    ) -> Result<CommandOutput, DocumentError> {
        if self.line_mode == LineMode::PhysicalSource {
            return self.join_physical_lines(document, count, insert_space);
        }
        let requested_count = count;
        let count = match self.mode_join_count(document, count)? {
            Ok(count) => count,
            Err(error) => return Ok(layout_error(error)),
        };
        if count < 2 {
            return Ok(CommandOutput::complete());
        }
        self.join_hard_lines(document, count, requested_count, insert_space)
    }

    fn join_hard_lines(
        &mut self,
        document: &mut Document,
        count: usize,
        requested_count: usize,
        insert_space: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let lines = document.hard_line_snapshot();
        let current = lines
            .line_at_offset(self.cursor)
            .expect("a command cursor resolves to one hard line");
        let joins = count
            .max(2)
            .saturating_sub(1)
            .min(lines.line_count().saturating_sub(current.index() + 1));
        let start = current.content_range().start;
        let mut joined_len = current.content_range().len();
        let mut joined_last = (joined_len>0).then(||document_char_before(document,current.content_range().end)).flatten();
        let mut last_join_offset = joined_len;
        let mut edits = Vec::with_capacity(joins);
        let mut actual = 0;
        for index in current.index() + 1..current.index() + 1 + joins {
            let Some(next_line) = lines.line(index) else {
                break;
            };
            let separator = lines
                .line(index - 1)
                .and_then(|line| line.separator_range())
                .expect("adjacent hard lines have one projected separator");
            let next_range = next_line.content_range();
            last_join_offset = joined_len;
            if insert_space {
                // Vim removes line indentation, not arbitrary Unicode white
                // space. In particular, NBSP remains document content.
                let trimmed_start = document_prefix_end(document,next_range.clone(),|c|matches!(c,' '| '\t'));
                let trimmed_len=next_range.end-trimmed_start;
                let insert_separator = joined_len>0
                    && !joined_last.is_some_and(|character| matches!(character, ' ' | '\t'))
                    && trimmed_len>0
                    && document_char_at(document,trimmed_start)!=Some(')');
                edits.push(TextEdit::new(
                    separator.start..trimmed_start,
                    if insert_separator { " " } else { "" },
                ));
                if insert_separator {
                    joined_len+=1;joined_last=Some(' ');
                }
                joined_len+=trimmed_len;
                if trimmed_len>0{joined_last=document_char_before(document,next_range.end);}
            } else {
                edits.push(TextEdit::new(separator, ""));
                joined_len+=next_range.len();
                if !next_range.is_empty(){joined_last=document_char_before(document,next_range.end);}
            }
            actual += 1;
        }
        if actual == 0 {
            return Ok(CommandOutput::complete());
        }
        document.validate_hard_line_snapshot(&lines)?;
        document.apply_edits(edits)?;
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            start + last_join_offset,
        );
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::Join {
                count: requested_count,
                insert_space,
            });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    fn repeat_last_change(
        &mut self,
        document: &mut Document,
        override_count: Option<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(action) = self.last_repeat.clone() else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no previous change".into()),
                ..CommandOutput::complete()
            });
        };
        self.replaying = true;
        document.begin_edit_group();
        let result = self.execute_repeat_action(document, action, override_count);
        document.end_edit_group();
        self.replaying = false;
        result
    }

    fn execute_repeat_action(
        &mut self,
        document: &mut Document,
        action: RepeatAction,
        override_count: Option<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        match action {
            RepeatAction::Noop => Ok(CommandOutput::complete()),
            RepeatAction::DeleteForward { count } => {
                self.delete_characters(document, choose_repeat_count(override_count, count), None, false)
            }
            RepeatAction::DeleteBackward { count } => {
                self.delete_characters(document, choose_repeat_count(override_count, count), None, true)
            }
            RepeatAction::ReplaceCharacter { count, value } => self.replace_characters(
                document,
                choose_repeat_count(override_count, count),
                &value,
                None,
            ),
            RepeatAction::Insert {
                placement,
                program,
                count,
            } => {
                let repetitions = choose_repeat_count(override_count, count);
                Self::validate_edit_session_encoding(document, &program)?;
                if let Err(error) = Self::validate_edit_session_repetition(
                    &program,
                    repetitions,
                    matches!(
                        placement,
                        InsertPlacement::OpenBelow | InsertPlacement::OpenAbove
                    ),
                ) {
                    return Ok(error.into_command_output());
                }
                let mut output = match placement {
                    InsertPlacement::OpenBelow => self.open_line(document, false, 1)?,
                    InsertPlacement::OpenAbove => self.open_line(document, true, 1)?,
                    _ => self.enter_insert(document, placement, 1),
                };
                output.merge(self.replay_edit_session_program(
                    document,
                    &program,
                    repetitions,
                    false,
                )?);
                output.merge(self.finish_insert(document)?);
                Ok(output)
            }
            RepeatAction::Join {
                count,
                insert_space,
            } => self.join_lines(
                document,
                choose_repeat_count(override_count, count),
                insert_space,
            ),
            RepeatAction::Operator { command, edits } => {
                self.repeat_operator(document, command, edits, override_count)
            }
            RepeatAction::VisualOperator { command, edits } => {
                // Vim deliberately ignores a count supplied to dot when the
                // recorded change originated from a Visual selection.
                self.repeat_visual_operator(document, command, edits)
            }
            RepeatAction::VisualReplace { shape, value } => {
                let extent = self.resolve_visual_repeat_extent(document, shape);
                self.replace_visual_extent(document, extent, &value)
                    .map(|(output, _)| output)
            }
            RepeatAction::VisualJoin {
                shape,
                insert_space,
            } => {
                let extent = self.resolve_visual_repeat_extent(document, shape);
                let count =
                    hard_line_count_for_range(&document.hard_line_snapshot(), &extent.range);
                self.cursor = extent.range.start;
                self.join_lines(document, count, insert_space)
            }
            RepeatAction::VisualBlock(_) => Ok(layout_required("Visual Block repeat")),
            RepeatAction::Paste {
                before,
                follow,
                count,
                register,
            } => {
                self.requested_register = Some(register);
                let count = choose_repeat_count(override_count, count);
                if follow {
                    self.paste_after_and_follow(document, before, count)
                } else {
                    self.paste(document, before, count)
                }
            }
        }
    }

    fn repeat_visual_operator(
        &mut self,
        document: &mut Document,
        command: VisualOperatorRepeat,
        edits: Option<EditSessionProgram>,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(program) = edits.as_ref() {
            Self::validate_edit_session_encoding(document, program)?;
            if let Err(error) = Self::validate_edit_session_repetition(program, 1, false) {
                return Ok(error.into_command_output());
            }
        }
        let extent = self.resolve_visual_repeat_extent(document, command.shape);
        let deletion_class = ordinary_deletion_class(&document.hard_line_snapshot(), &extent);
        let mut output = self.apply_operator_with_application_count(
            document,
            command.operator,
            extent,
            command.register,
            deletion_class,
            command.application_count,
        )?;
        if command.operator == Operator::Change {
            if let Some(program) = edits {
                output.merge(self.replay_edit_session_program(document, &program, 1, false)?);
            }
            output.merge(self.finish_insert(document)?);
        }
        Ok(output)
    }

    fn repeat_operator(
        &mut self,
        document: &mut Document,
        command: OperatorRepeat,
        edits: Option<EditSessionProgram>,
        override_count: Option<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        let count = choose_repeat_count(override_count, command.count);
        if let Some(program) = edits.as_ref() {
            Self::validate_edit_session_encoding(document, program)?;
            if let Err(error) = Self::validate_edit_session_repetition(program, 1, false) {
                return Ok(error.into_command_output());
            }
        }
        if command.operator == Operator::ToggleCase
            && matches!(command.target, RepeatTarget::Characters)
        {
            return self.toggle_at_cursor(document, count);
        }
        if let RepeatTarget::ViewLine(shape, applications) = command.target {
            let mut output = self.apply_mode_line_operator_repeated(
                document,
                command.operator,
                shape,
                count,
                command.register,
                applications,
            )?;
            if command.operator == Operator::Change {
                if let Some(program) = edits {
                    output.merge(self.replay_edit_session_program(document, &program, 1, false)?);
                }
                output.merge(self.finish_insert(document)?);
            }
            return Ok(output);
        }
        let extent = match &command.target {
            RepeatTarget::ViewLine(..) => unreachable!("handled above"),
            RepeatTarget::Motion(motion) => {
                self.resolve_operator_extent(document, command.operator, *motion, count)
            }
            RepeatTarget::Lines => Some(MotionExtent {
                range: linewise_range(&document.hard_line_snapshot(), self.cursor, count),
                kind: MotionKind::Linewise,
            }),
            RepeatTarget::TextObject(object) => {
                match resolve_text_object(
                    document.text(),
                    &document.hard_line_snapshot(),
                    self.cursor,
                    *object,
                    count,
                ) {
                    Ok(range) => Some(MotionExtent {
                        range,
                        kind: MotionKind::Characterwise,
                    }),
                    Err(error) => {
                        return Ok(CommandOutput {
                            status: CommandStatus::Error(format!(
                                "text object could not be resolved: {error:?}"
                            )),
                            ..CommandOutput::complete()
                        });
                    }
                }
            }
            RepeatTarget::Find(find) => {
                let lines = document.hard_line_snapshot();
                let Some(target) = find_character(
                    document.text(),
                    &lines,
                    self.cursor,
                    &find.needle,
                    find.forward,
                    find.till,
                    count,
                ) else {
                    return Ok(CommandOutput {
                        status: CommandStatus::SearchNotFound,
                        ..CommandOutput::complete()
                    });
                };
                let range = if target < self.cursor {
                    target..advance_graphemes(document.text(), self.cursor, 1)
                } else {
                    self.cursor..advance_graphemes(document.text(), target, 1)
                };
                Some(MotionExtent {
                    range,
                    kind: MotionKind::Characterwise,
                })
            }
            RepeatTarget::Mark { name, linewise } => {
                let Some(&marked) = self.marks.get(name) else {
                    return Ok(CommandOutput {
                        status: CommandStatus::Error(format!("mark {name} is not set")),
                        ..CommandOutput::complete()
                    });
                };
                Some(
                    operator_mark_extent(
                        document.text(),
                        &document.hard_line_snapshot(),
                        self.cursor,
                        marked.min(document.projection().text_tree().byte_len()),
                        *linewise,
                    )
                    .0,
                )
            }
            RepeatTarget::Search(search) => match search_destination(
                document.text(),
                &document.hard_line_snapshot(),
                self.cursor,
                search.direction,
                &search.pattern,
                count,
                self.search_options,
            ) {
                Ok(Some(destination)) => Some(exclusive_motion_extent(
                    document.text(),
                    &document.hard_line_snapshot(),
                    self.cursor,
                    destination,
                )),
                Ok(None) => {
                    return Ok(CommandOutput {
                        status: CommandStatus::SearchNotFound,
                        ..CommandOutput::complete()
                    });
                }
                Err(error) => {
                    return Ok(CommandOutput {
                        status: CommandStatus::Error(error),
                        ..CommandOutput::complete()
                    });
                }
            },
            RepeatTarget::Characters => Some(MotionExtent {
                range: self.cursor
                    ..advance_graphemes(document.text(), self.cursor, count)
                        .min(line_end(&document.hard_line_snapshot(), self.cursor)),
                kind: MotionKind::Characterwise,
            }),
        };
        let Some(extent) = extent else {
            return Ok(CommandOutput::complete());
        };
        let deletion_class =
            deletion_class_for_target(&document.hard_line_snapshot(), &extent, &command.target);
        let mut output = self.apply_operator(
            document,
            command.operator,
            extent,
            command.register,
            deletion_class,
        )?;
        if command.operator == Operator::Change {
            if let Some(program) = edits {
                output.merge(self.replay_edit_session_program(document, &program, 1, false)?);
            }
            output.merge(self.finish_insert(document)?);
        }
        Ok(output)
    }

    fn history(&mut self, document: &mut Document, redo: bool, count: usize) -> CommandOutput {
        if self.insert_session.is_some() {
            let _ = self.finish_insert(document);
        }
        let navigation = if redo {
            document.try_redo_count(count)
        } else {
            document.try_undo_count(count)
        };
        match navigation {
            Ok(_) => {
                let old_cursor = self.cursor;
                self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                    self.cursor.min(document.projection().text_tree().byte_len()),
                );
                CommandOutput {
                    document_changed: true,
                    cursor_moved: self.cursor != old_cursor,
                    history_navigation: true,
                    ..CommandOutput::complete()
                }
            }
            Err(error) => CommandOutput {
                status: CommandStatus::Error(error.to_string()),
                ..CommandOutput::complete()
            },
        }
    }

    fn enter_visual(
        &mut self,
        document: &Document,
        mode: Mode,
        count: usize,
        reuse_previous: bool,
    ) -> CommandOutput {
        if reuse_previous {
            if let Some(memory) = self.last_visual {
                if memory.mode == Mode::VisualBlock {
                    return layout_required("counted previous Visual Block selection");
                }
                return self.enter_scaled_previous_visual(document, memory, count);
            }
        }
        if mode == Mode::VisualLine {
            if let Some(output) = self.enter_mode_visual_line(document, count) {
                return output;
            }
        }
        let origin = self.cursor;
        let lines = document.hard_line_snapshot();
        let active = match mode {
            // Characterwise Visual mode may place its active endpoint on a
            // hard-line separator. Normal-cursor normalization would pull
            // that endpoint back onto the preceding character and make a
            // count unable to select a newline.
            Mode::VisualCharacter => {
                advance_graphemes(document.text(), origin, count.saturating_sub(1))
            }
            Mode::VisualLine => {
                let current = lines
                    .line_at_offset(origin)
                    .expect("a Normal cursor resolves to one hard line");
                let target = current
                    .index()
                    .saturating_add(count.saturating_sub(1))
                    .min(lines.line_count().saturating_sub(1));
                let target_start = lines
                    .line(target)
                    .expect("a clamped Visual line target exists")
                    .content_range()
                    .start;
                position_at_column(
                    document.text(),
                    &lines,
                    target_start,
                    grapheme_column(document.text(), &lines, origin),
                )
            }
            _ => origin,
        };
        self.mode = mode;
        self.visual_to_line_end = false;
        self.visual_anchor = Some(origin);
        self.cursor = active;
        self.visual_block = None;
        self.active_visual_block = None;
        CommandOutput {
            mode_changed: true,
            cursor_moved: self.cursor != origin,
            ..CommandOutput::complete()
        }
    }

    fn enter_scaled_previous_visual(
        &mut self,
        document: &Document,
        memory: VisualMemory,
        count: usize,
    ) -> CommandOutput {
        debug_assert!(memory.mode != Mode::VisualBlock);
        let count = count.max(1);
        let origin = self.cursor;
        let text = document.text();
        let lines = document.hard_line_snapshot();
        let shape = Self::visual_repeat_shape_for(
            document,
            memory.mode,
            memory.anchor,
            memory.active,
            memory.to_line_end,
        );
        let active = match shape {
            VisualRepeatShape::CharacterSingleLine {
                grapheme_count,
                to_line_end,
            } => {
                let Some(total) = grapheme_count.checked_mul(count) else {
                    return CommandOutput::count_error(CountError::Overflow);
                };
                if to_line_end {
                    last_grapheme_on_line(text, &lines, origin)
                } else {
                    advance_graphemes(text, origin, total.saturating_sub(1))
                }
            }
            VisualRepeatShape::CharacterMultiLine {
                hard_line_count,
                last_line_grapheme_count,
                to_line_end,
            } => {
                let Some(total_lines) = hard_line_count.checked_mul(count) else {
                    return CommandOutput::count_error(CountError::Overflow);
                };
                let current = lines
                    .line_at_offset(origin)
                    .expect("a Normal cursor resolves to one hard line");
                let target_index = current
                    .index()
                    .saturating_add(total_lines.saturating_sub(1))
                    .min(lines.line_count().saturating_sub(1));
                let target = lines
                    .line(target_index)
                    .expect("a clamped counted Visual line exists");
                if to_line_end {
                    last_grapheme_on_line(text, &lines, target.content_range().start)
                } else {
                    advance_graphemes(
                        text,
                        target.content_range().start,
                        last_line_grapheme_count.saturating_sub(1),
                    )
                    .min(target.content_range().end)
                }
            }
            VisualRepeatShape::Line { hard_line_count } => {
                let Some(total_lines) = hard_line_count.checked_mul(count) else {
                    return CommandOutput::count_error(CountError::Overflow);
                };
                let current = lines
                    .line_at_offset(origin)
                    .expect("a Normal cursor resolves to one hard line");
                let target_index = current
                    .index()
                    .saturating_add(total_lines.saturating_sub(1))
                    .min(lines.line_count().saturating_sub(1));
                let target_start = lines
                    .line(target_index)
                    .expect("a clamped counted Visual line exists")
                    .content_range()
                    .start;
                position_at_column(
                    text,
                    &lines,
                    target_start,
                    grapheme_column(text, &lines, origin),
                )
            }
        };

        self.mode = memory.mode;
        self.visual_to_line_end = memory.to_line_end;
        self.visual_anchor = Some(origin);
        self.cursor = active;
        self.visual_block = None;
        self.active_visual_block = None;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        CommandOutput {
            mode_changed: true,
            cursor_moved: self.cursor != origin,
            ..CommandOutput::complete()
        }
    }

    fn leave_visual(&mut self) {
        self.remember_visual();
        self.mode = Mode::Normal;
        self.visual_to_line_end = false;
        self.visual_anchor = None;
        self.clear_pending();
    }

    fn remember_visual(&mut self) {
        if let Some(anchor) = self.visual_anchor {
            self.last_visual = Some(VisualMemory {
                mode: self.mode,
                anchor,
                active: self.cursor,
                to_line_end: self.visual_to_line_end,
                block: None,
            });
        }
    }

    fn enter_search(&mut self, direction: SearchDirection, count: usize) {
        let kind = match direction {
            SearchDirection::Forward => CommandLineKind::SearchForward,
            SearchDirection::Backward => CommandLineKind::SearchBackward,
        };
        self.enter_command_line(kind);
        self.command_line_state
            .as_mut()
            .expect("enter_command_line installs command-line state")
            .count = count.max(1);
    }

    fn enter_operator_search(&mut self, direction: SearchDirection, operator: PendingOperator) {
        self.enter_search(direction, 1);
        self.command_line_state
            .as_mut()
            .expect("enter_search installs a command-line state")
            .operator = Some(operator);
    }

    fn enter_visual_ex(&mut self, document: &Document) -> CommandOutput {
        let (anchor, active) = if self.mode == Mode::VisualBlock {
            let Some(memory) = self.visual_block_memory() else {
                return CommandOutput::unsupported("Visual Block has no resolved selection");
            };
            (memory.anchor, memory.active)
        } else {
            (self.visual_anchor.unwrap_or(self.cursor), self.cursor)
        };
        let Some(first) = document.hard_line_at_offset(anchor.min(active)) else {
            return CommandOutput::unsupported("Visual selection has no starting hard line");
        };
        let Some(last) = document.hard_line_at_offset(anchor.max(active)) else {
            return CommandOutput::unsupported("Visual selection has no ending hard line");
        };
        // Ex addresses complete logical hard lines, including when the source
        // selection was characterwise or a rectangle of wrapped visual rows.
        // Leaving Visual records its stable reselect memory before the prompt.
        if self.mode == Mode::VisualBlock {
            self.leave_visual_block();
        } else {
            self.leave_visual();
        }
        self.enter_command_line(CommandLineKind::Ex);
        let state = self.command_line_state.as_mut().unwrap();
        state.buffer.set(format!("{},{}", first + 1, last + 1));
        state.visual_range_revision = Some(document.revision());
        CommandOutput {
            mode_changed: true,
            ..CommandOutput::pending()
        }
    }

    fn enter_command_line(&mut self, kind: CommandLineKind) {
        let return_mode = self.mode;
        let requested_register = self.requested_register;
        self.mode = Mode::CommandLine;
        self.command_line_state = Some(CommandLineState {
            kind,
            buffer: CommandLineBuffer::default(),
            return_mode,
            count: 1,
            operator: None,
            visual_range_revision: None,
        });
        self.clear_pending();
        self.requested_register = requested_register;
    }

    fn submit_command_line(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(state) = self.command_line_state.take() else {
            return Ok(CommandOutput::unsupported("command line"));
        };
        self.mode = state.return_mode;
        if state
            .visual_range_revision
            .is_some_and(|revision| revision != document.revision())
        {
            return Ok(CommandOutput {
                status: CommandStatus::Error(
                    "Visual Ex range is stale; reselect the range".into(),
                ),
                mode_changed: true,
                ..CommandOutput::complete()
            });
        }
        match state.kind {
            CommandLineKind::SearchForward | CommandLineKind::SearchBackward => {
                let direction = match state.kind {
                    CommandLineKind::SearchForward => SearchDirection::Forward,
                    CommandLineKind::SearchBackward => SearchDirection::Backward,
                    CommandLineKind::Ex => unreachable!(),
                };
                let entered = state.buffer.input;
                let pattern = if entered.is_empty() {
                    self.last_search
                        .as_ref()
                        .map(|(_, pattern)| pattern.clone())
                        .unwrap_or_default()
                } else {
                    entered
                };
                if pattern.is_empty() {
                    return Ok(CommandOutput {
                        status: CommandStatus::Error("no previous search".into()),
                        mode_changed: true,
                        ..CommandOutput::complete()
                    });
                }
                let origin = self.cursor;
                let mut output = if let Some(operator) = state.operator {
                    self.execute_operator_search(
                        document,
                        operator,
                        OperatorSearch {
                            direction,
                            pattern: pattern.clone(),
                        },
                    )?
                } else {
                    self.search_pattern(document, direction, &pattern, state.count)
                };
                output.mode_changed = true;
                if !matches!(output.status, CommandStatus::Error(_))
                    && (state.operator.is_none()
                        || matches!(output.status, CommandStatus::Complete))
                {
                    push_history(&mut self.search_history, pattern.clone());
                    self.last_search = Some((direction, pattern));
                }
                Ok(if state.operator.is_some() {
                    output
                } else {
                    self.record_successful_jump(document, origin, output)
                })
            }
            CommandLineKind::Ex => {
                let command = state.buffer.input;
                if command.is_empty() {
                    return Ok(CommandOutput {
                        mode_changed: true,
                        ..CommandOutput::complete()
                    });
                }
                let output = self.execute_ex_command(document, &command);
                if !matches!(
                    output.status,
                    CommandStatus::Error(_) | CommandStatus::ExError(_)
                ) {
                    push_history(&mut self.ex_history, command);
                }
                Ok(output)
            }
        }
    }

    fn navigate_command_line_history(&mut self, older: bool) {
        let Some(state) = self.command_line_state.as_mut() else {
            return;
        };
        let history = match state.kind {
            CommandLineKind::Ex => &self.ex_history,
            CommandLineKind::SearchForward | CommandLineKind::SearchBackward => {
                &self.search_history
            }
        };
        navigate_history(&mut state.buffer, history, older);
    }

    fn execute_ex_command(&mut self, document: &mut Document, input: &str) -> CommandOutput {
        let command = match parse_ex(input) {
            Ok(command) => command,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::ExError(ExCommandError::Parse(error)),
                    mode_changed: true,
                    ..CommandOutput::complete()
                };
            }
        };
        if let ExAction::Delete(arguments) | ExAction::Yank(arguments) = &command.action {
            if let Err(mut output) = self.require_register_write(arguments.register) {
                output.mode_changed = true;
                return output;
            }
        }
        let contextual_register = match &command.action {
            ExAction::Put { register } => {
                let name = register.unwrap_or('"');
                match self.read_register_value_for_put(document, name) {
                    Ok(Some(value)) if matches!(name, '+' | '*' | '%') => {
                        Some((*register, register_to_ex_value(&value)))
                    }
                    Ok(_) => None,
                    Err(error) => {
                        return CommandOutput {
                            status: CommandStatus::RegisterReadError(error),
                            mode_changed: true,
                            ..CommandOutput::complete()
                        };
                    }
                }
            }
            _ => None,
        };
        let register_reader = ContextualExRegisterReader {
            stored: &self.registers,
            override_value: contextual_register,
        };
        let records_jump = matches!(
            command.action,
            ExAction::Substitute(_) | ExAction::RepeatSubstitute { .. }
        );
        let old_cursor = self.cursor;
        let jump_origin = records_jump.then(|| {
            document.text_point(old_cursor).ok().and_then(|point| {
                document
                    .text_anchor(
                        point,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )
                    .ok()
            })
        });
        let jump_origin = jump_origin.flatten();
        let context = ExExecutionContext {
            current_line: document
                .hard_line_at_offset(self.cursor.min(document.projection().text_tree().byte_len()))
                .expect("a command cursor resolves to one hard line"),
            wrap: self.wrap,
            fileformats: self.fileformats.clone(),
            search_options: self.search_options,
            text_width: self.text_width,
            last_search_pattern: self
                .last_search
                .as_ref()
                .map(|(_, pattern)| pattern.clone()),
        };
        let plan = match prepare_ex(
            document,
            &self.ex_state,
            &context,
            &command,
            &register_reader,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::ExError(ExCommandError::Execute(error)),
                    mode_changed: true,
                    ..CommandOutput::complete()
                };
            }
        };
        let mut outcome = match commit_ex(document, &mut self.ex_state, plan) {
            Ok(outcome) => outcome,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::ExError(ExCommandError::Execute(error)),
                    mode_changed: true,
                    ..CommandOutput::complete()
                };
            }
        };
        if matches!(
            command.action,
            ExAction::Substitute(_) | ExAction::RepeatSubstitute { .. }
        ) {
            // Vim's substitute pattern becomes the last search pattern while
            // retaining the previous search direction. Publish it only after
            // commit so every failed Ex path leaves search state untouched.
            if let Some(pattern) = self.ex_state.previous_substitute_pattern() {
                let direction = self
                    .last_search
                    .as_ref()
                    .map_or(SearchDirection::Forward, |(direction, _)| *direction);
                self.last_search = Some((direction, pattern.to_owned()));
            }
        }

        if let Some(index) = outcome
            .frontend_requests
            .iter()
            .position(|request| matches!(request, ExFrontendRequest::Normal(_)))
        {
            let ExFrontendRequest::Normal(request) = outcome.frontend_requests.remove(index) else {
                unreachable!("the located Ex request is Normal")
            };
            if self.plan_compound_replay {
                let Some(targets) = self.ex_normal_targets(document, &request) else {
                    return CommandOutput {
                        status: CommandStatus::Error(
                            ":normal range could not be bound to stable document blocks".into(),
                        ),
                        mode_changed: true,
                        ex_outcome: Some(outcome),
                        ..CommandOutput::complete()
                    };
                };
                self.pending_replay = Some(ReplayPlan::ExNormal(ExNormalReplayPlan {
                    literal: request.literal,
                    events: request
                        .commands
                        .chars()
                        .map(|character| InputEvent::Key(Key::Char(character)))
                        .collect(),
                    targets,
                }));
                return CommandOutput {
                    status: CommandStatus::Complete,
                    cursor_moved: self.cursor != old_cursor,
                    mode_changed: true,
                    ex_outcome: Some(outcome),
                    ..CommandOutput::complete()
                };
            }
            let mut normal_output = self.execute_ex_normal(document, &request);
            outcome.document_changed |= normal_output.document_changed;
            if let Some(nested) = normal_output.ex_outcome.take() {
                if let Err(error) = outcome.try_merge(nested) {
                    normal_output.status = CommandStatus::Error(format!(
                        ":normal could not merge committed Ex effects: {error}"
                    ));
                }
            }
            normal_output.cursor_moved |= self.cursor != old_cursor;
            normal_output.mode_changed = true;
            normal_output.ex_outcome = Some(outcome);
            return normal_output;
        }

        self.apply_ex_register_effects(&outcome);
        self.apply_ex_option_effects(&outcome.option_effects);
        let mapped_jump_origin = jump_origin.and_then(|origin| {
            outcome
                .model_transaction()
                .map_or(Some(origin.offset()), |transaction| {
                    mapped_anchor(transaction.text_position_map(), origin)
                        .ok()
                        .flatten()
                })
        });
        let jump_was_taken = records_jump
            && matches!(
                outcome.navigation,
                Some(ExNavigation::TextOffset(offset)) if offset != old_cursor
            );
        match outcome.navigation {
            Some(ExNavigation::TextOffset(offset)) => {
                self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                    offset,
                );
                self.boundary_affinity = BoundaryAffinity::Downstream;
            }
            Some(ExNavigation::HistoryRestoration) => {
                self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                    self.cursor.min(document.projection().text_tree().byte_len()),
                );
            }
            None => {
                if outcome.document_changed {
                    self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                        self.cursor.min(document.projection().text_tree().byte_len()),
                    );
                }
            }
        }
        self.preferred_column = None;
        self.visual_position = None;
        self.desired_x = None;
        if let Some(origin) = mapped_jump_origin.filter(|_| jump_was_taken) {
            self.record_jump(document, origin, self.cursor);
        }
        CommandOutput {
            status: CommandStatus::Complete,
            // An explicit post-transaction cursor is authoritative even if its
            // ordinal equals the old one. Otherwise the coordinator would map
            // it again through the edit (for example :sort starting at zero).
            cursor_moved: self.cursor != old_cursor
                || (outcome.document_changed
                    && matches!(outcome.navigation, Some(ExNavigation::TextOffset(_)))),
            document_changed: outcome.document_changed,
            mode_changed: true,
            history_navigation: matches!(
                outcome.navigation,
                Some(ExNavigation::HistoryRestoration)
            ),
            ex_outcome: Some(outcome),
            clipboard_writes: Vec::new(),
        }
    }

    fn execute_ex_normal(
        &mut self,
        document: &mut Document,
        request: &ExNormalRequest,
    ) -> CommandOutput {
        if self.compound_replay_depth >= COMPOUND_REPLAY_LIMIT {
            return CommandOutput {
                status: CommandStatus::Error(format!(
                    ":normal recursion limit ({COMPOUND_REPLAY_LIMIT}) reached"
                )),
                ..CommandOutput::complete()
            };
        }

        let Some(mut targets) = self.ex_normal_targets(document, request) else {
            return CommandOutput {
                status: CommandStatus::Error(
                    ":normal range could not be bound to stable document blocks".into(),
                ),
                ..CommandOutput::complete()
            };
        };

        // Mappings do not exist in this core yet, so both `:normal` and
        // `:normal!` execute the same built-in event stream. Retain and consume
        // the literal bit here rather than leaking either spelling to a host.
        let _literal = request.literal;
        let original_cursor = self.cursor;
        let original_revision = document.revision();
        let mut output = CommandOutput::complete();
        self.compound_replay_depth += 1;
        document.begin_edit_group();

        for index in 0..targets.len() {
            let Some(target) = targets[index] else {
                continue;
            };
            let Some(position) = ex_normal_target_position(document, target) else {
                targets[index] = None;
                continue;
            };
            self.prepare_ex_normal_line(document, position);

            let (line_output, map) = document.capture_position_maps(|document| {
                Ok::<_, std::convert::Infallible>(
                    self.execute_normal_event_text(document, &request.commands),
                )
            });
            let line_output = match line_output {
                Ok(line_output) => line_output,
                Err(never) => match never {},
            };
            let stop = command_status_stops_compound(&line_output.status);
            output.merge(line_output);
            let stop = stop || command_status_stops_compound(&output.status);

            if let Err(error) = rebase_ex_normal_targets(document, &mut targets[index + 1..], &map)
            {
                output.status = CommandStatus::Error(format!(
                    ":normal could not rebase its remaining range: {error}"
                ));
                break;
            }
            if stop {
                break;
            }
        }

        document.end_edit_group();
        self.compound_replay_depth -= 1;
        output.document_changed |= document.revision() != original_revision;
        output.cursor_moved |= self.cursor != original_cursor;
        if matches!(
            output.status,
            CommandStatus::Pending | CommandStatus::Cancelled
        ) {
            output.status = CommandStatus::Complete;
        }
        output
    }

    fn ex_normal_targets(
        &self,
        document: &Document,
        request: &ExNormalRequest,
    ) -> Option<Vec<Option<ExNormalTarget>>> {
        (request.range.start..=request.range.end)
            .map(|line| {
                let start = document.line_start(line)?;
                let block_id = block_id_at_hard_line_start(document, start)?;
                let point = document.text_point(start).ok()?;
                Some(Some(ExNormalTarget {
                    anchor: document
                        .text_anchor(
                            point,
                            Association::AfterInsertion,
                            BoundaryAffinity::Downstream,
                            DeletionRecovery::PreferFollowingThenPreceding,
                        )
                        .ok()?,
                    block_id,
                }))
            })
            .collect()
    }

    pub(crate) fn prepare_ex_normal_line(&mut self, document: &Document, position: usize) {
        self.mode = Mode::Normal;
        debug_assert!(position <= document.projection().text_tree().byte_len());
        self.cursor = position;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.visual_block_insert = None;
        self.command_line_state = None;
        self.insert_session = None;
        if self.compound_replay_depth == 0 {
            self.insert_normal_once = None;
        }
        self.clear_pending();
    }

    fn execute_normal_event_text(
        &mut self,
        document: &mut Document,
        commands: &str,
    ) -> CommandOutput {
        let mut output = CommandOutput::complete();
        let mut failure = None;

        for character in commands.chars() {
            if self.mode == Mode::Normal && self.pending == Pending::None && character == 'q' {
                failure = Some(CommandStatus::Unsupported(
                    ":normal macro recording is not supported".into(),
                ));
                break;
            }
            match self.handle(document, InputEvent::Key(Key::Char(character))) {
                Ok(next) => {
                    let stop = command_status_stops_compound(&next.status);
                    output.merge(next);
                    if stop || command_status_stops_compound(&output.status) {
                        failure = Some(output.status.clone());
                        break;
                    }
                }
                Err(error) => {
                    failure = Some(CommandStatus::Error(format!(
                        ":normal command failed after its successful prefix: {error}"
                    )));
                    break;
                }
            }
        }

        let abort = self.abort_incomplete_replay(document);
        let abort_failure =
            command_status_stops_compound(&abort.status).then(|| abort.status.clone());
        output.cursor_moved |= abort.cursor_moved;
        output.document_changed |= abort.document_changed;
        output.mode_changed |= abort.mode_changed;
        if let Some(outcome) = abort.ex_outcome {
            output.merge(CommandOutput {
                status: CommandStatus::Complete,
                ex_outcome: Some(outcome),
                ..CommandOutput::complete()
            });
        }
        output.status = failure.or(abort_failure).unwrap_or(CommandStatus::Complete);
        output
    }

    fn abort_incomplete_replay(&mut self, document: &mut Document) -> CommandOutput {
        let incomplete = self.mode != Mode::Normal
            || self.pending != Pending::None
            || self.register_pending
            || self.command_line_state.is_some()
            || self.visual_anchor.is_some()
            || self.visual_block.is_some()
            || self.visual_block_insert.is_some();
        if !incomplete {
            return CommandOutput::complete();
        }
        match self.handle(document, InputEvent::Key(Key::Escape)) {
            Ok(mut output) => {
                output.status = CommandStatus::Complete;
                output
            }
            Err(error) => CommandOutput {
                status: CommandStatus::Error(format!(
                    "could not abort an incomplete replayed command: {error}"
                )),
                ..CommandOutput::complete()
            },
        }
    }

    fn apply_ex_register_effects(&mut self, outcome: &ExOutcome) {
        for effect in &outcome.register_effects {
            let value = match effect.value.kind {
                ExRegisterKind::Characterwise => RegisterValue::try_new(
                    effect.value.text.clone(),
                    RegisterKind::Characterwise,
                    effect.value.hard_break_offsets().to_vec(),
                )
                .expect("Ex register effects carry validated semantic-break offsets"),
                ExRegisterKind::Linewise => {
                    let mut text = effect.value.text.clone();
                    let mut hard_break_offsets = effect.value.hard_break_offsets().to_vec();
                    let ends_with_hard_break = hard_break_offsets
                        .last()
                        .is_some_and(|offset| offset.saturating_add(1) == text.len());
                    if !ends_with_hard_break {
                        hard_break_offsets.push(text.len());
                        text.push('\n');
                    }
                    RegisterValue::try_new(text, RegisterKind::Linewise, hard_break_offsets)
                        .expect("Ex linewise register effects carry valid hard breaks")
                }
            };
            match effect.kind {
                ExRegisterEffectKind::Yank => self.yank_register(effect.requested, value),
                ExRegisterEffectKind::Delete => {
                    self.delete_register(effect.requested, value, DeletionClass::Large)
                }
            }
        }
    }

    fn apply_ex_option_effects(&mut self, effects: &[ExOptionEffect]) {
        for effect in effects {
            match (&effect.name, &effect.new_value) {
                (ExOptionName::IgnoreCase, ExOptionValue::Boolean(value)) => {
                    self.search_options.ignorecase = *value
                }
                (ExOptionName::SmartCase, ExOptionValue::Boolean(value)) => {
                    self.search_options.smartcase = *value
                }
                (ExOptionName::WrapScan, ExOptionValue::Boolean(value)) => {
                    self.search_options.wrapscan = *value
                }
                (ExOptionName::Wrap, ExOptionValue::Boolean(value)) => self.wrap = *value,
                (ExOptionName::TextWidth, ExOptionValue::OptionalNumber(value)) => {
                    self.text_width.set_local(*value);
                }
                (ExOptionName::FileFormats, ExOptionValue::FileFormats(value)) => {
                    self.fileformats.clone_from(value);
                }
                (ExOptionName::FileFormat, ExOptionValue::FileFormat(_)) => {
                    // The document mutation is authoritative for `fileformat`.
                }
                _ => {}
            }
        }
    }

    fn repeat_search(
        &mut self,
        document: &Document,
        opposite: bool,
        count: usize,
    ) -> CommandOutput {
        let Some((mut direction, pattern)) = self.last_search.clone() else {
            return CommandOutput {
                status: CommandStatus::Error("no previous search".into()),
                ..CommandOutput::complete()
            };
        };
        if opposite {
            direction = match direction {
                SearchDirection::Forward => SearchDirection::Backward,
                SearchDirection::Backward => SearchDirection::Forward,
            };
        }
        let origin = self.cursor;
        let output = self.search_pattern(document, direction, &pattern, count);
        self.record_successful_jump(document, origin, output)
    }

    fn execute_find(
        &mut self,
        document: &Document,
        find: FindState,
        count: usize,
        remember: bool,
    ) -> CommandOutput {
        let old = self.cursor;
        let lines = document.hard_line_snapshot();
        if remember {
            self.last_find = Some(find.clone());
        }
        let Some(target) = find_character(
            document.text(),
            &lines,
            self.cursor,
            &find.needle,
            find.forward,
            find.till,
            count,
        ) else {
            return CommandOutput {
                status: CommandStatus::SearchNotFound,
                ..CommandOutput::complete()
            };
        };
        self.cursor = target;
        self.preferred_column = None;
        CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        }
    }

    fn repeat_find(&mut self, document: &Document, opposite: bool, count: usize) -> CommandOutput {
        let Some(mut find) = self.last_find.clone() else {
            return CommandOutput {
                status: CommandStatus::Error("no previous character find".into()),
                ..CommandOutput::complete()
            };
        };
        if opposite {
            find.forward = !find.forward;
        }
        let old = self.cursor;
        let lines = document.hard_line_snapshot();
        let Some(target) = repeat_find_character(
            document.text(),
            &lines,
            self.cursor,
            &find.needle,
            find.forward,
            find.till,
            count,
        ) else {
            return CommandOutput {
                status: CommandStatus::SearchNotFound,
                ..CommandOutput::complete()
            };
        };
        self.cursor = target;
        self.preferred_column = None;
        CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        }
    }

    fn execute_operator_find(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        find: FindState,
        repeat: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let count = match effective_operator_count(pending) {
            Ok(count) => count,
            Err(error) => return Ok(CommandOutput::count_error(error)),
        };
        let lines = document.hard_line_snapshot();
        if !repeat {
            self.last_find = Some(find.clone());
        }
        let target = if repeat {
            repeat_find_character(
                document.text(),
                &lines,
                self.cursor,
                &find.needle,
                find.forward,
                find.till,
                count,
            )
        } else {
            find_character(
                document.text(),
                &lines,
                self.cursor,
                &find.needle,
                find.forward,
                find.till,
                count,
            )
        };
        let Some(target) = target else {
            return Ok(CommandOutput {
                status: CommandStatus::SearchNotFound,
                ..CommandOutput::complete()
            });
        };
        let range = if target < self.cursor {
            target..advance_graphemes(document.text(), self.cursor, 1)
        } else {
            self.cursor..advance_graphemes(document.text(), target, 1)
        };
        self.apply_operator_with_repeat(
            document,
            pending.operator,
            MotionExtent {
                range,
                kind: MotionKind::Characterwise,
            },
            pending.register,
            RepeatTarget::Find(find),
            count,
        )
    }

    fn execute_operator_mark(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        name: char,
        linewise: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(&marked) = self.marks.get(&name) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!("mark {name} is not set")),
                ..CommandOutput::complete()
            });
        };
        let (extent, destination) = operator_mark_extent(
            document.text(),
            &document.hard_line_snapshot(),
            self.cursor,
            marked.min(document.projection().text_tree().byte_len()),
            linewise,
        );
        let count = match effective_operator_count(pending) {
            Ok(count) => count,
            Err(error) => return Ok(CommandOutput::count_error(error)),
        };
        self.apply_operator_target_with_jump(
            document,
            pending.operator,
            extent,
            pending.register,
            OperatorTarget {
                repeat: RepeatTarget::Mark { name, linewise },
                count,
                jump_destination: Some(destination),
            },
        )
    }

    fn execute_operator_repeat_search(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        opposite: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let Some((mut direction, pattern)) = self.last_search.clone() else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no previous search".into()),
                ..CommandOutput::complete()
            });
        };
        if opposite {
            direction = match direction {
                SearchDirection::Forward => SearchDirection::Backward,
                SearchDirection::Backward => SearchDirection::Forward,
            };
        }
        self.execute_operator_search(document, pending, OperatorSearch { direction, pattern })
    }

    fn execute_operator_word_search(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        forward: bool,
        whole_word: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(range) = keyword_range(document.text(), self.cursor) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no word under cursor".into()),
                ..CommandOutput::complete()
            });
        };
        let escaped = regex_v1::escape_literal(&document.text()[range]);
        let pattern = if whole_word {
            format!(r"\b{escaped}\b")
        } else {
            escaped
        };
        let direction = if forward {
            SearchDirection::Forward
        } else {
            SearchDirection::Backward
        };
        let output = self.execute_operator_search(
            document,
            pending,
            OperatorSearch {
                direction,
                pattern: pattern.clone(),
            },
        )?;
        if matches!(output.status, CommandStatus::Complete) {
            self.last_search = Some((direction, pattern));
        }
        Ok(output)
    }

    fn execute_operator_search(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        search: OperatorSearch,
    ) -> Result<CommandOutput, DocumentError> {
        let count = match effective_operator_count(pending) {
            Ok(count) => count,
            Err(error) => return Ok(CommandOutput::count_error(error)),
        };
        let destination = match search_destination(
            document.text(),
            &document.hard_line_snapshot(),
            self.cursor,
            search.direction,
            &search.pattern,
            count,
            self.search_options,
        ) {
            Ok(Some(destination)) => destination,
            Ok(None) => {
                return Ok(CommandOutput {
                    status: CommandStatus::SearchNotFound,
                    ..CommandOutput::complete()
                });
            }
            Err(error) => {
                return Ok(CommandOutput {
                    status: CommandStatus::Error(error),
                    ..CommandOutput::complete()
                });
            }
        };
        let extent = exclusive_motion_extent(
            document.text(),
            &document.hard_line_snapshot(),
            self.cursor,
            destination,
        );
        self.apply_operator_target_with_jump(
            document,
            pending.operator,
            extent,
            pending.register,
            OperatorTarget {
                repeat: RepeatTarget::Search(search),
                count,
                jump_destination: Some(destination),
            },
        )
    }

    fn execute_text_object_operator(
        &mut self,
        document: &mut Document,
        pending: PendingOperator,
        scope: TextObjectScope,
        key: char,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(kind) = TextObjectKind::from_vim_key(key) else {
            return Ok(CommandOutput::unsupported(format!(
                "unknown text object {key}"
            )));
        };
        let count = match effective_operator_count(pending) {
            Ok(count) => count,
            Err(error) => return Ok(CommandOutput::count_error(error)),
        };
        let range = match resolve_text_object(
            document.text(),
            &document.hard_line_snapshot(),
            self.cursor,
            TextObject { scope, kind },
            count,
        ) {
            Ok(range) => range,
            Err(error) => {
                return Ok(CommandOutput {
                    status: CommandStatus::Error(format!(
                        "text object could not be resolved: {error:?}"
                    )),
                    ..CommandOutput::complete()
                })
            }
        };
        self.apply_operator_with_repeat(
            document,
            pending.operator,
            MotionExtent {
                range,
                kind: MotionKind::Characterwise,
            },
            pending.register,
            RepeatTarget::TextObject(TextObject { scope, kind }),
            count,
        )
    }

    fn match_pair_motion(&mut self, document: &Document) -> CommandOutput {
        let old = self.cursor;
        let Some(target) =
            matching_pair(document.text(), &document.hard_line_snapshot(), self.cursor)
        else {
            return CommandOutput {
                status: CommandStatus::SearchNotFound,
                ..CommandOutput::complete()
            };
        };
        self.cursor = target;
        self.preferred_column = None;
        let output = CommandOutput {
            cursor_moved: old != target,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, old, output)
    }

    fn search_word_at_cursor(
        &mut self,
        document: &Document,
        forward: bool,
        whole_word: bool,
        count: usize,
    ) -> CommandOutput {
        let Some(range) = keyword_range(document.text(), self.cursor) else {
            return CommandOutput {
                status: CommandStatus::Error("no word under cursor".into()),
                ..CommandOutput::complete()
            };
        };
        let escaped = regex_v1::escape_literal(&document.text()[range]);
        let pattern = if whole_word {
            format!(r"\b{escaped}\b")
        } else {
            escaped
        };
        let direction = if forward {
            SearchDirection::Forward
        } else {
            SearchDirection::Backward
        };
        let origin = self.cursor;
        let output = self.search_pattern(document, direction, &pattern, count);
        if !matches!(output.status, CommandStatus::Error(_)) {
            self.last_search = Some((direction, pattern));
        }
        self.record_successful_jump(document, origin, output)
    }

    fn jump_to_mark(&mut self, document: &Document, name: char, linewise: bool) -> CommandOutput {
        let Some(&marked) = self.marks.get(&name) else {
            return CommandOutput {
                status: CommandStatus::Error(format!("mark {name} is not set")),
                ..CommandOutput::complete()
            };
        };
        let lines = document.hard_line_snapshot();
        let target = if linewise {
            first_nonblank_document(document, &lines, marked.min(document.projection().text_tree().byte_len()))
        } else {
            normalize_normal_cursor_document(document, &lines, marked.min(document.projection().text_tree().byte_len()))
        };
        let origin = self.cursor;
        let moved = origin != target;
        self.cursor = target;
        let output = CommandOutput {
            cursor_moved: moved,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, origin, output)
    }

    fn record_successful_jump(
        &mut self,
        document: &Document,
        origin: usize,
        output: CommandOutput,
    ) -> CommandOutput {
        if output.cursor_moved {
            self.typing_style = Default::default();
            self.record_jump(document, origin, self.cursor);
        }
        output
    }

    fn record_jump(&mut self, document: &Document, from: usize, to: usize) {
        let lines = document.hard_line_snapshot();
        // `jumps[jump_index]` is the live location. A new jump after CTRL-O
        // discards the newer branch, and an ordinary motion since the last
        // jump replaces that live slot with its exact origin.
        if self.jump_index + 1 < self.jumps.len() {
            self.jumps.truncate(self.jump_index + 1);
        }
        if !self.jumps.is_empty() {
            self.jumps.pop();
        }
        let origin_line = lines
            .line_at_offset(from.min(lines.text_length()))
            .expect("a jump origin resolves to one hard line")
            .index();
        self.jumps.retain(|position| {
            lines
                .line_at_offset((*position).min(lines.text_length()))
                .expect("a retained jump resolves to one hard line")
                .index()
                != origin_line
        });
        self.jumps.push(from);
        const JUMP_LIST_CAPACITY: usize = 100;
        if self.jumps.len() > JUMP_LIST_CAPACITY {
            let excess = self.jumps.len() - JUMP_LIST_CAPACITY;
            self.jumps.drain(..excess);
        }
        // Keep one live destination in addition to Vim's 100 remembered jump
        // origins. It becomes an origin (and is deduplicated by hard line) at
        // the next jump.
        self.jumps.push(to);
        self.jump_index = self.jumps.len() - 1;
    }

    fn navigate_jump(&mut self, document: &Document, newer: bool, count: usize) -> CommandOutput {
        if self.jumps.is_empty() {
            return CommandOutput {
                status: CommandStatus::Error("jump list is empty".into()),
                ..CommandOutput::complete()
            };
        }
        let old = self.cursor;
        // Vim snapshots the live cursor when CTRL-O first leaves the newest
        // position. Once traversal is within the list, ordinary cursor motion
        // does not rewrite the historical entry under the traversal index.
        if !newer && self.jump_index + 1 == self.jumps.len() {
            self.jumps[self.jump_index] = old;
        }
        let old_index = self.jump_index;
        for _ in 0..count {
            if newer {
                if self.jump_index + 1 >= self.jumps.len() {
                    break;
                }
                self.jump_index += 1;
            } else {
                if self.jump_index == 0 {
                    break;
                }
                self.jump_index -= 1;
            }
        }
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            self.jumps[self.jump_index].min(document.projection().text_tree().byte_len()),
        );
        CommandOutput {
            status: if self.jump_index == old_index {
                CommandStatus::Error(if newer {
                    "already at newest jump".into()
                } else {
                    "already at oldest jump".into()
                })
            } else {
                CommandStatus::Complete
            },
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        }
    }

    fn play_macro(
        &mut self,
        document: &mut Document,
        requested: Option<char>,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(name) = requested.or(self.last_macro) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error("no previous macro".into()),
                ..CommandOutput::complete()
            });
        };
        let Some(events) = self.registers.macro_events(name) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!("macro {name} is empty")),
                ..CommandOutput::complete()
            });
        };
        if events.is_empty() {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!("macro {name} is empty")),
                ..CommandOutput::complete()
            });
        }
        if self.compound_replay_depth >= COMPOUND_REPLAY_LIMIT {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!(
                    "macro recursion limit ({COMPOUND_REPLAY_LIMIT}) reached"
                )),
                ..CommandOutput::complete()
            });
        }
        let iterations = count.max(1);
        let replay_too_large = events
            .len()
            .checked_mul(iterations)
            .map_or(true, |total| total > MACRO_REPLAY_EVENT_LIMIT);
        if replay_too_large {
            return Ok(CommandOutput::count_error(
                CountError::ReplayEventBudgetExceeded {
                    count: iterations,
                    events_per_iteration: events.len(),
                    limit: MACRO_REPLAY_EVENT_LIMIT,
                },
            ));
        }
        self.last_macro = Some(name);
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::Macro(MacroReplayPlan {
                register: name,
                iterations,
                events: events.to_vec(),
            }));
            return Ok(CommandOutput::complete());
        }
        self.compound_replay_depth += 1;
        document.begin_edit_group();
        let mut output = CommandOutput::complete();
        let mut failure_status = None;
        'replay: for _ in 0..iterations {
            for event in &events {
                match self.handle(document, event.clone()) {
                    Ok(next) => {
                        let stop = command_status_stops_compound(&next.status);
                        output.merge(next);
                        if stop || command_status_stops_compound(&output.status) {
                            failure_status = Some(output.status.clone());
                            break 'replay;
                        }
                    }
                    Err(error) => {
                        failure_status = Some(CommandStatus::Error(format!(
                            "macro {name} stopped after its successful prefix: {error}"
                        )));
                        break 'replay;
                    }
                }
            }
        }
        if failure_status.is_some() {
            let abort = self.abort_incomplete_replay(document);
            output.cursor_moved |= abort.cursor_moved;
            output.document_changed |= abort.document_changed;
            output.mode_changed |= abort.mode_changed;
            if let Some(outcome) = abort.ex_outcome {
                output.merge(CommandOutput {
                    status: CommandStatus::Complete,
                    ex_outcome: Some(outcome),
                    ..CommandOutput::complete()
                });
            }
        }
        document.end_edit_group();
        self.compound_replay_depth -= 1;
        if let Some(status) = failure_status {
            output.status = status;
        }
        Ok(output)
    }

    fn restore_visual(&mut self, document: &Document) -> CommandOutput {
        let Some(memory) = self.last_visual else {
            return CommandOutput {
                status: CommandStatus::Error("no previous visual selection".into()),
                ..CommandOutput::complete()
            };
        };
        if memory.mode == Mode::VisualBlock {
            return layout_required("restoring Visual Block");
        }
        self.mode = memory.mode;
        self.visual_to_line_end = memory.to_line_end;
        self.visual_anchor = Some(normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            memory.anchor.min(document.projection().text_tree().byte_len()),
        ));
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
            memory.active.min(document.projection().text_tree().byte_len()),
        );
        CommandOutput {
            mode_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        }
    }

    fn exchange_visual(&mut self, document: &Document) -> CommandOutput {
        let Some(previous) = self.last_visual else {
            return CommandOutput {
                status: CommandStatus::Error("no previous visual selection".into()),
                ..CommandOutput::complete()
            };
        };
        if previous.mode == Mode::VisualBlock {
            return layout_required("exchanging Visual Block");
        }
        let Some(current_anchor) = self.visual_anchor else {
            return CommandOutput::unsupported("current Visual selection is missing");
        };
        let current = VisualMemory {
            mode: self.mode,
            anchor: current_anchor,
            active: self.cursor,
            to_line_end: self.visual_to_line_end,
            block: None,
        };
        let old_mode = self.mode;
        let lines = document.hard_line_snapshot();
        self.mode = previous.mode;
        self.visual_to_line_end = previous.to_line_end;
        self.visual_anchor = Some(normalize_normal_cursor_document(document, &lines,
            previous.anchor.min(document.projection().text_tree().byte_len()),
        ));
        self.cursor = normalize_normal_cursor_document(document, &lines,
            previous.active.min(document.projection().text_tree().byte_len()),
        );
        self.last_visual = Some(current);
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        CommandOutput {
            cursor_moved: true,
            mode_changed: old_mode != self.mode,
            ..CommandOutput::complete()
        }
    }

    fn paste_after_and_follow(
        &mut self,
        document: &mut Document,
        before: bool,
        count: usize,
    ) -> Result<CommandOutput, DocumentError> {
        self.paste_impl(document, before, count, true)
    }

    fn search_pattern(
        &mut self,
        document: &Document,
        direction: SearchDirection,
        pattern: &str,
        count: usize,
    ) -> CommandOutput {
        let old = self.cursor;
        match search_destination(
            document.text(),
            &document.hard_line_snapshot(),
            old,
            direction,
            pattern,
            count,
            self.search_options,
        ) {
            Ok(Some(destination)) => {
                self.cursor = destination;
                CommandOutput {
                    cursor_moved: destination != old,
                    ..CommandOutput::complete()
                }
            }
            Ok(None) => CommandOutput {
                status: CommandStatus::SearchNotFound,
                ..CommandOutput::complete()
            },
            Err(error) => CommandOutput {
                status: CommandStatus::Error(error),
                ..CommandOutput::complete()
            },
        }
    }

    fn move_cursor(&mut self, document: &Document, motion: Motion, count: usize) -> CommandOutput {
        self.typing_style = Default::default();
        let text = document.text();
        let lines = document.hard_line_snapshot();
        let old = self.cursor;
        let in_linear_visual = matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine);
        let retain_visual_line_end = in_linear_visual && self.visual_to_line_end;
        self.cursor = match motion {
            Motion::Horizontal(amount) => move_horizontal(
                text,
                &lines,
                self.cursor,
                directional_count(count, amount > 0),
            ),
            Motion::InsertionHorizontal(amount) => {
                let mut position = self.cursor;
                if amount > 0 {
                    for _ in 0..count {
                        let Some(next) = next_grapheme_boundary(text, position) else {
                            break;
                        };
                        position = next;
                    }
                } else {
                    for _ in 0..count {
                        let Some(previous) = previous_grapheme_boundary(text, position) else {
                            break;
                        };
                        position = previous;
                    }
                }
                position
            }
            Motion::Vertical(amount) => {
                let line_position = move_vertical(
                    text,
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                );
                if retain_visual_line_end {
                    self.preferred_column = None;
                    last_grapheme_on_line(text, &lines, line_position)
                } else {
                    let desired = self
                        .preferred_column
                        .unwrap_or_else(|| grapheme_column(text, &lines, self.cursor));
                    self.preferred_column = Some(desired);
                    position_at_column(text, &lines, line_start(&lines, line_position), desired)
                }
            }
            Motion::LineStart => line_start(&lines, self.cursor),
            Motion::InsertionLineStart => line_start(&lines, self.cursor),
            Motion::FirstNonBlank => first_non_blank(text, &lines, self.cursor),
            Motion::LineEnd => {
                let mut position = self.cursor;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, position) else {
                        break;
                    };
                    position = next;
                }
                last_grapheme_on_line(text, &lines, position)
            }
            Motion::InsertionLineEnd => line_end(&lines, self.cursor),
            Motion::WordForward(big) => normalize_normal_cursor(
                text,
                &lines,
                move_word_forward(text, self.cursor, big, count),
            ),
            Motion::WordEnd(big) => move_word_end(text, self.cursor, big, count),
            Motion::WordBackward(big) => move_word_backward(text, self.cursor, big, count),
            Motion::WordEndBackward(big) => move_word_end_backward(text, self.cursor, big, count),
            Motion::LastNonBlank => {
                let mut position = self.cursor;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, position) else {
                        break;
                    };
                    position = next;
                }
                last_non_blank(text, &lines, position)
            }
            Motion::Column(one_based) => position_at_column(
                text,
                &lines,
                line_start(&lines, self.cursor),
                one_based.saturating_sub(1),
            ),
            Motion::LineOffsetFirstNonBlank(amount) => {
                let line = move_vertical(
                    text,
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                );
                first_non_blank(text, &lines, line)
            }
            Motion::Sentence(forward) => normalize_normal_cursor(
                text,
                &lines,
                move_sentence(text, &lines, self.cursor, forward, count),
            ),
            Motion::Paragraph(forward) => normalize_normal_cursor(
                text,
                &lines,
                move_paragraph(&lines, self.cursor, forward, count),
            ),
        };
        if !matches!(motion, Motion::Vertical(_)) {
            self.preferred_column = None;
        }
        self.visual_to_line_end = in_linear_visual
            && (matches!(motion, Motion::LineEnd)
                || (matches!(motion, Motion::Vertical(_)) && retain_visual_line_end));
        CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        }
    }

    fn move_cursor_as_jump(
        &mut self,
        document: &Document,
        motion: Motion,
        count: usize,
    ) -> CommandOutput {
        let origin = self.cursor;
        let output = self.move_cursor(document, motion, count);
        self.record_successful_jump(document, origin, output)
    }

    fn goto_line(&mut self, document: &Document, one_based: usize) -> CommandOutput {
        let old = self.cursor;
        let lines = document.hard_line_snapshot();
        self.cursor = first_nonblank_document(document, &lines, nth_line_start(&lines, one_based));
        self.preferred_column = None;
        let output = CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, old, output)
    }

    fn push_count(&mut self, digit: char) -> CommandOutput {
        if self.count_overflowed {
            return CommandOutput::count_error(CountError::Overflow);
        }
        match push_decimal(self.count, digit) {
            Ok(count) => {
                self.count = Some(count);
                CommandOutput::pending()
            }
            Err(error) => {
                self.clear_pending();
                self.count_overflowed = true;
                CommandOutput::count_error(error)
            }
        }
    }

    fn push_operator_motion_count(
        &mut self,
        mut pending: PendingOperator,
        digit: char,
    ) -> CommandOutput {
        if self.count_overflowed {
            return CommandOutput::count_error(CountError::Overflow);
        }
        match push_decimal(pending.motion_count, digit) {
            Ok(count) => {
                pending.motion_count = Some(count);
                self.pending = Pending::Operator(pending);
                CommandOutput::pending()
            }
            Err(error) => {
                self.clear_pending();
                self.count_overflowed = true;
                CommandOutput::count_error(error)
            }
        }
    }

    fn finish_overflowed_count(&mut self) -> Option<CommandOutput> {
        if !self.count_overflowed {
            return None;
        }
        self.clear_pending();
        Some(CommandOutput::count_error(CountError::Overflow))
    }

    fn visual_command_is_pending(&self) -> bool {
        self.pending != Pending::None
    }

    fn clear_pending(&mut self) {
        self.clipboard_copy_as_seen = false;
        self.count = None;
        self.count_overflowed = false;
        self.register_pending = false;
        self.requested_register = None;
        self.pending = Pending::None;
    }
}

impl CommandPlan {
    /// Supporting HTML whitespace edits can change the UTF-8 length before a
    /// deletion boundary or a newly split paragraph. Resolve those cursors
    /// through the prepared transaction before publishing the controller.
    pub(crate) fn map_prepared_cursor(
        &mut self,
        document: &Document,
        prepared: &PreparedModelTransaction,
    ) -> Result<(), DocumentError> {
        if document.format() == crate::document::Format::MarkdownSource {
            if let Some(CommandModelRequest::FormattedPayload(request)) = self.model.as_ref() {
                if let [edit] = request.edits() {
                    self.success_controller.cursor = prepared_payload_caret(document, prepared, edit)?;
                }
            }
        }
        if document.format() != crate::document::Format::Html {
            return Ok(());
        }
        let target = match self.model.as_ref() {
            Some(CommandModelRequest::Model(ModelRequest::ApplyTextEdits { edits, .. }))
                if edits.iter().all(|edit| edit.replacement.is_empty()) =>
            {
                Some((self.success_controller.cursor, Association::BeforeInsertion))
            }
            Some(CommandModelRequest::Model(ModelRequest::ContinueList { at, .. })) => {
                self.success_controller.cursor = prepared_break_cursor(document, prepared, *at)?;
                return Ok(());
            }
            _ => None,
        };
        if let Some((at, association)) = target {
            self.success_controller.cursor = prepared_cursor(document, prepared, at, association)?;
        }
        Ok(())
    }

    pub(crate) fn prepare_model(
        &self,
        document: &Document,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.document != document.id() {
            return Err(ModelTransactionError::WrongDocument {
                expected: document.id(),
                actual: self.document,
            });
        }
        if self.revision != document.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: self.revision,
                actual: document.revision(),
            });
        }
        self.model
            .as_ref()
            .map(|request| request.prepare(document))
            .transpose()
    }

    pub(crate) fn publish_failure(self, interpreter: &mut CommandInterpreter, revision: Revision) {
        let mut failure = *self.failure_controller;
        failure.position_revision = Some(revision);
        *interpreter = failure;
    }

    pub(crate) fn publish_success(
        self,
        interpreter: &mut CommandInterpreter,
        document: &Document,
        document_changed: bool,
    ) -> CommandStep {
        let mut next = *self.success_controller;
        next.position_revision = Some(document.revision());
        match self.post_commit {
            PlannedPostCommit::None => {}
            PlannedPostCommit::ReplaceJournal(entries) => {
                next.finish_typing_caret(document)
                    .expect("committed typing ends at a valid UTF-8 boundary");
                if let Some(session) = next.insert_session.as_mut() {
                    let continues_frontier = session.replace_journal.last().map_or(true, |entry| {
                        entry.start.checked_add(entry.inserted.len())
                            == entries.first().map(|entry| entry.start)
                    });
                    let lines = document.hard_line_snapshot();
                    let has_legal_boundaries = entries.iter().all(|entry| {
                        lines.is_grapheme_boundary(entry.start)
                            && entry
                                .start
                                .checked_add(entry.inserted.len())
                                .is_some_and(|end| lines.is_grapheme_boundary(end))
                    });
                    if continues_frontier && has_legal_boundaries {
                        session.replace_journal.extend(entries);
                    } else {
                        session.replace_journal.clear();
                    }
                }
            }
            PlannedPostCommit::NormalizeNormalCursor => {
                next.cursor =
                    normalize_normal_cursor_document(document, &document.hard_line_snapshot(), next.cursor);
            }
            PlannedPostCommit::NormalizeTypingCursor => {
                next.finish_typing_caret(document)
                    .expect("committed typing ends at a valid UTF-8 boundary");
            }
        }
        let mut output = self.output;
        output.document_changed = document_changed;
        *interpreter = next;
        CommandStep {
            output,
            replay: self.replay,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowEdge {
    Start,
    FirstNonBlank,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertLayoutMotion {
    Rows(bool),
    Screen(ScreenMotion),
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Horizontal(isize),
    InsertionHorizontal(isize),
    Vertical(isize),
    LineStart,
    InsertionLineStart,
    FirstNonBlank,
    LineEnd,
    InsertionLineEnd,
    WordForward(bool),
    WordEnd(bool),
    WordBackward(bool),
    WordEndBackward(bool),
    LastNonBlank,
    Column(usize),
    LineOffsetFirstNonBlank(isize),
    Sentence(bool),
    Paragraph(bool),
}

fn layout_required(command: &str) -> CommandOutput {
    CommandOutput::unsupported(format!("{command} requires layout context"))
}

fn layout_error(error: LayoutMotionError) -> CommandOutput {
    let needs_more_layout = matches!(&error, LayoutMotionError::OutsideMaterializedCoverage(_));
    CommandOutput {
        status: if needs_more_layout {
            CommandStatus::NeedsMoreLayout(error)
        } else {
            CommandStatus::Error(format!("layout motion failed: {error:?}"))
        },
        ..CommandOutput::complete()
    }
}

fn visual_block_error(error: VisualBlockError) -> CommandOutput {
    CommandOutput {
        status: CommandStatus::VisualBlockError(error),
        ..CommandOutput::complete()
    }
}

fn block_register_value(document: &Document, resolved: &ResolvedBlockSelection, requested: Option<char>) -> RegisterValue {
    let text = document.text();
    let mut result = String::new();
    for (index, row) in resolved.rows.iter().enumerate() {
        if index > 0 {
            result.push('\n');
        }
        for range in &row.ranges {
            result.push_str(&text[range.clone()]);
        }
    }
    let mut value = RegisterValue::blockwise(result);
    if matches!(requested, Some('+' | '*')) && document.format() != crate::document::Format::PlainText {
        let rows = resolved.rows.iter().map(|row| row.ranges.clone()).collect::<Vec<_>>();
        value.clipboard_fragment = document.clipboard_fragment_rows(&rows).ok();
    }
    value
}

#[derive(Clone, Debug)]
struct StructuredFragment {
    text: String,
    hard_break_offsets: Vec<usize>,
}

impl StructuredFragment {
    fn literal(text: String) -> Self {
        Self {
            text,
            hard_break_offsets: Vec::new(),
        }
    }

    fn from_register(value: &RegisterValue) -> Self {
        Self {
            text: value.text.clone(),
            hard_break_offsets: value.hard_break_offsets().to_vec(),
        }
    }

    fn append(&mut self, other: Self) {
        let base = self.text.len();
        self.text.push_str(&other.text);
        self.hard_break_offsets.extend(
            other
                .hard_break_offsets
                .into_iter()
                .map(|offset| base + offset),
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisualPasteCursor {
    Start,
    FirstNonBlank { relative: usize },
    LastInsertedGrapheme,
}

fn fragment_ends_in_hard_break(fragment: &StructuredFragment) -> bool {
    fragment
        .text
        .len()
        .checked_sub(1)
        .is_some_and(|offset| fragment.hard_break_offsets.last() == Some(&offset))
}

fn ensure_fragment_trailing_hard_break(fragment: &mut StructuredFragment, required: bool) {
    let has_trailing = fragment_ends_in_hard_break(fragment);
    match (required, has_trailing) {
        (true, false) => {
            fragment.hard_break_offsets.push(fragment.text.len());
            fragment.text.push('\n');
        }
        (false, true) => {
            fragment.text.pop();
            fragment.hard_break_offsets.pop();
        }
        _ => {}
    }
}

fn linewise_fragment_for_character_selection(register: &RegisterValue) -> StructuredFragment {
    let mut register_fragment = StructuredFragment::from_register(register);
    ensure_fragment_trailing_hard_break(&mut register_fragment, true);
    let mut fragment = StructuredFragment {
        text: String::from("\n"),
        hard_break_offsets: vec![0],
    };
    fragment.append(register_fragment);
    fragment
}

fn fragment_for_linewise_selection(
    register: &RegisterValue,
    retain_trailing_hard_break: bool,
) -> StructuredFragment {
    let mut fragment = StructuredFragment::from_register(register);
    ensure_fragment_trailing_hard_break(&mut fragment, retain_trailing_hard_break);
    fragment
}

#[derive(Clone, Debug)]
struct PlannedFormattedEdit {
    range: Range<usize>,
    fragment: StructuredFragment,
}

impl PlannedFormattedEdit {
    fn literal(range: Range<usize>, text: String) -> Self {
        Self {
            range,
            fragment: StructuredFragment::literal(text),
        }
    }

    fn from_register(range: Range<usize>, value: &RegisterValue) -> Self {
        Self {
            range,
            fragment: StructuredFragment::from_register(value),
        }
    }
}

fn block_row_replacement_plans(
    resolved: &ResolvedBlockSelection,
    payloads: &[Option<StructuredFragment>],
) -> Vec<PlannedFormattedEdit> {
    debug_assert_eq!(resolved.rows.len(), payloads.len());
    let mut edits = Vec::new();
    for (row, payload) in resolved.rows.iter().zip(payloads) {
        let mut ranges = row.ranges.iter().filter(|range| !range.is_empty());
        if let Some(first) = ranges.next() {
            edits.push(PlannedFormattedEdit {
                range: first.clone(),
                fragment: payload
                    .clone()
                    .unwrap_or_else(|| StructuredFragment::literal(String::new())),
            });
            edits.extend(
                ranges.map(|range| PlannedFormattedEdit::literal(range.clone(), String::new())),
            );
        } else if let Some(payload) = payload.as_ref().filter(|payload| !payload.text.is_empty()) {
            let at = row.visual_left.point.text_offset;
            edits.push(PlannedFormattedEdit {
                range: at..at,
                fragment: payload.clone(),
            });
        }
    }
    edits
}

fn block_deletion_plans(resolved: &ResolvedBlockSelection) -> Vec<PlannedFormattedEdit> {
    resolved
        .range_set
        .segments
        .iter()
        .filter(|segment| !segment.range.is_empty())
        .map(|segment| PlannedFormattedEdit::literal(segment.range.clone(), String::new()))
        .collect()
}

fn appended_block_rows_plan(at: usize, rows: &[String]) -> PlannedFormattedEdit {
    let mut text = String::new();
    let mut hard_break_offsets = Vec::with_capacity(rows.len());
    for row in rows {
        hard_break_offsets.push(text.len());
        text.push('\n');
        text.push_str(row);
    }
    PlannedFormattedEdit {
        range: at..at,
        fragment: StructuredFragment {
            text,
            hard_break_offsets,
        },
    }
}

fn nearest_layout_caret_offset(row: &crate::layout::VisualRow, x: f32) -> Option<usize> {
    crate::layout::nearest_caret(row, x).map(|caret| caret.point.text_offset)
}

fn normal_block_insertion_origin(
    snapshot: &LayoutSnapshot,
    position: VisualPosition,
    before: bool,
) -> Option<(usize, usize, f32)> {
    let point = snapshot
        .caret_point(position.text_offset, position.affinity)
        .ok()?;
    let (row_index, row) = snapshot
        .rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.carets.iter().any(|caret| caret.point == point))?;
    let associated = row.clusters.iter().find(|cluster| match point.affinity {
        BoundaryAffinity::Downstream => {
            cluster.text_range.start <= point.text_offset
                && point.text_offset < cluster.text_range.end
        }
        BoundaryAffinity::Upstream => {
            cluster.text_range.start < point.text_offset
                && point.text_offset <= cluster.text_range.end
        }
    });
    if let Some(cluster) = associated {
        let logical_start_x = if cluster.bidi_level % 2 == 0 {
            cluster.x
        } else {
            cluster.x + cluster.advance
        };
        let logical_end_x = if cluster.bidi_level % 2 == 0 {
            cluster.x + cluster.advance
        } else {
            cluster.x
        };
        if before {
            Some((row_index, cluster.text_range.start, logical_start_x))
        } else {
            Some((row_index, cluster.text_range.end, logical_end_x))
        }
    } else {
        let caret = row.carets.iter().find(|caret| caret.point == point)?;
        Some((row_index, point.text_offset, caret.x))
    }
}

fn commit_planned_formatted_edits(
    commands: &CommandInterpreter,
    document: &mut Document,
    snapshot: &HardLineSnapshot,
    plans: Vec<PlannedFormattedEdit>,
    target: usize,
    association: Association,
) -> Result<usize, DocumentError> {
    document.validate_hard_line_snapshot(snapshot)?;
    let target_anchor = document
        .text_anchor(
            document.text_point(target)?,
            association,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .map_err(command_position_document_error)?;
    let mut insertions = BTreeMap::<usize, StructuredFragment>::new();
    let mut normalized = Vec::with_capacity(plans.len());
    for plan in plans {
        if plan.range.is_empty() {
            if plan.fragment.text.is_empty() {
                continue;
            }
            insertions
                .entry(plan.range.start)
                .and_modify(|fragment| fragment.append(plan.fragment.clone()))
                .or_insert(plan.fragment);
        } else {
            normalized.push(plan);
        }
    }
    normalized.extend(
        insertions
            .into_iter()
            .map(|(at, fragment)| PlannedFormattedEdit {
                range: at..at,
                fragment,
            }),
    );
    let edits = normalized
        .into_iter()
        .map(|plan| {
            let value = RegisterValue::try_new(
                plan.fragment.text, RegisterKind::Characterwise, plan.fragment.hard_break_offsets,
            ).expect("command payload plans contain validated break markers");
            let value = commands.assist_input_payload(
                document, plan.range.clone(), BoundaryAffinity::Downstream, &value,
            )?;
            let payload = FormattedTextPayload::new(
                snapshot, value.text.clone(), value.hard_break_offsets().to_vec(),
            )
            .expect("command payload plans contain validated break markers");
            Ok(FormattedPayloadEdit::new(plan.range, payload))
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    let (result, map) =
        document.capture_position_maps(|document| document.apply_formatted_payload_edits(edits));
    result?;
    let mapped = map
        .map_text_anchor(target_anchor)
        .expect("a command-local anchor matches the committed position map");
    Ok(mapped
        .value()
        .expect("the command target uses deletion recovery")
        .offset())
}

fn block_session_replacement_edits(rows: &[VisualBlockInsertRow], payload: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    for row in rows {
        let mut ranges = row.ranges.iter();
        if let Some(first) = ranges.next() {
            edits.push(TextEdit::new(first.clone(), payload));
            edits.extend(ranges.map(|range| TextEdit::new(range.clone(), "")));
        } else if !payload.is_empty() {
            edits.push(TextEdit::new(
                row.insertion_offset..row.insertion_offset,
                payload,
            ));
        }
    }
    merge_same_boundary_insertions(edits)
}

/// `Document::apply_edits` deliberately rejects two insertion patches at the
/// same logical boundary. Soft-wrap/bidi rows can share such a boundary, so
/// combine their row payloads deterministically before preparing the batch.
fn merge_same_boundary_insertions(edits: Vec<TextEdit>) -> Vec<TextEdit> {
    let mut insertions = BTreeMap::<usize, String>::new();
    let mut merged = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.range.is_empty() {
            if !edit.replacement.is_empty() {
                insertions
                    .entry(edit.range.start)
                    .or_default()
                    .push_str(&edit.replacement);
            }
        } else {
            merged.push(edit);
        }
    }
    merged.extend(
        insertions
            .into_iter()
            .map(|(at, replacement)| TextEdit::new(at..at, replacement)),
    );
    merged
}

fn document_char_at(document: &Document, at: usize) -> Option<char> {
    std::str::from_utf8(document.projection().text_tree().byte_chunk_at(at))
        .ok()?
        .chars()
        .next()
}

fn document_char_before(document: &Document, at: usize) -> Option<char> {
    if at == 0 {
        return None;
    }
    let tree = document.projection().text_tree();
    let mut start = at - 1;
    while start > 0
        && tree.byte_chunk_at(start).first().is_some_and(|b| b & 0xc0 == 0x80)
    {
        start -= 1;
    }
    tree.slice(start..at).ok()?.chars().next()
}

fn document_prefix_end(
    document: &Document,
    range: Range<usize>,
    predicate: impl Fn(char) -> bool,
) -> usize {
    let tree = document.projection().text_tree();
    let mut at = range.start;
    while at < range.end {
        let chunk = tree.byte_chunk_at(at);
        let chunk = &chunk[..chunk.len().min(range.end - at)];
        let text = std::str::from_utf8(chunk).expect("scalar-aligned formatted leaf");
        for c in text.chars() {
            if !predicate(c) {
                return at;
            }
            at += c.len_utf8();
        }
    }
    at
}

fn first_nonblank_document(document: &Document, lines: &HardLineSnapshot, at: usize) -> usize {
    let start = line_start(lines, at);
    let end = line_end(lines, at);
    let position = document_prefix_end(document, start..end, char::is_whitespace);
    if position == end {
        start
    } else if lines.is_grapheme_boundary(position) {
        position
    } else {
        lines.previous_grapheme_boundary(position).unwrap_or(start)
    }
}

/// A few command-local cursor calculations, including direct history replay,
/// refer to a byte ordinal from before the text changed. Preserve the old
/// Normal-mode flooring behavior without copying the document to a string.
fn normalize_normal_cursor_document(
    document: &Document,
    lines: &HardLineSnapshot,
    offset: usize,
) -> usize {
    let mut offset = offset.min(lines.text_length());
    if !lines.is_grapheme_boundary(offset) {
        while offset > 0
            && document.projection().text_tree().byte_chunk_at(offset)
                .first().is_some_and(|byte| byte & 0xc0 == 0x80)
        {
            offset -= 1;
        }
        if !lines.is_grapheme_boundary(offset) {
            offset = lines.previous_grapheme_boundary(offset).unwrap_or(0);
        }
    }
    normalize_normal_cursor_snapshot(lines, offset)
}

fn is_empty_row_at_position(snapshot: &LayoutSnapshot, position: VisualPosition) -> bool {
    snapshot.rows.iter().any(|row| {
        row.text_range.is_empty()
            && row.carets.iter().any(|caret| {
                caret.point.text_offset == position.text_offset
                    && caret.point.affinity == position.affinity
            })
    })
}

fn normal_offset_for_visual(
    text: &str,
    lines: &HardLineSnapshot,
    snapshot: &LayoutSnapshot,
    position: VisualPosition,
) -> usize {
    if position.affinity == BoundaryAffinity::Upstream
        && !is_empty_row_at_position(snapshot, position)
    {
        previous_grapheme_boundary(text, position.text_offset).unwrap_or(position.text_offset)
    } else {
        normalize_normal_cursor(text, lines, position.text_offset)
    }
}

fn layout_position_for_offset(
    snapshot: &LayoutSnapshot,
    text_offset: usize,
    preferred_affinity: BoundaryAffinity,
) -> Result<VisualPosition, LayoutMotionError> {
    for affinity in [
        preferred_affinity,
        match preferred_affinity {
            BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
        },
    ] {
        if snapshot.caret_point(text_offset, affinity).is_ok() {
            return Ok(VisualPosition {
                text_offset,
                affinity,
            });
        }
    }
    let candidate = VisualPosition {
        text_offset,
        affinity: preferred_affinity,
    };
    match g0(snapshot, candidate) {
        Err(error) => Err(error),
        Ok(_) => Err(LayoutMotionError::PositionNotInLayout(candidate)),
    }
}

fn visual_block_anchor_row(
    snapshot: &LayoutSnapshot,
    endpoint: VisualBlockEndpoint,
    anchor: TextAnchor,
) -> Result<Option<usize>, VisualBlockRebindError> {
    if !snapshot.coverage.contains_text_offset(anchor.offset()) {
        return Ok(None);
    }
    if let Some(row) = snapshot.rows.iter().position(|row| {
        row.carets.iter().any(|caret| {
            caret.point.text_offset == anchor.offset() && caret.point.affinity == anchor.affinity()
        })
    }) {
        return Ok(Some(row));
    }
    match snapshot.logical_endpoint_geometry(anchor.offset(), anchor.affinity()) {
        Ok(geometry) => Ok(Some(geometry.row_index)),
        Err(LayoutError::OutsideMaterializedCoverage) => Ok(None),
        Err(_) => Err(VisualBlockRebindError::EndpointNotInLayout {
            endpoint,
            offset: anchor.offset(),
            affinity: anchor.affinity(),
        }),
    }
}

fn active_visual_block_from_selection(
    document: &Document,
    selection: &BlockSelection,
) -> Result<ActiveVisualBlock, DocumentError> {
    let (anchor_association, active_association) =
        if selection.anchor.text_offset <= selection.active.text_offset {
            (Association::AfterInsertion, Association::BeforeInsertion)
        } else {
            (Association::BeforeInsertion, Association::AfterInsertion)
        };
    Ok(ActiveVisualBlock {
        anchor: document
            .text_anchor(
                document.text_point(selection.anchor.text_offset)?,
                anchor_association,
                selection.anchor.affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .map_err(command_position_document_error)?,
        active: document
            .text_anchor(
                document.text_point(selection.active.text_offset)?,
                active_association,
                selection.active.affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .map_err(command_position_document_error)?,
        anchor_x: selection.anchor_x,
        active_x: selection.active_x,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperatorMotion {
    Left,
    Right,
    Down,
    Up,
    LineStart,
    FirstNonBlank,
    LineEnd,
    Column,
    WordForward(bool),
    WordEnd(bool),
    WordBackward(bool),
    WordEndBackward(bool),
    LastNonBlank,
    Sentence(bool),
    Paragraph(bool),
    MatchPair,
    Percentage,
    LastLine,
    FirstLine,
}

fn operator_motion_for_key(key: Key) -> Option<OperatorMotion> {
    match key {
        Key::Char('h') | Key::Left | Key::Backspace => Some(OperatorMotion::Left),
        Key::Char('l') | Key::Right | Key::Char(' ') => Some(OperatorMotion::Right),
        Key::Char('j') | Key::Down | Key::Enter => Some(OperatorMotion::Down),
        Key::Char('+') => Some(OperatorMotion::Down),
        Key::Char('k') | Key::Up => Some(OperatorMotion::Up),
        Key::Char('-') => Some(OperatorMotion::Up),
        Key::Char('0') | Key::Home => Some(OperatorMotion::LineStart),
        Key::Char('^') => Some(OperatorMotion::FirstNonBlank),
        Key::Char('$') | Key::End => Some(OperatorMotion::LineEnd),
        Key::Char('|') => Some(OperatorMotion::Column),
        Key::Char('w') => Some(OperatorMotion::WordForward(false)),
        Key::Char('W') => Some(OperatorMotion::WordForward(true)),
        Key::Char('e') => Some(OperatorMotion::WordEnd(false)),
        Key::Char('E') => Some(OperatorMotion::WordEnd(true)),
        Key::Char('b') => Some(OperatorMotion::WordBackward(false)),
        Key::Char('B') => Some(OperatorMotion::WordBackward(true)),
        Key::Char('(') => Some(OperatorMotion::Sentence(false)),
        Key::Char(')') => Some(OperatorMotion::Sentence(true)),
        Key::Char('{') => Some(OperatorMotion::Paragraph(false)),
        Key::Char('}') => Some(OperatorMotion::Paragraph(true)),
        Key::Char('%') => Some(OperatorMotion::MatchPair),
        Key::Char('G') => Some(OperatorMotion::LastLine),
        _ => None,
    }
}

fn effective_operator_count(pending: PendingOperator) -> Result<usize, CountError> {
    pending
        .operator_count
        .checked_mul(pending.motion_count.unwrap_or(1))
        .map(|count| count.max(1))
        .ok_or(CountError::Overflow)
}

fn operator_mark_extent(
    text: &str,
    lines: &HardLineSnapshot,
    origin: usize,
    marked: usize,
    linewise: bool,
) -> (MotionExtent, usize) {
    let destination = if linewise {
        first_non_blank(text, lines, marked)
    } else {
        normalize_normal_cursor(text, lines, marked)
    };
    if linewise {
        let first = line_start(lines, origin.min(destination));
        let last = line_start(lines, origin.max(destination));
        (
            MotionExtent {
                range: first..line_range(lines, last).end,
                kind: MotionKind::Linewise,
            },
            destination,
        )
    } else {
        (
            exclusive_motion_extent(text, lines, origin, destination),
            destination,
        )
    }
}

/// Resolve Vim's exclusive motion rules into one half-open operator extent.
/// A forward motion to column zero excludes the newline immediately before
/// the destination unless the start-of-line exception promotes it to a
/// linewise motion. Backward exclusive motions use the ordinary ordered span.
fn exclusive_motion_extent(
    text: &str,
    lines: &HardLineSnapshot,
    origin: usize,
    destination: usize,
) -> MotionExtent {
    if destination > origin && destination == line_start(lines, destination) {
        if origin <= first_non_blank(text, lines, origin) {
            return MotionExtent {
                range: line_start(lines, origin)..destination,
                kind: MotionKind::Linewise,
            };
        }
        return MotionExtent {
            range: origin..previous_grapheme_boundary(text, destination).unwrap_or(destination),
            kind: MotionKind::Characterwise,
        };
    }
    MotionExtent {
        range: origin.min(destination)..origin.max(destination),
        kind: MotionKind::Characterwise,
    }
}

fn search_destination(
    text: &str,
    lines: &HardLineSnapshot,
    origin: usize,
    direction: SearchDirection,
    pattern: &str,
    count: usize,
    options: regex_v1::SearchOptions,
) -> Result<Option<usize>, String> {
    use regex_v1::{CompiledRegex, RegexInput, RegexLimits, RegexWork};
    let limits = RegexLimits::default();
    let regex = CompiledRegex::compile(
        pattern,
        options
            .case_insensitive(pattern)
            .map_err(|e| e.to_string())?,
        limits,
    )
    .map_err(|e| e.to_string())?;
    let input = RegexInput::new(lines);
    let mut work = RegexWork::new(limits);
    let mut cursor = origin;
    let mut completed = 0;
    let mut seen = HashMap::new();
    while completed < count.max(1) {
        if let Some(previous) = seen.insert(cursor, completed) {
            let cycle = completed - previous;
            let skip = (count.max(1) - completed) / cycle;
            if skip > 0 {
                completed += skip * cycle;
                continue;
            }
        }
        let mut seek = |start: usize, before: Option<usize>| -> Result<Option<usize>, String> {
            let mut at = start;
            let mut last = None;
            while at <= text.len() {
                let Some(matched) = regex
                    .find(&input, at, text.len(), &mut work)
                    .map_err(|e| e.to_string())?
                else {
                    break;
                };
                let found = matched.range().start;
                if before.is_some_and(|limit| found >= limit) {
                    break;
                }
                if lines.is_grapheme_boundary(found) {
                    if before.is_none() {
                        return Ok(Some(found));
                    }
                    last = Some(found);
                }
                let Some(next) = lines.next_grapheme_boundary(found) else {
                    break;
                };
                at = next;
            }
            Ok(last)
        };
        let found = match direction {
            SearchDirection::Forward => {
                let found = if let Some(start) = lines.next_grapheme_boundary(cursor) {
                    seek(start, None)?
                } else {
                    None
                };
                if found.is_none() && options.wrapscan {
                    seek(0, None)?
                } else {
                    found
                }
            }
            SearchDirection::Backward => {
                let found = seek(0, Some(cursor))?;
                if found.is_none() && options.wrapscan {
                    seek(0, Some(text.len().saturating_add(1)))?
                } else {
                    found
                }
            }
        };
        let Some(found) = found else {
            return Ok(None);
        };
        cursor = found;
        completed += 1;
    }
    Ok(Some(cursor))
}

fn percentage_line(lines: &HardLineSnapshot, percent: usize) -> Option<usize> {
    (1..=100).contains(&percent).then(|| {
        line_count(lines)
            .saturating_mul(percent)
            .div_ceil(100)
            .max(1)
    })
}

fn invalid_percentage(percent: usize) -> CommandOutput {
    CommandOutput {
        status: CommandStatus::Error(format!(
            "percentage must be between 1 and 100, got {percent}"
        )),
        ..CommandOutput::complete()
    }
}

fn push_decimal(current: Option<usize>, digit: char) -> Result<usize, CountError> {
    let digit = digit.to_digit(10).ok_or(CountError::Overflow)? as usize;
    current
        .unwrap_or(0)
        .checked_mul(10)
        .and_then(|value| value.checked_add(digit))
        .ok_or(CountError::Overflow)
}

fn command_document_error(error: ModelTransactionError) -> DocumentError {
    match error {
        ModelTransactionError::Document(error) => error,
        ModelTransactionError::WrongDocument { .. } => DocumentError::WrongDocument,
        ModelTransactionError::StaleRevision { expected, actual } => DocumentError::WrongSnapshot {
            expected: actual,
            actual: expected,
        },
        ModelTransactionError::Style(_) => DocumentError::UnsupportedFormatting,
        // The compatibility interpreter's public error type predates typed
        // transaction failures. Planned coordinator execution preserves those
        // through `CoreError::ModelTransaction`; this legacy bridge can only
        // report a generic verification failure.
        _ => DocumentError::VerificationFailed,
    }
}

fn command_position_document_error(error: PositionError) -> DocumentError {
    match error {
        PositionError::WrongDocument { .. } => DocumentError::WrongDocument,
        PositionError::WrongSnapshot { expected, actual } => {
            DocumentError::WrongSnapshot { expected, actual }
        }
        PositionError::InvalidUnicodeBoundary { offset } => {
            DocumentError::NotGraphemeBoundary(offset)
        }
        PositionError::InvalidBoundary { offset, length, .. } => DocumentError::InvalidRange {
            start: offset,
            end: offset,
            length,
        },
        PositionError::WrongDomain { .. }
        | PositionError::WrongSourcePart { .. }
        | PositionError::InvertedRange { .. }
        | PositionError::OverlappingSplices { .. }
        | PositionError::TargetLengthMismatch { .. }
        | PositionError::MapChainLengthMismatch { .. }
        | PositionError::ArithmeticOverflow => DocumentError::VerificationFailed,
    }
}

fn directional_count(count: usize, positive: bool) -> isize {
    let magnitude = isize::try_from(count.max(1)).unwrap_or(isize::MAX);
    if positive {
        magnitude
    } else {
        -magnitude
    }
}

fn covered_line_range(lines: &HardLineSnapshot, range: Range<usize>) -> Range<usize> {
    let start = line_start(lines, range.start);
    let end_line = if range.end > range.start {
        previous_grapheme_boundary(lines.text(), range.end).unwrap_or(range.end)
    } else {
        range.end
    };
    start..line_range(lines, end_line).end
}

fn linewise_edit_range(lines: &HardLineSnapshot, range: Range<usize>) -> Range<usize> {
    if range.end != lines.text_length() || range.start == 0 {
        return range;
    }
    let first = lines
        .line_at_offset(range.start)
        .expect("a linewise edit starts at a valid hard-line boundary");
    let Some(previous) = first
        .index()
        .checked_sub(1)
        .and_then(|index| lines.line(index))
    else {
        return range;
    };
    previous
        .separator_range()
        .map_or(range.clone(), |separator| separator.start..range.end)
}

fn remove_typing_inserted_suffix(
    format: crate::document::Format,
    value: &mut RegisterValue,
    removed: &str,
) {
    if format == crate::document::Format::Html {
        // Repeat stores the user's input. The HTML typing adapter may spell
        // those spaces as NBSP or turn a tab into a displayed ordinary space.
        // Compare that local authored frontier by scalar, retaining its own
        // byte coordinates when truncating the repeat register.
        let mut authored = value.text.char_indices().rev();
        let mut start = value.text.len();
        let matches = removed.chars().rev().all(|actual| {
            let Some((at, typed)) = authored.next() else { return false; };
            start = at;
            typed == actual
                || (matches!(typed, ' ' | '\t' | '\r')
                    && matches!(actual, ' ' | '\u{a0}'))
        });
        if matches {
            value.truncate_inserted_payload(start);
            return;
        }
    }
    remove_inserted_suffix(value, removed);
}

fn remove_inserted_suffix(value: &mut RegisterValue, removed: &str) {
    if let Some(new_length) = value.text.len().checked_sub(removed.len()) {
        if value.text.get(new_length..) == Some(removed) {
            value.truncate_inserted_payload(new_length);
            return;
        }
    }
    // A deletion which cannot be described as removing the active inserted
    // frontier ends that frontier. Later inserted text becomes the new `.`
    // candidate after the caller's undo break.
    *value = RegisterValue::characterwise("");
}

/// Resolve the complete replacement against its input snapshot before any
/// mutation, so one text event remains atomic even if later input is rejected.
fn replacement_payload_targets(
    document: &Document,
    lines: &HardLineSnapshot,
    start: usize,
    value: &RegisterValue,
    journalable: bool,
) -> (usize, Vec<ReplaceJournalEntry>) {
    let mut end = start;
    let mut journal_entries = Vec::new();
    for (relative, inserted) in value.text.grapheme_indices(true) {
        // Semantic breaks insert without consuming the next grapheme. Literal
        // LF follows the same replacement rules as any other source character.
        if inserted == "\n" && value.hard_break_offsets().binary_search(&relative).is_ok() {
            continue;
        }
        let replaced = grapheme_range_at(document.text(), end)
            .filter(|range| !is_hard_line_separator(lines, range));
        if journalable {
            journal_entries.push(ReplaceJournalEntry {
                source_record: None,
                start: start + relative,
                inserted: inserted.to_owned(),
                original: replaced.as_ref().map(|range| document.text()[range.clone()].to_owned()),
            });
        }
        if let Some(range) = replaced {
            end = range.end;
        }
    }
    (end, journal_entries)
}

fn replacement_payload_end(
    lines: &HardLineSnapshot,
    start: usize,
    value: &RegisterValue,
) -> usize {
    let mut end = start;
    for (relative, inserted) in value.text.grapheme_indices(true) {
        let semantic_break =
            inserted == "\n" && value.hard_break_offsets().binary_search(&relative).is_ok();
        if semantic_break {
            continue;
        }
        let Some(range) = lines.grapheme_range_at(end) else {
            continue;
        };
        if !is_hard_line_separator(lines, &range) {
            end = range.end;
        }
    }
    end
}

fn prepared_cursor(
    document: &Document,
    prepared: &PreparedModelTransaction,
    at: usize,
    association: Association,
) -> Result<usize, DocumentError> {
    let offset = prepared
        .text_position_map()
        .map_text_point(
            document.text_point(at)?,
            association,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .map_err(command_position_document_error)?
        .value()
        .ok_or(DocumentError::AmbiguousProjection)?
        .offset();
    Ok(document.prepared_text_point(prepared, offset)?.offset())
}

fn commit_model_with_cursor(
    document: &mut Document,
    request: ModelRequest,
    at: usize,
    association: Association,
) -> Result<usize, DocumentError> {
    let prepared = document
        .prepare_model_request(request)
        .map_err(command_document_error)?;
    let caret = prepared_cursor(document, &prepared, at, association)?;
    document
        .commit_model_transaction(prepared)
        .map_err(command_document_error)?;
    Ok(caret)
}

fn prepared_break_cursor(
    document: &Document,
    prepared: &PreparedModelTransaction,
    at: usize,
) -> Result<usize, DocumentError> {
    let start = prepared_cursor(document, prepared, at, Association::BeforeInsertion)?;
    // A protected space immediately after the break shares the old insertion
    // boundary. AfterInsertion would also cross that supporting replacement.
    let inserted = prepared
        .summary()
        .formatted_splices()
        .iter()
        .find(|splice| splice.old_range() == (at..at))
        .map_or(0, |splice| splice.inserted_len());
    let cursor = start
        .checked_add(inserted)
        .ok_or(DocumentError::AmbiguousProjection)?;
    Ok(document.prepared_text_point(prepared, cursor)?.offset())
}

fn continue_list_with_cursor(document: &mut Document, at: usize) -> Result<usize, DocumentError> {
    let prepared = document
        .prepare_model_request(ModelRequest::ContinueList {
            document: document.id(),
            revision: document.revision(),
            at,
        })
        .map_err(command_document_error)?;
    let cursor = if document.format() == crate::document::Format::Html {
        prepared_break_cursor(document, &prepared, at)?
    } else {
        prepared_cursor(document, &prepared, at, Association::AfterInsertion)?
    };
    document
        .commit_model_transaction(prepared)
        .map_err(command_document_error)?;
    Ok(cursor)
}

fn prepare_open_line_with_cursor(
    document: &Document,
    at: usize,
    origin: usize,
    after: bool,
) -> Result<(PreparedModelTransaction, usize), DocumentError> {
    let prepared = document
        .prepare_model_request(ModelRequest::OpenLine {
            document: document.id(),
            revision: document.revision(),
            at,
            origin,
            after,
        })
        .map_err(command_document_error)?;
    // Opening a wrapped HTML row can also protect adjacent whitespace. Resolve
    // the caret before committing, including the UTF-8 growth of those spaces.
    // Cross only the inserted break for o; a following supporting replacement
    // must not advance the caret past the newly opened row.
    let cursor = if document.format() == crate::document::Format::Html {
        if after {
            prepared_break_cursor(document, &prepared, at)?
        } else {
            prepared_cursor(document, &prepared, at, Association::BeforeInsertion)?
        }
    } else {
        at + usize::from(after)
    };
    document.prepared_text_point(&prepared, cursor)?;
    Ok((prepared, cursor))
}

fn normal_delete_range(
    lines: &HardLineSnapshot,
    cursor: usize,
    count: usize,
    backward: bool,
) -> Range<usize> {
    if backward {
        let mut start = cursor;
        let floor = line_start(lines, cursor);
        for _ in 0..count {
            match lines.previous_grapheme_boundary(start) {
                Some(previous) if previous >= floor => start = previous,
                _ => break,
            }
        }
        start..cursor
    } else {
        let end = line_end(lines, cursor);
        cursor..lines.advance_graphemes(cursor, count).unwrap_or(end).min(end)
    }
}

fn delete_with_cursor(document: &mut Document, range: Range<usize>) -> Result<usize, DocumentError> {
    let at = range.start;
    commit_model_with_cursor(
        document,
        ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(range, "")],
        },
        at,
        Association::BeforeInsertion,
    )
}

fn prepared_payload_caret(
    document: &Document,
    prepared: &PreparedModelTransaction,
    edit: &FormattedPayloadEdit,
) -> Result<usize, DocumentError> {
    let exact_splice = matches!(prepared.summary().formatted_splices(), [splice]
        if splice.old_range() == edit.range() && splice.inserted_len() == edit.payload().text().len());
    if document.format() == crate::document::Format::MarkdownSource
        && !prepared.is_no_op() && !exact_splice
    {
        prepared_cursor(document, prepared, edit.range().end, Association::AfterInsertion)
    } else {
        Ok(edit.range().start + edit.payload().text().len())
    }
}

fn commit_typing_payload(
    document: &mut Document,
    edit: FormattedPayloadEdit,
) -> Result<usize, DocumentError> {
    // This is the authored insertion boundary, before grapheme normalization.
    // A grapheme-closed map can also include unchanged suffix characters (for
    // example regional indicators), so mapping its endpoint would skip them.
    let request = FormattedPayloadEditRequest::new(document.id(), document.revision(), vec![edit.clone()]);
    let prepared = document
        .prepare_formatted_payload_request(request)
        .map_err(command_document_error)?;
    let caret = prepared_payload_caret(document, &prepared, &edit)?;
    document
        .commit_model_transaction(prepared)
        .map_err(command_document_error)?;
    Ok(caret)
}

/// Text delivered as one frontend text event is literal formatted content.
/// A literal LF is representable in a Mac-line-ending source, where CR is the
/// delimiter. Unix and DOS source projections necessarily spell an inserted
/// formatted LF as a semantic source-line break; leaving it unmarked would
/// make the reverse edit fail its re-projection check.
fn external_text_register_value(document: &Document, text: &str) -> RegisterValue {
    if document.file_format() == FileFormat::Mac {
        RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
            .expect("literal input text has valid empty hard-break metadata")
    } else {
        RegisterValue::characterwise(text)
    }
}

fn register_value(document: &Document, lines: &HardLineSnapshot, extent: &MotionExtent, requested: Option<char>) -> RegisterValue {
    let captured = lines
        .capture(extent.range.clone())
        .expect("a resolved operator extent uses valid grapheme boundaries");
    let mut contents = captured.text().to_owned();
    let mut hard_break_offsets = captured.break_offsets().to_vec();
    let mut result = if extent.kind == MotionKind::Linewise {
        let ends_with_hard_break = hard_break_offsets
            .last()
            .is_some_and(|offset| offset.saturating_add(1) == contents.len());
        if !ends_with_hard_break {
            hard_break_offsets.push(contents.len());
            contents.push('\n');
        }
        RegisterValue::try_new(contents, RegisterKind::Linewise, hard_break_offsets)
            .expect("captured and synthetic linewise register breaks are valid")
    } else {
        RegisterValue::try_new(contents, RegisterKind::Characterwise, hard_break_offsets)
            .expect("captured characterwise register breaks are valid")
    };
    if matches!(requested, Some('+' | '*')) && document.format() != crate::document::Format::PlainText {
        result.clipboard_fragment = document.clipboard_fragment(extent.range.clone()).ok().map(|fragment| {
            fragment.with_register(&result.text, match result.kind { RegisterKind::Characterwise => 1, RegisterKind::Linewise => 2, RegisterKind::Blockwise => 3 }, result.hard_break_offsets())
        });
    }
    result
}

fn yank_cursor_after_motion(
    text: &str,
    lines: &HardLineSnapshot,
    origin: usize,
    extent: &MotionExtent,
) -> usize {
    if extent.kind != MotionKind::Linewise {
        return extent.range.start;
    }

    let origin_line = line_start(lines, origin);
    if extent.range.start >= origin_line {
        // A doubled or forward linewise yank leaves the cursor exactly where
        // it started, including its nonzero column.
        return origin;
    }

    // A backward linewise motion lands on the destination line while
    // preserving the origin's logical column, clamped for a shorter line.
    position_at_column(
        text,
        lines,
        extent.range.start,
        grapheme_column(text, lines, origin),
    )
}

fn ordinary_deletion_class(lines: &HardLineSnapshot, extent: &MotionExtent) -> DeletionClass {
    if extent.kind == MotionKind::Linewise || range_contains_hard_break(lines, &extent.range) {
        DeletionClass::Large
    } else {
        DeletionClass::Small
    }
}

fn deletion_class_for_target(
    lines: &HardLineSnapshot,
    extent: &MotionExtent,
    target: &RepeatTarget,
) -> DeletionClass {
    let within_line = extent.kind == MotionKind::Characterwise
        && !range_contains_hard_break(lines, &extent.range);
    let exceptional = matches!(
        target,
        RepeatTarget::Motion(
            OperatorMotion::Sentence(_)
                | OperatorMotion::Paragraph(_)
                | OperatorMotion::MatchPair
                | OperatorMotion::Percentage
        )
    ) || matches!(
        target,
        RepeatTarget::Mark {
            linewise: false,
            ..
        } | RepeatTarget::Search(_)
    );
    if exceptional {
        DeletionClass::Exceptional { within_line }
    } else if within_line {
        DeletionClass::Small
    } else {
        DeletionClass::Large
    }
}

fn visual_block_deletion_class(row_count: usize) -> DeletionClass {
    if row_count <= 1 {
        DeletionClass::Small
    } else {
        DeletionClass::Large
    }
}

fn range_contains_hard_break(lines: &HardLineSnapshot, range: &Range<usize>) -> bool {
    !lines
        .capture(range.clone())
        .expect("operator extents use valid grapheme boundaries")
        .break_offsets()
        .is_empty()
}

fn is_hard_line_separator(lines: &HardLineSnapshot, range: &Range<usize>) -> bool {
    lines
        .line_at_offset(range.start)
        .ok()
        .and_then(|line| line.separator_range())
        .is_some_and(|separator| separator == *range)
}

fn hard_line_count_for_range(lines: &HardLineSnapshot, range: &Range<usize>) -> usize {
    if range.is_empty() {
        return 1;
    }
    let first = lines
        .line_at_offset(range.start)
        .expect("a command range starts at a valid hard-line boundary")
        .index();
    let last_offset = previous_grapheme_boundary(lines.text(), range.end).unwrap_or(range.start);
    let last = lines
        .line_at_offset(last_offset)
        .expect("a command range ends at a valid hard-line boundary")
        .index();
    last.saturating_sub(first).saturating_add(1)
}

fn indent_edits(
    text: &str,
    lines: &HardLineSnapshot,
    range: &Range<usize>,
    operator: Operator,
    count: usize,
) -> Result<Vec<TextEdit>, TextRepetitionError> {
    let shift_width = 4usize
        .checked_mul(count)
        .ok_or_else(|| TextRepetitionError::new(count))?;
    let padding = if operator == Operator::Indent {
        Some(checked_text_repetition("    ", count)?)
    } else {
        None
    };
    let first = lines
        .line_at_offset(range.start)
        .expect("an indent range starts on a hard line")
        .index();
    let last_offset = previous_grapheme_boundary(text, range.end).unwrap_or(range.start);
    let last = lines
        .line_at_offset(last_offset)
        .expect("an indent range ends on a hard line")
        .index();
    let mut edits = Vec::with_capacity(last.saturating_sub(first).saturating_add(1));
    for info in lines
        .lines(first..last.saturating_add(1))
        .expect("an ordinal range derived from one snapshot is valid")
    {
        let content = info.content_range();
        let line = &text[content.clone()];
        match operator {
            Operator::Indent => {
                edits.push(TextEdit::new(
                    content.start..content.start,
                    padding
                        .as_ref()
                        .expect("indent precomputes its checked padding")
                        .clone(),
                ));
            }
            Operator::Outdent => {
                let spaces = line
                    .bytes()
                    .take_while(|byte| *byte == b' ')
                    .take(shift_width)
                    .count();
                if spaces > 0 {
                    edits.push(TextEdit::new(content.start..content.start + spaces, ""));
                }
            }
            Operator::Reindent => {
                let trimmed = line.trim_start();
                let whitespace = line.len() - trimmed.len();
                if whitespace > 0 {
                    edits.push(TextEdit::new(content.start..content.start + whitespace, ""));
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(edits)
}

fn change_case(text: &str, operator: Operator) -> String {
    match operator {
        Operator::Lowercase => text.to_lowercase(),
        Operator::Uppercase => text.to_uppercase(),
        Operator::ToggleCase => text
            .chars()
            .flat_map(|character| {
                if character.is_lowercase() {
                    character.to_uppercase().collect::<Vec<_>>()
                } else {
                    character.to_lowercase().collect::<Vec<_>>()
                }
            })
            .collect(),
        _ => unreachable!(),
    }
}

fn choose_repeat_count(requested: Option<usize>, recorded: usize) -> usize {
    requested.unwrap_or(recorded).max(1)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TextRepetitionError {
    count: usize,
}

impl TextRepetitionError {
    fn new(count: usize) -> Self {
        Self { count }
    }

    fn into_command_output(self) -> CommandOutput {
        CommandOutput {
            status: CommandStatus::Error(format!(
                "count {} would create text too large to represent",
                self.count
            )),
            ..CommandOutput::complete()
        }
    }
}

fn checked_text_repetition(input: &str, count: usize) -> Result<String, TextRepetitionError> {
    let Some(capacity) = input.len().checked_mul(count) else {
        return Err(TextRepetitionError::new(count));
    };
    if capacity == 0 {
        return Ok(String::new());
    }
    let mut repeated = String::new();
    if repeated.try_reserve_exact(capacity).is_err() {
        return Err(TextRepetitionError::new(count));
    }
    for _ in 0..count {
        repeated.push_str(input);
    }
    Ok(repeated)
}

fn one_extended_grapheme(input: &str) -> Option<&str> {
    let mut graphemes = input.graphemes(true);
    let grapheme = graphemes.next()?;
    graphemes.next().is_none().then_some(grapheme)
}

fn is_single_semantic_hard_break(value: &RegisterValue) -> bool {
    value.kind == RegisterKind::Characterwise
        && value.text == "\n"
        && value.hard_break_offsets() == [0]
}

/// Retain the last-insert register's original one-input-unit convention even
/// when a counted command expanded and assisted that unit at its destination.
fn inserted_input_unit(inserted: &RegisterValue, input: &str) -> RegisterValue {
    let end = inserted.text.grapheme_indices(true)
        .nth(input.graphemes(true).count())
        .map_or(inserted.text.len(), |(at, _)| at);
    let mut unit = inserted.clone();
    unit.truncate_inserted_payload(end);
    unit
}

fn checked_register_repetition(
    input: &RegisterValue,
    count: usize,
) -> Result<RegisterValue, TextRepetitionError> {
    if count == 1 { return Ok(input.clone()); }
    if input.text.is_empty() && input.hard_break_offsets().is_empty() {
        return Ok(input.clone());
    }
    let text = checked_text_repetition(&input.text, count)?;
    let Some(marker_capacity) = input.hard_break_offsets().len().checked_mul(count) else {
        return Err(TextRepetitionError::new(count));
    };
    let mut hard_break_offsets = Vec::new();
    if hard_break_offsets
        .try_reserve_exact(marker_capacity)
        .is_err()
    {
        return Err(TextRepetitionError::new(count));
    }
    for repetition in 0..count {
        let Some(base) = input.text.len().checked_mul(repetition) else {
            return Err(TextRepetitionError::new(count));
        };
        for &offset in input.hard_break_offsets() {
            let Some(offset) = base.checked_add(offset) else {
                return Err(TextRepetitionError::new(count));
            };
            hard_break_offsets.push(offset);
        }
    }
    Ok(RegisterValue::try_new(text, input.kind, hard_break_offsets)
        .expect("repeating validated register contents preserves break offsets"))
}

fn checked_block_register_rows(
    input: &RegisterValue,
    count: usize,
) -> Result<Vec<String>, TextRepetitionError> {
    debug_assert_eq!(input.kind, RegisterKind::Blockwise);
    input
        .text
        .split('\n')
        .map(|row| checked_text_repetition(row, count))
        .collect()
}

fn linewise_register_at_eof(
    input: RegisterValue,
    count: usize,
) -> Result<RegisterValue, TextRepetitionError> {
    let mut content_end = input.text.len();
    let mut hard_break_offsets = input.hard_break_offsets().to_vec();
    if content_end
        .checked_sub(1)
        .is_some_and(|offset| input.is_hard_break(offset))
    {
        content_end -= 1;
        hard_break_offsets.pop();
    }
    let Some(capacity) = content_end.checked_add(1) else {
        return Err(TextRepetitionError::new(count));
    };
    let mut output = String::new();
    if output.try_reserve_exact(capacity).is_err() {
        return Err(TextRepetitionError::new(count));
    }
    output.push('\n');
    output.push_str(&input.text[..content_end]);
    for offset in &mut hard_break_offsets {
        *offset = offset
            .checked_add(1)
            .ok_or_else(|| TextRepetitionError::new(count))?;
    }
    hard_break_offsets.insert(0, 0);
    Ok(
        RegisterValue::try_new(output, input.kind, hard_break_offsets)
            .expect("prefixing a semantic hard break preserves register validity"),
    )
}

fn repetition_too_large(count: usize) -> CommandOutput {
    TextRepetitionError::new(count).into_command_output()
}

fn mapped_anchor(map: &PositionMap, anchor: TextAnchor) -> Result<Option<usize>, PositionError> {
    Ok(map
        .map_text_anchor(anchor)?
        .value()
        .copied()
        .map(TextAnchor::offset))
}

fn map_visual_block_anchor(
    map: &PositionMap,
    endpoint: VisualBlockEndpoint,
    anchor: TextAnchor,
) -> Result<Result<TextAnchor, VisualBlockRebindError>, PositionError> {
    Ok(match map.map_text_anchor(anchor)? {
        MappingOutcome::Exact(anchor)
        | MappingOutcome::Moved(anchor)
        | MappingOutcome::CollapsedByDeletion(anchor)
        | MappingOutcome::RecoveredFromProvenance(anchor) => Ok(anchor),
        MappingOutcome::Ambiguous(candidates) => Err(VisualBlockRebindError::AmbiguousAnchor {
            endpoint,
            candidate_count: candidates.len(),
        }),
        MappingOutcome::Unresolvable(reason) => {
            Err(VisualBlockRebindError::UnresolvableAnchor { endpoint, reason })
        }
    })
}

fn block_id_at_hard_line_start(document: &Document, start: usize) -> Option<u64> {
    let line = document.hard_line_at_offset(start)?;
    document.projection().hard_line_id(line)
}

pub(crate) fn ex_normal_target_position(
    document: &Document,
    target: ExNormalTarget,
) -> Option<usize> {
    let block = document
        .projection()
        .blocks()
        .iter()
        .find(|block| block.id == target.block_id)?;
    let offset = target.anchor.offset();
    if offset < block.range.start || offset > block.range.end {
        return None;
    }
    Some(block.range.start)
}

pub(crate) fn rebase_ex_normal_targets(
    document: &Document,
    targets: &mut [Option<ExNormalTarget>],
    map: &PositionMap,
) -> Result<(), PositionError> {
    for slot in targets {
        let Some(mut target) = *slot else {
            continue;
        };
        let mapped = match map.map_text_anchor(target.anchor)? {
            MappingOutcome::Exact(anchor)
            | MappingOutcome::Moved(anchor)
            | MappingOutcome::RecoveredFromProvenance(anchor) => anchor,
            MappingOutcome::CollapsedByDeletion(_)
            | MappingOutcome::Ambiguous(_)
            | MappingOutcome::Unresolvable(_) => {
                *slot = None;
                continue;
            }
        };
        if !document
            .projection()
            .blocks()
            .iter()
            .any(|block| block.id == target.block_id)
        {
            *slot = None;
            continue;
        }
        target.anchor = mapped;
        *slot = Some(target);
    }
    Ok(())
}

pub(crate) fn command_status_stops_compound(status: &CommandStatus) -> bool {
    matches!(
        status,
        CommandStatus::NeedsMoreLayout(_)
            | CommandStatus::SearchNotFound
            | CommandStatus::CountError(_)
            | CommandStatus::Unsupported(_)
            | CommandStatus::Error(_)
            | CommandStatus::RegisterReadError(_)
            | CommandStatus::RegisterWriteError(_)
            | CommandStatus::ExError(_)
            | CommandStatus::VisualBlockError(_)
    )
}

fn push_history(history: &mut Vec<String>, entry: String) {
    if history.last() != Some(&entry) {
        history.push(entry);
    }
}

fn navigate_history(buffer: &mut CommandLineBuffer, history: &[String], older: bool) {
    if history.is_empty() {
        return;
    }
    let next = if older {
        match buffer.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                buffer.draft = buffer.input.clone();
                history.len() - 1
            }
        }
    } else {
        let Some(index) = buffer.history_index else {
            return;
        };
        if index + 1 >= history.len() {
            buffer.history_index = None;
            buffer.set(buffer.draft.clone());
            return;
        }
        index + 1
    };
    buffer.history_index = Some(next);
    buffer.set(history[next].clone());
}

fn command_line_backspace(buffer: &mut CommandLineBuffer) {
    if buffer.delete_selection() {
        return;
    }
    let Some(previous) = previous_grapheme_boundary(&buffer.input, buffer.cursor) else {
        return;
    };
    buffer.input.replace_range(previous..buffer.cursor, "");
    buffer.cursor = previous;
    buffer.detach_from_history();
}

fn command_line_delete(buffer: &mut CommandLineBuffer) {
    if buffer.delete_selection() {
        return;
    }
    let Some(next) = next_grapheme_boundary(&buffer.input, buffer.cursor) else {
        return;
    };
    buffer.input.replace_range(buffer.cursor..next, "");
    buffer.detach_from_history();
}

fn command_line_delete_word(buffer: &mut CommandLineBuffer) {
    if buffer.delete_selection() {
        return;
    }
    let mut start = buffer.cursor;
    while let Some(previous) = previous_grapheme_boundary(&buffer.input, start) {
        if !buffer.input[previous..start]
            .chars()
            .all(char::is_whitespace)
        {
            break;
        }
        start = previous;
    }
    while let Some(previous) = previous_grapheme_boundary(&buffer.input, start) {
        if buffer.input[previous..start]
            .chars()
            .all(char::is_whitespace)
        {
            break;
        }
        start = previous;
    }
    if start != buffer.cursor {
        buffer.input.replace_range(start..buffer.cursor, "");
        buffer.cursor = start;
        buffer.detach_from_history();
    }
}

fn keyword_range(text: &str, offset: usize) -> Option<Range<usize>> {
    let current = grapheme_range_at(text, offset)?;
    let is_keyword = |grapheme: &str| {
        grapheme
            .chars()
            .next()
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
    };
    if !is_keyword(&text[current.clone()]) {
        return None;
    }
    // Scan each neighboring grapheme once. Repeated document-wide boundary
    // lookups make an oversized word quadratic before the regex size guard.
    let mut start = current.start;
    for (previous, grapheme) in text[..current.start].grapheme_indices(true).rev() {
        if !is_keyword(grapheme) {
            break;
        }
        start = previous;
    }
    let mut end = current.end;
    for (relative, grapheme) in text[current.end..].grapheme_indices(true) {
        if !is_keyword(grapheme) {
            break;
        }
        end = current.end + relative + grapheme.len();
    }
    Some(start..end)
}

fn matching_pair(text: &str, lines: &HardLineSnapshot, offset: usize) -> Option<usize> {
    let line_limit = line_end(lines, offset);
    let mut at = offset.min(text.len());
    let (opening, closing, forward) = loop {
        let character = text[at..].chars().next()?;
        let pair = match character {
            '(' => Some(('(', ')', true)),
            '[' => Some(('[', ']', true)),
            '{' => Some(('{', '}', true)),
            ')' => Some(('(', ')', false)),
            ']' => Some(('[', ']', false)),
            '}' => Some(('{', '}', false)),
            _ => None,
        };
        if let Some(pair) = pair {
            break pair;
        }
        at += character.len_utf8();
        if at >= line_limit {
            return None;
        }
    };

    if forward {
        let mut depth = 0usize;
        for (relative, character) in text[at..].char_indices() {
            if character == opening {
                depth += 1;
            } else if character == closing {
                depth -= 1;
                if depth == 0 {
                    return Some(at + relative);
                }
            }
        }
    } else {
        let mut depth = 0usize;
        for (position, character) in text[..=at].char_indices().rev() {
            if character == closing {
                depth += 1;
            } else if character == opening {
                depth -= 1;
                if depth == 0 {
                    return Some(position);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};
    use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

    fn line_start(text: &str, offset: usize) -> usize {
        let document = Document::new(text);
        super::line_start(&document.hard_line_snapshot(), offset)
    }

    fn nth_line_start(text: &str, one_based: usize) -> usize {
        let document = Document::new(text);
        super::nth_line_start(&document.hard_line_snapshot(), one_based)
    }

    fn forced_mac_source(text: &str, format: Format) -> Vec<u8> {
        if format == Format::Markdown {
            text.replace('\r', "\r\r").into_bytes()
        } else {
            text.as_bytes().to_vec()
        }
    }

    fn forced_mac_document(format: Format, encoding: Encoding) -> Document {
        Document::from_bytes_with_file_format(
            forced_mac_source("a\rb\nc", format),
            encoding,
            format,
            FileFormat::Mac,
        )
        .unwrap()
    }

    fn keys(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        input: &str,
    ) -> CommandOutput {
        let mut output = CommandOutput::complete();
        for character in input.chars() {
            output = commands
                .handle(document, InputEvent::key(character))
                .unwrap_or_else(|error| panic!("key {character:?} failed: {error:?}"));
        }
        output
    }

    fn key(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        input: Key,
    ) -> CommandOutput {
        commands.handle(document, InputEvent::Key(input)).unwrap()
    }

    fn layout_snapshot(document: &Document, width: f32) -> LayoutSnapshot {
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(width, 500.0);
        engine.relayout(document, &mut view).unwrap();
        view.snapshot().unwrap().clone()
    }

    fn layout_key(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        context: &mut LayoutCommandContext<'_>,
        key: Key,
    ) -> CommandOutput {
        commands
            .handle_with_layout(document, InputEvent::Key(key), context)
            .unwrap()
    }

    fn layout_keys(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        width: f32,
        input: &[Key],
    ) -> CommandOutput {
        let snapshot = layout_snapshot(document, width);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        let mut output = CommandOutput::complete();
        for key in input {
            output = layout_key(commands, document, &mut context, *key);
        }
        output
    }

    #[test]
    fn immutable_resolution_emits_a_revision_bound_normal_delete_plan() {
        let document = Document::new("abc");
        let commands = CommandInterpreter::new();
        let context = CommandContext::new(&document);
        let register_before = commands.register('"').cloned();

        let plan = match commands.resolve(&context, InputEvent::key('x')).unwrap() {
            CommandResolution::Planned(plan) => plan,
            CommandResolution::Legacy(reason) => {
                panic!("Normal x unexpectedly used legacy resolution: {reason:?}")
            }
        };

        assert_eq!(plan.document(), document.id());
        assert_eq!(plan.revision(), document.revision());
        assert_eq!(plan.undo_group_directive(), UndoGroupDirective::Preserve);
        assert_eq!(
            plan.presentation_requests(),
            &[
                CommandPresentationRequest::Relayout,
                CommandPresentationRequest::RevealCaret,
            ]
        );
        assert!(matches!(
            plan.model_request(),
            Some(CommandModelRequest::Model(ModelRequest::ApplyTextEdits {
                document: request_document,
                revision,
                edits,
            })) if *request_document == document.id()
                && *revision == document.revision()
                && edits == &[TextEdit::new(0..1, "")]
        ));

        // Resolution only produced a value; neither the source nor controller
        // success effects are visible until the coordinator publishes it.
        assert_eq!(document.text(), "abc");
        assert_eq!(document.revision(), Revision(0));
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.register('"'), register_before.as_ref());
    }

    #[test]
    fn controller_only_grammar_motions_visual_and_command_line_emit_plans() {
        fn planned(
            commands: &mut CommandInterpreter,
            document: &Document,
            event: InputEvent,
        ) -> (CommandOutput, Vec<CommandPresentationRequest>) {
            let context = CommandContext::new(document);
            let plan = match commands.resolve(&context, event).unwrap() {
                CommandResolution::Planned(plan) => plan,
                CommandResolution::Legacy(reason) => {
                    panic!("controller-only event unexpectedly used legacy path: {reason:?}")
                }
            };
            assert!(plan.model_request().is_none());
            let presentation = plan.presentation_requests().to_vec();
            let step = plan.publish_success(commands, document, false);
            assert!(step.replay.is_none());
            (step.output, presentation)
        }

        let document = Document::new("one two\nthree four");
        let mut planned_commands = CommandInterpreter::new();
        let mut direct_document = Document::new("one two\nthree four");
        let mut direct_commands = CommandInterpreter::new();
        let events = [
            InputEvent::key('2'),
            InputEvent::key('w'),
            InputEvent::key('m'),
            InputEvent::key('a'),
            InputEvent::key('g'),
            InputEvent::key('g'),
            InputEvent::key('\''),
            InputEvent::key('a'),
            InputEvent::key('/'),
            InputEvent::text("one"),
            InputEvent::Key(Key::Left),
            InputEvent::Key(Key::Delete),
            InputEvent::Key(Key::Escape),
            InputEvent::key('v'),
            InputEvent::key('l'),
            InputEvent::key('o'),
            InputEvent::Key(Key::Escape),
        ];

        let mut saw_reveal = false;
        for event in events {
            let direct = direct_commands
                .handle(&mut direct_document, event.clone())
                .unwrap();
            let (resolved, presentation) = planned(&mut planned_commands, &document, event);
            assert_eq!(resolved, direct);
            if resolved.cursor_moved {
                assert_eq!(presentation, vec![CommandPresentationRequest::RevealCaret]);
                saw_reveal = true;
            } else {
                assert!(presentation.is_empty());
            }
            assert_eq!(planned_commands.mode(), direct_commands.mode());
            assert_eq!(planned_commands.cursor(), direct_commands.cursor());
            assert_eq!(
                planned_commands.command_line(),
                direct_commands.command_line()
            );
            assert_eq!(
                planned_commands.command_line_cursor(),
                direct_commands.command_line_cursor()
            );
            assert_eq!(document.text(), "one two\nthree four");
        }
        assert!(saw_reveal);
    }

    #[test]
    fn immutable_resolution_classifies_layout_and_compound_boundaries() {
        let document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands.set_layout_options(true);
        let context = CommandContext::new(&document);
        assert!(matches!(
            commands.resolve(&context, InputEvent::key('j')).unwrap(),
            CommandResolution::Legacy(LegacyCommandReason::LayoutDependent)
        ));
        assert!(matches!(
            commands.resolve(&context, InputEvent::key('p')).unwrap(),
            CommandResolution::Legacy(LegacyCommandReason::CompoundOrUnmigrated)
        ));

        commands.set_layout_options(false);
        let count = match commands.resolve(&context, InputEvent::key('2')).unwrap() {
            CommandResolution::Planned(plan) => plan,
            CommandResolution::Legacy(reason) => panic!("count used legacy path: {reason:?}"),
        };
        count.publish_success(&mut commands, &document, false);
        assert!(
            matches!(
                commands.resolve(&context, InputEvent::key('+')).unwrap(),
                CommandResolution::Legacy(LegacyCommandReason::LayoutDependent)
            ),
            "Visual line mode uses the exact view even when wrapping is disabled"
        );
    }

    #[test]
    fn stale_controller_only_plan_cannot_publish_its_controller_image() {
        let mut document = Document::new("one two");
        let commands = CommandInterpreter::new();
        let plan = match commands
            .resolve(&CommandContext::new(&document), InputEvent::key('w'))
            .unwrap()
        {
            CommandResolution::Planned(plan) => plan,
            CommandResolution::Legacy(reason) => panic!("word motion used legacy: {reason:?}"),
        };

        document.insert(0, "x ").unwrap();
        assert!(matches!(
            plan.prepare_model(&document),
            Err(ModelTransactionError::StaleRevision {
                expected: Revision(0),
                actual: Revision(1)
            })
        ));
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.position_revision(), None);
    }

    #[test]
    fn counts_and_unicode_motions_use_graphemes() {
        let mut document = Document::new("a\u{301}é日z");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2l");
        assert_eq!(commands.cursor(), "a\u{301}é".len());
        assert!(is_grapheme_boundary(document.text(), commands.cursor()));

        keys(&mut commands, &mut document, "2h");
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn multiplied_operator_counts_and_doubled_operator_are_linewise() {
        let mut document = Document::new("a\nb\nc\nd\ne");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2d2d");
        assert_eq!(document.text(), "e");
        assert_eq!(commands.register('1').unwrap().text, "a\nb\nc\nd\n");
    }

    #[test]
    fn named_yank_preserves_zero_and_linewise_put_preserves_register_kind() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(None, RegisterValue::characterwise("seed"));

        keys(&mut commands, &mut document, "\"ayyG\"ap");
        assert_eq!(document.text(), "one\ntwo\none");
        assert_eq!(commands.register('a').unwrap().text, "one\n");
        assert_eq!(commands.register('a').unwrap().kind, RegisterKind::Linewise);
        assert_eq!(commands.register('0').unwrap().text, "seed");
        assert_eq!(commands.register('"'), commands.register('a'));
    }

    #[test]
    fn unicode_small_deletes_respect_explicit_and_black_hole_register_rules() {
        let mut document = Document::new("a\u{301}😀bc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "x");
        assert_eq!(commands.register('-').unwrap().text, "a\u{301}");
        assert_eq!(commands.register('"').unwrap().text, "a\u{301}");
        assert!(commands.register('1').is_none());

        keys(&mut commands, &mut document, "\"ax");
        assert_eq!(commands.register('a').unwrap().text, "😀");
        assert_eq!(commands.register('"').unwrap().text, "😀");
        assert_eq!(commands.register('-').unwrap().text, "a\u{301}");
        assert_eq!(commands.register('1').unwrap().text, "😀");

        keys(&mut commands, &mut document, "\"\"x");
        assert_eq!(commands.register('"').unwrap().text, "b");
        assert_eq!(commands.register('1').unwrap().text, "b");
        assert_eq!(
            commands.register('-').unwrap().text,
            "a\u{301}",
            "an explicit unnamed register suppresses only the small-delete register"
        );

        keys(&mut commands, &mut document, "\"_x");
        assert_eq!(document.text(), "");
        assert_eq!(commands.register('"').unwrap().text, "b");
        assert_eq!(commands.register('-').unwrap().text, "a\u{301}");
    }

    #[test]
    fn explicit_unnamed_yank_updates_zero_while_named_visual_yank_does_not() {
        let mut document = Document::new("a\u{301}😀\nlast");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "yiw");
        assert_eq!(commands.register('0').unwrap().text, "a\u{301}");

        keys(&mut commands, &mut document, "lv\"ay");
        assert_eq!(commands.register('a').unwrap().text, "😀");
        assert_eq!(commands.register('0').unwrap().text, "a\u{301}");

        keys(&mut commands, &mut document, "G\"\"yy");
        assert_eq!(commands.register('0').unwrap().text, "last\n");
        assert_eq!(commands.register('"').unwrap().text, "last\n");
    }

    #[test]
    fn large_named_deletes_still_rotate_numbered_registers() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "dd\"add");
        assert_eq!(document.text(), "three");
        assert_eq!(commands.register('a').unwrap().text, "two\n");
        assert_eq!(commands.register('1').unwrap().text, "two\n");
        assert_eq!(commands.register('2').unwrap().text, "one\n");
        assert_eq!(commands.register('"').unwrap().text, "two\n");
        assert!(commands.register('-').is_none());
    }

    #[test]
    fn exceptional_operator_motions_use_numbered_and_conditionally_small_registers() {
        let mut document = Document::new("(a\u{301}😀) tail");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "d%");
        assert_eq!(document.text(), " tail");
        assert_eq!(commands.register('1').unwrap().text, "(a\u{301}😀)");
        assert_eq!(commands.register('-').unwrap().text, "(a\u{301}😀)");

        let mut document = Document::new("One. Two.");
        let mut commands = CommandInterpreter::new();
        commands.registers.delete(
            None,
            RegisterValue::characterwise("keep"),
            DeletionClass::Small,
        );
        keys(&mut commands, &mut document, "\"ad)");
        assert_eq!(document.text(), "Two.");
        assert_eq!(commands.register('a').unwrap().text, "One. ");
        assert_eq!(commands.register('1').unwrap().text, "One. ");
        assert_eq!(commands.register('-').unwrap().text, "keep");

        let mut document = Document::new("One. Two.");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "c)");
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(commands.register('1').unwrap().text, "One. ");
        assert_eq!(commands.register('-').unwrap().text, "One. ");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Two.");

        let mut document = Document::new("One. Two! Three.\n\nNext paragraph.");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "d}");
        assert_eq!(document.text(), "\nNext paragraph.");
        assert_eq!(commands.register('1').unwrap().text, "One. Two! Three.\n");
        assert_eq!(commands.register('1').unwrap().kind, RegisterKind::Linewise);
        assert!(commands.register('-').is_none());
    }

    #[test]
    fn operator_mark_motions_distinguish_exact_and_linewise_extents() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "4lma0l2d`a");
        assert_eq!(document.text(), "aef");
        assert_eq!(commands.register('1').unwrap().text, "bcd");
        assert_eq!(commands.register('-').unwrap().text, "bcd");
        assert_eq!(
            commands.register('1').unwrap().kind,
            RegisterKind::Characterwise
        );
        assert_eq!(commands.jumps, vec![1, 1]);
        assert!(document.undo());
        assert_eq!(document.text(), "abcdef");
        assert!(!document.undo(), "the mark operator is one undo unit");

        let mut document = Document::new("one\n  two\nthree\nfour");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3Gma1Gd2'a");
        assert_eq!(document.text(), "four");
        assert_eq!(commands.register('1').unwrap().text, "one\n  two\nthree\n");
        assert_eq!(commands.register('1').unwrap().kind, RegisterKind::Linewise);
        assert!(commands.register('-').is_none());
        assert_eq!(commands.jumps, vec![0, 0]);
    }

    #[test]
    fn missing_and_cancelled_operator_targets_are_atomic() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        let revision = document.revision();

        let missing_mark = keys(&mut commands, &mut document, "d`z");
        assert!(matches!(missing_mark.status, CommandStatus::Error(_)));
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.text(), "one two");
        assert!(commands.register('1').is_none());
        assert!(commands.jumps.is_empty());

        keys(&mut commands, &mut document, "d/missing");
        assert_eq!(commands.mode(), Mode::CommandLine);
        let cancelled = key(&mut commands, &mut document, Key::Escape);
        assert_eq!(cancelled.status, CommandStatus::Cancelled);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.revision(), revision);
        assert!(commands.last_search.is_none());
        assert!(commands.search_history.is_empty());
        assert!(commands.jumps.is_empty());
        assert!(!document.undo());
    }

    #[test]
    fn invalid_operator_search_regex_changes_no_edit_state() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        commands.registers.delete(
            None,
            RegisterValue::characterwise("seed"),
            DeletionClass::Small,
        );
        let revision = document.revision();

        keys(&mut commands, &mut document, "d/[");
        let invalid = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(invalid.status, CommandStatus::Error(_)));
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.text(), "one two");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.register('-').unwrap().text, "seed");
        assert!(commands.register('1').is_none());
        assert!(commands.last_search.is_none());
        assert!(commands.jumps.is_empty());
        assert!(!document.undo());
    }

    #[test]
    fn counted_operator_search_is_exclusive_repeatable_and_one_undo_unit() {
        let original = "cat xx cat yy cat";
        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "d2/cat");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), "cat");
        assert_eq!(commands.register('1').unwrap().text, "cat xx cat yy ");
        assert_eq!(commands.register('-').unwrap().text, "cat xx cat yy ");
        assert_eq!(
            commands.last_search,
            Some((SearchDirection::Forward, "cat".to_owned()))
        );
        assert_eq!(commands.search_history, vec!["cat"]);
        assert_eq!(commands.jumps, vec![0, 0]);
        assert!(document.undo());
        assert_eq!(document.text(), original);
        assert!(!document.undo());

        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "cat");
        assert!(document.undo(), "dot replay is a separate undo unit");
        assert_eq!(document.text(), original);
    }

    #[test]
    fn named_operator_search_keeps_exceptional_numbered_register_policy() {
        let mut document = Document::new("cat xx cat");
        let mut commands = CommandInterpreter::new();
        commands.registers.delete(
            None,
            RegisterValue::characterwise("seed"),
            DeletionClass::Small,
        );

        keys(&mut commands, &mut document, "\"ad/cat");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "cat");
        assert_eq!(commands.register('a').unwrap().text, "cat xx ");
        assert_eq!(commands.register('1').unwrap().text, "cat xx ");
        assert_eq!(
            commands.register('-').unwrap().text,
            "seed",
            "an explicit destination suppresses the small-delete side write"
        );
    }

    #[test]
    fn operator_search_change_and_insert_share_one_undo_unit() {
        let original = "cat x cat";
        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "c/cat");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.mode(), Mode::Insert);
        keys(&mut commands, &mut document, "X");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xcat");
        assert!(document.undo());
        assert_eq!(document.text(), original);
        assert!(!document.undo(), "change plus insertion is one undo unit");
    }

    #[test]
    fn operator_repeat_search_honors_direction_and_counts() {
        let mut document = Document::new("cat xx cat yy cat");
        let mut commands = CommandInterpreter::new();
        commands.last_search = Some((SearchDirection::Forward, "cat".to_owned()));

        keys(&mut commands, &mut document, "d2n");
        assert_eq!(document.text(), "cat");
        assert_eq!(commands.register('1').unwrap().text, "cat xx cat yy ");

        let mut document = Document::new("cat xx cat yy cat");
        let mut commands = CommandInterpreter::new();
        commands.last_search = Some((SearchDirection::Forward, "cat".to_owned()));
        assert!(commands.set_cursor(&document, 14));
        keys(&mut commands, &mut document, "dN");
        assert_eq!(document.text(), "cat xx cat");
        assert_eq!(commands.register('1').unwrap().text, "cat yy ");
        assert_eq!(commands.jumps, vec![7, 7]);
    }

    #[test]
    fn operator_star_searches_support_whole_word_and_substring_variants() {
        for (command, original, expected, captured) in [
            ("d*", "cat xx cat yy", "cat yy", "cat xx "),
            ("dg*", "cat scatter cat", "catter cat", "cat s"),
        ] {
            let mut document = Document::new(original);
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.register('1').unwrap().text, captured, "{command}");
            assert_eq!(commands.register('-').unwrap().text, captured, "{command}");
            assert_eq!(commands.jumps, vec![0, 0], "{command}");
        }

        for command in ["d#", "dg#"] {
            let mut document = Document::new("cat scatter cat");
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, 12));
            keys(&mut commands, &mut document, command);
            assert_ne!(document.text(), "cat scatter cat", "{command}");
            assert!(commands.register('1').is_some(), "{command}");
            assert!(commands.register('-').is_some(), "{command}");
            assert_eq!(commands.jumps.len(), 2, "{command}");
        }
    }

    #[test]
    fn forward_search_to_column_zero_uses_vim_exclusive_motion_rules() {
        let original = "  aaa\nbbb\n  ccc";
        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        keys(&mut commands, &mut document, "d/  ccc");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "  ccc");
        assert_eq!(commands.register('1').unwrap().text, "  aaa\nbbb\n");
        assert_eq!(commands.register('1').unwrap().kind, RegisterKind::Linewise);

        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 3));
        keys(&mut commands, &mut document, "d/  ccc");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "  a\n  ccc");
        assert_eq!(commands.register('1').unwrap().text, "aa\nbbb");
        assert_eq!(
            commands.register('1').unwrap().kind,
            RegisterKind::Characterwise
        );
    }

    #[test]
    fn operator_search_never_splits_extended_graphemes() {
        let mut document = Document::new("a\u{301}😀 xx a\u{301}😀");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "d/😀");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "😀 xx a\u{301}😀");
        assert_eq!(commands.register('1').unwrap().text, "a\u{301}");
        assert!(is_grapheme_boundary(document.text(), commands.cursor()));
    }

    #[test]
    fn operator_search_reverse_projects_through_markdown() {
        use crate::document::{Encoding, Format};

        let mut document = Document::from_bytes(
            b"**bold** cat cat".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        assert_eq!(document.text(), "bold cat cat");
        assert!(commands.set_cursor(&document, 5));

        keys(&mut commands, &mut document, "d/cat");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "bold cat");
        assert_eq!(document.source_bytes(), b"**bold** cat".to_vec());
        assert_eq!(commands.register('1').unwrap().text, "cat ");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"**bold** cat cat".to_vec());
    }

    #[test]
    fn latin1_operator_search_failure_restores_command_and_document_state() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(vec![0xff, b' ', b'x'], Encoding::Latin1, Format::PlainText)
                .unwrap();
        let mut commands = CommandInterpreter::new();
        let revision = document.revision();
        let source = document.source_bytes();

        keys(&mut commands, &mut document, "gU/x");
        let result = commands.handle(&mut document, InputEvent::Key(Key::Enter));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "ÿ x");
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.mode(), Mode::CommandLine);
        assert_eq!(commands.command_line(), Some("x"));
        assert!(commands.last_search.is_none());
        assert!(commands.search_history.is_empty());
        assert!(commands.jumps.is_empty());
        assert!(!document.undo());

        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.mode(), Mode::Normal);
    }

    #[test]
    fn uppercase_yank_append_publishes_full_destination_and_promotes_linewise_kind() {
        let mut document = Document::new("one two\nthree");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "\"ayiw");
        keys(&mut commands, &mut document, "w\"Ayiw");
        assert_eq!(
            commands.register('a'),
            Some(&RegisterValue::characterwise("onetwo"))
        );
        assert_eq!(commands.register('"'), commands.register('a'));

        keys(&mut commands, &mut document, "G\"Ayy");
        assert_eq!(
            commands.register('a'),
            Some(&RegisterValue::linewise("onetwo\nthree\n"))
        );
        assert_eq!(commands.register('"'), commands.register('a'));
        assert!(
            commands.register('0').is_none(),
            "explicit named yanks, including append, do not publish register zero"
        );
    }

    #[test]
    fn insert_session_is_one_undo_unit_and_redo_restores_it() {
        let mut document = Document::new("world");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "i");
        commands
            .handle(&mut document, InputEvent::text("hé"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "héworld");
        assert_eq!(commands.mode(), Mode::Normal);

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "world");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(document.text(), "héworld");
    }

    #[test]
    fn insert_ctrl_r_uses_a_register_as_its_own_undo_unit() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "yyGI");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        keys(&mut commands, &mut document, "0");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "one\none\ntwo");

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "one\ntwo");
    }

    #[test]
    fn typing_caret_follows_a_grapheme_joined_to_unchanged_suffix() {
        for (original, mode, input, expected, caret) in [
            ("\u{301}", 'i', "🇨🇦", "🇨🇦\u{301}", "🇨🇦\u{301}".len()),
            ("\u{301}tail", 'i', "e", "e\u{301}tail", "e\u{301}".len()),
            ("🇨🇦", 'i', "🇫", "🇫🇨🇦", "🇫🇨".len()),
            ("x🇨🇦", 'R', "🇫", "🇫🇨🇦", "🇫🇨".len()),
        ] {
            for planned in [false, true] {
                let mut document = Document::new(original);
                let mut commands = CommandInterpreter::new();
                key(&mut commands, &mut document, Key::Char(mode));
                if planned {
                    let context = CommandContext::new(&document);
                    let CommandResolution::Planned(plan) =
                        commands.resolve(&context, InputEvent::text(input)).unwrap()
                    else {
                        panic!("ordinary typing should use the immutable plan");
                    };
                    let prepared = plan.prepare_model(&document).unwrap().unwrap();
                    document.commit_model_transaction(prepared).unwrap();
                    plan.publish_success(&mut commands, &document, true);
                } else {
                    commands
                        .handle(&mut document, InputEvent::text(input))
                        .unwrap();
                }
                assert_eq!(document.text(), expected);
                assert_eq!(
                    commands.cursor(),
                    caret,
                    "{original:?} {mode} {input:?}, planned={planned}"
                );
                assert!(document.text_point(commands.cursor()).is_ok());
                key(&mut commands, &mut document, Key::Escape);
                keys(&mut commands, &mut document, "u");
                assert_eq!(document.text(), original);
                key(&mut commands, &mut document, Key::Ctrl('r'));
                assert_eq!(document.text(), expected);
                assert!(document.text_point(commands.cursor()).is_ok());
            }
        }
    }

    #[test]
    fn replace_mode_replaces_graphemes_without_splitting_them() {
        let mut document = Document::new("a\u{301}bc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("XY"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "XYc");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "a\u{301}bc");
    }

    #[test]
    fn replace_backspace_restores_overwritten_extended_graphemes_lifo() {
        let original = "a\u{301}👩‍💻z";
        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("é日"))
            .unwrap();
        assert_eq!(document.text(), "é日z");

        let restored_emoji = key(&mut commands, &mut document, Key::Backspace);
        assert!(restored_emoji.document_changed);
        assert_eq!(document.text(), "é👩‍💻z");
        assert_eq!(commands.cursor(), "é".len());

        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), original);
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn replace_backspace_removes_text_appended_past_end_of_line() {
        let mut document = Document::new("a");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("XYZ"))
            .unwrap();
        assert_eq!(document.text(), "XYZ");

        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "XY");
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "X");
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "a");
    }

    #[test]
    fn replace_backspace_and_overwrite_remain_one_undo_unit() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("XY"))
            .unwrap();
        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xbc");

        assert!(document.undo());
        assert_eq!(document.text(), "abc");
        assert!(!document.undo());
    }

    #[test]
    fn replace_restoration_is_invalidated_by_cursor_motion_and_delete() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Left);
        key(&mut commands, &mut document, Key::Right);
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(
            document.text(),
            "bc",
            "fallback deletes; it cannot restore a"
        );

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Delete);
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(
            document.text(),
            "c",
            "Delete also ends the restore frontier"
        );
    }

    #[test]
    fn replace_newline_does_not_consume_text_and_ends_restore_frontier() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "X\nbc");
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "Xbc");
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "bc", "the pre-newline a is not restored");
    }

    #[test]
    fn insert_enter_continues_lists_and_empty_item_ends_list_in_one_undo_group() {
        for format in [Format::PlainText, Format::Markdown, Format::MarkdownSource] {
            for source in ["- first", "3. first"] {
                let mut document =
                    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format)
                        .unwrap();
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "A");
                key(&mut commands, &mut document, Key::Enter);
                assert_eq!(
                    document.text(),
                    if format == Format::Markdown {
                        "first\n"
                    } else if source.starts_with('-') {
                        "- first\n- "
                    } else {
                        "3. first\n4. "
                    }
                );
                commands
                    .handle(&mut document, InputEvent::text("second"))
                    .unwrap();
                key(&mut commands, &mut document, Key::Enter);
                key(&mut commands, &mut document, Key::Enter);
                assert_eq!(
                    document.text(),
                    if format == Format::Markdown {
                        "first\nsecond\n"
                    } else if source.starts_with('-') {
                        "- first\n- second\n"
                    } else {
                        "3. first\n4. second\n"
                    }
                );
                key(&mut commands, &mut document, Key::Escape);
                keys(&mut commands, &mut document, "u");
                assert_eq!(document.source_bytes(), source.as_bytes());
                key(&mut commands, &mut document, Key::Ctrl('r'));
                assert!(document.text().ends_with("second\n"));
            }
        }
    }

    #[test]
    fn failed_latin1_replace_event_preserves_the_existing_restore_journal() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"ab".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        assert_eq!(document.text(), "Xb");

        let result = commands.handle(&mut document, InputEvent::text("😀"));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "Xb");
        assert_eq!(commands.cursor(), 1);

        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(
            document.text(),
            "ab",
            "the successful entry remains restorable"
        );
    }

    #[test]
    fn visual_character_and_visual_line_edits_are_inclusive() {
        let mut document = Document::new("aé日z");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "v2ld");
        assert_eq!(document.text(), "z");
        assert_eq!(commands.mode(), Mode::Normal);

        let mut document = Document::new("a\nb\nc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vjd");
        assert_eq!(document.text(), "c");
    }

    #[test]
    fn characterwise_visual_delete_crossing_a_line_is_large() {
        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "lvjd");
        assert_eq!(document.text(), "a");
        assert_eq!(commands.register('1').unwrap().text, "b\ncd");
        assert_eq!(
            commands.register('1').unwrap().kind,
            RegisterKind::Characterwise
        );
        assert!(commands.register('-').is_none());
    }

    #[test]
    fn visual_register_prefix_writes_the_requested_register() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "ve\"ay");
        assert_eq!(commands.register('a').unwrap().text, "one");
        assert_eq!(document.text(), "one two");
    }

    #[test]
    fn failed_normal_model_edit_consumes_its_explicit_register_prefix() {
        let mut document =
            Document::from_bytes(b"ab".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("😀"));
        commands
            .registers
            .yank(None, RegisterValue::characterwise("X"));
        let revision = document.revision();

        keys(&mut commands, &mut document, "\"a");
        let failed = commands.handle(&mut document, InputEvent::key('p'));
        assert!(matches!(
            failed,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "ab");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.requested_register, None);

        let output = key(&mut commands, &mut document, Key::Char('p'));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), "aXb", "the next command uses unnamed");
    }

    #[test]
    fn visual_character_and_line_register_failures_preserve_selection_and_memory() {
        for (entry, expected_mode) in [("vl", Mode::VisualCharacter), ("Vj", Mode::VisualLine)] {
            let mut document = Document::new("one\ntwo\nthree");
            let mut commands = CommandInterpreter::new();
            let previous = VisualMemory {
                mode: Mode::VisualCharacter,
                anchor: 4,
                active: 6,
                to_line_end: false,
                block: None,
            };
            commands.last_visual = Some(previous);
            keys(&mut commands, &mut document, entry);
            let anchor = commands.visual_anchor;
            let cursor = commands.cursor();
            let revision = document.revision();

            let read = keys(&mut commands, &mut document, "\"+p");
            assert!(matches!(
                read.status,
                CommandStatus::RegisterReadError(RegisterReadError::ClipboardUnavailable(_))
            ));
            assert_eq!(commands.mode(), expected_mode, "{entry} read mode");
            assert_eq!(commands.visual_anchor, anchor, "{entry} read anchor");
            assert_eq!(commands.cursor(), cursor, "{entry} read cursor");
            assert_eq!(commands.last_visual, Some(previous), "{entry} read memory");
            assert_eq!(commands.requested_register, None, "{entry} read prefix");
            assert_eq!(document.revision(), revision, "{entry} read revision");

            commands
                .registers
                .yank(Some('a'), RegisterValue::characterwise("X"));
            let recovered = keys(&mut commands, &mut document, "\"ap");
            assert_eq!(recovered.status, CommandStatus::Complete, "{entry} paste");
            assert_eq!(commands.mode(), Mode::Normal, "{entry} paste mode");

            let mut document = Document::new("one\ntwo\nthree");
            let mut commands = CommandInterpreter::new();
            commands.last_visual = Some(previous);
            keys(&mut commands, &mut document, entry);
            let anchor = commands.visual_anchor;
            let cursor = commands.cursor();
            let revision = document.revision();

            let write = keys(&mut commands, &mut document, "\"%d");
            assert_eq!(
                write.status,
                CommandStatus::RegisterWriteError(RegisterWriteError::ReadOnly('%')),
                "{entry} write"
            );
            assert_eq!(commands.mode(), expected_mode, "{entry} write mode");
            assert_eq!(commands.visual_anchor, anchor, "{entry} write anchor");
            assert_eq!(commands.cursor(), cursor, "{entry} write cursor");
            assert_eq!(commands.last_visual, Some(previous), "{entry} write memory");
            assert_eq!(commands.requested_register, None, "{entry} write prefix");
            assert_eq!(document.revision(), revision, "{entry} write revision");

            let recovered = keys(&mut commands, &mut document, "\"ay");
            assert_eq!(recovered.status, CommandStatus::Complete, "{entry} yank");
            assert_eq!(commands.mode(), Mode::Normal, "{entry} yank mode");
            assert!(commands.register('a').is_some(), "{entry} yank register");
        }
    }

    #[test]
    fn visual_block_register_failures_preserve_selection_and_memory() {
        for (suffix, expected_read) in [("+p", true), ("%d", false)] {
            let mut document = Document::new("ab\ncd");
            let mut commands = CommandInterpreter::new();
            let previous = VisualMemory {
                mode: Mode::VisualLine,
                anchor: 0,
                active: 3,
                to_line_end: false,
                block: None,
            };
            commands.last_visual = Some(previous);
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            for key in [Key::Ctrl('v'), Key::Char('l'), Key::Char('j')] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }
            let selection = commands.visual_block.clone();
            let cursor = commands.cursor();
            let revision = document.revision();

            layout_key(&mut commands, &mut document, &mut context, Key::Char('"'));
            let output = layout_key(
                &mut commands,
                &mut document,
                &mut context,
                Key::Char(suffix.chars().next().unwrap()),
            );
            assert_eq!(output.status, CommandStatus::Pending);
            let output = layout_key(
                &mut commands,
                &mut document,
                &mut context,
                Key::Char(suffix.chars().nth(1).unwrap()),
            );
            if expected_read {
                assert!(matches!(
                    output.status,
                    CommandStatus::RegisterReadError(RegisterReadError::ClipboardUnavailable(_))
                ));
            } else {
                assert_eq!(
                    output.status,
                    CommandStatus::RegisterWriteError(RegisterWriteError::ReadOnly('%'))
                );
            }
            assert_eq!(commands.mode(), Mode::VisualBlock);
            assert_eq!(commands.visual_block, selection);
            assert_eq!(commands.cursor(), cursor);
            assert_eq!(commands.last_visual, Some(previous));
            assert_eq!(commands.requested_register, None);
            assert_eq!(document.revision(), revision);

            commands
                .registers
                .yank(Some('a'), RegisterValue::characterwise("X"));
            for key in [Key::Char('"'), Key::Char('a'), Key::Char('y')] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }
            assert_eq!(commands.mode(), Mode::Normal);
            assert_eq!(document.revision(), revision);
            assert_eq!(
                commands.register('a').unwrap().kind,
                RegisterKind::Blockwise
            );
        }
    }

    #[test]
    fn failed_encoded_visual_paste_changes_neither_document_nor_register_policy() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"abc".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        let previous_visual = VisualMemory {
            mode: Mode::VisualLine,
            anchor: 0,
            active: 2,
            to_line_end: false,
            block: None,
        };
        commands.last_visual = Some(previous_visual);
        commands
            .registers
            .yank(None, RegisterValue::characterwise("seed"));
        commands.registers.delete(
            None,
            RegisterValue::characterwise("old-small"),
            DeletionClass::Small,
        );
        commands.registers.delete(
            None,
            RegisterValue::linewise("old-large\n"),
            DeletionClass::Large,
        );
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("😀"));
        let revision = document.revision();
        let source = document.source_bytes();

        keys(&mut commands, &mut document, "v\"a");
        let visual_anchor = commands.visual_anchor;
        let visual_cursor = commands.cursor();
        let result = commands.handle(&mut document, InputEvent::key('p'));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "abc");
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.register('0').unwrap().text, "seed");
        assert_eq!(commands.register('a').unwrap().text, "😀");
        assert_eq!(commands.register('"').unwrap().text, "😀");
        assert_eq!(commands.register('-').unwrap().text, "old-small");
        assert_eq!(commands.register('1').unwrap().text, "old-large\n");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor, visual_anchor);
        assert_eq!(commands.cursor(), visual_cursor);
        assert_eq!(commands.last_visual, Some(previous_visual));
        assert_eq!(commands.requested_register, None);
        assert!(!document.undo());
    }

    #[test]
    fn word_operators_and_case_operators_compose() {
        let mut document = Document::new("ONE two THREE");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "guw");
        assert_eq!(document.text(), "one two THREE");
        keys(&mut commands, &mut document, "w");
        keys(&mut commands, &mut document, "gUw");
        assert_eq!(document.text(), "one TWO THREE");
        keys(&mut commands, &mut document, "de");
        assert_eq!(document.text(), "one  THREE");
    }

    #[test]
    fn indent_outdent_and_reindent_are_undoable_line_edits() {
        let mut document = Document::new("  a\n    b");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ">>");
        assert_eq!(document.text(), "      a\n    b");
        keys(&mut commands, &mut document, "<<");
        assert_eq!(document.text(), "  a\n    b");
        keys(&mut commands, &mut document, "==");
        assert_eq!(document.text(), "a\n    b");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "  a\n    b");
    }

    #[test]
    fn forward_backward_and_repeat_search_wrap() {
        let mut document = Document::new("foo bar foo baz");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "/foo");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.cursor(), 8);
        keys(&mut commands, &mut document, "N");
        assert_eq!(commands.cursor(), 0);
        keys(&mut commands, &mut document, "n");
        assert_eq!(commands.cursor(), 8);

        keys(&mut commands, &mut document, "?bar");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.cursor(), 4);
    }

    #[test]
    fn invalid_search_is_non_destructive() {
        let mut document = Document::new("unchanged");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "/[");
        let output = key(&mut commands, &mut document, Key::Enter);

        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "unchanged");
        assert_eq!(commands.mode(), Mode::Normal);
    }

    #[test]
    fn dot_repeats_simple_delete_and_insert_changes() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "x2l.");
        assert_eq!(document.text(), "bcef");

        let mut document = Document::new("one");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "A!");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "one!!");
    }

    #[test]
    fn replace_character_count_is_atomic_and_checks_line_end() {
        let mut document = Document::new("aé日\nnext");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "2rX");
        assert_eq!(document.text(), "XX日\nnext");
        keys(&mut commands, &mut document, "5rQ");
        assert_eq!(document.text(), "XX日\nnext");
    }

    #[test]
    fn joins_distinguish_j_and_gj_spacing() {
        let mut document = Document::new("one\n  two\nthree");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "J");
        assert_eq!(document.text(), "one two\nthree");
        keys(&mut commands, &mut document, "gJ");
        assert_eq!(document.text(), "one twothree");
    }

    #[test]
    fn absolute_line_motions_honor_explicit_counts() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "G");
        assert_eq!(commands.cursor(), 8);
        keys(&mut commands, &mut document, "1G");
        assert_eq!(commands.cursor(), 0);
        keys(&mut commands, &mut document, "2gg");
        assert_eq!(commands.cursor(), 4);
    }

    #[test]
    fn escape_cancels_pending_operator_and_unknown_prefix_is_non_destructive() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "d");
        let output = key(&mut commands, &mut document, Key::Escape);
        assert_eq!(output.status, CommandStatus::Cancelled);
        keys(&mut commands, &mut document, "w");
        assert_eq!(document.text(), "one two");

        let output = keys(&mut commands, &mut document, "gz");
        assert!(matches!(output.status, CommandStatus::Unsupported(_)));
        assert_eq!(document.text(), "one two");
    }

    #[test]
    fn open_above_and_below_join_the_insert_to_its_undo_unit() {
        let mut document = Document::new("middle");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "Otop");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "top\nmiddle");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "middle");

        keys(&mut commands, &mut document, "obottom");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "middle\nbottom");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "middle");
    }

    #[test]
    fn character_find_till_repeat_and_reverse_are_grapheme_safe() {
        let mut document = Document::new("aβcβd");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "fβ");
        assert_eq!(commands.cursor(), 1);
        keys(&mut commands, &mut document, ";");
        assert_eq!(commands.cursor(), 4);
        keys(&mut commands, &mut document, ",");
        assert_eq!(commands.cursor(), 1);

        keys(&mut commands, &mut document, "0dtβ");
        assert_eq!(document.text(), "βcβd");
        assert!(is_grapheme_boundary(document.text(), commands.cursor()));
    }

    #[test]
    fn backward_word_end_last_nonblank_column_and_line_offsets_work() {
        let mut document = Document::new("one   \n  two xyz  \nlast");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "wge");
        assert_eq!(commands.cursor(), 2);
        keys(&mut commands, &mut document, "g_");
        assert_eq!(commands.cursor(), 2);
        keys(&mut commands, &mut document, "+");
        assert_eq!(commands.cursor(), 9);
        keys(&mut commands, &mut document, "5|");
        assert_eq!(commands.cursor(), 11);
        keys(&mut commands, &mut document, "-");
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn percent_matches_nested_pairs_and_count_is_document_percentage() {
        let mut document = Document::new("a(b(c)d)e\nsecond\nthird\nfourth");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "%");
        assert_eq!(&document.text()[commands.cursor()..=commands.cursor()], ")");
        keys(&mut commands, &mut document, "%");
        assert_eq!(commands.cursor(), 1);

        keys(&mut commands, &mut document, "50%");
        assert_eq!(commands.cursor(), 10);
    }

    #[test]
    fn forced_mac_normal_motions_use_projected_hard_lines_for_plain_markdown_and_latin1() {
        for format in [Format::PlainText, Format::Markdown] {
            for encoding in [Encoding::Utf8, Encoding::Latin1] {
                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                assert_eq!(document.text(), "a\nb\nc");
                assert_eq!(document.line_count(), 2);

                keys(&mut commands, &mut document, "j");
                assert_eq!(commands.cursor(), 2, "{format:?} {encoding:?}: j");
                keys(&mut commands, &mut document, "l");
                assert_eq!(commands.cursor(), 3, "literal LF is a movable grapheme");
                keys(&mut commands, &mut document, "l");
                assert_eq!(commands.cursor(), 4, "motion continues after literal LF");
                keys(&mut commands, &mut document, "0");
                assert_eq!(commands.cursor(), 2, "0 uses the Mac hard-line start");
                keys(&mut commands, &mut document, "fc");
                assert_eq!(commands.cursor(), 4, "find crosses a literal LF");
                keys(&mut commands, &mut document, "$");
                assert_eq!(commands.cursor(), 4, "$ uses the Mac hard-line end");
                keys(&mut commands, &mut document, "50%");
                assert_eq!(commands.cursor(), 0, "percentage sees two hard lines");
                keys(&mut commands, &mut document, "G");
                assert_eq!(commands.cursor(), 2, "G sees two hard lines");

                assert!(commands.set_cursor(&document, 4));
                keys(&mut commands, &mut document, "ma");
                assert!(commands.set_cursor(&document, 0));
                keys(&mut commands, &mut document, "`a");
                assert_eq!(
                    commands.cursor(),
                    4,
                    "exact mark retains the literal-LF column"
                );
                assert!(commands.set_cursor(&document, 0));
                keys(&mut commands, &mut document, "'a");
                assert_eq!(
                    commands.cursor(),
                    2,
                    "linewise mark uses projected first nonblank"
                );

                commands.jumps.clear();
                commands.jump_index = 0;
                commands.record_jump(&document, 2, 0);
                commands.record_jump(&document, 4, 0);
                assert_eq!(
                    commands.jumps,
                    vec![4, 0],
                    "literal LF does not create a distinct jumplist line"
                );
            }
        }
    }

    #[test]
    fn forced_mac_line_edits_preserve_literal_lf_and_final_line_delete_rules() {
        for format in [Format::PlainText, Format::Markdown] {
            for encoding in [Encoding::Utf8, Encoding::Latin1] {
                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "dd");
                assert_eq!(document.text(), "b\nc", "{format:?} {encoding:?}: first dd");
                assert_eq!(document.line_count(), 1);
                assert_eq!(document.source_bytes(), forced_mac_source("b\nc", format));
                assert_eq!(commands.register('1').unwrap().text, "a\n");
                assert_eq!(commands.register('1').unwrap().hard_break_offsets(), &[1]);

                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "Gdd");
                assert_eq!(document.text(), "a", "{format:?} {encoding:?}: final dd");
                assert_eq!(document.source_bytes(), forced_mac_source("a", format));
                assert_eq!(commands.register('1').unwrap().text, "b\nc\n");
                assert_eq!(commands.register('1').unwrap().hard_break_offsets(), &[3]);
                assert!(document.undo());
                assert_eq!(
                    document.source_bytes(),
                    forced_mac_source("a\rb\nc", format)
                );

                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "GVd");
                assert_eq!(document.text(), "a", "Visual Line final-line deletion");

                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "J");
                assert_eq!(
                    document.text(),
                    "a b\nc",
                    "J removes only the Mac separator"
                );
                assert_eq!(document.line_count(), 1);
                assert_eq!(document.source_bytes(), forced_mac_source("a b\nc", format));

                let mut document = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "G>>");
                assert_eq!(document.text(), "a\n    b\nc");
                assert_eq!(
                    document.source_bytes(),
                    forced_mac_source("a\r    b\nc", format)
                );
            }
        }
    }

    #[test]
    fn forced_mac_register_round_trips_preserve_literal_lf_and_semantic_breaks() {
        for format in [Format::PlainText, Format::Markdown] {
            for encoding in [Encoding::Utf8, Encoding::Latin1] {
                let mut characterwise = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut characterwise, "jv$y");
                let captured = commands.register('0').unwrap();
                assert_eq!(captured.text, "b\nc");
                assert!(captured.hard_break_offsets().is_empty());

                keys(&mut commands, &mut characterwise, "ggP");
                assert_eq!(characterwise.text(), "b\nca\nb\nc");
                assert_eq!(characterwise.line_count(), 2);
                assert_eq!(
                    characterwise.source_bytes(),
                    forced_mac_source("b\nca\rb\nc", format)
                );
                keys(&mut commands, &mut characterwise, "u");
                assert_eq!(
                    characterwise.source_bytes(),
                    forced_mac_source("a\rb\nc", format)
                );

                let mut linewise = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut linewise, "jyy");
                let captured = commands.register('0').unwrap();
                assert_eq!(captured.text, "b\nc\n");
                assert_eq!(captured.hard_break_offsets(), &[3]);

                keys(&mut commands, &mut linewise, "ggP");
                assert_eq!(linewise.text(), "b\nc\na\nb\nc");
                assert_eq!(linewise.line_count(), 3);
                assert_eq!(
                    linewise.source_bytes(),
                    forced_mac_source("b\nc\ra\rb\nc", format)
                );
                keys(&mut commands, &mut linewise, "u");
                assert_eq!(
                    linewise.source_bytes(),
                    forced_mac_source("a\rb\nc", format)
                );

                let mut counted = forced_mac_document(format, encoding);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut counted, "jyygg2P");
                assert_eq!(counted.text(), "b\nc\nb\nc\na\nb\nc");
                assert_eq!(counted.line_count(), 4);
                assert_eq!(
                    counted.source_bytes(),
                    forced_mac_source("b\nc\rb\nc\ra\rb\nc", format)
                );
            }
        }
    }

    #[test]
    fn forced_mac_insert_dot_preserves_semantic_and_literal_line_feeds() {
        let new_document = || {
            Document::from_bytes_with_file_format(
                b"a".to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                FileFormat::Mac,
            )
            .unwrap()
        };
        let graphemes = "e\u{301}👩‍💻";

        let mut semantic = new_document();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut semantic, "A");
        key(&mut commands, &mut semantic, Key::Enter);
        commands
            .handle(&mut semantic, InputEvent::Text(graphemes.to_owned()))
            .unwrap();
        key(&mut commands, &mut semantic, Key::Escape);
        assert_eq!(
            semantic.source_bytes(),
            format!("a\r{graphemes}").as_bytes()
        );
        assert_eq!(
            commands.register('.').unwrap().text,
            format!("\n{graphemes}")
        );
        assert_eq!(commands.register('.').unwrap().hard_break_offsets(), &[0]);

        keys(&mut commands, &mut semantic, "2.");
        assert_eq!(
            semantic.source_bytes(),
            format!("a\r{graphemes}\r{graphemes}\r{graphemes}").as_bytes()
        );
        assert_eq!(semantic.line_count(), 4);
        assert!(semantic.undo(), "counted dot is one undo unit");
        assert_eq!(
            semantic.source_bytes(),
            format!("a\r{graphemes}").as_bytes()
        );

        let mut literal = new_document();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut literal, "A");
        commands
            .handle(&mut literal, InputEvent::Text("\nX".to_owned()))
            .unwrap();
        key(&mut commands, &mut literal, Key::Escape);
        assert_eq!(literal.source_bytes(), b"a\nX");
        assert!(commands
            .register('.')
            .unwrap()
            .hard_break_offsets()
            .is_empty());

        keys(&mut commands, &mut literal, "2.");
        assert_eq!(literal.source_bytes(), b"a\nX\nX\nX");
        assert_eq!(literal.line_count(), 1);
    }

    #[test]
    fn forced_mac_counted_open_line_dot_marks_only_synthetic_separators() {
        let mut document = Document::from_bytes_with_file_format(
            b"ab".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2o");
        commands
            .handle(&mut document, InputEvent::Text("x\ny".to_owned()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.source_bytes(), b"ab\rx\ny\rx\ny");
        assert_eq!(document.line_count(), 3);
        assert!(commands
            .register('.')
            .unwrap()
            .hard_break_offsets()
            .is_empty());

        keys(&mut commands, &mut document, "2.");
        assert_eq!(document.source_bytes(), b"ab\rx\ny\rx\ny\rx\ny\rx\ny");
        assert_eq!(document.line_count(), 5);
    }

    #[test]
    fn forced_mac_change_dot_preserves_structured_insert_payload() {
        let new_document = || {
            Document::from_bytes_with_file_format(
                b"aa bb".to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                FileFormat::Mac,
            )
            .unwrap()
        };

        let mut semantic = new_document();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut semantic, "cw");
        key(&mut commands, &mut semantic, Key::Enter);
        commands
            .handle(&mut semantic, InputEvent::Text("X".to_owned()))
            .unwrap();
        key(&mut commands, &mut semantic, Key::Escape);
        assert_eq!(semantic.source_bytes(), b"\rX bb");
        assert!(commands.set_cursor(&semantic, semantic.text().rfind("bb").unwrap()));
        keys(&mut commands, &mut semantic, ".");
        assert_eq!(semantic.source_bytes(), b"\rX \rX");
        assert_eq!(semantic.line_count(), 3);

        let mut literal = new_document();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut literal, "cw");
        commands
            .handle(&mut literal, InputEvent::Text("\nX".to_owned()))
            .unwrap();
        key(&mut commands, &mut literal, Key::Escape);
        assert_eq!(literal.source_bytes(), b"\nX bb");
        assert!(commands.set_cursor(&literal, literal.text().rfind("bb").unwrap()));
        keys(&mut commands, &mut literal, ".");
        assert_eq!(literal.source_bytes(), b"\nX \nX");
        assert_eq!(literal.line_count(), 1);
    }

    #[test]
    fn structured_dot_encoding_failure_is_atomic_and_preserves_dot_register() {
        let mut document = Document::from_bytes_with_file_format(
            b"ab".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        let payload = RegisterValue::try_new("\n😀", RegisterKind::Characterwise, vec![0]).unwrap();
        commands.last_repeat = Some(RepeatAction::Insert {
            placement: InsertPlacement::Before,
            program: EditSessionProgram {
                // Encoding is preflighted for the complete program, so this
                // earlier destructive intention cannot commit by itself.
                steps: vec![EditSessionStep::Delete, EditSessionStep::Text(payload)],
            },
            count: 1,
        });
        commands
            .registers
            .set_last_insert(RegisterValue::characterwise("seed"));
        let revision = document.revision();
        let repeat = commands.last_repeat.clone();

        let result = commands.handle(&mut document, InputEvent::key('.'));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.source_bytes(), b"ab");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.last_repeat, repeat);
        assert_eq!(commands.register('.').unwrap().text, "seed");
        assert!(!document.undo());
    }

    #[test]
    fn forced_mac_visual_paste_and_insert_ctrl_r_use_structured_register_payloads() {
        let mut visual = forced_mac_document(Format::PlainText, Encoding::Latin1);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut visual, "jv$yggvp");
        assert_eq!(visual.text(), "b\nc\nb\nc");
        assert_eq!(visual.line_count(), 2);
        assert_eq!(visual.source_bytes(), b"b\nc\rb\nc");
        keys(&mut commands, &mut visual, "u");
        assert_eq!(visual.source_bytes(), b"a\rb\nc");

        let mut inserted = forced_mac_document(Format::PlainText, Encoding::Latin1);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut inserted, "jyyggI");
        key(&mut commands, &mut inserted, Key::Ctrl('r'));
        keys(&mut commands, &mut inserted, "0");
        key(&mut commands, &mut inserted, Key::Escape);
        assert_eq!(inserted.text(), "b\nc\na\nb\nc");
        assert_eq!(inserted.line_count(), 3);
        assert_eq!(inserted.source_bytes(), b"b\nc\ra\rb\nc");
        keys(&mut commands, &mut inserted, "u");
        assert_eq!(inserted.source_bytes(), b"a\rb\nc");
    }

    #[test]
    fn forced_mac_ex_yank_put_bridge_preserves_register_break_metadata() {
        let mut document = forced_mac_document(Format::PlainText, Encoding::Utf8);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":2yank");
        key(&mut commands, &mut document, Key::Enter);
        let register = commands.register('0').unwrap();
        assert_eq!(register.text, "b\nc\n");
        assert_eq!(register.hard_break_offsets(), &[3]);

        keys(&mut commands, &mut document, ":1put");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "a\nb\nc\nb\nc");
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.source_bytes(), b"a\rb\nc\rb\nc");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.source_bytes(), b"a\rb\nc");
    }

    #[test]
    fn register_paste_spells_marked_breaks_with_forced_dos_file_format() {
        let new_document = || {
            Document::from_bytes_with_file_format(
                b"a\r\nb".to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                FileFormat::Dos,
            )
            .unwrap()
        };

        let mut normal = new_document();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(
            Some('a'),
            RegisterValue::try_new("x\ny\n", RegisterKind::Linewise, vec![1, 3]).unwrap(),
        );
        keys(&mut commands, &mut normal, "\"aP");
        assert_eq!(normal.text(), "x\ny\na\nb");
        assert_eq!(normal.line_count(), 4);
        assert_eq!(normal.source_bytes(), b"x\r\ny\r\na\r\nb");

        let mut inserted = new_document();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(
            Some('a'),
            RegisterValue::try_new("x\ny", RegisterKind::Characterwise, vec![1]).unwrap(),
        );
        keys(&mut commands, &mut inserted, "i");
        key(&mut commands, &mut inserted, Key::Ctrl('r'));
        keys(&mut commands, &mut inserted, "a");
        key(&mut commands, &mut inserted, Key::Escape);
        assert_eq!(inserted.text(), "x\nya\nb");
        assert_eq!(inserted.line_count(), 3);
        assert_eq!(inserted.source_bytes(), b"x\r\nya\r\nb");
    }

    #[test]
    fn impossible_literal_lf_register_paste_rolls_back_document_and_insert_state() {
        let new_document = || {
            Document::from_bytes_with_file_format(
                b"a\r\nb".to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                FileFormat::Dos,
            )
            .unwrap()
        };
        let literal =
            RegisterValue::try_new("x\ny", RegisterKind::Characterwise, Vec::new()).unwrap();

        let mut normal = new_document();
        let revision = normal.revision();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(None, literal.clone());
        let result = commands.handle(&mut normal, InputEvent::key('p'));
        assert_eq!(result, Err(DocumentError::FormattedPayloadCannotReproject));
        assert_eq!(normal.source_bytes(), b"a\r\nb");
        assert_eq!(normal.revision(), revision);
        assert_eq!(commands.register('"'), Some(&literal));
        assert!(!normal.undo());

        let mut inserted = new_document();
        let revision = inserted.revision();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), literal.clone());
        keys(&mut commands, &mut inserted, "i");
        key(&mut commands, &mut inserted, Key::Ctrl('r'));
        let result = commands.handle(&mut inserted, InputEvent::key('a'));
        assert_eq!(result, Err(DocumentError::FormattedPayloadCannotReproject));
        assert_eq!(inserted.source_bytes(), b"a\r\nb");
        assert_eq!(inserted.revision(), revision);
        assert_eq!(commands.mode(), Mode::Insert);
        assert!(!commands.register_pending);
        key(&mut commands, &mut inserted, Key::Escape);
        assert!(!inserted.undo());
    }

    #[test]
    fn forced_mac_visual_character_replace_changes_literal_lf_but_not_the_separator() {
        let mut document = forced_mac_document(Format::PlainText, Encoding::Latin1);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "jv2lrX");
        assert_eq!(document.text(), "a\nXXX");
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.source_bytes(), b"a\rXXX");
    }

    #[test]
    fn sentence_and_paragraph_motions_have_deterministic_boundaries() {
        let mut document = Document::new("One. Two! Three.\n\nNext paragraph.");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ")");
        assert_eq!(commands.cursor(), 5);
        keys(&mut commands, &mut document, ")");
        assert_eq!(commands.cursor(), 10);
        keys(&mut commands, &mut document, "(");
        assert_eq!(commands.cursor(), 5);
        keys(&mut commands, &mut document, "}");
        assert_eq!(commands.cursor(), 17);
        keys(&mut commands, &mut document, "{");
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn marks_and_jump_list_distinguish_exact_and_linewise_jumps() {
        let mut document = Document::new("  one\n  two\nthree");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2lmaG`a");
        assert_eq!(commands.cursor(), 2);
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 12);
        key(&mut commands, &mut document, Key::Ctrl('i'));
        assert_eq!(commands.cursor(), 2);

        keys(&mut commands, &mut document, "0maG'a");
        assert_eq!(commands.cursor(), 2);
    }

    #[test]
    fn documented_jump_motions_round_trip_through_ctrl_o_and_ctrl_i() {
        for (text, start, command, target) in [
            ("a\nb\nc", 0, "G", 4),
            ("a\nb\nc", 4, "gg", 0),
            ("a\nb\nc\nd", 0, "50%", 2),
            ("(x)", 0, "%", 2),
            ("One. Two.", 0, ")", 5),
            ("One. Two.", 5, "(", 0),
            ("a\n\nb", 0, "}", 2),
            ("a\n\nb", 3, "{", 2),
        ] {
            let mut document = Document::new(text);
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, start));

            keys(&mut commands, &mut document, command);
            assert_eq!(commands.cursor(), target, "{command}");
            assert_eq!(commands.jumps, vec![start, target], "{command}");

            let older = key(&mut commands, &mut document, Key::Ctrl('o'));
            assert_eq!(older.status, CommandStatus::Complete, "{command}");
            assert_eq!(commands.cursor(), start, "{command}");
            let newer = key(&mut commands, &mut document, Key::Ctrl('i'));
            assert_eq!(newer.status, CommandStatus::Complete, "{command}");
            assert_eq!(commands.cursor(), target, "{command}");
        }
    }

    #[test]
    fn search_jump_variants_record_one_origin_and_failures_record_nothing() {
        let fixtures = [
            ("*", 0, 12, None),
            ("#", 12, 0, None),
            ("g*", 0, 5, None),
            ("g#", 12, 5, None),
            (
                "n",
                0,
                5,
                Some((SearchDirection::Forward, "cat".to_owned())),
            ),
            (
                "N",
                12,
                5,
                Some((SearchDirection::Forward, "cat".to_owned())),
            ),
        ];
        for (command, start, target, last_search) in fixtures {
            let mut document = Document::new("cat scatter cat");
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, start));
            commands.last_search = last_search;

            keys(&mut commands, &mut document, command);
            assert_eq!(commands.cursor(), target, "{command}");
            assert_eq!(commands.jumps, vec![start, target], "{command}");
        }

        for (leader, start, target) in [('/', 0, 5), ('?', 12, 5)] {
            let mut document = Document::new("cat scatter cat");
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, start));
            key(&mut commands, &mut document, Key::Char(leader));
            commands
                .handle(&mut document, InputEvent::text("cat"))
                .unwrap();
            key(&mut commands, &mut document, Key::Enter);
            assert_eq!(commands.cursor(), target, "{leader}");
            assert_eq!(commands.jumps, vec![start, target], "{leader}");
        }

        let mut document = Document::new("one line");
        let mut commands = CommandInterpreter::new();
        for command in ["G", "gg", "%", "`z"] {
            let output = keys(&mut commands, &mut document, command);
            assert!(
                !output.cursor_moved,
                "failed or no-op jump {command} unexpectedly moved"
            );
            assert!(commands.jumps.is_empty(), "{command}");
        }
        keys(&mut commands, &mut document, "/missing");
        let missing = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(missing.status, CommandStatus::SearchNotFound);
        assert!(commands.jumps.is_empty());
    }

    #[test]
    fn jumplist_truncates_newer_branches_deduplicates_lines_and_clamps_counts() {
        let mut document = Document::new("a0\nb1\nc2\nd3\ne4");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "G2G");
        assert_eq!(commands.jumps, vec![0, 12, 3]);

        keys(&mut commands, &mut document, "2");
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut document, Key::Ctrl('i'));
        assert_eq!(commands.cursor(), 12);

        keys(&mut commands, &mut document, "4G");
        assert_eq!(commands.cursor(), 9);
        assert_eq!(commands.jumps, vec![0, 12, 9]);
        let no_newer = key(&mut commands, &mut document, Key::Ctrl('i'));
        assert!(matches!(no_newer.status, CommandStatus::Error(_)));
        assert_eq!(commands.cursor(), 9);

        keys(&mut commands, &mut document, "99");
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 0);
        let no_older = key(&mut commands, &mut document, Key::Ctrl('o'));
        assert!(matches!(no_older.status, CommandStatus::Error(_)));
        assert_eq!(commands.cursor(), 0);
        keys(&mut commands, &mut document, "99");
        key(&mut commands, &mut document, Key::Ctrl('i'));
        assert_eq!(commands.cursor(), 9);

        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Ggg4G");
        assert_eq!(commands.jumps, vec![12, 0, 9]);
        let remembered_lines = commands.jumps[..commands.jumps.len() - 1]
            .iter()
            .map(|offset| line_start(document.text(), *offset))
            .collect::<Vec<_>>();
        assert_eq!(remembered_lines, vec![12, 0]);

        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "G2G");
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 12);
        keys(&mut commands, &mut document, "l");
        assert_eq!(commands.cursor(), 13);
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut document, Key::Ctrl('i'));
        assert_eq!(
            commands.cursor(),
            12,
            "motion during traversal does not rewrite a historical jump"
        );
    }

    #[test]
    fn jumplist_keeps_one_hundred_origins_plus_the_live_position() {
        let text = (1..=110)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut document = Document::new(text);
        let mut commands = CommandInterpreter::new();

        for line in 2..=110 {
            keys(&mut commands, &mut document, &format!("{line}G"));
        }
        assert_eq!(commands.jumps.len(), 101);
        assert_eq!(commands.jump_index, 100);
        assert_eq!(
            line_start(document.text(), commands.jumps[0]),
            nth_line_start(document.text(), 10)
        );
        keys(&mut commands, &mut document, "200");
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.jump_index, 0);
    }

    #[test]
    fn viewport_h_m_l_are_jumps_but_page_scrolling_is_not() {
        let text = "  a\n  b\n  c\n  d\n  e";
        for (command, target) in [('H', 6), ('M', 10), ('L', 14)] {
            let mut document = Document::new(text);
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, 2));
            let snapshot = layout_snapshot(&document, 500.0);
            let row_height = snapshot.rows[0].height();
            let mut context = LayoutCommandContext::new(
                &snapshot,
                true,
                Viewport::new(snapshot.rows[1].y, row_height * 3.0).unwrap(),
            );

            layout_key(
                &mut commands,
                &mut document,
                &mut context,
                Key::Char(command),
            );
            assert_eq!(commands.cursor(), target, "{command}");
            assert_eq!(commands.jumps, vec![2, target], "{command}");
        }

        let mut document = Document::new(text);
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let row_height = snapshot.rows[0].height();
        let mut context = LayoutCommandContext::new(
            &snapshot,
            true,
            Viewport::new(0.0, row_height * 2.0).unwrap(),
        );
        layout_key(&mut commands, &mut document, &mut context, Key::PageDown);
        assert!(commands.jumps.is_empty());
    }

    #[test]
    fn operator_pending_viewport_motions_are_inclusive_and_linewise() {
        let text = (0..9)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(snapshot.rows[2].y, row_height * 5.0).unwrap();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[6].text_range.start));
        let mut context = LayoutCommandContext::new(&snapshot, true, viewport);
        keys(&mut commands, &mut document, "\"ay");
        let yank = layout_key(&mut commands, &mut document, &mut context, Key::Char('H'));
        assert_eq!(yank.status, CommandStatus::Complete);
        assert!(!yank.document_changed);
        assert!(yank.cursor_moved);
        assert_eq!(document.text(), text);
        assert_eq!(commands.cursor(), snapshot.rows[2].text_range.start);
        assert_eq!(commands.register('a').unwrap().kind, RegisterKind::Linewise);
        assert_eq!(commands.register('a').unwrap().text, "2\n3\n4\n5\n6\n");

        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context = LayoutCommandContext::new(&snapshot, true, viewport);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[6].text_range.start));
        key(&mut commands, &mut document, Key::Char('d'));
        let delete = layout_key(&mut commands, &mut document, &mut context, Key::Char('M'));
        assert!(delete.document_changed);
        assert_eq!(document.text(), "0\n1\n2\n3\n7\n8");
        assert_eq!(commands.register('1').unwrap().text, "4\n5\n6\n");

        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context = LayoutCommandContext::new(&snapshot, true, viewport);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[2].text_range.start));
        key(&mut commands, &mut document, Key::Char('c'));
        let change = layout_key(&mut commands, &mut document, &mut context, Key::Char('L'));
        assert!(change.document_changed);
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(document.text(), "0\n1\n\n7\n8");
        assert_eq!(commands.register('1').unwrap().text, "2\n3\n4\n5\n6\n");
    }

    #[test]
    fn operator_pending_viewport_motions_honor_counts_and_wrapped_hard_lines() {
        let text = (0..9)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(snapshot.rows[2].y, row_height * 5.0).unwrap();
        let mut context = LayoutCommandContext::new(&snapshot, true, viewport);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[6].text_range.start));
        keys(&mut commands, &mut document, "\"a2y2");
        layout_key(&mut commands, &mut document, &mut context, Key::Char('H'));
        assert_eq!(commands.register('a').unwrap().text, "5\n6\n");
        assert_eq!(
            commands.cursor(),
            snapshot.rows[5].text_range.start,
            "operator and motion counts multiply before H applies its inset"
        );

        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context = LayoutCommandContext::new(&snapshot, true, viewport);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[6].text_range.start));
        keys(&mut commands, &mut document, "9y");
        layout_key(&mut commands, &mut document, &mut context, Key::Char('M'));
        assert_eq!(
            commands.register('0').unwrap().text,
            "4\n5\n6\n",
            "M ignores a count, including an operator count"
        );

        let mut document =
            Document::new("alpha beta gamma delta epsilon zeta eta theta iota kappa\nTAIL\nEND");
        let snapshot = layout_snapshot(&document, 45.0);
        assert!(
            snapshot
                .rows
                .iter()
                .take(3)
                .all(|row| row.hard_line_index == 0),
            "fixture must wrap its first hard line"
        );
        let row_height = snapshot.rows[0].height();
        let mut context = LayoutCommandContext::new(
            &snapshot,
            true,
            Viewport::new(0.0, row_height * 4.0).unwrap(),
        );
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, snapshot.rows[2].text_range.start));
        key(&mut commands, &mut document, Key::Char('d'));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('H'));
        assert_eq!(document.text(), "TAIL\nEND");
        assert_eq!(
            commands.register('1').unwrap().text,
            "alpha beta gamma delta epsilon zeta eta theta iota kappa\n",
            "a visual-row destination is promoted to its inclusive hard-line extent"
        );
    }

    #[test]
    fn counted_ctrl_d_u_use_and_remember_an_exact_visual_row_amount() {
        let text = (0..20)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut document = Document::new(&text);
        let snapshot = layout_snapshot(&document, 500.0);
        let row_height = snapshot.rows[0].height();
        let start_row = 5;
        let start = snapshot.rows[start_row].text_range.start;
        let initial_viewport = Viewport::new(snapshot.rows[start_row].y, row_height * 6.0).unwrap();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, start));
        let mut context = LayoutCommandContext::new(&snapshot, true, initial_viewport);

        layout_key(&mut commands, &mut document, &mut context, Key::Char('3'));
        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('d'));
        assert_eq!(
            commands.cursor(),
            snapshot.rows[start_row + 3].text_range.start
        );
        assert_eq!(context.viewport.top, snapshot.rows[start_row + 3].y);

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('u'));
        assert_eq!(commands.cursor(), start);
        assert_eq!(context.viewport, initial_viewport);

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('d'));
        assert_eq!(
            commands.cursor(),
            snapshot.rows[start_row + 3].text_range.start
        );

        let mut fresh = CommandInterpreter::new();
        assert!(fresh.set_cursor(&document, start));
        let mut fresh_context = LayoutCommandContext::new(&snapshot, true, initial_viewport);
        layout_key(
            &mut fresh,
            &mut document,
            &mut fresh_context,
            Key::Ctrl('d'),
        );
        assert_eq!(
            fresh.cursor(),
            snapshot.rows[start_row + 3].text_range.start,
            "an unset amount defaults to half of six visible rows"
        );
    }

    #[test]
    fn counted_ctrl_d_u_preserve_a_visual_block_across_wrapped_rows() {
        let mut document = Document::new(
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen",
        );
        let snapshot = layout_snapshot(&document, 45.0);
        assert!(snapshot.rows.len() >= 7);
        let row_height = snapshot.rows[0].height();
        let initial_viewport = Viewport::new(0.0, row_height * 4.0).unwrap();
        let mut context = LayoutCommandContext::new(&snapshot, true, initial_viewport);
        let mut commands = CommandInterpreter::new();
        let origin = commands.current_visual_position(&snapshot).unwrap();
        let expected = gj(&snapshot, origin, 2, None).unwrap().position;

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('2'));
        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('d'));
        assert_eq!(commands.mode(), Mode::VisualBlock);
        assert_eq!(
            commands.current_visual_position(&snapshot).unwrap(),
            expected
        );
        assert_eq!(context.viewport.top, snapshot.rows[2].y);

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('u'));
        assert_eq!(commands.mode(), Mode::VisualBlock);
        assert_eq!(commands.current_visual_position(&snapshot).unwrap(), origin);
        assert_eq!(context.viewport, initial_viewport);
    }

    #[test]
    fn counted_z_targets_a_hard_line_before_aligning_the_viewport() {
        let text = "one\ntwo\nthree\n  four\nfive\nsix\nseven";
        for (command, alignment) in [
            ('t', ViewportAlignment::Top),
            ('z', ViewportAlignment::Middle),
            ('b', ViewportAlignment::Bottom),
        ] {
            let mut document = Document::new(text);
            let mut commands = CommandInterpreter::new();
            let snapshot = layout_snapshot(&document, 500.0);
            let row_height = snapshot.rows[0].height();
            let initial = Viewport::new(0.0, row_height * 3.0).unwrap();
            let target = nth_line_start(document.text(), 4) + 2;
            let expected = align_viewport(
                &snapshot,
                VisualPosition {
                    text_offset: target,
                    affinity: BoundaryAffinity::Downstream,
                },
                initial,
                alignment,
            )
            .unwrap();
            let mut context = LayoutCommandContext::new(&snapshot, true, initial);

            for key in [Key::Char('4'), Key::Char('z'), Key::Char(command)] {
                let output = layout_key(&mut commands, &mut document, &mut context, key);
                assert!(matches!(
                    output.status,
                    CommandStatus::Pending | CommandStatus::Complete
                ));
            }
            assert_eq!(commands.cursor(), target, "4z{command}");
            assert_eq!(context.viewport, expected, "4z{command}");
        }
    }

    #[test]
    fn counted_g_dollar_uses_the_requested_visual_row_in_all_visual_modes() {
        for entry in [None, Some(Key::Char('v')), Some(Key::Ctrl('v'))] {
            let mut document = Document::new("one two three four five");
            let mut commands = CommandInterpreter::new();
            let snapshot = layout_snapshot(&document, 45.0);
            assert!(snapshot.rows.len() >= 3);
            let initial = commands.current_visual_position(&snapshot).unwrap();
            let third_row = gj(&snapshot, initial, 2, None).unwrap().position;
            let expected = layout_motion::g_dollar(&snapshot, third_row).unwrap();
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            if let Some(entry) = entry {
                layout_key(&mut commands, &mut document, &mut context, entry);
            }
            for key in [Key::Char('3'), Key::Char('g'), Key::Char('$')] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }

            assert_eq!(
                commands.current_visual_position(&snapshot).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn overflowing_layout_motion_count_does_not_move() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for _ in 0..128 {
            layout_key(&mut commands, &mut document, &mut context, Key::Char('9'));
        }
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('j'));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn substitute_jump_origin_uses_the_committed_position_map() {
        let mut document = Document::new("aa\nmiddle");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 5));
        assert!(matches!(
            parse_ex("%s/a/ZZ/").unwrap().action,
            ExAction::Substitute(_)
        ));
        let origin_anchor = TextAnchor::new(
            document.text_point(5).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );

        keys(&mut commands, &mut document, ":%s/a/ZZ/");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert!(output.document_changed);
        assert_eq!(
            mapped_anchor(
                output
                    .ex_outcome
                    .as_ref()
                    .unwrap()
                    .model_transaction()
                    .unwrap()
                    .text_position_map(),
                origin_anchor,
            )
            .unwrap(),
            Some(6)
        );
        assert_eq!(document.text(), "ZZa\nmiddle");
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.jumps, vec![6, 0]);

        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 6);
        key(&mut commands, &mut document, Key::Ctrl('i'));
        assert_eq!(commands.cursor(), 0);

        let before_jumps = commands.jumps.clone();
        keys(&mut commands, &mut document, ":s/missing/x/");
        let failure = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(failure.status, CommandStatus::ExError(_)));
        assert_eq!(commands.jumps, before_jumps);

        let mut document = Document::new("aa xx");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 4));
        keys(&mut commands, &mut document, ":s/a/ZZ/");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            commands.jumps,
            vec![5, 0],
            "an origin after the substituted character follows the exact committed splice"
        );
    }

    #[test]
    fn operator_jump_origins_publish_only_after_successful_execution() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 4));
        keys(&mut commands, &mut document, "yG");
        assert_eq!(commands.cursor(), 4);
        assert_eq!(commands.jumps, vec![4, 4]);

        let mut document = Document::new("One. Two.");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "d)");
        assert_eq!(document.text(), "Two.");
        assert_eq!(commands.jumps, vec![0, 0]);

        use crate::document::{Encoding, Format};
        let mut document = Document::from_bytes(
            vec![0xff, b'.', b' ', b'a'],
            Encoding::Latin1,
            Format::PlainText,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        let revision = document.revision();
        keys(&mut commands, &mut document, "gU");
        let result = commands.handle(&mut document, InputEvent::key(')'));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "ÿ. a");
        assert_eq!(document.revision(), revision);
        assert!(commands.jumps.is_empty());
    }

    #[test]
    fn star_search_observes_whole_word_while_gstar_allows_substrings() {
        let mut document = Document::new("cat scatter cat");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "*");
        assert_eq!(commands.cursor(), 12);
        keys(&mut commands, &mut document, "0g*");
        assert_eq!(commands.cursor(), 5);
        assert!(commands.set_cursor(&document, 12));
        keys(&mut commands, &mut document, "g#");
        assert_eq!(commands.cursor(), 5);
    }

    #[test]
    fn gv_restores_the_last_directed_visual_selection() {
        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "v2l");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.mode(), Mode::Normal);
        keys(&mut commands, &mut document, "gv");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 2);
    }

    #[test]
    fn gp_leaves_the_cursor_after_characterwise_inserted_text() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "yllgp");
        assert_eq!(document.text(), "abac");
        assert_eq!(commands.cursor(), 3);
    }

    #[test]
    fn normalized_macro_events_replay_atomically_and_at_at_reuses_register() {
        let mut document = Document::new("x");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "qaA!");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "q@a");
        assert_eq!(document.text(), "x!!");
        keys(&mut commands, &mut document, "@@");
        assert_eq!(document.text(), "x!!!");

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "x!!");
    }

    #[test]
    fn non_text_macro_keys_replay_but_cannot_be_put_as_inspection_notation() {
        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('a', vec![InputEvent::Key(Key::Left)]);
        assert_eq!(commands.register('a').unwrap().text, "<Left>");
        assert!(commands.set_cursor(&document, 1));
        let revision = document.revision();

        let rejected = keys(&mut commands, &mut document, "\"ap");
        assert_eq!(
            rejected.status,
            CommandStatus::RegisterReadError(RegisterReadError::MacroContainsNonTextKeys {
                register: 'a',
            })
        );
        assert_eq!(document.text(), "ab");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.cursor(), 1);
        assert!(!document.undo());

        let replayed = keys(&mut commands, &mut document, "@a");
        assert_eq!(replayed.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), 0, "playback uses the stored Left event");

        commands
            .registers
            .yank(Some('b'), RegisterValue::characterwise("<Left>"));
        let literal = keys(&mut commands, &mut document, "\"bp");
        assert_eq!(literal.status, CommandStatus::Complete);
        assert_eq!(document.text(), "a<Left>b");
    }

    #[test]
    fn macro_enter_puts_a_literal_carriage_return_not_a_hard_break() {
        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('a', vec![InputEvent::Key(Key::Enter)]);
        assert_eq!(commands.register('a').unwrap().text, "<Enter>");

        let put = keys(&mut commands, &mut document, "\"ap");
        assert_eq!(put.status, CommandStatus::Complete);
        assert_eq!(document.text(), "a\rb");
        assert_eq!(document.hard_line_snapshot().line_count(), 1);
        assert!(commands
            .register('a')
            .unwrap()
            .hard_break_offsets()
            .is_empty());

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "ab");

        let mut mac = Document::from_bytes_with_file_format(
            b"a\rb".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('a', vec![InputEvent::Key(Key::Enter)]);
        keys(&mut commands, &mut mac, "\"a");
        let rejected = commands.handle(&mut mac, InputEvent::key('p'));
        assert_eq!(
            rejected,
            Err(DocumentError::UnrepresentableFormattedCharacter {
                format: Format::PlainText,
                character: '\r',
            })
        );
        assert_eq!(mac.source_bytes(), b"a\rb");
        assert!(!mac.undo());
    }

    #[test]
    fn macro_put_rejection_is_shared_by_ctrl_r_and_ex_put() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('a', vec![InputEvent::Key(Key::Down)]);
        let revision = document.revision();

        keys(&mut commands, &mut document, "i");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        let ctrl_r = keys(&mut commands, &mut document, "a");
        assert_eq!(
            ctrl_r.status,
            CommandStatus::RegisterReadError(RegisterReadError::MacroContainsNonTextKeys {
                register: 'a',
            })
        );
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(document.revision(), revision);
        key(&mut commands, &mut document, Key::Escape);

        keys(&mut commands, &mut document, ":put a");
        let ex_put = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            ex_put.status,
            CommandStatus::RegisterReadError(RegisterReadError::MacroContainsNonTextKeys {
                register: 'a',
            })
        );
        assert_eq!(document.text(), "one\ntwo");
        assert_eq!(document.revision(), revision);
        assert!(!document.undo());
    }

    #[test]
    fn operator_pending_text_objects_support_counts_and_change_grouping() {
        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "d2iw");
        assert_eq!(document.text(), " three");
        assert_eq!(commands.register('"').unwrap().text, "one two");

        let mut document = Document::new("say \"hello\" now");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "f\"lci\"");
        assert_eq!(commands.mode(), Mode::Insert);
        keys(&mut commands, &mut document, "world");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "say \"world\" now");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "say \"hello\" now");
    }

    #[test]
    fn pair_text_object_aliases_all_reach_the_shared_resolver() {
        for (text, aliases, expected) in [
            ("(inside)", vec!['(', ')', 'b'], "()"),
            ("[inside]", vec!['[', ']'], "[]"),
            ("{inside}", vec!['{', '}', 'B'], "{}"),
            ("<inside>", vec!['<', '>'], "<>"),
        ] {
            for alias in aliases {
                let mut document = Document::new(text);
                let mut commands = CommandInterpreter::new();
                keys(&mut commands, &mut document, "l");
                keys(&mut commands, &mut document, "di");
                key(&mut commands, &mut document, Key::Char(alias));
                assert_eq!(document.text(), expected, "alias {alias}");
            }
        }
    }

    #[test]
    fn text_objects_compose_with_yank_indent_and_case_operators() {
        let mut document = Document::new("word next");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "yiw");
        assert_eq!(commands.register('0').unwrap().text, "word");
        keys(&mut commands, &mut document, "gUiw");
        assert_eq!(document.text(), "WORD next");

        let mut document = Document::new("first line\nsecond line\n\nthird");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ">ip");
        assert_eq!(document.text(), "    first line\n    second line\n\nthird");
    }

    #[test]
    fn visual_inner_and_around_objects_replace_the_old_selection() {
        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "v2l");
        keys(&mut commands, &mut document, "wiw");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(4));
        assert_eq!(commands.cursor(), 6);
        keys(&mut commands, &mut document, "d");
        assert_eq!(document.text(), "one  three");

        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "wvawy");
        assert_eq!(commands.register('0').unwrap().text, "two ");
    }

    #[test]
    fn insert_ctrl_w_and_ctrl_u_are_grapheme_safe_and_share_the_session_undo() {
        let mut document = Document::new("base");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "A one a\u{301}bc");
        key(&mut commands, &mut document, Key::Ctrl('w'));
        assert_eq!(document.text(), "base one ");
        key(&mut commands, &mut document, Key::Ctrl('u'));
        assert_eq!(document.text(), "base");
        keys(&mut commands, &mut document, "done");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "basedone");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "base");
    }

    #[test]
    fn insert_ctrl_o_waits_for_one_complete_normal_command_and_splits_undo() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "i");
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.mode(), Mode::Normal);
        keys(&mut commands, &mut document, "d");
        assert_eq!(commands.mode(), Mode::Normal);
        keys(&mut commands, &mut document, "w");
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(document.text(), "two");
        keys(&mut commands, &mut document, "new ");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "new two");

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "two");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "one two");
    }

    #[test]
    fn uppercase_macro_register_appends_normalized_events() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "qaxq");
        assert_eq!(document.text(), "bcdef");
        keys(&mut commands, &mut document, "qAxq");
        assert_eq!(document.text(), "cdef");
        keys(&mut commands, &mut document, "@a");
        assert_eq!(document.text(), "ef");
    }

    #[test]
    fn maximum_count_noop_macro_is_rejected_by_typed_event_budget() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('a', vec![InputEvent::key('h')]);
        let revision = document.revision();

        let output = keys(&mut commands, &mut document, &format!("{}@a", usize::MAX));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::ReplayEventBudgetExceeded {
                count: usize::MAX,
                events_per_iteration: 1,
                limit: MACRO_REPLAY_EVENT_LIMIT,
            })
        );
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.last_macro, None);
        assert_eq!(document.text(), "abc");
        assert_eq!(document.revision(), revision);
        assert!(!document.undo());
    }

    #[test]
    fn ex_command_line_edits_at_grapheme_boundaries_and_recalls_history() {
        let mut document = Document::new("text");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ":");
        assert_eq!(commands.command_line_kind(), Some(CommandLineKind::Ex));
        commands
            .handle(&mut document, InputEvent::text("set wrxp"))
            .unwrap();
        key(&mut commands, &mut document, Key::Left);
        key(&mut commands, &mut document, Key::Left);
        key(&mut commands, &mut document, Key::Delete);
        keys(&mut commands, &mut document, "a");
        assert_eq!(commands.command_line(), Some("set wrap"));
        assert_eq!(commands.command_line_cursor(), Some(7));
        key(&mut commands, &mut document, Key::Enter);
        assert!(commands.wrap_option());

        keys(&mut commands, &mut document, ":");
        key(&mut commands, &mut document, Key::Up);
        assert_eq!(commands.command_line(), Some("set wrap"));
        key(&mut commands, &mut document, Key::Down);
        assert_eq!(commands.command_line(), Some(""));
        key(&mut commands, &mut document, Key::Escape);
    }

    #[test]
    fn search_and_ex_command_lines_keep_independent_histories() {
        let mut document = Document::new("one two one");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "/one");
        key(&mut commands, &mut document, Key::Enter);
        keys(&mut commands, &mut document, ":set wrap");
        key(&mut commands, &mut document, Key::Enter);

        keys(&mut commands, &mut document, "/");
        key(&mut commands, &mut document, Key::Up);
        assert_eq!(commands.command_line(), Some("one"));
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, ":");
        key(&mut commands, &mut document, Key::Up);
        assert_eq!(commands.command_line(), Some("set wrap"));
    }

    #[test]
    fn ex_delete_commits_atomically_then_publishes_register_effects() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut commands = CommandInterpreter::new();
        commands.registers.delete(
            None,
            RegisterValue::linewise("older\n"),
            DeletionClass::Large,
        );

        keys(&mut commands, &mut document, ":2delete a");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "one\nthree");
        assert_eq!(commands.register('a').unwrap().text, "two\n");
        assert_eq!(commands.register('1').unwrap().text, "two\n");
        assert_eq!(commands.register('2').unwrap().text, "older\n");
        assert!(output.document_changed);
        assert_eq!(
            output.ex_outcome.as_ref().unwrap().register_effects.len(),
            1
        );

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "one\ntwo\nthree");
    }

    #[test]
    fn ex_yank_shares_normal_register_zero_policy() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(None, RegisterValue::characterwise("seed"));

        keys(&mut commands, &mut document, ":1yank a");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.register('a').unwrap().text, "one\n");
        assert_eq!(commands.register('0').unwrap().text, "seed");
        assert_eq!(commands.register('"').unwrap().text, "one\n");

        keys(&mut commands, &mut document, ":2yank");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.register('0').unwrap().text, "two\n");
        assert_eq!(commands.register('"').unwrap().text, "two\n");
    }

    #[test]
    fn ex_option_navigation_and_frontend_results_remain_typed() {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();

        keys(
            &mut commands,
            &mut document,
            ":set wrap ff=dos ffs=mac,unix",
        );
        let options = key(&mut commands, &mut document, Key::Enter);
        assert!(commands.wrap_option());
        assert_eq!(
            commands.fileformats_option(),
            &[FileFormat::Mac, FileFormat::Unix]
        );
        assert_eq!(document.file_format(), FileFormat::Dos);
        assert_eq!(options.ex_outcome.as_ref().unwrap().option_effects.len(), 3);

        keys(&mut commands, &mut document, ":2");
        let navigation = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.cursor(), 4);
        assert_eq!(
            navigation.ex_outcome.as_ref().unwrap().navigation,
            Some(ExNavigation::TextOffset(4))
        );

        keys(&mut commands, &mut document, ":write! copy.txt");
        let request = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(
            &request.ex_outcome.as_ref().unwrap().frontend_requests[0],
            ex_execute::ExFrontendRequest::File(ex_execute::ExFileRequest::Write {
                path: Some(path),
                force: true,
                ..
            }) if path == "copy.txt"
        ));
    }

    #[test]
    fn ex_normal_executes_in_core_over_a_stable_range_and_groups_undo() {
        let mut document = Document::new("one\ntwo\n三");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ":%normal! A!");
        let output = key(&mut commands, &mut document, Key::Enter);

        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.document_changed);
        assert_eq!(document.text(), "one!\ntwo!\n三!");
        assert_eq!(commands.mode(), Mode::Normal);
        assert!(output
            .ex_outcome
            .as_ref()
            .unwrap()
            .frontend_requests
            .is_empty());
        assert!(document.undo(), "the complete range is one undo unit");
        assert_eq!(document.text(), "one\ntwo\n三");
        assert!(!document.undo());

        let mut document = Document::new("a\nb\nc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":%normal! OX");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            document.text(),
            "X\na\nX\nb\nX\nc",
            "new lines do not become additional range targets"
        );

        let mut document = Document::new("a\nb\nc\nd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":1,3normal! dd");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            document.text(),
            "d",
            "each originally targeted stable line is visited once"
        );
    }

    #[test]
    fn ex_normal_aborts_incomplete_commands_and_retains_a_failing_prefix() {
        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ":%normal! d");
        let incomplete = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(incomplete.status, CommandStatus::Complete);
        assert_eq!(document.text(), "ab\ncd");
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.pending, Pending::None);

        keys(&mut commands, &mut document, ":%normal! xQ");
        let failure = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(failure.status, CommandStatus::Unsupported(_)));
        assert_eq!(
            document.text(),
            "b\ncd",
            "the successful prefix remains while later range targets are not visited"
        );
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.pending, Pending::None);
        assert!(
            document.undo(),
            "the retained prefix is still one undo unit"
        );
        assert_eq!(document.text(), "ab\ncd");
        assert!(!document.undo());
    }

    #[test]
    fn ex_normal_mapping_and_layout_boundaries_are_explicit() {
        let mut document = Document::new("text");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":normal A!");
        let non_literal = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(non_literal.status, CommandStatus::Complete);
        assert_eq!(
            document.text(),
            "text!",
            "without a mapping layer, non-bang uses the same built-in path as normal!"
        );

        keys(&mut commands, &mut document, ":normal! H");
        let layout_only = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(
            layout_only.status,
            CommandStatus::Unsupported(ref message) if message.contains("viewport command")
        ));
        assert_eq!(document.text(), "text!");
    }

    #[test]
    fn ex_normal_composes_an_exact_outer_position_map() {
        let mut document = Document::new("one\ntwo\n三");
        let second_line = TextAnchor::new(
            document.text_point(4).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );
        let source_revision = document.revision();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":%normal! A!");

        let (result, map) = document.capture_position_maps(|document| {
            commands.handle(document, InputEvent::Key(Key::Enter))
        });
        let output = result.unwrap();

        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(map.source_revision(), source_revision);
        assert_eq!(map.target_revision(), document.revision());
        assert_eq!(
            map.map_text_anchor(second_line)
                .unwrap()
                .value()
                .unwrap()
                .offset(),
            5
        );
        assert_eq!(document.text(), "one!\ntwo!\n三!");
    }

    #[test]
    fn ex_normal_encoding_failures_are_event_atomic_but_keep_prior_events() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"a\nb".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":%normal! A😀");
        let failure = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(failure.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "a\nb");
        assert_eq!(document.source_bytes(), b"a\nb");
        assert!(!document.undo());
        assert_eq!(commands.mode(), Mode::Normal);

        let mut document =
            Document::from_bytes(b"a\nb".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":%normal! Aé😀");
        let failure = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(failure.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "aé\nb");
        assert_eq!(document.source_bytes(), &[b'a', 0xe9, b'\n', b'b']);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"a\nb");

        let mut document = Document::new("é\nβ");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ":%normal! A😀");
        let success = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(success.status, CommandStatus::Complete);
        assert_eq!(document.text(), "é😀\nβ😀");
    }

    #[test]
    fn macro_document_errors_retain_controller_and_document_prefixes() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"abc".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.registers.set_macro(
            'a',
            vec![
                InputEvent::key('i'),
                InputEvent::text("é"),
                InputEvent::text("😀"),
            ],
        );

        let output = keys(&mut commands, &mut document, "@a");
        assert!(matches!(
            output.status,
            CommandStatus::Error(ref message) if message.contains("successful prefix")
        ));
        assert_eq!(document.text(), "éabc");
        assert_eq!(document.source_bytes(), &[0xe9, b'a', b'b', b'c']);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.pending, Pending::None);
        assert!(document.undo());
        assert_eq!(document.text(), "abc");
        assert!(!document.undo());

        keys(&mut commands, &mut document, ".");
        assert_eq!(
            document.text(),
            "éabc",
            "Escape finalizes the successful insert prefix for dot replay"
        );

        let mut document =
            Document::from_bytes(b"abc".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .set_macro('b', vec![InputEvent::key('i'), InputEvent::text("é😀")]);
        let output = keys(&mut commands, &mut document, "@b");
        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "abc", "one text event commits atomically");
        assert!(!document.undo());
    }

    #[test]
    fn macro_and_ex_normal_share_a_bounded_recursion_guard() {
        let mut document = Document::new("text");
        let mut commands = CommandInterpreter::new();
        let mut events = vec![InputEvent::key(':')];
        events.extend("normal! @a".chars().map(InputEvent::key));
        events.push(InputEvent::Key(Key::Enter));
        commands.registers.set_macro('a', events);

        let output = keys(&mut commands, &mut document, "@a");
        assert!(matches!(
            output.status,
            CommandStatus::Error(ref message)
                if message.contains("recursion limit") && message.contains("32")
        ));
        assert_eq!(document.text(), "text");
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.pending, Pending::None);
        assert_eq!(commands.compound_replay_depth, 0);
        assert!(!document.undo());
    }

    #[test]
    fn ex_parse_and_execution_errors_are_typed_and_non_destructive() {
        let mut document = Document::new("unchanged");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, ":notacommand");
        let parse = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(
            parse.status,
            CommandStatus::ExError(ExCommandError::Parse(_))
        ));
        assert_eq!(document.text(), "unchanged");

        keys(&mut commands, &mut document, ":put z");
        let execute = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(
            execute.status,
            CommandStatus::ExError(ExCommandError::Execute(ExExecuteError::EmptyRegister(
                Some('z')
            )))
        ));
        assert_eq!(document.text(), "unchanged");
    }

    #[test]
    fn normal_backspace_is_a_motion_while_upper_x_is_a_delete() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "2l");

        let moved = key(&mut commands, &mut document, Key::Backspace);
        assert!(moved.cursor_moved);
        assert!(!moved.document_changed);
        assert_eq!(commands.cursor(), 1);
        assert_eq!(document.text(), "abc");

        keys(&mut commands, &mut document, "X");
        assert_eq!(document.text(), "bc");
    }

    #[test]
    fn operator_visual_row_motions_require_layout_and_stay_characterwise() {
        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 55.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        for command in ['d', 'g', 'j'] {
            layout_key(
                &mut commands,
                &mut document,
                &mut context,
                Key::Char(command),
            );
        }
        assert_eq!(document.text(), "two three");
        assert!(document.undo());
        assert_eq!(document.text(), "one two three");

        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        let headless = keys(&mut commands, &mut document, "dgj");
        assert!(matches!(
            headless.status,
            CommandStatus::Unsupported(message) if message.contains("requires layout context")
        ));
        assert_eq!(document.text(), "one two three");
    }

    #[test]
    fn operator_visual_row_edges_use_exact_soft_row_boundaries() {
        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 6));
        let snapshot = layout_snapshot(&document, 55.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        for command in ['d', 'g', '0'] {
            layout_key(
                &mut commands,
                &mut document,
                &mut context,
                Key::Char(command),
            );
        }
        assert_eq!(document.text(), "one o three");
    }

    #[test]
    fn control_q_alias_preserves_block_counts_registers_and_atomic_operators() {
        for control in ['q', 'Q', 'v', 'V'] {
            for operator in ['d', 'y', 'r'] {
                let mut document = Document::new("abcd\nefgh\nopqr");
                let mut commands = CommandInterpreter::new();
                let snapshot = layout_snapshot(&document, 500.0);
                let mut context =
                    LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
                for input in [
                    Key::Char('2'),
                    Key::Ctrl(control),
                    Key::Char('2'),
                    Key::Char('j'),
                ] {
                    layout_key(&mut commands, &mut document, &mut context, input);
                }
                assert_eq!(commands.mode(), Mode::VisualBlock);
                for input in [Key::Char('"'), Key::Char('a'), Key::Char(operator)] {
                    layout_key(&mut commands, &mut document, &mut context, input);
                }
                if operator == 'r' {
                    layout_key(&mut commands, &mut document, &mut context, Key::Char('X'));
                    assert_eq!(document.text(), "XXcd\nXXgh\nXXqr");
                    assert!(commands.register('a').is_none());
                } else {
                    let value = commands.register('a').unwrap();
                    assert_eq!(value.kind, RegisterKind::Blockwise);
                    assert_eq!(value.text, "ab\nef\nop");
                    assert_eq!(
                        document.text(),
                        if operator == 'd' {
                            "cd\ngh\nqr"
                        } else {
                            "abcd\nefgh\nopqr"
                        }
                    );
                }
                assert_eq!(commands.mode(), Mode::Normal);
                if operator != 'y' {
                    assert!(document.undo());
                    assert_eq!(document.text(), "abcd\nefgh\nopqr");
                    assert!(document.redo());
                    assert!(document.undo());
                }
                assert!(!document.undo());
            }
        }
    }

    #[test]
    fn control_q_requests_layout_and_toggles_from_each_visual_mode() {
        for previous in [None, Some('v'), Some('V')] {
            let mut document = Document::new("a\u{301}b\n\n😀z");
            let mut commands = CommandInterpreter::new();
            if let Some(previous) = previous {
                key(&mut commands, &mut document, Key::Char(previous));
            }
            assert!(commands.requires_layout_for_input(
                &document,
                &InputEvent::Key(Key::Ctrl('q')),
                None
            ));
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('q'));
            assert_eq!(commands.mode(), Mode::VisualBlock);
            let cancelled = layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('q'));
            assert_eq!(cancelled.status, CommandStatus::Cancelled);
            assert_eq!(commands.mode(), Mode::Normal);
            assert_eq!(document.text(), "a\u{301}b\n\n😀z");
            assert!(!document.undo());
        }
    }

    #[test]
    fn visual_block_delete_and_replace_are_atomic_exact_layout_edits() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        assert_eq!(commands.mode(), Mode::VisualBlock);
        layout_key(&mut commands, &mut document, &mut context, Key::Char('l'));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('j'));
        let deleted = layout_key(&mut commands, &mut document, &mut context, Key::Char('d'));
        assert!(deleted.document_changed);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.text(), "cd\ngh");
        assert_eq!(commands.register('"').unwrap().text, "ab\nef");
        assert_eq!(commands.register('1').unwrap().text, "ab\nef");
        assert_eq!(
            commands.register('1').unwrap().kind,
            RegisterKind::Blockwise
        );
        assert!(commands.register('-').is_none());
        assert!(document.undo(), "the rectangle is one undo transaction");
        assert_eq!(document.text(), "abcd\nefgh");

        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('r'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        layout_key(&mut commands, &mut document, &mut context, Key::Char('X'));
        assert_eq!(document.text(), "XXcd\nXXgh");
        assert!(document.undo(), "block replacement is one undo transaction");
        assert_eq!(document.text(), "abcd\nefgh");
    }

    #[test]
    fn visual_g_case_operators_share_character_line_and_block_extents() {
        for (prefix, operator, expected) in [
            ("vl", '~', "aBCd\nEfGh\nTail"),
            ("Vj", 'u', "abcd\nefgh\nTail"),
        ] {
            let mut document = Document::new("AbCd\nEfGh\nTail");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, prefix);
            for event_key in [Key::Char('g'), Key::Char(operator)] {
                key(&mut commands, &mut document, event_key);
            }
            assert_eq!(document.text(), expected, "g{operator} after {prefix}");
            assert_eq!(commands.mode(), Mode::Normal);
            assert!(document.undo(), "Visual g-case is one undo unit");
            assert_eq!(document.text(), "AbCd\nEfGh\nTail");
            assert!(!document.undo());
        }

        let mut document = Document::new("Abcd\neFGh");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('g'),
            Key::Char('U'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "ABcd\nEFGh");
        assert_eq!(commands.mode(), Mode::Normal);
        assert!(document.undo(), "Visual Block gU is one undo unit");
        assert_eq!(document.text(), "Abcd\neFGh");
        assert!(!document.undo());
    }

    #[test]
    fn visual_paste_shift_reindent_and_case_counts_have_vim_semantics() {
        for (paste, preserve_unnamed) in [('p', false), ('P', true)] {
            let mut document = Document::new("abcdef");
            let mut commands = CommandInterpreter::new();
            commands
                .registers
                .yank(Some('a'), RegisterValue::characterwise("XY"));
            commands
                .registers
                .yank(None, RegisterValue::characterwise("old"));
            keys(&mut commands, &mut document, &format!("vl\"a3{paste}"));
            assert_eq!(document.text(), "XYXYXYcdef");
            assert_eq!(commands.mode(), Mode::Normal);
            if preserve_unnamed {
                assert_eq!(commands.register('"').unwrap().text, "old");
            } else {
                assert_eq!(commands.register('"').unwrap().text, "ab");
            }
            assert!(document.undo());
            assert_eq!(document.text(), "abcdef");
            assert!(!document.undo());
        }

        let mut document = Document::new("aa\nbb\ncc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vj3>");
        assert_eq!(document.text(), "            aa\n            bb\ncc");
        assert!(document.undo());
        assert_eq!(document.text(), "aa\nbb\ncc");
        assert!(!document.undo());

        let mut document = Document::new("            aa\n        bb\ncc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vj2<");
        assert_eq!(document.text(), "    aa\nbb\ncc");
        assert!(document.undo());

        let mut document = Document::new("    aa\n  bb\ncc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vj7=");
        assert_eq!(document.text(), "aa\nbb\ncc");
        assert!(document.undo());

        let mut document = Document::new("AbCd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vl2~");
        assert_eq!(
            document.text(),
            "aBCd",
            "a Visual case count is consumed once"
        );

        let mut document = Document::new("Abcd\neFGh");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('9'),
            Key::Char('g'),
            Key::Char('U'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "ABcd\nEFGh");
        assert_eq!(commands.mode(), Mode::Normal);
    }

    #[test]
    fn visual_block_selection_includes_the_current_and_active_graphemes() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        let deleted = layout_key(&mut commands, &mut document, &mut context, Key::Char('d'));
        assert!(deleted.document_changed);
        assert_eq!(document.text(), "bc");
        assert_eq!(commands.register('"').unwrap().text, "a");
        assert_eq!(commands.register('-').unwrap().text, "a");
        assert_eq!(
            commands.register('-').unwrap().kind,
            RegisterKind::Blockwise
        );
        assert!(commands.register('1').is_none());

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('l'), Key::Char('d')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "c");
        assert_eq!(commands.register('"').unwrap().text, "ab");
    }

    #[test]
    fn visual_block_single_cell_edges_cover_yank_change_paste_insert_and_append() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('y')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "abc");
        assert_eq!(commands.register('"').unwrap().text, "a");

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('c')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xbc");

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("X"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('p'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "Xbc");

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('I')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xabc");

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('A')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "aXbc");
    }

    #[test]
    fn visual_block_dot_replays_two_by_two_operators_and_ignores_dot_count() {
        let mut document = Document::new("abcd\nefgh\nqrst\nuvwx");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('d'),
            ],
        );
        assert_eq!(document.text(), "cd\ngh\nqrst\nuvwx");
        let after_source = document.text().to_owned();
        assert!(commands.set_cursor(&document, document.text().find("qrst").unwrap()));
        let repeated = layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Char('9'), Key::Char('9'), Key::Char('.')],
        );
        assert_eq!(document.text(), "cd\ngh\nst\nwx");
        assert!(
            !repeated.mode_changed,
            "dot never exposes its synthetic Visual mode"
        );
        assert_eq!(commands.register('"').unwrap().text, "qr\nuv");
        assert!(
            document.undo(),
            "the repeated multi-row edit is one undo unit"
        );
        assert_eq!(document.text(), after_source);

        let mut document = Document::new("ab12\ncd34\nef56\ngh78");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('r'),
                Key::Char('X'),
            ],
        );
        assert!(commands.set_cursor(&document, document.text().find("ef56").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "XX12\nXX34\nXX56\nXX78");

        let mut document = Document::new("ab12\ncd34\nef56\ngh78");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('U'),
            ],
        );
        assert!(commands.set_cursor(&document, document.text().find("ef56").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "AB12\nCD34\nEF56\nGH78");
    }

    #[test]
    fn visual_block_dot_rehits_one_cell_proportional_geometry() {
        let mut document = Document::new("i\ni\nW\nW");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('j'),
                Key::Char('r'),
                Key::Char('X'),
            ],
        );
        assert_eq!(document.text(), "X\nX\nW\nW");
        assert!(commands.set_cursor(&document, document.text().find('W').unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "X\nX\nX\nX",
            "a zero endpoint span still includes each wide destination item"
        );
    }

    #[test]
    fn visual_block_dot_preserves_counted_shift_and_maxcol_intent() {
        let mut document = Document::new("ab\ncd\nef\ngh");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('j'),
                Key::Char('2'),
                Key::Char('>'),
            ],
        );
        assert!(commands.set_cursor(&document, document.text().find("ef").unwrap()));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Char('7'), Key::Char('.')],
        );
        assert_eq!(
            document.text(),
            "        ab\n        cd\n        ef\n        gh"
        );

        let mut document = Document::new("ab\ncde\nwxyz\nmnopq");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('$'),
                Key::Char('j'),
                Key::Char('d'),
            ],
        );
        assert_eq!(document.text(), "\n\nwxyz\nmnopq");
        assert!(commands.set_cursor(&document, document.text().find("wxyz").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "\n\n\n",
            "MAXCOL is resolved against each longer destination row"
        );

        let mut document = Document::new("ab\ncde\nwxyz\nmnopq");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('$'),
                Key::Char('h'),
                Key::Char('j'),
                Key::Char('d'),
            ],
        );
        assert_eq!(document.text(), "b\nde\nwxyz\nmnopq");
        assert!(commands.set_cursor(&document, document.text().find("wxyz").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "b\nde\nxyz\nnopq");
    }

    #[test]
    fn visual_block_dot_replays_insert_append_and_change_on_short_rows() {
        let mut document = Document::new("abcd\nefgh\nwxyz\nx");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('2'),
                Key::Char('I'),
            ],
        );
        commands
            .handle(&mut document, InputEvent::Text("Q".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "abQQcd\nefQQgh\nwxyz\nx");
        let after_source = document.text().to_owned();
        assert!(commands.set_cursor(&document, document.text().find("wxyz").unwrap() + 2));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Char('8'), Key::Char('.')],
        );
        assert_eq!(document.text(), "abQQcd\nefQQgh\nwxQQyz\nx");
        assert!(document.undo());
        assert_eq!(document.text(), after_source);

        let mut document = Document::new("abcd\nefgh\nwxyz\nx");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('2'),
                Key::Char('A'),
            ],
        );
        commands
            .handle(&mut document, InputEvent::Text("Q".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "abcdQQ\nefghQQ\nwxyz\nx");
        assert!(commands.set_cursor(&document, document.text().find("wxyz").unwrap() + 2));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "abcdQQ\nefghQQ\nwxyzQQ\nxQQ");

        let mut document = Document::new("abcd\nefgh\nwxyz\nx");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('3'),
                Key::Char('c'),
            ],
        );
        commands
            .handle(&mut document, InputEvent::Text("Z".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "abZ\nefZ\nwxyz\nx");
        assert!(commands.set_cursor(&document, document.text().find("wxyz").unwrap() + 2));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "abZ\nefZ\nwxZ\nx",
            "block change skips a destination row that does not reach the rectangle"
        );
    }

    #[test]
    fn visual_block_put_dot_repeats_delete_with_p_and_black_hole_with_upper_p() {
        let mut document = Document::new("ab12\ncd34\nef56\ngh78");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("Q"));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('"'),
                Key::Char('a'),
                Key::Char('p'),
            ],
        );
        assert_eq!(document.text(), "Q12\nQ34\nef56\ngh78");
        assert!(commands.set_cursor(&document, document.text().find("ef56").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "Q12\nQ34\n56\n78");
        assert_eq!(commands.register('"').unwrap().text, "ef\ngh");

        let mut document = Document::new("ab12\ncd34\nef56\ngh78");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("Q"));
        commands
            .registers
            .yank(None, RegisterValue::characterwise("seed"));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('"'),
                Key::Char('a'),
                Key::Char('P'),
            ],
        );
        assert_eq!(commands.register('"').unwrap().text, "seed");
        assert!(commands.set_cursor(&document, document.text().find("ef56").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "Q12\nQ34\n56\n78");
        assert_eq!(
            commands.register('"').unwrap().text,
            "seed",
            "P records a black-hole block deletion for dot"
        );
    }

    #[test]
    fn visual_block_dot_replays_join_and_g_join() {
        let mut document = Document::new("a\nb\nc\nd");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Ctrl('v'), Key::Char('j'), Key::Char('J')],
        );
        assert_eq!(document.text(), "a b\nc\nd");
        assert!(commands.set_cursor(&document, document.text().find("c\nd").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "a b\nc d");

        let mut document = Document::new("a\nb\nc\nd");
        let mut commands = CommandInterpreter::new();
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('j'),
                Key::Char('g'),
                Key::Char('J'),
            ],
        );
        assert_eq!(document.text(), "ab\nc\nd");
        assert!(commands.set_cursor(&document, document.text().find("c\nd").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(document.text(), "ab\ncd");
    }

    #[test]
    fn visual_block_dot_rehits_wrapped_visual_rows() {
        let mut document = Document::new("aa aa aa aa aa aa aa aa aa aa");
        let mut commands = CommandInterpreter::new();
        let before = document.text().to_owned();
        layout_keys(
            &mut commands,
            &mut document,
            45.0,
            &[
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('U'),
            ],
        );
        let after_source_uppercase = document
            .text()
            .chars()
            .filter(|character| character.is_uppercase())
            .count();
        let destination = {
            let snapshot = layout_snapshot(&document, 45.0);
            assert!(snapshot.rows.len() >= 4);
            snapshot.rows[2]
                .clusters
                .first()
                .expect("the destination wrapped row contains text")
                .text_range
                .start
        };
        assert!(commands.set_cursor(&document, destination));
        let after_source = document.text().to_owned();
        layout_keys(&mut commands, &mut document, 45.0, &[Key::Char('.')]);
        assert!(
            document
                .text()
                .chars()
                .filter(|character| character.is_uppercase())
                .count()
                > after_source_uppercase
        );
        assert!(document.undo());
        assert_eq!(document.text(), after_source);
        assert!(document.undo());
        assert_eq!(document.text(), before);
    }

    #[test]
    fn successful_noop_visual_block_commands_replace_a_stale_dot_recipe() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "x");
        assert!(document.undo());
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Ctrl('v'), Key::Char('r'), Key::Char('a')],
        );
        assert_eq!(document.text(), "abcd\nefgh");
        assert!(commands.set_cursor(&document, document.text().find("efgh").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "abcd\nafgh",
            "a successful same-character r replaces the older x recipe"
        );

        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "x");
        assert!(document.undo());
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[Key::Ctrl('v'), Key::Char('I')],
        );
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.set_cursor(&document, document.text().find("efgh").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "abcd\nefgh",
            "an empty block Insert publishes an explicit no-op"
        );

        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "x");
        assert!(document.undo());
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("a"));
        layout_keys(
            &mut commands,
            &mut document,
            500.0,
            &[
                Key::Ctrl('v'),
                Key::Char('"'),
                Key::Char('a'),
                Key::Char('p'),
            ],
        );
        assert!(commands.set_cursor(&document, document.text().find("efgh").unwrap()));
        layout_keys(&mut commands, &mut document, 500.0, &[Key::Char('.')]);
        assert_eq!(
            document.text(),
            "abcd\nfgh",
            "a byte-identical block put still records its block-delete repeat"
        );
    }

    #[test]
    fn visual_block_dot_encoding_failure_is_atomic() {
        let mut document =
            Document::from_bytes(b"a\nb".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        let repeat = RepeatAction::VisualBlock(VisualBlockRepeat {
            shape: VisualBlockRepeatShape {
                visual_row_count: 2,
                endpoint_x_span_bits: 0.0_f32.to_bits(),
                to_line_end: false,
            },
            action: VisualBlockRepeatAction::Insert {
                kind: VisualBlockInsertKind::Insert,
                payload: "😀".to_owned(),
                application_count: 1,
                register: None,
            },
        });
        commands.last_repeat = Some(repeat.clone());
        let revision = document.revision();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        let result = commands.handle_with_layout(
            &mut document,
            InputEvent::Key(Key::Char('.')),
            &mut context,
        );
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "a\nb");
        assert_eq!(document.source_bytes(), b"a\nb");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.last_repeat, Some(repeat));
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_replace_is_per_grapheme_and_noop_aware() {
        let mut document = Document::new("a\u{301}b\na\u{301}b");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('r'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let replaced = layout_key(&mut commands, &mut document, &mut context, Key::Char('X'));
        assert!(replaced.document_changed);
        assert_eq!(document.text(), "XX\nXX");
        assert!(document.undo(), "all per-grapheme replacements are atomic");
        assert!(!document.undo());

        let mut document = Document::new("aa\naa");
        let before_revision = document.revision();
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('r'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let noop = layout_key(&mut commands, &mut document, &mut context, Key::Char('a'));
        assert!(!noop.document_changed);
        assert_eq!(document.revision(), before_revision);
        assert_eq!(document.text(), "aa\naa");
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_uppercase_o_moves_the_active_horizontal_corner() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('l'), Key::Char('j')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let before = commands.resolved_visual_block(&document, &context).unwrap();
        let old_active = commands.visual_block().unwrap().active;

        let switched = layout_key(&mut commands, &mut document, &mut context, Key::Char('O'));
        let after = commands.resolved_visual_block(&document, &context).unwrap();
        assert!(switched.cursor_moved);
        assert_ne!(commands.visual_block().unwrap().active, old_active);
        assert_eq!(commands.cursor(), 5, "the active corner moved from f to e");
        assert_eq!(after.anchor_row, before.anchor_row);
        assert_eq!(after.active_row, before.active_row);
        assert_eq!(after.range_set, before.range_set);

        layout_key(&mut commands, &mut document, &mut context, Key::Char('l'));
        let contracted = commands.resolved_visual_block(&document, &context).unwrap();
        assert_eq!(contracted.rows[0].ranges, vec![1..2]);
        assert_eq!(contracted.rows[1].ranges, vec![6..7]);
    }

    #[test]
    fn visual_block_replace_skips_rows_shorter_than_the_rectangle() {
        let mut document = Document::new("abcd\nx\nwxyz");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('2'),
            Key::Char('j'),
            Key::Char('r'),
            Key::Char('Q'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "abQQ\nx\nwxQQ");
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn saturated_visual_block_counts_report_errors_without_mutation() {
        let mut document = Document::new("ab\ncd");
        let original = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        for _ in 0..128 {
            layout_key(&mut commands, &mut document, &mut context, Key::Char('9'));
        }
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('I'));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(document.text(), original);
        assert_eq!(commands.mode(), Mode::VisualBlock);
        assert_eq!(commands.visual_block_insert_payload(), None);
        assert!(!document.undo());

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XX"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('"'), Key::Char('a')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        for _ in 0..128 {
            layout_key(&mut commands, &mut document, &mut context, Key::Char('9'));
        }
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('p'));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(document.text(), "ab\ncd");
        assert_eq!(commands.mode(), Mode::VisualBlock);
        assert!(!document.undo());

        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        for _ in 0..128 {
            layout_key(&mut commands, &mut document, &mut context, Key::Char('9'));
        }
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('r'));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('X'));
        assert!(!output.document_changed);
        assert_eq!(document.text(), "ab");
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_insert_is_deferred_counted_and_one_undo_unit() {
        let mut document = Document::new("abcd\nefgh");
        let original = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());

        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('2'),
            Key::Char('I'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(
            commands.visual_block_insert_kind(),
            Some(VisualBlockInsertKind::Insert)
        );
        assert_eq!(commands.visual_block_insert_payload(), Some(""));
        assert_eq!(document.text(), original, "payload collection is deferred");

        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        assert_eq!(commands.visual_block_insert_payload(), Some("X"));
        assert_eq!(document.text(), original);
        let committed = key(&mut commands, &mut document, Key::Escape);
        assert!(committed.document_changed);
        assert_eq!(document.text(), "XXabcd\nXXefgh");
        assert_eq!(commands.mode(), Mode::Normal);
        assert!(document.undo());
        assert_eq!(document.text(), original);
        assert!(
            !document.undo(),
            "all row insertions share one history node"
        );
    }

    #[test]
    fn visual_block_insert_skips_short_rows_but_append_uses_their_row_end() {
        fn select_block(
            commands: &mut CommandInterpreter,
            document: &mut Document,
            context: &mut LayoutCommandContext<'_>,
        ) {
            assert!(commands.set_cursor(document, 2));
            for key in [
                Key::Ctrl('v'),
                Key::Char('2'),
                Key::Char('l'),
                Key::Char('2'),
                Key::Char('j'),
            ] {
                layout_key(commands, document, context, key);
            }
        }

        let mut document = Document::new("abcd\nx\nwxyz");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        select_block(&mut commands, &mut document, &mut context);
        layout_key(&mut commands, &mut document, &mut context, Key::Char('I'));
        commands
            .handle(&mut document, InputEvent::Text("Q".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "abQcd\nx\nwxQyz");
        assert!(document.undo());

        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        select_block(&mut commands, &mut document, &mut context);
        layout_key(&mut commands, &mut document, &mut context, Key::Char('A'));
        commands
            .handle(&mut document, InputEvent::Text("Q".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "abcdQ\nxQ\nwxyzQ");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nx\nwxyz");
    }

    #[test]
    fn visual_block_change_collects_payload_and_updates_requested_register() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('b'), RegisterValue::characterwise("Z"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('c'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(
            commands.visual_block_insert_kind(),
            Some(VisualBlockInsertKind::Change)
        );
        commands
            .handle(&mut document, InputEvent::Text("a\u{301}".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Ctrl('r'));
        key(&mut commands, &mut document, Key::Char('b'));
        assert_eq!(commands.visual_block_insert_payload(), Some("Z"));
        assert_eq!(document.text(), "abcd\nefgh");
        key(&mut commands, &mut document, Key::Escape);

        assert_eq!(document.text(), "Zd\nZh");
        let captured = commands.register('a').unwrap();
        assert_eq!(captured.kind, RegisterKind::Blockwise);
        assert_eq!(captured.text, "abc\nefg");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
    }

    #[test]
    fn deferred_visual_block_change_rejects_line_break_and_failed_encoding_is_atomic() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"ab\ncd".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('s'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let line_break = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(
            line_break.status,
            CommandStatus::Unsupported(ref message) if message.contains("hard line break")
        ));
        assert_eq!(document.text(), "ab\ncd");

        commands
            .handle(&mut document, InputEvent::Text("😀".into()))
            .unwrap();
        let result = commands.handle(&mut document, InputEvent::Key(Key::Escape));
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.text(), "ab\ncd");
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(commands.visual_block_insert_payload(), Some("😀"));

        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "\n");
        assert!(document.undo());
        assert_eq!(document.text(), "ab\ncd");
    }

    #[test]
    fn visual_block_p_and_uppercase_p_obey_register_and_count_semantics() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("Q"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('2'),
            Key::Char('p'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "QQd\nQQh");
        assert_eq!(
            commands.register('"').unwrap().kind,
            RegisterKind::Blockwise
        );
        assert_eq!(commands.register('"').unwrap().text, "abc\nefg");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");

        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("Q"));
        commands
            .registers
            .yank(Some('b'), RegisterValue::characterwise("keep"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('P'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "Qd\nQh");
        assert_eq!(commands.register('"').unwrap().text, "keep");
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_register_rows_round_trip_and_extend_past_the_selection() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('y'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(
            commands.register('a').unwrap().kind,
            RegisterKind::Blockwise
        );
        assert_eq!(commands.register('a').unwrap().text, "abc\nefg");

        assert!(commands.set_cursor(&document, 1));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('P'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "aabc\neefg");
        assert!(document.undo());

        commands
            .registers
            .yank(Some('a'), RegisterValue::blockwise("one\ntwo\nthree"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let output = layout_key(&mut commands, &mut document, &mut context, Key::Char('p'));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.text(), "aoned\netwoh\nthree");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
    }

    #[test]
    fn visual_block_shift_outdent_and_reindent_are_atomic() {
        let mut document = Document::new("ab cd\nab ef");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('2'),
            Key::Char('>'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "ab         cd\nab         ef");
        assert!(document.undo());
        assert!(!document.undo());

        let mut document = Document::new("ab      cd\nab  ef");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('<'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "ab  cd\nabef");
        assert!(document.undo());

        let mut document = Document::new("    one\n  two");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('='),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "one\ntwo");
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_edits_follow_proportional_and_wrapped_row_hit_tests() {
        let mut document = Document::new("iiii\nWWWW");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 1));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let resolved = commands.resolved_visual_block(&document, &context).unwrap();
        let positions = resolved
            .rows
            .iter()
            .filter(|row| row.ranges.iter().any(|range| !range.is_empty()))
            .map(|row| row.visual_left.point.text_offset)
            .collect::<Vec<_>>();
        assert_ne!(
            positions[0] % 5,
            positions[1] % 5,
            "the same x intersects different proportional character columns"
        );
        let mut expected = document.text().to_owned();
        for at in positions.iter().rev() {
            expected.insert(*at, 'X');
        }
        layout_key(&mut commands, &mut document, &mut context, Key::Char('I'));
        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), expected);
        assert!(document.undo());

        let mut document = Document::new("one two three four five");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 45.0);
        assert!(snapshot.rows.len() >= 3);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('2'),
            Key::Char('j'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let resolved = commands.resolved_visual_block(&document, &context).unwrap();
        assert_eq!(resolved.rows.len(), 3);
        let positions = resolved
            .rows
            .iter()
            .filter(|row| row.ranges.iter().any(|range| !range.is_empty()))
            .map(|row| row.visual_left.point.text_offset)
            .collect::<Vec<_>>();
        let mut expected = document.text().to_owned();
        for at in positions.iter().rev() {
            expected.insert_str(*at, "    ");
        }
        layout_key(&mut commands, &mut document, &mut context, Key::Char('>'));
        assert_eq!(document.text(), expected);
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn visual_block_paste_and_append_use_proportional_and_wrapped_outer_edges() {
        let mut document = Document::new("iiii\nWWWW");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("Q"));
        assert!(commands.set_cursor(&document, 1));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('2'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('p'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "iQ\nQWW");
        assert!(document.undo());
        assert!(!document.undo());

        let mut document = Document::new("one two three four five");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 45.0);
        assert!(snapshot.rows.len() >= 3);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('2'),
            Key::Char('j'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        let resolved = commands.resolved_visual_block(&document, &context).unwrap();
        assert_eq!(resolved.rows.len(), 3);
        let mut insertion_offsets = resolved
            .rows
            .iter()
            .map(|row| row.visual_right.point.text_offset)
            .collect::<Vec<_>>();
        let mut expected = document.text().to_owned();
        insertion_offsets.sort_unstable();
        for offset in insertion_offsets.into_iter().rev() {
            expected.insert(offset, 'X');
        }
        layout_key(&mut commands, &mut document, &mut context, Key::Char('A'));
        commands
            .handle(&mut document, InputEvent::Text("X".into()))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), expected);
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn stale_position_map_is_rejected_without_controller_effects() {
        let document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 1));
        commands.marks.insert('a', 1);
        let anchors = commands.capture_position_anchors(&document).unwrap();
        let map = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abc",
            "Xabc",
            vec![crate::document::Splice::new(0..0, 1).unwrap()],
        )
        .unwrap();

        commands.note_document_revision(Revision(9));
        let before = format!("{commands:?}");
        assert!(!commands.apply_position_map(&anchors, &map).unwrap());
        assert_eq!(format!("{commands:?}"), before);
    }

    #[test]
    fn wrong_document_position_map_has_no_partial_controller_effects() {
        let document = Document::new("abc");
        let other_document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 1));
        commands.marks.insert('a', 2);
        commands.visual_anchor = Some(0);
        commands.mode = Mode::VisualCharacter;
        let anchors = commands.capture_position_anchors(&document).unwrap();
        let map = PositionMap::for_text(
            other_document.id(),
            Revision(0),
            Revision(1),
            "abc",
            "Xabc",
            vec![crate::document::Splice::new(0..0, 1).unwrap()],
        )
        .unwrap();

        let before = format!("{commands:?}");
        assert!(matches!(
            commands.apply_position_map(&anchors, &map),
            Err(PositionError::WrongDocument { .. })
        ));
        assert_eq!(format!("{commands:?}"), before);
    }

    #[test]
    fn unsupported_normal_u_keeps_encoded_history_and_visual_uppercase() {
        for (source, encoding, format) in [
            (b"__bold__".to_vec(), Encoding::Utf8, Format::Markdown),
            (b"caf\xe9".to_vec(), Encoding::Latin1, Format::PlainText),
            (
                vec![0xff, 0xfe, b'a', 0, b'b', 0],
                Encoding::Utf16Le,
                Format::PlainText,
            ),
        ] {
            let mut document = Document::from_bytes(source.clone(), encoding, format).unwrap();
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, "rX");
            let edited = document.source_bytes();
            let revision = document.revision();
            let output = key(&mut commands, &mut document, Key::Char('U'));
            assert!(matches!(output.status, CommandStatus::Unsupported(_)));
            assert_eq!(document.revision(), revision);
            assert_eq!(document.source_bytes(), edited);
            keys(&mut commands, &mut document, "u");
            assert_eq!(document.source_bytes(), source);
            key(&mut commands, &mut document, Key::Ctrl('r'));
            assert_eq!(document.source_bytes(), edited);
        }
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vllU");
        assert_eq!(document.text(), "ONE two");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "one two");
    }

    #[test]
    fn dot_repeats_operator_change_indent_and_case_recipes() {
        let mut document = Document::new("one two three four");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "dw.");
        assert_eq!(document.text(), "three four");
        keys(&mut commands, &mut document, "2.");
        assert_eq!(document.text(), "");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "three four", "one dot is one undo unit");

        let mut document = Document::new("one two three four");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "cwX");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "X X three four");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "X two three four");

        let mut document = Document::new("a\nb");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, ">>j.");
        assert_eq!(document.text(), "    a\n    b");

        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "gUiww.");
        assert_eq!(document.text(), "ONE TWO");

        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "\"adw.");
        assert_eq!(document.text(), "");
        assert_eq!(commands.register('a').unwrap().text, "two");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "two");
    }

    #[test]
    fn explicit_one_dot_overrides_a_recorded_count() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2x1.");
        assert_eq!(document.text(), "def");
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "f", "plain dot retains the recorded count");
    }

    #[test]
    fn dot_repeats_named_register_paste_and_open_line_as_single_units() {
        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("X"));
        keys(&mut commands, &mut document, "\"ap.2.");
        assert_eq!(document.text(), "aXXXXb");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "aXXb");

        let mut document = Document::new("a");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "ox");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "a\nx\nx");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "a\nx");
    }

    #[test]
    fn search_rejects_vim_only_atoms_in_both_directions() {
        for pattern in [r"one\+", r"\(one\)", r"one\zs"] {
            for command in ['/', '?'] {
                let mut document = Document::new("one one");
                let mut commands = CommandInterpreter::new();
                key(&mut commands, &mut document, Key::Char(command));
                commands
                    .handle(&mut document, InputEvent::Text(pattern.to_owned()))
                    .unwrap();
                let output = key(&mut commands, &mut document, Key::Enter);
                assert!(
                    matches!(output.status, CommandStatus::Error(ref error) if error.contains("UnsupportedRegexAtom")),
                    "{command}{pattern} returned {:?}",
                    output.status
                );
                assert_eq!(commands.cursor(), 0);
                assert!(commands.last_search.is_none());
                assert_eq!(document.text(), "one one");
            }
        }
    }

    #[test]
    fn macros_share_named_register_storage_with_text_commands() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "qalq");
        assert_eq!(commands.register('a').unwrap().text, "l");

        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "@a");
        assert_eq!(commands.cursor(), 1);

        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("x"));
        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "@a");
        assert_eq!(
            document.text(),
            "bc",
            "text overwrite changes macro playback"
        );
    }

    #[test]
    fn operator_line_offset_column_and_backspace_motions_are_composable() {
        let mut document = Document::new("a\nb\nc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "d+");
        assert_eq!(document.text(), "c");
        keys(&mut commands, &mut document, "u");
        keys(&mut commands, &mut document, "jd-");
        assert_eq!(document.text(), "c");

        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "4ld3|");
        assert_eq!(document.text(), "abef");
        keys(&mut commands, &mut document, "l");
        key(&mut commands, &mut document, Key::Backspace);
        keys(&mut commands, &mut document, "d");
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "aef");
    }

    #[test]
    fn visual_search_and_g_search_retain_the_directed_selection() {
        let mut document = Document::new("one two one");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vl/one");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 8);

        keys(&mut commands, &mut document, "g#");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 0);

        keys(&mut commands, &mut document, "/");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(0));
    }

    #[test]
    fn change_word_uses_end_motion_on_nonblank_but_forward_motion_on_blank() {
        let mut document = Document::new("one two three four");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "\"a2cwX");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "X three four");
        assert_eq!(commands.register('a').unwrap().text, "one two");
        assert!(document.undo());
        assert_eq!(document.text(), "one two three four");
        assert!(
            !document.undo(),
            "the change and insertion are one undo unit"
        );

        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 3));
        keys(&mut commands, &mut document, "cwX");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "oneXtwo");
        assert_eq!(commands.register('"').unwrap().text, " ");

        let mut document = Document::new("one,two three four");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "2cWX");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "X four");
        assert_eq!(commands.register('"').unwrap().text, "one,two three");
    }

    #[test]
    fn counted_change_word_dot_repeats_the_semantic_motion_and_named_register() {
        let mut document = Document::new("one two three four");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "\"a2cwX");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "X X");
        assert_eq!(commands.register('a').unwrap().text, "three four");
        assert!(document.undo());
        assert_eq!(document.text(), "X three four");
    }

    #[test]
    fn insert_and_replace_entry_counts_repeat_payload_in_one_undo_unit() {
        for (command, original, expected, cursor) in [
            ("3i", "ab", "XXXab", 2),
            ("3a", "ab", "aXXXb", 3),
            ("3I", "  ab", "  XXXab", 4),
            ("3A", "ab", "abXXX", 4),
        ] {
            let mut document = Document::new(original);
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            commands
                .handle(&mut document, InputEvent::text("X"))
                .unwrap();
            key(&mut commands, &mut document, Key::Escape);
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.cursor(), cursor, "{command}");
            assert!(document.undo(), "{command}");
            assert_eq!(document.text(), original, "{command}");
            assert!(!document.undo(), "{command} must be one undo unit");
        }

        let mut document = Document::new("abcdefghij");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3R");
        commands
            .handle(&mut document, InputEvent::text("XY"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "XYXYXYghij");
        assert_eq!(commands.cursor(), 5);
        assert!(document.undo());
        assert_eq!(document.text(), "abcdefghij");
        assert!(!document.undo());
    }

    #[test]
    fn counted_open_line_repeats_whole_lines_and_dot_retains_its_count() {
        for (command, expected, cursor) in [
            ("3o", "one\nX\nX\nX\ntwo", 8),
            ("3O", "X\nX\nX\none\ntwo", 4),
        ] {
            let mut document = Document::new("one\ntwo");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            commands
                .handle(&mut document, InputEvent::text("X"))
                .unwrap();
            key(&mut commands, &mut document, Key::Escape);
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.cursor(), cursor, "{command}");
            assert!(document.undo(), "{command}");
            assert_eq!(document.text(), "one\ntwo", "{command}");
            assert!(!document.undo(), "{command} must be one undo unit");
        }

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "2oX");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "one\nX\nX\nX\nX\ntwo");
        assert!(document.undo());
        assert_eq!(document.text(), "one\nX\nX\ntwo");
    }

    #[test]
    fn operator_pending_g_and_percent_distinguish_default_and_explicit_counts() {
        let original = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 4)));
        keys(&mut commands, &mut document, "2d3G");
        assert_eq!(document.text(), "1\n2\n3\n7\n8\n9\n10");
        assert_eq!(commands.register('1').unwrap().text, "4\n5\n6\n");
        assert!(document.undo());

        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 4)));
        keys(&mut commands, &mut document, "d2G");
        assert_eq!(document.text(), "1\n5\n6\n7\n8\n9\n10");
        assert!(document.undo());

        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 8)));
        keys(&mut commands, &mut document, "2d25%");
        assert_eq!(document.text(), "1\n2\n3\n4\n9\n10");
        assert_eq!(commands.register('1').unwrap().text, "5\n6\n7\n8\n");

        let mut document = Document::new(original);
        let mut commands = CommandInterpreter::new();
        let revision = document.revision();
        let output = keys(&mut commands, &mut document, "d101%");
        assert!(matches!(
            output.status,
            CommandStatus::Error(ref message) if message.contains("between 1 and 100")
        ));
        assert_eq!(document.text(), original);
        assert_eq!(document.revision(), revision);
        assert!(commands.register('1').is_none());

        let output = keys(&mut commands, &mut document, "101%");
        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), original);
    }

    #[test]
    fn normal_case_join_and_put_commands_leave_vim_compatible_cursors() {
        let mut document = Document::new("ab cd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "~");
        assert_eq!(document.text(), "Ab cd");
        assert_eq!(commands.cursor(), 1);
        keys(&mut commands, &mut document, "2~");
        assert_eq!(document.text(), "AB cd");
        assert_eq!(commands.cursor(), 3);

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "J");
        assert_eq!(document.text(), "one two");
        assert_eq!(commands.cursor(), 3);

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "gJ");
        assert_eq!(document.text(), "onetwo");
        assert_eq!(commands.cursor(), 3);

        for (command, expected, cursor) in [
            ("\"ap", "aXYb", 2),
            ("\"aP", "XYab", 1),
            ("\"agp", "aXYb", 3),
            ("\"agP", "XYab", 2),
            ("\"a2p", "aXYXYb", 4),
        ] {
            let mut document = Document::new("ab");
            let mut commands = CommandInterpreter::new();
            commands
                .registers
                .yank(Some('a'), RegisterValue::characterwise("XY"));
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.cursor(), cursor, "{command}");
        }

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::linewise("  X\n"));
        keys(&mut commands, &mut document, "\"ap");
        assert_eq!(document.text(), "one\n  X\ntwo");
        assert_eq!(commands.cursor(), 6);

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::linewise("  X\n"));
        keys(&mut commands, &mut document, "\"agp");
        assert_eq!(commands.cursor(), 8);
    }

    #[test]
    fn saturated_repeat_counts_report_errors_without_mutation_or_panics() {
        let huge = "9".repeat(128);

        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("X"));
        let revision = document.revision();
        let output = keys(&mut commands, &mut document, &format!("\"a{huge}p"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(document.text(), "ab");
        assert_eq!(document.revision(), revision);

        let output = keys(&mut commands, &mut document, &format!("{huge}i"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(document.text(), "ab");

        keys(&mut commands, &mut document, "iX");
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.set_cursor(&document, 2));
        let before = document.text().to_owned();
        let revision = document.revision();
        let output = keys(&mut commands, &mut document, &format!("{huge}."));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(document.text(), before);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn overflowing_counts_are_typed_and_non_destructive_across_command_families() {
        let huge = "9".repeat(128);

        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 4));
        let revision = document.revision();
        let output = keys(&mut commands, &mut document, &format!("{huge}h"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(
            commands.cursor(),
            4,
            "an overflowing count cannot reverse h"
        );
        assert_eq!(document.revision(), revision);

        let output = keys(&mut commands, &mut document, &format!("d{huge}w"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(commands.cursor(), 4);
        assert_eq!(document.text(), "one two three");
        assert_eq!(document.revision(), revision);
        assert!(!document.undo());

        let output = keys(&mut commands, &mut document, &format!("{huge}/"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(commands.mode(), Mode::Normal);
        assert!(commands.command_line_state.is_none());
        assert_eq!(commands.cursor(), 4);

        let output = keys(&mut commands, &mut document, &format!("{huge}x"));
        assert_eq!(
            output.status,
            CommandStatus::CountError(CountError::Overflow)
        );
        assert_eq!(document.text(), "one two three");
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn maximum_valid_counts_stay_directional_and_finish_at_reachable_edges() {
        let maximum = usize::MAX.to_string();
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 1));

        let output = keys(&mut commands, &mut document, &format!("{maximum}l"));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(
            commands.cursor(),
            2,
            "a large positive count cannot reverse"
        );

        assert!(commands.set_cursor(&document, 1));
        let output = keys(&mut commands, &mut document, &format!("{maximum}h"));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(
            commands.cursor(),
            0,
            "a large negative motion cannot reverse"
        );

        let mut document = Document::new("one two. three four.\n\nlast");
        let mut commands = CommandInterpreter::new();
        for motion in ["w", ")", "}"] {
            assert!(commands.set_cursor(&document, 0));
            let output = keys(&mut commands, &mut document, &format!("{maximum}{motion}"));
            assert_eq!(output.status, CommandStatus::Complete, "{motion}");
            assert!(commands.cursor() <= document.projection().text_tree().byte_len(), "{motion}");
        }

        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        let output = keys(&mut commands, &mut document, &format!("d{maximum}w"));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), "");

        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise(""));
        let revision = document.revision();
        let output = keys(&mut commands, &mut document, &format!("\"a{maximum}p"));
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), "ab");
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn standalone_search_counts_cover_direction_wrap_visual_and_maximum_count() {
        let mut document = Document::new("a x a x a");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2/a");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), 8);

        keys(&mut commands, &mut document, "2?a");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), 0);

        keys(&mut commands, &mut document, "3/a");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), 0, "the third match wraps to the origin");

        keys(&mut commands, &mut document, "v2/a");
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 8);

        let mut document = Document::new("a a");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, &format!("{}/a", usize::MAX));
        let output = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), 2);
    }

    #[test]
    fn compound_ex_output_retains_ordered_committed_transactions() {
        let mut document = Document::new("a\nb\nc");
        let mut commands = CommandInterpreter::new();
        let mut events = Vec::new();
        for _ in 0..2 {
            events.push(InputEvent::Key(Key::Char(':')));
            events.extend("delete".chars().map(InputEvent::key));
            events.push(InputEvent::Key(Key::Enter));
        }
        commands.registers.set_macro('a', events);

        let output = keys(&mut commands, &mut document, "@a");

        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.document_changed);
        assert_eq!(document.text(), "c");
        let outcome = output
            .ex_outcome
            .as_ref()
            .expect("the compound output retains its Ex effects");
        assert!(
            outcome.model_transaction().is_none(),
            "an aggregate must not masquerade as one transaction"
        );
        let transactions = outcome.model_transactions();
        assert_eq!(transactions.len(), 2);
        assert_eq!(
            transactions[0].after_revision(),
            transactions[1].before_revision(),
            "the two exact publications remain in commit order"
        );
        assert_eq!(transactions[0].summary().source_patches().len(), 1);
        assert_eq!(transactions[1].summary().source_patches().len(), 1);
        assert!(document.undo(), "the macro remains one undo unit");
        assert_eq!(document.text(), "a\nb\nc");
    }

    #[test]
    fn tab_is_ordinary_insert_and_replace_text_in_the_current_undo_unit() {
        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        key(&mut commands, &mut document, Key::Tab);
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "\tXab");
        assert!(document.undo());
        assert_eq!(document.text(), "ab");
        assert!(!document.undo());

        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('R'));
        key(&mut commands, &mut document, Key::Tab);
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "\tX");
        assert!(document.undo());
        assert_eq!(document.text(), "ab");
        assert!(!document.undo());
    }

    #[test]
    fn pending_text_commands_consume_one_complete_extended_grapheme() {
        let combined = "a\u{301}";
        let mut document = Document::new("xy");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('r'));
        let replaced = commands
            .handle(&mut document, InputEvent::text(combined))
            .unwrap();
        assert_eq!(replaced.status, CommandStatus::Complete);
        assert_eq!(document.text(), "a\u{301}y");

        let mut document = Document::new("x a\u{301} y");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('f'));
        commands
            .handle(&mut document, InputEvent::text(combined))
            .unwrap();
        assert_eq!(commands.cursor(), 2);

        let mut document = Document::new("xy");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vlr");
        commands
            .handle(&mut document, InputEvent::text(combined))
            .unwrap();
        assert_eq!(document.text(), "a\u{301}a\u{301}");

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Ctrl('v'), Key::Char('j'), Key::Char('r')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        commands
            .handle_with_layout(&mut document, InputEvent::text(combined), &mut context)
            .unwrap();
        assert_eq!(document.text(), "a\u{301}b\na\u{301}d");
    }

    #[test]
    fn long_and_short_doubled_case_operators_are_linewise() {
        for command in ["g~~", "g~g~"] {
            let mut document = Document::new("Ab C\nDe F");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), "aB c\nDe F", "{command}");
        }
        for command in ["guu", "gugu"] {
            let mut document = Document::new("AB C\nDE F");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), "ab c\nDE F", "{command}");
        }
        for command in ["2gUU", "2gUgU"] {
            let mut document = Document::new("ab c\nde f\ngh i");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), "AB C\nDE F\ngh i", "{command}");
        }
    }

    #[test]
    fn visual_gv_exchanges_current_and_previous_directed_selections() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vly");
        assert_eq!(commands.last_visual.unwrap().mode, Mode::VisualCharacter);

        keys(&mut commands, &mut document, "GVk");
        let current = VisualMemory {
            mode: commands.mode,
            anchor: commands.visual_anchor.unwrap(),
            active: commands.cursor,
            to_line_end: commands.visual_to_line_end,
            block: None,
        };
        let previous = commands.last_visual.unwrap();
        keys(&mut commands, &mut document, "gv");
        assert_eq!(commands.mode(), previous.mode);
        assert_eq!(commands.visual_anchor, Some(previous.anchor));
        assert_eq!(commands.cursor(), previous.active);
        assert_eq!(commands.last_visual, Some(current));

        keys(&mut commands, &mut document, "gv");
        assert_eq!(commands.mode(), current.mode);
        assert_eq!(commands.visual_anchor, Some(current.anchor));
        assert_eq!(commands.cursor(), current.active);
    }

    #[test]
    fn visual_block_paste_matches_vim_for_short_long_and_counted_block_rows() {
        let run = |register: RegisterValue, selection_rows: usize, count: usize| {
            let mut document = Document::new("abcd\nefgh\nijkl\nmnop");
            let mut commands = CommandInterpreter::new();
            commands.registers.yank(Some('a'), register);
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
            layout_key(&mut commands, &mut document, &mut context, Key::Char('l'));
            for _ in 1..selection_rows {
                layout_key(&mut commands, &mut document, &mut context, Key::Char('j'));
            }
            layout_key(&mut commands, &mut document, &mut context, Key::Char('"'));
            layout_key(&mut commands, &mut document, &mut context, Key::Char('a'));
            if count > 1 {
                layout_key(
                    &mut commands,
                    &mut document,
                    &mut context,
                    Key::Char(char::from_digit(count as u32, 10).unwrap()),
                );
            }
            layout_key(&mut commands, &mut document, &mut context, Key::Char('p'));
            document
        };

        let mut short = run(RegisterValue::blockwise("XY"), 3, 1);
        assert_eq!(short.text(), "XYcd\ngh\nkl\nmnop");
        assert!(short.undo());
        assert_eq!(short.text(), "abcd\nefgh\nijkl\nmnop");

        let mut long = run(RegisterValue::blockwise("one\ntwo\nthree"), 2, 1);
        assert_eq!(long.text(), "onecd\ntwogh\nthreeijkl\nmnop");
        assert!(long.undo());
        assert!(!long.undo());

        let counted = run(RegisterValue::blockwise("one\ntwo\nthree"), 2, 2);
        assert_eq!(counted.text(), "oneonecd\ntwotwogh\nthreethreeijkl\nmnop");
    }

    #[test]
    fn visual_block_multiline_characterwise_and_linewise_puts_match_vim() {
        let mut document = Document::new("abcd\nefgh\nijkl\nmnop");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("X\nY"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('p'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "X\nYcd\ngh\nijkl\nmnop");
        assert_eq!(commands.register('"').unwrap().text, "ab\nef");
        assert_eq!(commands.cursor(), 0);
        assert!(document.undo());

        for (put, expected) in [
            ('p', "cd\ngh\nXX\nYY\nijkl\nmnop"),
            ('P', "XX\nYY\ncd\ngh\nijkl\nmnop"),
        ] {
            let mut document = Document::new("abcd\nefgh\nijkl\nmnop");
            let mut commands = CommandInterpreter::new();
            commands
                .registers
                .yank(Some('a'), RegisterValue::linewise("XX\nYY\n"));
            commands
                .registers
                .yank(Some('b'), RegisterValue::characterwise("keep"));
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            for key in [
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('"'),
                Key::Char('a'),
                Key::Char(put),
            ] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }
            assert_eq!(document.text(), expected, "Visual Block {put}");
            assert_eq!(commands.cursor(), if put == 'P' { 0 } else { 6 });
            if put == 'P' {
                assert_eq!(commands.register('"').unwrap().text, "keep");
            } else {
                assert_eq!(commands.register('"').unwrap().text, "ab\nef");
            }
            assert!(document.undo());
            assert!(!document.undo());
        }
    }

    #[test]
    fn visual_block_characterwise_literal_lf_stays_literal_in_forced_mac() {
        let mut document = Document::from_bytes_with_file_format(
            b"ab\rcd\nz".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        let literal =
            RegisterValue::try_new("X\nY", RegisterKind::Characterwise, Vec::new()).unwrap();
        commands.registers.yank(Some('a'), literal);
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('a'),
            Key::Char('p'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "X\nYb\nX\nYd\nz");
        assert_eq!(document.source_bytes(), b"X\nYb\rX\nYd\nz");
        assert_eq!(document.line_count(), 2);
    }

    #[test]
    fn normal_block_put_uses_layout_for_p_upper_p_follow_count_and_dot() {
        for (command, expected, cursor) in [
            ('p', "abcXYd\nefgZZh\nijkl\nmnop", 3),
            ('P', "abXYcd\nefZZgh\nijkl\nmnop", 2),
        ] {
            let mut document = Document::new("abcd\nefgh\nijkl\nmnop");
            let mut commands = CommandInterpreter::new();
            commands
                .registers
                .yank(Some('a'), RegisterValue::blockwise("XY\nZZ"));
            assert!(commands.set_cursor(&document, 2));
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            for key in [Key::Char('"'), Key::Char('a'), Key::Char(command)] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.cursor(), cursor, "{command}");
            assert!(document.undo());
            assert!(!document.undo());
        }

        let mut document = Document::new("abcd\nefgh\nijkl\nmnop");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::blockwise("X\nY"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Char('"'), Key::Char('a'), Key::Char('P')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "Xabcd\nYefgh\nijkl\nmnop");

        assert!(commands.set_cursor(&document, document.line_start(2).unwrap()));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Char('2'));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('.'));
        assert_eq!(document.text(), "Xabcd\nYefgh\nXXijkl\nYYmnop");
        assert!(document.undo());
        assert_eq!(document.text(), "Xabcd\nYefgh\nijkl\nmnop");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh\nijkl\nmnop");

        for (put, expected, cursor) in [
            ('p', "abcXYd\nefgZZh\nijkl", 12),
            ('P', "abXYcd\nefZZgh\nijkl", 11),
        ] {
            let mut document = Document::new("abcd\nefgh\nijkl");
            let mut commands = CommandInterpreter::new();
            commands
                .registers
                .yank(Some('a'), RegisterValue::blockwise("XY\nZZ"));
            assert!(commands.set_cursor(&document, 2));
            let snapshot = layout_snapshot(&document, 500.0);
            let mut context =
                LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
            for key in [
                Key::Char('"'),
                Key::Char('a'),
                Key::Char('g'),
                Key::Char(put),
            ] {
                layout_key(&mut commands, &mut document, &mut context, key);
            }
            assert_eq!(document.text(), expected, "g{put}");
            assert_eq!(commands.cursor(), cursor, "g{put} follows the final row");
        }
    }

    #[test]
    fn normal_block_put_hit_tests_ligature_bidi_and_emoji_rows() {
        let mut document = Document::new("fix\nאבג\n😀z");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::blockwise("X\nY\nZ"));
        let snapshot = layout_snapshot(&document, 500.0);
        let current = commands.current_visual_position(&snapshot).unwrap();
        let (row, first, x) = normal_block_insertion_origin(&snapshot, current, false).unwrap();
        assert_eq!(
            first, 2,
            "p advances past the indivisible fi shaping cluster"
        );
        let second = nearest_layout_caret_offset(&snapshot.rows[row + 1], x).unwrap();
        let third = nearest_layout_caret_offset(&snapshot.rows[row + 2], x).unwrap();
        for boundary in [first, second, third] {
            assert!(is_grapheme_boundary(document.text(), boundary));
        }
        let mut expected = document.text().to_owned();
        for (at, value) in [(third, "Z"), (second, "Y"), (first, "X")] {
            expected.insert_str(at, value);
        }
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [Key::Char('"'), Key::Char('a'), Key::Char('p')] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), expected);
    }

    #[test]
    fn failed_normal_block_put_is_atomic_for_document_cursor_and_registers() {
        let mut document =
            Document::from_bytes(b"ab\ncd".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::blockwise("\u{e9}\n😀"));
        let before_register = commands.register('"').cloned();
        let before_revision = document.revision();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Char('"'));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('a'));
        let result = commands.handle_with_layout(&mut document, InputEvent::key('p'), &mut context);
        assert!(matches!(
            result,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.source_bytes(), b"ab\ncd");
        assert_eq!(document.revision(), before_revision);
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.register('"'), before_register.as_ref());
        assert!(!document.undo());
    }

    #[test]
    fn visual_character_and_line_put_shapes_follow_vim_and_upper_p_preserves_unnamed() {
        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        keys(&mut commands, &mut document, "lvl\"ap");
        assert_eq!(document.text(), "aXYd");
        assert_eq!(commands.cursor(), 2);
        assert_eq!(commands.register('"').unwrap().text, "bc");
        assert_eq!(commands.register('-').unwrap().text, "bc");

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        commands
            .registers
            .yank(Some('b'), RegisterValue::characterwise("keep"));
        keys(&mut commands, &mut document, "lvl\"aP");
        assert_eq!(document.text(), "aXYd");
        assert_eq!(commands.cursor(), 2);
        assert_eq!(commands.register('"').unwrap().text, "keep");
        assert!(commands.register('-').is_none());

        let mut document = Document::new("abcd\nefgh\nijkl");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::linewise("XX\nYY\n"));
        keys(&mut commands, &mut document, "lvl\"aP");
        assert_eq!(document.text(), "a\nXX\nYY\nd\nefgh\nijkl");
        assert_eq!(commands.cursor(), 2);

        let mut document = Document::new("abcd\nefgh\nijkl");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        keys(&mut commands, &mut document, "Vj\"ap");
        assert_eq!(document.text(), "XY\nijkl");
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.register('1').unwrap().text, "abcd\nefgh\n");

        let mut document = Document::new("abcd\nefgh\nijkl");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::linewise("XX\nYY\n"));
        keys(&mut commands, &mut document, "Vj\"aP");
        assert_eq!(document.text(), "XX\nYY\nijkl");
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn dot_repeats_visual_character_replace_by_graphemes_and_ignores_dot_count() {
        let mut document = Document::new("a\u{301}b😀c xxYYz");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "vlrX");
        assert_eq!(document.text(), "XX😀c xxYYz");
        keys(&mut commands, &mut document, "f l3.");
        assert_eq!(document.text(), "XX😀c XXYYz");

        assert!(document.undo(), "one Visual dot is one undo unit");
        assert_eq!(document.text(), "XX😀c xxYYz");
        assert!(document.undo());
        assert_eq!(document.text(), "a\u{301}b😀c xxYYz");
    }

    #[test]
    fn dot_repeats_multiline_visual_character_shape_not_a_flat_count() {
        let mut document = Document::new("abcdef\nghijkl\nmnopqr\nstuvwx\nyzABCD");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "2lvj2ld");
        assert_eq!(document.text(), "abl\nmnopqr\nstuvwx\nyzABCD");
        keys(&mut commands, &mut document, "2j3.");
        assert_eq!(document.text(), "abl\nmnopqr\nstD");

        assert!(document.undo());
        assert_eq!(document.text(), "abl\nmnopqr\nstuvwx\nyzABCD");
        assert!(document.undo());
        assert_eq!(document.text(), "abcdef\nghijkl\nmnopqr\nstuvwx\nyzABCD");
    }

    #[test]
    fn visual_dollar_repeat_extends_to_the_target_line_end() {
        let mut document = Document::new("ab\ncdef\nXX\nuvwxyz\nTAIL");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "v$jrQ");
        assert_eq!(document.text(), "QQ\nQQQQ\nXX\nuvwxyz\nTAIL");
        assert!(commands.last_visual.unwrap().to_line_end);

        keys(&mut commands, &mut document, "2j9.");
        assert_eq!(document.text(), "QQ\nQQQQ\nQQ\nQQQQQQ\nTAIL");
        assert!(
            document.undo(),
            "one repeated Visual replace is one undo unit"
        );
        assert_eq!(document.text(), "QQ\nQQQQ\nXX\nuvwxyz\nTAIL");
    }

    #[test]
    fn dot_repeats_visual_line_change_and_replace_shapes() {
        let mut document = Document::new("aa\nbb\ncc\ndd\nee\nff");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "VjcX");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "X\ncc\ndd\nee\nff");
        keys(&mut commands, &mut document, "j4.");
        assert_eq!(document.text(), "X\nX\nee\nff");
        assert!(
            document.undo(),
            "the repeated change and insert are one unit"
        );
        assert_eq!(document.text(), "X\ncc\ndd\nee\nff");

        let mut document = Document::new("abc\ndefgh\nijkl\nmnop\nqrst");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "VjrX2j5.");
        assert_eq!(document.text(), "XXX\nXXXXX\nXXXX\nXXXX\nqrst");
    }

    #[test]
    fn visual_shift_retains_its_application_count_while_dot_ignores_its_count() {
        let mut document = Document::new("aa\nbb\ncc\ndd\nee");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "Vj3>2j2.");
        let padding = " ".repeat(12);
        assert_eq!(
            document.text(),
            format!("{padding}aa\n{padding}bb\n{padding}cc\n{padding}dd\nee")
        );
        assert!(document.undo());
        assert_eq!(
            document.text(),
            format!("{padding}aa\n{padding}bb\ncc\ndd\nee")
        );
    }

    #[test]
    fn dot_repeats_visual_case_and_join_shapes_without_count_override() {
        let mut document = Document::new("abCD efGH");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vl~w3.");
        assert_eq!(document.text(), "ABCD EFGH");

        let mut document = Document::new("aa\nbb\ncc\ndd\nee\nff");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "VjJj3.");
        assert_eq!(document.text(), "aa bb\ncc dd\nee\nff");
        assert!(document.undo());
        assert_eq!(document.text(), "aa bb\ncc\ndd\nee\nff");

        let mut document = Document::new("aa\nbb\ncc\ndd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "VjgJj9.");
        assert_eq!(document.text(), "aabb\nccdd");
    }

    #[test]
    fn visual_put_dot_repeats_its_internal_delete_register_policy() {
        let mut document = Document::new("abcdef uvwxyz");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        keys(&mut commands, &mut document, "vl\"ap0f l7.");
        assert_eq!(document.text(), "XYcdef wxyz");
        assert_eq!(commands.register('a').unwrap().text, "XY");
        assert_eq!(commands.register('"').unwrap().text, "uv");

        let mut document = Document::new("abcdef uvwxyz");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        commands
            .registers
            .yank(Some('b'), RegisterValue::characterwise("keep"));
        keys(&mut commands, &mut document, "vl\"aP0f l7.");
        assert_eq!(document.text(), "XYcdef wxyz");
        assert_eq!(commands.register('a').unwrap().text, "XY");
        assert_eq!(commands.register('"').unwrap().text, "keep");
    }

    #[test]
    fn empty_insert_and_failed_backward_deletions_replace_stale_dot_with_noop() {
        for deletion in [
            None,
            Some(Key::Backspace),
            Some(Key::Ctrl('w')),
            Some(Key::Ctrl('u')),
        ] {
            let mut document = Document::new("abcd");
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, "x");
            assert_eq!(document.text(), "bcd");

            keys(&mut commands, &mut document, "i");
            if let Some(deletion_key) = deletion {
                let output = key(&mut commands, &mut document, deletion_key);
                assert!(!output.document_changed);
            }
            key(&mut commands, &mut document, Key::Escape);
            assert!(matches!(
                commands.last_repeat.as_ref(),
                Some(RepeatAction::Insert {
                    program,
                    ..
                }) if program.steps.is_empty()
            ));

            keys(&mut commands, &mut document, ".");
            assert_eq!(document.text(), "bcd", "failed deletion: {deletion:?}");
            assert!(document.undo(), "only the seeded delete is undoable");
            assert_eq!(document.text(), "abcd");
            assert!(!document.undo());
        }
    }

    #[test]
    fn accepted_insert_delete_at_eof_replays_at_the_destination() {
        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "$a");
        let deletion = key(&mut commands, &mut document, Key::Delete);
        assert!(!deletion.document_changed, "Delete is accepted at EOF");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "0.");
        assert_eq!(document.text(), "acd");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd");
        assert!(!document.undo(), "the EOF no-op created no history node");
    }

    #[test]
    fn insert_edit_program_replays_preexisting_backspace_delete_and_ctrl_w() {
        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "lli");
        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "acdefgh");
        assert!(document.undo());
        assert_eq!(document.text(), "acd efgh");

        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "i");
        key(&mut commands, &mut document, Key::Delete);
        keys(&mut commands, &mut document, "Q");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "Qbcd Qfgh");

        let mut document = Document::new("one two three four");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "wea");
        key(&mut commands, &mut document, Key::Ctrl('w'));
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "e.");
        assert_eq!(document.text(), "one   four");
    }

    #[test]
    fn insert_program_preserves_hard_breaks_and_ctrl_u_on_dot() {
        use crate::document::{Encoding, Format};

        let mut document = Document::from_bytes_with_file_format(
            b"one\rtwo".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "Afoo");
        key(&mut commands, &mut document, Key::Enter);
        keys(&mut commands, &mut document, "bar");
        key(&mut commands, &mut document, Key::Ctrl('u'));
        keys(&mut commands, &mut document, "Z");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "j.");

        assert_eq!(document.source_bytes(), b"onefoo\rZ\rtwofoo\rZ");
        assert_eq!(document.line_count(), 4);
        assert!(document.undo(), "the compound dot is one undo unit");
        assert_eq!(document.source_bytes(), b"onefoo\rZ\rtwo");
    }

    #[test]
    fn insert_program_entry_count_and_dot_override_replay_semantic_delete() {
        let mut document = Document::new("abcdef ghijkl mnopqr");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "3i");
        key(&mut commands, &mut document, Key::Delete);
        keys(&mut commands, &mut document, "Q");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "QQQdef ghijkl mnopqr");

        keys(&mut commands, &mut document, "w2.");
        assert_eq!(document.text(), "QQQdef QQijkl mnopqr");
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "QQQdef QQijkl QQQpqr");
        assert!(document.undo());
        assert_eq!(document.text(), "QQQdef QQijkl mnopqr");
    }

    #[test]
    fn replace_program_replays_unicode_restoration_and_counts() {
        let mut document = Document::new("a\u{301}👩‍💻z αβγ");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "R");
        commands
            .handle(&mut document, InputEvent::text("é日"))
            .unwrap();
        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "W.");
        assert_eq!(document.text(), "é👩‍💻z éβγ");

        let mut document = Document::new("abcdefghij KLMNOPQRST uvwxyz");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3R");
        commands
            .handle(&mut document, InputEvent::text("XY"))
            .unwrap();
        key(&mut commands, &mut document, Key::Backspace);
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "XXXdefghij KLMNOPQRST uvwxyz");
        keys(&mut commands, &mut document, "w.");
        assert_eq!(document.text(), "XXXdefghij XXXNOPQRST uvwxyz");
        keys(&mut commands, &mut document, "w2.");
        assert_eq!(document.text(), "XXXdefghij XXXNOPQRST XXwxyz");
    }

    #[test]
    fn failed_latin1_insert_is_omitted_and_cannot_revive_stale_dot() {
        use crate::document::{Encoding, Format};

        let mut document =
            Document::from_bytes(b"abcd".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "x");
        keys(&mut commands, &mut document, "i");
        let failed = commands.handle(&mut document, InputEvent::text("😀"));
        assert!(matches!(
            failed,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(document.source_bytes(), b"bcd");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.source_bytes(), b"bcd");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"abcd");
    }

    #[test]
    fn counted_history_navigation_is_atomic_at_both_boundaries_and_on_branches() {
        let mut document = Document::new("");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "ia");
        key(&mut commands, &mut document, Key::Escape);

        let revision = document.revision();
        let output = keys(&mut commands, &mut document, "2u");
        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "a");
        assert_eq!(document.revision(), revision);

        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "");
        key(&mut commands, &mut document, Key::Char('2'));
        let output = key(&mut commands, &mut document, Key::Ctrl('r'));
        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(document.text(), "a");

        keys(&mut commands, &mut document, "ib");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "uic");
        key(&mut commands, &mut document, Key::Escape);
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "a");

        key(&mut commands, &mut document, Key::Char('2'));
        let output = key(&mut commands, &mut document, Key::Ctrl('r'));
        assert!(matches!(output.status, CommandStatus::Error(_)));
        assert_eq!(document.text(), "a");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(
            document.text(),
            "ca",
            "failed over-count preserves the preferred branch"
        );
    }

    #[test]
    fn till_repetition_counts_reverse_operator_and_visual_follow_vim() {
        let mut document = Document::new("abacadaba");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "ta");
        assert_eq!(commands.cursor(), 1);
        keys(&mut commands, &mut document, ";");
        assert_eq!(commands.cursor(), 3);
        keys(&mut commands, &mut document, ";");
        assert_eq!(commands.cursor(), 5);
        keys(&mut commands, &mut document, ",");
        assert_eq!(commands.cursor(), 3);

        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "ta2;");
        assert_eq!(
            commands.cursor(),
            3,
            "the adjacent match is part of the count"
        );
        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "ta3;");
        assert_eq!(commands.cursor(), 5);

        let mut document = Document::new("abacadaba");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "tad;");
        assert_eq!(document.text(), "aadaba");
        assert_eq!(commands.register('-').unwrap().text, "bac");

        let mut document = Document::new("abacadaba");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vta;d");
        assert_eq!(document.text(), "adaba");
    }

    #[test]
    fn failed_new_find_replaces_the_repeat_target_in_normal_and_operator_forms() {
        let mut document = Document::new("abacadaba");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "fa");
        assert_eq!(commands.cursor(), 2);
        let failed = keys(&mut commands, &mut document, "fz");
        assert_eq!(failed.status, CommandStatus::SearchNotFound);
        let repeated = keys(&mut commands, &mut document, ";");
        assert_eq!(repeated.status, CommandStatus::SearchNotFound);
        assert_eq!(commands.cursor(), 2);

        assert!(commands.set_cursor(&document, 0));
        let failed = keys(&mut commands, &mut document, "dfz");
        assert_eq!(failed.status, CommandStatus::SearchNotFound);
        assert_eq!(document.text(), "abacadaba");
        let repeated = keys(&mut commands, &mut document, ";");
        assert_eq!(repeated.status, CommandStatus::SearchNotFound);
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn normal_yanks_preserve_or_follow_the_motion_column() {
        for command in ["yy", "Y", "yj"] {
            let mut document = Document::new("abcd\nefgh\nijkl");
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, 2));
            let output = keys(&mut commands, &mut document, command);
            assert_eq!(commands.cursor(), 2, "{command}");
            assert!(!output.cursor_moved, "{command}");
        }

        let mut document = Document::new("abcd\nxy\nijkl");
        let mut commands = CommandInterpreter::new();
        let third_column_four = document.text().find("ijkl").unwrap() + 3;
        assert!(commands.set_cursor(&document, third_column_four));
        let output = keys(&mut commands, &mut document, "yk");
        assert_eq!(commands.cursor(), document.text().find("xy").unwrap() + 1);
        assert!(output.cursor_moved);

        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, document.text().find("three").unwrap()));
        keys(&mut commands, &mut document, "yge");
        assert_eq!(commands.cursor(), document.text().find("two").unwrap() + 2);
    }

    #[test]
    fn normal_case_operators_leave_the_cursor_at_the_range_start() {
        for (text, command, expected) in [
            ("one two", "g~w", "ONE two"),
            ("ONE two", "guw", "one two"),
            ("one two", "gUw", "ONE two"),
        ] {
            let mut document = Document::new(text);
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), expected, "{command}");
            assert_eq!(commands.cursor(), 0, "{command}");
        }

        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 4));
        keys(&mut commands, &mut document, "gUb");
        assert_eq!(document.text(), "ONE two");
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn counts_before_visual_entry_select_characters_lines_newlines_and_block_width() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3vd");
        assert_eq!(document.text(), "def");

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3vd");
        assert_eq!(document.text(), "cd");

        let mut document = Document::new("a\nb\nc\nd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3Vd");
        assert_eq!(document.text(), "d");

        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [Key::Char('3'), Key::Ctrl('v'), Key::Char('d')] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        assert_eq!(document.text(), "def");
    }

    #[test]
    fn escape_cancels_visual_subcommands_without_discarding_the_selection() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vlf");
        let cancelled = key(&mut commands, &mut document, Key::Escape);
        assert_eq!(cancelled.status, CommandStatus::Cancelled);
        assert!(!cancelled.mode_changed);
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        keys(&mut commands, &mut document, "x");
        assert_eq!(document.text(), "cdef");

        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vg");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.mode(), Mode::VisualLine);
        keys(&mut commands, &mut document, "x");
        assert_eq!(document.text(), "two");

        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [Key::Ctrl('v'), Key::Char('l'), Key::Char('f')] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        let cancelled = layout_key(&mut commands, &mut document, &mut context, Key::Escape);
        assert!(!cancelled.mode_changed);
        assert_eq!(commands.mode(), Mode::VisualBlock);
        layout_key(&mut commands, &mut document, &mut context, Key::Char('d'));
        assert_eq!(document.text(), "cdef");

        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vl3");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(
            commands.mode(),
            Mode::Normal,
            "a count alone is not a subcommand"
        );
    }

    #[test]
    fn visual_block_replace_is_register_pure_and_counted_change_inserts_once() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(None, RegisterValue::characterwise("seed"));
        commands
            .registers
            .set_last_insert(RegisterValue::characterwise("inserted"));
        let before = ['"', '0', '1', '-', 'a', '.'].map(|name| commands.register(name).cloned());
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('"'),
            Key::Char('%'),
            Key::Char('r'),
            Key::Char('X'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        assert_eq!(document.text(), "XXcd\nXXgh");
        let after = ['"', '0', '1', '-', 'a', '.'].map(|name| commands.register(name).cloned());
        assert_eq!(after, before);
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
        assert!(!document.undo());

        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('3'),
            Key::Char('c'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        commands
            .handle(&mut document, InputEvent::text("X"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xcd\nXgh");
        assert_eq!(commands.register('"').unwrap().text, "ab\nef");
        assert_eq!(commands.register('.').unwrap().text, "X");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
    }

    #[test]
    fn rejected_insert_and_replace_ctrl_r_do_not_split_the_surrounding_typed_undo_unit() {
        let mut document =
            Document::from_bytes(Vec::new(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("😀"));
        keys(&mut commands, &mut document, "ia");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        let failed = commands.handle(&mut document, InputEvent::key('a'));
        assert!(matches!(
            failed,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        keys(&mut commands, &mut document, "b");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "ab");
        assert!(document.undo());
        assert_eq!(document.text(), "");
        assert!(!document.undo());

        let mut document =
            Document::from_bytes(Vec::new(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("😀"));
        keys(&mut commands, &mut document, "Ra");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        let failed = commands.handle(&mut document, InputEvent::key('a'));
        assert!(matches!(
            failed,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        keys(&mut commands, &mut document, "b");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "ab");
        assert!(document.undo());
        assert_eq!(document.text(), "");
        assert!(!document.undo());
    }

    #[test]
    fn successful_ex_substitute_publishes_search_state_after_commit_only() {
        let mut document = Document::new("foo foo foo foo");
        let mut commands = CommandInterpreter::new();
        commands.last_search = Some((SearchDirection::Backward, "older".into()));

        keys(&mut commands, &mut document, ":s/foo/X/");
        let substituted = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(substituted.status, CommandStatus::Complete);
        assert_eq!(document.text(), "X foo foo foo");
        assert_eq!(
            commands.last_search,
            Some((SearchDirection::Backward, "foo".into()))
        );

        keys(&mut commands, &mut document, ":~");
        let repeated = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(repeated.status, CommandStatus::Complete);
        assert_eq!(document.text(), "X X foo foo");

        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "n");
        assert_eq!(commands.cursor(), document.text().rfind("foo").unwrap());
        keys(&mut commands, &mut document, "N");
        assert_eq!(commands.cursor(), document.text().find("foo").unwrap());

        let saved_search = commands.last_search.clone();
        keys(&mut commands, &mut document, ":s/missing/nope/");
        let failed = key(&mut commands, &mut document, Key::Enter);
        assert!(matches!(failed.status, CommandStatus::ExError(_)));
        assert_eq!(commands.last_search, saved_search);
    }

    #[test]
    fn counted_visual_restart_reuses_prior_linear_mode_and_shape() {
        let mut document = Document::new("αa\u{301}日bcdefghijklmnop");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "v2ly");
        let origin = advance_graphemes(document.text(), 0, 4);
        assert!(commands.set_cursor(&document, origin));

        keys(&mut commands, &mut document, "2V");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor, Some(origin));
        let extent = commands.visual_extent(&document);
        assert_eq!(
            document.text()[extent.range].graphemes(true).count(),
            6,
            "the combining sequence remains one selected character"
        );

        let text = (1..=12)
            .map(|line| format!("line{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut document = Document::new(&text);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vjy");
        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 3)));
        keys(&mut commands, &mut document, "3v");
        assert_eq!(commands.mode(), Mode::VisualLine);
        let extent = commands.visual_extent(&document);
        assert_eq!(document.text()[extent.range].lines().count(), 6);

        let mut document = Document::new("aa\nbbbb\ncccc\ndddd\neeee\nffff\ngggg");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vjly");
        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 3)));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Char('2'));
        layout_key(&mut commands, &mut document, &mut context, Key::Ctrl('v'));
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        let active_line = document
            .hard_line_snapshot()
            .line_at_offset(commands.cursor())
            .unwrap();
        assert_eq!(active_line.index(), 5);
        assert_eq!(
            grapheme_column(
                document.text(),
                &document.hard_line_snapshot(),
                commands.cursor()
            ),
            1
        );
    }

    #[test]
    fn counted_visual_restart_multiplies_block_width_and_height_in_layout_space() {
        let mut document =
            Document::new("iiiiiiii\niiiiiiii\niiiiiiii\niiiiiiii\niiiiiiii\niiiiiiii\niiiiiiii");
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('y'),
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(commands.register('0').unwrap().text, "ii\nii");
        assert!(commands.set_cursor(&document, nth_line_start(document.text(), 3)));
        layout_key(&mut commands, &mut document, &mut context, Key::Char('2'));
        assert!(commands.requires_layout_for_input(
            &document,
            &InputEvent::Key(Key::Char('v')),
            None,
        ));

        layout_key(&mut commands, &mut document, &mut context, Key::Char('v'));
        assert_eq!(commands.mode(), Mode::VisualBlock);
        let resolved = commands.resolved_visual_block(&document, &context).unwrap();
        assert_eq!(resolved.rows.len(), 4);
        let counts = resolved
            .range_set
            .segments
            .iter()
            .map(|segment| segment.grapheme_count)
            .collect::<Vec<_>>();
        assert_eq!(counts, vec![4; 4]);
    }

    #[test]
    fn percent_ignores_angle_brackets_but_angle_text_objects_remain_supported() {
        let mut document = Document::new("<one> (two)");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "%");
        assert_eq!(commands.cursor(), document.text().find(')').unwrap());

        let mut document = Document::new("<inside>");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "ldi<");
        assert_eq!(document.text(), "<>");
    }

    #[test]
    fn joins_use_ascii_indentation_and_leave_cursor_at_final_boundary() {
        for (source, command, expected, cursor) in [
            ("a\nb\nc", "3J", "a b c", 3),
            ("a\n b\n c", "3gJ", "a b c", 3),
            ("one\n.two", "J", "one .two", 3),
            ("\n  two", "J", "two", 0),
            ("one\n\u{a0}two", "J", "one \u{a0}two", 3),
        ] {
            let mut document = Document::new(source);
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, command);
            assert_eq!(document.text(), expected, "{command} on {source:?}");
            assert_eq!(commands.cursor(), cursor, "{command} on {source:?}");
        }
    }

    #[test]
    fn explicit_register_prefix_is_consumed_by_the_next_complete_command() {
        let seeded = RegisterValue::characterwise("KEEP");

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), seeded.clone());
        keys(&mut commands, &mut document, "\"alx");
        assert_eq!(document.text(), "acd");
        assert_eq!(commands.register('a'), Some(&seeded));

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "x");
        commands.registers.yank(Some('a'), seeded.clone());
        keys(&mut commands, &mut document, "\"au");
        assert_eq!(document.text(), "abcd");
        keys(&mut commands, &mut document, "x");
        assert_eq!(commands.register('a'), Some(&seeded));

        let mut document = Document::new("zero target tail");
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), seeded.clone());
        keys(&mut commands, &mut document, "\"a/target");
        assert_eq!(commands.requested_register, Some('a'));
        key(&mut commands, &mut document, Key::Enter);
        keys(&mut commands, &mut document, "x");
        assert_eq!(commands.register('a'), Some(&seeded));

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), seeded.clone());
        commands
            .registers
            .set_macro('b', vec![InputEvent::key('l')]);
        keys(&mut commands, &mut document, "\"a@b");
        keys(&mut commands, &mut document, "x");
        assert_eq!(document.text(), "acd");
        assert_eq!(commands.register('a'), Some(&seeded));

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), seeded.clone());
        keys(&mut commands, &mut document, "v\"alx");
        assert_eq!(document.text(), "cd");
        assert_eq!(commands.register('a'), Some(&seeded));
    }

    #[test]
    fn register_prefix_survives_valid_pending_command_grammar() {
        let mut document = Document::new("one two");
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "\"a");
        let pending = key(&mut commands, &mut document, Key::Char('d'));
        assert_eq!(pending.status, CommandStatus::Pending);
        let changed = key(&mut commands, &mut document, Key::Char('w'));
        assert_eq!(changed.status, CommandStatus::Complete);
        assert_eq!(commands.register('a').unwrap().text, "one ");
        assert_eq!(commands.requested_register, None);
    }

    #[test]
    fn search_anchors_follow_semantic_hard_lines_not_literal_lf_content() {
        let mut document = Document::from_bytes_with_file_format(
            b"one\nliteral\rtwo\nliteral\rthree".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();

        keys(&mut commands, &mut document, "/^literal");
        let missing = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(missing.status, CommandStatus::SearchNotFound);

        keys(&mut commands, &mut document, "/^two");
        let found = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(found.status, CommandStatus::Complete);
        assert_eq!(
            document
                .hard_line_snapshot()
                .line_at_offset(commands.cursor())
                .unwrap()
                .index(),
            1
        );

        assert!(commands.set_cursor(&document, 0));
        keys(&mut commands, &mut document, "/one$");
        let missing = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(missing.status, CommandStatus::SearchNotFound);
        keys(&mut commands, &mut document, "/literal$");
        let found = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(found.status, CommandStatus::Complete);
        assert_eq!(commands.cursor(), "one\n".len());

        assert!(commands.set_cursor(&document, document.projection().text_tree().byte_len()));
        keys(&mut commands, &mut document, "?^two");
        let backward = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(backward.status, CommandStatus::Complete);
        assert_eq!(
            commands.last_search.as_ref().unwrap().0,
            SearchDirection::Backward
        );
        assert_eq!(
            document
                .hard_line_snapshot()
                .line_at_offset(commands.cursor())
                .unwrap()
                .index(),
            1
        );
    }

    #[test]
    fn normalized_tab_is_a_repeatable_normal_and_visual_replacement() {
        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('r'));
        key(&mut commands, &mut document, Key::Tab);
        assert_eq!(document.text(), "\tbcd");
        assert!(commands.set_cursor(&document, 2));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "\tb\td");
        assert!(document.undo());
        assert_eq!(document.text(), "\tbcd");

        let mut document = Document::new("abcd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vlr");
        key(&mut commands, &mut document, Key::Tab);
        assert_eq!(document.text(), "\t\tcd");
        assert!(commands.set_cursor(&document, 2));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "\t\t\t\t");

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vjr");
        key(&mut commands, &mut document, Key::Tab);
        assert_eq!(document.text(), "\t\t\n\t\t");
    }

    #[test]
    fn normalized_tab_replaces_a_visual_block_without_register_effects() {
        let mut document = Document::new("abcd\nefgh");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("KEEP"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for key in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('r'),
            Key::Tab,
        ] {
            layout_key(&mut commands, &mut document, &mut context, key);
        }
        assert_eq!(document.text(), "\t\tcd\n\t\tgh");
        assert_eq!(commands.register('a').unwrap().text, "KEEP");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
    }

    #[test]
    fn normalized_enter_normal_replace_inserts_one_break_and_repeats_typed_operand() {
        let mut document = Document::new("abcdef ghijkl");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 1));

        keys(&mut commands, &mut document, "3r");
        let replaced = key(&mut commands, &mut document, Key::Enter);
        assert!(replaced.document_changed);
        assert_eq!(document.text(), "a\nef ghijkl");
        assert_eq!(commands.cursor(), 2, "cursor starts the following line");
        assert_eq!(document.line_count(), 2);
        assert_eq!(commands.register('.').unwrap().text, "\n");
        assert_eq!(commands.register('.').unwrap().hard_break_offsets(), &[0]);

        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "a\n\nghijkl");
        assert_eq!(commands.cursor(), 3);
        assert!(document.undo(), "dot is one undo unit");
        assert_eq!(document.text(), "a\nef ghijkl");

        assert!(commands.set_cursor(&document, 2));
        keys(&mut commands, &mut document, "2.");
        assert_eq!(document.text(), "a\n\n ghijkl");
        assert_eq!(document.line_count(), 3);
        assert!(document.undo(), "counted dot is one undo unit");
        assert_eq!(document.text(), "a\nef ghijkl");
        assert!(document.undo(), "the original replacement is one undo unit");
        assert_eq!(document.text(), "abcdef ghijkl");
        assert!(!document.undo());

        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "3r");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "\n");
        assert_eq!(commands.cursor(), 1, "the trailing empty line owns EOF");
        assert_eq!(
            document
                .hard_line_snapshot()
                .line_at_offset(commands.cursor())
                .unwrap()
                .index(),
            1
        );
    }

    #[test]
    fn normalized_enter_normal_replace_preflights_count_atomically() {
        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        commands.last_repeat = Some(RepeatAction::Noop);
        let revision = document.revision();

        keys(&mut commands, &mut document, "3r");
        let failed = key(&mut commands, &mut document, Key::Enter);
        assert_eq!(
            failed.status,
            CommandStatus::Error("not enough characters on line".into())
        );
        assert_eq!(document.text(), "ab\ncd");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.pending, Pending::None);
        assert_eq!(commands.last_repeat, Some(RepeatAction::Noop));
        assert!(!document.undo());
    }

    #[test]
    fn normalized_enter_normal_replace_spells_each_source_format_and_encoding() {
        for format in [Format::PlainText, Format::Markdown] {
            for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le] {
                for (file_format, delimiter) in [
                    (FileFormat::Unix, "\n"),
                    (FileFormat::Dos, "\r\n"),
                    (FileFormat::Mac, "\r"),
                ] {
                    let delimiter = if format == Format::Markdown {
                        delimiter.repeat(2)
                    } else {
                        delimiter.to_owned()
                    };
                    let original = encoding.encode_fragment("abcdef").unwrap();
                    let mut document = Document::from_bytes_with_file_format(
                        original.clone(),
                        encoding,
                        format,
                        file_format,
                    )
                    .unwrap();
                    let mut commands = CommandInterpreter::new();
                    assert!(commands.set_cursor(&document, 1));

                    keys(&mut commands, &mut document, "3r");
                    key(&mut commands, &mut document, Key::Enter);
                    assert_eq!(document.text(), "a\nef");
                    assert_eq!(
                        document.source_bytes(),
                        encoding
                            .encode_fragment(&format!("a{delimiter}ef"))
                            .unwrap(),
                        "{format:?}/{encoding:?}/{file_format:?}"
                    );
                    assert_eq!(commands.cursor(), 2);
                    assert!(document.text_point(1).is_ok());
                    assert!(document.text_point(2).is_ok());
                    assert!(document.undo());
                    assert_eq!(document.source_bytes(), original);
                    assert!(!document.undo());
                }
            }
        }
    }

    #[test]
    fn normalized_enter_visual_character_and_line_insert_literal_cr() {
        let mut document = Document::new("abc\ndef");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "vllr");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "\r\r\r\ndef");
        assert_eq!(document.source_bytes(), b"\r\r\r\ndef");
        assert_eq!(document.line_count(), 2);
        assert!(document.text_point(3).is_ok());
        assert_eq!(commands.register('.').unwrap().text, "\r");
        assert!(commands
            .register('.')
            .unwrap()
            .hard_break_offsets()
            .is_empty());

        assert!(commands.set_cursor(&document, document.text().find('d').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "\r\r\r\n\r\r\r");
        assert_eq!(document.line_count(), 2);
        assert!(document.undo());
        assert_eq!(document.text(), "\r\r\r\ndef");
        assert!(document.undo());
        assert_eq!(document.text(), "abc\ndef");

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "Vjr");
        key(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.text(), "\r\r\n\r\r");
        assert_eq!(document.line_count(), 2);
        assert!(document.undo(), "Visual Line replacement is one undo unit");
        assert_eq!(document.text(), "ab\ncd");
    }

    #[test]
    fn visual_literal_cr_acceptance_and_rejection_follow_line_ending_projection() {
        for format in [Format::PlainText, Format::MarkdownSource] {
            let mut dos = Document::from_bytes_with_file_format(
                b"a\r\nb".to_vec(),
                Encoding::Utf8,
                format,
                FileFormat::Dos,
            )
            .unwrap();
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut dos, "vr");
            key(&mut commands, &mut dos, Key::Enter);
            assert_eq!(dos.text(), "\r\nb");
            assert_eq!(dos.source_bytes(), b"\r\r\nb");
            assert_eq!(dos.line_count(), 2);
            assert!(dos.text_point(1).is_ok());

            for (source, file_format) in [
                (b"a\nb".as_slice(), FileFormat::Dos),
                (b"a\rb".as_slice(), FileFormat::Mac),
            ] {
                let mut document = Document::from_bytes_with_file_format(
                    source.to_vec(),
                    Encoding::Utf8,
                    format,
                    file_format,
                )
                .unwrap();
                let mut commands = CommandInterpreter::new();
                commands.last_repeat = Some(RepeatAction::Noop);
                keys(&mut commands, &mut document, "vr");
                let revision = document.revision();
                let visual_anchor = commands.visual_anchor;
                let cursor = commands.cursor();

                let failed = commands.handle(&mut document, InputEvent::Key(Key::Enter));
                let expected_error = match file_format {
                    // Mac projection consumes every source CR, so the scalar
                    // itself cannot be represented as literal formatted text.
                    FileFormat::Mac => DocumentError::UnrepresentableFormattedCharacter {
                        format,
                        character: '\r',
                    },
                    // DOS can retain a literal CR, but here it would combine
                    // with the following bare LF and change the hard break.
                    FileFormat::Dos => DocumentError::FormattedPayloadCannotReproject,
                    FileFormat::Unix => unreachable!(),
                };
                assert_eq!(failed, Err(expected_error));
                assert_eq!(document.source_bytes(), source);
                assert_eq!(document.revision(), revision);
                assert_eq!(commands.mode(), Mode::VisualCharacter);
                assert_eq!(commands.visual_anchor, visual_anchor);
                assert_eq!(commands.cursor(), cursor);
                assert_eq!(commands.pending, Pending::None);
                assert_eq!(commands.last_repeat, Some(RepeatAction::Noop));
                assert!(!document.undo());
            }
        }
    }

    #[test]
    fn normalized_enter_visual_block_splits_each_nonempty_row_once_and_dots() {
        let mut document = Document::new("abc\nDEF\nghi\nJKL");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("KEEP"));
        commands
            .registers
            .set_last_insert(RegisterValue::characterwise("seed"));
        let registers = ['"', '0', '1', '-', 'a', '.'].map(|name| commands.register(name).cloned());
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('j'),
            Key::Char('r'),
            Key::Enter,
        ] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        assert_eq!(document.text(), "\nc\n\nF\nghi\nJKL");
        assert_eq!(document.line_count(), 6);
        assert_eq!(
            commands.cursor(),
            0,
            "Vim leaves the cursor on the first split row"
        );
        assert_eq!(
            ['"', '0', '1', '-', 'a', '.'].map(|name| commands.register(name).cloned()),
            registers,
            "Visual Block r is register-pure"
        );

        assert!(commands.set_cursor(&document, document.text().find("ghi").unwrap()));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Char('.'));
        assert_eq!(document.text(), "\nc\n\nF\n\ni\n\nL");
        assert_eq!(document.line_count(), 8);
        assert!(document.undo(), "block dot is one undo unit");
        assert_eq!(document.text(), "\nc\n\nF\nghi\nJKL");
        assert!(document.undo(), "block replacement is one undo unit");
        assert_eq!(document.text(), "abc\nDEF\nghi\nJKL");
        assert!(!document.undo());
    }

    #[test]
    fn normalized_enter_visual_block_skips_short_and_empty_rows() {
        let mut document = Document::new("abcd\nx\n\nwxyz");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 2));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [
            Key::Ctrl('v'),
            Key::Char('l'),
            Key::Char('3'),
            Key::Char('j'),
            Key::Char('r'),
            Key::Enter,
        ] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        assert_eq!(document.text(), "ab\n\nx\n\nwx\n");
        assert_eq!(document.line_count(), 6);
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nx\n\nwxyz");
        assert!(!document.undo());
    }

    #[test]
    fn normalized_enter_visual_block_uses_source_file_format() {
        for format in [Format::PlainText, Format::Markdown] {
            for (file_format, source, expected) in [
                (
                    FileFormat::Unix,
                    b"ab\ncd".as_slice(),
                    b"\nb\n\nd".as_slice(),
                ),
                (
                    FileFormat::Dos,
                    b"ab\r\ncd".as_slice(),
                    b"\r\nb\r\n\r\nd".as_slice(),
                ),
                (
                    FileFormat::Mac,
                    b"ab\rcd".as_slice(),
                    b"\rb\r\rd".as_slice(),
                ),
            ] {
                let source = if format == Format::Markdown {
                    String::from_utf8(source.to_vec())
                        .unwrap()
                        .replace(file_format.spelling(), &file_format.spelling().repeat(2))
                        .into_bytes()
                } else {
                    source.to_vec()
                };
                let expected = if format == Format::Markdown {
                    String::from_utf8(expected.to_vec())
                        .unwrap()
                        .replace(file_format.spelling(), &file_format.spelling().repeat(2))
                        .into_bytes()
                } else {
                    expected.to_vec()
                };
                let mut document = Document::from_bytes_with_file_format(
                    source.clone(),
                    Encoding::Utf8,
                    format,
                    file_format,
                )
                .unwrap();
                let mut commands = CommandInterpreter::new();
                let snapshot = layout_snapshot(&document, 500.0);
                let mut context =
                    LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
                for input in [Key::Ctrl('v'), Key::Char('j'), Key::Char('r'), Key::Enter] {
                    layout_key(&mut commands, &mut document, &mut context, input);
                }
                assert_eq!(document.text(), "\nb\n\nd");
                assert_eq!(
                    document.source_bytes(),
                    expected,
                    "{format:?}/{file_format:?}"
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source);
            }
        }
    }

    #[test]
    fn visual_block_text_lf_stays_literal_through_dot_in_forced_mac() {
        let mut document = Document::from_bytes_with_file_format(
            b"ab\rcd\ref".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [Key::Ctrl('v'), Key::Char('j'), Key::Char('r')] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        commands
            .handle_with_layout(&mut document, InputEvent::Text("\n".into()), &mut context)
            .unwrap();
        assert_eq!(document.source_bytes(), b"\nb\r\nd\ref");
        assert_eq!(document.line_count(), 3);
        let Some(RepeatAction::VisualBlock(VisualBlockRepeat {
            action: VisualBlockRepeatAction::Replace { replacement },
            ..
        })) = commands.last_repeat.as_ref()
        else {
            panic!("Visual Block replacement records a repeat recipe")
        };
        assert_eq!(replacement.text, "\n");
        assert!(replacement.hard_break_offsets().is_empty());

        assert!(commands.set_cursor(&document, document.text().find("ef").unwrap()));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Char('.'));
        assert_eq!(document.source_bytes(), b"\nb\r\nd\r\nf");
        assert_eq!(document.line_count(), 3, "dot retained literal LF metadata");
    }

    #[test]
    fn rejected_visual_block_literal_lf_restores_selection_and_repeat_state() {
        let mut document = Document::from_bytes_with_file_format(
            b"ab\r\ncd".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Dos,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        commands.last_repeat = Some(RepeatAction::Noop);
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("KEEP"));
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        for input in [Key::Ctrl('v'), Key::Char('j'), Key::Char('r')] {
            layout_key(&mut commands, &mut document, &mut context, input);
        }
        let selection = commands.visual_block.clone();
        let revision = document.revision();

        let failed =
            commands.handle_with_layout(&mut document, InputEvent::Text("\n".into()), &mut context);
        assert_eq!(failed, Err(DocumentError::FormattedPayloadCannotReproject));
        assert_eq!(document.source_bytes(), b"ab\r\ncd");
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.mode(), Mode::VisualBlock);
        assert_eq!(commands.visual_block, selection);
        assert_eq!(commands.pending, Pending::None);
        assert_eq!(commands.last_repeat, Some(RepeatAction::Noop));
        assert_eq!(commands.register('a').unwrap().text, "KEEP");
        assert!(!document.undo());
    }

    #[test]
    fn insert_and_replace_ctrl_r_remain_dot_repeatable_and_split_undo() {
        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("λ"));
        keys(&mut commands, &mut document, "iA");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        key(&mut commands, &mut document, Key::Char('a'));
        keys(&mut commands, &mut document, "B");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "AλBabcd efgh");
        assert!(commands.set_cursor(&document, document.text().find('e').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "AλBabcd AλBefgh");
        assert!(document.undo(), "dot is one undo unit");
        assert_eq!(document.text(), "AλBabcd efgh");
        assert!(document.undo(), "post-register typing is its own unit");
        assert_eq!(document.text(), "Aλabcd efgh");
        assert!(document.undo(), "Ctrl-R payload is its own unit");
        assert_eq!(document.text(), "Aabcd efgh");
        assert!(document.undo(), "pre-register typing is its own unit");
        assert_eq!(document.text(), "abcd efgh");

        let mut document = Document::new("abcd wxyz");
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("XY"));
        key(&mut commands, &mut document, Key::Char('R'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        key(&mut commands, &mut document, Key::Char('a'));
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "XYcd wxyz");
        assert!(commands.set_cursor(&document, document.text().find('w').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "XYcd XYyz");
    }

    #[test]
    fn structured_ctrl_r_payload_survives_insert_dot_in_forced_mac() {
        let mut document = Document::from_bytes_with_file_format(
            b"x\ry".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(
            Some('a'),
            RegisterValue::try_new("\nβ", RegisterKind::Characterwise, vec![0]).unwrap(),
        );

        keys(&mut commands, &mut document, "iA");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        key(&mut commands, &mut document, Key::Char('a'));
        keys(&mut commands, &mut document, "B");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.source_bytes(), "A\rβBx\ry".as_bytes());
        keys(&mut commands, &mut document, "G.");
        assert_eq!(document.source_bytes(), "A\rβBx\rA\rβBy".as_bytes());
        assert_eq!(document.line_count(), 4);
    }

    #[test]
    fn edit_mode_motion_rebuilds_dot_from_only_the_post_motion_fragment() {
        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "iX");
        key(&mut commands, &mut document, Key::Right);
        commands
            .handle(&mut document, InputEvent::text("β"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "Xaβbcd efgh");
        assert!(commands.set_cursor(&document, document.text().find('e').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "Xaβbcd βefgh");

        let mut document = Document::new("ab\ncd");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "iX");
        let snapshot = layout_snapshot(&document, 500.0);
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0.0, 500.0).unwrap());
        layout_key(&mut commands, &mut document, &mut context, Key::Down);
        keys(&mut commands, &mut document, "Y");
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.set_cursor(&document, 0));
        key(&mut commands, &mut document, Key::Char('.'));
        assert!(document.text().starts_with('Y'));
        assert!(!document.text().starts_with("XY"));
    }

    #[test]
    fn ctrl_o_normal_change_owns_dot_until_later_inserted_text() {
        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        key(&mut commands, &mut document, Key::Ctrl('o'));
        key(&mut commands, &mut document, Key::Char('x'));
        assert_eq!(commands.mode(), Mode::Insert);
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.set_cursor(&document, document.text().find('e').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "bcd fgh");

        let mut document = Document::new("abcd efgh");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        key(&mut commands, &mut document, Key::Ctrl('o'));
        key(&mut commands, &mut document, Key::Char('x'));
        commands
            .handle(&mut document, InputEvent::text("β"))
            .unwrap();
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.set_cursor(&document, document.text().find('e').unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "βbcd βefgh");
    }

    #[test]
    fn rejected_latin1_ctrl_r_is_absent_from_the_surviving_dot_recipe() {
        let mut document =
            Document::from_bytes(b"cd".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("😀"));
        keys(&mut commands, &mut document, "iA");
        key(&mut commands, &mut document, Key::Ctrl('r'));
        let failed = commands.handle(&mut document, InputEvent::key('a'));
        assert!(matches!(
            failed,
            Err(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        keys(&mut commands, &mut document, "B");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.source_bytes(), b"ABcd");
        assert!(commands.set_cursor(&document, 2));
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.source_bytes(), b"ABABcd");
    }
}
