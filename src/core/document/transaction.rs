//! Revision-bound document requests and two-phase model transactions.
//!
//! Preparation performs every fallible reverse-projection, encoding, and
//! verification step against immutable document state.  A prepared value owns
//! the candidate state needed for publication.  Commit only rechecks the
//! captured model preconditions and installs that already-verified candidate.

use super::formatted_text::{FormattedTextSpliceStats, LogicalGraphemeSnapshot};
use super::line_endings::{detect, normalize};
use super::projection::{escape_markdown_insert, project, splice_line_local_projection};
use super::source_line_index::SourceHardLineSpliceStats;
use super::transfer::{self, HardLineTransfer};
use super::{
    build_state_from_decoded, hard_line_source_image_from_state, ranges_overlap,
    spell_logical_breaks, Association, BlockProperties, BoundaryAffinity, CharacterProperties,
    ConfigurationStyleIntent, DeletionRecovery, Document, DocumentError, DocumentId, DocumentState,
    DocumentStyleAssignment, FileFormat, FileFormatOrigin, Format, FormattedDocument,
    FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload, FormattedTextTree,
    HardLineSourceImage, HistoryChangeNumber, HistoryError, HistoryLocation, HistoryNavigation,
    HistoryNodeId, HistoryRestoration, HistoryRestorationSnapshot, HistorySemanticChangeKind,
    HistorySourcePatch, HistoryTransactionSummary, MappingOutcome, PipelineCapabilityDecision,
    PipelineEditIntent, PipelinePolicyRequest, PositionError, PositionMap, Revision,
    SemanticInlineStyle, SourcePartId, Splice, StyleApplication, StyleDefinitionEdit, StyleError,
    StyleId, StyleInvalidationEffect, StyleProperty, StyleSheet, StyleSheetRevision, StyleSpan,
    TextEdit, TextRange, UnsupportedEditReason,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// A model-level history request, independent of Vim command spelling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryNavigationRequest {
    Undo,
    Redo,
    SelectNode(HistoryNodeId),
    SelectChange(HistoryChangeNumber),
}

/// Paragraph/document target for a source-backed block-style operation.
/// Paragraph ranges are exact snapshot ranges; adapters decide which touched
/// paragraph-bearing blocks participate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleBlockTarget {
    DocumentRoot,
    Paragraphs(TextRange),
}

/// Provenance/editability class of a normalized style definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleDefinitionOrigin {
    SourceBacked,
    GeneratedConfiguration,
    SyntheticReadOnly,
}

/// A persisted-content style intention. These are deliberately representable
/// even when an adapter cannot translate them, so capability failures are
/// typed and non-destructive instead of silently creating a sidecar.
#[derive(Clone, Debug, PartialEq)]
pub enum PersistedStyleIntent {
    AssignBlockStyle {
        target: StyleBlockTarget,
        style: StyleId,
    },
    AssignCharacterStyle {
        range: TextRange,
        style: StyleId,
    },
    SetDirectCharacterProperties {
        range: TextRange,
        properties: CharacterProperties,
    },
    ClearDirectCharacterProperties {
        range: TextRange,
        properties: BTreeSet<StyleProperty>,
    },
    SetDirectBlockProperties {
        target: StyleBlockTarget,
        properties: BlockProperties,
    },
    ClearDirectBlockProperties {
        target: StyleBlockTarget,
        properties: BTreeSet<StyleProperty>,
    },
    EditStyleDefinition {
        origin: StyleDefinitionOrigin,
        edit: StyleDefinitionEdit,
    },
}

/// Typed model-level style operation bound to one exact document revision.
/// Configuration is a distinct authority from source-backed content styling.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleModelIntent {
    Persisted(PersistedStyleIntent),
    Configuration(ConfigurationStyleIntent),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleModelRequest {
    document: DocumentId,
    revision: Revision,
    intent: StyleModelIntent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StylePropertyTarget {
    DocumentCanvas,
    Character,
}

/// Structured style-specific failure retained under one model-transaction
/// error arm so general coordinators need not understand adapter policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StyleTransactionError {
    Definition(StyleError),
    Unsupported {
        format: Format,
        reason: UnsupportedEditReason,
    },
    NeedsPolicy(PipelinePolicyRequest),
    TranslationUnavailable,
    DefinitionReadOnly(StyleId),
    ConfigurationIntentRequired(StyleId),
    InvalidPropertyTarget {
        property: StyleProperty,
        target: StylePropertyTarget,
    },
}

impl StyleModelRequest {
    pub fn new(document: DocumentId, revision: Revision, intent: StyleModelIntent) -> Self {
        Self {
            document,
            revision,
            intent,
        }
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn intent(&self) -> &StyleModelIntent {
        &self.intent
    }
}

/// A typed model operation bound to one exact document revision.
///
/// The numeric ranges in edit variants are interpreted only in the formatted
/// snapshot named by `document` and `revision`.  Preparation rejects a request
/// rather than silently rebasing it to the current state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelRequest {
    ApplyTextEdits {
        document: DocumentId,
        revision: Revision,
        edits: Vec<TextEdit>,
    },
    SetSemanticStyle {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    },
    SetFileFormat {
        document: DocumentId,
        revision: Revision,
        target: FileFormat,
    },
    TransferHardLines {
        document: DocumentId,
        revision: Revision,
        operation: HardLineTransfer,
        /// Non-empty, half-open zero-based authoritative hard-line ordinals.
        source_lines: Range<usize>,
        /// Insertion gap in the pre-transaction hard-line order. Valid gaps
        /// are `0..=line_count`.
        destination: usize,
    },
    RestoreHardLineSource {
        document: DocumentId,
        revision: Revision,
        /// Current zero-based ordinal of the stable hard line named by the
        /// retained image. Cursor location is deliberately irrelevant.
        target_line: usize,
        image: HardLineSourceImage,
    },
    NavigateHistory {
        document: DocumentId,
        revision: Revision,
        navigation: HistoryNavigationRequest,
    },
}

impl ModelRequest {
    pub fn document(&self) -> DocumentId {
        match self {
            Self::ApplyTextEdits { document, .. }
            | Self::SetSemanticStyle { document, .. }
            | Self::SetFileFormat { document, .. }
            | Self::TransferHardLines { document, .. }
            | Self::RestoreHardLineSource { document, .. }
            | Self::NavigateHistory { document, .. } => *document,
        }
    }

    pub fn revision(&self) -> Revision {
        match self {
            Self::ApplyTextEdits { revision, .. }
            | Self::SetSemanticStyle { revision, .. }
            | Self::SetFileFormat { revision, .. }
            | Self::TransferHardLines { revision, .. }
            | Self::RestoreHardLineSource { revision, .. }
            | Self::NavigateHistory { revision, .. } => *revision,
        }
    }
}

/// The model-level reason for a source transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelChangeKind {
    TextEdits,
    SemanticStyle,
    ConfigurationStyle,
    FileFormat,
    HardLineTransfer,
    HardLineSourceRestoration,
    SourceMetadata,
    HistoryNavigation,
    NoOp,
}

/// Exact style dependency and invalidation information for one prepared
/// configuration transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleChangeSummary {
    before_revision: StyleSheetRevision,
    after_revision: StyleSheetRevision,
    changed_properties: BTreeSet<StyleProperty>,
    invalidation_effects: BTreeSet<StyleInvalidationEffect>,
    affected_block_styles: Vec<StyleId>,
    affected_character_styles: Vec<StyleId>,
    affected_ranges: Vec<Range<usize>>,
}

impl StyleChangeSummary {
    pub fn before_revision(&self) -> StyleSheetRevision {
        self.before_revision
    }

    pub fn after_revision(&self) -> StyleSheetRevision {
        self.after_revision
    }

    pub fn changed_properties(&self) -> &BTreeSet<StyleProperty> {
        &self.changed_properties
    }

    pub fn invalidation_effects(&self) -> &BTreeSet<StyleInvalidationEffect> {
        &self.invalidation_effects
    }

    pub fn affected_block_styles(&self) -> &[StyleId] {
        &self.affected_block_styles
    }

    pub fn affected_character_styles(&self) -> &[StyleId] {
        &self.affected_character_styles
    }

    pub fn affected_ranges(&self) -> &[Range<usize>] {
        &self.affected_ranges
    }
}

/// How much projection and persistent candidate-construction work was
/// performed while preparing a model transaction.
///
/// The scope describes decode/normalize/format-projection work. The associated
/// statistics also expose persistent text, projection-index, and source-line
/// maintenance so a regional transaction cannot hide a document-wide rebuild.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProjectionWorkScope {
    #[default]
    None,
    RegionalHardLines,
    FullDocument,
}

/// Auditable transformation work performed for one prepared candidate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectionWorkStatistics {
    scope: ProjectionWorkScope,
    source_decode_passes: usize,
    decoded_source_bytes: usize,
    projected_formatted_bytes: usize,
    projected_hard_lines: usize,
    source_hard_line_records_rebuilt: usize,
    formatted_text_nodes_visited: usize,
    formatted_text_nodes_copied: usize,
    formatted_text_leaves_copied: usize,
    range_index_nodes_visited: usize,
    range_index_nodes_copied: usize,
    range_index_leaves_copied: usize,
    range_index_records_copied: usize,
    source_hard_line_nodes_visited: usize,
    source_hard_line_nodes_copied: usize,
    source_hard_line_leaves_copied: usize,
    source_hard_line_records_copied: usize,
    full_text_bytes_materialized: usize,
}

impl ProjectionWorkStatistics {
    const fn none() -> Self {
        Self {
            scope: ProjectionWorkScope::None,
            source_decode_passes: 0,
            decoded_source_bytes: 0,
            projected_formatted_bytes: 0,
            projected_hard_lines: 0,
            source_hard_line_records_rebuilt: 0,
            formatted_text_nodes_visited: 0,
            formatted_text_nodes_copied: 0,
            formatted_text_leaves_copied: 0,
            range_index_nodes_visited: 0,
            range_index_nodes_copied: 0,
            range_index_leaves_copied: 0,
            range_index_records_copied: 0,
            source_hard_line_nodes_visited: 0,
            source_hard_line_nodes_copied: 0,
            source_hard_line_leaves_copied: 0,
            source_hard_line_records_copied: 0,
            full_text_bytes_materialized: 0,
        }
    }

    fn regional(
        decoded_source_bytes: usize,
        projected_formatted_bytes: usize,
        projected_hard_lines: usize,
        text: FormattedTextSpliceStats,
        ranges: super::projection::ProjectionSpliceStatistics,
        source_lines: SourceHardLineSpliceStats,
    ) -> Self {
        Self {
            scope: ProjectionWorkScope::RegionalHardLines,
            source_decode_passes: 1,
            decoded_source_bytes,
            projected_formatted_bytes,
            projected_hard_lines,
            source_hard_line_records_rebuilt: projected_hard_lines,
            formatted_text_nodes_visited: text.nodes_visited,
            formatted_text_nodes_copied: text.nodes_copied,
            formatted_text_leaves_copied: text.leaves_copied,
            range_index_nodes_visited: ranges.range_index_nodes_visited(),
            range_index_nodes_copied: ranges.range_index_nodes_copied(),
            range_index_leaves_copied: ranges.range_index_leaves_copied(),
            range_index_records_copied: ranges.range_index_records_copied(),
            source_hard_line_nodes_visited: source_lines.nodes_visited,
            source_hard_line_nodes_copied: source_lines.nodes_copied,
            source_hard_line_leaves_copied: source_lines.leaves_copied,
            source_hard_line_records_copied: source_lines.records_copied,
            full_text_bytes_materialized: 0,
        }
    }

    fn full(candidate: &DocumentState) -> Self {
        Self {
            scope: ProjectionWorkScope::FullDocument,
            source_decode_passes: 1,
            // Detection and state construction consume one shared decoded
            // value, so a complete candidate performs one decoder pass.
            decoded_source_bytes: candidate.source.len(),
            projected_formatted_bytes: candidate.projection.text().len(),
            projected_hard_lines: candidate.projection.hard_line_count(),
            source_hard_line_records_rebuilt: candidate.projection.hard_line_count(),
            formatted_text_nodes_visited: 0,
            formatted_text_nodes_copied: 0,
            formatted_text_leaves_copied: 0,
            range_index_nodes_visited: 0,
            range_index_nodes_copied: 0,
            range_index_leaves_copied: 0,
            range_index_records_copied: 0,
            source_hard_line_nodes_visited: 0,
            source_hard_line_nodes_copied: 0,
            source_hard_line_leaves_copied: 0,
            source_hard_line_records_copied: candidate.projection.hard_line_count(),
            full_text_bytes_materialized: candidate.projection.text().len(),
        }
    }

