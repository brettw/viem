//! Revision-bound document requests and two-phase model transactions.
//!
//! Preparation performs every fallible reverse-projection, encoding, and
//! verification step against immutable document state.  A prepared value owns
//! the candidate state needed for publication.  Commit only rechecks the
//! captured model preconditions and installs that already-verified candidate.

#[path = "clear_content.rs"]
mod clear_content;
#[path = "clipboard_fragment.rs"]
mod clipboard_fragment;
#[path = "fragments.rs"]
mod fragments;
pub use clipboard_fragment::{ClipboardFragment, TableClipboardCell};
#[path = "edit_translation.rs"]
mod edit_translation;
#[path = "input_context.rs"]
mod input_context;
#[path = "list_indent.rs"]
mod list_indent;
#[path = "markdown_block_styles.rs"]
mod markdown_block_styles;
#[path = "markdown_code_style.rs"]
mod markdown_code_style;
#[path = "markdown_code_language.rs"]
mod markdown_code_language;
#[path = "markdown_gfm_edit.rs"]
mod markdown_gfm_edit;
#[path = "markdown_reference_edit.rs"]
mod markdown_reference_edit;
#[path = "markdown_table_edit.rs"]
mod markdown_table_edit;
#[path = "markdown_table_projection.rs"]
mod markdown_table_projection;
#[path = "markdown_html_edit.rs"]
mod markdown_html_edit;
#[path = "markdown_indented_edit.rs"]
mod markdown_indented_edit;
#[path = "markdown_list_edit.rs"]
mod markdown_list_edit;
#[path = "markdown_list_structure.rs"]
mod markdown_list_structure;
#[path = "markdown_numbering.rs"]
mod markdown_numbering;
#[path = "markdown_quote_edit.rs"]
mod markdown_quote_edit;
#[path = "markdown_split.rs"]
mod markdown_split;
#[path = "markdown_edit_spelling.rs"]
mod markdown_edit_spelling;
#[path = "markdown_typing.rs"]
mod markdown_typing;
#[path = "markdown_autodetect.rs"]
mod markdown_autodetect;
#[path = "named_character.rs"]
mod named_character;
#[path = "paragraph_insertion.rs"]
mod paragraph_insertion;
#[path = "paragraph_keys.rs"]
mod paragraph_keys;
#[path = "replacement.rs"]
mod replacement;
#[path = "structural_style.rs"]
mod structural_style;
#[path = "strikethrough.rs"]
mod strikethrough;
#[path = "inline_properties.rs"]
mod inline_properties;
pub(crate) use inline_properties::resolved_inline_boolean;
#[path = "typing.rs"]
mod typing;
pub use fragments::{FragmentEdit, ReplacementFragment};
pub(crate) use replacement::RecordedReplacement;

