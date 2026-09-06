//! Authoritative document state and reversible text projections.
//!
//! Source bytes live in persistent [`source::SourceSnapshot`] ropes. Formatted
//! UTF-8 is an immutable derived projection and is always regenerated and
//! verified before a source transaction is committed.

mod encoding;
mod formatted_text;
mod history;
mod line_endings;
mod persistence;
mod pipeline;
mod position;
mod projection;
mod range_index;
mod source;
mod source_line_index;
mod style;
mod transaction;
mod transfer;

pub use encoding::{BomPolicy, DecodingDiagnostic, DecodingDiagnosticKind, Encoding};
pub use formatted_text::{
    FormattedBufferId, FormattedLeafId, FormattedLeafInfo, FormattedLeafLocation,
    FormattedLeafRevision, FormattedTextError, FormattedTextTree, LeafBoundarySide,
    FORMATTED_TEXT_LEAF_BYTES,
};
pub use history::{
    HistoryBoundary, HistoryBranch, HistoryChangeNumber, HistoryError, HistoryLocation,
    HistoryNavigation, HistoryNodeDetails, HistoryNodeId, HistoryRestoration,
    HistoryRestorationSnapshot, HistoryRetentionPolicy, HistorySemanticChangeKind,
    HistorySemanticSummary, HistorySourcePatch, HistoryStatus, HistoryTransactionSummary,
};
pub use line_endings::{
    FileFormat, FileFormatOrigin, LineEndingEvidence, LineEndingOpenPolicy, LineEndingPolicyError,
};
pub use persistence::{
    execute_prepared_artifact_write, load_artifact, ArtifactBinding, ArtifactIdentity,
    ArtifactOverwrite, ArtifactPath, ArtifactStorageProvider, ArtifactWriteCompletion,
    ArtifactWriteCompletionStatus, ArtifactWriteIntent, ArtifactWritePurpose, ArtifactWriteReceipt,
    ArtifactWriteScope, ArtifactWriteToken, AtomicArtifactWrite, InMemoryArtifactStorage,
    InMemoryStorageError, LoadedArtifact, PersistenceError, PreparedArtifactWrite,
};
pub use pipeline::{
    PipelineCapabilityDecision, PipelineCapabilityReport, PipelineConfigurationIdentity,
    PipelineEditIntent, PipelineInputChange, PipelineInvalidationReason, PipelineInvalidationScope,
    PipelinePolicyRequest, StageCapabilityReport, StageEditDisposition, StageInvalidation,
    TransformationDirectionality, TransformationIdentity, TransformationPipelineSnapshot,
    TransformationStageConfiguration, TransformationStageRole, TransformationStageSnapshot,
    UnsupportedEditReason,
};
pub use position::{
    ActiveEndpoint, AdjacentRangePolicy, AnchorBacking, DeletionRecovery, DirectedSelection,
    MappingOutcome, NormalizedSelection, PositionDomain, PositionError, PositionMap,
    ProjectedBlockBoundary, ProjectedLeafBoundary, RangeSet, SourceAnchorProvenance, SourcePartId,
    SourcePoint, SourceRange, Splice, TextAnchor, TextRange, UnresolvableAnchor,
};
pub use projection::{
    Block, BlockKind, Format, FormattedDocument, FormattedPayloadError, FormattedTextPayload,
    HardLineInfo, HardLineQueryError, HardLineSnapshot, ProjectedSourceBoundary, ProvenanceSpan,
    SourceBoundaryRelation, SourceToTextError, StyleSpan,
};
pub use source::SourceArtifactDigest;
pub use style::{
    BlockProperties, BlockRole, BlockStyle, CharacterProperties, CharacterStyle, Color,
    ConfigurationStyleIntent, DocumentStyleAssignment, FontSlant, LineSpacing, ParagraphAlignment,
    ResolvedCharacterStyle, ResolvedDocumentStyle, ResolvedParagraphStyle, ResolvedStyle,
    SemanticInlineStyle, StyleApplication, StyleContribution, StyleContributionOrigin,
    StyleDefinitionEdit, StyleDefinitionFieldEdit, StyleDefinitionMetadata, StyleDefinitionOrigin,
    StyleDependency, StyleDependencyIndex, StyleError, StyleId, StyleInvalidationEffect,
    StyleNamespace, StyleProperty, StylePropertyValue, StyleSheet, StyleSheetRevision,
    WritingDirection,
};
pub use transaction::{
    CommittedModelTransaction, HistoryNavigationRequest, ModelChangeKind, ModelChangeSummary,
    ModelRequest, ModelTransactionError, PersistedStyleIntent, PreparedModelTransaction,
    ProjectionWorkScope, ProjectionWorkStatistics, SourcePatch, StyleBlockTarget,
    StyleChangeSummary, StyleModelIntent, StyleModelRequest, StylePropertyTarget,
    StyleTransactionError,
};
pub use transfer::HardLineTransfer;

use encoding::DecodedText;
use formatted_text::LogicalGraphemeSnapshot;
use history::History;
use line_endings::{normalize, open_interpretation};
use persistence::{ArtifactWriteResult, PendingArtifactWrite, PendingArtifactWriteKind};
use projection::{project, BlockIdentityError};
use source::SourceSnapshot;
use source_line_index::SourceHardLineIndex;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_document_id(next: &AtomicU64) -> Result<DocumentId, DocumentError> {
    next.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |candidate| {
        (candidate != 0).then(|| candidate.wrapping_add(1))
    })
    .map(DocumentId)
    .map_err(|_| DocumentError::DocumentIdentityExhausted)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct DocumentId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Revision(pub u64);

/// Auditable whole-document work performed while opening a source artifact.
///
/// Opening currently publishes a complete semantic projection. These counters
/// make its unavoidable eager work explicit and, in particular, ensure that
/// line-ending detection and state construction share one decoding result
/// rather than decoding the authoritative bytes twice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentOpenWorkStatistics {
    source_decode_passes: usize,
    decoded_source_bytes: usize,
    decoded_utf8_bytes: usize,
    projected_formatted_bytes: usize,
    projected_hard_lines: usize,
}

impl DocumentOpenWorkStatistics {
    fn complete(
        decoded_source_bytes: usize,
        decoded_utf8_bytes: usize,
        projection: &FormattedDocument,
    ) -> Self {
        Self {
            source_decode_passes: 1,
            decoded_source_bytes,
            decoded_utf8_bytes,
            projected_formatted_bytes: projection.text().len(),
            projected_hard_lines: projection.hard_line_count(),
        }
    }

    /// Number of complete passes through the encoding decoder.
    pub fn source_decode_passes(self) -> usize {
        self.source_decode_passes
    }

    /// Authoritative source bytes consumed by the decoder, including a BOM.
    pub fn decoded_source_bytes(self) -> usize {
        self.decoded_source_bytes
    }

    /// Valid UTF-8 bytes produced by decoding before line-ending projection.
    pub fn decoded_utf8_bytes(self) -> usize {
        self.decoded_utf8_bytes
    }

    /// UTF-8 bytes in the initial formatted projection.
    pub fn projected_formatted_bytes(self) -> usize {
        self.projected_formatted_bytes
    }

    /// Semantic hard lines in the initial formatted projection.
    pub fn projected_hard_lines(self) -> usize {
        self.projected_hard_lines
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Association {
    BeforeInsertion,
    AfterInsertion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryAffinity {
    Upstream,
    Downstream,
}

/// An exact boundary in one immutable formatted snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextPoint {
    document: DocumentId,
    revision: Revision,
    offset: usize,
}

/// Checked cross-domain result for one exact source boundary in the current
/// immutable document revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceToTextMapping {
    source: SourcePoint,
    text: TextPoint,
    affinity: BoundaryAffinity,
    relation: SourceBoundaryRelation,
}

impl SourceToTextMapping {
    pub fn source(self) -> SourcePoint {
        self.source
    }

    pub fn text(self) -> TextPoint {
        self.text
    }

    pub fn affinity(self) -> BoundaryAffinity {
        self.affinity
    }

    pub fn relation(self) -> SourceBoundaryRelation {
        self.relation
    }
}

impl TextPoint {
    pub fn document(self) -> DocumentId {
        self.document
    }

    pub fn revision(self) -> Revision {
        self.revision
    }

    pub fn offset(self) -> usize {
        self.offset
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub replacement: String,
}

impl TextEdit {
    pub fn new(range: Range<usize>, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HardLineStyleImage {
    range: Range<usize>,
    style: SemanticInlineStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HardLineDiagnosticImage {
    kind: DecodingDiagnosticKind,
    formatted_range: Range<usize>,
    source_range: Range<usize>,
}

/// Opaque, revision-bound copy of one source-backed hard line.
///
/// The image retains the exact authoritative bytes, including an existing
/// line terminator, together with enough projected semantics to verify a
/// later restoration. It is intentionally not a general clipboard payload:
/// it can only be restored into the same document, pipeline, and stable hard
/// line identity. This is the document-side primitive needed by Vim's `U`
/// command.
#[derive(Clone, Eq, PartialEq)]
pub struct HardLineSourceImage {
    document: DocumentId,
    revision: Revision,
    captured_line: usize,
    hard_line_count: usize,
    hard_line_id: u64,
    encoding: Encoding,
    format: Format,
    file_format: FileFormat,
    terminated: bool,
    source_bytes: Vec<u8>,
    formatted_text: String,
    block_kind: BlockKind,
    block_style: StyleId,
    styles: Vec<HardLineStyleImage>,
    diagnostics: Vec<HardLineDiagnosticImage>,
}

impl fmt::Debug for HardLineSourceImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HardLineSourceImage")
            .field("document", &self.document)
            .field("revision", &self.revision)
            .field("captured_line", &self.captured_line)
            .field("hard_line_count", &self.hard_line_count)
            .field("hard_line_id", &self.hard_line_id)
            .field("encoding", &self.encoding)
            .field("format", &self.format)
            .field("file_format", &self.file_format)
            .field("terminated", &self.terminated)
            .field("source_byte_len", &self.source_bytes.len())
            .field("formatted_text", &self.formatted_text)
            .field("block_kind", &self.block_kind)
            .field("block_style", &self.block_style)
            .field("styles", &self.styles)
            .field("diagnostics", &self.diagnostics)
            .finish()
    }
}

impl HardLineSourceImage {
    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Ordinal at capture time. Callers retain the stable ID as authority and
    /// may supply a different current ordinal after an explicitly tracked
    /// move.
    pub fn captured_line(&self) -> usize {
        self.captured_line
    }

    pub fn hard_line_id(&self) -> u64 {
        self.hard_line_id
    }

    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    pub fn source_byte_len(&self) -> usize {
        self.source_bytes.len()
    }

