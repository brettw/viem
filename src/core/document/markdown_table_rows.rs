//! Persistent table metadata with logarithmic borrowed row lookup. A source
//! edit shifts the untouched suffix in the range tree; the small compatibility
//! cache only resolves rows actually queried by commands or the viewport.
use super::range_index::{OrderedRangeStore, RangeSpliceStats, RangedItem};
use std::ops::{Index, IndexMut, Range};
use std::sync::{Arc, OnceLock};

pub struct TableRows<T> {
    store: OrderedRangeStore<T>,
    lookup: Arc<RowLookup<T>>,
    building: Option<Vec<T>>,
}
struct RowLookup<T> {
    range: Range<usize>,
    children: OnceLock<(Box<RowLookup<T>>, Box<RowLookup<T>>)>,
    value: OnceLock<T>,
}
impl<T: Clone + RangedItem> RowLookup<T> {
    fn new(range: Range<usize>) -> Self {
        Self {
            range,
            children: OnceLock::new(),
            value: OnceLock::new(),
        }
    }
    fn get<'a>(&'a self, at: usize, store: &OrderedRangeStore<T>) -> Option<&'a T> {
        if !self.range.contains(&at) {
            return None;
        }
        if self.range.len() == 1 {
            return Some(
                self.value
                    .get_or_init(|| store.get(at).expect("validated row ordinal")),
            );
        }
        let mid = self.range.start + self.range.len() / 2;
        let (left, right) = self.children.get_or_init(|| {
            (
                Box::new(Self::new(self.range.start..mid)),
                Box::new(Self::new(mid..self.range.end)),
            )
        });
        if at < mid {
            left.get(at, store)
        } else {
            right.get(at, store)
        }
    }
    fn visit(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        if let Some(value) = self.value.get() {
            visitor.owned(value as *const T as usize, 0, value.owned_heap_bytes());
        }
        if let Some((left, right)) = self.children.get() {
            visitor.owned(
                left.as_ref() as *const Self as usize,
                1,
                std::mem::size_of::<Self>(),
            );
            visitor.owned(
                right.as_ref() as *const Self as usize,
                1,
                std::mem::size_of::<Self>(),
            );
            left.visit(visitor);
            right.visit(visitor);
        }
    }
}
impl<T: Clone + RangedItem> Default for TableRows<T> {
    fn default() -> Self {
        Self::from(Vec::new())
    }
}
impl<T: Clone + RangedItem> From<Vec<T>> for TableRows<T> {
    fn from(rows: Vec<T>) -> Self {
        Self {
            store: OrderedRangeStore::new(Vec::new()),
            lookup: Arc::new(RowLookup::new(0..rows.len())),
            building: Some(rows),
        }
    }
}
impl<T: Clone + RangedItem> Clone for TableRows<T> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            lookup: Arc::clone(&self.lookup),
            building: self.building.clone(),
        }
    }
}
impl<T: Clone + RangedItem + std::fmt::Debug> std::fmt::Debug for TableRows<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}
impl<T: Clone + RangedItem + PartialEq> PartialEq for TableRows<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.building.is_none() && other.building.is_none() {
            self.store == other.store
        } else {
            self.iter().eq(other.iter())
        }
    }
}
impl<T: Clone + RangedItem + Eq> Eq for TableRows<T> {}
#[allow(private_bounds)]
impl<T: Clone + RangedItem> TableRows<T> {
    pub fn len(&self) -> usize {
        self.building
            .as_ref()
            .map_or_else(|| self.store.len(), Vec::len)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        if let Some(rows) = &self.building {
            rows.get(index)
        } else {
            self.lookup.get(index, &self.store)
        }
    }
    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }
    pub fn last(&self) -> Option<&T> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }
    pub fn iter(&self) -> TableRowsIter<'_, T> {
        TableRowsIter {
            rows: self,
            range: 0..self.len(),
        }
    }
    pub fn partition_point(&self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let mid = low + (high - low) / 2;
            if predicate(self.get(mid).unwrap()) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }
    fn make_mut(&mut self) -> &mut Vec<T> {
        if self.building.is_none() {
            self.building = Some(self.store.to_vec());
        }
        self.building.as_mut().unwrap()
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.make_mut().iter_mut()
    }
    pub fn push(&mut self, value: T) {
        self.make_mut().push(value);
    }
    pub(super) fn is_frozen(&self) -> bool {
        self.building.is_none()
    }
    pub(super) fn freeze(&mut self) {
        if let Some(rows) = self.building.take() {
            self.store = OrderedRangeStore::new(rows);
            self.lookup = Arc::new(RowLookup::new(0..self.store.len()));
        }
    }
    pub(super) fn visit_retained_memory(
        &self,
        visitor: &mut super::history_memory::MemoryVisitor<'_>,
    ) {
        self.store.visit_retained_memory(visitor);
        visitor.arc(&self.lookup, |visitor| self.lookup.visit(visitor));
        if let Some(rows) = &self.building {
            visitor.vector(rows, rows.iter().map(RangedItem::owned_heap_bytes).sum());
        }
    }
    pub(super) fn splice_transformed(
        &self,
        indices: Range<usize>,
        replacement: Vec<T>,
        old_text_end: usize,
        new_text_end: usize,
        old_source_end: usize,
        new_source_end: usize,
        stats: &mut RangeSpliceStats,
    ) -> Option<Self> {
        let mut rows = self.clone();
        rows.freeze();
        let store = rows.store.splice_transformed(
            indices,
            replacement,
            old_text_end,
            new_text_end,
            Some((old_source_end, new_source_end)),
            None,
            stats,
        )?;
        Some(Self {
            lookup: Arc::new(RowLookup::new(0..store.len())),
            store,
            building: None,
        })
    }
}
impl<T: Clone + RangedItem> Index<usize> for TableRows<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.get(index).expect("table row index")
    }
}
impl<T: Clone + RangedItem> IndexMut<usize> for TableRows<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        &mut self.make_mut()[index]
    }
}
pub struct TableRowsIter<'a, T> {
    rows: &'a TableRows<T>,
    range: Range<usize>,
}
impl<'a, T: Clone + RangedItem> Iterator for TableRowsIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        self.range.next().and_then(|index| self.rows.get(index))
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.range.nth(n).and_then(|index| self.rows.get(index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}
impl<T: Clone + RangedItem> DoubleEndedIterator for TableRowsIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.range
            .next_back()
            .and_then(|index| self.rows.get(index))
    }
    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.range
            .nth_back(n)
            .and_then(|index| self.rows.get(index))
    }
}
impl<T: Clone + RangedItem> ExactSizeIterator for TableRowsIter<'_, T> {}
impl<'a, T: Clone + RangedItem> IntoIterator for &'a TableRows<T> {
    type Item = &'a T;
    type IntoIter = TableRowsIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, T: Clone + RangedItem> IntoIterator for &'a mut TableRows<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
