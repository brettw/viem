//! Conservative retained-heap accounting for immutable history snapshots.
//!
//! The ledger counts each shared allocation once. Registering an already known
//! tree node adds a reference without walking its children; releasing its last
//! reference iteratively releases its edges. Local persistent edits therefore
//! account only their newly retained paths, rather than scanning every snapshot.

use std::collections::HashMap;
use std::mem::{align_of_val, size_of, size_of_val};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum AllocationId {
    // Arc::as_ptr identifies the payload, while Vec::as_ptr identifies its
    // backing allocation. An empty Arc payload can point just past its
    // allocation, at the address of an adjacent live Vec. Keep those address
    // domains distinct even though both describe retained heap allocations.
    Arc(usize),
    Vector(usize),
    Owned(usize, u8),
}

struct Allocation {
    bytes: usize,
    references: usize,
    children: Vec<AllocationId>,
}

#[derive(Default)]
pub(super) struct RetainedMemory {
    allocations: HashMap<AllocationId, Allocation>,
    bytes: usize,
    bookkeeping_child_bytes: usize,
    #[cfg(test)]
    visited: usize,
}

pub(super) struct MemoryVisitor<'a> {
    ledger: &'a mut RetainedMemory,
    roots: Vec<AllocationId>,
}

impl RetainedMemory {
    #[cfg(test)]
    pub(super) fn assert_same_allocations(&self, other: &Self) {
        assert_eq!(self.allocations.len(), other.allocations.len());
        for (id, allocation) in &self.allocations {
            let expected = &other.allocations[id];
            assert_eq!(
                allocation.bytes, expected.bytes,
                "allocation charge differs: {id:?}"
            );
            assert_eq!(
                allocation.references, expected.references,
                "allocation references differ: {id:?}"
            );
            assert_eq!(
                allocation.children, expected.children,
                "allocation children differ: {id:?}"
            );
        }
    }

