//! Persistent range-indexed storage used by formatted projections.
//!
//! Records live in bounded immutable leaves under balanced aggregate trees.
//! Snapshot and history clones share the root. After full or regional
//! projection has reconciled stable identities, equal leaves and their
//! unchanged ancestor nodes are reused. Regional projection splices these
//! indexes directly; flat slice access is a lazy compatibility view.
//!
//! Blocks form an ordered partition, so aggregate pruning reports blocks that
//! touch a query in `O(log n + k)`. Style spans may overlap; max-end augmented
//! nodes report strict intersections in `O(log n + k)`. The public slice APIs
//! are backed by a lazy compatibility cache and are not used for regional
//! layout queries.

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use crate::persistent::{self, Item, Node, Summary, Tag, Tree, View, Work};

pub(super) const RANGE_INDEX_LEAF_ITEMS: usize = 64;

/// Optional structural navigation keys. Minima/maxima and flags compose under
/// persistent splices, allowing callers to skip unrelated subtrees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NavigationSummary {
    pub minimum: u16,
    pub maximum: u16,
    pub minimum_start: u16,
    pub flags: u16,
}
impl Default for NavigationSummary {
    fn default() -> Self { Self { minimum: u16::MAX, maximum: 0, minimum_start: u16::MAX, flags: 0 } }
}
impl NavigationSummary {
    fn union(self, other: Self) -> Self {
        Self { minimum: self.minimum.min(other.minimum), maximum: self.maximum.max(other.maximum),
            minimum_start: self.minimum_start.min(other.minimum_start), flags: self.flags | other.flags }
    }
}

pub(super) trait RangedItem {
    fn navigation_summary(&self) -> NavigationSummary { NavigationSummary::default() }

    /// An independently shifted coordinate, when this record needs reverse
    /// lookup through the same tree. Callers use ordered auxiliary lookups
    /// only when item order is monotonic in this coordinate as well.
    fn auxiliary_range(&self) -> Option<&Range<usize>> { None }

    fn owned_heap_bytes(&self) -> usize { 0 }

    fn visit_shared_memory(&self, _visitor: &mut super::history_memory::MemoryVisitor<'_>) {}

    fn range(&self) -> &Range<usize>;

    fn with_range(&self, range: Range<usize>) -> Self;

    /// Apply lazily accumulated non-primary coordinate and snapshot changes.
    /// Most indexed records have neither and use this default.
    fn with_transform(
        &self,
        range: Range<usize>,
        _auxiliary_shift: i128,
        _revision: Option<u64>,
    ) -> Option<Self>
    where
        Self: Sized,
    {
        Some(self.with_range(range))
    }
}

pub(super) struct OrderedRangeStore<T: Clone + RangedItem> {
    inner: PersistentRangeStore<T>,
}

impl<T: Clone + RangedItem> OrderedRangeStore<T> {
    /// Conservative O(1) identity comparison, including lazy transforms.
    pub(super) fn shares_storage_with(&self, other: &Self) -> bool {
        self.inner.shares_storage_with(&other.inner)
    }

    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        self.inner.visit_retained_memory(visitor);
    }

    pub(super) fn new(items: Vec<T>) -> Self {
        Self {
            inner: PersistentRangeStore::new(items),
        }
    }

    /// Translate a complete immutable tree without copying its records.
    pub(super) fn shifted(&self, text_delta: i128, source_delta: i128) -> Option<Self> {
        let mut inner = self.inner.clone();
        if !inner.tree.is_empty() {
            inner.origin = usize::try_from(inner.origin as i128 + text_delta).ok()?;
            if source_delta != 0 {
                let lazy = Lazy { auxiliary_shift: source_delta, revision: None };
                inner.tree = inner.tree.tagged(&lazy, &mut Work::default());
            }
        }
        inner.compatibility_flat = OnceLock::new();
        Some(Self { inner })
    }

    /// A flat copy of every record, built on first use and then retained
    /// with this snapshot, including in history. This is the only whole-store
    /// slice view; editing paths use `items`, `get_range`, `partition_point`
    /// or a one-off `to_vec` instead. Work statistics count each new copy.
    pub(super) fn compatibility_slice(&self) -> &[T] {
        self.inner.compatibility_slice()
    }

    pub(super) fn to_vec(&self) -> Vec<T> {
        self.inner.to_vec()
    }