    fn same_restorable_content(&self, other: &Self) -> bool {
        self.hard_line_id == other.hard_line_id
            && self.encoding == other.encoding
            && self.format == other.format
            && self.file_format == other.file_format
            && self.terminated == other.terminated
            && self.source_bytes == other.source_bytes
            && self.formatted_text == other.formatted_text
            && self.block_kind == other.block_kind
            && self.block_style == other.block_style
            && self.styles == other.styles
            && self.diagnostics == other.diagnostics
    }
}

/// One replacement in formatted snapshot coordinates whose inserted content
/// explicitly distinguishes semantic hard-break items from literal U+000A.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedPayloadEdit {
    range: Range<usize>,
    payload: FormattedTextPayload,
}

impl FormattedPayloadEdit {
    pub fn new(range: Range<usize>, payload: FormattedTextPayload) -> Self {
        Self { range, payload }
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn payload(&self) -> &FormattedTextPayload {
        &self.payload
    }
}

/// Explicit target snapshot plus one atomic set of structured payload edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedPayloadEditRequest {
    document: DocumentId,
    revision: Revision,
    edits: Vec<FormattedPayloadEdit>,
}

impl FormattedPayloadEditRequest {
    pub fn new(document: DocumentId, revision: Revision, edits: Vec<FormattedPayloadEdit>) -> Self {
        Self {
            document,
            revision,
            edits,
        }
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn edits(&self) -> &[FormattedPayloadEdit] {
        &self.edits
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentError {
    InvalidEncoding {
        encoding: Encoding,
        offset: usize,
    },
    MismatchedBom,
    UnrepresentableCharacter {
        encoding: Encoding,
        character: char,
    },
    InvalidRange {
        start: usize,
        end: usize,
        length: usize,
    },
    NotGraphemeBoundary(usize),
    OverlappingEdits,
    WrongDocument,
    WrongSnapshot {
        expected: Revision,
        actual: Revision,
    },
    AmbiguousProjection,
    VerificationFailed,
    UnsupportedBom(Encoding),
    LineEndingConversionWouldReinterpretContent,
    /// Candidate source bytes could not reproduce both the payload's exact
    /// flat UTF-8 and its marked-vs-literal hard-break structure.
    FormattedPayloadCannotReproject,
    InvalidHardLineTransferRange {
        start: usize,
        end: usize,
        line_count: usize,
    },
    InvalidHardLineTransferDestination {
        destination: usize,
        line_count: usize,
    },
    HardLineTransferDestinationInsideSource {
        destination: usize,
        source: Range<usize>,
    },
    HardLineTransferProjectionMismatch,
    InvalidHardLineSourceImageTarget {
        line: usize,
        line_count: usize,
    },
    /// The requested ordinal no longer names the stable hard line captured by
    /// the image. This normally means a topology-changing operation made a
    /// retained Vim `U` slot stale.
    StaleHardLineSourceImage {
        target_line: usize,
        expected_id: u64,
        actual_id: u64,
    },
    IncompatibleHardLineSourceImage,
    HardLineSourceImageTopologyChanged {
        captured_line_count: usize,
        current_line_count: usize,
    },
    HardLineSourceImageTerminatorShapeChanged {
        captured_terminated: bool,
        current_terminated: bool,
    },
    HardLineSourceImageProjectionMismatch,
    UnsupportedFormatting,
    OverlappingFormatting,
    /// A reverse projection selected only part of an opaque malformed source
    /// item. Committing it could silently discard bytes, so no state changed.
    OpaqueDecodingConflict {
        source_range: Range<usize>,
    },
    DocumentIdentityExhausted,
    BlockIdentityExhausted,
    FormattedTextStorage(FormattedTextError),
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEncoding { encoding, offset } => {
                write!(
                    formatter,
                    "invalid {encoding:?} sequence at source byte {offset}"
                )
            }
            Self::MismatchedBom => formatter.write_str("byte-order mark contradicts encoding"),
            Self::UnrepresentableCharacter {
                encoding,
                character,
            } => write!(
                formatter,
                "{character:?} is not representable in {encoding:?}"
            ),
            Self::InvalidRange { start, end, length } => {
                write!(
                    formatter,
                    "invalid formatted range {start}..{end} for length {length}"
                )
            }
            Self::NotGraphemeBoundary(offset) => {
                write!(
                    formatter,
                    "offset {offset} splits an extended grapheme cluster"
                )
            }
            Self::OverlappingEdits => formatter.write_str("text edits overlap"),
            Self::WrongDocument => formatter.write_str("text point belongs to another document"),
            Self::WrongSnapshot { expected, actual } => write!(
                formatter,
                "stale text point from revision {}; current revision is {}",
                actual.0, expected.0
            ),
            Self::AmbiguousProjection => {
                formatter.write_str("formatted range has no unambiguous source mapping")
            }
            Self::VerificationFailed => {
                formatter.write_str("candidate source did not reproduce the requested edit")
            }
            Self::UnsupportedBom(encoding) => {
                write!(formatter, "{encoding:?} has no byte-order mark")
            }
            Self::LineEndingConversionWouldReinterpretContent => formatter
                .write_str("line-ending conversion would reinterpret literal CR or LF content"),
            Self::FormattedPayloadCannotReproject => formatter.write_str(
                "source format or line-ending mode cannot reproduce the structured payload",
            ),
            Self::InvalidHardLineTransferRange {
                start,
                end,
                line_count,
            } => write!(
                formatter,
                "hard-line transfer range {start}..{end} is invalid for {line_count} lines"
            ),
            Self::InvalidHardLineTransferDestination {
                destination,
                line_count,
            } => write!(
                formatter,
                "hard-line transfer destination {destination} is invalid for {line_count} lines"
            ),
            Self::HardLineTransferDestinationInsideSource {
                destination,
                source,
            } => write!(
                formatter,
                "hard-line move destination {destination} lies strictly inside source gap span {}..{}",
                source.start, source.end
            ),
            Self::HardLineTransferProjectionMismatch => formatter.write_str(
                "source-backed hard-line transfer did not reproduce the requested structure",
            ),
            Self::InvalidHardLineSourceImageTarget { line, line_count } => write!(
                formatter,
                "hard-line source image target {line} is invalid for {line_count} lines"
            ),
            Self::StaleHardLineSourceImage {
                target_line,
                expected_id,
                actual_id,
            } => write!(
                formatter,
                "hard-line source image targets stable line {expected_id}, but current line {target_line} has identity {actual_id}"
            ),
            Self::IncompatibleHardLineSourceImage => formatter.write_str(
                "hard-line source image belongs to an incompatible document pipeline",
            ),
            Self::HardLineSourceImageTopologyChanged {
                captured_line_count,
                current_line_count,
            } => write!(
                formatter,
                "hard-line topology changed from {captured_line_count} to {current_line_count} lines"
            ),
            Self::HardLineSourceImageTerminatorShapeChanged {
                captured_terminated,
                current_terminated,
            } => write!(
                formatter,
                "hard-line terminator shape changed from terminated={captured_terminated} to terminated={current_terminated}"
            ),
            Self::HardLineSourceImageProjectionMismatch => formatter.write_str(
                "restored source bytes did not reproduce the captured hard-line semantics",
            ),
            Self::UnsupportedFormatting => {
                formatter.write_str("the source format cannot represent that formatting edit")
            }
            Self::OverlappingFormatting => formatter.write_str(
                "this initial Markdown adapter cannot safely represent overlapping inline styles",
            ),
            Self::OpaqueDecodingConflict { source_range } => write!(
                formatter,
                "edit maps through only part of opaque source bytes {}..{}",
                source_range.start, source_range.end
            ),
            Self::DocumentIdentityExhausted => {
                formatter.write_str("document identities were exhausted")
            }
            Self::BlockIdentityExhausted => {
                formatter.write_str("projected block identities were exhausted")
            }
            Self::FormattedTextStorage(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DocumentError {}

/// Failures while translating a formatted hard-line span to an authoritative
/// source-byte extent for ranged serialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HardLineSourceRangeError {
    InvalidRange {
        start: usize,
        end: usize,
        line_count: usize,
    },
    ProjectionMismatch {
        source_line_count: usize,
        formatted_line_count: usize,
    },
    Document(DocumentError),
}

impl From<DocumentError> for HardLineSourceRangeError {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}

impl fmt::Display for HardLineSourceRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange {
                start,
                end,
                line_count,
            } => write!(
                formatter,
                "hard-line source range {start}..{end} is invalid for {line_count} lines"
            ),
            Self::ProjectionMismatch {
                source_line_count,
                formatted_line_count,
            } => write!(
                formatter,
                "source has {source_line_count} physical hard lines but the formatted projection has {formatted_line_count} hard lines"
            ),
            Self::Document(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HardLineSourceRangeError {}

#[derive(Clone)]
struct DocumentState {
    revision: Revision,
    source: SourceSnapshot,
    projection: FormattedDocument,
    /// Exact primary-source extents for the projection's hard lines. The
    /// persistent prefix-sum index is built by the shared decoding/line-ending
    /// stage. A regional edit replaces only touched line lengths, so every
    /// later absolute extent moves without an `O(n)` suffix rewrite.
    source_hard_lines: SourceHardLineIndex,
    encoding: Encoding,
    format: Format,
    file_format: FileFormat,
    file_format_origin: FileFormatOrigin,
    line_ending_evidence: LineEndingEvidence,
    has_bom: bool,
}

fn visit_document_state_source_buffers(
    state: &DocumentState,
    visitor: &mut dyn FnMut(usize, usize),
) {
    state.source.visit_retained_buffers(visitor);
}

fn document_state_source_digest(state: &DocumentState) -> SourceArtifactDigest {
    state.source.artifact_digest()
}

fn new_document_history(state: DocumentState) -> History<DocumentState, PositionMap> {
    History::new_accounted(
        state,
        HistoryRetentionPolicy::default(),
        visit_document_state_source_buffers,
        document_state_source_digest,
    )
}

/// One editing buffer's document state and branching undo history.
pub struct Document {
    id: DocumentId,
    history: History<DocumentState, PositionMap>,
    open_work: DocumentOpenWorkStatistics,
    next_revision: u64,
    /// Monotonic across the complete undo tree. History navigation restores
    /// recorded block assignments but never rewinds this allocator.
    next_projected_block_id: u64,
    edit_group_depth: usize,
    /// Advances whenever a new outer undo group begins, including an undo
    /// break which closes and immediately reopens Insert mode. This lets the
    /// coordinator retain the correct initiating restoration snapshot without
    /// inferring grouping from source revisions.
    edit_group_generation: u64,
    /// Short-lived exact transition accumulator installed by the serial core
    /// coordinator around one input event. Model transactions compose into it
    /// before publication, so a command which performs several source commits
    /// still exposes one exact map from the event's input snapshot to its
    /// output snapshot. Ordinary document users pay no logging cost.
    position_map_capture: Option<PositionMap>,
    artifact_binding: Option<ArtifactBinding>,
    pending_artifact_writes: HashMap<ArtifactWriteToken, PendingArtifactWrite>,
    next_artifact_write_token: u64,
    next_save_sequence: u64,
    last_successful_save_sequence: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("")
    }
}

impl Document {
    /// Create a UTF-8, Unix-line-ending plain-text document.
    pub fn new(text: impl Into<String>) -> Self {
        Self::try_new(text).expect("process-wide document identities were exhausted")
    }

    /// Fallible form of [`Self::new`] for long-lived hosts that require a
    /// typed no-reuse guarantee even at identity exhaustion.
    pub fn try_new(text: impl Into<String>) -> Result<Self, DocumentError> {
        let text = text.into();
        Self::from_bytes_with_file_format(
            text.into_bytes(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
    }

    /// Construct the real source/text-tree/block shape needed by a
    /// million-line layout test without also allocating per-scalar decoding
    /// and provenance fixtures. Encoding/provenance scalability has separate
    /// tests; this keeps the layout work bound measurable in isolation.
    #[cfg(test)]
    pub(crate) fn layout_test_with_line_count(line_count: usize) -> Self {
        assert!(line_count > 0);
        let mut text = "x\n".repeat(line_count - 1);
        text.push('x');
        let text_len = text.len();
        let source = SourceSnapshot::new(text.as_bytes().to_vec());
        let projection = projection::layout_test_plain_projection(text);
        let state = DocumentState {
            revision: Revision(0),
            source,
            projection,
            source_hard_lines: SourceHardLineIndex::new(
                (0..line_count)
                    .map(|line| {
                        let start = line * 2;
                        start..(start + 2).min(text_len)
                    })
                    .collect(),
            )
            .expect("layout fixture source-line ranges form a partition"),
            encoding: Encoding::Utf8,
            format: Format::PlainText,
            file_format: FileFormat::Unix,
            file_format_origin: FileFormatOrigin::Defaulted,
            line_ending_evidence: LineEndingEvidence {
                bare_lf: line_count - 1,
                ..LineEndingEvidence::default()
            },
            has_bom: false,
        };
        Self {
            id: allocate_document_id(&NEXT_DOCUMENT_ID)
                .expect("process-wide document identities were exhausted in a test fixture"),
            history: new_document_history(state),
            // This fixture deliberately bypasses decoding/provenance setup so
            // layout scalability can be measured in isolation.
            open_work: DocumentOpenWorkStatistics {
                source_decode_passes: 0,
                decoded_source_bytes: 0,
                decoded_utf8_bytes: 0,
                projected_formatted_bytes: text_len,
                projected_hard_lines: line_count,
            },
            next_revision: 1,
            next_projected_block_id: u64::try_from(line_count)
                .expect("fixture line count is representable")
                .checked_add(1)
                .expect("fixture block identity is representable"),
            edit_group_depth: 0,
            edit_group_generation: 0,
            position_map_capture: None,
            artifact_binding: None,
            pending_artifact_writes: HashMap::new(),
            next_artifact_write_token: 1,
            next_save_sequence: 1,
            last_successful_save_sequence: 0,
        }
    }

    /// Open source bytes and detect their line-ending interpretation.
    pub fn from_bytes(
        bytes: Vec<u8>,
        encoding: Encoding,
        format: Format,
    ) -> Result<Self, DocumentError> {
        Self::from_source(bytes, encoding, format, LineEndingOpenPolicy::default())
    }

    /// Detect the source encoding in core, then open source bytes and detect
    /// their line-ending interpretation. Detection is BOM-first, otherwise
    /// strict UTF-8 with ISO-8859-1 fallback; it never changes source bytes.
    pub fn from_bytes_detect_encoding(
        bytes: Vec<u8>,
        format: Format,
    ) -> Result<Self, DocumentError> {
        Self::from_source_detect_encoding(bytes, format, LineEndingOpenPolicy::default())
    }

    /// Open source bytes using an explicit line-ending interpretation.
    pub fn from_bytes_with_file_format(
        bytes: Vec<u8>,
        encoding: Encoding,
        format: Format,
        file_format: FileFormat,
    ) -> Result<Self, DocumentError> {
        Self::from_source(
            bytes,
            encoding,
            format,
            LineEndingOpenPolicy::forced(file_format),
        )
    }

    /// Detect the source encoding in core while using an explicit line-ending
    /// interpretation. Encoding detection and line-ending selection remain
    /// independent policies.
    pub fn from_bytes_detect_encoding_with_file_format(
        bytes: Vec<u8>,
        format: Format,
        file_format: FileFormat,
    ) -> Result<Self, DocumentError> {
        Self::from_source_detect_encoding(bytes, format, LineEndingOpenPolicy::forced(file_format))
    }

    /// Open source bytes through the shared line-ending interpretation
    /// component. The same policy applies before plain-text, Markdown, and
    /// future textual format adapters, so `fileformats` behavior is not
    /// reimplemented by each format.
    pub fn from_bytes_with_line_ending_policy(
        bytes: Vec<u8>,
        encoding: Encoding,
        format: Format,
        line_endings: LineEndingOpenPolicy,
    ) -> Result<Self, DocumentError> {
        Self::from_source(bytes, encoding, format, line_endings)
    }

    /// Construct a document from a provider read and retain its logical file
    /// binding for later ordinary saves. Format, encoding, and line-ending
    /// selection remain explicit core policy rather than provider behavior.
    pub fn from_loaded_artifact(
        loaded: LoadedArtifact,
        encoding: Encoding,
        format: Format,
        line_endings: LineEndingOpenPolicy,
    ) -> Result<Self, DocumentError> {
        let (binding, bytes) = loaded.into_parts();
        let mut document = Self::from_source(bytes, encoding, format, line_endings)?;
        document.artifact_binding = Some(binding);
        Ok(document)
    }

    fn from_source(
        bytes: Vec<u8>,
        encoding: Encoding,
        format: Format,
        line_endings: LineEndingOpenPolicy,
    ) -> Result<Self, DocumentError> {
        // Decode the caller-owned contiguous bytes before moving them into the
        // persistent source tree. Besides avoiding a whole-source copy at
        // open, the same decoded value is consumed by detection and state
        // construction, so opening performs exactly one decoder pass.
        let decoded = encoding.decode(&bytes)?;
        Self::from_decoded_source(bytes, decoded, format, line_endings)
    }

    fn from_source_detect_encoding(
        bytes: Vec<u8>,
        format: Format,
        line_endings: LineEndingOpenPolicy,
    ) -> Result<Self, DocumentError> {
        // Detection and decoding share their strict UTF-8 validation pass, so
        // automatic opening has the same bounded opening work as a forced
        // encoding while still retaining the original source bytes.
        let decoded = Encoding::detect_and_decode(&bytes)?;
        Self::from_decoded_source(bytes, decoded, format, line_endings)
    }

    fn from_decoded_source(
        bytes: Vec<u8>,
        decoded: DecodedText,
        format: Format,
        line_endings: LineEndingOpenPolicy,
    ) -> Result<Self, DocumentError> {
        let source_byte_len = bytes.len();
        let decoded_utf8_len = decoded.text.len();
        let (file_format, origin, evidence) = open_interpretation(&decoded.text, &line_endings);
        let revision = Revision(0);
        let source = SourceSnapshot::new(bytes);
        let mut state = build_state_from_decoded(
            source,
            decoded,
            format,
            file_format,
            origin,
            evidence,
            revision,
        )?;
        let next_projected_block_id = state
            .projection
            .assign_initial_block_ids(1)
            .map_err(block_identity_document_error)?;
        let open_work = DocumentOpenWorkStatistics::complete(
            source_byte_len,
            decoded_utf8_len,
            &state.projection,
        );
        Ok(Self {
            id: allocate_document_id(&NEXT_DOCUMENT_ID)?,
            history: new_document_history(state),
            open_work,
            next_revision: 1,
            next_projected_block_id,
            edit_group_depth: 0,
            edit_group_generation: 0,
            position_map_capture: None,
            artifact_binding: None,
            pending_artifact_writes: HashMap::new(),
            next_artifact_write_token: 1,
            next_save_sequence: 1,
            last_successful_save_sequence: 0,
        })
    }

    fn state(&self) -> &DocumentState {
        self.history.current()
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.state().revision
    }

    /// Work performed to construct the initial immutable projection.
    ///
    /// The value remains the opening measurement after edits and history
    /// navigation; per-transaction work is reported by
    /// [`ModelChangeSummary::projection_work`].
    pub fn open_work_statistics(&self) -> DocumentOpenWorkStatistics {
        self.open_work
    }

    pub fn encoding(&self) -> Encoding {
        self.state().encoding
    }

    pub fn format(&self) -> Format {
        self.state().format
    }

    pub fn file_format(&self) -> FileFormat {
        self.state().file_format
    }

    pub fn file_format_origin(&self) -> FileFormatOrigin {
        self.state().file_format_origin
    }

    pub fn line_ending_evidence(&self) -> LineEndingEvidence {
        self.state().line_ending_evidence
    }

    pub fn has_bom(&self) -> bool {
        self.state().has_bom
    }

    pub fn text(&self) -> &str {
        self.state().projection.text()
    }

    pub fn projection(&self) -> &FormattedDocument {
        &self.state().projection
    }

    /// Run one serial controller operation while composing every committed
    /// formatted-text transition into a single exact revision map.
    ///
    /// This is deliberately core-private: public callers should use explicit
    /// prepared/committed model transactions. The compatibility command
    /// interpreter still performs more than one model call for compound Vim
    /// commands, so the coordinator uses this bridge until every command emits
    /// one `CommandPlan`.
    pub(crate) fn capture_position_maps<R, E>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> (Result<R, E>, PositionMap) {
        // Compound core commands (for example `:normal` invoking a macro)
        // need a local transition in order to rebase their own persistent
        // targets while the coordinator simultaneously captures the complete
        // outer transition. Temporarily make this frame authoritative, then
        // compose it into its parent on every ordinary return, including an
        // error whose successful prefix committed document revisions.
        let parent_capture = self.position_map_capture.take();
        self.position_map_capture = Some(PositionMap::identity(
            self.id,
            PositionDomain::FormattedText,
            self.revision(),
            self.projection().text_tree().byte_len(),
        ));
        let result = operation(self);
        let map = self
            .position_map_capture
            .take()
            .expect("the active capture remains installed for the operation");
        self.position_map_capture = parent_capture.map(|parent| {
            parent
                .then(&map)
                .expect("a nested capture follows its parent transition exactly")
        });
        (result, map)
    }

    /// Structured diagnostics for malformed bytes in the current source
    /// snapshot. Each range is also present as one visible U+FFFD item in the
    /// formatted projection and can be explicitly selected and replaced.
    pub fn decoding_diagnostics(&self) -> &[DecodingDiagnostic] {
        self.state().projection.decoding_diagnostics()
    }

    /// Serialize the authoritative snapshot. This is never regenerated from
    /// formatted text, so a no-op open/save is byte exact.
    pub fn source_bytes(&self) -> Vec<u8> {
        self.state().source.bytes()
    }

    pub fn source_byte_len(&self) -> usize {
        self.state().source.len()
    }

    /// Current logical file binding. It is buffer metadata and is deliberately
    /// not restored by text undo/redo.
    pub fn artifact_binding(&self) -> Option<&ArtifactBinding> {
        self.artifact_binding.as_ref()
    }

    /// Finalize any open undo group and capture exact immutable source bytes
    /// for host-side persistence. No provider is invoked by this method.
    pub fn prepare_artifact_write(
        &mut self,
        intent: ArtifactWriteIntent,
    ) -> Result<PreparedArtifactWrite, PersistenceError> {
        self.close_edit_group();

        let history = self.history.status().current;
        let (destination, bytes, overwrite, purpose, kind) = match intent {
            ArtifactWriteIntent::Save { overwrite } => {
                let binding = self
                    .artifact_binding
                    .clone()
                    .ok_or(PersistenceError::NoCurrentArtifact)?;
                let sequence = self.allocate_save_sequence()?;
                (
                    binding.path().clone(),
                    self.state().source.bytes(),
                    overwrite,
                    ArtifactWritePurpose::Save,
                    PendingArtifactWriteKind::Save {
                        captured_binding: binding,
                        sequence,
                    },
                )
            }
            ArtifactWriteIntent::SaveAs {
                destination,
                overwrite,
            } => {
                let sequence = self.allocate_save_sequence()?;
                (
                    destination,
                    self.state().source.bytes(),
                    overwrite,
                    ArtifactWritePurpose::SaveAs,
                    PendingArtifactWriteKind::SaveAs { sequence },
                )
            }
            ArtifactWriteIntent::WriteAlternate {
                destination,
                scope,
                overwrite,
            } => {
                let bytes = match scope {
                    ArtifactWriteScope::WholeArtifact => self.state().source.bytes(),
                    ArtifactWriteScope::PrimarySourceBytes(range) => {
                        let source_length = self.state().source.len();
                        if range.start > range.end || range.end > source_length {
                            return Err(PersistenceError::InvalidSourceRange {
                                start: range.start,
                                end: range.end,
                                source_length,
                            });
                        }
                        self.state()
                            .source
                            .bytes_in(range)
                            .expect("the source range was validated against this snapshot")
                    }
                };
                (
                    destination,
                    bytes,
                    overwrite,
                    ArtifactWritePurpose::WriteAlternate,
                    PendingArtifactWriteKind::Alternate,
                )
            }
        };

        if self
            .pending_artifact_writes
            .values()
            .any(|pending| pending.prepared.storage.destination() == &destination)
        {
            return Err(PersistenceError::DestinationWritePending(destination));
        }

        let token = self.allocate_artifact_write_token()?;
        let prepared = PreparedArtifactWrite {
            token,
            document: self.id,
            revision: self.revision(),
            history,
            purpose,
            storage: AtomicArtifactWrite::new(destination, bytes.into(), overwrite),
        };
        self.pending_artifact_writes.insert(
            token,
            PendingArtifactWrite {
                prepared: prepared.clone(),
                kind,
            },
        );
        Ok(prepared)
    }

    /// Publish the model-side result of an already completed external write.
    /// A successful save marks the exact captured history node, even if the
    /// user has since moved to or committed a newer node.
    pub fn complete_artifact_write(
        &mut self,
        completion: ArtifactWriteCompletion,
    ) -> Result<ArtifactWriteCompletionStatus, PersistenceError> {
        if completion.document() != self.id {
            return Err(PersistenceError::WrongDocument {
                expected: self.id,
                actual: completion.document(),
            });
        }
        let token = completion.token();
        let pending = match self.pending_artifact_writes.get(&token).cloned() {
            Some(pending) => pending,
            None if token.as_u64() != 0 && token.as_u64() < self.next_artifact_write_token => {
                return Err(PersistenceError::DuplicateCompletion(token));
            }
            None => return Err(PersistenceError::UnknownCompletion(token)),
        };

        match completion.into_result() {
            ArtifactWriteResult::Failed => {
                self.finish_artifact_write(token);
                Ok(ArtifactWriteCompletionStatus::Failed {
                    token,
                    written_revision: pending.prepared.revision,
                    document_is_dirty: self.is_dirty(),
                })
            }
            ArtifactWriteResult::Succeeded(receipt) => {
                if receipt.destination() != pending.prepared.storage.destination() {
                    return Err(PersistenceError::ReceiptDestinationMismatch);
                }

                let save_sequence = match &pending.kind {
                    PendingArtifactWriteKind::Save {
                        captured_binding,
                        sequence,
                    } => {
                        if *sequence <= self.last_successful_save_sequence
                            || self.artifact_binding.as_ref() != Some(captured_binding)
                        {
                            self.finish_artifact_write(token);
                            return Err(PersistenceError::StaleCompletion(token));
                        }
                        if receipt.identity() != captured_binding.identity() {
                            return Err(PersistenceError::ReceiptIdentityMismatch);
                        }
                        Some(*sequence)
                    }
                    PendingArtifactWriteKind::SaveAs { sequence } => {
                        if *sequence <= self.last_successful_save_sequence {
                            self.finish_artifact_write(token);
                            return Err(PersistenceError::StaleCompletion(token));
                        }
                        Some(*sequence)
                    }
                    PendingArtifactWriteKind::Alternate => None,
                };

                let save_point = if save_sequence.is_some() {
                    Some(self.history.mark_location_saved(
                        pending.prepared.history,
                        SourceArtifactDigest::from_bytes(
                            pending.prepared.storage_request().bytes(),
                        ),
                    ))
                } else {
                    None
                };
                let previous_binding = self.artifact_binding.clone();
                if matches!(pending.kind, PendingArtifactWriteKind::SaveAs { .. }) {
                    self.artifact_binding = Some(receipt.into_binding());
                }
                if let Some(sequence) = save_sequence {
                    self.last_successful_save_sequence = sequence;
                }
                let file_identity_changed = self.artifact_binding != previous_binding;
                self.finish_artifact_write(token);

                Ok(ArtifactWriteCompletionStatus::Succeeded {
                    token,
                    written_revision: pending.prepared.revision,
                    save_point,
                    file_identity_changed,
                    document_is_dirty: self.is_dirty(),
                })
            }
        }
    }

    fn allocate_artifact_write_token(&mut self) -> Result<ArtifactWriteToken, PersistenceError> {
        let token = ArtifactWriteToken::from_u64(self.next_artifact_write_token);
        self.next_artifact_write_token = self
            .next_artifact_write_token
            .checked_add(1)
            .ok_or(PersistenceError::WriteIdentityExhausted)?;
        Ok(token)
    }

    fn allocate_save_sequence(&mut self) -> Result<u64, PersistenceError> {
        let sequence = self.next_save_sequence;
        self.next_save_sequence = self
            .next_save_sequence
            .checked_add(1)
            .ok_or(PersistenceError::WriteIdentityExhausted)?;
        Ok(sequence)
    }

    fn finish_artifact_write(&mut self, token: ArtifactWriteToken) {
        let removed = self.pending_artifact_writes.remove(&token);
        debug_assert!(removed.is_some());
    }

    /// Construct an exact source point in the current primary source part.
    pub fn source_point(&self, offset: usize) -> Result<SourcePoint, PositionError> {
        SourcePoint::new(
            self.id,
            self.revision(),
            SourcePartId::PRIMARY,
            offset,
            self.state().source.len(),
        )
    }

    /// Map an explicitly revision-bound source point through decoding,
    /// line-ending normalization, and the lossless format projection.
    pub fn map_source_point(
        &self,
        source: SourcePoint,
        affinity: BoundaryAffinity,
    ) -> Result<SourceToTextMapping, SourceToTextError> {
        if source.document() != self.id {
            return Err(SourceToTextError::WrongDocument {
                expected: self.id,
                actual: source.document(),
            });
        }
        if source.part() != SourcePartId::PRIMARY {
            return Err(SourceToTextError::WrongSourcePart {
                expected: SourcePartId::PRIMARY,
                actual: source.part(),
            });
        }
        if source.revision() != self.revision() {
            return Err(SourceToTextError::WrongSnapshot {
                expected: self.revision(),
                actual: source.revision(),
            });
        }
        let mapped = self.state().projection.map_source_boundary(
            source.revision(),
            source.offset(),
            affinity,
        )?;
        Ok(SourceToTextMapping {
            source,
            text: TextPoint {
                document: self.id,
                revision: mapped.revision,
                offset: mapped.formatted_offset,
            },
            affinity,
            relation: mapped.relation,
        })
    }

    pub fn text_point(&self, offset: usize) -> Result<TextPoint, DocumentError> {
        self.validate_boundary(offset)?;
        Ok(TextPoint {
            document: self.id,
            revision: self.revision(),
            offset,
        })
    }

    /// Capture a persistent formatted anchor from an exact current-snapshot
    /// point. Unlike [`TextAnchor::new`], this records the adjacent persistent
    /// rope identities, stable logical-block context, and source provenance
    /// available from the projection.
    pub fn text_anchor(
        &self,
        point: TextPoint,
        association: Association,
        affinity: BoundaryAffinity,
        deletion_recovery: DeletionRecovery,
    ) -> Result<TextAnchor, PositionError> {
        self.projection().capture_text_anchor(
            self.id,
            point,
            association,
            affinity,
            deletion_recovery,
        )
    }

    /// Resolve an identity-backed anchor in the current projection. A stale
    /// compatibility ordinal is never treated as current; callers which used
    /// [`TextAnchor::new`] must explicitly supply a [`PositionMap`] instead.
    pub fn resolve_text_anchor(
        &self,
        anchor: TextAnchor,
    ) -> Result<MappingOutcome<TextPoint>, PositionError> {
        self.projection().resolve_text_anchor(self.id, anchor)
    }

    pub fn replace_points(
        &mut self,
        start: TextPoint,
        end: TextPoint,
        replacement: &str,
    ) -> Result<(), DocumentError> {
        for point in [start, end] {
            if point.document != self.id {
                return Err(DocumentError::WrongDocument);
            }
            if point.revision != self.revision() {
                return Err(DocumentError::WrongSnapshot {
                    expected: self.revision(),
                    actual: point.revision,
                });
            }
        }
        self.replace(start.offset..end.offset, replacement)
    }

    pub fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), DocumentError> {
        self.apply_edits(vec![TextEdit::new(range, replacement)])
    }

    pub fn insert(&mut self, at: usize, text: &str) -> Result<(), DocumentError> {
        self.replace(at..at, text)
    }

    pub fn delete(&mut self, range: Range<usize>) -> Result<(), DocumentError> {
        self.replace(range, "")
    }

    /// Apply or remove a Markdown semantic inline style without changing the
    /// formatted text. Plain text correctly reports the capability as
    /// unsupported rather than creating hidden rich-text state.
    pub fn set_semantic_style(
        &mut self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<(), DocumentError> {
        self.execute_compat_request(ModelRequest::SetSemanticStyle {
            document: self.id,
            revision: self.revision(),
            range,
            style,
            enabled,
        })
    }

    /// Apply non-overlapping edits expressed in the current formatted
    /// snapshot. All source patches and verification commit as one undo unit.
    pub fn apply_edits(&mut self, edits: Vec<TextEdit>) -> Result<(), DocumentError> {
        self.execute_compat_request(ModelRequest::ApplyTextEdits {
            document: self.id,
            revision: self.revision(),
            edits,
        })
    }

    /// Applies structured formatted payloads atomically in the current
    /// revision. Unlike [`TextEdit`], only explicitly marked U+000A items are
    /// serialized as logical line breaks.
    pub fn apply_formatted_payload_edits(
        &mut self,
        edits: Vec<FormattedPayloadEdit>,
    ) -> Result<(), DocumentError> {
        let request = FormattedPayloadEditRequest::new(self.id, self.revision(), edits);
        self.execute_compat_formatted_payload_request(request)
    }

    pub fn replace_with_formatted_payload(
        &mut self,
        range: Range<usize>,
        payload: FormattedTextPayload,
    ) -> Result<(), DocumentError> {
        self.apply_formatted_payload_edits(vec![FormattedPayloadEdit::new(range, payload)])
    }

    pub fn insert_formatted_payload(
        &mut self,
        at: usize,
        payload: FormattedTextPayload,
    ) -> Result<(), DocumentError> {
        self.replace_with_formatted_payload(at..at, payload)
    }

    /// Copy or move a non-empty range of authoritative hard lines to an
    /// insertion gap in the current snapshot. Source syntax is transferred
    /// directly; Markdown is never flattened through its formatted text.
    pub fn transfer_hard_lines(
        &mut self,
        operation: HardLineTransfer,
        source_lines: Range<usize>,
        destination: usize,
    ) -> Result<(), DocumentError> {
        self.execute_compat_request(ModelRequest::TransferHardLines {
            document: self.id,
            revision: self.revision(),
            operation,
            source_lines,
            destination,
        })
    }

    /// Restore an exact source-backed hard-line image as one atomic history
    /// change. `target_line` is resolved in the current revision and must
    /// still carry the image's stable hard-line identity.
    pub fn restore_hard_line_source_image(
        &mut self,
        target_line: usize,
        image: HardLineSourceImage,
    ) -> Result<(), DocumentError> {
        self.execute_compat_request(ModelRequest::RestoreHardLineSource {
            document: self.id,
            revision: self.revision(),
            target_line,
            image,
        })
    }

    /// Begin or nest an edit group. Commits made before the matching
    /// `end_edit_group` share one undo node.
    pub fn begin_edit_group(&mut self) {
        if self.edit_group_depth == 0 {
            self.history.begin_group();
            self.edit_group_generation = self
                .edit_group_generation
                .checked_add(1)
                .expect("undo-group generations exhausted");
        }
        self.edit_group_depth += 1;
    }

    pub(crate) fn edit_group_generation(&self) -> u64 {
        self.edit_group_generation
    }

    pub fn end_edit_group(&mut self) {
        if self.edit_group_depth == 0 {
            return;
        }
        self.edit_group_depth -= 1;
        if self.edit_group_depth == 0 {
            self.history.end_group();
        }
    }

    /// Current nesting depth of the command-layer undo group. This is exposed
    /// only inside the core so a fallible controller event can restore the
    /// transaction boundary it observed before attempting an edit.
    pub(crate) fn edit_group_depth(&self) -> usize {
        self.edit_group_depth
    }

    /// Restore the undo-group nesting boundary after a failed command event.
    /// Document mutations themselves are prepared before commit; therefore a
    /// failure has no history node to discard, only possibly leaked grouping
    /// ownership to unwind.
    pub(crate) fn restore_edit_group_depth(&mut self, depth: usize) {
        while self.edit_group_depth > depth {
            self.end_edit_group();
        }
        while self.edit_group_depth < depth {
            self.begin_edit_group();
        }
    }

    /// End any open edit unit before another view becomes the writer.
    pub(crate) fn close_edit_group(&mut self) {
        self.edit_group_depth = 0;
        self.history.end_group();
    }

    /// Step to the parent undo state, returning the exact identities traversed.
    pub fn try_undo(&mut self) -> Result<HistoryNavigation, HistoryError> {
        self.try_undo_count(1)
    }

    /// Atomically navigate `count` parent edges. The complete path is resolved
    /// before the open edit unit or preferred redo path is changed, so an
    /// over-count is a non-destructive boundary error.
    pub fn try_undo_count(&mut self, count: usize) -> Result<HistoryNavigation, HistoryError> {
        if count == 0 {
            let current = self.history.status().current;
            return Ok(HistoryNavigation {
                from: current,
                to: current,
            });
        }
        // Resolve the protected open-unit ancestry before finalizing the
        // group, so an aggressively small budget cannot erase the target.
        let target = self.history.undo_target(count)?;
        let map = self
            .position_map_to_history_node(target.node)
            .expect("retained history edges compose exactly");
        let next_capture = self
            .composed_position_capture(&map)
            .expect("captured history transition follows the current revision");
        self.edit_group_depth = 0;
        let navigation = self.history.select_node(target.node)?;
        self.position_map_capture = next_capture;
        Ok(navigation)
    }

    /// Step to the currently preferred redo state.
    pub fn try_redo(&mut self) -> Result<HistoryNavigation, HistoryError> {
        self.try_redo_count(1)
    }

    /// Atomically navigate `count` preferred-child edges. If any edge is
    /// unavailable, the document, history cursor, and branch preferences are
    /// left untouched.
    pub fn try_redo_count(&mut self, count: usize) -> Result<HistoryNavigation, HistoryError> {
        if count == 0 {
            let current = self.history.status().current;
            return Ok(HistoryNavigation {
                from: current,
                to: current,
            });
        }
        let target = self.history.redo_target(count)?;
        let map = self
            .position_map_to_history_node(target.node)
            .expect("retained history edges compose exactly");
        let next_capture = self
            .composed_position_capture(&map)
            .expect("captured history transition follows the current revision");
        self.edit_group_depth = 0;
        let navigation = self.history.select_node(target.node)?;
        self.position_map_capture = next_capture;
        Ok(navigation)
    }

    /// Compatibility helper for callers that only need a boundary boolean.
    pub fn undo(&mut self) -> bool {
        self.try_undo().is_ok()
    }

    /// Compatibility helper for callers that only need a boundary boolean.
    pub fn redo(&mut self) -> bool {
        self.try_redo().is_ok()
    }

    /// Read-only state of the branching undo tree.
    pub fn history_status(&self) -> HistoryStatus {
        self.history.status()
    }

    /// Immutable diagnostics/audit metadata for one retained history node.
    /// The root has no initiating transaction or restoration state.
    pub fn history_node_details(
        &self,
        node: HistoryNodeId,
    ) -> Result<HistoryNodeDetails, HistoryError> {
        self.history
            .node_details(node)
            .ok_or(HistoryError::NodeNotFound(node))
    }

    /// Resolve the controller state associated with the final edge of an
    /// already validated history traversal.
    pub(crate) fn history_restoration_between(
        &self,
        from: HistoryNodeId,
        to: HistoryNodeId,
    ) -> Result<Option<HistoryRestorationSnapshot>, HistoryError> {
        self.history.restoration_between(from, to)
    }

    /// Replace the deterministic model-only fallback with authentic command
    /// state during the same serial coordinator turn. Repeated calls while an
    /// Insert/Replace unit is open preserve the first before-state and advance
    /// only the final after-state.
    pub(crate) fn attach_history_restoration(
        &mut self,
        node: HistoryNodeId,
        restoration: HistoryRestoration,
    ) -> Result<(), HistoryError> {
        debug_assert_eq!(restoration.before().cursor().document(), self.id);
        debug_assert_eq!(restoration.after().cursor().document(), self.id);
        debug_assert!(restoration
            .before()
            .marks()
            .values()
            .all(|anchor| anchor.document() == self.id));
        debug_assert!(restoration
            .after()
            .marks()
            .values()
            .all(|anchor| anchor.document() == self.id));
        self.history
            .attach_current_command_restoration(node, restoration)
    }

    /// Current in-memory undo-tree resource targets.
    pub fn history_retention_policy(&self) -> HistoryRetentionPolicy {
        self.history.retention_policy()
    }

    /// Apply new undo-tree resource targets immediately. The active state and
    /// an open undo unit's fixed parent/result remain protected even when they
    /// alone exceed the requested budget.
    pub fn set_history_retention_policy(&mut self, policy: HistoryRetentionPolicy) {
        self.history.set_retention_policy(policy);
    }

    /// Enumerate immediate redo branches without changing the current state.
    pub fn redo_branches(&self) -> Vec<HistoryBranch> {
        self.history.redo_branches()
    }

    pub fn redo_branch_count(&self) -> usize {
        self.history.redo_branch_count()
    }

    /// Set the preferred immediate redo branch without navigating to it.
    pub fn prefer_redo_branch(&mut self, branch: usize) -> Result<HistoryBranch, HistoryError> {
        self.history.prefer_redo_branch(branch)
    }

    /// Compatibility helper for callers that only need a validity boolean.
    pub fn select_redo_branch(&mut self, branch: usize) -> bool {
        self.history.select_redo_branch(branch)
    }

    /// Navigate directly to a retained undo-tree node.
    pub fn select_history_node(
        &mut self,
        node: HistoryNodeId,
    ) -> Result<HistoryNavigation, HistoryError> {
        if self.history.location_for_node(node).is_none() {
            return Err(HistoryError::NodeNotFound(node));
        }
        let map = self
            .position_map_to_history_node(node)
            .expect("retained history edges compose exactly");
        let next_capture = self
            .composed_position_capture(&map)
            .expect("captured history transition follows the current revision");
        let navigation = self.history.select_node(node)?;
        self.edit_group_depth = 0;
        self.position_map_capture = next_capture;
        Ok(navigation)
    }

    /// Navigate directly to a retained monotonic change number.
    pub fn select_history_change(
        &mut self,
        change: HistoryChangeNumber,
    ) -> Result<HistoryNavigation, HistoryError> {
        let target = self
            .history
            .location_for_change(change)
            .ok_or(HistoryError::ChangeNotFound(change))?;
        let map = self
            .position_map_to_history_node(target.node)
            .expect("retained history edges compose exactly");
        let next_capture = self
            .composed_position_capture(&map)
            .expect("captured history transition follows the current revision");
        let navigation = self.history.select_change(change)?;
        self.edit_group_depth = 0;
        self.position_map_capture = next_capture;
        Ok(navigation)
    }

    /// Mark the current immutable state as the last successfully persisted
    /// state. This performs no I/O.
    pub fn mark_saved(&mut self) -> HistoryNodeId {
        self.close_edit_group();
        self.history.mark_saved()
    }

    /// Whether the current history state differs from the save-point identity.
    pub fn is_dirty(&self) -> bool {
        self.history.status().is_dirty
    }

    /// Convert only logical line-ending tokens. Literal CR/LF content is
    /// retained and the formatted projection must remain identical.
    pub fn set_file_format(&mut self, target: FileFormat) -> Result<(), DocumentError> {
        self.execute_compat_request(ModelRequest::SetFileFormat {
            document: self.id,
            revision: self.revision(),
            target,
        })
    }

    /// Explicitly add/remove a BOM as one source transaction. `Preserve` is a
    /// no-op and Latin-1 rejects `Always` because it has no BOM.
    pub fn set_bom_policy(&mut self, policy: BomPolicy) -> Result<(), DocumentError> {
        let should_have = match policy {
            BomPolicy::Preserve => return Ok(()),
            BomPolicy::Always => true,
            BomPolicy::Never => false,
        };
        if should_have == self.state().has_bom {
            return Ok(());
        }
        let bom = self.state().encoding.bom_bytes();
        if should_have && bom.is_empty() {
            return Err(DocumentError::UnsupportedBom(self.state().encoding));
        }
        let mut source = self.state().source.clone();
        if should_have {
            source = source.replace(0, 0, bom.to_vec()).unwrap();
        } else {
            let length = bom.len();
            source = source.replace(0, length, Vec::new()).unwrap();
        }
        self.commit_candidate(source, self.state().file_format, self.text().to_owned())
    }

    pub fn line_count(&self) -> usize {
        self.projection().hard_line_count()
    }

    /// Captures an immutable, revision-bound view of the authoritative
    /// hard-line index. The capture is cheap and remains usable as an exact
    /// historical read snapshot after this document commits another revision.
    pub fn hard_line_snapshot(&self) -> HardLineSnapshot {
        self.projection().hard_line_snapshot(self.id)
    }

    /// Capture one current hard line as exact source bytes plus a semantic
    /// projection witness. The returned image remains immutable after later
    /// revisions and can be retained by the command layer for Vim `U`.
    pub fn capture_hard_line_source_image(
        &self,
        line: usize,
    ) -> Result<HardLineSourceImage, DocumentError> {
        hard_line_source_image_from_state(self.id, self.state(), line)
    }

    /// Map a non-empty, half-open span of formatted hard-line ordinals to its
    /// exact contiguous byte extent in the authoritative primary source part.
    ///
    /// The returned extent starts after an encoding BOM (which is artifact
    /// metadata, not line content) and includes each selected line's original
    /// source delimiter when it has one. Consequently CRLF/CR/LF spelling,
    /// literal CR/LF content, Markdown markers, malformed opaque bytes, and an
    /// unterminated final line are copied exactly as stored. The current plain
    /// text and Markdown adapters each map one physical source line to one
    /// formatted hard line. A future adapter that does not have that shape is
    /// rejected explicitly until it supplies a relational implementation.
    pub fn source_byte_range_for_hard_lines(
        &self,
        lines: Range<usize>,
    ) -> Result<Range<usize>, HardLineSourceRangeError> {
        let formatted_line_count = self.line_count();
        if lines.start >= lines.end || lines.end > formatted_line_count {
            return Err(HardLineSourceRangeError::InvalidRange {
                start: lines.start,
                end: lines.end,
                line_count: formatted_line_count,
            });
        }

        let source_lines = &self.state().source_hard_lines;
        let source_line_count = source_lines.len();
        if source_line_count != formatted_line_count {
            return Err(HardLineSourceRangeError::ProjectionMismatch {
                source_line_count,
                formatted_line_count,
            });
        }
        Ok(source_lines
            .get(lines.start)
            .expect("validated source hard-line ordinal")
            .start
            ..source_lines
                .get(lines.end - 1)
                .expect("validated source hard-line ordinal")
                .end)
    }

    /// Captures formatted text plus explicit semantic hard-break metadata from
    /// the current immutable revision.
    pub fn capture_formatted_payload(
        &self,
        range: Range<usize>,
    ) -> Result<FormattedTextPayload, FormattedPayloadError> {
        self.hard_line_snapshot().capture(range)
    }

    /// Ensures a retained hard-line snapshot belongs to this document's
    /// current revision before its ordinal results are used for mutation.
    pub fn validate_hard_line_snapshot(
        &self,
        snapshot: &HardLineSnapshot,
    ) -> Result<(), DocumentError> {
        if snapshot.document() != self.id {
            return Err(DocumentError::WrongDocument);
        }
        if snapshot.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: snapshot.revision(),
            });
        }
        Ok(())
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.projection()
            .hard_line_range(line)
            .map(|range| range.start)
    }

    pub fn line_end(&self, line: usize) -> Option<usize> {
        self.projection()
            .hard_line_range(line)
            .map(|range| range.end)
    }

    /// Resolve a formatted UTF-8 boundary to its authoritative hard line.
    /// Explicit delimiter boundaries and EOF follow
    /// [`FormattedDocument::hard_line_at_offset`].
    pub fn hard_line_at_offset(&self, offset: usize) -> Option<usize> {
        self.projection().hard_line_at_offset(offset)
    }

    pub fn next_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        self.validate_boundary(offset).ok()?;
        self.projection()
            .next_logical_grapheme_boundary(offset)
            .ok()?
    }

    pub fn previous_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        self.validate_boundary(offset).ok()?;
        self.projection()
            .previous_logical_grapheme_boundary(offset)
            .ok()?
    }

    fn validate_range(&self, range: &Range<usize>) -> Result<(), DocumentError> {
        let length = self.projection().text_tree().byte_len();
        if range.start > range.end || range.end > length {
            return Err(DocumentError::InvalidRange {
                start: range.start,
                end: range.end,
                length,
            });
        }
        self.validate_boundary(range.start)?;
        self.validate_boundary(range.end)
    }

    fn validate_boundary(&self, offset: usize) -> Result<(), DocumentError> {
        let projection = self.projection();
        if offset > projection.text_tree().byte_len() {
            return Err(DocumentError::InvalidRange {
                start: offset,
                end: offset,
                length: projection.text_tree().byte_len(),
            });
        }
        if projection
            .is_logical_grapheme_boundary(offset)
            .map_err(|_| DocumentError::NotGraphemeBoundary(offset))?
        {
            Ok(())
        } else {
            Err(DocumentError::NotGraphemeBoundary(offset))
        }
    }

    fn reject_unsafe_opaque_mapping(
        &self,
        formatted_range: &Range<usize>,
        source_range: &Range<usize>,
    ) -> Result<(), DocumentError> {
        for diagnostic in self
            .projection()
            .decoding_diagnostics_for_region(formatted_range)
        {
            let opaque = &diagnostic.source_range;
            let visible = &diagnostic.formatted_range;
            let source_overlaps = ranges_overlap(source_range, opaque);
            let formatted_overlaps = ranges_overlap(formatted_range, visible);
            let source_contains =
                source_range.start <= opaque.start && opaque.end <= source_range.end;
            let formatted_contains =
                formatted_range.start <= visible.start && visible.end <= formatted_range.end;

            // Reverse projection may replace an opaque source extent only
            // when the semantic edit selected the entire visible diagnostic
            // item and the resulting source patch contains all of its bytes.
            if (source_overlaps || formatted_overlaps) && !(source_contains && formatted_contains) {
                return Err(DocumentError::OpaqueDecodingConflict {
                    source_range: opaque.clone(),
                });
            }
        }
        Ok(())
    }

    fn commit_candidate(
        &mut self,
        source: SourceSnapshot,
        file_format: FileFormat,
        expected_text: String,
    ) -> Result<(), DocumentError> {
        self.execute_compat_source_metadata_candidate(source, file_format, expected_text)
    }

    fn position_map_to_history_node(
        &self,
        target: HistoryNodeId,
    ) -> Result<PositionMap, PositionError> {
        let current = self.history.status().current.node;
        self.history.map_between(
            current,
            target,
            PositionMap::identity(
                self.id,
                PositionDomain::FormattedText,
                self.revision(),
                self.text().len(),
            ),
        )
    }

    fn composed_position_capture(
        &self,
        transition: &PositionMap,
    ) -> Result<Option<PositionMap>, PositionError> {
        self.position_map_capture
            .as_ref()
            .map(|captured| captured.then(transition))
            .transpose()
    }
}

fn build_state_from_decoded(
    source: SourceSnapshot,
    decoded: DecodedText,
    format: Format,
    file_format: FileFormat,
    file_format_origin: FileFormatOrigin,
    line_ending_evidence: LineEndingEvidence,
    revision: Revision,
) -> Result<DocumentState, DocumentError> {
    let normalized = normalize(&decoded, file_format);
    let source_content_start = decoded.bom_len;
    let source_content_end = decoded
        .source_boundary(decoded.text.len())
        .unwrap_or(source_content_start);
    let mut source_hard_lines = Vec::with_capacity(normalized.endings.len() + 1);
    let mut source_line_start = source_content_start;
    for ending in &normalized.endings {
        source_hard_lines.push(source_line_start..ending.source.end);
        source_line_start = ending.source.end;
    }
    source_hard_lines.push(source_line_start..source_content_end);
    let projection = project(
        &normalized,
        format,
        revision,
        source_content_start,
        source_content_end,
    );
    Ok(DocumentState {
        revision,
        source,
        projection,
        source_hard_lines: SourceHardLineIndex::new(source_hard_lines)
            .expect("decoded hard-line ranges form a contiguous source partition"),
        encoding: decoded.encoding,
        format,
        file_format,
        file_format_origin,
        line_ending_evidence,
        has_bom: decoded.bom_len != 0,
    })
}

fn hard_line_source_image_from_state(
    document: DocumentId,
    state: &DocumentState,
    line: usize,
) -> Result<HardLineSourceImage, DocumentError> {
    let line_count = state.projection.hard_line_count();
    if line >= line_count {
        return Err(DocumentError::InvalidHardLineSourceImageTarget { line, line_count });
    }
    if state.source_hard_lines.len() != line_count || state.projection.blocks().len() != line_count
    {
        return Err(DocumentError::HardLineSourceImageProjectionMismatch);
    }

    let formatted_range = state
        .projection
        .hard_line_range(line)
        .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
    let hard_line_id = state
        .projection
        .hard_line_id(line)
        .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
    let block = state
        .projection
        .blocks()
        .get(line)
        .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
    if block.id != hard_line_id || block.range != formatted_range {
        return Err(DocumentError::HardLineSourceImageProjectionMismatch);
    }
    let source_range = state
        .source_hard_lines
        .get(line)
        .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
    let source_bytes = state
        .source
        .bytes_in(source_range.clone())
        .ok_or(DocumentError::HardLineSourceImageProjectionMismatch)?;
    let formatted_text = state
        .projection
        .text_tree()
        .slice(formatted_range.clone())
        .map_err(DocumentError::FormattedTextStorage)?;

    let mut styles = Vec::new();
    for span in state.projection.style_spans_for_region(&formatted_range) {
        if span.range.start < formatted_range.start || span.range.end > formatted_range.end {
            return Err(DocumentError::HardLineSourceImageProjectionMismatch);
        }
        if let StyleApplication::Semantic(style) = span.application {
            styles.push(HardLineStyleImage {
                range: span.range.start - formatted_range.start
                    ..span.range.end - formatted_range.start,
                style,
            });
        }
    }

    let mut diagnostics = Vec::new();
    for diagnostic in state
        .projection
        .decoding_diagnostics_for_region(&formatted_range)
    {
        if diagnostic.formatted_range.start < formatted_range.start
            || diagnostic.formatted_range.end > formatted_range.end
            || diagnostic.source_range.start < source_range.start
            || diagnostic.source_range.end > source_range.end
        {
            return Err(DocumentError::HardLineSourceImageProjectionMismatch);
        }
        diagnostics.push(HardLineDiagnosticImage {
            kind: diagnostic.kind,
            formatted_range: diagnostic.formatted_range.start - formatted_range.start
                ..diagnostic.formatted_range.end - formatted_range.start,
            source_range: diagnostic.source_range.start - source_range.start
                ..diagnostic.source_range.end - source_range.start,
        });
    }

    Ok(HardLineSourceImage {
        document,
        revision: state.revision,
        captured_line: line,
        hard_line_count: line_count,
        hard_line_id,
        encoding: state.encoding,
        format: state.format,
        file_format: state.file_format,
        terminated: line + 1 < line_count,
        source_bytes,
        formatted_text,
        block_kind: block.kind.clone(),
        block_style: block.style.clone(),
        styles,
        diagnostics,
    })
}

fn block_identity_document_error(error: BlockIdentityError) -> DocumentError {
    match error {
        BlockIdentityError::Exhausted => DocumentError::BlockIdentityExhausted,
        BlockIdentityError::InvalidProjection => DocumentError::VerificationFailed,
    }
}

fn spell_logical_breaks(text: &str, file_format: FileFormat) -> String {
    if file_format == FileFormat::Unix || !text.contains('\n') {
        return text.to_owned();
    }
    text.replace('\n', file_format.spelling())
}

fn ranges_overlap(first: &Range<usize>, second: &Range<usize>) -> bool {
    first.start < second.end && second.start < first.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_open_save_is_byte_exact_for_every_encoding() {
        let cases = [
            (Encoding::Utf8, b"\xef\xbb\xbfhello\r\n".to_vec()),
            (Encoding::Latin1, vec![0x48, 0xe9, 0x0d, 0x0a]),
            (
                Encoding::Utf16Le,
                vec![0xff, 0xfe, 0x48, 0x00, 0xe9, 0x00, 0x0a, 0x00],
            ),
            (
                Encoding::Utf16Be,
                vec![0xfe, 0xff, 0x00, 0x48, 0x00, 0xe9, 0x00, 0x0a],
            ),
        ];
        for (encoding, bytes) in cases {
            let document =
                Document::from_bytes(bytes.clone(), encoding, Format::PlainText).unwrap();
            assert_eq!(document.source_bytes(), bytes);
        }
    }

    #[test]
    fn utf8_bom_survives_local_edit() {
        let mut document = Document::from_bytes(
            b"\xef\xbb\xbfhello".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
        )
        .unwrap();
        document.replace(1..4, "i").unwrap();
        assert_eq!(document.text(), "hio");
        assert_eq!(document.source_bytes(), b"\xef\xbb\xbfhio");
    }

    #[test]
    fn latin1_edit_is_exact_and_unrepresentable_edit_is_atomic() {
        let mut document = Document::from_bytes(
            vec![b'c', b'a', b'f', 0xe9],
            Encoding::Latin1,
            Format::PlainText,
        )
        .unwrap();
        document.replace(0..3, "thé").unwrap();
        assert_eq!(document.source_bytes(), vec![b't', b'h', 0xe9, 0xe9]);
        let before = document.source_bytes();
        let revision = document.revision();
        assert!(matches!(
            document.insert(0, "😀"),
            Err(DocumentError::UnrepresentableCharacter { .. })
        ));
        assert_eq!(document.source_bytes(), before);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn utf16_edits_preserve_endianness_and_bom() {
        let mut le = Document::from_bytes(
            vec![0xff, 0xfe, b'a', 0, b'b', 0],
            Encoding::Utf16Le,
            Format::PlainText,
        )
        .unwrap();
        le.replace(1..2, "😀").unwrap();
        assert_eq!(le.text(), "a😀");
        assert_eq!(
            le.source_bytes(),
            [0xff, 0xfe, b'a', 0, 0x3d, 0xd8, 0x00, 0xde]
        );

        let mut be = Document::from_bytes(
            vec![0xfe, 0xff, 0, b'a', 0, b'b'],
            Encoding::Utf16Be,
            Format::PlainText,
        )
        .unwrap();
        be.replace(1..2, "é").unwrap();
        assert_eq!(be.source_bytes(), [0xfe, 0xff, 0, b'a', 0, 0xe9]);
    }

    #[test]
    fn dos_line_endings_normalize_and_new_breaks_use_dos_spelling() {
        let original = b"one\r\ntwo\r\n".to_vec();
        let mut document =
            Document::from_bytes(original.clone(), Encoding::Utf8, Format::PlainText).unwrap();
        assert_eq!(document.file_format(), FileFormat::Dos);
        assert_eq!(document.text(), "one\ntwo\n");
        assert_eq!(document.source_bytes(), original);
        document.insert(3, "\nnew").unwrap();
        assert_eq!(document.text(), "one\nnew\ntwo\n");
        assert_eq!(document.source_bytes(), b"one\r\nnew\r\ntwo\r\n");
    }

    #[test]
    fn forced_mac_keeps_literal_lf_inside_plain_and_markdown_hard_lines_for_all_encodings() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for format in [Format::PlainText, Format::Markdown] {
                let source_text = match format {
                    Format::PlainText => "a\rb\nc",
                    Format::Markdown => "# a\r# b\nc",
                };
                let mut bytes = encoding.encode_fragment(source_text).unwrap();
                if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
                    let mut with_bom = encoding.bom_bytes().to_vec();
                    with_bom.append(&mut bytes);
                    bytes = with_bom;
                }
                let original = bytes.clone();
                let mut document =
                    Document::from_bytes_with_file_format(bytes, encoding, format, FileFormat::Mac)
                        .unwrap();

                assert_eq!(document.text(), "a\nb\nc");
                assert_eq!(document.line_count(), 2);
                assert_eq!(document.line_start(0), Some(0));
                assert_eq!(document.line_end(0), Some(1));
                assert_eq!(document.line_start(1), Some(2));
                assert_eq!(document.line_end(1), Some(5));
                assert_eq!(document.source_bytes(), original);
                assert_eq!(
                    document.projection().hard_lines_for_region(&(3..4)),
                    vec![2..5]
                );
                assert_eq!(document.projection().blocks().len(), 2);
                assert_eq!(document.projection().blocks()[1].range, 2..5);
                if format == Format::Markdown {
                    assert!(document
                        .projection()
                        .blocks()
                        .iter()
                        .all(|block| block.kind == BlockKind::Heading(1)));
                }

                let line_ids = (0..document.line_count())
                    .map(|line| document.projection().hard_line_id(line).unwrap())
                    .collect::<Vec<_>>();
                document.replace(2..3, "B").unwrap();
                assert_eq!(document.text(), "a\nB\nc");
                assert_eq!(document.line_count(), 2);
                assert_eq!(
                    (0..document.line_count())
                        .map(|line| document.projection().hard_line_id(line).unwrap())
                        .collect::<Vec<_>>(),
                    line_ids
                );
                assert!(document.undo());
                assert_eq!(document.text(), "a\nb\nc");
                assert_eq!(
                    (0..document.line_count())
                        .map(|line| document.projection().hard_line_id(line).unwrap())
                        .collect::<Vec<_>>(),
                    line_ids
                );
            }
        }
    }

