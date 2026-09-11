//! Snapshot-bound positions, checked ranges, persistent anchors, and splice maps.
//!
//! Snapshot ordinals remain useful compatibility coordinates, but anchors
//! captured by a document additionally retain identities for both adjacent
//! persistent rope content and their containing logical blocks. A recoverable
//! source boundary is the final fallback when a projection-only rebuild retires
//! those identities. Stale ordinals are never silently interpreted in a newer
//! snapshot.

use super::formatted_text::{
    FormattedBufferId, FormattedLeafId, FormattedLeafRevision, LeafBoundarySide,
    LogicalGraphemeSnapshot,
};
use super::{Association, BoundaryAffinity, DocumentId, Revision, TextPoint};
use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Identifies one named part of a source artifact.
///
/// The first implementation has a single source part.  Giving that part a
/// nominal identity now prevents source offsets from being mixed with formatted
/// offsets and leaves room for packaged formats later.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePartId(pub u64);

impl SourcePartId {
    pub const PRIMARY: Self = Self(0);
}

/// The coordinate domain carried by a position map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PositionDomain {
    Source(SourcePartId),
    FormattedText,
}

/// A byte boundary in one part of one immutable source snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourcePoint {
    document: DocumentId,
    revision: Revision,
    part: SourcePartId,
    offset: usize,
}

impl SourcePoint {
    /// Construct a checked source byte boundary.
    ///
    /// Every byte boundary is structurally legal at this layer.  Encoding and
    /// format stages remain responsible for rejecting patches that split a
    /// construct which they require to remain indivisible.
    pub fn new(
        document: DocumentId,
        revision: Revision,
        part: SourcePartId,
        offset: usize,
        part_len: usize,
    ) -> Result<Self, PositionError> {
        if offset > part_len {
            return Err(PositionError::InvalidBoundary {
                domain: PositionDomain::Source(part),
                offset,
                length: part_len,
            });
        }
        Ok(Self {
            document,
            revision,
            part,
            offset,
        })
    }

    pub fn document(self) -> DocumentId {
        self.document
    }

    pub fn revision(self) -> Revision {
        self.revision
    }

    pub fn part(self) -> SourcePartId {
        self.part
    }

    pub fn offset(self) -> usize {
        self.offset
    }

    pub fn compare_checked(self, other: Self) -> Result<Ordering, PositionError> {
        validate_source_pair(self, other)?;
        Ok(self.offset.cmp(&other.offset))
    }
}

/// Structured failures for checked coordinate and mapping operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionError {
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    WrongDomain {
        expected: PositionDomain,
        actual: PositionDomain,
    },
    WrongSnapshot {
        expected: Revision,
        actual: Revision,
    },
    WrongSourcePart {
        expected: SourcePartId,
        actual: SourcePartId,
    },
    InvalidBoundary {
        domain: PositionDomain,
        offset: usize,
        length: usize,
    },
    InvalidUnicodeBoundary {
        offset: usize,
    },
    InvertedRange {
        start: usize,
        end: usize,
    },
    OverlappingSplices {
        first: Range<usize>,
        second: Range<usize>,
    },
    TargetLengthMismatch {
        computed: usize,
        actual: usize,
    },
    MapChainLengthMismatch {
        expected: usize,
        actual: usize,
    },
    ArithmeticOverflow,
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongDocument { expected, actual } => write!(
                formatter,
                "position belongs to document {}, expected {}",
                actual.0, expected.0
            ),
            Self::WrongDomain { expected, actual } => {
                write!(
                    formatter,
                    "position domain is {actual:?}, expected {expected:?}"
                )
            }
            Self::WrongSnapshot { expected, actual } => write!(
                formatter,
                "position belongs to revision {}, expected {}",
                actual.0, expected.0
            ),
            Self::WrongSourcePart { expected, actual } => write!(
                formatter,
                "source position belongs to part {}, expected {}",
                actual.0, expected.0
            ),
            Self::InvalidBoundary {
                domain,
                offset,
                length,
            } => write!(
                formatter,
                "offset {offset} is not a boundary in {domain:?} of length {length}"
            ),
            Self::InvalidUnicodeBoundary { offset } => {
                write!(
                    formatter,
                    "offset {offset} is not an extended-grapheme boundary"
                )
            }
            Self::InvertedRange { start, end } => {
                write!(formatter, "range start {start} follows its end {end}")
            }
            Self::OverlappingSplices { first, second } => write!(
                formatter,
                "splice {}..{} overlaps {}..{}",
                first.start, first.end, second.start, second.end
            ),
            Self::TargetLengthMismatch { computed, actual } => write!(
                formatter,
                "splice map computes target length {computed}, but target length is {actual}"
            ),
            Self::MapChainLengthMismatch { expected, actual } => write!(
                formatter,
                "position maps meet at different lengths: {expected} and {actual}"
            ),
            Self::ArithmeticOverflow => formatter.write_str("position arithmetic overflowed"),
        }
    }
}

impl std::error::Error for PositionError {}

impl TextPoint {
    /// Compare points only when their document and immutable snapshot match.
    pub fn compare_checked(self, other: Self) -> Result<Ordering, PositionError> {
        validate_text_pair(self, other)?;
        Ok(self.offset().cmp(&other.offset()))
    }

    /// Return the snapshot-local byte distance between two ordered text points.
    pub fn distance_to(self, other: Self) -> Result<usize, PositionError> {
        validate_text_pair(self, other)?;
        other
            .offset()
            .checked_sub(self.offset())
            .ok_or(PositionError::InvertedRange {
                start: self.offset(),
                end: other.offset(),
            })
    }
}

/// A checked half-open range in one source part and source snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceRange {
    start: SourcePoint,
    end: SourcePoint,
}

impl SourceRange {
    pub fn new(start: SourcePoint, end: SourcePoint) -> Result<Self, PositionError> {
        validate_source_pair(start, end)?;
        if start.offset > end.offset {
            return Err(PositionError::InvertedRange {
                start: start.offset,
                end: end.offset,
            });
        }
        Ok(Self { start, end })
    }

    pub fn start(self) -> SourcePoint {
        self.start
    }

    pub fn end(self) -> SourcePoint {
        self.end
    }

    pub fn is_empty(self) -> bool {
        self.start.offset == self.end.offset
    }

    pub fn len(self) -> usize {
        self.end.offset - self.start.offset
    }
}

/// A checked half-open range in one immutable formatted snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextRange {
    start: TextPoint,
    end: TextPoint,
}

impl TextRange {
    pub fn new(start: TextPoint, end: TextPoint) -> Result<Self, PositionError> {
        validate_text_pair(start, end)?;
        if start.offset() > end.offset() {
            return Err(PositionError::InvertedRange {
                start: start.offset(),
                end: end.offset(),
            });
        }
        Ok(Self { start, end })
    }

    pub fn start(self) -> TextPoint {
        self.start
    }

    pub fn end(self) -> TextPoint {
        self.end
    }

    pub fn is_empty(self) -> bool {
        self.start.offset() == self.end.offset()
    }

    pub fn len(self) -> usize {
        self.end.offset() - self.start.offset()
    }

    pub fn contains(self, point: TextPoint) -> Result<bool, PositionError> {
        validate_text_pair(self.start, point)?;
        Ok(self.start.offset() <= point.offset() && point.offset() < self.end.offset())
    }

    pub fn intersection(self, other: Self) -> Result<Option<Self>, PositionError> {
        validate_text_pair(self.start, other.start)?;
        let start = self.start.offset().max(other.start.offset());
        let end = self.end.offset().min(other.end.offset());
        if start >= end {
            return Ok(None);
        }
        Ok(Some(text_range_unchecked(
            self.start.document(),
            self.start.revision(),
            start,
            end,
        )))
    }
}

/// Whether adjacent input segments retain semantic separation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjacentRangePolicy {
    Coalesce,
    Preserve,
}

/// Sorted, non-overlapping ranges in one formatted snapshot.
///
/// Empty ranges are valid as standalone [`TextRange`] values but do not cover
/// content, so mathematical set construction drops them.  Row-tagged Visual
/// Block ranges use their own semantic container rather than relying on empty
/// generic set segments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeSet {
    document: DocumentId,
    revision: Revision,
    segments: Vec<TextRange>,
    adjacent_policy: AdjacentRangePolicy,
}

impl RangeSet {
    pub fn empty(document: DocumentId, revision: Revision) -> Self {
        Self {
            document,
            revision,
            segments: Vec::new(),
            adjacent_policy: AdjacentRangePolicy::Coalesce,
        }
    }

    pub fn new(
        document: DocumentId,
        revision: Revision,
        ranges: impl IntoIterator<Item = TextRange>,
    ) -> Result<Self, PositionError> {
        Self::with_policy(document, revision, ranges, AdjacentRangePolicy::Coalesce)
    }

    pub fn preserving_adjacent(
        document: DocumentId,
        revision: Revision,
        ranges: impl IntoIterator<Item = TextRange>,
    ) -> Result<Self, PositionError> {
        Self::with_policy(document, revision, ranges, AdjacentRangePolicy::Preserve)
    }

