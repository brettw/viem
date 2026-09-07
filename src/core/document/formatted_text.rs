//! Persistent formatted UTF-8 storage.
//!
//! [`FormattedTextTree`] is the canonical text container used by a formatted
//! projection.  It is an immutable AVL rope: cloning is constant time and a
//! splice rebuilds only the paths to its boundary leaves.  Leaves are bounded
//! byte slices into immutable, shared UTF-8 buffers, so untouched leaves retain
//! both their identity and their backing allocation across revisions.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// Target maximum size of a formatted-text leaf.
///
/// A leaf may be smaller after a splice. UTF-8 scalar boundaries are never
/// split, and a single scalar is at most four bytes, so initial construction
/// can always honor this bound.
pub const FORMATTED_TEXT_LEAF_BYTES: usize = 4 * 1024;

static NEXT_LEAF_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_BUFFER_ID: AtomicU64 = AtomicU64::new(1);

/// Stable identity of one immutable UTF-8 allocation retained by rope leaves.
///
/// A splice may split one leaf into several slices with different leaf
/// identities. The allocation identity plus an allocation-local boundary lets
/// a persistent anchor continue to identify the original adjacent content
/// across that split without retaining a document-wide ordinal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FormattedBufferId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FormattedLeafId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FormattedLeafRevision(pub u64);

/// Which adjacent leaf should represent a boundary shared by two leaves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeafBoundarySide {
    Preceding,
    Following,
}

/// Stable leaf information for a byte boundary in one immutable tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedLeafLocation {
    pub id: FormattedLeafId,
    pub revision: FormattedLeafRevision,
    pub buffer_id: FormattedBufferId,
    pub leaf_range: Range<usize>,
    pub local_byte: usize,
    /// Boundary in the immutable backing allocation's coordinate space.
    pub buffer_byte: usize,
}

/// A compact inventory entry useful for cache keys and incremental consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedLeafInfo {
    pub id: FormattedLeafId,
    pub revision: FormattedLeafRevision,
    pub byte_range: Range<usize>,
    pub hard_line_breaks: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormattedTextError {
    InvalidByteOffset {
        offset: usize,
        length: usize,
    },
    InvalidRange {
        start: usize,
        end: usize,
        length: usize,
    },
    OverlappingSplices {
        first: Range<usize>,
        second: Range<usize>,
    },
    NotCharBoundary(usize),
    InvalidUtf16Offset {
        offset: usize,
        length: usize,
    },
    NotUtf16Boundary(usize),
    NotGraphemeBoundary(usize),
    HardLineOutOfBounds {
        line: usize,
        line_count: usize,
    },
    ArithmeticOverflow,
    LeafIdentityExhausted,
    UnicodeBoundaryResolutionFailed,
    ResultTextMismatch,
}

impl fmt::Display for FormattedTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteOffset { offset, length } => {
                write!(
                    formatter,
                    "byte offset {offset} exceeds text length {length}"
                )
            }
            Self::InvalidRange { start, end, length } => write!(
                formatter,
                "invalid formatted byte range {start}..{end} for length {length}"
            ),
            Self::OverlappingSplices { first, second } => write!(
                formatter,
                "formatted splice {}..{} overlaps {}..{}",
                first.start, first.end, second.start, second.end
            ),
            Self::NotCharBoundary(offset) => {
                write!(formatter, "byte offset {offset} splits a UTF-8 scalar")
            }
            Self::InvalidUtf16Offset { offset, length } => write!(
                formatter,
                "UTF-16 offset {offset} exceeds text length {length}"
            ),
            Self::NotUtf16Boundary(offset) => {
                write!(formatter, "UTF-16 offset {offset} splits a surrogate pair")
            }
            Self::NotGraphemeBoundary(offset) => write!(
                formatter,
                "byte offset {offset} splits an extended grapheme cluster"
            ),
            Self::HardLineOutOfBounds { line, line_count } => write!(
                formatter,
                "hard line {line} is outside the document's {line_count} hard lines"
            ),
            Self::ArithmeticOverflow => {
                formatter.write_str("formatted-text aggregate arithmetic overflowed")
            }
            Self::LeafIdentityExhausted => {
                formatter.write_str("formatted-text leaf identities were exhausted")
            }
            Self::UnicodeBoundaryResolutionFailed => formatter
                .write_str("extended-grapheme boundary resolution could not obtain valid context"),
            Self::ResultTextMismatch => {
                formatter.write_str("persistent text splices did not reproduce the projection")
            }
        }
    }
}

impl std::error::Error for FormattedTextError {}

#[derive(Clone, Copy, Debug)]
struct Aggregate {
    bytes: usize,
    utf16_units: usize,
    hard_line_breaks: usize,
    leaves: usize,
    height: u32,
}

#[derive(Clone, Debug)]
struct Leaf {
    id: FormattedLeafId,
    revision: FormattedLeafRevision,
    buffer_id: FormattedBufferId,
    buffer: Arc<str>,
    range: Range<usize>,
    utf16_units: usize,
    hard_line_breaks: usize,
}

impl Leaf {
    fn text(&self) -> &str {
        &self.buffer[self.range.clone()]
    }

    fn byte_len(&self) -> usize {
        self.range.len()
    }
}

#[derive(Clone, Debug)]
struct Branch {
    left: Arc<Node>,
    right: Arc<Node>,
    aggregate: Aggregate,
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(Leaf),
    Branch(Branch),
}

type NodeSplit = Result<(Option<Arc<Node>>, Option<Arc<Node>>), FormattedTextError>;

impl Node {
    fn aggregate(&self) -> Aggregate {
        match self {
            Self::Leaf(leaf) => Aggregate {
                bytes: leaf.byte_len(),
                utf16_units: leaf.utf16_units,
                hard_line_breaks: leaf.hard_line_breaks,
                leaves: 1,
                height: 1,
            },
            Self::Branch(branch) => branch.aggregate,
        }
    }
}

/// An immutable balanced rope of valid UTF-8 formatted text.
#[derive(Clone, Debug, Default)]
pub struct FormattedTextTree {
    root: Option<Arc<Node>>,
}

impl FormattedTextTree {
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        fn visit(node: &Arc<Node>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
            visitor.arc(node, |visitor| match node.as_ref() {
                Node::Leaf(leaf) => visitor.arc(&leaf.buffer, |_| {}),
                Node::Branch(branch) => {
                    visit(&branch.left, visitor);
                    visit(&branch.right, visitor);
                }
            });
        }
        if let Some(root) = &self.root { visit(root, visitor); }
    }
}

/// Snapshot-local logical grapheme navigation used by position maps.
///
/// A bare [`FormattedTextTree`] implements Unicode extended-grapheme
/// segmentation over its flat UTF-8. A formatted projection may additionally
/// force boundaries around non-text atomic items, such as a normalized U+000A
/// semantic hard break. Keeping that policy outside the rope lets an unmarked
/// literal CRLF continue to behave as one UAX #29 grapheme.
pub(crate) trait LogicalGraphemeSnapshot {
    fn text_len(&self) -> usize;

    fn is_logical_grapheme_boundary(&self, offset: usize) -> Result<bool, FormattedTextError>;

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError>;

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError>;
}

/// Exact structural work performed by persistent formatted-text splices.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FormattedTextSpliceStats {
    pub(crate) nodes_visited: usize,
    pub(crate) nodes_copied: usize,
    pub(crate) leaves_copied: usize,
    pub(crate) inserted_bytes: usize,
    /// Original UTF-8 bytes scanned to recalculate split-leaf aggregates.
    pub(crate) original_bytes_recounted: usize,
}

/// Test-only witness for bounded regional reads. Production queries use the
/// same traversal without retaining counters.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FormattedTextReadStats {
    pub(crate) nodes_visited: usize,
    pub(crate) leaves_visited: usize,
    pub(crate) bytes_copied: usize,
}