    #[test]
    fn file_format_verification_compares_authoritative_hard_line_structure() {
        let mut ambiguous = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let before_revision = ambiguous.revision();
        let before_source = ambiguous.source_bytes();
        let before_ids = (0..ambiguous.line_count())
            .map(|line| ambiguous.projection().hard_line_id(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ambiguous.text(), "a\nb\nc");
        assert!(matches!(
            ambiguous.set_file_format(FileFormat::Unix),
            Err(DocumentError::LineEndingConversionWouldReinterpretContent)
        ));
        assert_eq!(ambiguous.revision(), before_revision);
        assert_eq!(ambiguous.source_bytes(), before_source);
        assert_eq!(ambiguous.file_format(), FileFormat::Mac);
        assert_eq!(
            (0..ambiguous.line_count())
                .map(|line| ambiguous.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>(),
            before_ids
        );

        let mut safe = Document::from_bytes_with_file_format(
            b"a\rb\r".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let safe_ids = (0..safe.line_count())
            .map(|line| safe.projection().hard_line_id(line).unwrap())
            .collect::<Vec<_>>();
        safe.set_file_format(FileFormat::Unix).unwrap();
        assert_eq!(safe.text(), "a\nb\n");
        assert_eq!(safe.line_count(), 3);
        assert_eq!(
            (0..safe.line_count())
                .map(|line| safe.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>(),
            safe_ids
        );
    }

    #[test]
    fn hard_line_at_offset_defines_delimiter_eof_and_literal_lf_boundaries() {
        let unix = Document::new("a\nb\n");
        assert_eq!(unix.line_count(), 3);
        assert_eq!(unix.hard_line_at_offset(0), Some(0));
        assert_eq!(unix.hard_line_at_offset(1), Some(0));
        assert_eq!(unix.hard_line_at_offset(2), Some(1));
        assert_eq!(unix.hard_line_at_offset(3), Some(1));
        assert_eq!(unix.hard_line_at_offset(4), Some(2));
        assert_eq!(unix.hard_line_at_offset(5), None);

        let mac = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(mac.hard_line_at_offset(1), Some(0));
        assert_eq!(mac.hard_line_at_offset(2), Some(1));
        assert_eq!(mac.hard_line_at_offset(3), Some(1));
        assert_eq!(mac.hard_line_at_offset(4), Some(1));
        assert_eq!(mac.hard_line_at_offset(5), Some(1));

        let unicode = Document::new("é");
        assert_eq!(unicode.hard_line_at_offset(1), None);
    }

    #[test]
    fn hard_line_read_snapshot_is_exact_revision_bound_and_retains_separators() {
        let mut document = Document::new("a\nb\n");
        let snapshot = document.hard_line_snapshot();
        assert_eq!(snapshot.document(), document.id());
        assert_eq!(snapshot.revision(), Revision(0));
        assert_eq!(snapshot.line_count(), 3);
        assert!(!snapshot.is_empty());

        let lines = snapshot.lines(0..3).unwrap();
        assert_eq!(lines[0].content_range(), 0..1);
        assert_eq!(lines[0].separator_range(), Some(1..2));
        assert_eq!(lines[0].linewise_range(), 0..2);
        assert_eq!(lines[1].content_range(), 2..3);
        assert_eq!(lines[1].separator_range(), Some(3..4));
        assert_eq!(lines[2].content_range(), 4..4);
        assert_eq!(lines[2].separator_range(), None);
        assert_eq!(snapshot.linewise_extent(0..2).unwrap(), 0..4);
        assert_eq!(snapshot.linewise_extent(1..3).unwrap(), 2..4);
        assert_eq!(snapshot.linewise_extent(1..1).unwrap(), 2..2);
        assert_eq!(snapshot.linewise_extent(3..3).unwrap(), 4..4);

        assert_eq!(snapshot.line_at_offset(1).unwrap().index(), 0);
        assert_eq!(snapshot.line_at_offset(2).unwrap().index(), 1);
        assert_eq!(snapshot.line_at_offset(4).unwrap().index(), 2);
        assert_eq!(
            snapshot.line_at_offset(5),
            Err(HardLineQueryError::FormattedOffsetOutOfBounds {
                offset: 5,
                text_length: 4,
            })
        );
        assert_eq!(
            snapshot.lines(2..4),
            Err(HardLineQueryError::InvalidLineRange {
                start: 2,
                end: 4,
                line_count: 3,
            })
        );

        document.insert(0, "z").unwrap();
        assert_eq!(snapshot.text(), "a\nb\n");
        assert_eq!(snapshot.line(0).unwrap().content_range(), 0..1);
        assert_eq!(
            document.validate_hard_line_snapshot(&snapshot),
            Err(DocumentError::WrongSnapshot {
                expected: Revision(1),
                actual: Revision(0),
            })
        );
        let current = document.hard_line_snapshot();
        assert_eq!(current.revision(), Revision(1));
        document.validate_hard_line_snapshot(&current).unwrap();

        let other = Document::new("a\nb\n");
        assert_eq!(
            other.validate_hard_line_snapshot(&snapshot),
            Err(DocumentError::WrongDocument)
        );
    }

    #[test]
    fn hard_line_snapshot_preserves_literal_lf_and_rejects_non_utf8_boundaries() {
        let mac = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let snapshot = mac.hard_line_snapshot();
        assert_eq!(snapshot.lines(0..2).unwrap()[1].content_range(), 2..5);
        assert_eq!(snapshot.line_at_offset(3).unwrap().index(), 1);

        let unicode = Document::new("é").hard_line_snapshot();
        assert_eq!(
            unicode.line_at_offset(1),
            Err(HardLineQueryError::NotCharacterBoundary { offset: 1 })
        );
    }

    #[test]
    fn structured_payload_capture_marks_only_semantic_breaks() {
        let document = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let snapshot = document.hard_line_snapshot();

        let all = snapshot.capture(0..5).unwrap();
        assert_eq!(all.document(), document.id());
        assert_eq!(all.revision(), Revision(0));
        assert_eq!(all.text(), "a\nb\nc");
        assert_eq!(all.break_offsets(), &[1]);
        let second = snapshot.capture(2..5).unwrap();
        assert_eq!(second.text(), "b\nc");
        assert!(second.break_offsets().is_empty());

        assert_eq!(
            FormattedTextPayload::new(&snapshot, "\n\n", vec![1, 0]),
            Err(FormattedPayloadError::BreakOffsetsNotStrictlyIncreasing {
                previous: 1,
                offset: 0,
            })
        );
        assert_eq!(
            FormattedTextPayload::new(&snapshot, "x", vec![0]),
            Err(FormattedPayloadError::BreakOffsetIsNotLineFeed { offset: 0 })
        );
        assert_eq!(
            FormattedTextPayload::new(&snapshot, "\n", vec![1]),
            Err(FormattedPayloadError::BreakOffsetOutOfBounds {
                offset: 1,
                text_length: 1,
            })
        );

        let grapheme = Document::new("e\u{301}").hard_line_snapshot();
        assert_eq!(
            grapheme.capture(1..3),
            Err(FormattedPayloadError::NotGraphemeBoundary { offset: 1 })
        );
    }

    #[test]
    fn structured_payload_round_trips_mac_literal_lf_for_formats_and_encodings() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for format in [Format::PlainText, Format::Markdown] {
                let source_text = match format {
                    Format::PlainText => "a\rb\nc",
                    Format::Markdown => "# a\r# b\nc",
                };
                let mut bytes = encoding.encode_fragment(source_text).unwrap();
                if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
                    let mut with_bom = encoding.bom_bytes().to_vec();
                    with_bom.append(&mut bytes);
                    bytes = with_bom;
                }
                let original = bytes.clone();
                let mut document =
                    Document::from_bytes_with_file_format(bytes, encoding, format, FileFormat::Mac)
                        .unwrap();
                let payload = document.capture_formatted_payload(0..5).unwrap();
                assert_eq!(payload.text(), "a\nb\nc");
                assert_eq!(payload.break_offsets(), &[1]);
                let at = document.text().len();
                let request = FormattedPayloadEditRequest::new(
                    document.id(),
                    document.revision(),
                    vec![FormattedPayloadEdit::new(at..at, payload)],
                );
                let prepared = document.prepare_formatted_payload_request(request).unwrap();
                assert_eq!(prepared.before_revision(), Revision(0));
                assert_eq!(prepared.after_revision(), Revision(1));
                assert_eq!(prepared.summary().kind(), ModelChangeKind::TextEdits);
                assert_eq!(prepared.summary().formatted_splices().len(), 1);
                assert_eq!(
                    prepared.summary().formatted_splices()[0].old_range(),
                    at..at
                );
                assert_eq!(prepared.summary().formatted_splices()[0].inserted_len(), 5);
                assert_eq!(prepared.summary().source_patches().len(), 1);
                assert_eq!(
                    prepared.summary().source_patches()[0].range(),
                    original.len()..original.len()
                );
                let committed = document.commit_model_transaction(prepared).unwrap();

                assert_eq!(document.text(), "a\nb\nca\nb\nc");
                assert_eq!(document.projection().hard_break_offsets(), vec![1, 6]);
                assert_eq!(committed.text_position_map().source_revision(), Revision(0));
                assert_eq!(committed.text_position_map().target_revision(), Revision(1));
                assert_eq!(
                    &document.source_bytes()[..original.len()],
                    original.as_slice()
                );
                let decoded = encoding.decode(&document.source_bytes()).unwrap();
                assert_eq!(decoded.text, format!("{source_text}a\rb\nc"));

                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }

    #[test]
    fn structured_payload_preserves_markdown_syntax_and_rejects_reinterpretation() {
        let mut markdown = Document::from_bytes_with_file_format(
            b"before".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let snapshot = markdown.hard_line_snapshot();
        let payload = FormattedTextPayload::new(&snapshot, "*literal*\n# title", vec![9]).unwrap();
        markdown
            .insert_formatted_payload(markdown.text().len(), payload)
            .unwrap();
        assert_eq!(markdown.text(), "before*literal*\n# title");
        assert_eq!(markdown.source_bytes(), b"before\\*literal\\*\n\\# title");

        let mut unix = Document::from_bytes_with_file_format(
            b"x".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        let literal_lf =
            FormattedTextPayload::new(&unix.hard_line_snapshot(), "\n", Vec::new()).unwrap();
        let bytes = unix.source_bytes();
        let revision = unix.revision();
        assert_eq!(
            unix.insert_formatted_payload(1, literal_lf),
            Err(DocumentError::FormattedPayloadCannotReproject)
        );
        assert_eq!(unix.source_bytes(), bytes);
        assert_eq!(unix.revision(), revision);
    }

    #[test]
    fn structured_payload_can_change_item_kind_when_flat_utf8_is_identical() {
        let mut document = Document::from_bytes_with_file_format(
            b"a\nb".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.text(), "a\nb");
        assert_eq!(document.line_count(), 1);

        let marked =
            FormattedTextPayload::new(&document.hard_line_snapshot(), "\n", vec![0]).unwrap();
        document
            .replace_with_formatted_payload(1..2, marked)
            .unwrap();
        assert_eq!(document.text(), "a\nb");
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.source_bytes(), b"a\rb");

        let literal =
            FormattedTextPayload::new(&document.hard_line_snapshot(), "\n", Vec::new()).unwrap();
        document
            .replace_with_formatted_payload(1..2, literal)
            .unwrap();
        assert_eq!(document.text(), "a\nb");
        assert_eq!(document.line_count(), 1);
        assert_eq!(document.source_bytes(), b"a\nb");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"a\rb");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"a\nb");
    }

    #[test]
    fn structured_payload_rejects_stale_and_wrong_document_origins() {
        let mut source = Document::new("source");
        let stale = source.capture_formatted_payload(0..6).unwrap();
        source.insert(6, "!").unwrap();
        assert_eq!(
            source.insert_formatted_payload(0, stale),
            Err(DocumentError::WrongSnapshot {
                expected: Revision(1),
                actual: Revision(0),
            })
        );

        let foreign = Document::new("foreign")
            .capture_formatted_payload(0..7)
            .unwrap();
        assert_eq!(
            source.insert_formatted_payload(0, foreign),
            Err(DocumentError::WrongDocument)
        );
    }

    #[test]
    fn final_terminator_is_not_invented() {
        let mut document = Document::new("one");
        document.insert(3, "!").unwrap();
        assert_eq!(document.source_bytes(), b"one!");
        assert_eq!(document.line_count(), 1);
    }

    #[test]
    fn markdown_projection_is_lossless_and_edits_visible_content_locally() {
        let source = b"# Hello **world**!\n".to_vec();
        let mut document =
            Document::from_bytes(source.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.text(), "Hello world!\n");
        assert_eq!(
            document.projection().blocks()[0].kind,
            BlockKind::Heading(1)
        );
        assert_eq!(document.projection().style_spans().len(), 1);

        document.replace(6..11, "earth").unwrap();
        assert_eq!(document.text(), "Hello earth!\n");
        assert_eq!(document.source_bytes(), b"# Hello **earth**!\n");
    }

    #[test]
    fn markdown_insertion_escapes_syntax_to_satisfy_semantic_intent() {
        let mut document =
            Document::from_bytes(b"plain".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        document.insert(5, " *literal*").unwrap();
        assert_eq!(document.text(), "plain *literal*");
        assert_eq!(document.source_bytes(), b"plain \\*literal\\*");
    }

    #[test]
    fn markdown_pipeline_handles_utf8_latin1_and_both_utf16_orders() {
        let logical = "# Hé **x**\n";
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let mut bytes = encoding.encode_fragment(logical).unwrap();
            if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
                let mut with_bom = encoding.bom_bytes().to_vec();
                with_bom.append(&mut bytes);
                bytes = with_bom;
            }
            let original = bytes.clone();
            let mut document = Document::from_bytes(bytes, encoding, Format::Markdown).unwrap();
            assert_eq!(document.text(), "Hé x\n");
            assert_eq!(document.source_bytes(), original);
            document.replace(4..5, "y").unwrap();
            assert_eq!(document.text(), "Hé y\n");
            assert_eq!(document.projection().style_spans().len(), 1);
        }
    }

    #[test]
    fn markdown_semantic_style_edits_are_source_backed_and_undoable() {
        let mut document =
            Document::from_bytes(b"make bold".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        document
            .set_semantic_style(5..9, SemanticInlineStyle::Strong, true)
            .unwrap();
        assert_eq!(document.text(), "make bold");
        assert_eq!(document.source_bytes(), b"make **bold**");
        assert_eq!(document.projection().style_spans()[0].range, 5..9);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"make bold");
        assert!(document.redo());
        document
            .set_semantic_style(5..9, SemanticInlineStyle::Strong, false)
            .unwrap();
        assert_eq!(document.source_bytes(), b"make bold");

        let mut plain = Document::new("plain");
        assert_eq!(
            plain.set_semantic_style(0..5, SemanticInlineStyle::Strong, true),
            Err(DocumentError::UnsupportedFormatting)
        );
    }

    #[test]
    fn multi_edit_is_atomic_and_uses_pre_edit_coordinates() {
        let mut document = Document::new("one two three");
        document
            .apply_edits(vec![TextEdit::new(0..3, "1"), TextEdit::new(8..13, "3")])
            .unwrap();
        assert_eq!(document.text(), "1 two 3");
        assert!(document.undo());
        assert_eq!(document.text(), "one two three");
    }

    #[test]
    fn controller_capture_composes_every_commit_in_one_edit_group() {
        let mut document = Document::new("a\nmiddle\na");
        let middle = TextAnchor::new(
            document.text_point(5).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );

        let (result, map) = document.capture_position_maps(|document| {
            document.begin_edit_group();
            document.replace(0..1, "A")?;
            document.replace(9..10, "A")?;
            document.end_edit_group();
            Ok::<_, DocumentError>(())
        });
        result.unwrap();

        assert_eq!(map.source_revision(), Revision(0));
        assert_eq!(map.target_revision(), document.revision());
        let mapped = map.map_text_anchor(middle).unwrap();
        assert!(matches!(
            mapped,
            MappingOutcome::Exact(anchor) if anchor.offset() == 5
        ));
        assert!(document.undo(), "the two commits still form one undo unit");
        assert_eq!(document.text(), "a\nmiddle\na");
    }

    #[test]
    fn controller_capture_identity_does_not_materialize_the_formatted_text() {
        let mut document = Document::new("line\n".repeat(20_000));
        document.insert(2, "X").unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());

        let (result, map) = document.capture_position_maps(|_| Ok::<_, DocumentError>(()));

        result.unwrap();
        assert_eq!(map.source_revision(), map.target_revision());
        assert_eq!(
            map.source_len(),
            document.projection().text_tree().byte_len()
        );
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn nested_controller_captures_publish_local_and_complete_outer_maps() {
        let mut document = Document::new("ab");
        let original_end = TextAnchor::new(
            document.text_point(2).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );

        let (result, outer) = document.capture_position_maps(|document| {
            document.begin_edit_group();
            let (result, inner) =
                document.capture_position_maps(|document| document.replace(0..1, "XY"));
            result?;
            document.insert(document.text().len(), "!")?;
            document.end_edit_group();
            Ok::<_, DocumentError>(inner)
        });
        let inner = result.unwrap();

        assert_eq!(document.text(), "XYb!");
        assert_eq!(inner.source_revision(), Revision(0));
        assert_eq!(inner.target_revision(), Revision(1));
        assert_eq!(
            inner
                .map_text_anchor(original_end)
                .unwrap()
                .value()
                .unwrap()
                .offset(),
            3
        );
        assert_eq!(outer.source_revision(), Revision(0));
        assert_eq!(outer.target_revision(), Revision(2));
        assert_eq!(
            outer
                .map_text_anchor(original_end)
                .unwrap()
                .value()
                .unwrap()
                .offset(),
            4
        );
        assert!(document.undo());
        assert_eq!(document.text(), "ab");
        assert!(!document.undo());
    }

    #[test]
    fn invalid_multi_edit_rolls_back_everything() {
        let mut document = Document::new("abcdef");
        let error = document
            .apply_edits(vec![TextEdit::new(1..4, "x"), TextEdit::new(3..5, "y")])
            .unwrap_err();
        assert_eq!(error, DocumentError::OverlappingEdits);
        assert_eq!(document.text(), "abcdef");
        assert_eq!(document.revision(), Revision(0));
    }

    #[test]
    fn undo_redo_is_branching() {
        let mut document = Document::new("a");
        document.insert(1, "b").unwrap();
        let ab_revision = document.revision();
        document.insert(2, "c").unwrap();
        assert!(document.undo());
        assert_eq!(document.revision(), ab_revision);
        assert!(document.undo());
        document.insert(1, "x").unwrap();
        assert_eq!(document.text(), "ax");
        assert!(document.undo());
        assert_eq!(document.redo_branch_count(), 2);
        assert!(document.select_redo_branch(0));
        assert!(document.redo());
        assert_eq!(document.text(), "ab");
    }

    #[test]
    fn edit_group_is_one_undo_step() {
        let mut document = Document::new("");
        document.begin_edit_group();
        document.insert(0, "a").unwrap();
        document.insert(1, "b").unwrap();
        document.insert(2, "c").unwrap();
        document.end_edit_group();
        assert_eq!(document.text(), "abc");
        assert!(document.undo());
        assert_eq!(document.text(), "");
        assert!(!document.undo());
    }

    #[test]
    fn grapheme_clusters_are_indivisible() {
        let mut document = Document::new("a\u{301}b");
        assert_eq!(
            document.delete(1..3),
            Err(DocumentError::NotGraphemeBoundary(1))
        );
        document.delete(0..3).unwrap();
        assert_eq!(document.text(), "b");
    }

    #[test]
    fn semantic_hard_break_interrupts_literal_crlf_grapheme() {
        let mut document = Document::from_bytes_with_file_format(
            b"\r\nrest".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        assert_eq!(document.text(), "\r\nrest");

        let lines = document.hard_line_snapshot();
        assert!(lines.is_grapheme_boundary(1));
        assert_eq!(lines.next_grapheme_boundary(0), Some(1));
        assert_eq!(lines.next_grapheme_boundary(1), Some(2));
        assert_eq!(lines.previous_grapheme_boundary(2), Some(1));
        assert_eq!(lines.grapheme_range_at(0), Some(0..1));
        assert_eq!(lines.grapheme_range_at(1), Some(1..2));
        assert_eq!(lines.grapheme_ranges(0..2), Some(vec![0..1, 1..2]));
        assert_eq!(lines.grapheme_count(0..2), Some(2));
        assert_eq!(lines.advance_graphemes(0, 2), Some(2));

        assert!(document.text_point(1).is_ok());
        assert!(document.text_point(2).is_ok());
        let content = document.capture_formatted_payload(0..1).unwrap();
        assert_eq!(content.text(), "\r");
        assert!(content.break_offsets().is_empty());
        let separator = document.capture_formatted_payload(1..2).unwrap();
        assert_eq!(separator.text(), "\n");
        assert_eq!(separator.break_offsets(), &[0]);

        document.insert(1, "x").unwrap();
        assert_eq!(document.text(), "\rx\nrest");
        assert_eq!(document.source_bytes(), b"\rx\nrest");
        document.delete(0..1).unwrap();
        assert_eq!(document.text(), "x\nrest");
    }

    #[test]
    fn text_points_reject_stale_snapshots_and_other_documents() {
        let mut first = Document::new("abc");
        let point = first.text_point(1).unwrap();
        first.insert(0, "x").unwrap();
        assert!(matches!(
            first.replace_points(point, point, "!"),
            Err(DocumentError::WrongSnapshot { .. })
        ));
        let second = Document::new("abc");
        let other = second.text_point(1).unwrap();
        assert_eq!(
            first.replace_points(other, other, "!"),
            Err(DocumentError::WrongDocument)
        );
    }

    #[test]
    fn line_queries_include_trailing_empty_line() {
        let document = Document::new("a\nb\n");
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.line_start(2), Some(4));
        assert_eq!(document.line_end(2), Some(4));
        assert_eq!(document.line_start(3), None);
    }

    #[test]
    fn file_format_conversion_is_undoable_and_preserves_formatted_text() {
        let mut document =
            Document::from_bytes(b"a\nb\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        document.set_file_format(FileFormat::Dos).unwrap();
        assert_eq!(document.text(), "a\nb\n");
        assert_eq!(document.source_bytes(), b"a\r\nb\r\n");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"a\nb\n");
    }

    #[test]
    fn bom_changes_are_explicit_and_undoable() {
        let mut document = Document::new("abc");
        document.set_bom_policy(BomPolicy::Always).unwrap();
        assert_eq!(document.source_bytes(), b"\xef\xbb\xbfabc");
        assert!(document.has_bom());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"abc");
    }