    fn with_policy(
        document: DocumentId,
        revision: Revision,
        ranges: impl IntoIterator<Item = TextRange>,
        adjacent_policy: AdjacentRangePolicy,
    ) -> Result<Self, PositionError> {
        let mut ranges = ranges.into_iter().collect::<Vec<_>>();
        for range in &ranges {
            validate_text_context(range.start, document, revision)?;
            validate_text_context(range.end, document, revision)?;
        }
        ranges.retain(|range| !range.is_empty());
        ranges.sort_by_key(|range| (range.start.offset(), range.end.offset()));

        let mut segments: Vec<TextRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = segments.last_mut() {
                let overlaps = range.start.offset() < previous.end.offset();
                let touches = range.start.offset() == previous.end.offset();
                if overlaps || (touches && adjacent_policy == AdjacentRangePolicy::Coalesce) {
                    if range.end.offset() > previous.end.offset() {
                        previous.end = range.end;
                    }
                    continue;
                }
            }
            segments.push(range);
        }
        Ok(Self {
            document,
            revision,
            segments,
            adjacent_policy,
        })
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn segments(&self) -> &[TextRange] {
        &self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn adjacent_policy(&self) -> AdjacentRangePolicy {
        self.adjacent_policy
    }

    pub fn contains(&self, point: TextPoint) -> Result<bool, PositionError> {
        validate_text_context(point, self.document, self.revision)?;
        let index = self
            .segments
            .partition_point(|range| range.end.offset() <= point.offset());
        Ok(self
            .segments
            .get(index)
            .map(|range| range.start.offset() <= point.offset())
            .unwrap_or(false))
    }

    pub fn union(&self, other: &Self) -> Result<Self, PositionError> {
        self.validate_compatible(other)?;
        Self::new(
            self.document,
            self.revision,
            self.segments.iter().chain(other.segments.iter()).copied(),
        )
    }

    pub fn intersection(&self, other: &Self) -> Result<Self, PositionError> {
        self.validate_compatible(other)?;
        let mut intersections = Vec::new();
        let (mut left, mut right) = (0, 0);
        while left < self.segments.len() && right < other.segments.len() {
            let a = self.segments[left];
            let b = other.segments[right];
            let start = a.start.offset().max(b.start.offset());
            let end = a.end.offset().min(b.end.offset());
            if start < end {
                intersections.push(text_range_unchecked(
                    self.document,
                    self.revision,
                    start,
                    end,
                ));
            }
            if a.end.offset() <= b.end.offset() {
                left += 1;
            } else {
                right += 1;
            }
        }
        Self::new(self.document, self.revision, intersections)
    }

    pub fn subtraction(&self, other: &Self) -> Result<Self, PositionError> {
        self.validate_compatible(other)?;
        let mut result = Vec::new();
        let mut right_index = 0;
        for source in &self.segments {
            let mut cursor = source.start.offset();
            while right_index < other.segments.len()
                && other.segments[right_index].end.offset() <= cursor
            {
                right_index += 1;
            }
            let mut scan = right_index;
            while scan < other.segments.len()
                && other.segments[scan].start.offset() < source.end.offset()
            {
                let removed = other.segments[scan];
                if cursor < removed.start.offset() {
                    result.push(text_range_unchecked(
                        self.document,
                        self.revision,
                        cursor,
                        removed.start.offset().min(source.end.offset()),
                    ));
                }
                cursor = cursor.max(removed.end.offset());
                if cursor >= source.end.offset() {
                    break;
                }
                scan += 1;
            }
            if cursor < source.end.offset() {
                result.push(text_range_unchecked(
                    self.document,
                    self.revision,
                    cursor,
                    source.end.offset(),
                ));
            }
        }
        Self::new(self.document, self.revision, result)
    }

    fn validate_compatible(&self, other: &Self) -> Result<(), PositionError> {
        validate_document(self.document, other.document)?;
        validate_revision(self.revision, other.revision)
    }
}

/// Recovery applied when the content associated with an anchor is deleted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionRecovery {
    PreferFollowingThenPreceding,
    PreferPrecedingThenFollowing,
    Unresolvable,
}

/// The backing available to a persistent anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnchorBacking {
    /// A checked ordinal advanced only through explicit position maps.
    ///
    /// This is not a stable leaf identity and provides no provenance recovery.
    MappedSnapshotOrdinal,
    /// Stable adjacent projected identities, optionally accompanied by a
    /// source-provenance boundary, were captured from a document snapshot.
    StableProjectedIdentity { has_source_provenance: bool },
}

/// One side of a boundary in an immutable formatted-text allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectedLeafBoundary {
    leaf: FormattedLeafId,
    leaf_revision: FormattedLeafRevision,
    buffer: FormattedBufferId,
    buffer_byte: usize,
    side: LeafBoundarySide,
}

impl ProjectedLeafBoundary {
    pub(crate) fn new(
        leaf: FormattedLeafId,
        leaf_revision: FormattedLeafRevision,
        buffer: FormattedBufferId,
        buffer_byte: usize,
        side: LeafBoundarySide,
    ) -> Self {
        Self {
            leaf,
            leaf_revision,
            buffer,
            buffer_byte,
            side,
        }
    }

    pub fn leaf(self) -> FormattedLeafId {
        self.leaf
    }

    pub fn leaf_revision(self) -> FormattedLeafRevision {
        self.leaf_revision
    }

    pub fn buffer(self) -> FormattedBufferId {
        self.buffer
    }

    pub fn buffer_byte(self) -> usize {
        self.buffer_byte
    }

    pub fn side(self) -> LeafBoundarySide {
        self.side
    }
}

/// Stable logical-block context for an anchor boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectedBlockBoundary {
    block: u64,
    local_byte: usize,
    block_len: usize,
    content_fingerprint: u64,
}

impl ProjectedBlockBoundary {
    pub(crate) fn new(
        block: u64,
        local_byte: usize,
        block_len: usize,
        content_fingerprint: u64,
    ) -> Self {
        Self {
            block,
            local_byte,
            block_len,
            content_fingerprint,
        }
    }

    pub fn block(self) -> u64 {
        self.block
    }

    pub fn local_byte(self) -> usize {
        self.local_byte
    }

    pub fn block_len(self) -> usize {
        self.block_len
    }

    pub fn content_fingerprint(self) -> u64 {
        self.content_fingerprint
    }
}

/// Exact source boundary related to a formatted anchor at capture time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceAnchorProvenance {
    part: SourcePartId,
    revision: Revision,
    offset: usize,
    preceding_text: Option<u64>,
    following_text: Option<u64>,
}

impl SourceAnchorProvenance {
    pub(crate) fn new(
        part: SourcePartId,
        revision: Revision,
        offset: usize,
        preceding_text: Option<u64>,
        following_text: Option<u64>,
    ) -> Self {
        Self {
            part,
            revision,
            offset,
            preceding_text,
            following_text,
        }
    }

    pub fn part(self) -> SourcePartId {
        self.part
    }

    pub fn revision(self) -> Revision {
        self.revision
    }

    pub fn offset(self) -> usize {
        self.offset
    }

    pub fn preceding_text_fingerprint(self) -> Option<u64> {
        self.preceding_text
    }

    pub fn following_text_fingerprint(self) -> Option<u64> {
        self.following_text
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedAnchorBacking {
    pub(crate) preceding_leaf: Option<ProjectedLeafBoundary>,
    pub(crate) following_leaf: Option<ProjectedLeafBoundary>,
    pub(crate) block: Option<ProjectedBlockBoundary>,
    pub(crate) provenance: Option<SourceAnchorProvenance>,
}

/// Persistent formatted position with independent edit and visual policies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextAnchor {
    document: DocumentId,
    revision: Revision,
    offset: usize,
    association: Association,
    affinity: BoundaryAffinity,
    deletion_recovery: DeletionRecovery,
    backing: Option<ProjectedAnchorBacking>,
}

impl TextAnchor {
    pub fn new(
        point: TextPoint,
        association: Association,
        affinity: BoundaryAffinity,
        deletion_recovery: DeletionRecovery,
    ) -> Self {
        Self {
            document: point.document(),
            revision: point.revision(),
            offset: point.offset(),
            association,
            affinity,
            deletion_recovery,
            backing: None,
        }
    }

    pub(crate) fn with_backing(mut self, backing: ProjectedAnchorBacking) -> Self {
        self.backing = Some(backing);
        self
    }

    pub fn document(self) -> DocumentId {
        self.document
    }

    pub fn revision(self) -> Revision {
        self.revision
    }

    pub fn offset(self) -> usize {
        self.offset
    }

    pub fn association(self) -> Association {
        self.association
    }

    pub fn affinity(self) -> BoundaryAffinity {
        self.affinity
    }

    pub fn deletion_recovery(self) -> DeletionRecovery {
        self.deletion_recovery
    }

    pub fn backing(self) -> AnchorBacking {
        match self.backing {
            Some(backing) => AnchorBacking::StableProjectedIdentity {
                has_source_provenance: backing.provenance.is_some(),
            },
            None => AnchorBacking::MappedSnapshotOrdinal,
        }
    }

    pub fn projected_leaf_boundaries(
        self,
    ) -> Option<(Option<ProjectedLeafBoundary>, Option<ProjectedLeafBoundary>)> {
        self.backing
            .map(|backing| (backing.preceding_leaf, backing.following_leaf))
    }

    pub fn projected_block_boundary(self) -> Option<ProjectedBlockBoundary> {
        self.backing.and_then(|backing| backing.block)
    }

    pub fn source_provenance(self) -> Option<SourceAnchorProvenance> {
        self.backing.and_then(|backing| backing.provenance)
    }

    /// Resolve only in the anchor's exact current snapshot.
    pub fn resolve_exact(
        self,
        document: DocumentId,
        revision: Revision,
    ) -> Result<MappingOutcome<TextPoint>, PositionError> {
        validate_document(document, self.document)?;
        validate_revision(revision, self.revision)?;
        Ok(MappingOutcome::Exact(text_point_unchecked(
            self.document,
            self.revision,
            self.offset,
        )))
    }

    fn with_location(self, revision: Revision, offset: usize) -> Self {
        Self {
            revision,
            offset,
            ..self
        }
    }
}

/// Which endpoint remains active after normalizing a directed selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveEndpoint {
    Start,
    End,
    Collapsed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormalizedSelection {
    range: TextRange,
    active_endpoint: ActiveEndpoint,
}

impl NormalizedSelection {
    pub fn range(self) -> TextRange {
        self.range
    }

    pub fn active_endpoint(self) -> ActiveEndpoint {
        self.active_endpoint
    }
}

/// A direction-preserving pair of persistent selection anchors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectedSelection {
    anchor: TextAnchor,
    active: TextAnchor,
}

impl DirectedSelection {
    pub fn new(anchor: TextAnchor, active: TextAnchor) -> Result<Self, PositionError> {
        validate_document(anchor.document, active.document)?;
        validate_revision(anchor.revision, active.revision)?;
        Ok(Self { anchor, active })
    }

    /// Build the usual inward-associated selection from exact points.
    pub fn from_points(
        anchor: TextPoint,
        active: TextPoint,
        anchor_affinity: BoundaryAffinity,
        active_affinity: BoundaryAffinity,
    ) -> Result<Self, PositionError> {
        validate_text_pair(anchor, active)?;
        let (anchor_association, active_association) = match anchor.offset().cmp(&active.offset()) {
            Ordering::Less => (Association::AfterInsertion, Association::BeforeInsertion),
            Ordering::Greater => (Association::BeforeInsertion, Association::AfterInsertion),
            Ordering::Equal => (Association::AfterInsertion, Association::AfterInsertion),
        };
        Self::new(
            TextAnchor::new(
                anchor,
                anchor_association,
                anchor_affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            ),
            TextAnchor::new(
                active,
                active_association,
                active_affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            ),
        )
    }

    pub fn anchor(self) -> TextAnchor {
        self.anchor
    }

    pub fn active(self) -> TextAnchor {
        self.active
    }

    pub fn normalize(self) -> Result<NormalizedSelection, PositionError> {
        validate_document(self.anchor.document, self.active.document)?;
        validate_revision(self.anchor.revision, self.active.revision)?;
        let (start, end, active_endpoint) = match self.anchor.offset.cmp(&self.active.offset) {
            Ordering::Less => (self.anchor.offset, self.active.offset, ActiveEndpoint::End),
            Ordering::Greater => (
                self.active.offset,
                self.anchor.offset,
                ActiveEndpoint::Start,
            ),
            Ordering::Equal => (
                self.anchor.offset,
                self.active.offset,
                ActiveEndpoint::Collapsed,
            ),
        };
        Ok(NormalizedSelection {
            range: text_range_unchecked(self.anchor.document, self.anchor.revision, start, end),
            active_endpoint,
        })
    }
}

