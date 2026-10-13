//! Conservative retained-heap accounting for immutable history snapshots.
//!
//! The ledger counts each shared allocation once. Registering an already known
//! tree node adds a reference without walking its children; releasing its last
//! reference iteratively releases its edges. Local persistent edits therefore
//! account only their newly retained paths, rather than scanning every snapshot.
//!
//! One table serves two domains: all retained history states, and the live
//! state alone. The live state is itself retained, so a second table would
//! duplicate every entry. Each entry counts its references per domain and is
//! charged to a domain while that domain references it.

use std::collections::HashMap;
use std::mem::{align_of_val, size_of, size_of_val};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Domain {
    History = 0,
    Live = 1,
}

/// A heap allocation's address and kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct AllocationId(u64);

impl AllocationId {
    // Arc::as_ptr identifies the payload, while Vec::as_ptr identifies its
    // backing allocation. An empty Arc payload can point just past its
    // allocation, at the address of an adjacent live Vec. Keep those address
    // domains distinct even though both describe retained heap allocations.
    fn tagged(address: usize, tag: u8) -> Self {
        let address = address as u64;
        debug_assert!(address >> 61 == 0 && tag < 8, "allocation address fits its tag");
        Self(address << 3 | u64::from(tag))
    }

    fn arc(address: usize) -> Self {
        Self::tagged(address, 0)
    }

    fn vector(address: usize) -> Self {
        Self::tagged(address, 1)
    }

    fn owned(owner: usize, category: u8) -> Self {
        assert!(category < 6, "owned allocation category {category} is out of range");
        Self::tagged(owner, 2 + category)
    }
}

struct Allocation {
    bytes: usize,
    references: [u32; 2],
    children: Box<[AllocationId]>,
}

#[derive(Default)]
pub(super) struct RetainedMemory {
    allocations: HashMap<AllocationId, Allocation>,
    bytes: [usize; 2],
    #[cfg(test)]
    visited: usize,
}

pub(super) struct MemoryVisitor<'a> {
    ledger: &'a mut RetainedMemory,
    domain: usize,
    roots: Vec<AllocationId>,
}

impl RetainedMemory {
    #[cfg(test)]
    pub(super) fn assert_same_allocations(&self, other: &Self) {
        assert_eq!(self.allocations.len(), other.allocations.len());
        assert_eq!(self.bytes, other.bytes);
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

    /// Whether both domains reference exactly the same allocations equally.
    #[cfg(test)]
    pub(super) fn assert_domains_agree(&self) {
        assert_eq!(self.bytes[0], self.bytes[1]);
        for (id, allocation) in &self.allocations {
            assert_eq!(allocation.references[0], allocation.references[1], "domain references differ: {id:?}");
        }
    }

    pub(super) fn capture(
        &mut self,
        domain: Domain,
        visit: impl FnOnce(&mut MemoryVisitor<'_>),
    ) -> Vec<AllocationId> {
        let mut visitor = MemoryVisitor {
            ledger: self,
            domain: domain as usize,
            roots: Vec::new(),
        };
        visit(&mut visitor);
        visitor.roots
    }

    pub(super) fn release(&mut self, domain: Domain, roots: Vec<AllocationId>) {
        let domain = domain as usize;
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
            allocation.references[domain] -= 1;
            if allocation.references[domain] == 0 {
                self.bytes[domain] = self.bytes[domain].saturating_sub(allocation.bytes);
                let children = if allocation.references[1 - domain] == 0 {
                    super::work_statistics::record(|stats| {
                        stats.retained_memory_allocations_released += 1
                    });
                    self.allocations.remove(&id).unwrap().children.into_vec()
                } else {
                    allocation.children.to_vec()
                };
                frames.push(children.into_iter());
            }
        }
    }

    /// Add a domain reference to a known allocation. A domain's first
    /// reference charges it and references its recorded edges.
    fn reference(&mut self, domain: usize, id: AllocationId) {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            let allocation = self.allocations.get_mut(&id).expect("recorded allocation exists");
            allocation.references[domain] += 1;
            if allocation.references[domain] == 1 {
                self.bytes[domain] = self.bytes[domain].saturating_add(allocation.bytes);
                pending.extend_from_slice(&allocation.children);
            }
        }
    }