    #[test]
    fn lossy_reverse_mapping_across_an_opaque_item_is_rejected() {
        let document = Document::from_bytes_with_file_format(
            vec![b'a', 0xff, b'b'],
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();

        // Simulate a future format projection that maps a visible selection
        // across bytes belonging to an unselected opaque diagnostic item.
        assert_eq!(
            document.reject_unsafe_opaque_mapping(&(0..1), &(0..2)),
            Err(DocumentError::OpaqueDecodingConflict { source_range: 1..2 })
        );
        assert_eq!(document.source_bytes(), [b'a', 0xff, b'b']);
        assert_eq!(document.revision(), Revision(0));
    }

    #[test]
    fn source_boundaries_map_across_bom_encoding_and_crlf_without_rounding() {
        let document = Document::from_bytes_with_file_format(
            vec![0xef, 0xbb, 0xbf, 0xc3, 0xa9, b'\r', b'\n', b'x'],
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Dos,
        )
        .unwrap();
        let map = |offset| {
            let source = document.source_point(offset).unwrap();
            document
                .map_source_point(source, BoundaryAffinity::Downstream)
                .map(|mapping| mapping.text().offset())
        };

        assert_eq!(map(0).unwrap(), 0);
        assert_eq!(map(3).unwrap(), 0);
        assert_eq!(map(5).unwrap(), "é".len());
        assert_eq!(map(7).unwrap(), "é\n".len());
        assert_eq!(map(8).unwrap(), "é\nx".len());
        assert!(matches!(map(1), Err(SourceToTextError::InteriorBom { .. })));
        assert!(matches!(
            map(4),
            Err(SourceToTextError::InteriorMappedUnit { .. })
        ));
        assert!(matches!(
            map(6),
            Err(SourceToTextError::InteriorMappedUnit { .. })
        ));
        assert!(matches!(
            document.projection().map_source_boundary(
                document.revision(),
                9,
                BoundaryAffinity::Downstream
            ),
            Err(SourceToTextError::SourceOffsetOutOfBounds {
                offset: 9,
                length: 8
            })
        ));
    }

    #[test]
    fn source_mapping_handles_latin1_utf16_and_opaque_units() {
        let latin1 =
            Document::from_bytes(vec![0xe9, b'x'], Encoding::Latin1, Format::PlainText).unwrap();
        assert_eq!(
            latin1
                .map_source_point(
                    latin1.source_point(1).unwrap(),
                    BoundaryAffinity::Downstream
                )
                .unwrap()
                .text()
                .offset(),
            "é".len()
        );

        for (encoding, bytes) in [
            (Encoding::Utf16Le, vec![0xff, 0xfe, b'A', 0, b'B', 0]),
            (Encoding::Utf16Be, vec![0xfe, 0xff, 0, b'A', 0, b'B']),
        ] {
            let document = Document::from_bytes(bytes, encoding, Format::PlainText).unwrap();
            let mapped = document
                .map_source_point(
                    document.source_point(4).unwrap(),
                    BoundaryAffinity::Downstream,
                )
                .unwrap();
            assert_eq!(mapped.text().offset(), 1);
            assert!(matches!(
                document.map_source_point(
                    document.source_point(3).unwrap(),
                    BoundaryAffinity::Downstream
                ),
                Err(SourceToTextError::InteriorMappedUnit { .. })
            ));
        }

        let opaque = Document::from_bytes_with_file_format(
            vec![b'a', 0xf0, 0x9f, b'b'],
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        assert!(matches!(
            opaque.map_source_point(
                opaque.source_point(2).unwrap(),
                BoundaryAffinity::Downstream
            ),
            Err(SourceToTextError::InteriorOpaqueUnit {
                source_range,
                ..
            }) if source_range == (1..3)
        ));
        assert_eq!(
            opaque
                .map_source_point(
                    opaque.source_point(3).unwrap(),
                    BoundaryAffinity::Downstream
                )
                .unwrap()
                .text()
                .offset(),
            "a�".len()
        );
    }

    #[test]
    fn markdown_source_mapping_exposes_hidden_and_many_to_one_syntax() {
        let document = Document::from_bytes(
            b"# **bold**\n\\*".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(document.text(), "bold\n*");
        let map = |offset, affinity| {
            document.map_source_point(document.source_point(offset).unwrap(), affinity)
        };

        assert_eq!(
            map(0, BoundaryAffinity::Downstream)
                .unwrap()
                .text()
                .offset(),
            0
        );
        assert!(matches!(
            map(1, BoundaryAffinity::Downstream),
            Err(SourceToTextError::InteriorHiddenSyntax { .. })
        ));
        assert_eq!(
            map(4, BoundaryAffinity::Downstream)
                .unwrap()
                .text()
                .offset(),
            0
        );
        assert_eq!(
            map(8, BoundaryAffinity::Upstream).unwrap().text().offset(),
            4
        );
        assert!(matches!(
            map(9, BoundaryAffinity::Downstream),
            Err(SourceToTextError::InteriorHiddenSyntax { .. })
        ));
        // The backslash and escaped asterisk are one many-to-one provenance
        // unit, so their interior source boundary is not guessed.
        assert!(matches!(
            map(12, BoundaryAffinity::Downstream),
            Err(SourceToTextError::InteriorMappedUnit { .. })
        ));
        let eof = map(document.source_byte_len(), BoundaryAffinity::Downstream).unwrap();
        assert_eq!(eof.text().offset(), document.text().len());
        assert_eq!(eof.relation(), SourceBoundaryRelation::DocumentEnd);
    }

    #[test]
    fn source_mapping_rejects_stale_foreign_and_non_grapheme_points() {
        let mut document = Document::new("abc");
        let stale = document.source_point(1).unwrap();
        document.insert(0, "x").unwrap();
        assert!(matches!(
            document.map_source_point(stale, BoundaryAffinity::Downstream),
            Err(SourceToTextError::WrongSnapshot { .. })
        ));

        let other = Document::new("abc");
        assert_eq!(
            document.map_source_point(other.source_point(1).unwrap(), BoundaryAffinity::Downstream),
            Err(SourceToTextError::WrongDocument {
                expected: document.id(),
                actual: other.id()
            })
        );
        let wrong_part = SourcePoint::new(
            document.id(),
            document.revision(),
            SourcePartId(9),
            0,
            document.source_byte_len(),
        )
        .unwrap();
        assert_eq!(
            document.map_source_point(wrong_part, BoundaryAffinity::Downstream),
            Err(SourceToTextError::WrongSourcePart {
                expected: SourcePartId::PRIMARY,
                actual: SourcePartId(9)
            })
        );

        let grapheme = Document::new("a\u{301}");
        assert_eq!(
            grapheme.map_source_point(
                grapheme.source_point(1).unwrap(),
                BoundaryAffinity::Downstream
            ),
            Err(SourceToTextError::NotFormattedGraphemeBoundary {
                source_offset: 1,
                formatted_offset: 1
            })
        );
    }

    fn persistent_tree_fixture() -> String {
        (0..3)
            .map(|index| {
                let character = char::from(b'a' + index);
                std::iter::repeat(character)
                    .take(FORMATTED_TEXT_LEAF_BYTES)
                    .collect::<String>()
            })
            .collect()
    }

    fn leaf_ids(projection: &FormattedDocument) -> Vec<FormattedLeafId> {
        projection
            .text_tree()
            .leaves()
            .into_iter()
            .map(|leaf| leaf.id)
            .collect()
    }

    fn block_ids(document: &Document) -> Vec<u64> {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect()
    }

    #[test]
    fn plain_block_ids_survive_edits_splits_joins_and_deletes() {
        let mut document = Document::new("alpha\nbeta\ngamma");
        let initial = block_ids(&document);
        assert_eq!(initial, vec![1, 2, 3]);

        document.replace(7..8, "E").unwrap();
        assert_eq!(block_ids(&document), initial);

        document.insert(8, "\n").unwrap();
        let split = block_ids(&document);
        assert_eq!(split[0], initial[0]);
        assert_eq!(split[1], initial[1]);
        assert_eq!(split[3], initial[2]);
        assert!(split[2] > *initial.last().unwrap());

        document.delete(8..9).unwrap();
        assert_eq!(block_ids(&document), initial);

        document.delete(0..6).unwrap();
        assert_eq!(block_ids(&document), initial[1..]);
    }

    #[test]
    fn replacing_all_block_content_or_editing_the_sole_empty_block_retains_identity() {
        let mut document = Document::new("first\nmiddle\nlast");
        let initial = block_ids(&document);
        document.replace(0..5, "FIRST").unwrap();
        document.replace(6..12, "MIDDLE").unwrap();
        document.replace(13..17, "LAST").unwrap();
        assert_eq!(block_ids(&document), initial);

        let mut sole = Document::new("");
        let sole_id = block_ids(&sole)[0];
        sole.insert(0, "content").unwrap();
        assert_eq!(block_ids(&sole), vec![sole_id]);
        sole.replace(0..sole.text().len(), "replacement").unwrap();
        assert_eq!(block_ids(&sole), vec![sole_id]);
        sole.delete(0..sole.text().len()).unwrap();
        assert_eq!(block_ids(&sole), vec![sole_id]);
    }

    #[test]
    fn joining_keeps_the_first_block_while_line_deletion_keeps_survivors() {
        let mut joined = Document::new("left\nright");
        let joined_initial = block_ids(&joined);
        joined.delete(4..5).unwrap();
        assert_eq!(block_ids(&joined), vec![joined_initial[0]]);

        let mut empty_first = Document::new("\nright");
        let empty_initial = block_ids(&empty_first);
        empty_first.delete(0..1).unwrap();
        assert_eq!(block_ids(&empty_first), vec![empty_initial[0]]);

        let mut deleted = Document::new("a\nb\nc");
        let initial = block_ids(&deleted);
        deleted.delete(2..4).unwrap();
        assert_eq!(block_ids(&deleted), vec![initial[0], initial[2]]);

        deleted.try_undo().unwrap();
        deleted.delete(3..5).unwrap();
        assert_eq!(block_ids(&deleted), initial[..2]);

        deleted.try_undo().unwrap();
        deleted.delete(2..3).unwrap();
        assert_eq!(block_ids(&deleted), initial);
    }

    #[test]
    fn newline_inserted_before_a_block_does_not_steal_its_identity() {
        let mut document = Document::new("a\nb\nc");
        let initial = block_ids(&document);
        document.insert(0, "\n").unwrap();
        let after = block_ids(&document);
        assert_eq!(&after[1..], initial);
        assert!(after[0] > *initial.last().unwrap());

        let mut later = Document::new("a\nb\nc");
        let initial = block_ids(&later);
        later.insert(2, "\n").unwrap();
        let after = block_ids(&later);
        assert_eq!(after[0], initial[0]);
        assert_eq!(after[2..], initial[1..]);
        assert!(after[1] > *initial.last().unwrap());
    }

    #[test]
    fn discontiguous_splits_preserve_unchanged_middle_block_ids() {
        let mut document = Document::new("aa\nbb\ncc\ndd");
        let initial = block_ids(&document);
        document
            .apply_edits(vec![TextEdit::new(1..1, "\n"), TextEdit::new(10..10, "\n")])
            .unwrap();
        let after = block_ids(&document);
        assert_eq!(after[0], initial[0]);
        assert_eq!(after[2..4], initial[1..3]);
        assert_eq!(after[4], initial[3]);
        assert!(after[1] > *initial.last().unwrap());
        assert!(after[5] > after[1]);
    }

    #[test]
    fn markdown_headings_and_following_blocks_keep_source_backed_ids() {
        let mut document = Document::from_bytes(
            b"# Head\nbody\nlast".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let initial = block_ids(&document);
        assert_eq!(
            document.projection().blocks()[0].kind,
            BlockKind::Heading(1)
        );

        document.replace(0..1, "h").unwrap();
        assert_eq!(block_ids(&document), initial);

        document.insert(2, "\n").unwrap();
        let split = block_ids(&document);
        assert_eq!(split[0], initial[0]);
        assert_eq!(split[2..], initial[1..]);
        assert!(split[1] > *initial.last().unwrap());
        assert_eq!(
            document.projection().blocks()[0].kind,
            BlockKind::Heading(1)
        );
        assert_eq!(document.projection().blocks()[1].kind, BlockKind::Paragraph);
    }

    #[test]
    fn block_allocator_never_reuses_an_undone_branch_identity() {
        let mut document = Document::new("a\nb");
        let root = document.history_status().current.node;

        document.insert(1, "\n").unwrap();
        let first_branch = document.history_status().current.node;
        let first_branch_ids = block_ids(&document);
        let first_new_id = first_branch_ids[1];

        document.try_undo().unwrap();
        document.insert(document.text().len(), "\n").unwrap();
        let second_branch = document.history_status().current.node;
        let second_branch_ids = block_ids(&document);
        let second_new_id = *second_branch_ids.last().unwrap();
        assert_ne!(first_new_id, second_new_id);
        assert!(second_new_id > first_new_id);

        document.select_history_node(first_branch).unwrap();
        assert_eq!(block_ids(&document), first_branch_ids);
        document.select_history_node(second_branch).unwrap();
        assert_eq!(block_ids(&document), second_branch_ids);
        document.select_history_node(root).unwrap();
        assert_eq!(block_ids(&document), vec![1, 2]);
        assert!(document.next_projected_block_id > second_new_id);
    }

    #[test]
    fn competing_preparations_cannot_publish_the_same_block_reservation() {
        let mut document = Document::new("a\nb");
        let request = |document: &Document, at| ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "\n")],
        };
        let first = document
            .prepare_model_request(request(&document, 1))
            .unwrap();
        let second = document
            .prepare_model_request(request(&document, 3))
            .unwrap();
        assert_eq!(document.next_projected_block_id, 3);

        document.commit_model_transaction(first).unwrap();
        let published_new_id = block_ids(&document)[1];
        assert_eq!(
            document.commit_model_transaction(second),
            Err(ModelTransactionError::StaleRevision {
                expected: Revision(0),
                actual: Revision(1),
            })
        );

        document.try_undo().unwrap();
        document.insert(document.text().len(), "\n").unwrap();
        let alternate_new_id = *block_ids(&document).last().unwrap();
        assert!(alternate_new_id > published_new_id);
    }

    #[test]
    fn prepared_block_allocator_precondition_and_exhaustion_are_atomic() {
        let mut stale = Document::new("a\nb");
        let prepared = stale
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: stale.id(),
                revision: stale.revision(),
                edits: vec![TextEdit::new(1..1, "\n")],
            })
            .unwrap();
        let before_ids = block_ids(&stale);
        stale.next_projected_block_id += 1;
        assert_eq!(
            stale.commit_model_transaction(prepared),
            Err(ModelTransactionError::StaleDocumentState)
        );
        assert_eq!(stale.revision(), Revision(0));
        assert_eq!(block_ids(&stale), before_ids);

