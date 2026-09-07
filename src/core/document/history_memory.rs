//! Conservative retained-heap accounting for immutable history snapshots.
//!
//! The ledger counts each shared allocation once. Registering an already known
//! tree node adds a reference without walking its children; releasing its last
//! reference recursively releases its edges. Local persistent edits therefore
//! account only their newly retained paths, rather than scanning every snapshot.

use std::collections::HashMap;
use std::mem::{align_of_val, size_of, size_of_val};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum AllocationId {
    Heap(usize),
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
        for id in roots {
            let allocation = self
                .allocations
                .get_mut(&id)
                .expect("retained allocation exists");
            allocation.references -= 1;
            if allocation.references == 0 {
                let allocation = self.allocations.remove(&id).unwrap();
                self.bytes = self.bytes.saturating_sub(allocation.bytes);
                self.release(allocation.children);
            }
        }
        if self.allocations.is_empty() {
            self.allocations.shrink_to_fit();
        } else if self.allocations.capacity() > self.allocations.len().saturating_mul(4).max(64) {
            self.allocations
                .shrink_to(self.allocations.len().saturating_mul(2));
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
        self.roots.push(id);
        if let Some(allocation) = self.ledger.allocations.get_mut(&id) {
            debug_assert_eq!(
                allocation.bytes - allocation.children.capacity() * size_of::<AllocationId>() - 96,
                bytes
            );
            allocation.references += 1;
            return;
        }
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
            AllocationId::Heap(Arc::as_ptr(value).cast::<u8>() as usize),
            bytes,
            visit_children,
        );
    }

    pub(super) fn vector<T>(&mut self, values: &Vec<T>, extra_owned_bytes: usize) {
        if values.capacity() != 0 && size_of::<T>() != 0 {
            self.allocation(
                AllocationId::Heap(values.as_ptr() as usize),
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