    pub fn scope(self) -> ProjectionWorkScope {
        self.scope
    }

    /// Complete or regional decoder invocations used to build this candidate.
    pub fn source_decode_passes(self) -> usize {
        self.source_decode_passes
    }

    pub fn decoded_source_bytes(self) -> usize {
        self.decoded_source_bytes
    }

    pub fn projected_formatted_bytes(self) -> usize {
        self.projected_formatted_bytes
    }

    pub fn projected_hard_lines(self) -> usize {
        self.projected_hard_lines
    }

    /// Number of source hard-line extent records rebuilt. Untouched suffix
    /// offsets move through persistent prefix-sum aggregates and are not
    /// included here because they are neither visited nor copied.
    pub fn source_hard_line_records_rebuilt(self) -> usize {
        self.source_hard_line_records_rebuilt
    }

    pub fn persistent_nodes_visited(self) -> usize {
        self.formatted_text_nodes_visited
            .saturating_add(self.range_index_nodes_visited)
            .saturating_add(self.source_hard_line_nodes_visited)
    }

    pub fn persistent_nodes_copied(self) -> usize {
        self.formatted_text_nodes_copied
            .saturating_add(self.range_index_nodes_copied)
            .saturating_add(self.source_hard_line_nodes_copied)
    }

    pub fn persistent_records_copied(self) -> usize {
        self.range_index_records_copied
            .saturating_add(self.source_hard_line_records_copied)
    }

    pub fn formatted_text_leaves_copied(self) -> usize {
        self.formatted_text_leaves_copied
    }

    pub fn persistent_leaves_copied(self) -> usize {
        self.formatted_text_leaves_copied
            .saturating_add(self.range_index_leaves_copied)
            .saturating_add(self.source_hard_line_leaves_copied)
    }

    /// Complete formatted bytes flattened solely to verify/build a candidate.
    /// Regional transactions keep this at zero.
    pub fn full_text_bytes_materialized(self) -> usize {
        self.full_text_bytes_materialized
    }
}

/// One exact patch in the pre-transaction primary source part.
///
/// Patches in a summary are sorted, non-overlapping, and all use coordinates
/// from the same before snapshot. Applying them in reverse order reproduces the
/// candidate source bytes. The replacement is retained because lengths alone
/// are insufficient for auditing a lossless source transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePatch {
    part: SourcePartId,
    range: Range<usize>,
    replacement: Vec<u8>,
}

impl SourcePatch {
    fn primary(range: Range<usize>, replacement: Vec<u8>) -> Self {
        Self {
            part: SourcePartId::PRIMARY,
            range,
            replacement,
        }
    }

    pub fn part(&self) -> SourcePartId {
        self.part
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn replacement(&self) -> &[u8] {
        &self.replacement
    }
}

/// Exact invalidation/audit information produced during preparation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelChangeSummary {
    kind: ModelChangeKind,
    source_patches: Vec<SourcePatch>,
    formatted_splices: Vec<Splice>,
    projection_work: ProjectionWorkStatistics,
    style_change: Option<StyleChangeSummary>,
}

impl ModelChangeSummary {
    pub fn kind(&self) -> ModelChangeKind {
        self.kind
    }

    pub fn source_patches(&self) -> &[SourcePatch] {
        &self.source_patches
    }

    /// Exact, discontiguous formatted changes in before-snapshot coordinates.
    ///
    /// These are deliberately not reduced to a common-prefix/common-suffix
    /// replacement: unchanged content between edits must retain its identity.
    pub fn formatted_splices(&self) -> &[Splice] {
        &self.formatted_splices
    }

    pub fn projection_work(&self) -> ProjectionWorkStatistics {
        self.projection_work
    }

    pub fn style_change(&self) -> Option<&StyleChangeSummary> {
        self.style_change.as_ref()
    }
}

fn history_semantic_kind(kind: ModelChangeKind) -> Option<HistorySemanticChangeKind> {
    match kind {
        ModelChangeKind::TextEdits => Some(HistorySemanticChangeKind::Text),
        ModelChangeKind::SemanticStyle => Some(HistorySemanticChangeKind::Style),
        ModelChangeKind::ConfigurationStyle => Some(HistorySemanticChangeKind::Style),
        ModelChangeKind::FileFormat => Some(HistorySemanticChangeKind::FileFormat),
        ModelChangeKind::HardLineTransfer => Some(HistorySemanticChangeKind::HardLineTransfer),
        ModelChangeKind::HardLineSourceRestoration => {
            Some(HistorySemanticChangeKind::HardLineSourceRestoration)
        }
        ModelChangeKind::SourceMetadata => Some(HistorySemanticChangeKind::SourceMetadata),
        ModelChangeKind::HistoryNavigation | ModelChangeKind::NoOp => None,
    }
}

fn history_transaction_summary(
    before_revision: Revision,
    after_revision: Revision,
    summary: &ModelChangeSummary,
) -> Option<HistoryTransactionSummary> {
    let semantic = history_semantic_kind(summary.kind)?;
    let source_patches = summary
        .source_patches
        .iter()
        .map(|patch| HistorySourcePatch::new(patch.part(), patch.range(), patch.replacement()))
        .collect();
    Some(HistoryTransactionSummary::new(
        before_revision,
        after_revision,
        semantic,
        source_patches,
        summary.formatted_splices.clone(),
    ))
}

enum PreparedPublication {
    NoOp,
    State(DocumentState),
    History { target: HistoryNodeId },
}

struct CandidateVerification<'a> {
    expected_text: String,
    expected_hard_breaks: Option<&'a [usize]>,
    mismatch_error: DocumentError,
}

const MAX_LINE_LOCAL_PROJECTION_HARD_LINES: usize = 64;

struct LineLocalProjectionRegion {
    hard_lines: Range<usize>,
    old_formatted: Range<usize>,
    old_source: Range<usize>,
}

struct TextEditCandidate {
    state: DocumentState,
    work: ProjectionWorkStatistics,
    block_ids_already_reconciled: bool,
}

/// Opaque, verified candidate returned by [`Document::prepare_model_request`].
///
/// A prepared transaction is intentionally not cloneable. Consuming it at
/// commit prevents accidental double publication.
pub struct PreparedModelTransaction {
    document: DocumentId,
    before_revision: Revision,
    after_revision: Revision,
    expected_next_revision: u64,
    expected_next_projected_block_id: u64,
    next_projected_block_id_after: u64,
    expected_history: HistoryLocation,
    expected_edit_group_depth: usize,
    expected_edit_group_generation: u64,
    summary: ModelChangeSummary,
    text_position_map: PositionMap,
    planned_history_navigation: Option<HistoryNavigation>,
    publication: PreparedPublication,
}

impl fmt::Debug for PreparedModelTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedModelTransaction")
            .field("document", &self.document)
            .field("before_revision", &self.before_revision)
            .field("after_revision", &self.after_revision)
            .field(
                "expected_next_projected_block_id",
                &self.expected_next_projected_block_id,
            )
            .field(
                "next_projected_block_id_after",
                &self.next_projected_block_id_after,
            )
            .field("summary", &self.summary)
            .field("text_position_map", &self.text_position_map)
            .field(
                "planned_history_navigation",
                &self.planned_history_navigation,
            )
            .field("is_no_op", &self.is_no_op())
            .finish_non_exhaustive()
    }
}

impl PreparedModelTransaction {
    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn before_revision(&self) -> Revision {
        self.before_revision
    }

    pub fn after_revision(&self) -> Revision {
        self.after_revision
    }

    pub fn summary(&self) -> &ModelChangeSummary {
        &self.summary
    }

    /// A precise map from the formatted before snapshot to the candidate.
    pub fn text_position_map(&self) -> &PositionMap {
        &self.text_position_map
    }

    pub fn planned_history_navigation(&self) -> Option<HistoryNavigation> {
        self.planned_history_navigation
    }

    pub fn is_no_op(&self) -> bool {
        matches!(self.publication, PreparedPublication::NoOp)
    }
}

/// Result published after a prepared candidate is installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedModelTransaction {
    before_revision: Revision,
    after_revision: Revision,
    summary: ModelChangeSummary,
    text_position_map: PositionMap,
    history_navigation: Option<HistoryNavigation>,
}

impl CommittedModelTransaction {
    pub fn before_revision(&self) -> Revision {
        self.before_revision
    }

    pub fn after_revision(&self) -> Revision {
        self.after_revision
    }

    pub fn summary(&self) -> &ModelChangeSummary {
        &self.summary
    }

    pub fn text_position_map(&self) -> &PositionMap {
        &self.text_position_map
    }

    pub fn history_navigation(&self) -> Option<HistoryNavigation> {
        self.history_navigation
    }
}

/// Structured failure from preparation or publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelTransactionError {
    Document(DocumentError),
    Position(PositionError),
    History(HistoryError),
    Style(StyleTransactionError),
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    /// The current snapshot returned to the same revision through history, or
    /// non-source transaction state changed after preparation.
    StaleDocumentState,
    RevisionExhausted,
}

impl fmt::Display for ModelTransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(formatter),
            Self::Position(error) => error.fmt(formatter),
            Self::History(error) => error.fmt(formatter),
            Self::Style(error) => write!(formatter, "invalid style operation: {error:?}"),
            Self::WrongDocument { expected, actual } => write!(
                formatter,
                "prepared transaction belongs to document {}, expected {}",
                actual.0, expected.0
            ),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "prepared transaction expects revision {}, current revision is {}",
                expected.0, actual.0
            ),
            Self::StaleDocumentState => {
                formatter.write_str("document transaction state changed after preparation")
            }
            Self::RevisionExhausted => {
                formatter.write_str("document revision identities exhausted")
            }
        }
    }
}

impl std::error::Error for ModelTransactionError {}