        let mut exhausted = Document::new("a\nb");
        exhausted.next_projected_block_id = u64::MAX;
        let before_source = exhausted.source_bytes();
        let before_ids = block_ids(&exhausted);
        assert_eq!(
            exhausted.insert(1, "\n"),
            Err(DocumentError::BlockIdentityExhausted)
        );
        assert_eq!(exhausted.revision(), Revision(0));
        assert_eq!(exhausted.source_bytes(), before_source);
        assert_eq!(block_ids(&exhausted), before_ids);
    }

    #[test]
    fn text_edits_reuse_untouched_leaves_at_beginning_middle_and_end() {
        let original = persistent_tree_fixture();
        let cases = [
            (0..0, "start"),
            (
                FORMATTED_TEXT_LEAF_BYTES + 100..FORMATTED_TEXT_LEAF_BYTES + 110,
                "middle",
            ),
            (original.len()..original.len(), "end"),
        ];

        for (range, replacement) in cases {
            let mut document = Document::new(original.clone());
            let before = document.projection().clone();
            let before_ids = leaf_ids(&before);
            document.replace(range, replacement).unwrap();
            let after = document.projection();
            let after_ids = leaf_ids(after);

            assert!(
                before_ids
                    .iter()
                    .filter(|identity| after_ids.contains(identity))
                    .count()
                    >= 2
            );
            assert!(
                before
                    .text_tree()
                    .shared_backing_leaf_count(after.text_tree())
                    >= 2
            );
            assert_eq!(after.text_tree().flatten(), after.text());
        }
    }