    /// Records in order, read in bounded chunks without retaining a flat copy.
    pub(super) fn items(&self) -> impl Iterator<Item = T> + '_ {
        const CHUNK: usize = 256;
        let len = self.len();
        (0..len).step_by(CHUNK).flat_map(move |start| {
            self.get_range(&(start..(start + CHUNK).min(len))).expect("chunk lies inside the store")
        })
    }

    pub(super) fn len(&self) -> usize {
        self.inner.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    pub(super) fn get(&self, index: usize) -> Option<T> {
        self.inner.get(index)
    }



    /// Materializes one half-open ordinal item range with one tree descent and
    /// a sequential walk of the covered leaves.
    pub(super) fn get_range(&self, indices: &Range<usize>) -> Option<Vec<T>> {
        self.inner.get_range(indices, None)
    }

    /// Number of items whose range starts before `offset`, by tree descent.
    pub(super) fn partition_point_start(&self, offset: usize) -> usize {
        self.inner.partition_point_start(offset)
    }

    /// First record ending strictly after an offset in ordered, nonoverlapping
    /// storage. The subtree max-end summaries make this one tree descent.
    pub(super) fn partition_point_end(&self, offset: usize) -> usize {
        self.inner.first_index_ending_after(offset)
    }

    /// `slice::partition_point` by ordinal search. Unlike the slice method
    /// `compatibility_slice`, this keeps no flat copy of the store.
    pub(super) fn partition_point(&self, pred: impl FnMut(&T) -> bool) -> usize {
        self.inner.partition_point(pred)
    }

    /// Persistently replace an ordinal region. Replacement records use final
    /// absolute coordinates; the untouched suffix is translated lazily by the
    /// change between `old_coordinate_end` and `new_coordinate_end`.
    pub(super) fn splice(
        &self,
        indices: Range<usize>,
        replacement: Vec<T>,
        old_coordinate_end: usize,
        new_coordinate_end: usize,
        stats: &mut RangeSpliceStats,
    ) -> Option<Self> {
        self.inner
            .splice(
                indices,
                replacement,
                old_coordinate_end,
                new_coordinate_end,
                None,
                None,
                stats,
            )
            .map(|inner| Self { inner })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn splice_transformed(&self, indices: Range<usize>, replacement: Vec<T>,
        old_coordinate_end: usize, new_coordinate_end: usize, auxiliary_ends: Option<(usize, usize)>,
        revision: Option<u64>, stats: &mut RangeSpliceStats) -> Option<Self> {
        self.inner.splice(indices, replacement, old_coordinate_end, new_coordinate_end, auxiliary_ends, revision, stats)
            .map(|inner| Self { inner })
    }

    /// Finds the last ordered range whose start is at or before `offset` and
    /// whose end is at or after it. This gives deterministic downstream
    /// behavior for adjacent zero-width boundaries in `O(log n)` time.
    pub(super) fn index_touching_point(&self, offset: usize) -> Option<usize> {
        self.index_touching_point_with_stats(offset).0
    }

    pub(super) fn index_touching_point_with_stats(
        &self,
        offset: usize,
    ) -> (Option<usize>, QueryStats) {
        let mut stats = QueryStats::default();
        let result = self.inner.root().and_then(|(root, place)| {
            find_touching_index(root, place.origin, offset, 0, &mut stats)
        });
        (result, stats)
    }

    /// Reports blocks whose closed boundary extents touch `query`. Including a
    /// block ending exactly at the query start preserves empty-paragraph and
    /// paragraph-boundary geometry.
    pub(super) fn query_touching(&self, query: &Range<usize>) -> Vec<T> {
        self.inner.query(query, Intersection::Touching, None)
    }

    /// Find the first/last matching structural record. The predicate must be
    /// false for an aggregate only when no descendant can match it.
    pub(super) fn find_navigation(&self, indices: Range<usize>, reverse: bool,
        matches: impl Fn(NavigationSummary) -> bool) -> Option<usize> {
        fn find<T: Clone + RangedItem>(node: &RangeNode<T>, base: usize, range: &Range<usize>, reverse: bool,
            matches: &impl Fn(NavigationSummary) -> bool) -> Option<usize> {
            super::work_statistics::record(|work| work.list_capability_nodes_visited += 1);
            let extent = node.summary();
            if base >= range.end || base + extent.items <= range.start || !matches(extent.navigation) { return None; }
            match node.view() {
                View::Leaf(chunk) => {
                    let items = &chunk.items;
                    let mut indices = range.start.saturating_sub(base)..items.len().min(range.end - base);
                    let mut visit = |index: usize| {
                        super::work_statistics::record(|work| work.list_capability_blocks_visited += 1);
                        matches(items[index].navigation_summary()).then_some(base + index)
                    };
                    if reverse { indices.rev().find_map(visit) } else { indices.find_map(&mut visit) }
                }
                View::Branch(left, right) => {
                    let right_base = base + left.summary().items;
                    if reverse {
                        find(right, right_base, range, reverse, matches).or_else(|| find(left, base, range, reverse, matches))
                    } else {
                        find(left, base, range, reverse, matches).or_else(|| find(right, right_base, range, reverse, matches))
                    }
                }
            }
        }
        self.inner.tree.root().and_then(|root| find(root, 0, &indices, reverse, &matches))
    }

    /// Reuses equal leaves and unchanged ancestor nodes from `previous`.
    pub(super) fn reuse_equal_chunks(&mut self, previous: &Self)
    where
        T: PartialEq,
    {
        self.inner.reuse_equal_chunks(&previous.inner);
    }

    #[cfg(test)]
    pub(super) fn query_touching_with_stats(&self, query: &Range<usize>) -> (Vec<T>, QueryStats) {
        let mut stats = QueryStats::default();
        let matches = self
            .inner
            .query(query, Intersection::Touching, Some(&mut stats));
        (matches, stats)
    }

    #[cfg(test)]
    pub(super) fn get_range_with_stats(
        &self,
        indices: &Range<usize>,
    ) -> Option<(Vec<T>, QueryStats)> {
        let mut stats = QueryStats::default();
        let items = self.inner.get_range(indices, Some(&mut stats))?;
        Some((items, stats))
    }



    #[cfg(test)]
    pub(super) fn shared_leaf_count_with(&self, other: &Self) -> usize {
        self.inner.shared_leaf_count_with(&other.inner)
    }

    #[cfg(test)]
    pub(super) fn leaf_count(&self) -> usize {
        self.inner.leaf_count()
    }

    #[cfg(test)]
    pub(super) fn invariant_holds(&self) -> bool {
        self.inner.invariant_holds(true)
    }
}

impl<T: Clone + RangedItem> Clone for OrderedRangeStore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone + RangedItem + fmt::Debug> fmt::Debug for OrderedRangeStore<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.to_vec()).finish()
    }
}

impl<T: Clone + RangedItem + PartialEq> PartialEq for OrderedRangeStore<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.items_equal(&other.inner)
    }
}

pub(super) struct IntervalRangeStore<T: Clone + RangedItem> {
    inner: PersistentRangeStore<T>,
}

impl<T: Clone + RangedItem> IntervalRangeStore<T> {
    pub(super) fn partition_point_auxiliary_start(&self, offset: usize) -> usize {
        self.inner.tree.root().map_or(0, |root| count_auxiliary_starts_before(root, 0, offset))
    }

    pub(super) fn query_auxiliary_touching(&self, query: &Range<usize>) -> Vec<T> {
        let mut output = Vec::new();
        if let Some((root, place)) = self.inner.root() {
            query_auxiliary_node(root, place, query, &mut output, None);
        }
        output
    }

