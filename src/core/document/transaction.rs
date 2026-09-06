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
    SemanticInlineStyle, SourcePartId, Splice, StyleApplication, StyleDefinitionEdit,
    StyleDefinitionOrigin, StyleError, StyleId, StyleInvalidationEffect, StyleProperty, StyleSheet,
    StyleSheetRevision, StyleSpan, TextEdit, TextRange, UnsupportedEditReason,
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
#[derive(Clone, Debug, PartialEq)]
pub enum ModelRequest {
    SetDirectCharacterProperties {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        values: Vec<(StyleProperty, super::StylePropertyValue)>,
    },
    EditDirectProperty {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        property: StyleProperty,
        value: Option<super::StylePropertyValue>,
    },
    ReplacePhysicalSource {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        replacement: String,
    },
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
    SetListStyle {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        style: Option<super::ListStyle>,
    },
    SetParagraphStyle {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        style: StyleId,
    },
    AssignNamedStyle {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        namespace: super::StyleNamespace,
        style: StyleId,
    },
    EditNamedStyleDefinition {
        document: DocumentId,
        revision: Revision,
        edit: StyleDefinitionEdit,
    },
    ContinueList {
        document: DocumentId,
        revision: Revision,
        at: usize,
    },
    SetFileFormat {
        document: DocumentId,
        revision: Revision,
        target: FileFormat,
    },
    /// Change source interpretation without changing any authoritative bytes.
    SetFormat {
        document: DocumentId,
        revision: Revision,
        target: Format,
    },
    /// Explicitly transcode the source, preserving decoded syntax and content.
    SetEncoding {
        document: DocumentId,
        revision: Revision,
        target: super::Encoding,
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
            Self::SetDirectCharacterProperties { document, .. } => *document,
            Self::EditDirectProperty { document, .. } => *document,
            Self::ReplacePhysicalSource { document, .. } => *document,
            Self::ApplyTextEdits { document, .. }
            | Self::SetSemanticStyle { document, .. }
            | Self::SetListStyle { document, .. }
            | Self::SetParagraphStyle { document, .. }
            | Self::AssignNamedStyle { document, .. }
            | Self::EditNamedStyleDefinition { document, .. }
            | Self::ContinueList { document, .. }
            | Self::SetFileFormat { document, .. }
            | Self::SetFormat { document, .. }
            | Self::SetEncoding { document, .. }
            | Self::TransferHardLines { document, .. }
            | Self::RestoreHardLineSource { document, .. }
            | Self::NavigateHistory { document, .. } => *document,
        }
    }

    pub fn revision(&self) -> Revision {
        match self {
            Self::SetDirectCharacterProperties { revision, .. } => *revision,
            Self::EditDirectProperty { revision, .. } => *revision,
            Self::ReplacePhysicalSource { revision, .. } => *revision,
            Self::ApplyTextEdits { revision, .. }
            | Self::SetSemanticStyle { revision, .. }
            | Self::SetListStyle { revision, .. }
            | Self::SetParagraphStyle { revision, .. }
            | Self::AssignNamedStyle { revision, .. }
            | Self::EditNamedStyleDefinition { revision, .. }
            | Self::ContinueList { revision, .. }
            | Self::SetFileFormat { revision, .. }
            | Self::SetFormat { revision, .. }
            | Self::SetEncoding { revision, .. }
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
            ModelRequest::SetDirectCharacterProperties { range, values, .. } => {
                let range =
                    TextRange::new(self.text_point(range.start)?, self.text_point(range.end)?)?;
                let mut properties = CharacterProperties::default();
                for (property, value) in values {
                    super::style::set_character_property(
                        &StyleId::from("Direct"),
                        &mut properties,
                        property,
                        &value,
                    )?;
                }
                super::style::validate_character_properties(&StyleId::from("Direct"), &properties)?;
                self.prepare_persisted_style_intent(
                    PersistedStyleIntent::SetDirectCharacterProperties { range, properties },
                )
            }
            ModelRequest::EditDirectProperty {
                range,
                property,
                value,
                ..
            } => {
                let range =
                    TextRange::new(self.text_point(range.start)?, self.text_point(range.end)?)?;
                let character = super::style::is_character_property(property);
                let intent = if let Some(value) = value {
                    if character {
                        let mut properties = CharacterProperties::default();
                        super::style::set_character_property(
                            &StyleId::from("Direct"),
                            &mut properties,
                            property,
                            &value,
                        )?;
                        super::style::validate_character_properties(
                            &StyleId::from("Direct"),
                            &properties,
                        )?;
                        PersistedStyleIntent::SetDirectCharacterProperties { range, properties }
                    } else {
                        let mut properties = BlockProperties::default();
                        super::style::set_block_property(
                            &StyleId::from("Direct"),
                            &mut properties,
                            property,
                            &value,
                        )?;
                        super::style::validate_block_property_values(
                            &StyleId::from("Direct"),
                            &properties,
                        )?;
                        PersistedStyleIntent::SetDirectBlockProperties {
                            target: StyleBlockTarget::Paragraphs(range),
                            properties,
                        }
                    }
                } else if character {
                    PersistedStyleIntent::ClearDirectCharacterProperties {
                        range,
                        properties: BTreeSet::from([property]),
                    }
                } else {
                    PersistedStyleIntent::ClearDirectBlockProperties {
                        target: StyleBlockTarget::Paragraphs(range),
                        properties: BTreeSet::from([property]),
                    }
                };
                self.prepare_persisted_style_intent(intent)
            }
            ModelRequest::ReplacePhysicalSource {
                range, replacement, ..
            } => self.prepare_physical_source(range, replacement),
            ModelRequest::ApplyTextEdits { edits, .. } => self.prepare_text_edits(edits),
            ModelRequest::SetSemanticStyle {
                range,
                style,
                enabled,
                ..
            } => self.prepare_semantic_style(range, style, enabled),
            ModelRequest::SetListStyle { range, style, .. } => {
                self.prepare_list_style(range, style)
            }
            ModelRequest::SetParagraphStyle { range, style, .. } => {
                self.prepare_markdown_paragraph_style(range, style)
            }
            ModelRequest::EditNamedStyleDefinition { edit, .. } => self
                .prepare_persisted_style_intent(PersistedStyleIntent::EditStyleDefinition {
                    origin: StyleDefinitionOrigin::SourceBacked,
                    edit,
                }),
            ModelRequest::AssignNamedStyle {
                range,
                namespace,
                style,
                ..
            } => {
                let range =
                    TextRange::new(self.text_point(range.start)?, self.text_point(range.end)?)?;
                let intent = match namespace {
                    super::StyleNamespace::Block => PersistedStyleIntent::AssignBlockStyle {
                        target: StyleBlockTarget::Paragraphs(range),
                        style,
                    },
                    super::StyleNamespace::Character => {
                        PersistedStyleIntent::AssignCharacterStyle { range, style }
                    }
                };
                self.prepare_persisted_style_intent(intent)
            }
            ModelRequest::ContinueList { at, .. } => self.prepare_rich_list_enter(at),
            ModelRequest::SetFileFormat { target, .. } => self.prepare_file_format(target),
            ModelRequest::SetFormat { target, .. } => self.prepare_format(target),
            ModelRequest::SetEncoding { target, .. } => self.prepare_encoding(target),
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
        if matches!(self.format(), Format::Markdown | Format::MarkdownSource) {
            if let PersistedStyleIntent::AssignBlockStyle {
                target: StyleBlockTarget::Paragraphs(range),
                style,
            } = &intent
            {
                self.validate_style_text_range(*range)?;
                return self.prepare_markdown_paragraph_style(
                    range.start().offset()..range.end().offset(),
                    style.clone(),
                );
            }
        }
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

        if matches!(self.format(), Format::Html | Format::Rtf) {
            match &intent {
                PersistedStyleIntent::SetDirectCharacterProperties { range, properties } => {
                    self.validate_style_text_range(*range)?;
                    return self.prepare_rich_character_properties(
                        range.start().offset()..range.end().offset(),
                        properties.clone(),
                        None,
                    );
                }
                PersistedStyleIntent::ClearDirectCharacterProperties { range, properties } => {
                    self.validate_style_text_range(*range)?;
                    return self.prepare_rich_clear_character(
                        range.start().offset()..range.end().offset(),
                        properties.clone(),
                    );
                }
                PersistedStyleIntent::SetDirectBlockProperties { .. }
                | PersistedStyleIntent::ClearDirectBlockProperties { .. } => {
                    return self.prepare_rich_block_properties(intent)
                }
                _ => {}
            }
        }
        if self.format() == Format::Html
            && matches!(
                intent,
                PersistedStyleIntent::EditStyleDefinition { .. }
                    | PersistedStyleIntent::AssignBlockStyle { .. }
                    | PersistedStyleIntent::AssignCharacterStyle { .. }
            )
        {
            return self.prepare_html_named_style(intent);
        }
        if self.format() == Format::Rtf
            && matches!(
                intent,
                PersistedStyleIntent::EditStyleDefinition { .. }
                    | PersistedStyleIntent::AssignBlockStyle { .. }
                    | PersistedStyleIntent::AssignCharacterStyle { .. }
            )
        {
            return self.prepare_rtf_named_style(intent);
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

    fn prepare_html_named_style(
        &self,
        intent: PersistedStyleIntent,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let revision = Revision(self.next_revision);
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let normalized = normalize(&decoded, self.file_format());
        let converter = super::rich_text::Builder::new(&normalized, revision);
        let before = self.projection().style_sheet();
        let mut expected = before.clone();
        let syntax = match &intent {
            PersistedStyleIntent::EditStyleDefinition { edit, .. } => {
                let deleting = matches!(
                    edit,
                    StyleDefinitionEdit::DeleteCharacter(_) | StyleDefinitionEdit::DeleteBlock(_)
                );
                expected.rebase_source_references_for_delete(
                    edit,
                    StyleSheetRevision(revision.0 + 1),
                )?;
                if !expected.apply_source_edit(edit, StyleSheetRevision(revision.0 + 1), false)? {
                    return Ok(self.no_op_prepared());
                }
                let mut patches =
                    super::html_styles::definition_patches(&normalized.text, before, &expected)?
                        .into_iter()
                        .map(|(range, text)| (converter.source_range(range), text))
                        .collect::<Vec<_>>();
                if deleting {
                    patches.extend(super::html_styles::remove_assignment_patches(
                        &normalized,
                        before,
                        edit.style_id(),
                        !edit.is_block(),
                    ));
                }
                patches
            }
            PersistedStyleIntent::AssignCharacterStyle { range, style } => {
                self.validate_style_text_range(*range)?;
                if expected.character_style(style).is_none() {
                    return Err(StyleError::UnknownStyle(style.clone()).into());
                }
                let range = range.start().offset()..range.end().offset();
                let mut position = range.start;
                let mut sources: Vec<Range<usize>> = Vec::new();
                for span in self.projection().provenance_for_region(&range) {
                    if span.formatted.start != position
                        || span.formatted.end > range.end
                        || span.source.is_empty()
                    {
                        return Err(DocumentError::AmbiguousProjection.into());
                    }
                    position = span.formatted.end;
                    if self.text().get(span.formatted.clone()) == Some("\n") {
                        continue;
                    }
                    if let Some(last) = sources
                        .last_mut()
                        .filter(|last| last.end == span.source.start)
                    {
                        last.end = span.source.end;
                    } else {
                        sources.push(span.source);
                    }
                }
                if position != range.end {
                    return Err(DocumentError::AmbiguousProjection.into());
                }
                if sources.is_empty() {
                    return Ok(self.no_op_prepared());
                }
                let mut patches = Vec::with_capacity(sources.len() * 2);
                for source in sources {
                    patches.push((
                        source.start..source.start,
                        format!(
                            "<span class=\"{}\">",
                            super::html_styles::class_name(style, true)
                        ),
                    ));
                    patches.push((source.end..source.end, "</span>".into()));
                }
                patches
            }
            PersistedStyleIntent::AssignBlockStyle {
                target: StyleBlockTarget::Paragraphs(range),
                style,
            } => {
                self.validate_style_text_range(*range)?;
                if expected.block_style(style).is_none() {
                    return Err(StyleError::UnknownStyle(style.clone()).into());
                }
                let start = range.start().offset();
                let end = range.end().offset();
                let sources = if self.text().is_empty() {
                    vec![super::rich_text::text_source_range(self, &(0..0))?]
                } else {
                    self.projection()
                        .blocks()
                        .iter()
                        .filter(|block| {
                            if start == end {
                                block.range.start <= start && start <= block.range.end
                            } else {
                                block.range.start < end && start < block.range.end
                            }
                        })
                        .map(|block| {
                            self.projection()
                                .source_range(block.range.clone())
                                .ok_or(DocumentError::AmbiguousProjection)
                        })
                        .collect::<Result<Vec<_>, _>>()?
                };
                let mut patches = super::html_styles::paragraph_assignment_patches(
                    &normalized,
                    &sources,
                    before,
                    style,
                )?;
                if style.0.starts_with("List") {
                    let owned = super::html_styles::read(&normalized.text);
                    if !owned.rules.iter().any(|rule| {
                        rule.definition.is_block() && rule.definition.style_id() == style
                    }) {
                        let supporting = if let Some(at) = owned.close {
                            vec![(
                                at..at,
                                super::html_styles::write_rule(before, style, false)
                                    .ok_or(DocumentError::UnsupportedFormatting)?,
                            )]
                        } else {
                            super::html_styles::definition_patches(
                                &normalized.text,
                                before,
                                before,
                            )?
                        };
                        patches.splice(
                            0..0,
                            supporting
                                .into_iter()
                                .map(|(range, text)| (converter.source_range(range), text)),
                        );
                    }
                }
                patches
            }
            _ => return Err(DocumentError::UnsupportedFormatting.into()),
        };
        let mut syntax = syntax;
        syntax.sort_by_key(|(range, _)| (range.start, range.end));
        let mut merged: Vec<(Range<usize>, String)> = Vec::new();
        for (range, text) in syntax {
            if let Some((_, old)) = merged
                .last_mut()
                .filter(|(old, _)| old.is_empty() && *old == range)
            {
                old.push_str(&text);
            } else {
                merged.push((range, text));
            }
        }
        let mut source_patches = merged
            .into_iter()
            .map(|(range, syntax)| {
                Ok(SourcePatch::primary(
                    range,
                    self.state().encoding.encode_fragment(&syntax)?,
                ))
            })
            .collect::<Result<Vec<_>, DocumentError>>()?;
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let new_decoded = self.state().encoding.decode(&source.bytes())?;
        let mut candidate = build_state_from_decoded(
            source,
            new_decoded,
            Format::Html,
            self.file_format(),
            self.file_format_origin(),
            self.line_ending_evidence(),
            revision,
        )?;
        if candidate.projection.text() != self.text()
            || !candidate
                .projection
                .has_same_hard_line_structure(self.projection())
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let mut actual = candidate.projection.style_sheet().clone();
        actual.set_configuration_revision(expected.revision);
        if actual != expected {
            return Err(DocumentError::VerificationFailed.into());
        }
        if let PersistedStyleIntent::AssignCharacterStyle { range, style } = &intent {
            for (offset, grapheme) in
                self.text()[range.start().offset()..range.end().offset()].grapheme_indices(true)
            {
                if grapheme == "\n" {
                    continue;
                }
                let offset = range.start().offset() + offset;
                if !candidate.projection.style_spans().iter().any(|span| {
                    span.range.contains(&offset)
                        && span.application == StyleApplication::Named(style.clone())
                }) {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        candidate
            .projection
            .install_unchanged_text_storage(self.projection())
            .map_err(DocumentError::FormattedTextStorage)?;
        candidate
            .projection
            .install_source_block_ids(self.projection())
            .map_err(super::block_identity_document_error)?;
        let assignment = candidate.projection.document_style().clone();
        candidate
            .projection
            .install_configuration_styles(revision, actual, assignment);
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SemanticStyle,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_rtf_named_style(
        &self,
        intent: PersistedStyleIntent,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let revision = Revision(self.next_revision);
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let before = self.projection().style_sheet();
        let mut expected = before.clone();
        let mut intent = intent;
        let mut supporting = Vec::new();
        if let PersistedStyleIntent::AssignBlockStyle { style, .. } = &mut intent {
            if StyleSheet::builtin_block(style) {
                let native = super::rtf_styles::read(&input);
                let name = if let Some(level) = style.0.strip_prefix("Heading") {
                    format!("Heading {level}")
                } else {
                    format!("List Level {}", style.0.strip_prefix("List").unwrap())
                };
                let matches = native
                    .entries
                    .iter()
                    .filter(|entry| {
                        !entry.character
                            && before
                                .block_style_metadata(&entry.id)
                                .is_some_and(|metadata| {
                                    metadata.display_name.eq_ignore_ascii_case(&name)
                                })
                    })
                    .collect::<Vec<_>>();
                if matches.len() > 1 {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                if let Some(entry) = matches.first() {
                    *style = entry.id.clone();
                } else {
                    let handle = native
                        .entries
                        .iter()
                        .filter(|entry| !entry.character)
                        .map(|entry| entry.handle)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or(DocumentError::UnsupportedFormatting)?;
                    let mut definition = before
                        .block_style(style)
                        .ok_or(DocumentError::UnsupportedFormatting)?
                        .clone();
                    definition.id = StyleId(format!("RtfP{handle}"));
                    definition.next_paragraph_style = Some("Paragraph".into());
                    *style = definition.id.clone();
                    expected.apply_source_edit(
                        &StyleDefinitionEdit::InsertBlock {
                            style: definition,
                            metadata: super::StyleDefinitionMetadata {
                                display_name: name,
                                origin: StyleDefinitionOrigin::SourceBacked,
                            },
                        },
                        StyleSheetRevision(revision.0 + 1),
                        false,
                    )?;
                    supporting = super::rtf_styles::definition_patches(&input, before, &expected)?;
                }
            }
        }
        let mut syntax = match &intent {
            PersistedStyleIntent::EditStyleDefinition { edit, .. } => {
                expected.rebase_source_references_for_delete(
                    edit,
                    StyleSheetRevision(revision.0 + 1),
                )?;
                if !expected.apply_source_edit(edit, StyleSheetRevision(revision.0 + 1), false)? {
                    return Ok(self.no_op_prepared());
                }
                let mut patches = super::rtf_styles::definition_patches(&input, before, &expected)?;
                if let StyleDefinitionEdit::DeleteBlock(id) = edit {
                    patches.extend(super::rtf_styles::remove_assignment_patches(
                        &input,
                        id,
                        false,
                        Some(&before.base_paragraph),
                    )?);
                }
                if let StyleDefinitionEdit::DeleteCharacter(id) = edit {
                    patches.extend(super::rtf_styles::remove_assignment_patches(
                        &input,
                        id,
                        true,
                        Some(&before.base_character),
                    )?);
                }
                patches
            }
            PersistedStyleIntent::AssignCharacterStyle { range, style } => {
                self.validate_style_text_range(*range)?;
                if expected.character_style(style).is_none() {
                    return Err(StyleError::UnknownStyle(style.clone()).into());
                }
                super::rtf_styles::character_assignment_patches(
                    &input,
                    self.projection(),
                    &(range.start().offset()..range.end().offset()),
                    style,
                )?
            }
            PersistedStyleIntent::AssignBlockStyle {
                target: StyleBlockTarget::Paragraphs(range),
                style,
            } => {
                self.validate_style_text_range(*range)?;
                if expected.block_style(style).is_none() {
                    return Err(StyleError::UnknownStyle(style.clone()).into());
                }
                let start = range.start().offset();
                let end = range.end().offset();
                let sources = self
                    .projection()
                    .blocks()
                    .iter()
                    .filter(|block| {
                        if start == end {
                            block.range.start <= start && start <= block.range.end
                        } else {
                            block.range.start < end && start < block.range.end
                        }
                    })
                    .map(|block| {
                        self.projection()
                            .source_range(block.range.clone())
                            .ok_or(DocumentError::AmbiguousProjection)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                super::rtf_styles::assignment_patches(&input, &sources, style, false)?
            }
            _ => return Err(DocumentError::UnsupportedFormatting.into()),
        };
        supporting.extend(syntax);
        supporting.sort_by_key(|(range, _)| (range.start, range.end));
        syntax = Vec::new();
        for (range, value) in supporting {
            if let Some((_, previous)) = syntax
                .last_mut()
                .filter(|(previous, _)| previous.is_empty() && *previous == range)
            {
                previous.push_str(&value);
            } else {
                syntax.push((range, value));
            }
        }
        let mut source_patches = syntax
            .into_iter()
            .map(|(range, syntax)| {
                self.state()
                    .encoding
                    .encode_fragment(&syntax)
                    .map(|replacement| SourcePatch::primary(range, replacement))
            })
            .collect::<Result<Vec<_>, _>>()?;
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let decoded = self.state().encoding.decode(&source.bytes())?;
        let mut candidate = build_state_from_decoded(
            source,
            decoded,
            Format::Rtf,
            self.file_format(),
            self.file_format_origin(),
            self.line_ending_evidence(),
            revision,
        )?;
        if candidate.projection.text() != self.text()
            || !candidate
                .projection
                .has_same_hard_line_structure(self.projection())
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let mut actual = candidate.projection.style_sheet().clone();
        actual.set_configuration_revision(expected.revision);
        if actual != expected {
            return Err(DocumentError::VerificationFailed.into());
        }
        if let PersistedStyleIntent::AssignCharacterStyle { range, style } = &intent {
            let range = range.start().offset()..range.end().offset();
            let mut boundaries = BTreeSet::from([0, range.start, range.end]);
            for span in self
                .projection()
                .style_spans()
                .iter()
                .chain(candidate.projection.style_spans().iter())
            {
                boundaries.extend([span.range.start, span.range.end]);
            }
            for span in self
                .projection()
                .provenance()
                .iter()
                .filter(|span| span.is_synthetic())
            {
                boundaries.extend([span.formatted.start, span.formatted.end]);
            }
            for at in boundaries {
                if at >= self.text().len() || self.text().as_bytes()[at] == b'\n' {
                    continue;
                }
                let old = self.projection().style_spans_for_region(&(at..at + 1));
                let new = candidate.projection.style_spans_for_region(&(at..at + 1));
                let old_named = old
                    .iter()
                    .filter_map(|span| {
                        if let StyleApplication::Named(id) = &span.application {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .last();
                let new_named = new
                    .iter()
                    .filter_map(|span| {
                        if let StyleApplication::Named(id) = &span.application {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .last();
                let synthetic = self
                    .projection()
                    .provenance_for_region(&(at..at + 1))
                    .iter()
                    .any(|span| span.is_synthetic());
                let expected = if range.contains(&at) && !synthetic {
                    Some(style)
                } else {
                    old_named
                };
                if new_named != expected {
                    return Err(DocumentError::VerificationFailed.into());
                }
                let direct = |spans: &Vec<super::StyleSpan>| {
                    let mut properties = CharacterProperties::default();
                    for span in spans {
                        if let StyleApplication::Direct(value) = &span.application {
                            super::rich_text::overlay(&mut properties, value);
                        }
                    }
                    properties
                };
                if direct(&old) != direct(&new) {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        candidate
            .projection
            .install_unchanged_text_storage(self.projection())
            .map_err(DocumentError::FormattedTextStorage)?;
        candidate
            .projection
            .install_source_block_ids(self.projection())
            .map_err(super::block_identity_document_error)?;
        let assignment = candidate.projection.document_style().clone();
        candidate
            .projection
            .install_configuration_styles(revision, actual, assignment);
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SemanticStyle,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
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
        let mut configured_projection = self.projection().clone();

        let changed = match &intent {
            ConfigurationStyleIntent::EditDefinition(edit) => {
                let has_assignment = if edit.is_block() {
                    self.projection()
                        .has_block_style_assignment(edit.style_id())
                } else {
                    self.projection()
                        .has_character_style_assignment(edit.style_id())
                };
                let deleting = matches!(
                    edit,
                    StyleDefinitionEdit::DeleteBlock(_) | StyleDefinitionEdit::DeleteCharacter(_)
                );
                if deleting {
                    style_sheet.rebase_source_references_for_delete(edit, style_sheet_revision)?;
                    configured_projection.reassign_deleted_style(edit.style_id(), edit.is_block());
                    if document_style.style == *edit.style_id() {
                        document_style.style = style_sheet.base_document.clone();
                    }
                }
                style_sheet.apply_configuration_edit(
                    edit,
                    style_sheet_revision,
                    has_assignment && !deleting,
                )?
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

        validate_projection_style_configuration(
            &configured_projection,
            &style_sheet,
            &document_style,
        )?;
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
        candidate.projection = configured_projection;
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
        edits: Vec<TextEdit>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_text_edits_with_patches(edits, None)
    }

    fn prepare_text_edits_with_patches(
        &self,
        mut edits: Vec<TextEdit>,
        explicit_source_patches: Option<Vec<SourcePatch>>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if edits.is_empty() {
            if let Some(patches) = explicit_source_patches {
                return self.prepare_source_only_patches(patches);
            }
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
            if let Some(patches) = explicit_source_patches {
                return self.prepare_source_only_patches(patches);
            }
            return Ok(self.no_op_prepared());
        }

        let mut source_patches = Vec::with_capacity(edits.len());
        if let Some(patches) = explicit_source_patches {
            source_patches = patches;
        } else {
            for edit in &edits {
                if matches!(self.format(), Format::Html | Format::Rtf) {
                    if self.format() == Format::Rtf
                        && !edit.range.is_empty()
                        && edit.replacement.is_empty()
                    {
                        let decoded = self.state().encoding.decode(&self.source_bytes())?;
                        let input = normalize(&decoded, self.file_format());
                        if let Some(patches) =
                            super::rtf_structure::deletion_patches(self, &input, &edit.range)?
                        {
                            for (range, syntax) in patches {
                                source_patches.push(SourcePatch::primary(
                                    range,
                                    self.state().encoding.encode_fragment(&syntax)?,
                                ));
                            }
                            continue;
                        }
                    }
                    if self.format() == Format::Html
                        && !edit.range.is_empty()
                        && edit.replacement.is_empty()
                        && (self
                            .projection()
                            .text_tree()
                            .slice(edit.range.clone())
                            .map_err(DocumentError::FormattedTextStorage)?
                            .contains('\n')
                            || self.projection().blocks_for_region(&edit.range).iter().any(
                                |block| {
                                    edit.range.start <= block.range.start
                                        && matches!(block.kind, super::BlockKind::ListItem { .. })
                                },
                            ))
                    {
                        let decoded = self.state().encoding.decode(&self.source_bytes())?;
                        let input = normalize(&decoded, self.file_format());
                        if let Some(patches) =
                            super::html_paragraph::deletion_patches(self, &input, &edit.range)?
                        {
                            for (range, syntax) in patches {
                                source_patches.push(SourcePatch::primary(
                                    range,
                                    self.state().encoding.encode_fragment(&syntax)?,
                                ));
                            }
                            continue;
                        }
                    }
                    if self.format() == Format::Html
                        && self
                            .projection()
                            .text_tree()
                            .slice(edit.range.clone())
                            .map_err(DocumentError::FormattedTextStorage)?
                            .contains('\n')
                    {
                        let decoded = self.state().encoding.decode(&self.source_bytes())?;
                        let input = normalize(&decoded, self.file_format());
                        if let Some(patches) =
                            super::html_paragraph::join_patches(self, &input, edit, &edits)?
                        {
                            for (range, syntax) in patches {
                                source_patches.push(SourcePatch::primary(
                                    range,
                                    self.state().encoding.encode_fragment(&syntax)?,
                                ));
                            }
                            continue;
                        }
                    }
                    let runs = super::rich_text::text_source_runs(self, &edit.range)?;
                    let syntax = if self.format() == Format::Html {
                        super::rich_text::escape_html_source_edit(
                            self,
                            runs[0].start,
                            &edit.replacement,
                        )?
                    } else if self.source_byte_len() == 0 {
                        format!("{{\\rtf1\\ansi {}}}", super::rtf::escape(&edit.replacement))
                    } else {
                        super::rtf::escape(&edit.replacement)
                    };
                    let replacement = self.encoding().encode_fragment(&syntax)?;
                    for (index, range) in runs.into_iter().enumerate() {
                        let value = if index == 0 {
                            replacement.clone()
                        } else if self.format() == Format::Html {
                            self.encoding().encode_fragment(
                                &super::rich_text::escape_html_source_edit(self, range.start, "")?,
                            )?
                        } else {
                            Vec::new()
                        };
                        source_patches.push(SourcePatch::primary(range, value));
                    }
                    continue;
                }
                let source_range = if matches!(self.format(), Format::Html | Format::Rtf) {
                    super::rich_text::text_source_range(self, &edit.range)?
                } else if edit.range.is_empty() {
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
                    if let Some(patches) = self
                        .markdown_line_local_text_rewrite_patches(&edit.range, &edit.replacement)?
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
                    Format::PlainText | Format::MarkdownSource => edit.replacement.clone(),
                    Format::Html => {
                        super::rich_text::escape_html_text(&edit.replacement, self.encoding())
                    }
                    Format::Rtf => {
                        if self.state().source.len() == 0 {
                            format!("{{\\rtf1\\ansi {}}}", super::rtf::escape(&edit.replacement))
                        } else {
                            super::rtf::escape(&edit.replacement)
                        }
                    }
                    Format::Markdown if in_code && !edit.replacement.contains('`') => {
                        edit.replacement.clone()
                    }
                    Format::Markdown => escape_markdown_insert(&edit.replacement),
                };
                let syntax = spell_logical_breaks(&syntax, self.state().file_format);
                let replacement = self.state().encoding.encode_fragment(&syntax)?;
                source_patches.push(SourcePatch::primary(source_range, replacement));
            }
        }
        if self.format() == Format::Html {
            for edit in &edits {
                if !edit.replacement.is_empty() || edit.range.is_empty() {
                    continue;
                }
                let line = self
                    .projection()
                    .hard_line_at_offset(edit.range.start)
                    .and_then(|line| self.projection().hard_line_range(line));
                let Some(line) = line.filter(|line| edit.range.end <= line.end) else {
                    continue;
                };
                let left = if edit.range.start > line.start {
                    self.projection()
                        .text_tree()
                        .slice(edit.range.start - 1..edit.range.start)
                        .ok()
                } else {
                    None
                };
                let right = if edit.range.end < line.end {
                    self.projection()
                        .text_tree()
                        .slice(edit.range.end..edit.range.end + 1)
                        .ok()
                } else {
                    None
                };
                let retain = if right.as_deref() == Some(" ")
                    && (left.is_none() || left.as_deref() == Some(" "))
                {
                    Some(edit.range.end..edit.range.end + 1)
                } else if left.as_deref() == Some(" ") && right.is_none() {
                    Some(edit.range.start - 1..edit.range.start)
                } else {
                    None
                };
                if let Some(range) = retain.filter(|range| {
                    !edits
                        .iter()
                        .any(|edit| edit.range.start < range.end && range.start < edit.range.end)
                }) {
                    let source_range =
                        super::rich_text::editable_source_range(self.projection(), &range)?;
                    source_patches.push(SourcePatch::primary(
                        source_range,
                        self.state()
                            .encoding
                            .encode_fragment(&super::html::escape(" "))?,
                    ));
                }
            }
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
                .install_reconciled_format_block_ids(
                    self.format(),
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

        if matches!(self.format(), Format::Html | Format::Rtf)
            && edits.iter().all(|edit| {
                !edit.payload.text().contains('\n') && edit.payload.break_offsets().is_empty()
            })
        {
            let mut patches = Vec::new();
            let mut text_edits = Vec::new();
            for edit in edits {
                let mut runs = super::rich_text::text_source_runs(self, &edit.range)?;
                if edit.range.is_empty() && self.projection().text_tree().byte_len() != 0 {
                    if edit.boundary_affinity == Some(BoundaryAffinity::Upstream) {
                        let at = self
                            .projection()
                            .source_insertion_point(edit.range.start, false)
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        runs = vec![at..at];
                    }
                }
                let syntax = if self.format() == Format::Html {
                    super::rich_text::escape_html_source_edit(
                        self,
                        runs[0].start,
                        edit.payload.text(),
                    )?
                } else if self.source_byte_len() == 0 {
                    format!(
                        "{{\\rtf1\\ansi {}}}",
                        super::rtf::escape(edit.payload.text())
                    )
                } else {
                    super::rtf::escape(edit.payload.text())
                };
                let replacement = self.encoding().encode_fragment(&syntax)?;
                for (index, range) in runs.into_iter().enumerate() {
                    let value = if index == 0 {
                        replacement.clone()
                    } else if self.format() == Format::Html {
                        self.encoding().encode_fragment(
                            &super::rich_text::escape_html_source_edit(self, range.start, "")?,
                        )?
                    } else {
                        Vec::new()
                    };
                    patches.push(SourcePatch::primary(range, value));
                }
                text_edits.push(TextEdit::new(edit.range, edit.payload.text()));
            }
            return self.prepare_text_edits_with_patches(text_edits, Some(patches));
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
            if matches!(self.format(), Format::Html | Format::Rtf) {
                let runs = super::rich_text::text_source_runs(self, &edit.range)?;
                let syntax = if self.format() == Format::Html {
                    super::rich_text::escape_html_source_edit(
                        self,
                        runs[0].start,
                        edit.payload.text(),
                    )?
                } else if self.source_byte_len() == 0 {
                    format!(
                        "{{\\rtf1\\ansi {}}}",
                        super::rtf::escape(edit.payload.text())
                    )
                } else {
                    super::rtf::escape(edit.payload.text())
                };
                let replacement = self.encoding().encode_fragment(&syntax)?;
                for (index, range) in runs.into_iter().enumerate() {
                    let value = if index == 0 {
                        replacement.clone()
                    } else if self.format() == Format::Html {
                        self.encoding().encode_fragment(
                            &super::rich_text::escape_html_source_edit(self, range.start, "")?,
                        )?
                    } else {
                        Vec::new()
                    };
                    source_patches.push(SourcePatch::primary(range, value));
                }
                continue;
            }
            let source_range = if matches!(self.format(), Format::Html | Format::Rtf) {
                super::rich_text::text_source_range(self, &edit.range)?
            } else if edit.range.is_empty() {
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
            .install_reconciled_format_block_ids(
                self.format(),
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
                .install_reconciled_format_block_ids(
                    self.format(),
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

    fn prepare_source_only_patches(
        &self,
        mut patches: Vec<SourcePatch>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if patches.is_empty() {
            return Ok(self.no_op_prepared());
        }
        validate_source_patches(&mut patches)?;
        let source = apply_source_patches(&self.state().source, &patches)?;
        let revision = Revision(self.next_revision);
        let candidate = self.build_verified_candidate(
            source,
            self.file_format(),
            self.state().file_format_origin,
            revision,
            CandidateVerification {
                expected_text: self.text().to_owned(),
                expected_hard_breaks: None,
                mismatch_error: DocumentError::VerificationFailed,
            },
        )?;
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SemanticStyle,
                source_patches: patches,
                formatted_splices: Vec::new(),
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_rich_clear_character(
        &self,
        range: Range<usize>,
        properties: BTreeSet<StyleProperty>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if range.is_empty() || properties.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let syntax = if self.format() == Format::Html {
            super::html_direct::clear_character_patches(
                &input,
                self.projection(),
                range.clone(),
                &properties,
            )?
        } else {
            super::rtf_direct::clear_character_patches(
                &input,
                self.projection(),
                range.clone(),
                &properties,
            )?
        };
        let patches = syntax
            .into_iter()
            .map(|(range, syntax)| {
                self.encoding()
                    .encode_fragment(&syntax)
                    .map(|bytes| SourcePatch::primary(range, bytes))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared = self.prepare_source_only_patches(patches)?;
        let after = match &prepared.publication {
            PreparedPublication::State(state) => &state.projection,
            _ => self.projection(),
        };
        if !super::rich_text::character_clear_verified(
            self.projection(),
            after,
            &range,
            &properties,
        ) {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    fn prepare_rich_block_properties(
        &self,
        intent: PersistedStyleIntent,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let (target, set, clear) = match intent {
            PersistedStyleIntent::SetDirectBlockProperties { target, properties } => {
                (target, properties, BTreeSet::new())
            }
            PersistedStyleIntent::ClearDirectBlockProperties { target, properties } => {
                (target, BlockProperties::default(), properties)
            }
            _ => unreachable!(),
        };
        let StyleBlockTarget::Paragraphs(range) = target else {
            return Err(DocumentError::UnsupportedFormatting.into());
        };
        self.validate_style_text_range(range)?;
        let range = range.start().offset()..range.end().offset();
        let mut desired = Vec::new();
        let mut expected = Vec::new();
        for block in self.projection().blocks() {
            let mut properties = block.direct_paragraph.clone();
            let selected = if range.is_empty() {
                block.range.start <= range.start && range.start <= block.range.end
            } else {
                block.range.start < range.end && range.start < block.range.end
            };
            if selected {
                super::rich_text::overlay_block(&mut properties, &set);
                for property in &clear {
                    super::style::clear_block_property(
                        &"Direct".into(),
                        &mut properties,
                        *property,
                    )?;
                }
                if properties != block.direct_paragraph {
                    let source = self
                        .projection()
                        .source_range(block.range.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    desired.push((source, properties.clone()));
                }
            }
            expected.push(properties);
        }
        if desired.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let syntax = if self.format() == Format::Html {
            super::html_direct::paragraph_patches(&input, &desired)?
        } else {
            self.rtf_paragraph_property_patches(&input, &expected, &clear)?
        };
        let patches = syntax
            .into_iter()
            .map(|(range, syntax)| {
                self.encoding()
                    .encode_fragment(&syntax)
                    .map(|bytes| SourcePatch::primary(range, bytes))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared = self.prepare_source_only_patches(patches)?;
        let after = match &prepared.publication {
            PreparedPublication::State(state) => &state.projection,
            _ => self.projection(),
        };
        if after.blocks().len() != expected.len()
            || after
                .blocks()
                .iter()
                .zip(expected)
                .any(|(block, expected)| block.direct_paragraph != expected)
            || after
                .blocks()
                .iter()
                .zip(self.projection().blocks())
                .any(|(new, old)| new.kind != old.kind || new.style != old.style)
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        if after.style_spans() != self.projection().style_spans() {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    fn rtf_paragraph_property_patches(
        &self,
        input: &super::line_endings::NormalizedText,
        expected: &[BlockProperties],
        clear: &BTreeSet<StyleProperty>,
    ) -> Result<Vec<(Range<usize>, String)>, ModelTransactionError> {
        let native = super::rtf_styles::read(input);
        let mut patches = Vec::new();
        for (block, properties) in self.projection().blocks().iter().zip(expected) {
            if properties == &block.direct_paragraph {
                continue;
            }
            // RTF readers attach paragraph properties to the entire paragraph,
            // including its terminator. A delta only around \par can leave
            // character-attributed readers with unchanged properties at its
            // first character, so cover every visible source run.
            let extent = block.range.start
                ..(block.range.end + usize::from(block.range.end < self.text().len()));
            let mut runs: Vec<(Range<usize>, CharacterProperties, Option<StyleId>)> = Vec::new();
            for span in self.projection().provenance_for_region(&extent) {
                if span.formatted.is_empty() || span.source.is_empty() {
                    continue;
                }
                let mut direct = CharacterProperties::default();
                let mut named = None;
                for style in self.projection().style_spans_for_region(&span.formatted) {
                    if !style.range.contains(&span.formatted.start) {
                        continue;
                    }
                    match style.application {
                        StyleApplication::Direct(properties) => {
                            super::rich_text::overlay(&mut direct, &properties)
                        }
                        StyleApplication::Named(id) => named = Some(id),
                        _ => {}
                    }
                }
                if let Some((source, _, _)) =
                    runs.last_mut()
                        .filter(|(source, previous, previous_named)| {
                            source.end == span.source.start
                                && previous == &direct
                                && previous_named == &named
                        })
                {
                    source.end = span.source.end;
                } else {
                    runs.push((span.source, direct, named));
                }
            }
            if runs.is_empty() {
                return Err(DocumentError::AmbiguousProjection.into());
            }
            for (source, direct, named) in runs {
                let mut controls = String::new();
                if !clear.is_empty() {
                    controls.push_str("\\pard");
                    if let Some(handle) = super::rtf_styles::handle(&native, &block.style, false) {
                        controls.push_str(&format!("\\s{handle}"));
                        if let Some(id) = named {
                            let character_handle = super::rtf_styles::handle(&native, &id, true)
                                .ok_or(DocumentError::UnsupportedFormatting)?;
                            controls.push_str(&format!("\\cs{character_handle}"));
                        }
                        if direct != CharacterProperties::default() {
                            let mut authored = super::rtf::character_patches(
                                input,
                                &(usize::MAX - 1..usize::MAX),
                                &direct,
                            )?;
                            authored.pop();
                            let (_, prefix) =
                                authored.pop().ok_or(DocumentError::UnsupportedFormatting)?;
                            controls.push_str(prefix.trim_start_matches('{').trim_end());
                            for patch in authored {
                                if !patches.contains(&patch) {
                                    patches.push(patch);
                                }
                            }
                        }
                    }
                    if let super::BlockKind::ListItem {
                        ordered, ordinal, ..
                    } = block.kind
                    {
                        if let Some((handle, level)) =
                            super::rtf_direct::modern_list_at(input, source.start)
                        {
                            controls.push_str(&format!("\\ls{handle}\\ilvl{level}"));
                        } else {
                            controls.push_str(&if ordered {
                                format!("{{\\*\\pn\\pndec\\pnstart{ordinal}}}")
                            } else {
                                "{\\*\\pn\\pnlvlblt}".to_owned()
                            });
                        }
                    }
                }
                controls.push_str(&super::rtf_styles::paragraph_controls(properties)?);
                patches.push((source.start..source.start, format!("{{{controls} ")));
                patches.push((source.end..source.end, "}".to_owned()));
            }
        }
        Ok(patches)
    }

    fn prepare_markdown_paragraph_style(
        &self,
        range: Range<usize>,
        style: StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if matches!(self.format(), Format::Html | Format::Rtf) {
            let range = TextRange::new(self.text_point(range.start)?, self.text_point(range.end)?)?;
            let intention = PersistedStyleIntent::AssignBlockStyle {
                target: StyleBlockTarget::Paragraphs(range),
                style,
            };
            return if self.format() == Format::Html {
                self.prepare_html_named_style(intention)
            } else {
                self.prepare_rtf_named_style(intention)
            };
        }
        self.validate_range(&range)?;
        if !matches!(self.format(), Format::Markdown | Format::MarkdownSource) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let level = if style.0 == "Paragraph" {
            0
        } else {
            style
                .0
                .strip_prefix("Heading")
                .and_then(|level| level.parse::<u8>().ok())
                .filter(|level| (1..=6).contains(level))
                .ok_or(DocumentError::UnsupportedFormatting)?
        };
        let first = self
            .projection()
            .hard_line_at_offset(range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let prefix = if level == 0 {
            String::new()
        } else {
            format!("{} ", "#".repeat(usize::from(level)))
        };
        let mut edits = Vec::new();
        let mut patches = Vec::new();
        for index in first..self.projection().hard_line_count() {
            let line = self
                .projection()
                .hard_line_range(index)
                .ok_or(DocumentError::VerificationFailed)?;
            if index != first && line.start >= range.end {
                break;
            }
            let source_line = self
                .state()
                .source_hard_lines
                .get(index)
                .ok_or(DocumentError::VerificationFailed)?;
            let bytes = self
                .state()
                .source
                .bytes_in(source_line.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = self
                .state()
                .encoding
                .decode_region(&bytes, source_line.start)?;
            let (old_prefix, _) =
                super::projection::markdown_block_prefix(&decoded.text, 0, decoded.text.len());
            if decoded.text[..old_prefix] == prefix {
                continue;
            }
            let removed_bytes = self
                .state()
                .encoding
                .encode_fragment(&decoded.text[..old_prefix])?
                .len();
            patches.push(SourcePatch::primary(
                source_line.start..source_line.start + removed_bytes,
                self.state().encoding.encode_fragment(&prefix)?,
            ));
            let visible = self
                .projection()
                .text_tree()
                .slice(line.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let (visible_prefix, kind) =
                super::projection::markdown_block_prefix(&visible, 0, visible.len());
            let remove = if self.format() == Format::MarkdownSource
                || matches!(kind, super::BlockKind::ListItem { .. })
            {
                visible_prefix
            } else {
                0
            };
            let replacement = if self.format() == Format::MarkdownSource {
                prefix.clone()
            } else {
                String::new()
            };
            edits.push(TextEdit::new(line.start..line.start + remove, replacement));
        }
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    fn prepare_list_style(
        &self,
        range: Range<usize>,
        style: Option<super::ListStyle>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if matches!(self.format(), Format::Html | Format::Rtf) {
            return self.prepare_rich_list_style(range, style);
        }
        if !matches!(
            self.format(),
            Format::PlainText | Format::Markdown | Format::MarkdownSource
        ) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let first = self
            .projection()
            .hard_line_at_offset(range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let mut edits = Vec::new();
        let mut patches = Vec::new();
        let mut ordinal = 1usize;
        for index in first..self.projection().hard_line_count() {
            let line = self
                .projection()
                .hard_line_range(index)
                .ok_or(DocumentError::VerificationFailed)?;
            if index != first && line.start >= range.end {
                break;
            }
            let visible = self
                .projection()
                .text_tree()
                .slice(line.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let (visible_prefix, visible_kind) =
                super::projection::markdown_block_prefix(&visible, 0, visible.len());
            let indent = visible.bytes().take_while(|byte| *byte == b' ').count();
            let remove_visible = if matches!(visible_kind, super::BlockKind::ListItem { .. })
                || (style.is_some()
                    && self.format() == Format::MarkdownSource
                    && matches!(visible_kind, super::BlockKind::Heading(_)))
            {
                visible_prefix
            } else if style.is_some() {
                indent
            } else {
                0
            };
            let prefix = match style {
                Some(super::ListStyle::Bullet) => format!("{}- ", " ".repeat(indent)),
                Some(super::ListStyle::Numbered) => format!("{}{ordinal}. ", " ".repeat(indent)),
                None => {
                    if remove_visible > 0 {
                        " ".repeat(indent)
                    } else {
                        String::new()
                    }
                }
            };
            ordinal += 1;
            if visible[..remove_visible] == prefix {
                continue;
            }
            let source_line = self
                .state()
                .source_hard_lines
                .get(index)
                .ok_or(DocumentError::VerificationFailed)?;
            let bytes = self
                .state()
                .source
                .bytes_in(source_line.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = self
                .state()
                .encoding
                .decode_region(&bytes, source_line.start)?;
            let (source_prefix, source_kind) =
                super::projection::markdown_block_prefix(&decoded.text, 0, decoded.text.len());
            let remove_source = if matches!(source_kind, super::BlockKind::ListItem { .. })
                || (style.is_some()
                    && self.format() != Format::PlainText
                    && matches!(source_kind, super::BlockKind::Heading(_)))
            {
                source_prefix
            } else if style.is_some() {
                decoded
                    .text
                    .bytes()
                    .take_while(|byte| *byte == b' ')
                    .count()
            } else {
                0
            };
            let removed_bytes = self
                .state()
                .encoding
                .encode_fragment(&decoded.text[..remove_source])?
                .len();
            patches.push(SourcePatch::primary(
                source_line.start..source_line.start + removed_bytes,
                self.state().encoding.encode_fragment(&prefix)?,
            ));
            edits.push(TextEdit::new(
                line.start..line.start + remove_visible,
                prefix,
            ));
        }
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    fn prepare_rich_list_style(
        &self,
        range: Range<usize>,
        style: Option<super::ListStyle>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut targets = Vec::new();
        let mut edits = Vec::new();
        let mut blocks = self
            .projection()
            .blocks_for_region(&range)
            .into_iter()
            .filter(|block| {
                if range.is_empty() {
                    block.range.start <= range.start && range.start <= block.range.end
                } else {
                    block.range.start < range.end && range.start < block.range.end
                }
            })
            .collect::<Vec<_>>();
        let selected = blocks.iter().map(|block| block.id).collect::<BTreeSet<_>>();
        let mut containers = std::collections::BTreeMap::new();
        for list in self.projection().list_structure().lists {
            for item in list.items {
                if !item.paragraph_ids.iter().any(|id| selected.contains(id)) {
                    continue;
                }
                let paragraphs = self
                    .projection()
                    .blocks()
                    .iter()
                    .filter(|block| item.paragraph_ids.contains(&block.id))
                    .collect::<Vec<_>>();
                if let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) {
                    let mut whole = (*first).clone();
                    whole.range.end = last.range.end;
                    blocks.retain(|block| !item.paragraph_ids.contains(&block.id));
                    containers.insert(whole.id, list.id.0);
                    blocks.push(whole);
                }
            }
        }
        blocks.sort_by_key(|block| block.range.start);
        let mut ordinals = std::collections::BTreeMap::new();
        for block in blocks {
            let next = ordinals
                .entry(containers.get(&block.id).copied().unwrap_or(0))
                .or_insert(1u64);
            let ordinal = *next;
            *next += 1;
            let line = block.range.clone();
            let kind = &block.kind;
            let (old_marker, old_style) = match kind {
                super::BlockKind::ListItem {
                    ordered, ordinal, ..
                } => (
                    if *ordered {
                        format!("{ordinal}. ")
                    } else {
                        "• ".to_owned()
                    },
                    Some(if *ordered {
                        super::ListStyle::Numbered
                    } else {
                        super::ListStyle::Bullet
                    }),
                ),
                _ => (String::new(), None),
            };
            if old_style == style {
                continue;
            }
            let new_marker = match style {
                Some(super::ListStyle::Bullet) => "• ".to_owned(),
                Some(super::ListStyle::Numbered) => format!("{ordinal}. "),
                None => String::new(),
            };
            let body = line.start + old_marker.len()..line.end;
            let spans = self
                .projection()
                .provenance()
                .iter()
                .filter(|span| span.formatted.start < body.end && body.start < span.formatted.end)
                .collect::<Vec<_>>();
            let source = if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
                first.source.start..last.source.end
            } else {
                let marker = self
                    .projection()
                    .provenance()
                    .iter()
                    .find(|span| span.formatted.start == line.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                marker.source.end..marker.source.end
            };
            let original_ordinal = if let super::BlockKind::ListItem { ordinal, .. } = kind {
                Some(*ordinal)
            } else {
                None
            };
            targets.push((source, style, ordinal, original_ordinal));
            edits.push(TextEdit::new(
                line.start..line.start + old_marker.len(),
                new_marker,
            ));
        }
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let bytes = self.source_bytes();
        let decoded = self.state().encoding.decode(&bytes)?;
        let normalized = normalize(&decoded, self.file_format());
        let raw_patches = if self.format() == Format::Html {
            super::html::list_patches(&normalized, &targets)?
        } else {
            super::rtf::list_patches(&normalized, &targets, self.projection())?
        };
        let patches = raw_patches
            .into_iter()
            .map(|(range, syntax)| {
                self.state()
                    .encoding
                    .encode_fragment(&syntax)
                    .map(|replacement| SourcePatch::primary(range, replacement))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    fn prepare_rich_list_enter(
        &self,
        at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let edit = self
            .list_enter_edit(at)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if !edit.range.is_empty() {
            return self.prepare_rich_list_style(edit.range, None);
        }
        let block = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| block.range.start <= at && at <= block.range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        let super::BlockKind::ListItem {
            ordered, ordinal, ..
        } = block.kind
        else {
            return if self.format() == Format::Html {
                self.prepare_html_paragraph_enter(at, &block)
            } else {
                self.prepare_rtf_next_paragraph_enter(at, &block)
            };
        };
        let source_at = self
            .projection()
            .source_insertion_point(at, false)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.source_bytes();
        let decoded = self.state().encoding.decode(&bytes)?;
        let input = normalize(&decoded, self.file_format());
        let (range, syntax) = if self.format() == Format::Html {
            super::html::list_enter_patch(
                &input,
                source_at,
                ordered.then_some(ordinal.saturating_add(1)),
            )?
        } else {
            (
                source_at..source_at,
                if ordered {
                    format!(
                        "\\par\\ls0 {{\\*\\pn\\pnlvlbody\\pndec\\pnstart{}{{\\pntxta .}}}}",
                        ordinal.saturating_add(1)
                    )
                } else {
                    "\\par\\ls0 {\\*\\pn\\pnlvlblt{\\pntxtb\\bullet}}".to_owned()
                },
            )
        };
        let mut edits = vec![edit];
        let mut source_targets = Vec::new();
        let mut legacy_patches = Vec::new();
        if ordered {
            let structure = self.projection().list_structure();
            if let Some((list, index)) = structure.lists.iter().find_map(|list| {
                list.items
                    .iter()
                    .position(|item| item.paragraph_ids.contains(&block.id))
                    .map(|index| (list, index))
            }) {
                for item in &list.items[index + 1..] {
                    let marker = self
                        .projection()
                        .provenance_for_region(&item.marker_range)
                        .into_iter()
                        .find(|span| span.is_synthetic())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    if self.format() == Format::Html
                        && super::html::list_item_has_explicit_value(&input, marker.source.start)
                    {
                        break;
                    }
                    edits.push(TextEdit::new(
                        item.marker_range.clone(),
                        format!("{}. ", item.ordinal.saturating_add(1)),
                    ));
                    if self.format() == Format::Rtf {
                        if let Some(patches) = super::rtf_structure::renumber_legacy_patches(
                            &input,
                            marker.source.start,
                            item.ordinal.saturating_add(1),
                        ) {
                            legacy_patches.extend(patches);
                            continue;
                        }
                        let spans = item
                            .paragraph_ids
                            .iter()
                            .flat_map(|id| {
                                self.projection()
                                    .blocks()
                                    .iter()
                                    .filter(move |block| block.id == *id)
                            })
                            .flat_map(|block| self.projection().provenance_for_region(&block.range))
                            .filter(|span| !span.source.is_empty())
                            .collect::<Vec<_>>();
                        let source =
                            if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
                                first.source.start..last.source.end
                            } else {
                                marker.source.start..marker.source.start
                            };
                        source_targets.push((
                            source,
                            Some(super::ListStyle::Numbered),
                            item.ordinal.saturating_add(1),
                            Some(item.ordinal),
                        ));
                    }
                }
            }
        }
        let mut syntax_patches = vec![(range, syntax)];
        if self.format() == Format::Rtf {
            if let Some(selector) = super::rtf_structure::modern_selector_at(&input, source_at) {
                if let Some(boundary) = self
                    .projection()
                    .provenance_for_region(&(block.range.end..block.range.end + 1))
                    .into_iter()
                    .find(|span| !span.source.is_empty())
                {
                    syntax_patches.push((boundary.source.end..boundary.source.end, selector));
                }
            }
        }
        syntax_patches.extend(legacy_patches);
        if !source_targets.is_empty() {
            syntax_patches.extend(super::rtf::list_patches(
                &input,
                &source_targets,
                self.projection(),
            )?);
        }
        syntax_patches.sort_by_key(|(range, _)| (range.start, range.end));
        let mut merged: Vec<(Range<usize>, String)> = Vec::new();
        for (range, syntax) in syntax_patches {
            if let Some((_, previous)) = merged
                .last_mut()
                .filter(|(previous, _)| previous.is_empty() && *previous == range)
            {
                previous.push_str(&syntax);
            } else {
                merged.push((range, syntax));
            }
        }
        let patches = merged
            .into_iter()
            .map(|(range, syntax)| {
                self.state()
                    .encoding
                    .encode_fragment(&syntax)
                    .map(|bytes| SourcePatch::primary(range, bytes))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    fn prepare_html_paragraph_enter(
        &self,
        at: usize,
        block: &super::Block,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let sheet = self.projection().style_sheet();
        let next = if at == block.range.end {
            sheet
                .block_style(&block.style)
                .and_then(|style| style.next_paragraph_style.clone())
                .unwrap_or_else(|| block.style.clone())
        } else {
            block.style.clone()
        };
        let source_at = if self.text().is_empty() {
            super::rich_text::text_source_range(self, &(at..at))?.start
        } else {
            self.projection()
                .source_insertion_point(at, at == block.range.start)
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let spans = self
            .projection()
            .provenance_for_region(&block.range)
            .into_iter()
            .filter(|span| !span.source.is_empty())
            .collect::<Vec<_>>();
        let extent = if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
            first.source.start..last.source.end
        } else {
            source_at..source_at
        };
        if !spans
            .windows(2)
            .all(|pair| pair[0].source.end <= pair[1].source.start)
        {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let syntax =
            super::html_paragraph::enter_patches(&input, source_at, extent, block, &next, sheet)?;
        let patches = syntax
            .into_iter()
            .map(|(range, syntax)| {
                self.state()
                    .encoding
                    .encode_fragment(&syntax)
                    .map(|replacement| SourcePatch::primary(range, replacement))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared =
            self.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], Some(patches))?;
        if let PreparedPublication::State(candidate) = &prepared.publication {
            let first = candidate
                .projection
                .blocks_for_region(&(at..at))
                .into_iter()
                .find(|block| block.range.start <= at && at <= block.range.end)
                .ok_or(DocumentError::VerificationFailed)?;
            let following = candidate
                .projection
                .blocks_for_region(&(at + 1..at + 1))
                .into_iter()
                .find(|block| block.range.start == at + 1)
                .ok_or(DocumentError::VerificationFailed)?;
            if first.style != block.style
                || following.style != next
                || first.direct_paragraph != block.direct_paragraph
                || following.direct_paragraph != block.direct_paragraph
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        Ok(prepared)
    }

    fn prepare_rtf_next_paragraph_enter(
        &self,
        at: usize,
        block: &super::Block,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format() != Format::Rtf {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let next = self
            .projection()
            .style_sheet()
            .block_style(&block.style)
            .and_then(|style| style.next_paragraph_style.clone())
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let native = super::rtf_styles::read(&input);
        let number = super::rtf_styles::handle(&native, &next, false)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let source_at = self
            .projection()
            .source_insertion_point(at, false)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let sample = if at > block.range.start { at - 1 } else { at };
        let mut direct = CharacterProperties::default();
        for span in self
            .projection()
            .style_spans_for_region(&(sample..sample + 1))
        {
            if let StyleApplication::Direct(properties) = span.application {
                super::rich_text::overlay(&mut direct, &properties);
            }
        }
        let mut syntax_patches =
            super::rtf::character_patches(&input, &(usize::MAX - 1..usize::MAX), &direct)?;
        syntax_patches.pop();
        let (_, prefix) = syntax_patches
            .pop()
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let paragraph = super::rtf_styles::paragraph_controls(&block.direct_paragraph)?;
        let control = prefix.trim_start_matches('{').trim_end();
        syntax_patches.push((
            source_at..source_at,
            format!("\\par\\s{number}{control}{paragraph} "),
        ));
        let patches = syntax_patches
            .into_iter()
            .map(|(range, syntax)| {
                self.state()
                    .encoding
                    .encode_fragment(&syntax)
                    .map(|replacement| SourcePatch::primary(range, replacement))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let prepared =
            self.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], Some(patches))?;
        if let PreparedPublication::State(candidate) = &prepared.publication {
            if !candidate
                .projection
                .blocks_for_region(&(at + 1..at + 1))
                .iter()
                .any(|block| block.range.start == at + 1 && block.style == next)
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
        }
        Ok(prepared)
    }

    fn prepare_semantic_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if matches!(self.format(), Format::Html | Format::Rtf) {
            let properties = rich_semantic_properties(style, enabled)?;
            return self.prepare_rich_character_properties(
                range,
                properties,
                (!enabled).then_some(style),
            );
        }
        if self.state().format == Format::MarkdownSource {
            let edits = self.markdown_source_style_edits(&range, style, enabled)?;
            let added = edits
                .iter()
                .map(|edit| edit.replacement.len())
                .sum::<usize>();
            let prepared = self.prepare_text_edits(edits)?;
            if enabled {
                if let PreparedPublication::State(candidate) = &prepared.publication {
                    if !candidate.projection.style_spans().iter().any(|span| {
                        span.application == StyleApplication::Semantic(style)
                            && span.range == (range.start..range.end + added)
                    }) {
                        return Err(DocumentError::UnsupportedFormatting.into());
                    }
                }
            }
            return Ok(prepared);
        }
        let Some(mut source_patches) =
            self.semantic_style_source_patches(&range, style, enabled)?
        else {
            return Ok(self.no_op_prepared());
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

    /// Cheap, source-aware validation for menu and command presentation. It
    /// performs the exact local capability checks used by preparation without
    /// constructing or projecting a candidate document.
    pub(crate) fn semantic_style_edit_capability(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<(), ModelTransactionError> {
        if self.state().format == Format::MarkdownSource {
            return self
                .markdown_source_style_edits(&range, style, enabled)
                .map(|_| ());
        }
        self.semantic_style_source_patches(&range, style, enabled)
            .map(|_| ())
    }

    /// In source-visible Markdown the delimiters are ordinary editable content.
    /// Formatting actions use the same text transaction as typing those markers,
    /// including its source patches, anchor maps, and undo restoration.
    fn markdown_source_style_edits(
        &self,
        range: &Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<Vec<TextEdit>, ModelTransactionError> {
        self.validate_range(range)?;
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let containing = self.projection().style_spans().iter().find(|span| {
            span.application == StyleApplication::Semantic(style)
                && span.range.start <= range.start
                && range.end <= span.range.end
        });
        if enabled {
            if containing.is_some() {
                return Ok(Vec::new());
            }
            if self
                .projection()
                .style_spans()
                .iter()
                .any(|span| span.range.start < range.end && range.start < span.range.end)
            {
                return Err(DocumentError::OverlappingFormatting.into());
            }
            let selected = self
                .projection()
                .text_tree()
                .slice(range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if selected.contains('\n') || selected.trim().is_empty() {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let marker = markdown_style_markers(style)
                .iter()
                .find(|marker| !selected.contains(**marker))
                .copied()
                .ok_or(DocumentError::UnsupportedFormatting)?;
            return Ok(vec![
                TextEdit::new(range.start..range.start, marker),
                TextEdit::new(range.end..range.end, marker),
            ]);
        }
        let span = containing.ok_or(DocumentError::UnsupportedFormatting)?;
        let text = self
            .projection()
            .text_tree()
            .slice(span.range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let marker = markdown_style_markers(style)
            .iter()
            .find(|marker| {
                text.starts_with(**marker)
                    && text.ends_with(**marker)
                    && text.len() >= 2 * marker.len()
            })
            .ok_or(DocumentError::UnsupportedFormatting)?;
        Ok(vec![
            TextEdit::new(span.range.start..span.range.start + marker.len(), ""),
            TextEdit::new(span.range.end - marker.len()..span.range.end, ""),
        ])
    }

    fn rich_character_source_patches(
        &self,
        range: &Range<usize>,
        properties: &CharacterProperties,
        remove_conventional: Option<SemanticInlineStyle>,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        self.validate_range(range)?;
        let mut sources: Vec<Range<usize>> = Vec::new();
        let mut position = range.start;
        for span in self.projection().provenance_for_region(range) {
            if span.formatted.start != position
                || span.formatted.end > range.end
                || span.source.is_empty()
            {
                return Err(DocumentError::AmbiguousProjection.into());
            }
            position = span.formatted.end;
            if self.text().get(span.formatted.clone()) == Some("\n") {
                continue;
            }
            if let Some(last) = sources
                .last_mut()
                .filter(|last| last.end == span.source.start)
            {
                last.end = span.source.end;
            } else {
                sources.push(span.source);
            }
        }
        if position != range.end {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        let decoded = self.state().encoding.decode(&self.source_bytes())?;
        let normalized = normalize(&decoded, self.state().file_format);
        let mut syntax_patches = Vec::new();
        for source_range in sources {
            let mut authored = properties.clone();
            if authored.font_families.is_some()
                && authored.weight.is_some()
                && authored.bold.is_none()
            {
                let at = self
                    .projection()
                    .provenance_for_region(range)
                    .iter()
                    .find(|span| span.source.start == source_range.start)
                    .map(|span| span.formatted.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if super::rich_text::resolved_character_at(self.projection(), at)
                    .ok_or(DocumentError::UnsupportedFormatting)?
                    .bold
                {
                    authored.bold = Some(true);
                }
            }
            let local = if self.format() == Format::Html {
                if let Some(patches) = remove_conventional.and_then(|style| {
                    super::html::exact_conventional_removal(
                        &normalized,
                        &source_range,
                        style == SemanticInlineStyle::Strong,
                    )
                }) {
                    patches
                } else {
                    if authored.bold.is_some() && authored.weight.is_none() {
                        let at = self
                            .projection()
                            .provenance_for_region(range)
                            .iter()
                            .find(|span| span.source.start == source_range.start)
                            .map(|span| span.formatted.start)
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        let inherited =
                            super::rich_text::resolved_character_at(self.projection(), at)
                                .ok_or(DocumentError::UnsupportedFormatting)?;
                        authored.weight = Some(inherited.base_weight);
                    }
                    if properties.underline.is_some() != properties.strikethrough.is_some() {
                        let at = self
                            .projection()
                            .provenance_for_region(range)
                            .iter()
                            .find(|span| span.source.start == source_range.start)
                            .map(|span| span.formatted.start)
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        let inherited =
                            super::rich_text::resolved_character_at(self.projection(), at)
                                .ok_or(DocumentError::UnsupportedFormatting)?;
                        if authored.underline.is_none() {
                            authored.underline = Some(inherited.underline);
                        }
                        if authored.strikethrough.is_none() {
                            authored.strikethrough = Some(inherited.strikethrough);
                        }
                    }
                    let (opening, closing) = super::html::character_wrapper(&authored);
                    vec![
                        (source_range.start..source_range.start, opening),
                        (source_range.end..source_range.end, closing),
                    ]
                }
            } else {
                super::rtf::character_patches(&normalized, &source_range, &authored)?
            };
            for patch in local {
                if !syntax_patches.contains(&patch) {
                    syntax_patches.push(patch);
                }
            }
        }
        let mut coalesced: Vec<(Range<usize>, String)> = Vec::new();
        for (range, syntax) in syntax_patches {
            if range.is_empty() {
                if let Some((_, existing)) = coalesced.iter_mut().find(|(prior, _)| *prior == range)
                {
                    existing.push_str(&syntax);
                    continue;
                }
            }
            coalesced.push((range, syntax));
        }
        coalesced
            .into_iter()
            .map(|(range, syntax)| {
                Ok(SourcePatch::primary(
                    range,
                    self.state().encoding.encode_fragment(&syntax)?,
                ))
            })
            .collect()
    }

    fn prepare_rich_character_properties(
        &self,
        range: Range<usize>,
        properties: CharacterProperties,
        remove_conventional: Option<SemanticInlineStyle>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if range.is_empty() || properties == CharacterProperties::default() {
            return Ok(self.no_op_prepared());
        }
        let after_revision = Revision(self.next_revision);
        for remove in [remove_conventional, None] {
            let mut source_patches =
                self.rich_character_source_patches(&range, &properties, remove)?;
            validate_source_patches(&mut source_patches)?;
            let source = apply_source_patches(&self.state().source, &source_patches)?;
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
            if !super::rich_text::character_edit_verified(
                self.projection(),
                &candidate.projection,
                &range,
                &properties,
            ) {
                if remove.is_some() {
                    continue;
                }
                return Err(DocumentError::VerificationFailed.into());
            }
            let map = PositionMap::for_text_snapshots(
                self.id,
                self.revision(),
                after_revision,
                self.projection(),
                &candidate.projection,
                Vec::new(),
            )?;
            let work = ProjectionWorkStatistics::full(&candidate);
            return Ok(self.prepared(
                after_revision,
                ModelChangeSummary {
                    kind: ModelChangeKind::SemanticStyle,
                    source_patches,
                    formatted_splices: Vec::new(),
                    projection_work: work,
                    style_change: None,
                },
                map,
                None,
                self.next_projected_block_id,
                PreparedPublication::State(candidate),
            ));
        }
        Err(DocumentError::VerificationFailed.into())
    }

    /// Return the minimal source patches for an authorized Markdown semantic
    /// style edit. `None` is an already-satisfied or empty no-op.
    fn semantic_style_source_patches(
        &self,
        range: &Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        self.validate_range(range)?;
        if range.is_empty() {
            return Ok(None);
        }
        if matches!(self.format(), Format::Html | Format::Rtf) {
            let properties = rich_semantic_properties(style, enabled)?;
            return self
                .rich_character_source_patches(range, &properties, None)
                .map(Some);
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
            return Ok(None);
        }
        if !enabled {
            let exact_count = matching.iter().filter(|span| &span.range == range).count();
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
        let source_patches = if enabled {
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
        Ok(Some(source_patches))
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

    fn prepare_physical_source(
        &self,
        range: Range<usize>,
        replacement: String,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format() == Format::Rtf {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        self.source_point(range.start)?;
        self.source_point(range.end)?;
        if range.start > range.end {
            return Err(DocumentError::InvalidRange {
                start: range.start,
                end: range.end,
                length: self.source_byte_len(),
            }
            .into());
        }
        let encoded = self.encoding().encode_fragment(&replacement.replace(
            "\n",
            match self.file_format() {
                FileFormat::Unix => "\n",
                FileFormat::Dos => "\r\n",
                FileFormat::Mac => "\r",
            },
        ))?;
        if self
            .state()
            .source
            .bytes_in(range.clone())
            .is_some_and(|original| original == encoded)
        {
            return Ok(self.no_op_prepared());
        }
        let patches = vec![SourcePatch::primary(range.clone(), encoded)];
        // Identity text projections use the existing line-local incremental
        // path. Source syntax edits in rich/cooked formats may reinterpret
        // following grammar and therefore require authoritative reprojection.
        if matches!(self.format(), Format::PlainText | Format::MarkdownSource) {
            let start = self
                .projection()
                .map_source_boundary(self.revision(), range.start, BoundaryAffinity::Downstream)
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .formatted_offset;
            let end = self
                .projection()
                .map_source_boundary(self.revision(), range.end, BoundaryAffinity::Upstream)
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .formatted_offset;
            return self.prepare_text_edits_with_patches(
                vec![TextEdit::new(start..end, replacement)],
                Some(patches),
            );
        }
        let source = apply_source_patches(&self.state().source, &patches)?;
        let decoded = self.encoding().decode(&source.bytes())?;
        let revision = Revision(self.next_revision);
        let mut candidate = build_state_from_decoded(
            source,
            decoded,
            self.format(),
            self.file_format(),
            self.file_format_origin(),
            self.line_ending_evidence(),
            revision,
        )?;
        let edits = source_backed_reprojection_edits(self.projection(), &candidate.projection);
        candidate
            .projection
            .install_persistent_text_edits(self.projection(), &edits)
            .map_err(DocumentError::FormattedTextStorage)?;
        let formatted_splices = edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let splices =
            grapheme_closed_snapshot_map_splices(self.projection(), &candidate.projection, &edits)?;
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            splices,
        )?;
        let next_id = candidate
            .projection
            .install_reconciled_source_block_ids(
                self.projection(),
                &edits,
                &map,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::TextEdits,
                source_patches: patches,
                formatted_splices,
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            next_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_format(
        &self,
        target: Format,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if target == self.state().format {
            return Ok(self.no_op_prepared());
        }
        let revision = Revision(self.next_revision);
        let decoded = self.state().encoding.decode(&self.state().source.bytes())?;
        let mut candidate = build_state_from_decoded(
            self.state().source.clone(),
            decoded,
            target,
            self.state().file_format,
            self.state().file_format_origin,
            self.state().line_ending_evidence,
            revision,
        )?;
        let edits = source_backed_reprojection_edits(self.projection(), &candidate.projection);
        candidate
            .projection
            .install_persistent_text_edits(self.projection(), &edits)
            .map_err(DocumentError::FormattedTextStorage)?;
        let formatted_splices = edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let map_splices = if edits.is_empty() {
            Vec::new()
        } else {
            grapheme_closed_snapshot_map_splices(self.projection(), &candidate.projection, &edits)?
        };
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            map_splices,
        )?;
        let next_id = candidate
            .projection
            .install_reconciled_source_block_ids(
                self.projection(),
                &edits,
                &map,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SourceMetadata,
                source_patches: Vec::new(),
                formatted_splices,
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            next_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_encoding(
        &self,
        target: super::Encoding,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if target == self.state().encoding {
            return Ok(self.no_op_prepared());
        }
        let original = self.state().source.bytes();
        let decoded = self.state().encoding.decode(&original)?;
        if let Some(opaque) = decoded.spans.iter().find(|span| span.diagnostic.is_some()) {
            return Err(DocumentError::OpaqueDecodingConflict {
                source_range: opaque.source.clone(),
            }
            .into());
        }
        let mut bytes = if self.state().has_bom {
            target.bom_bytes().to_vec()
        } else {
            Vec::new()
        };
        bytes.extend(target.encode_fragment(&decoded.text)?);
        let source_patches = byte_difference(&original, &bytes);
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let revision = Revision(self.next_revision);
        let new_decoded = target.decode(&bytes)?;
        if new_decoded.text != decoded.text {
            return Err(DocumentError::VerificationFailed.into());
        }
        let mut candidate = build_state_from_decoded(
            source,
            new_decoded,
            self.state().format,
            self.state().file_format,
            self.state().file_format_origin,
            self.state().line_ending_evidence,
            revision,
        )?;
        if candidate.projection.text() != self.text()
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
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            Vec::new(),
        )?;
        let work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SourceMetadata,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work: work,
                style_change: None,
            },
            map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_file_format(
        &self,
        target: FileFormat,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format() == Format::Rtf {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
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
                if matches!(self.format(), Format::Html | Format::Rtf) {
                    candidate
                        .projection
                        .install_source_block_ids(self.projection())
                        .map_err(super::block_identity_document_error)?;
                } else {
                    candidate
                        .projection
                        .install_unchanged_block_ids(self.projection())
                        .map_err(super::block_identity_document_error)?;
                }
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
        if let Some(candidate) = self.build_rich_local_text_edit_candidate(
            &source,
            revision,
            target_text,
            text_splice_work,
            edits,
            source_patches,
        )? {
            return Ok(candidate);
        }
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

    /// Ordinary rich text splices cannot change parsing state: the reverse
    /// adapter has accepted a contiguous visible source extent and emitted
    /// escaped HTML text or a balanced Unicode-scoped RTF group. Reparse the
    /// affected HTML hard line (or the self-contained inserted RTF syntax),
    /// verify the exact text, and splice only that line's persistent indexes.
    fn build_rich_local_text_edit_candidate(
        &self,
        source: &super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        patches: &[SourcePatch],
    ) -> Result<Option<TextEditCandidate>, ModelTransactionError> {
        if !matches!(self.format(), Format::Html | Format::Rtf)
            || edits.len() != 1
            || patches.len() != 1
        {
            return Ok(None);
        }
        let edit = &edits[0];
        let patch = &patches[0];
        // Context inheritance is valid only for ordinary escaped text. List
        // and paragraph intents may produce identical text while changing the
        // enclosing state, and must pass through a structural reparse.
        let canonical = if self.format() == Format::Html {
            super::rich_text::escape_html_source_edit(self, patch.range.start, &edit.replacement)?
        } else {
            super::rtf::escape(&edit.replacement)
        };
        if patch.replacement != self.state().encoding.encode_fragment(&canonical)? {
            return Ok(None);
        }
        let Some(line) = self.projection().hard_line_at_offset(edit.range.start) else {
            return Ok(None);
        };
        let Some(edited_line) = self.projection().hard_line_range(line) else {
            return Ok(None);
        };
        let mut blocks = self.projection().blocks_for_region(&edited_line);
        blocks.retain(|block| {
            block.range.start <= edited_line.start && edited_line.end <= block.range.end
        });
        let Some(mut block) = blocks.pop() else {
            return Ok(None);
        };
        let old_line = block.range.clone();
        let first_line = self
            .projection()
            .hard_line_at_offset(old_line.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let last_line = self
            .projection()
            .hard_line_at_offset(old_line.end)
            .ok_or(DocumentError::VerificationFailed)?;
        if edit.range.end > edited_line.end
            || edit.replacement.contains(['\r', '\n'])
            || self
                .projection()
                .has_decoding_diagnostic_overlapping(&old_line)
        {
            return Ok(None);
        }
        let old_provenance = self.projection().provenance_for_region(&old_line);
        if edit.range.is_empty()
            && !old_provenance.iter().any(|span| {
                !span.formatted.is_empty()
                    && (span.source.start == patch.range.start
                        || span.source.end == patch.range.start)
            })
        {
            // Empty formatting elements expose a valid typing anchor but no
            // character sample. Parse their active source context explicitly.
            return Ok(None);
        }
        let (Some(first), Some(last)) = (old_provenance.first(), old_provenance.last()) else {
            return Ok(None);
        };
        let old_source = first.source.start..last.source.end;
        if patch.range.start < old_source.start || patch.range.end > old_source.end {
            return Ok(None);
        }
        let Some(physical_line) = self
            .state()
            .source_hard_lines
            .line_at_offset(patch.range.start)
        else {
            return Ok(None);
        };
        let physical = self
            .state()
            .source_hard_lines
            .get(physical_line)
            .ok_or(DocumentError::VerificationFailed)?;
        if patch.range.end >= physical.end
            && physical_line + 1 != self.state().source_hard_lines.len()
        {
            return Ok(None);
        }
        let old_patch_bytes = self
            .state()
            .source
            .bytes_in(patch.range.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        let decoded_old_patch = self
            .state()
            .encoding
            .decode_region(&old_patch_bytes, patch.range.start)?;
        if decoded_old_patch.text.contains(['\r', '\n']) {
            return Ok(None);
        }
        let new_source_end =
            rebase_source_boundary(old_source.end, patches, Association::AfterInsertion)?;
        let new_source = old_source.start..new_source_end;
        let new_line_end = old_line.end - edit.range.len() + edit.replacement.len();
        let new_text = target_text
            .slice(old_line.start..new_line_end)
            .map_err(DocumentError::FormattedTextStorage)?;
        let prefix_length = first.formatted.start - old_line.start;
        let old_styles = self.projection().style_spans_for_region(&old_line);
        if old_styles
            .iter()
            .any(|span| span.range.start < old_line.start || span.range.end > old_line.end)
        {
            return Ok(None);
        }
        block.range = 0..new_text.len();
        let mut provenance;
        let decoded_bytes;
        if self.format() == Format::Html {
            let content = &new_text[prefix_length..];
            if content.starts_with(char::is_whitespace) || content.ends_with(char::is_whitespace) {
                return Ok(None);
            }
            let fragment = source
                .bytes_in(new_source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let sentinel = self.state().encoding.encode_fragment("X")?;
            let mut bytes = sentinel.clone();
            bytes.extend(&fragment);
            bytes.extend(&sentinel);
            let decoded = self.state().encoding.decode_region(&bytes, 0)?;
            let normalized = normalize(&decoded, self.state().file_format);
            let parsed = super::html::project(&normalized, revision, 0, bytes.len());
            if parsed.text() != format!("X{content}X") {
                return Ok(None);
            }
            provenance = parsed.provenance_for_region(&(1..1 + content.len()));
            for span in &mut provenance {
                if span.formatted.start < 1
                    || span.formatted.end > 1 + content.len()
                    || span.source.start < sentinel.len()
                {
                    return Ok(None);
                }
                span.formatted = span.formatted.start - 1 + prefix_length
                    ..span.formatted.end - 1 + prefix_length;
                span.source = span.source.start - sentinel.len() + new_source.start
                    ..span.source.end - sentinel.len() + new_source.start;
            }
            decoded_bytes = bytes.len() + old_patch_bytes.len();
        } else {
            let decoded = self
                .state()
                .encoding
                .decode_region(&patch.replacement, patch.range.start)?;
            let normalized = normalize(&decoded, self.state().file_format);
            let parsed = super::rtf::project(
                &normalized,
                revision,
                patch.range.start,
                patch.range.start + patch.replacement.len(),
            );
            if parsed.text() != edit.replacement {
                return Ok(None);
            }
            provenance = Vec::new();
            for mut span in old_provenance {
                if span.formatted.end <= edit.range.start {
                    span.formatted =
                        span.formatted.start - old_line.start..span.formatted.end - old_line.start;
                } else if span.formatted.start >= edit.range.end {
                    span.formatted = span.formatted.start - edit.range.len()
                        + edit.replacement.len()
                        - old_line.start
                        ..span.formatted.end - edit.range.len() + edit.replacement.len()
                            - old_line.start;
                    span.source = rebase_source_boundary(
                        span.source.start,
                        patches,
                        Association::AfterInsertion,
                    )?
                        ..rebase_source_boundary(
                            span.source.end,
                            patches,
                            Association::AfterInsertion,
                        )?;
                } else {
                    continue;
                }
                provenance.push(span);
            }
            for mut span in parsed.provenance().iter().cloned() {
                span.formatted = span.formatted.start + edit.range.start - old_line.start
                    ..span.formatted.end + edit.range.start - old_line.start;
                provenance.push(span);
            }
            provenance.sort_by_key(|span| span.formatted.start);
            decoded_bytes = patch.replacement.len() + old_patch_bytes.len();
        }
        let sampled_at = if edit.range.is_empty() {
            self.projection()
                .provenance_for_region(&edited_line)
                .iter()
                .find(|span| span.source.end == patch.range.start && !span.formatted.is_empty())
                .map(|span| span.formatted.end - 1)
                .unwrap_or(edit.range.start)
        } else if edit.range.start == edited_line.end {
            edited_line.end.saturating_sub(1)
        } else {
            edit.range.start
        };
        let insertion_styles = old_styles
            .iter()
            .filter(|span| span.range.contains(&sampled_at))
            .map(|span| span.application.clone())
            .collect::<Vec<_>>();
        let mut styles = Vec::new();
        for span in old_styles {
            if span.range.start < edit.range.start {
                let end = span.range.end.min(edit.range.start);
                styles.push(StyleSpan {
                    range: span.range.start - old_line.start..end - old_line.start,
                    application: span.application.clone(),
                });
            }
            if span.range.end > edit.range.end {
                let start = span.range.start.max(edit.range.end);
                styles.push(StyleSpan {
                    range: start - edit.range.len() + edit.replacement.len() - old_line.start
                        ..span.range.end - edit.range.len() + edit.replacement.len()
                            - old_line.start,
                    application: span.application,
                });
            }
        }
        if !edit.replacement.is_empty() {
            for application in insertion_styles {
                styles.push(StyleSpan {
                    range: edit.range.start - old_line.start
                        ..edit.range.start - old_line.start + edit.replacement.len(),
                    application,
                });
            }
        }
        styles.sort_by_key(|span| span.range.start);
        let mut merged: Vec<StyleSpan> = Vec::new();
        for span in styles {
            if let Some(previous) = merged.last_mut().filter(|previous| {
                previous.range.end == span.range.start && previous.application == span.application
            }) {
                previous.range.end = span.range.end;
            } else if !span.range.is_empty() {
                merged.push(span);
            }
        }
        let mut regional = FormattedDocument::from_parts(
            revision,
            new_text,
            vec![block],
            merged,
            provenance,
            Vec::new(),
            self.projection().style_sheet().clone(),
            new_source.start,
            new_source.end,
        );
        let hard_line_ranges = (first_line..=last_line)
            .map(|index| {
                let range = self
                    .projection()
                    .hard_line_range(index)
                    .ok_or(DocumentError::VerificationFailed)?;
                let start = if index > line {
                    range.start - edit.range.len() + edit.replacement.len()
                } else {
                    range.start
                };
                let end = if index >= line {
                    range.end - edit.range.len() + edit.replacement.len()
                } else {
                    range.end
                };
                Ok(start - old_line.start..end - old_line.start)
            })
            .collect::<Result<Vec<_>, DocumentError>>()?;
        regional.install_hard_line_partition(hard_line_ranges);
        let projected_bytes = regional.text().len();
        let (projection, range_work) = match splice_line_local_projection(
            self.projection(),
            regional,
            revision,
            first_line..last_line + 1,
            old_line,
            old_source,
            new_source,
            target_text.clone(),
            source.len(),
        ) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let physical_end =
            rebase_source_boundary(physical.end, patches, Association::AfterInsertion)?;
        let (source_hard_lines, source_line_work) = self
            .state()
            .source_hard_lines
            .replace_ranges_with_stats(
                physical_line..physical_line + 1,
                &[physical.start..physical_end],
            )
            .ok_or(DocumentError::VerificationFailed)?;
        let state = DocumentState {
            revision,
            source: source.clone(),
            projection,
            source_hard_lines,
            encoding: self.state().encoding,
            format: self.state().format,
            file_format: self.state().file_format,
            file_format_origin: self.state().file_format_origin,
            line_ending_evidence: self.state().line_ending_evidence,
            has_bom: self.state().has_bom,
        };
        let mut work = ProjectionWorkStatistics::regional(
            decoded_bytes,
            projected_bytes,
            last_line - first_line + 1,
            text_work,
            range_work,
            source_line_work,
        );
        work.source_decode_passes = 2;
        work.source_hard_line_records_rebuilt = 1;
        Ok(Some(TextEditCandidate {
            state,
            work,
            block_ids_already_reconciled: true,
        }))
    }

    fn line_local_projection_region(
        &self,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
    ) -> Result<Option<LineLocalProjectionRegion>, ModelTransactionError> {
        // Stateful HTML/RTF parsing cannot restart at a physical source line.
        if edits.is_empty() || matches!(self.format(), Format::Html | Format::Rtf) {
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
            StyleProperty::CharacterBold => target.bold = None,
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
        let Some(before) = before_sheet.block_style(id) else {
            continue;
        };
        let after_id = if after_sheet.block_style(id).is_some() {
            id
        } else if before.role == super::BlockRole::Document {
            &after_sheet.base_document
        } else {
            &after_sheet.base_paragraph
        };
        let after = after_sheet
            .block_style(after_id)
            .ok_or_else(|| StyleError::UnknownStyle(after_id.clone()))?;
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
                    after_id,
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
                    after_id,
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
        if before_sheet.character_style(id).is_none() {
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
        let after_id = if after_sheet.character_style(id).is_some() {
            id
        } else {
            &after_sheet.base_character
        };
        let after = after_sheet.resolve_assigned_paragraph_style(
            after_assignment,
            &after_sheet.base_paragraph,
            &BlockProperties::default(),
            &CharacterProperties::default(),
            Some(after_id),
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
fn rich_semantic_properties(
    style: SemanticInlineStyle,
    enabled: bool,
) -> Result<CharacterProperties, DocumentError> {
    let mut properties = CharacterProperties::default();
    match style {
        SemanticInlineStyle::Strong => properties.bold = Some(enabled),
        SemanticInlineStyle::Emphasis => {
            properties.slant = Some(if enabled {
                super::FontSlant::Italic
            } else {
                super::FontSlant::Upright
            })
        }
        SemanticInlineStyle::Code => return Err(DocumentError::UnsupportedFormatting),
    }
    Ok(properties)
}

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
    if format == Format::Html {
        return super::html::escape(payload.text());
    }
    if format == Format::Rtf {
        return super::rtf::escape(payload.text());
    }
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

/// Match unchanged graphemes by their exact source extent, so changing format
/// interpretation retains anchors throughout the document, including between
/// distant delimiter edits. Equal spelling alone never establishes identity.
fn source_backed_reprojection_edits(
    before: &FormattedDocument,
    after: &FormattedDocument,
) -> Vec<TextEdit> {
    let mut new_items = BTreeMap::new();
    for (offset, item) in after.text().grapheme_indices(true) {
        if let Some(source) = after.source_range(offset..offset + item.len()) {
            new_items.insert((source.start, source.end), (offset, item));
        }
    }
    let mut edits = Vec::new();
    let mut old_start = 0;
    let mut new_start = 0;
    for (offset, item) in before.text().grapheme_indices(true) {
        let Some(source) = before.source_range(offset..offset + item.len()) else {
            continue;
        };
        let Some(&(new_offset, new_item)) = new_items.get(&(source.start, source.end)) else {
            continue;
        };
        if new_offset < new_start || item != new_item {
            continue;
        }
        if old_start != offset || new_start != new_offset {
            edits.push(TextEdit::new(
                old_start..offset,
                &after.text()[new_start..new_offset],
            ));
        }
        old_start = offset + item.len();
        new_start = new_offset + item.len();
    }
    if old_start != before.text().len() || new_start != after.text().len() {
        edits.push(TextEdit::new(
            old_start..before.text().len(),
            &after.text()[new_start..],
        ));
    }
    edits
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