impl FormattedTextTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_from_text(text: impl Into<Arc<str>>) -> Result<Self, FormattedTextError> {
        Self::try_from_shared(text.into())
    }

    pub(crate) fn try_from_shared(text: Arc<str>) -> Result<Self, FormattedTextError> {
        if text.is_empty() {
            return Ok(Self::new());
        }

        let buffer_id = next_buffer_id()?;
        let mut leaves = Vec::with_capacity(
            text.len()
                .checked_add(FORMATTED_TEXT_LEAF_BYTES - 1)
                .ok_or(FormattedTextError::ArithmeticOverflow)?
                / FORMATTED_TEXT_LEAF_BYTES,
        );
        let mut start = 0;
        while start < text.len() {
            let mut end = start
                .checked_add(FORMATTED_TEXT_LEAF_BYTES)
                .unwrap_or(text.len())
                .min(text.len());
            while end > start && !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == start {
                return Err(FormattedTextError::UnicodeBoundaryResolutionFailed);
            }
            leaves.push(new_leaf(buffer_id, text.clone(), start..end)?);
            start = end;
        }

        Ok(Self {
            root: build_balanced(&leaves)?,
        })
    }

    pub fn byte_len(&self) -> usize {
        aggregate(self.root.as_ref()).bytes
    }

    /// Number of UTF-16 code units in this immutable formatted snapshot.
    /// This is a root aggregate and does not visit text leaves.
    pub fn utf16_len(&self) -> usize {
        aggregate(self.root.as_ref()).utf16_units
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn leaf_count(&self) -> usize {
        aggregate(self.root.as_ref()).leaves
    }

    pub fn height(&self) -> u32 {
        aggregate(self.root.as_ref()).height
    }

    /// Whether two immutable snapshots share the exact persistent root.
    pub fn shares_root_with(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, Some(_)) | (Some(_), None) => false,
        }
    }

    /// The number of hard lines, including the final empty line after a
    /// trailing U+000A. An empty document has one hard line.
    pub fn hard_line_count(&self) -> usize {
        // A materialized string cannot contain usize::MAX newlines, so this is
        // guaranteed by construction and checked whenever branch aggregates
        // are formed.
        aggregate(self.root.as_ref()).hard_line_breaks + 1
    }

    /// Materialize this immutable snapshot as one flat UTF-8 string.
    pub fn flatten(&self) -> String {
        let mut output = String::with_capacity(self.byte_len());
        if let Some(root) = &self.root {
            append_text(root, &mut output);
        }
        output
    }

    /// Return a copied UTF-8 slice. Reading only visits intersecting leaves.
    pub fn slice(&self, range: Range<usize>) -> Result<String, FormattedTextError> {
        self.validate_range(&range, false)?;
        let mut output = String::with_capacity(range.len());
        if let Some(root) = &self.root {
            append_range(root, 0, &range, &mut output);
        }
        Ok(output)
    }

    #[cfg(test)]
    pub(crate) fn slice_with_stats(
        &self,
        range: Range<usize>,
    ) -> Result<(String, FormattedTextReadStats), FormattedTextError> {
        self.validate_range(&range, false)?;
        let mut output = String::with_capacity(range.len());
        let mut stats = FormattedTextReadStats::default();
        if let Some(root) = &self.root {
            append_range_with_stats(root, 0, &range, &mut output, &mut stats);
        }
        Ok((output, stats))
    }

    /// Map a UTF-8 scalar boundary to its UTF-16 code-unit boundary. Tree
    /// aggregates make prefix traversal logarithmic; only the containing
    /// bounded leaf is decoded.
    pub fn utf16_offset_for_byte(&self, offset: usize) -> Result<usize, FormattedTextError> {
        self.validate_offset(offset)?;
        if !self.is_char_boundary(offset)? {
            return Err(FormattedTextError::NotCharBoundary(offset));
        }
        let Some(root) = &self.root else {
            return Ok(0);
        };
        utf16_offset_for_byte(root, offset)
    }

    /// Map a UTF-16 code-unit boundary to its UTF-8 scalar boundary. An offset
    /// between the two code units of a surrogate pair is rejected rather than
    /// rounded. Prefix traversal is logarithmic with bounded leaf-local work.
    pub fn byte_offset_for_utf16(&self, offset: usize) -> Result<usize, FormattedTextError> {
        let length = self.utf16_len();
        if offset > length {
            return Err(FormattedTextError::InvalidUtf16Offset { offset, length });
        }
        let Some(root) = &self.root else {
            return Ok(0);
        };
        byte_offset_for_utf16(root, offset, offset)
    }

    /// Persistently replace a grapheme-aligned range.
    ///
    /// Untouched subtrees and leaf buffers are shared with `self`. Newly
    /// inserted text is chunked into bounded immutable leaves.
    pub fn splice(
        &self,
        range: Range<usize>,
        replacement: &str,
    ) -> Result<Self, FormattedTextError> {
        self.validate_range(&range, true)?;
        self.splice_char_aligned(range, replacement, &mut FormattedTextSpliceStats::default())
    }

    /// Apply a batch whose ranges are expressed in `self` and have already
    /// been semantically validated as grapheme boundaries. The batch traverses
    /// the original snapshot once, retaining unaffected subtrees. Boundaries
    /// need not remain grapheme boundaries in the result: a replacement may
    /// legitimately join a grapheme across another edit's old boundary.
    #[cfg(test)]
    pub(crate) fn splice_batch_with_stats(
        &self,
        edits: &[(Range<usize>, &str)],
    ) -> Result<(Self, FormattedTextSpliceStats), FormattedTextError> {
        self.splice_batch_with_stats_and_boundary_policy(edits, true)
    }

    /// Apply a batch whose ranges were already checked against the formatted
    /// projection's logical-item boundary space.
    ///
    /// The rope itself sees only flat UTF-8, so it cannot distinguish a marked
    /// semantic LF from an unmarked literal LF. In particular, UAX #29 merges a
    /// literal CR followed by a marked LF into CRLF. Projection-validated edits
    /// must therefore require scalar alignment here rather than incorrectly
    /// rejecting the forced logical boundary between those two items.
    pub(crate) fn splice_prevalidated_batch(
        &self,
        edits: &[(Range<usize>, &str)],
    ) -> Result<Self, FormattedTextError> {
        self.splice_prevalidated_batch_with_stats(edits)
            .map(|(tree, _)| tree)
    }

    pub(crate) fn splice_prevalidated_batch_with_stats(
        &self,
        edits: &[(Range<usize>, &str)],
    ) -> Result<(Self, FormattedTextSpliceStats), FormattedTextError> {
        self.splice_batch_with_stats_and_boundary_policy(edits, false)
    }

    fn splice_batch_with_stats_and_boundary_policy(
        &self,
        edits: &[(Range<usize>, &str)],
        require_flat_grapheme: bool,
    ) -> Result<(Self, FormattedTextSpliceStats), FormattedTextError> {
        for (range, _) in edits {
            self.validate_range(range, require_flat_grapheme)?;
        }
        for pair in edits.windows(2) {
            let previous = &pair[0].0;
            let next = &pair[1].0;
            if previous.end > next.start
                || (previous.is_empty() && next.is_empty() && previous.start == next.start)
            {
                return Err(FormattedTextError::OverlappingSplices {
                    first: previous.clone(),
                    second: next.clone(),
                });
            }
        }
        let mut batch = BatchSplice {
            edits,
            next: 0,
            deleted_until: 0,
            stats: FormattedTextSpliceStats::default(),
        };
        batch.skip_noops();
        if batch.next == edits.len() {
            return Ok((self.clone(), batch.stats));
        }
        let mut root = match &self.root {
            Some(root) => batch.visit(root, 0)?,
            None => None,
        };
        // An insertion at EOF belongs after the final subtree, including the
        // sole boundary of an empty tree.
        while batch.next < edits.len() {
            debug_assert_eq!(edits[batch.next].0, self.byte_len()..self.byte_len());
            let inserted = batch.take_replacement()?;
            root = join_optional(root, inserted, &mut batch.stats)?;
        }
        Ok((Self { root }, batch.stats))
    }

    fn splice_char_aligned(
        &self,
        range: Range<usize>,
        replacement: &str,
        stats: &mut FormattedTextSpliceStats,
    ) -> Result<Self, FormattedTextError> {
        self.validate_range(&range, false)?;
        if range.is_empty() && replacement.is_empty() {
            return Ok(self.clone());
        }

        let (before, remainder) =
            split_optional(self.root.clone(), range.start, Retain::Left, stats)?;
        let (_, after) = split_optional(
            remainder,
            range
                .end
                .checked_sub(range.start)
                .ok_or(FormattedTextError::ArithmeticOverflow)?,
            Retain::Right,
            stats,
        )?;
        let inserted = Self::try_from_text(Arc::<str>::from(replacement))?.root;
        let inserted_leaves = aggregate(inserted.as_ref()).leaves;
        stats.inserted_bytes = stats.inserted_bytes.saturating_add(replacement.len());
        stats.leaves_copied = stats.leaves_copied.saturating_add(inserted_leaves);
        stats.nodes_copied = stats
            .nodes_copied
            .saturating_add(inserted_leaves.saturating_mul(2).saturating_sub(1));
        let joined = join_optional(before, inserted, stats)?;
        let root = join_optional(joined, after, stats)?;
        Ok(Self { root })
    }

    #[cfg(test)]
    pub(crate) fn shared_backing_leaf_count(&self, other: &Self) -> usize {
        let mut own = Vec::new();
        let mut theirs = Vec::new();
        if let Some(root) = &self.root {
            collect_leaf_refs(root, &mut own);
        }
        if let Some(root) = &other.root {
            collect_leaf_refs(root, &mut theirs);
        }
        own.iter()
            .filter(|leaf| {
                theirs.iter().any(|candidate| {
                    leaf.id == candidate.id
                        && leaf.revision == candidate.revision
                        && Arc::ptr_eq(&leaf.buffer, &candidate.buffer)
                })
            })
            .count()
    }

    /// Locate a UTF-8 byte boundary in a leaf. At a shared boundary, `side`
    /// selects the adjacent leaf; the opposite side is used at document edges.
    pub fn locate_byte(
        &self,
        offset: usize,
        side: LeafBoundarySide,
    ) -> Result<Option<FormattedLeafLocation>, FormattedTextError> {
        self.validate_offset(offset)?;
        if !self.is_char_boundary(offset)? {
            return Err(FormattedTextError::NotCharBoundary(offset));
        }
        let Some(root) = &self.root else {
            return Ok(None);
        };
        let (leaf, start) = match side {
            LeafBoundarySide::Following if offset < self.byte_len() => {
                leaf_at_or_after(root, offset, 0)
            }
            LeafBoundarySide::Preceding if offset > 0 => leaf_before_or_at(root, offset, 0),
            LeafBoundarySide::Following => leaf_before_or_at(root, offset, 0),
            LeafBoundarySide::Preceding => leaf_at_or_after(root, offset, 0),
        };
        Ok(Some(FormattedLeafLocation {
            id: leaf.id,
            revision: leaf.revision,
            buffer_id: leaf.buffer_id,
            leaf_range: start..start + leaf.byte_len(),
            local_byte: offset - start,
            buffer_byte: leaf.range.start + (offset - start),
        }))
    }

    /// Resolve an allocation-local boundary captured from an earlier
    /// persistent snapshot.
    ///
    /// The leaf identity is tried first. If a splice split that leaf and the
    /// requested side moved to the freshly identified sibling, the immutable
    /// allocation identity recovers it. Work is currently proportional to the
    /// number of leaves only on this stale-anchor recovery path; ordinary
    /// snapshot-local lookup remains logarithmic.
    pub(crate) fn resolve_stable_boundary(
        &self,
        id: FormattedLeafId,
        revision: FormattedLeafRevision,
        buffer_id: FormattedBufferId,
        buffer_byte: usize,
        side: LeafBoundarySide,
    ) -> Option<usize> {
        let root = self.root.as_ref()?;
        find_stable_boundary(root, 0, id, revision, buffer_id, buffer_byte, side, true).or_else(
            || find_stable_boundary(root, 0, id, revision, buffer_id, buffer_byte, side, false),
        )
    }

    /// Inventory all current leaves in document order.
    pub fn leaves(&self) -> Vec<FormattedLeafInfo> {
        let mut output = Vec::with_capacity(self.leaf_count());
        if let Some(root) = &self.root {
            collect_leaf_info(root, 0, &mut output);
        }
        output
    }

    pub fn is_char_boundary(&self, offset: usize) -> Result<bool, FormattedTextError> {
        self.validate_offset(offset)?;
        if offset == 0 || offset == self.byte_len() {
            return Ok(true);
        }
        let root = self.root.as_ref().expect("non-edge offset has a root");
        let (leaf, start) = leaf_at_or_after(root, offset, 0);
        Ok(leaf.text().is_char_boundary(offset - start))
    }

    /// Resolve an extended-grapheme boundary without flattening the rope.
    pub fn is_grapheme_boundary(&self, offset: usize) -> Result<bool, FormattedTextError> {
        self.validate_offset(offset)?;
        if offset == 0 || offset == self.byte_len() {
            return Ok(true);
        }
        if !self.is_char_boundary(offset)? {
            return Ok(false);
        }

        let root = self.root.as_ref().expect("interior offset has a root");
        let (leaf, chunk_start) = leaf_at_or_after(root, offset, 0);
        let chunk = leaf.text();
        let mut cursor = GraphemeCursor::new(offset, self.byte_len(), true);
        loop {
            match cursor.is_boundary(chunk, chunk_start) {
                Ok(boundary) => return Ok(boundary),
                Err(GraphemeIncomplete::PreContext(context_end)) => {
                    let Some((context_leaf, context_start, prefix_len)) =
                        context_before(root, context_end, 0)
                    else {
                        return Err(FormattedTextError::UnicodeBoundaryResolutionFailed);
                    };
                    let context = &context_leaf.text()[..prefix_len];
                    cursor.provide_context(context, context_start);
                }
                Err(
                    GraphemeIncomplete::PrevChunk
                    | GraphemeIncomplete::NextChunk
                    | GraphemeIncomplete::InvalidOffset,
                ) => return Err(FormattedTextError::UnicodeBoundaryResolutionFailed),
            }
        }
    }

    /// Return the next extended-grapheme boundary without flattening the
    /// tree. Work is proportional to the local grapheme context plus tree
    /// descent, including deliberately long combining/RI sequences.
    pub(crate) fn next_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        self.validate_offset(offset)?;
        if !self.is_char_boundary(offset)? {
            return Err(FormattedTextError::NotCharBoundary(offset));
        }
        let mut candidate = offset;
        loop {
            let Some(next) = self.next_char_boundary(candidate)? else {
                return Ok(None);
            };
            candidate = next;
            if self.is_grapheme_boundary(candidate)? {
                return Ok(Some(candidate));
            }
        }
    }

    /// Return the preceding extended-grapheme boundary without flattening the
    /// tree.
    pub(crate) fn previous_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        self.validate_offset(offset)?;
        if !self.is_char_boundary(offset)? {
            return Err(FormattedTextError::NotCharBoundary(offset));
        }
        let mut candidate = offset;
        loop {
            let Some(previous) = self.previous_char_boundary(candidate)? else {
                return Ok(None);
            };
            candidate = previous;
            if self.is_grapheme_boundary(candidate)? {
                return Ok(Some(candidate));
            }
        }
    }

    fn next_char_boundary(&self, offset: usize) -> Result<Option<usize>, FormattedTextError> {
        self.validate_offset(offset)?;
        if offset == self.byte_len() {
            return Ok(None);
        }
        let root = self.root.as_ref().expect("a non-EOF offset has a root");
        let (leaf, start) = leaf_at_or_after(root, offset, 0);
        let local = offset
            .checked_sub(start)
            .ok_or(FormattedTextError::ArithmeticOverflow)?;
        let scalar = leaf.text()[local..]
            .chars()
            .next()
            .ok_or(FormattedTextError::UnicodeBoundaryResolutionFailed)?;
        offset
            .checked_add(scalar.len_utf8())
            .map(Some)
            .ok_or(FormattedTextError::ArithmeticOverflow)
    }

    fn previous_char_boundary(&self, offset: usize) -> Result<Option<usize>, FormattedTextError> {
        self.validate_offset(offset)?;
        if offset == 0 {
            return Ok(None);
        }
        let root = self.root.as_ref().expect("a nonzero offset has a root");
        let (leaf, start) = leaf_before_or_at(root, offset, 0);
        let local = offset
            .checked_sub(start)
            .ok_or(FormattedTextError::ArithmeticOverflow)?;
        let scalar = leaf.text()[..local]
            .chars()
            .next_back()
            .ok_or(FormattedTextError::UnicodeBoundaryResolutionFailed)?;
        offset
            .checked_sub(scalar.len_utf8())
            .map(Some)
            .ok_or(FormattedTextError::ArithmeticOverflow)
    }

    /// Return the byte boundary at the start of a zero-based hard line.
    pub fn hard_line_start(&self, line: usize) -> Result<usize, FormattedTextError> {
        let line_count = self.hard_line_count();
        if line >= line_count {
            return Err(FormattedTextError::HardLineOutOfBounds { line, line_count });
        }
        if line == 0 {
            return Ok(0);
        }
        let root = self
            .root
            .as_ref()
            .ok_or(FormattedTextError::HardLineOutOfBounds { line, line_count })?;
        nth_line_break(root, line - 1, 0)?
            .checked_add(1)
            .ok_or(FormattedTextError::ArithmeticOverflow)
    }

    /// Return the byte boundary before a line's terminating U+000A, or the
    /// document end for the final unterminated line.
    pub fn hard_line_end(&self, line: usize) -> Result<usize, FormattedTextError> {
        let line_count = self.hard_line_count();
        if line >= line_count {
            return Err(FormattedTextError::HardLineOutOfBounds { line, line_count });
        }
        if line + 1 == line_count {
            return Ok(self.byte_len());
        }
        let root = self
            .root
            .as_ref()
            .ok_or(FormattedTextError::HardLineOutOfBounds { line, line_count })?;
        nth_line_break(root, line, 0)
    }

    /// Resolve the hard line containing a byte boundary. A boundary immediately
    /// before U+000A belongs to the preceding line; the boundary immediately
    /// after it belongs to the following line.
    pub fn hard_line_at_byte(&self, offset: usize) -> Result<usize, FormattedTextError> {
        self.validate_offset(offset)?;
        if !self.is_char_boundary(offset)? {
            return Err(FormattedTextError::NotCharBoundary(offset));
        }
        Ok(self
            .root
            .as_ref()
            .map_or(0, |root| count_line_breaks_before(root, offset)))
    }

    fn validate_offset(&self, offset: usize) -> Result<(), FormattedTextError> {
        if offset > self.byte_len() {
            Err(FormattedTextError::InvalidByteOffset {
                offset,
                length: self.byte_len(),
            })
        } else {
            Ok(())
        }
    }

    fn validate_range(
        &self,
        range: &Range<usize>,
        require_grapheme: bool,
    ) -> Result<(), FormattedTextError> {
        if range.start > range.end || range.end > self.byte_len() {
            return Err(FormattedTextError::InvalidRange {
                start: range.start,
                end: range.end,
                length: self.byte_len(),
            });
        }
        for offset in [range.start, range.end] {
            if !self.is_char_boundary(offset)? {
                return Err(FormattedTextError::NotCharBoundary(offset));
            }
            if require_grapheme && !self.is_grapheme_boundary(offset)? {
                return Err(FormattedTextError::NotGraphemeBoundary(offset));
            }
        }
        Ok(())
    }
}