    #[cfg(test)]
    pub(super) fn query_auxiliary_touching_with_stats(&self, query: &Range<usize>) -> (Vec<T>, QueryStats) {
        let mut output = Vec::new();
        let mut stats = QueryStats::default();
        if let Some((root, place)) = self.inner.root() {
            query_auxiliary_node(root, place, query, &mut output, Some(&mut stats));
        }
        (output, stats)
    }

    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        self.inner.visit_retained_memory(visitor);
    }

    pub(super) fn new(mut items: Vec<T>) -> Self {
        // Canonical ordering makes query output deterministic. Stable sorting
        // retains the declared cascade order for spans with the same start.
        items.sort_by_key(|item| item.range().start);
        Self {
            inner: PersistentRangeStore::new(items),
        }
    }

    /// A flat copy of every record, built on first use and then retained
    /// with this snapshot, including in history. This is the only whole-store
    /// slice view; editing paths use `items`, `get_range`, `partition_point`
    /// or a one-off `to_vec` instead. Work statistics count each new copy.
    pub(super) fn compatibility_slice(&self) -> &[T] {
        self.inner.compatibility_slice()
    }

    pub(super) fn to_vec(&self) -> Vec<T> {
        self.inner.to_vec()
    }

    /// Items in order, read in bounded chunks. Unlike `compatibility_slice`, this keeps
    /// no flat copy alive after a whole-store pass.
    pub(super) fn items(&self) -> impl Iterator<Item = T> + '_ {
        const CHUNK: usize = 256;
        let len = self.len();
        (0..len).step_by(CHUNK).flat_map(move |start| {
            self.get_range(&(start..(start + CHUNK).min(len))).expect("chunk lies inside the store")
        })
    }

    pub(super) fn len(&self) -> usize {
        self.inner.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Reports spans with a non-empty intersection with `query`.
    pub(super) fn query_overlapping(&self, query: &Range<usize>) -> Vec<T> {
        self.inner.query(query, Intersection::Overlapping, None)
    }

    pub(super) fn query_touching(&self, query: &Range<usize>) -> Vec<T> {
        self.inner.query(query, Intersection::Touching, None)
    }

    pub(super) fn partition_point_start(&self, offset: usize) -> usize {
        self.inner.partition_point_start(offset)
    }

    /// The first index whose span ends after `offset`, or the item count.
    /// Max-end aggregates prune subtrees ending at or before `offset`, so
    /// non-overlapping spans resolve this in `O(log n)`.
    pub(super) fn first_index_ending_after(&self, offset: usize) -> usize {
        self.inner.first_index_ending_after(offset)
    }

    /// The contiguous index span containing every item that strictly
    /// overlaps `query` (an empty query matches items strictly containing its
    /// offset). Overlapping stores may include non-matching items inside it.
    pub(super) fn overlapping_index_span(&self, query: &Range<usize>) -> Range<usize> {
        let first = self.first_index_ending_after(query.start);
        let last = self.partition_point_start(query.end).max(first);
        first..last
    }

    pub(super) fn get(&self, index: usize) -> Option<T> {
        self.inner.get(index)
    }

    pub(super) fn get_range(&self, indices: &Range<usize>) -> Option<Vec<T>> {
        self.inner.get_range(indices, None)
    }

    pub(super) fn splice(
        &self,
        indices: Range<usize>,
        mut replacement: Vec<T>,
        old_coordinate_end: usize,
        new_coordinate_end: usize,
        stats: &mut RangeSpliceStats,
    ) -> Option<Self> {
        replacement.sort_by_key(|item| item.range().start);
        self.inner
            .splice(
                indices,
                replacement,
                old_coordinate_end,
                new_coordinate_end,
                None,
                None,
                stats,
            )
            .map(|inner| Self { inner })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn splice_transformed(
        &self,
        indices: Range<usize>,
        mut replacement: Vec<T>,
        old_coordinate_end: usize,
        new_coordinate_end: usize,
        auxiliary_ends: Option<(usize, usize)>,
        revision: Option<u64>,
        stats: &mut RangeSpliceStats,
    ) -> Option<Self> {
        replacement.sort_by_key(|item| item.range().start);
        self.inner
            .splice(
                indices,
                replacement,
                old_coordinate_end,
                new_coordinate_end,
                auxiliary_ends,
                revision,
                stats,
            )
            .map(|inner| Self { inner })
    }

    pub(super) fn reuse_equal_chunks(&mut self, previous: &Self)
    where
        T: PartialEq,
    {
        self.inner.reuse_equal_chunks(&previous.inner);
    }

    #[cfg(test)]
    pub(super) fn query_overlapping_with_stats(
        &self,
        query: &Range<usize>,
    ) -> (Vec<T>, QueryStats) {
        let mut stats = QueryStats::default();
        let matches = self
            .inner
            .query(query, Intersection::Overlapping, Some(&mut stats));
        (matches, stats)
    }

    #[cfg(test)]
    pub(super) fn invariant_holds(&self) -> bool {
        self.inner.invariant_holds(false)
    }
}

impl<T: Clone + RangedItem> Clone for IntervalRangeStore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone + RangedItem + fmt::Debug> fmt::Debug for IntervalRangeStore<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.to_vec()).finish()
    }
}

impl<T: Clone + RangedItem + PartialEq> PartialEq for IntervalRangeStore<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.items_equal(&other.inner)
    }
}

/// A lazily applied change to every record of a subtree: a shift of the
/// auxiliary coordinate and, when set, a newer snapshot revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Lazy {
    auxiliary_shift: i128,
    revision: Option<u64>,
}

impl Lazy {
    /// This change followed by the later change `outer`.
    fn under(self, outer: Self) -> Self {
        Self {
            auxiliary_shift: self.auxiliary_shift + outer.auxiliary_shift,
            revision: outer.revision.or(self.revision),
        }
    }
}

impl<T: Clone + RangedItem> Tag<Chunk<T>> for Lazy {
    fn then(&mut self, later: &Self) {
        *self = self.under(*later);
    }

    fn item(&self, chunk: &Chunk<T>) -> Chunk<T> {
        Chunk { lazy: chunk.lazy.under(*self), ..chunk.clone() }
    }

    fn summary(&self, summary: &mut Extent) {
        summary.auxiliary = shifted_auxiliary_bounds(&summary.auxiliary, self.auxiliary_shift);
    }
}

/// One bounded leaf of records. Their ranges are relative to the chunk's
/// origin, the start of its first record, so an edit moves the records after
/// it by changing one `advance`, and equal chunks compare equal wherever they
/// lie.
#[derive(Clone)]
struct Chunk<T> {
    items: Arc<[T]>,
    /// Distance to the following chunk's origin; zero in a store's last chunk.
    advance: usize,
    /// Not yet applied to `items`.
    lazy: Lazy,
}

