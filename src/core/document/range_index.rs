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

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Deref, Range};
use std::sync::{Arc, OnceLock};

pub(super) const RANGE_INDEX_LEAF_ITEMS: usize = 64;

pub(super) trait RangedItem {
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

pub(super) struct OrderedRangeStore<T> {
    inner: PersistentRangeStore<T>,
}

impl<T: Clone + RangedItem> OrderedRangeStore<T> {
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        self.inner.visit_retained_memory(visitor);
    }

    pub(super) fn new(items: Vec<T>) -> Self {
        Self {
            inner: PersistentRangeStore::new(items),
        }
    }

    pub(super) fn as_slice(&self) -> &[T] {
        self.inner.as_slice()
    }

    pub(super) fn to_vec(&self) -> Vec<T> {
        self.inner.to_vec()
    }

    pub(super) fn len(&self) -> usize {
        self.inner.item_count
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.item_count == 0
    }

    pub(super) fn get(&self, index: usize) -> Option<T> {
        self.inner.get(index)
    }

    pub(super) fn get_with_stats(&self, index: usize) -> (Option<T>, QueryStats) {
        let mut stats = QueryStats::default();
        let result = self
            .inner
            .root
            .as_ref()
            .filter(|_| index < self.inner.item_count)
            .map(|root| {
                get_item(
                    root,
                    self.inner.root_origin,
                    self.inner.root_auxiliary_shift,
                    self.inner.root_revision,
                    index,
                    Some(&mut stats),
                )
            });
        (result, stats)
    }

    /// Materializes one half-open ordinal item range with one tree descent and
    /// a sequential walk of the covered leaves.
    pub(super) fn get_range(&self, indices: &Range<usize>) -> Option<Vec<T>> {
        self.inner.get_range(indices, None)
    }

    #[cfg(test)]
    pub(super) fn partition_point_start(&self, offset: usize) -> usize {
        self.inner.partition_point_start(offset)
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
        let result = self.inner.root.as_ref().and_then(|root| {
            find_touching_index(root, self.inner.root_origin, offset, 0, &mut stats)
        });
        (result, stats)
    }

    /// Reports blocks whose closed boundary extents touch `query`. Including a
    /// block ending exactly at the query start preserves empty-paragraph and
    /// paragraph-boundary geometry.
    pub(super) fn query_touching(&self, query: &Range<usize>) -> Vec<T> {
        self.inner.query(query, Intersection::Touching, None)
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
    pub(super) fn shares_root_with(&self, other: &Self) -> bool {
        self.inner.shares_root_with(&other.inner)
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

impl<T: Clone + RangedItem> Deref for OrderedRangeStore<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T: Clone + RangedItem + fmt::Debug> fmt::Debug for OrderedRangeStore<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T: Clone + RangedItem + PartialEq> PartialEq for OrderedRangeStore<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.items_equal(&other.inner)
    }
}

pub(super) struct IntervalRangeStore<T> {
    inner: PersistentRangeStore<T>,
}

impl<T: Clone + RangedItem> IntervalRangeStore<T> {
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

    pub(super) fn as_slice(&self) -> &[T] {
        self.inner.as_slice()
    }

    pub(super) fn len(&self) -> usize {
        self.inner.item_count
    }

    pub(super) fn is_empty(&self) -> bool {
        self.inner.item_count == 0
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
    pub(super) fn shares_root_with(&self, other: &Self) -> bool {
        self.inner.shares_root_with(&other.inner)
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

impl<T: Clone + RangedItem> Deref for IntervalRangeStore<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T: Clone + RangedItem + fmt::Debug> fmt::Debug for IntervalRangeStore<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T: Clone + RangedItem + PartialEq> PartialEq for IntervalRangeStore<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.items_equal(&other.inner)
    }
}

struct PersistentRangeStore<T> {
    root: Option<Arc<RangeNode<T>>>,
    root_origin: usize,
    root_auxiliary_shift: i128,
    root_revision: Option<u64>,
    item_count: usize,
    compatibility_flat: OnceLock<Arc<Vec<T>>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RangeSpliceStats {
    pub(super) nodes_visited: usize,
    pub(super) nodes_copied: usize,
    pub(super) leaves_copied: usize,
    pub(super) items_copied: usize,
}

impl<T: Clone + RangedItem> PersistentRangeStore<T> {
    fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        fn visit<T: RangedItem>(node: &Arc<RangeNode<T>>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
            visitor.arc(node, |visitor| match &node.kind {
                RangeNodeKind::Leaf(items) => {
                    visitor.vector(items, items.iter().map(RangedItem::owned_heap_bytes).sum());
                    for item in items { item.visit_shared_memory(visitor); }
                },
                RangeNodeKind::Branch { left, right, .. } => {
                    visit(left, visitor);
                    visit(right, visitor);
                }
            });
        }
        if let Some(root) = &self.root { visit(root, visitor); }
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
        let item_count = items.len();
        let leaves = items
            .chunks(RANGE_INDEX_LEAF_ITEMS)
            .map(|chunk| RangeNode::leaf(chunk.to_vec()))
            .collect::<Vec<_>>();
        let positioned_root = build_balanced_root(&leaves, 0..leaves.len());
        let (root_origin, root) = positioned_root
            .map(|positioned| (positioned.origin, Some(positioned.node)))
            .unwrap_or((0, None));
        Self {
            root,
            root_origin,
            root_auxiliary_shift: 0,
            root_revision: None,
            item_count,
            compatibility_flat: OnceLock::new(),
        }
    }

    fn as_slice(&self) -> &[T] {
        self.compatibility_flat
            .get_or_init(|| Arc::new(self.to_vec()))
            .as_slice()
    }