    #[test]
    fn discontiguous_edits_keep_the_unchanged_middle_leaf_identities() {
        let original = persistent_tree_fixture();
        let mut document = Document::new(original);
        let before = document.projection().clone();
        let middle_ids: Vec<_> = before.text_tree().leaves()[1..2]
            .iter()
            .map(|leaf| leaf.id)
            .collect();
        document
            .apply_edits(vec![
                TextEdit::new(10..11, "X"),
                TextEdit::new(
                    FORMATTED_TEXT_LEAF_BYTES * 2 + 10..FORMATTED_TEXT_LEAF_BYTES * 2 + 11,
                    "Y",
                ),
            ])
            .unwrap();

        let after_ids = leaf_ids(document.projection());
        assert!(middle_ids
            .iter()
            .all(|identity| after_ids.contains(identity)));
        assert!(
            before
                .text_tree()
                .shared_backing_leaf_count(document.projection().text_tree())
                >= 1
        );
    }

    #[test]
    fn style_fileformat_and_bom_revisions_share_identical_text_storage() {
        let mut markdown =
            Document::from_bytes(b"plain text".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let before_style = markdown.projection().clone();
        let before_style_blocks = block_ids(&markdown);
        markdown
            .set_semantic_style(0..5, SemanticInlineStyle::Strong, true)
            .unwrap();
        assert!(before_style
            .text_tree()
            .shares_root_with(markdown.projection().text_tree()));
        assert!(before_style.shares_flat_text_with(markdown.projection()));
        assert_eq!(block_ids(&markdown), before_style_blocks);

        let mut endings = Document::new("a\nb\n");
        let before_fileformat = endings.projection().clone();
        let before_fileformat_blocks = block_ids(&endings);
        endings.set_file_format(FileFormat::Dos).unwrap();
        assert!(before_fileformat
            .text_tree()
            .shares_root_with(endings.projection().text_tree()));
        assert!(before_fileformat.shares_flat_text_with(endings.projection()));
        assert_eq!(block_ids(&endings), before_fileformat_blocks);

        let mut bom = Document::new("same text");
        let before_bom = bom.projection().clone();
        let before_bom_blocks = block_ids(&bom);
        bom.set_bom_policy(BomPolicy::Always).unwrap();
        assert!(before_bom
            .text_tree()
            .shares_root_with(bom.projection().text_tree()));
        assert!(before_bom.shares_flat_text_with(bom.projection()));
        assert_eq!(block_ids(&bom), before_bom_blocks);
    }

    #[test]
    fn undo_restores_the_retained_historical_tree() {
        let mut document = Document::new(persistent_tree_fixture());
        let initial = document.projection().clone();
        document.insert(FORMATTED_TEXT_LEAF_BYTES, "new").unwrap();
        assert!(!initial
            .text_tree()
            .shares_root_with(document.projection().text_tree()));
        assert!(document.undo());
        assert!(initial
            .text_tree()
            .shares_root_with(document.projection().text_tree()));
        assert!(initial.shares_flat_text_with(document.projection()));
    }

    #[test]
    fn persistent_batch_handles_grapheme_joins_and_failures_are_atomic() {
        let mut joined = Document::new("xb");
        joined
            .apply_edits(vec![
                TextEdit::new(0..1, "a"),
                TextEdit::new(1..1, "\u{301}"),
            ])
            .unwrap();
        assert_eq!(joined.text(), "a\u{301}b");
        assert_eq!(joined.projection().text_tree().flatten(), joined.text());

        let mut invalid = Document::new("a\u{301}tail");
        let before_invalid = invalid.projection().clone();
        let revision = invalid.revision();
        assert_eq!(
            invalid.insert(1, "x"),
            Err(DocumentError::NotGraphemeBoundary(1))
        );
        assert_eq!(invalid.revision(), revision);
        assert!(before_invalid
            .text_tree()
            .shares_root_with(invalid.projection().text_tree()));
        assert!(before_invalid.shares_flat_text_with(invalid.projection()));

        let mut latin1 =
            Document::from_bytes(b"latin one".to_vec(), Encoding::Latin1, Format::PlainText)
                .unwrap();
        let before_latin1 = latin1.projection().clone();
        let revision = latin1.revision();
        assert!(matches!(
            latin1.insert(5, "😀"),
            Err(DocumentError::UnrepresentableCharacter { .. })
        ));
        assert_eq!(latin1.revision(), revision);
        assert!(before_latin1
            .text_tree()
            .shares_root_with(latin1.projection().text_tree()));
        assert!(before_latin1.shares_flat_text_with(latin1.projection()));
    }

    #[test]
    fn one_line_edit_in_large_document_projects_only_that_line() {
        use std::fmt::Write as _;

        let line_count = 50_000usize;
        let mut source = String::with_capacity(line_count * 24);
        let mut starts = Vec::with_capacity(line_count);
        for line in 0..line_count {
            starts.push(source.len());
            writeln!(&mut source, "line {line:05} payload").unwrap();
        }
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::PlainText).unwrap();
        let before_source_lines = document.state().source_hard_lines.clone();
        let target_line = line_count / 2;
        let target = starts[target_line] + "line 00000 ".len();
        let before_projection = document.projection().clone();
        let before_ids = [
            document.projection().hard_line_id(0).unwrap(),
            document.projection().hard_line_id(target_line - 1).unwrap(),
            document.projection().hard_line_id(target_line + 1).unwrap(),
            document.projection().hard_line_id(line_count - 1).unwrap(),
        ];
        let request = ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(target..target + "payload".len(), "word")],
        };
        let prepared = document.prepare_model_request(request).unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert_eq!(work.projected_hard_lines(), 1);
        assert_eq!(work.source_hard_line_records_rebuilt(), 1);
        assert_eq!(work.projected_formatted_bytes(), "line 25000 word".len());
        assert!(work.decoded_source_bytes() < document.source_bytes().len() / 1_000);
        assert_eq!(work.full_text_bytes_materialized(), 0);
        assert!(work.persistent_nodes_visited() < 512, "{work:?}");
        assert!(work.persistent_nodes_copied() < 512, "{work:?}");
        assert!(work.persistent_records_copied() < 1_024, "{work:?}");
        assert!(work.persistent_leaves_copied() < 64, "{work:?}");
        assert!(work.formatted_text_leaves_copied() < 16, "{work:?}");
        assert_eq!(prepared.summary().source_patches().len(), 1);

        document.commit_model_transaction(prepared).unwrap();
        assert!(
            !document.projection().compatibility_text_is_materialized(),
            "regional publication must not eagerly flatten the target tree"
        );
        assert_eq!(
            [
                document.projection().hard_line_id(0).unwrap(),
                document.projection().hard_line_id(target_line - 1).unwrap(),
                document.projection().hard_line_id(target_line + 1).unwrap(),
                document.projection().hard_line_id(line_count - 1).unwrap(),
            ],
            before_ids
        );
        assert!(
            before_projection
                .text_tree()
                .shared_backing_leaf_count(document.projection().text_tree())
                > 100
        );
        assert_eq!(
            document
                .state()
                .source_hard_lines
                .shared_leaf_count_with(&before_source_lines),
            before_source_lines.leaf_count() - 1
        );

        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::PlainText)
                .unwrap();
        assert_eq!(document.text(), reopened.text());
        assert_eq!(
            document.projection().provenance(),
            reopened.projection().provenance()
        );
        assert_eq!(
            document.state().source_hard_lines,
            reopened.state().source_hard_lines
        );
    }

    #[test]
    fn line_local_projection_is_exact_for_formats_and_encodings() {
        for format in [Format::PlainText, Format::Markdown] {
            for encoding in [
                Encoding::Utf8,
                Encoding::Latin1,
                Encoding::Utf16Le,
                Encoding::Utf16Be,
            ] {
                let source_text = match format {
                    Format::PlainText => "first\nsecond\nthird",
                    Format::Markdown => "first\n**second**\nthird",
                };
                let bytes = encoding.encode_fragment(source_text).unwrap();
                let mut document =
                    Document::from_bytes(bytes, encoding, format).expect("test source decodes");
                let start = document.text().find("second").unwrap();
                let request = ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(start..start + "second".len(), "changed")],
                };
                let prepared = document.prepare_model_request(request).unwrap();
                assert_eq!(
                    prepared.summary().projection_work().scope(),
                    ProjectionWorkScope::RegionalHardLines,
                    "{format:?} {encoding:?}"
                );
                assert_eq!(
                    prepared.summary().projection_work().projected_hard_lines(),
                    1
                );
                document.commit_model_transaction(prepared).unwrap();

                let reopened = Document::from_bytes(document.source_bytes(), encoding, format)
                    .expect("regional candidate remains reopenable");
                assert_eq!(document.text(), reopened.text(), "{format:?} {encoding:?}");
                assert_eq!(
                    document.projection().provenance(),
                    reopened.projection().provenance(),
                    "{format:?} {encoding:?}"
                );
                assert_eq!(
                    document.projection().style_spans(),
                    reopened.projection().style_spans(),
                    "{format:?} {encoding:?}"
                );
                assert_eq!(
                    document.state().source_hard_lines,
                    reopened.state().source_hard_lines,
                    "{format:?} {encoding:?}"
                );
            }
        }
    }

    #[test]
    fn regional_decode_does_not_consume_a_line_initial_bom_scalar() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = encoding
                .encode_fragment("first\n\u{feff}second\nthird")
                .unwrap();
            let mut document = Document::from_bytes(bytes, encoding, Format::PlainText).unwrap();
            let start = document.text().find("second").unwrap();
            let request = ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(start..start + "second".len(), "changed")],
            };
            let prepared = document.prepare_model_request(request).unwrap();
            assert_eq!(
                prepared.summary().projection_work().scope(),
                ProjectionWorkScope::RegionalHardLines
            );
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(
                document.text(),
                "first\n\u{feff}changed\nthird",
                "{encoding:?}"
            );
        }
    }

    #[test]
    fn regional_suffix_transforms_match_clean_projection_with_opaque_units() {
        let cases = [
            (Encoding::Utf8, vec![b'a', b'\n', 0xff, b'z']),
            (
                Encoding::Utf16Le,
                vec![
                    0xff, 0xfe, // BOM
                    0x61, 0x00, // a
                    0x0a, 0x00, // newline
                    0x00, 0xdc, // unpaired low surrogate
                    0x7a, 0x00, // z
                ],
            ),
            (
                Encoding::Utf16Be,
                vec![
                    0xfe, 0xff, // BOM
                    0x00, 0x61, // a
                    0x00, 0x0a, // newline
                    0xdc, 0x00, // unpaired low surrogate
                    0x00, 0x7a, // z
                ],
            ),
        ];

        for (encoding, source) in cases {
            let mut document = Document::from_bytes(source, encoding, Format::PlainText).unwrap();
            let request = ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(0..1, "lengthened")],
            };
            let prepared = document.prepare_model_request(request).unwrap();
            assert_eq!(
                prepared.summary().projection_work().scope(),
                ProjectionWorkScope::RegionalHardLines,
                "{encoding:?}"
            );
            document.commit_model_transaction(prepared).unwrap();

            let reopened =
                Document::from_bytes(document.source_bytes(), encoding, Format::PlainText).unwrap();
            assert_eq!(
                document.projection().provenance(),
                reopened.projection().provenance(),
                "{encoding:?}"
            );
            let diagnostic_coordinates = |document: &Document| {
                document
                    .decoding_diagnostics()
                    .iter()
                    .map(|diagnostic| {
                        (
                            diagnostic.encoding,
                            diagnostic.kind,
                            diagnostic.source_range.clone(),
                            diagnostic.formatted_range.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                diagnostic_coordinates(&document),
                diagnostic_coordinates(&reopened),
                "{encoding:?}"
            );
            assert!(document
                .decoding_diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.revision == document.revision()));
        }
    }

    #[test]
    fn hard_line_topology_edits_report_full_projection_fallback() {
        let document = Document::new("first\nsecond\nthird");
        let request = ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(8..8, "new\n")],
        };
        let prepared = document.prepare_model_request(request).unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::FullDocument
        );
        assert_eq!(
            prepared.summary().projection_work().projected_hard_lines(),
            4
        );
    }

    #[test]
    fn identity_backed_anchor_follows_an_untouched_leaf_after_an_unrelated_edit() {
        let mut document = Document::new(format!("{}target", "a".repeat(8_192)));
        let original_offset = 6_000;
        let anchor = document
            .text_anchor(
                document.text_point(original_offset).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();
        assert!(matches!(
            anchor.backing(),
            AnchorBacking::StableProjectedIdentity { .. }
        ));

        document.insert(0, "prefix").unwrap();
        assert!(matches!(
            document.resolve_text_anchor(anchor).unwrap(),
            MappingOutcome::Moved(point) if point.offset() == original_offset + "prefix".len()
        ));
    }

    #[test]
    fn stable_boundary_identity_keeps_insertion_association_independent_of_affinity() {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            let mut document = Document::new("abcdef");
            let point = document.text_point(3).unwrap();
            let before = document
                .text_anchor(
                    point,
                    Association::BeforeInsertion,
                    affinity,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )
                .unwrap();
            let after = document
                .text_anchor(
                    point,
                    Association::AfterInsertion,
                    affinity,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )
                .unwrap();

            document.insert(3, "X").unwrap();
            assert!(matches!(
                document.resolve_text_anchor(before).unwrap(),
                MappingOutcome::Moved(point) if point.offset() == 3
            ));
            assert!(matches!(
                document.resolve_text_anchor(after).unwrap(),
                MappingOutcome::Moved(point) if point.offset() == 4
            ));
        }
    }

    #[test]
    fn stable_block_identity_follows_an_intact_moved_hard_line() {
        let mut document = Document::new("one\ntwo\nthree");
        let anchor = document
            .text_anchor(
                document.text_point(5).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();

        document
            .transfer_hard_lines(HardLineTransfer::Move, 1..2, 0)
            .unwrap();
        assert_eq!(document.text(), "two\none\nthree");
        assert!(matches!(
            document.resolve_text_anchor(anchor).unwrap(),
            MappingOutcome::Moved(point) if point.offset() == 1
        ));
    }

    #[test]
    fn source_provenance_recovers_after_a_projection_only_storage_rebuild() {
        let line = "abcdefghij".repeat(8);
        let document = Document::new(format!("{line}\nsecond"));
        let anchor = document
            .text_anchor(
                document.text_point(30).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();

        // Build the same semantic projection through a fresh pipeline snapshot
        // without installing the normal persistent-storage reuse optimization.
        let source = document.source_bytes();
        let decoded = document.encoding().decode(&source).unwrap();
        let rebuilt = build_state_from_decoded(
            SourceSnapshot::new(source),
            decoded,
            document.format(),
            document.file_format(),
            document.file_format_origin(),
            document.line_ending_evidence(),
            Revision(1),
        )
        .unwrap();
        let outcome = rebuilt
            .projection
            .resolve_text_anchor(document.id(), anchor)
            .unwrap();
        assert!(matches!(
            outcome,
            MappingOutcome::RecoveredFromProvenance(point) if point.offset() == 30
        ));
    }

    #[test]
    fn deleted_anchor_is_not_guessed_from_its_stale_ordinal_or_provenance() {
        let mut document = Document::new("abcdef");
        let anchor = document
            .text_anchor(
                document.text_point(3).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::Unresolvable,
            )
            .unwrap();
        document.replace(2..4, "").unwrap();
        assert_eq!(
            document.resolve_text_anchor(anchor).unwrap(),
            MappingOutcome::Unresolvable(UnresolvableAnchor::MissingProvenance)
        );
    }

    #[test]
    fn document_identity_allocator_never_wraps_or_reuses_zero() {
        let allocator = AtomicU64::new(u64::MAX - 1);
        assert_eq!(
            allocate_document_id(&allocator),
            Ok(DocumentId(u64::MAX - 1))
        );
        assert_eq!(allocate_document_id(&allocator), Ok(DocumentId(u64::MAX)));
        assert_eq!(
            allocate_document_id(&allocator),
            Err(DocumentError::DocumentIdentityExhausted)
        );
        assert_eq!(allocator.load(Ordering::Relaxed), 0);
    }
}