use super::formatted_text::{FormattedTextSpliceStats, LogicalGraphemeSnapshot};
use super::line_endings::{detect, normalize, normalize_literal};
use super::projection::{
    escape_markdown_insert, escape_markdown_insert_in_encoding, project,
    splice_line_local_projection,
};
use super::source_line_index::SourceHardLineSpliceStats;
use super::transfer::{self, HardLineTransfer};
use super::{
    build_state_from_decoded, ranges_overlap, spell_logical_breaks, Association, BlockProperties,
    BoundaryAffinity, CharacterProperties, ConfigurationStyleIntent, DeletionRecovery, Document,
    DocumentError, DocumentId, DocumentState, DocumentStyleAssignment, FileFormat,
    FileFormatOrigin, Format, FormattedDocument, FormattedPayloadEdit, FormattedPayloadEditRequest,
    FormattedTextPayload, FormattedTextTree, HistoryChangeNumber, HistoryError, HistoryLocation,
    HistoryNavigation, HistoryNodeId, HistoryRestoration, HistoryRestorationSnapshot,
    HistorySemanticChangeKind, HistorySourcePatch, HistoryTransactionSummary, MappingOutcome,
    PositionError,
    PositionMap, Revision, SemanticInlineStyle, SourcePartId, Splice, StyleApplication,
    StyleDefinitionEdit, StyleError, StyleId, StyleInvalidationEffect,
    StyleProperty, StyleSheet, StyleSheetRevision, StyleSpan, TextEdit,
    TextRange, UnsupportedEditReason,
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

/// A persisted-content style intention. These are deliberately representable
/// even when an adapter cannot translate them, so capability failures are
/// typed and non-destructive instead of silently creating a sidecar.
#[derive(Clone, Debug, PartialEq)]
pub enum PersistedStyleIntent {
    AssignBlockStyle {
        range: TextRange,
        style: StyleId,
    },
    AssignCharacterStyle {
        range: TextRange,
        style: StyleId,
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
    SetInlineProperty { document: DocumentId, revision: Revision, range: Range<usize>, property: StyleProperty, enabled: bool },
    SetStrikethrough {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        enabled: bool,
    },
    ApplyFragmentEdits {
        document: DocumentId,
        revision: Revision,
        edits: Vec<FragmentEdit>,
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
    /// Explicit linewise deletion, distinct from removing all body characters.
    DeleteLines {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
    },
    /// Remove all content and its enclosing formatting scopes, retaining the
    /// source format's document envelope and metadata.
    ClearDocumentContent {
        document: DocumentId,
        revision: Revision,
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
    SetBlockQuote {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        enabled: bool,
    },
    IndentList {
        document: DocumentId,
        revision: Revision,
        range: Range<usize>,
        unindent: bool,
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
    ContinueList {
        document: DocumentId,
        revision: Revision,
        at: usize,
    },
    InsertHardBreak {
        document: DocumentId,
        revision: Revision,
        at: usize,
        affinity: BoundaryAffinity,
    },
    /// Open a paragraph at a line boundary, using the originating paragraph's
    /// following style. Markdown Source continues parsed list items; other
    /// source-view line insertion remains literal.
    OpenLine {
        document: DocumentId,
        revision: Revision,
        at: usize,
        origin: usize,
        after: bool,
    },
    SetFileFormat {
        document: DocumentId,
        revision: Revision,
        target: FileFormat,
    },

    /// Switch between the two views of the same Markdown source.
    SetMarkdownSource {
        document: DocumentId,
        revision: Revision,
        source: bool,
    },
    /// Reproject the same physical source in another supported view format.
    SetViewFormat {
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
    /// Reorder a nonempty hard-line span, optionally removing duplicates.
    /// `order` contains distinct pre-edit ordinals inside `source_lines`.
    ReorderHardLines {
        document: DocumentId,
        revision: Revision,
        source_lines: Range<usize>,
        order: Vec<usize>,
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
    NavigateHistory {
        document: DocumentId,
        revision: Revision,
        navigation: HistoryNavigationRequest,
    },
}

impl ModelRequest {
    pub fn document(&self) -> DocumentId {
        match self {
            Self::SetStrikethrough { document, .. } | Self::SetInlineProperty { document, .. } => *document,
            Self::ReplacePhysicalSource { document, .. } => *document,
            Self::ApplyFragmentEdits { document, .. }
            | Self::ApplyTextEdits { document, .. }
            | Self::DeleteLines { document, .. }
            | Self::ClearDocumentContent { document, .. }
            | Self::SetSemanticStyle { document, .. }
            | Self::SetListStyle { document, .. }
            | Self::SetBlockQuote { document, .. }
            | Self::IndentList { document, .. }
            | Self::SetParagraphStyle { document, .. }
            | Self::AssignNamedStyle { document, .. }
            | Self::ContinueList { document, .. }
            | Self::InsertHardBreak { document, .. }
            | Self::OpenLine { document, .. }
            | Self::SetFileFormat { document, .. }
            | Self::SetMarkdownSource { document, .. }
            | Self::SetViewFormat { document, .. }
            | Self::SetEncoding { document, .. }
            | Self::ReorderHardLines { document, .. }
            | Self::TransferHardLines { document, .. }
            | Self::NavigateHistory { document, .. } => *document,
        }
    }

    pub fn revision(&self) -> Revision {
        match self {
            Self::SetStrikethrough { revision, .. } | Self::SetInlineProperty { revision, .. } => *revision,
            Self::ReplacePhysicalSource { revision, .. } => *revision,
            Self::ApplyFragmentEdits { revision, .. }
            | Self::ApplyTextEdits { revision, .. }
            | Self::DeleteLines { revision, .. }
            | Self::ClearDocumentContent { revision, .. }
            | Self::SetSemanticStyle { revision, .. }
            | Self::SetListStyle { revision, .. }
            | Self::SetBlockQuote { revision, .. }
            | Self::IndentList { revision, .. }
            | Self::SetParagraphStyle { revision, .. }
            | Self::AssignNamedStyle { revision, .. }
            | Self::ContinueList { revision, .. }
            | Self::InsertHardBreak { revision, .. }
            | Self::OpenLine { revision, .. }
            | Self::SetFileFormat { revision, .. }
            | Self::SetMarkdownSource { revision, .. }
            | Self::SetViewFormat { revision, .. }
            | Self::SetEncoding { revision, .. }
            | Self::ReorderHardLines { revision, .. }
            | Self::TransferHardLines { revision, .. }
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
        super::work_statistics::record(|stats| {
            stats.regional_projection_candidates += 1;
            stats.projected_formatted_bytes += projected_formatted_bytes;
            stats.projection_persistent_nodes_visited += text.nodes_visited
                + ranges.range_index_nodes_visited()
                + source_lines.nodes_visited;
            stats.projection_persistent_nodes_copied +=
                text.nodes_copied + ranges.range_index_nodes_copied() + source_lines.nodes_copied;
        });
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
        super::work_statistics::record(|stats| {
            stats.full_projection_candidates += 1;

            stats.projected_formatted_bytes += candidate.projection.text_tree().byte_len();
            stats.formatted_full_materialized_bytes += candidate.projection.text_tree().byte_len();
        });
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

    fn accumulate(&mut self, other: Self) {
        self.scope = match (self.scope, other.scope) {
            (ProjectionWorkScope::FullDocument, _) | (_, ProjectionWorkScope::FullDocument) => {
                ProjectionWorkScope::FullDocument
            }
            (ProjectionWorkScope::RegionalHardLines, _)
            | (_, ProjectionWorkScope::RegionalHardLines) => ProjectionWorkScope::RegionalHardLines,
            _ => ProjectionWorkScope::None,
        };
        macro_rules! sum { ($($field:ident),+ $(,)?) => { $(self.$field = self.$field.saturating_add(other.$field);)+ }; }
        sum!(
            source_decode_passes,
            decoded_source_bytes,
            projected_formatted_bytes,
            projected_hard_lines,
            source_hard_line_records_rebuilt,
            formatted_text_nodes_visited,
            formatted_text_nodes_copied,
            formatted_text_leaves_copied,
            range_index_nodes_visited,
            range_index_nodes_copied,
            range_index_leaves_copied,
            range_index_records_copied,
            source_hard_line_nodes_visited,
            source_hard_line_nodes_copied,
            source_hard_line_leaves_copied,
            source_hard_line_records_copied,
            full_text_bytes_materialized
        );
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
    pub(super) fn primary(range: Range<usize>, replacement: Vec<u8>) -> Self {
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
    conversion_warnings: Vec<super::ConversionWarning>,
}

impl ModelChangeSummary {
    pub fn conversion_warnings(&self) -> &[super::ConversionWarning] {
        &self.conversion_warnings
    }

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
// Literal edits only need bounded encoding/line-ending context around the
// changed text. Hard lines themselves are not a safe allocation bound.
const LITERAL_EDIT_CONTEXT_BYTES: usize = 4096;

struct LineLocalProjectionRegion {
    hard_lines: Range<usize>,
    source_lines: Range<usize>,
    old_formatted: Range<usize>,
    old_source: Range<usize>,
    retained_separator: Option<Range<usize>>,
}

struct TextEditCandidate {
    state: DocumentState,
    work: ProjectionWorkStatistics,
    block_ids_already_reconciled: bool,
    next_projected_block_id: u64,
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
    /// Isolated candidate editing state: share the immutable document snapshot,
    /// but do not inherit UI history groups, position captures or file writes.
    fn scratch_document(&self) -> Self {
        super::work_statistics::record(|stats| stats.scratch_documents += 1);
        Document {
            id: self.id,
            history: super::history::History::transient(self.state().clone()),
            open_work: self.open_work,
            next_revision: self.next_revision,
            next_projected_block_id: self.next_projected_block_id,
            edit_group_depth: 0,
            edit_group_generation: 0,
            position_map_capture: None,
            layout_changes: std::collections::VecDeque::new(),
            artifact_binding: None,
            pending_artifact_writes: Default::default(),
            next_artifact_write_token: 1,
            next_save_sequence: 1,
            last_successful_save_sequence: 0,
            read_only: false,
            recovered_dirty: false,
            code_presentation: None,
            configuration: self.configuration.clone(),
            configuration_state: None,
        }
    }

    pub(super) fn prepared_candidate_document(&self, prepared: &PreparedModelTransaction) -> Result<Document, DocumentError> {
        if prepared.document != self.id() || prepared.before_revision != self.revision() { return Err(DocumentError::VerificationFailed); }
        let mut scratch = self.scratch_document();
        if let PreparedPublication::State(state) = &prepared.publication {
            scratch.history = super::history::History::transient(state.clone());
        } else if !prepared.is_no_op() { return Err(DocumentError::VerificationFailed); }
        Ok(scratch)
    }

    /// Validate a controller boundary in the prepared result before any source
    /// or history state is published. Position maps describe structural shifts;
    /// their numeric result alone does not prove a logical grapheme boundary.
    pub(crate) fn prepared_text_point(
        &self,
        prepared: &PreparedModelTransaction,
        offset: usize,
    ) -> Result<super::TextPoint, DocumentError> {
        if prepared.document != self.id {
            return Err(DocumentError::WrongDocument);
        }
        if prepared.before_revision != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: prepared.before_revision,
            });
        }
        let state = match &prepared.publication {
            PreparedPublication::NoOp => self.state(),
            PreparedPublication::State(state) => state,
            PreparedPublication::History { target } => self
                .history
                .state_at_node(*target)
                .ok_or(DocumentError::VerificationFailed)?
                .as_ref(),
        };
        let length = state.projection.text_tree().byte_len();
        if offset > length {
            return Err(DocumentError::InvalidRange {
                start: offset,
                end: offset,
                length,
            });
        }
        if !state
            .projection
            .is_logical_grapheme_boundary(offset)
            .map_err(|_| DocumentError::NotGraphemeBoundary(offset))?
        {
            return Err(DocumentError::NotGraphemeBoundary(offset));
        }
        Ok(super::TextPoint {
            document: self.id,
            revision: prepared.after_revision,
            offset,
        })
    }

    /// Prepare and verify a revision-bound model operation without mutation.
    pub fn prepare_model_request(
        &self,
        request: ModelRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        super::work_statistics::record(|stats| stats.model_requests += 1);
        self.validate_request_target(&request)?;
        let numbering = self
            .format()
            .is_markdown()
            .then(|| markdown_numbering::scope(&request))
            .flatten();
        let prepared = match request {
            ModelRequest::ApplyFragmentEdits { edits, .. } => self.prepare_fragment_edits(edits),
            ModelRequest::SetInlineProperty { range, property, enabled, .. } => self.prepare_inline_property(range, property, enabled),
            ModelRequest::SetStrikethrough { range, enabled, .. } => {
                self.prepare_strikethrough(range, enabled)
            }
            ModelRequest::ReplacePhysicalSource {
                range, replacement, ..
            } => self.prepare_physical_source(range, replacement),
            ModelRequest::ApplyTextEdits { edits, .. } => self.prepare_text_edits(edits),
            ModelRequest::DeleteLines { range, .. } => self.prepare_line_deletion(range),
            ModelRequest::ClearDocumentContent { .. } => self.prepare_clear_document_content(),
            ModelRequest::SetSemanticStyle {
                range,
                style,
                enabled,
                ..
            } => self.prepare_semantic_style(range, style, enabled),
            ModelRequest::IndentList {
                range, unindent, ..
            } => self.prepare_list_indent(range, unindent),
            ModelRequest::SetListStyle { range, style, .. } => {
                self.prepare_list_style(range, style)
            }
            ModelRequest::SetBlockQuote { range, enabled, .. } => {
                if !self.format().is_markdown() || self.projection().range_intersects_table(&range) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                self.prepare_markdown_quote_style(range, enabled)
            }
            ModelRequest::SetParagraphStyle { range, style, .. } => {
                self.prepare_markdown_paragraph_style(range, style)
            }
            ModelRequest::AssignNamedStyle {
                range,
                namespace,
                style,
                ..
            } => {
                if style.is_internal() {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                let range =
                    TextRange::new(self.text_point(range.start)?, self.text_point(range.end)?)?;
                let intent = match namespace {
                    super::StyleNamespace::Block => PersistedStyleIntent::AssignBlockStyle {
                        range,
                        style,
                    },
                    super::StyleNamespace::Character => {
                        return self.prepare_character_style_choice(
                            range.start().offset()..range.end().offset(),
                            style,
                        );
                    }
                };
                self.prepare_persisted_style_intent(intent)
            }
            ModelRequest::ContinueList { at, .. } => self.prepare_structural_list_enter(at),
            ModelRequest::InsertHardBreak { at, affinity, .. } => {
                self.prepare_intra_paragraph_break(at, affinity)
            }
            ModelRequest::OpenLine {
                at, origin, after, ..
            } => self.prepare_open_line(at, origin, after),
            ModelRequest::SetFileFormat { target, .. } => self.prepare_file_format(target),

            ModelRequest::SetMarkdownSource { source, .. } => self.prepare_markdown_source_change(source),
            ModelRequest::SetViewFormat { target, .. } => self.prepare_view_format_change(target),
            ModelRequest::SetEncoding { target, .. } => self.prepare_encoding(target),
            ModelRequest::ReorderHardLines {
                source_lines,
                order,
                ..
            } => match super::reorder::plan(self, source_lines, order)? {
                Some(plan) => self.prepare_hard_line_transfer_plan(plan),
                None => Ok(self.no_op_prepared()),
            },
            ModelRequest::TransferHardLines {
                operation,
                source_lines,
                destination,
                ..
            } => self.prepare_hard_line_transfer(operation, source_lines, destination),
            ModelRequest::NavigateHistory { navigation, .. } => {
                self.prepare_history_navigation(navigation)
            }
        }?;
        if let Some(range) = numbering {
            self.prepare_markdown_numbering(prepared, range)
        } else {
            Ok(prepared)
        }
    }

    /// Prepare a typed style operation against one exact formatted snapshot.
    /// Persisted-content operations validate their Markdown representation.
    /// Generated configuration operations use their distinct authority and
    /// never modify source bytes.
    pub fn prepare_style_request(
        &self,
        request: StyleModelRequest,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format().is_code() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
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

    /// Reuse verified source/projection work after the coordinator closes an
    /// Insert/Replace undo group. Closing a group changes publication grouping,
    /// not source, projection, allocation, revision, or history-node identity.
    /// Every other captured precondition remains exact; this is not a rebase.
    pub(crate) fn rebind_prepared_after_group_close(
        &self,
        mut prepared: PreparedModelTransaction,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
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
        if self.edit_group_depth != 0
            || prepared.expected_next_revision != self.next_revision
            || prepared.expected_next_projected_block_id != self.next_projected_block_id
            || prepared.expected_history != self.history.status().current
            || prepared.expected_edit_group_generation != self.edit_group_generation
            || !matches!(prepared.publication, PreparedPublication::State(_))
        {
            return Err(ModelTransactionError::StaleDocumentState);
        }
        prepared.expected_edit_group_depth = 0;
        Ok(prepared)
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
        // A compatibility command may issue several independently prepared
        // model operations during one input event (for example a macro). Fold
        // this transition into the coordinator's short-lived capture before
        // the first authoritative mutation so publication remains infallible.
        let next_position_capture = self.composed_position_capture(&prepared.text_position_map)?;
        let old_line_counts = [
            self.projection().presentation_line_count(false),
            self.projection().presentation_line_count(true),
        ];
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
        self.refresh_configuration();
        self.position_map_capture = next_position_capture;
        self.advance_code_presentation(&prepared.text_position_map);
        if prepared.after_revision != prepared.before_revision {
            let splices = prepared.summary.formatted_splices();
            let local = (prepared.summary.projection_work().scope()
                == ProjectionWorkScope::RegionalHardLines
                && !splices.is_empty())
            .then(|| {
                let old = splices
                    .iter()
                    .map(|s| s.old_range().start)
                    .min()
                    .unwrap_or(0)
                    ..splices.iter().map(|s| s.old_range().end).max().unwrap_or(0);
                let delta = splices.iter().fold(0i128, |sum, s| {
                    sum + s.inserted_len() as i128 - s.old_range().len() as i128
                });
                let new_end = usize::try_from(old.end as i128 + delta).ok()?;
                Some(super::LocalFormattedChange {
                    new: old.start..new_end,
                    old,
                    old_line_counts,
                    new_line_counts: [
                        self.projection().presentation_line_count(false),
                        self.projection().presentation_line_count(true),
                    ],
                })
            })
            .flatten();
            self.record_layout_change(super::LayoutChangeRecord {
                before: prepared.before_revision,
                after: prepared.after_revision,
                local,
            });
        }

        debug_assert_eq!(self.revision(), prepared.after_revision);
        super::work_statistics::record(|stats| stats.transaction_commits += 1);
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
                conversion_warnings: Vec::new(),
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
                conversion_warnings: Vec::new(),
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
        if matches!(&intent, PersistedStyleIntent::AssignCharacterStyle { style, .. } if style.is_internal())
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }

        if self.format().is_markdown() {
            if let PersistedStyleIntent::AssignCharacterStyle { range, style } = &intent {
                if style.0 == "Code" || style.0.is_empty() {
                    self.validate_style_text_range(*range)?;
                    return self.prepare_markdown_named_character(
                        range.start().offset()..range.end().offset(),
                        style,
                    );
                }
            }
            if let PersistedStyleIntent::AssignBlockStyle {
                range,
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
        let range = match intent {
            PersistedStyleIntent::AssignBlockStyle { range, .. }
            | PersistedStyleIntent::AssignCharacterStyle { range, .. } => range,
        };
        self.validate_style_text_range(range)?;
        Err(ModelTransactionError::Style(StyleTransactionError::Unsupported {
            format: self.format(),
            reason: if self.format() == Format::PlainText {
                UnsupportedEditReason::PlainTextHasNoRichStyleStorage
            } else {
                UnsupportedEditReason::FormatHasNoNamedStyleStorage
            },
        }))
    }

    pub(super) fn validate_style_text_range(
        &self,
        range: TextRange,
    ) -> Result<(), ModelTransactionError> {
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
                    style_sheet.rebase_definition_references_for_delete(edit, style_sheet_revision)?;
                    configured_projection.reassign_deleted_style(edit.style_id(), edit.is_block());
                    if document_style.style == *edit.style_id() {
                        document_style.style = style_sheet.base_paragraph.clone();
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
                document_style.direct_canvas.merge_declarations(properties);
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
                conversion_warnings: Vec::new(),
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_line_deletion(
        &self,
        range: Range<usize>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if let Some(prepared) = self.prepare_table_line_deletion(&range)? { return Ok(prepared); }


        let range = { range };
        let patches = if self.format() == Format::Markdown {
            markdown_list_structure::deletion_patches(self, &range, true)?
        } else {
            None
        };
        self.prepare_text_edits_with_patches(vec![TextEdit::new(range, "")], patches)
    }

    fn prepare_text_edits(
        &self,
        edits: Vec<TextEdit>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_text_edits_with_patches(edits, None)
    }

    fn validate_formatted_replacement_characters(&self, text: &str) -> Result<(), DocumentError> {
        let unrepresentable = if self.file_format() == FileFormat::Mac
            && text.contains('\r')
        {
            // The shared line-ending stage consumes every literal source CR
            // before these projections run. Plain/source modes and the current
            // Markdown adapter have no escape that can recreate a literal CR.
            Some('\r')
        } else {
            None
        };
        if let Some(character) = unrepresentable {
            return Err(DocumentError::UnrepresentableFormattedCharacter {
                format: self.format(),
                character,
            });
        }
        Ok(())
    }

    fn prepare_text_edits_with_patches(
        &self,
        edits: Vec<TextEdit>,
        explicit_source_patches: Option<Vec<SourcePatch>>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_text_edits_with_patch_policy(edits, explicit_source_patches, false)
    }

    pub(super) fn prepare_text_edits_with_patch_policy(
        &self,
        mut edits: Vec<TextEdit>,
        explicit_source_patches: Option<Vec<SourcePatch>>,
        authored_syntax: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if authored_syntax && edits.is_empty() {
            if let Some(patch) = explicit_source_patches.as_ref().and_then(|patches| patches.first()) {
                let at = self.projection().map_source_boundary(self.revision(), patch.range.start, BoundaryAffinity::Downstream)
                    .map_err(|_| DocumentError::AmbiguousProjection)?.formatted_offset;
                // A real source/style change with unchanged visible text still
                // needs a local verification window, not a full-buffer copy.
                edits.push(TextEdit::new(at..at, ""));
            }
        }
        if edits.is_empty() {
            if let Some(patches) = explicit_source_patches {
                return self.prepare_source_only_patches(patches);
            }
            return Ok(self.no_op_prepared());
        }
        for edit in &edits {
            self.validate_range(&edit.range)?;
            self.validate_formatted_replacement_characters(&edit.replacement)?;
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
            if changes_text || authored_syntax
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

        if explicit_source_patches.is_none() {
            let support = self.markdown_structural_support(&edits)?;
            if !support.is_empty() {
                return self.prepare_markdown_supporting_patches(support, |doc| {
                    doc.prepare_text_edits(edits.clone())
                });
            }
            if let Some(prepared) = self.prepare_markdown_html_text_edits(&edits)? {
                return Ok(prepared);
            }
            if let Some(prepared) = self.prepare_indented_code_text_edits(&edits)? {
                return Ok(prepared);
            }

            if let Some(prepared) = self.prepare_structural_text_batch(&edits)? {
                return Ok(prepared);
            }
        }
        let translate_source = explicit_source_patches.is_none();
        let mut source_patches = match explicit_source_patches {
            Some(patches) => patches,
            None => self.translate_source_edits(edits.iter().map(|edit| (edit, None)))?,
        };
        if translate_source {
            self.preserve_markdown_edit_boundaries(
                &edits,
                edits
                    .iter()
                    .filter(|edit| edit.replacement.contains('\n'))
                    .map(|edit| &edit.range),
                &mut source_patches,
            )?;
        }
        if !authored_syntax && !translate_source && self.format() == Format::Markdown {
            markdown_block_styles::preserve_edited_paragraph_boundaries(
                self, &edits, &mut source_patches,
            )?;
            markdown_split::remove_empty_emphasis(self, &edits, &mut source_patches)?;
            markdown_block_styles::preserve_retained_literals(self, &edits, &mut source_patches)?;
            markdown_split::repair_flanking(self, &edits, &mut source_patches)?;
            self.repair_markdown_authored_spaces(&edits, &mut source_patches)?;
            self.repair_markdown_reference_spaces(&edits, &mut source_patches)?;
            markdown_list_structure::preserve_empty_item_boundaries(
                self, &edits, &mut source_patches,
            )?;
        }
        if !authored_syntax {
            self.repair_markdown_reference_dependents(&edits, &mut source_patches)?;
            self.simplify_markdown_edit_spelling(&edits, &mut source_patches)?;
        }
        let repaired_utf16 =
            self.repair_incomplete_utf16_insertions(&edits, &mut source_patches)?;
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
        // Table rows own their physical source grammar. A verified row stub
        // proves literal Source edits without reparsing unrelated table rows.
        let table_candidate = if self.format() == Format::MarkdownSource {
            self.build_table_row_candidate(&source, after_revision, &target_text,
                text_splice_work, &edits, &source_patches)?
        } else { None };
        if table_candidate.is_none() && translate_source && self.source_edit_requires_reprojection(&edits, &source_patches)? {
            // Source-mode delimiters are editable syntax. Changing a fence
            // can make formerly literal blank lines become paired paragraph
            // separators, so the new projection need not be a flat splice of
            // the old one. Commit the exact translated source intention and
            // derive its complete formatted change from authoritative parsing.
            // Proven ordinary line edits retain the incremental path below.
            let mut prepared = self.prepare_reprojected_source_patches(source_patches)?;
            if repaired_utf16 {
                prepared
                    .summary
                    .conversion_warnings
                    .push(super::ConversionWarning::RepairedTruncatedUtf16);
            }
            return Ok(prepared);
        }

        let built = if let Some(candidate) = table_candidate { Ok(candidate) } else { self.build_verified_text_edit_candidate(
            source,
            after_revision,
            &target_text,
            text_splice_work,
            &edits,
            &source_patches,
            authored_syntax,
        ) };
        let TextEditCandidate {
            state: mut candidate,
            work: projection_work,
            block_ids_already_reconciled,
            next_projected_block_id: reconciled_next_projected_block_id,
        } = built?;
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
            reconciled_next_projected_block_id
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
                conversion_warnings: if repaired_utf16 {
                    vec![super::ConversionWarning::RepairedTruncatedUtf16]
                } else {
                    Vec::new()
                },
            },
            text_position_map,
            None,
            next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    /// Preserve existing hard-break syntax when a multiline replacement keeps
    /// the same boundaries. Re-serializing every break as a paragraph separator
    /// changes indented lines and can consume neighboring list/inline syntax.
    fn markdown_retained_break_rewrite_patches(
        &self,
        range: &Range<usize>,
        replacement: &str,
        replacement_breaks: &[usize],
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if self.format() != Format::Markdown || replacement_breaks.is_empty() {
            return Ok(None);
        }
        let old = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let breaks = self.projection().hard_breaks_for_region(range);
        // Literal LF content in a Mac file is not a hard-line boundary.
        if breaks.len() != replacement_breaks.len()
            || old.matches('\n').count() != breaks.len()
            || replacement.matches('\n').count() != replacement_breaks.len()
        {
            return Ok(None);
        }
        let mut patches = Vec::new();
        let mut old_at = range.start;
        let mut replacement_at = 0;
        for (old_end, replacement_end) in breaks.iter().copied().chain([range.end]).zip(
            replacement_breaks
                .iter()
                .copied()
                .chain([replacement.len()]),
        ) {
            let segment = &replacement[replacement_at..replacement_end];
            let old_segment = old_at..old_end;
            if &old[old_at - range.start..old_end - range.start] != segment {
                if old_segment.is_empty() {
                    if let Some(empty) = markdown_list_structure::empty_insertion_patches(
                        self,
                        &old_segment,
                        segment,
                    )? {
                        patches.extend(empty);
                    } else {
                        let Some(at) = self.projection().source_insertion_point(old_at, true)
                        else {
                            return Ok(None);
                        };
                        let syntax = self.escape_markdown_source_text(at, segment)?;
                        patches.push(SourcePatch::primary(
                            at..at,
                            self.encoding().encode_fragment(&syntax)?,
                        ));
                    }
                } else if let Some(local) =
                    self.markdown_line_local_text_rewrite_patches(&old_segment, segment)?
                {
                    patches.extend(local);
                } else {
                    return Ok(None);
                }
            }
            if old_end < range.end && segment.is_empty() {
                // A retained two-space hard break becomes an ordinary blank
                // line once its preceding body disappears. An explicit break
                // keeps its original inline role and exact visible boundary.
                if let Some(source) = self.projection().source_range(old_end..old_end + 1) {
                    let bytes = self
                        .state()
                        .source
                        .bytes_in(source.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let decoded = self.encoding().decode_region(&bytes, source.start)?;
                    let normalized = normalize(&decoded, self.file_format());
                    if normalized.text == "  \n" || normalized.text == "\\\n" {
                        let syntax = "<br>";
                        patches.push(SourcePatch::primary(
                            source,
                            self.encoding().encode_fragment(&syntax)?,
                        ));
                    }
                }
            }
            old_at = old_end + 1;
            replacement_at = replacement_end + 1;
        }
        Ok(Some(patches))
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
            let syntax = if in_code {
                segment
            } else if !segment.is_empty()
                && segment.chars().all(|ch| matches!(ch, ' ' | '\t'))
                && self
                    .projection()
                    .hard_line_at_offset(run.formatted.start)
                    .and_then(|index| self.projection().hard_line_range(index))
                    .is_some_and(|line| {
                        formatted_range.start <= line.start && line.end <= formatted_range.end
                    })
            {
                // Whitespace replacing a complete visible line is authored
                // text, not an empty physical line or hard-break padding.
                segment
                    .chars()
                    .map(|ch| if ch == ' ' { "&#32;" } else { "&#9;" })
                    .collect()
            } else {
                self.escape_markdown_source_text(run.source.start, &segment)?
            };
            patches.push(SourcePatch::primary(
                run.source,
                self.state().encoding.encode_fragment(&syntax)?,
            ));
        }
        debug_assert_eq!(replacement_at, replacement_graphemes.len());
        Ok(Some(patches))
    }

    /// Escape an authored list-looking prefix on a physical continuation line.
    /// Its displayed position can be mid-paragraph even though Markdown parses
    /// the underlying source at line start. Inspect only a bounded prefix;
    /// deeper indentation uses a conservative escaped authored punctuation.
    pub(super) fn escape_markdown_source_text(
        &self,
        source_at: usize,
        text: &str,
    ) -> Result<String, DocumentError> {
        let mut escaped = escape_markdown_insert_in_encoding(text, self.encoding());
        if self.format()==Format::Markdown {
            let tables = self.projection().tables();
            let cell = tables.partition_point(|table| table.source_range.start <= source_at)
                .checked_sub(1).and_then(|index| tables.get(index))
                .filter(|table| source_at <= table.source_range.end)
                .and_then(|table| table.rows.partition_point(|row| row.source_range.start <= source_at)
                    .checked_sub(1).and_then(|index| table.rows.get(index)))
                .and_then(|row| row.cells.get(row.cells.partition_point(|cell| cell.source_range.end < source_at)))
                .filter(|cell| cell.source_range.start <= source_at && source_at <= cell.source_range.end);
            if let Some(cell)=cell {
                escaped=escaped.replace('|',"\\|").replace('\n',"<br>");
                if source_at==cell.source_range.start || source_at==cell.source_range.end {
                    let leading=escaped.len()-escaped.trim_start_matches([' ','\t']).len();
                    let trailing=escaped.trim_end_matches([' ','\t']).len().max(leading);
                    let protect=|value:&str|value.chars().map(|ch|if ch==' ' {"&#32;"}else{"&#9;"}).collect::<String>();
                    escaped=format!("{}{}{}",protect(&escaped[..leading]),&escaped[leading..trailing],protect(&escaped[trailing..]));
                }
                return Ok(escaped);
            }
        }
        let Some(line) = self.state().source_hard_lines.line_at_offset(source_at) else {
            return Err(DocumentError::AmbiguousProjection);
        };
        let physical = self
            .state()
            .source_hard_lines
            .get(line)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = if source_at - physical.start <= 128 {
            let bytes = self
                .state()
                .source
                .bytes_in(physical.start..source_at)
                .ok_or(DocumentError::AmbiguousProjection)?;
            self.encoding().decode_region(&bytes, physical.start)?.text
        } else {
            // No whole-line prefix scan for late edits in a giant source line.
            // Escaping punctuation authored at this boundary is semantically
            // harmless even when the inaccessible prefix was ordinary prose.
            String::new()
        };
        let mut body_start = super::markdown_quotes::prefix(&prefix);
        while let Some(marker) = super::markdown_blocks::marker_prefix_length(&prefix[body_start..]) {
            body_start += marker;
            body_start += super::markdown_quotes::prefix(&prefix[body_start..]);
        }
        let body_prefix = &prefix[body_start..];
        let list_padding = super::markdown_blocks::marker_prefix_length(body_prefix)
            == Some(body_prefix.len())
            || (body_prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
                && self
                    .projection()
                    .map_source_boundary(self.revision(), source_at, BoundaryAffinity::Downstream)
                    .ok()
                    .is_some_and(|point| {
                        super::edit_boundary::paragraph_at(self, point.formatted_offset)
                            .ok()
                            .flatten()
                            .is_some_and(|block| {
                                matches!(block.kind, super::BlockKind::ListItem { .. })
                            })
                    }));
        let leading = escaped.len() - escaped.trim_start_matches([' ', '\t']).len();
        let begins_indented_code = body_prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
            && body_prefix
                .bytes()
                .chain(escaped[..leading].bytes())
                .fold(0usize, |column, byte| {
                    column + if byte == b'\t' { 4 - column % 4 } else { 1 }
                })
                >= 4;
        if list_padding || begins_indented_code || body_prefix.trim().is_empty() {
            // Indentation must not reinterpret authored prose as code. List
            // indentation and marker padding are also hidden source syntax.
            // Authored body-leading whitespace must be content instead of
            // extending that padding; references preserve exact characters.
            if leading > 0 {
                let mut body = String::new();
                for whitespace in escaped[..leading].bytes() {
                    body.push_str(if whitespace == b' ' { "&#32;" } else { "&#9;" });
                }
                body.push_str(&escaped[leading..]);
                escaped = body;
            }
        }
        if escaped.ends_with([' ', '\t']) && physical.end - source_at <= 128 {
            let bytes = self
                .state()
                .source
                .bytes_in(source_at..physical.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let tail = self.encoding().decode_region(&bytes, source_at)?.text;
            let tail = tail.trim_end_matches(['\r', '\n']);
            if tail == "\\"
                || tail.len() >= 2 && tail.bytes().all(|b| b == b' ')
                || line + 1 < self.state().source_hard_lines.len()
                    && self.file_format() != FileFormat::Mac
                    && tail.trim().is_empty()
                || prefix.trim_start().starts_with('#') && tail.trim().is_empty()
            {
                let end = escaped.trim_end_matches([' ', '\t']).len();
                let spaces = escaped[end..]
                    .bytes()
                    .map(|b| if b == b' ' { "&#32;" } else { "&#9;" })
                    .collect::<String>();
                escaped.truncate(end);
                escaped.push_str(&spaces);
            }
        }
        let prefix = prefix.trim_start_matches([' ', '\t']);
        if prefix.is_empty() {
            let whitespace = escaped.len() - escaped.trim_start_matches([' ', '\t']).len();
            let body = &escaped[whitespace..];
            let digits = body.bytes().take_while(u8::is_ascii_digit).count();
            let punctuation = if body.starts_with(['-', '+']) {
                Some(whitespace)
            } else if (1..=9).contains(&digits)
                && matches!(body.as_bytes().get(digits), Some(b'.' | b')'))
            {
                Some(whitespace + digits)
            } else {
                None
            };
            if let Some(at) = punctuation {
                escaped.insert(at, '\\');
            }
        } else if prefix.len() <= 9
            && prefix.bytes().all(|byte| byte.is_ascii_digit())
            && escaped.starts_with(['.', ')'])
        {
            escaped.insert(0, '\\');
        }
        Ok(escaped)
    }

    /// Source fence edits may reinterpret later paragraph separators. Both
    /// literal and structured payload edits publish the same parsed source
    /// result instead of imposing a pre-edit paragraph/break partition.
    fn source_edit_requires_reprojection(
        &self,
        edits: &[TextEdit],
        patches: &[SourcePatch],
    ) -> Result<bool, ModelTransactionError> {
        if self.format() != Format::MarkdownSource {
            return Ok(false);
        }
        if edits.len()==1 && patches.len()==1 {
            let edit=&edits[0];
            if !edit.replacement.is_empty() && edit.replacement.chars().all(char::is_alphanumeric)
                && self.projection().table_cell_at(edit.range.start).is_some_and(|(_,_,cell)|edit.range.end<=cell.range.end && self.table_source_text(cell.source_range.clone()).is_ok_and(|text|text.chars().all(|ch|ch.is_alphanumeric()||matches!(ch,' '|'\t'))))
                && self.encoding().encode_fragment(&edit.replacement)?==patches[0].replacement {return Ok(false);}
        }
        let Some(region) = self.line_local_projection_region(edits, patches)? else {
            return Ok(true);
        };
        // A region that cannot be proven independent of the rest of the
        // document is treated like an edit without a local region.
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_end =
            rebase_source_boundary(region.old_source.end, patches, Association::AfterInsertion)?;
        let Some(bytes) = source.bytes_in(region.old_source.start..new_end) else {
            return Ok(true);
        };
        let decoded = self
            .state()
            .encoding
            .decode_region(&bytes, region.old_source.start)?;
        let regional = project(
            &normalize(&decoded, self.state().file_format),
            self.state().format,
            Revision(self.next_revision),
            region.old_source.start,
            new_end,
        );
        let inherited_blocks = self.can_inherit_markdown_source_list_context(edits);
        if self
            .verify_markdown_source_region(&region, &regional, edits, inherited_blocks)
            .is_err()
        {
            return Ok(true);
        }
        // Source edits can change folded separator whitespace without changing
        // the regional row count or owners. Prove the literal splice as well
        // before selecting the text-preserving candidate path.
        let mut expected = self.projection().text_tree()
            .slice(region.old_formatted.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        for edit in edits.iter().rev() {
            if edit.range.start < region.old_formatted.start
                || edit.range.end > region.old_formatted.end
            {
                return Ok(true);
            }
            expected.replace_range(edit.range.start - region.old_formatted.start
                ..edit.range.end - region.old_formatted.start, &edit.replacement);
        }
        Ok(regional.text_tree().slice(0..regional.text_tree().byte_len())
            .map_err(DocumentError::FormattedTextStorage)? != expected)
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
            self.validate_formatted_replacement_characters(edit.payload.text())?;
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
        let mut only_text_changing_payloads = true;
        edits.retain(|edit| {
            let captured = snapshot
                .capture(edit.range.clone())
                .expect("document range validation also validates payload capture boundaries");
            let changes_text = captured.text() != edit.payload.text();
            let keep = changes_text
                || captured.break_offsets() != edit.payload.break_offsets()
                || self
                    .decoding_diagnostics()
                    .iter()
                    .any(|diagnostic| ranges_overlap(&edit.range, &diagnostic.formatted_range));
            if keep && !changes_text {
                only_text_changing_payloads = false;
            }
            keep
        });
        if edits.is_empty() {
            return Ok(self.no_op_prepared());
        }

        let logical_edits = edits
            .iter()
            .map(FormattedPayloadEdit::text_edit)
            .collect::<Vec<_>>();
        let support = self.markdown_structural_support(&logical_edits)?;
        if !support.is_empty() {
            return self.prepare_markdown_supporting_patches(support, |doc| {
                doc.prepare_formatted_payload_edits(edits.clone())
            });
        }
        if edits.iter().all(|edit| {
            edit.payload.break_offsets().iter().copied().eq(edit
                .payload
                .text()
                .match_indices('\n')
                .map(|(at, _)| at))
        }) {
            if let Some(prepared) = self.prepare_markdown_html_text_edits(&logical_edits)? {
                return Ok(prepared);
            }
        }
        let indented = self.indented_code_requiring_fences(&logical_edits)?;
        if !indented.is_empty() {
            return self.prepare_with_fenced_indented_code(&indented, |scratch| {
                let mut rebound = edits.clone();
                for edit in &mut rebound {
                    edit.payload = super::FormattedTextPayload::new(
                        &scratch.hard_line_snapshot(),
                        edit.payload.text(),
                        edit.payload.break_offsets().to_vec(),
                    )
                    .expect("rebinding preserves validated payload boundaries");
                }
                scratch.prepare_formatted_payload_edits(rebound)
            });
        }

        // Literal payloads with ordinary logical newlines have the same
        // graph as a text splice. Keep typing, IME and paste on the persistent
        // regional path; rich-payload verification must not flatten/reproject
        // an entire buffer for every inserted character or newline.
        if self.format().is_literal()
            && only_text_changing_payloads
            && edits.iter().all(|edit| {
                !edit.payload.text().contains('\r')
                    && edit.payload.break_offsets().iter().copied().eq(edit
                        .payload
                        .text()
                        .match_indices('\n')
                        .map(|(at, _)| at))
            })
        {
            let patches = self.translate_source_edits(
                logical_edits
                    .iter()
                    .zip(&edits)
                    .map(|(text, payload)| (text, Some(payload))),
            )?;
            return self.prepare_text_edits_with_patches(logical_edits, Some(patches));
        }

        let without_breaks = edits.iter().all(|edit| {
            !edit.payload.text().contains('\n') && edit.payload.break_offsets().is_empty()
        });
        // Markdown Source delimiters are literal, editable text, so such a
        // payload is an ordinary text splice. Text-edit preparation
        // keeps a proven line-local edit regional and reparses the complete
        // source itself when a delimiter could reshape other lines. An edit
        // it cannot verify keeps the payload reparse below as its reference.
        // Every Markdown Source U+000A is a hard-line boundary, so a payload
        // whose breaks are exactly its newlines is also a text splice.
        let breaks_are_newlines = edits.iter().all(|edit| {
            !edit.payload.text().contains('\r')
                && edit.payload.break_offsets().iter().copied().eq(edit
                    .payload
                    .text()
                    .match_indices('\n')
                    .map(|(at, _)| at))
        });
        if self.format() == Format::MarkdownSource && (without_breaks || breaks_are_newlines) {
            let patches = self.translate_source_edits(
                logical_edits
                    .iter()
                    .zip(&edits)
                    .map(|(text, payload)| (text, Some(payload))),
            )?;
            if let Ok(prepared) =
                self.prepare_text_edits_with_patches(logical_edits.clone(), Some(patches))
            {
                return Ok(prepared);
            }
        }
        if self.format().is_wysiwyg() && without_breaks {
            if let Some(prepared) = self.prepare_structural_text_batch(&logical_edits)? {
                return Ok(prepared);
            }
            let patches = self.translate_source_edits(
                logical_edits
                    .iter()
                    .zip(&edits)
                    .map(|(text, payload)| (text, Some(payload))),
            )?;

            // Markdown reaches its bounded regional candidates through the same
            // text-edit preparation. An edit that route cannot verify, such as
            // a delimiter typed into a fenced list item, keeps the complete
            // reparse below as its reference behavior.
            if let Ok(prepared) =
                self.prepare_text_edits_with_patches(logical_edits.clone(), Some(patches))
            {
                return Ok(prepared);
            }
        }

        let text_edits = logical_edits;
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

        let mut source_patches = self.translate_source_edits(
            text_edits
                .iter()
                .zip(&edits)
                .map(|(text, payload)| (text, Some(payload))),
        )?;
        self.preserve_markdown_edit_boundaries(
            &text_edits,
            edits
                .iter()
                .filter(|edit| !edit.payload.break_offsets().is_empty())
                .map(|edit| &edit.range),
            &mut source_patches,
        )?;
        self.simplify_markdown_edit_spelling(&text_edits, &mut source_patches)?;
        let repaired_utf16 =
            self.repair_incomplete_utf16_insertions(&text_edits, &mut source_patches)?;
        validate_source_patches(&mut source_patches)?;
        // A literal CR can combine with a following bare LF under DOS.
        // Source grammar may reshape paragraphs, but must not silently consume
        // an explicitly requested character through line-ending normalization.
        if edits.iter().all(|edit| !edit.payload.text().contains('\r'))
            && self.source_edit_requires_reprojection(&text_edits, &source_patches)?
        {
            let mut prepared = self.prepare_reprojected_source_patches(source_patches)?;
            if repaired_utf16 {
                prepared
                    .summary
                    .conversion_warnings
                    .push(super::ConversionWarning::RepairedTruncatedUtf16);
            }
            return Ok(prepared);
        }

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
                conversion_warnings: if repaired_utf16 {
                    vec![super::ConversionWarning::RepairedTruncatedUtf16]
                } else {
                    Vec::new()
                },
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
        let Some(plan) = transfer::plan(self, operation, source_lines, destination)? else {
            return Ok(self.no_op_prepared());
        };
        self.prepare_hard_line_transfer_plan(plan)
    }

    fn prepare_hard_line_transfer_plan(
        &self,
        mut plan: transfer::HardLineTransferPlan,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut source_patches = plan
            .source_patches
            .drain(..)
            .map(|patch| SourcePatch::primary(patch.range, patch.replacement))
            .collect::<Vec<_>>();
        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let mut candidate = if self.format() == Format::MarkdownSource {
            let decoded = self.encoding().decode(&source.bytes())?;
            let candidate = build_state_from_decoded(
                source,
                decoded,
                self.format(),
                self.file_format(),
                self.file_format_origin(),
                self.line_ending_evidence(),
                after_revision,
            )?;
            if candidate.projection.text() != plan.expected_text {
                // Moving source fences can change whether separator bytes
                // are literal code whitespace or folded paragraph boundaries.
                // The transferred source remains authoritative; reconcile its
                // new paragraph partition rather than impose the old row count.
                return self.prepare_reprojected_source_candidate(
                    source_patches,
                    candidate,
                    ModelChangeKind::HardLineTransfer,
                );
            }
            candidate
        } else {
            self.build_verified_candidate(
                source,
                self.state().file_format,
                self.state().file_format_origin,
                after_revision,
                CandidateVerification {
                    expected_text: plan.expected_text.clone(),
                    expected_hard_breaks: Some(&plan.expected_hard_breaks),
                    mismatch_error: DocumentError::HardLineTransferProjectionMismatch,
                },
            )?
        };
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
        let source_mode_styles = plan.expected_signatures.is_none().then(|| {
            (
                candidate.projection.style_sheet().clone(),
                candidate.projection.document_style().clone(),
            )
        });
        let next_projected_block_id = candidate
            .projection
            .install_transferred_block_ids(
                self.projection(),
                &plan.origins,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        if let Some((sheet, document_style)) = source_mode_styles {
            candidate.projection.install_configuration_styles(
                after_revision,
                sheet,
                document_style,
            );
        }
        let projection_work = ProjectionWorkStatistics::full(&candidate);
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::HardLineTransfer,
                source_patches,
                formatted_splices,
                projection_work,
                style_change: None,
                conversion_warnings: Vec::new(),
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
                conversion_warnings: Vec::new(),
            },
            map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    pub(super) fn prepare_visible_source_patches(
        &self,
        patches: Vec<SourcePatch>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut edits = Vec::new();
        for patch in &patches {
            let start = self
                .projection()
                .map_source_boundary(
                    self.revision(),
                    patch.range.start,
                    BoundaryAffinity::Downstream,
                )
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .formatted_offset;
            let end = self
                .projection()
                .map_source_boundary(self.revision(), patch.range.end, BoundaryAffinity::Upstream)
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .formatted_offset;
            let decoded = self
                .encoding()
                .decode_region(&patch.replacement, patch.range.start)?;
            edits.push(TextEdit::new(
                start..end,
                normalize(&decoded, self.file_format()).text,
            ));
        }
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    fn prepare_markdown_paragraph_style(
        &self,
        range: Range<usize>,
        style: StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format().is_markdown() {
            if let Some(prepared) = self.prepare_exclusive_structural_style(
                range.clone(),
                structural_style::Assignment::Paragraph(style.clone()),
            )? {
                return Ok(prepared);
            }
        }
        self.prepare_markdown_paragraph_style_raw(range, style)
    }

    /// Resolve and decode the source hard line backing one projected hard line.
    ///
    /// Markdown's projection hides syntax, so a projected line maps through an
    /// insertion point rather than by index; every other format keeps them in
    /// step. Paragraph-style and list-style rewriting both need this and must
    /// agree on which source line they are about to edit.
    fn decoded_source_hard_line(
        &self,
        line: &Range<usize>,
        index: usize,
    ) -> Result<(Range<usize>, super::DecodedText), DocumentError> {
        let source_index = if self.format().is_markdown() {
            let source_at = self
                .projection()
                .source_insertion_point(line.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            self.state()
                .source_hard_lines
                .line_at_offset(source_at)
                .ok_or(DocumentError::AmbiguousProjection)?
        } else {
            index
        };
        let source_line = self
            .state()
            .source_hard_lines
            .get(source_index)
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
        Ok((source_line, decoded))
    }

    fn prepare_markdown_paragraph_style_raw(
        &self,
        mut range: Range<usize>,
        style: StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {

        self.validate_range(&range)?;
        if !self.format().is_markdown() || !style.supports_markdown_paragraph_assignment() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if style.0 == "Code Block" {
            return self.prepare_markdown_code_style(&range, true);
        }
        if self.format() == Format::Markdown {
            let selected = self
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
            if let (Some(first), Some(last)) = (selected.first(), selected.last()) {
                range = first.range.start..last.range.end;
            }
        }
        if style == self.projection().style_sheet().base_paragraph
            && !structural_style::selected_blocks(self, &range)
                .iter()
                .any(|block| block.style.0.starts_with("Heading"))
        {
            if let Some(prepared) = self.prepare_markdown_list_as_prose(&range)? {
                return Ok(prepared);
            }
            if structural_style::selected_blocks(self, &range)
                .iter()
                .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }))
            {
                return self.prepare_list_style_raw(range, None);
            }
        }
        if style.0 == "Block quote"
            || style.0 == "Paragraph"
                && self
                    .projection()
                    .blocks_for_region(&range)
                    .iter()
                    .any(|block| {
                        (block.style.0 == "Block quote" || block.quote_depth > 0)
                            && matches!(block.kind, super::BlockKind::Paragraph)
                            && block.style.0 != "Code Block"
                    })
        {
            return self.prepare_markdown_quote_style(range, style.0 == "Block quote");
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
        let mut patches = if self.format().is_markdown() {
            markdown_block_styles::support_patches(
                self,
                &range,
                self.format() == Format::Markdown && level > 0,
                level == 0,
            )?
        } else {
            Vec::new()
        };
        for index in first..self.projection().hard_line_count() {
            let line = self
                .projection()
                .hard_line_range(index)
                .ok_or(DocumentError::VerificationFailed)?;
            if index != first && line.start >= range.end {
                break;
            }
            let (source_line, decoded) = self.decoded_source_hard_line(&line, index)?;
            let mut container_prefix = super::markdown_quotes::prefix(&decoded.text);
            let in_list = self
                .projection()
                .blocks_for_region(&line)
                .iter()
                .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }));
            if in_list {
                container_prefix +=
                    super::markdown_blocks::marker_prefix_length(&decoded.text[container_prefix..])
                        .unwrap_or(0);
            }
            let body = &decoded.text[container_prefix..];
            let (old_prefix, _) = super::projection::markdown_block_prefix(body, 0, body.len());
            let old_prefix =
                super::markdown_blocks::marker_prefix_length(body).unwrap_or(old_prefix);
            if body[..old_prefix] == prefix {
                continue;
            }
            let source_start = source_line.start
                + self
                    .encoding()
                    .encode_fragment(&decoded.text[..container_prefix])?
                    .len();
            let removed_bytes = self.encoding().encode_fragment(&body[..old_prefix])?.len();
            patches.push(SourcePatch::primary(
                source_start..source_start + removed_bytes,
                self.state().encoding.encode_fragment(&prefix)?,
            ));
            let visible = self
                .projection()
                .text_tree()
                .slice(line.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let mut visible_container = if self.format() == Format::MarkdownSource {
                super::markdown_quotes::prefix(&visible)
            } else {
                0
            };
            if self.format() == Format::MarkdownSource && in_list {
                visible_container +=
                    super::markdown_blocks::marker_prefix_length(&visible[visible_container..])
                        .unwrap_or(0);
            }
            let visible_body = &visible[visible_container..];
            let (visible_prefix, _) =
                super::projection::markdown_block_prefix(visible_body, 0, visible_body.len());
            let remove = if self.format() == Format::MarkdownSource {
                visible_prefix
            } else {
                0
            };
            let replacement = if self.format() == Format::MarkdownSource {
                prefix.clone()
            } else {
                String::new()
            };
            edits.push(TextEdit::new(
                line.start + visible_container..line.start + visible_container + remove,
                replacement,
            ));
        }
        if self.format() == Format::Markdown && level > 0 {
            self.prepare_markdown_heading_patches(patches, &range, &style)
        } else {
            self.prepare_text_edits_with_patches(edits, Some(patches))
        }
    }

    fn prepare_list_style(
        &self,
        range: Range<usize>,
        style: Option<super::ListStyle>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if let Some(prepared) = self.prepare_exclusive_structural_style(
            range.clone(),
            structural_style::Assignment::List(style),
        )? {
            return Ok(prepared);
        }
        self.prepare_list_style_raw(range, style)
    }

    fn prepare_list_style_raw(
        &self,
        range: Range<usize>,
        style: Option<super::ListStyle>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if style.is_none() {
            if let Some(prepared) = self.prepare_markdown_list_as_prose(&range)? {
                return Ok(prepared);
            }
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
        let mut patches = if self.format().is_markdown() {
            markdown_block_styles::support_patches(
                self,
                &range,
                self.format() == Format::Markdown && style.is_some(),
                style.is_none(),
            )?
        } else {
            Vec::new()
        };
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
            let visible_container = if self.format() == Format::MarkdownSource {
                super::markdown_quotes::prefix(&visible)
            } else {
                0
            };
            let visible_body = &visible[visible_container..];
            let (visible_prefix, visible_kind) =
                super::projection::markdown_block_prefix(visible_body, 0, visible_body.len());
            let indent = visible_body
                .bytes()
                .take_while(|byte| *byte == b' ')
                .count();
            let already_list = self
                .projection()
                .blocks_for_region(&line)
                .iter()
                .any(|block| {
                    block.range.start <= line.start
                        && line.end <= block.range.end
                        && matches!(block.kind, super::BlockKind::ListItem { .. })
                });
            let remove_visible = if self.format() == Format::Markdown {
                if style.is_some() && !already_list {
                    indent
                } else {
                    0
                }
            } else if matches!(visible_kind, super::BlockKind::ListItem { .. })
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
                Some(_) if self.format() == Format::Markdown => String::new(),
                Some(super::ListStyle::Bullet) => format!("{}- ", " ".repeat(indent)),
                Some(super::ListStyle::Numbered) => format!("{}{ordinal}. ", " ".repeat(indent)),
                None => {
                    if remove_visible > 0 && self.format() != Format::Markdown {
                        " ".repeat(indent)
                    } else {
                        String::new()
                    }
                }
            };
            ordinal += 1;
            if self.format() != Format::Markdown && visible_body[..remove_visible] == prefix {
                continue;
            }
            let (source_line, decoded) = self.decoded_source_hard_line(&line, index)?;
            let source_container = if self.format().is_markdown() {
                super::markdown_quotes::prefix(&decoded.text)
            } else {
                0
            };
            let source_body = &decoded.text[source_container..];
            let (source_prefix, source_kind) =
                super::projection::markdown_block_prefix(source_body, 0, source_body.len());
            let complete_marker = super::markdown_blocks::marker_prefix_length(source_body);
            let remove_source = if let Some(prefix) = complete_marker {
                prefix
            } else if matches!(source_kind, super::BlockKind::ListItem { .. })
                || (style.is_some()
                    && self.format() != Format::PlainText
                    && matches!(source_kind, super::BlockKind::Heading(_)))
            {
                source_prefix
            } else if style.is_some() {
                source_body.bytes().take_while(|byte| *byte == b' ').count()
            } else {
                0
            };
            let removed_bytes = self
                .state()
                .encoding
                .encode_fragment(&source_body[..remove_source])?
                .len();
            let source_prefix = if self.format() == Format::Markdown {
                let indent = source_body.bytes().take_while(|byte| *byte == b' ').count();
                match style {
                    Some(super::ListStyle::Bullet) => format!("{}- ", " ".repeat(indent)),
                    Some(super::ListStyle::Numbered) => {
                        format!("{}{}. ", " ".repeat(indent), ordinal - 1)
                    }
                    None => " ".repeat(indent),
                }
            } else {
                prefix.clone()
            };
            if source_body[..remove_source] == source_prefix {
                continue;
            }
            let source_start = source_line.start
                + self
                    .encoding()
                    .encode_fragment(&decoded.text[..source_container])?
                    .len();
            patches.push(SourcePatch::primary(
                source_start..source_start + removed_bytes,
                self.state().encoding.encode_fragment(&source_prefix)?,
            ));
            edits.push(TextEdit::new(
                line.start + visible_container..line.start + visible_container + remove_visible,
                prefix,
            ));
        }
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        let mut combined: Vec<SourcePatch> = Vec::new();
        for patch in patches {
            if let Some(previous) = combined.last_mut().filter(|previous| {
                previous.range.is_empty() && previous.range.start == patch.range.start
            }) {
                previous.range.end = patch.range.end;
                previous.replacement.extend(patch.replacement);
            } else {
                combined.push(patch);
            }
        }
        self.prepare_text_edits_with_patches(edits, Some(combined))
    }

    fn prepare_structural_list_enter(
        &self, at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format() == Format::MarkdownSource {
            return self.prepare_markdown_source_list_enter(at);
        }
        if let Some(prepared) = self.prepare_markdown_quote_enter(at)? {
            return Ok(prepared);
        }
        if self.format() == Format::Markdown {
            return self.prepare_markdown_list_enter(at);
        }
        Err(DocumentError::UnsupportedFormatting.into())
    }

    fn prepare_semantic_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if self.format() == Format::Markdown && !range.is_empty()
            && self.projection().table_cell_at(range.start).is_some_and(|(_,_,cell)| range.end <= cell.range.end)
            && matches!(style, SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis) {
            return self.prepare_table_semantic_style(range, style, enabled);
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
                    if !candidate
                        .projection
                        .style_spans_for_region(&(range.start..range.end + added))
                        .iter()
                        .any(|span| {
                            span.application == StyleApplication::Semantic(style)
                                // Adjacent emphasis delimiters can join into a
                                // triple run whose source-visible style owner
                                // also includes the original outer markers.
                                && span.range.start <= range.start
                                && range.end + added <= span.range.end
                        })
                    {
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
        if enabled
            && matches!(style, SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis)
        {
            let coverage = semantic_style_coverage(
                &self.projection().style_spans_for_region(&range),
                style,
            );
            if coverage.iter().any(|span| ranges_overlap(span, &range)) {
                // Author only the uncovered runs. The typing adapter can move
                // an adjacent closing delimiter, or repair it with passive
                // HTML when Markdown's delimiter rules require that.
                let mut scratch = self.scratch_document();
                let mut sources = replacement::PatchComposition::new(self.source_byte_len());
                for uncovered in subtract_style_coverage(&range, &coverage).into_iter().rev() {
                    let prepared = scratch.prepare_typing_markdown_style(uncovered, style, true)?;
                    for patch in prepared.summary.source_patches.iter().rev() {
                        sources.splice(patch.range(), patch.replacement());
                    }
                    scratch.commit_model_transaction(prepared)?;
                }
                source_patches = sources.source_patches();
            }
        }

        validate_source_patches(&mut source_patches)?;
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let after_revision = Revision(self.next_revision);
        let unchanged = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let edits = [TextEdit::new(range.clone(), unchanged)];
        let built = self.build_verified_text_edit_candidate(
            source,
            after_revision,
            self.projection().text_tree(),
            FormattedTextSpliceStats::default(),
            &edits,
            &source_patches,
            false,
        )?;
        let verification_range = if built.work.scope == ProjectionWorkScope::RegionalHardLines {
            self.line_local_projection_region(&edits, &source_patches)?
                .map(|region| region.old_formatted)
        } else {
            None
        };
        let candidate = built.state;
        let before_styles = verification_range
            .as_ref()
            .map(|region| self.projection().style_spans_for_region(region));
        let after_styles = verification_range
            .as_ref()
            .map(|region| candidate.projection.style_spans_for_region(region));
        if !semantic_style_edit_was_exactly_projected(
            before_styles
                .as_deref()
                .unwrap_or_else(|| self.projection().style_spans()),
            after_styles
                .as_deref()
                .unwrap_or_else(|| candidate.projection.style_spans()),
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
        let projection_work = built.work;
        Ok(self.prepared(
            after_revision,
            ModelChangeSummary {
                kind: ModelChangeKind::SemanticStyle,
                source_patches,
                formatted_splices: Vec::new(),
                projection_work,
                style_change: None,
                conversion_warnings: Vec::new(),
            },
            text_position_map,
            None,
            built.next_projected_block_id,
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

        self.validate_range(&range)?;
        if self.format() == Format::Markdown && !range.is_empty()
            && self.projection().table_cell_at(range.start).is_some_and(|(_,_,cell)| range.end <= cell.range.end)
            && matches!(style, SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis) {
            for fragment in self.table_style_fragments(&range)? {
                let spans = self.projection().style_spans_for_region(&fragment);
                let containing = spans.iter().find(|span|
                    span.application == StyleApplication::Semantic(style)
                        && span.range.start <= fragment.start && fragment.end <= span.range.end);
                if enabled == containing.is_some() { continue; }
                if !enabled {
                    let source = self.projection().source_range(containing.unwrap().range.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    self.markdown_style_removal_patches(&source, style)?;
                } else {
                    self.projection().source_range(fragment.clone()).ok_or(DocumentError::AmbiguousProjection)?;
                    if spans.iter().any(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                        && (span.range.start < fragment.start || fragment.end < span.range.end)) {
                        return Err(DocumentError::UnsupportedFormatting.into());
                    }
                }
            }
            return Ok(());
        }
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
        let spans = self.projection().style_spans_for_region(range);
        let containing = spans.iter().find(|span| {
            span.application == StyleApplication::Semantic(style)
                && span.range.start <= range.start
                && range.end <= span.range.end
        });
        if enabled {
            if containing.is_some() {
                return Ok(Vec::new());
            }
            if spans
                .iter()
                .any(|span| span.range.start < range.end && range.start < span.range.end
                    && (!matches!(style, SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis)
                        || span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)))
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

        if self.state().format != Format::Markdown {
            return Err(DocumentError::UnsupportedFormatting.into());
        }

        let spans = self.projection().style_spans_for_region(range);
        let matching = spans
            .iter()
            .filter(|span| {
                span.application == StyleApplication::Semantic(style)
                    && span.range.start <= range.start
                    && range.end <= span.range.end
            })
            .collect::<Vec<_>>();
        if enabled
            && subtract_style_coverage(range, &semantic_style_coverage(&spans, style)).is_empty()
        {
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
            && spans
                .iter()
                .any(|span| span.range.start < range.end && range.start < span.range.end
                && (!matches!(style, SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis)
                    || span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)))
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
        if style == SemanticInlineStyle::Code {
            let (opening, closing) = super::markdown_code::delimiter_ranges(self, source_range)?
                .ok_or(DocumentError::UnsupportedFormatting)?;
            return Ok(vec![
                SourcePatch::primary(opening, Vec::new()),
                SourcePatch::primary(closing, Vec::new()),
            ]);
        }
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

        let tags: &[(&str, &str)] = if style == SemanticInlineStyle::Strong {
            &[("<strong>", "</strong>"), ("<b>", "</b>")]
        } else {
            &[("<em>", "</em>"), ("<i>", "</i>")]
        };
        for (open, close) in tags {
            let a = self.encoding().encode_fragment(open)?;
            let b = self.encoding().encode_fragment(close)?;
            if let Some(start) = source_range.start.checked_sub(a.len()) {
                let opening = start..source_range.start;
                let closing = source_range.end..source_range.end + b.len();
                if self.state().source.bytes_in(opening.clone()).as_ref() == Some(&a)
                    && self.state().source.bytes_in(closing.clone()).as_ref() == Some(&b)
                {
                    matches.push((opening, closing));
                }
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
        if matches!(self.format(), Format::PlainText | Format::Code) {
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
        self.prepare_reprojected_source_patches(patches)
    }

    pub(super) fn prepare_reprojected_source_patches(
        &self,
        mut patches: Vec<SourcePatch>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if patches.is_empty() {
            return Ok(self.no_op_prepared());
        }
        validate_source_patches(&mut patches)?;
        if let Ok(Some(prepared)) = self.prepare_table_local_reprojection(&patches) { return Ok(prepared); }
        let source = apply_source_patches(&self.state().source, &patches)?;
        let decoded = self.encoding().decode(&source.bytes())?;
        let revision = Revision(self.next_revision);
        let candidate = build_state_from_decoded(
            source,
            decoded,
            self.format(),
            self.file_format(),
            self.file_format_origin(),
            self.line_ending_evidence(),
            revision,
        )?;
        self.prepare_reprojected_source_candidate(patches, candidate, ModelChangeKind::TextEdits)
    }

    fn prepare_reprojected_source_candidate(
        &self,
        patches: Vec<SourcePatch>,
        mut candidate: DocumentState,
        kind: ModelChangeKind,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let revision = candidate.revision;
        let edits = source_backed_reprojection_edits_with_patches(
            self.projection(),
            &candidate.projection,
            &patches,
        );
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
                kind,
                source_patches: patches,
                formatted_splices,
                projection_work: work,
                style_change: None,
                conversion_warnings: Vec::new(),
            },
            map,
            None,
            next_id,
            PreparedPublication::State(candidate),
        ))
    }

    fn prepare_markdown_source_change(
        &self, source: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let target = if source { Format::MarkdownSource } else { Format::Markdown };
        self.prepare_view_format_change(target)
    }

    fn prepare_view_format_change(&self, target: Format) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if target == self.format() { return Ok(self.no_op_prepared()); }
        let revision = Revision(self.next_revision);
        let mut candidate = build_state_from_decoded(
            self.state().source.clone(),
            self.encoding().decode(&self.source_bytes())?,
            target, self.file_format(), self.state().file_format_origin,
            self.state().line_ending_evidence, revision,
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
                conversion_warnings: Vec::new(),
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
        // Explicit conversion to Latin-1 authorizes substitution. Ordinary
        // edits continue using the strict encoder and never substitute.
        let lost = if target == super::Encoding::Latin1 {
            decoded.text.chars().filter(|c| *c as u32 > 255).count()
        } else {
            0
        };
        let converted_text = if lost > 0 {
            decoded
                .text
                .chars()
                .map(|c| if c as u32 > 255 { '?' } else { c })
                .collect::<String>()
        } else {
            decoded.text.clone()
        };
        bytes.extend(target.encode_fragment(&converted_text)?);
        let source_patches = byte_difference(&original, &bytes);
        let source = apply_source_patches(&self.state().source, &source_patches)?;
        let revision = Revision(self.next_revision);
        let new_decoded = target.decode(&bytes)?;
        if new_decoded.text != converted_text {
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
        let edits = formatted_text_difference(self.text(), candidate.projection.text());
        candidate
            .projection
            .install_persistent_text_edits(self.projection(), &edits)
            .map_err(DocumentError::FormattedTextStorage)?;
        let formatted_splices = edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &candidate.projection,
            formatted_splices.clone(),
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
                source_patches,
                formatted_splices,
                projection_work: work,
                style_change: None,
                conversion_warnings: if lost == 0 {
                    Vec::new()
                } else {
                    vec![super::ConversionWarning::UnrepresentableCharacters {
                        encoding: target,
                        count: lost,
                    }]
                },
            },
            map,
            None,
            next_id,
            PreparedPublication::State(candidate),
        ))
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
        let current = if self.format().is_literal() {
            normalize_literal(&decoded, self.state().file_format)
        } else {
            normalize(&decoded, self.state().file_format)
        };
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
                conversion_warnings: Vec::new(),
            },
            text_position_map,
            None,
            self.next_projected_block_id,
            PreparedPublication::State(candidate),
        ))
    }

    /// Visible extent of source-only formatting changed by history. Use the
    /// persistent source diff and indexed provenance, not a whole-paragraph
    /// guess for an inline style change. This query is presentation-only.
    pub(crate) fn history_formatting_change_range(&self, from: HistoryNodeId) -> Option<Range<usize>> {
        let previous = self.history.state_at_node(from)?;
        let changed = previous.source.changed_extents(&self.state().source).into_iter()
            .map(|(_, new)| new).reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))?;
        let projection = self.projection();
        let start = projection.nearest_text_boundary_for_source(changed.start, true)?;
        let end = projection.nearest_text_boundary_for_source(changed.end, false)?;
        let range = start.min(end)..start.max(end);
        if range.is_empty() {
            // A changed block prefix has no body bytes of its own, but its
            // treatment affects the containing paragraph or code container.
            if let Some(block) = projection.blocks_for_region(&range).into_iter()
                .find(|block| block.range.start == range.start) {
                return Some(block.range);
            }
        }
        Some(range)
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
        let old_text = self.projection().text_tree();
        let new_text = candidate.projection.text_tree();
        // History restores persistent snapshots. Only their changed boundary
        // paths need comparison; neither source nor text is flattened for undo.
        let formatted_splices = match old_text.changed_extent(new_text) {
            None => Vec::new(),
            Some((old, new)) => vec![Splice::new(old, new.len())?],
        };
        let identity = PositionMap::identity(
            self.id,
            super::PositionDomain::FormattedText,
            self.revision(),
            old_text.byte_len(),
        );
        let text_position_map = self
            .history
            .map_between(current.node, target.node, identity)?;
        debug_assert_eq!(text_position_map.target_revision(), candidate.revision);
        debug_assert_eq!(text_position_map.target_len(), new_text.byte_len());
        let source_patches = self
            .state()
            .source
            .changed_extents(&candidate.source)
            .into_iter()
            .map(|(old, new)| {
                SourcePatch::primary(
                    old,
                    candidate
                        .source
                        .bytes_in(new)
                        .expect("validated source difference"),
                )
            })
            .collect();
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
                conversion_warnings: Vec::new(),
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
                {
                    candidate
                        .projection
                        .install_unchanged_block_ids(self.projection())
                        .map_err(super::block_identity_document_error)?;
                }
            }
        }
        Ok(candidate)
    }

    /// Independently changed literal lines must not turn into one document-sized
    /// projection window. Adjacent edits stay together so simultaneous changes
    /// can create graphemes across their boundaries without an invalid interim
    /// caret. Separated groups are verified in reverse source order, then the
    /// outer transaction supplies the one history entry and original batch map.
    fn build_disjoint_literal_edit_candidate(
        &self,
        source: &super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_splice_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
    ) -> Result<Option<TextEditCandidate>, ModelTransactionError> {
        if !self.format().is_literal() || edits.len() < 2 || source_patches.len() != edits.len() {
            return Ok(None);
        }
        let mut groups = Vec::new();
        let mut first = 0;
        let mut previous_last_line: usize = 0;
        let mut previous_end: usize = 0;
        for (index, edit) in edits.iter().enumerate() {
            let start_line = self
                .projection()
                .hard_line_at_offset(edit.range.start)
                .ok_or(DocumentError::VerificationFailed)?;
            let end_line = self
                .projection()
                .hard_line_at_offset(edit.range.end)
                .ok_or(DocumentError::VerificationFailed)?;
            if index > first
                && (start_line > previous_last_line.saturating_add(1)
                    || edit.range.start
                        > previous_end.saturating_add(2 * LITERAL_EDIT_CONTEXT_BYTES))
            {
                groups.push(first..index);
                first = index;
            }
            previous_last_line = end_line;
            previous_end = edit.range.end;
        }
        groups.push(first..edits.len());
        if groups.len() < 2 {
            return Ok(None);
        }
        // Literal text uses monotonic one-patch translations. Keep uncommon
        // supporting patch sets on the existing fully verified path.
        for group in &groups {
            if self
                .line_local_projection_region(
                    &edits[group.clone()],
                    &source_patches[group.clone()],
                )?
                .is_none()
            {
                return Ok(None);
            }
        }
        let mut scratch = self.scratch_document();
        let mut work = ProjectionWorkStatistics::none();
        // The caller already constructed the simultaneous target rope; count
        // that work as well as each regional candidate's persistent updates.
        work.formatted_text_nodes_visited = text_splice_work.nodes_visited;
        work.formatted_text_nodes_copied = text_splice_work.nodes_copied;
        work.formatted_text_leaves_copied = text_splice_work.leaves_copied;
        for group in groups.iter().rev() {
            let prepared = scratch.prepare_text_edits_with_patches(
                edits[group.clone()].to_vec(),
                Some(source_patches[group.clone()].to_vec()),
            )?;
            work.accumulate(prepared.summary.projection_work);
            scratch.commit_model_transaction(prepared)?;
        }
        let mut state = scratch.state().clone();
        if state.projection.text_tree().byte_len() != target_text.byte_len()
            || state.source.len() != source.len()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        // Regional verification plus reverse-ordered, separated source patches
        // proves the complete batch without flattening either full snapshot.
        state.source = source.clone();
        state.revision = revision;
        state
            .projection
            .order_new_literal_block_ids(
                self.projection().text_tree().byte_len(),
                edits,
                self.next_projected_block_id,
            )
            .map_err(super::block_identity_document_error)?;
        let sheet = state.projection.style_sheet().clone();
        let assignment = state.projection.document_style().clone();
        state
            .projection
            .install_configuration_styles(revision, sheet, assignment);
        Ok(Some(TextEditCandidate {
            state,
            work,
            block_ids_already_reconciled: true,
            next_projected_block_id: scratch.next_projected_block_id,
        }))
    }

    fn build_verified_text_edit_candidate(
        &self,
        source: super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_splice_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
        authored_syntax: bool,
    ) -> Result<TextEditCandidate, ModelTransactionError> {
        if let Some(candidate) = self.build_disjoint_literal_edit_candidate(
            &source,
            revision,
            target_text,
            text_splice_work,
            edits,
            source_patches,
        )? {
            return Ok(candidate);
        }
        if let Some(candidate)=self.build_table_row_candidate(&source,revision,target_text,text_splice_work,edits,source_patches)? {return Ok(candidate);}
        if let Some(candidate) = self.build_markdown_structured_local_candidate(
            &source,
            revision,
            target_text,
            text_splice_work,
            edits,
            source_patches,
        )? {
            return Ok(candidate);
        }
        // A regional Markdown Source result that cannot be proven exact, or
        // that the splice cannot place, leaves this block for the complete
        // verified candidate below.
        // Filling an empty item can activate an ordered marker beside prose
        // or change a following indented code block into continuation text.
        // Its previous classifier context cannot prove that new ownership.
        let changes_empty_item_grammar = edits.iter().any(|edit| {
            markdown_list_structure::empty_insertion_patches(self, &edit.range, &edit.replacement)
                .is_ok_and(|patches| patches.is_some())
        });
        let region = if changes_empty_item_grammar { None } else {
            self.line_local_projection_region_with_context(edits, source_patches, authored_syntax)?
        };
        if let Some(region) = region {
            let regional = (|| -> Result<Option<TextEditCandidate>, ModelTransactionError> {
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
                let normalized = if self.format().is_literal() {
                    normalize_literal(&decoded, self.state().file_format)
                } else {
                    normalize(&decoded, self.state().file_format)
                };
                // A folded separator can end a regional Source capture exactly
                // at the next physical row; its parser-only terminal row is not
                // one of the source records replaced by this regional edit.
                let unowned_terminal_row = (region.retained_separator.is_some()
                    || (self.format() == Format::MarkdownSource && new_source.end < source.len()))
                    && normalized
                        .endings
                        .last()
                        .is_some_and(|ending| ending.source.end == new_source.end);
                // A Markdown Source break adds or removes source rows, even
                // where the formatted rows stay, as when an empty list item
                // becomes a paragraph. The regional parse's endings then
                // delimit the region's new source rows.
                let row_delta = if self.format() == Format::MarkdownSource {
                    self.markdown_source_row_delta(edits)?
                } else {
                    0
                };
                let source_rows_change = (self.format() == Format::MarkdownSource || authored_syntax
                    || region.retained_separator.is_some())
                    && normalized.endings.len() + usize::from(!unowned_terminal_row)
                        != region.source_lines.len();
                if self.format() != Format::Markdown
                    && !self.format().is_literal()
                    && !source_rows_change
                    && normalized.endings.len() + usize::from(!unowned_terminal_row)
                        != region.source_lines.len()
                {
                    return Err(DocumentError::VerificationFailed.into());
                }
                let retained_separator = region.retained_separator.as_ref().map(|separator| -> Result<_, ModelTransactionError> {
                    Ok((separator.clone(),
                        rebase_source_boundary(separator.start, source_patches, Association::AfterInsertion)?
                            ..rebase_source_boundary(separator.end, source_patches, Association::AfterInsertion)?))
                }).transpose()?;
                let mut parser_input = normalized.clone();
                if let Some((_, separator)) = &retained_separator {
                    let end = parser_input.units.iter().find(|unit| unit.source.start >= separator.start)
                        .map_or(parser_input.text.len(), |unit| unit.normalized.start);
                    parser_input.text.truncate(end);
                    parser_input.units.retain(|unit| unit.normalized.start < end);
                    parser_input.endings.retain(|ending| ending.normalized.start < end);
                }
                if self.can_inherit_markdown_list_context(edits, source_patches) {
                    parser_input = self.markdown_list_region_body_input(
                        &parser_input, &region.old_formatted, region.old_source.start, source_patches,
                    )?;
                }
                let mut regional_projection = {
                    project(
                        &parser_input,
                        self.state().format,
                        revision,
                        new_source.start,
                        new_source.end,
                    )
                };
                if self.can_inherit_markdown_source_list_context(edits)
                    || self.can_inherit_markdown_list_context(edits, source_patches)
                {
                    let old_blocks = self.projection().blocks_for_region(&region.old_formatted);
                    let map = |at: usize| {
                        let delta: i128 = edits
                            .iter()
                            .filter(|edit| edit.range.end <= at)
                            .map(|edit| edit.replacement.len() as i128 - edit.range.len() as i128)
                            .sum();
                        (at as i128 - region.old_formatted.start as i128 + delta) as usize
                    };
                    // Source syntax can consume an authored character (for
                    // example list-marker whitespace). Validate the local text
                    // extent before installing a partition over that projection.
                    if map(region.old_formatted.end) != regional_projection.text_tree().byte_len() {
                        return Err(DocumentError::VerificationFailed.into());
                    }
                    // Text inserted exactly at a block's start is typed at the
                    // beginning of that block's line, so the start stays before it.
                    let map_start = |at: usize| {
                        let delta: i128 = edits
                            .iter()
                            .filter(|edit| {
                                edit.range.end < at
                                    || (edit.range.end == at && !edit.range.is_empty())
                            })
                            .map(|edit| edit.replacement.len() as i128 - edit.range.len() as i128)
                            .sum();
                        (at as i128 - region.old_formatted.start as i128 + delta) as usize
                    };
                    let blocks = old_blocks
                        .into_iter()
                        .map(|mut block| {
                            let start = block.range.start.max(region.old_formatted.start);
                            let end = block.range.end.min(region.old_formatted.end);
                            block.range = if start == region.old_formatted.start {
                                0
                            } else {
                                map_start(start)
                            }..map(end);
                            block
                        })
                        .collect();
                    // Literal body edits retain the validated enclosing list stack.
                    // A local restart may not see its ancestor markers; inherit
                    // paragraph membership as well as the list's style and ordinal.
                    // An inherited partition that no longer covers the new text is
                    // not proven, so the caller falls back to a complete reparse.
                    regional_projection
                        .try_install_paragraph_partition(blocks)
                        .map_err(|_| DocumentError::VerificationFailed)?;
                    let first = self
                        .projection()
                        .presentation_line_at_offset(region.old_formatted.start, true)
                        .unwrap();
                    let last = self
                        .projection()
                        .presentation_line_at_offset(region.old_formatted.end, true)
                        .unwrap();
                    let map_end = |at: usize| {
                        let delta: i128 = edits
                            .iter()
                            .filter(|edit| edit.range.end <= at)
                            .map(|edit| edit.replacement.len() as i128 - edit.range.len() as i128)
                            .sum();
                        (at as i128 - region.old_formatted.start as i128 + delta) as usize
                    };
                    let ranges = (first..=last)
                        .map(|index| {
                            let old = self
                                .projection()
                                .presentation_line_range(index, true)
                                .unwrap();
                            let start = old.start.max(region.old_formatted.start);
                            let end = old.end.min(region.old_formatted.end);
                            let start = if start == region.old_formatted.start {
                                0
                            } else {
                                map_end(start)
                            };
                            start..map_end(end)
                        })
                        .collect();
                    regional_projection.install_flow_ranges(ranges);
                    if self.format() == Format::MarkdownSource
                        && self
                            .verify_markdown_source_region(
                                &region,
                                &regional_projection,
                                edits,
                                true,
                            )
                            .is_err()
                    {
                        return Ok(None);
                    }
                } else if self.format() == Format::MarkdownSource {
                    if self
                        .verify_markdown_source_region(&region, &regional_projection, edits, false)
                        .is_err()
                    {
                        return Ok(None);
                    }
                }
                let projected_formatted_bytes = regional_projection.text().len();
                let projected_hard_lines = regional_projection.hard_line_count();
                let mut next_projected_block_id = self.next_projected_block_id;
                let (projection, splice_work) = splice_line_local_projection(
                    if self.format().is_code() {
                        &self.state().projection
                    } else {
                        self.projection()
                    },
                    regional_projection,
                    revision,
                    region.hard_lines.clone(),
                    region.old_formatted,
                    region.old_source,
                    new_source.clone(),
                    target_text.clone(),
                    source.len(),
                    self.format() == Format::MarkdownSource,
                    self.format().is_literal(),
                    edits,
                    source_patches,
                    retained_separator,
                    &mut next_projected_block_id,
                )
                .map_err(super::block_identity_document_error)?;
                if !self.format().is_literal()
                    && Some(projection.hard_line_count())
                        != self
                            .projection()
                            .hard_line_count()
                            .checked_add_signed(row_delta)
                {
                    return Err(DocumentError::VerificationFailed.into());
                }

                let source_hard_lines = if source_rows_change {
                    let mut start = self
                        .state()
                        .source_hard_lines
                        .get(region.source_lines.start)
                        .ok_or(DocumentError::VerificationFailed)?
                        .start;
                    let mut ranges = Vec::with_capacity(normalized.endings.len() + 1);
                    for ending in &normalized.endings {
                        ranges.push(start..ending.source.end);
                        start = ending.source.end;
                    }
                    if !unowned_terminal_row {
                        let line = region.source_lines.end - 1;
                        let trailing = self
                            .state()
                            .source_hard_lines
                            .get(line)
                            .ok_or(DocumentError::VerificationFailed)?;
                        let association = if line + 1 == self.state().source_hard_lines.len() {
                            Association::AfterInsertion
                        } else {
                            Association::BeforeInsertion
                        };
                        ranges.push(
                            start
                                ..rebase_source_boundary(
                                    trailing.end,
                                    source_patches,
                                    association,
                                )?,
                        );
                    }
                    ranges
                } else if self.format().is_literal() {
                    // A chunk can begin inside the first line. Preserve its
                    // unchanged source prefix while rebuilding only local breaks.
                    let mut start = self
                        .state()
                        .source_hard_lines
                        .get(region.source_lines.start)
                        .ok_or(DocumentError::VerificationFailed)?
                        .start;
                    let mut ranges = Vec::with_capacity(normalized.endings.len() + 1);
                    for ending in &normalized.endings {
                        ranges.push(start..ending.source.end);
                        start = ending.source.end;
                    }
                    let trailing = self
                        .state()
                        .source_hard_lines
                        .get(region.source_lines.end - 1)
                        .unwrap();
                    let end = rebase_source_boundary(
                        trailing.end,
                        source_patches,
                        Association::AfterInsertion,
                    )?;
                    ranges.push(start..end);
                    ranges
                } else {
                    region
                        .source_lines
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
                            let end_association =
                                if line + 1 == self.state().source_hard_lines.len() {
                                    Association::AfterInsertion
                                } else {
                                    Association::BeforeInsertion
                                };
                            let end =
                                rebase_source_boundary(old.end, source_patches, end_association)?;
                            Ok(start..end)
                        })
                        .collect::<Result<Vec<_>, ModelTransactionError>>()?
                };
                let (source_hard_lines, source_line_work) = self
                    .state()
                    .source_hard_lines
                    .replace_ranges_with_stats(region.source_lines.clone(), &source_hard_lines)
                    .ok_or(DocumentError::VerificationFailed)?;
                if source_hard_lines.source_end() != source.len() {
                    return Err(DocumentError::VerificationFailed.into());
                }

                return Ok(Some(TextEditCandidate {
                    state: DocumentState {
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
                    next_projected_block_id,
                }));
            })();
            match regional {
                Ok(Some(candidate)) => return Ok(candidate),
                Ok(None) => {}
                Err(_) if self.format() == Format::MarkdownSource => {}
                Err(error) => return Err(error),
            }
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
            next_projected_block_id: self.next_projected_block_id,
        })
    }

    /// Verify a Markdown code/list edit in its affected paragraph while
    /// preserving the existing semantic style.
    fn build_markdown_structured_local_candidate(
        &self,
        source: &super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        patches: &[SourcePatch],
    ) -> Result<Option<TextEditCandidate>, ModelTransactionError> {
        if markdown_html_edit::edit_changes_literal_html_grammar(self, edits)? {
            return Ok(None);
        }
        if self.markdown_edit_needs_reference_context(edits) {
            return Ok(None);
        }
        if self.format() == Format::Markdown
            && edits.iter().any(|edit| {
                edit.replacement.is_empty()
                    && self
                        .projection()
                        .blocks_for_region(&edit.range)
                        .iter()
                        .any(|block| {
                            matches!(block.kind, super::BlockKind::ListItem { .. })
                                && !block.range.is_empty()
                                && edit.range.start <= block.range.start
                                && block.range.end <= edit.range.end
                        })
            })
        {
            return Ok(None);
        }
        if !self.format().is_markdown() || edits.len() != 1 || patches.len() != 1 {
            return Ok(None);
        }
        let (edit, patch) = (&edits[0], &patches[0]);
        let markdown_table = self.projection().table_cell_at(edit.range.start).is_some_and(|(_,_,cell)| {
            edit.range.end <= cell.range.end
                && !edit.replacement.chars().any(|ch| !ch.is_alphanumeric())
                && self.table_source_text(cell.source_range.clone()).is_ok_and(|text| text.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '\t')))
        });
        if markdown_table && self.format() == Format::Markdown {
            let (_, _, cell) = self.projection().table_cell_at(edit.range.start).unwrap();
            let mut text = self.projection().text_tree().slice(cell.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            text.replace_range(edit.range.start - cell.range.start..edit.range.end - cell.range.start, &edit.replacement);
            if text.starts_with([' ', '\t']) || text.ends_with([' ', '\t']) {
                // Cell-edge whitespace is parsed, not inherited. Its protection
                // must be verified against a newly parsed table row.
                return Ok(None);
            }
        }
        if self.format().is_source_view() && !markdown_table {return Ok(None);}
        let markdown_code = self.format() == Format::Markdown;
        let inline_sample = edit.range.start.saturating_sub(1)
            ..(edit.range.end + 1).min(self.projection().text_tree().byte_len());
        let markdown_inline_code = markdown_code
            && self.projection().style_spans_for_region(&inline_sample).iter().any(|span| {
                span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                    && span.range.start <= edit.range.start && edit.range.end <= span.range.end
                    && self.projection().source_range(span.range.clone()).is_some_and(|source| {
                        super::markdown_code::delimiter_ranges(self, &source).is_ok_and(|delimiters| {
                            delimiters.is_some_and(|(opening, closing)| {
                                let Ok(ending) = self.encoding().encode_fragment(self.file_format().spelling()) else { return false; };
                                // A trimmed multiline scope can restart on a
                                // body row without its backticks. Ordinary
                                // inline scopes retain their parsing path.
                                opening.len() >= ending.len() && closing.len() >= ending.len()
                                    && self.state().source.bytes_in(opening.end - ending.len()..opening.end).as_deref() == Some(ending.as_slice())
                                    && self.state().source.bytes_in(closing.start..closing.start + ending.len()).as_deref() == Some(ending.as_slice())
                            })
                        })
                    })
            });
        let markdown_list = markdown_code
            && self
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| {
                    block.style.0 != "Code Block"
                        && matches!(block.kind, super::BlockKind::ListItem { .. })
                        && self
                            .projection()
                            .list_marker_range_for_block(block)
                            .map_or(true, |marker| edit.range.start >= marker.end)
                });
        if markdown_code
            && (!self
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| block.style.0 == "Code Block")
                && !markdown_list && !markdown_table && !markdown_inline_code
                || edit.replacement.contains(['`', '~'])
                || markdown_list && edit.replacement.contains(['*', '_', '#', '\\']))
        {
            return Ok(None);
        }
        // Explicit/inverse patches may restore plain text while removing its
        // old formatting wrapper. Escaped replacement syntax alone does not
        // prove that the removed bytes preserve parser state. Require one
        // uninterrupted visible source run covering exactly the replaced
        // formatted extent before inheriting any old character context.
        if edit.range.is_empty() {
            if !patch.range.is_empty() {
                return Ok(None);
            }
        } else {
            let mut text_at = edit.range.start;
            let mut source_at = patch.range.start;
            for span in self.projection().provenance_for_region(&edit.range) {
                if span.formatted.start != text_at
                    || span.formatted.end > edit.range.end
                    || span.source.start != source_at
                    || span.source.is_empty()
                {
                    return Ok(None);
                }
                text_at = span.formatted.end;
                source_at = span.source.end;
            }
            if text_at != edit.range.end || source_at != patch.range.end {
                return Ok(None);
            }
        }
        // Context inheritance is valid only for ordinary escaped text. List
        // and paragraph intents may produce identical text while changing the
        // enclosing state, and must pass through a structural reparse.
        let canonical = edit.replacement.clone();
        let Ok(canonical_bytes) = self.state().encoding.encode_fragment(&canonical) else {
            // A prose character reference may be representable even though
            // its decoded text is not. This literal-fragment fast path cannot
            // prove that edit; let ordinary source parsing verify it instead.
            return Ok(None);
        };
        if patch.replacement != canonical_bytes {
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

        let old_line = if markdown_code && !markdown_list && !markdown_table {
            edited_line.clone()
        } else {
            block.range.clone()
        };

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
        let old_provenance = self
            .projection()
            .provenance_touching(&old_line)
            .into_iter()
            .filter(|span| {
                old_line.start <= span.formatted.start && span.formatted.end <= old_line.end
            })
            .collect::<Vec<_>>();
        let insertion_has_character_sample = old_provenance.iter().any(|span| {
            !span.formatted.is_empty()
                && (span.source.start == patch.range.start || span.source.end == patch.range.start)
        });

        let verified_empty_code = markdown_code && !markdown_list && block.range.is_empty()
            && super::markdown_code::fenced_source(self, &block)?.is_some_and(|fence| fence.body == patch.range);
        if edit.range.is_empty() && !insertion_has_character_sample && !verified_empty_code && !markdown_table {
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
        if markdown_code && !markdown_list && !markdown_table {
            if let Some(fence) = super::markdown_code::fenced_source(self, &block)? {
                let opening = self.state().source.bytes_in(fence.opening.clone())
                    .ok_or(DocumentError::VerificationFailed)?;
                let opening = self.encoding().decode_region(&opening, fence.opening.start)?.text;
                let quote = super::markdown_quotes::prefix(&opening);
                let indent = if matches!(block.kind, super::BlockKind::ListItem { .. }) { 0 } else {
                    opening[quote..].bytes().take_while(|byte| *byte == b' ').count()
                };
                let line_start = rebase_source_boundary(physical.start, patches, Association::BeforeInsertion)?;
                let line_end = rebase_source_boundary(physical.end, patches, Association::AfterInsertion)?;
                let bytes = source.bytes_in(line_start..line_end).ok_or(DocumentError::VerificationFailed)?;
                let decoded = self.encoding().decode_region(&bytes, line_start)?;
                let normalized = normalize(&decoded, self.file_format());
                let raw = normalized.text.trim_end_matches('\n');
                let quote = if block.quote_depth > 0 { super::markdown_quotes::prefix(raw) } else { 0 };
                let raw = &raw[quote..];
                let container = if matches!(block.kind, super::BlockKind::ListItem { .. }) {
                    fence.body_prefix[super::markdown_quotes::prefix(&fence.body_prefix)..].len()
                } else { 0 };
                let skip = raw.bytes().take(container).take_while(|byte| *byte == b' ').count();
                let raw = &raw[skip..];
                let trim = raw.bytes().take(indent).take_while(|byte| *byte == b' ').count();
                if super::markdown_syntax::fence_close(raw, fence.delimiter, fence.width)
                    || raw[trim..] != new_text
                {
                    return Ok(None);
                }
            }
        }
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

        {
            let (parsed_provenance, parsed_bytes) = {
                let decoded = self
                    .state()
                    .encoding
                    .decode_region(&patch.replacement, patch.range.start)?;
                let normalized = normalize(&decoded, self.state().file_format);
                let parsed = super::projection::project_plain(
                    &normalized, revision, patch.range.start,
                    patch.range.start + patch.replacement.len(),
                );
                if parsed.text() != edit.replacement {
                    return Ok(None);
                }

                (parsed.provenance().to_vec(), patch.replacement.len())
            };
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
            for mut span in parsed_provenance {
                span.formatted = span.formatted.start + edit.range.start - old_line.start
                    ..span.formatted.end + edit.range.start - old_line.start;
                provenance.push(span);
            }
            provenance.sort_by_key(|span| span.formatted.start);
            decoded_bytes = parsed_bytes + old_patch_bytes.len();
        }

        let sampled_at = if edit.range.is_empty() && edit.range.start == edited_line.start {
            // A hard row's start has only its body-side character context.
            // The preceding source ending is not a Code character sample.
            edit.range.start
        } else if edit.range.is_empty() {
            self.projection()
                .provenance_for_region(
                    &(edit.range.start.saturating_sub(4)
                        ..edit.range.start.saturating_add(4).min(edited_line.end)),
                )
                .iter()
                .find(|span| span.source.end == patch.range.start && !span.formatted.is_empty())
                .map(|span| span.formatted.end - 1)
                .unwrap_or(edit.range.start)
        } else if edit.range.start == edited_line.end {
            edited_line.end.saturating_sub(1)
        } else {
            edit.range.start
        };

        let mut insertion_styles = old_styles
            .iter()
            .filter(|span| span.range.contains(&sampled_at))
            .map(|span| span.application.clone())
            .collect::<Vec<_>>();
        if verified_empty_code && !insertion_styles.contains(&StyleApplication::Semantic(SemanticInlineStyle::Code)) {
            insertion_styles.push(StyleApplication::Semantic(SemanticInlineStyle::Code));
        }

        let mut styles = Vec::new();
        for span in old_styles {
            let prefix_end = edit.range.start;
            if span.range.start < prefix_end {
                let end = span.range.end.min(prefix_end);
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
        let hard_line_ranges = {
            (first_line..=last_line)
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
                .collect::<Result<Vec<_>, DocumentError>>()?
        };
        regional.install_hard_line_partition(hard_line_ranges);
        let projected_bytes = regional.text().len();
        let previous_projection = self.projection();
        let mut next_projected_block_id = self.next_projected_block_id;
        let (projection, range_work) = match splice_line_local_projection(
            previous_projection,
            regional,
            revision,
            first_line..last_line + 1,
            old_line,
            old_source,
            new_source,
            target_text.clone(),
            source.len(),
            false,
            false,
            edits,
            patches,
            None,
            &mut next_projected_block_id,
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
            next_projected_block_id,
        }))
    }

    fn can_inherit_markdown_list_context(
        &self,
        edits: &[TextEdit],
        patches: &[SourcePatch],
    ) -> bool {
        if self.format() != Format::Markdown || edits.is_empty() {
            return false;
        }
        let mut source_bodies = Vec::new();
        for edit in edits {
            if markdown_list_structure::empty_insertion_patches(self, &edit.range, &edit.replacement)
                .is_ok_and(|patches| patches.is_some())
            {
                return false;
            }
            let Some(block) = self
                .projection()
                .blocks_for_region(&edit.range)
                .into_iter()
                .find(|block| {
                    block.range.start <= edit.range.start
                        && edit.range.end <= block.range.end
                        && block.style.0 != "Code Block"
                        && matches!(block.kind, super::BlockKind::ListItem { .. })
                })
            else {
                return false;
            };
            let Some(body) = self.projection().source_range(block.range) else {
                return false;
            };
            source_bodies.push(body);
        }
        // Cooked body patches retain the source labels. Their enclosing list
        // stack may begin before this local parse region, so retain its already
        // validated level, ordinal, and paragraph membership.
        patches.iter().all(|patch| {
            source_bodies
                .iter()
                .any(|body| body.start <= patch.range.start && patch.range.end <= body.end)
        })
    }

    /// Ancestor list markers can lie before the regional restart. Remove only
    /// the old prose body's hidden indentation, keeping source coordinates and
    /// all visible contributors. Otherwise a continuation is parsed as code
    /// before its validated list ownership is restored.
    fn markdown_list_region_body_input(
        &self,
        input: &super::line_endings::NormalizedText,
        old_formatted: &Range<usize>,
        old_source_start: usize,
        patches: &[SourcePatch],
    ) -> Result<super::line_endings::NormalizedText, ModelTransactionError> {
        let Some(first_source) = input.units.first().map(|unit| unit.source.start) else {
            return Ok(input.clone());
        };
        let Ok(capture_start) = self.projection().map_source_boundary(
            self.revision(), old_source_start, BoundaryAffinity::Downstream,
        ) else { return Ok(input.clone()); };
        let blocks = self.projection().blocks_for_region(
            &(capture_start.formatted_offset.min(old_formatted.start)..old_formatted.end),
        );
        let mut omitted_items = std::collections::BTreeMap::new();
        for block in &blocks {
            let Some(item) = block.containers.iter().rev()
                .find(|member| member.container.kind == super::ContainerKind::ListItem)
            else { continue; };
            let omitted = omitted_items.entry(item.container.id).or_insert(true);
            if item.starts_here {
                let at = self.projection().source_insertion_point(block.range.start, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let row = self.state().source_hard_lines.line_at_offset(at)
                    .and_then(|index| self.state().source_hard_lines.get(index))
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let row_start = rebase_source_boundary(row.start, patches, Association::BeforeInsertion)?;
                if row_start >= first_source {
                    *omitted = false;
                }
            }
        }
        let mut prefixes = std::collections::BTreeMap::new();
        for span in self.projection().provenance_for_region(old_formatted) {
            let owner = blocks.partition_point(|block| block.range.start <= span.formatted.start)
                .checked_sub(1).and_then(|index| blocks.get(index));
            if span.formatted.is_empty() || span.source.is_empty()
                || !owner.is_some_and(|block| {
                    span.formatted.end <= block.range.end && block.style.0 != "Code Block"
                        && matches!(block.kind, super::BlockKind::ListItem { .. })
                        && block.containers.iter().rev()
                            .find(|member| member.container.kind == super::ContainerKind::ListItem)
                            .is_some_and(|member| omitted_items.get(&member.container.id) == Some(&true))
                })
            {
                continue;
            }
            let Some(index) = self.state().source_hard_lines.line_at_offset(span.source.start) else { continue; };
            let row = self.state().source_hard_lines.get(index).ok_or(DocumentError::VerificationFailed)?;
            if span.source.end > row.end || prefixes.contains_key(&row.start) {
                continue;
            }
            if self.projection().text_tree().slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?.chars().all(|ch| ch == '\n')
            {
                continue;
            }
            prefixes.insert(row.start, span.source.start);
        }
        let mut removed = Vec::new();
        for (start, visible) in prefixes {
            let start = rebase_source_boundary(start, patches, Association::BeforeInsertion)?;
            let visible = rebase_source_boundary(visible, patches, Association::BeforeInsertion)?;
            let first = input.units.partition_point(|unit| unit.source.start < start);
            let end = input.units.partition_point(|unit| unit.source.end <= visible);
            if first >= end { continue; }
            let prefix = &input.text[input.units[first].normalized.start..input.units[end - 1].normalized.end];
            let quote = super::markdown_quotes::prefix(prefix);
            let white = prefix[quote..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
            if white > 0 {
                let at = input.units[first].normalized.start + quote;
                removed.push(at..at + white);
            }
        }
        let mut result = super::line_endings::NormalizedText {
            text: String::new(), units: Vec::with_capacity(input.units.len()),
            endings: Vec::with_capacity(input.endings.len()), encoding: input.encoding,
        };
        let mut ending = 0;
        let mut hidden = 0;
        for unit in &input.units {
            while hidden < removed.len() && removed[hidden].end <= unit.normalized.start { hidden += 1; }
            if removed.get(hidden).is_some_and(|range| range.contains(&unit.normalized.start)) { continue; }
            let start = result.text.len();
            result.text.push_str(&input.text[unit.normalized.clone()]);
            let mut next = unit.clone();
            next.normalized = start..result.text.len();
            while ending < input.endings.len() && input.endings[ending].normalized.start < unit.normalized.start {
                ending += 1;
            }
            if let Some(original) = input.endings.get(ending).filter(|ending| ending.normalized == unit.normalized) {
                let mut original = original.clone();
                original.normalized = next.normalized.clone();
                result.endings.push(original);
            }
            result.units.push(next);
        }
        Ok(result)
    }

    /// A regional Markdown Source reparse is exact only when the region does
    /// not depend on the rest of the document and the edit does not reach
    /// outside it. Prove both before a regional candidate is published:
    /// the old region must parse alone exactly as it does in the whole
    /// document, and the unchanged rows around the edited rows must parse
    /// unchanged. Anything else is left to the complete reparse.
    ///
    /// With `inherited_blocks`, the regional projection carries the old
    /// block partition, so only its text and styles come from the reparse.
    fn verify_markdown_source_region(
        &self,
        region: &LineLocalProjectionRegion,
        regional: &FormattedDocument,
        edits: &[TextEdit],
        inherited_blocks: bool,
    ) -> Result<(), ModelTransactionError> {
        let signature = |projection: &FormattedDocument, range: &Range<usize>| {
            let mut signature = markdown_source_region_signature(projection, range);
            if inherited_blocks {
                signature.blocks.clear();
                signature.flows = None;
            }
            signature
        };
        let failed = || ModelTransactionError::from(DocumentError::VerificationFailed);
        let old_bytes = self
            .state()
            .source
            .bytes_in(region.old_source.clone())
            .ok_or_else(failed)?;
        let decoded = self
            .state()
            .encoding
            .decode_region(&old_bytes, region.old_source.start)?;
        let old_alone = project(
            &normalize(&decoded, self.state().file_format),
            self.state().format,
            self.revision(),
            region.old_source.start,
            region.old_source.end,
        );
        let old_text = self
            .projection()
            .text_tree()
            .slice(region.old_formatted.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        if old_alone.text() != old_text
            || signature(&old_alone, &(0..old_text.len()))
                != signature(self.projection(), &region.old_formatted)
        {
            return Err(failed());
        }

        // Every unchanged row around the edited rows must parse as before;
        // the edited rows themselves are the regional parse's to decide.
        let (Some(first), Some(last)) = (edits.first(), edits.last()) else {
            return Err(failed());
        };
        let (Some(first_edited), Some(last_edited)) = (
            self.projection().hard_line_at_offset(first.range.start),
            self.projection().hard_line_at_offset(last.range.end),
        ) else {
            return Err(failed());
        };
        // Source syntax can reassign every continuation row of the edited
        // paragraph or item, for example when an empty marker gains a body.
        // These rows belong to the captured grammar owner; outer rows remain proof
        // boundaries even when their physical text is unchanged.
        let mut affected_rows = first_edited..last_edited + 1;
        if !inherited_blocks {
            let owners = self.projection().blocks_for_region(&(first.range.start..last.range.end));
            let items: std::collections::BTreeSet<_> = owners.iter().filter_map(|block| {
                block.containers.iter().rev()
                    .find(|member| member.container.kind == super::ContainerKind::ListItem)
                    .map(|member| member.container.id)
            }).collect();
            for block in self.projection().blocks_for_region(&region.old_formatted) {
                if !owners.iter().any(|owner| owner.id == block.id)
                    && !block.containers.iter().any(|member| items.contains(&member.container.id)) {
                    continue;
                }
                let Some(start) = self.projection().hard_line_at_offset(block.range.start) else { continue; };
                let Some(end) = self.projection().hard_line_at_offset(block.range.end) else { continue; };
                affected_rows.start = affected_rows.start.min(start);
                affected_rows.end = affected_rows.end.max(end + 1);
            }
        }
        // Rows after the edited rows move by the number of breaks it adds.
        let delta = self.markdown_source_row_delta(edits)?;
        let new_line = |line: usize| {
            let local = line - region.hard_lines.start;
            if line > last_edited {
                local.checked_add_signed(delta)
            } else {
                Some(local)
            }
        };
        if Some(regional.hard_line_count()) != region.hard_lines.len().checked_add_signed(delta) {
            return Err(failed());
        }
        // Consecutive source endings pair into paragraph separators. A run
        // of them just outside the region is bounded by the region's edge
        // row, and would join endings an edit adds or removes there if that
        // row became blank.
        let changes_breaks =
            first_edited != last_edited || edits.iter().any(|edit| edit.replacement.contains('\n'));
        if changes_breaks {
            let blank = |line: usize| -> Result<bool, ModelTransactionError> {
                let row = regional.hard_line_range(line).ok_or_else(failed)?;
                let text = regional
                    .text_tree()
                    .slice(row)
                    .map_err(DocumentError::FormattedTextStorage)?;
                Ok(text.trim().is_empty())
            };
            let last_row = regional
                .hard_line_count()
                .checked_sub(1)
                .ok_or_else(failed)?;
            if (region.hard_lines.start > 0 && blank(0)?)
                || (region.hard_lines.end < self.projection().hard_line_count() && blank(last_row)?)
            {
                return Err(failed());
            }
        }
        // A region cut inside a paragraph too long to reparse must leave the
        // rows outside it with the parser state they had.
        if !inherited_blocks {
            let row_text = |projection: &FormattedDocument, row: Range<usize>| {
                projection.text_tree().slice(row).map_err(|error| {
                    ModelTransactionError::from(DocumentError::FormattedTextStorage(error))
                })
            };
            let last = region.hard_lines.end - 1;
            let old_row = self.projection().hard_line_range(last).ok_or_else(failed)?;
            let new_last = new_line(last).ok_or_else(failed)?;
            let new_row = regional.hard_line_range(new_last).ok_or_else(failed)?;
            let old_owner =
                markdown_source_row_owner(self.projection(), &old_row).ok_or_else(failed)?;
            if old_owner.range.end > old_row.end {
                // The rows after the region see one open plain paragraph and
                // whether a hard break has occurred in it, after which the
                // paragraph ends at the next row without one. Continuing in
                // the old document, an unedited last row without a hard break
                // means none occurred; the new paragraph must agree.
                let new_owner = markdown_source_row_owner(regional, &new_row).ok_or_else(failed)?;
                if (first_edited..=last_edited).contains(&last)
                    || old_owner.kind != super::BlockKind::Paragraph
                    || new_owner.kind != super::BlockKind::Paragraph
                    || old_owner.style != new_owner.style
                {
                    return Err(failed());
                }
                if !markdown_row_has_hard_break(&row_text(regional, new_row)?) {
                    let owner_first = regional
                        .hard_line_at_offset(new_owner.range.start)
                        .ok_or_else(failed)?;
                    for local in owner_first..new_last {
                        let row = regional.hard_line_range(local).ok_or_else(failed)?;
                        if markdown_row_has_hard_break(&row_text(regional, row)?) {
                            return Err(failed());
                        }
                    }
                }
            }

            let first = region.hard_lines.start;
            let old_row = self
                .projection()
                .hard_line_range(first)
                .ok_or_else(failed)?;
            let old_owner =
                markdown_source_row_owner(self.projection(), &old_row).ok_or_else(failed)?;
            if old_owner.range.start < old_row.start {
                // Earlier rows pass the same fact forward. A first row ending
                // in a hard break fixes it, and so does a previous row without
                // one, which could not still belong to the paragraph after a
                // hard break.
                let hard_break = |line: usize| -> Result<bool, ModelTransactionError> {
                    let row = self.projection().hard_line_range(line).ok_or_else(failed)?;
                    Ok(markdown_row_has_hard_break(&row_text(
                        self.projection(),
                        row,
                    )?))
                };
                if first_edited == first || (!hard_break(first)? && hard_break(first - 1)?) {
                    return Err(failed());
                }
            }
        }
        for line in region.hard_lines.clone() {
            if affected_rows.contains(&line) {
                continue;
            }
            let (Some(old_row), Some(new_row)) = (
                self.projection().hard_line_range(line),
                new_line(line).and_then(|local| regional.hard_line_range(local)),
            ) else {
                return Err(failed());
            };
            let (old_owner, old_styles) =
                markdown_source_row_signature(self.projection(), &old_row);
            let (new_owner, new_styles) = markdown_source_row_signature(regional, &new_row);
            if old_styles != new_styles || (!inherited_blocks && old_owner != new_owner) {
                return Err(failed());
            }
        }
        Ok(())
    }

    /// The formatted edit a proven regional reparse makes for `edit`'s
    /// source `patches`. It is `edit` itself unless the parse reshapes the
    /// text beside it, such as trimming whitespace that now ends or begins a
    /// paragraph; then it is the smallest change that covers `edit` and the
    /// parse's text. `None` when the region cannot be proven.
    pub(super) fn markdown_source_reparsed_edit(
        &self,
        edit: &TextEdit,
        patches: &[SourcePatch],
    ) -> Result<Option<TextEdit>, ModelTransactionError> {
        if self.format() != Format::MarkdownSource {
            return Ok(None);
        }
        let Some(region) =
            self.line_local_projection_region(std::slice::from_ref(edit), patches)?
        else {
            return Ok(None);
        };
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_end =
            rebase_source_boundary(region.old_source.end, patches, Association::AfterInsertion)?;
        let Some(bytes) = source.bytes_in(region.old_source.start..new_end) else {
            return Ok(None);
        };
        let decoded = self
            .state()
            .encoding
            .decode_region(&bytes, region.old_source.start)?;
        let regional = project(
            &normalize(&decoded, self.state().file_format),
            self.state().format,
            Revision(self.next_revision),
            region.old_source.start,
            new_end,
        );
        let old = self
            .projection()
            .text_tree()
            .slice(region.old_formatted.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let new = regional.text();
        let (local_start, local_end) = (
            edit.range.start - region.old_formatted.start,
            edit.range.end - region.old_formatted.start,
        );
        let prefix = old
            .char_indices()
            .zip(new.chars())
            .take_while(|((at, a), b)| *at < local_start && a == b)
            .map(|((at, a), _)| at + a.len_utf8())
            .last()
            .unwrap_or(0)
            .min(local_start);
        let suffix = old[prefix..]
            .chars()
            .rev()
            .zip(new[prefix..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .scan(0, |length, add| {
                *length += add;
                Some(*length)
            })
            .take_while(|&length| length <= old.len() - local_end)
            .last()
            .unwrap_or(0);
        let effective = TextEdit::new(
            region.old_formatted.start + prefix..region.old_formatted.start + old.len() - suffix,
            &new[prefix..new.len() - suffix],
        );
        let edits = std::slice::from_ref(&effective);
        if self
            .verify_markdown_source_region(&region, &regional, edits, false)
            .is_err()
        {
            return Ok(None);
        }
        Ok(Some(effective))
    }

    /// The change in hard-line count made by `edits`: every U+000A in
    /// Markdown Source formatted text is a hard-line boundary.
    fn markdown_source_row_delta(
        &self,
        edits: &[TextEdit],
    ) -> Result<isize, ModelTransactionError> {
        let mut delta = 0isize;
        for edit in edits {
            let replaced = self
                .projection()
                .text_tree()
                .slice(edit.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            delta += edit.replacement.matches('\n').count() as isize
                - replaced.matches('\n').count() as isize;
        }
        Ok(delta)
    }

    /// Within a paragraph, only links and hard breaks connect rows: a link
    /// label can cross rows, and a row's trailing hard break divides the
    /// paragraph's flow. An edit is row-local when its rows contain no link
    /// syntax and keep their hard-break state.
    fn markdown_source_edit_is_row_local(&self, edits: &[TextEdit], rows: Range<usize>) -> bool {
        let link_syntax =
            |text: &str| text.contains(['[', ']', '(', ')', '<', '>', '\\', '"', '\'']);
        // A break splits the paragraph, and with it any link crossing the row.
        if edits.iter().any(|edit| edit.replacement.contains('\n')) {
            return false;
        }
        rows.into_iter().all(|line| {
            let Some(range) = self.projection().hard_line_range(line) else {
                return false;
            };
            let Ok(old) = self.projection().text_tree().slice(range.clone()) else {
                return false;
            };
            let mut new = old.clone();
            for edit in edits.iter().rev() {
                if edit.range.start < range.start || edit.range.end > range.end {
                    if edit.range.end > range.start && edit.range.start < range.end {
                        return false;
                    }
                    continue;
                }
                new.replace_range(
                    edit.range.start - range.start..edit.range.end - range.start,
                    &edit.replacement,
                );
            }
            !link_syntax(&old)
                && !link_syntax(&new)
                && markdown_row_has_hard_break(&old) == markdown_row_has_hard_break(&new)
        })
    }

    fn can_inherit_markdown_source_list_context(&self, edits: &[TextEdit]) -> bool {
        self.format() == Format::MarkdownSource
            && !edits.is_empty()
            && edits.iter().all(|edit| {
                let Some(line) = self
                    .projection()
                    .hard_line_at_offset(edit.range.start)
                    .and_then(|index| self.projection().hard_line_range(index))
                else {
                    return false;
                };
                let Some(block) =
                    self.projection()
                        .blocks_for_region(&line)
                        .into_iter()
                        .find(|block| {
                            block.range.start <= line.start
                                && line.end <= block.range.end
                                && matches!(block.kind, super::BlockKind::ListItem { .. })
                        })
                else {
                    return false;
                };
                if edit.range.end > line.end || block.style.0 == "Code Block" {
                    return false;
                }
                let Ok(old) = self.projection().text_tree().slice(line.clone()) else {
                    return false;
                };
                let prefix = self
                    .projection()
                    .list_marker_range_for_block(&block)
                    .map_or_else(
                        || old.len() - old.trim_start_matches([' ', '\t']).len(),
                        |marker| marker.len(),
                    );
                if edit.range.start < line.start + prefix {
                    return false;
                }
                // Code delimiters can reach past the paragraph; other
                // punctuation is inline and checked by the regional proof.
                let literal = |text: &str| {
                    text.chars().all(|ch| {
                        ch == ' '
                            || !(ch.is_control() || ch.is_whitespace() || matches!(ch, '`' | '~'))
                    })
                };
                if !literal(&edit.replacement)
                    || !literal(&old[edit.range.start - line.start..edit.range.end - line.start])
                {
                    return false;
                }
                let mut next = old.clone();
                next.replace_range(
                    edit.range.start - line.start..edit.range.end - line.start,
                    &edit.replacement,
                );
                // The first content character selects this line's block
                // syntax: a quote, heading, marker, fence or thematic break.
                // Literal text preserves that choice only while a letter
                // begins the content both before and after the edit.
                let starts_with_letter = |text: &str| {
                    text[prefix..]
                        .trim_start_matches(['*', '_'])
                        .chars()
                        .next()
                        .is_some_and(char::is_alphabetic)
                };
                !next[prefix..].trim().is_empty()
                    && old[..prefix] == next[..prefix]
                    && old[prefix..].starts_with(' ') == next[prefix..].starts_with(' ')
                    && starts_with_letter(&old)
                    && starts_with_letter(&next)
            })
    }

    fn markdown_edit_needs_reference_context(&self, edits: &[TextEdit]) -> bool {
        if !self.format().is_markdown() {
            return false;
        }
        edits.iter().any(|edit| {
            // A reference image hides its brackets in WYSIWYG, but its
            // destination still depends on definitions outside this paragraph.
            // Regional reparsing must retain that global grammar dependency.
            if self.projection().blocks_for_region(&edit.range).iter().any(|block|
                self.projection().inline_images_for_region(&block.range).iter().any(|image| !image.inline && !image.html)) {
                return true;
            }
            if self
                .projection()
                .style_spans_for_region(&(edit.range.start.saturating_sub(1)
                    ..edit.range.end.saturating_add(1).min(self.projection().text_tree().byte_len())))
                .iter()
                // Reference definitions may span several physical rows. A
                // continuation row has no bracket of its own, but carries the
                // same global dependency; boundary insertions touch it too.
                .any(|span| span.application == StyleApplication::Automatic("Markdown reference".into()))
            {
                return true;
            }
            if self.format() != Format::MarkdownSource {
                return false;
            }
            if edit.replacement.contains('[') {
                return true;
            }
            self.projection()
                .hard_line_at_offset(edit.range.start)
                .and_then(|i| self.projection().hard_line_range(i))
                .is_some_and(|range| {
                    self.projection()
                        .text_tree()
                        .slice(range)
                        .is_ok_and(|text| super::markdown_syntax::needs_reference_context(&text))
                })
        })
    }

    fn line_local_projection_region(
        &self,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
    ) -> Result<Option<LineLocalProjectionRegion>, ModelTransactionError> {
        self.line_local_projection_region_with_context(edits, source_patches, false)
    }

    fn line_local_projection_region_with_context(
        &self,
        edits: &[TextEdit],
        source_patches: &[SourcePatch],
        authored_syntax: bool,
    ) -> Result<Option<LineLocalProjectionRegion>, ModelTransactionError> {
        if markdown_html_edit::edit_changes_literal_html_grammar(self, edits)? {
            return Ok(None);
        }
        if self.markdown_edit_needs_reference_context(edits) {
            return Ok(None);
        }
        if edits.iter().any(|edit| self.projection().table_at(edit.range.start).is_some() || self.projection().table_at(edit.range.end).is_some()) { return Ok(None); }
        // An explicit prose separator proves independence from the following
        // paragraph. Capture its small source contributor, never that body's
        // potentially unbounded physical line.
        let retained_separator = if self.format() == Format::Markdown {
            edits.first().and_then(|edit| {
                let line = self.projection().hard_line_at_offset(edit.range.start)?;
                let row = self.projection().hard_line_range(line)?;
                if !edits.iter().all(|edit| row.start <= edit.range.start
                    && edit.range.end <= row.end && !edit.replacement.contains(['\n', '\r'])) {
                    return None;
                }
                let blocks = self.projection().blocks_for_region(&row);
                if !blocks.iter().any(|block| block.range == row
                    && block.kind == super::BlockKind::Paragraph && block.quote_depth == 0
                    && !block.markdown_html) {
                    return None;
                }
                self.projection().provenance_for_region(&(row.end..row.end + 1))
                    .into_iter().find(|span| span.formatted == (row.end..row.end + 1)
                        && !span.source.is_empty()).map(|span| span.source)
            }).filter(|boundary| markdown_block_styles::explicit_paragraph_separator(
                self, boundary, source_patches).unwrap_or(false))
        } else {
            None
        };
        if self.format() == Format::Markdown {
            if edits.iter().any(|edit| {
                edit.replacement.is_empty()
                    && self
                        .projection()
                        .blocks_for_region(&edit.range)
                        .iter()
                        .any(|block| {
                            matches!(block.kind, super::BlockKind::ListItem { .. })
                                && !block.range.is_empty()
                                && edit.range.start <= block.range.start
                                && block.range.end <= edit.range.end
                        })
            }) {
                return Ok(None);
            }
            for patch in source_patches.iter().filter(|_| !authored_syntax) {
                let old = self
                    .state()
                    .source
                    .bytes_in(patch.range.clone())
                    .ok_or(DocumentError::VerificationFailed)?;
                for bytes in [&old, &patch.replacement] {
                    let decoded = self.encoding().decode_region(bytes, patch.range.start)?;
                    if !normalize(&decoded, self.file_format()).endings.is_empty() {
                        if !retained_separator.as_ref().is_some_and(|separator| {
                            separator.start < patch.range.start
                                && patch.range.end <= separator.end
                        }) {
                            return Ok(None);
                        }
                    }
                }
            }
        }
        if self.format().is_markdown()
            && edits.iter().any(|edit| {
                (!authored_syntax && edit.replacement.contains(['`', '~']))
                    || self
                        .projection()
                        .blocks_for_region(&edit.range)
                        .iter()
                        .any(|block| {
                            block.markdown_html
                                || super::edit_boundary::is_code_paragraph(self, block)
                                    .unwrap_or(true)
                        })
            })
        {
            return Ok(None);
        }
        if edits.is_empty() {
            return Ok(None);
        }
        // Markdown Source rows are regional source rows: the regional proof
        // below maps rows that follow an added or removed break by the change
        // in the row count.
        let splits_rows = self.format() == Format::MarkdownSource;
        for edit in edits.iter().filter(|_| !self.format().is_literal()) {
            let replaced = self
                .projection()
                .text_tree()
                .slice(edit.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if edit.replacement.contains('\r')
                || replaced.contains('\r')
                || (!splits_rows && (edit.replacement.contains('\n') || replaced.contains('\n')))
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
            if edit.range.start < line_range.start
                || (!self.format().is_literal() && !splits_rows && edit.range.end > line_range.end)
            {
                return Ok(None);
            }
            first_line = first_line.min(line);
            last_line = last_line.max(if self.format().is_literal() || splits_rows {
                self.projection()
                    .hard_line_at_offset(edit.range.end)
                    .unwrap_or(line)
            } else {
                line
            });
        }
        if self.format() == Format::MarkdownSource {
            let in_list = |line| {
                self.projection()
                    .hard_line_range(line)
                    .is_some_and(|range| {
                        self.projection()
                            .blocks_for_region(&range)
                            .iter()
                            .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }))
                    })
            };
            // Inline syntax such as a link can span every row of its
            // paragraph, so the region covers the edited rows' whole blocks.
            let edited = self.projection().hard_line_range(first_line).unwrap().start
                ..self.projection().hard_line_range(last_line).unwrap().end;
            let (mut block_first, mut block_last) = (first_line, last_line);
            for block in self.projection().blocks_for_region(&edited) {
                if block.range.end < edited.start || block.range.start > edited.end {
                    continue;
                }
                let (Some(first), Some(last)) = (
                    self.projection().hard_line_at_offset(block.range.start),
                    self.projection().hard_line_at_offset(block.range.end),
                ) else {
                    return Ok(None);
                };
                block_first = block_first.min(first);
                block_last = block_last.max(last);
            }
            if block_last - block_first <= MAX_LINE_LOCAL_PROJECTION_HARD_LINES {
                (first_line, last_line) = (block_first, block_last);
            } else if !self.markdown_source_edit_is_row_local(edits, first_line..last_line + 1) {
                // A paragraph too long to reparse keeps a row-sized region,
                // which is exact only for edits that cannot reach another row.
                return Ok(None);
            }
            let edited_in_list = (first_line..=last_line).any(in_list);
            // Structural list syntax depends on its enclosing stack and can
            // change following siblings. Literal body edits retain the
            // validated old context without reparsing prefixes; any other
            // list edit reparses the whole list, which the regional proof
            // requires to parse alone as it does in the document.
            let whole_list =
                edited_in_list && !self.can_inherit_markdown_source_list_context(edits);
            // A source marker can change whether either adjacent physical
            // break is prose whitespace, so the region captures unchanged
            // neighboring rows on each side, bounded by the regional limit.
            let neighbor_is_code = |line| {
                self.projection()
                    .hard_line_range(line)
                    .is_some_and(|range| {
                        self.projection()
                            .blocks_for_region(&range)
                            .iter()
                            .any(|block| {
                                block.markdown_html
                                    || super::edit_boundary::is_code_paragraph(self, block)
                                        .unwrap_or(true)
                            })
                    })
            };
            // A closing fence cannot be parsed in isolation as the preceding
            // neighbor: it would become an opening fence. Code boundaries are
            // already structural, so prose edits need no context inside them.
            //
            // An empty row's existence depends on the lines around it, so a
            // region never ends on one: it extends to the next row with text.
            let empty = |line| {
                self.projection()
                    .hard_line_range(line)
                    .is_some_and(|range| {
                        self.projection()
                            .text_tree()
                            .slice(range)
                            .is_ok_and(|text| {
                                text[super::markdown_quotes::prefix(&text)..]
                                    .trim()
                                    .is_empty()
                            })
                    })
            };
            // A neighboring list parses independently only from its first
            // item, so an edit outside a list takes in the whole adjacent list
            // as context. The regional proof rejects a list that is cut off.
            let extends = |line| empty(line) || ((!edited_in_list || whole_list) && in_list(line));
            while first_line > 0 && !neighbor_is_code(first_line - 1) {
                first_line -= 1;
                if !extends(first_line)
                    || last_line - first_line > MAX_LINE_LOCAL_PROJECTION_HARD_LINES
                {
                    break;
                }
            }
            while last_line + 1 < self.projection().hard_line_count()
                && !neighbor_is_code(last_line + 1)
            {
                last_line += 1;
                if !extends(last_line)
                    || last_line - first_line > MAX_LINE_LOCAL_PROJECTION_HARD_LINES
                {
                    break;
                }
            }
            // A fence inside a list inherits the preceding item's ownership.
            // Cutting there is safe for body typing, but a structural edit may
            // release the fenced block or change its nesting and later items.
            if !self.can_inherit_markdown_source_list_context(edits) {
                for line in [
                    first_line.checked_sub(1),
                    (last_line + 1 < self.projection().hard_line_count()).then_some(last_line + 1),
                ]
                .into_iter()
                .flatten()
                .filter(|&line| neighbor_is_code(line))
                {
                    let row = self.projection().hard_line_range(line).unwrap();
                    let text = self
                        .projection()
                        .text_tree()
                        .slice(row)
                        .map_err(DocumentError::FormattedTextStorage)?;
                    if text.starts_with([' ', '\t', '>']) {
                        return Ok(None);
                    }
                }
            }
            // Parser state carries through a paragraph, so a region starts
            // and ends on block boundaries when they fit. Otherwise the
            // regional proof decides whether a cut paragraph is unaffected.
            if !edited_in_list || whole_list {
                let owner = |line| {
                    self.projection()
                        .hard_line_range(line)
                        .and_then(|row| markdown_source_row_owner(self.projection(), &row))
                };
                let owner_end = owner(last_line)
                    .and_then(|block| self.projection().hard_line_at_offset(block.range.end));
                if let Some(end) = owner_end
                    .filter(|&end| end - first_line <= MAX_LINE_LOCAL_PROJECTION_HARD_LINES)
                {
                    last_line = last_line.max(end);
                }
                let owner_start = owner(first_line)
                    .and_then(|block| self.projection().hard_line_at_offset(block.range.start));
                if let Some(start) = owner_start
                    .filter(|&start| last_line - start <= MAX_LINE_LOCAL_PROJECTION_HARD_LINES)
                {
                    first_line = first_line.min(start);
                }
            }
        } else if self.format() == Format::Markdown {
            let start = self.projection().hard_line_range(first_line).unwrap().start;
            let end = self.projection().hard_line_range(last_line).unwrap().end;
            let blocks = self.projection().blocks_for_region(&(start..end));
            if let (Some(first), Some(last)) = (blocks.first(), blocks.last()) {
                first_line = first_line.min(
                    self.projection()
                        .hard_line_at_offset(first.range.start)
                        .unwrap(),
                );
                last_line = last_line.max(
                    self.projection()
                        .hard_line_at_offset(last.range.end)
                        .unwrap(),
                );
            }
            // Prose can absorb the next physical line as a setext underline
            // or continuation. Verify sensitive neighbors with the edit
            // instead of inheriting their old projection.
            if last_line + 1 < self.projection().hard_line_count() {
                let next = self.projection().hard_line_range(last_line + 1).unwrap();
                let neighbors = self.projection().blocks_for_region(&next);
                if retained_separator.is_none() && blocks.iter().any(|block| {
                    block.range.is_empty() && block.kind == super::BlockKind::Paragraph
                })
                {
                    if neighbors.iter().any(|block| {
                        block.markdown_html
                            || super::edit_boundary::is_code_paragraph(self, block).unwrap_or(true)
                    }) {
                        return Ok(None);
                    }
                    if let Some(last) = neighbors.last() {
                        last_line = self.projection().hard_line_at_offset(last.range.end).unwrap();
                    }
                }
            }
            if authored_syntax {
                let in_list = |line| {
                    self.projection()
                        .hard_line_range(line)
                        .is_some_and(|range| {
                            self.projection()
                                .blocks_for_region(&range)
                                .iter()
                                .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }))
                        })
                };
                if (first_line..=last_line).any(in_list) {
                    // Authored inline punctuation must be parsed with its
                    // list stack. A continuation's indentation alone has a
                    // different meaning at a standalone parser restart.
                    while first_line > 0 && in_list(first_line - 1) {
                        first_line -= 1;
                        if last_line - first_line >= MAX_LINE_LOCAL_PROJECTION_HARD_LINES {
                            return Err(DocumentError::UnsupportedFormatting.into());
                        }
                    }
                    while last_line + 1 < self.projection().hard_line_count() && in_list(last_line + 1) {
                        last_line += 1;
                        if last_line - first_line >= MAX_LINE_LOCAL_PROJECTION_HARD_LINES {
                            return Err(DocumentError::UnsupportedFormatting.into());
                        }
                    }
                }
            }
        }
        let hard_lines = first_line..last_line.saturating_add(1);
        if hard_lines.is_empty()
            || (!self.format().is_literal()
                && hard_lines.len() > MAX_LINE_LOCAL_PROJECTION_HARD_LINES)
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
        let mut old_formatted = first_formatted.start..last_formatted.end;
        if self.format().is_literal() && old_formatted.len() > 2 * LITERAL_EDIT_CONTEXT_BYTES {
            let first_edit = edits
                .first()
                .ok_or(DocumentError::VerificationFailed)?
                .range
                .start;
            let last_edit = edits
                .last()
                .ok_or(DocumentError::VerificationFailed)?
                .range
                .end;
            if let Some((extent, _)) = self
                .projection()
                .literal_projection_extent(&(first_edit..last_edit))
            {
                // Compact mapping runs have bounded byte size and break at
                // hard-line separators. The unchanged outer block/line tails
                // are retained by the persistent projection splice.
                old_formatted =
                    extent.start.max(first_formatted.start)..extent.end.min(last_formatted.end);
            }
        }

        // Equal row counts do not establish a Markdown mapping: paragraph
        // separators can hide source rows while inline <br> creates others.
        let mut source_lines = if !self.format().is_markdown()
            && self.state().source_hard_lines.len()
            == self.projection().hard_line_count()
        {
            hard_lines.clone()
        } else if self.format().is_markdown() {
            let provenance = self.projection().provenance_for_region(&old_formatted);
            let (source_start, source_end) = match (provenance.first(), provenance.last()) {
                (Some(first), Some(last)) => (first.source.start, last.source.end),
                _ => {
                    // An empty paragraph contributes no provenance span; its
                    // editable boundary still identifies the physical row.
                    let Ok(range) = super::rich_text::text_source_range(self, &old_formatted)
                    else {
                        return Ok(None);
                    };
                    (range.start, range.end.max(range.start + 1))
                }
            };
            let Some(first) = self.state().source_hard_lines.line_at_offset(source_start) else {
                return Ok(None);
            };
            let Some(last) = self
                .state()
                .source_hard_lines
                .line_at_offset(source_end.saturating_sub(1))
            else {
                return Ok(None);
            };
            // The final empty formatted line owns the terminal empty
            // physical source row as well; end-1 identifies its preceding
            // delimiter and would otherwise omit that source record.
            first..if last_line + 1 == self.projection().hard_line_count() {
                self.state().source_hard_lines.len()
            } else {
                last + 1
            }
        } else {
            return Ok(None);
        };
        let old_source_start = self
            .state()
            .source_hard_lines
            .get(source_lines.start)
            .ok_or(DocumentError::VerificationFailed)?
            .start;
        let old_source_end = if let Some(separator) = &retained_separator {
            source_lines.end = self.state().source_hard_lines.line_at_offset(separator.end)
                .ok_or(DocumentError::VerificationFailed)?;
            separator.end
        } else if last_line + 1 == self.projection().hard_line_count() {
            self.state()
                .source_hard_lines
                .get(source_lines.end - 1)
                .ok_or(DocumentError::VerificationFailed)?
                .end
        } else {
            self.projection()
                .source_range(last_formatted.end..last_formatted.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?
                .start
        };
        let mut old_source = old_source_start..old_source_end;
        if (self.format().is_literal())
            && (old_formatted.start != first_formatted.start
                || old_formatted.end != last_formatted.end)
        {
            old_source = self
                .projection()
                .source_range(old_formatted.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
        }
        if source_patches.iter().any(|patch| {
            patch.part != SourcePartId::PRIMARY
                || patch.range.start < old_source.start
                || patch.range.end > old_source.end
        }) {
            return Ok(None);
        }
        if authored_syntax && old_source.len() > 32768 {
            return Err(DocumentError::UnsupportedFormatting.into());
        }

        Ok(Some(LineLocalProjectionRegion {
            hard_lines,
            source_lines,
            old_formatted,
            old_source,
            retained_separator,
        }))
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
    // Configuration updates historically leave semantic bold unchanged. Keep
    // that policy separate from ordinary source/style cascade merging.
    let bold = target.bold;
    target.merge_declarations(declarations);
    target.bold = bold;
}

fn clear_character_properties(
    target: &mut CharacterProperties,
    properties: &BTreeSet<StyleProperty>,
    expected: StylePropertyTarget,
) -> Result<(), ModelTransactionError> {
    for property in properties {
        if !target.clear_declaration(*property) {
            return Err(invalid_style_property_target(*property, expected));
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
    validate_percentage_font_contexts(projection, style_sheet, document_style, None)
}

/// Only extreme point sizes can overflow or underflow an allowed percentage.
/// Inspect declarations/identities first; ordinary edits do not resolve every
/// character interval merely because the sheet contains a relative size.
fn validate_percentage_font_contexts(
    projection: &FormattedDocument,
    sheet: &StyleSheet,
    document_style: &DocumentStyleAssignment,
    affected: Option<Range<usize>>,
) -> Result<(), StyleError> {
    use super::FontSize;
    if !sheet
        .character_styles()
        .any(|style| matches!(style.properties.size, Some(FontSize::Percentage(_))))
    {
        return Ok(());
    }
    let extreme = |size: f32| size > f32::MAX / 10.0 || size < f32::from_bits(10);
    let mut extreme_styles = BTreeSet::new();
    for style in sheet
        .block_styles()
        .filter(|style| style.role == super::BlockRole::Paragraph)
    {
        let resolved = sheet.resolve_assigned_paragraph_style(
            document_style,
            &style.id,
            &BlockProperties::default(),
            &CharacterProperties::default(),
            None,
            &CharacterProperties::default(),
        )?;
        if extreme(resolved.character.size) {
            extreme_styles.insert(style.id.clone());
        }
    }
    let could_be_extreme = |style: &StyleId, properties: &CharacterProperties| match properties.size
    {
        Some(FontSize::Points(size)) => extreme(size),
        Some(FontSize::Percentage(_)) => true,
        None => extreme_styles.contains(style),
    };
    let mut ranges = BTreeSet::new();
    let blocks = affected
        .as_ref()
        .map(|range| projection.blocks_for_region(range));
    for block in blocks.as_deref().unwrap_or_else(|| projection.blocks()) {
        if !block.range.is_empty()
            && could_be_extreme(&block.style, &block.direct_default_character)
        {
            ranges.insert((block.range.start, block.range.end));
        }
    }

    for (start, end) in ranges {
        let range = start..end;
        let mut boundaries = BTreeSet::from([start, end]);
        for span in projection.style_spans_for_region(&range) {
            boundaries.extend([span.range.start.max(start), span.range.end.min(end)]);
        }
        for block in projection.blocks_for_region(&range) {
            boundaries.extend([block.range.start.max(start), block.range.end.min(end)]);
        }
        for at in boundaries.into_iter().filter(|at| *at < end) {
            // A hard-line boundary has no font-bearing text of its own.
            if !projection
                .blocks_for_region(&(at..at))
                .iter()
                .any(|block| block.range.contains(&at))
            {
                continue;
            }
            if super::rich_text::resolved_character_at_with_style_context(
                projection,
                at,
                sheet,
                document_style,
            )
            .is_none()
            {
                return Err(StyleError::InvalidStylePropertyValue {
                    style: sheet.base_paragraph.clone(),
                    property: StyleProperty::CharacterSize,
                });
            }
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
            &after_sheet.base_paragraph
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
            super::BlockRole::Paragraph
            | super::BlockRole::Quote
            | super::BlockRole::CodeBlock
            | super::BlockRole::List
            | super::BlockRole::ListItem
            | super::BlockRole::Table => {
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
        let after_id = after_sheet.character_style(id).map(|_| id);
        // Equal values in Base Paragraph do not imply equal values elsewhere:
        // clearing an explicit 14pt declaration exposes a heading's 24pt size.
        // Compare sparse named chains as well, so declarations that begin or
        // stop masking paragraph context invalidate their dependent ranges.
        // This depends only on style ancestry, never a whole-document traversal.
        changed.extend(
            before_sheet
                .named_character_declarations(Some(id))?
                .changed_properties(&after_sheet.named_character_declarations(after_id)?),
        );
        let after = after_sheet.resolve_assigned_paragraph_style(
            after_assignment,
            &after_sheet.base_paragraph,
            &BlockProperties::default(),
            &CharacterProperties::default(),
            after_id,
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

fn semantic_style_coverage(spans: &[StyleSpan], style: SemanticInlineStyle) -> Vec<Range<usize>> {
    let mut ranges = spans
        .iter()
        .filter(|span| span.application == StyleApplication::Semantic(style))
        .map(|span| span.range.clone())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut coverage: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = coverage.last_mut().filter(|previous| range.start <= previous.end) {
            previous.end = previous.end.max(range.end);
        } else {
            coverage.push(range);
        }
    }
    coverage
}

fn subtract_style_coverage(range: &Range<usize>, coverage: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut uncovered = Vec::new();
    let mut at = range.start;
    for covered in coverage {
        if covered.end <= at {
            continue;
        }
        if range.end <= covered.start {
            break;
        }
        if at < covered.start {
            uncovered.push(at..covered.start);
        }
        at = at.max(covered.end);
    }
    if at < range.end {
        uncovered.push(at..range.end);
    }
    uncovered
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

    // Semantic emphasis is a boolean treatment. Extending an existing scope
    // may merge adjacent spans, but must produce exactly the requested coverage
    // and retain every unrelated style span, including nested treatments.
    let expected_coverage = if enabled {
        semantic_style_coverage(&expected, style)
    } else {
        semantic_style_coverage(before, style)
            .into_iter()
            .flat_map(|covered| subtract_style_coverage(&covered, std::slice::from_ref(range)))
            .collect()
    };
    if expected_coverage != semantic_style_coverage(after, style) {
        return false;
    }
    expected.retain(|span| span.application != StyleApplication::Semantic(style));
    let mut unmatched = after
        .iter()
        .filter(|span| span.application != StyleApplication::Semantic(style))
        .cloned()
        .collect::<Vec<_>>();

    // Named Code is the editable appearance of the semantic code marker.
    // Its assignment is created/removed by the same source delimiters.
    if style == SemanticInlineStyle::Code {
        let named = StyleSpan {
            range: range.clone(),
            application: StyleApplication::Named("Code".into()),
        };
        if enabled {
            expected.push(named);
        } else if let Some(index) = expected.iter().position(|span| span == &named) {
            expected.remove(index);
        }
    }
    if expected.len() != unmatched.len() {
        return false;
    }
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

/// The block that owns a whole Markdown Source row.
fn markdown_source_row_owner(
    projection: &FormattedDocument,
    row: &Range<usize>,
) -> Option<super::Block> {
    projection
        .blocks_for_region(row)
        .into_iter()
        .find(|block| block.range.start <= row.start && row.end <= block.range.end)
}

/// Markdown's hard break: a row ending in two spaces or a backslash.
fn markdown_row_has_hard_break(row: &str) -> bool {
    row.ends_with("  ") || row.ends_with('\\')
}

/// What a Markdown Source parse determines inside `range`: block kinds and
/// styles, flow rows and style spans, clipped to the range and made relative
/// to its start. Regional reparses are compared with the published
/// projection through this signature.
#[derive(Debug, PartialEq)]
struct MarkdownSourceRegionSignature {
    blocks: Vec<(
        Range<usize>,
        std::sync::Arc<super::projection::BlockAttributes>,
    )>,
    flows: Option<Vec<Range<usize>>>,
    styles: Vec<(Range<usize>, super::StyleApplication)>,
}

fn markdown_regional_attributes(
    block: &super::Block,
) -> std::sync::Arc<super::projection::BlockAttributes> {
    if block.containers.is_empty() {
        return block.attributes.clone();
    }
    let mut attributes = (*block.attributes).clone();
    attributes.containers = attributes
        .containers
        .iter()
        .map(|member| {
            let mut member = member.clone();
            member.starts_here = false;
            member.ends_here = false;
            member
        })
        .collect::<Vec<_>>()
        .into();
    std::sync::Arc::new(attributes)
}

fn markdown_source_region_signature(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> MarkdownSourceRegionSignature {
    let clip = |inner: &Range<usize>| {
        inner.start.max(range.start) - range.start..inner.end.min(range.end) - range.start
    };
    MarkdownSourceRegionSignature {
        blocks: projection
            .blocks_for_region(range)
            .into_iter()
            .map(|block| (clip(&block.range), markdown_regional_attributes(&block)))
            .collect(),
        flows: projection
            .flow_ranges_for_region(range)
            .map(|flows| flows.iter().map(clip).collect()),
        styles: projection
            .style_spans_for_region(range)
            .into_iter()
            .map(|span| (clip(&span.range), span.application))
            .collect(),
    }
}

/// What a Markdown Source parse determines for one row: the kind and style
/// of the block that owns it and the style spans on it, relative to the row.
fn markdown_source_row_signature(
    projection: &FormattedDocument,
    row: &Range<usize>,
) -> (
    Option<std::sync::Arc<super::projection::BlockAttributes>>,
    Vec<(Range<usize>, super::StyleApplication)>,
) {
    let owner = markdown_source_row_owner(projection, row)
        .map(|block| markdown_regional_attributes(&block));
    let styles = projection
        .style_spans_for_region(row)
        .into_iter()
        .map(|span| {
            (
                span.range.start.max(row.start) - row.start
                    ..span.range.end.min(row.end) - row.start,
                span.application,
            )
        })
        .collect();
    (owner, styles)
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
    encoding: super::Encoding,
) -> String {

    let escape_markdown = format == Format::Markdown && (!in_code || payload.text().contains('`'));
    let mut syntax = String::with_capacity(payload.text().len());
    let mut start = 0usize;
    for &hard_break in payload.break_offsets() {
        let segment = &payload.text()[start..hard_break];
        if escape_markdown {
            syntax.push_str(&if in_code {
                escape_markdown_insert(segment)
            } else {
                escape_markdown_insert_in_encoding(segment, encoding)
            });
        } else {
            syntax.push_str(segment);
        }
        syntax.push_str(file_format.spelling());
        if format == Format::Markdown && !in_code {
            syntax.push_str(file_format.spelling());
        }
        start = hard_break
            .checked_add(1)
            .expect("validated payload break offset is representable");
    }
    let segment = &payload.text()[start..];
    if escape_markdown {
        syntax.push_str(&if in_code {
            escape_markdown_insert(segment)
        } else {
            escape_markdown_insert_in_encoding(segment, encoding)
        });
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
    source_backed_reprojection_edits_with_patches(before, after, &[])
}

fn source_backed_reprojection_edits_with_patches(
    before: &FormattedDocument,
    after: &FormattedDocument,
    patches: &[SourcePatch],
) -> Vec<TextEdit> {
    let mut deltas = Vec::with_capacity(patches.len() + 1);
    deltas.push(0i128);
    for patch in patches {
        deltas.push(
            deltas.last().unwrap() + patch.replacement.len() as i128 - patch.range.len() as i128,
        );
    }
    // Only unchanged source-backed graphemes retain identity. Rebase their
    // physical ranges before matching the new projection; equal ordinal
    // ranges on opposite sides of an insertion are unrelated source bytes.
    let rebase = |source: Range<usize>| {
        let index = patches.partition_point(|patch| patch.range.end <= source.start);
        if patches
            .get(index)
            .is_some_and(|patch| patch.range.start < source.end)
        {
            return None;
        }
        let delta = deltas[index];
        Some((
            (source.start as i128 + delta) as usize,
            (source.end as i128 + delta) as usize,
        ))
    };
    reprojection_edits_matching_sources(before, after, rebase)
}

fn reprojection_edits_matching_sources(
    before: &FormattedDocument,
    after: &FormattedDocument,
    rebase: impl Fn(Range<usize>) -> Option<(usize, usize)>,
) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    let mut old_start = 0;
    let mut new_start = 0;
    let mut retain = |offset: usize, item: &str, new_offset: usize, new_item: &str| {
        if new_offset < new_start || item != new_item {
            return;
        }
        if old_start != offset || new_start != new_offset {
            edits.push(TextEdit::new(
                old_start..offset,
                &after.text()[new_start..new_offset],
            ));
        }
        old_start = offset + item.len();
        new_start = new_offset + item.len();
    };
    if before.has_monotonic_grapheme_provenance() && after.has_monotonic_grapheme_provenance() {
        // Both adapters retain source order: merge the two frontiers directly.
        // Reordered or genuinely relational projections use exact-key lookup.
        let mut target = after
            .source_grapheme_ranges()
            .filter_map(|(at, item, source)| {
                source.map(|source| ((source.start, source.end), at, item))
            })
            .peekable();
        for (at, item, source) in before.source_grapheme_ranges() {
            let Some(key) = source.and_then(&rebase) else {
                continue;
            };
            while target.peek().is_some_and(|next| next.0 < key) {
                target.next();
            }
            if target.peek().is_some_and(|next| next.0 == key) {
                let (_, new_at, new_item) = target.next().unwrap();
                retain(at, item, new_at, new_item);
            }
        }
    } else {
        let mut new_items = std::collections::HashMap::new();
        for (offset, item, source) in after.source_grapheme_ranges() {
            if let Some(source) = source {
                new_items.insert((source.start, source.end), (offset, item));
            }
        }
        for (offset, item, source) in before.source_grapheme_ranges() {
            let Some(key) = source.and_then(&rebase) else {
                continue;
            };
            if let Some(&(new_offset, new_item)) = new_items.get(&key) {
                retain(offset, item, new_offset, new_item);
            }
        }
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
    // A source-only change can leave every projected grapheme intact. Its
    // empty edit list is the identity map, including for an empty document.
    if edits.is_empty() {
        return Ok(Vec::new());
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

#[cfg(test)]
mod scratch_history_tests {
    use super::*;

    #[test]
    fn scratch_planning_shares_live_trees_without_reaccounting_the_document() {
        let source = "plain **bold** tail\n\n".repeat(10_000);
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            super::super::Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let original_history = document.history_status();
        let (mut scratch, work) =
            super::super::measure_document_work(|| document.scratch_document());
        assert_eq!(work.source_full_materialized_bytes, 0, "{work:?}");
        assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
        assert_eq!(scratch.history_status().retained_memory_bytes, 0);
        for _ in 0..8 {
            scratch.insert(1, "X").unwrap();
            assert_eq!(scratch.history_status().node_count, 1);
        }
        assert_eq!(
            scratch.projection().text_tree().slice(0..13).unwrap(),
            "pXXXXXXXXlain"
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.history_status(), original_history);
    }
}

#[cfg(test)]
mod prepared_group_reuse_tests {
    use super::*;

    fn request(document: &Document) -> ModelRequest {
        ModelRequest::SetMarkdownSource {
            document: document.id(),
            revision: document.revision(),
            source: false,
        }
    }

    #[test]
    fn closed_group_reuse_is_explicit_and_other_preconditions_remain_strict() {
        let mut document = Document::from_bytes(b"__word__".to_vec(), super::super::Encoding::Utf8, Format::MarkdownSource).unwrap();
        document.begin_edit_group();
        document.insert(0, "X").unwrap();
        let prepared = document.prepare_model_request(request(&document)).unwrap();
        document.end_edit_group();
        assert!(matches!(
            document.commit_model_transaction(prepared),
            Err(ModelTransactionError::StaleDocumentState)
        ));
        assert_eq!(document.text(), "X__word__");

        document.begin_edit_group();
        let prepared = document.prepare_model_request(request(&document)).unwrap();
        document.end_edit_group();
        let prepared = document
            .rebind_prepared_after_group_close(prepared)
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        // Intraword underscores are literal in GFM.
        assert_eq!(document.text(), "X__word__");
        assert!(document.undo());
        assert_eq!(document.text(), "X__word__");
        assert!(document.undo());
        assert_eq!(document.text(), "__word__");

        let prepared = document.prepare_model_request(request(&document)).unwrap();
        document.insert(0, "new").unwrap();
        assert!(matches!(
            document.rebind_prepared_after_group_close(prepared),
            Err(ModelTransactionError::StaleRevision { .. })
        ));
        let prepared = document.prepare_model_request(request(&document)).unwrap();
        document.begin_edit_group();
        document.end_edit_group();
        assert!(matches!(
            document.rebind_prepared_after_group_close(prepared),
            Err(ModelTransactionError::StaleDocumentState)
        ));
        document.begin_edit_group();
        let prepared = document.prepare_model_request(request(&document)).unwrap();
        assert!(matches!(
            document.rebind_prepared_after_group_close(prepared),
            Err(ModelTransactionError::StaleDocumentState)
        ));
    }
}

#[cfg(test)]
mod prepared_cursor_tests {
    use super::*;

    #[test]
    fn prepared_cursor_checks_graphemes_and_snapshot_identity() {
        let mut document = Document::new("e");
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(1..1, "\u{301}")],
            })
            .unwrap();
        assert_eq!(
            document.prepared_text_point(&prepared, 1),
            Err(DocumentError::NotGraphemeBoundary(1))
        );
        document.prepared_text_point(&prepared, 3).unwrap();
        document.insert(0, "x").unwrap();
        assert!(matches!(
            document.prepared_text_point(&prepared, 0),
            Err(DocumentError::WrongSnapshot { .. })
        ));
        assert_eq!(
            Document::new("e").prepared_text_point(&prepared, 0),
            Err(DocumentError::WrongDocument)
        );
    }
}

fn formatted_text_difference(before: &str, after: &str) -> Vec<TextEdit> {
    if before == after {
        Vec::new()
    } else {
        vec![TextEdit::new(0..before.len(), after)]
    }
}