    fn to_vec(&self) -> Vec<T> {
        let mut items = Vec::with_capacity(self.item_count);
        if let Some(root) = &self.root {
            append_items(
                root,
                self.root_origin,
                self.root_auxiliary_shift,
                self.root_revision,
                &mut items,
            );
        }
        items
    }

    fn get(&self, index: usize) -> Option<T> {
        let root = self.root.as_ref()?;
        (index < self.item_count).then(|| {
            get_item(
                root,
                self.root_origin,
                self.root_auxiliary_shift,
                self.root_revision,
                index,
                None,
            )
        })
    }

    fn partition_point_start(&self, offset: usize) -> usize {
        self.root.as_ref().map_or(0, |root| {
            count_starts_before(root, self.root_origin, offset)
        })
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
        if indices.start > indices.end || indices.end > self.item_count {
            return None;
        }
        let positioned = self.root.as_ref().map(|root| PositionedNode {
            origin: self.root_origin,
            auxiliary_shift: self.root_auxiliary_shift,
            revision: self.root_revision,
            node: root.clone(),
        });
        let (before, remainder) = split_positioned(positioned, indices.start, stats)?;
        let (_, after) = split_positioned(remainder, indices.len(), stats)?;
        let replacement_count = replacement.len();
        let inserted_store = PersistentRangeStore::new(replacement);
        stats.items_copied = stats.items_copied.saturating_add(replacement_count);
        if let Some(root) = &inserted_store.root {
            stats.leaves_copied = stats.leaves_copied.saturating_add(root.leaf_count);
            stats.nodes_copied = stats
                .nodes_copied
                .saturating_add(root.leaf_count.saturating_mul(2).saturating_sub(1));
        }
        let inserted = inserted_store.root.map(|node| PositionedNode {
            origin: inserted_store.root_origin,
            auxiliary_shift: inserted_store.root_auxiliary_shift,
            revision: inserted_store.root_revision,
            node,
        });
        let after = match after {
            Some(positioned) => Some(shift_positioned(
                positioned,
                old_coordinate_end,
                new_coordinate_end,
                auxiliary_ends,
            )?),
            None => None,
        };
        let root = join_optional_positioned(
            join_optional_positioned(before, inserted, stats)?,
            after,
            stats,
        )?;
        let item_count = self
            .item_count
            .checked_sub(indices.len())?
            .checked_add(replacement_count)?;
        let (root_origin, root_auxiliary_shift, root_revision, root) = root
            .map(|mut positioned| {
                if let Some(revision) = revision {
                    positioned.revision = Some(revision);
                }
                (
                    positioned.origin,
                    positioned.auxiliary_shift,
                    positioned.revision,
                    Some(positioned.node),
                )
            })
            .unwrap_or((0, 0, revision, None));
        Some(Self {
            root,
            root_origin,
            root_auxiliary_shift,
            root_revision,
            item_count,
            compatibility_flat: OnceLock::new(),
        })
    }

    fn get_range(&self, indices: &Range<usize>, stats: Option<&mut QueryStats>) -> Option<Vec<T>> {
        if indices.start > indices.end || indices.end > self.item_count {
            return None;
        }
        let mut items = Vec::with_capacity(indices.end - indices.start);
        if let Some(root) = &self.root {
            append_item_range(
                root,
                self.root_origin,
                self.root_auxiliary_shift,
                self.root_revision,
                0,
                indices,
                &mut items,
                stats,
            );
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
        if let Some(root) = &self.root {
            query_node(
                root,
                self.root_origin,
                self.root_auxiliary_shift,
                self.root_revision,
                query,
                intersection,
                &mut matches,
                stats,
            );
        }
        matches
    }

    fn reuse_equal_chunks(&mut self, previous: &Self)
    where
        T: PartialEq,
    {
        let Some(candidate) = self.root.take() else {
            if previous.root.is_none() {
                self.compatibility_flat = clone_once_lock(&previous.compatibility_flat);
            }
            return;
        };
        let Some(old) = &previous.root else {
            self.root = Some(candidate);
            return;
        };
        if self.root_auxiliary_shift != previous.root_auxiliary_shift
            || self.root_revision != previous.root_revision
        {
            self.root = Some(candidate);
            self.compatibility_flat = OnceLock::new();
            return;
        }
        let reconciled = reuse_node(candidate, old);
        let shares_root = Arc::ptr_eq(&reconciled, old);
        self.root = Some(reconciled);
        if shares_root && self.root_origin == previous.root_origin {
            self.compatibility_flat = clone_once_lock(&previous.compatibility_flat);
        } else {
            self.compatibility_flat = OnceLock::new();
        }
    }

    fn items_equal(&self, other: &Self) -> bool
    where
        T: PartialEq,
    {
        if self.item_count != other.item_count
            || self.root_origin != other.root_origin
            || self.root_auxiliary_shift != other.root_auxiliary_shift
            || self.root_revision != other.root_revision
        {
            return false;
        }
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(left), Some(right)) => node_items_equal(left, right),
            _ => false,
        }
    }

    #[cfg(test)]
    fn shares_root_with(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }

    #[cfg(test)]
    fn shared_leaf_count_with(&self, other: &Self) -> usize {
        match (&self.root, &other.root) {
            (Some(left), Some(right)) => shared_leaf_count(left, right),
            _ => 0,
        }
    }

    #[cfg(test)]
    fn leaf_count(&self) -> usize {
        self.root.as_ref().map_or(0, |root| root.leaf_count)
    }