impl LogicalGraphemeSnapshot for FormattedTextTree {
    fn text_len(&self) -> usize {
        self.byte_len()
    }

    fn is_logical_grapheme_boundary(&self, offset: usize) -> Result<bool, FormattedTextError> {
        self.is_grapheme_boundary(offset)
    }

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        self.next_grapheme_boundary(offset)
    }

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        self.previous_grapheme_boundary(offset)
    }
}

impl PartialEq for FormattedTextTree {
    fn eq(&self, other: &Self) -> bool {
        self.byte_len() == other.byte_len() && self.flatten() == other.flatten()
    }
}

impl Eq for FormattedTextTree {}

#[derive(Clone, Copy)]
enum Retain {
    Left,
    Right,
}

fn next_leaf_id() -> Result<FormattedLeafId, FormattedTextError> {
    NEXT_LEAF_ID
        .fetch_update(
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
            |current| current.checked_add(1),
        )
        .map(FormattedLeafId)
        .map_err(|_| FormattedTextError::LeafIdentityExhausted)
}

fn next_buffer_id() -> Result<FormattedBufferId, FormattedTextError> {
    NEXT_BUFFER_ID
        .fetch_update(
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
            |current| current.checked_add(1),
        )
        .map(FormattedBufferId)
        .map_err(|_| FormattedTextError::LeafIdentityExhausted)
}