/// One replacement in the source coordinates of a position-map step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Splice {
    old: Range<usize>,
    inserted_len: usize,
}

impl Splice {
    pub fn new(old: Range<usize>, inserted_len: usize) -> Result<Self, PositionError> {
        if old.start > old.end {
            return Err(PositionError::InvertedRange {
                start: old.start,
                end: old.end,
            });
        }
        Ok(Self { old, inserted_len })
    }

    pub fn old_range(&self) -> Range<usize> {
        self.old.clone()
    }

    pub fn inserted_len(&self) -> usize {
        self.inserted_len
    }
}

/// Why an anchor could not be advanced through a map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnresolvableAnchor {
    DeletedContent {
        revision: Revision,
        range: Range<usize>,
    },
    /// Reserved for identity/provenance-backed anchors once the projection
    /// exposes those facilities.  Splice maps never synthesize provenance.
    MissingProvenance,
}

/// Structured result of mapping a persistent point or anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MappingOutcome<T> {
    Exact(T),
    Moved(T),
    CollapsedByDeletion(T),
    RecoveredFromProvenance(T),
    Ambiguous(Vec<T>),
    Unresolvable(UnresolvableAnchor),
}

impl<T> MappingOutcome<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Exact(value)
            | Self::Moved(value)
            | Self::CollapsedByDeletion(value)
            | Self::RecoveredFromProvenance(value) => Some(value),
            Self::Ambiguous(_) | Self::Unresolvable(_) => None,
        }
    }

    pub fn map<U>(self, convert: impl Fn(T) -> U) -> MappingOutcome<U> {
        match self {
            Self::Exact(value) => MappingOutcome::Exact(convert(value)),
            Self::Moved(value) => MappingOutcome::Moved(convert(value)),
            Self::CollapsedByDeletion(value) => MappingOutcome::CollapsedByDeletion(convert(value)),
            Self::RecoveredFromProvenance(value) => {
                MappingOutcome::RecoveredFromProvenance(convert(value))
            }
            Self::Ambiguous(values) => {
                MappingOutcome::Ambiguous(values.into_iter().map(convert).collect())
            }
            Self::Unresolvable(reason) => MappingOutcome::Unresolvable(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedStatus {
    Exact,
    Moved,
    CollapsedByDeletion,
    RecoveredFromProvenance,
}

impl ResolvedStatus {
    fn combine(self, other: Self) -> Self {
        use ResolvedStatus::{CollapsedByDeletion, Exact, Moved, RecoveredFromProvenance};
        match (self, other) {
            (CollapsedByDeletion, _) | (_, CollapsedByDeletion) => CollapsedByDeletion,
            (RecoveredFromProvenance, _) | (_, RecoveredFromProvenance) => RecoveredFromProvenance,
            (Moved, _) | (_, Moved) => Moved,
            (Exact, Exact) => Exact,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PositionMapStep {
    source_revision: Revision,
    target_revision: Revision,
    source_len: usize,
    target_len: usize,
    splices: Vec<Splice>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompiledTransition {
    ordinal: [CompiledOrdinalMap; 6],
    surviving_content: Vec<SurvivingContent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompiledOrdinalMap {
    pieces: Vec<OrdinalPiece>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OrdinalPiece {
    /// Inclusive source-boundary interval. Boundary positions are discrete, so
    /// an interval can represent a special splice endpoint without allocating
    /// one entry per byte.
    start: usize,
    end: usize,
    mapping: OrdinalMapping,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OrdinalMapping {
    Linear {
        delta: isize,
        status: ResolvedStatus,
    },
    Constant {
        target: usize,
        status: ResolvedStatus,
    },
    Unresolvable(UnresolvableAnchor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SurvivingContent {
    source: Range<usize>,
    target_start: usize,
}

/// A revision-tagged, composable monotonic position map.
///
/// Construction compiles splice steps into piecewise affine boundary maps and
/// surviving-content intervals. Composition combines those representations
/// directly instead of retaining an ever-growing vector of historical steps.
/// A lookup is therefore logarithmic in the compacted change boundaries and
/// independent of the number of revisions through which the map was composed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionMap {
    document: DocumentId,
    domain: PositionDomain,
    source_revision: Revision,
    target_revision: Revision,
    source_len: usize,
    target_len: usize,
    forward: CompiledTransition,
    reverse: CompiledTransition,
}

impl PositionMap {
    /// Heap held by the compacted map itself, without walking document text or
    /// any history chain. Optional presentation caches use this for eviction.
    pub(crate) fn owned_heap_bytes(&self) -> usize {
        [&self.forward, &self.reverse].into_iter().map(|transition| {
            transition.ordinal.iter().map(|map| map.pieces.capacity() * std::mem::size_of::<OrdinalPiece>()).sum::<usize>()
                + transition.surviving_content.capacity() * std::mem::size_of::<SurvivingContent>()
        }).sum()
    }

    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        for transition in [&self.forward, &self.reverse] {
            for ordinal in &transition.ordinal { visitor.vector(&ordinal.pieces, 0); }
            visitor.vector(&transition.surviving_content, 0);
        }
    }

    pub fn identity(
        document: DocumentId,
        domain: PositionDomain,
        revision: Revision,
        length: usize,
    ) -> Self {
        Self {
            document,
            domain,
            source_revision: revision,
            target_revision: revision,
            source_len: length,
            target_len: length,
            forward: CompiledTransition::identity(length),
            reverse: CompiledTransition::identity(length),
        }
    }

    /// Build a formatted-text map and validate all grapheme boundaries against
    /// the supplied immutable source and target snapshots.
    pub fn for_text(
        document: DocumentId,
        source_revision: Revision,
        target_revision: Revision,
        source_text: &str,
        target_text: &str,
        splices: Vec<Splice>,
    ) -> Result<Self, PositionError> {
        for splice in &splices {
            for offset in [splice.old.start, splice.old.end] {
                if offset > source_text.len() {
                    return Err(PositionError::InvalidBoundary {
                        domain: PositionDomain::FormattedText,
                        offset,
                        length: source_text.len(),
                    });
                }
                if !is_grapheme_boundary(source_text, offset) {
                    return Err(PositionError::InvalidUnicodeBoundary { offset });
                }
            }
        }
        let validation_splices = splices.clone();
        let map = Self::from_splices(
            document,
            PositionDomain::FormattedText,
            source_revision,
            target_revision,
            source_text.len(),
            target_text.len(),
            splices,
        )?;
        {
            let mut delta: isize = 0;
            for splice in &validation_splices {
                let target_start = add_signed(splice.old.start, delta)?;
                let target_end = target_start
                    .checked_add(splice.inserted_len)
                    .ok_or(PositionError::ArithmeticOverflow)?;
                for offset in [target_start, target_end] {
                    if !is_grapheme_boundary(target_text, offset) {
                        return Err(PositionError::InvalidUnicodeBoundary { offset });
                    }
                }
                delta = checked_delta(delta, splice)?;
            }
        }
        Ok(map)
    }

    /// Snapshot-backed counterpart of [`Self::for_text`]. A formatted
    /// projection can augment UAX #29 with forced boundaries around semantic
    /// items while retaining logarithmic, non-flattening navigation.
    pub(crate) fn for_text_snapshots<S, T>(
        document: DocumentId,
        source_revision: Revision,
        target_revision: Revision,
        source_text: &S,
        target_text: &T,
        splices: Vec<Splice>,
    ) -> Result<Self, PositionError>
    where
        S: LogicalGraphemeSnapshot,
        T: LogicalGraphemeSnapshot,
    {
        for splice in &splices {
            for offset in [splice.old.start, splice.old.end] {
                if offset > source_text.text_len() {
                    return Err(PositionError::InvalidBoundary {
                        domain: PositionDomain::FormattedText,
                        offset,
                        length: source_text.text_len(),
                    });
                }
                if !source_text
                    .is_logical_grapheme_boundary(offset)
                    .unwrap_or(false)
                {
                    return Err(PositionError::InvalidUnicodeBoundary { offset });
                }
            }
        }
        let validation_splices = splices.clone();
        let map = Self::from_splices(
            document,
            PositionDomain::FormattedText,
            source_revision,
            target_revision,
            source_text.text_len(),
            target_text.text_len(),
            splices,
        )?;
        {
            let mut delta: isize = 0;
            for splice in &validation_splices {
                let target_start = add_signed(splice.old.start, delta)?;
                let target_end = target_start
                    .checked_add(splice.inserted_len)
                    .ok_or(PositionError::ArithmeticOverflow)?;
                for offset in [target_start, target_end] {
                    if !target_text
                        .is_logical_grapheme_boundary(offset)
                        .unwrap_or(false)
                    {
                        return Err(PositionError::InvalidUnicodeBoundary { offset });
                    }
                }
                delta = checked_delta(delta, splice)?;
            }
        }
        Ok(map)
    }

    /// Build a byte-oriented map for one source-artifact part.
    pub fn for_source_part(
        document: DocumentId,
        part: SourcePartId,
        source_revision: Revision,
        target_revision: Revision,
        source_len: usize,
        target_len: usize,
        splices: Vec<Splice>,
    ) -> Result<Self, PositionError> {
        Self::from_splices(
            document,
            PositionDomain::Source(part),
            source_revision,
            target_revision,
            source_len,
            target_len,
            splices,
        )
    }

    fn from_splices(
        document: DocumentId,
        domain: PositionDomain,
        source_revision: Revision,
        target_revision: Revision,
        source_len: usize,
        target_len: usize,
        mut splices: Vec<Splice>,
    ) -> Result<Self, PositionError> {
        splices.sort_by_key(|splice| (splice.old.start, splice.old.end));
        for splice in &splices {
            if splice.old.end > source_len {
                return Err(PositionError::InvalidBoundary {
                    domain,
                    offset: splice.old.end,
                    length: source_len,
                });
            }
        }
        for pair in splices.windows(2) {
            if pair[0].old.end > pair[1].old.start {
                return Err(PositionError::OverlappingSplices {
                    first: pair[0].old.clone(),
                    second: pair[1].old.clone(),
                });
            }
        }
        let computed = computed_target_len(source_len, &splices)?;
        if computed != target_len {
            return Err(PositionError::TargetLengthMismatch {
                computed,
                actual: target_len,
            });
        }
        let step = PositionMapStep {
            source_revision,
            target_revision,
            source_len,
            target_len,
            splices,
        };
        let reverse_step = invert_step(&step)?;
        Ok(Self {
            document,
            domain,
            source_revision,
            target_revision,
            source_len,
            target_len,
            forward: CompiledTransition::from_step(&step)?,
            reverse: CompiledTransition::from_step(&reverse_step)?,
        })
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn domain(&self) -> PositionDomain {
        self.domain
    }

    pub fn source_revision(&self) -> Revision {
        self.source_revision
    }

    pub fn target_revision(&self) -> Revision {
        self.target_revision
    }

    pub fn source_len(&self) -> usize {
        self.source_len
    }

    pub fn target_len(&self) -> usize {
        self.target_len
    }

    #[cfg(test)]
    fn compiled_segment_count(&self) -> usize {
        self.forward
            .ordinal
            .iter()
            .map(|map| map.pieces.len())
            .sum::<usize>()
            + self.forward.surviving_content.len()
    }

    /// Compose `self` followed by `next`.
    pub fn then(&self, next: &Self) -> Result<Self, PositionError> {
        validate_document(self.document, next.document)?;
        validate_domain(self.domain, next.domain)?;
        validate_revision(self.target_revision, next.source_revision)?;
        if self.target_len != next.source_len {
            return Err(PositionError::MapChainLengthMismatch {
                expected: self.target_len,
                actual: next.source_len,
            });
        }
        Ok(Self {
            document: self.document,
            domain: self.domain,
            source_revision: self.source_revision,
            target_revision: next.target_revision,
            source_len: self.source_len,
            target_len: next.target_len,
            forward: self.forward.then(&next.forward)?,
            reverse: next.reverse.then(&self.reverse)?,
        })
    }

    /// Build the checked reverse traversal of this splice map.
    ///
    /// This reverses the structural transition, not every individual anchor
    /// outcome: content removed by a forward splice has no identity in the
    /// target snapshot, and content inserted by that splice is treated as
    /// removed while traversing back. Unchanged content, revision identities,
    /// insertion association, and deletion recovery continue to use the same
    /// step semantics in the opposite direction.
    pub fn inverted(&self) -> Result<Self, PositionError> {
        Ok(Self {
            document: self.document,
            domain: self.domain,
            source_revision: self.target_revision,
            target_revision: self.source_revision,
            source_len: self.target_len,
            target_len: self.source_len,
            forward: self.reverse.clone(),
            reverse: self.forward.clone(),
        })
    }

    pub fn map_text_anchor(
        &self,
        anchor: TextAnchor,
    ) -> Result<MappingOutcome<TextAnchor>, PositionError> {
        validate_domain(PositionDomain::FormattedText, self.domain)?;
        validate_document(self.document, anchor.document)?;
        validate_revision(self.source_revision, anchor.revision)?;
        if anchor.offset > self.source_len {
            return Err(PositionError::InvalidBoundary {
                domain: PositionDomain::FormattedText,
                offset: anchor.offset,
                length: self.source_len,
            });
        }
        self.map_ordinal(anchor.offset, anchor.association, anchor.deletion_recovery)
            .map(|outcome| outcome.map(|offset| anchor.with_location(self.target_revision, offset)))
    }

    pub fn map_text_point(
        &self,
        point: TextPoint,
        association: Association,
        affinity: BoundaryAffinity,
        deletion_recovery: DeletionRecovery,
    ) -> Result<MappingOutcome<TextPoint>, PositionError> {
        let anchor = TextAnchor::new(point, association, affinity, deletion_recovery);
        self.map_text_anchor(anchor).map(|outcome| {
            outcome
                .map(|anchor| text_point_unchecked(anchor.document, anchor.revision, anchor.offset))
        })
    }

    pub fn map_source_point(
        &self,
        point: SourcePoint,
        association: Association,
        deletion_recovery: DeletionRecovery,
    ) -> Result<MappingOutcome<SourcePoint>, PositionError> {
        let part = match self.domain {
            PositionDomain::Source(part) => part,
            actual => {
                return Err(PositionError::WrongDomain {
                    expected: PositionDomain::Source(point.part),
                    actual,
                });
            }
        };
        validate_document(self.document, point.document)?;
        validate_revision(self.source_revision, point.revision)?;
        if point.part != part {
            return Err(PositionError::WrongSourcePart {
                expected: part,
                actual: point.part,
            });
        }
        if point.offset > self.source_len {
            return Err(PositionError::InvalidBoundary {
                domain: self.domain,
                offset: point.offset,
                length: self.source_len,
            });
        }
        self.map_ordinal(point.offset, association, deletion_recovery)
            .map(|outcome| {
                outcome.map(|offset| SourcePoint {
                    document: point.document,
                    revision: self.target_revision,
                    part,
                    offset,
                })
            })
    }

    /// Map the identity of all original content in a range.
    ///
    /// Inserted/replacement content is not silently pulled into the mapped
    /// range.  An insertion inside the range therefore splits it, demonstrating
    /// why persistent range mapping is not equivalent to mapping two endpoints.
    pub fn map_text_range(
        &self,
        range: TextRange,
    ) -> Result<MappingOutcome<RangeSet>, PositionError> {
        validate_domain(PositionDomain::FormattedText, self.domain)?;
        validate_text_context(range.start, self.document, self.source_revision)?;
        validate_text_context(range.end, self.document, self.source_revision)?;

        let original_offsets: Vec<_> =
            std::iter::once(range.start.offset()..range.end.offset()).collect();
        let (offsets, deleted) = self
            .forward
            .map_content_range(range.start.offset()..range.end.offset())?;
        let ranges = offsets
            .iter()
            .map(|range| {
                text_range_unchecked(self.document, self.target_revision, range.start, range.end)
            })
            .collect::<Vec<_>>();
        let set = RangeSet::new(self.document, self.target_revision, ranges)?;
        if deleted {
            Ok(MappingOutcome::CollapsedByDeletion(set))
        } else if offsets == original_offsets {
            Ok(MappingOutcome::Exact(set))
        } else {
            Ok(MappingOutcome::Moved(set))
        }
    }

    fn map_ordinal(
        &self,
        offset: usize,
        association: Association,
        recovery: DeletionRecovery,
    ) -> Result<MappingOutcome<usize>, PositionError> {
        self.forward.map_ordinal(offset, association, recovery)
    }
}

impl CompiledTransition {
    fn identity(length: usize) -> Self {
        Self {
            ordinal: std::array::from_fn(|_| CompiledOrdinalMap::identity(length)),
            surviving_content: (!matches!(length, 0))
                .then_some(SurvivingContent {
                    source: 0..length,
                    target_start: 0,
                })
                .into_iter()
                .collect(),
        }
    }

    fn from_step(step: &PositionMapStep) -> Result<Self, PositionError> {
        let ordinal = std::array::from_fn(|index| {
            let (association, recovery) = ordinal_policy(index);
            CompiledOrdinalMap::from_step(step, association, recovery)
                .expect("validated splice arithmetic compiles for every anchor policy")
        });
        Ok(Self {
            ordinal,
            surviving_content: surviving_content_for_step(step)?,
        })
    }

    fn then(&self, next: &Self) -> Result<Self, PositionError> {
        let ordinal = [
            self.ordinal[0].then(&next.ordinal[0])?,
            self.ordinal[1].then(&next.ordinal[1])?,
            self.ordinal[2].then(&next.ordinal[2])?,
            self.ordinal[3].then(&next.ordinal[3])?,
            self.ordinal[4].then(&next.ordinal[4])?,
            self.ordinal[5].then(&next.ordinal[5])?,
        ];
        Ok(Self {
            ordinal,
            surviving_content: compose_surviving_content(
                &self.surviving_content,
                &next.surviving_content,
            )?,
        })
    }

    fn map_ordinal(
        &self,
        offset: usize,
        association: Association,
        recovery: DeletionRecovery,
    ) -> Result<MappingOutcome<usize>, PositionError> {
        self.ordinal[ordinal_policy_index(association, recovery)].map(offset)
    }

    fn map_content_range(
        &self,
        range: Range<usize>,
    ) -> Result<(Vec<Range<usize>>, bool), PositionError> {
        if range.is_empty() {
            return Ok((Vec::new(), false));
        }
        let mut mapped = Vec::new();
        let mut surviving_bytes = 0usize;
        for piece in &self.surviving_content {
            if piece.source.end <= range.start {
                continue;
            }
            if piece.source.start >= range.end {
                break;
            }
            let start = piece.source.start.max(range.start);
            let end = piece.source.end.min(range.end);
            if start >= end {
                continue;
            }
            let relative = start
                .checked_sub(piece.source.start)
                .ok_or(PositionError::ArithmeticOverflow)?;
            let target_start = piece
                .target_start
                .checked_add(relative)
                .ok_or(PositionError::ArithmeticOverflow)?;
            let length = end
                .checked_sub(start)
                .ok_or(PositionError::ArithmeticOverflow)?;
            mapped.push(
                target_start
                    ..target_start
                        .checked_add(length)
                        .ok_or(PositionError::ArithmeticOverflow)?,
            );
            surviving_bytes = surviving_bytes
                .checked_add(length)
                .ok_or(PositionError::ArithmeticOverflow)?;
        }
        Ok((
            normalize_offset_ranges(mapped),
            surviving_bytes != range.end - range.start,
        ))
    }
}

impl CompiledOrdinalMap {
    fn identity(length: usize) -> Self {
        Self {
            pieces: vec![OrdinalPiece {
                start: 0,
                end: length,
                mapping: OrdinalMapping::Linear {
                    delta: 0,
                    status: ResolvedStatus::Exact,
                },
            }],
        }
    }

    fn from_step(
        step: &PositionMapStep,
        association: Association,
        recovery: DeletionRecovery,
    ) -> Result<Self, PositionError> {
        let mut special = Vec::with_capacity(step.splices.len().saturating_mul(2) + 2);
        special.push(0);
        special.push(step.source_len);
        for splice in &step.splices {
            special.push(splice.old.start);
            special.push(splice.old.end);
        }
        special.sort_unstable();
        special.dedup();

        let mut pieces = Vec::with_capacity(special.len().saturating_mul(2));
        let mut evaluator = OrdinalStepCursor::new(step, association, recovery);
        let mut cursor = 0usize;
        for point in special {
            if cursor < point {
                pieces.push(compile_raw_interval(cursor, point - 1, &mut evaluator)?);
            }
            pieces.push(compile_raw_interval(point, point, &mut evaluator)?);
            cursor = point.saturating_add(1);
        }
        if cursor <= step.source_len {
            pieces.push(compile_raw_interval(
                cursor,
                step.source_len,
                &mut evaluator,
            )?);
        }
        Ok(Self {
            pieces: normalize_ordinal_pieces(pieces)?,
        })
    }

    fn then(&self, next: &Self) -> Result<Self, PositionError> {
        let mut output = Vec::new();
        for source in &self.pieces {
            match &source.mapping {
                OrdinalMapping::Unresolvable(reason) => output.push(OrdinalPiece {
                    start: source.start,
                    end: source.end,
                    mapping: OrdinalMapping::Unresolvable(reason.clone()),
                }),
                OrdinalMapping::Constant { target, status } => {
                    let mapped = next
                        .piece_at(*target)
                        .ok_or(PositionError::InvalidBoundary {
                            domain: PositionDomain::FormattedText,
                            offset: *target,
                            length: next.pieces.last().map_or(0, |piece| piece.end),
                        })?;
                    output.push(OrdinalPiece {
                        start: source.start,
                        end: source.end,
                        mapping: compose_constant_mapping(*target, *status, &mapped.mapping)?,
                    });
                }
                OrdinalMapping::Linear { delta, status } => {
                    let mapped_start = add_signed(source.start, *delta)?;
                    let mapped_end = add_signed(source.end, *delta)?;
                    for target in next.pieces_intersecting(mapped_start, mapped_end) {
                        let intersection_start = mapped_start.max(target.start);
                        let intersection_end = mapped_end.min(target.end);
                        let source_start = add_signed(intersection_start, delta.saturating_neg())?;
                        let source_end = add_signed(intersection_end, delta.saturating_neg())?;
                        output.push(OrdinalPiece {
                            start: source_start,
                            end: source_end,
                            mapping: compose_linear_mapping(*delta, *status, &target.mapping)?,
                        });
                    }
                }
            }
        }
        Ok(Self {
            pieces: normalize_ordinal_pieces(output)?,
        })
    }

    fn piece_at(&self, offset: usize) -> Option<&OrdinalPiece> {
        let index = self.pieces.partition_point(|piece| piece.end < offset);
        self.pieces.get(index).filter(|piece| piece.start <= offset)
    }

    fn pieces_intersecting(&self, start: usize, end: usize) -> impl Iterator<Item = &OrdinalPiece> {
        let first = self.pieces.partition_point(|piece| piece.end < start);
        self.pieces[first..]
            .iter()
            .take_while(move |piece| piece.start <= end)
    }

    fn map(&self, offset: usize) -> Result<MappingOutcome<usize>, PositionError> {
        let piece = self
            .piece_at(offset)
            .ok_or(PositionError::InvalidBoundary {
                domain: PositionDomain::FormattedText,
                offset,
                length: self.pieces.last().map_or(0, |piece| piece.end),
            })?;
        mapping_outcome_at(&piece.mapping, offset)
    }
}

fn ordinal_policy(index: usize) -> (Association, DeletionRecovery) {
    let association = if index < 3 {
        Association::BeforeInsertion
    } else {
        Association::AfterInsertion
    };
    let recovery = match index % 3 {
        0 => DeletionRecovery::PreferFollowingThenPreceding,
        1 => DeletionRecovery::PreferPrecedingThenFollowing,
        _ => DeletionRecovery::Unresolvable,
    };
    (association, recovery)
}

fn ordinal_policy_index(association: Association, recovery: DeletionRecovery) -> usize {
    let association = match association {
        Association::BeforeInsertion => 0,
        Association::AfterInsertion => 3,
    };
    association
        + match recovery {
            DeletionRecovery::PreferFollowingThenPreceding => 0,
            DeletionRecovery::PreferPrecedingThenFollowing => 1,
            DeletionRecovery::Unresolvable => 2,
        }
}

/// Evaluate the sorted compilation frontier once. Previously every interval
/// endpoint rescanned all earlier splices, making a format change with S
/// delimiter edits take O(S²) work for each of twelve policy/direction maps.
struct OrdinalStepCursor<'a> {
    step: &'a PositionMapStep,
    association: Association,
    recovery: DeletionRecovery,
    next: usize,
    delta: isize,
    #[cfg(test)]
    visited_splices: usize,
}

impl<'a> OrdinalStepCursor<'a> {
    fn new(
        step: &'a PositionMapStep,
        association: Association,
        recovery: DeletionRecovery,
    ) -> Self {
        Self {
            step,
            association,
            recovery,
            next: 0,
            delta: 0,
            #[cfg(test)]
            visited_splices: 0,
        }
    }

    fn map(&mut self, offset: usize) -> Result<MappingOutcome<usize>, PositionError> {
        while let Some(splice) = self
            .step
            .splices
            .get(self.next)
            .filter(|splice| splice.old.end <= offset && splice.old.start < offset)
        {
            self.delta = checked_delta(self.delta, splice)?;
            self.next += 1;
            #[cfg(test)]
            {
                self.visited_splices += 1;
            }
        }
        if let Some(splice) = self
            .step
            .splices
            .get(self.next)
            .filter(|splice| splice.old.start < offset && offset < splice.old.end)
        {
            if self.recovery == DeletionRecovery::Unresolvable {
                return Ok(MappingOutcome::Unresolvable(
                    UnresolvableAnchor::DeletedContent {
                        revision: self.step.source_revision,
                        range: splice.old.clone(),
                    },
                ));
            }
            let start = add_signed(splice.old.start, self.delta)?;
            return Ok(MappingOutcome::CollapsedByDeletion(recovery_target(
                self.recovery,
                splice,
                self.step,
                start,
            )));
        }
        let mut mapped = add_signed(offset, self.delta)?;
        if self.association == Association::AfterInsertion {
            for splice in self.step.splices[self.next..]
                .iter()
                .take_while(|splice| splice.old.start == offset)
            {
                mapped = mapped
                    .checked_add(splice.inserted_len)
                    .ok_or(PositionError::ArithmeticOverflow)?;
                #[cfg(test)]
                {
                    self.visited_splices += 1;
                }
            }
        }
        Ok(if mapped == offset {
            MappingOutcome::Exact(mapped)
        } else {
            MappingOutcome::Moved(mapped)
        })
    }
}

fn compile_raw_interval(
    start: usize,
    end: usize,
    evaluator: &mut OrdinalStepCursor<'_>,
) -> Result<OrdinalPiece, PositionError> {
    let first = evaluator.map(start)?;
    let last = evaluator.map(end)?;
    let mapping = mapping_for_interval(start, end, first, last)?;
    Ok(OrdinalPiece {
        start,
        end,
        mapping,
    })
}

fn mapping_for_interval(
    start: usize,
    end: usize,
    first: MappingOutcome<usize>,
    last: MappingOutcome<usize>,
) -> Result<OrdinalMapping, PositionError> {
    match (&first, &last) {
        (MappingOutcome::Unresolvable(first), MappingOutcome::Unresolvable(last))
            if first == last =>
        {
            Ok(OrdinalMapping::Unresolvable(first.clone()))
        }
        _ => {
            let (first, first_status) =
                resolved_outcome_parts(first).ok_or(PositionError::ArithmeticOverflow)?;
            let (last, last_status) =
                resolved_outcome_parts(last).ok_or(PositionError::ArithmeticOverflow)?;
            if first_status != last_status {
                return Err(PositionError::ArithmeticOverflow);
            }
            if start == end || last.checked_sub(first) == end.checked_sub(start) {
                Ok(OrdinalMapping::Linear {
                    delta: signed_difference(first, start)?,
                    status: first_status,
                })
            } else if first == last {
                Ok(OrdinalMapping::Constant {
                    target: first,
                    status: first_status,
                })
            } else {
                Err(PositionError::ArithmeticOverflow)
            }
        }
    }
}

fn resolved_outcome_parts(outcome: MappingOutcome<usize>) -> Option<(usize, ResolvedStatus)> {
    match outcome {
        MappingOutcome::Exact(target) => Some((target, ResolvedStatus::Exact)),
        MappingOutcome::Moved(target) => Some((target, ResolvedStatus::Moved)),
        MappingOutcome::CollapsedByDeletion(target) => {
            Some((target, ResolvedStatus::CollapsedByDeletion))
        }
        MappingOutcome::RecoveredFromProvenance(target) => {
            Some((target, ResolvedStatus::RecoveredFromProvenance))
        }
        MappingOutcome::Ambiguous(_) | MappingOutcome::Unresolvable(_) => None,
    }
}

fn compose_constant_mapping(
    intermediate: usize,
    prior_status: ResolvedStatus,
    next: &OrdinalMapping,
) -> Result<OrdinalMapping, PositionError> {
    match next {
        OrdinalMapping::Linear { delta, status } => Ok(OrdinalMapping::Constant {
            target: add_signed(intermediate, *delta)?,
            status: prior_status.combine(*status),
        }),
        OrdinalMapping::Constant { target, status } => Ok(OrdinalMapping::Constant {
            target: *target,
            status: prior_status.combine(*status),
        }),
        OrdinalMapping::Unresolvable(reason) => Ok(OrdinalMapping::Unresolvable(reason.clone())),
    }
}

fn compose_linear_mapping(
    prior_delta: isize,
    prior_status: ResolvedStatus,
    next: &OrdinalMapping,
) -> Result<OrdinalMapping, PositionError> {
    match next {
        OrdinalMapping::Linear { delta, status } => Ok(OrdinalMapping::Linear {
            delta: prior_delta
                .checked_add(*delta)
                .ok_or(PositionError::ArithmeticOverflow)?,
            status: prior_status.combine(*status),
        }),
        OrdinalMapping::Constant { target, status } => Ok(OrdinalMapping::Constant {
            target: *target,
            status: prior_status.combine(*status),
        }),
        OrdinalMapping::Unresolvable(reason) => Ok(OrdinalMapping::Unresolvable(reason.clone())),
    }
}

fn mapping_outcome_at(
    mapping: &OrdinalMapping,
    source: usize,
) -> Result<MappingOutcome<usize>, PositionError> {
    let (target, status) = match mapping {
        OrdinalMapping::Linear { delta, status } => (add_signed(source, *delta)?, *status),
        OrdinalMapping::Constant { target, status } => (*target, *status),
        OrdinalMapping::Unresolvable(reason) => {
            return Ok(MappingOutcome::Unresolvable(reason.clone()));
        }
    };
    Ok(match status {
        ResolvedStatus::Exact => MappingOutcome::Exact(target),
        ResolvedStatus::Moved => MappingOutcome::Moved(target),
        ResolvedStatus::CollapsedByDeletion => MappingOutcome::CollapsedByDeletion(target),
        ResolvedStatus::RecoveredFromProvenance => MappingOutcome::RecoveredFromProvenance(target),
    })
}

fn normalize_ordinal_pieces(pieces: Vec<OrdinalPiece>) -> Result<Vec<OrdinalPiece>, PositionError> {
    let mut normalized: Vec<OrdinalPiece> = Vec::with_capacity(pieces.len());
    for mut piece in pieces {
        if piece.start > piece.end {
            continue;
        }
        if let OrdinalMapping::Constant { target, status } = piece.mapping {
            if piece.start == piece.end {
                piece.mapping = OrdinalMapping::Linear {
                    delta: signed_difference(target, piece.start)?,
                    status,
                };
            } else {
                piece.mapping = OrdinalMapping::Constant { target, status };
            }
        }
        if let Some(previous) = normalized.last_mut() {
            let adjacent = previous
                .end
                .checked_add(1)
                .is_some_and(|next| next == piece.start);
            if adjacent && previous.mapping == piece.mapping {
                previous.end = piece.end;
                continue;
            }
        }
        normalized.push(piece);
    }
    Ok(normalized)
}

fn signed_difference(target: usize, source: usize) -> Result<isize, PositionError> {
    if target >= source {
        isize::try_from(target - source).map_err(|_| PositionError::ArithmeticOverflow)
    } else {
        isize::try_from(source - target)
            .ok()
            .and_then(isize::checked_neg)
            .ok_or(PositionError::ArithmeticOverflow)
    }
}

fn surviving_content_for_step(
    step: &PositionMapStep,
) -> Result<Vec<SurvivingContent>, PositionError> {
    let mut result = Vec::with_capacity(step.splices.len() + 1);
    let mut source_start = 0usize;
    let mut delta = 0isize;
    for splice in &step.splices {
        if source_start < splice.old.start {
            result.push(SurvivingContent {
                source: source_start..splice.old.start,
                target_start: add_signed(source_start, delta)?,
            });
        }
        source_start = splice.old.end;
        delta = checked_delta(delta, splice)?;
    }
    if source_start < step.source_len {
        result.push(SurvivingContent {
            source: source_start..step.source_len,
            target_start: add_signed(source_start, delta)?,
        });
    }
    Ok(result)
}

fn compose_surviving_content(
    first: &[SurvivingContent],
    second: &[SurvivingContent],
) -> Result<Vec<SurvivingContent>, PositionError> {
    let mut result = Vec::new();
    let mut second_index = 0usize;
    for first_piece in first {
        let first_target = first_piece.target_start
            ..first_piece
                .target_start
                .checked_add(first_piece.source.len())
                .ok_or(PositionError::ArithmeticOverflow)?;
        while second_index < second.len() && second[second_index].source.end <= first_target.start {
            second_index += 1;
        }
        let mut scan = second_index;
        while let Some(second_piece) = second.get(scan) {
            if second_piece.source.start >= first_target.end {
                break;
            }
            let middle_start = first_target.start.max(second_piece.source.start);
            let middle_end = first_target.end.min(second_piece.source.end);
            if middle_start < middle_end {
                let original_start = first_piece
                    .source
                    .start
                    .checked_add(middle_start - first_target.start)
                    .ok_or(PositionError::ArithmeticOverflow)?;
                let target_start = second_piece
                    .target_start
                    .checked_add(middle_start - second_piece.source.start)
                    .ok_or(PositionError::ArithmeticOverflow)?;
                result.push(SurvivingContent {
                    source: original_start
                        ..original_start
                            .checked_add(middle_end - middle_start)
                            .ok_or(PositionError::ArithmeticOverflow)?,
                    target_start,
                });
            }
            scan += 1;
        }
    }
    Ok(normalize_surviving_content(result))
}

fn normalize_surviving_content(pieces: Vec<SurvivingContent>) -> Vec<SurvivingContent> {
    let mut normalized: Vec<SurvivingContent> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        if let Some(previous) = normalized.last_mut() {
            let previous_target_end = previous.target_start + previous.source.len();
            if previous.source.end == piece.source.start
                && previous_target_end == piece.target_start
            {
                previous.source.end = piece.source.end;
                continue;
            }
        }
        normalized.push(piece);
    }
    normalized
}

fn invert_step(step: &PositionMapStep) -> Result<PositionMapStep, PositionError> {
    let mut splices = Vec::with_capacity(step.splices.len());
    let mut delta = 0isize;
    for splice in &step.splices {
        let target_start = add_signed(splice.old.start, delta)?;
        let target_end = target_start
            .checked_add(splice.inserted_len)
            .ok_or(PositionError::ArithmeticOverflow)?;
        splices.push(Splice {
            old: target_start..target_end,
            inserted_len: splice.old.end - splice.old.start,
        });
        delta = checked_delta(delta, splice)?;
    }
    let computed = computed_target_len(step.target_len, &splices)?;
    if computed != step.source_len {
        return Err(PositionError::TargetLengthMismatch {
            computed,
            actual: step.source_len,
        });
    }
    Ok(PositionMapStep {
        source_revision: step.target_revision,
        target_revision: step.source_revision,
        source_len: step.target_len,
        target_len: step.source_len,
        splices,
    })
}

#[cfg(test)]
fn map_ordinal_through_step(
    offset: usize,
    association: Association,
    recovery: DeletionRecovery,
    step: &PositionMapStep,
) -> Result<MappingOutcome<usize>, PositionError> {
    debug_assert!(offset <= step.source_len);
    let mut delta: isize = 0;
    let mut insertion_shift = 0usize;

    for splice in &step.splices {
        if splice.old.start < offset && offset < splice.old.end {
            if recovery == DeletionRecovery::Unresolvable {
                return Ok(MappingOutcome::Unresolvable(
                    UnresolvableAnchor::DeletedContent {
                        revision: step.source_revision,
                        range: splice.old.clone(),
                    },
                ));
            }
            let target_start = add_signed(splice.old.start, delta)?;
            let target = recovery_target(recovery, splice, step, target_start);
            return Ok(MappingOutcome::CollapsedByDeletion(target));
        }

        if splice.old.end <= offset && splice.old.start < offset {
            delta = checked_delta(delta, splice)?;
            continue;
        }

        if splice.old.start == offset {
            if association == Association::AfterInsertion {
                insertion_shift = insertion_shift
                    .checked_add(splice.inserted_len)
                    .ok_or(PositionError::ArithmeticOverflow)?;
            }
            // A non-empty replacement starts here.  Its removed extent is
            // after the boundary and therefore does not contribute to delta.
            continue;
        }

        if splice.old.start > offset {
            break;
        }
    }

    let base = add_signed(offset, delta)?;
    let mapped = base
        .checked_add(insertion_shift)
        .ok_or(PositionError::ArithmeticOverflow)?;
    if mapped == offset {
        Ok(MappingOutcome::Exact(mapped))
    } else {
        Ok(MappingOutcome::Moved(mapped))
    }
}

fn recovery_target(
    recovery: DeletionRecovery,
    splice: &Splice,
    step: &PositionMapStep,
    target_start: usize,
) -> usize {
    let following_exists = splice.old.end < step.source_len;
    let preceding_exists = splice.old.start > 0;
    match recovery {
        DeletionRecovery::PreferFollowingThenPreceding if following_exists => {
            target_start + splice.inserted_len
        }
        DeletionRecovery::PreferFollowingThenPreceding if preceding_exists => target_start,
        DeletionRecovery::PreferPrecedingThenFollowing if preceding_exists => target_start,
        DeletionRecovery::PreferPrecedingThenFollowing if following_exists => {
            target_start + splice.inserted_len
        }
        DeletionRecovery::PreferFollowingThenPreceding
        | DeletionRecovery::PreferPrecedingThenFollowing => match splice.inserted_len {
            0 => target_start,
            _ => target_start,
        },
        DeletionRecovery::Unresolvable => unreachable!("handled by caller"),
    }
}

fn normalize_offset_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|range| range.start < range.end);
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut normalized: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = normalized.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
                continue;
            }
        }
        normalized.push(range);
    }
    normalized
}

fn computed_target_len(source_len: usize, splices: &[Splice]) -> Result<usize, PositionError> {
    let mut length = source_len;
    for splice in splices.iter().rev() {
        length = length
            .checked_sub(splice.old.end - splice.old.start)
            .and_then(|value| value.checked_add(splice.inserted_len))
            .ok_or(PositionError::ArithmeticOverflow)?;
    }
    Ok(length)
}

fn checked_delta(delta: isize, splice: &Splice) -> Result<isize, PositionError> {
    let inserted =
        isize::try_from(splice.inserted_len).map_err(|_| PositionError::ArithmeticOverflow)?;
    let removed = isize::try_from(splice.old.end - splice.old.start)
        .map_err(|_| PositionError::ArithmeticOverflow)?;
    delta
        .checked_add(inserted)
        .and_then(|value| value.checked_sub(removed))
        .ok_or(PositionError::ArithmeticOverflow)
}

fn add_signed(value: usize, delta: isize) -> Result<usize, PositionError> {
    if delta >= 0 {
        value
            .checked_add(delta as usize)
            .ok_or(PositionError::ArithmeticOverflow)
    } else {
        value
            .checked_sub(delta.unsigned_abs())
            .ok_or(PositionError::ArithmeticOverflow)
    }
}

fn validate_document(expected: DocumentId, actual: DocumentId) -> Result<(), PositionError> {
    if expected == actual {
        Ok(())
    } else {
        Err(PositionError::WrongDocument { expected, actual })
    }
}

fn validate_revision(expected: Revision, actual: Revision) -> Result<(), PositionError> {
    if expected == actual {
        Ok(())
    } else {
        Err(PositionError::WrongSnapshot { expected, actual })
    }
}

fn validate_domain(expected: PositionDomain, actual: PositionDomain) -> Result<(), PositionError> {
    if expected == actual {
        Ok(())
    } else {
        Err(PositionError::WrongDomain { expected, actual })
    }
}

fn validate_text_context(
    point: TextPoint,
    document: DocumentId,
    revision: Revision,
) -> Result<(), PositionError> {
    validate_document(document, point.document())?;
    validate_revision(revision, point.revision())
}

fn validate_text_pair(first: TextPoint, second: TextPoint) -> Result<(), PositionError> {
    validate_document(first.document(), second.document())?;
    validate_revision(first.revision(), second.revision())
}

fn validate_source_pair(first: SourcePoint, second: SourcePoint) -> Result<(), PositionError> {
    validate_document(first.document, second.document)?;
    validate_revision(first.revision, second.revision)?;
    if first.part == second.part {
        Ok(())
    } else {
        Err(PositionError::WrongSourcePart {
            expected: first.part,
            actual: second.part,
        })
    }
}

fn text_point_unchecked(document: DocumentId, revision: Revision, offset: usize) -> TextPoint {
    TextPoint {
        document,
        revision,
        offset,
    }
}

fn text_range_unchecked(
    document: DocumentId,
    revision: Revision,
    start: usize,
    end: usize,
) -> TextRange {
    TextRange {
        start: text_point_unchecked(document, revision, start),
        end: text_point_unchecked(document, revision, end),
    }
}

fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn points(document: &Document, start: usize, end: usize) -> TextRange {
        TextRange::new(
            document.text_point(start).unwrap(),
            document.text_point(end).unwrap(),
        )
        .unwrap()
    }

    fn offsets(set: &RangeSet) -> Vec<Range<usize>> {
        set.segments
            .iter()
            .map(|range| range.start.offset()..range.end.offset())
            .collect()
    }

    fn outcome_offset(outcome: &MappingOutcome<TextAnchor>) -> Option<usize> {
        outcome.value().map(|anchor| anchor.offset())
    }

    fn outcome_rank<T>(outcome: &MappingOutcome<T>) -> u8 {
        match outcome {
            MappingOutcome::Exact(_) => 0,
            MappingOutcome::Moved(_) => 1,
            MappingOutcome::CollapsedByDeletion(_) => 2,
            MappingOutcome::RecoveredFromProvenance(_) => 3,
            MappingOutcome::Ambiguous(_) => 4,
            MappingOutcome::Unresolvable(_) => 5,
        }
    }

    #[test]
    fn splice_compilation_sweep_matches_reference_for_all_anchor_policies() {
        let cases = [
            vec![],
            vec![
                Splice::new(0..0, 3).unwrap(),
                Splice::new(0..3, 2).unwrap(),
                Splice::new(3..3, 4).unwrap(),
                Splice::new(5..8, 0).unwrap(),
                Splice::new(8..8, 2).unwrap(),
            ],
            vec![Splice::new(0..12, 0).unwrap()],
            vec![
                Splice::new(2..5, 2).unwrap(),
                Splice::new(5..8, 0).unwrap(),
                Splice::new(8..12, 4).unwrap(),
            ],
            vec![
                Splice::new(3..3, 1).unwrap(),
                Splice::new(3..3, 2).unwrap(),
                Splice::new(12..12, 4).unwrap(),
            ],
        ];
        for splices in cases {
            let step = PositionMapStep {
                source_revision: Revision(1),
                target_revision: Revision(2),
                source_len: 12,
                target_len: computed_target_len(12, &splices).unwrap(),
                splices,
            };
            for step in [&step, &invert_step(&step).unwrap()] {
                for policy in 0..6 {
                    let (association, recovery) = ordinal_policy(policy);
                    let compiled =
                        CompiledOrdinalMap::from_step(step, association, recovery).unwrap();
                    let mut cursor = OrdinalStepCursor::new(step, association, recovery);
                    for at in 0..=step.source_len {
                        let expected =
                            map_ordinal_through_step(at, association, recovery, step).unwrap();
                        assert_eq!(cursor.map(at).unwrap(), expected);
                        assert_eq!(cursor.map(at).unwrap(), expected, "repeated boundary");
                        assert_eq!(compiled.map(at).unwrap(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn compiling_many_disjoint_splices_visits_each_frontier_only_once() {
        let count = 20_000;
        let splices = (0..count)
            .map(|index| Splice::new(index * 4 + 1..index * 4 + 3, 1).unwrap())
            .collect::<Vec<_>>();
        let step = PositionMapStep {
            source_revision: Revision(1),
            target_revision: Revision(2),
            source_len: count * 4,
            target_len: count * 3,
            splices,
        };
        for policy in 0..6 {
            let (association, recovery) = ordinal_policy(policy);
            let mut cursor = OrdinalStepCursor::new(&step, association, recovery);
            for at in 0..=step.source_len {
                cursor.map(at).unwrap();
            }
            assert!(cursor.visited_splices <= count * 2);
            let map = CompiledOrdinalMap::from_step(&step, association, recovery).unwrap();
            assert!(map.pieces.len() <= count * 4 + 1);
        }
    }

    #[test]
    fn source_points_are_part_and_snapshot_bound() {
        let document = DocumentId(7);
        let point = SourcePoint::new(document, Revision(2), SourcePartId::PRIMARY, 4, 4).unwrap();
        assert_eq!(point.offset(), 4);
        assert_eq!(point.part(), SourcePartId::PRIMARY);
        assert_eq!(
            SourcePoint::new(document, Revision(2), SourcePartId::PRIMARY, 5, 4),
            Err(PositionError::InvalidBoundary {
                domain: PositionDomain::Source(SourcePartId::PRIMARY),
                offset: 5,
                length: 4,
            })
        );
        let other_part = SourcePoint::new(document, Revision(2), SourcePartId(1), 4, 4).unwrap();
        assert!(matches!(
            SourceRange::new(point, other_part),
            Err(PositionError::WrongSourcePart { .. })
        ));
    }

    #[test]
    fn text_ranges_reject_mixed_snapshots_documents_and_inversion() {
        let mut first = Document::new("abc");
        let other = Document::new("abc");
        let old_start = first.text_point(0).unwrap();
        let old_end = first.text_point(2).unwrap();
        assert_eq!(
            TextRange::new(old_end, old_start),
            Err(PositionError::InvertedRange { start: 2, end: 0 })
        );
        assert!(matches!(
            TextRange::new(old_start, other.text_point(2).unwrap()),
            Err(PositionError::WrongDocument { .. })
        ));
        first.insert(0, "x").unwrap();
        assert!(matches!(
            TextRange::new(old_start, first.text_point(2).unwrap()),
            Err(PositionError::WrongSnapshot { .. })
        ));
    }

    #[test]
    fn text_points_are_created_only_at_grapheme_boundaries() {
        let document = Document::new("a\u{301}b");
        assert!(document.text_point(0).is_ok());
        assert!(document.text_point("a\u{301}".len()).is_ok());
        assert!(document.text_point(1).is_err());
    }

    #[test]
    fn directed_selection_normalization_preserves_active_end() {
        let document = Document::new("abcd");
        let start = document.text_point(1).unwrap();
        let end = document.text_point(3).unwrap();
        let forward = DirectedSelection::from_points(
            start,
            end,
            BoundaryAffinity::Downstream,
            BoundaryAffinity::Upstream,
        )
        .unwrap();
        let backward = DirectedSelection::from_points(
            end,
            start,
            BoundaryAffinity::Upstream,
            BoundaryAffinity::Downstream,
        )
        .unwrap();
        assert_eq!(
            forward.normalize().unwrap().range(),
            points(&document, 1, 3)
        );
        assert_eq!(
            forward.normalize().unwrap().active_endpoint(),
            ActiveEndpoint::End
        );
        assert_eq!(
            backward.normalize().unwrap().range(),
            points(&document, 1, 3)
        );
        assert_eq!(
            backward.normalize().unwrap().active_endpoint(),
            ActiveEndpoint::Start
        );
        assert_eq!(forward.anchor().association(), Association::AfterInsertion);
        assert_eq!(forward.active().association(), Association::BeforeInsertion);
        assert_eq!(
            backward.anchor().association(),
            Association::BeforeInsertion
        );
        assert_eq!(backward.active().association(), Association::AfterInsertion);
    }

    #[test]
    fn range_set_normalizes_and_can_preserve_observable_adjacency() {
        let document = Document::new("abcdefghij");
        let coalesced = RangeSet::new(
            document.id(),
            document.revision(),
            [
                points(&document, 5, 8),
                points(&document, 1, 3),
                points(&document, 3, 6),
            ],
        )
        .unwrap();
        assert_eq!(offsets(&coalesced), vec![1..8]);

        let preserved = RangeSet::preserving_adjacent(
            document.id(),
            document.revision(),
            [points(&document, 1, 3), points(&document, 3, 5)],
        )
        .unwrap();
        assert_eq!(offsets(&preserved), vec![1..3, 3..5]);
    }

    #[test]
    fn range_set_union_intersection_and_subtraction_are_half_open() {
        let document = Document::new("0123456789");
        let left = RangeSet::new(
            document.id(),
            document.revision(),
            [points(&document, 1, 4), points(&document, 6, 9)],
        )
        .unwrap();
        let right = RangeSet::new(
            document.id(),
            document.revision(),
            [points(&document, 3, 7)],
        )
        .unwrap();
        assert_eq!(offsets(&left.union(&right).unwrap()), vec![1..9]);
        assert_eq!(
            offsets(&left.intersection(&right).unwrap()),
            vec![3..4, 6..7]
        );
        assert_eq!(
            offsets(&left.subtraction(&right).unwrap()),
            vec![1..3, 7..9]
        );
        assert!(!left.contains(document.text_point(4).unwrap()).unwrap());
        assert!(left.contains(document.text_point(6).unwrap()).unwrap());
    }

    #[test]
    fn range_set_algebra_matches_a_bitmap_model_exhaustively() {
        const WIDTH: usize = 8;
        let document = Document::new("abcdefgh");
        for left_bits in 0u16..(1 << WIDTH) {
            for right_bits in 0u16..(1 << WIDTH) {
                let left = bitmap_set(&document, left_bits, WIDTH);
                let right = bitmap_set(&document, right_bits, WIDTH);
                assert_eq!(
                    set_bitmap(&left.union(&right).unwrap(), WIDTH),
                    left_bits | right_bits
                );
                assert_eq!(
                    set_bitmap(&left.intersection(&right).unwrap(), WIDTH),
                    left_bits & right_bits
                );
                assert_eq!(
                    set_bitmap(&left.subtraction(&right).unwrap(), WIDTH),
                    left_bits & !right_bits & ((1 << WIDTH) - 1)
                );
            }
        }
    }

    #[test]
    fn insertion_association_is_independent_of_boundary_affinity() {
        let document = Document::new("ab");
        let map = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "ab",
            "aXYb",
            vec![Splice::new(1..1, 2).unwrap()],
        )
        .unwrap();
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            let before = TextAnchor::new(
                document.text_point(1).unwrap(),
                Association::BeforeInsertion,
                affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            );
            let after = TextAnchor::new(
                document.text_point(1).unwrap(),
                Association::AfterInsertion,
                affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            );
            assert_eq!(
                outcome_offset(&map.map_text_anchor(before).unwrap()),
                Some(1)
            );
            assert_eq!(
                outcome_offset(&map.map_text_anchor(after).unwrap()),
                Some(3)
            );
            assert_eq!(
                map.map_text_anchor(before)
                    .unwrap()
                    .value()
                    .unwrap()
                    .affinity(),
                affinity
            );
            assert_eq!(
                map.map_text_anchor(after)
                    .unwrap()
                    .value()
                    .unwrap()
                    .affinity(),
                affinity
            );
        }
    }

    #[test]
    fn deleted_anchor_uses_declared_recovery_or_fails() {
        let document = Document::new("abcdef");
        let map = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abcdef",
            "abXYef",
            vec![Splice::new(2..4, 2).unwrap()],
        )
        .unwrap();
        let point = document.text_point(3).unwrap();
        let following = TextAnchor::new(
            point,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );
        let preceding = TextAnchor::new(
            point,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferPrecedingThenFollowing,
        );
        let failing = TextAnchor::new(
            point,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::Unresolvable,
        );
        assert_eq!(
            map.map_text_anchor(following).unwrap(),
            MappingOutcome::CollapsedByDeletion(following.with_location(Revision(1), 4))
        );
        assert_eq!(
            map.map_text_anchor(preceding).unwrap(),
            MappingOutcome::CollapsedByDeletion(preceding.with_location(Revision(1), 2))
        );
        assert!(matches!(
            map.map_text_anchor(failing).unwrap(),
            MappingOutcome::Unresolvable(UnresolvableAnchor::DeletedContent { .. })
        ));
    }

    #[test]
    fn mapping_a_range_excludes_inserted_content_and_returns_a_range_set() {
        let document = Document::new("abcd");
        let map = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abcd",
            "abXYcd",
            vec![Splice::new(2..2, 2).unwrap()],
        )
        .unwrap();
        let mapped = map.map_text_range(points(&document, 1, 3)).unwrap();
        let set = mapped.value().unwrap();
        assert_eq!(offsets(set), vec![1..2, 4..5]);
        assert!(matches!(mapped, MappingOutcome::Moved(_)));
    }

    #[test]
    fn text_map_validates_source_and_target_grapheme_boundaries() {
        let document = DocumentId(9);
        assert_eq!(
            PositionMap::for_text(
                document,
                Revision(0),
                Revision(1),
                "aé",
                "aXé",
                vec![Splice::new(2..2, 1).unwrap()],
            ),
            Err(PositionError::InvalidUnicodeBoundary { offset: 2 })
        );
        assert_eq!(
            PositionMap::for_text(
                document,
                Revision(0),
                Revision(1),
                "aé",
                "aXé?",
                vec![Splice::new(1..1, 2).unwrap()],
            ),
            Err(PositionError::InvalidUnicodeBoundary { offset: 3 })
        );
    }

    #[test]
    fn map_rejects_wrong_document_domain_and_snapshot() {
        let document = Document::new("abc");
        let other = Document::new("abc");
        let text_map = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abc",
            "xabc",
            vec![Splice::new(0..0, 1).unwrap()],
        )
        .unwrap();
        let anchor = TextAnchor::new(
            other.text_point(0).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );
        assert!(matches!(
            text_map.map_text_anchor(anchor),
            Err(PositionError::WrongDocument { .. })
        ));

        let source_map = PositionMap::for_source_part(
            document.id(),
            SourcePartId::PRIMARY,
            Revision(0),
            Revision(1),
            3,
            4,
            vec![Splice::new(0..0, 1).unwrap()],
        )
        .unwrap();
        let text_anchor = TextAnchor::new(
            document.text_point(0).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );
        assert!(matches!(
            source_map.map_text_anchor(text_anchor),
            Err(PositionError::WrongDomain { .. })
        ));

        let stale = text_anchor.with_location(Revision(8), 0);
        assert!(matches!(
            text_map.map_text_anchor(stale),
            Err(PositionError::WrongSnapshot { .. })
        ));
    }

    #[test]
    fn source_position_maps_remain_nominally_separate() {
        let map = PositionMap::for_source_part(
            DocumentId(1),
            SourcePartId(4),
            Revision(10),
            Revision(11),
            5,
            7,
            vec![Splice::new(2..2, 2).unwrap()],
        )
        .unwrap();
        let point = SourcePoint::new(DocumentId(1), Revision(10), SourcePartId(4), 2, 5).unwrap();
        assert_eq!(
            map.map_source_point(
                point,
                Association::AfterInsertion,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap(),
            MappingOutcome::Moved(
                SourcePoint::new(DocumentId(1), Revision(11), SourcePartId(4), 4, 7,).unwrap()
            )
        );
    }

    #[test]
    fn composition_is_associative_and_retains_collapse_status() {
        let document = Document::new("abcdef");
        let first = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abcdef",
            "aXXbcdef",
            vec![Splice::new(1..1, 2).unwrap()],
        )
        .unwrap();
        let second = PositionMap::for_text(
            document.id(),
            Revision(1),
            Revision(2),
            "aXXbcdef",
            "aXXbef",
            vec![Splice::new(4..6, 0).unwrap()],
        )
        .unwrap();
        let third = PositionMap::for_text(
            document.id(),
            Revision(2),
            Revision(3),
            "aXXbef",
            "QaXXbef",
            vec![Splice::new(0..0, 1).unwrap()],
        )
        .unwrap();
        let left = first.then(&second).unwrap().then(&third).unwrap();
        let right = first.then(&second.then(&third).unwrap()).unwrap();
        assert_eq!(left, right);

        for offset in 0..=document.text().len() {
            for association in [Association::BeforeInsertion, Association::AfterInsertion] {
                for recovery in [
                    DeletionRecovery::PreferFollowingThenPreceding,
                    DeletionRecovery::PreferPrecedingThenFollowing,
                    DeletionRecovery::Unresolvable,
                ] {
                    let anchor = TextAnchor::new(
                        document.text_point(offset).unwrap(),
                        association,
                        BoundaryAffinity::Downstream,
                        recovery,
                    );
                    assert_eq!(
                        left.map_text_anchor(anchor).unwrap(),
                        right.map_text_anchor(anchor).unwrap()
                    );
                }
            }
        }
        let deleted = TextAnchor::new(
            document.text_point(3).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        );
        assert_eq!(outcome_rank(&left.map_text_anchor(deleted).unwrap()), 2);
    }

    #[test]
    fn long_composition_chain_is_compacted_and_associative() {
        fn balanced(maps: &[PositionMap]) -> PositionMap {
            if maps.len() == 1 {
                return maps[0].clone();
            }
            let middle = maps.len() / 2;
            balanced(&maps[..middle])
                .then(&balanced(&maps[middle..]))
                .unwrap()
        }

        let document = Document::new("abc");
        let mut text = "abc".to_owned();
        let mut maps = Vec::new();
        for revision in 0..512u64 {
            let target = format!("x{text}");
            maps.push(
                PositionMap::for_text(
                    document.id(),
                    Revision(revision),
                    Revision(revision + 1),
                    &text,
                    &target,
                    vec![Splice::new(0..0, 1).unwrap()],
                )
                .unwrap(),
            );
            text = target;
        }
        let left = maps
            .iter()
            .skip(1)
            .try_fold(maps[0].clone(), |map, next| map.then(next))
            .unwrap();
        let grouped = balanced(&maps);

        // The representation grows with distinct semantic change boundaries,
        // not with the 512 historical revisions. Repeated edits at one boundary
        // stay a small compiled map.
        assert!(left.compiled_segment_count() <= 24);
        assert!(grouped.compiled_segment_count() <= 24);
        for offset in 0..=3 {
            for association in [Association::BeforeInsertion, Association::AfterInsertion] {
                for recovery in [
                    DeletionRecovery::PreferFollowingThenPreceding,
                    DeletionRecovery::PreferPrecedingThenFollowing,
                    DeletionRecovery::Unresolvable,
                ] {
                    let anchor = TextAnchor::new(
                        document.text_point(offset).unwrap(),
                        association,
                        BoundaryAffinity::Downstream,
                        recovery,
                    );
                    assert_eq!(
                        left.map_text_anchor(anchor).unwrap(),
                        grouped.map_text_anchor(anchor).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn composed_mapping_matches_stepwise_offsets_for_many_anchors() {
        let document = Document::new("abcdefgh");
        let first = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "abcdefgh",
            "abXXcdefgh",
            vec![Splice::new(2..2, 2).unwrap()],
        )
        .unwrap();
        let second = PositionMap::for_text(
            document.id(),
            Revision(1),
            Revision(2),
            "abXXcdefgh",
            "abXXcdeQh",
            vec![Splice::new(7..9, 1).unwrap()],
        )
        .unwrap();
        let composed = first.then(&second).unwrap();
        for offset in 0..=8 {
            for association in [Association::BeforeInsertion, Association::AfterInsertion] {
                let anchor = TextAnchor::new(
                    document.text_point(offset).unwrap(),
                    association,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                );
                let direct = composed.map_text_anchor(anchor).unwrap();
                let first_result = first.map_text_anchor(anchor).unwrap();
                if let Some(intermediate) = first_result.value() {
                    let stepwise = second.map_text_anchor(*intermediate).unwrap();
                    assert_eq!(outcome_offset(&direct), outcome_offset(&stepwise));
                }
            }
        }
    }

    #[test]
    fn inversion_reverses_discontiguous_steps_without_losing_the_middle() {
        let document = Document::new("aa MID zz");
        let forward = PositionMap::for_text(
            document.id(),
            Revision(0),
            Revision(1),
            "aa MID zz",
            "alpha MID !",
            vec![
                Splice::new(0..2, "alpha".len()).unwrap(),
                Splice::new(7..9, 1).unwrap(),
            ],
        )
        .unwrap();
        let reverse = forward.inverted().unwrap();
        assert_eq!(reverse.source_revision(), Revision(1));
        assert_eq!(reverse.target_revision(), Revision(0));
        assert_eq!(reverse.source_len(), "alpha MID !".len());
        assert_eq!(reverse.target_len(), "aa MID zz".len());

        let middle = TextRange::new(
            text_point_unchecked(document.id(), Revision(1), 6),
            text_point_unchecked(document.id(), Revision(1), 9),
        )
        .unwrap();
        let MappingOutcome::Moved(mapped) = reverse.map_text_range(middle).unwrap() else {
            panic!("unchanged middle content should retain identity through inversion");
        };
        assert_eq!(offsets(&mapped), vec![3..6]);

        let inserted = TextAnchor::new(
            text_point_unchecked(document.id(), Revision(1), 2),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::Unresolvable,
        );
        assert!(matches!(
            reverse.map_text_anchor(inserted).unwrap(),
            MappingOutcome::Unresolvable(UnresolvableAnchor::DeletedContent { .. })
        ));
    }

    #[test]
    fn inversion_preserves_revision_changing_identity_steps() {
        let forward = PositionMap::for_text(
            DocumentId(9),
            Revision(12),
            Revision(13),
            "same",
            "same",
            Vec::new(),
        )
        .unwrap();
        let reverse = forward.inverted().unwrap();
        assert_eq!(reverse.source_revision(), Revision(13));
        assert_eq!(reverse.target_revision(), Revision(12));
        assert_eq!(reverse.source_len(), 4);
        assert_eq!(reverse.target_len(), 4);
    }

    #[test]
    fn map_composition_reports_all_mismatches() {
        let a = PositionMap::identity(DocumentId(1), PositionDomain::FormattedText, Revision(0), 3);
        let wrong_document =
            PositionMap::identity(DocumentId(2), PositionDomain::FormattedText, Revision(0), 3);
        assert!(matches!(
            a.then(&wrong_document),
            Err(PositionError::WrongDocument { .. })
        ));

        let wrong_domain = PositionMap::identity(
            DocumentId(1),
            PositionDomain::Source(SourcePartId::PRIMARY),
            Revision(0),
            3,
        );
        assert!(matches!(
            a.then(&wrong_domain),
            Err(PositionError::WrongDomain { .. })
        ));

        let wrong_revision =
            PositionMap::identity(DocumentId(1), PositionDomain::FormattedText, Revision(4), 3);
        assert!(matches!(
            a.then(&wrong_revision),
            Err(PositionError::WrongSnapshot { .. })
        ));

        let wrong_length =
            PositionMap::identity(DocumentId(1), PositionDomain::FormattedText, Revision(0), 4);
        assert!(matches!(
            a.then(&wrong_length),
            Err(PositionError::MapChainLengthMismatch { .. })
        ));
    }

    fn bitmap_set(document: &Document, bits: u16, width: usize) -> RangeSet {
        let mut ranges = Vec::new();
        let mut index = 0;
        while index < width {
            if bits & (1 << index) == 0 {
                index += 1;
                continue;
            }
            let start = index;
            while index < width && bits & (1 << index) != 0 {
                index += 1;
            }
            ranges.push(points(document, start, index));
        }
        RangeSet::new(document.id(), document.revision(), ranges).unwrap()
    }

    fn set_bitmap(set: &RangeSet, width: usize) -> u16 {
        let mut bits = 0;
        for range in &set.segments {
            for index in range.start.offset()..range.end.offset().min(width) {
                bits |= 1 << index;
            }
        }
        bits
    }
}