    #[cfg(test)]
    fn invariant_holds(&self, require_nondecreasing_end: bool) -> bool {
        let items = self.to_vec();
        let ordered = items.windows(2).all(|pair| {
            pair[0].range().start <= pair[1].range().start
                && (!require_nondecreasing_end || pair[0].range().end <= pair[1].range().end)
        });
        if !ordered {
            return false;
        }
        match &self.root {
            None => self.item_count == 0,
            Some(root) => root.item_count == self.item_count && validate_node(root).is_some(),
        }
    }
}

impl<T: Clone + RangedItem> Clone for PersistentRangeStore<T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            root_origin: self.root_origin,
            root_auxiliary_shift: self.root_auxiliary_shift,
            root_revision: self.root_revision,
            item_count: self.item_count,
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

struct RangeNode<T> {
    max_end: usize,
    item_count: usize,
    leaf_count: usize,
    height: usize,
    kind: RangeNodeKind<T>,
}

enum RangeNodeKind<T> {
    Leaf(Vec<T>),
    Branch {
        left: Arc<RangeNode<T>>,
        left_offset: usize,
        left_auxiliary_shift: i128,
        left_revision: Option<u64>,
        right: Arc<RangeNode<T>>,
        right_offset: usize,
        right_auxiliary_shift: i128,
        right_revision: Option<u64>,
    },
}

struct PositionedNode<T> {
    origin: usize,
    auxiliary_shift: i128,
    revision: Option<u64>,
    node: Arc<RangeNode<T>>,
}

impl<T: Clone + RangedItem> RangeNode<T> {
    fn leaf(items: Vec<T>) -> PositionedNode<T> {
        let origin = items
            .first()
            .expect("range-index leaves are constructed from non-empty chunks")
            .range()
            .start;
        let normalized = items
            .iter()
            .map(|item| {
                let range = item.range();
                let start = range
                    .start
                    .checked_sub(origin)
                    .expect("range-index input is ordered by start boundary");
                let end = range
                    .end
                    .checked_sub(origin)
                    .expect("a valid range ends at or after its leaf origin");
                item.with_range(start..end)
            })
            .collect::<Vec<_>>();
        let max_end = normalized
            .iter()
            .map(|item| item.range().end)
            .max()
            .expect("a non-empty range-index leaf has an end boundary");
        PositionedNode {
            origin,
            auxiliary_shift: 0,
            revision: None,
            node: Arc::new(Self {
                max_end,
                item_count: normalized.len(),
                leaf_count: 1,
                height: 1,
                kind: RangeNodeKind::Leaf(normalized),
            }),
        }
    }