/// Consume sorted old-snapshot edits while descending only affected paths.
/// Each original leaf is split into all of its surviving slices together, so
/// dense delimiter edits never repeatedly recount the same remaining suffix.
struct BatchSplice<'a> {
    edits: &'a [(Range<usize>, &'a str)],
    next: usize,
    deleted_until: usize,
    stats: FormattedTextSpliceStats,
}

impl BatchSplice<'_> {
    fn skip_noops(&mut self) {
        while self.next < self.edits.len()
            && self.edits[self.next].0.is_empty()
            && self.edits[self.next].1.is_empty()
        {
            self.next += 1;
        }
    }

    fn take_replacement(&mut self) -> Result<Option<Arc<Node>>, FormattedTextError> {
        let (range, replacement) = &self.edits[self.next];
        let inserted = FormattedTextTree::try_from_text(*replacement)?.root;
        let leaves = aggregate(inserted.as_ref()).leaves;
        self.stats.inserted_bytes = self.stats.inserted_bytes.saturating_add(replacement.len());
        self.stats.leaves_copied = self.stats.leaves_copied.saturating_add(leaves);
        self.stats.nodes_copied = self
            .stats
            .nodes_copied
            .saturating_add(leaves.saturating_mul(2).saturating_sub(1));
        self.deleted_until = range.end;
        self.next += 1;
        self.skip_noops();
        Ok(inserted)
    }

    fn visit(
        &mut self,
        node: &Arc<Node>,
        start: usize,
    ) -> Result<Option<Arc<Node>>, FormattedTextError> {
        self.stats.nodes_visited = self.stats.nodes_visited.saturating_add(1);
        let end = start + node.aggregate().bytes;
        if self.deleted_until >= end {
            return Ok(None);
        }
        if self.deleted_until <= start
            && self
                .edits
                .get(self.next)
                .map_or(true, |(range, _)| range.start >= end)
        {
            return Ok(Some(node.clone()));
        }
        match node.as_ref() {
            Node::Branch(branch) => {
                let left = self.visit(&branch.left, start)?;
                let right = self.visit(&branch.right, start + branch.left.aggregate().bytes)?;
                join_optional(left, right, &mut self.stats)
            }
            Node::Leaf(leaf) => {
                let mut parts = Vec::new();
                let mut at = start.max(self.deleted_until);
                let mut retained_identity = false;
                while let Some((range, _)) = self.edits.get(self.next) {
                    if range.start >= end {
                        break;
                    }
                    if at < range.start {
                        parts.push(self.original_slice(
                            node,
                            leaf,
                            at - start..range.start - start,
                            &mut retained_identity,
                        )?);
                    }
                    if let Some(inserted) = self.take_replacement()? {
                        parts.push(inserted);
                    }
                    at = self.deleted_until;
                }
                if at < end {
                    parts.push(self.original_slice(
                        node,
                        leaf,
                        at - start..leaf.byte_len(),
                        &mut retained_identity,
                    )?);
                }
                join_batch_parts(&parts, &mut self.stats)
            }
        }
    }

    fn original_slice(
        &mut self,
        node: &Arc<Node>,
        leaf: &Leaf,
        local: Range<usize>,
        retained_identity: &mut bool,
    ) -> Result<Arc<Node>, FormattedTextError> {
        if local == (0..leaf.byte_len()) {
            return Ok(node.clone());
        }
        let (id, revision) = if *retained_identity {
            (next_leaf_id()?, FormattedLeafRevision(0))
        } else {
            *retained_identity = true;
            (
                leaf.id,
                FormattedLeafRevision(
                    leaf.revision
                        .0
                        .checked_add(1)
                        .ok_or(FormattedTextError::ArithmeticOverflow)?,
                ),
            )
        };
        self.stats.original_bytes_recounted = self
            .stats
            .original_bytes_recounted
            .saturating_add(local.len());
        self.stats.leaves_copied = self.stats.leaves_copied.saturating_add(1);
        self.stats.nodes_copied = self.stats.nodes_copied.saturating_add(1);
        new_leaf_with_identity(
            id,
            revision,
            leaf.buffer_id,
            leaf.buffer.clone(),
            leaf.range.start + local.start..leaf.range.start + local.end,
        )
    }
}

/// Parts may include arbitrarily tall replacement subtrees. AVL joins retain
/// those subtrees while balancing them with short surviving original slices.
fn join_batch_parts(
    nodes: &[Arc<Node>],
    stats: &mut FormattedTextSpliceStats,
) -> Result<Option<Arc<Node>>, FormattedTextError> {
    match nodes.len() {
        0 => Ok(None),
        1 => Ok(Some(nodes[0].clone())),
        length => {
            let middle = length / 2;
            let left = join_batch_parts(&nodes[..middle], stats)?;
            let right = join_batch_parts(&nodes[middle..], stats)?;
            join_optional(left, right, stats)
        }
    }
}

fn new_leaf(
    buffer_id: FormattedBufferId,
    buffer: Arc<str>,
    range: Range<usize>,
) -> Result<Arc<Node>, FormattedTextError> {
    new_leaf_with_identity(
        next_leaf_id()?,
        FormattedLeafRevision(0),
        buffer_id,
        buffer,
        range,
    )
}

fn new_leaf_with_identity(
    id: FormattedLeafId,
    revision: FormattedLeafRevision,
    buffer_id: FormattedBufferId,
    buffer: Arc<str>,
    range: Range<usize>,
) -> Result<Arc<Node>, FormattedTextError> {
    if range.start > range.end
        || range.end > buffer.len()
        || !buffer.is_char_boundary(range.start)
        || !buffer.is_char_boundary(range.end)
    {
        return Err(FormattedTextError::UnicodeBoundaryResolutionFailed);
    }
    if range.is_empty() {
        return Err(FormattedTextError::UnicodeBoundaryResolutionFailed);
    }
    let hard_line_breaks = buffer[range.clone()]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let utf16_units = buffer[range.clone()].encode_utf16().count();
    Ok(Arc::new(Node::Leaf(Leaf {
        id,
        revision,
        buffer_id,
        buffer,
        range,
        utf16_units,
        hard_line_breaks,
    })))
}

fn aggregate(node: Option<&Arc<Node>>) -> Aggregate {
    node.map_or(
        Aggregate {
            bytes: 0,
            utf16_units: 0,
            hard_line_breaks: 0,
            leaves: 0,
            height: 0,
        },
        |node| node.aggregate(),
    )
}

fn branch(left: Arc<Node>, right: Arc<Node>) -> Result<Arc<Node>, FormattedTextError> {
    let left_aggregate = left.aggregate();
    let right_aggregate = right.aggregate();
    let aggregate = Aggregate {
        bytes: left_aggregate
            .bytes
            .checked_add(right_aggregate.bytes)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
        utf16_units: left_aggregate
            .utf16_units
            .checked_add(right_aggregate.utf16_units)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
        hard_line_breaks: left_aggregate
            .hard_line_breaks
            .checked_add(right_aggregate.hard_line_breaks)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
        leaves: left_aggregate
            .leaves
            .checked_add(right_aggregate.leaves)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
        height: left_aggregate
            .height
            .max(right_aggregate.height)
            .checked_add(1)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
    };
    Ok(Arc::new(Node::Branch(Branch {
        left,
        right,
        aggregate,
    })))
}