    pub(super) fn capture(
        &mut self,
        visit: impl FnOnce(&mut MemoryVisitor<'_>),
    ) -> Vec<AllocationId> {
        let mut visitor = MemoryVisitor {
            ledger: self,
            roots: Vec::new(),
        };
        visit(&mut visitor);
        visitor.roots
    }

    pub(super) fn release(&mut self, roots: Vec<AllocationId>) {
        // Preserve depth-first edge order and postorder capacity maintenance
        // without putting arbitrarily deep ownership chains on the call stack.
        let mut frames = vec![roots.into_iter()];
        while let Some(frame) = frames.last_mut() {
            let Some(id) = frame.next() else {
                frames.pop();
                if self.allocations.is_empty() {
                    self.allocations.shrink_to_fit();
                } else if self.allocations.capacity()
                    > self.allocations.len().saturating_mul(4).max(64)
                {
                    self.allocations
                        .shrink_to(self.allocations.len().saturating_mul(2));
                }
                continue;
            };
            super::work_statistics::record(|stats| stats.retained_memory_release_visits += 1);
            let allocation = self
                .allocations
                .get_mut(&id)
                .expect("retained allocation exists");
            allocation.references -= 1;
            if allocation.references == 0 {
                super::work_statistics::record(|stats| {
                    stats.retained_memory_allocations_released += 1
                });
                let allocation = self.allocations.remove(&id).unwrap();
                self.bytes = self.bytes.saturating_sub(allocation.bytes);
                self.bookkeeping_child_bytes -= allocation.children.capacity() * size_of::<AllocationId>();
                frames.push(allocation.children.into_iter());
            }
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.bytes.saturating_add(
            self.allocations
                .capacity()
                .saturating_sub(self.allocations.len())
                * 96,
        )
    }

    pub(super) fn allocation_count(&self) -> usize {
        self.allocations.len()
    }

    /// The ledger's own storage, excluding the allocations it describes.
    /// Useful when a second incremental ledger tracks the active state: the
    /// shared document allocations count once, but both ledgers occupy heap.
    pub(super) fn bookkeeping_bytes(&self) -> usize {
        self.allocations.capacity().saturating_mul(96)
            .saturating_add(self.bookkeeping_child_bytes)
    }
}

impl MemoryVisitor<'_> {
    pub(super) fn retained_bytes(&self) -> usize {
        self.ledger.bytes
    }

    pub(super) fn allocation(
        &mut self,
        id: AllocationId,
        bytes: usize,
        visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        super::work_statistics::record(|stats| stats.retained_memory_allocation_visits += 1);
        self.roots.push(id);
        if let Some(allocation) = self.ledger.allocations.get_mut(&id) {
            debug_assert_eq!(
                allocation.bytes - allocation.children.capacity() * size_of::<AllocationId>() - 96,
                bytes,
                "retained allocation changed size: {id:?}"
            );
            allocation.references += 1;
            return;
        }
        super::work_statistics::record(|stats| stats.retained_memory_allocations_registered += 1);
        #[cfg(test)]
        {
            self.ledger.visited += 1;
        }
        let parent_roots = std::mem::take(&mut self.roots);
        visit_children(self);
        let children = std::mem::replace(&mut self.roots, parent_roots);
        // Include a conservative allowance for the ledger entry/hash bucket,
        // not just the document allocation this entry represents.
        let charged = bytes
            .saturating_add(96)
            .saturating_add(children.capacity() * size_of::<AllocationId>());
        self.ledger.bytes = self.ledger.bytes.saturating_add(charged);
        self.ledger.bookkeeping_child_bytes += children.capacity() * size_of::<AllocationId>();
        self.ledger.allocations.insert(
            id,
            Allocation {
                bytes: charged,
                references: 1,
                children,
            },
        );
    }

    pub(super) fn owned(&mut self, owner: usize, category: u8, bytes: usize) {
        if bytes != 0 {
            self.allocation(AllocationId::Owned(owner, category), bytes, |_| {});
        }
    }

    /// One graph edge per shared allocation in this parent, even when many
    /// packed values reference the same immutable default attributes.
    pub(super) fn arc_once<T: ?Sized>(
        &mut self, value: &Arc<T>, visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        let id = AllocationId::Arc(Arc::as_ptr(value).cast::<u8>() as usize);
        if !self.roots.contains(&id) { self.arc(value, visit_children); }
    }

    pub(super) fn arc<T: ?Sized>(
        &mut self,
        value: &Arc<T>,
        visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        let bytes = size_of_val(value.as_ref())
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(align_of_val(value.as_ref()) - 1)
            .saturating_add(16);
        self.allocation(
            AllocationId::Arc(Arc::as_ptr(value).cast::<u8>() as usize),
            bytes,
            visit_children,
        );
    }

    pub(super) fn vector<T>(&mut self, values: &Vec<T>, extra_owned_bytes: usize) {
        if values.capacity() != 0 && size_of::<T>() != 0 {
            self.allocation(
                AllocationId::Vector(values.as_ptr() as usize),
                values
                    .capacity()
                    .saturating_mul(size_of::<T>())
                    .saturating_add(extra_owned_bytes)
                    .saturating_add(16),
                |_| {},
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_arc_payload_and_adjacent_vector_have_distinct_allocation_identities() {
        // Model the legal address collision independently of allocator layout:
        // an empty Arc payload is one-past its allocation, and the next Vec
        // allocation can begin at exactly that address.
        let address = 0x1000;
        let mut memory = RetainedMemory::default();
        let arc = memory.capture(|v| v.allocation(AllocationId::Arc(address), 32, |_| {}));
        let vector = memory.capture(|v| v.allocation(AllocationId::Vector(address), 48, |_| {}));
        assert_eq!(memory.allocation_count(), 2);
        assert_eq!(memory.allocations[&AllocationId::Arc(address)].bytes, 32 + 96);
        assert_eq!(memory.allocations[&AllocationId::Vector(address)].bytes, 48 + 96);
        memory.release(arc);
        assert_eq!(memory.allocation_count(), 1);
        assert!(memory.allocations.contains_key(&AllocationId::Vector(address)));
        memory.release(vector);
        assert_eq!(memory.bytes(), 0);
    }

    #[test]
    fn shared_subtrees_are_charged_once_and_not_revisited() {
        let leaf = Arc::new([1_u8; 8192]);
        let mut memory = RetainedMemory::default();
        let first = memory.capture(|visitor| visitor.arc(&leaf, |_| {}));
        let bytes = memory.bytes();
        let second =
            memory.capture(|visitor| visitor.arc(&leaf, |_| panic!("shared leaf revisited")));
        assert_eq!(memory.bytes(), bytes);
        assert_eq!(memory.visited, 1);
        memory.release(first);
        assert_eq!(memory.bytes(), bytes);
        memory.release(second);
        assert_eq!(memory.bytes(), 0);
    }

    #[test]
    fn releasing_one_root_preserves_shared_children_until_last_reference() {
        let leaf = Arc::new([0_u8; 100]);
        let a = Arc::new(leaf.clone());
        let b = Arc::new(leaf.clone());
        let mut memory = RetainedMemory::default();
        let roots_a = memory.capture(|v| v.arc(&a, |v| v.arc(&leaf, |_| {})));
        let first_bytes = memory.bytes();
        let roots_b = memory.capture(|v| v.arc(&b, |v| v.arc(&leaf, |_| {})));
        assert!(memory.bytes() < first_bytes * 2);
        memory.release(roots_a);
        assert_eq!(memory.bytes(), first_bytes);
        memory.release(roots_b);
        assert_eq!(memory.bytes(), 0);
    }

    #[test]
    fn releasing_wide_shared_graph_preserves_remaining_root_accounting() {
        let leaves: Vec<_> = (0..50_000).map(|_| Arc::new([0_u8; 8])).collect();
        let first = Arc::new(leaves.clone());
        let second = Arc::new(leaves);
        let capture = |memory: &mut RetainedMemory, root: &Arc<Vec<Arc<[u8; 8]>>>| {
            memory.capture(|visitor| {
                visitor.arc(root, |visitor| {
                    visitor.vector(root, 0);
                    for leaf in root.iter() {
                        visitor.arc(leaf, |_| {});
                    }
                });
            })
        };
        let mut memory = RetainedMemory::default();
        let first_roots = capture(&mut memory, &first);
        let second_roots = capture(&mut memory, &second);
        memory.release(first_roots);
        let mut expected = RetainedMemory::default();
        let _ = capture(&mut expected, &second);
        memory.assert_same_allocations(&expected);
        memory.release(second_roots);
        assert_eq!(memory.allocation_count(), 0);
        assert_eq!(memory.bytes(), 0);
    }

    #[test]
    fn a_local_edit_in_large_text_accounts_only_new_paths_and_invalidates_old_charge() {
        use crate::document::FormattedTextTree;
        let original =
            FormattedTextTree::try_from_text("paragraph text\n".repeat(100_000)).unwrap();
        let mut memory = RetainedMemory::default();
        let roots = memory.capture(|v| original.visit_retained_memory(v));
        let initial_bytes = memory.bytes();
        let initial_visits = memory.visited;
        assert!(initial_visits > 100);
        let changed = original.splice(700_000..700_001, "x").unwrap();
        let changed_roots = memory.capture(|v| changed.visit_retained_memory(v));
        assert!(
            memory.visited - initial_visits < 80,
            "shared document was rescanned"
        );
        assert!(memory.bytes() - initial_bytes < 32 * 1024);
        memory.release(roots);
        memory.release(changed_roots);
        assert_eq!(memory.bytes(), 0);
    }
}
