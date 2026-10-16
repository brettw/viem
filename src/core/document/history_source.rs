//! Incremental accounting of the source buffers retained by history.
//!
//! Every retained state's persistent source rope is registered as a graph of
//! rope nodes and backing buffers. Registering an already known rope node only
//! adds a reference; releasing a node's last reference releases its edges.
//! Committing a local edit therefore visits only its new rope path, and pruning
//! a state visits only the nodes it alone retained, independently of how many
//! pieces earlier edits created or how many other states history keeps.

use std::collections::HashMap;

/// A rope node or backing buffer address, tagged so the two cannot collide.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceRetentionId(usize);

impl SourceRetentionId {
    fn node(address: usize) -> Self {
        Self(address << 1)
    }

    fn buffer(address: usize) -> Self {
        Self(address << 1 | 1)
    }
}

struct Entry {
    references: usize,
    /// Backing-buffer bytes; zero for rope nodes.
    bytes: usize,
    children: [Option<SourceRetentionId>; 2],
}

/// Reference counts for every rope node and buffer reachable from retained
/// states, with the total bytes of distinct retained backing buffers.
#[derive(Default)]
pub(crate) struct SourceRetention {
    entries: HashMap<SourceRetentionId, Entry>,
    bytes: usize,
}

pub(crate) struct SourceRetentionVisitor<'a> {
    ledger: &'a mut SourceRetention,
    roots: Vec<SourceRetentionId>,
}

impl SourceRetention {
    /// Register one state's source graph and return the roots that release it.
    pub(crate) fn capture(
        &mut self,
        visit: impl FnOnce(&mut SourceRetentionVisitor<'_>),
    ) -> Vec<SourceRetentionId> {
        let mut visitor = SourceRetentionVisitor { ledger: self, roots: Vec::new() };
        visit(&mut visitor);
        visitor.roots
    }

    /// Release roots returned by [`Self::capture`]. Call this before the state
    /// they describe is dropped, so no released address can be reused first.
    pub(crate) fn release(&mut self, roots: Vec<SourceRetentionId>) {
        let mut pending = roots;
        while let Some(id) = pending.pop() {
            super::work_statistics::record(|stats| stats.history_source_release_visits += 1);
            let entry = self.entries.get_mut(&id).expect("retained source entry exists");
            entry.references -= 1;
            if entry.references == 0 {
                let entry = self.entries.remove(&id).expect("retained source entry exists");
                self.bytes -= entry.bytes;
                pending.extend(entry.children.into_iter().flatten());
            }
        }
        if self.entries.capacity() > self.entries.len().saturating_mul(4).max(64) {
            self.entries.shrink_to(self.entries.len().saturating_mul(2));
        }
    }

    /// Bytes of the distinct backing buffers reachable from captured roots.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// A conservative charge for the ledger's own entries. It depends only on
    /// what is retained, so a rolled-back command restores exactly the same
    /// accounting even if the table's spare capacity grew meanwhile.
    pub(crate) fn metadata_bytes(&self) -> usize {
        self.entries.len()
            * (std::mem::size_of::<SourceRetentionId>() + std::mem::size_of::<Entry>() + 16)
    }
}

impl SourceRetentionVisitor<'_> {
    /// A persistent rope node. Its children are visited only when this node is
    /// not already retained; otherwise its existing edges already count them.
    pub(crate) fn node(&mut self, address: usize, visit_children: impl FnOnce(&mut Self)) {
        let id = SourceRetentionId::node(address);
        super::work_statistics::record(|stats| stats.history_source_visits += 1);
        self.roots.push(id);
        if let Some(entry) = self.ledger.entries.get_mut(&id) {
            entry.references += 1;
            return;
        }
        let parent_roots = std::mem::take(&mut self.roots);
        visit_children(self);
        let children = std::mem::replace(&mut self.roots, parent_roots);
        assert!(children.len() <= 2, "a source rope node has at most two edges");
        let mut edges = [None; 2];
        for (edge, child) in edges.iter_mut().zip(children) {
            *edge = Some(child);
        }
        self.ledger.entries.insert(id, Entry { references: 1, bytes: 0, children: edges });
    }

    /// An immutable backing buffer, charged once however many pieces slice it.
    pub(crate) fn buffer(&mut self, address: usize, bytes: usize) {
        let id = SourceRetentionId::buffer(address);
        super::work_statistics::record(|stats| stats.history_source_visits += 1);
        self.roots.push(id);
        if let Some(entry) = self.ledger.entries.get_mut(&id) {
            debug_assert_eq!(entry.bytes, bytes, "retained source buffer changed size");
            entry.references += 1;
            return;
        }
        self.ledger.bytes += bytes;
        self.ledger.entries.insert(id, Entry { references: 1, bytes, children: [None; 2] });
    }
}
