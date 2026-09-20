use crate::command::clipboard::ClipboardCommandContext;
use crate::command::composition::{
    CompositionCommit, CompositionError, CompositionEvent, CompositionOverlay,
    CompositionRestoration, CompositionSession,
};
use crate::command::ex_execute::{
    CompletedExArtifactWrite, ExFileRequest, ExFrontendRequest, ExOptionName, ExOptionValue,
    ExPostWriteDisposition, ExRequestTag, PreparedExArtifactWrite, PreparedExFileRequest,
    TaggedExFileRequest,
};
use crate::command::layout_motion::{LayoutDemand, LayoutMotionError, Viewport};
use crate::command::{
    command_status_stops_compound, ex_normal_target_position, rebase_ex_normal_targets,
    BufferCommandState, CommandContext, CommandInterpreter, CommandOutput, CommandPlan,
    CommandPresentationRequest, CommandResolution, CommandStatus, CommandStep, CountError,
    ExNormalReplayPlan, ExNormalTarget, InputEvent, Key, LayoutCommandContext, MacroReplayPlan,
    Mode, ReplayPlan, COMPOUND_REPLAY_LIMIT, MACRO_REPLAY_EVENT_LIMIT,
};
use crate::document::StyleProperty;
use crate::document::{
    ArtifactOverwrite, ArtifactPath, ArtifactWriteCompletion, ArtifactWriteCompletionStatus,
    ArtifactWriteIntent, ArtifactWriteScope, ArtifactWriteToken, Association, BoundaryAffinity,
    ConfigurationStyleIntent, DeletionRecovery, Document, DocumentError, DocumentId, Encoding,
    FileFormat, Format, FormatOperation, HardLineSourceRangeError, HistoryError, HistoryLocation,
    HistoryNavigationRequest, HistoryRestoration, HistoryRestorationSnapshot, MappingOutcome,
    ModelRequest, ModelTransactionError, PersistenceError, PositionDomain, PositionError,
    PositionMap, PreparedArtifactWrite, Revision, SemanticInlineStyle, StyleApplication,
    StyleDefinitionFieldEdit, StyleId, StyleModelIntent, StyleModelRequest, StyleNamespace,
    StyleSheetRevision, TextAnchor,
};
use crate::layout::{
    compute_layout_job, hard_line_ranges, inspect_layout_provider, install_layout_job,
    prepare_layout_job, DocumentLayoutStyles, InstalledLayoutJob, LayoutCancellationToken, LayoutComputationError,
    LayoutCoverage, LayoutEngine, LayoutError, LayoutExecutionContext, LayoutInstallTarget,
    LayoutJobCandidate, LayoutJobError, LayoutJobId, LayoutJobInstallRejection, LayoutJobPriority,
    LayoutJobRegion, LayoutProviderRequirements, LayoutRevision, LongLineCheckpointCache,
    LongLineLayoutCheckpoint, MeasurementEnvironmentId, MetricsGeneration, PaintStyleRun,
    ParagraphLayoutStyle, ShapeStyleRun, TextMeasurementProvider, ViewConfigurationGeneration,
    ViewLayout, ViewportLayoutRegion, MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

mod input_layout;
mod prelayout;
mod formatting;
mod startup;
mod ex_files;
mod completion;
mod completion_layout;
mod search;
pub use completion_layout::CompletionPopupAnchor;
mod viewport;
use viewport::capture_caret_baseline_anchor;

static NEXT_STYLE_EDIT_GROUP_ID: AtomicU64 = AtomicU64::new(1);

fn boolean_style_state(values: impl Iterator<Item = bool>) -> SemanticStyleState {
    let mut any = false;
    let mut all = true;
    for value in values {
        any |= value;
        all &= value;
    }
    if !any {
        SemanticStyleState::Off
    } else if all {
        SemanticStyleState::On
    } else {
        SemanticStyleState::Mixed
    }
}

/// Layout exports only runs that differ from its document default. Uncovered
/// selected text still participates in a mixed-state query with that default.
fn ranged_boolean_style_state(
    text: &crate::document::FormattedTextTree,
    selected: std::ops::Range<usize>,
    default: bool,
    runs: impl Iterator<Item = (std::ops::Range<usize>, bool)>,
) -> SemanticStyleState {
    let mut cursor = selected.start;
    let mut values = Vec::new();
    let has_text = |start, end| {
        text.slice(start..end)
            .ok()
            .is_some_and(|value| value.chars().any(|character| character != '\n'))
    };
    for (range, value) in runs {
        let start = range.start.max(selected.start);
        let end = range.end.min(selected.end);
        if start >= end {
            continue;
        }
        if cursor < start && has_text(cursor, start) {
            values.push(default);
        }
        if has_text(start, end) {
            values.push(value);
        }
        cursor = cursor.max(end);
    }
    if cursor < selected.end && has_text(cursor, selected.end) {
        values.push(default);
    }
    boolean_style_state(values.into_iter())
}

/// Stable identity for a view attached to the core buffer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ViewId(pub u64);

/// Logical selection shape reported without consulting layout geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalSelectionKind {
    None,
    Character,
    Line,
    Block,
}

/// Exact snapshot-local identity of one active linear Visual selection.
/// The normalized range is retained with the directed endpoints so a stale
/// native menu action cannot silently target a different selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalSelectionIdentity {
    view: ViewId,
    document: DocumentId,
    revision: Revision,
    kind: LogicalSelectionKind,
    anchor: usize,
    active: usize,
    active_affinity: BoundaryAffinity,
    range: std::ops::Range<usize>,
}

impl LogicalSelectionIdentity {
    pub fn view(&self) -> ViewId {
        self.view
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn kind(&self) -> LogicalSelectionKind {
        self.kind
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_affinity(&self) -> BoundaryAffinity {
        self.active_affinity
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.range.clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticStyleState {
    Off,
    On,
    Mixed,
}

/// Query-only native formatting presentation for one semantic inline style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticStylePresentation {
    selection_kind: LogicalSelectionKind,
    selection: Option<LogicalSelectionIdentity>,
    state: SemanticStyleState,
    can_set: bool,
    can_clear: bool,
}

impl SemanticStylePresentation {
    pub fn selection_kind(&self) -> LogicalSelectionKind {
        self.selection_kind
    }

    pub fn selection(&self) -> Option<&LogicalSelectionIdentity> {
        self.selection.as_ref()
    }

    pub fn state(&self) -> SemanticStyleState {
        self.state
    }

    pub fn can_set(&self) -> bool {
        self.can_set
    }

    pub fn can_clear(&self) -> bool {
        self.can_clear
    }
}

/// Process-wide, non-reused identity for one explicitly owned live style-edit
/// group. The complete [`StyleEditGroup`] value remains scoped to its core,
/// document, and owning view; the numeric ID alone is never sufficient
/// authority to mutate a group.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct StyleEditGroupId(pub u64);

/// Immutable capability returned when a frontend begins a live style-edit
/// gesture. The begin identities deliberately remain fixed while successful
/// edits advance the document and style-sheet revisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleEditGroup {
    id: StyleEditGroupId,
    view: ViewId,
    document: DocumentId,
    begin_document_revision: Revision,
    begin_style_sheet_revision: StyleSheetRevision,
}

impl StyleEditGroup {
    pub fn id(self) -> StyleEditGroupId {
        self.id
    }

    pub fn view(self) -> ViewId {
        self.view
    }

    pub fn document(self) -> DocumentId {
        self.document
    }

    pub fn begin_document_revision(self) -> Revision {
        self.begin_document_revision
    }

    pub fn begin_style_sheet_revision(self) -> StyleSheetRevision {
        self.begin_style_sheet_revision
    }

    pub fn from_parts(
        id: StyleEditGroupId,
        view: ViewId,
        document: DocumentId,
        begin_document_revision: Revision,
        begin_style_sheet_revision: StyleSheetRevision,
    ) -> Self {
        Self {
            id,
            view,
            document,
            begin_document_revision,
            begin_style_sheet_revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreIdentifierKind {
    View,
    LayoutJob,
    StyleEditGroup,
}

/// Typed ownership and lifecycle failures for a live style-edit group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleEditGroupError {
    AlreadyActive(StyleEditGroupId),
    NoActiveGroup,
    WrongGroup {
        expected: StyleEditGroupId,
        actual: StyleEditGroupId,
    },
    WrongOwner {
        expected: ViewId,
        actual: ViewId,
    },
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    IdentityMismatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    ReadFile { document: DocumentId, revision: Revision, after: usize, bytes: Vec<u8> },
    FlushMappingPrefix,
    FlushMappingPrefixWithClipboard(ClipboardCommandContext),
    EditCommandLine(crate::command::CommandLineEditRequest),
    SetDirectCharacterProperties {
        expected: LogicalSelectionIdentity,
        values: Vec<(
            crate::document::StyleProperty,
            crate::document::StylePropertyValue,
        )>,
    },
    EditDirectProperties {
        expected: LogicalSelectionIdentity,
        values: Vec<(crate::document::StyleProperty, Option<crate::document::StylePropertyValue>)>,
    },
    EditDirectProperty {
        expected: LogicalSelectionIdentity,
        property: crate::document::StyleProperty,
        value: Option<crate::document::StylePropertyValue>,
    },
    /// Ordinary input cancels active marked text before dispatch. Undo/redo
    /// continue in the same coordinator turn after cancellation; other input
    /// is consumed by the cancellation.
    Input(InputEvent),
    /// Ordinary input with immutable clipboard reads and write capabilities
    /// captured by the host before this coordinator turn. Provider I/O is not
    /// performed while core owns mutable document/view state.
    InputWithClipboard {
        input: InputEvent,
        clipboard: ClipboardCommandContext,
    },
    /// Begin, update, commit, or explicitly cancel per-view marked text.
    Composition(CompositionEvent),
    Resize {
        width: f32,
        height: f32,
    },
    /// Change only this view's magnification. Scale participates in shaping,
    /// wrapping, and layout identity but never changes source or semantic
    /// projection state.
    SetScale(f32),
    /// Update the shared last-search target without moving any view. The FFI
    /// entry point derives this literal from an exact core selection before
    /// dispatching the event.
    SetFindPattern(String),
    /// Materialize and reveal the active endpoint of the current core-owned
    /// Visual selection. The menu-specific FFI entry point rejects non-Visual
    /// modes before dispatch.
    RevealSelection,
    /// Set or clear one source-backed semantic inline style on an exact,
    /// core-owned linear Visual selection. The identity contains no layout
    /// generation and remains valid when the selection is offscreen.
    SetSelectionSemanticStyle {
        expected: LogicalSelectionIdentity,
        style: SemanticInlineStyle,
        enabled: bool,
    },
    SetWrap(bool),
    SetParagraphFlow(bool),
    /// Change the view-local domain used by unprefixed line commands.
    SetLineMode(crate::command::LineMode),
    SetSmartQuotes(bool),
    /// Change the shared source line-ending spelling through one exact,
    /// revision-bound model transaction. This is intentionally typed rather
    /// than routed through Ex parsing so native UI can preserve model policy
    /// and structured failures.
    SetFileFormat {
        document: DocumentId,
        revision: Revision,
        target: FileFormat,
    },
    SetIncludeStyleDefinitionsInFile {
        document: DocumentId,
        revision: Revision,
        enabled: bool,
    },
    SetFormat {
        document: DocumentId,
        revision: Revision,
        target: Format,
        operation: FormatOperation,
    },
    SetEncoding {
        document: DocumentId,
        revision: Revision,
        target: Encoding,
    },
    SetListStyle {
        expected: LogicalSelectionIdentity,
        style: Option<crate::document::ListStyle>,
    },
    IndentList {
        expected: LogicalSelectionIdentity,
        unindent: bool,
    },
    SetParagraphStyle {
        expected: LogicalSelectionIdentity,
        style: StyleId,
    },
    AssignNamedStyle {
        expected: LogicalSelectionIdentity,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: StyleId,
    },
    EditNamedStyleDefinition {
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        edit: crate::document::StyleDefinitionEdit,
    },
    /// Atomically edit one field through its core-owned source or configuration
    /// authority. Callers cannot promote synthetic definitions to editable.
    EditGeneratedStyle {
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: StyleId,
        edit: StyleDefinitionFieldEdit,
    },
    /// Set an absolute presentation origin. `left` is always requested;
    /// `top: None` is a horizontal-only event. A vertical request maps through
    /// the compact height index, refines only its local neighborhood, and then
    /// installs one exact immutable viewport while retaining an anchor to the
    /// newly visible text. Estimated prefixes remain explicitly inexact.
    SetViewportOrigin {
        left: f32,
        top: Option<f32>,
    },
    /// Place or extend the authoritative view cursor from a frontend hit-test.
    /// The offset is bound to the supplied formatted snapshot revision; stale
    /// numeric offsets are rejected rather than reinterpreted.
    PlaceCursor {
        document_revision: Revision,
        text_offset: usize,
        affinity: BoundaryAffinity,
        extend_selection: bool,
    },
    /// Select every logical content item in the exact document snapshot.
    /// Line policy and partial viewport layout do not limit this selection.
    SelectAll {
        document: DocumentId,
        revision: Revision,
    },
    /// Enter Normal mode and reveal the first nonblank grapheme of a logical
    /// hard line in the exact snapshot, without interpreting pending input.
    /// Lines are one-based; zero selects the first and excess selects the last.
    GoToLine {
        document: DocumentId,
        revision: Revision,
        line: u64,
    },
    /// Navigate one retained history edge independently of the current Vim
    /// mode. Native Edit menu actions use this instead of synthesizing `u` or
    /// Ctrl-R key input.
    NavigateHistory(HistoryNavigationRequest),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreOutcome {
    pub command: Option<CommandOutput>,
    pub document_changed: bool,
    /// Exact formatted-text transition for this committed event. Frontends and
    /// caches can advance their own revision-bound anchors without diffing two
    /// flattened snapshots. Non-mutating events carry no map.
    pub position_map: Option<PositionMap>,
    pub layout_changed: bool,
    /// Composition changes may target an inactive view when an edit from
    /// another view invalidates its revision-bound marked-text overlay.
    pub composition_changes: Vec<ViewCompositionChange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionCancelReason {
    Explicit,
    OrdinaryInput,
    Undo,
    Redo,
    ExternalDocumentChange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewCompositionOutcome {
    Began(CompositionOverlay),
    Updated(CompositionOverlay),
    Cancelled {
        reason: CompositionCancelReason,
        restoration: CompositionRestoration,
    },
    Committed(CompositionCommit),
    /// The base revision cannot be restored for display after another view
    /// commits. The frontend discards the overlay and displays the new exact
    /// document/layout revision instead.
    Invalidated {
        reason: CompositionCancelReason,
        base_revision: Revision,
        current_revision: Revision,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewCompositionChange {
    pub view: ViewId,
    pub outcome: ViewCompositionOutcome,
}

/// Cleanup performed while detaching a view. Marked text is a presentation
/// overlay and is discarded rather than committed; a view-owned Insert,
/// Replace, or explicit style edit group is closed so its already committed
/// edits remain one complete undo unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewRemovalOutcome {
    pub edit_group_closed: bool,
    pub composition_discarded: bool,
    pub layout_work_cancelled: bool,
}

/// Query-only presentation state for one view. Optional bounds are absent
/// when current partial or stale layout cannot prove them exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportState {
    document_id: DocumentId,
    document_revision: Revision,
    layout_revision: Option<LayoutRevision>,
    configuration_generation: ViewConfigurationGeneration,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
    left: f32,
    top: f32,
    maximum_left: Option<f32>,
    estimated_maximum_left: f32,
    maximum_top: Option<f32>,
    estimated_maximum_top: f32,
    scale: f32,
    wrap: bool,
    top_is_exact: bool,
}

impl ViewportState {
    pub fn document_id(self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(self) -> Revision {
        self.document_revision
    }

    pub fn layout_revision(self) -> Option<LayoutRevision> {
        self.layout_revision
    }

    pub fn configuration_generation(self) -> ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn measurement_environment_id(self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    pub fn metrics_generation(self) -> MetricsGeneration {
        self.metrics_generation
    }

    pub fn left(self) -> f32 {
        self.left
    }

    pub fn top(self) -> f32 {
        self.top
    }

    pub fn maximum_left(self) -> Option<f32> {
        self.maximum_left
    }

    pub fn estimated_maximum_left(self) -> f32 {
        self.estimated_maximum_left
    }

    /// Exact document-end clamp in the current layout coordinate system.
    /// The prefix may remain estimated, as reported separately by `top_is_exact`.
    pub fn maximum_top(self) -> Option<f32> {
        self.maximum_top
    }

    pub fn estimated_maximum_top(self) -> f32 {
        self.estimated_maximum_top
    }

    pub fn scale(self) -> f32 {
        self.scale
    }

    pub fn wrap(self) -> bool {
        self.wrap
    }

    pub fn top_is_exact(self) -> bool {
        self.top_is_exact
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Completion(crate::command::completion::CompletionError),
    UnknownView(ViewId),
    IdentifierExhausted(CoreIdentifierKind),
    StaleLayoutDemand {
        demand: LayoutRevision,
        current: Option<LayoutRevision>,
    },
    Document(DocumentError),
    History(HistoryError),
    Position(PositionError),
    /// Typed model preparation/commit failures which do not reduce to the
    /// older document, position, or history facade errors (for example style
    /// policy decisions).
    ModelTransaction(ModelTransactionError),
    StaleStyleSheet {
        expected: StyleSheetRevision,
        actual: StyleSheetRevision,
    },
    StyleEditGroup(StyleEditGroupError),
    Layout(LayoutError),
    LayoutMotion(LayoutMotionError),
    LayoutJob(LayoutJobError),
    LayoutInstall(LayoutJobInstallRejection),
    /// Retained for Rust-facing compatibility with the original placeholder
    /// protocol. `SetViewportOrigin` no longer produces this error.
    Composition(CompositionError),
    Persistence(PersistenceError),
    HardLineSourceRange(HardLineSourceRangeError),
    /// A menu action requiring a live Visual selection was dispatched after
    /// that selection disappeared or resolved to no text.
    NoVisualSelection,
    /// A revision-bound native action named a Visual selection which is no
    /// longer the invoking view's exact current logical selection.
    StaleLogicalSelection,
    /// Vim refuses to replace the current artifact with only a line range
    /// unless the command used `:write!` (E140). This policy is distinct from
    /// the storage provider's ordinary existing-destination check.
    PartialCurrentWriteRequiresForce {
        range: crate::command::ex_execute::HardLineRange,
    },
    ExWriteCompletionMismatch {
        expected_document: DocumentId,
        actual_document: DocumentId,
        expected_token: ArtifactWriteToken,
        actual_token: ArtifactWriteToken,
    },
}

impl From<DocumentError> for CoreError {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}

impl From<HistoryError> for CoreError {
    fn from(value: HistoryError) -> Self {
        Self::History(value)
    }
}

impl From<PositionError> for CoreError {
    fn from(value: PositionError) -> Self {
        Self::Position(value)
    }
}

impl From<LayoutError> for CoreError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

impl From<LayoutMotionError> for CoreError {
    fn from(value: LayoutMotionError) -> Self {
        Self::LayoutMotion(value)
    }
}

impl From<LayoutJobError> for CoreError {
    fn from(value: LayoutJobError) -> Self {
        Self::LayoutJob(value)
    }
}

impl From<LayoutJobInstallRejection> for CoreError {
    fn from(value: LayoutJobInstallRejection) -> Self {
        Self::LayoutInstall(value)
    }
}

impl From<CompositionError> for CoreError {
    fn from(value: CompositionError) -> Self {
        Self::Composition(value)
    }
}

impl From<PersistenceError> for CoreError {
    fn from(value: PersistenceError) -> Self {
        Self::Persistence(value)
    }
}

impl From<HardLineSourceRangeError> for CoreError {
    fn from(value: HardLineSourceRangeError) -> Self {
        Self::HardLineSourceRange(value)
    }
}

fn command_model_transaction_error(error: ModelTransactionError) -> CoreError {
    match error {
        ModelTransactionError::Document(error) => CoreError::Document(error),
        ModelTransactionError::Position(error) => CoreError::Position(error),
        ModelTransactionError::History(error) => CoreError::History(error),
        ModelTransactionError::WrongDocument { .. } => {
            CoreError::Document(DocumentError::WrongDocument)
        }
        ModelTransactionError::StaleRevision { expected, actual } => {
            CoreError::Document(DocumentError::WrongSnapshot {
                expected: actual,
                actual: expected,
            })
        }
        ModelTransactionError::StaleDocumentState | ModelTransactionError::RevisionExhausted => {
            CoreError::Document(DocumentError::VerificationFailed)
        }
        error => CoreError::ModelTransaction(error),
    }
}

/// Prepare every fallible model operation before publishing either document
/// or success-dependent controller state. A stale delayed plan is rejected
/// before even its failure-cleanup image can overwrite newer controller state.
fn execute_command_plan(
    document: &mut Document,
    interpreter: &mut CommandInterpreter,
    mut plan: CommandPlan,
) -> Result<(CommandStep, PositionMap, Vec<CommandPresentationRequest>), CoreError> {
    if plan.document() != document.id() {
        return Err(CoreError::Document(DocumentError::WrongDocument));
    }
    if plan.revision() != document.revision() {
        return Err(CoreError::Document(DocumentError::WrongSnapshot {
            expected: document.revision(),
            actual: plan.revision(),
        }));
    }
    let undo_group = plan.undo_group_directive();
    let presentation = plan.presentation_requests().to_vec();

    let prepared = match plan.prepare_model(document) {
        Ok(prepared) => prepared,
        Err(error) => {
            let error = command_model_transaction_error(error);
            plan.publish_failure(interpreter, document.revision());
            return Err(error);
        }
    };
    let (changed, map) = match prepared {
        Some(prepared) => {
            if let Err(error) = plan.map_prepared_cursor(document, &prepared) {
                plan.publish_failure(interpreter, document.revision());
                return Err(CoreError::Document(error));
            }
            let committed = match document.commit_model_transaction(prepared) {
                Ok(committed) => committed,
                Err(error) => {
                    let error = command_model_transaction_error(error);
                    plan.publish_failure(interpreter, document.revision());
                    return Err(error);
                }
            };
            (
                committed.after_revision() != committed.before_revision(),
                committed.text_position_map().clone(),
            )
        }
        None => (
            false,
            PositionMap::identity(
                document.id(),
                PositionDomain::FormattedText,
                document.revision(),
                document.projection().text_tree().byte_len(),
            ),
        ),
    };
    match undo_group {
        crate::command::UndoGroupDirective::Preserve => {}
        crate::command::UndoGroupDirective::End => document.close_edit_group(),
    }
    let step = plan.publish_success(interpreter, document, changed);
    Ok((step, map, presentation))
}

struct View<P: TextMeasurementProvider> {
    commands: CommandInterpreter,
    search: search::SearchViewState,
    layout: ViewLayout,
    engine: LayoutEngine<P>,
    composition: Option<CompositionSession>,
    completion: Option<crate::command::completion::CompletionSession>,
    /// Disposable, source-nonmutating layout of `composition`. The ordinary
    /// view layout remains intact so cancellation is an O(1) restoration.
    composition_layout: Option<ViewLayout>,
    viewport_anchor: Option<ViewportTextAnchor>,
    immediate_layout_context: LayoutExecutionContext,
    observed_metrics_generation: MetricsGeneration,
    active_layout_work: Option<ActiveLayoutWork>,
    /// One cache-only worker, independent of synchronous viewport publication.
    active_prelayout_work: Option<ActiveLayoutWork>,
    /// Exact wrap checkpoints, keyed by their snapshot-local text boundary.
    /// Each value carries every dependency identity; obsolete values are
    /// discarded before lookup and this cache has a fixed entry limit.
    long_line_checkpoints: LongLineCheckpointCache,
}

#[derive(Clone, Debug)]
struct ActiveLayoutWork {
    job_id: LayoutJobId,
    cancellation: LayoutCancellationToken,
    document_revision: Revision,
    configuration_generation: ViewConfigurationGeneration,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
}

impl ActiveLayoutWork {
    fn is_obsolete(
        &self,
        document_revision: Revision,
        configuration_generation: ViewConfigurationGeneration,
        measurement_environment_id: MeasurementEnvironmentId,
        metrics_generation: MetricsGeneration,
    ) -> bool {
        self.document_revision != document_revision
            || self.configuration_generation != configuration_generation
            || self.measurement_environment_id != measurement_environment_id
            || self.metrics_generation != metrics_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewportTextAnchor {
    anchor: TextAnchor,
    offset_from_reference: f32,
    reference: ViewportAnchorReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewportAnchorReference {
    RowTop,
    Baseline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImmediateLayoutIntent {
    PreserveViewport,
    PreserveViewportAndRevealCaret,
    RevealCaret,
}

fn overwrite_for_alternate(force: bool) -> ArtifactOverwrite {
    if force {
        ArtifactOverwrite::ReplaceExisting
    } else {
        ArtifactOverwrite::RefuseExisting
    }
}

fn allocate_style_edit_group_id() -> Option<StyleEditGroupId> {
    NEXT_STYLE_EDIT_GROUP_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .ok()
        .map(StyleEditGroupId)
}

/// Serial composition root for one buffer and its attached views.
mod syntax;
mod whitespace;

pub struct Core<P: TextMeasurementProvider> {
    document: Document,
    syntax: syntax::CoreSyntax,
    buffer_commands: BufferCommandState,
    whitespace_defaults: crate::layout::WhitespacePresentationOptions,
    startup_view_options: Option<(bool, crate::command::VisibleWhitespaceSetting)>,
    views: BTreeMap<ViewId, View<P>>,
    next_view: Option<u64>,
    next_layout_job: Option<u64>,
    edit_group_owner: Option<ViewId>,
    edit_group_restoration: Option<OpenGroupRestoration>,
    /// Explicit frontend-owned style gesture. It owns the document's sole
    /// open history group until ended, its view is removed, or an unrelated
    /// coordinator event consumes it.
    style_edit_group: Option<OpenStyleEditGroup>,
    /// Set only during the synchronous coordinator-owned compound runner.
    /// Nested calls to `handle` publish one ordinary event and return their
    /// continuation to the iterative runner through `queued_replay`.
    replay_undo_floor: Option<usize>,
    queued_replay: Option<ReplayPlan>,
    compound_replay_event_limit: usize,
    #[cfg(test)]
    input_position_map_override: Option<PositionMap>,
}

#[derive(Clone, Debug)]
struct OpenGroupRestoration {
    generation: u64,
    parent: HistoryLocation,
    before: HistoryRestorationSnapshot,
}

/// Only model/controller state participates in input publication. Layout
/// snapshots remain disposable, and no document-sized layout cache is cloned.
struct InputPublicationCheckpoint {
    document: crate::document::DocumentCommandCheckpoint,
    commands: CommandInterpreter,
    views: BTreeMap<ViewId, InputViewCheckpoint>,
    edit_group_owner: Option<ViewId>,
    edit_group_restoration: Option<OpenGroupRestoration>,
    replay_undo_floor: Option<usize>,
}

struct InputViewCheckpoint {
    viewport_anchor: Option<ViewportTextAnchor>,
    viewport_left: f32,
    viewport_top: f32,
    wrap: bool,
}

#[derive(Clone, Debug)]
struct OpenStyleEditGroup {
    identity: StyleEditGroup,
    generation: u64,
    parent: HistoryLocation,
    before: HistoryRestorationSnapshot,
}

#[derive(Debug)]
enum ReplayFrame {
    Mapping { plan: crate::command::mappings::MappingReplayPlan, event: usize },
    Macro {
        plan: MacroReplayPlan,
        iteration: usize,
        event: usize,
    },
    ExNormal {
        plan: ExNormalReplayPlan,
        target: usize,
        event: usize,
        line_started: bool,
    },
}

#[derive(Debug)]
enum ReplayAction {
    Input { event: InputEvent, ex_normal: bool, remap: bool },
    BeginExNormalLine(ExNormalTarget),
    FinishExNormalLine,
    PopFrame,
}

struct CoreOutcomeAccumulator {
    command: Option<CommandOutput>,
    document_changed: bool,
    position_map: Option<PositionMap>,
    layout_changed: bool,
    composition_changes: Vec<ViewCompositionChange>,
}

impl CoreOutcomeAccumulator {
    fn new(mut outcome: CoreOutcome) -> Self {
        Self {
            command: outcome.command.take(),
            document_changed: outcome.document_changed,
            position_map: outcome.position_map.take(),
            layout_changed: outcome.layout_changed,
            composition_changes: outcome.composition_changes,
        }
    }

    fn merge(&mut self, mut outcome: CoreOutcome) -> Result<(), CoreError> {
        if let Some(next) = outcome.command.take() {
            match self.command.as_mut() {
                Some(command) => command.merge(next),
                None => self.command = Some(next),
            }
        }
        self.document_changed |= outcome.document_changed;
        self.layout_changed |= outcome.layout_changed;
        self.composition_changes
            .append(&mut outcome.composition_changes);
        if let Some(next) = outcome.position_map.take() {
            self.position_map = Some(match self.position_map.take() {
                Some(previous) => previous.then(&next)?,
                None => next,
            });
        }
        Ok(())
    }

    fn command_mut(&mut self) -> &mut CommandOutput {
        self.command
            .as_mut()
            .expect("an input replay starts with one command output")
    }

    fn finish(self) -> CoreOutcome {
        CoreOutcome {
            command: self.command,
            document_changed: self.document_changed,
            position_map: self.position_map,
            layout_changed: self.layout_changed,
            composition_changes: self.composition_changes,
        }
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn new(document: Document) -> Self {
        let buffer_commands = CommandInterpreter::new().export_buffer_state();
        Self {
            document,
            syntax: syntax::CoreSyntax::default(),
            buffer_commands,
            whitespace_defaults: Default::default(),
            startup_view_options: None,
            views: BTreeMap::new(),
            next_view: Some(1),
            next_layout_job: Some(1),
            edit_group_owner: None,
            edit_group_restoration: None,
            style_edit_group: None,
            replay_undo_floor: None,
            queued_replay: None,
            compound_replay_event_limit: MACRO_REPLAY_EVENT_LIMIT,
            #[cfg(test)]
            input_position_map_override: None,
        }
    }

    pub fn initialize_style_defaults(
        &mut self,
        json: &[u8],
    ) -> Result<(), crate::document::StyleDefaultsError> {
        if !self.views.is_empty() {
            return Err(crate::document::StyleDefaultsError::NotPristine);
        }
        self.document.initialize_style_defaults(json)
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn selected_typography(
        &self,
        view_id: ViewId,
    ) -> Result<(crate::document::ResolvedCharacterStyle, bool), CoreError> {
        self.selected_typography_details(view_id).map(|(style, mixed, _)| (style, mixed))
    }

    /// The script-specific flag keeps unrelated font/color differences from
    /// changing Superscript and Subscript menu toggle behavior.
    pub fn selected_typography_details(
        &self,
        view_id: ViewId,
    ) -> Result<(crate::document::ResolvedCharacterStyle, bool, bool), CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let selection = self.active_linear_selection_identity(view_id)?;
        let range = selection.as_ref().map(|value| value.range());
        let at = range
            .as_ref()
            .map_or(view.commands.cursor(), |range| range.start);
        let upstream = range.is_none()
            && if matches!(view.commands.mode(), Mode::Insert | Mode::Replace) {
                view.commands.insertion_boundary_affinity()
            } else {
                view.commands.boundary_affinity()
            } == BoundaryAffinity::Upstream;
        let mut first =
            DocumentLayoutStyles::semantic_character_at(self.document.projection(), at, upstream)
                .map_err(LayoutError::from)?;
        if range.is_none() {
            view.commands
                .apply_typing_presentation(&self.document, &mut first)?;
        }
        let (mixed, script_mixed) = if let Some(range) = range.filter(|range| !range.is_empty()) {
            let styles =
                DocumentLayoutStyles::resolve_region(self.document.projection(), range.clone())
                    .map_err(LayoutError::from)?;
            let mut boundaries = std::collections::BTreeSet::from([range.start, range.end]);
            for run in &styles.shaping_runs {
                boundaries.insert(run.text_range.start.max(range.start));
                boundaries.insert(run.text_range.end.min(range.end));
            }
            for run in &styles.paint_runs {
                boundaries.insert(run.text_range.start.max(range.start));
                boundaries.insert(run.text_range.end.min(range.end));
            }
            for paragraph in &styles.paragraphs {
                boundaries.insert(paragraph.text_range.start.max(range.start));
            }
            let mut mixed = false;
            let mut script_mixed = false;
            for at in boundaries.into_iter().filter(|at| *at < range.end) {
                let value = DocumentLayoutStyles::semantic_character_at(
                    self.document.projection(), at, false,
                ).map_err(LayoutError::from)?;
                mixed |= value != first;
                script_mixed |= value.script_position != first.script_position;
                if mixed && script_mixed { break; }
            }
            (mixed, script_mixed)
        } else {
            (false, false)
        };
        Ok((first, mixed, script_mixed))
    }

    /// Return native Bold/Italic presentation from the authoritative logical
    /// selection and transformation capability. This query performs no layout
    /// work and therefore remains valid for an offscreen linear selection.
    pub fn selection_semantic_style_presentation(
        &self,
        view_id: ViewId,
        style: SemanticInlineStyle,
    ) -> Result<SemanticStylePresentation, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let selection_kind = match view.commands.mode() {
            Mode::VisualCharacter => LogicalSelectionKind::Character,
            Mode::VisualLine => LogicalSelectionKind::Line,
            Mode::VisualBlock => LogicalSelectionKind::Block,
            Mode::Normal | Mode::Insert | Mode::Replace | Mode::CommandLine => {
                LogicalSelectionKind::None
            }
        };
        if selection_kind == LogicalSelectionKind::None
            && matches!(view.commands.mode(), Mode::Insert | Mode::Replace)
        {
            let values = match style {
                SemanticInlineStyle::Strong => vec![(
                    StyleProperty::CharacterBold,
                    crate::document::StylePropertyValue::Boolean(true),
                )],
                SemanticInlineStyle::Emphasis => vec![(
                    StyleProperty::CharacterSlant,
                    crate::document::StylePropertyValue::FontSlant(
                        crate::document::FontSlant::Italic,
                    ),
                )],
                _ => Vec::new(),
            };
            let supported = !values.is_empty()
                && self.document.validate_typing_properties(&values).is_ok()
                && (self.document.format() != Format::HtmlSource
                    || self
                        .document
                        .html_source_prose_at(
                            view.commands.cursor(),
                            view.commands.insertion_boundary_affinity(),
                        )
                        .unwrap_or(false));
            let (current, _) = self.selected_typography(view_id)?;
            let on = match style {
                SemanticInlineStyle::Strong => current.bold,
                SemanticInlineStyle::Emphasis => {
                    current.slant != crate::document::FontSlant::Upright
                }
                _ => false,
            };
            return Ok(SemanticStylePresentation {
                selection_kind,
                selection: Some(self.list_selection_identity(view_id)?),
                state: if on {
                    SemanticStyleState::On
                } else {
                    SemanticStyleState::Off
                },
                can_set: supported,
                can_clear: supported,
            });
        }
        if !matches!(
            selection_kind,
            LogicalSelectionKind::Character | LogicalSelectionKind::Line
        ) {
            return Ok(SemanticStylePresentation {
                selection_kind,
                selection: None,
                state: SemanticStyleState::Off,
                can_set: false,
                can_clear: false,
            });
        }

        let selection = self
            .active_linear_selection_identity(view_id)?
            .ok_or(CoreError::NoVisualSelection)?;
        let range = selection.range();
        if range.is_empty() {
            return Ok(SemanticStylePresentation {
                selection_kind,
                selection: Some(selection),
                state: SemanticStyleState::Off,
                can_set: false,
                can_clear: false,
            });
        }

        let mut covered = self
            .document
            .projection()
            .style_spans()
            .iter()
            .filter(|span| span.application == StyleApplication::Semantic(style))
            .filter_map(|span| {
                let start = span.range.start.max(range.start);
                let end = span.range.end.min(range.end);
                (start < end).then_some(start..end)
            })
            .collect::<Vec<_>>();
        covered.sort_by_key(|segment| (segment.start, segment.end));
        let state = if self.document.format().has_rich_source() {
            let resolved =
                DocumentLayoutStyles::resolve_region(self.document.projection(), range.clone())
                    .map_err(LayoutError::from)?;
            let enabled = |value: &crate::layout::ResolvedTextStyle| match style {
                SemanticInlineStyle::Strong => value.relative_bold,
                SemanticInlineStyle::Emphasis => value.slant != crate::document::FontSlant::Upright,
                SemanticInlineStyle::Code => false,
            };
            ranged_boolean_style_state(
                self.document.projection().text_tree(),
                range.clone(),
                enabled(&resolved.default_shaping_style),
                resolved
                    .shaping_runs
                    .iter()
                    .map(|run| (run.text_range.clone(), enabled(&run.style))),
            )
        } else if covered.is_empty() {
            SemanticStyleState::Off
        } else {
            let mut cursor = range.start;
            let mut has_gap = false;
            for segment in covered {
                if segment.start > cursor {
                    has_gap = true;
                    break;
                }
                cursor = cursor.max(segment.end);
                if cursor >= range.end {
                    break;
                }
            }
            if !has_gap && cursor >= range.end {
                SemanticStyleState::On
            } else {
                SemanticStyleState::Mixed
            }
        };
        let can_set = self
            .document
            .semantic_style_edit_capability(range.clone(), style, true)
            .is_ok();
        let can_clear = self
            .document
            .semantic_style_edit_capability(range, style, false)
            .is_ok();
        Ok(SemanticStylePresentation {
            selection_kind,
            selection: Some(selection),
            state,
            can_set,
            can_clear,
        })
    }

    /// Exact paragraph target for native list actions. An insertion/Normal
    /// caret names its current paragraph through a checked empty range.
    pub fn selection_decoration_state(
        &self,
        view_id: ViewId,
        strike: bool,
    ) -> Result<SemanticStyleState, CoreError> {
        if self.active_linear_selection_identity(view_id)?.is_none() {
            let view = self
                .views
                .get(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            if !matches!(view.commands.mode(), Mode::Insert | Mode::Replace) {
                return Err(CoreError::StaleLogicalSelection);
            }
            self.document.validate_typing_properties(&[(
                if strike {
                    StyleProperty::CharacterStrikethrough
                } else {
                    StyleProperty::CharacterUnderline
                },
                crate::document::StylePropertyValue::Boolean(true),
            )])?;
            let (style, _) = self.selected_typography(view_id)?;
            return Ok(
                if if strike {
                    style.strikethrough
                } else {
                    style.underline
                } {
                    SemanticStyleState::On
                } else {
                    SemanticStyleState::Off
                },
            );
        }
        let selection = self
            .active_linear_selection_identity(view_id)?
            .ok_or(CoreError::StaleLogicalSelection)?;
        let range = selection.range();
        let resolved =
            DocumentLayoutStyles::resolve_region(self.document.projection(), range.clone())
                .map_err(LayoutError::from)?;
        let enabled = |value: &crate::layout::ResolvedTextPaint| {
            if strike {
                value.strikethrough
            } else {
                value.underline
            }
        };
        Ok(ranged_boolean_style_state(
            self.document.projection().text_tree(),
            range,
            enabled(&resolved.default_paint),
            resolved
                .paint_runs
                .iter()
                .map(|run| (run.text_range.clone(), enabled(&run.paint))),
        ))
    }

    pub fn list_selection_identity(
        &self,
        view_id: ViewId,
    ) -> Result<LogicalSelectionIdentity, CoreError> {
        if let Some(selection) = self.active_linear_selection_identity(view_id)? {
            return Ok(selection);
        }
        let commands = &self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?
            .commands;
        if !matches!(commands.mode(), Mode::Normal | Mode::Insert | Mode::Replace) {
            return Err(CoreError::NoVisualSelection);
        }
        let cursor = commands.cursor();
        Ok(LogicalSelectionIdentity {
            view: view_id,
            document: self.document.id(),
            revision: self.document.revision(),
            kind: LogicalSelectionKind::None,
            anchor: cursor,
            active: cursor,
            active_affinity: commands.boundary_affinity(),
            range: cursor..cursor,
        })
    }

    pub fn selected_named_styles(
        &self,
        view_id: ViewId,
    ) -> Result<crate::document::SelectedNamedStyles, CoreError> {
        let selection = self.list_selection_identity(view_id)?;
        let mut selected = if self.document.format().is_code() {
            self.document.projection().selected_code_named_styles(
                selection.range(),
                selection.active_affinity(),
            )
        } else {
            self.document.projection().selected_named_styles(
                selection.range(),
                selection.active_affinity(),
            )
        };
        if !self.document.format().is_code() && selection.kind() == LogicalSelectionKind::None {
            if let Some(named) = self.views[&view_id].commands.typing_named_style() {
                selected.character = (!named.0.is_empty()).then(|| named.clone());
                selected.character_mixed = false;
            }
        }
        Ok(selected)
    }

    fn active_linear_selection_identity(
        &self,
        view_id: ViewId,
    ) -> Result<Option<LogicalSelectionIdentity>, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let kind = match view.commands.mode() {
            Mode::VisualCharacter => LogicalSelectionKind::Character,
            Mode::VisualLine => LogicalSelectionKind::Line,
            Mode::Normal | Mode::Insert | Mode::Replace | Mode::VisualBlock | Mode::CommandLine => {
                return Ok(None)
            }
        };
        let range = view
            .commands
            .line_selection_range(&self.document, view.layout.snapshot())
            .ok_or(CoreError::NoVisualSelection)?;
        let active = view.commands.cursor();
        let anchor = view.commands.visual_anchor().unwrap_or(active);
        Ok(Some(LogicalSelectionIdentity {
            view: view_id,
            document: self.document.id(),
            revision: self.document.revision(),
            kind,
            anchor,
            active,
            active_affinity: view.commands.boundary_affinity(),
            range,
        }))
    }

    /// Begin one explicit live style-edit gesture at an exact document and
    /// style-sheet identity. Only one explicit style group may be active in a
    /// core. A group with no successful edits creates no history entry.
    pub fn begin_style_edit_group(
        &mut self,
        view_id: ViewId,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
    ) -> Result<StyleEditGroup, CoreError> {
        if !self.views.contains_key(&view_id) {
            return Err(CoreError::UnknownView(view_id));
        }
        if let Some(open) = &self.style_edit_group {
            return Err(CoreError::StyleEditGroup(
                StyleEditGroupError::AlreadyActive(open.identity.id),
            ));
        }
        self.validate_style_sheet_identity(document, revision, style_sheet_revision)?;
        let id = allocate_style_edit_group_id().ok_or(CoreError::IdentifierExhausted(
            CoreIdentifierKind::StyleEditGroup,
        ))?;

        // A style gesture is independent of any command-layer Insert/Replace
        // unit. Preserve that unit's true owner endpoint before opening the
        // model history group used by this gesture.
        self.finalize_open_edit_group(view_id)?;
        let identity = StyleEditGroup {
            id,
            view: view_id,
            document,
            begin_document_revision: revision,
            begin_style_sheet_revision: style_sheet_revision,
        };
        let parent = self.document.history_status().current;
        let before = self
            .views
            .get(&view_id)
            .expect("view existence checked before beginning style edit group")
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.begin_edit_group();
        self.style_edit_group = Some(OpenStyleEditGroup {
            identity,
            generation: self.document.edit_group_generation(),
            parent,
            before,
        });
        Ok(identity)
    }

    /// End and consume an explicit style-edit capability. Successfully
    /// committed edits remain published and become one history unit; ending
    /// never rolls them back. Reusing the capability after this call fails.
    pub fn end_style_edit_group(
        &mut self,
        view_id: ViewId,
        group: StyleEditGroup,
    ) -> Result<(), CoreError> {
        self.validate_style_edit_group(view_id, group)?;
        self.finalize_style_edit_group()?;
        Ok(())
    }

    /// Change buffer write policy against an exact current source snapshot.
    pub fn set_read_only(
        &mut self,
        document: DocumentId,
        revision: Revision,
        value: bool,
    ) -> Result<(), CoreError> {
        self.validate_buffer_policy_snapshot(document, revision)?;
        self.document.set_read_only(value);
        Ok(())
    }
    pub fn mark_recovered(
        &mut self,
        document: DocumentId,
        revision: Revision,
    ) -> Result<(), CoreError> {
        self.validate_buffer_policy_snapshot(document, revision)?;
        self.document.mark_recovered();
        Ok(())
    }
    fn validate_buffer_policy_snapshot(
        &self,
        document: DocumentId,
        revision: Revision,
    ) -> Result<(), CoreError> {
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
        Ok(())
    }

    /// Acknowledge that a native frontend successfully persisted the exact
    /// current source snapshot. Stale acknowledgements are rejected before an
    /// open edit group or save-point state is changed.
    pub fn mark_saved(
        &mut self,
        document: DocumentId,
        revision: Revision,
    ) -> Result<(), CoreError> {
        if document != self.document.id() {
            return Err(CoreError::Document(DocumentError::WrongDocument));
        }
        if revision != self.document.revision() {
            return Err(CoreError::Document(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }));
        }
        self.finalize_style_edit_group()?;
        if let Some(owner) = self.edit_group_owner {
            self.finalize_open_edit_group(owner)?;
        } else {
            self.edit_group_restoration = None;
        }
        self.document.mark_saved();
        Ok(())
    }

    /// Finalize the active command-layer edit unit and capture immutable bytes
    /// for external storage. The returned value can be sent to a provider
    /// without retaining a mutable borrow of `Core`.
    pub fn prepare_artifact_write(
        &mut self,
        intent: ArtifactWriteIntent,
    ) -> Result<PreparedArtifactWrite, CoreError> {
        self.prepare_artifact_write_with_force(intent, false)
    }
    pub fn prepare_artifact_write_with_force(
        &mut self,
        intent: ArtifactWriteIntent,
        force: bool,
    ) -> Result<PreparedArtifactWrite, CoreError> {
        self.document.validate_write_policy(force)?;
        if let Some(view) = self.edit_group_owner {
            self.accept_completion(view)?;
        }
        self.finalize_style_edit_group()?;
        self.edit_group_owner = None;
        self.edit_group_restoration = None;
        Ok(self
            .document
            .prepare_artifact_write_with_force(intent, force)?)
    }

    /// Publish a provider result after the external operation has finished.
    /// Persistence metadata changes do not invalidate formatted or layout
    /// snapshots.
    pub fn complete_artifact_write(
        &mut self,
        completion: ArtifactWriteCompletion,
    ) -> Result<ArtifactWriteCompletionStatus, CoreError> {
        Ok(self.document.complete_artifact_write(completion)?)
    }

    /// Adapt one typed Ex file request into either immutable source bytes for
    /// external storage or a revision-tagged host/session request.
    ///
    /// The method performs no I/O. Full writes to the current binding are
    /// saves and establish a save point only after successful completion.
    /// Explicit alternate paths and every partial write remain alternate
    /// writes. `:saveas` adopts its receipt only after success. An unchanged
    /// pathless `:xit` skips storage and immediately yields its tagged quit
    /// request.
    pub fn prepare_ex_file_request(
        &mut self,
        request: &ExFileRequest,
    ) -> Result<PreparedExFileRequest, CoreError> {
        if let ExFileRequest::WriteAll { force } = request {
            self.document.validate_write_policy(*force)?;
        }
        let tag = ExRequestTag::new(self.document.id(), self.document.revision());
        let tagged = |request| TaggedExFileRequest::new(tag, request);

        if let ExFileRequest::NavigateArgument { target, force, write_first: true, path, line } = request {
            self.document.validate_write_policy(*force)?;
            let intent = self.ex_artifact_write_intent(path.as_deref(), *force, None)?;
            let write = self.prepare_artifact_write_with_force(intent, *force)?;
            let after_success = tagged(ExFileRequest::NavigateArgument {
                target: *target, force: *force, write_first: false, path: None, line: *line,
            });
            return Ok(PreparedExFileRequest::ArtifactWrite(
                PreparedExArtifactWrite::new(tag, write, Some(after_success)),
            ));
        }

        let (path, force, range, quit_after_success) = match request {
            ExFileRequest::Write { path, force, range } => (path.as_deref(), *force, *range, false),
            ExFileRequest::WriteQuit { path, force, range } => {
                (path.as_deref(), *force, *range, true)
            }
            ExFileRequest::Xit { path: None, force } if !self.document.is_dirty() => {
                return Ok(PreparedExFileRequest::Host(tagged(ExFileRequest::Quit {
                    force: *force,
                })));
            }
            ExFileRequest::Xit { path, force } => (path.as_deref(), *force, None, true),
            ExFileRequest::SaveAs { path, force } => {
                let write = self.prepare_artifact_write_with_force(
                    ArtifactWriteIntent::SaveAs {
                        destination: ArtifactPath::from(path.clone()),
                        overwrite: overwrite_for_alternate(*force),
                    },
                    *force,
                )?;
                return Ok(PreparedExFileRequest::ArtifactWrite(
                    PreparedExArtifactWrite::new(tag, write, None),
                ));
            }
            request => return Ok(PreparedExFileRequest::Host(tagged(request.clone()))),
        };

        self.document.validate_write_policy(force)?;
        let intent = self.ex_artifact_write_intent(path, force, range)?;
        let write = self.prepare_artifact_write_with_force(intent, force)?;
        let after_success = quit_after_success.then(|| tagged(ExFileRequest::Quit { force }));
        Ok(PreparedExFileRequest::ArtifactWrite(
            PreparedExArtifactWrite::new(tag, write, after_success),
        ))
    }

    /// Publish an Ex write result and release its attached host action only
    /// after success. If the document moved to another history state while I/O
    /// was in flight, a `:wq`/`:xit` close is explicitly suppressed so newer
    /// edits cannot be discarded.
    pub fn complete_ex_artifact_write(
        &mut self,
        prepared: &PreparedExArtifactWrite,
        completion: ArtifactWriteCompletion,
    ) -> Result<CompletedExArtifactWrite, CoreError> {
        let expected_document = prepared.write().document();
        let expected_token = prepared.write().token();
        if expected_document != self.document.id()
            || completion.document() != expected_document
            || completion.token() != expected_token
        {
            return Err(CoreError::ExWriteCompletionMismatch {
                expected_document,
                actual_document: completion.document(),
                expected_token,
                actual_token: completion.token(),
            });
        }

        let status = self.complete_artifact_write(completion)?;
        let post_write = match prepared.after_success().cloned() {
            None => ExPostWriteDisposition::NotRequested,
            Some(request) if matches!(status, ArtifactWriteCompletionStatus::Failed { .. }) => {
                ExPostWriteDisposition::WriteFailed(request)
            }
            Some(request) => {
                let captured_history = prepared.write().history_location();
                let current_history = self.document.history_status().current;
                if captured_history == current_history {
                    ExPostWriteDisposition::Ready(request)
                } else {
                    ExPostWriteDisposition::DocumentChanged {
                        request,
                        captured_history,
                        current_history,
                        current_revision: self.document.revision(),
                    }
                }
            }
        };
        Ok(CompletedExArtifactWrite::new(status, post_write))
    }

    fn ex_artifact_write_intent(
        &self,
        path: Option<&str>,
        force: bool,
        range: Option<crate::command::ex_execute::HardLineRange>,
    ) -> Result<ArtifactWriteIntent, CoreError> {
        let line_count = self.document.line_count();
        // Validate and resolve an explicit line range before applying write
        // policy.  In particular, a malformed range aimed at the current
        // artifact is a range error, not Vim's E140 partial-write refusal.
        let ranged_scope = match range {
            Some(range) => {
                let end =
                    range
                        .end
                        .checked_add(1)
                        .ok_or(HardLineSourceRangeError::InvalidRange {
                            start: range.start,
                            end: usize::MAX,
                            line_count,
                        })?;
                Some(ArtifactWriteScope::PrimarySourceBytes(
                    self.document
                        .source_byte_range_for_hard_lines(range.start..end)?,
                ))
            }
            None => None,
        };
        let range_is_whole_document = range
            .is_some_and(|range| range.start == 0 && range.end.checked_add(1) == Some(line_count));
        let requested_destination = path.map(ArtifactPath::from);
        let targets_current_binding = match (
            requested_destination.as_ref(),
            self.document.artifact_binding(),
        ) {
            (None, Some(_)) => true,
            (Some(destination), Some(binding)) => destination == binding.path(),
            _ => false,
        };

        if targets_current_binding
            && range.is_some_and(|range| {
                range.start != 0 || range.end.checked_add(1) != Some(line_count)
            })
            && !force
        {
            return Err(CoreError::PartialCurrentWriteRequiresForce {
                range: range.expect("partial current write has an explicit range"),
            });
        }

        if targets_current_binding && (range.is_none() || range_is_whole_document) {
            return Ok(ArtifactWriteIntent::Save {
                // An existing current binding is the normal save target and
                // never needs `!` merely because it already exists.
                overwrite: ArtifactOverwrite::ReplaceExisting,
            });
        }

        let destination = match requested_destination {
            Some(destination) => destination,
            None => self
                .document
                .artifact_binding()
                .map(|binding| binding.path().clone())
                .ok_or(PersistenceError::NoCurrentArtifact)?,
        };
        let scope = ranged_scope.unwrap_or(ArtifactWriteScope::WholeArtifact);
        Ok(ArtifactWriteIntent::WriteAlternate {
            destination,
            scope,
            overwrite: if targets_current_binding {
                ArtifactOverwrite::ReplaceExisting
            } else {
                overwrite_for_alternate(force)
            },
        })
    }

    pub fn add_view(&mut self, provider: P, width: f32, height: f32) -> ViewId {
        self.try_add_view(provider, width, height)
            .expect("the compatibility add_view API exhausted a non-reusing u64 identifier space")
    }

    /// Fallible host-facing attachment API. View identifiers are never reused,
    /// including after a view is removed.
    pub fn try_add_view(
        &mut self,
        provider: P,
        width: f32,
        height: f32,
    ) -> Result<ViewId, CoreError> {
        self.try_add_view_with_layout_execution_context(
            provider,
            width,
            height,
            LayoutExecutionContext::WorkerPool,
        )
    }

    /// Attach a view and declare the executor on which immediate synchronous
    /// shaping callbacks will actually run. The declaration is validated
    /// against the provider; it never causes Core to masquerade as another
    /// executor or thread.
    pub fn add_view_with_layout_execution_context(
        &mut self,
        provider: P,
        width: f32,
        height: f32,
        immediate_layout_context: LayoutExecutionContext,
    ) -> ViewId {
        self.try_add_view_with_layout_execution_context(
            provider,
            width,
            height,
            immediate_layout_context,
        )
        .expect("the compatibility add_view API exhausted a non-reusing u64 identifier space")
    }

    pub fn try_add_view_with_layout_execution_context(
        &mut self,
        provider: P,
        width: f32,
        height: f32,
        immediate_layout_context: LayoutExecutionContext,
    ) -> Result<ViewId, CoreError> {
        self.try_add_view_with_initial_layout(provider, ViewLayout::new(width, height), immediate_layout_context)
    }

    pub(crate) fn try_add_view_with_initial_layout(
        &mut self,
        provider: P,
        mut layout: ViewLayout,
        immediate_layout_context: LayoutExecutionContext,
    ) -> Result<ViewId, CoreError> {
        let id = self.allocate_view_id()?;
        let mut commands = CommandInterpreter::new();
        commands.install_buffer_state(&self.buffer_commands);
        commands.set_reflow_language(self.reflow_language());
        commands.note_document_revision(self.document.revision());
        commands.set_layout_options(layout.wrap());
        if let Some(options) = &self.startup_view_options {
            commands.install_startup_view_options(options);
            layout.set_wrap(options.0);
        }
        let engine = LayoutEngine::new(provider);
        let observed_metrics_generation = inspect_layout_provider(&engine).metrics_generation;
        self.views.insert(
            id,
            View {
                commands,
                layout,
                engine,
                composition: None,
                completion: None,
                composition_layout: None,
                search: Default::default(),
                viewport_anchor: None,
                immediate_layout_context,
                observed_metrics_generation,
                active_layout_work: None,
                active_prelayout_work: None,
                long_line_checkpoints: LongLineCheckpointCache::default(),
            },
        );
        // Measurement failures leave the attached view without a current
        // snapshot and are retained as presentation diagnostics. Identifier
        // exhaustion is structural, so the fallible API removes the partial
        // attachment and reports it; the compatibility wrapper panics only in
        // that unreachable-at-human-scale case.
        if let Err(error) =
            self.materialize_immediate_viewport(id, ImmediateLayoutIntent::RevealCaret)
        {
            if matches!(error, CoreError::IdentifierExhausted(_)) {
                self.views.remove(&id);
                return Err(error);
            }
            self.record_presentation_error(id, error);
        }
        Ok(id)
    }

    /// Detach one view without committing any of its transient marked text.
    /// Existing Insert/Replace or live style edits remain committed and their
    /// open undo group is closed. Any worker request owned by this view is
    /// cooperatively cancelled before the view state is dropped.
    pub fn remove_view(&mut self, view_id: ViewId) -> Result<ViewRemovalOutcome, CoreError> {
        if !self.views.contains_key(&view_id) {
            return Err(CoreError::UnknownView(view_id));
        }
        let style_group_closed = self
            .style_edit_group
            .as_ref()
            .is_some_and(|open| open.identity.view == view_id);
        if style_group_closed {
            // Restoration must be captured while the owning view still
            // exists. The capability is consumed even if final attachment
            // unexpectedly fails; already committed edits are never rolled
            // back by view teardown.
            self.finalize_style_edit_group()?;
        }
        let mut view = self
            .views
            .remove(&view_id)
            .expect("view existence checked before style-group finalization");
        let layout_work_cancelled = cancel_active_layout_work(&mut view);
        let composition_discarded = view.composition.take().is_some();
        let command_group_closed = self.edit_group_owner == Some(view_id);
        if command_group_closed {
            self.document.close_edit_group();
            self.edit_group_owner = None;
            self.edit_group_restoration = None;
        }
        Ok(ViewRemovalOutcome {
            edit_group_closed: style_group_closed || command_group_closed,
            composition_discarded,
            layout_work_cancelled,
        })
    }

    fn install_buffer_commands(&mut self, view_id: ViewId) {
        self.views
            .get_mut(&view_id)
            .expect("view existence checked by serial coordinator")
            .commands
            .install_buffer_state(&self.buffer_commands);
    }

    fn publish_buffer_commands(&mut self, source: ViewId) {
        let next = self
            .views
            .get(&source)
            .expect("source view remains attached during serial dispatch")
            .commands
            .export_buffer_state();
        self.buffer_commands = next;
        for view in self.views.values_mut() {
            view.commands.install_buffer_state(&self.buffer_commands);
        }
    }

    pub fn command_state(&self, view: ViewId) -> Option<&CommandInterpreter> {
        self.views.get(&view).map(|view| &view.commands)
    }

    /// The buffer's `textwidth` state shared by every view.
    pub fn text_width(&self) -> crate::document::TextWidthSetting {
        self.buffer_commands.text_width
    }

    /// Propagate the application default. Every view of this buffer sees the
    /// new value unless the buffer holds an explicit `:set textwidth` override.
    pub fn set_text_width_default(&mut self, width: u32) {
        self.buffer_commands.text_width.set_default(width);
        for view in self.views.values_mut() {
            view.commands.set_text_width_default(width);
        }
    }

    /// Install the canonical detected language into every view so reflow can
    /// select its comment profile without consulting syntax coverage.
    fn publish_reflow_language(&mut self) {
        let language = self.reflow_language();
        for view in self.views.values_mut() {
            if view.commands.reflow_language() != language.as_deref() {
                view.commands.set_reflow_language(language.clone());
            }
        }
    }

    pub fn line_location(&self, view: ViewId) -> Result<crate::command::LineLocation, CoreError> {
        let state = self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        state
            .commands
            .line_location(&self.document, state.layout.snapshot())
    }

    pub fn layout(&self, view: ViewId) -> Option<&ViewLayout> {
        self.views.get(&view).map(|view| &view.layout)
    }

    /// Return the layout a frontend should paint. Native marked text owns a
    /// disposable composed layout while the command model keeps using the
    /// source-backed layout for document-coordinate motions.
    /// Application-owned padding belongs to the document canvas coordinates,
    /// so scrolling moves it out of view. It never changes persisted styles.
    pub fn set_view_insets(
        &mut self,
        view_id: ViewId,
        insets: crate::layout::EdgeInsets,
    ) -> Result<(), CoreError> {
        if [insets.top, insets.left, insets.bottom, insets.right]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(LayoutError::InvalidGeometry.into());
        }
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let pinned_to_top = view.layout.viewport_top() == 0.0;
        let before = view.layout.configuration_generation();
        view.layout.set_insets(insets);
        if before != view.layout.configuration_generation() {
            cancel_active_layout_work(view);
        } else if current_snapshot_for_layout(&self.document, &view.layout, inspect_layout_provider(&view.engine)).is_some() {
            return Ok(());
        }
        self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::PreserveViewport)?;
        if pinned_to_top {
            let view = self
                .views
                .get_mut(&view_id)
                .expect("inset view remains attached");
            view.layout.set_viewport_top(0.0)?;
            update_viewport_anchor(&self.document, view);
        }
        self.rematerialize_active_composition(view_id, true)?;
        Ok(())
    }

    pub fn presentation_layout(&self, view: ViewId) -> Option<&ViewLayout> {
        self.views
            .get(&view)
            .map(|view| view.composition_layout.as_ref().unwrap_or(&view.layout))
    }

    /// Return the current absolute presentation origin and only those bounds
    /// proven exact for the current document/configuration/provider identity.
    pub fn viewport_state(&self, view_id: ViewId) -> Result<ViewportState, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let requirements = inspect_layout_provider(&view.engine);
        let presentation_layout = view.composition_layout.as_ref().unwrap_or(&view.layout);
        let current_snapshot =
            current_snapshot_for_layout(&self.document, presentation_layout, requirements);
        let snapshot_is_current = current_snapshot.is_some();
        Ok(ViewportState {
            document_id: self.document.id(),
            document_revision: self.document.revision(),
            layout_revision: current_snapshot.map(|snapshot| snapshot.revision),
            configuration_generation: presentation_layout.configuration_generation(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
            left: presentation_layout.viewport_left(),
            top: presentation_layout.viewport_top(),
            maximum_left: if snapshot_is_current {
                presentation_layout.maximum_viewport_left()
            } else {
                None
            },
            estimated_maximum_left: if snapshot_is_current {
                presentation_layout.estimated_maximum_viewport_left()
            } else {
                presentation_layout.viewport_left()
            },
            maximum_top: if snapshot_is_current {
                presentation_layout.maximum_viewport_top()
            } else {
                None
            },
            estimated_maximum_top: if snapshot_is_current {
                presentation_layout.estimated_maximum_viewport_top()
            } else {
                presentation_layout.viewport_top()
            },
            scale: presentation_layout.scale(),
            wrap: presentation_layout.wrap(),
            top_is_exact: layout_origin_has_exact_geometry(
                current_snapshot,
                presentation_layout.height(),
                presentation_layout.viewport_top(),
            ),
        })
    }

    /// Scheduling requirements for a view's provider. A frontend uses this
    /// before selecting the compatible executor for `compute_layout_job`.
    pub fn layout_provider_requirements(
        &self,
        view: ViewId,
    ) -> Result<LayoutProviderRequirements, CoreError> {
        let view = self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        Ok(inspect_layout_provider(&view.engine))
    }

    /// Capture an owned, revision-tagged layout request during one serial
    /// coordinator turn. The returned request contains only the requested
    /// regional text and can be computed without access to this `Core`.
    pub fn paragraph_flow(&self, view_id: ViewId) -> Result<bool, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        Ok(match self.document.format() {
            Format::Markdown | Format::Html => true,
            Format::MarkdownSource | Format::HtmlSource => view.layout.paragraph_flow(),
            _ => false,
        })
    }

    fn presentation_flow(&self, view_id: ViewId) -> bool {
        self.views
            .get(&view_id)
            .is_some_and(|view| view.layout.paragraph_flow())
    }

    pub fn prepare_view_layout_job(
        &mut self,
        view_id: ViewId,
        priority: LayoutJobPriority,
        mut region: LayoutJobRegion,
        cancellation: LayoutCancellationToken,
    ) -> Result<crate::layout::LayoutJobRequest, CoreError> {
        self.poll_syntax();
        if !self.views.contains_key(&view_id) {
            return Err(CoreError::UnknownView(view_id));
        }
        if let LayoutJobRegion::Viewport(viewport) = &region {
            if !viewport.has_horizontal_focus() {
                let view = &self.views[&view_id];
                let position = view.commands.visual_position();
                let offset = position.map_or(view.commands.cursor(), |position| position.text_offset);
                let desired_x = view.commands.desired_x().or_else(|| {
                    view.layout.snapshot()?.logical_endpoint_geometry(offset, view.commands.boundary_affinity()).ok().map(|geometry| geometry.rect.x)
                });
                region = LayoutJobRegion::Viewport(viewport.clone().with_horizontal_focus(offset, desired_x));
            }
        }
        if self.views[&view_id].commands.active_visual_block_endpoint_offsets().is_some() {
            if let LayoutJobRegion::Viewport(viewport) = region {
                region = LayoutJobRegion::Viewport(viewport.with_complete_horizontal_geometry());
            }
        }
        let document_revision = self.document.revision();
        let requirements = {
            let view = self
                .views
                .get_mut(&view_id)
                .expect("view existence was checked above");
            refresh_observed_metrics(view);
            let requirements = inspect_layout_provider(&view.engine);
            cancel_obsolete_layout_work(view, document_revision, requirements);
            if cancellation.is_cancelled() {
                return Err(CoreError::LayoutJob(LayoutJobError::Cancelled));
            }
            requirements
        };
        if let Some(active) = self
            .views
            .values()
            .flat_map(|view| [view.active_layout_work.as_ref(), view.active_prelayout_work.as_ref()].into_iter().flatten())
            .find(|active| active.cancellation.shares_state_with(&cancellation))
        {
            // One token represents exactly one request. Sharing it across
            // views would let cancellation in one view leak into another.
            return Err(CoreError::LayoutJob(
                LayoutJobError::CancellationTokenAlreadyActive {
                    job_id: active.job_id,
                },
            ));
        }
        let job_id = self.allocate_layout_job_id()?;
        let tracked_cancellation = cancellation.clone();
        let request = {
            let view = self
                .views
                .get_mut(&view_id)
                .expect("view remains attached during serial preparation");
            prepare_layout_job(
                &self.document,
                &mut view.layout,
                requirements,
                job_id,
                priority,
                region,
                cancellation,
            )?
        };
        let next = ActiveLayoutWork {
            job_id: request.job_id(),
            cancellation: tracked_cancellation,
            document_revision: request.document_revision(),
            configuration_generation: request.configuration_generation(),
            measurement_environment_id: request.measurement_environment_id(),
            metrics_generation: request.metrics_generation(),
        };
        let previous = self
            .views
            .get_mut(&view_id)
            .expect("view remains attached after serial preparation")
            .active_layout_work
            .replace(next);
        if let Some(previous) = previous {
            previous.cancellation.cancel();
        }
        Ok(request)
    }

    /// Prepare the exact viewport extension carried by a partial-edge command
    /// result. Identity checks prevent a frontend from accidentally applying a
    /// demand after another edit, reflow, metrics generation, or installation.
    pub fn prepare_view_layout_demand(
        &mut self,
        view_id: ViewId,
        priority: LayoutJobPriority,
        demand: &LayoutDemand,
        cancellation: LayoutCancellationToken,
    ) -> Result<crate::layout::LayoutJobRequest, CoreError> {
        let (viewport_top, viewport_height) = self.validate_view_layout_demand(view_id, demand)?;
        let mut viewport = ViewportLayoutRegion::new(
            demand.requested_hard_lines(),
            viewport_top,
            viewport_height,
        )?;
        if let Some((offset, x)) = demand.horizontal_focus() { viewport = viewport.with_horizontal_focus(offset, x); }
        if demand.requires_complete_horizontal_geometry() { viewport = viewport.with_complete_horizontal_geometry(); }
        let region = LayoutJobRegion::Viewport(viewport);
        self.prepare_view_layout_job(view_id, priority, region, cancellation)
    }

    fn validate_view_layout_demand(
        &mut self,
        view_id: ViewId,
        demand: &LayoutDemand,
    ) -> Result<(f32, f32), CoreError> {
        let (viewport_top, viewport_height) = {
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            refresh_observed_metrics(view);
            let snapshot = view.layout.snapshot();
            let current_revision = snapshot.map(|snapshot| snapshot.revision);
            let is_current = snapshot.is_some_and(|snapshot| {
                snapshot.document_id == demand.document_id()
                    && snapshot.document_revision == demand.document_revision()
                    && snapshot.revision == demand.layout_revision()
                    && snapshot.configuration_generation == demand.configuration_generation()
                    && snapshot.metrics_generation == demand.metrics_generation()
                    && snapshot.coverage.hard_lines() == demand.materialized_hard_lines()
            });
            if !is_current {
                return Err(CoreError::StaleLayoutDemand {
                    demand: demand.layout_revision(),
                    current: current_revision,
                });
            }
            (
                view.layout.viewport_top(),
                view.layout.height().max(f32::EPSILON),
            )
        };
        Ok((viewport_top, viewport_height))
    }

    /// Atomically validate and install a worker candidate into one view. A
    /// stale, cancelled, superseded, or provider-incompatible result changes
    /// neither its visible snapshot nor its regional height/cache state.
    pub fn install_view_layout_job(
        &mut self,
        view_id: ViewId,
        candidate: LayoutJobCandidate,
    ) -> Result<InstalledLayoutJob, CoreError> {
        // Validate against already-published presentation state. Publishing a
        // newly completed syntax result here would cancel this very candidate
        // during synchronous shaping. Syntax is published before preparation
        // or by an explicit host poll; either boundary still retires stale jobs.
        let job_id = candidate.job_id();
        let checkpoint = candidate.next_long_line_checkpoint().cloned();
        let view = self
            .views
            .get_mut(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        refresh_observed_metrics(view);
        let requirements = inspect_layout_provider(&view.engine);
        let installed = install_layout_job(
            &mut view.layout,
            LayoutInstallTarget {
                document_id: self.document.id(),
                document_revision: self.document.revision(),
                measurement_environment_id: requirements.measurement_environment_id,
                metrics_generation: requirements.metrics_generation,
            },
            candidate,
        )?;
        if let Some(checkpoint) = checkpoint {
            view.long_line_checkpoints
                .insert(&self.document, checkpoint);
        }
        if view
            .active_layout_work
            .as_ref()
            .is_some_and(|active| active.job_id == job_id)
        {
            // Successful installation is completion, not cancellation. Any
            // scheduler clone of the token must remain false.
            view.active_layout_work = None;
        }
        if let Some(snapshot) = view.layout.snapshot() {
            view.commands.rebind_visual_block(&self.document, snapshot);
        }
        // A host may install its own asynchronously computed viewport rather
        // than use the immediate layout helper. Honor the same pending syntax
        // baseline anchor before replacing it with the ordinary scroll anchor.
        if view.viewport_anchor.is_some_and(|anchor| {
            anchor.reference == ViewportAnchorReference::Baseline
                && anchor.anchor.document() == self.document.id()
                && anchor.anchor.revision() == self.document.revision()
                && view.layout.snapshot().is_some_and(|snapshot| {
                    snapshot.coverage.contains_text_offset(anchor.anchor.offset())
                })
        }) {
            if let Err(error) = restore_viewport_anchor(view) {
                view.layout.record_error(error);
            }
        }
        update_viewport_anchor(&self.document, view);
        Ok(installed)
    }

    fn allocate_view_id(&mut self) -> Result<ViewId, CoreError> {
        let raw = self
            .next_view
            .ok_or(CoreError::IdentifierExhausted(CoreIdentifierKind::View))?;
        self.next_view = raw.checked_add(1);
        Ok(ViewId(raw))
    }

    fn allocate_layout_job_id(&mut self) -> Result<LayoutJobId, CoreError> {
        let raw = self.next_layout_job.ok_or(CoreError::IdentifierExhausted(
            CoreIdentifierKind::LayoutJob,
        ))?;
        self.next_layout_job = raw.checked_add(1);
        Ok(LayoutJobId(raw))
    }

    fn materialize_immediate_viewport(
        &mut self,
        view_id: ViewId,
        intent: ImmediateLayoutIntent,
    ) -> Result<(), CoreError> {
        self.poll_syntax();
        self.synchronize_whitespace(view_id)?;
        // Font registration may advance metrics synchronously during shaping.
        // Retry only disposable layout work, never the input/source transaction.
        for attempt in 0..3 {
            let before = self.layout_provider_requirements(view_id)?;
            let result = self.materialize_immediate_viewport_once(view_id, intent);
            if result.is_ok() || attempt == 2 {
                return result;
            }
            let after = self.layout_provider_requirements(view_id)?;
            if before.metrics_generation == after.metrics_generation
                && before.measurement_environment_id == after.measurement_environment_id
            {
                return result;
            }
        }
        unreachable!("bounded layout retry always returns")
    }

    fn materialize_immediate_viewport_once(
        &mut self,
        view_id: ViewId,
        intent: ImmediateLayoutIntent,
    ) -> Result<(), CoreError> {
        const MIN_OVERSCAN_LINES: usize = 8;

        let flow = self.presentation_flow(view_id);
        let hard_line_count = self.document.projection().presentation_line_count(flow);
        let document_revision = self.document.revision();
        let (focus_offset, viewport_top, viewport_height, preserved_anchor, visual_block_endpoints) = {
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            if !self.document.format().is_source_view() {
                view.layout.set_paragraph_flow(false);
            }
            let document_is_stale = view.layout.snapshot().is_some_and(|snapshot| {
                snapshot.document_id != self.document.id()
                    || snapshot.document_revision != self.document.revision()
            });
            refresh_observed_metrics(view);
            let requirements = inspect_layout_provider(&view.engine);
            cancel_obsolete_layout_work(view, document_revision, requirements);
            view.layout
                .synchronize_document_hard_line_count(hard_line_count, document_is_stale)
                .map_err(LayoutError::from)?;
            let caret_offset = view.search_preview_destination(&self.document).unwrap_or_else(|| view
                .commands
                .visual_position()
                .map_or(view.commands.cursor(), |position| position.text_offset));
            let focus_offset = match (intent, view.viewport_anchor) {
                (ImmediateLayoutIntent::PreserveViewport | ImmediateLayoutIntent::PreserveViewportAndRevealCaret, Some(anchor))
                    if anchor.anchor.document() == self.document.id()
                        && anchor.anchor.revision() == self.document.revision() =>
                {
                    anchor.anchor.offset()
                }
                _ => caret_offset,
            };
            (
                focus_offset,
                view.layout.viewport_top(),
                view.layout.height(),
                view.viewport_anchor,
                view.commands.active_visual_block_endpoint_offsets(),
            )
        };
        let focus_line = self
            .document
            .projection()
            .presentation_line_at_offset(focus_offset, flow)
            .ok_or(LayoutError::InvalidTextOffset(focus_offset))?;
        if self.materialize_long_line_focus(
            view_id,
            focus_offset,
            focus_line,
            intent,
            preserved_anchor,
        )? {
            return Ok(());
        }
        let requested_top = {
            let view = self.views.get(&view_id).expect("view was validated above");
            match (intent, preserved_anchor) {
                (ImmediateLayoutIntent::PreserveViewport | ImmediateLayoutIntent::PreserveViewportAndRevealCaret, Some(anchor))
                    if anchor.anchor.document() == self.document.id()
                        && anchor.anchor.revision() == self.document.revision() =>
                {
                    let focus_top = view
                        .layout
                        .hard_line_prefix_height(focus_line)
                        .map_err(LayoutError::from)?
                        .height()
                        .min(f64::from(f32::MAX)) as f32;
                    (focus_top + anchor.offset_from_reference).max(0.0)
                }
                (ImmediateLayoutIntent::RevealCaret, _) => {
                    let cursor_is_materialized = view.layout.snapshot().is_some_and(|snapshot| {
                        snapshot.document_id == self.document.id()
                            && snapshot.document_revision == self.document.revision()
                            && snapshot.configuration_generation
                                == view.layout.configuration_generation()
                            && snapshot.coverage.contains_text_offset(focus_offset)
                    });
                    if cursor_is_materialized {
                        viewport_top
                    } else {
                        let focus_top =
                            view.layout
                                .hard_line_prefix_height(focus_line)
                                .map_err(LayoutError::from)?
                                .height()
                                .min(f64::from(f32::MAX)) as f32;
                        (focus_top - viewport_height / 2.0).max(0.0)
                    }
                }
                (ImmediateLayoutIntent::PreserveViewport | ImmediateLayoutIntent::PreserveViewportAndRevealCaret, _) => viewport_top,
            }
        };
        let initial_layout = self.views[&view_id].layout.snapshot().is_none() && preserved_anchor.is_none();
        let (visible_start, visible_end) = {
            let view = self.views.get(&view_id).expect("view was validated above");
            // A width change resets offscreen heights to estimates. In prose,
            // one hard line can span many rows: those estimates would schedule
            // several screens of unnecessary reflow on every resize tick.
            // Seed work from the previously visible lines instead. This is
            // only a work hint; the extension loop below still requires exact
            // coverage at the new size before publishing the presentation.
            let previous_visible = view.layout.snapshot().filter(|snapshot| {
                intent == ImmediateLayoutIntent::PreserveViewport
                    && snapshot.document_id == self.document.id()
                    && snapshot.document_revision == document_revision
                    && (snapshot.viewport_width != view.layout.width()
                        || snapshot.viewport_height != viewport_height)
            }).and_then(|snapshot| {
                let mut rows = snapshot.rows.iter().filter(|row| {
                    row.y + row.height() > viewport_top
                        && row.y < viewport_top + snapshot.viewport_height
                });
                let first = rows.next()?;
                let last = rows.last().unwrap_or(first);
                Some((first.hard_line_index, last.hard_line_index + 1))
            });
            let (start, end) = if initial_layout {
                // Unmeasured prose heights describe hard lines, not wrapped
                // screens. Start small and let the exact coverage loop extend
                // only as far as the first viewport needs. Speculation can fill
                // the neighboring pages after the first correct presentation.
                (focus_line, focus_line.saturating_add(4).min(hard_line_count))
            } else if let Some(previous) = previous_visible {
                previous
            } else {
                let start = view.layout.hard_line_at_y(f64::from(requested_top))
                    .map_err(LayoutError::from)?
                    .map_or(hard_line_count - 1, |hit| hit.hard_line());
                let bottom = requested_top + viewport_height.max(f32::EPSILON);
                let end = view.layout.hard_line_at_y(f64::from(bottom))
                    .map_err(LayoutError::from)?
                    .map_or(hard_line_count, |hit| hit.hard_line() + 1);
                (start, end)
            };
            (start.min(focus_line), end.max(focus_line + 1))
        };
        let overscan = if initial_layout { 0 } else { (visible_end - visible_start).max(MIN_OVERSCAN_LINES) };
        let mut start = visible_start.saturating_sub(overscan);
        let mut end = visible_end.saturating_add(overscan).min(hard_line_count);
        if let Some((anchor, active)) = visual_block_endpoints {
            let anchor_line = self
                .document
                .projection()
                .presentation_line_at_offset(anchor, flow)
                .ok_or(LayoutError::InvalidTextOffset(anchor))?;
            let active_line = self
                .document
                .projection()
                .presentation_line_at_offset(active, flow)
                .ok_or(LayoutError::InvalidTextOffset(active))?;
            start = start.min(anchor_line.min(active_line));
            end = end.max(anchor_line.max(active_line).saturating_add(1));
        }
        let mut next_requested_top = requested_top;

        loop {
            let region = LayoutJobRegion::Viewport(ViewportLayoutRegion::new(
                start..end,
                next_requested_top,
                viewport_height.max(f32::EPSILON),
            )?);
            let request = self.prepare_view_layout_job(
                view_id,
                LayoutJobPriority::ChangedVisibleRows,
                region,
                LayoutCancellationToken::new(),
            )?;
            let candidate = {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view remains attached during serial layout");
                compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?
            };
            self.install_view_layout_job(view_id, candidate)?;

            let (extend_before, extend_after) = {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view remains attached after layout installation");
                if intent != ImmediateLayoutIntent::RevealCaret {
                    view.viewport_anchor = preserved_anchor;
                    restore_viewport_anchor(view)?;
                }
                if intent != ImmediateLayoutIntent::PreserveViewport {
                    viewport::reveal_presentation_caret_row(&self.document, view)?;
                }
                update_viewport_anchor(&self.document, view);
                viewport_extension_needed(view)
            };
            if !extend_before && !extend_after {
                return Ok(());
            }
            let growth = (end - start).max(1);
            let previous = start..end;
            if extend_before {
                start = start.saturating_sub(growth);
            }
            if extend_after {
                end = end.saturating_add(growth).min(hard_line_count);
            }
            if start == previous.start && end == previous.end {
                return Ok(());
            }
            next_requested_top = self
                .views
                .get(&view_id)
                .expect("view remains attached while extending layout")
                .layout
                .viewport_top();
        }
    }

    /// Reach a distant caret or viewport anchor by resuming exact wrap state.
    /// The first visit must discover preceding row boundaries, but each job
    /// captures at most one bounded text slice and only the final slice is
    /// published. Later visits start from the nearest validated checkpoint.
    fn materialize_long_line_focus(
        &mut self,
        view_id: ViewId,
        focus_offset: usize,
        focus_line: usize,
        intent: ImmediateLayoutIntent,
        preserved_anchor: Option<ViewportTextAnchor>,
    ) -> Result<bool, CoreError> {
        let flow = self.presentation_flow(view_id);
        let line = self
            .document
            .projection()
            .presentation_line_range(focus_line, flow)
            .ok_or(LayoutError::InvalidTextOffset(focus_offset))?;
        let (line_start, line_end) = (line.start, line.end);
        let (mut checkpoint, top, height) = {
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            if !view.layout.wrap() || line_end - line_start <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES {
                return Ok(false);
            }
            let requirements = inspect_layout_provider(&view.engine);
            view.long_line_checkpoints
                .discard_stale(&self.document, &view.layout, requirements);
            let mut checkpoint = view
                .long_line_checkpoints
                .before(line_start..line_end, focus_offset);
            if let Some(current) = &checkpoint {
                // Retain enough of the preceding slice to paint the viewport
                // when the editing row falls just after a checkpoint boundary.
                checkpoint = view.long_line_checkpoints.before_height(
                    line_start..line_end,
                    (current.completed_height() - view.layout.height()).max(0.0),
                );
            }
            (
                checkpoint,
                view.layout.viewport_top(),
                view.layout.height().max(f32::EPSILON),
            )
        };
        let mut preceding_tail = None;
        loop {
            let work_start = checkpoint
                .as_ref()
                .map_or(line_start, LongLineLayoutCheckpoint::next_text_offset);
            let viewport = match checkpoint.take() {
                Some(checkpoint) => {
                    ViewportLayoutRegion::resume_long_line(checkpoint, top, height)?
                }
                None => ViewportLayoutRegion::new(focus_line..focus_line + 1, top, height)?,
            };
            let request = self.prepare_view_layout_job(
                view_id,
                LayoutJobPriority::ChangedVisibleRows,
                LayoutJobRegion::Viewport(viewport),
                LayoutCancellationToken::new(),
            )?;
            let mut candidate = {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("layout view remains attached");
                compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?
            };
            let coverage = candidate.regional_snapshot().lines()[0].text_coverage();
            if coverage.start <= focus_offset && focus_offset <= coverage.end {
                if let Some(preceding) = &preceding_tail {
                    let rows = candidate.regional_snapshot().lines()[0].rows();
                    let row = rows.iter().find(|row| row.text_range.start <= focus_offset
                        && focus_offset <= row.text_range.end).expect("focus lies in the candidate");
                    let view = &self.views[&view_id];
                    let desired_top = if intent != ImmediateLayoutIntent::RevealCaret {
                        preserved_anchor.map(|anchor| match anchor.reference {
                            ViewportAnchorReference::Baseline => row.baseline,
                            ViewportAnchorReference::RowTop => row.y,
                        } + anchor.offset_from_reference)
                    } else { None }.unwrap_or_else(|| {
                        let prefix = view.layout.hard_line_prefix_height(focus_line)
                            .expect("validated focus line").height() as f32;
                        view.layout.reveal_viewport_top(row, top - prefix)
                    });
                    if rows[0].y > desired_top {
                        candidate.prepend_long_line_slice(preceding);
                    }
                }
                let caret = self.views[&view_id].commands.cursor();
                let following_line = focus_line + 1;
                if intent == ImmediateLayoutIntent::PreserveViewportAndRevealCaret
                    && coverage.end < caret
                    && self.document.projection().presentation_line_range(following_line, flow)
                        .is_some_and(|line| line.start == caret)
                {
                    // Enter can split a giant paragraph. Preserve the saved
                    // row and shape the new paragraph's first bounded slice in
                    // the same published snapshot before revealing the caret.
                    let preceding = candidate.regional_snapshot().clone();
                    let request = self.prepare_view_layout_job(
                        view_id,
                        LayoutJobPriority::ChangedVisibleRows,
                        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(
                            following_line..following_line + 1, top, height,
                        )?),
                        LayoutCancellationToken::new(),
                    )?;
                    let view = self.views.get_mut(&view_id).expect("validated edit view");
                    candidate = compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?;
                    candidate.prepend_adjacent_region(&preceding);
                }
                if intent != ImmediateLayoutIntent::RevealCaret {
                    if let Some(anchor) = preserved_anchor {
                        let relative_reference = candidate.regional_snapshot().lines()[0].rows()
                            .iter().find(|row| row.text_range.start <= focus_offset && focus_offset <= row.text_range.end)
                            .map(|row| match anchor.reference {
                                ViewportAnchorReference::Baseline => row.baseline,
                                ViewportAnchorReference::RowTop => row.y,
                            });
                        if let Some(reference) = relative_reference {
                            let required = -(reference + anchor.offset_from_reference);
                            if required > 0. {
                                candidate = viewport::include_preceding_rows(self, view_id, candidate, required)?;
                            }
                        }
                    }
                }
                loop {
                    let preceding = candidate.regional_snapshot().clone();
                    let continuation = candidate.next_long_line_checkpoint().cloned();
                    self.install_view_layout_job(view_id, candidate)?;
                    let view = self.views.get_mut(&view_id).expect("layout view remains attached");
                    if intent != ImmediateLayoutIntent::RevealCaret {
                        view.viewport_anchor = preserved_anchor;
                        restore_viewport_anchor(view)?;
                    }
                    if intent != ImmediateLayoutIntent::PreserveViewport {
                        viewport::reveal_presentation_caret_row(&self.document, view)?;
                    }
                    update_viewport_anchor(&self.document, view);
                    let top = view.layout.viewport_top();
                    let bottom = top + view.layout.height();
                    let snapshot = view.layout.snapshot().expect("installed long-line layout");
                    if snapshot.coverage.vertical_range().is_none_or(|coverage| bottom <= coverage.end) {
                        break;
                    }
                    let region = if let Some(checkpoint) = continuation {
                        ViewportLayoutRegion::resume_long_line(checkpoint, top, height)?
                    } else {
                        let following = preceding.hard_lines().end;
                        if following >= self.document.projection().presentation_line_count(flow) { break; }
                        ViewportLayoutRegion::new(following..following + 1, top, height)?
                    };
                    let same_line = region.long_line_checkpoint().is_some();
                    let request = self.prepare_view_layout_job(
                        view_id, LayoutJobPriority::ChangedVisibleRows,
                        LayoutJobRegion::Viewport(region), LayoutCancellationToken::new(),
                    )?;
                    let view = self.views.get_mut(&view_id).expect("validated editing view");
                    candidate = compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?;
                    if same_line {
                        candidate.prepend_long_line_slice(&preceding);
                    } else {
                        candidate.prepend_adjacent_region(&preceding);
                    }
                }
                return Ok(true);
            }
            let next = candidate
                .next_long_line_checkpoint()
                .cloned()
                .filter(|next| next.next_text_offset() > work_start)
                .ok_or(LayoutJobError::InvalidLongLineCheckpoint(
                    "continuation did not reach or advance toward the requested caret",
                ))?;
            self.views
                .get_mut(&view_id)
                .expect("layout view remains attached")
                .long_line_checkpoints
                .insert(&self.document, next.clone());
            candidate.retain_viewport_tail(preceding_tail.as_ref());
            preceding_tail = Some(candidate.regional_snapshot().clone());
            checkpoint = Some(next);
        }
    }

    /// Measure the terminal viewport backward by hard-line band. A trailing
    /// empty line must not cause an overscan job to capture a preceding giant
    /// paragraph in full; long bands use the same resumable chunks as caret
    /// navigation, and only their viewport tail is retained between chunks.
    fn materialize_document_end(
        &mut self,
        view_id: ViewId,
        staged_layout: &mut ViewLayout,
        requirements: LayoutProviderRequirements,
        immediate_layout_context: LayoutExecutionContext,
    ) -> Result<Vec<LongLineLayoutCheckpoint>, CoreError> {
        let flow = self.presentation_flow(view_id);
        let count = self.document.projection().presentation_line_count(flow);
        let height = staged_layout.height().max(f32::EPSILON);
        let mut following = None;
        let mut following_height = 0.0;
        let mut checkpoints = Vec::new();
        for line in (0..count).rev() {
            let range = self
                .document
                .projection()
                .presentation_line_range(line, flow)
                .ok_or(LayoutError::InvalidTextOffset(self.document.projection().text_tree().byte_len()))?;
            let long = staged_layout.wrap() && range.len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES;
            let mut checkpoint = if long {
                let view = self.views.get_mut(&view_id).expect("view was validated");
                view.long_line_checkpoints.discard_stale(
                    &self.document,
                    staged_layout,
                    requirements,
                );
                view.long_line_checkpoints
                    .before(range.clone(), range.end)
                    .and_then(|last| {
                        view.long_line_checkpoints.before_height(
                            range.clone(),
                            (last.completed_height() - height).max(0.0),
                        )
                    })
            } else {
                None
            };
            let mut chunk_tail = None;
            loop {
                let work_start = checkpoint
                    .as_ref()
                    .map_or(range.start, LongLineLayoutCheckpoint::next_text_offset);
                let region = match checkpoint.take() {
                    Some(checkpoint) => {
                        ViewportLayoutRegion::resume_long_line(checkpoint, f32::MAX, height)?
                    }
                    None => ViewportLayoutRegion::new(line..line + 1, f32::MAX, height)?,
                };
                let job_id = self.allocate_layout_job_id()?;
                let request = prepare_layout_job(
                    &self.document,
                    staged_layout,
                    requirements,
                    job_id,
                    LayoutJobPriority::NewlyExposedRows,
                    LayoutJobRegion::Viewport(region),
                    LayoutCancellationToken::new(),
                )?;
                let mut candidate = {
                    let view = self.views.get_mut(&view_id).expect("view remains attached");
                    compute_layout_job(&mut view.engine, &request, immediate_layout_context)?
                };
                let next = candidate.next_long_line_checkpoint().cloned();
                candidate.retain_viewport_tail(chunk_tail.as_ref());
                if let Some(next) = next {
                    if next.next_text_offset() <= work_start {
                        return Err(LayoutJobError::InvalidLongLineCheckpoint(
                            "document-end continuation did not advance",
                        )
                        .into());
                    }
                    chunk_tail = Some(candidate.regional_snapshot().clone());
                    checkpoints.push(next.clone());
                    if checkpoints.len() > 256 {
                        checkpoints.remove(0);
                    }
                    checkpoint = Some(next);
                    continue;
                }
                following_height += candidate.regional_snapshot().lines()[0].height();
                candidate.append_following_viewport_tail(following.as_ref());
                if following_height >= f64::from(height) || line == 0 {
                    install_layout_job(
                        staged_layout,
                        LayoutInstallTarget {
                            document_id: self.document.id(),
                            document_revision: self.document.revision(),
                            measurement_environment_id: requirements.measurement_environment_id,
                            metrics_generation: requirements.metrics_generation,
                        },
                        candidate,
                    )?;
                    staged_layout.set_viewport_top(f32::MAX)?;
                    return Ok(checkpoints);
                }
                following = Some(candidate.regional_snapshot().clone());
                break;
            }
        }
        unreachable!("a formatted document always has one hard line")
    }

    /// Stage and atomically install the exact local layout needed for an
    /// absolute vertical presentation request. The requested y is first
    /// resolved through the compact height index. Its hard line and within-line
    /// offset then act as the refinement anchor, so discovering different
    /// heights in the local overscan cannot strand the viewport on neighboring
    /// text. Unknown heights before that neighborhood remain estimates.
    fn materialize_requested_viewport(
        &mut self,
        view_id: ViewId,
        left: f32,
        requested_top: f32,
    ) -> Result<(), CoreError> {
        // A wheel tick usually stays inside the already materialized overscan.
        // Reuse that exact document-coordinate geometry without cloning caches,
        // publishing another layout revision or cancelling its background job.
        let already_covered = {
            let view = self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?;
            let requirements = inspect_layout_provider(&view.engine);
            current_snapshot_for_layout(&self.document, &view.layout, requirements)
                .is_some_and(|snapshot| {
                    !snapshot.has_horizontal_materialization()
                        && snapshot.missing_viewport_edges(
                            snapshot.clamp_viewport_top(requested_top, view.layout.height()),
                            view.layout.height(),
                        ) == (false, false)
                })
        };
        if already_covered {
            let view = self.views.get_mut(&view_id).expect("view was validated");
            view.layout.set_viewport_top(requested_top.max(0.0))?;
            view.layout.set_viewport_left(left)?;
            update_viewport_anchor(&self.document, view);
            return Ok(());
        }
        // Keep the content location resolved from the caller's geometry even
        // if a retry retires exact heights and replaces them with estimates.
        // None is the explicit end-of-document refinement intent.
        let target_hit = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?
            .layout
            .hard_line_at_y(f64::from(requested_top.max(0.0)))
            .map_err(LayoutError::from)?;
        // Resolving newly exposed text can register a fallback font and retire
        // metrics while the disposable viewport is being shaped. Rebuild only
        // that staged viewport; the scroll has not been published, so neither
        // axis, anchors, nor command/source state is applied twice. Persistent
        // invalidation and unrelated provider failures must still terminate.
        for attempt in 0..3 {
            let before = self.layout_provider_requirements(view_id)?;
            let result =
                self.materialize_requested_viewport_once(view_id, left, requested_top, target_hit);
            if result.is_ok() || attempt == 2 {
                return result;
            }
            let after = self.layout_provider_requirements(view_id)?;
            if before.metrics_generation == after.metrics_generation
                || before.measurement_environment_id != after.measurement_environment_id
                || before.threading != after.threading
            {
                return result;
            }
        }
        unreachable!("bounded viewport retry always returns")
    }

    fn materialize_requested_viewport_once(
        &mut self,
        view_id: ViewId,
        left: f32,
        requested_top: f32,
        target_hit: Option<crate::layout::HardLineHeightHit>,
    ) -> Result<(), CoreError> {
        const MIN_OVERSCAN_LINES: usize = 8;

        let flow = self.presentation_flow(view_id);
        let hard_line_count = self.document.projection().presentation_line_count(flow);
        let document_revision = self.document.revision();
        let (mut staged_layout, requirements, immediate_layout_context, metrics_changed) = {
            let view = self
                .views
                .get(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            let requirements = inspect_layout_provider(&view.engine);
            (
                view.layout.clone(),
                requirements,
                view.immediate_layout_context,
                view.observed_metrics_generation != requirements.metrics_generation,
            )
        };
        let dependency_changed = staged_layout.snapshot().is_some_and(|snapshot| {
            snapshot.measurement_environment_id != requirements.measurement_environment_id
                || snapshot.metrics_generation != requirements.metrics_generation
        });
        if metrics_changed || dependency_changed {
            staged_layout.invalidate_text_metrics();
        }
        let document_is_stale = staged_layout.snapshot().is_some_and(|snapshot| {
            snapshot.document_id != self.document.id()
                || snapshot.document_revision != document_revision
        });
        staged_layout
            .synchronize_document_hard_line_count(hard_line_count, document_is_stale)
            .map_err(LayoutError::from)?;

        // Horizontal materialization uses the requested origin during capture;
        // publication remains atomic with the staged vertical viewport.
        staged_layout.set_viewport_left(left)?;

        let viewport_height = staged_layout.height().max(f32::EPSILON);
        let requested_top = requested_top.max(0.0);
        let target_line = target_hit.map_or(hard_line_count - 1, |hit| hit.hard_line());
        let target_line_top = match target_hit {
            Some(hit) => hit.line_top(),
            None => staged_layout
                .hard_line_prefix_height(target_line)
                .map_err(LayoutError::from)?
                .height(),
        };
        let offset_from_target_line = (f64::from(requested_top) - target_line_top).max(0.0);

        let mut new_checkpoints = Vec::new();
        if target_hit.is_none() {
            new_checkpoints = self.materialize_document_end(
                view_id,
                &mut staged_layout,
                requirements,
                immediate_layout_context,
            )?;
        } else {
            let estimated_line_top = staged_layout
                .hard_line_prefix_height(target_line)
                .map_err(LayoutError::from)?
                .height();
            let estimated_target_top = (estimated_line_top + offset_from_target_line)
                .min(f64::from(f32::MAX)) as f32;
            // A metric retry can replace a tall wrapped line with a short
            // estimate. Its retained within-line offset still belongs to that
            // line, not to hundreds of later estimated lines. Bound only the
            // interval estimate; shaping retains the full requested offset.
            let estimated_line_height = staged_layout
                .hard_line_range_height(target_line..target_line + 1)
                .map_err(LayoutError::from)?
                .height();
            let estimated_visible_bottom = estimated_line_top
                + offset_from_target_line.min(estimated_line_height)
                + f64::from(viewport_height);
            let visible_end = staged_layout
                .hard_line_at_y(estimated_visible_bottom)
                .map_err(LayoutError::from)?
                .map_or(hard_line_count, |hit| hit.hard_line().saturating_add(1));
            let visible_start = target_line;
            let visible_end = visible_end.max(target_line.saturating_add(1));
            let overscan = (visible_end - visible_start).max(MIN_OVERSCAN_LINES);
            let mut start = visible_start.saturating_sub(overscan);
            let mut end = visible_end.saturating_add(overscan).min(hard_line_count);
            let mut next_requested_top = estimated_target_top;

            loop {
                let region = LayoutJobRegion::Viewport(ViewportLayoutRegion::new(
                    start..end,
                    next_requested_top,
                    viewport_height,
                )?);
                let job_id = self.allocate_layout_job_id()?;
                let request = prepare_layout_job(
                    &self.document,
                    &mut staged_layout,
                    requirements,
                    job_id,
                    LayoutJobPriority::NewlyExposedRows,
                    region,
                    LayoutCancellationToken::new(),
                )?;
                let candidate = {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("view remains attached during serial layout");
                    compute_layout_job(&mut view.engine, &request, immediate_layout_context)?
                };
                install_layout_job(
                    &mut staged_layout,
                    LayoutInstallTarget {
                        document_id: self.document.id(),
                        document_revision,
                        measurement_environment_id: requirements.measurement_environment_id,
                        metrics_generation: requirements.metrics_generation,
                    },
                    candidate,
                )?;

                let refined_line_top = staged_layout
                    .hard_line_prefix_height(target_line)
                    .map_err(LayoutError::from)?
                    .height();
                let refined_line_height = staged_layout
                    .hard_line_range_height(target_line..target_line + 1)
                    .map_err(LayoutError::from)?
                    .height();
                let anchored_top =
                    refined_line_top + offset_from_target_line.min(refined_line_height);
                staged_layout.set_viewport_top(anchored_top.min(f64::from(f32::MAX)) as f32)?;

                let (extend_before, extend_after) =
                    viewport_layout_extension_needed(&staged_layout);
                if !extend_before && !extend_after {
                    break;
                }
                let growth = (end - start).max(1);
                let previous = start..end;
                if extend_before {
                    start = start.saturating_sub(growth);
                }
                if extend_after {
                    end = end.saturating_add(growth).min(hard_line_count);
                }
                if start == previous.start && end == previous.end {
                    break;
                }
                next_requested_top = staged_layout.viewport_top();
            }
        }

        staged_layout.set_viewport_left(left)?;
        let actual_requirements = {
            let view = self
                .views
                .get(&view_id)
                .expect("view remains attached before atomic layout publication");
            inspect_layout_provider(&view.engine)
        };
        if actual_requirements.threading != requirements.threading {
            return Err(CoreError::LayoutJob(
                LayoutJobError::ProviderThreadingChanged {
                    expected: requirements.threading,
                    actual: actual_requirements.threading,
                },
            ));
        }
        if actual_requirements.measurement_environment_id != requirements.measurement_environment_id
        {
            return Err(CoreError::LayoutJob(
                LayoutJobError::WrongMeasurementEnvironment {
                    expected: requirements.measurement_environment_id,
                    actual: actual_requirements.measurement_environment_id,
                },
            ));
        }
        if actual_requirements.metrics_generation != requirements.metrics_generation {
            return Err(CoreError::LayoutJob(LayoutJobError::StaleMetrics {
                expected: requirements.metrics_generation,
                actual: actual_requirements.metrics_generation,
            }));
        }

        let view = self
            .views
            .get_mut(&view_id)
            .expect("view remains attached for atomic layout publication");
        if let Some(active) = view.active_layout_work.take() { active.cancellation.cancel(); }
        view.layout = staged_layout;
        cancel_obsolete_layout_work(view, document_revision, requirements);
        for checkpoint in new_checkpoints {
            view.long_line_checkpoints
                .insert(&self.document, checkpoint);
        }
        view.observed_metrics_generation = requirements.metrics_generation;
        update_viewport_anchor(&self.document, view);
        Ok(())
    }

    fn ensure_command_layout(
        &mut self,
        view_id: ViewId,
        intent: ImmediateLayoutIntent,
    ) -> Result<bool, CoreError> {
        let document_revision = self.document.revision();
        let needs_layout = {
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            let metrics_changed = refresh_observed_metrics(view);
            let requirements = inspect_layout_provider(&view.engine);
            cancel_obsolete_layout_work(view, document_revision, requirements);
            let cursor = view.commands.cursor();
            let visual_block_endpoints = view.commands.active_visual_block_endpoint_offsets();
            let top = view.layout.viewport_top();
            metrics_changed
                || view.layout.snapshot().map_or(true, |snapshot| {
                    snapshot.document_id != self.document.id()
                        || snapshot.document_revision != self.document.revision()
                        || snapshot.configuration_generation
                            != view.layout.configuration_generation()
                        || snapshot.measurement_environment_id
                            != requirements.measurement_environment_id
                        || snapshot.metrics_generation != requirements.metrics_generation
                        || (intent != ImmediateLayoutIntent::PreserveViewport
                            && (!snapshot.coverage.contains_text_offset(cursor)
                                || !snapshot.horizontal_text_is_materialized(cursor)))
                        || (visual_block_endpoints.is_some() && snapshot.has_horizontal_materialization())
                        || visual_block_endpoints.is_some_and(|(anchor, active)| {
                            !snapshot.coverage.contains_text_offset(anchor)
                                || !snapshot.coverage.contains_text_offset(active)
                        })
                        || snapshot.missing_viewport_edges(top, view.layout.height()) != (false, false)
                })
        };
        if needs_layout {
            // Rebuilding only the viewport can leave a scrolled-away caret
            // outside the new snapshot too. Caret-relative commands need its
            // row; viewport-relative commands need their original viewport.
            self.materialize_immediate_viewport(view_id, intent)?;
        }
        Ok(needs_layout)
    }

    fn record_presentation_error(&mut self, view_id: ViewId, error: CoreError) {
        let layout_error = match error {
            CoreError::Layout(error) => Some(error),
            CoreError::LayoutJob(LayoutJobError::Layout(error)) => Some(error),
            _ => None,
        };
        if let (Some(view), Some(error)) = (self.views.get_mut(&view_id), layout_error) {
            view.layout.record_error(error);
        }
    }

    fn rebase_viewport_anchors(&mut self, map: &PositionMap) -> Result<(), CoreError> {
        for view in self.views.values_mut() {
            view.long_line_checkpoints
                .rebase(&self.document, map, view.layout.paragraph_flow());
            let Some(current) = view.viewport_anchor else {
                continue;
            };
            if current.anchor.revision() != map.source_revision() {
                view.viewport_anchor = None;
                continue;
            }
            view.viewport_anchor = match map.map_text_anchor(current.anchor)? {
                MappingOutcome::Exact(anchor)
                | MappingOutcome::Moved(anchor)
                | MappingOutcome::CollapsedByDeletion(anchor)
                | MappingOutcome::RecoveredFromProvenance(anchor) => {
                    Some(ViewportTextAnchor { anchor, ..current })
                }
                MappingOutcome::Ambiguous(_) | MappingOutcome::Unresolvable(_) => None,
            };
        }
        Ok(())
    }

    fn materialize_views_after_document_change(
        &mut self,
        active: ViewId,
        active_intent: ImmediateLayoutIntent,
    ) {
        let view_ids: Vec<_> = self.views.keys().copied().collect();
        for view_id in view_ids {
            // A source commit retires snapshot-bound completion queries and
            // their preview layouts before any new presentation is built.
            let _ = self.retire_stale_completion(view_id);
            let intent = if view_id == active {
                active_intent
            } else {
                ImmediateLayoutIntent::PreserveViewport
            };
            if let Err(error) = self.materialize_immediate_viewport(view_id, intent) {
                self.record_presentation_error(view_id, error);
            }
        }
    }

    fn cancel_all_active_layout_work(&mut self) {
        for view in self.views.values_mut() {
            cancel_active_layout_work(view);
        }
    }

    /// Return the active revision-bound overlay for a view, if any.
    pub fn composition_overlay(
        &self,
        view: ViewId,
    ) -> Result<Option<CompositionOverlay>, CoreError> {
        let view = self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        if let Some(session) = &view.composition {
            return session.overlay(&self.document).map(Some).map_err(CoreError::from);
        }
        view.completion.as_ref()
            .filter(|session| session.is_current(&self.document, &view.commands))
            .map(|session| session.overlay(&self.document).map_err(CoreError::from))
            .transpose().map(Option::flatten)
    }

    /// Shape and wrap the active composition through the same provider and
    /// resolved style inputs as ordinary document layout. Only the affected
    /// hard lines and an already materialized viewport neighborhood are
    /// captured. Thus repeated marked-text updates do not flatten or lay out
    /// the complete document. The source-backed `ViewLayout` is cloned as
    /// disposable presentation state; publishing it cannot mutate source,
    /// history, commands, or the restorable base layout.
    fn materialize_composition_layout(
        &mut self,
        view_id: ViewId,
        reveal_selection: bool,
    ) -> Result<(), CoreError> {
        let (overlay, affinity, mut composed_layout, base_coverage, requested_top) = {
            let view = self
                .views
                .get(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            let overlay = self.composition_overlay(view_id)?
                .ok_or(CoreError::Composition(CompositionError::NoActiveSession))?;
            (
                overlay,
                view.commands.boundary_affinity(),
                view.layout.clone(),
                view.layout
                    .snapshot()
                    .map(|snapshot| snapshot.coverage.hard_lines()),
                view.layout.viewport_top(),
            )
        };

        let flow = composed_layout.paragraph_flow();
        let base_line_count = self.document.projection().presentation_line_count(flow);
        let replaced = overlay.replacement_range();
        let affected_start = self
            .document
            .projection()
            .presentation_line_at_offset(replaced.start, flow)
            .ok_or(LayoutError::InvalidTextOffset(replaced.start))?;
        let affected_end = self
            .document
            .projection()
            .presentation_line_at_offset(replaced.end, flow)
            .ok_or(LayoutError::InvalidTextOffset(replaced.end))?
            .saturating_add(1)
            .min(base_line_count);
        let affected = affected_start..affected_end;
        let affected_line = self
            .document
            .projection()
            .presentation_line_range(affected_start, flow)
            .ok_or(LayoutError::InvalidTextOffset(replaced.start))?;
        if affected.len() == 1
            && composed_layout.wrap()
            && affected_line.len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES
            && !overlay.marked_text().contains('\n')
        {
            return self.materialize_long_composition_layout(
                view_id,
                overlay,
                affinity,
                composed_layout,
                affected_line,
                affected_start,
                base_line_count,
                reveal_selection,
            );
        }
        let base_region = base_coverage
            .filter(|visible| ranges_touch(visible, &affected))
            .map_or_else(
                || affected.clone(),
                |visible| union_ranges(visible, &affected),
            );
        let base_text_start = self
            .document
            .projection()
            .presentation_line_range(base_region.start, flow)
            .ok_or(LayoutError::InvalidTextOffset(replaced.start))?
            .start;
        let base_text_end = self
            .document
            .projection()
            .presentation_line_range(base_region.end - 1, flow)
            .ok_or(LayoutError::InvalidTextOffset(replaced.end))?
            .end;
        let text_origin = overlay
            .overlay_offset_for_base_boundary(base_text_start, Association::BeforeInsertion)
            .ok_or(LayoutError::InvalidTextOffset(base_text_start))?;
        let text_end = overlay
            .overlay_offset_for_base_boundary(base_text_end, Association::AfterInsertion)
            .ok_or(LayoutError::InvalidTextOffset(base_text_end))?;
        let mut text = overlay
            .text_in_range(text_origin..text_end)
            .ok_or(LayoutError::InvalidTextOffset(text_end))?;
        if flow {
            let mut bytes = text.into_bytes();
            for line in base_region.clone() {
                let range = self
                    .document
                    .projection()
                    .presentation_line_range(line, true)
                    .unwrap();
                let original = self
                    .document
                    .projection()
                    .text_tree()
                    .slice(range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                for (relative, _) in original.match_indices('\n') {
                    let at = range.start + relative;
                    if replaced.contains(&at) {
                        continue;
                    }
                    if let Some(mapped) =
                        overlay.overlay_offset_for_base_boundary(at, Association::AfterInsertion)
                    {
                        if let Some(byte) = bytes.get_mut(mapped.saturating_sub(text_origin)) {
                            if *byte == b'\n' {
                                *byte = b' ';
                            }
                        }
                    }
                }
            }
            text = String::from_utf8(bytes).expect("ASCII whitespace changes preserve UTF-8");
        }
        let mut line_ranges = hard_line_ranges(&text, text_origin);
        if flow {
            // Source-flow block boundaries may have no newline character.
            // Preserve their positions in the composition overlay alongside
            // the physical breaks and newly authored composition newlines.
            let mut boundaries = Vec::new();
            for line in base_region.start + 1..base_region.end {
                let previous = self
                    .document
                    .projection()
                    .presentation_line_range(line - 1, true)
                    .unwrap();
                let next = self
                    .document
                    .projection()
                    .presentation_line_range(line, true)
                    .unwrap();
                if previous.end == next.start && !replaced.contains(&next.start) {
                    let association = if affinity == BoundaryAffinity::Upstream {
                        Association::AfterInsertion
                    } else {
                        Association::BeforeInsertion
                    };
                    if let Some(at) =
                        overlay.overlay_offset_for_base_boundary(next.start, association)
                    {
                        boundaries.push(at);
                    }
                }
            }
            line_ranges = line_ranges
                .into_iter()
                .flat_map(|range| {
                    let mut start = range.start;
                    let mut split = Vec::new();
                    for &at in boundaries
                        .iter()
                        .filter(|&&at| range.start < at && at < range.end)
                    {
                        split.push(start..at);
                        start = at;
                    }
                    split.push(start..range.end);
                    split
                })
                .collect();
        }
        let overlay_line_count = base_line_count - base_region.len() + line_ranges.len();
        let requested_end = base_region
            .start
            .checked_add(line_ranges.len())
            .ok_or(LayoutError::InvalidTextOffset(overlay.utf8_len()))?;
        if requested_end > overlay_line_count {
            return Err(LayoutError::MalformedMeasurement(
                "composition hard-line range exceeds the overlay",
            )
            .into());
        }
        let following_base_range = (base_region.end < base_line_count).then(|| {
            self.document
                .projection()
                .presentation_line_range(base_region.end, flow)
                .expect("validated following presentation line")
        });
        let following_line_range = following_base_range
            .as_ref()
            .map(|range| map_base_range_to_overlay(&overlay, range))
            .transpose()?;
        if (requested_end < overlay_line_count) != following_line_range.is_some() {
            return Err(LayoutError::MalformedMeasurement(
                "composition regional layout has inconsistent following-line context",
            )
            .into());
        }

        let style_end = following_base_range
            .as_ref()
            .map(|range| {
                self.document
                    .projection()
                    .text_tree()
                    .next_grapheme_boundary(range.start)
                    .map(|next| next.unwrap_or(range.start).min(range.end))
            })
            .transpose()
            .map_err(DocumentError::FormattedTextStorage)?
            .unwrap_or(base_text_end);
        let mut styles = DocumentLayoutStyles::resolve_region_with_search(
            self.document.projection(),
            base_text_start..style_end,
            flow,
            self.views[&view_id].layout.search_matches(self.document.id(), self.document.revision()),
        )
        .map_err(LayoutError::from)?;
        if flow {
            let mut base_ranges = base_region
                .clone()
                .map(|line| {
                    self.document
                        .projection()
                        .presentation_line_range(line, true)
                        .expect("validated presentation line")
                })
                .collect::<Vec<_>>();
            base_ranges.extend(following_base_range.clone());
            crate::layout::resolve_flow_paragraph_styles(
                self.document.projection(),
                &mut styles,
                &base_ranges,
            )
            .map_err(LayoutError::from)?;
        }
        let mut styles = composition_layout_styles(styles, &overlay, affinity)?;
        if flow {
            let mut overlay_ranges = line_ranges.clone();
            overlay_ranges.extend(following_line_range.clone());
            crate::layout::flow_paragraph_styles(&mut styles, &overlay_ranges);
        }
        composed_layout
            .synchronize_document_hard_line_count(overlay_line_count, true)
            .map_err(LayoutError::from)?;
        let captured_view = composed_layout.capture_for_regional_layout_job(text_origin..text_end);
        let job_id = self.allocate_layout_job_id()?;
        if !composed_layout.begin_layout_job(job_id) {
            return Err(CoreError::IdentifierExhausted(
                CoreIdentifierKind::LayoutJob,
            ));
        }
        let cancellation = LayoutCancellationToken::new();
        let view = self
            .views
            .get_mut(&view_id)
            .expect("composition view remains attached during synchronous layout");
        let region = view.engine.layout_hard_line_region_cancellable(
            self.document.id(),
            self.document.revision(),
            &text,
            text_origin,
            &line_ranges,
            base_region.start,
            overlay_line_count,
            overlay.utf8_len(),
            following_line_range,
            &styles,
            &captured_view,
            &cancellation,
        );
        let region = match region {
            Ok(region) => region,
            Err(LayoutComputationError::Layout(error)) => return Err(error.into()),
            Err(LayoutComputationError::Cancelled) => return Err(LayoutJobError::Cancelled.into()),
        };
        composed_layout
            .publish_layout_job_viewport(job_id, region, requested_top)
            .map_err(LayoutError::from)?;
        if reveal_selection {
            viewport::reveal_layout_row_at(
                &mut composed_layout,
                overlay.selected_range_in_overlay().end,
                BoundaryAffinity::Downstream,
            )?;
        }
        view.composition_layout = Some(composed_layout);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_long_composition_layout(
        &mut self,
        view_id: ViewId,
        overlay: CompositionOverlay,
        affinity: BoundaryAffinity,
        mut composed_layout: ViewLayout,
        base_range: std::ops::Range<usize>,
        line_index: usize,
        line_count: usize,
        reveal_selection: bool,
    ) -> Result<(), CoreError> {
        let flow = composed_layout.paragraph_flow();
        let replaced = overlay.replacement_range();
        let marked = overlay.marked_range();
        let full_range = base_range.start
            ..overlay
                .overlay_offset_for_base_boundary(base_range.end, Association::AfterInsertion)
                .ok_or(LayoutError::InvalidTextOffset(base_range.end))?;
        let tree = overlay
            .layout_text_tree()
            .map_err(DocumentError::FormattedTextStorage)?;
        let focus = overlay.selected_range_in_overlay().end;
        let mut checkpoint = {
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            let requirements = inspect_layout_provider(&view.engine);
            view.long_line_checkpoints
                .discard_stale(&self.document, &view.layout, requirements);
            view.long_line_checkpoints
                .before(
                    base_range.clone(),
                    replaced.start.saturating_sub(128).max(base_range.start),
                )
                .and_then(|checkpoint| {
                    checkpoint.for_unchanged_prefix(
                        full_range.clone(),
                        replaced.start.saturating_sub(128),
                    )
                })
        };
        let to_base = |offset: usize, end: bool| {
            if offset <= marked.start {
                offset
            } else if offset < marked.end {
                if end {
                    replaced.end
                } else {
                    replaced.start
                }
            } else {
                replaced.end + offset - marked.end
            }
        };
        let cancellation = LayoutCancellationToken::new();
        loop {
            let work_start = checkpoint
                .as_ref()
                .map_or(full_range.start, LongLineLayoutCheckpoint::next_text_offset);
            let (work_end, capture) =
                crate::layout::capture_composition_range(&tree, work_start, full_range.clone())?;
            let mut text = tree
                .slice(capture.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if flow {
                text = text.replace('\n', " ");
            }
            let indentation_tree = (checkpoint.is_none()
                && self.document.format().is_code()
                && composed_layout.wrap()
                && work_end < full_range.end)
                .then(|| tree.clone());
            let mut style_capture = capture.clone();
            if let Some(tree) = &indentation_tree {
                let indentation_end = crate::layout::ascii_indentation_end(
                    tree,
                    full_range.clone(),
                    &cancellation,
                )?;
                style_capture.end = style_capture.end.max(indentation_end);
            }
            let following_base =
                (work_end == full_range.end && line_index + 1 < line_count).then(|| {
                    self.document
                        .projection()
                        .presentation_line_range(line_index + 1, flow)
                        .unwrap()
                });
            let following = following_base
                .as_ref()
                .map(|range| map_base_range_to_overlay(&overlay, range))
                .transpose()?;
            let style_end = following_base
                .as_ref()
                .map(|range| {
                    self.document
                        .projection()
                        .text_tree()
                        .next_grapheme_boundary(range.start)
                        .map(|next| next.unwrap_or(range.start).min(range.end))
                })
                .transpose()
                .map_err(DocumentError::FormattedTextStorage)?
                .unwrap_or_else(|| to_base(style_capture.end, true));
            let mut styles = DocumentLayoutStyles::resolve_region_with_search(
                self.document.projection(),
                to_base(capture.start, false)..style_end,
                flow,
                self.views[&view_id].layout.search_matches(self.document.id(), self.document.revision()),
            )
            .map_err(LayoutError::from)?;
            if flow {
                let mut ranges = vec![base_range.clone()];
                ranges.extend(following_base.clone());
                crate::layout::resolve_flow_paragraph_styles(
                    self.document.projection(),
                    &mut styles,
                    &ranges,
                )
                .map_err(LayoutError::from)?;
            }
            let mut styles = composition_layout_styles(styles, &overlay, affinity)?;
            if flow {
                let mut ranges = vec![full_range.clone()];
                ranges.extend(following.clone());
                crate::layout::flow_paragraph_styles(&mut styles, &ranges);
            }
            let captured_view = composed_layout.capture_for_regional_layout_job(style_capture);
            let job_id = self.allocate_layout_job_id()?;
            if !composed_layout.begin_layout_job(job_id) {
                return Err(CoreError::IdentifierExhausted(
                    CoreIdentifierKind::LayoutJob,
                ));
            }
            let view = self
                .views
                .get_mut(&view_id)
                .ok_or(CoreError::UnknownView(view_id))?;
            let region = view
                .engine
                .layout_hard_line_slices_cancellable(
                    self.document.id(),
                    self.document.revision(),
                    &text,
                    capture.start,
                    &[crate::layout::HardLineLayoutSlice {
                        indentation_tree,
                        full_range: full_range.clone(),
                        work_range: work_start..work_end,
                        shaping_context_range: capture,
                        hard_line_index: line_index,
                        checkpoint,
                    }],
                    line_count,
                    overlay.utf8_len(),
                    following,
                    &styles,
                    &captured_view,
                    &cancellation,
                )
                .map_err(|error| match error {
                    LayoutComputationError::Layout(error) => CoreError::Layout(error),
                    LayoutComputationError::Cancelled => {
                        CoreError::LayoutJob(LayoutJobError::Cancelled)
                    }
                })?;
            let coverage = region.lines()[0].text_coverage();
            if coverage.start <= focus && focus <= coverage.end {
                let top = composed_layout.viewport_top();
                composed_layout
                    .publish_layout_job_viewport(job_id, region, top)
                    .map_err(LayoutError::from)?;
                if reveal_selection {
                    viewport::reveal_layout_row_at(
                        &mut composed_layout,
                        focus,
                        BoundaryAffinity::Downstream,
                    )?;
                }
                view.composition_layout = Some(composed_layout);
                return Ok(());
            }
            checkpoint = Some(
                region
                    .next_long_line_checkpoint()
                    .cloned()
                    .filter(|next| next.next_text_offset() > work_start)
                    .ok_or(LayoutJobError::InvalidLongLineCheckpoint(
                        "composition continuation did not advance toward its caret",
                    ))?,
            );
        }
    }

    fn rematerialize_active_composition(
        &mut self,
        view_id: ViewId,
        reveal_selection: bool,
    ) -> Result<(), CoreError> {
        self.refresh_completion_preview_policy(view_id)?;
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| view.composition.is_some() || view.completion.as_ref().is_some_and(|session| session.has_inline_preview()))
        {
            self.materialize_composition_layout(view_id, reveal_selection)?;
        }
        Ok(())
    }

    fn place_cursor(
        &mut self,
        view_id: ViewId,
        document_revision: Revision,
        text_offset: usize,
        affinity: BoundaryAffinity,
        extend_selection: bool,
    ) -> Result<CoreOutcome, CoreError> {
        if document_revision != self.document.revision() {
            return Err(CoreError::Document(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: document_revision,
            }));
        }
        self.document.text_point(text_offset)?;

        let mut composition_changes = Vec::new();
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| view.composition.is_some())
        {
            let cancelled = self.handle_composition_event(view_id, CompositionEvent::Cancel)?;
            composition_changes.extend(cancelled.composition_changes);
        }

        // Pointer motion is an explicit Insert/Replace undo break. Capture the
        // final pre-motion caret from the view that owns the open unit before
        // moving the invoking view.
        self.finalize_open_edit_group(view_id)?;

        let placed = self
            .views
            .get_mut(&view_id)
            .expect("view existence checked above")
            .commands
            .set_cursor_from_pointer(&self.document, text_offset, affinity, extend_selection);
        debug_assert!(placed, "the boundary was validated before placement");

        // An on-screen pointer move changes selection, not text geometry. Keep
        // the exact snapshot when its caret row and viewport are already fully
        // materialized. Distant/partial long-line destinations retain the normal
        // materialization path, including its checkpoint and viewport policy.
        self.poll_syntax();
        self.synchronize_whitespace(view_id)?;
        let reuse_visible = {
            let view = &self.views[&view_id];
            let requirements = inspect_layout_provider(&view.engine);
            current_snapshot_for_layout(&self.document, &view.layout, requirements)
                .is_some_and(|snapshot| {
                    !snapshot.has_horizontal_materialization()
                        && viewport_extension_needed(view) == (false, false)
                        && viewport::capture_caret_baseline_anchor(&self.document, view)
                            .and_then(|anchor| viewport::anchor_geometry(snapshot, anchor).ok())
                            .is_some_and(|geometry| {
                                let bounds = snapshot.rows[geometry.row_index].reveal_bounds();
                                bounds.start >= view.layout.viewport_top()
                                    && bounds.end <= view.layout.viewport_top() + view.layout.height()
                            })
                })
        };
        if reuse_visible {
            let view = self.views.get_mut(&view_id).expect("validated view");
            viewport::reveal_presentation_caret_row(&self.document, view)?;
            update_viewport_anchor(&self.document, view);
        } else {
            self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            // Even if row geometry was reusable, the old and new caret and
            // selection regions need presentation invalidation.
            layout_changed: true,
            composition_changes,
        })
    }

    fn select_all(
        &mut self,
        view_id: ViewId,
        document: DocumentId,
        revision: Revision,
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
        let mut composition_changes = Vec::new();
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| view.composition.is_some())
        {
            let cancelled = self.handle_composition_event(view_id, CompositionEvent::Cancel)?;
            composition_changes.extend(cancelled.composition_changes);
        }
        // The dedicated input intention is recordable and replayable without
        // depending on the original document's final visual row or byte length.
        let mut outcome = self.handle(view_id, CoreEvent::Input(InputEvent::Key(Key::SelectAll)))?;
        outcome.composition_changes.extend(composition_changes);
        Ok(outcome)
    }

    fn go_to_line(
        &mut self,
        view_id: ViewId,
        document: DocumentId,
        revision: Revision,
        line: u64,
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
        let mut composition_changes = Vec::new();
        if self.views[&view_id].composition.is_some() {
            let cancelled = self.handle_composition_event(view_id, CompositionEvent::Cancel)?;
            composition_changes.extend(cancelled.composition_changes);
        }
        self.finalize_open_edit_group(view_id)?;
        let command = self
            .views
            .get_mut(&view_id)
            .expect("view checked")
            .commands
            .go_to_line(&self.document, line);
        self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
        Ok(CoreOutcome {
            command: Some(command),
            document_changed: false,
            position_map: None,
            layout_changed: true,
            composition_changes,
        })
    }

    fn validate_style_sheet_identity(
        &self,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
    ) -> Result<(), CoreError> {
        if document != self.document.id() {
            return Err(CoreError::Document(DocumentError::WrongDocument));
        }
        if revision != self.document.revision() {
            return Err(CoreError::Document(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }));
        }
        let actual_sheet_revision = self.document.projection().style_sheet().revision;
        if style_sheet_revision != actual_sheet_revision {
            return Err(CoreError::StaleStyleSheet {
                expected: style_sheet_revision,
                actual: actual_sheet_revision,
            });
        }
        Ok(())
    }

    fn validate_style_edit_group(
        &self,
        view_id: ViewId,
        group: StyleEditGroup,
    ) -> Result<(), CoreError> {
        if !self.views.contains_key(&view_id) {
            return Err(CoreError::UnknownView(view_id));
        }
        let Some(open) = self.style_edit_group.as_ref() else {
            return Err(CoreError::StyleEditGroup(
                StyleEditGroupError::NoActiveGroup,
            ));
        };
        if group.id != open.identity.id {
            return Err(CoreError::StyleEditGroup(StyleEditGroupError::WrongGroup {
                expected: open.identity.id,
                actual: group.id,
            }));
        }
        if view_id != open.identity.view {
            return Err(CoreError::StyleEditGroup(StyleEditGroupError::WrongOwner {
                expected: open.identity.view,
                actual: view_id,
            }));
        }
        if group.view != open.identity.view {
            return Err(CoreError::StyleEditGroup(StyleEditGroupError::WrongOwner {
                expected: open.identity.view,
                actual: group.view,
            }));
        }
        if group.document != open.identity.document {
            return Err(CoreError::StyleEditGroup(
                StyleEditGroupError::WrongDocument {
                    expected: open.identity.document,
                    actual: group.document,
                },
            ));
        }
        if group.begin_document_revision != open.identity.begin_document_revision
            || group.begin_style_sheet_revision != open.identity.begin_style_sheet_revision
            || self.document.edit_group_depth() == 0
            || self.document.edit_group_generation() != open.generation
        {
            return Err(CoreError::StyleEditGroup(
                StyleEditGroupError::IdentityMismatch,
            ));
        }
        Ok(())
    }

    /// Consume the active explicit style capability and close its document
    /// history group. The capability is taken before any fallible restoration
    /// work, and the model group is always closed: end/failure never rolls
    /// back or poisons the successfully committed prefix.
    fn finalize_style_edit_group(&mut self) -> Result<bool, CoreError> {
        let Some(open) = self.style_edit_group.take() else {
            return Ok(false);
        };
        let current = self.document.history_status().current;
        let restoration = if current != open.parent {
            match self.views.get(&open.identity.view) {
                Some(view) => match view.commands.capture_history_restoration(&self.document) {
                    Ok(after) => self
                        .document
                        .attach_history_restoration(
                            current.node,
                            HistoryRestoration::new(open.before, after),
                        )
                        .map_err(CoreError::from),
                    Err(error) => Err(CoreError::from(error)),
                },
                None => Err(CoreError::UnknownView(open.identity.view)),
            }
        } else {
            Ok(())
        };
        self.document.close_edit_group();
        restoration?;
        Ok(true)
    }

    /// Finalize the active Insert/Replace undo unit at a host-driven edit-group
    /// boundary. The unit's last cursor/mark restoration belongs to the view
    /// that opened it, even when a different view invokes the boundary.
    fn finalize_open_edit_group(&mut self, invoking_view: ViewId) -> Result<(), CoreError> {
        let owner = self.edit_group_owner.unwrap_or(invoking_view);
        if let Some(open) = self.edit_group_restoration.as_ref() {
            let current = self.document.history_status().current;
            if current != open.parent {
                let after = self
                    .views
                    .get(&owner)
                    .ok_or(CoreError::UnknownView(owner))?
                    .commands
                    .capture_history_restoration(&self.document)?;
                self.document.attach_history_restoration(
                    current.node,
                    HistoryRestoration::new(open.before.clone(), after),
                )?;
            }
        }
        if self.edit_group_owner.is_some() {
            self.document.close_edit_group();
        }
        self.edit_group_owner = None;
        self.edit_group_restoration = None;
        Ok(())
    }

    fn pending_typing_outcome(
        &mut self,
        view_id: ViewId,
        previous_cursor: usize,
    ) -> Result<CoreOutcome, CoreError> {
        let moved = self
            .views
            .get(&view_id)
            .expect("view checked")
            .commands
            .cursor()
            != previous_cursor;
        if moved {
            self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            layout_changed: moved,
            composition_changes: Vec::new(),
        })
    }

    /// Prepare all controller images before a native transaction commits. The
    /// caller chooses ordinary or format-conversion anchor association; every
    /// view must map successfully before any live controller is replaced.
    fn prepare_mapped_commands(
        &self,
        map: &PositionMap,
        capture_anchors: fn(
            &CommandInterpreter,
            &Document,
        ) -> Result<crate::command::CommandPositionAnchors, DocumentError>,
    ) -> Result<BTreeMap<ViewId, CommandInterpreter>, CoreError> {
        let anchors = self
            .views
            .iter()
            .map(|(id, view)| {
                capture_anchors(&view.commands, &self.document).map(|anchors| (*id, anchors))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut next_commands = self
            .views
            .iter()
            .map(|(id, view)| (*id, view.commands.clone()))
            .collect::<BTreeMap<_, _>>();
        for (id, captured) in &anchors {
            let commands = next_commands
                .get_mut(id)
                .expect("captured view remains attached during serial dispatch");
            if !commands.apply_position_map(captured, map)? {
                return Err(CoreError::Position(PositionError::WrongSnapshot {
                    expected: self.document.revision(),
                    actual: captured.revision(),
                }));
            }
        }
        Ok(next_commands)
    }

    /// Publish native model changes to viewport and composition state only
    /// after the prepared document and command images have been installed.
    fn refresh_views_after_native_change(
        &mut self,
        view_id: ViewId,
        map: &PositionMap,
    ) -> Result<Vec<ViewCompositionChange>, CoreError> {
        self.cancel_all_active_layout_work();
        self.rebase_viewport_anchors(map)?;
        self.publish_buffer_commands(view_id);
        let current_revision = self.document.revision();
        let mut composition_changes = Vec::new();
        for (id, view) in &mut self.views {
            if let Some(session) = view.composition.take() {
                view.composition_layout = None;
                composition_changes.push(ViewCompositionChange {
                    view: *id,
                    outcome: ViewCompositionOutcome::Invalidated {
                        reason: CompositionCancelReason::ExternalDocumentChange,
                        base_revision: session.base_revision(),
                        current_revision,
                    },
                });
            }
        }
        self.materialize_views_after_document_change(
            view_id,
            ImmediateLayoutIntent::PreserveViewport,
        );
        Ok(composition_changes)
    }

    /// Publish one source-backed semantic inline-style change as a standalone
    /// undo unit. Selection identity and model capability are validated before
    /// an existing Insert/Replace unit is closed, so rejection is atomic with
    /// respect to both document state and undo grouping.
    fn set_selection_semantic_style(
        &mut self,
        view_id: ViewId,
        expected: LogicalSelectionIdentity,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<CoreOutcome, CoreError> {
        if expected.kind() == LogicalSelectionKind::None {
            if self.list_selection_identity(view_id)? != expected {
                return Err(CoreError::StaleLogicalSelection);
            }
            let values = match style {
                SemanticInlineStyle::Strong => vec![(
                    StyleProperty::CharacterBold,
                    crate::document::StylePropertyValue::Boolean(enabled),
                )],
                SemanticInlineStyle::Emphasis => vec![(
                    StyleProperty::CharacterSlant,
                    crate::document::StylePropertyValue::FontSlant(if enabled {
                        crate::document::FontSlant::Italic
                    } else {
                        crate::document::FontSlant::Upright
                    }),
                )],
                _ => return Err(CoreError::Document(DocumentError::UnsupportedFormatting)),
            };
            let previous_cursor = self
                .views
                .get(&view_id)
                .expect("view checked")
                .commands
                .cursor();
            self.views
                .get_mut(&view_id)
                .expect("view checked")
                .commands
                .set_typing_properties(&self.document, values)?;
            return self.pending_typing_outcome(view_id, previous_cursor);
        }
        let current = self
            .active_linear_selection_identity(view_id)?
            .ok_or(CoreError::StaleLogicalSelection)?;
        if current != expected {
            return Err(CoreError::StaleLogicalSelection);
        }

        let request = || ModelRequest::SetSemanticStyle {
            document: expected.document(),
            revision: expected.revision(),
            range: expected.range(),
            style,
            enabled,
        };
        let preflight = self
            .document
            .prepare_model_request(request())
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

        let before_revision = self.document.revision();
        let expected_map = preflight.text_position_map().clone();
        let before_restoration = self
            .views
            .get(&view_id)
            .expect("view existence checked before native semantic style change")
            .commands
            .capture_history_restoration(&self.document)?;
        let next_commands = self.prepare_mapped_commands(
            &expected_map,
            CommandInterpreter::capture_position_anchors,
        )?;
        drop(preflight);

        self.finalize_open_edit_group(view_id)?;
        let prepared = self
            .document
            .prepare_model_request(request())
            .map_err(command_model_transaction_error)?;
        debug_assert!(!prepared.is_no_op());
        debug_assert_eq!(prepared.text_position_map(), &expected_map);
        let committed = self
            .document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        debug_assert_eq!(committed.before_revision(), before_revision);
        debug_assert_eq!(committed.text_position_map(), &expected_map);
        let changed = committed.after_revision() != committed.before_revision();
        debug_assert!(changed);

        for (id, commands) in next_commands {
            self.views
                .get_mut(&id)
                .expect("prepared view remains attached during serial dispatch")
                .commands = commands;
        }
        let after_restoration = self
            .views
            .get(&view_id)
            .expect("invoking view remains attached after semantic style change")
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before_restoration, after_restoration),
        )?;

        let composition_changes = self.refresh_views_after_native_change(view_id, &expected_map)?;
        Ok(CoreOutcome {
            command: None,
            document_changed: changed,
            position_map: Some(expected_map),
            layout_changed: true,
            composition_changes,
        })
    }

    /// Publish a native file-format change as a standalone source transaction.
    ///
    /// Preparation is deliberately performed once before closing any open
    /// Insert/Replace group. A policy rejection, stale identity, or other
    /// preparation failure therefore changes neither the document nor
    /// controller grouping. Once preparation succeeds, the prior owner gets
    /// its exact restoration endpoint. The verified candidate is then rebound
    /// only to the closed group, preserving all source/revision preconditions.
    fn apply_native_model_request(
        &mut self,
        view_id: ViewId,
        request: ModelRequest,
    ) -> Result<CoreOutcome, CoreError> {
        let quote_at_end = match &request {
            ModelRequest::AssignNamedStyle { range, namespace: StyleNamespace::Block, style, .. }
            | ModelRequest::SetParagraphStyle { range, style, .. }
                if range.is_empty() && style.0 == "Block quote"
                    && self.list_selection_identity(view_id)?.kind() == LogicalSelectionKind::None =>
            {
                self.document.prepare_quote_after_paragraph(range.start)
                    .map_err(command_model_transaction_error)?
            }
            _ => None,
        };
        let preflight = match quote_at_end {
            Some(prepared) => prepared,
            None => self.document.prepare_model_request(request.clone())
                .map_err(command_model_transaction_error)?,
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

        let before_revision = self.document.revision();
        let expected_map = preflight.text_position_map().clone();
        let before_restoration = self
            .views
            .get(&view_id)
            .expect("view existence checked before native file-format change")
            .commands
            .capture_history_restoration(&self.document)?;
        let capture_anchors = if matches!(request, ModelRequest::SetFormat { .. }) {
            CommandInterpreter::capture_format_position_anchors
        } else {
            CommandInterpreter::capture_position_anchors
        };
        let next_commands = self.prepare_mapped_commands(&expected_map, capture_anchors)?;
        // Preserve the restoration endpoint of the view which owns an open
        // edit group, even when another view invoked this native operation.
        self.finalize_open_edit_group(view_id)?;
        let prepared = self
            .document
            .rebind_prepared_after_group_close(preflight)
            .map_err(command_model_transaction_error)?;
        debug_assert!(!prepared.is_no_op());
        debug_assert_eq!(prepared.text_position_map(), &expected_map);
        let committed = self
            .document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        debug_assert_eq!(committed.before_revision(), before_revision);
        debug_assert_eq!(committed.text_position_map(), &expected_map);
        let changed = committed.after_revision() != committed.before_revision();
        debug_assert!(changed);

        for (id, mut commands) in next_commands {
            if self.document.format() == Format::Rtf
                && commands.line_mode() == crate::command::LineMode::PhysicalSource
            {
                commands.set_line_mode(&self.document, crate::command::LineMode::Visual)?;
            }
            self.views
                .get_mut(&id)
                .expect("prepared view remains attached during serial dispatch")
                .commands = commands;
        }
        let after_restoration = self
            .views
            .get(&view_id)
            .expect("invoking view remains attached after file-format change")
            .commands
            .capture_history_restoration(&self.document)?;
        let history_after = self.document.history_status().current;
        self.document.attach_history_restoration(
            history_after.node,
            HistoryRestoration::new(before_restoration, after_restoration),
        )?;

        let composition_changes = self.refresh_views_after_native_change(view_id, &expected_map)?;
        let warnings = committed.summary().conversion_warnings();
        let command = if warnings.is_empty() {
            None
        } else {
            let mut ex = crate::command::ex_execute::ExOutcome::default();
            ex.frontend_requests = warnings
                .iter()
                .map(|warning| {
                    crate::command::ex_execute::ExFrontendRequest::Info(
                        crate::command::ex_execute::ExInfoRequest::Message(warning.message()),
                    )
                })
                .collect();
            Some(CommandOutput {
                status: CommandStatus::Complete,
                cursor_moved: false,
                document_changed: changed,
                mode_changed: false,
                history_navigation: false,
                ex_outcome: Some(ex),
                clipboard_writes: Vec::new(),
            })
        };
        Ok(CoreOutcome {
            command,
            document_changed: changed,
            position_map: Some(expected_map),
            layout_changed: true,
            composition_changes,
        })
    }

    fn generated_style_model_request(
        &self,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: &StyleId,
        edit: &StyleDefinitionFieldEdit,
    ) -> Result<StyleModelRequest, CoreError> {
        self.validate_style_sheet_identity(document, revision, style_sheet_revision)?;
        let sheet = self.document.projection().style_sheet();
        let origin = match namespace {
            StyleNamespace::Block => sheet.block_style_metadata(style),
            StyleNamespace::Character => sheet.character_style_metadata(style),
        }
        .map(|metadata| metadata.origin);
        if self.document.format().has_rich_source()
            && origin == Some(crate::document::StyleDefinitionOrigin::SourceBacked)
        {
            let definition_edit = sheet
                .prepare_source_field_edit(namespace, style, edit)
                .map_err(ModelTransactionError::from)
                .map_err(command_model_transaction_error)?;
            return Ok(StyleModelRequest::new(
                document,
                revision,
                StyleModelIntent::Persisted(
                    crate::document::PersistedStyleIntent::EditStyleDefinition {
                        origin: crate::document::StyleDefinitionOrigin::SourceBacked,
                        edit: definition_edit,
                    },
                ),
            ));
        }
        let definition_edit = self
            .document
            .projection()
            .style_sheet()
            .prepare_generated_field_edit(namespace, style, edit)
            .map_err(ModelTransactionError::from)
            .map_err(command_model_transaction_error)?;
        Ok(StyleModelRequest::new(
            document,
            revision,
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                definition_edit,
            )),
        ))
    }

    /// Publish one exact, core-authorized generated-style field edit as a
    /// standalone history unit while preserving every view's logical and
    /// viewport anchors across the style-only revision transition.
    #[allow(clippy::too_many_arguments)]
    fn edit_generated_style(
        &mut self,
        view_id: ViewId,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: StyleId,
        edit: StyleDefinitionFieldEdit,
    ) -> Result<CoreOutcome, CoreError> {
        self.edit_generated_style_with_history(
            view_id,
            document,
            revision,
            style_sheet_revision,
            namespace,
            style,
            edit,
            false,
        )
    }

    /// Commit one exact style edit into an explicitly owned live group. Each
    /// successful call is immediately visible in every view, while the
    /// document history composes all successful calls into the group's single
    /// undo node. A rejected or no-op edit leaves the group usable.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_generated_style_in_group(
        &mut self,
        view_id: ViewId,
        group: StyleEditGroup,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: StyleId,
        edit: StyleDefinitionFieldEdit,
    ) -> Result<CoreOutcome, CoreError> {
        self.validate_style_edit_group(view_id, group)?;
        self.edit_generated_style_with_history(
            view_id,
            document,
            revision,
            style_sheet_revision,
            namespace,
            style,
            edit,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_generated_style_with_history(
        &mut self,
        view_id: ViewId,
        document: DocumentId,
        revision: Revision,
        style_sheet_revision: StyleSheetRevision,
        namespace: StyleNamespace,
        style: StyleId,
        edit: StyleDefinitionFieldEdit,
        grouped: bool,
    ) -> Result<CoreOutcome, CoreError> {
        let request = self.generated_style_model_request(
            document,
            revision,
            style_sheet_revision,
            namespace,
            &style,
            &edit,
        )?;
        let preflight = self
            .document
            .prepare_style_request(request)
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

        let expected_map = preflight.text_position_map().clone();
        let before_restoration = if grouped {
            None
        } else {
            Some(
                self.views
                    .get(&view_id)
                    .expect("view existence checked before native style edit")
                    .commands
                    .capture_history_restoration(&self.document)?,
            )
        };
        let next_commands = self.prepare_mapped_commands(
            &expected_map,
            CommandInterpreter::capture_position_anchors,
        )?;
        let prepared = if grouped {
            preflight
        } else {
            drop(preflight);

            // As with native history and file-format changes, preserve the
            // true owner endpoint of any Insert/Replace group before creating
            // this standalone semantic unit.
            self.finalize_open_edit_group(view_id)?;
            let request = self.generated_style_model_request(
                document,
                revision,
                style_sheet_revision,
                namespace,
                &style,
                &edit,
            )?;
            self.document
                .prepare_style_request(request)
                .map_err(command_model_transaction_error)?
        };
        debug_assert!(!prepared.is_no_op());
        debug_assert_eq!(prepared.text_position_map(), &expected_map);
        let committed = self
            .document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        let changed = committed.after_revision() != committed.before_revision();
        debug_assert!(changed);

        for (id, commands) in next_commands {
            self.views
                .get_mut(&id)
                .expect("prepared view remains attached during serial dispatch")
                .commands = commands;
        }
        if let Some(before_restoration) = before_restoration {
            let after_restoration = self
                .views
                .get(&view_id)
                .expect("invoking view remains attached after style edit")
                .commands
                .capture_history_restoration(&self.document)?;
            self.document.attach_history_restoration(
                self.document.history_status().current.node,
                HistoryRestoration::new(before_restoration, after_restoration),
            )?;
        }

        let composition_changes = self.refresh_views_after_native_change(view_id, &expected_map)?;
        Ok(CoreOutcome {
            command: None,
            document_changed: changed,
            position_map: Some(expected_map),
            layout_changed: true,
            composition_changes,
        })
    }

    fn navigate_history(
        &mut self,
        view_id: ViewId,
        navigation: HistoryNavigationRequest,
    ) -> Result<CoreOutcome, CoreError> {
        let mut composition_changes = Vec::new();
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| view.composition.is_some())
        {
            let cancelled = self.handle_composition_event(view_id, CompositionEvent::Cancel)?;
            composition_changes.extend(cancelled.composition_changes);
        }

        self.finalize_open_edit_group(view_id)?;
        let available = match navigation {
            HistoryNavigationRequest::Undo => self.document.history_status().can_undo,
            HistoryNavigationRequest::Redo => self.document.history_status().can_redo,
            HistoryNavigationRequest::SelectNode(_) | HistoryNavigationRequest::SelectChange(_) => {
                true
            }
        };
        if !available {
            let message = match navigation {
                HistoryNavigationRequest::Undo => "already at the oldest document state",
                HistoryNavigationRequest::Redo => "no preferred redo state is available",
                HistoryNavigationRequest::SelectNode(_)
                | HistoryNavigationRequest::SelectChange(_) => {
                    unreachable!("exact history selections are validated by the model")
                }
            };
            return Ok(CoreOutcome {
                command: Some(CommandOutput {
                    status: CommandStatus::Error(message.to_owned()),
                    cursor_moved: false,
                    document_changed: false,
                    mode_changed: false,
                    history_navigation: false,
                    ex_outcome: None,
                    clipboard_writes: Vec::new(),
                }),
                document_changed: false,
                position_map: None,
                layout_changed: !composition_changes.is_empty(),
                composition_changes,
            });
        }

        let before_revision = self.document.revision();
        let history_before = self.document.history_status().current;
        let (old_cursor, old_mode) = {
            let commands = &self
                .views
                .get(&view_id)
                .expect("view existence checked before native history navigation")
                .commands;
            (commands.cursor(), commands.mode())
        };
        let invoking_positions = self.views[&view_id].commands.capture_position_anchors(&self.document)?;
        let inactive_positions = self
            .views
            .iter()
            .filter(|(id, _)| **id != view_id)
            .map(|(id, view)| {
                view.commands
                    .capture_position_anchors(&self.document)
                    .map(|anchors| (*id, anchors))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let request = ModelRequest::NavigateHistory {
            document: self.document.id(),
            revision: before_revision,
            navigation,
        };
        let prepared = self
            .document
            .prepare_model_request(request)
            .map_err(command_model_transaction_error)?;
        let committed = self
            .document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        let changed = committed.after_revision() != committed.before_revision();
        let map = committed.text_position_map().clone();
        let history_after = self.document.history_status().current;
        let restoration = self
            .document
            .history_restoration_between(history_before.node, history_after.node)?
            .expect("a successful history navigation traverses a recorded edge");

        // Prepare every controller image before publishing any of them. A
        // map/restoration invariant failure cannot leave only some views on
        // the newly installed document revision.
        let mut invoking_commands = self
            .views
            .get(&view_id)
            .expect("invoking view remains attached")
            .commands
            .clone();
        // History records restore the cursor and named marks. Rebase the
        // remaining per-view state (notably gv memory and jumps) first, just
        // as for inactive views, so it cannot retain pre-Undo offsets.
        if !invoking_commands.apply_position_map(&invoking_positions, &map)? {
            return Err(CoreError::Position(PositionError::WrongSnapshot {
                expected: before_revision,
                actual: invoking_positions.revision(),
            }));
        }
        invoking_commands.apply_history_restoration(&self.document, &restoration)?;
        let mut rebased = Vec::with_capacity(inactive_positions.len());
        for (id, anchors) in inactive_positions {
            let mut commands = self
                .views
                .get(&id)
                .expect("captured view remains attached")
                .commands
                .clone();
            if !commands.apply_position_map(&anchors, &map)? {
                return Err(CoreError::Position(PositionError::WrongSnapshot {
                    expected: before_revision,
                    actual: anchors.revision(),
                }));
            }
            rebased.push((id, commands));
        }
        self.views
            .get_mut(&view_id)
            .expect("invoking view remains attached")
            .commands = invoking_commands;
        for (id, commands) in rebased {
            self.views
                .get_mut(&id)
                .expect("captured view remains attached")
                .commands = commands;
        }

        // Native history actions bypass ordinary input dispatch. Publish the
        // restored buffer marks too, or the next key/new view reinstalls stale
        // offsets from before Undo (which can now be past the document end).
        self.publish_buffer_commands(view_id);
        self.replay_undo_floor = None;
        self.cancel_all_active_layout_work();
        self.rebase_viewport_anchors(&map)?;
        let revision = self.document.revision();
        for (id, view) in &mut self.views {
            if *id == view_id {
                continue;
            }
            if let Some(session) = view.composition.take() {
                view.composition_layout = None;
                composition_changes.push(ViewCompositionChange {
                    view: *id,
                    outcome: ViewCompositionOutcome::Invalidated {
                        reason: CompositionCancelReason::ExternalDocumentChange,
                        base_revision: session.base_revision(),
                        current_revision: revision,
                    },
                });
            }
        }
        self.materialize_views_after_document_change(view_id, ImmediateLayoutIntent::RevealCaret);
        let commands = &self
            .views
            .get(&view_id)
            .expect("invoking view remains attached")
            .commands;
        Ok(CoreOutcome {
            command: Some(CommandOutput {
                status: CommandStatus::Complete,
                cursor_moved: commands.cursor() != old_cursor,
                document_changed: changed,
                mode_changed: commands.mode() != old_mode,
                history_navigation: true,
                ex_outcome: None,
                clipboard_writes: Vec::new(),
            }),
            document_changed: changed,
            position_map: Some(map),
            layout_changed: true,
            composition_changes,
        })
    }

    fn begin_input_publication(&mut self, invoking: ViewId) -> InputPublicationCheckpoint {
        InputPublicationCheckpoint {
            document: self.document.begin_command_checkpoint(),
            commands: self.views[&invoking].commands.clone(),
            views: self
                .views
                .iter()
                .map(|(id, view)| {
                    (
                        *id,
                        InputViewCheckpoint {
                            viewport_anchor: view.viewport_anchor,
                            viewport_left: view.layout.viewport_left(),
                            viewport_top: view.layout.viewport_top(),
                            wrap: view.layout.wrap(),
                        },
                    )
                })
                .collect(),
            edit_group_owner: self.edit_group_owner,
            edit_group_restoration: self.edit_group_restoration.clone(),
            replay_undo_floor: self.replay_undo_floor,
        }
    }

    fn rollback_input_publication(
        &mut self,
        invoking: ViewId,
        checkpoint: InputPublicationCheckpoint,
        failed: bool,
    ) {
        self.document
            .rollback_command_checkpoint(checkpoint.document);
        let commands = &mut self
            .views
            .get_mut(&invoking)
            .expect("input does not detach views")
            .commands;
        if failed {
            commands.restore_failed_command(checkpoint.commands);
        } else {
            *commands = checkpoint.commands;
        }
        self.edit_group_owner = checkpoint.edit_group_owner;
        self.edit_group_restoration = checkpoint.edit_group_restoration;
        self.replay_undo_floor = checkpoint.replay_undo_floor;
        for (id, saved) in checkpoint.views {
            let view = self
                .views
                .get_mut(&id)
                .expect("input does not detach views");
            view.viewport_anchor = saved.viewport_anchor;
            view.layout.set_wrap(saved.wrap);
            view.layout
                .set_viewport_top(saved.viewport_top)
                .expect("captured finite viewport");
            view.layout
                .set_viewport_left(saved.viewport_left)
                .expect("captured finite viewport");
            // NeedsMoreLayout rolls back an uncommitted input while retaining
            // its unchanged geometry. Keep validated wrap checkpoints so a
            // retry can continue locally; failed-edit revisions or restored
            // configuration changes still retire incompatible checkpoints.
            let requirements = inspect_layout_provider(&view.engine);
            view.long_line_checkpoints.discard_stale(&self.document, &view.layout, requirements);
        }
    }

    pub fn handle(&mut self, view_id: ViewId, event: CoreEvent) -> Result<CoreOutcome, CoreError> {
        let mut outcome = self.handle_without_search_presentation(view_id, event)?;
        // Search is disposable presentation work. A failed bounded query or
        // shaper cannot turn a committed edit into a failed command.
        match self.poll_search(view_id) {
            Ok(changed) => outcome.layout_changed |= changed,
            Err(error) => self.record_presentation_error(view_id, error),
        }
        Ok(outcome)
    }

    fn handle_without_search_presentation(&mut self, view_id: ViewId, event: CoreEvent) -> Result<CoreOutcome, CoreError> {
        if !self.views.contains_key(&view_id) {
            return Err(CoreError::UnknownView(view_id));
        }
        self.retire_stale_completion(view_id)?;
        if self.views[&view_id].completion.is_some() {
            if let CoreEvent::Composition(CompositionEvent::Begin(target)) = &event {
                return self.begin_composition_after_completion(view_id, target.clone());
            }
            if !matches!(&event, CoreEvent::Input(_) | CoreEvent::InputWithClipboard { .. }
                | CoreEvent::Resize { .. } | CoreEvent::SetScale(_) | CoreEvent::SetWrap(_)
                | CoreEvent::SetParagraphFlow(_) | CoreEvent::SetViewportOrigin { .. }
                | CoreEvent::RevealSelection) {
                self.discard_completion(view_id);
            }
        }
        // An explicit style gesture admits only its dedicated grouped-edit and
        // end APIs. Any ordinary coordinator event is an unambiguous boundary:
        // close the successful prefix, consume the token, then process the
        // event. Frontends may therefore recover from a lost end notification
        // without risking later commands joining the style undo unit.
        self.finalize_style_edit_group()?;
        let event = match event {
            CoreEvent::FlushMappingPrefix | CoreEvent::FlushMappingPrefixWithClipboard(_) => {
                let clipboard = match event { CoreEvent::FlushMappingPrefixWithClipboard(value) => Some(value), _ => None };
                self.install_buffer_commands(view_id);
                let plan = self.views.get_mut(&view_id).expect("validated view").commands.flush_mapping();
                let initial = CoreOutcome { command: Some(crate::command::mappings::empty_mapping_output()), document_changed: false, position_map: None, layout_changed: false, composition_changes: Vec::new() };
                let Some(plan) = plan else { return Ok(initial); };
                return self.run_compound_replay(view_id, clipboard, initial, ReplayPlan::Mapping(plan));
            }
            CoreEvent::ReadFile { document, revision, after, bytes } => {
                return self.complete_read_file(view_id, document, revision, after, &bytes);
            }
            CoreEvent::EditCommandLine(request) => {
                self.install_buffer_commands(view_id);
                let command = self
                    .views
                    .get_mut(&view_id)
                    .expect("view checked")
                    .commands
                    .edit_command_line(&self.document, request)?;
                self.publish_buffer_commands(view_id);
                return Ok(CoreOutcome {
                    command: Some(command),
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                });
            }
            CoreEvent::SetDirectCharacterProperties { expected, values } => {
                if expected.kind() == LogicalSelectionKind::None {
                    if self.list_selection_identity(view_id)? != expected {
                        return Err(CoreError::StaleLogicalSelection);
                    }
                    let previous_cursor = self
                        .views
                        .get(&view_id)
                        .expect("view checked")
                        .commands
                        .cursor();
                    self.views
                        .get_mut(&view_id)
                        .expect("view checked")
                        .commands
                        .set_typing_properties(&self.document, values)?;
                    return self.pending_typing_outcome(view_id, previous_cursor);
                }
                let actual = self
                    .active_linear_selection_identity(view_id)?
                    .ok_or(CoreError::StaleLogicalSelection)?;
                if actual != expected {
                    return Err(CoreError::StaleLogicalSelection);
                }
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetDirectCharacterProperties {
                        document: expected.document(),
                        revision: expected.revision(),
                        range: expected.range(),
                        values,
                    },
                );
            }
            CoreEvent::EditDirectProperties { expected, values } => {
                return self.edit_direct_properties(view_id, expected, values);
            }
            CoreEvent::EditDirectProperty {
                expected,
                property,
                value,
            } => {
                let character = crate::document::is_character_property(property);
                if character && expected.kind() == LogicalSelectionKind::None {
                    if self.list_selection_identity(view_id)? != expected {
                        return Err(CoreError::StaleLogicalSelection);
                    }
                    let commands =
                        &mut self.views.get_mut(&view_id).expect("view checked").commands;
                    if !matches!(commands.mode(), Mode::Insert | Mode::Replace) {
                        return Err(CoreError::Document(DocumentError::UnsupportedFormatting));
                    }
                    let previous_cursor = commands.cursor();
                    if let Some(value) = value {
                        commands.set_typing_properties(&self.document, vec![(property, value)])?;
                    } else {
                        commands.clear_typing_property(property);
                    }
                    return self.pending_typing_outcome(view_id, previous_cursor);
                }
                let actual = if character {
                    self.active_linear_selection_identity(view_id)?
                        .ok_or(CoreError::StaleLogicalSelection)?
                } else {
                    self.list_selection_identity(view_id)?
                };
                if actual != expected {
                    return Err(CoreError::StaleLogicalSelection);
                }
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::EditDirectProperty {
                        document: expected.document(),
                        revision: expected.revision(),
                        range: expected.range(),
                        property,
                        value,
                    },
                );
            }
            CoreEvent::Composition(event) => {
                return self.handle_composition_event(view_id, event);
            }
            CoreEvent::PlaceCursor {
                document_revision,
                text_offset,
                affinity,
                extend_selection,
            } => {
                return self.place_cursor(
                    view_id,
                    document_revision,
                    text_offset,
                    affinity,
                    extend_selection,
                );
            }
            CoreEvent::SelectAll { document, revision } => {
                return self.select_all(view_id, document, revision);
            }
            CoreEvent::GoToLine {
                document,
                revision,
                line,
            } => {
                return self.go_to_line(view_id, document, revision, line);
            }
            CoreEvent::NavigateHistory(navigation) => {
                return self.navigate_history(view_id, navigation);
            }
            CoreEvent::SetFileFormat {
                document,
                revision,
                target,
            } => {
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetFileFormat {
                        document,
                        revision,
                        target,
                    },
                );
            }
            CoreEvent::SetIncludeStyleDefinitionsInFile {
                document,
                revision,
                enabled,
            } => {
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetIncludeStyleDefinitionsInFile {
                        document,
                        revision,
                        enabled,
                    },
                );
            }
            CoreEvent::SetFormat {
                document,
                revision,
                target,
                operation,
            } => {
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetFormat {
                        document,
                        revision,
                        target,
                        operation,
                    },
                );
            }
            CoreEvent::SetEncoding {
                document,
                revision,
                target,
            } => {
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetEncoding {
                        document,
                        revision,
                        target,
                    },
                );
            }
            CoreEvent::IndentList { expected, unindent } => {
                if self.list_selection_identity(view_id)? != expected { return Err(CoreError::StaleLogicalSelection); }
                return self.apply_native_model_request(view_id, ModelRequest::IndentList {
                    document: expected.document(), revision: expected.revision(), range: expected.range(), unindent,
                });
            }
            CoreEvent::SetListStyle { expected, style } => {
                if self.list_selection_identity(view_id)? != expected {
                    return Err(CoreError::StaleLogicalSelection);
                }
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetListStyle {
                        document: expected.document(),
                        revision: expected.revision(),
                        range: expected.range(),
                        style,
                    },
                );
            }
            CoreEvent::SetParagraphStyle { expected, style } => {
                if self.list_selection_identity(view_id)? != expected {
                    return Err(CoreError::StaleLogicalSelection);
                }
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::SetParagraphStyle {
                        document: expected.document(),
                        revision: expected.revision(),
                        range: expected.range(),
                        style,
                    },
                );
            }
            CoreEvent::AssignNamedStyle {
                expected,
                style_sheet_revision,
                namespace,
                style,
            } => {
                let actual = if namespace == StyleNamespace::Character
                    && expected.kind() != LogicalSelectionKind::None
                {
                    self.active_linear_selection_identity(view_id)?
                        .ok_or(CoreError::StaleLogicalSelection)?
                } else {
                    self.list_selection_identity(view_id)?
                };
                if actual != expected {
                    return Err(CoreError::StaleLogicalSelection);
                }
                self.validate_style_sheet_identity(
                    expected.document(),
                    expected.revision(),
                    style_sheet_revision,
                )?;
                if namespace == StyleNamespace::Character
                    && expected.kind() == LogicalSelectionKind::None
                {
                    let commands =
                        &mut self.views.get_mut(&view_id).expect("view checked").commands;
                    let previous_cursor = commands.cursor();
                    commands.set_typing_named_style(&self.document, style)?;
                    return self.pending_typing_outcome(view_id, previous_cursor);
                }
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::AssignNamedStyle {
                        document: expected.document(),
                        revision: expected.revision(),
                        range: expected.range(),
                        namespace,
                        style,
                    },
                );
            }
            CoreEvent::EditNamedStyleDefinition {
                document,
                revision,
                style_sheet_revision,
                edit,
            } => {
                self.validate_style_sheet_identity(document, revision, style_sheet_revision)?;
                return self.apply_native_model_request(
                    view_id,
                    ModelRequest::EditNamedStyleDefinition {
                        document,
                        revision,
                        edit,
                    },
                );
            }
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style,
                enabled,
            } => {
                return self.set_selection_semantic_style(view_id, expected, style, enabled);
            }
            CoreEvent::EditGeneratedStyle {
                document,
                revision,
                style_sheet_revision,
                namespace,
                style,
                edit,
            } => {
                return self.edit_generated_style(
                    view_id,
                    document,
                    revision,
                    style_sheet_revision,
                    namespace,
                    style,
                    edit,
                );
            }
            event => event,
        };
        let (event, clipboard_context) = match event {
            CoreEvent::InputWithClipboard { input, clipboard } => {
                (CoreEvent::Input(input), Some(clipboard))
            }
            event => (event, None),
        };
        if let CoreEvent::Input(input) = &event {
            if let Some(outcome) = self.handle_completion_input(view_id, input, clipboard_context.as_ref())? {
                return Ok(outcome);
            }
        }
        if let CoreEvent::Input(input) = &event {
            if self
                .views
                .get(&view_id)
                .is_some_and(|view| view.composition.is_some())
            {
                let continues_with_history = matches!(
                    (
                        self.views
                            .get(&view_id)
                            .expect("view existence checked above")
                            .commands
                            .mode(),
                        input,
                    ),
                    (
                        Mode::Normal,
                        InputEvent::Key(Key::Char('u') | Key::Ctrl('r' | 'R'))
                    )
                );
                let mut cancellation = self.cancel_composition_for_input(view_id, input)?;
                if !continues_with_history {
                    return Ok(cancellation);
                }
                let continued_event = clipboard_context.map_or_else(
                    || event.clone(),
                    |clipboard| CoreEvent::InputWithClipboard {
                        input: input.clone(),
                        clipboard,
                    },
                );
                let mut continued = self.handle(view_id, continued_event)?;
                cancellation
                    .composition_changes
                    .append(&mut continued.composition_changes);
                continued.composition_changes = cancellation.composition_changes;
                continued.layout_changed = true;
                return Ok(continued);
            }
        }
        let mut emitted_replay = None;
        let checkpoint =
            matches!(&event, CoreEvent::Input(_)).then(|| self.begin_input_publication(view_id));
        let result = self.dispatch_core_event(view_id, event, &clipboard_context, &mut emitted_replay);
        if let Some(checkpoint) = checkpoint {
            match &result {
                Ok(outcome)
                    if outcome.command.as_ref().is_some_and(|command| {
                        matches!(command.status, CommandStatus::NeedsMoreLayout(_))
                    }) =>
                {
                    self.rollback_input_publication(view_id, checkpoint, false)
                }
                Ok(_) => self.document.commit_command_checkpoint(checkpoint.document),
                Err(_) => self.rollback_input_publication(view_id, checkpoint, true),
            }
        }
        let outcome = result?;
        let Some(plan) = emitted_replay else {
            return Ok(outcome);
        };
        if let ReplayPlan::LiteralTerminator(input) = plan {
            // Numeric literal input can finish with a visual motion. Publish
            // the inserted scalar first so that motion resolves in the new
            // layout. It is still the same physical key and must not be
            // recorded twice or introduce a compound-replay undo boundary.
            let mut accumulator = CoreOutcomeAccumulator::new(outcome);
            let recording_was_suppressed = self.views.get_mut(&view_id).expect("input view remains attached")
                .commands.set_macro_recording_suppressed(true);
            let continued = self.dispatch_replay_input(view_id, input, clipboard_context.as_ref());
            self.views.get_mut(&view_id).expect("input view remains attached")
                .commands.set_macro_recording_suppressed(recording_was_suppressed);
            accumulator.merge(continued)?;
            return Ok(accumulator.finish());
        }
        if self.replay_undo_floor.is_some() {
            debug_assert!(self.queued_replay.is_none());
            self.queued_replay = Some(plan);
            return Ok(outcome);
        }
        self.run_compound_replay(view_id, clipboard_context, outcome, plan)
    }

    /// One event remains tentative through controller rebasing and history
    /// restoration capture. Compound replay calls this boundary for each event,
    /// so an error never discards the successful prefix of a macro.
    fn dispatch_core_event(
        &mut self,
        view_id: ViewId,
        event: CoreEvent,
        clipboard_context: &Option<ClipboardCommandContext>,
        emitted_replay: &mut Option<ReplayPlan>,
    ) -> Result<CoreOutcome, CoreError> {
        if matches!(&event, CoreEvent::Input(_)) {
            self.install_buffer_commands(view_id);
            self.prepare_input_edit_group(view_id);
        }
        let history_before =
            matches!(&event, CoreEvent::Input(_)).then(|| self.document.history_status().current);
        let edit_group_depth_before = self.document.edit_group_depth();
        let edit_group_generation_before = self.document.edit_group_generation();
        let active_position_state = if matches!(&event, CoreEvent::Input(_)) {
            let view = self
                .views
                .get(&view_id)
                .expect("view existence checked above");
            Some((
                view.commands.clone(),
                view.commands.capture_position_anchors(&self.document)?,
            ))
        } else {
            None
        };
        let invoking_restoration_before = active_position_state
            .as_ref()
            .map(|(commands, anchors)| commands.substitute_confirmation_history(&self.document)
                .cloned().unwrap_or_else(|| anchors.history_snapshot()));
        let inactive_positions = if matches!(&event, CoreEvent::Input(_)) {
            self.views
                .iter()
                .filter(|(id, _)| **id != view_id)
                .map(|(id, view)| {
                    view.commands
                        .capture_position_anchors(&self.document)
                        .map(|anchors| (*id, anchors))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let outcome: Result<CoreOutcome, CoreError> = match event {
            CoreEvent::ReadFile { .. } => unreachable!("read completion handled before dispatch"),
            CoreEvent::EditCommandLine(_) => {
                unreachable!("prompt edits handled before document/layout dispatch")
            }
            CoreEvent::Resize { width, height } => {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view existence checked above");
                let before = view.layout.configuration_generation();
                view.layout.resize(width, height);
                if view.layout.configuration_generation() != before {
                    cancel_active_layout_work(view);
                }
                self.materialize_immediate_viewport(
                    view_id,
                    ImmediateLayoutIntent::PreserveViewport,
                )?;
                self.rematerialize_active_composition(view_id, true)?;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: true,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetScale(scale) => {
                let changed = {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("view existence checked above");
                    let before = view.layout.configuration_generation();
                    view.layout.set_scale(scale)?;
                    let changed = view.layout.configuration_generation() != before;
                    if changed {
                        cancel_active_layout_work(view);
                    }
                    changed
                };
                if changed {
                    self.materialize_immediate_viewport(
                        view_id,
                        ImmediateLayoutIntent::PreserveViewport,
                    )?;
                    self.rematerialize_active_composition(view_id, true)?;
                }
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: changed,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetFindPattern(pattern) => {
                if pattern.is_empty() {
                    return Err(CoreError::NoVisualSelection);
                }
                self.install_buffer_commands(view_id);
                let accepted = self
                    .views
                    .get_mut(&view_id)
                    .expect("view existence checked above")
                    .commands
                    .set_literal_search_pattern(&pattern);
                if !accepted {
                    return Err(CoreError::NoVisualSelection);
                }
                self.publish_buffer_commands(view_id);
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::RevealSelection => {
                let mode = self
                    .views
                    .get(&view_id)
                    .expect("view existence checked above")
                    .commands
                    .mode();
                if !matches!(
                    mode,
                    Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
                ) {
                    return Err(CoreError::NoVisualSelection);
                }
                let (before_top, before_layout) = {
                    let layout = &self
                        .views
                        .get(&view_id)
                        .expect("view existence checked above")
                        .layout;
                    (
                        layout.viewport_top(),
                        layout.snapshot().map(|snapshot| snapshot.revision),
                    )
                };
                self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
                let layout = &self
                    .views
                    .get(&view_id)
                    .expect("view remains attached after selection reveal")
                    .layout;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: layout.viewport_top() != before_top
                        || layout.snapshot().map(|snapshot| snapshot.revision) != before_layout,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetSmartQuotes(enabled) => {
                self.views
                    .get_mut(&view_id)
                    .expect("view checked")
                    .commands
                    .set_smart_quotes(enabled);
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetParagraphFlow(enabled) => {
                if !self.document.format().is_source_view() {
                    return Err(CoreError::Document(DocumentError::UnsupportedFormatting));
                }
                let view = self.views.get_mut(&view_id).expect("view checked");
                let before = view.layout.configuration_generation();
                view.layout.set_paragraph_flow(enabled);
                let changed = before != view.layout.configuration_generation();
                if changed {
                    cancel_active_layout_work(view);
                }
                self.materialize_immediate_viewport(
                    view_id,
                    ImmediateLayoutIntent::PreserveViewport,
                )?;
                self.rematerialize_active_composition(view_id, true)?;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: changed,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetLineMode(mode) => {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view existence checked above");
                view.commands.set_line_mode(&self.document, mode)?;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: false,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetWrap(wrap) => {
                let view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view existence checked above");
                let before = view.layout.configuration_generation();
                view.layout.set_wrap(wrap);
                view.commands
                    .set_layout_options(view.layout.wrap());
                if view.layout.configuration_generation() != before {
                    cancel_active_layout_work(view);
                }
                self.materialize_immediate_viewport(
                    view_id,
                    ImmediateLayoutIntent::PreserveViewport,
                )?;
                self.rematerialize_active_composition(view_id, true)?;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: true,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::SetViewportOrigin { left, top } => {
                // Validate every requested component before checking coverage
                // or mutating either coordinate. Vertical layout is staged so
                // a combined event remains atomic on every later failure.
                if !left.is_finite() || top.is_some_and(|top| !top.is_finite()) {
                    return Err(CoreError::Layout(LayoutError::InvalidGeometry));
                }
                let (previous_left, previous_top, previous_layout_revision) = {
                    let view = self
                        .views
                        .get(&view_id)
                        .expect("view existence checked above");
                    (
                        view.layout.viewport_left(),
                        view.layout.viewport_top(),
                        view.layout.snapshot().map(|snapshot| snapshot.revision),
                    )
                };
                if let Some(top) = top {
                    self.materialize_requested_viewport(view_id, left, top)?;
                } else {
                    let sparse = self.views[&view_id].layout.snapshot().is_some_and(|snapshot| snapshot.has_horizontal_materialization());
                    if sparse {
                        self.materialize_requested_viewport(view_id, left, previous_top)?;
                    } else {
                        self.views.get_mut(&view_id).expect("view existence checked above").layout.set_viewport_left(left)?;
                    }
                }
                self.rematerialize_active_composition(view_id, false)?;
                let view = self
                    .views
                    .get(&view_id)
                    .expect("view existence checked above");
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: view.layout.viewport_left() != previous_left
                        || view.layout.viewport_top() != previous_top
                        || view.layout.snapshot().map(|snapshot| snapshot.revision)
                            != previous_layout_revision,
                    composition_changes: Vec::new(),
                })
            }
            CoreEvent::Input(input) => {
                let was_visual_block = self.views[&view_id].commands.active_visual_block_endpoint_offsets().is_some();
                let (requires_layout, layout_intent) = {
                    let target_view = self
                        .views
                        .get(&view_id)
                        .expect("view existence checked above");
                    let requires_layout = target_view.commands.requires_layout_for_input(
                        &self.document,
                        &input,
                        clipboard_context.as_ref(),
                    );
                    let intent = if target_view.commands.layout_input_preserves_viewport(&input) {
                        ImmediateLayoutIntent::PreserveViewport
                    } else {
                        ImmediateLayoutIntent::RevealCaret
                    };
                    (requires_layout, intent)
                };
                let layout_is_stale = if requires_layout {
                    self.ensure_command_layout(view_id, layout_intent)?
                } else {
                    false
                };
                let edit_anchor = viewport::capture_edit_baseline_anchor(
                    &self.document,
                    self.views.get(&view_id).expect("validated view"),
                );
                let target_view = self
                    .views
                    .get_mut(&view_id)
                    .expect("view existence checked above");
                let before = self.document.revision();
                let old_viewport_top = target_view.layout.viewport_top();
                let resolution = if requires_layout {
                    CommandResolution::Legacy(crate::command::LegacyCommandReason::LayoutDependent)
                } else {
                    let context = clipboard_context.as_ref().map_or_else(
                        || CommandContext::new(&self.document),
                        |clipboard| CommandContext::with_clipboard(&self.document, clipboard),
                    );
                    target_view.commands.resolve(&context, input.clone())?
                };
                let (command_result, committed_position_map, planned_presentation) =
                    match resolution {
                        CommandResolution::Planned(plan) => {
                            let (step, map, presentation) = execute_command_plan(
                                &mut self.document,
                                &mut target_view.commands,
                                plan,
                            )?;
                            (Ok((step, old_viewport_top, false)), map, Some(presentation))
                        }
                        CommandResolution::Legacy(_) => {
                            let (result, map) = self.document.capture_position_maps(
                                |document| -> Result<_, CoreError> {
                                    if requires_layout {
                                        let snapshot = target_view
                                            .layout
                                            .snapshot()
                                            .expect("required layout was just made exact");
                                        let viewport = Viewport::new(
                                            target_view.layout.viewport_top(),
                                            target_view.layout.height(),
                                        )?;
                                        let mut context = LayoutCommandContext::new(
                                            snapshot,
                                            target_view.layout.wrap(),
                                            viewport,
                                        );
                                        let step =
                                            target_view.commands.handle_with_layout_for_core(
                                                document,
                                                input,
                                                &mut context,
                                                clipboard_context.as_ref(),
                                            )?;
                                        Ok((step, context.viewport().top, context.viewport_command()))
                                    } else {
                                        let step = target_view.commands.handle_for_core(
                                            document,
                                            input,
                                            clipboard_context.as_ref(),
                                        )?;
                                        Ok((step, old_viewport_top, false))
                                    }
                                },
                            );
                            (result, map, None)
                        }
                    };
                // Always finish the capture before propagating a controller
                // error. A failed model operation contributes no transition;
                // a compound command which deliberately retains earlier
                // commits still leaves a complete exact map available to the
                // coordinator once its error policy is published.
                let (step, viewport_top, viewport_command) = command_result?;
                let CommandStep {
                    output: command,
                    replay,
                } = step;
                *emitted_replay = replay;
                if matches!(command.status, CommandStatus::NeedsMoreLayout(_)) {
                    debug_assert_eq!(self.document.revision(), before);
                    if let Some((checkpoint, _)) = active_position_state.as_ref() {
                        target_view.commands = checkpoint.clone();
                    }
                    target_view.layout.set_viewport_top(old_viewport_top)?;
                    return Ok(CoreOutcome {
                        command: Some(command),
                        document_changed: false,
                        position_map: None,
                        layout_changed: layout_is_stale,
                        composition_changes: Vec::new(),
                    });
                }
                target_view.layout.set_viewport_top(viewport_top)?;
                let mut option_layout_changed = false;
                let mut buffer_whitespace_changed = false;
                if let Some(ex) = command.ex_outcome.as_ref() {
                    for effect in &ex.option_effects {
                        match (&effect.name, &effect.new_value) {
                            (_, ExOptionValue::VisibleWhitespace(_)) => {
                                option_layout_changed = true;
                            }
                            (_, ExOptionValue::Indentation(value)) => {
                                if let ExOptionValue::Indentation(old) = &effect.old_value {
                                    buffer_whitespace_changed |= old.effective().tabstop != value.effective().tabstop;
                                    option_layout_changed |= buffer_whitespace_changed;
                                }
                            }
                            (ExOptionName::Wrap, ExOptionValue::Boolean(value)) => {
                                let changed = target_view.layout.wrap() != *value;
                                option_layout_changed |= changed;
                                if changed {
                                    cancel_active_layout_work(target_view);
                                }
                                target_view.layout.set_wrap(*value);
                            }
                            _ => {}
                        }
                    }
                }
                let changed = self.document.revision() != before;
                if changed {
                    if let Some(anchor) = edit_anchor {
                        // Rebase this pre-edit boundary with the transaction.
                        // Unlike a new caret reveal, local typing must not
                        // recenter because the old layout revision is stale.
                        target_view.viewport_anchor = Some(anchor);
                    }
                }
                let cursor_moved = command.cursor_moved;
                let history_navigation = command.history_navigation;
                // Search prompts temporarily change mode while retaining the
                // rectangle and its exact layout. Release dense geometry only
                // when the command actually discards that selection.
                let command_requests_relayout =
                    planned_presentation.as_ref().is_some_and(|requests| {
                        requests.contains(&CommandPresentationRequest::Relayout)
                    }) || (was_visual_block && target_view.commands.visual_block().is_none());
                let command_requests_reveal = (!viewport_command || changed) && planned_presentation
                    .as_ref()
                    .map_or(changed || cursor_moved, |requests| {
                        requests.contains(&CommandPresentationRequest::RevealCaret)
                    });
                let command_presentation_intent = if command_requests_reveal && changed && edit_anchor.is_some() {
                    ImmediateLayoutIntent::PreserveViewportAndRevealCaret
                } else if command_requests_reveal {
                    ImmediateLayoutIntent::RevealCaret
                } else {
                    ImmediateLayoutIntent::PreserveViewport
                };
                let viewport_changed = target_view.layout.viewport_top() != old_viewport_top;
                let mut remains_in_edit =
                    matches!(target_view.commands.mode(), Mode::Insert | Mode::Replace);
                if let Some(floor) = self.replay_undo_floor {
                    debug_assert!(
                        history_navigation || self.document.edit_group_depth() >= floor,
                        "compound replay closed its coordinator-owned undo group"
                    );
                } else if remains_in_edit {
                    self.edit_group_owner = Some(view_id);
                } else if self.edit_group_owner == Some(view_id) {
                    self.document.close_edit_group();
                    self.edit_group_owner = None;
                }
                let mut outcome = CoreOutcome {
                    command: Some(command),
                    document_changed: changed,
                    position_map: None,
                    layout_changed: layout_is_stale
                        || changed
                        || option_layout_changed
                        || command_requests_relayout
                        || command_requests_reveal
                        || viewport_changed,
                    composition_changes: Vec::new(),
                };
                let mut rebased = Vec::with_capacity(inactive_positions.len());
                if changed {
                    // The source publication is the invalidation point for
                    // every attached view, not only the invoking one.
                    self.cancel_all_active_layout_work();
                    let revision = self.document.revision();
                    if committed_position_map.document() != self.document.id()
                        || committed_position_map.source_revision() != before
                        || committed_position_map.target_revision() != revision
                    {
                        return Err(CoreError::Position(PositionError::WrongSnapshot {
                            expected: revision,
                            actual: committed_position_map.target_revision(),
                        }));
                    }
                    let map = committed_position_map;
                    #[cfg(test)]
                    let map = self.input_position_map_override.take().unwrap_or(map);
                    outcome.position_map = Some(map.clone());
                    if let Some((checkpoint, anchors)) = active_position_state {
                        let commands = &mut self
                            .views
                            .get_mut(&view_id)
                            .expect("invoking view remains attached during serial dispatch")
                            .commands;
                        if !commands.rebase_unchanged_persistent_positions(
                            &self.document,
                            &checkpoint,
                            &anchors,
                            &map,
                            cursor_moved,
                        )? {
                            return Err(CoreError::Position(PositionError::WrongSnapshot {
                                expected: before,
                                actual: anchors.revision(),
                            }));
                        }
                    }
                    for (id, anchors) in inactive_positions {
                        let mut commands = self
                            .views
                            .get(&id)
                            .expect("captured view remains attached during serial dispatch")
                            .commands
                            .clone();
                        if !commands.apply_position_map(&anchors, &map)? {
                            // Captures and the map are both prepared against
                            // `before`; a mismatch means the serial coordinator
                            // invariant was violated. Do not partially publish
                            // any of the prepared controller states.
                            return Err(CoreError::Position(PositionError::WrongSnapshot {
                                expected: before,
                                actual: anchors.revision(),
                            }));
                        }
                        commands.capture_position_anchors(&self.document)?;
                        rebased.push((id, commands));
                    }
                    let history_after = self.document.history_status().current;
                    if history_navigation {
                        let restoration = self
                            .document
                            .history_restoration_between(
                                history_before
                                    .expect("input events capture their starting history node")
                                    .node,
                                history_after.node,
                            )?
                            .expect("a successful history navigation traverses a recorded edge");
                        self.views
                            .get_mut(&view_id)
                            .expect("invoking view remains attached during serial dispatch")
                            .commands
                            .apply_history_restoration(&self.document, &restoration)?;
                        // A one-shot Normal command entered through Insert
                        // Ctrl-O may have reopened its edit group before the
                        // coordinator installs the history-owned state. Undo
                        // and redo always finalize that transient group and
                        // leave the invoking view in Normal mode.
                        self.document.close_edit_group();
                        self.edit_group_owner = None;
                        self.edit_group_restoration = None;
                        // A successful history transaction closes the current
                        // compound edit group in the model. Leave no stale
                        // floor behind; the replay driver opens a fresh
                        // restoration segment only after this outcome has
                        // been completely published.
                        self.replay_undo_floor = None;
                        remains_in_edit = false;
                    } else {
                        let generation_after = self.document.edit_group_generation();
                        let before_restoration = self
                            .edit_group_restoration
                            .as_ref()
                            .filter(|open| {
                                edit_group_depth_before > 0
                                    && open.generation == edit_group_generation_before
                                    && generation_after == edit_group_generation_before
                            })
                            .map(|open| open.before.clone())
                            .unwrap_or_else(|| {
                                invoking_restoration_before
                                    .clone()
                                    .expect("input events capture invoking restoration state")
                            });
                        let after_restoration = self
                            .views
                            .get(&view_id)
                            .expect("invoking view remains attached during serial dispatch")
                            .commands
                            .capture_history_restoration(&self.document)?;
                        self.document.attach_history_restoration(
                            history_after.node,
                            HistoryRestoration::new(before_restoration, after_restoration),
                        )?;
                    }
                    self.rebase_viewport_anchors(&map)?;
                }
                if !changed && !remains_in_edit {
                    if let Some(open) = self.edit_group_restoration.as_ref() {
                        let current = self.document.history_status().current;
                        if current != open.parent {
                            let after = self
                                .views
                                .get(&view_id)
                                .expect("invoking view remains attached during serial dispatch")
                                .commands
                                .capture_history_restoration(&self.document)?;
                            self.document.attach_history_restoration(
                                current.node,
                                HistoryRestoration::new(open.before.clone(), after),
                            )?;
                        }
                    }
                }
                if remains_in_edit {
                    let generation = self.document.edit_group_generation();
                    let retains_current_origin = self
                        .edit_group_restoration
                        .as_ref()
                        .is_some_and(|open| open.generation == generation);
                    if !retains_current_origin {
                        let group_began_from_this_event = edit_group_depth_before == 0
                            && generation == edit_group_generation_before.saturating_add(1);
                        let resumed_after_normal_once = remains_in_edit
                            && group_began_from_this_event
                            && self
                                .views
                                .get(&view_id)
                                .expect("invoking view remains attached during serial dispatch")
                                .commands
                                .reopened_group_after_insert_normal_once();
                        let (before, parent) = if resumed_after_normal_once {
                            // Ctrl-O's Normal command has already finished;
                            // `finish_insert_normal_once` opened a fresh edit
                            // group for subsequent typed text. Its restoration
                            // baseline is the post-command revision/cursor,
                            // never the snapshot from before the Normal
                            // command (which may be an older source revision).
                            (
                                self.views
                                    .get(&view_id)
                                    .expect("invoking view remains attached during serial dispatch")
                                    .commands
                                    .capture_history_restoration(&self.document)?,
                                self.document.history_status().current,
                            )
                        } else if self.edit_group_restoration.is_none()
                            && (generation == edit_group_generation_before
                                || group_began_from_this_event)
                        {
                            (
                                invoking_restoration_before
                                    .clone()
                                    .expect("input events capture invoking restoration state"),
                                history_before
                                    .expect("input events capture their starting history node"),
                            )
                        } else {
                            (
                                self.views
                                    .get(&view_id)
                                    .expect("invoking view remains attached during serial dispatch")
                                    .commands
                                    .capture_history_restoration(&self.document)?,
                                self.document.history_status().current,
                            )
                        };
                        self.edit_group_restoration = Some(OpenGroupRestoration {
                            generation,
                            parent,
                            before,
                        });
                    }
                } else if self.replay_undo_floor.is_none() {
                    self.edit_group_restoration = None;
                }
                // All fallible controller/history publication checks have
                // succeeded. Presentation sessions can now be invalidated.
                for (id, commands) in rebased {
                    self.views
                        .get_mut(&id)
                        .expect("prepared view remains attached during serial dispatch")
                        .commands = commands;
                }
                if changed {
                    for (id, view) in &mut self.views {
                        if *id == view_id {
                            continue;
                        }
                        if let Some(session) = view.composition.take() {
                            view.composition_layout = None;
                            outcome.composition_changes.push(ViewCompositionChange {
                                view: *id,
                                outcome: ViewCompositionOutcome::Invalidated {
                                    reason: CompositionCancelReason::ExternalDocumentChange,
                                    base_revision: session.base_revision(),
                                    current_revision: self.document.revision(),
                                },
                            });
                        }
                    }
                }
                self.publish_buffer_commands(view_id);
                if buffer_whitespace_changed && !changed {
                    for id in self.views.keys().copied().filter(|id| *id != view_id).collect::<Vec<_>>() {
                        let result = self.materialize_immediate_viewport(id, ImmediateLayoutIntent::PreserveViewport)
                            .and_then(|_| self.rematerialize_active_composition(id, false));
                        if let Err(error) = result { self.record_presentation_error(id, error); }
                    }
                }
                let presentation_result = if changed {
                    self.materialize_views_after_document_change(
                        view_id,
                        command_presentation_intent,
                    );
                    Ok(())
                } else if option_layout_changed || command_requests_relayout {
                    self.materialize_immediate_viewport(view_id, command_presentation_intent)
                } else if command_requests_reveal {
                    let reveal = {
                        let view = self
                            .views
                            .get_mut(&view_id)
                            .expect("invoking view remains attached during serial dispatch");
                        viewport::reveal_caret_row(view)
                    };
                    match reveal {
                        Ok(()) => {
                            let view = self
                                .views
                                .get_mut(&view_id)
                                .expect("invoking view remains attached during serial dispatch");
                            update_viewport_anchor(&self.document, view);
                            Ok(())
                        }
                        Err(LayoutError::OutsideMaterializedCoverage) => self
                            .materialize_immediate_viewport(
                                view_id,
                                ImmediateLayoutIntent::RevealCaret,
                            ),
                        Err(error) => Err(CoreError::Layout(error)),
                    }
                } else {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("invoking view remains attached during serial dispatch");
                    update_viewport_anchor(&self.document, view);
                    Ok(())
                };
                if let Err(error) = presentation_result {
                    // Model/controller publication is complete. Retain the
                    // previous immutable snapshot and surface only a view
                    // presentation diagnostic, matching the existing commit
                    // failure policy.
                    self.record_presentation_error(view_id, error);
                }
                Ok(outcome)
            }
            CoreEvent::Composition(_) => {
                unreachable!("composition events return before ordinary dispatch")
            }
            CoreEvent::PlaceCursor { .. } => {
                unreachable!("pointer placements return before ordinary dispatch")
            }
            CoreEvent::SelectAll { .. } => {
                unreachable!("native whole-document selection returns before ordinary dispatch")
            }
            CoreEvent::GoToLine { .. } => {
                unreachable!("native line navigation returns before ordinary dispatch")
            }
            CoreEvent::NavigateHistory(_) => {
                unreachable!("native history navigation returns before ordinary dispatch")
            }
            CoreEvent::SetDirectCharacterProperties { .. }
            | CoreEvent::EditDirectProperty { .. }
            | CoreEvent::EditDirectProperties { .. }
            | CoreEvent::SetFileFormat { .. }
            | CoreEvent::SetIncludeStyleDefinitionsInFile { .. }
            | CoreEvent::SetFormat { .. }
            | CoreEvent::SetEncoding { .. }
            | CoreEvent::SetListStyle { .. }
            | CoreEvent::IndentList { .. }
            | CoreEvent::SetParagraphStyle { .. }
            | CoreEvent::AssignNamedStyle { .. }
            | CoreEvent::EditNamedStyleDefinition { .. } => {
                unreachable!("native file-format changes return before ordinary dispatch")
            }
            CoreEvent::SetSelectionSemanticStyle { .. } => {
                unreachable!("native semantic-style changes return before ordinary dispatch")
            }
            CoreEvent::EditGeneratedStyle { .. } => {
                unreachable!("native style edits return before ordinary dispatch")
            }
            CoreEvent::FlushMappingPrefix | CoreEvent::FlushMappingPrefixWithClipboard(_) => unreachable!("mapping flush handled before dispatch"),
            CoreEvent::InputWithClipboard { .. } => {
                unreachable!("clipboard input is normalized before ordinary dispatch")
            }
        };
        outcome
    }

    /// Open one undo/restoration segment of a synchronous compound replay.
    /// History navigation ends the current model group, after which the
    /// replay driver calls this again before dispatching later events.
    fn begin_replay_undo_segment(&mut self, view_id: ViewId) -> Result<(), CoreError> {
        debug_assert!(self.replay_undo_floor.is_none());
        debug_assert!(self.edit_group_restoration.is_none());
        let before = self
            .views
            .get(&view_id)
            .expect("replay view remains attached")
            .commands
            .capture_history_restoration(&self.document)?;
        let parent = self.document.history_status().current;
        self.document.begin_edit_group();
        let floor = self.document.edit_group_depth();
        let generation = self.document.edit_group_generation();
        self.edit_group_restoration = Some(OpenGroupRestoration {
            generation,
            parent,
            before,
        });
        self.replay_undo_floor = Some(floor);
        Ok(())
    }

    /// Publish the final restoration for the current replay segment and close
    /// only the coordinator-owned group nesting. A successful undo/redo has
    /// already closed and cleared its segment before control returns here.
    fn finish_replay_undo_segment(&mut self, view_id: ViewId) -> Result<(), CoreError> {
        let open = self.edit_group_restoration.take();
        let floor = self.replay_undo_floor.take();
        let restoration_result = (|| -> Result<(), CoreError> {
            let Some(open) = open else {
                return Ok(());
            };
            let current = self.document.history_status().current;
            if current == open.parent {
                return Ok(());
            }
            let after = self
                .views
                .get(&view_id)
                .expect("replay view remains attached")
                .commands
                .capture_history_restoration(&self.document)?;
            self.document.attach_history_restoration(
                current.node,
                HistoryRestoration::new(open.before, after),
            )?;
            Ok(())
        })();
        if let Some(floor) = floor {
            debug_assert!(
                self.document.edit_group_depth() >= floor,
                "an active replay segment owns an open model group"
            );
            if self.document.edit_group_depth() >= floor {
                self.document.restore_edit_group_depth(floor);
                self.document.end_edit_group();
            }
        }
        self.edit_group_owner = None;
        restoration_result
    }

    fn run_compound_replay(
        &mut self,
        view_id: ViewId,
        clipboard: Option<ClipboardCommandContext>,
        initial: CoreOutcome,
        initial_plan: ReplayPlan,
    ) -> Result<CoreOutcome, CoreError> {
        let mapping_replay = matches!(&initial_plan, ReplayPlan::Mapping(_));
        let editing = matches!(self.views[&view_id].commands.mode(), Mode::Insert | Mode::Replace);
        if mapping_replay && editing && self.edit_group_owner == Some(view_id)
            && self.document.edit_group_depth() > 0 {
            // An Insert mapping belongs to the existing typing undo group.
            // Reserve a nesting level above the replay floor for Escape to
            // close without consuming the coordinator-owned group.
            self.edit_group_owner = None;
            self.replay_undo_floor = Some(self.document.edit_group_depth());
            self.document.begin_edit_group();
        } else {
            if self.edit_group_owner.take().is_some() {
                self.document.close_edit_group();
            }
            self.edit_group_restoration = None;
            self.begin_replay_undo_segment(view_id)?;
            if mapping_replay && editing { self.document.begin_edit_group(); }
        }

        let mut accumulator = CoreOutcomeAccumulator::new(initial);
        let mut frames = Vec::new();
        let mut terminal_status = None;
        let mut global_error = None;
        let mut dispatched_events = 0usize;
        if self
            .views
            .get_mut(&view_id)
            .expect("replay view remains attached")
            .commands
            .begin_replay_frame()
        {
            if matches!(&initial_plan, ReplayPlan::ExNormal(plan) if plan.global) {
                self.views.get_mut(&view_id).unwrap().commands.global_replay_depth += 1;
            }
            frames.push(replay_frame(initial_plan));
        } else {
            terminal_status = Some(CommandStatus::Error(format!(
                "compound replay recursion limit ({COMPOUND_REPLAY_LIMIT}) reached"
            )));
        }

        let run_result = (|| -> Result<(), CoreError> {
            while terminal_status.is_none() && !frames.is_empty() {
                let action = loop {
                    let Some(frame) = frames.last_mut() else {
                        break ReplayAction::PopFrame;
                    };
                    match frame {
                        ReplayFrame::Mapping { plan, event } => {
                            if *event >= plan.events.len() { break ReplayAction::PopFrame; }
                            let (next, remap) = plan.events[*event].clone();
                            *event += 1;
                            break ReplayAction::Input { event: next, ex_normal: false, remap };
                        }
                        ReplayFrame::Macro {
                            plan,
                            iteration,
                            event,
                        } => {
                            if *iteration >= plan.iterations {
                                break ReplayAction::PopFrame;
                            }
                            if *event >= plan.events.len() {
                                *event = 0;
                                *iteration += 1;
                                continue;
                            }
                            let next = plan.events[*event].clone();
                            *event += 1;
                            break ReplayAction::Input {
                                event: next,
                                ex_normal: false,
                                remap: true,
                            };
                        }
                        ReplayFrame::ExNormal {
                            plan,
                            target,
                            event,
                            line_started,
                        } => {
                            if *target >= plan.targets.len() {
                                break ReplayAction::PopFrame;
                            }
                            if !*line_started {
                                let Some(bound) = plan.bound_target(*target)? else {
                                    *target += 1;
                                    continue;
                                };
                                *line_started = true;
                                *event = 0;
                                break ReplayAction::BeginExNormalLine(bound);
                            }
                            if *event < plan.events.len() {
                                let next = plan.events[*event].clone();
                                *event += 1;
                                break ReplayAction::Input {
                                    event: next,
                                    ex_normal: true,
                                    remap: !plan.literal,
                                };
                            }
                            break ReplayAction::FinishExNormalLine;
                        }
                    }
                };

                match action {
                    ReplayAction::PopFrame => {
                        let popped = frames.pop();
                        if matches!(&popped, Some(ReplayFrame::ExNormal { plan, .. }) if plan.global) {
                            self.views.get_mut(&view_id).unwrap().commands.global_replay_depth -= 1;
                        }
                        if popped.is_some() {
                            self.views
                                .get_mut(&view_id)
                                .expect("replay view remains attached")
                                .commands
                                .end_replay_frame();
                        }
                        if matches!(popped, Some(ReplayFrame::ExNormal { .. }))
                            && matches!(
                                accumulator.command_mut().status,
                                CommandStatus::Pending | CommandStatus::Cancelled
                            )
                        {
                            accumulator.command_mut().status = CommandStatus::Complete;
                        }
                    }
                    ReplayAction::BeginExNormalLine(target) => {
                        let Some(position) = ex_normal_target_position(&self.document, target)
                        else {
                            if let Some(ReplayFrame::ExNormal {
                                plan,
                                target,
                                line_started,
                                ..
                            }) = frames.last_mut()
                            {
                                plan.targets[*target] = None;
                                *target += 1;
                                *line_started = false;
                            }
                            continue;
                        };
                        let (old_cursor, old_mode) = {
                            let commands = &self
                                .views
                                .get(&view_id)
                                .expect("replay view remains attached")
                                .commands;
                            (commands.cursor(), commands.mode())
                        };
                        self.views
                            .get_mut(&view_id)
                            .expect("replay view remains attached")
                            .commands
                            .prepare_ex_normal_line(&self.document, position);
                        accumulator.command_mut().cursor_moved |= old_cursor != position;
                        accumulator.command_mut().mode_changed |= old_mode != Mode::Normal;
                        self.publish_buffer_commands(view_id);
                        accumulator.layout_changed = true;
                        if let Err(error) = self.materialize_immediate_viewport(
                            view_id,
                            ImmediateLayoutIntent::RevealCaret,
                        ) {
                            self.record_presentation_error(view_id, error);
                        }
                    }
                    ReplayAction::FinishExNormalLine => {
                        let needs_abort = self
                            .views
                            .get(&view_id)
                            .expect("replay view remains attached")
                            .commands
                            .replay_needs_abort();
                        if needs_abort {
                            let cleanup = self.dispatch_replay_input(
                                view_id,
                                InputEvent::Key(Key::Escape),
                                clipboard.as_ref(),
                            );
                            let cleanup_status = cleanup
                                .command
                                .as_ref()
                                .map(|command| command.status.clone());
                            accumulator.merge(cleanup)?;
                            if cleanup_status
                                .as_ref()
                                .is_some_and(command_status_stops_compound)
                            {
                                terminal_status = cleanup_status;
                                continue;
                            }
                        }
                        if matches!(
                            accumulator.command_mut().status,
                            CommandStatus::Pending | CommandStatus::Cancelled
                        ) {
                            accumulator.command_mut().status = CommandStatus::Complete;
                        }
                        if let Some(ReplayFrame::ExNormal {
                            target,
                            event,
                            line_started,
                            ..
                        }) = frames.last_mut()
                        {
                            *target += 1;
                            *event = 0;
                            *line_started = false;
                        }
                    }
                    ReplayAction::Input { event, ex_normal, remap } => {
                        if dispatched_events >= self.compound_replay_event_limit {
                            terminal_status = Some(CommandStatus::CountError(
                                CountError::ReplayEventBudgetExceeded {
                                    count: dispatched_events.saturating_add(1),
                                    events_per_iteration: 1,
                                    limit: self.compound_replay_event_limit,
                                },
                            ));
                            continue;
                        }
                        dispatched_events += 1;
                        if ex_normal
                            && self
                                .views
                                .get(&view_id)
                                .expect("replay view remains attached")
                                .commands
                                .rejects_ex_normal_macro_recording(&event)
                        {
                            terminal_status = Some(CommandStatus::Unsupported(
                                ":normal macro recording is not supported".into(),
                            ));
                            continue;
                        }

                        let old_suppressed = self.views.get_mut(&view_id).expect("replay view").commands.set_mapping_suppressed(!remap);
                        let outcome = self.dispatch_replay_input(view_id, event, clipboard.as_ref());
                        self.views.get_mut(&view_id).expect("replay view").commands.set_mapping_suppressed(old_suppressed);
                        let status = outcome
                            .command
                            .as_ref()
                            .map(|command| command.status.clone())
                            .unwrap_or(CommandStatus::Complete);
                        let history_navigation = outcome
                            .command
                            .as_ref()
                            .is_some_and(|command| command.history_navigation);
                        let host_action = outcome.command.as_ref().is_some_and(|command| {
                            command.ex_outcome.as_ref().is_some_and(|ex| {
                                ex.frontend_requests
                                    .iter()
                                    .any(|request| matches!(request, ExFrontendRequest::File(_)))
                            })
                        });
                        let map = outcome.position_map.clone();
                        accumulator.merge(outcome)?;
                        if let Some(map) = map.as_ref() {
                            if let Err(error) =
                                rebase_replay_ex_normal_targets(&self.document, &mut frames, map)
                            {
                                terminal_status = Some(CommandStatus::Error(format!(
                                    ":normal could not rebase its remaining range: {error}"
                                )));
                                continue;
                            }
                        }
                        if command_status_stops_compound(&status) {
                            if let Some(global) = frames.iter().rposition(|frame| matches!(frame, ReplayFrame::ExNormal { plan, .. } if plan.global)) {
                                // A failed line does not discard successful global
                                // edits or skip later selected identities.
                                global_error.get_or_insert(status);
                                while frames.len() > global + 1 {
                                    frames.pop();
                                    self.views.get_mut(&view_id).unwrap().commands.end_replay_frame();
                                }
                                if let ReplayFrame::ExNormal { plan, event, .. } = &mut frames[global] { *event = plan.events.len(); }
                                accumulator.command_mut().status = CommandStatus::Complete;
                                self.queued_replay = None;
                            } else { terminal_status = Some(status); }
                            continue;
                        }
                        if host_action {
                            let mapping_finished = mapping_replay && frames.iter().all(|frame| {
                                matches!(frame, ReplayFrame::Mapping { plan, event } if *event == plan.events.len())
                            });
                            terminal_status = Some(if mapping_finished {
                                status
                            } else {
                                CommandStatus::Unsupported("compound replay stopped at a host action".into())
                            });
                            continue;
                        }
                        if history_navigation {
                            // The model closed the old segment before moving
                            // through history. Start the next replay segment
                            // only after its exact map, controller
                            // restoration, Ex outcome, and presentation state
                            // have all been merged and published.
                            self.begin_replay_undo_segment(view_id)?;
                        }
                        if let Some(nested) = self.queued_replay.take() {
                            if frames.len() >= COMPOUND_REPLAY_LIMIT
                                || !self
                                    .views
                                    .get_mut(&view_id)
                                    .expect("replay view remains attached")
                                    .commands
                                    .begin_replay_frame()
                            {
                                terminal_status = Some(CommandStatus::Error(format!(
                                    "compound replay recursion limit ({COMPOUND_REPLAY_LIMIT}) reached"
                                )));
                                continue;
                            }
                            if matches!(&nested, ReplayPlan::ExNormal(plan) if plan.global) {
                                self.views.get_mut(&view_id).unwrap().commands.global_replay_depth += 1;
                            }
                            frames.push(replay_frame(nested));
                        }
                    }
                }
            }

            if terminal_status.is_some()
                && self
                    .views
                    .get(&view_id)
                    .expect("replay view remains attached")
                    .commands
                    .replay_needs_abort()
            {
                let cleanup = self.dispatch_replay_input(
                    view_id,
                    InputEvent::Key(Key::Escape),
                    clipboard.as_ref(),
                );
                accumulator.merge(cleanup)?;
            }
            Ok(())
        })();

        while let Some(frame) = frames.pop() {
            if matches!(&frame, ReplayFrame::ExNormal { plan, .. } if plan.global) {
                self.views.get_mut(&view_id).unwrap().commands.global_replay_depth -= 1;
            }
            self.views
                .get_mut(&view_id)
                .expect("replay view remains attached")
                .commands
                .end_replay_frame();
        }
        self.queued_replay = None;

        // Capture the final controller state before finalizing the current
        // replay segment. A replay with history navigation may have finalized
        // earlier segments already; each segment retains its own restoration
        // endpoints and undo grouping.
        let mapping_continues_edit = mapping_replay
            && matches!(self.views[&view_id].commands.mode(), Mode::Insert | Mode::Replace);
        let restoration_result = if mapping_continues_edit {
            if let Some(floor) = self.replay_undo_floor.take() {
                self.document.restore_edit_group_depth(floor);
            }
            self.edit_group_owner = Some(view_id);
            Ok(())
        } else {
            self.finish_replay_undo_segment(view_id)
        };

        if let Some(status) = terminal_status.or(global_error) {
            accumulator.command_mut().status = status;
        }
        self.views
            .get_mut(&view_id)
            .expect("replay view remains attached")
            .commands
            .finish_deferred_insert_normal_once(&mut self.document, accumulator.command_mut());
        let resumes_edit = self
            .views
            .get(&view_id)
            .is_some_and(|view| matches!(view.commands.mode(), Mode::Insert | Mode::Replace));
        if resumes_edit && !mapping_continues_edit {
            self.edit_group_owner = Some(view_id);
            let generation = self.document.edit_group_generation();
            let before = self
                .views
                .get(&view_id)
                .expect("replay view remains attached")
                .commands
                .capture_history_restoration(&self.document)?;
            self.edit_group_restoration = Some(OpenGroupRestoration {
                generation,
                parent: self.document.history_status().current,
                before,
            });
        }
        self.publish_buffer_commands(view_id);
        restoration_result?;
        run_result?;
        Ok(accumulator.finish())
    }

    fn dispatch_replay_input(
        &mut self,
        view_id: ViewId,
        input: InputEvent,
        clipboard: Option<&ClipboardCommandContext>,
    ) -> CoreOutcome {
        const MAX_LAYOUT_RETRIES: usize = 16;
        let mut last_requested = None;
        for _ in 0..MAX_LAYOUT_RETRIES {
            self.queued_replay = None;
            let event = clipboard.map_or_else(
                || CoreEvent::Input(input.clone()),
                |clipboard| CoreEvent::InputWithClipboard {
                    input: input.clone(),
                    clipboard: clipboard.clone(),
                },
            );
            let mut outcome = match self.handle(view_id, event) {
                Ok(outcome) => outcome,
                Err(error) => {
                    return replay_error_outcome(format!(
                        "compound replay stopped after its successful prefix: {error:?}"
                    ));
                }
            };
            let demand = match outcome.command.as_ref().map(|command| &command.status) {
                Some(CommandStatus::NeedsMoreLayout(
                    LayoutMotionError::OutsideMaterializedCoverage(demand),
                )) => demand.clone(),
                _ => return outcome,
            };
            outcome.layout_changed = true;
            let requested = demand.requested_hard_lines();
            let request_identity = self.input_layout_request_identity(view_id, &demand, requested);
            if last_requested.as_ref() == Some(&request_identity) {
                if let Some(command) = outcome.command.as_mut() {
                    command.status = CommandStatus::Error(
                        "compound replay layout demand did not expand coverage".into(),
                    );
                }
                return outcome;
            }
            last_requested = Some(request_identity);
            if let Err(error) = self.satisfy_replay_layout_demand(view_id, &demand) {
                if let Some(command) = outcome.command.as_mut() {
                    command.status = CommandStatus::Error(format!(
                        "compound replay could not satisfy layout demand: {error:?}"
                    ));
                }
                return outcome;
            }
        }
        replay_error_outcome("compound replay layout retry limit reached".into())
    }

    fn satisfy_replay_layout_demand(
        &mut self,
        view_id: ViewId,
        demand: &LayoutDemand,
    ) -> Result<(), CoreError> {
        let Some(requested) = self.input_layout_region(view_id, demand)? else {
            return Err(LayoutMotionError::OutsideMaterializedCoverage(demand.clone()).into());
        };
        self.satisfy_input_layout_demand(view_id, demand, requested)
    }

    fn cancel_composition_for_input(
        &mut self,
        view_id: ViewId,
        input: &InputEvent,
    ) -> Result<CoreOutcome, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .expect("view existence checked by handle");
        let reason = match (view.commands.mode(), input) {
            (Mode::Normal, InputEvent::Key(Key::Char('u'))) => CompositionCancelReason::Undo,
            (Mode::Normal, InputEvent::Key(Key::Ctrl('r' | 'R'))) => CompositionCancelReason::Redo,
            _ => CompositionCancelReason::OrdinaryInput,
        };
        let session = view
            .composition
            .as_ref()
            .expect("active composition checked by handle")
            .clone();
        let restoration = session.cancel(&self.document)?;
        self.views
            .get_mut(&view_id)
            .expect("view remains attached during serial dispatch")
            .composition = None;
        self.views
            .get_mut(&view_id)
            .expect("view remains attached during serial dispatch")
            .composition_layout = None;
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            layout_changed: true,
            composition_changes: vec![ViewCompositionChange {
                view: view_id,
                outcome: ViewCompositionOutcome::Cancelled {
                    reason,
                    restoration,
                },
            }],
        })
    }

    fn handle_composition_event(
        &mut self,
        view_id: ViewId,
        event: CompositionEvent,
    ) -> Result<CoreOutcome, CoreError> {
        match event {
            CompositionEvent::Begin(target) => {
                if self
                    .views
                    .get(&view_id)
                    .expect("view existence checked by handle")
                    .composition
                    .is_some()
                {
                    return Err(CoreError::Composition(CompositionError::AlreadyActive));
                }
                let session = CompositionSession::begin(&self.document, target)?;
                let overlay = session.overlay(&self.document)?;
                if self.edit_group_owner.take().is_some() {
                    // An IME commit is an independent native text-input
                    // boundary, so it cannot join a preceding Insert unit.
                    self.document.close_edit_group();
                }
                self.edit_group_restoration = None;
                self.views
                    .get_mut(&view_id)
                    .expect("view remains attached during serial dispatch")
                    .composition = Some(session);
                if let Err(error) = self.materialize_composition_layout(view_id, true) {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("view remains attached after composition layout failure");
                    view.composition = None;
                    view.composition_layout = None;
                    return Err(error);
                }
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: true,
                    composition_changes: vec![ViewCompositionChange {
                        view: view_id,
                        outcome: ViewCompositionOutcome::Began(overlay),
                    }],
                })
            }
            CompositionEvent::Update(update) => {
                let (previous_session, previous_layout, overlay) = {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("view existence checked by handle");
                    let previous_session = view
                        .composition
                        .as_ref()
                        .ok_or(CoreError::Composition(CompositionError::NoActiveSession))?
                        .clone();
                    let previous_layout = view.composition_layout.clone();
                    let overlay = view
                        .composition
                        .as_mut()
                        .expect("active session was just validated")
                        .update(&self.document, update)?;
                    (previous_session, previous_layout, overlay)
                };
                if let Err(error) = self.materialize_composition_layout(view_id, true) {
                    let view = self
                        .views
                        .get_mut(&view_id)
                        .expect("view remains attached after composition layout failure");
                    view.composition = Some(previous_session);
                    view.composition_layout = previous_layout;
                    return Err(error);
                }
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: true,
                    composition_changes: vec![ViewCompositionChange {
                        view: view_id,
                        outcome: ViewCompositionOutcome::Updated(overlay),
                    }],
                })
            }
            CompositionEvent::Cancel => {
                let session = self
                    .views
                    .get(&view_id)
                    .expect("view existence checked by handle")
                    .composition
                    .as_ref()
                    .ok_or(CoreError::Composition(CompositionError::NoActiveSession))?
                    .clone();
                let restoration = session.cancel(&self.document)?;
                self.views
                    .get_mut(&view_id)
                    .expect("view remains attached during serial dispatch")
                    .composition = None;
                self.views
                    .get_mut(&view_id)
                    .expect("view remains attached during serial dispatch")
                    .composition_layout = None;
                Ok(CoreOutcome {
                    command: None,
                    document_changed: false,
                    position_map: None,
                    layout_changed: true,
                    composition_changes: vec![ViewCompositionChange {
                        view: view_id,
                        outcome: ViewCompositionOutcome::Cancelled {
                            reason: CompositionCancelReason::Explicit,
                            restoration,
                        },
                    }],
                })
            }
            CompositionEvent::Commit => self.commit_composition(view_id),
        }
    }

    fn commit_composition(&mut self, view_id: ViewId) -> Result<CoreOutcome, CoreError> {
        self.install_buffer_commands(view_id);
        let composition_baseline = viewport::composition_caret_baseline(
            &self.document, self.views.get(&view_id).expect("validated composition view"),
        );
        let session = self
            .views
            .get(&view_id)
            .expect("view existence checked by handle")
            .composition
            .as_ref()
            .ok_or(CoreError::Composition(CompositionError::NoActiveSession))?
            .clone();
        if self.edit_group_owner.take().is_some() {
            // Begin normally closes this already. Another view may since have
            // opened an empty Insert group without changing the revision. It
            // must be closed before model preparation captures its publication
            // preconditions.
            self.document.close_edit_group();
        }
        self.edit_group_restoration = None;
        let invoking_commands = &self
            .views
            .get(&view_id)
            .expect("composition view remains attached")
            .commands;
        let typing_properties = invoking_commands.typing_properties().to_vec();
        let typing_named = invoking_commands.typing_named_style().cloned();
        let request = session.prepare_commit_with_input_policy(&self.document, invoking_commands)?;
        let replaced_empty_range = request.edit().range.is_empty();
        let inserted_text = request.edit().replacement.clone();
        let caret_offset = request.caret_offset();
        let before = request.prepared_model_transaction().before_revision();
        let planned_after = request.prepared_model_transaction().after_revision();
        let changed = planned_after != before;
        let exact_position_map = request
            .prepared_model_transaction()
            .text_position_map()
            .clone();
        let anchors = self
            .views
            .iter()
            .map(|(id, view)| {
                view.commands
                    .capture_position_anchors(&self.document)
                    .map(|anchors| (*id, anchors))
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Clone and validate every controller mapping before the authoritative
        // source pointer moves. The prepared map is the same owned map returned
        // by the committed transaction; commit consumes rather than recomputes
        // it, so no broad before/after diff participates in this path.
        let mut next_commands = self
            .views
            .iter()
            .map(|(id, view)| (*id, view.commands.clone()))
            .collect::<BTreeMap<_, _>>();
        if changed {
            for (id, captured) in &anchors {
                let commands = next_commands
                    .get_mut(id)
                    .expect("captured view remains attached during serial dispatch");
                if !commands.apply_position_map(captured, &exact_position_map)? {
                    return Err(CoreError::Position(PositionError::WrongSnapshot {
                        expected: before,
                        actual: captured.revision(),
                    }));
                }
            }
        }

        let (commit, committed_model) = request.apply_with_model_transaction(&mut self.document)?;
        let revision = committed_model.after_revision();
        debug_assert_eq!(committed_model.before_revision(), before);
        debug_assert_eq!(revision, planned_after);
        debug_assert_eq!(committed_model.text_position_map(), &exact_position_map);
        debug_assert_eq!(self.document.revision(), revision);
        if changed {
            self.cancel_all_active_layout_work();
        }
        let target_commands = next_commands
            .get_mut(&view_id)
            .expect("target view remains attached during serial dispatch");
        target_commands
            .note_external_text_commit(
                &self.document,
                caret_offset,
                replaced_empty_range,
                &inserted_text,
                session.marked_text(),
            )
            .expect("composition preparation validated the committed caret boundary");
        target_commands.restore_typing_style(typing_named, typing_properties);
        for (id, commands) in next_commands {
            self.views
                .get_mut(&id)
                .expect("prepared view remains attached during serial dispatch")
                .commands = commands;
        }
        if changed {
            self.rebase_viewport_anchors(&exact_position_map)?;
        }
        let mut presentation_intent = ImmediateLayoutIntent::RevealCaret;
        if let Some(baseline) = composition_baseline {
            let view = self.views.get_mut(&view_id).expect("validated composition view");
            let point = self.document.text_point(caret_offset)
                .expect("composition preparation validated the committed caret");
            let anchor = self.document.text_anchor(
                point, Association::BeforeInsertion, view.commands.boundary_affinity(),
                DeletionRecovery::PreferFollowingThenPreceding,
            ).expect("validated composition caret has a persistent anchor");
            view.viewport_anchor = Some(ViewportTextAnchor {
                anchor,
                offset_from_reference: -baseline,
                reference: ViewportAnchorReference::Baseline,
            });
            presentation_intent = ImmediateLayoutIntent::PreserveViewportAndRevealCaret;
        }
        self.publish_buffer_commands(view_id);

        let mut composition_changes = vec![ViewCompositionChange {
            view: view_id,
            outcome: ViewCompositionOutcome::Committed(commit),
        }];
        self.views
            .get_mut(&view_id)
            .expect("target view remains attached during serial dispatch")
            .composition = None;
        self.views
            .get_mut(&view_id)
            .expect("target view remains attached during serial dispatch")
            .composition_layout = None;
        if changed {
            for (id, view) in &mut self.views {
                if *id == view_id {
                    continue;
                }
                if let Some(other) = view.composition.take() {
                    view.composition_layout = None;
                    composition_changes.push(ViewCompositionChange {
                        view: *id,
                        outcome: ViewCompositionOutcome::Invalidated {
                            reason: CompositionCancelReason::ExternalDocumentChange,
                            base_revision: other.base_revision(),
                            current_revision: revision,
                        },
                    });
                }
            }
            // Composition source/controller publication is complete before
            // shaping. A provider failure leaves the old snapshot installed
            // and is observable through `ViewLayout::last_error`; it must not
            // make the committed composition appear to have rolled back.
            self.materialize_views_after_document_change(
                view_id,
                presentation_intent,
            );
        }

        Ok(CoreOutcome {
            command: None,
            document_changed: changed,
            position_map: changed.then_some(exact_position_map),
            layout_changed: true,
            composition_changes,
        })
    }
}

fn ranges_touch(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    left.start <= right.end && right.start <= left.end
}

fn union_ranges(
    left: std::ops::Range<usize>,
    right: &std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    left.start.min(right.start)..left.end.max(right.end)
}

fn map_base_range_to_overlay(
    overlay: &CompositionOverlay,
    range: &std::ops::Range<usize>,
) -> Result<std::ops::Range<usize>, LayoutError> {
    let start = overlay
        .overlay_offset_for_base_boundary(range.start, Association::AfterInsertion)
        .ok_or(LayoutError::InvalidTextOffset(range.start))?;
    let end = overlay
        .overlay_offset_for_base_boundary(range.end, Association::AfterInsertion)
        .ok_or(LayoutError::InvalidTextOffset(range.end))?;
    Ok(start..end)
}

fn composition_layout_styles(
    mut styles: DocumentLayoutStyles,
    overlay: &CompositionOverlay,
    affinity: BoundaryAffinity,
) -> Result<DocumentLayoutStyles, LayoutError> {
    let replaced = overlay.replacement_range();
    let inserted = overlay.marked_range();
    let old_text_len = overlay.base_utf8_len();

    let shaping =
        resolved_composition_shaping_style(&styles, replaced.start, old_text_len, affinity);
    let paint = resolved_composition_paint(&styles, replaced.start, old_text_len, affinity);
    styles.shaping_runs = splice_shaping_runs(styles.shaping_runs, &replaced, &inserted, shaping)?;
    styles.paint_runs = splice_paint_runs(styles.paint_runs, &replaced, &inserted, paint)?;
    if let Some(search) = &mut styles.search_paint_overlay {
        search.remap_ranges(|range| map_base_range_to_overlay(overlay, &range).ok());
    }
    styles.paragraphs = splice_paragraph_styles(styles.paragraphs, &replaced, &inserted, affinity)?;
    Ok(styles)
}

fn resolved_composition_shaping_style(
    styles: &DocumentLayoutStyles,
    offset: usize,
    text_len: usize,
    affinity: BoundaryAffinity,
) -> crate::layout::ResolvedTextStyle {
    adjacent_run(&styles.shaping_runs, offset, text_len, affinity, |run| {
        &run.text_range
    })
    .map(|run| run.style.clone())
    .or_else(|| {
        adjacent_run(
            &styles.paragraphs,
            offset,
            text_len,
            affinity,
            |paragraph| &paragraph.text_range,
        )
        .map(|paragraph| paragraph.default_shaping_style.clone())
    })
    .unwrap_or_else(|| styles.default_shaping_style.clone())
}

fn resolved_composition_paint(
    styles: &DocumentLayoutStyles,
    offset: usize,
    text_len: usize,
    affinity: BoundaryAffinity,
) -> crate::layout::ResolvedTextPaint {
    adjacent_run(&styles.paint_runs, offset, text_len, affinity, |run| {
        &run.text_range
    })
    .map(|run| run.paint.clone())
    .unwrap_or_else(|| styles.default_paint.clone())
}

fn adjacent_run<T>(
    values: &[T],
    offset: usize,
    text_len: usize,
    affinity: BoundaryAffinity,
    range: impl Fn(&T) -> &std::ops::Range<usize>,
) -> Option<&T> {
    let downstream = || {
        values
            .iter()
            .find(|value| range(value).start <= offset && offset < range(value).end)
    };
    let upstream = || {
        values
            .iter()
            .rev()
            .find(|value| range(value).start < offset && offset <= range(value).end)
    };
    match affinity {
        BoundaryAffinity::Downstream => downstream().or_else(upstream),
        BoundaryAffinity::Upstream => upstream().or_else(downstream),
    }
    .or_else(|| if text_len == 0 { values.first() } else { None })
}

fn mapped_suffix_offset(
    offset: usize,
    replaced_end: usize,
    inserted_end: usize,
) -> Result<usize, LayoutError> {
    inserted_end
        .checked_add(offset.saturating_sub(replaced_end))
        .ok_or(LayoutError::InvalidTextOffset(offset))
}

fn splice_shaping_runs(
    runs: Vec<ShapeStyleRun>,
    replaced: &std::ops::Range<usize>,
    inserted: &std::ops::Range<usize>,
    inserted_style: crate::layout::ResolvedTextStyle,
) -> Result<Vec<ShapeStyleRun>, LayoutError> {
    let mut output = Vec::with_capacity(runs.len() + 1);
    for run in runs {
        let before_end = run.text_range.end.min(replaced.start);
        if run.text_range.start < before_end {
            output.push(ShapeStyleRun {
                text_range: run.text_range.start..before_end,
                style: run.style.clone(),
            });
        }
        let after_start = run.text_range.start.max(replaced.end);
        if after_start < run.text_range.end {
            output.push(ShapeStyleRun {
                text_range: mapped_suffix_offset(after_start, replaced.end, inserted.end)?
                    ..mapped_suffix_offset(run.text_range.end, replaced.end, inserted.end)?,
                style: run.style,
            });
        }
    }
    if !inserted.is_empty() {
        output.push(ShapeStyleRun {
            text_range: inserted.clone(),
            style: inserted_style,
        });
    }
    output.sort_by_key(|run| run.text_range.start);
    merge_shaping_runs(output)
}

fn merge_shaping_runs(runs: Vec<ShapeStyleRun>) -> Result<Vec<ShapeStyleRun>, LayoutError> {
    let mut merged: Vec<ShapeStyleRun> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(previous) = merged.last_mut() {
            if run.text_range.start < previous.text_range.end {
                return Err(LayoutError::InvalidStyleRun {
                    index: merged.len(),
                    reason: "composition style runs overlap",
                });
            }
            if previous.text_range.end == run.text_range.start && previous.style == run.style {
                previous.text_range.end = run.text_range.end;
                continue;
            }
        }
        merged.push(run);
    }
    Ok(merged)
}

fn splice_paint_runs(
    runs: Vec<PaintStyleRun>,
    replaced: &std::ops::Range<usize>,
    inserted: &std::ops::Range<usize>,
    inserted_paint: crate::layout::ResolvedTextPaint,
) -> Result<Vec<PaintStyleRun>, LayoutError> {
    let mut output = Vec::with_capacity(runs.len() + 1);
    for run in runs {
        let before_end = run.text_range.end.min(replaced.start);
        if run.text_range.start < before_end {
            output.push(PaintStyleRun {
                text_range: run.text_range.start..before_end,
                paint: run.paint.clone(),
            });
        }
        let after_start = run.text_range.start.max(replaced.end);
        if after_start < run.text_range.end {
            output.push(PaintStyleRun {
                text_range: mapped_suffix_offset(after_start, replaced.end, inserted.end)?
                    ..mapped_suffix_offset(run.text_range.end, replaced.end, inserted.end)?,
                paint: run.paint,
            });
        }
    }
    if !inserted.is_empty() {
        output.push(PaintStyleRun {
            text_range: inserted.clone(),
            paint: inserted_paint,
        });
    }
    output.sort_by_key(|run| run.text_range.start);
    let mut merged: Vec<PaintStyleRun> = Vec::with_capacity(output.len());
    for run in output {
        if let Some(previous) = merged.last_mut() {
            if run.text_range.start < previous.text_range.end {
                return Err(LayoutError::InvalidStyleRun {
                    index: merged.len(),
                    reason: "composition paint runs overlap",
                });
            }
            if previous.text_range.end == run.text_range.start && previous.paint == run.paint {
                previous.text_range.end = run.text_range.end;
                continue;
            }
        }
        merged.push(run);
    }
    Ok(merged)
}

fn splice_paragraph_styles(
    paragraphs: Vec<ParagraphLayoutStyle>,
    replaced: &std::ops::Range<usize>,
    inserted: &std::ops::Range<usize>,
    affinity: BoundaryAffinity,
) -> Result<Vec<ParagraphLayoutStyle>, LayoutError> {
    let chosen = adjacent_run(
        &paragraphs,
        replaced.start,
        paragraphs.last().map_or(0, |value| value.text_range.end),
        affinity,
        |paragraph| &paragraph.text_range,
    )
    .map(|paragraph| paragraph.block_id);
    let mut output = Vec::with_capacity(paragraphs.len());
    for mut paragraph in paragraphs {
        let is_chosen = chosen == Some(paragraph.block_id);
        let range = paragraph.text_range.clone();
        let mapped = if range.end <= replaced.start && !is_chosen {
            range
        } else if range.start >= replaced.end && !is_chosen {
            mapped_suffix_offset(range.start, replaced.end, inserted.end)?
                ..mapped_suffix_offset(range.end, replaced.end, inserted.end)?
        } else {
            let start = range.start.min(replaced.start);
            let end = if range.end <= replaced.end {
                inserted.end
            } else {
                mapped_suffix_offset(range.end, replaced.end, inserted.end)?
            }
            .max(inserted.end);
            start..end
        };
        paragraph.list_marker_range = match paragraph.list_marker_range {
            Some(marker) if marker.end <= replaced.start => Some(marker),
            Some(marker) if marker.start >= replaced.end => Some(
                mapped_suffix_offset(marker.start, replaced.end, inserted.end)?
                    ..mapped_suffix_offset(marker.end, replaced.end, inserted.end)?,
            ),
            _ => None,
        };
        paragraph.text_range = mapped;
        output.push(paragraph);
    }
    output.sort_by_key(|paragraph| paragraph.text_range.start);
    Ok(output)
}

fn replay_frame(plan: ReplayPlan) -> ReplayFrame {
    match plan {
        ReplayPlan::LiteralTerminator(_) => unreachable!("literal input continues before compound replay"),
        ReplayPlan::Mapping(plan) => ReplayFrame::Mapping { plan, event: 0 },
        ReplayPlan::Macro(plan) => ReplayFrame::Macro {
            plan,
            iteration: 0,
            event: 0,
        },
        ReplayPlan::ExNormal(plan) => ReplayFrame::ExNormal {
            plan,
            target: 0,
            event: 0,
            line_started: false,
        },
    }
}

fn rebase_replay_ex_normal_targets(
    document: &Document,
    frames: &mut [ReplayFrame],
    map: &PositionMap,
) -> Result<(), PositionError> {
    for frame in frames {
        let ReplayFrame::ExNormal {
            plan,
            target,
            line_started,
            ..
        } = frame
        else {
            continue;
        };
        if plan.global {
            let composed = plan.global_map.as_ref().map_or_else(|| Ok(map.clone()), |previous| previous.then(map))?;
            plan.global_map = Some(Box::new(composed));
            continue;
        }
        let start = if *line_started {
            target.saturating_add(1)
        } else {
            *target
        };
        if start < plan.targets.len() {
            rebase_ex_normal_targets(document, &mut plan.targets[start..], map)?;
        }
    }
    Ok(())
}

fn replay_error_outcome(message: String) -> CoreOutcome {
    CoreOutcome {
        command: Some(CommandOutput::unsupported(message)),
        document_changed: false,
        position_map: None,
        layout_changed: false,
        composition_changes: Vec::new(),
    }
}

fn cancel_active_layout_work<P: TextMeasurementProvider>(view: &mut View<P>) -> bool {
    let mut cancelled = false;
    for slot in [&mut view.active_layout_work, &mut view.active_prelayout_work] {
        if let Some(active) = slot.take() { active.cancellation.cancel(); cancelled = true; }
    }
    cancelled
}

fn cancel_obsolete_layout_work<P: TextMeasurementProvider>(
    view: &mut View<P>,
    document_revision: Revision,
    requirements: LayoutProviderRequirements,
) -> bool {
    let configuration = view.layout.configuration_generation();
    let mut cancelled = false;
    for slot in [&mut view.active_layout_work, &mut view.active_prelayout_work] {
        if slot.as_ref().is_some_and(|active| active.is_obsolete(document_revision, configuration,
            requirements.measurement_environment_id, requirements.metrics_generation)) {
            slot.take().expect("obsolete job exists").cancellation.cancel();
            cancelled = true;
        }
    }
    cancelled
}

fn refresh_observed_metrics<P: TextMeasurementProvider>(view: &mut View<P>) -> bool {
    let metrics_generation = inspect_layout_provider(&view.engine).metrics_generation;
    if metrics_generation == view.observed_metrics_generation {
        return false;
    }
    cancel_active_layout_work(view);
    view.layout.invalidate_text_metrics();
    view.observed_metrics_generation = metrics_generation;
    true
}

fn current_snapshot_for_layout<'a>(
    document: &Document,
    layout: &'a ViewLayout,
    requirements: LayoutProviderRequirements,
) -> Option<&'a crate::layout::LayoutSnapshot> {
    layout.snapshot().filter(|snapshot| {
        snapshot.document_id == document.id()
            && snapshot.document_revision == document.revision()
            && snapshot.configuration_generation == layout.configuration_generation()
            && snapshot.measurement_environment_id == requirements.measurement_environment_id
            && snapshot.metrics_generation == requirements.metrics_generation
    })
}

fn layout_origin_has_exact_geometry(
    snapshot: Option<&crate::layout::LayoutSnapshot>,
    viewport_height: f32,
    top: f32,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    match &snapshot.coverage {
        LayoutCoverage::FullDocument { .. } => true,
        LayoutCoverage::PartialHardLines { prefix_is_exact, .. } => *prefix_is_exact
            && snapshot.missing_viewport_edges(top, viewport_height) == (false, false),
    }
}

fn restore_viewport_anchor<P: TextMeasurementProvider>(
    view: &mut View<P>,
) -> Result<(), LayoutError> {
    let Some(anchor) = view.viewport_anchor else {
        return Ok(());
    };
    let requested_top = {
        let Some(snapshot) = view.layout.snapshot() else {
            return Ok(());
        };
        let geometry = viewport::anchor_geometry(snapshot, anchor)?;
        let row = &snapshot.rows[geometry.row_index];
        let reference = match anchor.reference {
            ViewportAnchorReference::RowTop => row.y,
            ViewportAnchorReference::Baseline => row.baseline,
        };
        reference + anchor.offset_from_reference
    };
    view.layout.set_viewport_top(requested_top)?;
    if anchor.reference == ViewportAnchorReference::Baseline {
        viewport::reveal_anchored_row(view, anchor)?;
    }
    Ok(())
}

fn viewport_extension_needed<P: TextMeasurementProvider>(view: &View<P>) -> (bool, bool) {
    viewport_layout_extension_needed(&view.layout)
}

fn viewport_layout_extension_needed(layout: &ViewLayout) -> (bool, bool) {
    let Some(snapshot) = layout.snapshot() else {
        return (true, true);
    };
    snapshot.missing_viewport_edges(layout.viewport_top(), layout.height())
}

fn update_viewport_anchor<P: TextMeasurementProvider>(document: &Document, view: &mut View<P>) {
    let Some(snapshot) = view.layout.snapshot() else {
        view.viewport_anchor = None;
        return;
    };
    if snapshot.document_id != document.id() || snapshot.document_revision != document.revision() {
        return;
    }
    let viewport_top = view.layout.viewport_top();
    let Some(row) = snapshot
        .rows
        .iter()
        .find(|row| row.y + row.height() > viewport_top)
        .or_else(|| snapshot.rows.last())
    else {
        view.viewport_anchor = None;
        return;
    };
    let Ok(point) = document.text_point(row.text_range.start) else {
        view.viewport_anchor = None;
        return;
    };
    let Ok(anchor) = document.text_anchor(
        point,
        Association::AfterInsertion,
        BoundaryAffinity::Downstream,
        DeletionRecovery::PreferFollowingThenPreceding,
    ) else {
        view.viewport_anchor = None;
        return;
    };
    view.viewport_anchor = Some(ViewportTextAnchor {
        anchor,
        offset_from_reference: viewport_top - row.y,
        reference: ViewportAnchorReference::RowTop,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::composition::CompositionUpdate;
    use crate::command::layout_motion::LayoutDemandEdge;
    use crate::command::{CommandStatus, Key, Mode};
    use crate::document::{Encoding, Format, Revision};
    use crate::layout::{
        BoundaryAffinity, HardLineLayoutRegion, LayoutCoverage, MeasurementError,
        MetricsGeneration, MockTextMeasurementProvider, ProviderThreading, RenderRunPolicy,
        ShapeRequest, ShapedFragment,
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn html_open_line_post_edit_rebase_failure_rolls_back_model_and_all_views() {
        for inactive_failure in [false, true] {
            let original = b"<p data-keep='x'>A\n\nB</p><!--keep-->".to_vec();
            let mut document =
                Document::from_bytes(original.clone(), Encoding::Utf8, Format::Html).unwrap();
            // Keep an existing redo branch: rollback must remove the tentative
            // node and restore branch preference, not synthesize an undo.
            document.insert(0, "X").unwrap();
            let redo_source = document.source_bytes();
            assert!(document.undo());
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 20., 300.);
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document.revision(),
                    text_offset: 2,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            if inactive_failure {
                let other = core.add_view(MockTextMeasurementProvider::new(), 200., 300.);
                core.handle(
                    other,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document.revision(),
                        text_offset: 2,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
            } else {
                // Active cursor mapping succeeds; a retained mark will expose
                // the invalid rebase while capturing history restoration.
                core.handle(view, CoreEvent::Input(InputEvent::key('m')))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::key('a')))
                    .unwrap();
            }
            let revision = core.document.revision();
            let prepared = core
                .document
                .prepare_model_request(ModelRequest::OpenLine {
                    document: core.document.id(),
                    revision,
                    at: 2,
                    origin: 2,
                    after: true,
                })
                .unwrap();
            // Model a defective post-edit map that retains the old offset 2.
            // Its dimensions/revisions are correct, but that offset is inside
            // NBSP in the real candidate "A\u{a0}\nB". This is injected only
            // after the actual source transaction has committed.
            core.input_position_map_override = Some(
                PositionMap::for_text(
                    core.document.id(),
                    revision,
                    prepared.after_revision(),
                    "A B",
                    "A Bxy",
                    vec![crate::document::Splice::new(3..3, 2).unwrap()],
                )
                .unwrap(),
            );
            let history = core.document.history_status();
            let node = core
                .document
                .history_node_details(history.current.node)
                .unwrap();
            let controllers = core
                .views
                .iter()
                .map(|(id, view)| {
                    (
                        *id,
                        format!(
                            "{:?}",
                            view.commands
                                .capture_position_anchors(&core.document)
                                .unwrap()
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let group = (
                core.document.edit_group_depth(),
                core.document.edit_group_generation(),
            );
            let error = core
                .handle(view, CoreEvent::Input(InputEvent::key('O')))
                .unwrap_err();
            assert_eq!(
                error,
                CoreError::Document(DocumentError::NotGraphemeBoundary(2))
            );
            assert_eq!(core.document.source_bytes(), original);
            assert_eq!(core.document.text(), "A B");
            assert_eq!(core.document.revision(), revision);
            assert_eq!(core.document.history_status(), history);
            assert_eq!(
                core.document
                    .history_node_details(history.current.node)
                    .unwrap(),
                node
            );
            assert_eq!(
                (
                    core.document.edit_group_depth(),
                    core.document.edit_group_generation()
                ),
                group
            );
            assert_eq!(core.views[&view].commands.mode(), Mode::Normal);
            assert_eq!(core.views[&view].commands.cursor(), 2);
            assert_eq!(core.edit_group_owner, None);
            for (id, anchors) in controllers {
                assert_eq!(
                    format!(
                        "{:?}",
                        core.views[&id]
                            .commands
                            .capture_position_anchors(&core.document)
                            .unwrap()
                    ),
                    anchors
                );
            }
            // The old branch still works, and a retry uses the unconsumed
            // revision and opens one ordinary Insert undo unit.
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                .unwrap();
            assert_eq!(core.document.source_bytes(), redo_source);
            core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                .unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::key('O')))
                .unwrap();
            assert_eq!(core.document.revision(), prepared.after_revision());
            assert_eq!(core.views[&view].commands.mode(), Mode::Insert);
            core.document
                .text_point(core.views[&view].commands.cursor())
                .unwrap();
        }
    }

    #[derive(Clone, Debug)]
    struct ControlledFailureProvider {
        inner: MockTextMeasurementProvider,
        fail_next: Arc<AtomicBool>,
        invalidate_during_shape: Arc<AtomicBool>,
        metric_changes_after_shape: Arc<AtomicUsize>,
        shape_calls: Arc<AtomicUsize>,
        generation: Arc<AtomicU64>,
    }

    impl ControlledFailureProvider {
        fn new() -> (Self, Arc<AtomicBool>) {
            let (provider, fail_next, _, _) = Self::new_counted();
            (provider, fail_next)
        }

        fn new_counted() -> (Self, Arc<AtomicBool>, Arc<AtomicUsize>, Arc<AtomicU64>) {
            let fail_next = Arc::new(AtomicBool::new(false));
            let shape_calls = Arc::new(AtomicUsize::new(0));
            let generation = Arc::new(AtomicU64::new(1));
            (
                Self {
                    inner: MockTextMeasurementProvider::new(),
                    fail_next: Arc::clone(&fail_next),
                    invalidate_during_shape: Arc::new(AtomicBool::new(false)),
                    metric_changes_after_shape: Arc::new(AtomicUsize::new(0)),
                    shape_calls: Arc::clone(&shape_calls),
                    generation: Arc::clone(&generation),
                },
                fail_next,
                shape_calls,
                generation,
            )
        }
    }

    impl TextMeasurementProvider for ControlledFailureProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.inner.measurement_environment_id()
        }

        fn metrics_generation(&self) -> MetricsGeneration {
            MetricsGeneration(self.generation.load(Ordering::Acquire))
        }

        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.inner.render_run_policy()
        }

        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            self.shape_calls.fetch_add(1, Ordering::AcqRel);
            if self.invalidate_during_shape.swap(false, Ordering::AcqRel) {
                self.generation.fetch_add(1, Ordering::AcqRel);
                return Err(MeasurementError::Provider(
                    "font registration during shaping".into(),
                ));
            }
            if self.fail_next.swap(false, Ordering::AcqRel) {
                return Err(MeasurementError::Provider(
                    "injected post-commit failure".to_owned(),
                ));
            }
            let generation = self.metrics_generation();
            self.inner.set_metrics_generation(generation);
            let result = self.inner.shape_batch(requests);
            if self
                .metric_changes_after_shape
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                self.generation.fetch_add(1, Ordering::AcqRel);
            }
            result
        }
    }

    #[derive(Clone, Debug)]
    struct InstrumentedCoordinatorProvider {
        inner: MockTextMeasurementProvider,
        shaped_bytes: Arc<AtomicUsize>,
        shape_calls: Arc<AtomicUsize>,
        generation: Arc<AtomicU64>,
        threading: ProviderThreading,
        metric_scale: f32,
    }

    impl InstrumentedCoordinatorProvider {
        fn new(
            threading: ProviderThreading,
        ) -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicU64>) {
            let shaped_bytes = Arc::new(AtomicUsize::new(0));
            let shape_calls = Arc::new(AtomicUsize::new(0));
            let generation = Arc::new(AtomicU64::new(1));
            (
                Self {
                    inner: MockTextMeasurementProvider::new(),
                    shaped_bytes: Arc::clone(&shaped_bytes),
                    shape_calls: Arc::clone(&shape_calls),
                    generation: Arc::clone(&generation),
                    threading,
                    metric_scale: 1.0,
                },
                shaped_bytes,
                shape_calls,
                generation,
            )
        }

        fn with_metric_scale(mut self, metric_scale: f32) -> Self {
            self.metric_scale = metric_scale;
            self
        }
    }

    impl TextMeasurementProvider for InstrumentedCoordinatorProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.inner.measurement_environment_id()
        }

        fn metrics_generation(&self) -> MetricsGeneration {
            MetricsGeneration(self.generation.load(Ordering::Acquire))
        }

        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.inner.render_run_policy()
        }

        fn threading(&self) -> ProviderThreading {
            self.threading
        }

        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            self.shape_calls.fetch_add(1, Ordering::AcqRel);
            self.shaped_bytes.fetch_add(
                requests
                    .iter()
                    .map(|request| {
                        request.text.len()
                            + request.context_before.len()
                            + request.context_after.len()
                    })
                    .sum::<usize>(),
                Ordering::AcqRel,
            );
            let generation = self.metrics_generation();
            self.inner.set_metrics_generation(generation);
            let mut responses = self.inner.shape_batch(requests)?;
            if self.metric_scale != 1.0 {
                for response in &mut responses {
                    response.default_metrics.ascent *= self.metric_scale;
                    response.default_metrics.descent *= self.metric_scale;
                    response.default_metrics.leading *= self.metric_scale;
                    for cluster in &mut response.clusters {
                        cluster.metrics.ascent *= self.metric_scale;
                        cluster.metrics.descent *= self.metric_scale;
                        cluster.metrics.leading *= self.metric_scale;
                    }
                }
            }
            Ok(responses)
        }
    }

    fn key(character: char) -> CoreEvent {
        CoreEvent::Input(InputEvent::Key(Key::Char(character)))
    }

    fn text(value: &str) -> CoreEvent {
        CoreEvent::Input(InputEvent::Text(value.to_owned()))
    }

    fn begin_composition<P: TextMeasurementProvider>(
        core: &Core<P>,
        range: std::ops::Range<usize>,
    ) -> CoreEvent {
        CoreEvent::Composition(CompositionEvent::Begin(
            crate::command::composition::CompositionTarget::at_offsets(core.document(), range)
                .unwrap(),
        ))
    }

    #[test]
    fn a_view_receives_an_initial_exact_layout() {
        let mut core = Core::new(Document::new("hello world"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().expect("initial layout");
        assert_eq!(snapshot.document_revision, core.document().revision());
        assert!(!snapshot.rows.is_empty());
        assert!(matches!(
            snapshot.coverage,
            LayoutCoverage::PartialHardLines { .. }
        ));
    }

    #[test]
    fn immediate_viewport_focus_uses_semantic_hard_lines_with_literal_lf_content() {
        let first_line = "x\n".repeat(100) + "z";
        let source = format!("{first_line}\rtail");
        let document = Document::from_bytes_with_file_format(
            source.into_bytes(),
            Encoding::Utf8,
            Format::PlainText,
            crate::document::FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.line_count(), 2);
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 20_000.0, 48.0);
        let focus = first_line.len() - 1;
        {
            let document = &core.document;
            let view = core.views.get_mut(&view).unwrap();
            assert!(view.commands.set_cursor(document, focus));
        }

        core.materialize_immediate_viewport(view, ImmediateLayoutIntent::RevealCaret)
            .unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.coverage.hard_lines().contains(&0));
        assert!(snapshot
            .rows
            .iter()
            .any(|row| row.hard_line_index == 0 && row.hard_line_range == (0..first_line.len())));
        assert!(snapshot
            .caret_point(focus, BoundaryAffinity::Downstream)
            .is_ok());
    }

    #[test]
    fn insert_undo_and_redo_flow_through_one_coordinator() {
        let mut core = Core::new(Document::new(""));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(view, key('i')).unwrap();
        let inserted = core.handle(view, text("héllo")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert!(inserted.document_changed);
        let position_map = inserted
            .position_map
            .as_ref()
            .expect("a committed input publishes its exact position map");
        assert_eq!(position_map.source_revision(), Revision(0));
        assert_eq!(position_map.target_revision(), core.document().revision());
        assert_eq!(core.document().text(), "héllo");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision()
        );

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "héllo");
    }

    #[test]
    fn views_share_the_buffer_but_keep_cursor_and_layout_state_local() {
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 80.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);

        core.handle(first, key('l')).unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 1);
        assert_eq!(core.command_state(second).unwrap().cursor(), 0);
        assert_ne!(
            core.layout(first).unwrap().width(),
            core.layout(second).unwrap().width()
        );

        core.handle(first, key('i')).unwrap();
        core.handle(first, text("X")).unwrap();
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "aXbc");
        assert_eq!(
            core.layout(second)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision(),
            "a shared edit rematerializes each visible view against the new revision"
        );
    }

    #[test]
    fn inactive_visual_block_rebases_across_an_insertion_before_it() {
        let mut core = Core::new(Document::new("head\nabcd\nefgh\nijkl"));
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        {
            let document = &core.document;
            assert!(core
                .views
                .get_mut(&reader)
                .unwrap()
                .commands
                .set_cursor(document, 5));
        }

        for event in [
            CoreEvent::Input(InputEvent::Key(Key::Ctrl('v'))),
            key('l'),
            key('j'),
        ] {
            core.handle(reader, event).unwrap();
        }
        assert_eq!(
            core.command_state(reader).unwrap().mode(),
            Mode::VisualBlock
        );

        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text("XX")).unwrap();
        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        let reader_state = core.command_state(reader).unwrap();
        assert_eq!(reader_state.mode(), Mode::VisualBlock);
        assert!(reader_state.visual_block().is_some());
        assert_eq!(reader_state.visual_block_rebind_error(), None);
        core.handle(reader, key('y')).unwrap();
        let register = core
            .command_state(reader)
            .unwrap()
            .register('0')
            .expect("Visual Block yank publishes register zero");
        assert_eq!(register.kind, crate::command::RegisterKind::Blockwise);
        assert_eq!(register.text, "ab\nef");
        assert_eq!(core.document().text(), "XXhead\nabcd\nefgh\nijkl");
    }

    #[test]
    fn inactive_visual_block_rebases_an_edit_inside_its_rectangle() {
        let mut core = Core::new(Document::new("abcd\nefgh\nijkl"));
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for event in [
            CoreEvent::Input(InputEvent::Key(Key::Ctrl('v'))),
            key('l'),
            key('j'),
        ] {
            core.handle(reader, event).unwrap();
        }
        {
            let document = &core.document;
            assert!(core
                .views
                .get_mut(&writer)
                .unwrap()
                .commands
                .set_cursor(document, 1));
        }
        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text("Z")).unwrap();
        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        assert_eq!(
            core.command_state(reader).unwrap().mode(),
            Mode::VisualBlock
        );
        assert!(core.command_state(reader).unwrap().visual_block().is_some());
        let deleted = core.handle(reader, key('d')).unwrap();
        assert!(deleted.document_changed);
        assert_eq!(core.document().text(), "bcd\ngh\nijkl");
        let register = core
            .command_state(reader)
            .unwrap()
            .register('1')
            .expect("Visual Block delete publishes register one");
        assert_eq!(register.kind, crate::command::RegisterKind::Blockwise);
        assert_eq!(register.text, "aZ\nef");
    }

    #[test]
    fn resize_rebinds_a_wrapped_proportional_visual_block_before_editing() {
        let source = "iiii WWWW one two three four five";
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 45.0, 100.0);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() >= 3);
        for event in [
            CoreEvent::Input(InputEvent::Key(Key::Ctrl('v'))),
            key('l'),
            key('2'),
            key('j'),
        ] {
            core.handle(view, event).unwrap();
        }

        core.handle(
            view,
            CoreEvent::Resize {
                width: 70.0,
                height: 100.0,
            },
        )
        .unwrap();
        let state = core.command_state(view).unwrap();
        assert_eq!(state.mode(), Mode::VisualBlock);
        assert_eq!(state.visual_block_rebind_error(), None);
        let selection = state
            .visual_block()
            .expect("resize installs freshly rebound block geometry");
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(selection.anchor.layout_revision, snapshot.revision);
        assert_eq!(selection.active.layout_revision, snapshot.revision);
        let resolved = crate::command::visual_block::resolve_block_selection(
            selection,
            snapshot,
            core.document().text(),
        )
        .unwrap();
        let expected_register = resolved
            .rows
            .iter()
            .map(|row| {
                row.ranges
                    .iter()
                    .map(|range| &core.document().text()[range.clone()])
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut expected_document = core.document().text().to_owned();
        for segment in resolved.range_set.segments.iter().rev() {
            expected_document.replace_range(segment.range.clone(), "");
        }

        let deleted = core.handle(view, key('d')).unwrap();
        assert!(deleted.document_changed);
        assert_eq!(core.document().text(), expected_document);
        let register = core
            .command_state(view)
            .unwrap()
            .register('1')
            .expect("the rebound rectangle is deleted blockwise");
        assert_eq!(register.kind, crate::command::RegisterKind::Blockwise);
        assert_eq!(register.text, expected_register);
    }

    #[test]
    fn registers_and_dot_repeat_are_shared_across_views() {
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(first, key('y')).unwrap();
        core.handle(first, key('l')).unwrap();
        assert_eq!(
            core.command_state(second)
                .unwrap()
                .register('"')
                .unwrap()
                .text,
            "a"
        );
        core.handle(second, key('p')).unwrap();
        assert_eq!(core.document().text(), "aabc");

        let mut core = Core::new(Document::new("one two"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['c', 'w', 'X'] {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(second, key('w')).unwrap();
        core.handle(second, key('.')).unwrap();
        assert_eq!(core.document().text(), "X X");
    }

    #[test]
    fn marks_are_shared_rebased_once_and_compound_assignments_stay_current() {
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for character in ['l', 'm', 'a'] {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(second, key('`')).unwrap();
        core.handle(second, key('a')).unwrap();
        assert_eq!(core.command_state(second).unwrap().cursor(), 1);
        let first_jump = core
            .handle(first, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert!(matches!(
            first_jump.command.unwrap().status,
            CommandStatus::Error(ref message) if message.contains("jump list")
        ));
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert_eq!(
            core.command_state(second).unwrap().cursor(),
            0,
            "the jump made in the second view is absent from the first"
        );

        core.handle(first, key('0')).unwrap();
        core.handle(second, key('0')).unwrap();
        core.handle(second, key('i')).unwrap();
        core.handle(second, text("X")).unwrap();
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(first, key('0')).unwrap();
        core.handle(first, key('`')).unwrap();
        core.handle(first, key('a')).unwrap();
        assert_eq!(
            core.command_state(first).unwrap().cursor(),
            2,
            "the shared mark follows the original b through exactly one map"
        );

        // Record an edit followed by a mark assignment. During macro replay,
        // that assignment is already expressed in the outer command's target
        // snapshot and must not be mapped a second time.
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for event in [
            key('q'),
            key('a'),
            key('i'),
            text("X"),
            CoreEvent::Input(InputEvent::Key(Key::Escape)),
            key('l'),
            key('m'),
            key('a'),
            key('q'),
        ] {
            core.handle(first, event).unwrap();
        }
        core.handle(first, key('u')).unwrap();
        core.handle(second, key('l')).unwrap();
        core.handle(second, key('@')).unwrap();
        core.handle(second, key('a')).unwrap();
        assert_eq!(core.document().text(), "aXbc");
        core.handle(first, key('0')).unwrap();
        core.handle(first, key('`')).unwrap();
        core.handle(first, key('a')).unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 2);
    }

    #[test]
    fn search_and_substitute_histories_are_shared_across_views() {
        let mut core = Core::new(Document::new("one two one"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for character in "/one".chars() {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 8);
        core.handle(second, key('/')).unwrap();
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Up)))
            .unwrap();
        assert_eq!(
            core.command_state(second).unwrap().command_line(),
            Some("one")
        );
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(second, key('n')).unwrap();
        assert_eq!(core.command_state(second).unwrap().cursor(), 8);
        core.handle(second, key('N')).unwrap();
        assert_eq!(core.command_state(second).unwrap().cursor(), 0);

        let mut core = Core::new(Document::new("a\na"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ":s/a/A/".chars() {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(core.document().text(), "A\na");

        core.handle(second, key(':')).unwrap();
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Up)))
            .unwrap();
        assert_eq!(
            core.command_state(second).unwrap().command_line(),
            Some("s/a/A/")
        );
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(second, key('j')).unwrap();
        assert_eq!(
            core.command_state(second).unwrap().cursor(),
            2,
            "j reaches the second visual row"
        );
        for character in ":&".chars() {
            core.handle(second, key(character)).unwrap();
        }
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(core.document().text(), "A\nA");

        for character in ":set fileformats=mac".chars() {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(
            core.command_state(second).unwrap().fileformats_option(),
            &[crate::document::FileFormat::Mac]
        );
    }

    #[test]
    fn composition_rebased_marks_use_buffer_state() {
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['l', 'm', 'a'] {
            core.handle(first, key(character)).unwrap();
        }
        core.handle(second, begin_composition(&core, 0..0)).unwrap();
        core.handle(
            second,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
                marked_text: "X".to_owned(),
                selected_range: 1..1,
            })),
        )
        .unwrap();
        core.handle(second, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        core.handle(first, key('0')).unwrap();
        core.handle(first, key('`')).unwrap();
        core.handle(first, key('a')).unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 2);
    }

    #[test]
    fn composition_commits_use_shared_ordinary_undo_and_redo() {
        let mut core = Core::new(Document::new("abc\ndef"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for (range, replacement) in [(1..2, "X"), (2..3, "Y")] {
            core.handle(writer, begin_composition(&core, range))
                .unwrap();
            core.handle(
                writer,
                CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
                    marked_text: replacement.to_owned(),
                    selected_range: replacement.len()..replacement.len(),
                })),
            )
            .unwrap();
            core.handle(writer, CoreEvent::Composition(CompositionEvent::Commit))
                .unwrap();
        }
        assert_eq!(core.document().text(), "aXY\ndef");

        core.handle(reader, key('j')).unwrap();
        core.handle(reader, key('2')).unwrap();
        core.handle(reader, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc\ndef");
        core.handle(writer, key('2')).unwrap();
        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
        assert_eq!(core.document().text(), "aXY\ndef");
    }

    #[test]
    fn failed_input_does_not_publish_partial_buffer_state() {
        let document =
            Document::from_bytes(b"ab".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let mut core = Core::new(document);
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        core.handle(first, key('y')).unwrap();
        core.handle(first, key('l')).unwrap();
        core.handle(second, key('R')).unwrap();

        let result = core.handle(second, text("😀"));
        assert!(matches!(
            result,
            Err(CoreError::Document(
                DocumentError::UnrepresentableCharacter {
                    encoding: Encoding::Latin1,
                    ..
                }
            ))
        ));
        assert_eq!(core.document().text(), "ab");
        assert_eq!(
            core.command_state(first)
                .unwrap()
                .register('"')
                .unwrap()
                .text,
            "a"
        );
        assert_eq!(
            core.command_state(second)
                .unwrap()
                .register('"')
                .unwrap()
                .text,
            "a"
        );
    }

    #[test]
    fn macro_registers_are_shared_while_view_interaction_state_stays_local() {
        let mut core = Core::new(Document::new("abcd"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 80.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);

        for character in ['q', 'a', 'x', 'q'] {
            core.handle(first, key(character)).unwrap();
        }
        assert_eq!(core.document().text(), "bcd");
        core.handle(second, key('@')).unwrap();
        core.handle(second, key('a')).unwrap();
        assert_eq!(core.document().text(), "cd");

        core.handle(first, key('v')).unwrap();
        core.handle(first, key('l')).unwrap();
        assert_eq!(
            core.command_state(first).unwrap().mode(),
            Mode::VisualCharacter
        );
        assert_eq!(core.command_state(second).unwrap().mode(), Mode::Normal);
        assert!(core.command_state(first).unwrap().visual_anchor().is_some());
        assert!(core
            .command_state(second)
            .unwrap()
            .visual_anchor()
            .is_none());

        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(first, key(':')).unwrap();
        core.handle(first, text("set")).unwrap();
        assert_eq!(
            core.command_state(first).unwrap().command_line(),
            Some("set")
        );
        assert_eq!(core.command_state(second).unwrap().command_line(), None);

        core.handle(first, CoreEvent::SetWrap(false)).unwrap();
        assert!(!core.layout(first).unwrap().wrap());
        assert!(core.layout(second).unwrap().wrap());
        assert_ne!(
            core.command_state(first).unwrap().cursor(),
            core.command_state(second).unwrap().cursor()
        );
    }

    #[test]
    fn resize_changes_only_the_target_view_layout() {
        let mut core = Core::new(Document::new("one two three four"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let document_revision = core.document().revision();
        let old_layout_revision = core.layout(view).unwrap().snapshot().unwrap().revision;

        let outcome = core
            .handle(
                view,
                CoreEvent::Resize {
                    width: 40.0,
                    height: 100.0,
                },
            )
            .unwrap();
        assert!(!outcome.document_changed);
        assert!(outcome.layout_changed);
        assert_eq!(core.document().revision(), document_revision);
        assert_ne!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            old_layout_revision
        );
    }

    #[test]
    fn native_file_format_is_shared_standalone_history_and_preserves_group_owner() {
        let document = Document::from_bytes_with_file_format(
            b"a\nb".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::new(document);
        let owner = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let invoker = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(owner, key('i')).unwrap();
        core.handle(owner, text("x")).unwrap();
        let text_revision = core.document().revision();
        assert_eq!(core.edit_group_owner, Some(owner));

        let outcome = core
            .handle(
                invoker,
                CoreEvent::SetFileFormat {
                    document: core.document().id(),
                    revision: text_revision,
                    target: FileFormat::Dos,
                },
            )
            .unwrap();
        assert!(outcome.document_changed);
        assert!(outcome.position_map.is_some());
        assert_eq!(core.document().file_format(), FileFormat::Dos);
        assert_eq!(core.document().source_bytes(), b"xa\r\nb");
        assert_eq!(core.document().history_status().node_count, 3);
        assert_eq!(core.edit_group_owner, None);
        assert_eq!(core.command_state(owner).unwrap().mode(), Mode::Insert);

        core.handle(
            owner,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().file_format(), FileFormat::Unix);
        assert_eq!(core.document().text(), "xa\nb");
        core.handle(
            owner,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().text(), "a\nb");
        assert_eq!(core.command_state(owner).unwrap().cursor(), 0);
    }

    #[test]
    fn markdown_semantic_style_query_and_event_are_exact_and_undoable() {
        let document = Document::from_bytes_with_file_format(
            b"alpha beta".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        core.handle(view, key('v')).unwrap();
        core.handle(view, key('4')).unwrap();
        core.handle(view, key('l')).unwrap();

        let before = core
            .selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
            .unwrap();
        assert_eq!(before.selection_kind(), LogicalSelectionKind::Character);
        assert_eq!(before.state(), SemanticStyleState::Off);
        assert!(before.can_set());
        assert!(!before.can_clear());
        let selection = before.selection().unwrap().clone();
        assert_eq!(selection.range(), 0..5);
        let history_before = core.document().history_status().node_count;
        let first_layout = core.layout(view).unwrap().snapshot().unwrap().revision;

        let outcome = core
            .handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected: selection,
                    style: SemanticInlineStyle::Strong,
                    enabled: true,
                },
            )
            .unwrap();
        assert!(outcome.document_changed);
        assert!(outcome.layout_changed);
        assert_eq!(core.document().text(), "alpha beta");
        assert_eq!(core.document().source_bytes(), b"**alpha** beta");
        assert_eq!(
            core.document().history_status().node_count,
            history_before + 1
        );
        assert_eq!(
            core.command_state(view).unwrap().mode(),
            Mode::VisualCharacter
        );
        assert_eq!(
            core.selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
                .unwrap()
                .state(),
            SemanticStyleState::On
        );
        assert_ne!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            first_layout
        );

        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), b"alpha beta");
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), b"**alpha** beta");
    }

    #[test]
    fn semantic_style_rejects_stale_or_non_linear_selection_without_mutation() {
        let document = Document::from_bytes_with_file_format(
            b"**bold** plain".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        core.handle(view, key('v')).unwrap();
        core.handle(view, key('3')).unwrap();
        core.handle(view, key('l')).unwrap();
        let exact = core
            .selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
            .unwrap();
        assert_eq!(exact.state(), SemanticStyleState::On);
        assert!(exact.can_clear());
        let stale = exact.selection().unwrap().clone();
        core.handle(view, key('l')).unwrap();
        let source = core.document().source_bytes();
        let revision = core.document().revision();
        assert_eq!(
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected: stale,
                    style: SemanticInlineStyle::Strong,
                    enabled: false,
                },
            ),
            Err(CoreError::StaleLogicalSelection)
        );
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);

        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('v'))))
            .unwrap();
        let block = core
            .selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
            .unwrap();
        assert_eq!(block.selection_kind(), LogicalSelectionKind::Block);
        assert!(block.selection().is_none());
        assert!(!block.can_set());
        assert!(!block.can_clear());
    }

    #[test]
    fn semantic_style_presentation_reports_plain_disabled_and_markdown_mixed() {
        let mut plain = Core::new(Document::new("plain"));
        let plain_view = plain.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        plain.handle(plain_view, key('v')).unwrap();
        plain.handle(plain_view, key('4')).unwrap();
        plain.handle(plain_view, key('l')).unwrap();
        let unsupported = plain
            .selection_semantic_style_presentation(plain_view, SemanticInlineStyle::Strong)
            .unwrap();
        assert_eq!(unsupported.state(), SemanticStyleState::Off);
        assert!(!unsupported.can_set());
        assert!(!unsupported.can_clear());

        let document = Document::from_bytes_with_file_format(
            b"**bold** plain".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut markdown = Core::new(document);
        let markdown_view = markdown.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        markdown.handle(markdown_view, key('v')).unwrap();
        markdown.handle(markdown_view, key('9')).unwrap();
        markdown.handle(markdown_view, key('l')).unwrap();
        let mixed = markdown
            .selection_semantic_style_presentation(markdown_view, SemanticInlineStyle::Strong)
            .unwrap();
        assert_eq!(mixed.state(), SemanticStyleState::Mixed);
        assert!(!mixed.can_set());
        assert!(!mixed.can_clear());
    }

    #[test]
    fn rejected_native_file_format_is_atomic_and_leaves_open_group_intact() {
        let document = Document::from_bytes_with_file_format(
            b"a\rb\n".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("x")).unwrap();
        let revision = core.document().revision();
        let source = core.document().source_bytes();
        let group_depth = core.document().edit_group_depth();

        let error = core
            .handle(
                view,
                CoreEvent::SetFileFormat {
                    document: core.document().id(),
                    revision,
                    target: FileFormat::Mac,
                },
            )
            .unwrap_err();
        assert_eq!(
            error,
            CoreError::Document(DocumentError::LineEndingConversionWouldReinterpretContent)
        );
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().edit_group_depth(), group_depth);
        assert_eq!(core.edit_group_owner, Some(view));
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);

        let stale = core
            .handle(
                view,
                CoreEvent::SetFileFormat {
                    document: core.document().id(),
                    revision: Revision(revision.0.saturating_sub(1)),
                    target: FileFormat::Dos,
                },
            )
            .unwrap_err();
        assert!(matches!(
            stale,
            CoreError::Document(DocumentError::WrongSnapshot { .. })
        ));
        assert_eq!(core.document().edit_group_depth(), group_depth);
    }

    #[test]
    fn horizontal_origin_is_independent_presentation_state_and_preserves_active_work() {
        let (provider, _, shape_calls, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::new("WWWWWWWWWWWWWWWW\nindependent"));
        let first = core.add_view(provider.clone(), 40.0, 48.0);
        let second = core.add_view(provider, 40.0, 48.0);
        core.handle(first, CoreEvent::SetWrap(false)).unwrap();
        core.handle(second, CoreEvent::SetWrap(false)).unwrap();

        let before_snapshot = core
            .layout(first)
            .unwrap()
            .snapshot()
            .expect("nowrap layout")
            .clone();
        let before_configuration = core.layout(first).unwrap().configuration_generation();
        let before_calls = shape_calls.load(Ordering::Acquire);
        let cancellation = LayoutCancellationToken::new();
        let request = core
            .prepare_view_layout_job(
                first,
                LayoutJobPriority::Background,
                LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..1).unwrap()),
                cancellation.clone(),
            )
            .unwrap();
        let active_job = request.job_id();

        let outcome = core
            .handle(
                first,
                CoreEvent::SetViewportOrigin {
                    left: 50.0,
                    top: None,
                },
            )
            .unwrap();

        assert!(outcome.layout_changed);
        assert_eq!(core.viewport_state(first).unwrap().left(), 50.0);
        assert_eq!(core.viewport_state(second).unwrap().left(), 0.0);
        assert_eq!(
            core.layout(first).unwrap().configuration_generation(),
            before_configuration
        );
        assert_eq!(
            core.layout(first).unwrap().snapshot().unwrap(),
            &before_snapshot,
            "scroll is not part of immutable document-coordinate geometry"
        );
        assert_eq!(shape_calls.load(Ordering::Acquire), before_calls);
        assert!(!cancellation.is_cancelled());
        assert_eq!(
            core.views
                .get(&first)
                .and_then(|view| view.active_layout_work.as_ref())
                .map(|work| work.job_id),
            Some(active_job)
        );

        core.handle(first, CoreEvent::SetWrap(true)).unwrap();
        let wrapped = core.viewport_state(first).unwrap();
        assert_eq!(wrapped.left(), 0.0);
        assert!(wrapped.maximum_left().unwrap() > 0.0);
        assert!(wrapped.wrap());
    }

    #[test]
    fn viewport_origin_validation_and_vertical_install_are_atomic() {
        let contents = (0..200)
            .map(|line| format!("WWWWWWWWWWWWWWWW {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(MockTextMeasurementProvider::new(), 40.0, 32.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 20.0,
                top: None,
            },
        )
        .unwrap();
        let initial = core.viewport_state(view).unwrap();
        let snapshot_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        let cursor = core.command_state(view).unwrap().cursor();

        assert_eq!(
            core.handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: f32::NAN,
                    top: Some(10.0),
                },
            ),
            Err(CoreError::Layout(LayoutError::InvalidGeometry))
        );
        assert_eq!(
            core.handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 30.0,
                    top: Some(f32::INFINITY),
                },
            ),
            Err(CoreError::Layout(LayoutError::InvalidGeometry))
        );
        assert_eq!(core.viewport_state(view).unwrap(), initial);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            snapshot_revision
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);

        let outcome = core
            .handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 30.0,
                    top: Some(800.0),
                },
            )
            .unwrap();
        let scrolled = core.viewport_state(view).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let vertical = snapshot.coverage.vertical_range().unwrap();
        assert!(outcome.layout_changed);
        assert_eq!(scrolled.left(), 30.0);
        assert!(vertical.start <= scrolled.top());
        assert!(scrolled.top() + core.layout(view).unwrap().height() <= vertical.end);
        assert_ne!(snapshot.revision, snapshot_revision);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);

        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 10.0,
                top: Some(-100.0),
            },
        )
        .unwrap();
        let clamped = core.viewport_state(view).unwrap();
        assert_eq!(clamped.left(), 10.0);
        assert_eq!(clamped.top(), 0.0);
        assert_eq!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .coverage
                .hard_lines()
                .start,
            0
        );
    }

    #[test]
    fn far_vertical_scrolls_are_bounded_and_do_not_layout_the_caret_gap() {
        const LINE_COUNT: usize = 1_000_000;
        let (provider, shaped_bytes, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::layout_test_with_line_count(LINE_COUNT));
        let view = core.add_view(provider, 320.0, 96.0);
        let cursor = core.command_state(view).unwrap().cursor();

        for requested_top in [12_000_000.0, 14_000_000.0, 1_000_000.0] {
            let before = shaped_bytes.load(Ordering::Acquire);
            let outcome = core
                .handle(
                    view,
                    CoreEvent::SetViewportOrigin {
                        left: 0.0,
                        top: Some(requested_top),
                    },
                )
                .unwrap();
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let coverage = snapshot.coverage.hard_lines();
            let vertical = snapshot.coverage.vertical_range().unwrap();
            let shaped = shaped_bytes.load(Ordering::Acquire) - before;

            assert!(outcome.layout_changed);
            assert!(matches!(
                snapshot.coverage,
                LayoutCoverage::PartialHardLines { .. }
            ));
            assert!(coverage.start > 10_000, "far scroll stayed near the caret");
            assert!(coverage.end - coverage.start < 100);
            assert!(shaped < 10_000, "far scroll shaped {shaped} bytes");
            assert!(vertical.start <= layout.viewport_top());
            assert!(layout.viewport_top() + layout.height() <= vertical.end);
            assert!(!core.viewport_state(view).unwrap().top_is_exact());
            assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        }

        let before = shaped_bytes.load(Ordering::Acquire);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(f32::MAX),
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        assert_eq!(snapshot.coverage.hard_lines().end, LINE_COUNT);
        assert!(snapshot.coverage.hard_lines().len() < 100);
        assert!(shaped_bytes.load(Ordering::Acquire) - before < 10_000);
        let vertical = snapshot.coverage.vertical_range().unwrap();
        assert_eq!(
            layout.viewport_top(),
            (vertical.end - layout.height()).max(vertical.start)
        );

        let before = shaped_bytes.load(Ordering::Acquire);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(-1.0),
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap();
        assert_eq!(layout.viewport_top(), 0.0);
        assert_eq!(layout.snapshot().unwrap().coverage.hard_lines().start, 0);
        assert!(shaped_bytes.load(Ordering::Acquire) - before < 10_000);
    }

    #[test]
    fn failed_vertical_layout_preserves_snapshot_origin_anchor_and_active_work() {
        let contents = (0..200)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (provider, fail_next, _, _) = ControlledFailureProvider::new_counted();
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(provider, 200.0, 48.0);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 20.0,
                top: None,
            },
        )
        .unwrap();
        let cancellation = LayoutCancellationToken::new();
        let request = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::HardLines(HardLineLayoutRegion::new(100..101).unwrap()),
                cancellation.clone(),
            )
            .unwrap();
        let active_job = request.job_id();
        let before_state = core.viewport_state(view).unwrap();
        let before_layout = core.layout(view).unwrap().clone();
        let before_anchor = core.views.get(&view).unwrap().viewport_anchor;
        let before_cursor = core.command_state(view).unwrap().cursor();

        fail_next.store(true, Ordering::Release);
        assert!(matches!(
            core.handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 40.0,
                    top: Some(1_000.0),
                },
            ),
            Err(CoreError::LayoutJob(LayoutJobError::Layout(
                LayoutError::Measurement(MeasurementError::Provider(message))
            ))) if message == "injected post-commit failure"
        ));

        assert_eq!(core.viewport_state(view).unwrap(), before_state);
        assert_eq!(core.layout(view).unwrap(), &before_layout);
        assert_eq!(
            core.views.get(&view).unwrap().viewport_anchor,
            before_anchor
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);
        assert!(!cancellation.is_cancelled());
        assert_eq!(
            core.views
                .get(&view)
                .and_then(|view| view.active_layout_work.as_ref())
                .map(|work| work.job_id),
            Some(active_job)
        );
    }

    #[test]
    fn vertical_scroll_recovers_font_registration_without_replaying_the_request() {
        for fails_during_shape in [true, false] {
            let (provider, _, calls, generation) = ControlledFailureProvider::new_counted();
            let during_shape = Arc::clone(&provider.invalidate_during_shape);
            let after_shape = Arc::clone(&provider.metric_changes_after_shape);
            let source = (0..10_000)
                .map(|line| format!("line {line}: {}\n", "text ".repeat(20)))
                .collect::<String>();
            let mut core = Core::new(Document::new(&source));
            let view = core.add_view(provider, 300.0, 100.0);
            core.handle(view, CoreEvent::SetWrap(false)).unwrap();
            let before_calls = calls.load(Ordering::Acquire);
            let before_cursor = core.command_state(view).unwrap().cursor();
            let target_line = core
                .layout(view)
                .unwrap()
                .hard_line_at_y(100_000.0)
                .unwrap()
                .unwrap()
                .hard_line();
            if fails_during_shape {
                during_shape.store(true, Ordering::Release);
            } else {
                // Font notifications can arrive after a successful callback,
                // before the staged viewport is checked for publication.
                after_shape.store(1, Ordering::Release);
            }

            let outcome = core
                .handle(
                    view,
                    CoreEvent::SetViewportOrigin {
                        left: 25.0,
                        top: Some(100_000.0),
                    },
                )
                .unwrap();
            assert!(outcome.layout_changed);
            assert!(!outcome.document_changed);
            let viewport = core.viewport_state(view).unwrap();
            assert_eq!(viewport.left(), 25.0);
            assert_eq!(
                core.layout(view)
                    .unwrap()
                    .hard_line_at_y(f64::from(viewport.top()))
                    .unwrap()
                    .unwrap()
                    .hard_line(),
                target_line,
                "metric retry must preserve the requested text location"
            );
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
            assert_eq!(generation.load(Ordering::Acquire), 2);
            assert!(snapshot.rows.len() < 100, "retry must keep shaping local");
            assert!(calls.load(Ordering::Acquire) - before_calls <= 3);
            assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_eq!(core.document().revision(), Revision(0));
            assert!(!core.document.undo());
        }
    }

    #[test]
    fn vertical_scroll_metric_retry_is_bounded_and_keeps_failed_install_atomic() {
        let (provider, _, calls, generation) = ControlledFailureProvider::new_counted();
        let changes = Arc::clone(&provider.metric_changes_after_shape);
        let source = (0..10_000)
            .map(|line| format!("line {line}: {}\n", "text ".repeat(20)))
            .collect::<String>();
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(provider, 300.0, 100.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        let before_layout = core.layout(view).unwrap().clone();
        let before_anchor = core.views.get(&view).unwrap().viewport_anchor;
        let before_cursor = core.command_state(view).unwrap().cursor();
        let before_calls = calls.load(Ordering::Acquire);
        changes.store(10, Ordering::Release);

        assert!(core
            .handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 25.0,
                    top: Some(100_000.0),
                },
            )
            .is_err());
        assert_eq!(calls.load(Ordering::Acquire) - before_calls, 3);
        assert_eq!(generation.load(Ordering::Acquire), 4);
        assert_eq!(core.layout(view).unwrap(), &before_layout);
        assert_eq!(core.views.get(&view).unwrap().viewport_anchor, before_anchor);
        assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);
        assert_eq!(core.document().revision(), Revision(0));
        assert!(!core.document.undo());
    }

    #[test]
    fn vertical_scroll_metric_retry_preserves_wrapped_target_and_local_offset() {
        let (provider, _, _, generation) = ControlledFailureProvider::new_counted();
        let changes = Arc::clone(&provider.metric_changes_after_shape);
        let source = (0..10_000)
            .map(|line| format!("line {line}: {}\n", "wrapped text ".repeat(20)))
            .collect::<String>();
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(provider, 300.0, 100.0);
        let before = core.layout(view).unwrap();
        assert!(before.wrap());
        assert!(
            before.hard_line_range_height(0..1).unwrap().height() > 32.0,
            "the exact wrapped prefix must differ substantially from estimates"
        );
        let target_line = 4_000;
        let requested_top =
            before.hard_line_prefix_height(target_line).unwrap().height() as f32 + 5.0;
        let original_hit = before
            .hard_line_at_y(f64::from(requested_top))
            .unwrap()
            .unwrap();
        assert_eq!(original_hit.hard_line(), target_line);
        let original_offset = f64::from(requested_top) - original_hit.line_top();
        changes.store(1, Ordering::Release);

        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(requested_top),
            },
        )
        .unwrap();

        let after = core.layout(view).unwrap();
        let actual_hit = after
            .hard_line_at_y(f64::from(after.viewport_top()))
            .unwrap()
            .unwrap();
        assert_eq!(generation.load(Ordering::Acquire), 2);
        assert_eq!(actual_hit.hard_line(), target_line);
        assert!(
            (f64::from(after.viewport_top()) - actual_hit.line_top() - original_offset).abs()
                < 0.01,
            "retry must retain the offset within the originally requested hard line"
        );
        assert!(after.snapshot().unwrap().coverage.hard_lines().len() < 100);
    }

    #[test]
    fn vertical_scroll_metric_retry_keeps_a_deep_wrapped_offset_local() {
        let (provider, _, _, generation) = ControlledFailureProvider::new_counted();
        let changes = Arc::clone(&provider.metric_changes_after_shape);
        let source = format!(
            "{}\n{}",
            "wrapped word ".repeat(1_000),
            "short line\n".repeat(10_000)
        );
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(provider, 300.0, 100.0);
        let before = core.layout(view).unwrap();
        let wrapped_height = before.hard_line_range_height(0..1).unwrap().height();
        assert!(wrapped_height > 1_000.0);
        let requested_top = (wrapped_height * 0.75).floor() as f32 + 5.0;
        assert_eq!(
            before
                .hard_line_at_y(f64::from(requested_top))
                .unwrap()
                .unwrap()
                .hard_line(),
            0
        );
        // Move the installed viewport away first: scrolling inside its exact
        // coverage now correctly bypasses shaping altogether. Evict disposable
        // geometry and shapes while retaining this paragraph's exact height.
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(100_000.0) }).unwrap();
        core.views.get_mut(&view).unwrap().layout.set_regional_cache_limits(
            crate::layout::RegionalLayoutCacheLimits { max_hard_lines: 0, ..Default::default() });
        core.views.get_mut(&view).unwrap().engine.clear_caches();
        changes.store(1, Ordering::Release);

        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(requested_top),
            },
        )
        .unwrap();

        let after = core.layout(view).unwrap();
        let actual_hit = after
            .hard_line_at_y(f64::from(after.viewport_top()))
            .unwrap()
            .unwrap();
        assert_eq!(generation.load(Ordering::Acquire), 2);
        assert_eq!(actual_hit.hard_line(), 0);
        assert!((after.viewport_top() - requested_top).abs() < 0.01);
        let coverage = after.snapshot().unwrap().coverage.hard_lines();
        assert!(
            coverage.len() < 100,
            "a deep offset in one wrapped line must not expand into unrelated hard lines: {coverage:?}"
        );
    }

    #[test]
    fn wrapped_vertical_motion_uses_each_views_exact_layout() {
        let mut core = Core::new(Document::new("one two three"));
        let narrow = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        let wide = core.add_view(MockTextMeasurementProvider::new(), 500.0, 100.0);
        let narrow_revision = core.layout(narrow).unwrap().snapshot().unwrap().revision;
        let wide_revision = core.layout(wide).unwrap().snapshot().unwrap().revision;

        core.handle(narrow, key('j')).unwrap();
        core.handle(wide, key('j')).unwrap();

        assert_eq!(core.command_state(narrow).unwrap().cursor(), 4);
        assert_eq!(core.command_state(wide).unwrap().cursor(), 0);
        assert_eq!(
            core.layout(narrow).unwrap().snapshot().unwrap().revision,
            narrow_revision,
            "caret motion must not manufacture a new layout revision"
        );
        assert_eq!(
            core.layout(wide).unwrap().snapshot().unwrap().revision,
            wide_revision
        );
        assert_eq!(
            core.layout(narrow)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision()
        );
    }

    #[test]
    fn nowrap_j_uses_hard_lines_even_in_a_narrow_view() {
        let mut core = Core::new(Document::new("one two three\nsecond"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();

        core.handle(view, key('j')).unwrap();

        assert_eq!(core.command_state(view).unwrap().cursor(), 14);
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().rows.len(), 2);
    }

    #[test]
    fn visual_vertical_motion_retains_desired_x() {
        let mut core = Core::new(Document::new("iiii\nWWWW"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 100.0);
        let revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        for _ in 0..3 {
            core.handle(view, key('l')).unwrap();
        }

        core.handle(view, key('j')).unwrap();
        let desired_x = core
            .command_state(view)
            .unwrap()
            .desired_x()
            .expect("vertical motion establishes desired x");
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
        core.handle(view, key('k')).unwrap();

        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        assert_eq!(
            core.command_state(view).unwrap().desired_x(),
            Some(desired_x)
        );
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            revision
        );
    }

    #[test]
    fn g_row_edges_preserve_the_layout_boundary_affinity() {
        let mut core = Core::new(Document::new("one two three"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        core.handle(view, key('g')).unwrap();
        core.handle(view, key('j')).unwrap();
        core.handle(view, key('g')).unwrap();
        core.handle(view, key('$')).unwrap();

        let state = core.command_state(view).unwrap();
        assert_eq!(
            state.visual_position(),
            Some(crate::command::layout_motion::VisualPosition {
                text_offset: 8,
                affinity: BoundaryAffinity::Upstream,
            })
        );
        assert_eq!(
            state.cursor(),
            7,
            "legacy cursor addresses the associated grapheme"
        );
        assert_eq!(state.boundary_affinity(), BoundaryAffinity::Upstream);
    }

    #[test]
    fn page_and_z_commands_persist_scroll_without_relayout() {
        let contents = (0..10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 78.4);
        let revision = core.layout(view).unwrap().snapshot().unwrap().revision;

        let page = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
            .unwrap();
        assert!(page.layout_changed);
        assert!(core.layout(view).unwrap().viewport_top() > 0.0);
        let page_top = core.layout(view).unwrap().viewport_top();

        core.handle(view, key('z')).unwrap();
        core.handle(view, key('z')).unwrap();
        assert_ne!(core.layout(view).unwrap().viewport_top(), page_top);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            revision
        );
    }

    #[test]
    fn insert_page_motion_splits_the_undo_group() {
        let contents = (0..10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 78.4);

        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
            .unwrap();
        core.handle(view, text("Y")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, key('u')).unwrap();
        assert!(core.document().text().starts_with("X0"));
        assert!(!core.document().text().contains('Y'));
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), contents);
    }

    #[test]
    fn inactive_view_is_rematerialized_when_another_view_commits() {
        let mut core = Core::new(Document::new("one two three"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        let old_revision = core.layout(reader).unwrap().snapshot().unwrap().revision;

        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text("X")).unwrap();
        assert_eq!(
            core.layout(reader)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision()
        );

        let outcome = core.handle(reader, key('j')).unwrap();
        assert!(outcome.layout_changed);
        assert_ne!(
            core.layout(reader).unwrap().snapshot().unwrap().revision,
            old_revision
        );
        assert_eq!(
            core.layout(reader)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision()
        );
    }

    #[test]
    fn inactive_visual_endpoints_rebase_as_one_revision_bound_selection() {
        let mut core = Core::new(Document::new("abcd"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(reader, key('l')).unwrap();
        core.handle(reader, key('v')).unwrap();
        core.handle(reader, key('l')).unwrap();
        assert_eq!(core.command_state(reader).unwrap().visual_anchor(), Some(1));
        assert_eq!(core.command_state(reader).unwrap().cursor(), 2);

        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text("X")).unwrap();
        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        let reader_state = core.command_state(reader).unwrap();
        assert_eq!(reader_state.visual_anchor(), Some(2));
        assert_eq!(reader_state.cursor(), 3);
        core.handle(reader, key('d')).unwrap();
        assert_eq!(core.document().text(), "Xad");
    }

    #[test]
    fn discontiguous_ex_edits_preserve_an_inactive_cursor_between_splices() {
        let mut core = Core::new(Document::new("a\nmiddle\na"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(reader, key('j')).unwrap();
        assert_eq!(core.command_state(reader).unwrap().cursor(), 2);

        for character in ":%s/a/A/g".chars() {
            core.handle(writer, key(character)).unwrap();
        }
        let outcome = core
            .handle(writer, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();

        assert!(outcome.document_changed);
        assert_eq!(core.document().text(), "A\nmiddle\nA");
        assert_eq!(
            core.command_state(reader).unwrap().cursor(),
            2,
            "the exact transaction map must not collapse untouched middle text"
        );
    }

    #[test]
    fn discontiguous_visual_block_edits_preserve_inactive_positions_between_rows() {
        let mut core = Core::new(Document::new("a\nmiddle\nb"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(reader, key('j')).unwrap();
        for _ in 0..3 {
            core.handle(reader, key('l')).unwrap();
        }
        assert_eq!(core.command_state(reader).unwrap().cursor(), 5);

        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Ctrl('v'))))
            .unwrap();
        core.handle(writer, key('j')).unwrap();
        core.handle(writer, key('j')).unwrap();
        let outcome = core.handle(writer, key('d')).unwrap();

        assert!(outcome.document_changed);
        assert_eq!(core.document().text(), "\niddle\n");
        assert_eq!(
            core.command_state(reader).unwrap().cursor(),
            3,
            "the untouched middle grapheme follows both preceding block splices"
        );
    }

    #[test]
    fn compound_macro_composes_all_exact_maps_for_other_views() {
        let mut core = Core::new(Document::new("a\nmiddle\na"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(reader, key('j')).unwrap();
        for _ in 0..3 {
            core.handle(reader, key('l')).unwrap();
        }
        assert_eq!(core.command_state(reader).unwrap().cursor(), 5);

        // Record `rAGrA`, then undo the changes made while recording. Macro
        // replay performs two separate model commits inside one input event.
        for character in ['q', 'a', 'r', 'A', 'G', 'r', 'A', 'q'] {
            core.handle(writer, key(character)).unwrap();
        }
        core.handle(writer, key('u')).unwrap();
        core.handle(writer, key('u')).unwrap();
        core.handle(writer, key('g')).unwrap();
        core.handle(writer, key('g')).unwrap();
        assert_eq!(core.document().text(), "a\nmiddle\na");

        core.handle(writer, key('@')).unwrap();
        let replayed = core.handle(writer, key('a')).unwrap();

        assert!(replayed.document_changed);
        assert_eq!(core.document().text(), "A\nmiddle\nA");
        assert_eq!(
            core.command_state(reader).unwrap().cursor(),
            5,
            "both discontiguous macro commits remain separate map steps"
        );
    }

    #[test]
    fn macro_replay_uses_the_views_exact_wrapped_layout() {
        let mut core = Core::new(Document::new("one two three four five six"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 48.0, 100.0);

        // Record `gj`, whose execution while recording already proves the
        // view has more than one visual row, then return to hard-line start.
        for character in ['q', 'a', 'g', 'j', 'q', 'g', 'g', '0'] {
            core.handle(view, key(character)).unwrap();
        }
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert_eq!(
            replay.command.as_ref().unwrap().status,
            CommandStatus::Complete
        );
        assert!(core.command_state(view).unwrap().cursor() > 0);
        assert!(replay.layout_changed);
    }

    #[test]
    fn macro_edit_then_partial_layout_demand_retries_and_is_one_undo_unit() {
        let contents = (0..500)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 32.0);

        core.handle(view, key('q')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('x')).unwrap();
        for character in ['2', '0', '0', 'g'] {
            core.handle(view, key(character)).unwrap();
        }
        let partial = core.handle(view, key('j')).unwrap();
        let demand = match partial.command.unwrap().status {
            CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(
                demand,
            )) => demand,
            status => panic!("unexpected partial-edge status: {status:?}"),
        };
        let request = core
            .prepare_view_layout_demand(
                view,
                LayoutJobPriority::NewlyExposedRows,
                &demand,
                LayoutCancellationToken::new(),
            )
            .unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let candidate =
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
        core.install_view_layout_job(view, candidate).unwrap();
        assert_eq!(
            core.handle(view, key('j')).unwrap().command.unwrap().status,
            CommandStatus::Complete
        );
        core.handle(view, key('q')).unwrap();
        core.handle(view, key('u')).unwrap();
        core.handle(view, key('g')).unwrap();
        core.handle(view, key('g')).unwrap();
        assert_eq!(core.document().text(), contents);

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert_eq!(
            replay.command.as_ref().unwrap().status,
            CommandStatus::Complete
        );
        assert!(replay.document_changed);
        assert!(core.command_state(view).unwrap().cursor() > 0);
        assert!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .coverage
                .hard_lines()
                .end
                >= 201,
            "the replay installed the exact partial-layout extension before retrying"
        );
        assert_ne!(core.document().text(), contents);
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), contents);
    }

    #[test]
    fn macro_failure_retains_its_prefix_as_one_undo_unit() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['q', 'a', 'x', 'Q', 'q'] {
            core.handle(view, key(character)).unwrap();
        }
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert!(matches!(
            replay.command.unwrap().status,
            CommandStatus::Unsupported(_)
        ));
        assert_eq!(core.document().text(), "bc");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
        let oldest = core.handle(view, key('u')).unwrap();
        assert!(matches!(
            oldest.command.unwrap().status,
            CommandStatus::Error(_)
        ));
    }

    #[test]
    fn macro_normal_history_navigation_splits_replay_into_deterministic_branches() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for event in [
            key('q'),
            key('a'),
            key('x'),
            key('u'),
            CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))),
            key('x'),
            key('q'),
        ] {
            core.handle(view, event).unwrap();
        }
        assert_eq!(core.document().text(), "c");
        core.handle(view, key('u')).unwrap();
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
        let observer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        {
            let document = &core.document;
            assert!(core
                .views
                .get_mut(&observer)
                .unwrap()
                .commands
                .set_cursor(document, 2));
        }
        let replay_source_revision = core.document().revision();

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert_eq!(
            replay.command.as_ref().unwrap().status,
            CommandStatus::Complete
        );
        let replay_map = replay
            .position_map
            .expect("replay publishes its composed map");
        assert_eq!(replay_map.source_revision(), replay_source_revision);
        assert_eq!(replay_map.target_revision(), core.document().revision());
        assert_eq!(core.document().text(), "c");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.command_state(observer).unwrap().cursor(), 0);

        // The edit following replayed redo owns a fresh node, and the edit
        // before replayed undo remains its deterministic parent branch.
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "bc");
        assert_eq!(core.command_state(observer).unwrap().cursor(), 1);
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
        assert_eq!(core.command_state(observer).unwrap().cursor(), 2);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "bc");
        assert_eq!(core.command_state(observer).unwrap().cursor(), 1);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "c");
        assert_eq!(core.command_state(observer).unwrap().cursor(), 0);
    }

    #[test]
    fn macro_ex_undo_and_redo_publish_restorations_before_later_edits() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for character in "qax:undo".chars() {
            core.handle(view, key(character)).unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        for character in ":redo".chars() {
            core.handle(view, key(character)).unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        core.handle(view, key('x')).unwrap();
        core.handle(view, key('q')).unwrap();
        assert_eq!(core.document().text(), "c");
        core.handle(view, key('u')).unwrap();
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert_eq!(replay.command.unwrap().status, CommandStatus::Complete);
        assert_eq!(core.document().text(), "c");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "bc");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn ex_normal_history_navigation_rebases_later_targets_and_redo_branch() {
        let mut core = Core::new(Document::new("abc\ndef"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for character in ":%normal xu".chars() {
            core.handle(view, key(character)).unwrap();
        }
        let replay = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(replay.command.unwrap().status, CommandStatus::Complete);
        assert_eq!(core.document().text(), "abc\ndef");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);

        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(
            core.document().text(),
            "abc\nef",
            "the preferred redo branch proves the rebased second target ran"
        );
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc\ndef");
    }

    #[test]
    fn replay_retains_informational_request_and_continues_to_later_edit() {
        use crate::command::ex_execute::ExInfoRequest;

        for (name, expected) in [
            ("marks", "marks"),
            ("registers", "registers"),
            ("jumps", "jumps"),
            ("set", "options"),
            ("s/a/a/p", "print"),
        ] {
            let mut core = Core::new(Document::new("abc"));
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
            for character in ['q', 'a', ':'] {
                core.handle(view, key(character)).unwrap();
            }
            for character in name.chars() {
                core.handle(view, key(character)).unwrap();
            }
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap();
            core.handle(view, key('x')).unwrap();
            core.handle(view, key('q')).unwrap();
            core.handle(view, key('u')).unwrap();
            assert_eq!(core.document().text(), "abc", "recording {name}");

            core.handle(view, key('@')).unwrap();
            let replay = core.handle(view, key('a')).unwrap();
            let command = replay.command.unwrap();
            assert_eq!(command.status, CommandStatus::Complete, "replaying {name}");
            let outcome = command.ex_outcome.expect("informational Ex outcome");
            assert_eq!(outcome.frontend_requests.len(), 1, "replaying {name}");
            let matches_expected = matches!(
                (expected, &outcome.frontend_requests[0]),
                ("marks", ExFrontendRequest::Info(ExInfoRequest::Marks(_)))
                    | (
                        "registers",
                        ExFrontendRequest::Info(ExInfoRequest::Registers(_))
                    )
                    | ("jumps", ExFrontendRequest::Info(ExInfoRequest::Jumps))
                    | (
                        "options",
                        ExFrontendRequest::Info(ExInfoRequest::Options(_))
                    )
                    | (
                        "print",
                        ExFrontendRequest::Info(ExInfoRequest::PrintLines { .. })
                    )
            );
            assert!(matches_expected, "replaying {name}");
            assert_eq!(core.document().text(), "bc", "later edit after {name}");
            core.handle(view, key('u')).unwrap();
            assert_eq!(core.document().text(), "abc", "undo after {name}");
        }
    }

    #[test]
    fn replay_still_stops_at_file_host_barrier_before_later_edit() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in "qa:quit".chars() {
            core.handle(view, key(character)).unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        core.handle(view, key('x')).unwrap();
        core.handle(view, key('q')).unwrap();
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        let command = replay.command.unwrap();
        assert!(matches!(
            command.status,
            CommandStatus::Unsupported(ref message) if message.contains("host action")
        ));
        assert!(command.ex_outcome.is_some_and(|outcome| {
            matches!(
                outcome.frontend_requests.as_slice(),
                [ExFrontendRequest::File(ExFileRequest::Quit { .. })]
            )
        }));
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn recursive_macro_is_stopped_by_the_shared_coordinator_depth_limit() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['q', 'a', '@', 'a', 'q'] {
            core.handle(view, key(character)).unwrap();
        }

        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert!(matches!(
            replay.command.unwrap().status,
            CommandStatus::Error(ref message) if message.contains("recursion limit")
        ));
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn coordinator_macro_count_obeys_the_shared_event_budget() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['q', 'a', 'h', 'q'] {
            core.handle(view, key(character)).unwrap();
        }
        for character in "100001@a".chars() {
            let outcome = core.handle(view, key(character)).unwrap();
            if character == 'a' {
                assert!(matches!(
                    outcome.command.unwrap().status,
                    CommandStatus::CountError(CountError::ReplayEventBudgetExceeded {
                        limit: MACRO_REPLAY_EVENT_LIMIT,
                        ..
                    })
                ));
            }
        }
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn nested_macros_share_one_coordinator_event_budget() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for character in ['q', 'b', 'h', 'h', 'h', 'q'] {
            core.handle(view, key(character)).unwrap();
        }
        for character in ['q', 'a', '@', 'b', '@', 'b', 'q'] {
            core.handle(view, key(character)).unwrap();
        }

        core.compound_replay_event_limit = 4;
        core.handle(view, key('@')).unwrap();
        let replay = core.handle(view, key('a')).unwrap();
        assert!(matches!(
            replay.command.unwrap().status,
            CommandStatus::CountError(CountError::ReplayEventBudgetExceeded { limit: 4, .. })
        ));
        assert_eq!(core.document().text(), "abc");

        // The budget failure unwinds every replay frame and its undo scope.
        assert_eq!(
            core.handle(view, key('l')).unwrap().command.unwrap().status,
            CommandStatus::Complete
        );
    }

    #[test]
    fn ex_normal_stays_in_core_and_publishes_one_exact_multi_commit_map() {
        let mut core = Core::new(Document::new("one\ntwo\nthree"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        core.handle(reader, key('j')).unwrap();
        assert_eq!(core.command_state(reader).unwrap().cursor(), 4);

        for character in ":%normal! A!".chars() {
            core.handle(writer, key(character)).unwrap();
        }
        let outcome = core
            .handle(writer, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();

        assert_eq!(core.document().text(), "one!\ntwo!\nthree!");
        assert!(outcome.document_changed);
        let map = outcome.position_map.as_ref().expect("one exact outer map");
        assert_eq!(map.source_revision(), Revision(0));
        assert_eq!(map.target_revision(), core.document().revision());
        assert_eq!(core.command_state(reader).unwrap().cursor(), 5);
        assert!(outcome
            .command
            .as_ref()
            .unwrap()
            .ex_outcome
            .as_ref()
            .unwrap()
            .frontend_requests
            .is_empty());

        core.handle(writer, key('u')).unwrap();
        assert_eq!(core.document().text(), "one\ntwo\nthree");
    }

    #[test]
    fn ex_normal_replay_uses_wrapped_layout_for_each_stable_target() {
        let first = "one two three four";
        let second = "five six seven eight";
        let mut core = Core::new(Document::new(format!("{first}\n{second}")));
        let view = core.add_view(MockTextMeasurementProvider::new(), 48.0, 100.0);

        for character in ":%normal! gj".chars() {
            core.handle(view, key(character)).unwrap();
        }
        let replay = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(
            replay.command.as_ref().unwrap().status,
            CommandStatus::Complete
        );
        assert!(core.command_state(view).unwrap().cursor() > first.len() + 1);
        assert!(replay.layout_changed);
        assert!(replay
            .command
            .as_ref()
            .unwrap()
            .ex_outcome
            .as_ref()
            .unwrap()
            .frontend_requests
            .is_empty());
    }

    #[test]
    fn invoking_view_mark_follows_its_own_insert() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(view, key('l')).unwrap();
        core.handle(view, key('m')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('0')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        assert_eq!(core.document().text(), "Xabc");
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            2,
            "mark a denotes the original b, not stale byte offset 1"
        );
    }

    #[test]
    fn invoking_view_mark_follows_exact_undo_and_redo_maps() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(view, key('l')).unwrap();
        core.handle(view, key('m')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('0')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, key('u')).unwrap();
        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        assert_eq!(core.document().text(), "abc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);

        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        assert_eq!(core.document().text(), "Xabc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);
    }

    #[test]
    fn jump_lists_remain_view_local_while_views_share_the_buffer() {
        let mut core = Core::new(Document::new("one\ntwo\nthree"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(first, key('G')).unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 8);

        let absent = core
            .handle(second, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert!(matches!(
            absent.command.unwrap().status,
            CommandStatus::Error(ref message) if message.contains("jump list")
        ));
        assert_eq!(core.command_state(second).unwrap().cursor(), 0);

        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert_eq!(core.command_state(first).unwrap().cursor(), 0);
    }

    #[test]
    fn invoking_view_jump_history_rebases_and_ctrl_o_captures_the_live_position() {
        let mut core = Core::new(Document::new("abcd"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        for _ in 0..2 {
            core.handle(view, key('l')).unwrap();
        }
        core.handle(view, key('m')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('0')).unwrap();
        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('0')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            1,
            "the older jump still denotes the original first grapheme"
        );
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('i'))))
            .unwrap();
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            0,
            "the first CTRL-O saves the current live position for CTRL-I"
        );
    }

    #[test]
    fn invoking_view_remembered_visual_selection_follows_its_own_insert() {
        let mut core = Core::new(Document::new("abcd"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

        core.handle(view, key('l')).unwrap();
        core.handle(view, key('v')).unwrap();
        core.handle(view, key('l')).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, key('0')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, key('g')).unwrap();
        core.handle(view, key('v')).unwrap();
        let state = core.command_state(view).unwrap();
        assert_eq!(state.mode(), Mode::VisualCharacter);
        assert_eq!(state.visual_anchor(), Some(2));
        assert_eq!(state.cursor(), 3);
    }

    #[test]
    fn ex_view_option_effects_update_layout_before_the_outcome_returns() {
        let mut core = Core::new(Document::new("one two three four"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 100.0);
        assert!(core.layout(view).unwrap().wrap());
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() > 1);

        for character in ":set nowrap".chars() {
            core.handle(view, key(character)).unwrap();
        }
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();

        assert!(!core.layout(view).unwrap().wrap());
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().rows.len(), 1);
        assert!(outcome.layout_changed);
        assert_eq!(
            outcome
                .command
                .as_ref()
                .unwrap()
                .ex_outcome
                .as_ref()
                .unwrap()
                .option_effects
                .len(),
            1
        );
    }

    #[test]
    fn composition_update_and_cancel_only_change_the_temporary_overlay() {
        let mut core = Core::new(Document::new("hello"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let source = core.document().source_bytes();
        let revision = core.document().revision();

        let begin = begin_composition(&core, 1..4);
        let began = core.handle(view, begin).unwrap();
        assert!(matches!(
            &began.composition_changes[0].outcome,
            ViewCompositionOutcome::Began(overlay) if overlay.formatted_text() == "ho"
        ));
        let updated = core
            .handle(
                view,
                CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("é", 2..2))),
            )
            .unwrap();
        assert!(matches!(
            &updated.composition_changes[0].outcome,
            ViewCompositionOutcome::Updated(overlay)
                if overlay.formatted_text() == "héo"
                    && overlay.selected_range_in_overlay() == (3..3)
        ));
        assert_eq!(
            core.composition_overlay(view)
                .unwrap()
                .unwrap()
                .formatted_text(),
            "héo"
        );
        assert_eq!(core.document().text(), "hello");
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);

        let cancelled = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
            .unwrap();
        assert!(matches!(
            &cancelled.composition_changes[0].outcome,
            ViewCompositionOutcome::Cancelled {
                reason: CompositionCancelReason::Explicit,
                restoration,
            } if restoration.formatted_text() == "hello"
        ));
        assert!(core.composition_overlay(view).unwrap().is_none());
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        assert!(!core.document.undo());
    }

    #[test]
    fn composition_layout_reflows_suffix_without_full_document_work() {
        let mut source = String::from("alpha beta gamma delta\n");
        source.push_str(&"unchanged line\n".repeat(100_000));
        source.push_str("tail");
        let mut core = Core::new(Document::new(&source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 72.0, 48.0);
        let revision = core.document().revision();
        let original = core.document().source_bytes();

        core.handle(view, begin_composition(&core, 6..10)).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                "an intentionally long marked phrase",
                35..35,
            ))),
        )
        .unwrap();

        let overlay = core.composition_overlay(view).unwrap().unwrap();
        let layout = core.presentation_layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.coverage.hard_lines().end < 100);
        assert!(
            snapshot
                .rows
                .iter()
                .filter(|row| row.hard_line_index == 0)
                .count()
                > 1
        );
        assert!(snapshot.rows.iter().any(|row| {
            row.hard_line_index == 0 && row.text_range.end == overlay.marked_range().end + 12
        }));
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), original);

        core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
            .unwrap();
        assert!(core.composition_overlay(view).unwrap().is_none());
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), original);
    }

    #[test]
    fn composition_in_long_plain_and_flowed_paragraphs_uses_bounded_slices() {
        for flow in [false, true] {
            let mut source = ("word ".repeat(30) + "\n").repeat(2_000);
            if !flow {
                source = source.replace('\n', " ");
            }
            let prefix = "intro ".repeat(40) + "\n\n";
            source.insert_str(0, &prefix);
            let document = Document::from_bytes(
                source.as_bytes().to_vec(),
                crate::document::Encoding::Utf8,
                if flow {
                    crate::document::Format::MarkdownSource
                } else {
                    crate::document::Format::PlainText
                },
            )
            .unwrap();
            let mut core = Core::new(document);
            let (provider, bytes, _, generation) =
                InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
            let view = core.add_view(provider, 240.0, 120.0);
            if flow {
                core.handle(view, CoreEvent::SetParagraphFlow(true))
                    .unwrap();
            }
            for at in [
                prefix.len() + (source.len() - prefix.len()) / 2,
                prefix.len(),
            ] {
                let expected_line = core
                    .document()
                    .projection()
                    .presentation_line_at_offset(at, flow)
                    .unwrap();
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: at,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                core.handle(view, begin_composition(&core, at..at)).unwrap();
                for value in ["é", "かな", "👩🏽‍💻"] {
                    let before = bytes.load(Ordering::Acquire);
                    core.handle(
                        view,
                        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                            value,
                            value.len()..value.len(),
                        ))),
                    )
                    .unwrap();
                    let shaped = bytes.load(Ordering::Acquire) - before;
                    assert!(
                        shaped < MAX_LONG_LINE_LAYOUT_SLICE_BYTES * 2,
                        "flow={flow} at={at} shaped={shaped}"
                    );
                    let layout = core.presentation_layout(view).unwrap();
                    let snapshot = layout.snapshot().unwrap();
                    assert!(snapshot.coverage.contains_text_offset(at + value.len()));
                    assert!(snapshot
                        .rows
                        .iter()
                        .all(|row| row.hard_line_index == expected_line));
                    snapshot
                        .logical_endpoint_geometry(at + value.len(), BoundaryAffinity::Downstream)
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                }
                generation.fetch_add(1, Ordering::AcqRel);
                core.handle(
                    view,
                    CoreEvent::Resize {
                        width: 180.0,
                        height: 100.0,
                    },
                )
                .unwrap();
                assert_eq!(
                    core.presentation_layout(view)
                        .unwrap()
                        .snapshot()
                        .unwrap()
                        .metrics_generation,
                    MetricsGeneration(generation.load(Ordering::Acquire))
                );
                core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                assert!(!core.document.undo());
            }
        }
    }

    #[test]
    fn compositions_are_independent_per_view_until_a_source_commit() {
        let mut core = Core::new(Document::new("abc"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let first_begin = begin_composition(&core, 0..1);
        let second_begin = begin_composition(&core, 2..3);
        core.handle(first, first_begin).unwrap();
        core.handle(second, second_begin).unwrap();
        core.handle(
            first,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("A", 1..1))),
        )
        .unwrap();
        core.handle(
            second,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("C", 1..1))),
        )
        .unwrap();

        assert_eq!(
            core.composition_overlay(first)
                .unwrap()
                .unwrap()
                .formatted_text(),
            "Abc"
        );
        assert_eq!(
            core.composition_overlay(second)
                .unwrap()
                .unwrap()
                .formatted_text(),
            "abC"
        );
        core.handle(first, CoreEvent::Composition(CompositionEvent::Cancel))
            .unwrap();
        assert!(core.composition_overlay(first).unwrap().is_none());
        assert!(core.composition_overlay(second).unwrap().is_some());
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn composition_commit_is_isolated_as_one_undo_step() {
        let mut core = Core::new(Document::new(""));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("a")).unwrap();

        let begin = begin_composition(&core, 1..1);
        core.handle(view, begin).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("é", 2..2))),
        )
        .unwrap();
        let committed = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();

        assert!(committed.document_changed);
        assert!(matches!(
            committed.composition_changes[0].outcome,
            ViewCompositionOutcome::Committed(_)
        ));
        assert_eq!(core.document().text(), "aé");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();

        core.handle(view, key('u')).unwrap();
        assert_eq!(
            core.document().text(),
            "a",
            "composition is its own undo unit"
        );
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "");
    }

    #[test]
    fn composition_commit_uses_grapheme_closed_map_for_carets_and_other_views() {
        let regional_a = "\u{1f1e6}";
        let regional_b = "\u{1f1e7}";
        let mut core = Core::new(Document::new(format!("{regional_b}x")));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let observer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(observer, key('l')).unwrap();
        assert_eq!(
            core.command_state(observer).unwrap().cursor(),
            regional_b.len()
        );
        core.handle(writer, key('i')).unwrap();
        let begin = begin_composition(&core, 0..0);
        core.handle(writer, begin).unwrap();
        core.handle(
            writer,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                regional_a,
                regional_a.len()..regional_a.len(),
            ))),
        )
        .unwrap();

        let committed = core
            .handle(writer, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        let joined_flag_end = format!("{regional_a}{regional_b}").len();

        assert!(committed.document_changed);
        assert_eq!(core.document().text(), format!("{regional_a}{regional_b}x"));
        assert_eq!(
            core.command_state(writer).unwrap().cursor(),
            joined_flag_end
        );
        assert_eq!(
            core.command_state(observer).unwrap().cursor(),
            joined_flag_end,
            "the observer remains attached to x across the right-side grapheme join"
        );
        assert!(core.document().history_status().can_undo);
    }

    #[test]
    fn unrepresentable_latin1_commit_is_atomic_and_keeps_the_overlay_active() {
        let document =
            Document::from_bytes(vec![0xe9], Encoding::Latin1, Format::PlainText).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("a")).unwrap();
        let source = core.document().source_bytes();
        let revision = core.document().revision();
        let begin = begin_composition(&core, 1..3);
        core.handle(view, begin).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("😀", 4..4))),
        )
        .unwrap();

        let error = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap_err();

        assert!(matches!(
            error,
            CoreError::Composition(CompositionError::Document(
                DocumentError::UnrepresentableCharacter {
                    encoding: Encoding::Latin1,
                    character: '😀',
                }
            ))
        ));
        assert_eq!(core.document().text(), "aé");
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        assert_eq!(
            core.composition_overlay(view)
                .unwrap()
                .unwrap()
                .formatted_text(),
            "a😀"
        );

        core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
            .unwrap();
        core.handle(view, text("b")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "aé");
        core.handle(view, key('u')).unwrap();
        assert_eq!(
            core.document().text(),
            "é",
            "failed commit did not leak or merge an Insert undo group"
        );
    }

    #[test]
    fn edit_in_another_view_invalidates_composition_instead_of_rebasing_it() {
        let mut core = Core::new(Document::new("abc"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let composing = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let base_revision = core.document().revision();
        let begin = begin_composition(&core, 1..2);
        core.handle(composing, begin).unwrap();
        core.handle(
            composing,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("B", 1..1))),
        )
        .unwrap();

        core.handle(writer, key('i')).unwrap();
        let edit = core.handle(writer, text("X")).unwrap();

        assert_eq!(core.document().text(), "Xabc");
        assert!(matches!(
            &edit.composition_changes[..],
            [ViewCompositionChange {
                view,
                outcome: ViewCompositionOutcome::Invalidated {
                    reason: CompositionCancelReason::ExternalDocumentChange,
                    base_revision: old,
                    current_revision,
                },
            }] if *view == composing
                && *old == base_revision
                && *current_revision == core.document().revision()
        ));
        assert!(core.composition_overlay(composing).unwrap().is_none());
        assert_eq!(
            core.command_state(composing).unwrap().position_revision(),
            Some(core.document().revision())
        );
    }

    #[test]
    fn undo_and_redo_cancel_marked_text_then_navigate_history_in_the_same_turn() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('x')).unwrap();
        assert_eq!(core.document().text(), "bc");

        let begin = begin_composition(&core, 0..0);
        core.handle(view, begin).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("z", 1..1))),
        )
        .unwrap();
        let cancel = core.handle(view, key('u')).unwrap();
        assert!(cancel.document_changed);
        assert!(cancel
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation));
        assert!(matches!(
            &cancel.composition_changes[..],
            [ViewCompositionChange {
                view: changed_view,
                outcome: ViewCompositionOutcome::Cancelled {
                    reason: CompositionCancelReason::Undo,
                    ..
                },
            }] if *changed_view == view
        ));
        assert_eq!(core.document().text(), "abc");
        let begin = begin_composition(&core, 0..0);
        core.handle(view, begin).unwrap();
        let cancel = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert!(cancel.document_changed);
        assert!(cancel
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation));
        assert!(matches!(
            &cancel.composition_changes[..],
            [ViewCompositionChange {
                view: changed_view,
                outcome: ViewCompositionOutcome::Cancelled {
                    reason: CompositionCancelReason::Redo,
                    ..
                },
            }] if *changed_view == view
        ));
        assert_eq!(core.document().text(), "bc");
    }

    #[test]
    fn history_restores_invoking_cursor_and_marks_but_not_registers() {
        let mut core = Core::new(Document::new("aa\nbb\ncc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(view, key('j')).unwrap();
        core.handle(view, key('l')).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        core.handle(view, key('m')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('x')).unwrap();
        assert_eq!(core.document().text(), "aa\nb\ncc");

        let changed = core.document().history_status().current;
        let details = core.document().history_node_details(changed.node).unwrap();
        let restoration = details.restoration.unwrap();
        assert_eq!(restoration.before().cursor().offset(), 4);
        assert_eq!(restoration.before().marks()[&'a'].offset(), 4);
        assert_eq!(restoration.after().cursor().offset(), 3);
        assert_eq!(restoration.after().marks()[&'a'].offset(), 4);

        // Replace both the live mark and unnamed register after the edit.
        core.handle(view, key('j')).unwrap();
        core.handle(view, key('m')).unwrap();
        core.handle(view, key('a')).unwrap();
        core.handle(view, key('y')).unwrap();
        core.handle(view, key('y')).unwrap();
        let register_after_edit = core.command_state(view).unwrap().register('"').cloned();

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "aa\nbb\ncc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().visual_anchor(), None);
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        assert_eq!(
            core.command_state(view).unwrap().register('"').cloned(),
            register_after_edit,
            "history navigation must not restore register side effects"
        );
        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);

        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "aa\nb\ncc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        core.handle(view, key('`')).unwrap();
        core.handle(view, key('a')).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        assert_eq!(
            core.command_state(view).unwrap().register('"').cloned(),
            register_after_edit
        );
    }

    #[test]
    fn history_navigation_does_not_restore_repeat_search_or_jump_state() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('x')).unwrap();
        core.handle(view, key('u')).unwrap();
        core.handle(view, key('.')).unwrap();
        assert_eq!(
            core.document().text(),
            "bc",
            "dot must retain the change recipe established before undo"
        );

        let mut core = Core::new(Document::new("aba"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('x')).unwrap();
        for event in [key('/'), text("a")] {
            core.handle(view, event).unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        core.handle(view, key('u')).unwrap();
        core.handle(view, key('n')).unwrap();
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            2,
            "the search selected after the edit must survive undo"
        );

        let mut core = Core::new(Document::new("a\nb\nc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('x')).unwrap();
        core.handle(view, key('G')).unwrap();
        core.handle(view, key('u')).unwrap();
        let jump = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert_eq!(
            jump.command.unwrap().status,
            CommandStatus::Complete,
            "the jump list accumulated after the edit must survive undo"
        );
    }

    #[test]
    fn history_navigation_exits_insert_ctrl_o_transient_state() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.handle(view, key('x')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);

        core.handle(view, key('u')).unwrap();

        assert_eq!(core.document().text(), "abc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.document.edit_group_depth(), 0);
        assert_eq!(core.edit_group_owner, None);
        assert!(core.edit_group_restoration.is_none());
    }

    #[test]
    fn preferred_history_branches_restore_their_own_invoking_view_state() {
        let mut core = Core::new(Document::new("abcd"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(view, key('l')).unwrap();
        core.handle(view, key('x')).unwrap();
        let first = core.document().history_status().current;
        assert_eq!(core.document().text(), "acd");
        core.handle(view, key('u')).unwrap();

        core.handle(view, key('h')).unwrap();
        core.handle(view, key('x')).unwrap();
        let second = core.document().history_status().current;
        assert_eq!(core.document().text(), "bcd");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().redo_branches().len(), 2);

        core.document.prefer_redo_branch(0).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().history_status().current, first);
        assert_eq!(core.document().text(), "acd");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);

        core.handle(view, key('u')).unwrap();
        core.document.prefer_redo_branch(1).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().history_status().current, second);
        assert_eq!(core.document().text(), "bcd");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }

    #[test]
    fn grouped_insert_uses_entry_and_final_normal_cursor_restoration() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(view, key('l')).unwrap();
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        core.handle(view, text("Y")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "aXYbc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);

        let node = core.document().history_status().current;
        let details = core.document().history_node_details(node.node).unwrap();
        assert_eq!(details.transactions.len(), 2);
        let restoration = details.restoration.unwrap();
        assert_eq!(restoration.before().cursor().offset(), 1);
        assert_eq!(restoration.before().cursor().revision(), Revision(0));
        assert_eq!(restoration.after().cursor().offset(), 2);
        assert_eq!(restoration.after().cursor().revision(), Revision(2));

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "aXYbc");
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);
    }

    #[test]
    fn inactive_visual_view_remains_visual_and_is_remapped_through_history() {
        let mut core = Core::new(Document::new("abcd"));
        let writer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let observer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);

        core.handle(observer, key('l')).unwrap();
        core.handle(observer, key('v')).unwrap();
        core.handle(observer, key('l')).unwrap();
        assert_eq!(
            core.command_state(observer).unwrap().mode(),
            Mode::VisualCharacter
        );
        assert_eq!(
            core.command_state(observer).unwrap().visual_anchor(),
            Some(1)
        );

        core.handle(writer, key('x')).unwrap();
        assert_eq!(core.document().text(), "bcd");
        assert_eq!(
            core.command_state(observer).unwrap().mode(),
            Mode::VisualCharacter
        );
        core.handle(writer, key('u')).unwrap();

        assert_eq!(core.document().text(), "abcd");
        assert_eq!(core.command_state(writer).unwrap().mode(), Mode::Normal);
        assert_eq!(
            core.command_state(observer).unwrap().mode(),
            Mode::VisualCharacter
        );
        assert_eq!(
            core.command_state(observer).unwrap().visual_anchor(),
            Some(1)
        );
        assert_eq!(core.command_state(observer).unwrap().cursor(), 2);
    }

    #[test]
    fn input_preflight_shapes_only_when_command_resolution_needs_layout() {
        let (provider, fail_next, shape_calls, generation) =
            ControlledFailureProvider::new_counted();
        let mut core = Core::new(Document::new("abc\ndef\nghi"));
        let view = core.add_view(provider, 200.0, 32.0);
        let initial_calls = shape_calls.load(Ordering::Acquire);
        assert!(
            initial_calls > 0,
            "adding a view establishes initial layout"
        );

        // Make the installed snapshot inexact without asking the provider to
        // rebuild it. A logical edit must still commit before presentation
        // layout is attempted; the injected failure is therefore recorded as
        // a post-commit diagnostic instead of rejecting the command.
        generation.fetch_add(1, Ordering::AcqRel);
        fail_next.store(true, Ordering::Release);
        let changed = core.handle(view, key('x')).unwrap();
        assert!(changed.document_changed);
        assert_eq!(core.document().text(), "bc\ndef\nghi");
        assert_eq!(shape_calls.load(Ordering::Acquire), initial_calls + 1);

        fail_next.store(true, Ordering::Release);
        let undone = core.handle(view, key('u')).unwrap();
        assert!(undone.document_changed);
        assert_eq!(core.document().text(), "abc\ndef\nghi");
        assert_eq!(shape_calls.load(Ordering::Acquire), initial_calls + 2);

        // Pending operator/search grammar is entirely logical. It preserves
        // the viewport and must not consume the next provider failure.
        let viewport_top = core.layout(view).unwrap().viewport_top();
        generation.fetch_add(1, Ordering::AcqRel);
        fail_next.store(true, Ordering::Release);
        let pending = core.handle(view, key('d')).unwrap();
        assert_eq!(pending.command.unwrap().status, CommandStatus::Pending);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, key('/')).unwrap();
        core.handle(view, text("abc")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(shape_calls.load(Ordering::Acquire), initial_calls + 2);
        assert!(fail_next.load(Ordering::Acquire));
        assert_eq!(core.layout(view).unwrap().viewport_top(), viewport_top);

        // Bare vertical motion uses desired-x in layout units even without
        // wrapping. It must acquire exact geometry and therefore reaches the
        // still-armed failing provider.
        let error = core.handle(view, key('j')).unwrap_err();
        assert!(
            matches!(
                &error,
                CoreError::Layout(LayoutError::Measurement(MeasurementError::Provider(
                    message
                )))
                    | CoreError::LayoutJob(LayoutJobError::Layout(LayoutError::Measurement(
                        MeasurementError::Provider(message)
                    ))) if message == "injected post-commit failure"
            ),
            "unexpected layout acquisition error: {error:?}"
        );
        assert_eq!(shape_calls.load(Ordering::Acquire), initial_calls + 3);
        assert!(!fail_next.load(Ordering::Acquire));
    }

    #[test]
    fn font_registration_during_shape_retries_only_local_layout_and_never_replays_input() {
        let (provider, _, calls, generation) = ControlledFailureProvider::new_counted();
        let invalidation = Arc::clone(&provider.invalidate_during_shape);
        invalidation.store(true, Ordering::Release);
        let mut document = Document::new(&"one line of text\n".repeat(100_000));
        // This oversized fixture checks metric retries and undo identity, not
        // retention pruning when the active projection exceeds its byte target.
        document.set_history_retention_policy(crate::document::HistoryRetentionPolicy::unlimited());
        let mut core = Core::new(document);
        let view = core.try_add_view(provider, 300.0, 100.0).unwrap();
        assert!(calls.load(Ordering::Acquire) <= 3);
        assert_eq!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .metrics_generation,
            MetricsGeneration(2)
        );

        core.handle(view, key('i')).unwrap();
        invalidation.store(true, Ordering::Release);
        let outcome = core.handle(view, text("X")).unwrap();
        assert!(outcome.document_changed);
        assert_eq!(core.document().revision(), Revision(1));
        assert!(core.document().text().starts_with("Xone line"));
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.document_revision, Revision(1));
        assert_eq!(
            snapshot.metrics_generation,
            MetricsGeneration(generation.load(Ordering::Acquire))
        );
        assert!(
            snapshot.rows.len() < 100,
            "a transient metric change must keep work local"
        );
        assert!(calls.load(Ordering::Acquire) <= 6);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, key('j')).unwrap();
        core.handle(view, key('u')).unwrap();
        assert!(core.document().text().starts_with("one line"));
    }

    #[test]
    fn post_commit_layout_failure_is_a_view_diagnostic_not_a_failed_edit() {
        let (provider, fail_next) = ControlledFailureProvider::new();
        let mut core = Core::new(Document::new("abc"));
        let writer = core.add_view(provider.clone(), 200.0, 100.0);
        let reader = core.add_view(provider, 200.0, 100.0);
        core.handle(reader, key('l')).unwrap();
        assert_eq!(core.command_state(reader).unwrap().cursor(), 1);

        core.handle(writer, key('i')).unwrap();
        fail_next.store(true, Ordering::Release);
        let outcome = core.handle(writer, text("X")).unwrap();

        assert!(outcome.document_changed);
        assert_eq!(core.document().text(), "Xabc");
        assert!(matches!(
            core.layout(writer).unwrap().last_error(),
            Some(LayoutError::Measurement(MeasurementError::Provider(message)))
                if message == "injected post-commit failure"
        ));
        assert_eq!(
            core.layout(writer)
                .unwrap()
                .snapshot()
                .expect("the previous exact snapshot remains installed")
                .document_revision,
            Revision(0)
        );
        assert_eq!(
            core.command_state(reader).unwrap().cursor(),
            2,
            "inactive view anchors still publish after the layout failure"
        );
        assert!(core.document().history_status().can_undo);
    }

    #[test]
    fn post_commit_composition_layout_failure_keeps_the_committed_outcome() {
        let (provider, fail_next) = ControlledFailureProvider::new();
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(provider, 200.0, 100.0);
        let begin = begin_composition(&core, 1..1);
        core.handle(view, begin).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("X", 1..1))),
        )
        .unwrap();

        fail_next.store(true, Ordering::Release);
        let outcome = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();

        assert_eq!(core.document().text(), "aXbc");
        assert!(outcome.document_changed);
        assert!(matches!(
            &outcome.composition_changes[..],
            [ViewCompositionChange {
                view: changed_view,
                outcome: ViewCompositionOutcome::Committed(_),
            }] if *changed_view == view
        ));
        assert!(core.composition_overlay(view).unwrap().is_none());
        assert!(core.layout(view).unwrap().last_error().is_some());
        assert!(core.document().history_status().can_undo);
    }

    #[test]
    fn million_line_core_materializes_and_captures_only_a_bounded_region() {
        const LINE_COUNT: usize = 1_000_000;
        let (provider, shaped_bytes, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::layout_test_with_line_count(LINE_COUNT));
        let view = core.add_view(provider, 320.0, 96.0);

        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().expect("bounded initial viewport");
        let coverage = snapshot.coverage.hard_lines();
        assert!(matches!(
            snapshot.coverage,
            LayoutCoverage::PartialHardLines { .. }
        ));
        assert!(coverage.end - coverage.start < 100);
        assert!(shaped_bytes.load(Ordering::Acquire) < 10_000);
        let index = layout.height_index_statistics();
        assert_eq!(index.hard_line_count(), LINE_COUNT);
        assert!(index.run_count() <= 3);

        let before_prepare = shaped_bytes.load(Ordering::Acquire);
        let cancellation = LayoutCancellationToken::new();
        let request = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(
                    ViewportLayoutRegion::new(750_000..750_010, 12_000_000.0, 96.0).unwrap(),
                ),
                cancellation.clone(),
            )
            .unwrap();
        assert!(request.captured_text_len() <= 20);
        assert_eq!(shaped_bytes.load(Ordering::Acquire), before_prepare);
    }

    #[test]
    fn tiny_line_metrics_expand_geometrically_until_the_viewport_is_exact() {
        let contents = "x\n".repeat(1_999) + "x";
        let (provider, _, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(provider.with_metric_scale(0.01), 200.0, 100.0);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().expect("tiny rows still materialize");
        let coverage = snapshot.coverage.hard_lines();
        let vertical = snapshot.coverage.vertical_range().unwrap();

        assert!(vertical.start <= layout.viewport_top());
        assert!(vertical.end >= layout.viewport_top() + layout.height());
        assert!(coverage.end - coverage.start > 100);
        assert!(coverage.end - coverage.start < 2_000);
    }

    #[test]
    fn stale_facade_candidate_is_rejected_after_a_document_commit() {
        let mut core = Core::new(Document::new("zero\none\ntwo\nthree"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let cancellation = LayoutCancellationToken::new();
        let request = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::ViewportOverscan,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(1..4, 16.0, 48.0).unwrap()),
                cancellation.clone(),
            )
            .unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let candidate =
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();

        core.handle(view, key('i')).unwrap();
        core.handle(view, text("X")).unwrap();
        assert!(cancellation.is_cancelled());
        let installed_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        assert!(matches!(
            core.install_view_layout_job(view, candidate),
            Err(CoreError::LayoutInstall(
                LayoutJobInstallRejection::Cancelled
            ))
        ));
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            installed_revision
        );
    }

    #[test]
    fn layout_demand_preserves_another_views_open_undo_group() {
        let contents = (0..500)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents.clone()));
        let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 32.0);
        let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 32.0);
        for character in ['2', '0', '0', 'g'] {
            core.handle(reader, key(character)).unwrap();
        }
        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text("X")).unwrap();
        let history = core.document.history_status();
        let depth = core.document.edit_group_depth();
        let generation = core.document.edit_group_generation();
        let edge = core.handle(reader, key('j')).unwrap();
        assert!(matches!(
            edge.command.unwrap().status,
            CommandStatus::NeedsMoreLayout(_)
        ));
        assert_eq!(core.document.history_status(), history);
        assert_eq!(core.document.edit_group_depth(), depth);
        assert_eq!(core.document.edit_group_generation(), generation);
        assert_eq!(core.edit_group_owner, Some(writer));
        core.handle(writer, text("Y")).unwrap();
        core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(writer, key('u')).unwrap();
        assert_eq!(core.document.text(), contents);
    }

    #[test]
    fn partial_edge_is_typed_non_mutating_and_can_retry_after_extension() {
        let contents = (0..500)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 32.0);
        let document_revision = core.document().revision();
        for character in ['2', '0', '0', 'g'] {
            core.handle(view, key(character)).unwrap();
        }
        let edge = core.handle(view, key('j')).unwrap();
        let demand = match edge.command.unwrap().status {
            CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(
                demand,
            )) => demand,
            status => panic!("unexpected partial-edge status: {status:?}"),
        };
        assert_eq!(demand.edge(), LayoutDemandEdge::After);
        assert_eq!(demand.materialized_hard_lines().start, 0);
        assert!(demand.requested_hard_lines().end >= 201);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.document().revision(), document_revision);

        let request = core
            .prepare_view_layout_demand(
                view,
                LayoutJobPriority::NewlyExposedRows,
                &demand,
                LayoutCancellationToken::new(),
            )
            .unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let candidate =
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
        core.install_view_layout_job(view, candidate).unwrap();
        let retry = core.handle(view, key('j')).unwrap();
        assert_eq!(retry.command.unwrap().status, CommandStatus::Complete);
        assert!(core.command_state(view).unwrap().cursor() > 0);
        assert!(matches!(
            core.prepare_view_layout_demand(
                view,
                LayoutJobPriority::NewlyExposedRows,
                &demand,
                LayoutCancellationToken::new(),
            ),
            Err(CoreError::StaleLayoutDemand {
                demand: stale,
                current: Some(current),
            }) if stale != current
        ));
    }

    #[test]
    fn resize_preserves_the_top_text_anchor_and_far_jump_reveals_caret() {
        let contents = (0..2_000)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut core = Core::new(Document::new(contents));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 48.0);
        for _ in 0..5 {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
                .unwrap();
        }
        let top_line_before = {
            let layout = core.layout(view).unwrap();
            let top = layout.viewport_top();
            layout
                .snapshot()
                .unwrap()
                .rows
                .iter()
                .find(|row| row.y + row.height() > top)
                .unwrap()
                .hard_line_index
        };
        core.handle(
            view,
            CoreEvent::Resize {
                width: 180.0,
                height: 48.0,
            },
        )
        .unwrap();
        let top_line_after = {
            let layout = core.layout(view).unwrap();
            let top = layout.viewport_top();
            layout
                .snapshot()
                .unwrap()
                .rows
                .iter()
                .find(|row| row.y + row.height() > top)
                .unwrap()
                .hard_line_index
        };
        assert_eq!(top_line_after, top_line_before);

        core.handle(view, key('G')).unwrap();
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        let cursor = core.command_state(view).unwrap().cursor();
        assert!(snapshot.coverage.contains_text_offset(cursor));
        assert!(snapshot.coverage.hard_lines().start > 1_900);
        let caret = snapshot
            .logical_endpoint_geometry(
                cursor,
                core.command_state(view).unwrap().boundary_affinity(),
            )
            .unwrap();
        assert!(caret.rect.y >= layout.viewport_top());
        assert!(caret.rect.y < layout.viewport_top() + layout.height());
    }

    #[test]
    fn shared_edit_rematerializes_independent_widths_for_all_views() {
        let contents = "one two three four five six seven eight\n".repeat(200);
        let mut core = Core::new(Document::new(contents));
        let narrow = core.add_view(MockTextMeasurementProvider::new(), 70.0, 64.0);
        let wide = core.add_view(MockTextMeasurementProvider::new(), 500.0, 64.0);

        core.handle(narrow, key('i')).unwrap();
        core.handle(narrow, text("X")).unwrap();
        let revision = core.document().revision();
        let narrow_snapshot = core.layout(narrow).unwrap().snapshot().unwrap();
        let wide_snapshot = core.layout(wide).unwrap().snapshot().unwrap();
        assert_eq!(narrow_snapshot.document_revision, revision);
        assert_eq!(wide_snapshot.document_revision, revision);
        assert_eq!(narrow_snapshot.viewport_width, 70.0);
        assert_eq!(wide_snapshot.viewport_width, 500.0);
        assert!(narrow_snapshot.rows.len() > wide_snapshot.rows.len());
        assert!(!narrow_snapshot.coverage.is_full_document());
        assert!(!wide_snapshot.coverage.is_full_document());
    }

    #[test]
    fn large_edit_above_an_inactive_view_reflows_only_its_mapped_neighborhood() {
        let contents = (0..1_000)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (writer_provider, _, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let (reader_provider, reader_bytes, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::new(contents));
        let writer = core.add_view(writer_provider, 240.0, 48.0);
        let reader = core.add_view(reader_provider, 240.0, 48.0);
        core.handle(reader, key('G')).unwrap();
        let before = reader_bytes.load(Ordering::Acquire);

        core.handle(writer, key('i')).unwrap();
        core.handle(writer, text(&"\n".repeat(500))).unwrap();

        let shaped_after_edit = reader_bytes.load(Ordering::Acquire) - before;
        let snapshot = core.layout(reader).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.document_revision, core.document().revision());
        assert!(snapshot.coverage.hard_lines().start > 1_400);
        assert!(snapshot.coverage.hard_lines().len() < 100);
        assert!(
            shaped_after_edit < 10_000,
            "mapped-anchor reflow shaped {shaped_after_edit} bytes"
        );
    }

    #[test]
    fn coordinator_honors_real_provider_context_and_generation() {
        let (main_provider, _, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::FrontendMainThread);
        let mut wrong_core = Core::new(Document::new("one\ntwo"));
        let wrong = wrong_core.add_view(main_provider, 200.0, 48.0);
        assert!(wrong_core.layout(wrong).unwrap().snapshot().is_none());
        assert!(matches!(
            wrong_core.handle(
                wrong,
                CoreEvent::Resize {
                    width: 201.0,
                    height: 48.0,
                }
            ),
            Err(CoreError::LayoutJob(
                LayoutJobError::WrongExecutionContext {
                    required: ProviderThreading::FrontendMainThread,
                    actual: LayoutExecutionContext::WorkerPool,
                }
            ))
        ));

        let (main_provider, _, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::FrontendMainThread);
        let mut main_core = Core::new(Document::new("one\ntwo"));
        let main = main_core.add_view_with_layout_execution_context(
            main_provider,
            200.0,
            48.0,
            LayoutExecutionContext::FrontendMainThread,
        );
        assert!(main_core.layout(main).unwrap().snapshot().is_some());

        let (provider, _, _, generation) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::new("zero\none\ntwo"));
        let view = core.add_view(provider, 200.0, 48.0);
        let request = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
        let (worker_provider, _, _, _) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut worker = LayoutEngine::new(worker_provider);
        let candidate =
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
        generation.store(2, Ordering::Release);
        assert!(matches!(
            core.install_view_layout_job(view, candidate),
            Err(CoreError::LayoutInstall(
                LayoutJobInstallRejection::Cancelled
            ))
        ));
        core.handle(
            view,
            CoreEvent::Resize {
                width: 201.0,
                height: 48.0,
            },
        )
        .unwrap();
        assert_eq!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .metrics_generation,
            MetricsGeneration(2)
        );
    }

    #[test]
    fn successful_prepare_supersedes_but_invalid_prepare_preserves_active_work() {
        let mut core = Core::new(Document::new("zero\none\ntwo\nthree"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let first_token = LayoutCancellationToken::new();
        let first = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                first_token.clone(),
            )
            .unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let first_candidate =
            compute_layout_job(&mut worker, &first, LayoutExecutionContext::WorkerPool).unwrap();

        let cancelled_replacement = LayoutCancellationToken::new();
        cancelled_replacement.cancel();
        assert!(matches!(
            core.prepare_view_layout_job(
                view,
                LayoutJobPriority::ViewportOverscan,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(1..3, 0.0, 48.0).unwrap()),
                cancelled_replacement,
            ),
            Err(CoreError::LayoutJob(LayoutJobError::Cancelled))
        ));
        assert!(!first_token.is_cancelled());

        let invalid = core.prepare_view_layout_job(
            view,
            LayoutJobPriority::ViewportOverscan,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(3..10, 0.0, 48.0).unwrap()),
            LayoutCancellationToken::new(),
        );
        assert!(matches!(
            invalid,
            Err(CoreError::LayoutJob(
                LayoutJobError::RegionOutsideDocument { .. }
            ))
        ));
        assert!(!first_token.is_cancelled());

        let second_token = LayoutCancellationToken::new();
        let second = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::NewlyExposedRows,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(1..4, 16.0, 48.0).unwrap()),
                second_token.clone(),
            )
            .unwrap();
        assert!(first_token.is_cancelled());
        assert!(!second_token.is_cancelled());
        assert!(matches!(
            core.install_view_layout_job(view, first_candidate),
            Err(CoreError::LayoutInstall(
                LayoutJobInstallRejection::Cancelled
            ))
        ));

        let second_candidate =
            compute_layout_job(&mut worker, &second, LayoutExecutionContext::WorkerPool).unwrap();
        core.install_view_layout_job(view, second_candidate)
            .unwrap();
        assert!(
            !second_token.is_cancelled(),
            "completion retires rather than cancels the installed token"
        );
        core.handle(
            view,
            CoreEvent::Resize {
                width: 201.0,
                height: 48.0,
            },
        )
        .unwrap();
        assert!(
            !second_token.is_cancelled(),
            "later invalidation must not cancel already installed work"
        );
    }

    #[test]
    fn reusing_an_active_cancellation_token_is_rejected_without_orphaning_it() {
        let mut core = Core::new(Document::new("zero\none\ntwo"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let token = LayoutCancellationToken::new();
        let first = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                token.clone(),
            )
            .unwrap();
        assert!(matches!(
            core.prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(
                    ViewportLayoutRegion::new(1..3, 0.0, 48.0).unwrap()
                ),
                token.clone(),
            ),
            Err(CoreError::LayoutJob(
                LayoutJobError::CancellationTokenAlreadyActive { job_id }
            )) if job_id == first.job_id()
        ));
        assert!(!token.is_cancelled());
    }

    #[test]
    fn removing_a_view_cancels_work_and_reports_typed_cleanup() {
        let mut core = Core::new(Document::new("zero\none\ntwo"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let token = LayoutCancellationToken::new();
        let request = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                token.clone(),
            )
            .unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let completed_before_removal =
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();

        assert_eq!(
            core.remove_view(view).unwrap(),
            ViewRemovalOutcome {
                edit_group_closed: false,
                composition_discarded: false,
                layout_work_cancelled: true,
            }
        );
        assert!(token.is_cancelled());
        assert!(matches!(
            compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool),
            Err(LayoutJobError::Cancelled)
        ));
        assert!(matches!(
            core.install_view_layout_job(view, completed_before_removal),
            Err(CoreError::UnknownView(id)) if id == view
        ));
        assert_eq!(core.remove_view(view), Err(CoreError::UnknownView(view)));
        assert!(matches!(
            core.layout_provider_requirements(view),
            Err(CoreError::UnknownView(id)) if id == view
        ));
    }

    #[test]
    fn layout_cancellation_is_isolated_per_view() {
        let mut core = Core::new(Document::new("zero\none\ntwo\nthree"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 100.0, 48.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 48.0);
        let first_token = LayoutCancellationToken::new();
        let second_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            first,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            first_token.clone(),
        )
        .unwrap();
        assert!(matches!(
            core.prepare_view_layout_job(
                second,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                first_token.clone(),
            ),
            Err(CoreError::LayoutJob(
                LayoutJobError::CancellationTokenAlreadyActive { .. }
            ))
        ));
        assert!(!first_token.is_cancelled());
        core.prepare_view_layout_job(
            second,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            second_token.clone(),
        )
        .unwrap();

        let replacement_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            first,
            LayoutJobPriority::ViewportOverscan,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(1..4, 0.0, 48.0).unwrap()),
            replacement_token.clone(),
        )
        .unwrap();
        assert!(first_token.is_cancelled());
        assert!(!second_token.is_cancelled());
        core.remove_view(first).unwrap();
        assert!(replacement_token.is_cancelled());
        assert!(!second_token.is_cancelled());
    }

    #[test]
    fn configuration_metrics_and_document_changes_cancel_obsolete_work() {
        let (provider, _, _, generation) =
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker);
        let mut core = Core::new(Document::new("zero\none\ntwo\nthree"));
        let writer = core.add_view(provider, 200.0, 48.0);
        let reader = core.add_view(
            InstrumentedCoordinatorProvider::new(ProviderThreading::AnyWorker).0,
            300.0,
            48.0,
        );

        let resize_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            writer,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            resize_token.clone(),
        )
        .unwrap();
        core.handle(
            writer,
            CoreEvent::Resize {
                width: 180.0,
                height: 48.0,
            },
        )
        .unwrap();
        assert!(resize_token.is_cancelled());

        let wrap_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            writer,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            wrap_token.clone(),
        )
        .unwrap();
        core.handle(writer, CoreEvent::SetWrap(false)).unwrap();
        assert!(wrap_token.is_cancelled());

        let metrics_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            writer,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            metrics_token.clone(),
        )
        .unwrap();
        generation.store(2, Ordering::Release);
        let already_cancelled = LayoutCancellationToken::new();
        already_cancelled.cancel();
        assert!(matches!(
            core.prepare_view_layout_job(
                writer,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                already_cancelled,
            ),
            Err(CoreError::LayoutJob(LayoutJobError::Cancelled))
        ));
        assert!(metrics_token.is_cancelled());

        // Make the invoking view exact for the new metrics before isolating
        // cancellation caused specifically by the source commit below.
        core.handle(writer, key('i')).unwrap();
        let writer_token = LayoutCancellationToken::new();
        let reader_token = LayoutCancellationToken::new();
        core.prepare_view_layout_job(
            writer,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            writer_token.clone(),
        )
        .unwrap();
        core.prepare_view_layout_job(
            reader,
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
            reader_token.clone(),
        )
        .unwrap();
        core.handle(writer, text("X")).unwrap();
        assert!(writer_token.is_cancelled());
        assert!(reader_token.is_cancelled());
    }

    #[test]
    fn identifiers_exhaust_without_wrapping_or_reuse() {
        let mut exhausted_on_attach = Core::new(Document::new("one"));
        exhausted_on_attach.next_layout_job = None;
        assert!(matches!(
            exhausted_on_attach.try_add_view(MockTextMeasurementProvider::new(), 200.0, 48.0),
            Err(CoreError::IdentifierExhausted(
                CoreIdentifierKind::LayoutJob
            ))
        ));
        assert!(exhausted_on_attach.views.is_empty());

        let mut core = Core::new(Document::new("one\ntwo"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.remove_view(first).unwrap();
        let second = core
            .try_add_view(MockTextMeasurementProvider::new(), 200.0, 48.0)
            .unwrap();
        assert!(second > first);

        core.next_view = Some(u64::MAX);
        let last = core
            .try_add_view(MockTextMeasurementProvider::new(), 200.0, 48.0)
            .unwrap();
        assert_eq!(last, ViewId(u64::MAX));
        assert!(matches!(
            core.try_add_view(MockTextMeasurementProvider::new(), 200.0, 48.0),
            Err(CoreError::IdentifierExhausted(CoreIdentifierKind::View))
        ));

        core.next_layout_job = Some(u64::MAX);
        let request = core
            .prepare_view_layout_job(
                second,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
        assert_eq!(request.job_id(), LayoutJobId(u64::MAX));
        assert!(matches!(
            core.prepare_view_layout_job(
                second,
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                LayoutCancellationToken::new(),
            ),
            Err(CoreError::IdentifierExhausted(
                CoreIdentifierKind::LayoutJob
            ))
        ));
        assert!(matches!(
            core.prepare_view_layout_job(
                ViewId(99),
                LayoutJobPriority::Background,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 48.0).unwrap()),
                LayoutCancellationToken::new(),
            ),
            Err(CoreError::UnknownView(ViewId(99)))
        ));
    }

    #[test]
    fn removing_insert_owner_closes_undo_group_and_removing_ime_discards_overlay() {
        let mut core = Core::new(Document::new("base"));
        let editing = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let observer = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(editing, key('i')).unwrap();
        core.handle(editing, text("ab")).unwrap();
        assert_eq!(core.document().text(), "abbase");
        let removed = core.remove_view(editing).unwrap();
        assert!(removed.edit_group_closed);
        assert!(!removed.composition_discarded);
        core.handle(observer, key('u')).unwrap();
        assert_eq!(core.document().text(), "base");

        let composing = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        let before = core.document().revision();
        core.handle(composing, begin_composition(&core, 0..0))
            .unwrap();
        core.handle(
            composing,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
                marked_text: "marked".to_owned(),
                selected_range: 6..6,
            })),
        )
        .unwrap();
        let removed = core.remove_view(composing).unwrap();
        assert!(removed.composition_discarded);
        assert!(!removed.edit_group_closed);
        assert_eq!(core.document().revision(), before);
        assert_eq!(core.document().text(), "base");
    }

    #[test]
    fn planned_insert_failure_publishes_no_model_or_success_controller_effects() {
        let document = Document::from_bytes_with_file_format(
            b"base".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            crate::document::FileFormat::Unix,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        let before_revision = core.document().revision();
        let before_history = core.document().history_status().current;
        let before_cursor = core.command_state(view).unwrap().cursor();

        let error = core.handle(view, text("🙂")).unwrap_err();
        assert!(matches!(
            error,
            CoreError::Document(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                ..
            })
        ));
        assert_eq!(core.document().text(), "base");
        assert_eq!(core.document().revision(), before_revision);
        assert_eq!(core.document().history_status().current, before_history);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);

        // The rejected payload was not appended to the Insert repeat recipe.
        core.handle(view, text("x")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "xbase");
    }

    #[test]
    fn stale_command_plan_is_rejected_before_controller_publication() {
        let mut core = Core::new(Document::new("base"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('"')).unwrap();
        core.handle(view, key('a')).unwrap();
        let plan = {
            let context = CommandContext::new(&core.document);
            match core
                .views
                .get(&view)
                .unwrap()
                .commands
                .resolve(&context, InputEvent::key('x'))
                .unwrap()
            {
                CommandResolution::Planned(plan) => plan,
                CommandResolution::Legacy(reason) => {
                    panic!("Normal x unexpectedly used legacy path: {reason:?}")
                }
            }
        };
        let cursor_before = core.command_state(view).unwrap().cursor();
        let mode_before = core.command_state(view).unwrap().mode();

        core.document.insert(0, "z").unwrap();
        let revision_after_newer_edit = core.document.revision();
        let error = {
            let view = core.views.get_mut(&view).unwrap();
            execute_command_plan(&mut core.document, &mut view.commands, plan).unwrap_err()
        };
        assert!(matches!(
            error,
            CoreError::Document(DocumentError::WrongSnapshot {
                expected,
                actual
            }) if expected == revision_after_newer_edit && actual == Revision(0)
        ));
        assert_eq!(core.document().text(), "zbase");
        assert_eq!(core.document().revision(), revision_after_newer_edit);
        assert_eq!(core.command_state(view).unwrap().mode(), mode_before);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor_before);
        assert!(core.command_state(view).unwrap().register('a').is_none());
    }

    #[test]
    fn planned_normal_delete_publishes_register_repeat_and_model_together() {
        let mut core = Core::new(Document::new("abcd"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('2')).unwrap();
        let outcome = core.handle(view, key('x')).unwrap();
        assert_eq!(core.document().text(), "cd");
        assert_eq!(
            core.command_state(view).unwrap().register('"'),
            Some(&crate::command::RegisterValue::characterwise("ab"))
        );
        assert!(outcome.document_changed);
        assert!(outcome.position_map.is_some());

        core.handle(view, key('.')).unwrap();
        assert_eq!(core.document().text(), "");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "cd");
    }

    #[test]
    fn insert_ctrl_o_delete_keeps_legacy_return_to_insert_semantics() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();

        core.handle(view, key('x')).unwrap();
        assert_eq!(core.document().text(), "bc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);

        core.handle(view, text("z")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "zbc");
    }

    #[test]
    fn insert_ctrl_o_reopened_group_uses_the_post_normal_restoration() {
        let mut core = Core::new(Document::new("one two"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        core.handle(view, key('d')).unwrap();
        core.handle(view, key('w')).unwrap();
        assert_eq!(core.document().text(), "two");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);

        core.handle(view, text("new ")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "new two");

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "two");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "one two");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }

    #[test]
    fn insert_ctrl_o_change_keeps_its_pre_change_group_restoration() {
        let mut core = Core::new(Document::new("one two"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
            .unwrap();
        core.handle(view, key('c')).unwrap();
        core.handle(view, key('w')).unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        core.handle(view, text("X")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "X two");

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "one two");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }

    #[test]
    fn planned_logical_motion_requests_caret_reveal_without_model_change() {
        let mut core = Core::new(Document::new("one two three"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 40.0, 24.0);
        let revision = core.document().revision();
        let outcome = core.handle(view, key('w')).unwrap();

        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        assert_eq!(core.document().revision(), revision);
        assert!(!outcome.document_changed);
        assert!(outcome.position_map.is_none());
        assert!(outcome.layout_changed);
    }

    #[test]
    fn planned_edit_keys_preserve_replace_journals_and_atomic_hard_breaks() {
        let mut replace = Core::new(Document::new("abc"));
        let view = replace.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        replace.handle(view, key('R')).unwrap();
        replace.handle(view, text("x")).unwrap();
        assert_eq!(replace.document().text(), "xbc");
        replace
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(replace.document().text(), "abc");
        assert_eq!(replace.command_state(view).unwrap().cursor(), 0);

        let document = Document::from_bytes_with_file_format(
            b"\r\nx".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            crate::document::FileFormat::Unix,
        )
        .unwrap();
        let mut hard_break = Core::new(document);
        let view = hard_break.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        hard_break.handle(view, key('a')).unwrap();
        assert_eq!(hard_break.command_state(view).unwrap().cursor(), 1);
        hard_break
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Delete)))
            .unwrap();
        assert_eq!(hard_break.document().text(), "\rx");
        assert_eq!(hard_break.document().source_bytes(), b"\rx");
    }

    #[test]
    fn one_step_history_navigation_uses_a_revision_bound_plan() {
        let mut core = Core::new(Document::new("base"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("x")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "xbase");

        let undo = core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "base");
        assert!(undo
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation));

        let redo = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().text(), "xbase");
        assert!(redo
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation));
    }

    #[test]
    fn native_history_from_insert_finalizes_the_open_group_and_restores_both_edges() {
        let mut core = Core::new(Document::new("base"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("xy")).unwrap();
        assert_eq!(core.document().text(), "xybase");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(core.document().edit_group_depth() > 0);

        let undo = core
            .handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
        assert_eq!(core.document().text(), "base");
        assert_eq!(core.document().edit_group_depth(), 0);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert!(undo.document_changed);
        assert!(undo.position_map.is_some());
        assert!(undo
            .command
            .as_ref()
            .is_some_and(|command| { command.history_navigation && command.mode_changed }));

        let redo = core
            .handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
        assert_eq!(core.document().text(), "xybase");
        assert_eq!(core.document().edit_group_depth(), 0);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            2,
            "redo restores the Insert boundary recorded when native Undo closed the group"
        );
        assert!(redo
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation));
    }

    #[test]
    fn native_history_from_visual_discards_selection_and_uses_recorded_restoration() {
        let mut core = Core::new(Document::new("base"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 48.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("x")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "xbase");

        core.handle(view, key('v')).unwrap();
        core.handle(view, key('l')).unwrap();
        assert_eq!(
            core.command_state(view).unwrap().mode(),
            Mode::VisualCharacter
        );
        assert!(core.command_state(view).unwrap().visual_anchor().is_some());

        let undo = core
            .handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
        assert_eq!(core.document().text(), "base");
        let commands = core.command_state(view).unwrap();
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), 0);
        assert_eq!(commands.visual_anchor(), None);
        assert!(undo
            .command
            .as_ref()
            .is_some_and(|command| command.history_navigation && command.mode_changed));
    }

    #[test]
    fn headless_layout_motion_is_explicitly_unsupported() {
        let mut document = Document::new("one two three");
        let mut commands = CommandInterpreter::new();
        commands
            .handle(&mut document, InputEvent::key('g'))
            .unwrap();
        let output = commands
            .handle(&mut document, InputEvent::key('j'))
            .unwrap();
        assert!(matches!(
            output.status,
            CommandStatus::Unsupported(message) if message.contains("requires layout context")
        ));
        assert_eq!(commands.cursor(), 0);
    }

    #[test]
    fn live_resize_bounds_reflow_by_previously_visible_paragraphs() {
        let paragraph = "alpha beta gamma delta ".repeat(20) + "\n";
        let mut core = Core::new(Document::new(paragraph.repeat(5_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
        let shaped = core.views[&view].engine.provider().request_calls();
        for width in [590.0, 550.0, 450.0, 700.0] {
            core.handle(view, CoreEvent::Resize { width, height: 400.0 }).unwrap();
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            assert!(snapshot.coverage.hard_lines().len() < 32,
                "resize must not turn estimated hard-line heights into screens of overscan");
            assert_eq!(viewport_layout_extension_needed(layout), (false, false));
            assert_eq!(snapshot.viewport_width, width);
            assert_eq!(core.views[&view].engine.provider().request_calls(), shaped,
                "width changes must reuse shaped text");

            let mut fresh = Core::new(Document::new(paragraph.repeat(100)));
            let fresh_view = fresh.add_view(MockTextMeasurementProvider::new(), width, 400.0);
            let visible_rows = |snapshot: &crate::layout::LayoutSnapshot| snapshot.rows.iter()
                .filter(|row| row.y < 400.0)
                .map(|row| (row.text_range.clone(), row.y, row.baseline, row.width))
                .collect::<Vec<_>>();
            assert_eq!(visible_rows(snapshot), visible_rows(fresh.layout(fresh_view).unwrap().snapshot().unwrap()),
                "the old geometry is a hint, never the new wrapping result");
        }

        // A much larger viewport needs more lines than the old hint provides.
        core.handle(view, CoreEvent::Resize { width: 4_000.0, height: 2_000.0 }).unwrap();
        assert_eq!(viewport_layout_extension_needed(core.layout(view).unwrap()), (false, false));
        assert!(core.layout(view).unwrap().snapshot().unwrap().coverage.hard_lines().len() > 32);

        core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2));
        core.handle(view, CoreEvent::Resize { width: 550.0, height: 400.0 }).unwrap();
        let layout = core.layout(view).unwrap();
        assert_eq!(layout.snapshot().unwrap().metrics_generation, MetricsGeneration(2));
        assert_eq!(viewport_layout_extension_needed(layout), (false, false));
        assert!(core.views[&view].engine.provider().request_calls() > shaped);

        core.document.insert(0, "new paragraph\n").unwrap();
        core.handle(view, CoreEvent::Resize { width: 600.0, height: 400.0 }).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.document_revision, core.document.revision());
        assert_eq!(snapshot.rows[0].text_range, 0..13);
    }

    #[test]
    fn pointer_drag_reuses_visible_layout_in_a_large_document() {
        let mut core = Core::new(Document::new("alpha beta gamma\n".repeat(25_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 300.0);
        let revision = core.document().revision();
        let jobs = core.next_layout_job;
        let layout_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        for offset in (0..100).chain((0..100).rev()) {
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: offset != 0,
            }).unwrap();
        }
        assert_eq!(core.next_layout_job, jobs, "dragging must not schedule visible reflow");
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().revision, layout_revision);
        assert_eq!(core.document().revision(), revision);

        // A destination outside coverage still materializes and reveals its row.
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: revision,
            text_offset: 200_000,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: true,
        }).unwrap();
        assert!(core.next_layout_job > jobs);
        let layout = core.layout(view).unwrap();
        assert!(layout.snapshot().unwrap().coverage.contains_text_offset(200_000));
        assert!(layout.viewport_top() > 0.0);
    }

    #[test]
    fn pointer_reuse_rejects_stale_document_configuration_and_metrics() {
        let mut core = Core::new(Document::new("alpha beta gamma\n".repeat(100)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 300.0);
        for invalidation in 0..3 {
            let previous = core.layout(view).unwrap().snapshot().unwrap().revision;
            match invalidation {
                0 => { core.document.insert(0, "new ").unwrap(); }
                1 => { core.views.get_mut(&view).unwrap().layout.resize(180.0, 200.0); }
                _ => core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2)),
            }
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 0,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            }).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert_ne!(snapshot.revision, previous);
            assert_eq!(snapshot.document_revision, core.document().revision());
        }
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().metrics_generation, MetricsGeneration(2));
    }

    #[test]
    fn native_undo_publishes_restored_marks_before_typing_in_a_new_view() {
        let mut core = Core::new(Document::new("alpha beta"));
        let first = core.add_view(MockTextMeasurementProvider::new(), 240.0, 120.0);
        core.handle(first, CoreEvent::SelectAll {
            document: core.document().id(), revision: core.document().revision(),
        }).unwrap();
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(first, key('g')).unwrap();
        core.handle(first, key('g')).unwrap();
        core.handle(first, key('i')).unwrap();
        core.handle(first, text("extra ")).unwrap();
        core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(first, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().text(), "alpha beta");
        let second = core.add_view(MockTextMeasurementProvider::new(), 240.0, 120.0);
        core.handle(second, key('i')).unwrap();
        core.handle(second, text("new ")).unwrap();
        core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        assert_eq!(core.document().text(), "new alpha beta");
    }

    #[test]
    fn native_pointer_placement_is_revision_bound_and_owns_visual_selection() {
        let mut core = Core::new(Document::new("alpha beta"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        let revision = core.document().revision();

        let outcome = core
            .handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: revision,
                    text_offset: 6,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
        assert!(outcome.layout_changed);

        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: 9,
                affinity: BoundaryAffinity::Upstream,
                extend_selection: true,
            },
        )
        .unwrap();
        let commands = core.command_state(view).unwrap();
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.visual_anchor(), Some(6));
        assert_eq!(commands.boundary_affinity(), BoundaryAffinity::Upstream);

        let stale = core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: Revision(revision.0 + 1),
                text_offset: 0,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        );
        assert!(matches!(
            stale,
            Err(CoreError::Document(DocumentError::WrongSnapshot { .. }))
        ));
        assert_eq!(core.command_state(view).unwrap().cursor(), 9);
    }

    #[test]
    fn pointer_at_hard_line_end_canonicalizes_normal_cursor_affinity() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);

        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 3,
                affinity: BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();

        let commands = core.command_state(view).unwrap();
        assert_eq!(
            commands.cursor(),
            2,
            "Normal mode addresses the final grapheme"
        );
        assert_eq!(
            commands.boundary_affinity(),
            BoundaryAffinity::Downstream,
            "the normalized start boundary must still associate that final grapheme"
        );

        core.handle(view, key('x')).unwrap();
        assert_eq!(core.document().text(), "ab");
    }

    #[test]
    fn pointer_motion_breaks_an_insert_undo_group_before_moving() {
        let mut core = Core::new(Document::new("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        core.handle(view, key('i')).unwrap();
        core.handle(view, text("x")).unwrap();
        assert_eq!(core.document().text(), "xabc");

        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 2,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(view, text("y")).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert_eq!(core.document().text(), "xaybc");

        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "xabc");
        core.handle(view, key('u')).unwrap();
        assert_eq!(core.document().text(), "abc");
    }

    #[test]
    fn cross_view_pointer_break_records_the_insert_group_owners_restoration() {
        let mut core = Core::new(Document::new("base"));
        let owner = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);
        let invoking = core.add_view(MockTextMeasurementProvider::new(), 240.0, 80.0);

        core.handle(
            invoking,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 3,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(owner, key('i')).unwrap();
        core.handle(owner, text("xy")).unwrap();
        assert_eq!(core.document().text(), "xybase");
        assert_eq!(core.command_state(owner).unwrap().cursor(), 2);
        assert_ne!(core.command_state(invoking).unwrap().cursor(), 2);
        assert!(core.document().edit_group_depth() > 0);

        core.handle(
            invoking,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 4,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        assert_eq!(core.document().edit_group_depth(), 0);

        core.handle(
            invoking,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().text(), "base");
        assert_eq!(core.command_state(invoking).unwrap().cursor(), 0);

        core.handle(
            invoking,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
        )
        .unwrap();
        assert_eq!(core.document().text(), "xybase");
        assert_eq!(
            core.command_state(invoking).unwrap().cursor(),
            2,
            "redo must use the final Insert boundary captured from the owning view"
        );
    }
}

#[cfg(test)]
mod long_line_focus_tests {
    use super::*;
    use crate::command::LineMode;
    use crate::layout::MockTextMeasurementProvider;

    fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
        for value in input.chars() {
            let outcome = core
                .handle(view, CoreEvent::Input(InputEvent::key(value)))
                .unwrap();
            assert!(matches!(
                outcome.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ));
        }
    }
    fn assert_caret_visible(core: &Core<MockTextMeasurementProvider>, id: ViewId) {
        let view = &core.views[&id];
        let snapshot = view.layout.snapshot().unwrap();
        let position = view.commands.visual_position().unwrap_or(
            crate::command::layout_motion::VisualPosition {
                text_offset: view.commands.cursor(),
                affinity: view.commands.boundary_affinity(),
            },
        );
        assert!(snapshot.coverage.contains_text_offset(position.text_offset));
        let geometry = snapshot
            .logical_endpoint_geometry(position.text_offset, position.affinity)
            .unwrap();
        assert!(geometry.rect.y >= view.layout.viewport_top() - 0.01);
        assert!(
            geometry.rect.y + geometry.rect.height
                <= view.layout.viewport_top() + view.layout.height() + 0.01
        );
        assert_eq!(view.layout.last_error(), None);
    }

    #[test]
    fn theme_padding_keeps_a_top_pinned_view_at_the_document_origin() {
        let mut core = Core::new(Document::new("paragraph\n".repeat(20_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 200.0);
        assert_eq!(core.layout(view).unwrap().viewport_top(), 0.0);
        core.set_view_insets(
            view,
            crate::layout::EdgeInsets {
                top: 28.0,
                left: 22.0,
                right: 22.0,
                bottom: 28.0,
            },
        )
        .unwrap();
        assert_eq!(core.layout(view).unwrap().viewport_top(), 0.0);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows[0].y >= 28.0);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 100);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(200.0),
            },
        )
        .unwrap();
        core.set_view_insets(
            view,
            crate::layout::EdgeInsets {
                top: 38.0,
                left: 22.0,
                right: 22.0,
                bottom: 28.0,
            },
        )
        .unwrap();
        assert!(
            core.layout(view).unwrap().viewport_top() > 0.0,
            "an already scrolled view retains its content anchor"
        );
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(0.0),
            },
        )
        .unwrap();
        core.set_view_insets(
            view,
            crate::layout::EdgeInsets {
                top: 18.0,
                left: 22.0,
                right: 22.0,
                bottom: 28.0,
            },
        )
        .unwrap();
        assert_eq!(core.layout(view).unwrap().viewport_top(), 0.0);
        assert!(!core.document().is_dirty());
    }

    #[test]
    fn terminal_empty_line_does_not_capture_a_long_preceding_paragraph_as_overscan() {
        let source = "word ".repeat(40_000) + "\nshort\nlast\n";
        let mut core = Core::new(Document::new(source.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 300.0);
        let before = core.next_layout_job.unwrap();
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(f32::MAX),
            },
        )
        .unwrap();
        assert!(
            core.next_layout_job.unwrap() - before >= 4,
            "the long paragraph advances through bounded chunk jobs, not one full overscan capture"
        );
        assert!(core.views[&view].long_line_checkpoints.len() >= 3);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows.len() < 100);
        assert!(snapshot.rows.first().unwrap().fragment_index > 0);
        assert_eq!(snapshot.rows.last().unwrap().text_range.end, source.len());
    }

    #[test]
    fn document_end_reuses_checkpoints_and_a_failed_chunk_keeps_the_old_viewport() {
        let source = "abcdef ".repeat(30_000);
        let mut core = Core::new(Document::new(source.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 90.0, 200.0);
        let original = core.viewport_state(view).unwrap();
        core.views
            .get_mut(&view)
            .unwrap()
            .engine
            .provider_mut()
            .fail_next_batch("end chunk failed");
        let request = CoreEvent::SetViewportOrigin {
            left: 0.0,
            top: Some(f32::MAX),
        };
        assert!(core.handle(view, request.clone()).is_err());
        assert_eq!(core.viewport_state(view).unwrap(), original);
        core.handle(view, request.clone()).unwrap();
        assert!(core.views[&view].long_line_checkpoints.len() >= 3);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 100);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(0.0),
            },
        )
        .unwrap();
        let jobs = core.next_layout_job.unwrap();
        core.handle(view, request).unwrap();
        assert!(
            core.next_layout_job.unwrap() - jobs <= 2,
            "returning to the document end resumes at the checkpoint before its viewport tail"
        );
        assert_eq!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows
                .last()
                .unwrap()
                .text_range
                .end,
            source.len()
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }

    #[test]
    fn distant_caret_resumes_bounded_long_line_jobs_and_reuses_checkpoints() {
        let long = "abcdef ".repeat(30_000);
        let source = format!("{long}\n{}", "unrelated paragraph\n".repeat(20_000));
        let mut core = Core::new(Document::new(source.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 90.0, 200.0);
        assert!(
            core.views[&view].engine.provider().request_calls() < 40,
            "opening a giant paragraph does not shape the whole paragraph or neighboring document"
        );
        core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
            .unwrap();
        keys(&mut core, view, "$");
        assert_eq!(core.command_state(view).unwrap().cursor(), long.len() - 1);
        assert_caret_visible(&core, view);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows[0].fragment_index > 0);
        assert!(snapshot.rows.iter().all(|row| row.hard_line_index == 0));
        assert!(
            snapshot.rows.last().unwrap().text_range.end - snapshot.rows[0].text_range.start
                <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES
        );
        assert!(core.views[&view].long_line_checkpoints.len() >= 3);
        keys(&mut core, view, "0");
        assert_caret_visible(&core, view);
        let requests = core.views[&view].engine.provider().request_calls();
        keys(&mut core, view, "$");
        assert_caret_visible(&core, view);
        assert!(
            core.views[&view].engine.provider().request_calls() - requests < 25,
            "revisiting the tail shapes at most its final slice"
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(!core.document().is_dirty());
    }

    #[test]
    fn long_line_checkpoints_invalidate_on_resize_metrics_edit_and_undo() {
        let original = "abcdef ".repeat(20_000);
        let mut core = Core::new(Document::new(original.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 90.0, 200.0);
        core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
            .unwrap();
        keys(&mut core, view, "$");
        assert_caret_visible(&core, view);
        let old = core.views[&view]
            .long_line_checkpoints
            .values()
            .next()
            .unwrap()
            .clone();
        core.handle(
            view,
            CoreEvent::Resize {
                width: 145.0,
                height: 200.0,
            },
        )
        .unwrap();
        assert!(core.views[&view]
            .long_line_checkpoints
            .values()
            .all(|checkpoint| checkpoint.configuration_generation()
                != old.configuration_generation()));
        keys(&mut core, view, "$");
        assert_caret_visible(&core, view);
        core.views
            .get_mut(&view)
            .unwrap()
            .engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        core.handle(
            view,
            CoreEvent::Resize {
                width: 145.0,
                height: 200.0,
            },
        )
        .unwrap();
        assert!(core.views[&view]
            .long_line_checkpoints
            .values()
            .all(|checkpoint| checkpoint.metrics_generation() == MetricsGeneration(2)));
        keys(&mut core, view, "$");
        assert_caret_visible(&core, view);
        let jobs_before_edit = core.next_layout_job.unwrap();
        keys(&mut core, view, "x");
        assert!(core.next_layout_job.unwrap() - jobs_before_edit <= 2, "an edit after a validated prefix resumes from that prefix instead of wrapping it again");
        assert!(core.views[&view]
            .long_line_checkpoints
            .values()
            .all(|checkpoint| checkpoint.document_revision() == core.document().revision()));
        assert_caret_visible(&core, view);
        keys(&mut core, view, "u");
        assert_caret_visible(&core, view);
        assert_eq!(core.document().source_bytes(), original.as_bytes());
        assert!(core.views[&view]
            .long_line_checkpoints
            .values()
            .all(|checkpoint| checkpoint.document_revision() == core.document().revision()));
        keys(&mut core, view, "0x");
        assert_eq!(
            core.views[&view].long_line_checkpoints.len(),
            1,
            "an edit in the consumed prefix discards every dependent continuation"
        );
        keys(&mut core, view, "$");
        assert_caret_visible(&core, view);
    }
}