impl From<DocumentError> for ModelTransactionError {
    fn from(error: DocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<PositionError> for ModelTransactionError {
    fn from(error: PositionError) -> Self {
        Self::Position(error)
    }
}

impl From<HistoryError> for ModelTransactionError {
    fn from(error: HistoryError) -> Self {
        Self::History(error)
    }
}

impl From<StyleError> for ModelTransactionError {
    fn from(error: StyleError) -> Self {
        Self::Style(StyleTransactionError::Definition(error))
    }
}

impl Document {
    /// Prepare and verify a revision-bound model operation without mutation.
    pub fn prepare_model_request(
        &self,
        request: ModelRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_request_target(&request)?;
        match request {
            ModelRequest::ApplyTextEdits { edits, .. } => self.prepare_text_edits(edits),
            ModelRequest::SetSemanticStyle {
                range,
                style,
                enabled,
                ..
            } => self.prepare_semantic_style(range, style, enabled),
            ModelRequest::SetFileFormat { target, .. } => self.prepare_file_format(target),
            ModelRequest::TransferHardLines {
                operation,
                source_lines,
                destination,
                ..
            } => self.prepare_hard_line_transfer(operation, source_lines, destination),
            ModelRequest::RestoreHardLineSource {
                target_line, image, ..
            } => self.prepare_hard_line_source_restoration(target_line, image),
            ModelRequest::NavigateHistory { navigation, .. } => {
                self.prepare_history_navigation(navigation)
            }
        }
    }

    /// Prepare a typed style operation against one exact formatted snapshot.
    /// Persisted-content operations are capability checked through every
    /// transformation stage. Generated configuration operations use their
    /// distinct authority and never modify source bytes.
    pub fn prepare_style_request(
        &self,
        request: StyleModelRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if request.document != self.id {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id,
                actual: request.document,
            });
        }
        if request.revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: request.revision,
                actual: self.revision(),
            });
        }
        match request.intent {
            StyleModelIntent::Persisted(intent) => self.prepare_persisted_style_intent(intent),
            StyleModelIntent::Configuration(intent) => {
                self.prepare_configuration_style_intent(intent)
            }
        }
    }

    pub fn apply_style_request(
        &mut self,
        request: StyleModelRequest,
    ) -> Result<CommittedModelTransaction, ModelTransactionError> {
        let prepared = self.prepare_style_request(request)?;
        self.commit_model_transaction(prepared)
    }

    /// Install an already verified candidate after checking all captured model
    /// preconditions. No decoding, projection, adapter, provider, or callback
    /// work occurs after the first mutation.
    pub fn commit_model_transaction(
        &mut self,
        prepared: PreparedModelTransaction,
    ) -> Result<CommittedModelTransaction, ModelTransactionError> {
        if prepared.document != self.id {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id,
                actual: prepared.document,
            });
        }
        if prepared.before_revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: prepared.before_revision,
                actual: self.revision(),
            });
        }
        if prepared.expected_next_revision != self.next_revision
            || prepared.expected_next_projected_block_id != self.next_projected_block_id
            || prepared.expected_history != self.history.status().current
            || prepared.expected_edit_group_depth != self.edit_group_depth
            || prepared.expected_edit_group_generation != self.edit_group_generation
        {
            return Err(ModelTransactionError::StaleDocumentState);
        }

        // Validate every remaining fallible condition before publishing.
        if let PreparedPublication::History { target } = &prepared.publication {
            if self.history.state_at_node(*target).is_none() {
                return Err(ModelTransactionError::History(HistoryError::NodeNotFound(
                    *target,
                )));
            }
        }
        let incremented_revision = match &prepared.publication {
            PreparedPublication::State(_) => Some(
                self.next_revision
                    .checked_add(1)
                    .ok_or(ModelTransactionError::RevisionExhausted)?,
            ),
            PreparedPublication::NoOp | PreparedPublication::History { .. } => None,
        };
        match &prepared.publication {
            PreparedPublication::State(_)
                if prepared.next_projected_block_id_after
                    < prepared.expected_next_projected_block_id =>
            {
                return Err(ModelTransactionError::StaleDocumentState);
            }
            PreparedPublication::NoOp | PreparedPublication::History { .. }
                if prepared.next_projected_block_id_after
                    != prepared.expected_next_projected_block_id =>
            {
                return Err(ModelTransactionError::StaleDocumentState);
            }
            PreparedPublication::State(_)
            | PreparedPublication::NoOp
            | PreparedPublication::History { .. } => {}
        }
        let reverse_position_map = match &prepared.publication {
            PreparedPublication::State(_) => Some(prepared.text_position_map.inverted()?),
            PreparedPublication::NoOp | PreparedPublication::History { .. } => None,
        };
        // A compatibility command may issue several independently prepared
        // model operations during one input event (for example a macro). Fold
        // this transition into the coordinator's short-lived capture before
        // the first authoritative mutation so publication remains infallible.
        let next_position_capture = self.composed_position_capture(&prepared.text_position_map)?;
        let history_record = match &prepared.publication {
            PreparedPublication::State(_) => Some((
                history_transaction_summary(
                    prepared.before_revision,
                    prepared.after_revision,
                    &prepared.summary,
                )
                .expect("a state publication has a source-changing semantic kind"),
                self.default_history_restoration(&prepared.summary, &prepared.text_position_map)?,
            )),
            PreparedPublication::NoOp | PreparedPublication::History { .. } => None,
        };

        let history_navigation = match prepared.publication {
            PreparedPublication::NoOp => None,
            PreparedPublication::State(candidate) => {
                debug_assert_eq!(candidate.revision, prepared.after_revision);
                let (transaction, restoration) =
                    history_record.expect("a state publication prepared history metadata");
                self.history.commit_with_maps(
                    candidate,
                    self.edit_group_depth > 0,
                    prepared.text_position_map.clone(),
                    reverse_position_map.expect("state publications have a reverse map"),
                    transaction,
                    restoration,
                )?;
                self.next_revision =
                    incremented_revision.expect("state commit increments revision");
                self.next_projected_block_id = prepared.next_projected_block_id_after;
                None
            }
            PreparedPublication::History { target } => {
                self.edit_group_depth = 0;
                self.history.end_group();
                Some(
                    self.history
                        .select_node(target)
                        .expect("history target was validated before publication"),
                )
            }
        };
        self.position_map_capture = next_position_capture;

        debug_assert_eq!(self.revision(), prepared.after_revision);
        Ok(CommittedModelTransaction {
            before_revision: prepared.before_revision,
            after_revision: prepared.after_revision,
            summary: prepared.summary,
            text_position_map: prepared.text_position_map,
            history_navigation,
        })
    }

    /// Convenience for non-coordinator callers that do not need a publication
    /// gap. Coordinators should retain the explicit prepare/commit boundary.
    pub fn apply_model_request(
        &mut self,
        request: ModelRequest,
    ) -> Result<CommittedModelTransaction, ModelTransactionError> {
        let prepared = self.prepare_model_request(request)?;
        self.commit_model_transaction(prepared)
    }

    /// Prepare a revision-bound set of structured formatted-text replacements.
    /// Preparation verifies both flat UTF-8 and semantic hard-break identity
    /// before returning a publishable transaction.
    pub fn prepare_formatted_payload_request(
        &self,
        request: FormattedPayloadEditRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if request.document != self.id {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id,
                actual: request.document,
            });
        }
        if request.revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: request.revision,
                actual: self.revision(),
            });
        }
        for edit in &request.edits {
            if edit.payload.document() != self.id {
                return Err(ModelTransactionError::WrongDocument {
                    expected: self.id,
                    actual: edit.payload.document(),
                });
            }
            if edit.payload.revision() != self.revision() {
                return Err(ModelTransactionError::StaleRevision {
                    expected: edit.payload.revision(),
                    actual: self.revision(),
                });
            }
        }
        self.prepare_formatted_payload_edits(request.edits)
    }

    pub fn apply_formatted_payload_request(
        &mut self,
        request: FormattedPayloadEditRequest,
    ) -> Result<CommittedModelTransaction, ModelTransactionError> {
        let prepared = self.prepare_formatted_payload_request(request)?;
        self.commit_model_transaction(prepared)
    }

    pub(super) fn execute_compat_request(
        &mut self,
        request: ModelRequest,
    ) -> Result<(), DocumentError> {
        self.apply_model_request(request)
            .map(|_| ())
            .map_err(compat_document_error)
    }

    pub(super) fn execute_compat_formatted_payload_request(
        &mut self,
        request: FormattedPayloadEditRequest,
    ) -> Result<(), DocumentError> {
        self.apply_formatted_payload_request(request)
            .map(|_| ())
            .map_err(compat_document_error)
    }

    /// Prepare a source-metadata-only candidate used by explicit artifact
    /// operations such as BOM policy changes. The formatted projection must
    /// remain semantically and structurally identical.
    pub(super) fn prepare_source_metadata_candidate(
        &self,
        source: super::source::SourceSnapshot,
        file_format: FileFormat,
        expected_text: String,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if source.bytes() == self.state().source.bytes() && file_format == self.state().file_format
        {
            return Ok(self.no_op_prepared());
        }
        let after_revision = Revision(self.next_revision);
        let bytes = source.bytes();
        let decoded = self.state().encoding.decode(&bytes)?;
        let evidence = detect(&decoded.text).1;
        let mut candidate = build_state_from_decoded(
            source,
            decoded,
            self.state().format,
            file_format,
            self.state().file_format_origin,
            evidence,
            after_revision,
        )?;
        if candidate.projection.text() != expected_text
            || !candidate
                .projection
                .has_same_hard_line_structure(self.projection())
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        candidate
            .projection
            .install_unchanged_text_storage(self.projection())
            .map_err(DocumentError::FormattedTextStorage)?;
        candidate
            .projection
            .install_unchanged_block_ids(self.projection())
            .map_err(super::block_identity_document_error)?;
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SourceMetadata,
                source_patches: byte_difference(
                    &self.state().source.bytes(),
                    &candidate.source.bytes(),
                ),
                formatted_splices: Vec::new(),
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    pub(super) fn execute_compat_source_metadata_candidate(
        &mut self,
        source: super::source::SourceSnapshot,
        file_format: FileFormat,
        expected_text: String,
    ) -> Result<(), DocumentError> {
        let prepared = self
            .prepare_source_metadata_candidate(source, file_format, expected_text)
            .map_err(compat_document_error)?;
        self.commit_model_transaction(prepared)
            .map(|_| ())
            .map_err(compat_document_error)
    }

    fn validate_request_target(&self, request: &ModelRequest) -> Result<(), ModelTransactionError> {
        if request.document() != self.id {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id,
                actual: request.document(),
            });
        }
        if request.revision() != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: request.revision(),
                actual: self.revision(),
            });
        }
        Ok(())
    }

    fn prepared(
        &self,
        after_revision: Revision,
        summary: ModelChangeSummary,
        text_position_map: PositionMap,
        planned_history_navigation: Option<HistoryNavigation>,
        next_projected_block_id_after: u64,
        publication: PreparedPublication,
    ) -> PreparedModelTransaction {
        PreparedModelTransaction {
            document: self.id,
            before_revision: self.revision(),
            after_revision,
            expected_next_revision: self.next_revision,
            expected_next_projected_block_id: self.next_projected_block_id,
            next_projected_block_id_after,
            expected_history: self.history.status().current,
            expected_edit_group_depth: self.edit_group_depth,
            expected_edit_group_generation: self.edit_group_generation,
            summary,
            text_position_map,
            planned_history_navigation,
            publication,
        }
    }

    /// Seed a complete stable-anchor restoration record for model-only users.
    /// The serial command coordinator replaces this deterministic fallback
    /// with the invoking command's authentic cursor and mark snapshots before
    /// the turn is published.
    fn default_history_restoration(
        &self,
        summary: &ModelChangeSummary,
        map: &PositionMap,
    ) -> Result<HistoryRestoration, ModelTransactionError> {
        let offset = summary
            .formatted_splices
            .first()
            .map(|splice| splice.old_range().start)
            .unwrap_or(0);
        let before = self.text_anchor(
            self.text_point(offset)?,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )?;
        let after = match map.map_text_anchor(before)? {
            MappingOutcome::Exact(anchor)
            | MappingOutcome::Moved(anchor)
            | MappingOutcome::CollapsedByDeletion(anchor)
            | MappingOutcome::RecoveredFromProvenance(anchor) => anchor,
            MappingOutcome::Ambiguous(_) | MappingOutcome::Unresolvable(_) => {
                return Err(ModelTransactionError::StaleDocumentState);
            }
        };
        Ok(HistoryRestoration::new(
            HistoryRestorationSnapshot::new(before, BTreeMap::new()),
            HistoryRestorationSnapshot::new(after, BTreeMap::new()),
        ))
    }

    fn no_op_prepared(&self) -> PreparedModelTransaction {
        self.prepared(
            self.revision(),
            ModelChangeSummary {
                kind: ModelChangeKind::NoOp,
                source_patches: Vec::new(),
                formatted_splices: Vec::new(),
                projection_work: ProjectionWorkStatistics::none(),
                style_change: None,
            },
            PositionMap::identity(
                self.id,
                super::PositionDomain::FormattedText,
                self.revision(),
                self.projection().text_tree().byte_len(),
            ),
            None,
            self.next_projected_block_id,
            PreparedPublication::NoOp,
        )
    }

    fn prepare_persisted_style_intent(
        &self,
        intent: PersistedStyleIntent,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if let PersistedStyleIntent::EditStyleDefinition { origin, edit } = &intent {
            match origin {
                StyleDefinitionOrigin::GeneratedConfiguration => {
                    return Err(ModelTransactionError::Style(
                        StyleTransactionError::ConfigurationIntentRequired(edit.style_id().clone()),
                    ));
                }
                StyleDefinitionOrigin::SyntheticReadOnly => {
                    return Err(ModelTransactionError::Style(
                        StyleTransactionError::DefinitionReadOnly(edit.style_id().clone()),
                    ));
                }
                StyleDefinitionOrigin::SourceBacked => {}
            }
        }

        let whole_document = self.whole_text_range()?;
        let (range, pipeline_intent) = match &intent {
            PersistedStyleIntent::AssignBlockStyle { target, style } => (
                style_block_target_range(*target, whole_document),
                PipelineEditIntent::AssignBlockStyle {
                    style: style.clone(),
                },
            ),
            PersistedStyleIntent::AssignCharacterStyle { range, style } => (
                *range,
                PipelineEditIntent::AssignCharacterStyle {
                    style: style.clone(),
                },
            ),
            PersistedStyleIntent::SetDirectCharacterProperties { range, properties } => {
                let Some(property) = properties.declared_properties().into_iter().next() else {
                    self.validate_style_text_range(*range)?;
                    return Ok(self.no_op_prepared());
                };
                (*range, PipelineEditIntent::SetDirectProperty { property })
            }
            PersistedStyleIntent::ClearDirectCharacterProperties { range, properties } => {
                let Some(property) = properties.iter().next().copied() else {
                    self.validate_style_text_range(*range)?;
                    return Ok(self.no_op_prepared());
                };
                (*range, PipelineEditIntent::ClearDirectProperty { property })
            }
            PersistedStyleIntent::SetDirectBlockProperties { target, properties } => {
                let Some(property) = properties.declared_properties().into_iter().next() else {
                    self.validate_style_text_range(style_block_target_range(
                        *target,
                        whole_document,
                    ))?;
                    return Ok(self.no_op_prepared());
                };
                (
                    style_block_target_range(*target, whole_document),
                    PipelineEditIntent::SetDirectProperty { property },
                )
            }
            PersistedStyleIntent::ClearDirectBlockProperties { target, properties } => {
                let Some(property) = properties.iter().next().copied() else {
                    self.validate_style_text_range(style_block_target_range(
                        *target,
                        whole_document,
                    ))?;
                    return Ok(self.no_op_prepared());
                };
                (
                    style_block_target_range(*target, whole_document),
                    PipelineEditIntent::ClearDirectProperty { property },
                )
            }
            PersistedStyleIntent::EditStyleDefinition { edit, .. } => {
                let pipeline_intent = if edit.is_block() {
                    PipelineEditIntent::EditBlockStyleDefinition {
                        style: edit.style_id().clone(),
                    }
                } else {
                    PipelineEditIntent::EditCharacterStyleDefinition {
                        style: edit.style_id().clone(),
                    }
                };
                (whole_document, pipeline_intent)
            }
        };
        let report = self
            .transformation_pipeline_snapshot()
            .capabilities(range, &pipeline_intent)?;
        match report.decision {
            PipelineCapabilityDecision::Supported => Err(ModelTransactionError::Style(
                StyleTransactionError::TranslationUnavailable,
            )),
            PipelineCapabilityDecision::Unsupported { reason, .. } => Err(
                ModelTransactionError::Style(StyleTransactionError::Unsupported {
                    format: self.format(),
                    reason,
                }),
            ),
            PipelineCapabilityDecision::NeedsPolicy { request, .. } => Err(
                ModelTransactionError::Style(StyleTransactionError::NeedsPolicy(request)),
            ),
        }
    }

    fn whole_text_range(&self) -> Result<TextRange, ModelTransactionError> {
        Ok(TextRange::new(
            self.text_point(0)?,
            self.text_point(self.projection().text_tree().byte_len())?,
        )?)
    }

    fn validate_style_text_range(&self, range: TextRange) -> Result<(), ModelTransactionError> {
        for point in [range.start(), range.end()] {
            if point.document() != self.id {
                return Err(PositionError::WrongDocument {
                    expected: self.id,
                    actual: point.document(),
                }
                .into());
            }
            if point.revision() != self.revision() {
                return Err(PositionError::WrongSnapshot {
                    expected: self.revision(),
                    actual: point.revision(),
                }
                .into());
            }
            self.text_point(point.offset())?;
        }
        Ok(())
    }

    fn prepare_configuration_style_intent(
        &self,
        intent: ConfigurationStyleIntent,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let after_revision = Revision(self.next_revision);
        let style_sheet_revision = after_revision
            .0
            .checked_add(1)
            .map(StyleSheetRevision)
            .ok_or(StyleError::StyleSheetRevisionExhausted)?;
        let before_sheet = self.projection().style_sheet();
        let before_assignment = self.projection().document_style();
        let mut style_sheet = before_sheet.clone();
        let mut document_style = before_assignment.clone();

        let changed = match &intent {
            ConfigurationStyleIntent::EditDefinition(edit) => {
                let has_assignment = if edit.is_block() {
                    self.projection()
                        .has_block_style_assignment(edit.style_id())
                } else {
                    self.projection()
                        .has_character_style_assignment(edit.style_id())
                };
                style_sheet.apply_configuration_edit(edit, style_sheet_revision, has_assignment)?
            }
            ConfigurationStyleIntent::AssignDocumentStyle(style) => {
                if document_style.style == *style {
                    false
                } else {
                    document_style.style.clone_from(style);
                    style_sheet.set_configuration_revision(style_sheet_revision);
                    true
                }
            }
            ConfigurationStyleIntent::SetDocumentCanvas(properties) => {
                let before = document_style.direct_canvas.clone();
                merge_block_properties(&mut document_style.direct_canvas, properties);
                let changed = before != document_style.direct_canvas;
                if changed {
                    style_sheet.set_configuration_revision(style_sheet_revision);
                }
                changed
            }
            ConfigurationStyleIntent::ClearDocumentCanvas(properties) => {
                let before = document_style.direct_canvas.clone();
                clear_block_properties(
                    &mut document_style.direct_canvas,
                    properties,
                    StylePropertyTarget::DocumentCanvas,
                )?;
                let changed = before != document_style.direct_canvas;
                if changed {
                    style_sheet.set_configuration_revision(style_sheet_revision);
                }
                changed
            }
            ConfigurationStyleIntent::SetDocumentDefaultCharacter(properties) => {
                let before = document_style.direct_default_character.clone();
                merge_character_properties(
                    &mut document_style.direct_default_character,
                    properties,
                );
                let changed = before != document_style.direct_default_character;
                if changed {
                    style_sheet.set_configuration_revision(style_sheet_revision);
                }
                changed
            }
            ConfigurationStyleIntent::ClearDocumentDefaultCharacter(properties) => {
                let before = document_style.direct_default_character.clone();
                clear_character_properties(
                    &mut document_style.direct_default_character,
                    properties,
                    StylePropertyTarget::Character,
                )?;
                let changed = before != document_style.direct_default_character;
                if changed {
                    style_sheet.set_configuration_revision(style_sheet_revision);
                }
                changed
            }
        };
        if !changed {
            return Ok(self.no_op_prepared());
        }

        validate_projection_style_configuration(self.projection(), &style_sheet, &document_style)?;
        let style_change = configuration_style_change_summary(
            self.projection(),
            before_sheet,
            &style_sheet,
            before_assignment,
            &document_style,
            &intent,
        )?;

        let mut candidate = self.state().clone();
        candidate.revision = after_revision;
        candidate.projection.install_configuration_styles(
            after_revision,
            style_sheet,
            document_style,
        );
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::ConfigurationStyle,
                source_patches: Vec::new(),
                formatted_splices: Vec::new(),
                projection_work: ProjectionWorkStatistics::none(),
                style_change: Some(style_change),
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_text_edits(
        &self,
        mut edits: Vec<TextEdit>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }
        for edit in &edits {
            self.validate_range(&edit.range)?;
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        for pair in edits.windows(2) {
            let first = &pair[0].range;
            let second = &pair[1].range;
            if first.end > second.start
                || (first.is_empty() && second.is_empty() && first.start == second.start)
            {
                return Err(DocumentError::OverlappingEdits.into());
            }
        }

        let mut retained = Vec::with_capacity(edits.len());
        let mut formatted_text_changed = false;
        for edit in edits {
            let current = self
                .projection()
                .text_tree()
                .slice(edit.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let changes_text = current != edit.replacement;
            if changes_text
                || self
                    .projection()
                    .has_decoding_diagnostic_overlapping(&edit.range)
            {
                formatted_text_changed |= changes_text;
                retained.push(edit);
            }
        }
        edits = retained;
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }

        let mut source_patches = Vec::with_capacity(edits.len());
        for edit in &edits {
            let source_range = if edit.range.is_empty() {
                let at = self
                    .state()
                    .projection
                    .source_insertion_point(edit.range.start, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                at..at
            } else {
                self.state()
                    .projection
                    .source_range(edit.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?
            };

            self.reject_unsafe_opaque_mapping(&edit.range, &source_range)?;
            if self.state().format == Format::Markdown && !edit.replacement.contains('\n') {
                if let Some(patches) =
                    self.markdown_line_local_text_rewrite_patches(&edit.range, &edit.replacement)?
                {
                    source_patches.extend(patches);
                    continue;
                }
            }

            let in_code = self.state().format == Format::Markdown
                && self
                    .projection()
                    .markdown_replacement_begins_in_code(&edit.range);
            let syntax = match self.state().format {
                Format::PlainText => edit.replacement.clone(),
                Format::Markdown if in_code && !edit.replacement.contains('`') => {
                    edit.replacement.clone()
                }
                Format::Markdown => escape_markdown_insert(&edit.replacement),
            };
            let syntax = spell_logical_breaks(&syntax, self.state().file_format);
            let replacement = self.state().encoding.encode_fragment(&syntax)?;
            source_patches.push(SourcePatch::primary(source_range, replacement));
        }
        validate_source_patches(&mut source_patches)?;

        let persistent_edits = edits
            .iter()
            .map(|edit| (edit.range.clone(), edit.replacement.as_str()))
            .collect::<Vec<_>>();
        let (target_text, text_splice_work) = self
            .projection()
            .text_tree()
            .splice_prevalidated_batch_with_stats(&persistent_edits)
            .map_err(DocumentError::FormattedTextStorage)?;

        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let TextEditCandidate {
            state: mut candidate,
            work: projection_work,
            block_ids_already_reconciled,
        } = self.build_verified_text_edit_candidate(
            source,
            after_revision,
            &target_text,
            text_splice_work,
            &edits,
            &source_patches,
        )?;
        if formatted_text_changed && !block_ids_already_reconciled {
            candidate
                .projection
                .install_persistent_text_edits(self.projection(), &edits)
                .map_err(DocumentError::FormattedTextStorage)?;
        }
        let formatted_splices = edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let map_splices =
            grapheme_closed_snapshot_map_splices(self.projection(), &candidate.projection, &edits)?;
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            map_splices,
        )?;
        let next_projected_block_id = if !formatted_text_changed || block_ids_already_reconciled {
            self.next_projected_block_id
        } else {
            candidate
                .projection
                .install_reconciled_block_ids(
                    self.projection(),
                    &edits,
                    &text_position_map,
                    self.next_projected_block_id,
                )
                .map_err(super::block_identity_document_error)?
        };
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::TextEdits,
                source_patches,
                formatted_splices,
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    /// Translate one line-local Markdown replacement through its discontiguous
    /// visible provenance. Replacement graphemes occupy the original visible
    /// runs from left to right; the last run absorbs any additional graphemes.
    /// This preserves the maximum possible prefix of existing style boundaries
    /// while leaving every hidden delimiter byte outside the patch set.
    fn markdown_line_local_text_rewrite_patches(
        &self,
        formatted_range: &Range<usize>,
        replacement: &str,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        let Some(runs) = self
            .projection()
            .line_local_visible_source_runs(formatted_range.clone())
        else {
            return Ok(None);
        };
        let replacement_graphemes = replacement.graphemes(true).collect::<Vec<_>>();
        let mut replacement_at = 0usize;
        let mut patches = Vec::with_capacity(runs.len());
        let last_run = runs.len() - 1;
        for (index, run) in runs.into_iter().enumerate() {
            let remaining = replacement_graphemes.len() - replacement_at;
            let take = if index == last_run {
                remaining
            } else {
                let original = self
                    .projection()
                    .text_tree()
                    .slice(run.formatted.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                original.graphemes(true).count().min(remaining)
            };
            let segment = replacement_graphemes[replacement_at..replacement_at + take].concat();
            replacement_at += take;
            let in_code = self
                .projection()
                .markdown_replacement_begins_in_code(&run.formatted);
            let syntax = if in_code && !segment.contains('`') {
                segment
            } else {
                escape_markdown_insert(&segment)
            };
            patches.push(SourcePatch::primary(
                run.source,
                self.state().encoding.encode_fragment(&syntax)?,
            ));
        }
        debug_assert_eq!(replacement_at, replacement_graphemes.len());
        Ok(Some(patches))
    }

    fn prepare_formatted_payload_edits(
        &self,
        mut edits: Vec<FormattedPayloadEdit>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }
        for edit in &edits {
            self.validate_range(&edit.range)?;
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        for pair in edits.windows(2) {
            let first = &pair[0].range;
            let second = &pair[1].range;
            if first.end > second.start
                || (first.is_empty() && second.is_empty() && first.start == second.start)
            {
                return Err(DocumentError::OverlappingEdits.into());
            }
        }

        let snapshot = self.hard_line_snapshot();
        edits.retain(|edit| {
            let captured = snapshot
                .capture(edit.range.clone())
                .expect("document range validation also validates payload capture boundaries");
            captured.text() != edit.payload.text()
                || captured.break_offsets() != edit.payload.break_offsets()
                || self
                    .decoding_diagnostics()
                    .iter()
                    .any(|diagnostic| ranges_overlap(&edit.range, &diagnostic.formatted_range))
        });
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }

        let text_edits = edits
            .iter()
            .map(|edit| TextEdit::new(edit.range.clone(), edit.payload.text()))
            .collect::<Vec<_>>();
        let old_text = self.text().to_owned();
        let mut expected_text = old_text.clone();
        for edit in edits.iter().rev() {
            expected_text.replace_range(edit.range.clone(), edit.payload.text());
        }
        let expected_hard_breaks = apply_payload_break_edits(
            &self.projection().hard_break_offsets(),
            &edits,
            old_text.len(),
            expected_text.len(),
        )?;

        let mut source_patches = Vec::with_capacity(edits.len());
        for edit in &edits {
            let source_range = if edit.range.is_empty() {
                let at = self
                    .projection()
                    .source_insertion_point(edit.range.start, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                at..at
            } else {
                self.projection()
                    .source_range(edit.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?
            };

            self.reject_unsafe_opaque_mapping(&edit.range, &source_range)?;
            if self.state().format == Format::Markdown && edit.payload.break_offsets().is_empty() {
                if let Some(patches) =
                    self.markdown_line_local_text_rewrite_patches(&edit.range, edit.payload.text())?
                {
                    source_patches.extend(patches);
                    continue;
                }
            }

            let in_code = self.state().format == Format::Markdown
                && self
                    .projection()
                    .markdown_replacement_begins_in_code(&edit.range);
            let syntax = structured_payload_syntax(
                &edit.payload,
                self.state().format,
                in_code,
                self.state().file_format,
            );
            let replacement = self.state().encoding.encode_fragment(&syntax)?;
            source_patches.push(SourcePatch::primary(source_range, replacement));
        }
        validate_source_patches(&mut source_patches)?;

        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let mut candidate = self.build_verified_candidate(
            source,
            self.state().file_format,
            self.state().file_format_origin,
            after_revision,
            CandidateVerification {
                expected_text: expected_text.clone(),
                expected_hard_breaks: Some(&expected_hard_breaks),
                mismatch_error: DocumentError::FormattedPayloadCannotReproject,
            },
        )?;
        if old_text != expected_text {
            candidate
                .projection
                .install_persistent_text_edits(self.projection(), &text_edits)
                .map_err(DocumentError::FormattedTextStorage)?;
        }
        let formatted_splices = text_edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let map_splices = grapheme_closed_snapshot_map_splices(
            self.projection(),
            &candidate.projection,
            &text_edits,
        )?;
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            map_splices,
        )?;
        let next_projected_block_id = candidate
            .projection
            .install_reconciled_block_ids(
                self.projection(),
                &text_edits,
                &text_position_map,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::TextEdits,
                source_patches,
                formatted_splices,
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_hard_line_transfer(
        &self,
        operation: HardLineTransfer,
        source_lines: Range<usize>,
        destination: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let Some(mut plan) = transfer::plan(self, operation, source_lines, destination)? else {
            return Ok(self.no_op_prepared());
        };
        let mut source_patches = plan
            .source_patches
            .drain(..)
            .map(|patch| SourcePatch::primary(patch.range, patch.replacement))
            .collect::<Vec<_>>();
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let mut candidate = self.build_verified_candidate(
            source,
            self.state().file_format,
            self.state().file_format_origin,
            after_revision,
            CandidateVerification {
                expected_text: plan.expected_text.clone(),
                expected_hard_breaks: Some(&plan.expected_hard_breaks),
                mismatch_error: DocumentError::HardLineTransferProjectionMismatch,
            },
        )?;
        transfer::verify_projection(&candidate.projection, &plan)?;
        candidate
            .projection
            .install_persistent_text_edits(self.projection(), &plan.text_edits)
            .map_err(DocumentError::FormattedTextStorage)?;

        let formatted_splices = plan
            .text_edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let map_splices = grapheme_closed_snapshot_map_splices(
            self.projection(),
            &candidate.projection,
            &plan.text_edits,
        )?;
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            map_splices,
        )?;
        let next_projected_block_id = candidate
            .projection
            .install_transferred_block_ids(
                self.projection(),
                &plan.origins,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::HardLineTransfer,
                source_patches,
                formatted_splices,
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_hard_line_source_restoration(
        &self,
        target_line: usize,
        image: HardLineSourceImage,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if image.document != self.id {
            return Err(DocumentError::WrongDocument.into());
        }
        if image.encoding != self.state().encoding
            || image.format != self.state().format
            || image.file_format != self.state().file_format
        {
            return Err(DocumentError::IncompatibleHardLineSourceImage.into());
        }
        let current_line_count = self.projection().hard_line_count();
        if image.hard_line_count != current_line_count {
            return Err(DocumentError::HardLineSourceImageTopologyChanged {
                captured_line_count: image.hard_line_count,
                current_line_count,
            }
            .into());
        }

        let current = hard_line_source_image_from_state(self.id, self.state(), target_line)?;
        if current.hard_line_id != image.hard_line_id {
            return Err(DocumentError::StaleHardLineSourceImage {
                target_line,
                expected_id: image.hard_line_id,
                actual_id: current.hard_line_id,
            }
            .into());
        }
        if current.terminated != image.terminated {
            return Err(DocumentError::HardLineSourceImageTerminatorShapeChanged {
                captured_terminated: image.terminated,
                current_terminated: current.terminated,
            }
            .into());
        }
        if current.source_bytes == image.source_bytes {
            return Ok(self.no_op_prepared());
        }

        let source_range = self
            .state()
            .source_hard_lines
            .get(target_line)
            .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
        let mut source_patches = hard_line_byte_difference(
            &current.source_bytes,
            &image.source_bytes,
            self.state().encoding,
        );
        for patch in &mut source_patches {
            patch.range = source_range
                .start
                .checked_add(patch.range.start)
                .ok_or(PositionError::ArithmeticOverflow)?
                ..source_range
                    .start
                    .checked_add(patch.range.end)
                    .ok_or(PositionError::ArithmeticOverflow)?;
        }
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;

        let formatted_range = self
            .projection()
            .hard_line_range(target_line)
            .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
        let text_edit = TextEdit::new(formatted_range.clone(), image.formatted_text.clone());
        let formatted_changed = self
            .projection()
            .text_tree()
            .slice(formatted_range.clone())
            .map_err(DocumentError::FormattedTextStorage)?
            != image.formatted_text;
        let target_text = self
            .projection()
            .text_tree()
            .splice_prevalidated_batch(&[(formatted_range.clone(), image.formatted_text.as_str())])
            .map_err(DocumentError::FormattedTextStorage)?;
        let expected_text = target_text.flatten();
        let expected_hard_breaks = hard_breaks_after_line_replacement(
            &self.projection().hard_break_offsets(),
            &formatted_range,
            image.formatted_text.len(),
        )?;
        let after_revision = Revision(self.next_revision);
        let mut candidate = self.build_verified_candidate(
            source,
            self.state().file_format,
            self.state().file_format_origin,
            after_revision,
            CandidateVerification {
                expected_text: expected_text.clone(),
                expected_hard_breaks: Some(&expected_hard_breaks),
                mismatch_error: DocumentError::HardLineSourceImageProjectionMismatch,
            },
        )?;

        let map_splices = if formatted_changed {
            grapheme_closed_snapshot_map_splices(
                self.projection(),
                &candidate.projection,
                std::slice::from_ref(&text_edit),
            )?
        } else {
            Vec::new()
        };
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            map_splices,
        )?;
        let next_projected_block_id = if formatted_changed {
            candidate
                .projection
                .install_persistent_text_edits(self.projection(), std::slice::from_ref(&text_edit))
                .map_err(DocumentError::FormattedTextStorage)?;
            candidate
                .projection
                .install_reconciled_block_ids(
                    self.projection(),
                    std::slice::from_ref(&text_edit),
                    &text_position_map,
                    self.next_projected_block_id,
                )
                .map_err(super::block_identity_document_error)?
        } else {
            self.next_projected_block_id
        };

        let restored = hard_line_source_image_from_state(self.id, &candidate, target_line)?;
        if !restored.same_restorable_content(&image) {
            return Err(DocumentError::HardLineSourceImageProjectionMismatch.into());
        }

        let formatted_splices = if formatted_changed {
            vec![Splice::new(formatted_range, image.formatted_text.len())?]
        } else {
            Vec::new()
        };
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::HardLineSourceRestoration,
                source_patches,
                formatted_splices,
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_semantic_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if range.is_empty() {
            return Ok(self.no_op_prepared());
        }
        if self.state().format != Format::Markdown {
            return Err(DocumentError::UnsupportedFormatting.into());
        }

        let matching = self
            .projection()
            .style_spans()
            .iter()
            .filter(|span| {
                span.application == StyleApplication::Semantic(style)
                    && span.range.start <= range.start
                    && range.end <= span.range.end
            })
            .collect::<Vec<_>>();
        if enabled && !matching.is_empty() {
            return Ok(self.no_op_prepared());
        }
        if !enabled {
            let exact_count = matching.iter().filter(|span| span.range == range).count();
            match exact_count {
                1 => {}
                0 => return Err(DocumentError::UnsupportedFormatting.into()),
                _ => return Err(DocumentError::AmbiguousProjection.into()),
            }
        }
        if enabled
            && self
                .projection()
                .style_spans()
                .iter()
                .any(|span| span.range.start < range.end && range.start < span.range.end)
        {
            return Err(DocumentError::OverlappingFormatting.into());
        }

        let source_range = self
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut source_patches = if enabled {
            let marker_bytes = self
                .state()
                .encoding
                .encode_fragment(preferred_markdown_style_marker(style))?;
            vec![
                SourcePatch::primary(source_range.start..source_range.start, marker_bytes.clone()),
                SourcePatch::primary(source_range.end..source_range.end, marker_bytes.clone()),
            ]
        } else {
            self.markdown_style_removal_patches(&source_range, style)?
        };
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let candidate = self.build_verified_candidate(
            source,
            self.state().file_format,
            self.state().file_format_origin,
            after_revision,
            CandidateVerification {
                expected_text: self.text().to_owned(),
                expected_hard_breaks: None,
                mismatch_error: DocumentError::VerificationFailed,
            },
        )?;
        if !semantic_style_edit_was_exactly_projected(
            self.projection().style_spans(),
            candidate.projection.style_spans(),
            &range,
            style,
            enabled,
        ) {
            return Err(DocumentError::VerificationFailed.into());
        }
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SemanticStyle,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    /// Locate the exact delimiter spelling which produced one projected
    /// Markdown style span. The formatted range is first mapped through its
    /// source provenance; only the small adjacent source slices are then
    /// inspected. This deliberately does not infer syntax from flattened
    /// formatted text, which cannot distinguish `**` from `__` (or `*` from
    /// `_`) and is not the persistence authority.
    fn markdown_style_removal_patches(
        &self,
        source_range: &Range<usize>,
        style: SemanticInlineStyle,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let mut matches = Vec::new();
        for marker in markdown_style_markers(style) {
            let marker_bytes = self.state().encoding.encode_fragment(marker)?;
            let Some(opening_start) = source_range.start.checked_sub(marker_bytes.len()) else {
                continue;
            };
            let Some(closing_end) = source_range.end.checked_add(marker_bytes.len()) else {
                continue;
            };
            if closing_end > self.state().source.len() {
                continue;
            }
            let opening = opening_start..source_range.start;
            let closing = source_range.end..closing_end;
            let opening_matches = self
                .state()
                .source
                .bytes_in(opening.clone())
                .is_some_and(|bytes| bytes == marker_bytes);
            let closing_matches = self
                .state()
                .source
                .bytes_in(closing.clone())
                .is_some_and(|bytes| bytes == marker_bytes);
            if opening_matches && closing_matches {
                matches.push((opening, closing));
            }
        }

        match matches.as_slice() {
            [(opening, closing)] => Ok(vec![
                SourcePatch::primary(opening.clone(), Vec::new()),
                SourcePatch::primary(closing.clone(), Vec::new()),
            ]),
            [] => Err(DocumentError::UnsupportedFormatting.into()),
            _ => Err(DocumentError::AmbiguousProjection.into()),
        }
    }

    fn prepare_file_format(
        &self,
        target: FileFormat,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if target == self.state().file_format {
            return Ok(self.no_op_prepared());
        }
        let bytes = self.state().source.bytes();
        let decoded = self.state().encoding.decode(&bytes)?;
        let current = normalize(&decoded, self.state().file_format);
        debug_assert!(current
            .endings
            .iter()
            .all(|ending| &current.text[ending.normalized.clone()] == "\n"));
        let replacement = self.state().encoding.encode_fragment(target.spelling())?;
        let mut source_patches = current
            .endings
            .iter()
            .filter(|ending| ending.original != target)
            .map(|ending| SourcePatch::primary(ending.source.clone(), replacement.clone()))
            .collect::<Vec<_>>();
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let candidate = self.build_verified_candidate(
            source,
            target,
            FileFormatOrigin::Forced,
            after_revision,
            CandidateVerification {
                expected_text: self.text().to_owned(),
                expected_hard_breaks: None,
                mismatch_error: DocumentError::LineEndingConversionWouldReinterpretContent,
            },
        )?;
        let text_position_map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            after_revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::FileFormat,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work,
                style_change: None,
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_history_navigation(
        &self,
        request: HistoryNavigationRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let current = self.history.status().current;
        let target = match request {
            HistoryNavigationRequest::Undo => self
                .history
                .status()
                .parent
                .ok_or(HistoryError::Boundary(super::HistoryBoundary::Oldest))?,
            HistoryNavigationRequest::Redo => {
                self.history
                    .status()
                    .preferred_redo
                    .ok_or(HistoryError::Boundary(
                        super::HistoryBoundary::NoPreferredRedo,
                    ))?
            }
            HistoryNavigationRequest::SelectNode(node) => self
                .history
                .location_for_node(node)
                .ok_or(HistoryError::NodeNotFound(node))?,
            HistoryNavigationRequest::SelectChange(change) => self
                .history
                .location_for_change(change)
                .ok_or(HistoryError::ChangeNotFound(change))?,
        };
        let candidate = self
            .history
            .state_at_node(target.node)
            .expect("resolved history locations have states");
        let old_text = self.text();
        let new_text = candidate.projection.text();
        let formatted_splices = if old_text == new_text {
            Vec::new()
        } else {
            // This summary is a conservative invalidation range. Persistent
            // positions use the exact composed history-edge map below, so
            // unchanged content inside this range retains identity.
            vec![Splice::new(0..old_text.len(), new_text.len())?]
        };
        let identity = PositionMap::identity(
            self.id,
            super::PositionDomain::FormattedText,
            self.revision(),
            old_text.len(),
        );
        let text_position_map = self
            .history
            .map_between(current.node, target.node, identity)?;
        debug_assert_eq!(text_position_map.target_revision(), candidate.revision);
        debug_assert_eq!(text_position_map.target_len(), new_text.len());
        let source_patches =
            byte_difference(&self.state().source.bytes(), &candidate.source.bytes());
        let navigation = HistoryNavigation {
            from: current,
            to: target,
        };
        Ok(self.prepared(
            candidate.revision,
            ModelChangeSummary {
                kind: ModelChangeKind::HistoryNavigation,
                source_patches,
                formatted_splices,
                projection_work: ProjectionWorkStatistics::none(),
                style_change: None,
            },
            text_position_map,
            Some(navigation),
            self.next_projected_block_id,
            PreparedPublication::History {
                target: target.node,
            },
        ))
    }

    fn build_verified_candidate(
        &self,
        source: super::source::SourceSnapshot,
        file_format: FileFormat,
        file_format_origin: FileFormatOrigin,
        revision: Revision,
        verification: CandidateVerification<'_>,
    ) -> Result<DocumentState, ModelTransactionError> {
        let CandidateVerification {
            expected_text,
            expected_hard_breaks,
            mismatch_error,
        } = verification;
        let decoded = self.state().encoding.decode(&source.bytes())?;
        let evidence = detect(&decoded.text).1;
        let mut candidate = build_state_from_decoded(
            source,
            decoded,
            self.state().format,
            file_format,
            file_format_origin,
            evidence,
            revision,
        )?;
        if candidate.projection.text() != expected_text {
            return Err(mismatch_error.into());
        }
        let preserves_current_hard_lines = candidate
            .projection
            .has_same_hard_line_structure(self.projection());
        if let Some(expected_hard_breaks) = expected_hard_breaks {
            if candidate.projection.hard_break_offsets() != expected_hard_breaks {
                return Err(mismatch_error.into());
            }
        } else if candidate.projection.text() == self.text() && !preserves_current_hard_lines {
            return Err(mismatch_error.into());
        }
        if candidate.projection.text() == self.text() {
            candidate
                .projection
                .install_unchanged_text_storage(self.projection())
                .map_err(DocumentError::FormattedTextStorage)?;
            if preserves_current_hard_lines {
                candidate
                    .projection
                    .install_unchanged_block_ids(self.projection())
                    .map_err(super::block_identity_document_error)?;
            }
        }
        Ok(candidate)
    }

    fn build_verified_text_edit_candidate(
        &self,
        source: super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_splice_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
    ) -> Result<TextEditCandidate, ModelTransactionError> {
        if let Some(region) = self.line_local_projection_region(edits, source_patches)? {
            let new_source_end = rebase_source_boundary(
                region.old_source.end,
                source_patches,
                Association::AfterInsertion,
            )?;
            let new_source = region.old_source.start..new_source_end;
            let regional_bytes = source
                .bytes_in(new_source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = self
                .state()
                .encoding
                .decode_region(&regional_bytes, new_source.start)?;
            let normalized = normalize(&decoded, self.state().file_format);
            if normalized.endings.len() + 1 != region.hard_lines.len() {
                return Err(DocumentError::VerificationFailed.into());
            }
            let regional_projection = project(
                &normalized,
                self.state().format,
                revision,
                new_source.start,
                new_source.end,
            );
            let projected_formatted_bytes = regional_projection.text().len();
            let projected_hard_lines = regional_projection.hard_line_count();
            let (projection, splice_work) = splice_line_local_projection(
                self.projection(),
                regional_projection,
                revision,
                region.hard_lines.clone(),
                region.old_formatted,
                region.old_source,
                new_source,
                target_text.clone(),
                source.len(),
            )
            .map_err(super::block_identity_document_error)?;
            if projection.hard_line_count() != self.projection().hard_line_count() {
                return Err(DocumentError::VerificationFailed.into());
            }

            let source_hard_lines = region
                .hard_lines
                .clone()
                .map(|line| {
                    let old = self
                        .state()
                        .source_hard_lines
                        .get(line)
                        .ok_or(DocumentError::VerificationFailed)?;
                    let start = rebase_source_boundary(
                        old.start,
                        source_patches,
                        Association::BeforeInsertion,
                    )?;
                    let end_association = if line + 1 == self.state().source_hard_lines.len() {
                        Association::AfterInsertion
                    } else {
                        Association::BeforeInsertion
                    };
                    let end = rebase_source_boundary(old.end, source_patches, end_association)?;
                    Ok(start..end)
                })
                .collect::<Result<Vec<_>, ModelTransactionError>>()?;
            let (source_hard_lines, source_line_work) = self
                .state()
                .source_hard_lines
                .replace_ranges_with_stats(region.hard_lines.clone(), &source_hard_lines)
                .ok_or(DocumentError::VerificationFailed)?;
            if source_hard_lines.source_end() != source.len() {
                return Err(DocumentError::VerificationFailed.into());
            }

            return Ok(TextEditCandidate {
                state: DocumentState {
                    revision,
                    source,
                    projection,
                    source_hard_lines,
                    encoding: self.state().encoding,
                    format: self.state().format,
                    file_format: self.state().file_format,
                    file_format_origin: self.state().file_format_origin,
                    line_ending_evidence: self.state().line_ending_evidence,
                    has_bom: self.state().has_bom,
                },
                work: ProjectionWorkStatistics::regional(
                    regional_bytes.len(),
                    projected_formatted_bytes,
                    projected_hard_lines,
                    text_splice_work,
                    splice_work,
                    source_line_work,
                ),
                block_ids_already_reconciled: true,
            });
        }

        let state = self.build_verified_candidate(
            source,
            self.state().file_format,
            self.state().file_format_origin,
            revision,
            CandidateVerification {
                expected_text: target_text.flatten(),
                expected_hard_breaks: None,
                mismatch_error: DocumentError::VerificationFailed,
            },
        )?;
        let work = ProjectionWorkStatistics::full(&state);
        Ok(TextEditCandidate {
            state,
            work,
            block_ids_already_reconciled: false,
        })
    }

    fn line_local_projection_region(
        &self,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
    ) -> Result<Option<LineLocalProjectionRegion>, ModelTransactionError> {
        if edits.is_empty() {
            return Ok(None);
        }
        for edit in edits {
            let replaced = self
                .projection()
                .text_tree()
                .slice(edit.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if edit.replacement.contains('\r')
                || edit.replacement.contains('\n')
                || replaced.contains('\r')
                || replaced.contains('\n')
            {
                // Edits that change hard-line topology require the whole pipeline.
                return Ok(None);
            }
        }

        let mut first_line = usize::MAX;
        let mut last_line = 0usize;
        for edit in edits {
            let Some(line) = self.projection().hard_line_at_offset(edit.range.start) else {
                return Ok(None);
            };
            let Some(line_range) = self.projection().hard_line_range(line) else {
                return Ok(None);
            };
            if edit.range.start < line_range.start || edit.range.end > line_range.end {
                return Ok(None);
            }
            first_line = first_line.min(line);
            last_line = last_line.max(line);
        }
        let hard_lines = first_line..last_line.saturating_add(1);
        if hard_lines.is_empty()
            || hard_lines.len() > MAX_LINE_LOCAL_PROJECTION_HARD_LINES
            || self.state().source_hard_lines.len() != self.projection().hard_line_count()
        {
            return Ok(None);
        }

        let first_formatted = self
            .projection()
            .hard_line_range(first_line)
            .ok_or(DocumentError::VerificationFailed)?;
        let last_formatted = self
            .projection()
            .hard_line_range(last_line)
            .ok_or(DocumentError::VerificationFailed)?;
        let old_formatted = first_formatted.start..last_formatted.end;
        let old_source_start = self
            .state()
            .source_hard_lines
            .get(first_line)
            .ok_or(DocumentError::VerificationFailed)?
            .start;
        let old_source_end = if last_line + 1 == self.projection().hard_line_count() {
            self.state()
                .source_hard_lines
                .get(last_line)
                .ok_or(DocumentError::VerificationFailed)?
                .end
        } else {
            self.projection()
                .source_range(last_formatted.end..last_formatted.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?
                .start
        };
        let old_source = old_source_start..old_source_end;
        if source_patches.iter().any(|patch| {
            patch.part != SourcePartId::PRIMARY
                || patch.range.start < old_source.start
                || patch.range.end > old_source.end
        }) {
            return Ok(None);
        }
        Ok(Some(LineLocalProjectionRegion {
            hard_lines,
            old_formatted,
            old_source,
        }))
    }
}

fn style_block_target_range(target: StyleBlockTarget, whole_document: TextRange) -> TextRange {
    match target {
        StyleBlockTarget::DocumentRoot => whole_document,
        StyleBlockTarget::Paragraphs(range) => range,
    }
}

fn invalid_style_property_target(
    property: StyleProperty,
    target: StylePropertyTarget,
) -> ModelTransactionError {
    ModelTransactionError::Style(StyleTransactionError::InvalidPropertyTarget { property, target })
}

fn merge_character_properties(
    target: &mut CharacterProperties,
    declarations: &CharacterProperties,
) {
    macro_rules! merge {
        ($field:ident) => {
            if declarations.$field.is_some() {
                target.$field.clone_from(&declarations.$field);
            }
        };
    }
    merge!(font_families);
    merge!(size);
    merge!(weight);
    merge!(slant);
    merge!(foreground);
    merge!(background);
    merge!(underline);
    merge!(strikethrough);
    merge!(language);
    merge!(direction);
    merge!(open_type_features);
    merge!(letter_spacing);
    merge!(baseline_shift);
}

fn merge_block_properties(target: &mut BlockProperties, declarations: &BlockProperties) {
    macro_rules! merge {
        ($field:ident) => {
            if declarations.$field.is_some() {
                target.$field.clone_from(&declarations.$field);
            }
        };
    }
    merge!(spacing_before);
    merge!(spacing_after);
    merge!(line_spacing);
    merge!(first_line_indent);
    merge!(leading_indent);
    merge!(trailing_indent);
    merge!(padding_top);
    merge!(padding_right);
    merge!(padding_bottom);
    merge!(padding_left);
    merge!(background);
    merge!(alignment);
    merge!(base_direction);
}

fn clear_character_properties(
    target: &mut CharacterProperties,
    properties: &BTreeSet<StyleProperty>,
    expected: StylePropertyTarget,
) -> Result<(), ModelTransactionError> {
    for property in properties {
        match property {
            StyleProperty::CharacterFontFamilies => target.font_families = None,
            StyleProperty::CharacterSize => target.size = None,
            StyleProperty::CharacterWeight => target.weight = None,
            StyleProperty::CharacterSlant => target.slant = None,
            StyleProperty::CharacterForeground => target.foreground = None,
            StyleProperty::CharacterBackground => target.background = None,
            StyleProperty::CharacterUnderline => target.underline = None,
            StyleProperty::CharacterStrikethrough => target.strikethrough = None,
            StyleProperty::CharacterLanguage => target.language = None,
            StyleProperty::CharacterDirection => target.direction = None,
            StyleProperty::CharacterOpenTypeFeatures => target.open_type_features = None,
            StyleProperty::CharacterLetterSpacing => target.letter_spacing = None,
            StyleProperty::CharacterBaselineShift => target.baseline_shift = None,
            _ => return Err(invalid_style_property_target(*property, expected)),
        }
    }
    Ok(())
}

fn clear_block_properties(
    target: &mut BlockProperties,
    properties: &BTreeSet<StyleProperty>,
    expected: StylePropertyTarget,
) -> Result<(), ModelTransactionError> {
    for property in properties {
        match property {
            StyleProperty::CanvasBackground => target.background = None,
            StyleProperty::CanvasPaddingTop => target.padding_top = None,
            StyleProperty::CanvasPaddingRight => target.padding_right = None,
            StyleProperty::CanvasPaddingBottom => target.padding_bottom = None,
            StyleProperty::CanvasPaddingLeft => target.padding_left = None,
            _ => return Err(invalid_style_property_target(*property, expected)),
        }
    }
    Ok(())
}

fn validate_projection_style_configuration(
    projection: &FormattedDocument,
    style_sheet: &StyleSheet,
    document_style: &DocumentStyleAssignment,
) -> Result<(), StyleError> {
    style_sheet.resolve_document_assignment(document_style)?;
    for block in projection.blocks() {
        style_sheet.resolve_assigned_paragraph_style(
            document_style,
            &block.style,
            &block.direct_paragraph,
            &block.direct_default_character,
            None,
            &CharacterProperties::default(),
        )?;
    }
    for span in projection.style_spans() {
        if let StyleApplication::Named(style) = &span.application {
            style_sheet
                .character_style(style)
                .ok_or_else(|| StyleError::UnknownStyle(style.clone()))?;
        }
    }
    Ok(())
}

fn configuration_style_change_summary(
    projection: &FormattedDocument,
    before_sheet: &StyleSheet,
    after_sheet: &StyleSheet,
    before_assignment: &DocumentStyleAssignment,
    after_assignment: &DocumentStyleAssignment,
    intent: &ConfigurationStyleIntent,
) -> Result<StyleChangeSummary, StyleError> {
    let mut affected_block_styles = BTreeSet::new();
    let mut affected_character_styles = BTreeSet::new();
    let mut changed_properties = BTreeSet::new();
    let root_configuration_changed = !matches!(intent, ConfigurationStyleIntent::EditDefinition(_));

    if let ConfigurationStyleIntent::EditDefinition(edit) = intent {
        if edit.is_block() {
            collect_block_dependents(before_sheet, edit.style_id(), &mut affected_block_styles);
            collect_block_dependents(after_sheet, edit.style_id(), &mut affected_block_styles);
            changed_properties.extend(effective_block_definition_changes(
                before_sheet,
                after_sheet,
                before_assignment,
                after_assignment,
                &affected_block_styles,
            )?);
        } else {
            collect_character_dependents(
                before_sheet,
                edit.style_id(),
                &mut affected_character_styles,
            );
            collect_character_dependents(
                after_sheet,
                edit.style_id(),
                &mut affected_character_styles,
            );
            changed_properties.extend(effective_character_definition_changes(
                before_sheet,
                after_sheet,
                before_assignment,
                after_assignment,
                &affected_character_styles,
            )?);
        }
    } else {
        if matches!(
            intent,
            ConfigurationStyleIntent::AssignDocumentStyle(_)
                | ConfigurationStyleIntent::SetDocumentDefaultCharacter(_)
                | ConfigurationStyleIntent::ClearDocumentDefaultCharacter(_)
        ) {
            affected_block_styles.extend(after_sheet.block_styles().map(|style| style.id.clone()));
        }
        let before = before_sheet.resolve_document_assignment(before_assignment)?;
        let after = after_sheet.resolve_document_assignment(after_assignment)?;
        changed_properties.extend(before.changed_properties(&after));
    }

    let affected_ranges = if root_configuration_changed {
        std::iter::once(0..projection.text_tree().byte_len()).collect()
    } else {
        projection.style_dependency_ranges(&affected_block_styles, &affected_character_styles)
    };
    let invalidation_effects = if affected_ranges.is_empty() {
        BTreeSet::new()
    } else {
        changed_properties
            .iter()
            .map(|property| property.invalidation_effect())
            .collect()
    };

    Ok(StyleChangeSummary {
        before_revision: before_sheet.revision,
        after_revision: after_sheet.revision,
        changed_properties,
        invalidation_effects,
        affected_block_styles: affected_block_styles.into_iter().collect(),
        affected_character_styles: affected_character_styles.into_iter().collect(),
        affected_ranges,
    })
}

fn collect_block_dependents(sheet: &StyleSheet, style: &StyleId, output: &mut BTreeSet<StyleId>) {
    if let Some(dependents) = sheet.dependency_index().block_dependents(style) {
        output.extend(dependents.iter().cloned());
    }
}

fn collect_character_dependents(
    sheet: &StyleSheet,
    style: &StyleId,
    output: &mut BTreeSet<StyleId>,
) {
    if let Some(dependents) = sheet.dependency_index().character_dependents(style) {
        output.extend(dependents.iter().cloned());
    }
}

fn effective_block_definition_changes(
    before_sheet: &StyleSheet,
    after_sheet: &StyleSheet,
    before_assignment: &DocumentStyleAssignment,
    after_assignment: &DocumentStyleAssignment,
    affected: &BTreeSet<StyleId>,
) -> Result<BTreeSet<StyleProperty>, StyleError> {
    let mut changed = BTreeSet::new();
    for id in affected {
        let (Some(before), Some(after)) =
            (before_sheet.block_style(id), after_sheet.block_style(id))
        else {
            continue;
        };
        if before.role != after.role {
            changed.extend(before.block.changed_properties(&BlockProperties::default()));
            changed.extend(after.block.changed_properties(&BlockProperties::default()));
            changed.extend(
                before
                    .character
                    .changed_properties(&CharacterProperties::default()),
            );
            changed.extend(
                after
                    .character
                    .changed_properties(&CharacterProperties::default()),
            );
            continue;
        }
        match before.role {
            super::BlockRole::Document => {
                let before = before_sheet.resolve_document_style(
                    id,
                    &BlockProperties::default(),
                    &CharacterProperties::default(),
                )?;
                let after = after_sheet.resolve_document_style(
                    id,
                    &BlockProperties::default(),
                    &CharacterProperties::default(),
                )?;
                changed.extend(before.changed_properties(&after));
            }
            super::BlockRole::Paragraph => {
                let before = before_sheet.resolve_assigned_paragraph_style(
                    before_assignment,
                    id,
                    &BlockProperties::default(),
                    &CharacterProperties::default(),
                    None,
                    &CharacterProperties::default(),
                )?;
                let after = after_sheet.resolve_assigned_paragraph_style(
                    after_assignment,
                    id,
                    &BlockProperties::default(),
                    &CharacterProperties::default(),
                    None,
                    &CharacterProperties::default(),
                )?;
                changed.extend(before.changed_properties(&after));
            }
        }
    }
    Ok(changed)
}

fn effective_character_definition_changes(
    before_sheet: &StyleSheet,
    after_sheet: &StyleSheet,
    before_assignment: &DocumentStyleAssignment,
    after_assignment: &DocumentStyleAssignment,
    affected: &BTreeSet<StyleId>,
) -> Result<BTreeSet<StyleProperty>, StyleError> {
    let mut changed = BTreeSet::new();
    for id in affected {
        if before_sheet.character_style(id).is_none() || after_sheet.character_style(id).is_none() {
            continue;
        }
        let before = before_sheet.resolve_assigned_paragraph_style(
            before_assignment,
            &before_sheet.base_paragraph,
            &BlockProperties::default(),
            &CharacterProperties::default(),
            Some(id),
            &CharacterProperties::default(),
        )?;
        let after = after_sheet.resolve_assigned_paragraph_style(
            after_assignment,
            &after_sheet.base_paragraph,
            &BlockProperties::default(),
            &CharacterProperties::default(),
            Some(id),
            &CharacterProperties::default(),
        )?;
        changed.extend(before.character.changed_properties(&after.character));
    }
    Ok(changed)
}

fn markdown_style_markers(style: SemanticInlineStyle) -> &'static [&'static str] {
    match style {
        SemanticInlineStyle::Strong => &["**", "__"],
        SemanticInlineStyle::Emphasis => &["*", "_"],
        SemanticInlineStyle::Code => &["`"],
    }
}

fn preferred_markdown_style_marker(style: SemanticInlineStyle) -> &'static str {
    markdown_style_markers(style)[0]
}

/// Verify the whole projected style collection after changing source syntax.
/// Checking only for presence/absence of the target would allow removing one
/// delimiter pair to reinterpret adjacent Markdown as a different style.
fn semantic_style_edit_was_exactly_projected(
    before: &[StyleSpan],
    after: &[StyleSpan],
    range: &Range<usize>,
    style: SemanticInlineStyle,
    enabled: bool,
) -> bool {
    let target = StyleSpan {
        range: range.clone(),
        application: StyleApplication::Semantic(style),
    };
    let mut expected = before.to_vec();
    if enabled {
        expected.push(target);
    } else if let Some(index) = expected.iter().position(|span| span == &target) {
        expected.remove(index);
    } else {
        return false;
    }

    if expected.len() != after.len() {
        return false;
    }
    let mut unmatched = after.to_vec();
    expected.into_iter().all(|span| {
        unmatched
            .iter()
            .position(|candidate| candidate == &span)
            .map(|index| {
                unmatched.remove(index);
            })
            .is_some()
    })
}

fn hard_breaks_after_line_replacement(
    current: &[usize],
    range: &Range<usize>,
    replacement_len: usize,
) -> Result<Vec<usize>, ModelTransactionError> {
    current
        .iter()
        .map(|&offset| {
            if range.start < offset && offset < range.end {
                return Err(DocumentError::HardLineSourceImageProjectionMismatch.into());
            }
            if offset < range.end {
                return Ok(offset);
            }
            offset
                .checked_sub(range.len())
                .and_then(|value| value.checked_add(replacement_len))
                .ok_or_else(|| PositionError::ArithmeticOverflow.into())
        })
        .collect()
}

fn rebase_source_boundary(
    boundary: usize,
    patches: &[SourcePatch],
    association: Association,
) -> Result<usize, ModelTransactionError> {
    let mut mapped = boundary;
    for patch in patches {
        let precedes = patch.range.end < boundary
            || (patch.range.end == boundary
                && (!patch.range.is_empty() || association == Association::AfterInsertion));
        if precedes {
            mapped = mapped
                .checked_sub(patch.range.len())
                .and_then(|value| value.checked_add(patch.replacement.len()))
                .ok_or(PositionError::ArithmeticOverflow)?;
        }
    }
    Ok(mapped)
}

fn apply_payload_break_edits(
    current_breaks: &[usize],
    edits: &[FormattedPayloadEdit],
    old_length: usize,
    expected_length: usize,
) -> Result<Vec<usize>, PositionError> {
    let inserted_breaks = edits.iter().try_fold(0usize, |count, edit| {
        count
            .checked_add(edit.payload.break_offsets().len())
            .ok_or(PositionError::ArithmeticOverflow)
    })?;
    let mut result = Vec::with_capacity(
        current_breaks
            .len()
            .checked_add(inserted_breaks)
            .ok_or(PositionError::ArithmeticOverflow)?,
    );
    let mut break_index = 0usize;
    let mut old_cursor = 0usize;
    let mut new_cursor = 0usize;

    for edit in edits {
        while let Some(&offset) = current_breaks.get(break_index) {
            if offset >= edit.range.start {
                break;
            }
            let relative = offset
                .checked_sub(old_cursor)
                .ok_or(PositionError::ArithmeticOverflow)?;
            result.push(
                new_cursor
                    .checked_add(relative)
                    .ok_or(PositionError::ArithmeticOverflow)?,
            );
            break_index += 1;
        }
        new_cursor = new_cursor
            .checked_add(
                edit.range
                    .start
                    .checked_sub(old_cursor)
                    .ok_or(PositionError::ArithmeticOverflow)?,
            )
            .ok_or(PositionError::ArithmeticOverflow)?;
        while current_breaks
            .get(break_index)
            .is_some_and(|offset| *offset < edit.range.end)
        {
            break_index += 1;
        }
        for &offset in edit.payload.break_offsets() {
            result.push(
                new_cursor
                    .checked_add(offset)
                    .ok_or(PositionError::ArithmeticOverflow)?,
            );
        }
        new_cursor = new_cursor
            .checked_add(edit.payload.text().len())
            .ok_or(PositionError::ArithmeticOverflow)?;
        old_cursor = edit.range.end;
    }

    while let Some(&offset) = current_breaks.get(break_index) {
        let relative = offset
            .checked_sub(old_cursor)
            .ok_or(PositionError::ArithmeticOverflow)?;
        result.push(
            new_cursor
                .checked_add(relative)
                .ok_or(PositionError::ArithmeticOverflow)?,
        );
        break_index += 1;
    }
    let trailing = old_length
        .checked_sub(old_cursor)
        .ok_or(PositionError::ArithmeticOverflow)?;
    let computed_length = new_cursor
        .checked_add(trailing)
        .ok_or(PositionError::ArithmeticOverflow)?;
    if computed_length != expected_length {
        return Err(PositionError::TargetLengthMismatch {
            computed: computed_length,
            actual: expected_length,
        });
    }
    Ok(result)
}

fn structured_payload_syntax(
    payload: &FormattedTextPayload,
    format: Format,
    in_code: bool,
    file_format: FileFormat,
) -> String {
    let escape_markdown = format == Format::Markdown && (!in_code || payload.text().contains('`'));
    let mut syntax = String::with_capacity(payload.text().len());
    let mut start = 0usize;
    for &hard_break in payload.break_offsets() {
        let segment = &payload.text()[start..hard_break];
        if escape_markdown {
            syntax.push_str(&escape_markdown_insert(segment));
        } else {
            syntax.push_str(segment);
        }
        syntax.push_str(file_format.spelling());
        start = hard_break
            .checked_add(1)
            .expect("validated payload break offset is representable");
    }
    let segment = &payload.text()[start..];
    if escape_markdown {
        syntax.push_str(&escape_markdown_insert(segment));
    } else {
        syntax.push_str(segment);
    }
    syntax
}

fn validate_source_patches(patches: &mut [SourcePatch]) -> Result<(), ModelTransactionError> {
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    for pair in patches.windows(2) {
        if pair[0].range.end > pair[1].range.start
            || (pair[0].range.is_empty()
                && pair[1].range.is_empty()
                && pair[0].range.start == pair[1].range.start)
        {
            return Err(DocumentError::AmbiguousProjection.into());
        }
    }
    Ok(())
}

fn apply_source_patches(
    source: &super::source::SourceSnapshot,
    patches: &[SourcePatch],
) -> Result<super::source::SourceSnapshot, ModelTransactionError> {
    let mut candidate = source.clone();
    for patch in patches.iter().rev() {
        candidate = candidate
            .replace(
                patch.range.start,
                patch.range.end,
                patch.replacement.clone(),
            )
            .ok_or(DocumentError::AmbiguousProjection)?;
    }
    Ok(candidate)
}

fn byte_difference(before: &[u8], after: &[u8]) -> Vec<SourcePatch> {
    if before == after {
        return Vec::new();
    }
    let prefix = before
        .iter()
        .zip(after)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    vec![SourcePatch::primary(
        prefix..before.len() - suffix,
        after[prefix..after.len() - suffix].to_vec(),
    )]
}

/// Minimal contiguous physical change for one source line, widened only as
/// needed to avoid cutting a UTF-8 sequence or UTF-16 code unit. Untouched
/// prefixes, suffixes, and the unchanged line terminator remain outside the
/// declared patch.
fn hard_line_byte_difference(
    before: &[u8],
    after: &[u8],
    encoding: super::Encoding,
) -> Vec<SourcePatch> {
    if before == after {
        return Vec::new();
    }
    let mut prefix = before
        .iter()
        .zip(after)
        .take_while(|(left, right)| left == right)
        .count();
    let mut suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();

    match encoding {
        super::Encoding::Utf16Le | super::Encoding::Utf16Be => {
            prefix -= prefix % 2;
            suffix -= suffix % 2;
        }
        super::Encoding::Utf8 => {
            while prefix > 0
                && ((prefix < before.len() && before[prefix] & 0xc0 == 0x80)
                    || (prefix < after.len() && after[prefix] & 0xc0 == 0x80))
            {
                prefix -= 1;
            }
            while suffix > 0 {
                let before_start = before.len() - suffix;
                let after_start = after.len() - suffix;
                if before[before_start] & 0xc0 != 0x80 && after[after_start] & 0xc0 != 0x80 {
                    break;
                }
                suffix -= 1;
            }
        }
        super::Encoding::Latin1 => {}
    }

    vec![SourcePatch::primary(
        prefix..before.len() - suffix,
        after[prefix..after.len() - suffix].to_vec(),
    )]
}

#[derive(Clone, Copy, Debug)]
struct GraphemeMapCut {
    old: usize,
    new: usize,
}

#[derive(Clone, Copy, Debug)]
struct GraphemeMapGap {
    first_stable: Option<GraphemeMapCut>,
    last_stable: Option<GraphemeMapCut>,
}

/// Snapshot-backed grapheme closure for formatted edits. The supplied
/// snapshots define logical item boundaries, including forced boundaries
/// around semantic hard breaks, without materializing either complete text.
fn grapheme_closed_snapshot_map_splices<B, A>(
    before: &B,
    after: &A,
    edits: &[TextEdit],
) -> Result<Vec<Splice>, PositionError>
where
    B: LogicalGraphemeSnapshot,
    A: LogicalGraphemeSnapshot,
{
    debug_assert!(!edits.is_empty());

    let mut target_ranges = Vec::with_capacity(edits.len());
    let mut old_cursor = 0usize;
    let mut new_cursor = 0usize;
    for edit in edits {
        let unchanged = edit
            .range
            .start
            .checked_sub(old_cursor)
            .ok_or(PositionError::ArithmeticOverflow)?;
        new_cursor = new_cursor
            .checked_add(unchanged)
            .ok_or(PositionError::ArithmeticOverflow)?;
        let target_start = new_cursor;
        new_cursor = new_cursor
            .checked_add(edit.replacement.len())
            .ok_or(PositionError::ArithmeticOverflow)?;
        target_ranges.push(target_start..new_cursor);
        old_cursor = edit.range.end;
    }
    let trailing = before
        .text_len()
        .checked_sub(old_cursor)
        .ok_or(PositionError::ArithmeticOverflow)?;
    let computed_after_len = new_cursor
        .checked_add(trailing)
        .ok_or(PositionError::ArithmeticOverflow)?;
    if computed_after_len != after.text_len() {
        return Err(PositionError::TargetLengthMismatch {
            computed: computed_after_len,
            actual: after.text_len(),
        });
    }

    let mut gaps = Vec::with_capacity(edits.len() + 1);
    gaps.push(stable_snapshot_gap_cuts(
        0..edits[0].range.start,
        0..target_ranges[0].start,
        before,
        after,
    )?);
    for index in 0..edits.len().saturating_sub(1) {
        gaps.push(stable_snapshot_gap_cuts(
            edits[index].range.end..edits[index + 1].range.start,
            target_ranges[index].end..target_ranges[index + 1].start,
            before,
            after,
        )?);
    }
    let last = edits.len() - 1;
    gaps.push(stable_snapshot_gap_cuts(
        edits[last].range.end..before.text_len(),
        target_ranges[last].end..after.text_len(),
        before,
        after,
    )?);

    let mut left = gaps[0]
        .last_stable
        .expect("the document-start boundary is stable");
    let mut splices = Vec::with_capacity(edits.len());
    for gap in gaps.iter().take(edits.len()).skip(1) {
        let Some(right) = gap.first_stable else {
            continue;
        };
        splices.push(Splice::new(
            left.old..right.old,
            right
                .new
                .checked_sub(left.new)
                .ok_or(PositionError::ArithmeticOverflow)?,
        )?);
        left = gap
            .last_stable
            .expect("a gap with a first stable cut also has a last one");
    }
    let right = gaps
        .last()
        .and_then(|gap| gap.first_stable)
        .expect("the document-end boundary is stable");
    splices.push(Splice::new(
        left.old..right.old,
        right
            .new
            .checked_sub(left.new)
            .ok_or(PositionError::ArithmeticOverflow)?,
    )?);
    Ok(splices)
}

fn stable_snapshot_gap_cuts<B, A>(
    old: Range<usize>,
    new: Range<usize>,
    old_text: &B,
    new_text: &A,
) -> Result<GraphemeMapGap, PositionError>
where
    B: LogicalGraphemeSnapshot,
    A: LogicalGraphemeSnapshot,
{
    debug_assert_eq!(old.end - old.start, new.end - new.start);
    let cut_at = |old_offset| -> Result<Option<GraphemeMapCut>, PositionError> {
        let new_offset = new
            .start
            .checked_add(old_offset - old.start)
            .ok_or(PositionError::ArithmeticOverflow)?;
        Ok(new_text
            .is_logical_grapheme_boundary(new_offset)
            .map_err(|_| PositionError::InvalidUnicodeBoundary { offset: new_offset })?
            .then_some(GraphemeMapCut {
                old: old_offset,
                new: new_offset,
            }))
    };

    let mut old_offset = old.start;
    let first_stable = loop {
        if let Some(cut) = cut_at(old_offset)? {
            break Some(cut);
        }
        if old_offset == old.end {
            break None;
        }
        let Some(next) = old_text
            .next_logical_grapheme_boundary(old_offset)
            .map_err(|_| PositionError::InvalidUnicodeBoundary { offset: old_offset })?
        else {
            break None;
        };
        if next > old.end {
            break None;
        }
        old_offset = next;
    };

    let mut old_offset = old.end;
    let last_stable = loop {
        if let Some(cut) = cut_at(old_offset)? {
            break Some(cut);
        }
        if old_offset == old.start {
            break None;
        }
        let Some(previous) = old_text
            .previous_logical_grapheme_boundary(old_offset)
            .map_err(|_| PositionError::InvalidUnicodeBoundary { offset: old_offset })?
        else {
            break None;
        };
        if previous < old.start {
            break None;
        }
        old_offset = previous;
    };
    Ok(GraphemeMapGap {
        first_stable,
        last_stable,
    })
}

fn compat_document_error(error: ModelTransactionError) -> DocumentError {
    match error {
        ModelTransactionError::Document(error) => error,
        ModelTransactionError::WrongDocument { .. } => DocumentError::WrongDocument,
        ModelTransactionError::StaleRevision { expected, actual } => {
            // The model-transaction error names the requested/prepared revision
            // as `expected` and the live document as `actual`. The older
            // compatibility error uses those field names in the opposite
            // sense: `expected` is the live revision and `actual` is the stale
            // snapshot carried by the caller.
            DocumentError::WrongSnapshot {
                expected: actual,
                actual: expected,
            }
        }
        ModelTransactionError::Position(_)
        | ModelTransactionError::History(_)
        | ModelTransactionError::Style(_)
        | ModelTransactionError::StaleDocumentState
        | ModelTransactionError::RevisionExhausted => DocumentError::VerificationFailed,
    }
}