fn build_balanced(nodes: &[Arc<Node>]) -> Result<Option<Arc<Node>>, FormattedTextError> {
    match nodes.len() {
        0 => Ok(None),
        1 => Ok(Some(nodes[0].clone())),
        length => {
            let middle = length / 2;
            let left = build_balanced(&nodes[..middle])?.expect("nonempty left half");
            let right = build_balanced(&nodes[middle..])?.expect("nonempty right half");
            Ok(Some(branch(left, right)?))
        }
    }
}

fn join_optional(
    left: Option<Arc<Node>>,
    right: Option<Arc<Node>>,
    stats: &mut FormattedTextSpliceStats,
) -> Result<Option<Arc<Node>>, FormattedTextError> {
    match (left, right) {
        (None, value) | (value, None) => Ok(value),
        (Some(left), Some(right)) => Ok(Some(join(left, right, stats)?)),
    }
}

fn join(
    left: Arc<Node>,
    right: Arc<Node>,
    stats: &mut FormattedTextSpliceStats,
) -> Result<Arc<Node>, FormattedTextError> {
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    let left_height = left.aggregate().height;
    let right_height = right.aggregate().height;
    if left_height > right_height.saturating_add(1) {
        let Node::Branch(left_branch) = left.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        let joined = join(left_branch.right.clone(), right, stats)?;
        rebalance(left_branch.left.clone(), joined, stats)
    } else if right_height > left_height.saturating_add(1) {
        let Node::Branch(right_branch) = right.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        let joined = join(left, right_branch.left.clone(), stats)?;
        rebalance(joined, right_branch.right.clone(), stats)
    } else {
        counted_branch(left, right, stats)
    }
}

fn rebalance(
    left: Arc<Node>,
    right: Arc<Node>,
    stats: &mut FormattedTextSpliceStats,
) -> Result<Arc<Node>, FormattedTextError> {
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    let left_height = left.aggregate().height;
    let right_height = right.aggregate().height;
    if left_height > right_height.saturating_add(1) {
        let Node::Branch(left_branch) = left.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        if left_branch.left.aggregate().height >= left_branch.right.aggregate().height {
            let nested = counted_branch(left_branch.right.clone(), right, stats)?;
            return counted_branch(left_branch.left.clone(), nested, stats);
        }
        let Node::Branch(left_right) = left_branch.right.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        let new_left = counted_branch(left_branch.left.clone(), left_right.left.clone(), stats)?;
        let new_right = counted_branch(left_right.right.clone(), right, stats)?;
        return counted_branch(new_left, new_right, stats);
    }
    if right_height > left_height.saturating_add(1) {
        let Node::Branch(right_branch) = right.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        if right_branch.right.aggregate().height >= right_branch.left.aggregate().height {
            let nested = counted_branch(left, right_branch.left.clone(), stats)?;
            return counted_branch(nested, right_branch.right.clone(), stats);
        }
        let Node::Branch(right_left) = right_branch.left.as_ref() else {
            return Err(FormattedTextError::ArithmeticOverflow);
        };
        let new_left = counted_branch(left, right_left.left.clone(), stats)?;
        let new_right =
            counted_branch(right_left.right.clone(), right_branch.right.clone(), stats)?;
        return counted_branch(new_left, new_right, stats);
    }
    counted_branch(left, right, stats)
}

fn counted_branch(
    left: Arc<Node>,
    right: Arc<Node>,
    stats: &mut FormattedTextSpliceStats,
) -> Result<Arc<Node>, FormattedTextError> {
    stats.nodes_copied = stats.nodes_copied.saturating_add(1);
    branch(left, right)
}

fn split_optional(
    node: Option<Arc<Node>>,
    at: usize,
    retain: Retain,
    stats: &mut FormattedTextSpliceStats,
) -> NodeSplit {
    let Some(node) = node else {
        if at == 0 {
            return Ok((None, None));
        }
        return Err(FormattedTextError::InvalidByteOffset {
            offset: at,
            length: 0,
        });
    };
    if at > node.aggregate().bytes {
        return Err(FormattedTextError::InvalidByteOffset {
            offset: at,
            length: node.aggregate().bytes,
        });
    }
    split(node, at, retain, stats)
}

fn split(
    node: Arc<Node>,
    at: usize,
    retain: Retain,
    stats: &mut FormattedTextSpliceStats,
) -> NodeSplit {
    let length = node.aggregate().bytes;
    if at == 0 {
        return Ok((None, Some(node)));
    }
    if at == length {
        return Ok((Some(node), None));
    }
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    match node.as_ref() {
        Node::Leaf(leaf) => split_leaf(leaf, at, retain, stats),
        Node::Branch(branch_node) => {
            let left_length = branch_node.left.aggregate().bytes;
            match at.cmp(&left_length) {
                Ordering::Less => {
                    let (before, middle) = split(branch_node.left.clone(), at, retain, stats)?;
                    let after = join_optional(middle, Some(branch_node.right.clone()), stats)?;
                    Ok((before, after))
                }
                Ordering::Equal => Ok((
                    Some(branch_node.left.clone()),
                    Some(branch_node.right.clone()),
                )),
                Ordering::Greater => {
                    let (middle, after) =
                        split(branch_node.right.clone(), at - left_length, retain, stats)?;
                    let before = join_optional(Some(branch_node.left.clone()), middle, stats)?;
                    Ok((before, after))
                }
            }
        }
    }
}

fn split_leaf(
    leaf: &Leaf,
    at: usize,
    retain: Retain,
    stats: &mut FormattedTextSpliceStats,
) -> NodeSplit {
    stats.original_bytes_recounted = stats
        .original_bytes_recounted
        .saturating_add(leaf.byte_len());
    if !leaf.text().is_char_boundary(at) {
        return Err(FormattedTextError::NotCharBoundary(at));
    }
    let split_at = leaf
        .range
        .start
        .checked_add(at)
        .ok_or(FormattedTextError::ArithmeticOverflow)?;
    let retained_revision = FormattedLeafRevision(
        leaf.revision
            .0
            .checked_add(1)
            .ok_or(FormattedTextError::ArithmeticOverflow)?,
    );
    let (left_id, left_revision, right_id, right_revision) = match retain {
        Retain::Left => (
            leaf.id,
            retained_revision,
            next_leaf_id()?,
            FormattedLeafRevision(0),
        ),
        Retain::Right => (
            next_leaf_id()?,
            FormattedLeafRevision(0),
            leaf.id,
            retained_revision,
        ),
    };
    let result = (
        Some(new_leaf_with_identity(
            left_id,
            left_revision,
            leaf.buffer_id,
            leaf.buffer.clone(),
            leaf.range.start..split_at,
        )?),
        Some(new_leaf_with_identity(
            right_id,
            right_revision,
            leaf.buffer_id,
            leaf.buffer.clone(),
            split_at..leaf.range.end,
        )?),
    );
    stats.leaves_copied = stats.leaves_copied.saturating_add(2);
    stats.nodes_copied = stats.nodes_copied.saturating_add(2);
    Ok(result)
}

fn append_text(node: &Node, output: &mut String) {
    match node {
        Node::Leaf(leaf) => output.push_str(leaf.text()),
        Node::Branch(branch) => {
            append_text(&branch.left, output);
            append_text(&branch.right, output);
        }
    }
}

fn append_range(node: &Node, start: usize, range: &Range<usize>, output: &mut String) {
    let end = start + node.aggregate().bytes;
    if range.end <= start || end <= range.start {
        return;
    }
    match node {
        Node::Leaf(leaf) => {
            let local_start = range.start.saturating_sub(start).min(leaf.byte_len());
            let local_end = range.end.saturating_sub(start).min(leaf.byte_len());
            output.push_str(&leaf.text()[local_start..local_end]);
        }
        Node::Branch(branch) => {
            append_range(&branch.left, start, range, output);
            append_range(
                &branch.right,
                start + branch.left.aggregate().bytes,
                range,
                output,
            );
        }
    }
}

#[cfg(test)]
fn append_range_with_stats(
    node: &Node,
    start: usize,
    range: &Range<usize>,
    output: &mut String,
    stats: &mut FormattedTextReadStats,
) {
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    let end = start + node.aggregate().bytes;
    if range.end <= start || end <= range.start {
        return;
    }
    match node {
        Node::Leaf(leaf) => {
            stats.leaves_visited = stats.leaves_visited.saturating_add(1);
            let local_start = range.start.saturating_sub(start).min(leaf.byte_len());
            let local_end = range.end.saturating_sub(start).min(leaf.byte_len());
            let text = &leaf.text()[local_start..local_end];
            stats.bytes_copied = stats.bytes_copied.saturating_add(text.len());
            output.push_str(text);
        }
        Node::Branch(branch) => {
            append_range_with_stats(&branch.left, start, range, output, stats);
            append_range_with_stats(
                &branch.right,
                start + branch.left.aggregate().bytes,
                range,
                output,
                stats,
            );
        }
    }
}