    fn branch(left: PositionedNode<T>, right: PositionedNode<T>) -> PositionedNode<T> {
        let origin = left.origin.min(right.origin);
        let left_offset = left
            .origin
            .checked_sub(origin)
            .expect("left range-index origin follows its parent origin");
        let right_offset = right
            .origin
            .checked_sub(origin)
            .expect("right range-index origin follows its parent origin");
        let node = Self::branch_with_offsets(
            left.node,
            left_offset,
            left.auxiliary_shift,
            left.revision,
            right.node,
            right_offset,
            right.auxiliary_shift,
            right.revision,
        );
        PositionedNode {
            origin,
            auxiliary_shift: 0,
            revision: None,
            node,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn branch_with_offsets(
        left: Arc<Self>,
        left_offset: usize,
        left_auxiliary_shift: i128,
        left_revision: Option<u64>,
        right: Arc<Self>,
        right_offset: usize,
        right_auxiliary_shift: i128,
        right_revision: Option<u64>,
    ) -> Arc<Self> {
        let left_end = left_offset
            .checked_add(left.max_end)
            .expect("range-index left aggregate is representable");
        let right_end = right_offset
            .checked_add(right.max_end)
            .expect("range-index right aggregate is representable");
        Arc::new(Self {
            max_end: left_end.max(right_end),
            item_count: left.item_count + right.item_count,
            leaf_count: left.leaf_count + right.leaf_count,
            height: left.height.max(right.height) + 1,
            kind: RangeNodeKind::Branch {
                left,
                left_offset,
                left_auxiliary_shift,
                left_revision,
                right,
                right_offset,
                right_auxiliary_shift,
                right_revision,
            },
        })
    }
}

fn build_balanced_root<T: Clone + RangedItem>(
    leaves: &[PositionedNode<T>],
    range: Range<usize>,
) -> Option<PositionedNode<T>> {
    match range.len() {
        0 => None,
        1 => Some(PositionedNode {
            origin: leaves[range.start].origin,
            auxiliary_shift: leaves[range.start].auxiliary_shift,
            revision: leaves[range.start].revision,
            node: leaves[range.start].node.clone(),
        }),
        _ => {
            let middle = range.start + range.len() / 2;
            let left = build_balanced_root(leaves, range.start..middle)
                .expect("a split range-index node has a left child");
            let right = build_balanced_root(leaves, middle..range.end)
                .expect("a split range-index node has a right child");
            Some(RangeNode::branch(left, right))
        }
    }
}

fn count_starts_before<T: RangedItem>(node: &RangeNode<T>, origin: usize, offset: usize) -> usize {
    match &node.kind {
        RangeNodeKind::Leaf(items) => items.partition_point(|item| {
            origin
                .checked_add(item.range().start)
                .is_some_and(|start| start < offset)
        }),
        RangeNodeKind::Branch {
            left,
            left_offset,
            right,
            right_offset,
            ..
        } => {
            let left_origin = origin
                .checked_add(*left_offset)
                .expect("range-index left origin is representable");
            let right_origin = origin
                .checked_add(*right_offset)
                .expect("range-index right origin is representable");
            if right_origin >= offset {
                count_starts_before(left, left_origin, offset)
            } else {
                left.item_count + count_starts_before(right, right_origin, offset)
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn split_positioned<T: Clone + RangedItem>(
    positioned: Option<PositionedNode<T>>,
    at: usize,
    stats: &mut RangeSpliceStats,
) -> Option<(Option<PositionedNode<T>>, Option<PositionedNode<T>>)> {
    let Some(positioned) = positioned else {
        return (at == 0).then_some((None, None));
    };
    if at > positioned.node.item_count {
        return None;
    }
    if at == 0 {
        return Some((None, Some(positioned)));
    }
    if at == positioned.node.item_count {
        return Some((Some(positioned), None));
    }
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    match &positioned.node.kind {
        RangeNodeKind::Leaf(items) => {
            let absolute = |item: &T| {
                let range = item.range();
                item.with_transform(
                    positioned.origin.checked_add(range.start).unwrap()
                        ..positioned.origin.checked_add(range.end).unwrap(),
                    positioned.auxiliary_shift,
                    positioned.revision,
                )
            };
            let left_items = items[..at]
                .iter()
                .map(&absolute)
                .collect::<Option<Vec<_>>>()?;
            let right_items = items[at..]
                .iter()
                .map(absolute)
                .collect::<Option<Vec<_>>>()?;
            stats.items_copied = stats.items_copied.saturating_add(items.len());
            stats.leaves_copied = stats.leaves_copied.saturating_add(2);
            stats.nodes_copied = stats.nodes_copied.saturating_add(2);
            Some((
                Some(RangeNode::leaf(left_items)),
                Some(RangeNode::leaf(right_items)),
            ))
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            left_auxiliary_shift,
            left_revision,
            right,
            right_offset,
            right_auxiliary_shift,
            right_revision,
        } => {
            let left_positioned = PositionedNode {
                origin: positioned.origin.checked_add(*left_offset)?,
                auxiliary_shift: positioned
                    .auxiliary_shift
                    .checked_add(*left_auxiliary_shift)?,
                revision: positioned.revision.or(*left_revision),
                node: left.clone(),
            };
            let right_positioned = PositionedNode {
                origin: positioned.origin.checked_add(*right_offset)?,
                auxiliary_shift: positioned
                    .auxiliary_shift
                    .checked_add(*right_auxiliary_shift)?,
                revision: positioned.revision.or(*right_revision),
                node: right.clone(),
            };
            match at.cmp(&left.item_count) {
                Ordering::Less => {
                    let (before, middle) = split_positioned(Some(left_positioned), at, stats)?;
                    let after = join_optional_positioned(middle, Some(right_positioned), stats)?;
                    Some((before, after))
                }
                Ordering::Equal => Some((Some(left_positioned), Some(right_positioned))),
                Ordering::Greater => {
                    let (middle, after) = split_positioned(
                        Some(right_positioned),
                        at.checked_sub(left.item_count)?,
                        stats,
                    )?;
                    let before = join_optional_positioned(Some(left_positioned), middle, stats)?;
                    Some((before, after))
                }
            }
        }
    }
}

fn shift_positioned<T>(
    mut positioned: PositionedNode<T>,
    old_coordinate_end: usize,
    new_coordinate_end: usize,
    auxiliary_ends: Option<(usize, usize)>,
) -> Option<PositionedNode<T>> {
    positioned.origin = if new_coordinate_end >= old_coordinate_end {
        positioned
            .origin
            .checked_add(new_coordinate_end - old_coordinate_end)?
    } else {
        positioned
            .origin
            .checked_sub(old_coordinate_end - new_coordinate_end)?
    };
    if let Some((old_end, new_end)) = auxiliary_ends {
        let delta = i128::try_from(new_end).ok()? - i128::try_from(old_end).ok()?;
        positioned.auxiliary_shift = positioned.auxiliary_shift.checked_add(delta)?;
    }
    Some(positioned)
}

fn join_optional_positioned<T: Clone + RangedItem>(
    left: Option<PositionedNode<T>>,
    right: Option<PositionedNode<T>>,
    stats: &mut RangeSpliceStats,
) -> Option<Option<PositionedNode<T>>> {
    match (left, right) {
        (None, None) => Some(None),
        (Some(node), None) | (None, Some(node)) => Some(Some(node)),
        (Some(left), Some(right)) => join_positioned(left, right, stats).map(Some),
    }
}

fn join_positioned<T: Clone + RangedItem>(
    left: PositionedNode<T>,
    right: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<PositionedNode<T>> {
    let boundary_items = rightmost_leaf_item_count(&left.node)
        .checked_add(leftmost_leaf_item_count(&right.node))?;
    if boundary_items <= RANGE_INDEX_LEAF_ITEMS {
        let (left_prefix, left_leaf) = take_last_leaf(left, stats)?;
        let (right_leaf, right_suffix) = take_first_leaf(right, stats)?;
        let merged = merge_positioned_leaves(&left_leaf, &right_leaf, stats)?;
        let prefix_and_merged = join_optional_positioned(left_prefix, Some(merged), stats)?;
        return join_optional_positioned(prefix_and_merged, right_suffix, stats)?;
    }
    join_positioned_balanced(left, right, stats)
}

fn join_positioned_balanced<T: Clone + RangedItem>(
    left: PositionedNode<T>,
    right: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<PositionedNode<T>> {
    if left.node.height > right.node.height.saturating_add(1) {
        let (left_left, left_right) = positioned_children(&left)?;
        let joined = join_positioned_balanced(left_right, right, stats)?;
        rebalance_positioned(left_left, joined, stats)
    } else if right.node.height > left.node.height.saturating_add(1) {
        let (right_left, right_right) = positioned_children(&right)?;
        let joined = join_positioned_balanced(left, right_left, stats)?;
        rebalance_positioned(joined, right_right, stats)
    } else {
        Some(copied_branch(left, right, stats))
    }
}

fn rightmost_leaf_item_count<T>(node: &RangeNode<T>) -> usize {
    match &node.kind {
        RangeNodeKind::Leaf(items) => items.len(),
        RangeNodeKind::Branch { right, .. } => rightmost_leaf_item_count(right),
    }
}

fn leftmost_leaf_item_count<T>(node: &RangeNode<T>) -> usize {
    match &node.kind {
        RangeNodeKind::Leaf(items) => items.len(),
        RangeNodeKind::Branch { left, .. } => leftmost_leaf_item_count(left),
    }
}

fn take_last_leaf<T: Clone + RangedItem>(
    positioned: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<(Option<PositionedNode<T>>, PositionedNode<T>)> {
    if matches!(&positioned.node.kind, RangeNodeKind::Leaf(_)) {
        return Some((None, positioned));
    }
    let (left, right) = positioned_children(&positioned)?;
    let (right_prefix, leaf) = take_last_leaf(right, stats)?;
    let prefix = join_optional_positioned_balanced(Some(left), right_prefix, stats)?;
    Some((prefix, leaf))
}

fn take_first_leaf<T: Clone + RangedItem>(
    positioned: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<(PositionedNode<T>, Option<PositionedNode<T>>)> {
    if matches!(&positioned.node.kind, RangeNodeKind::Leaf(_)) {
        return Some((positioned, None));
    }
    let (left, right) = positioned_children(&positioned)?;
    let (leaf, left_suffix) = take_first_leaf(left, stats)?;
    let suffix = join_optional_positioned_balanced(left_suffix, Some(right), stats)?;
    Some((leaf, suffix))
}

fn join_optional_positioned_balanced<T: Clone + RangedItem>(
    left: Option<PositionedNode<T>>,
    right: Option<PositionedNode<T>>,
    stats: &mut RangeSpliceStats,
) -> Option<Option<PositionedNode<T>>> {
    match (left, right) {
        (None, None) => Some(None),
        (Some(node), None) | (None, Some(node)) => Some(Some(node)),
        (Some(left), Some(right)) => join_positioned_balanced(left, right, stats).map(Some),
    }
}

fn merge_positioned_leaves<T: Clone + RangedItem>(
    left: &PositionedNode<T>,
    right: &PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<PositionedNode<T>> {
    let item_count = left.node.item_count.checked_add(right.node.item_count)?;
    if item_count > RANGE_INDEX_LEAF_ITEMS {
        return None;
    }
    let mut items = positioned_leaf_items(left)?;
    items.extend(positioned_leaf_items(right)?);
    stats.items_copied = stats.items_copied.saturating_add(item_count);
    stats.leaves_copied = stats.leaves_copied.saturating_add(1);
    stats.nodes_copied = stats.nodes_copied.saturating_add(1);
    Some(RangeNode::leaf(items))
}

fn positioned_leaf_items<T: Clone + RangedItem>(
    positioned: &PositionedNode<T>,
) -> Option<Vec<T>> {
    let RangeNodeKind::Leaf(items) = &positioned.node.kind else {
        return None;
    };
    items
        .iter()
        .map(|item| {
            let range = item.range();
            item.with_transform(
                positioned.origin.checked_add(range.start)?
                    ..positioned.origin.checked_add(range.end)?,
                positioned.auxiliary_shift,
                positioned.revision,
            )
        })
        .collect()
}

fn rebalance_positioned<T: Clone + RangedItem>(
    left: PositionedNode<T>,
    right: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> Option<PositionedNode<T>> {
    if left.node.height > right.node.height.saturating_add(1) {
        let (left_left, left_right) = positioned_children(&left)?;
        if left_left.node.height >= left_right.node.height {
            let new_right = copied_branch(left_right, right, stats);
            return Some(copied_branch(left_left, new_right, stats));
        }
        let (middle_left, middle_right) = positioned_children(&left_right)?;
        let new_left = copied_branch(left_left, middle_left, stats);
        let new_right = copied_branch(middle_right, right, stats);
        return Some(copied_branch(new_left, new_right, stats));
    }
    if right.node.height > left.node.height.saturating_add(1) {
        let (right_left, right_right) = positioned_children(&right)?;
        if right_right.node.height >= right_left.node.height {
            let new_left = copied_branch(left, right_left, stats);
            return Some(copied_branch(new_left, right_right, stats));
        }
        let (middle_left, middle_right) = positioned_children(&right_left)?;
        let new_left = copied_branch(left, middle_left, stats);
        let new_right = copied_branch(middle_right, right_right, stats);
        return Some(copied_branch(new_left, new_right, stats));
    }
    Some(copied_branch(left, right, stats))
}

fn positioned_children<T>(
    positioned: &PositionedNode<T>,
) -> Option<(PositionedNode<T>, PositionedNode<T>)> {
    let RangeNodeKind::Branch {
        left,
        left_offset,
        left_auxiliary_shift,
        left_revision,
        right,
        right_offset,
        right_auxiliary_shift,
        right_revision,
    } = &positioned.node.kind
    else {
        return None;
    };
    Some((
        PositionedNode {
            origin: positioned.origin.checked_add(*left_offset)?,
            auxiliary_shift: positioned
                .auxiliary_shift
                .checked_add(*left_auxiliary_shift)?,
            revision: positioned.revision.or(*left_revision),
            node: left.clone(),
        },
        PositionedNode {
            origin: positioned.origin.checked_add(*right_offset)?,
            auxiliary_shift: positioned
                .auxiliary_shift
                .checked_add(*right_auxiliary_shift)?,
            revision: positioned.revision.or(*right_revision),
            node: right.clone(),
        },
    ))
}

fn copied_branch<T: Clone + RangedItem>(
    left: PositionedNode<T>,
    right: PositionedNode<T>,
    stats: &mut RangeSpliceStats,
) -> PositionedNode<T> {
    stats.nodes_copied = stats.nodes_copied.saturating_add(1);
    RangeNode::branch(left, right)
}

fn append_items<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    auxiliary_shift: i128,
    revision: Option<u64>,
    output: &mut Vec<T>,
) {
    match &node.kind {
        RangeNodeKind::Leaf(items) => output.extend(items.iter().map(|item| {
            let range = item.range();
            item.with_transform(
                origin
                    .checked_add(range.start)
                    .expect("range-index start is representable")
                    ..origin
                        .checked_add(range.end)
                        .expect("range-index end is representable"),
                auxiliary_shift,
                revision,
            )
            .expect("validated lazy range-index transformation is representable")
        })),
        RangeNodeKind::Branch {
            left,
            left_offset,
            left_auxiliary_shift,
            left_revision,
            right,
            right_offset,
            right_auxiliary_shift,
            right_revision,
        } => {
            append_items(
                left,
                origin
                    .checked_add(*left_offset)
                    .expect("range-index left origin is representable"),
                auxiliary_shift
                    .checked_add(*left_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*left_revision),
                output,
            );
            append_items(
                right,
                origin
                    .checked_add(*right_offset)
                    .expect("range-index right origin is representable"),
                auxiliary_shift
                    .checked_add(*right_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*right_revision),
                output,
            );
        }
    }
}

fn get_item<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    auxiliary_shift: i128,
    revision: Option<u64>,
    index: usize,
    mut stats: Option<&mut QueryStats>,
) -> T {
    if let Some(stats) = stats.as_deref_mut() {
        stats.nodes_visited += 1;
    }
    match &node.kind {
        RangeNodeKind::Leaf(items) => {
            if let Some(stats) = stats.as_deref_mut() {
                stats.items_examined += 1;
            }
            let item = &items[index];
            let relative = item.range();
            item.with_transform(
                origin
                    .checked_add(relative.start)
                    .expect("range-index item start is representable")
                    ..origin
                        .checked_add(relative.end)
                        .expect("range-index item end is representable"),
                auxiliary_shift,
                revision,
            )
            .expect("validated lazy range-index transformation is representable")
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            left_auxiliary_shift,
            left_revision,
            right,
            right_offset,
            right_auxiliary_shift,
            right_revision,
        } => {
            if index < left.item_count {
                get_item(
                    left,
                    origin
                        .checked_add(*left_offset)
                        .expect("range-index left lookup origin is representable"),
                    auxiliary_shift
                        .checked_add(*left_auxiliary_shift)
                        .expect("range-index auxiliary shift is representable"),
                    revision.or(*left_revision),
                    index,
                    stats,
                )
            } else {
                get_item(
                    right,
                    origin
                        .checked_add(*right_offset)
                        .expect("range-index right lookup origin is representable"),
                    auxiliary_shift
                        .checked_add(*right_auxiliary_shift)
                        .expect("range-index auxiliary shift is representable"),
                    revision.or(*right_revision),
                    index - left.item_count,
                    stats,
                )
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_item_range<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    auxiliary_shift: i128,
    revision: Option<u64>,
    base_index: usize,
    indices: &Range<usize>,
    output: &mut Vec<T>,
    mut stats: Option<&mut QueryStats>,
) {
    if let Some(stats) = stats.as_deref_mut() {
        stats.nodes_visited += 1;
    }
    let node_end = base_index
        .checked_add(node.item_count)
        .expect("range-index ordinal aggregate is representable");
    if node_end <= indices.start || base_index >= indices.end {
        return;
    }
    match &node.kind {
        RangeNodeKind::Leaf(items) => {
            let local_start = indices.start.saturating_sub(base_index).min(items.len());
            let local_end = indices.end.saturating_sub(base_index).min(items.len());
            for item in &items[local_start..local_end] {
                if let Some(stats) = stats.as_deref_mut() {
                    stats.items_examined += 1;
                }
                let relative = item.range();
                output.push(
                    item.with_transform(
                        origin
                            .checked_add(relative.start)
                            .expect("range-index item start is representable")
                            ..origin
                                .checked_add(relative.end)
                                .expect("range-index item end is representable"),
                        auxiliary_shift,
                        revision,
                    )
                    .expect("validated lazy range-index transformation is representable"),
                );
            }
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            left_auxiliary_shift,
            left_revision,
            right,
            right_offset,
            right_auxiliary_shift,
            right_revision,
        } => {
            append_item_range(
                left,
                origin
                    .checked_add(*left_offset)
                    .expect("range-index left ordinal origin is representable"),
                auxiliary_shift
                    .checked_add(*left_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*left_revision),
                base_index,
                indices,
                output,
                stats.as_deref_mut(),
            );
            append_item_range(
                right,
                origin
                    .checked_add(*right_offset)
                    .expect("range-index right ordinal origin is representable"),
                auxiliary_shift
                    .checked_add(*right_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*right_revision),
                base_index
                    .checked_add(left.item_count)
                    .expect("range-index right ordinal base is representable"),
                indices,
                output,
                stats,
            );
        }
    }
}

fn find_touching_index<T: RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    offset: usize,
    base_index: usize,
    stats: &mut QueryStats,
) -> Option<usize> {
    stats.nodes_visited += 1;
    let maximum_end = origin.checked_add(node.max_end)?;
    if origin > offset || maximum_end < offset {
        return None;
    }
    match &node.kind {
        RangeNodeKind::Leaf(items) => {
            let relative_offset = offset.checked_sub(origin)?;
            let after = items.partition_point(|item| item.range().start <= relative_offset);
            let local = after.checked_sub(1)?;
            if relative_offset <= items[local].range().end {
                base_index.checked_add(local)
            } else {
                None
            }
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            right,
            right_offset,
            ..
        } => {
            let right_origin = origin.checked_add(*right_offset)?;
            if right_origin <= offset {
                find_touching_index(
                    right,
                    right_origin,
                    offset,
                    base_index.checked_add(left.item_count)?,
                    stats,
                )
                .or_else(|| {
                    find_touching_index(
                        left,
                        origin.checked_add(*left_offset)?,
                        offset,
                        base_index,
                        stats,
                    )
                })
            } else {
                find_touching_index(
                    left,
                    origin.checked_add(*left_offset)?,
                    offset,
                    base_index,
                    stats,
                )
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Intersection {
    Touching,
    Overlapping,
}

impl Intersection {
    fn subtree_misses(
        self,
        node: &RangeNode<impl RangedItem>,
        origin: usize,
        query: &Range<usize>,
    ) -> bool {
        let max_end = origin
            .checked_add(node.max_end)
            .expect("range-index aggregate is representable");
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

#[allow(clippy::too_many_arguments)]
fn query_node<T: Clone + RangedItem>(
    node: &RangeNode<T>,
    origin: usize,
    auxiliary_shift: i128,
    revision: Option<u64>,
    query: &Range<usize>,
    intersection: Intersection,
    matches: &mut Vec<T>,
    mut stats: Option<&mut QueryStats>,
) {
    if let Some(stats) = stats.as_deref_mut() {
        stats.nodes_visited += 1;
    }
    if intersection.subtree_misses(node, origin, query) {
        return;
    }
    match &node.kind {
        RangeNodeKind::Leaf(items) => {
            for item in items {
                if let Some(stats) = stats.as_deref_mut() {
                    stats.items_examined += 1;
                }
                let relative = item.range();
                let range = origin
                    .checked_add(relative.start)
                    .expect("range-index item start is representable")
                    ..origin
                        .checked_add(relative.end)
                        .expect("range-index item end is representable");
                if range.start > query.end {
                    break;
                }
                if intersection.includes(&range, query) {
                    matches.push(
                        item.with_transform(range, auxiliary_shift, revision)
                            .expect("validated lazy range-index transformation is representable"),
                    );
                }
            }
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            left_auxiliary_shift,
            left_revision,
            right,
            right_offset,
            right_auxiliary_shift,
            right_revision,
        } => {
            query_node(
                left,
                origin
                    .checked_add(*left_offset)
                    .expect("range-index left query origin is representable"),
                auxiliary_shift
                    .checked_add(*left_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*left_revision),
                query,
                intersection,
                matches,
                stats.as_deref_mut(),
            );
            query_node(
                right,
                origin
                    .checked_add(*right_offset)
                    .expect("range-index right query origin is representable"),
                auxiliary_shift
                    .checked_add(*right_auxiliary_shift)
                    .expect("range-index auxiliary shift is representable"),
                revision.or(*right_revision),
                query,
                intersection,
                matches,
                stats,
            );
        }
    }
}

fn reuse_node<T: Clone + RangedItem + PartialEq>(
    candidate: Arc<RangeNode<T>>,
    previous: &Arc<RangeNode<T>>,
) -> Arc<RangeNode<T>> {
    match (&candidate.kind, &previous.kind) {
        (RangeNodeKind::Leaf(candidate_items), RangeNodeKind::Leaf(previous_items)) => {
            if candidate_items == previous_items {
                previous.clone()
            } else {
                candidate
            }
        }
        (
            RangeNodeKind::Branch {
                left: candidate_left,
                left_offset: candidate_left_offset,
                left_auxiliary_shift: candidate_left_auxiliary_shift,
                left_revision: candidate_left_revision,
                right: candidate_right,
                right_offset: candidate_right_offset,
                right_auxiliary_shift: candidate_right_auxiliary_shift,
                right_revision: candidate_right_revision,
            },
            RangeNodeKind::Branch {
                left: previous_left,
                left_offset: previous_left_offset,
                left_auxiliary_shift: previous_left_auxiliary_shift,
                left_revision: previous_left_revision,
                right: previous_right,
                right_offset: previous_right_offset,
                right_auxiliary_shift: previous_right_auxiliary_shift,
                right_revision: previous_right_revision,
            },
        ) if candidate_left.item_count == previous_left.item_count
            && candidate_right.item_count == previous_right.item_count =>
        {
            let left = reuse_node(candidate_left.clone(), previous_left);
            let right = reuse_node(candidate_right.clone(), previous_right);
            if Arc::ptr_eq(&left, previous_left)
                && Arc::ptr_eq(&right, previous_right)
                && candidate_left_offset == previous_left_offset
                && candidate_right_offset == previous_right_offset
                && candidate_left_auxiliary_shift == previous_left_auxiliary_shift
                && candidate_right_auxiliary_shift == previous_right_auxiliary_shift
                && candidate_left_revision == previous_left_revision
                && candidate_right_revision == previous_right_revision
            {
                previous.clone()
            } else if Arc::ptr_eq(&left, candidate_left) && Arc::ptr_eq(&right, candidate_right) {
                candidate
            } else {
                RangeNode::branch_with_offsets(
                    left,
                    *candidate_left_offset,
                    *candidate_left_auxiliary_shift,
                    *candidate_left_revision,
                    right,
                    *candidate_right_offset,
                    *candidate_right_auxiliary_shift,
                    *candidate_right_revision,
                )
            }
        }
        _ => candidate,
    }
}

fn node_items_equal<T: PartialEq>(left: &RangeNode<T>, right: &RangeNode<T>) -> bool {
    if std::ptr::eq(left, right) {
        return true;
    }
    match (&left.kind, &right.kind) {
        (RangeNodeKind::Leaf(left), RangeNodeKind::Leaf(right)) => left == right,
        (
            RangeNodeKind::Branch {
                left: left_left,
                left_offset: left_left_offset,
                left_auxiliary_shift: left_left_auxiliary_shift,
                left_revision: left_left_revision,
                right: left_right,
                right_offset: left_right_offset,
                right_auxiliary_shift: left_right_auxiliary_shift,
                right_revision: left_right_revision,
            },
            RangeNodeKind::Branch {
                left: right_left,
                left_offset: right_left_offset,
                left_auxiliary_shift: right_left_auxiliary_shift,
                left_revision: right_left_revision,
                right: right_right,
                right_offset: right_right_offset,
                right_auxiliary_shift: right_right_auxiliary_shift,
                right_revision: right_right_revision,
            },
        ) => {
            left_left_offset == right_left_offset
                && left_right_offset == right_right_offset
                && left_left_auxiliary_shift == right_left_auxiliary_shift
                && left_right_auxiliary_shift == right_right_auxiliary_shift
                && left_left_revision == right_left_revision
                && left_right_revision == right_right_revision
                && node_items_equal(left_left, right_left)
                && node_items_equal(left_right, right_right)
        }
        _ => false,
    }
}

#[cfg(test)]
fn shared_leaf_count<T>(left: &Arc<RangeNode<T>>, right: &Arc<RangeNode<T>>) -> usize {
    fn collect<T>(
        node: &Arc<RangeNode<T>>,
        output: &mut std::collections::HashSet<*const RangeNode<T>>,
    ) {
        match &node.kind {
            RangeNodeKind::Leaf(_) => {
                output.insert(Arc::as_ptr(node));
            }
            RangeNodeKind::Branch { left, right, .. } => {
                collect(left, output);
                collect(right, output);
            }
        }
    }
    fn count<T>(
        node: &Arc<RangeNode<T>>,
        candidates: &std::collections::HashSet<*const RangeNode<T>>,
    ) -> usize {
        match &node.kind {
            RangeNodeKind::Leaf(_) => usize::from(candidates.contains(&Arc::as_ptr(node))),
            RangeNodeKind::Branch { left, right, .. } => {
                count(left, candidates) + count(right, candidates)
            }
        }
    }
    let mut left_leaves = std::collections::HashSet::new();
    collect(left, &mut left_leaves);
    count(right, &left_leaves)
}

#[cfg(test)]
fn validate_node<T: RangedItem>(node: &RangeNode<T>) -> Option<(usize, usize, usize, usize)> {
    match &node.kind {
        RangeNodeKind::Leaf(items) => {
            if items.is_empty() || items.len() > RANGE_INDEX_LEAF_ITEMS {
                return None;
            }
            let starts_ordered = items
                .windows(2)
                .all(|pair| pair[0].range().start <= pair[1].range().start);
            let max_end = items.iter().map(|item| item.range().end).max()?;
            (starts_ordered
                && items.first()?.range().start == 0
                && node.max_end == max_end
                && node.item_count == items.len()
                && node.leaf_count == 1
                && node.height == 1)
                .then_some((max_end, items.len(), 1, 1))
        }
        RangeNodeKind::Branch {
            left,
            left_offset,
            right,
            right_offset,
            ..
        } => {
            let (left_max, left_items, left_leaves, left_height) = validate_node(left)?;
            let (right_max, right_items, right_leaves, right_height) = validate_node(right)?;
            let aggregate_max = left_offset
                .checked_add(left_max)?
                .max(right_offset.checked_add(right_max)?);
            (*left_offset == 0
                && left_height.abs_diff(right_height) <= 1
                && node.max_end == aggregate_max
                && node.item_count == left_items + right_items
                && node.leaf_count == left_leaves + right_leaves
                && node.height == left_height.max(right_height) + 1)
                .then_some((node.max_end, node.item_count, node.leaf_count, node.height))
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
        fn range(&self) -> &Range<usize> {
            &self.0
        }

        fn with_range(&self, range: Range<usize>) -> Self {
            Self(range, self.1)
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
                .iter()
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
                .iter()
                .filter(|item| item.range().start < query.end && query.start < item.range().end)
                .map(|item| item.1)
                .collect::<Vec<_>>();
            let actual = store
                .query_overlapping(&query)
                .into_iter()
                .map(|item| item.1)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query {query:?}");
        }
    }

    #[test]
    fn clones_and_exact_rebuilds_share_roots() {
        let original = IntervalRangeStore::new(vec![Item(1..3, 1), Item(4..8, 2)]);
        let cloned = original.clone();
        assert!(original.shares_root_with(&cloned));

        let mut rebuilt = IntervalRangeStore::new(vec![Item(1..3, 1), Item(4..8, 2)]);
        assert!(!original.shares_root_with(&rebuilt));
        rebuilt.reuse_equal_chunks(&original);
        assert!(original.shares_root_with(&rebuilt));
    }

    #[test]
    fn local_last_leaf_change_shares_all_other_leaves_and_ancestors() {
        let items = (0..512)
            .map(|index| Item(index * 2..index * 2 + 1, index))
            .collect::<Vec<_>>();
        let previous = OrderedRangeStore::new(items.clone());
        let mut changed = items;
        changed.last_mut().unwrap().1 += 1;
        let mut candidate = OrderedRangeStore::new(changed);
        candidate.reuse_equal_chunks(&previous);

        assert_eq!(candidate.leaf_count(), 8);
        assert_eq!(candidate.shared_leaf_count_with(&previous), 7);
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
        assert!(stats.nodes_visited <= 48, "{stats:?}");
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