impl<T: PartialEq> PartialEq for Chunk<T> {
    fn eq(&self, other: &Self) -> bool {
        self.advance == other.advance
            && self.lazy == other.lazy
            && (Arc::ptr_eq(&self.items, &other.items) || self.items == other.items)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Extent {
    items: usize,
    /// Distance from the first chunk's origin to the origin after the last.
    span: usize,
    /// Greatest record end, relative to the first chunk's origin.
    max_end: usize,
    navigation: NavigationSummary,
    auxiliary: Option<Range<usize>>,
}

impl Summary for Extent {
    fn add(&mut self, right: &Self) {
        self.max_end = self.max_end.max(self.span + right.max_end);
        self.items += right.items;
        self.span += right.span;
        self.navigation = self.navigation.union(right.navigation);
        self.auxiliary = self.auxiliary.iter().chain(&right.auxiliary).cloned().reduce(union);
    }
}

fn union(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
}

fn shifted_auxiliary_bounds(bounds: &Option<Range<usize>>, shift: i128) -> Option<Range<usize>> {
    bounds.as_ref().map(|bounds| {
        usize::try_from(bounds.start as i128 + shift)
            .expect("range-index auxiliary start is representable")
            ..usize::try_from(bounds.end as i128 + shift)
                .expect("range-index auxiliary end is representable")
    })
}

impl<T: Clone + RangedItem> Item for Chunk<T> {
    type Summary = Extent;
    type Tag = Lazy;

    fn summary(&self) -> Extent {
        Extent {
            items: self.items.len(),
            span: self.advance,
            max_end: self.items.iter().map(|item| item.range().end).max().unwrap_or(0),
            navigation: self.items.iter().fold(NavigationSummary::default(), |value, item| {
                value.union(item.navigation_summary())
            }),
            auxiliary: shifted_auxiliary_bounds(
                &self.items.iter().filter_map(RangedItem::auxiliary_range).cloned().reduce(union),
                self.lazy.auxiliary_shift,
            ),
        }
    }
}

impl<T: Clone + RangedItem> Chunk<T> {
    /// A chunk of ordered records in absolute coordinates starting at `origin`.
    fn new(items: &[T], origin: usize, advance: usize) -> Self {
        let relative = |offset: usize| {
            offset.checked_sub(origin).expect("range-index input is ordered by start boundary")
        };
        let items = items
            .iter()
            .map(|item| item.with_range(relative(item.range().start)..relative(item.range().end)))
            .collect();
        Self { items, advance, lazy: Lazy::default() }
    }

    /// The record at `index` as its owner sees it.
    fn item(&self, index: usize, place: Place) -> T {
        let item = &self.items[index];
        let lazy = self.lazy.under(place.lazy);
        item.with_transform(
            place.origin + item.range().start..place.origin + item.range().end,
            lazy.auxiliary_shift,
            lazy.revision,
        )
        .expect("validated lazy range-index transformation is representable")
    }

    /// The records before and from `at`, which lies strictly inside.
    fn cut(&self, at: usize) -> (Self, Self) {
        let start = self.items[at].range().start;
        let after = self.items[at..]
            .iter()
            .map(|item| item.with_range(item.range().start - start..item.range().end - start))
            .collect();
        (
            Self { items: Arc::from(&self.items[..at]), advance: start, lazy: self.lazy },
            // A store's last chunk has no following origin to keep.
            Self { items: after, advance: self.advance.saturating_sub(start), lazy: self.lazy },
        )
    }
}

type RangeNode<T> = Node<Chunk<T>>;

/// Where a subtree lies and what its ancestors have yet to apply to it.
#[derive(Clone, Copy, Default)]
struct Place {
    origin: usize,
    lazy: Lazy,
}

impl Place {
    fn children<T: Clone + RangedItem>(self, node: &RangeNode<T>, left: &RangeNode<T>) -> (Self, Self) {
        let lazy = node.tag().map_or(self.lazy, |tag| tag.under(self.lazy));
        (Self { origin: self.origin, lazy }, Self { origin: self.origin + left.summary().span, lazy })
    }
}

/// A run of chunks and the origin of its first one.
struct Piece<T: Clone + RangedItem> {
    tree: Tree<Chunk<T>>,
    origin: usize,
}

struct PersistentRangeStore<T: Clone + RangedItem> {
    tree: Tree<Chunk<T>>,
    /// Start of the first record; zero when empty.
    origin: usize,
    compatibility_flat: OnceLock<Arc<Vec<T>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RangeSpliceStats {
    pub(super) nodes_visited: usize,
    pub(super) nodes_copied: usize,
    pub(super) leaves_copied: usize,
    pub(super) items_copied: usize,
}

fn item_count(extent: &Extent) -> usize {
    extent.items
}

impl<T: Clone + RangedItem> PersistentRangeStore<T> {
    fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        fn visit<T: Clone + RangedItem>(
            node: &Arc<RangeNode<T>>,
            visitor: &mut super::history_memory::MemoryVisitor<'_>,
        ) {
            match node.view() {
                // One entry covers a leaf and its records, which keeps the
                // accounting table at one entry for each leaf. A leaf node
                // that shares another's records is the only part left out.
                View::Leaf(chunk) => {
                    let owned = chunk.items.iter().map(RangedItem::owned_heap_bytes).sum::<usize>();
                    visitor.arc_owning(&chunk.items, owned + size_of::<RangeNode<T>>() + 16, |visitor| {
                        for item in chunk.items.iter() {
                            item.visit_shared_memory(visitor);
                        }
                    });
                }
                View::Branch(left, right) => visitor.arc(node, |visitor| {
                    visit(left, visitor);
                    visit(right, visitor);
                }),
            }
        }
        if let Some(root) = self.tree.root() {
            visit(root, visitor);
        }
        // The compatibility cache may be populated after a snapshot was first
        // retained. It is a separate root, so refreshing history can discover
        // it without revisiting the immutable range tree.
        if let Some(flat) = self.compatibility_flat.get() {
            visitor.arc(flat, |visitor| {
                visitor.vector(flat, flat.iter().map(RangedItem::owned_heap_bytes).sum());
                for item in flat.iter() { item.visit_shared_memory(visitor); }
            });
        }
    }

    fn new(items: Vec<T>) -> Self {
        // Release each input record as its bounded chunk is built. Cloning
        // all chunks first keeps a second whole-document set of owned table,
        // style and provenance payloads alive during projection construction.
        let mut items = items.into_iter().peekable();
        let origin = items.peek().map_or(0, |item| item.range().start);
        let mut chunks = Vec::with_capacity(items.len().div_ceil(RANGE_INDEX_LEAF_ITEMS));
        let mut chunk_origin = origin;
        while items.peek().is_some() {
            let chunk = items.by_ref().take(RANGE_INDEX_LEAF_ITEMS).collect::<Vec<_>>();
            let next = items.peek().map_or(chunk_origin, |item| item.range().start);
            chunks.push(Chunk::new(&chunk, chunk_origin, next.checked_sub(chunk_origin)
                .expect("range-index input is ordered by start boundary")));
            chunk_origin = next;
        }
        Self {
            tree: Tree::from_items(chunks, &mut Work::default()),
            origin,
            compatibility_flat: OnceLock::new(),
        }
    }

    fn len(&self) -> usize {
        self.tree.measure(item_count)
    }

    fn root(&self) -> Option<(&RangeNode<T>, Place)> {
        Some((self.tree.root()?, Place { origin: self.origin, lazy: Lazy::default() }))
    }

    fn partition_point(&self, mut pred: impl FnMut(&T) -> bool) -> usize {
        if let Some(flat) = self.compatibility_flat.get() {
            return flat.partition_point(pred);
        }
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if pred(&self.get(middle).expect("ordinal inside the store")) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    /// The compatibility copy, built on first use and retained by this
    /// snapshot (and so by every history state that shares it).
    fn compatibility_slice(&self) -> &[T] {
        self.compatibility_flat
            .get_or_init(|| {
                super::work_statistics::record(|stats| {
                    stats.compatibility_record_materializations += 1;
                    stats.compatibility_records_materialized += self.len();
                });
                Arc::new(self.to_vec())
            })
            .as_slice()
    }

    fn to_vec(&self) -> Vec<T> {
        self.get_range(&(0..self.len()), None).expect("the whole store is inside it")
    }

    fn get(&self, index: usize) -> Option<T> {
        let (mut node, mut place) = self.root()?;
        let mut index = (index < self.len()).then_some(index)?;
        loop {
            match node.view() {
                View::Leaf(chunk) => return Some(chunk.item(index, place)),
                View::Branch(left, right) => {
                    let places = place.children(node, left);
                    let before = left.summary().items;
                    (node, place) = if index < before { (left, places.0) } else { (right, places.1) };
                    index -= if index < before { 0 } else { before };
                }
            }
        }
    }

    fn partition_point_start(&self, offset: usize) -> usize {
        self.root().map_or(0, |(root, place)| count_starts_before(root, place.origin, offset))
    }

    fn first_index_ending_after(&self, offset: usize) -> usize {
        self.root()
            .and_then(|(root, place)| first_index_ending_after(root, place.origin, offset, 0))
            .unwrap_or(self.len())
    }

    #[allow(clippy::too_many_arguments)]
    fn splice(
        &self,
        indices: Range<usize>,
        replacement: Vec<T>,
        old_coordinate_end: usize,
        new_coordinate_end: usize,
        auxiliary_ends: Option<(usize, usize)>,
        revision: Option<u64>,
        stats: &mut RangeSpliceStats,
    ) -> Option<Self> {
        if indices.start > indices.end || indices.end > self.len() {
            return None;
        }
        let mut work = Work::default();
        let mut split = |piece: &Piece<T>, at: usize| {
            let (before, after) = piece.tree.split_at(
                at,
                item_count,
                |chunk, at| {
                    stats.items_copied = stats.items_copied.saturating_add(chunk.items.len());
                    stats.leaves_copied = stats.leaves_copied.saturating_add(2);
                    chunk.cut(at)
                },
                &mut work,
            );
            let origin = piece.origin + before.measure(|extent| extent.span);
            (Piece { tree: before, origin: piece.origin }, Piece { tree: after, origin })
        };
        let (before, rest) = split(&Piece { tree: self.tree.clone(), origin: self.origin }, indices.start);
        let (_, mut after) = split(&rest, indices.len());
        stats.nodes_visited = stats.nodes_visited.saturating_add(work.visited);

        let inserted = Self::new(replacement);
        stats.items_copied = stats.items_copied.saturating_add(inserted.len());
        stats.leaves_copied =
            stats.leaves_copied.saturating_add(inserted.len().div_ceil(RANGE_INDEX_LEAF_ITEMS));
        // The untouched suffix moves without copying its records.
        if !after.tree.is_empty() {
            after.origin = if new_coordinate_end >= old_coordinate_end {
                after.origin.checked_add(new_coordinate_end - old_coordinate_end)?
            } else {
                after.origin.checked_sub(old_coordinate_end - new_coordinate_end)?
            };
        }
        if let Some((old_end, new_end)) = auxiliary_ends.filter(|(old_end, new_end)| old_end != new_end) {
            let auxiliary_shift = i128::try_from(new_end).ok()? - i128::try_from(old_end).ok()?;
            after.tree = after.tree.tagged(&Lazy { auxiliary_shift, revision: None }, &mut work);
        }
        let inserted = Piece { tree: inserted.tree, origin: inserted.origin };
        let joined = join(join(before, inserted, &mut work, stats)?, after, &mut work, stats)?;
        let mut tree = joined.tree;
        if let Some(last) = tree.last() {
            let span = tree.measure(|extent| extent.span) - last.advance;
            tree = with_span(&tree, span, &mut work)?;
        }
        if revision.is_some() {
            tree = tree.tagged(&Lazy { auxiliary_shift: 0, revision }, &mut work);
        }
        stats.nodes_copied = stats.nodes_copied.saturating_add(work.created);
        Some(Self {
            origin: if tree.is_empty() { 0 } else { joined.origin },
            tree,
            compatibility_flat: OnceLock::new(),
        })
    }

    fn get_range(&self, indices: &Range<usize>, stats: Option<&mut QueryStats>) -> Option<Vec<T>> {
        if indices.start > indices.end || indices.end > self.len() {
            return None;
        }
        let mut items = Vec::with_capacity(indices.end - indices.start);
        if let Some((root, place)) = self.root() {
            append_item_range(root, place, 0, indices, &mut items, stats);
        }
        Some(items)
    }

    fn query(
        &self,
        query: &Range<usize>,
        intersection: Intersection,
        stats: Option<&mut QueryStats>,
    ) -> Vec<T> {
        let mut matches = Vec::new();
        if let Some((root, place)) = self.root() {
            query_node(root, place, query, intersection, &mut matches, stats);
        }
        matches
    }

    fn reuse_equal_chunks(&mut self, previous: &Self)
    where
        T: PartialEq,
    {
        if self.tree.is_empty() || previous.tree.is_empty() {
            if self.tree.is_empty() && previous.tree.is_empty() {
                self.compatibility_flat = clone_once_lock(&previous.compatibility_flat);
            }
            return;
        }
        self.tree = self.tree.reusing(
            &previous.tree,
            |new, old| new.items == old.items,
            |new, old| {
                (new.items == old.items).then(|| Chunk { items: old.items.clone(), ..new.clone() })
            },
        );
        self.compatibility_flat = if self.shares_storage_with(previous) {
            clone_once_lock(&previous.compatibility_flat)
        } else {
            OnceLock::new()
        };
    }

    /// Conservative O(1) identity comparison, including lazy transforms.
    fn shares_storage_with(&self, other: &Self) -> bool {
        self.origin == other.origin && self.tree.ptr_eq(&other.tree)
    }

    fn items_equal(&self, other: &Self) -> bool
    where
        T: PartialEq,
    {
        fn equal<T: Clone + RangedItem + PartialEq>(left: &RangeNode<T>, right: &RangeNode<T>) -> bool {
            std::ptr::eq(left, right)
                || match (left.view(), right.view()) {
                    (View::Leaf(left), View::Leaf(right)) => left == right,
                    (View::Branch(left_left, left_right), View::Branch(right_left, right_right)) => {
                        left.tag() == right.tag()
                            && equal(left_left, right_left)
                            && equal(left_right, right_right)
                    }
                    _ => false,
                }
        }
        self.origin == other.origin
            && match (self.tree.root(), other.tree.root()) {
                (None, None) => true,
                (Some(left), Some(right)) => equal(left, right),
                _ => false,
            }
    }

    #[cfg(test)]
    fn shared_leaf_count_with(&self, other: &Self) -> usize {
        self.tree.shared_leaf_count(&other.tree)
    }

    #[cfg(test)]
    fn leaf_count(&self) -> usize {
        self.tree.iter().count()
    }

    #[cfg(test)]
    fn invariant_holds(&self, require_nondecreasing_end: bool) -> bool {
        let items = self.to_vec();
        let ordered = items.windows(2).all(|pair| {
            pair[0].range().start <= pair[1].range().start
                && (!require_nondecreasing_end || pair[0].range().end <= pair[1].range().end)
        });
        let chunks = self.tree.iter().collect::<Vec<_>>();
        ordered
            && self.tree.invariant_holds()
            && chunks.last().map_or(self.origin == 0, |last| last.advance == 0)
            && chunks.iter().all(|chunk| {
                (1..=RANGE_INDEX_LEAF_ITEMS).contains(&chunk.items.len())
                    && chunk.items[0].range().start == 0
                    && chunk.items.windows(2).all(|pair| pair[0].range().start <= pair[1].range().start)
            })
            && chunks.windows(2).all(|pair| {
                pair[0].items.last().is_some_and(|item| item.range().start <= pair[0].advance)
            })
    }
}

impl<T: Clone + RangedItem> Clone for PersistentRangeStore<T> {
    fn clone(&self) -> Self {
        Self {
            tree: self.tree.clone(),
            origin: self.origin,
            compatibility_flat: clone_once_lock(&self.compatibility_flat),
        }
    }
}

fn clone_once_lock<T>(source: &OnceLock<Arc<Vec<T>>>) -> OnceLock<Arc<Vec<T>>> {
    let cloned = OnceLock::new();
    if let Some(value) = source.get() {
        let _ = cloned.set(value.clone());
    }
    cloned
}

/// `tree` with its last chunk's advance changed so that the chunks span `span`.
fn with_span<T: Clone + RangedItem>(
    tree: &Tree<Chunk<T>>,
    span: usize,
    work: &mut Work,
) -> Option<Tree<Chunk<T>>> {
    let current = tree.measure(|extent| extent.span);
    if current == span {
        return Some(tree.clone());
    }
    let advance = (tree.last()?.advance + span).checked_sub(current)?;
    let last = tree.measure(item_count) - 1;
    Some(tree.update(&mut persistent::at(last, item_count), |chunk, _| Chunk { advance, ..chunk.clone() }, work))
}

/// `left` followed by `right`. Small chunks meeting at the seam are merged, so
/// repeated splices do not fragment the leaves.
fn join<T: Clone + RangedItem>(
    left: Piece<T>,
    right: Piece<T>,
    work: &mut Work,
    stats: &mut RangeSpliceStats,
) -> Option<Piece<T>> {
    let (Some(last), Some(first)) = (left.tree.last(), right.tree.first()) else {
        return Some(if left.tree.is_empty() { right } else { left });
    };
    let (last_items, first_items) = (last.items.len(), first.items.len());
    if last_items + first_items > RANGE_INDEX_LEAF_ITEMS {
        let tree = with_span(&left.tree, right.origin.checked_sub(left.origin)?, work)?;
        return Some(Piece { tree: tree.concat(&right.tree, work), origin: left.origin });
    }
    let whole = |_: &Chunk<T>, _| unreachable!("a chunk boundary divides no chunk");
    let (prefix, last) =
        left.tree.split_at(left.tree.measure(item_count) - last_items, item_count, whole, work);
    let (first, suffix) = right.tree.split_at(first_items, item_count, whole, work);
    // The splits applied every pending change to the two chunks.
    let (last, first) = (last.first()?, first.first()?);
    let last_place = Place { origin: left.origin + prefix.measure(|extent| extent.span), ..Place::default() };
    let first_place = Place { origin: right.origin, ..Place::default() };
    let suffix_origin = right.origin + first.advance;
    let items = (0..last_items)
        .map(|index| last.item(index, last_place))
        .chain((0..first_items).map(|index| first.item(index, first_place)))
        .collect::<Vec<_>>();
    stats.items_copied = stats.items_copied.saturating_add(items.len());
    stats.leaves_copied = stats.leaves_copied.saturating_add(1);
    let advance = if suffix.is_empty() { 0 } else { suffix_origin.checked_sub(last_place.origin)? };
    let merged = Chunk::new(&items, last_place.origin, advance);
    let merged = Piece { tree: Tree::from_items([merged], work), origin: last_place.origin };
    let prefix = Piece { tree: prefix, origin: left.origin };
    join(join(prefix, merged, work, stats)?, Piece { tree: suffix, origin: suffix_origin }, work, stats)
}

fn count_starts_before<T: Clone + RangedItem>(node: &RangeNode<T>, origin: usize, offset: usize) -> usize {
    match node.view() {
        View::Leaf(chunk) => chunk.items.partition_point(|item| origin + item.range().start < offset),
        View::Branch(left, right) => {
            let right_origin = origin + left.summary().span;
            if right_origin >= offset {
                count_starts_before(left, origin, offset)
            } else {
                left.summary().items + count_starts_before(right, right_origin, offset)
            }
        }
    }
}

/// `shift` is what the node's ancestors have yet to apply to it.
fn count_auxiliary_starts_before<T: Clone + RangedItem>(
    node: &RangeNode<T>, shift: i128, offset: usize,
) -> usize {
    let starts_before = |start: usize, shift: i128| start as i128 + shift < offset as i128;
    match node.view() {
        View::Leaf(chunk) => chunk.items.partition_point(|item| {
            item.auxiliary_range()
                .is_some_and(|range| starts_before(range.start, shift + chunk.lazy.auxiliary_shift))
        }),
        View::Branch(left, right) => {
            let shift = shift + node.tag().map_or(0, |tag| tag.auxiliary_shift);
            if right.summary().auxiliary.as_ref().is_some_and(|range| !starts_before(range.start, shift)) {
                count_auxiliary_starts_before(left, shift, offset)
            } else {
                left.summary().items + count_auxiliary_starts_before(right, shift, offset)
            }
        }
    }
}

fn query_auxiliary_node<T: Clone + RangedItem>(
    node: &RangeNode<T>, place: Place, query: &Range<usize>, matches: &mut Vec<T>,
    mut stats: Option<&mut QueryStats>,
) {
    if let Some(stats) = stats.as_deref_mut() { stats.nodes_visited += 1; }
    let Some(bounds) = &node.summary().auxiliary else { return; };
    let shift = place.lazy.auxiliary_shift;
    if bounds.end as i128 + shift < query.start as i128 || bounds.start as i128 + shift > query.end as i128 {
        return;
    }
    match node.view() {
        View::Leaf(chunk) => {
            let shift = shift + chunk.lazy.auxiliary_shift;
            for (index, item) in chunk.items.iter().enumerate() {
                if let Some(stats) = stats.as_deref_mut() { stats.items_examined += 1; }
                let Some(auxiliary) = item.auxiliary_range() else { continue; };
                if auxiliary.start as i128 + shift > query.end as i128 { break; }
                if auxiliary.end as i128 + shift >= query.start as i128 {
                    matches.push(chunk.item(index, place));
                }
            }
        }
        View::Branch(left, right) => {
            let places = place.children(node, left);
            query_auxiliary_node(left, places.0, query, matches, stats.as_deref_mut());
            query_auxiliary_node(right, places.1, query, matches, stats);
        }
    }
}

fn first_index_ending_after<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    offset: usize,
    base_index: usize,
) -> Option<usize> {
    if origin + node.summary().max_end <= offset {
        return None;
    }
    match node.view() {
        View::Leaf(chunk) => chunk
            .items
            .iter()
            .position(|item| origin + item.range().end > offset)
            .map(|position| base_index + position),
        View::Branch(left, right) => first_index_ending_after(left, origin, offset, base_index)
            .or_else(|| {
                first_index_ending_after(
                    right,
                    origin + left.summary().span,
                    offset,
                    base_index + left.summary().items,
                )
            }),
    }
}

fn append_item_range<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    place: Place,
    base_index: usize,
    indices: &Range<usize>,
    output: &mut Vec<T>,
    mut stats: Option<&mut QueryStats>,
) {
    if let Some(stats) = stats.as_deref_mut() {
        stats.nodes_visited += 1;
    }
    if base_index + node.summary().items <= indices.start || base_index >= indices.end {
        return;
    }
    match node.view() {
        View::Leaf(chunk) => {
            let local_start = indices.start.saturating_sub(base_index).min(chunk.items.len());
            let local_end = indices.end.saturating_sub(base_index).min(chunk.items.len());
            if let Some(stats) = stats.as_deref_mut() {
                stats.items_examined += local_end - local_start;
            }
            output.extend((local_start..local_end).map(|index| chunk.item(index, place)));
        }
        View::Branch(left, right) => {
            let places = place.children(node, left);
            append_item_range(left, places.0, base_index, indices, output, stats.as_deref_mut());
            append_item_range(right, places.1, base_index + left.summary().items, indices, output, stats);
        }
    }
}

fn find_touching_index<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    offset: usize,
    base_index: usize,
    stats: &mut QueryStats,
) -> Option<usize> {
    stats.nodes_visited += 1;
    if origin > offset || origin + node.summary().max_end < offset {
        return None;
    }
    match node.view() {
        View::Leaf(chunk) => {
            let relative_offset = offset - origin;
            let after = chunk.items.partition_point(|item| item.range().start <= relative_offset);
            let local = after.checked_sub(1)?;
            (relative_offset <= chunk.items[local].range().end).then_some(base_index + local)
        }
        View::Branch(left, right) => {
            let right_origin = origin + left.summary().span;
            let in_right = (right_origin <= offset)
                .then(|| find_touching_index(right, right_origin, offset, base_index + left.summary().items, stats))
                .flatten();
            in_right.or_else(|| find_touching_index(left, origin, offset, base_index, stats))
        }
    }
}