fn utf16_offset_for_byte(node: &Node, offset: usize) -> Result<usize, FormattedTextError> {
    match node {
        Node::Leaf(leaf) => Ok(leaf.text()[..offset].encode_utf16().count()),
        Node::Branch(branch) => {
            let left = branch.left.aggregate();
            if offset <= left.bytes {
                utf16_offset_for_byte(&branch.left, offset)
            } else {
                left.utf16_units
                    .checked_add(utf16_offset_for_byte(&branch.right, offset - left.bytes)?)
                    .ok_or(FormattedTextError::ArithmeticOverflow)
            }
        }
    }
}

fn byte_offset_for_utf16(
    node: &Node,
    offset: usize,
    requested: usize,
) -> Result<usize, FormattedTextError> {
    match node {
        Node::Leaf(leaf) => {
            let mut utf16 = 0usize;
            for (byte, scalar) in leaf.text().char_indices() {
                if utf16 == offset {
                    return Ok(byte);
                }
                let next = utf16
                    .checked_add(scalar.len_utf16())
                    .ok_or(FormattedTextError::ArithmeticOverflow)?;
                if offset < next {
                    return Err(FormattedTextError::NotUtf16Boundary(requested));
                }
                utf16 = next;
            }
            if utf16 == offset {
                Ok(leaf.byte_len())
            } else {
                Err(FormattedTextError::InvalidUtf16Offset {
                    offset: requested,
                    length: node.aggregate().utf16_units,
                })
            }
        }
        Node::Branch(branch) => {
            let left = branch.left.aggregate();
            if offset <= left.utf16_units {
                byte_offset_for_utf16(&branch.left, offset, requested)
            } else {
                left.bytes
                    .checked_add(byte_offset_for_utf16(
                        &branch.right,
                        offset - left.utf16_units,
                        requested,
                    )?)
                    .ok_or(FormattedTextError::ArithmeticOverflow)
            }
        }
    }
}

fn leaf_at_or_after(node: &Node, offset: usize, start: usize) -> (&Leaf, usize) {
    match node {
        Node::Leaf(leaf) => (leaf, start),
        Node::Branch(branch) => {
            let right_start = start + branch.left.aggregate().bytes;
            if offset < right_start {
                leaf_at_or_after(&branch.left, offset, start)
            } else {
                leaf_at_or_after(&branch.right, offset, right_start)
            }
        }
    }
}

fn leaf_before_or_at(node: &Node, offset: usize, start: usize) -> (&Leaf, usize) {
    match node {
        Node::Leaf(leaf) => (leaf, start),
        Node::Branch(branch) => {
            let right_start = start + branch.left.aggregate().bytes;
            if offset <= right_start {
                leaf_before_or_at(&branch.left, offset, start)
            } else {
                leaf_before_or_at(&branch.right, offset, right_start)
            }
        }
    }
}

fn context_before(node: &Node, end: usize, start: usize) -> Option<(&Leaf, usize, usize)> {
    if end == 0 || end > start + node.aggregate().bytes {
        return None;
    }
    match node {
        Node::Leaf(leaf) => Some((leaf, start, end - start)),
        Node::Branch(branch) => {
            let right_start = start + branch.left.aggregate().bytes;
            if end <= right_start {
                context_before(&branch.left, end, start)
            } else {
                context_before(&branch.right, end, right_start)
            }
        }
    }
}