    /// Allocations charged to a domain, plus the table's unused capacity.
    pub(super) fn bytes(&self, domain: Domain) -> usize {
        self.bytes[domain as usize].saturating_add(
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
        self.ledger.bytes[self.domain]
    }

    pub(super) fn allocation(
        &mut self,
        id: AllocationId,
        bytes: usize,
        visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        super::work_statistics::record(|stats| stats.retained_memory_allocation_visits += 1);
        self.roots.push(id);
        if let Some(allocation) = self.ledger.allocations.get(&id) {
            debug_assert_eq!(
                allocation.bytes - allocation.children.len() * size_of::<AllocationId>() - 96,
                bytes,
                "retained allocation changed size: {id:?}"
            );
            self.ledger.reference(self.domain, id);
            return;
        }
        super::work_statistics::record(|stats| stats.retained_memory_allocations_registered += 1);
        #[cfg(test)]
        {
            self.ledger.visited += 1;
        }
        let parent_roots = std::mem::take(&mut self.roots);
        visit_children(self);
        let children = std::mem::replace(&mut self.roots, parent_roots).into_boxed_slice();
        // Include a conservative allowance for the ledger entry/hash bucket,
        // not just the document allocation this entry represents.
        let child_bytes = children.len() * size_of::<AllocationId>();
        let charged = bytes.saturating_add(96).saturating_add(child_bytes);
        self.ledger.bytes[self.domain] = self.ledger.bytes[self.domain].saturating_add(charged);
        let mut references = [0; 2];
        references[self.domain] = 1;
        self.ledger.allocations.insert(
            id,
            Allocation {
                bytes: charged,
                references,
                children,
            },
        );
    }

    pub(super) fn owned(&mut self, owner: usize, category: u8, bytes: usize) {
        if bytes != 0 {
            self.allocation(AllocationId::owned(owner, category), bytes, |_| {});
        }
    }

    /// One graph edge per shared allocation in this parent, even when many
    /// packed values reference the same immutable default attributes.
    pub(super) fn arc_once<T: ?Sized>(
        &mut self, value: &Arc<T>, visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        let id = AllocationId::arc(Arc::as_ptr(value).cast::<u8>() as usize);
        if !self.roots.contains(&id) { self.arc(value, visit_children); }
    }

    /// `arc_once` for a payload that alone owns `owned_bytes` of further heap.
    pub(super) fn arc_owning_once<T: ?Sized>(
        &mut self, value: &Arc<T>, owned_bytes: usize, visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        let id = AllocationId::arc(Arc::as_ptr(value).cast::<u8>() as usize);
        if !self.roots.contains(&id) { self.arc_owning(value, owned_bytes, visit_children); }
    }

    pub(super) fn arc<T: ?Sized>(
        &mut self,
        value: &Arc<T>,
        visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        self.arc_owning(value, 0, visit_children);
    }

    /// An `Arc` whose immutable payload alone owns `owned_bytes` of further
    /// heap, such as a persistent node's item vector. One ledger entry then
    /// covers both allocations.
    pub(super) fn arc_owning<T: ?Sized>(
        &mut self,
        value: &Arc<T>,
        owned_bytes: usize,
        visit_children: impl FnOnce(&mut MemoryVisitor<'_>),
    ) {
        let bytes = size_of_val(value.as_ref())
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(align_of_val(value.as_ref()) - 1)
            .saturating_add(16)
            .saturating_add(owned_bytes);
        self.allocation(
            AllocationId::arc(Arc::as_ptr(value).cast::<u8>() as usize),
            bytes,
            visit_children,
        );
    }

    pub(super) fn vector<T>(&mut self, values: &Vec<T>, extra_owned_bytes: usize) {
        let bytes = vector_bytes(values, extra_owned_bytes);
        if bytes != 0 {
            self.allocation(AllocationId::vector(values.as_ptr() as usize), bytes, |_| {});
        }
    }
}

/// The charge for a vector's allocation and the heap its elements own.
pub(super) fn vector_bytes<T>(values: &Vec<T>, extra_owned_bytes: usize) -> usize {
    if values.capacity() != 0 && size_of::<T>() != 0 {
        values
            .capacity()
            .saturating_mul(size_of::<T>())
            .saturating_add(extra_owned_bytes)
            .saturating_add(16)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_subtrees_are_charged_once_and_not_revisited() {
        let leaf = Arc::new([1_u8; 8192]);
        let mut memory = RetainedMemory::default();
        let first = memory.capture(Domain::History, |visitor| visitor.arc(&leaf, |_| {}));
        let bytes = memory.bytes(Domain::History);
        let second =
            memory.capture(Domain::History, |visitor| visitor.arc(&leaf, |_| panic!("shared leaf revisited")));
        assert_eq!(memory.bytes(Domain::History), bytes);
        assert_eq!(memory.visited, 1);
        memory.release(Domain::History, first);
        assert_eq!(memory.bytes(Domain::History), bytes);
        memory.release(Domain::History, second);
        assert_eq!(memory.bytes(Domain::History), 0);
    }

    #[test]
    fn releasing_one_root_preserves_shared_children_until_last_reference() {
        let leaf = Arc::new([0_u8; 100]);
        let a = Arc::new(leaf.clone());
        let b = Arc::new(leaf.clone());
        let mut memory = RetainedMemory::default();
        let roots_a = memory.capture(Domain::History, |v| v.arc(&a, |v| v.arc(&leaf, |_| {})));
        let first_bytes = memory.bytes(Domain::History);
        let roots_b = memory.capture(Domain::History, |v| v.arc(&b, |v| v.arc(&leaf, |_| {})));
        assert!(memory.bytes(Domain::History) < first_bytes * 2);
        memory.release(Domain::History, roots_a);
        assert_eq!(memory.bytes(Domain::History), first_bytes);
        memory.release(Domain::History, roots_b);
        assert_eq!(memory.bytes(Domain::History), 0);
    }

    #[test]
    fn releasing_wide_shared_graph_preserves_remaining_root_accounting() {
        let leaves: Vec<_> = (0..50_000).map(|_| Arc::new([0_u8; 8])).collect();
        let first = Arc::new(leaves.clone());
        let second = Arc::new(leaves);
        let capture = |memory: &mut RetainedMemory, root: &Arc<Vec<Arc<[u8; 8]>>>| {
            memory.capture(Domain::History, |visitor| {
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
        memory.release(Domain::History, first_roots);
        let mut expected = RetainedMemory::default();
        let _ = capture(&mut expected, &second);
        memory.assert_same_allocations(&expected);
        memory.release(Domain::History, second_roots);
        assert_eq!(memory.allocation_count(), 0);
        assert_eq!(memory.bytes(Domain::History), 0);
    }

    #[test]
    fn domains_share_entries_and_charge_each_reference_once() {
        let leaf = Arc::new([0_u8; 100]);
        let old = Arc::new(leaf.clone());
        let live = Arc::new(leaf.clone());
        let mut memory = RetainedMemory::default();
        let capture = |memory: &mut RetainedMemory, domain, root: &Arc<Arc<[u8; 100]>>| {
            memory.capture(domain, |v| v.arc(root, |v| v.arc(&leaf, |_| {})))
        };
        let history_old = capture(&mut memory, Domain::History, &old);
        let history_live = capture(&mut memory, Domain::History, &live);
        let history = memory.bytes(Domain::History);
        // The live domain reuses the recorded edges of a known allocation.
        let live_roots = memory.capture(Domain::Live, |v| v.arc(&live, |_| panic!("known node revisited")));
        assert_eq!(memory.allocation_count(), 3);
        assert!(memory.bytes(Domain::Live) < history);
        let mut expected = RetainedMemory::default();
        let _ = capture(&mut expected, Domain::History, &old);
        let _ = capture(&mut expected, Domain::History, &live);
        let _ = capture(&mut expected, Domain::Live, &live);
        memory.assert_same_allocations(&expected);
        // Live references keep shared entries after their history roots go.
        memory.release(Domain::History, history_old);
        memory.release(Domain::History, history_live);
        assert_eq!(memory.allocation_count(), 2);
        assert_eq!(memory.bytes[Domain::History as usize], 0);
        memory.release(Domain::Live, live_roots);
        assert_eq!(memory.allocation_count(), 0);
        assert_eq!(memory.bytes, [0, 0]);
    }

    #[test]
    fn a_local_edit_in_large_text_accounts_only_new_paths_and_invalidates_old_charge() {
        use crate::document::FormattedTextTree;
        let original =
            FormattedTextTree::try_from_text("paragraph text\n".repeat(100_000)).unwrap();
        let mut memory = RetainedMemory::default();
        let roots = memory.capture(Domain::History, |v| original.visit_retained_memory(v));
        let initial_bytes = memory.bytes(Domain::History);
        let initial_visits = memory.visited;
        assert!(initial_visits > 100);
        let changed = original.splice(700_000..700_001, "x").unwrap();
        let changed_roots = memory.capture(Domain::History, |v| changed.visit_retained_memory(v));
        assert!(
            memory.visited - initial_visits < 80,
            "shared document was rescanned"
        );
        assert!(memory.bytes(Domain::History) - initial_bytes < 32 * 1024);
        memory.release(Domain::History, roots);
        memory.release(Domain::History, changed_roots);
        assert_eq!(memory.bytes(Domain::History), 0);
    }
}