#[derive(Clone, Copy)]
enum Intersection {
    Touching,
    Overlapping,
}

impl Intersection {
    fn subtree_misses(self, extent: &Extent, origin: usize, query: &Range<usize>) -> bool {
        let max_end = origin + extent.max_end;
        match self {
            Self::Touching => max_end < query.start || origin > query.end,
            Self::Overlapping => max_end <= query.start || origin >= query.end,
        }
    }

    fn includes(self, range: &Range<usize>, query: &Range<usize>) -> bool {
        match self {
            Self::Touching => range.end >= query.start && range.start <= query.end,
            Self::Overlapping => range.start < query.end && query.start < range.end,
        }
    }
}

fn query_node<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    place: Place,
    query: &Range<usize>,
    intersection: Intersection,
    matches: &mut Vec<T>,
    mut stats: Option<&mut QueryStats>,
) {
    if let Some(stats) = stats.as_deref_mut() {
        stats.nodes_visited += 1;
    }
    if intersection.subtree_misses(node.summary(), place.origin, query) {
        return;
    }
    match node.view() {
        View::Leaf(chunk) => {
            for (index, item) in chunk.items.iter().enumerate() {
                if let Some(stats) = stats.as_deref_mut() {
                    stats.items_examined += 1;
                }
                let range = place.origin + item.range().start..place.origin + item.range().end;
                if range.start > query.end {
                    break;
                }
                if intersection.includes(&range, query) {
                    matches.push(chunk.item(index, place));
                }
            }
        }
        View::Branch(left, right) => {
            let places = place.children(node, left);
            query_node(left, places.0, query, intersection, matches, stats.as_deref_mut());
            query_node(right, places.1, query, intersection, matches, stats);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QueryStats {
    pub(super) nodes_visited: usize,
    pub(super) items_examined: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Item(Range<usize>, usize);

    impl RangedItem for Item {
        fn navigation_summary(&self) -> NavigationSummary {
            let key = (self.1 % 7) as u16;
            NavigationSummary { minimum: key, maximum: key, minimum_start: key, flags: 0 }
        }
        fn range(&self) -> &Range<usize> {
            &self.0
        }

        fn with_range(&self, range: Range<usize>) -> Self {
            Self(range, self.1)
        }
    }

    #[test]
    fn navigation_search_matches_flat_oracle_after_persistent_splicing() {
        let before = OrderedRangeStore::new((0..10_000).map(|index| Item(index * 4..index * 4 + 3, index)).collect());
        let after = before.splice(400..500, vec![Item(1600..1601, 13)], 2000, 1602, &mut RangeSpliceStats::default()).unwrap();
        for store in [&before, &after] {
            assert!(store.invariant_holds());
            for start in [0, 1, 399, 400, 401, 600, 9000] {
                for length in [0, 1, 7, 73, 500] {
                    let range = start..(start + length).min(store.len());
                    for key in 0..7 {
                        let matching = range.clone().filter(|&index| store.get(index).unwrap().navigation_summary().minimum < key).collect::<Vec<_>>();
                        assert_eq!(store.find_navigation(range.clone(), false, |summary| summary.minimum < key), matching.first().copied());
                        assert_eq!(store.find_navigation(range.clone(), true, |summary| summary.minimum < key), matching.last().copied());
                    }
                }
            }
        }
    }

    #[test]
    fn ordered_query_matches_flat_oracle_for_random_partitions() {
        let mut seed = 0x9e37_79b9_u64;
        let mut items = Vec::new();
        let mut start = 0;
        for id in 0..10_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let length = usize::try_from(seed % 8).unwrap();
            items.push(Item(start..start + length, id));
            start += length + 1;
        }
        let store = OrderedRangeStore::new(items);
        assert!(store.invariant_holds());

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let left = usize::try_from(seed % u64::try_from(start).unwrap()).unwrap();
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let width = usize::try_from(seed % 20).unwrap();
            let query = left..left.saturating_add(width).min(start);
            let expected = store
                .to_vec()
                .into_iter()
                .filter(|item| item.range().end >= query.start && item.range().start <= query.end)
                .map(|item| item.1)
                .collect::<Vec<_>>();
            let actual = store
                .query_touching(&query)
                .into_iter()
                .map(|item| item.1)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query {query:?}");
        }
    }

    #[test]
    fn interval_query_matches_flat_oracle_for_random_overlaps() {
        let mut seed = 0xd1b5_4a32_d192_ed03_u64;
        let mut items = Vec::new();
        for id in 0..20_000 {
            seed = seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1);
            let start = usize::try_from(seed % 100_000).unwrap();
            seed = seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1);
            let length = usize::try_from(seed % 256 + 1).unwrap();
            items.push(Item(start..start + length, id));
        }
        let store = IntervalRangeStore::new(items);
        assert!(store.invariant_holds());

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1);
            let start = usize::try_from(seed % 100_000).unwrap();
            seed = seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1);
            let query = start..start + usize::try_from(seed % 128 + 1).unwrap();
            let expected = store
                .to_vec()
                .into_iter()
                .filter(|item| item.range().start < query.end && query.start < item.range().end)
                .map(|item| item.1)
                .collect::<Vec<_>>();
            let actual = store
                .query_overlapping(&query)
                .into_iter()
                .map(|item| item.1)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query {query:?}");
            // The index span is the tightest contiguous cover of the matches.
            let span = store.overlapping_index_span(&query);
            let flat = store.compatibility_slice();
            let first_end_after = flat
                .iter()
                .position(|item| item.range().end > query.start)
                .unwrap_or(flat.len());
            assert_eq!(span.start, first_end_after, "query {query:?}");
            assert!(flat[span.clone()].iter().all(|item| item.range().start < query.end));
            assert!(flat[span.end..].iter().all(|item| item.range().start >= query.end));
            assert!(expected.iter().all(|id| flat[span.clone()].iter().any(|item| item.1 == *id)));
        }
    }

    #[test]
    fn one_million_short_block_ranges_have_bounded_query_work() {
        const BLOCKS: usize = 1_000_000;
        let items = (0..BLOCKS)
            .map(|index| Item(index * 2..index * 2 + 1, index))
            .collect();
        let store = OrderedRangeStore::new(items);
        let query_start = (BLOCKS - 2) * 2;
        let (matches, stats) = store.query_touching_with_stats(&(query_start..query_start + 1));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].1, BLOCKS - 2);
        assert!(stats.nodes_visited <= 40, "{stats:?}");
        assert!(stats.items_examined <= RANGE_INDEX_LEAF_ITEMS, "{stats:?}");
    }

    #[test]
    fn many_disjoint_style_spans_prune_to_a_bounded_frontier() {
        const SPANS: usize = 250_000;
        let items = (0..SPANS)
            .map(|index| Item(index * 4..index * 4 + 2, index))
            .collect();
        let store = IntervalRangeStore::new(items);
        let query_start = (SPANS - 3) * 4;
        let (matches, stats) = store.query_overlapping_with_stats(&(query_start..query_start + 1));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].1, SPANS - 3);
        assert!(stats.nodes_visited <= 40, "{stats:?}");
        assert!(stats.items_examined <= RANGE_INDEX_LEAF_ITEMS, "{stats:?}");
    }

    #[test]
    fn one_extremely_long_range_is_found_without_scanning_contents() {
        let store = OrderedRangeStore::new(vec![Item(0..64 * 1024 * 1024, 7)]);
        let middle = 32 * 1024 * 1024;
        let (matches, stats) = store.query_touching_with_stats(&(middle..middle));
        assert_eq!(matches, vec![Item(0..64 * 1024 * 1024, 7)]);
        assert_eq!(stats.nodes_visited, 1);
        assert_eq!(stats.items_examined, 1);
    }

    #[test]
    fn million_item_regional_splice_visits_only_boundary_paths() {
        const ITEMS: usize = 1_000_000;
        let original = OrderedRangeStore::new(
            (0..ITEMS)
                .map(|index| Item(index * 2..index * 2 + 1, index))
                .collect(),
        );
        let target = ITEMS / 2;
        let mut stats = RangeSpliceStats::default();
        let changed = original
            .splice(
                target..target + 1,
                vec![Item(target * 2..target * 2 + 4, target)],
                target * 2 + 1,
                target * 2 + 4,
                &mut stats,
            )
            .unwrap();

        assert!(changed.invariant_holds());
        assert_eq!(changed.get(target).unwrap().0, target * 2..target * 2 + 4);
        assert_eq!(
            changed.get(target + 1).unwrap().0,
            target * 2 + 5..target * 2 + 6
        );
        // Two splits of a 15-level tree, each descending and rejoining one path.
        assert!(stats.nodes_visited <= 80, "{stats:?}");
        assert!(
            stats.items_copied <= RANGE_INDEX_LEAF_ITEMS * 4,
            "{stats:?}"
        );
        assert!(
            changed.shared_leaf_count_with(&original) + 3 >= original.leaf_count(),
            "{stats:?}"
        );
    }

    #[test]
    fn repeated_fixed_size_splices_do_not_fragment_leaves() {
        let mut store = OrderedRangeStore::new(
            (0..512)
                .map(|index| Item(index * 2..index * 2 + 1, index))
                .collect(),
        );
        let initial_leaves = store.leaf_count();
        for step in 0..2_000 {
            let index = step * 197 % store.len();
            let mut stats = RangeSpliceStats::default();
            store = store
                .splice(
                    index..index + 1,
                    vec![Item(index * 2..index * 2 + 1, 10_000 + step)],
                    index * 2 + 1,
                    index * 2 + 1,
                    &mut stats,
                )
                .unwrap();
            assert!(store.invariant_holds());
        }
        assert_eq!(store.leaf_count(), initial_leaves);
    }

    #[test]
    fn variable_count_splice_and_start_partition_match_flat_oracle() {
        let original = OrderedRangeStore::new(
            (0..200)
                .map(|index| Item(index * 3..index * 3 + 2, index))
                .collect(),
        );
        let mut stats = RangeSpliceStats::default();
        let changed = original
            .splice(
                80..82,
                vec![
                    Item(240..241, 1_000),
                    Item(242..245, 1_001),
                    Item(246..247, 1_002),
                ],
                245,
                247,
                &mut stats,
            )
            .unwrap();
        assert!(changed.invariant_holds());
        assert_eq!(changed.len(), 201);
        assert_eq!(changed.partition_point_start(240), 80);
        assert_eq!(changed.partition_point_start(247), 83);
        assert_eq!(changed.get(79).unwrap(), Item(237..239, 79));
        assert_eq!(changed.get(83).unwrap(), Item(248..250, 82));
    }
}