fn collect_leaf_info(node: &Node, start: usize, output: &mut Vec<FormattedLeafInfo>) {
    match node {
        Node::Leaf(leaf) => output.push(FormattedLeafInfo {
            id: leaf.id,
            revision: leaf.revision,
            byte_range: start..start + leaf.byte_len(),
            hard_line_breaks: leaf.hard_line_breaks,
        }),
        Node::Branch(branch) => {
            collect_leaf_info(&branch.left, start, output);
            collect_leaf_info(&branch.right, start + branch.left.aggregate().bytes, output);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn find_stable_boundary(
    node: &Node,
    document_start: usize,
    id: FormattedLeafId,
    captured_revision: FormattedLeafRevision,
    buffer_id: FormattedBufferId,
    buffer_byte: usize,
    side: LeafBoundarySide,
    require_leaf_id: bool,
) -> Option<usize> {
    match node {
        Node::Leaf(leaf) => {
            if leaf.buffer_id != buffer_id || (require_leaf_id && leaf.id != id) {
                return None;
            }
            let exact_leaf_revision = leaf.id == id && leaf.revision == captured_revision;
            let contains = if exact_leaf_revision {
                leaf.range.start <= buffer_byte && buffer_byte <= leaf.range.end
            } else {
                match side {
                    LeafBoundarySide::Preceding => {
                        leaf.range.start < buffer_byte && buffer_byte <= leaf.range.end
                    }
                    LeafBoundarySide::Following => {
                        leaf.range.start <= buffer_byte && buffer_byte < leaf.range.end
                    }
                }
            };
            contains.then(|| document_start + buffer_byte - leaf.range.start)
        }
        Node::Branch(branch) => find_stable_boundary(
            &branch.left,
            document_start,
            id,
            captured_revision,
            buffer_id,
            buffer_byte,
            side,
            require_leaf_id,
        )
        .or_else(|| {
            find_stable_boundary(
                &branch.right,
                document_start + branch.left.aggregate().bytes,
                id,
                captured_revision,
                buffer_id,
                buffer_byte,
                side,
                require_leaf_id,
            )
        }),
    }
}

#[cfg(test)]
fn collect_leaf_refs<'a>(node: &'a Node, output: &mut Vec<&'a Leaf>) {
    match node {
        Node::Leaf(leaf) => output.push(leaf),
        Node::Branch(branch) => {
            collect_leaf_refs(&branch.left, output);
            collect_leaf_refs(&branch.right, output);
        }
    }
}

fn nth_line_break(node: &Node, index: usize, start: usize) -> Result<usize, FormattedTextError> {
    match node {
        Node::Leaf(leaf) => leaf
            .text()
            .bytes()
            .enumerate()
            .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
            .nth(index)
            .and_then(|offset| start.checked_add(offset))
            .ok_or(FormattedTextError::ArithmeticOverflow),
        Node::Branch(branch) => {
            let left_breaks = branch.left.aggregate().hard_line_breaks;
            if index < left_breaks {
                nth_line_break(&branch.left, index, start)
            } else {
                nth_line_break(
                    &branch.right,
                    index - left_breaks,
                    start
                        .checked_add(branch.left.aggregate().bytes)
                        .ok_or(FormattedTextError::ArithmeticOverflow)?,
                )
            }
        }
    }
}

fn count_line_breaks_before(node: &Node, offset: usize) -> usize {
    match node {
        Node::Leaf(leaf) => leaf.text().as_bytes()[..offset]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        Node::Branch(branch) => {
            let left_length = branch.left.aggregate().bytes;
            if offset <= left_length {
                count_line_breaks_before(&branch.left, offset)
            } else {
                branch.left.aggregate().hard_line_breaks
                    + count_line_breaks_before(&branch.right, offset - left_length)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn assert_invariants(tree: &FormattedTextTree) {
        fn visit(node: &Node) -> Aggregate {
            match node {
                Node::Leaf(leaf) => {
                    assert!(!leaf.text().is_empty());
                    assert!(leaf.buffer.is_char_boundary(leaf.range.start));
                    assert!(leaf.buffer.is_char_boundary(leaf.range.end));
                    assert_eq!(
                        leaf.hard_line_breaks,
                        leaf.text().bytes().filter(|byte| *byte == b'\n').count()
                    );
                    assert_eq!(leaf.utf16_units, leaf.text().encode_utf16().count());
                    node.aggregate()
                }
                Node::Branch(branch) => {
                    let left = visit(&branch.left);
                    let right = visit(&branch.right);
                    assert!(left.height.abs_diff(right.height) <= 1);
                    assert_eq!(branch.aggregate.bytes, left.bytes + right.bytes);
                    assert_eq!(
                        branch.aggregate.utf16_units,
                        left.utf16_units + right.utf16_units
                    );
                    assert_eq!(
                        branch.aggregate.hard_line_breaks,
                        left.hard_line_breaks + right.hard_line_breaks
                    );
                    assert_eq!(branch.aggregate.leaves, left.leaves + right.leaves);
                    assert_eq!(branch.aggregate.height, left.height.max(right.height) + 1);
                    branch.aggregate
                }
            }
        }
        if let Some(root) = &tree.root {
            let aggregate = visit(root);
            assert_eq!(aggregate.bytes, tree.byte_len());
            assert_eq!(aggregate.leaves, tree.leaf_count());
        } else {
            assert_eq!(tree.byte_len(), 0);
            assert_eq!(tree.leaf_count(), 0);
        }
    }

    fn flat_line_start(text: &str, line: usize) -> Option<usize> {
        if line == 0 {
            return Some(0);
        }
        text.match_indices('\n')
            .nth(line - 1)
            .map(|(offset, _)| offset + 1)
    }

    fn flat_line_end(text: &str, line: usize) -> Option<usize> {
        let start = flat_line_start(text, line)?;
        Some(
            text[start..]
                .find('\n')
                .map_or(text.len(), |relative| start + relative),
        )
    }

    #[test]
    fn construction_chunks_utf8_and_supports_logarithmic_lookups() {
        let text = format!("{}\nlast", "é".repeat(FORMATTED_TEXT_LEAF_BYTES));
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        assert_eq!(tree.flatten(), text);
        assert!(tree.leaf_count() > 1);
        assert_eq!(tree.hard_line_count(), 2);
        assert_eq!(
            tree.hard_line_start(1).unwrap(),
            text.find('\n').unwrap() + 1
        );
        assert_eq!(tree.hard_line_end(0).unwrap(), text.find('\n').unwrap());
        assert_eq!(tree.hard_line_end(1).unwrap(), text.len());
        assert_invariants(&tree);
    }

    #[test]
    fn splice_is_persistent_and_shares_untouched_leaf_identities() {
        let text = "0123456789".repeat(FORMATTED_TEXT_LEAF_BYTES);
        let original = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        let original_leaves = original.leaves();
        let middle = text.len() / 2;
        let changed = original.splice(middle..middle + 10, "changed").unwrap();

        assert_eq!(original.flatten(), text);
        let mut expected = text.clone();
        expected.replace_range(middle..middle + 10, "changed");
        assert_eq!(changed.flatten(), expected);
        let changed_leaves = changed.leaves();
        assert_eq!(
            original_leaves.first().unwrap(),
            changed_leaves.first().unwrap()
        );
        assert_eq!(
            original_leaves.last().unwrap().id,
            changed_leaves.last().unwrap().id
        );
        assert_eq!(
            original_leaves.last().unwrap().revision,
            changed_leaves.last().unwrap().revision
        );
        assert_invariants(&original);
        assert_invariants(&changed);
    }

    #[test]
    fn split_boundary_retains_identity_and_advances_local_revision() {
        let tree = FormattedTextTree::try_from_text("abcdefghij").unwrap();
        let original = tree.leaves()[0].clone();
        let changed = tree.splice(4..6, "X").unwrap();
        let leaves = changed.leaves();
        assert_eq!(leaves[0].id, original.id);
        assert_eq!(leaves[0].revision, FormattedLeafRevision(1));
        assert_ne!(leaves.last().unwrap().id, original.id);
        assert_eq!(leaves.last().unwrap().revision, FormattedLeafRevision(1));
    }

    #[test]
    fn editing_rejects_scalar_and_grapheme_splits() {
        let tree = FormattedTextTree::try_from_text("a\u{301}é").unwrap();
        assert_eq!(
            tree.splice(2..2, "x"),
            Err(FormattedTextError::NotCharBoundary(2))
        );
        assert_eq!(
            tree.splice(1..1, "x"),
            Err(FormattedTextError::NotGraphemeBoundary(1))
        );
        assert!(tree.splice(3..5, "e").is_ok());
    }

    #[test]
    fn unmarked_literal_crlf_remains_one_unicode_grapheme() {
        let tree = FormattedTextTree::try_from_text("\r\nrest").unwrap();
        assert!(!tree.is_grapheme_boundary(1).unwrap());
        assert_eq!(tree.next_grapheme_boundary(0).unwrap(), Some(2));
        assert_eq!(tree.previous_grapheme_boundary(2).unwrap(), Some(0));
        assert_eq!(
            tree.splice(1..1, "x"),
            Err(FormattedTextError::NotGraphemeBoundary(1))
        );
    }

    #[test]
    fn grapheme_cursor_crosses_leaf_boundaries_and_long_ri_context() {
        let mut text = "a".repeat(FORMATTED_TEXT_LEAF_BYTES - 1);
        text.push('x');
        text.push('\u{301}');
        text.push_str(&"🇺🇸".repeat(FORMATTED_TEXT_LEAF_BYTES));
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        let combining_boundary = FORMATTED_TEXT_LEAF_BYTES;
        assert!(!tree.is_grapheme_boundary(combining_boundary).unwrap());
        for (offset, _) in text.grapheme_indices(true).take(64) {
            assert!(tree.is_grapheme_boundary(offset).unwrap());
        }
    }

    #[test]
    fn leaf_boundary_affinity_is_explicit() {
        let text = "a".repeat(FORMATTED_TEXT_LEAF_BYTES * 2);
        let tree = FormattedTextTree::try_from_text(text).unwrap();
        let boundary = tree.leaves()[0].byte_range.end;
        let preceding = tree
            .locate_byte(boundary, LeafBoundarySide::Preceding)
            .unwrap()
            .unwrap();
        let following = tree
            .locate_byte(boundary, LeafBoundarySide::Following)
            .unwrap()
            .unwrap();
        assert_eq!(preceding.local_byte, preceding.leaf_range.len());
        assert_eq!(following.local_byte, 0);
        assert_ne!(preceding.id, following.id);
    }

    #[test]
    fn randomized_splices_match_a_flat_string_oracle() {
        let replacements = ["", "x", "é", "\n", "👩‍💻", "ab\ncd", "a\u{301}"];
        let mut oracle = String::from("alpha\nβeta\n👩‍💻 end");
        let mut tree = FormattedTextTree::try_from_text(oracle.as_str()).unwrap();
        let mut random = 0x9e37_79b9_7f4a_7c15_u64;

        for _ in 0..1_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let boundaries: Vec<_> = oracle
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain(std::iter::once(oracle.len()))
                .collect();
            let first = random as usize % boundaries.len();
            random = random.rotate_left(19).wrapping_mul(0x2545_f491_4f6c_dd1d);
            let second = random as usize % boundaries.len();
            let (start_index, end_index) = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            let range = boundaries[start_index]..boundaries[end_index];
            let replacement = replacements[(random >> 32) as usize % replacements.len()];
            tree = tree.splice(range.clone(), replacement).unwrap();
            oracle.replace_range(range, replacement);

            assert_eq!(tree.flatten(), oracle);
            assert_eq!(
                tree.hard_line_count(),
                oracle.bytes().filter(|byte| *byte == b'\n').count() + 1
            );
            for line in 0..tree.hard_line_count() {
                assert_eq!(
                    tree.hard_line_start(line).ok(),
                    flat_line_start(&oracle, line)
                );
                assert_eq!(tree.hard_line_end(line).ok(), flat_line_end(&oracle, line));
            }
            assert_invariants(&tree);
        }
    }

    #[test]
    fn million_short_lines_remain_balanced_and_indexable() {
        let text = "x\n".repeat(1_000_000);
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        assert_eq!(tree.byte_len(), 2_000_000);
        assert_eq!(tree.utf16_len(), 2_000_000);
        assert_eq!(tree.hard_line_count(), 1_000_001);
        assert_eq!(tree.hard_line_start(999_999).unwrap(), 1_999_998);
        assert_eq!(tree.hard_line_start(1_000_000).unwrap(), 2_000_000);
        assert_eq!(tree.hard_line_at_byte(1_234_567).unwrap(), 617_283);
        assert!(tree.height() < 20);
        assert_invariants(&tree);

        let edit_start = 1_000_000;
        let expected_line_count = tree.hard_line_count();
        let (changed, stats) = tree
            .splice_batch_with_stats(&[(edit_start..edit_start + 1, "yz")])
            .unwrap();
        assert_eq!(
            changed.slice(edit_start - 1..edit_start + 3).unwrap(),
            "\nyz\n"
        );
        assert_eq!(changed.hard_line_count(), expected_line_count);
        assert!(stats.nodes_visited <= 64, "{stats:?}");
        assert!(stats.nodes_copied <= 64, "{stats:?}");
        assert!(stats.leaves_copied <= 8, "{stats:?}");
        assert_eq!(tree.slice(edit_start - 1..edit_start + 2).unwrap(), "\nx\n");
        assert_invariants(&changed);
    }

    #[test]
    fn utf8_and_utf16_boundaries_map_through_tree_aggregates() {
        let text = format!(
            "{}Aé👩‍💻e\u{301}Z",
            "x".repeat(FORMATTED_TEXT_LEAF_BYTES - 2)
        );
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        assert!(tree.leaf_count() > 1);
        assert_eq!(tree.utf16_len(), text.encode_utf16().count());

        for byte in text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
        {
            let expected = text[..byte].encode_utf16().count();
            assert_eq!(tree.utf16_offset_for_byte(byte).unwrap(), expected);
            assert_eq!(tree.byte_offset_for_utf16(expected).unwrap(), byte);
        }

        let emoji = text.find('👩').unwrap();
        assert_eq!(
            tree.utf16_offset_for_byte(emoji + 1),
            Err(FormattedTextError::NotCharBoundary(emoji + 1))
        );
        let emoji_utf16 = text[..emoji].encode_utf16().count();
        assert_eq!(
            tree.byte_offset_for_utf16(emoji_utf16 + 1),
            Err(FormattedTextError::NotUtf16Boundary(emoji_utf16 + 1))
        );
        assert_eq!(
            tree.byte_offset_for_utf16(tree.utf16_len() + 1),
            Err(FormattedTextError::InvalidUtf16Offset {
                offset: tree.utf16_len() + 1,
                length: tree.utf16_len(),
            })
        );

        let changed = tree.splice(0..1, "🎉").unwrap();
        let mut expected = text;
        expected.replace_range(0..1, "🎉");
        assert_eq!(changed.utf16_len(), expected.encode_utf16().count());
        assert_eq!(
            changed.byte_offset_for_utf16(changed.utf16_len()).unwrap(),
            changed.byte_len()
        );
        assert_invariants(&changed);
    }

    #[test]
    fn million_line_regional_read_visits_only_its_tree_frontier() {
        let text = "x\n".repeat(1_000_000);
        let tree = FormattedTextTree::try_from_text(text).unwrap();
        let range = 1_765_431..1_765_447;
        let (slice, stats) = tree.slice_with_stats(range.clone()).unwrap();

        assert_eq!(slice, "\nx\nx\nx\nx\nx\nx\nx\nx");
        assert_eq!(stats.bytes_copied, range.len());
        assert!(stats.leaves_visited <= 2, "{stats:?}");
        assert!(
            stats.nodes_visited <= (tree.height() as usize * 4 + 4),
            "{stats:?}"
        );
    }

    #[test]
    fn one_extremely_long_line_is_balanced_and_editable() {
        let text = "é".repeat(1_000_000);
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        assert_eq!(tree.hard_line_count(), 1);
        assert!(tree.leaf_count() > 400);
        assert!(tree.height() < 20);
        let (changed, stats) = tree
            .splice_batch_with_stats(&[(1_000_000..1_000_002, "Z")])
            .unwrap();
        assert_eq!(changed.byte_len(), text.len() - 1);
        assert_eq!(changed.hard_line_count(), 1);
        assert!(stats.nodes_visited <= 64, "{stats:?}");
        assert!(stats.nodes_copied <= 64, "{stats:?}");
        assert!(stats.leaves_copied <= 8, "{stats:?}");
        assert_eq!(tree.slice(999_998..1_000_004).unwrap(), "ééé");
        assert_eq!(changed.slice(999_998..1_000_003).unwrap(), "éZé");
        assert_invariants(&changed);
    }

    #[test]
    fn dense_batch_recounts_each_original_slice_only_once() {
        let text = "**é👩‍💻** and _words_\n".repeat(8_000);
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        let edits = text
            .match_indices(['*', '_'])
            .map(|(at, _)| (at..at + 1, ""))
            .collect::<Vec<_>>();
        let (changed, stats) = tree.splice_prevalidated_batch_with_stats(&edits).unwrap();
        let expected = text.replace(['*', '_'], "");
        assert_eq!(changed.flatten(), expected);
        assert_eq!(tree.flatten(), text);
        assert_eq!(changed.utf16_len(), expected.encode_utf16().count());
        assert_eq!(changed.hard_line_count(), 8_001);
        assert!(stats.original_bytes_recounted <= text.len(), "{stats:?}");
        assert!(stats.nodes_visited < edits.len() * 3, "{stats:?}");
        assert_invariants(&changed);
    }

    #[test]
    fn batch_keeps_unaffected_leaves_and_original_backing_boundaries() {
        let text = "x\n".repeat(1_000_000);
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        let before = tree.leaves();
        let target = &before[before.len() / 2];
        let edits = (target.byte_range.start + 2..target.byte_range.end - 2)
            .step_by(4)
            .map(|at| (at..at + 1, "é"))
            .collect::<Vec<_>>();
        let boundary = target.byte_range.end - 1;
        let captured = tree
            .locate_byte(boundary, LeafBoundarySide::Following)
            .unwrap()
            .unwrap();
        let (changed, stats) = tree.splice_batch_with_stats(&edits).unwrap();
        let after = changed.leaves();
        for leaf in before.iter().filter(|leaf| leaf.id != target.id) {
            assert!(after
                .iter()
                .any(|other| other.id == leaf.id && other.revision == leaf.revision));
        }
        assert_eq!(
            changed.resolve_stable_boundary(
                captured.id,
                captured.revision,
                captured.buffer_id,
                captured.buffer_byte,
                LeafBoundarySide::Following
            ),
            Some(boundary + edits.len()),
        );
        assert!(
            stats.original_bytes_recounted <= FORMATTED_TEXT_LEAF_BYTES,
            "{stats:?}"
        );
        assert!(
            stats.nodes_visited < edits.len() * 5 + tree.height() as usize * 4,
            "{stats:?}"
        );
        let mut expected = text;
        for (range, replacement) in edits.iter().rev() {
            expected.replace_range(range.clone(), replacement);
        }
        assert_eq!(changed.flatten(), expected);
        assert_invariants(&changed);
    }

    #[test]
    fn batch_matches_old_snapshot_splices_across_leaf_and_edit_boundaries() {
        let text = "aé👩‍💻\n".repeat(2_000);
        let boundaries = text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let original = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        let large = "large é\n".repeat(2_000);
        let replacements = ["", "X", "é\n", "\u{301}", large.as_str()];
        let mut seed = 17_u64;
        for case in 0..48 {
            let mut edits = Vec::new();
            let mut at = 0;
            while at + 1 < boundaries.len() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                at = (at + (seed as usize % 401)).min(boundaries.len() - 1);
                let end = (at + ((seed >> 32) as usize % 601)).min(boundaries.len() - 1);
                edits.push((
                    boundaries[at]..boundaries[end],
                    replacements[(case + edits.len()) % replacements.len()],
                ));
                at = end + 1;
            }
            if edits
                .last()
                .is_some_and(|(range, _)| range.start == text.len())
            {
                edits.pop();
            }
            edits.push((text.len()..text.len(), "END"));
            let (changed, stats) = original
                .splice_prevalidated_batch_with_stats(&edits)
                .unwrap();
            let mut expected = text.clone();
            for (range, replacement) in edits.iter().rev() {
                expected.replace_range(range.clone(), replacement);
            }
            assert_eq!(changed.flatten(), expected, "case {case}");
            assert_eq!(changed.utf16_len(), expected.encode_utf16().count());
            assert!(
                stats.original_bytes_recounted <= text.len(),
                "case {case}: {stats:?}"
            );
            assert_invariants(&changed);
        }
        // Adjacent insertion/replacement endpoints preserve their declared order.
        let tree = FormattedTextTree::try_from_text("abcd").unwrap();
        let edits = [
            (0..0, "<"),
            (0..2, "A"),
            (2..2, "|"),
            (2..4, "B"),
            (4..4, ">"),
        ];
        assert_eq!(
            tree.splice_prevalidated_batch(&edits).unwrap().flatten(),
            "<A|B>"
        );
        assert_eq!(
            FormattedTextTree::default()
                .splice_prevalidated_batch(&[(0..0, "é")])
                .unwrap()
                .flatten(),
            "é"
        );
        let (unchanged, stats) = tree
            .splice_prevalidated_batch_with_stats(&[(2..2, "")])
            .unwrap();
        assert!(Arc::ptr_eq(
            tree.root.as_ref().unwrap(),
            unchanged.root.as_ref().unwrap()
        ));
        assert_eq!(stats, FormattedTextSpliceStats::default());
    }

    #[test]
    fn batch_validates_original_boundaries_before_forming_new_graphemes() {
        let tree = FormattedTextTree::try_from_text("a b").unwrap();
        let (joined, _) = tree
            .splice_batch_with_stats(&[(1..2, ""), (2..3, "\u{301}")])
            .unwrap();
        assert_eq!(joined.flatten(), "a\u{301}");
        assert!(!joined.is_grapheme_boundary(1).unwrap());
        assert_eq!(tree.flatten(), "a b");
        assert!(matches!(
            tree.splice_prevalidated_batch(&[(0..2, ""), (1..3, "")]),
            Err(FormattedTextError::OverlappingSplices { .. }),
        ));
        assert!(matches!(
            tree.splice_prevalidated_batch(&[(1..1, "a"), (1..1, "b")]),
            Err(FormattedTextError::OverlappingSplices { .. }),
        ));
        let crlf = FormattedTextTree::try_from_text("\r\né").unwrap();
        assert_eq!(
            crlf.splice_batch_with_stats(&[(1..2, "")]),
            Err(FormattedTextError::NotGraphemeBoundary(1)),
        );
        assert_eq!(
            crlf.splice_prevalidated_batch(&[(1..2, "")])
                .unwrap()
                .flatten(),
            "\ré",
        );
        assert_eq!(
            crlf.splice_prevalidated_batch(&[(0..1, ""), (3..4, "x")]),
            Err(FormattedTextError::NotCharBoundary(3)),
        );
        assert_eq!(crlf.flatten(), "\r\né");
    }

    #[test]
    fn structured_lookup_errors_do_not_clamp() {
        let tree = FormattedTextTree::try_from_text("one\ntwo").unwrap();
        assert!(matches!(
            tree.hard_line_start(2),
            Err(FormattedTextError::HardLineOutOfBounds {
                line: 2,
                line_count: 2
            })
        ));
        assert!(matches!(
            tree.slice(9..9),
            Err(FormattedTextError::InvalidRange { .. })
        ));
        assert!(matches!(
            tree.locate_byte(9, LeafBoundarySide::Following),
            Err(FormattedTextError::InvalidByteOffset { .. })
        ));
    }
}
