use super::history_memory::{AllocationId, MemoryVisitor, RetainedMemory};
use super::source::{SourceArtifactDigest, SourceSnapshotIdentity};
use super::{PositionError, PositionMap, Revision, SourcePartId, Splice, TextAnchor};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod timeline;
pub use timeline::{HistoryTimeAmount, HistoryTimelineEntry};

static NEXT_HISTORY_NODE_ID: AtomicU64 = AtomicU64::new(1);

/// Resource limits applied to the in-memory branching undo tree.
///
/// A budget is a target rather than permission to discard the active state.
/// Consequently one oversized current snapshot, or the parent and result of
/// an open undo unit, may temporarily exceed these values.
/// Total diagnostics charge source, projections, maps and bookkeeping. The
/// default target gives history headroom above current live state; explicitly
/// constructed combined targets can instead bound their total together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryRetentionPolicy {
    node_budget: usize,
    retained_byte_budget: usize,
    additional_to_live_state: bool,
}

impl HistoryRetentionPolicy {
    pub const DEFAULT_NODE_BUDGET: usize = 10_000;
    pub const DEFAULT_RETAINED_BYTE_BUDGET: usize = 256 * 1024 * 1024;

    pub const fn new(node_budget: usize, retained_byte_budget: usize) -> Self {
        Self {
            node_budget,
            retained_byte_budget,
            additional_to_live_state: false,
        }
    }

    /// Allow this many bytes of retained history above the current state.
    /// The live-state allocation estimate remains included in diagnostics.
    pub const fn additional_history(node_budget: usize, history_byte_budget: usize) -> Self {
        Self { node_budget, retained_byte_budget: history_byte_budget, additional_to_live_state: true }
    }

    pub const fn is_additional_to_live_state(self) -> bool {
        self.additional_to_live_state
    }

    pub const fn unlimited() -> Self {
        Self::new(usize::MAX, usize::MAX)
    }

    pub const fn node_budget(self) -> usize {
        self.node_budget
    }

    pub const fn retained_byte_budget(self) -> usize {
        self.retained_byte_budget
    }
}

impl Default for HistoryRetentionPolicy {
    fn default() -> Self {
        Self::additional_history(
            Self::DEFAULT_NODE_BUDGET,
            Self::DEFAULT_RETAINED_BYTE_BUDGET,
        )
    }
}

/// Stable identity of one undo-tree node.
///
/// The numeric representation is intentionally private. An ID can be retained
/// and later passed back to the owning [`crate::document::Document`], but it
/// does not expose the source snapshot stored by the node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HistoryNodeId(u64);

/// Monotonically increasing number assigned to a committed undo unit.
///
/// Change zero is the state with which the document was opened. Branching
/// never reuses a number, and commits coalesced into one edit group retain the
/// number of that group's node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HistoryChangeNumber(u64);

impl HistoryChangeNumber {
    pub const INITIAL: Self = Self(0);

    /// Construct a typed user-facing change number for an exact lookup.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for HistoryChangeNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Public identity of a state in the undo tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryLocation {
    pub node: HistoryNodeId,
    pub change: HistoryChangeNumber,
}

/// Result of an exact history navigation operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryNavigation {
    pub from: HistoryLocation,
    pub to: HistoryLocation,
}

/// Stable semantic category retained for one source-changing transaction.
///
/// This deliberately describes the user's change without becoming the
/// authority for undo or redo. History navigation always installs the stored
/// immutable document state instead of replaying this description.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistorySemanticChangeKind {
    Text,
    Style,
    FileFormat,
    HardLineTransfer,
    SourceMetadata,
}

/// Ordered semantic description of all transactions in one undo unit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistorySemanticSummary {
    changes: Vec<HistorySemanticChangeKind>,
}

impl HistorySemanticSummary {
    pub fn changes(&self) -> &[HistorySemanticChangeKind] {
        &self.changes
    }
}

/// Exact retained source-patch metadata used for history auditing.
///
/// Replacement bytes are identified by length and digest instead of being
/// copied into the history record. The adjacent immutable document snapshots
/// remain the byte authority, so this keeps the retention byte budget honest
/// without losing the exact patch coordinates or replacement identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistorySourcePatch {
    part: SourcePartId,
    range: std::ops::Range<usize>,
    replacement_len: usize,
    replacement_digest: SourceArtifactDigest,
}

impl HistorySourcePatch {
    pub(crate) fn new(
        part: SourcePartId,
        range: std::ops::Range<usize>,
        replacement: &[u8],
    ) -> Self {
        Self {
            part,
            range,
            replacement_len: replacement.len(),
            replacement_digest: SourceArtifactDigest::from_bytes(replacement),
        }
    }

    pub fn part(&self) -> SourcePartId {
        self.part
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.range.clone()
    }

    pub fn replacement_len(&self) -> usize {
        self.replacement_len
    }

    pub fn replacement_digest(&self) -> SourceArtifactDigest {
        self.replacement_digest
    }
}

/// Exact summary of one committed source transaction within an undo unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryTransactionSummary {
    before_revision: Revision,
    after_revision: Revision,
    semantic: HistorySemanticChangeKind,
    source_patches: Vec<HistorySourcePatch>,
    formatted_splices: Vec<Splice>,
}

impl HistoryTransactionSummary {
    pub(crate) fn new(
        before_revision: Revision,
        after_revision: Revision,
        semantic: HistorySemanticChangeKind,
        source_patches: Vec<HistorySourcePatch>,
        formatted_splices: Vec<Splice>,
    ) -> Self {
        Self {
            before_revision,
            after_revision,
            semantic,
            source_patches,
            formatted_splices,
        }
    }

    pub fn before_revision(&self) -> Revision {
        self.before_revision
    }

    pub fn after_revision(&self) -> Revision {
        self.after_revision
    }

    pub fn semantic(&self) -> HistorySemanticChangeKind {
        self.semantic
    }

    pub fn source_patches(&self) -> &[HistorySourcePatch] {
        &self.source_patches
    }

    pub fn formatted_splices(&self) -> &[Splice] {
        &self.formatted_splices
    }
}

/// Cursor and required buffer-local marks for one end of an undo unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryRestorationSnapshot {
    cursor: TextAnchor,
    marks: BTreeMap<char, TextAnchor>,
}

impl HistoryRestorationSnapshot {
    pub fn new(cursor: TextAnchor, marks: BTreeMap<char, TextAnchor>) -> Self {
        Self { cursor, marks }
    }

    pub fn cursor(&self) -> TextAnchor {
        self.cursor
    }

    pub fn marks(&self) -> &BTreeMap<char, TextAnchor> {
        &self.marks
    }
}

/// Stable before/after controller restoration state stored by an undo unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryRestoration {
    before: HistoryRestorationSnapshot,
    after: HistoryRestorationSnapshot,
}

impl HistoryRestoration {
    pub fn new(before: HistoryRestorationSnapshot, after: HistoryRestorationSnapshot) -> Self {
        Self { before, after }
    }

    pub fn before(&self) -> &HistoryRestorationSnapshot {
        &self.before
    }

    pub fn after(&self) -> &HistoryRestorationSnapshot {
        &self.after
    }
}

/// Read-only immutable metadata retained for one undo-tree node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryNodeDetails {
    pub location: HistoryLocation,
    pub parent: Option<HistoryLocation>,
    pub semantic: Option<HistorySemanticSummary>,
    pub transactions: Vec<HistoryTransactionSummary>,
    pub restoration: Option<HistoryRestoration>,
}

impl HistoryNavigation {
    pub fn changed(self) -> bool {
        self.from != self.to
    }
}

/// Which end of history prevented a navigation step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryBoundary {
    Oldest,
    NoPreferredRedo,
}

/// Structured failures from public history navigation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryError {
    Boundary(HistoryBoundary),
    RedoBranchNotFound { requested: usize, available: usize },
    NodeNotFound(HistoryNodeId),
    ChangeNotFound(HistoryChangeNumber),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boundary(HistoryBoundary::Oldest) => {
                formatter.write_str("already at the oldest document state")
            }
            Self::Boundary(HistoryBoundary::NoPreferredRedo) => {
                formatter.write_str("no preferred redo state is available")
            }
            Self::RedoBranchNotFound {
                requested,
                available,
            } => write!(
                formatter,
                "redo branch {requested} does not exist ({available} available)"
            ),
            Self::NodeNotFound(node) => write!(formatter, "history node {node:?} was not found"),
            Self::ChangeNotFound(change) => {
                write!(formatter, "history change {change} was not found")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

/// Read-only summary of the current undo-tree state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryStatus {
    pub current: HistoryLocation,
    pub save_point: HistoryNodeId,
    pub parent: Option<HistoryLocation>,
    pub preferred_redo: Option<HistoryLocation>,
    pub node_count: usize,
    pub redo_branch_count: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub is_dirty: bool,
    /// Semantic label for the unit which `undo` would leave.
    pub undo_summary: Option<HistorySemanticSummary>,
    /// Semantic label for the currently preferred redo unit.
    pub redo_summary: Option<HistorySemanticSummary>,
    /// Effective configured targets for this history.
    pub retention: HistoryRetentionPolicy,
    /// Bytes in unique immutable source-buffer allocations reachable from all
    /// retained nodes. Structurally shared buffers are charged exactly once.
    pub retained_source_bytes: usize,
    /// Conservative retained heap estimate, including shared source/projection
    /// allocations, history maps and metadata. Shared allocations count once.
    /// This is the byte-budget charge; it is distinct from source byte length.
    pub retained_memory_bytes: usize,
    /// Current immutable state and the bookkeeping needed to retain that state
    /// alone, independent of older undo states.
    pub live_state_memory_bytes: usize,
    /// Retained bytes above the live-state estimate, including history metadata.
    pub additional_history_memory_bytes: usize,
    /// Whether the persisted state remains available for history navigation.
    /// Its identity and digest survive when this is false.
    pub save_point_retained: bool,
    pub save_point_digest: Option<SourceArtifactDigest>,
}

/// One immediate redo branch from the current history node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryBranch {
    /// Stable enumeration order. Existing ordinals do not change when another
    /// branch is appended to this node.
    pub ordinal: usize,
    pub destination: HistoryLocation,
    pub preferred: bool,
}

/// Branching history of immutable document states.
pub(crate) struct History<T, M = ()> {
    nodes: Vec<Node<T, M>>,
    node_indexes: HashMap<HistoryNodeId, usize>,
    change_indexes: HashMap<HistoryChangeNumber, usize>,
    current: usize,
    group_node: Option<usize>,
    next_change_number: u64,
    next_write_number: u64,
    save_point: HistoryNodeId,
    save_point_digest: Option<SourceArtifactDigest>,
    save_point_source: Option<SourceSnapshotIdentity>,
    retention: HistoryRetentionPolicy,
    retained_source_bytes: usize,
    accounting: Option<HistoryAccounting<T>>,
    memory: RetainedMemory,
    live_memory: RetainedMemory,
    live_memory_roots: Vec<AllocationId>,
    singleton_index_bytes: usize,
    memory_metadata_bytes: usize,
    visit_state_memory: Option<fn(&T, &mut MemoryVisitor<'_>)>,
    visit_map_memory: Option<fn(&M, &mut MemoryVisitor<'_>)>,
    command_checkpoints: Vec<HistoryCommandCheckpoint<T, M>>,
}

/// A short-lived mutation journal, never a copy of the retained history tree.
/// Existing transaction vectors remain in place; rollback truncates only the
/// entries appended by the command. Retention is deferred while this exists.
struct HistoryCommandCheckpoint<T, M> {
    node_count: usize,
    nodes_capacity: usize,
    node_indexes_capacity: usize,
    change_indexes_capacity: usize,
    current: usize,
    group_node: Option<usize>,
    next_change_number: u64,
    next_write_number: u64,
    save_point: HistoryNodeId,
    save_point_digest: Option<SourceArtifactDigest>,
    save_point_source: Option<SourceSnapshotIdentity>,
    retention: HistoryRetentionPolicy,
    retained_source_bytes: usize,
    retention_requested: bool,
    nodes: HashMap<usize, NodeCommandCheckpoint<T, M>>,
}

struct NodeCommandCheckpoint<T, M> {
    children_len: usize,
    children_capacity: usize,
    preferred_child: Option<usize>,
    saved_write: Option<u64>,
    content: Option<NodeContentCheckpoint<T, M>>,
}

struct NodeContentCheckpoint<T, M> {
    state: Arc<T>,
    retained_buffers_capacity: usize,
    edge: Option<EdgeCommandCheckpoint<M>>,
}

struct EdgeCommandCheckpoint<M> {
    map: M,
    semantic_len: usize,
    semantic_capacity: usize,
    transactions_len: usize,
    transactions_capacity: usize,
    restoration: Option<HistoryRestoration>,
    command_restoration_attached: bool,
}

struct Node<T, M> {
    id: HistoryNodeId,
    change: HistoryChangeNumber,
    timestamp: u64,
    creation_write: u64,
    saved_write: Option<u64>,
    state: Arc<T>,
    parent: Option<usize>,
    children: Vec<usize>,
    preferred_child: Option<usize>,
    incoming: Option<HistoryEdge<M>>,
    retained_buffers: Vec<RetainedBuffer>,
    memory_roots: Vec<AllocationId>,
    memory_metadata_bytes: usize,
}

struct HistoryEdge<M> {
    map: M,
    record: HistoryUnitRecord,
}

#[derive(Clone, Debug)]
struct HistoryUnitRecord {
    semantic: HistorySemanticSummary,
    transactions: Vec<HistoryTransactionSummary>,
    restoration: Option<HistoryRestoration>,
    command_restoration_attached: bool,
}

impl HistoryUnitRecord {
    fn owned_heap_bytes(&self) -> usize {
        let mut bytes = self.semantic.changes.capacity() * std::mem::size_of::<HistorySemanticChangeKind>()
            + self.transactions.capacity() * std::mem::size_of::<HistoryTransactionSummary>();
        for transaction in &self.transactions {
            bytes += transaction.source_patches.capacity() * std::mem::size_of::<HistorySourcePatch>()
                + transaction.formatted_splices.capacity() * std::mem::size_of::<Splice>();
        }
        if let Some(restoration) = &self.restoration {
            for snapshot in [&restoration.before, &restoration.after] {
                if !snapshot.marks.is_empty() {
                    bytes += (3 * snapshot.marks.len() + 11) * std::mem::size_of::<(char, TextAnchor)>()
                        + (snapshot.marks.len() + 1) * 128;
                }
            }
        }
        bytes
    }

    fn new(
        transaction: HistoryTransactionSummary,
        restoration: Option<HistoryRestoration>,
    ) -> Self {
        Self {
            semantic: HistorySemanticSummary {
                changes: vec![transaction.semantic],
            },
            transactions: vec![transaction],
            restoration,
            command_restoration_attached: false,
        }
    }

    fn append(
        &mut self,
        transaction: HistoryTransactionSummary,
        restoration: Option<HistoryRestoration>,
    ) {
        self.semantic.changes.push(transaction.semantic);
        self.transactions.push(transaction);
        if let Some(restoration) = restoration {
            match self.restoration.as_mut() {
                Some(current) => current.after = restoration.after,
                None => self.restoration = Some(restoration),
            }
        }
    }

    fn attach_command_restoration(&mut self, restoration: HistoryRestoration) {
        if self.command_restoration_attached {
            if let Some(current) = self.restoration.as_mut() {
                current.after = restoration.after;
            } else {
                self.restoration = Some(restoration);
            }
        } else {
            self.restoration = Some(restoration);
            self.command_restoration_attached = true;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RetainedBuffer {
    identity: usize,
    bytes: usize,
}

struct HistoryAccounting<T> {
    source_identity: fn(&T) -> SourceSnapshotIdentity,
    visit_buffers: fn(&T, &mut dyn FnMut(usize, usize)),
    digest: fn(&T) -> SourceArtifactDigest,
}

impl<T, M> History<T, M> {
    /// Private transaction planning only: keep the current candidate, without
    /// rescanning the shared live document into a second allocation ledger.
    /// Published documents always use accounted history. Scratch planning does
    /// not expose undo, saving, or retention diagnostics to the user.
    pub(crate) fn transient(initial: T) -> Self {
        Self::new_internal(initial, HistoryRetentionPolicy::new(1, usize::MAX), None)
    }

    #[cfg(test)]
    pub(crate) fn new(initial: T) -> Self {
        Self::new_internal(initial, HistoryRetentionPolicy::default(), None)
    }

    pub(crate) fn new_accounted(
        initial: T,
        retention: HistoryRetentionPolicy,
        visit_buffers: fn(&T, &mut dyn FnMut(usize, usize)),
        digest: fn(&T) -> SourceArtifactDigest,
        source_identity: fn(&T) -> SourceSnapshotIdentity,
    ) -> Self {
        Self::new_internal(
            initial,
            retention,
            Some(HistoryAccounting {
                source_identity,
                visit_buffers,
                digest,
            }),
        )
    }

    fn new_internal(
        initial: T,
        retention: HistoryRetentionPolicy,
        accounting: Option<HistoryAccounting<T>>,
    ) -> Self {
        let root_id = next_node_id();
        let root_change = HistoryChangeNumber::INITIAL;
        let retained_buffers = collect_retained_buffers(accounting.as_ref(), &initial);
        let retained_source_bytes = retained_buffers.iter().map(|buffer| buffer.bytes).sum();
        let save_point_digest = accounting.as_ref().map(|value| (value.digest)(&initial));
        let save_point_source = accounting.as_ref().map(|value| (value.source_identity)(&initial));
        let node_indexes = HashMap::from([(root_id, 0)]);
        let change_indexes = HashMap::from([(root_change, 0)]);
        let singleton_index_bytes = (node_indexes.capacity() + change_indexes.capacity()) * 48;
        Self {
            nodes: vec![Node {
                id: root_id,
                change: root_change,
                timestamp: timeline::now_seconds(),
                creation_write: 0,
                saved_write: None,
                state: Arc::new(initial),
                parent: None,
                children: Vec::new(),
                preferred_child: None,
                incoming: None,
                retained_buffers,
                memory_roots: Vec::new(),
                memory_metadata_bytes: 0,
            }],
            node_indexes,
            change_indexes,
            current: 0,
            group_node: None,
            next_change_number: 1,
            next_write_number: 1,
            save_point: root_id,
            save_point_digest,
            save_point_source,
            retention,
            retained_source_bytes,
            accounting,
            memory: RetainedMemory::default(),
            live_memory: RetainedMemory::default(),
            live_memory_roots: Vec::new(),
            singleton_index_bytes,
            memory_metadata_bytes: 0,
            visit_state_memory: None,
            visit_map_memory: None,
            command_checkpoints: Vec::new(),
        }
    }

    pub(crate) fn with_retained_memory(
        mut self,
        state: fn(&T, &mut MemoryVisitor<'_>),
        map: fn(&M, &mut MemoryVisitor<'_>),
    ) -> Self {
        self.visit_state_memory = Some(state);
        self.visit_map_memory = Some(map);
        self.refresh_node_memory(self.current);
        self
    }

    fn refresh_node_memory(&mut self, index: usize) {
        if !self.command_checkpoints.is_empty() {
            return;
        }
        let Some(visit_state) = self.visit_state_memory else { return; };
        let node = &self.nodes[index];
        let roots = self.memory.capture(|visitor| {
            // The state's lazy caches are separate roots. Revisiting these
            // top-level handles discovers newly materialized compatibility
            // views without traversing unchanged immutable tree nodes.
            visitor.arc(&node.state, |_| {});
            visit_state(&node.state, visitor);
            if let (Some(edge), Some(visit_map)) = (&node.incoming, self.visit_map_memory) {
                visit_map(&edge.map, visitor);
            }
        });
        let node = &mut self.nodes[index];
        let previous = std::mem::replace(&mut node.memory_roots, roots);
        self.memory.release(previous);
        self.memory_metadata_bytes = self.memory_metadata_bytes.saturating_sub(node.memory_metadata_bytes);
        node.memory_metadata_bytes = node.children.capacity() * std::mem::size_of::<usize>()
            + node.retained_buffers.capacity() * std::mem::size_of::<RetainedBuffer>()
            + node.memory_roots.capacity() * std::mem::size_of::<AllocationId>()
            + node.incoming.as_ref().map_or(0, |edge| edge.record.owned_heap_bytes());
        self.memory_metadata_bytes = self.memory_metadata_bytes.saturating_add(node.memory_metadata_bytes);
        if index == self.current {
            self.refresh_live_memory();
        }
    }

    fn refresh_live_memory(&mut self) {
        if !self.command_checkpoints.is_empty() { return; }
        let Some(visit_state) = self.visit_state_memory else { return; };
        let state = &self.nodes[self.current].state;
        let roots = self.live_memory.capture(|visitor| {
            visitor.arc(state, |_| {});
            visit_state(state, visitor);
        });
        let previous = std::mem::replace(&mut self.live_memory_roots, roots);
        self.live_memory.release(previous);
    }

    fn live_state_memory_bytes(&self) -> usize {
        if self.visit_state_memory.is_none() {
            return self.nodes[self.current].retained_buffers.iter().map(|buffer| buffer.bytes).sum();
        }
        // A current-only document still needs both allocation ledgers, their
        // root vectors, one history node and its source-buffer index. Charging
        // that baseline to undo would consume the allowance in proportion to
        // file size before there was any old state to retain.
        let state_roots = self.live_memory_roots.capacity() * std::mem::size_of::<AllocationId>();
        self.live_memory.bytes()
            .saturating_add(self.live_memory.bookkeeping_bytes())
            .saturating_add(state_roots.saturating_mul(2))
            .saturating_add(std::mem::size_of::<Node<T, M>>())
            .saturating_add(self.nodes[self.current].retained_buffers.capacity() * std::mem::size_of::<RetainedBuffer>())
            .saturating_add(self.singleton_index_bytes)
    }

    fn release_node_memory(&mut self, index: usize) {
        self.memory.release(std::mem::take(&mut self.nodes[index].memory_roots));
        self.memory_metadata_bytes = self.memory_metadata_bytes.saturating_sub(self.nodes[index].memory_metadata_bytes);
        self.nodes[index].memory_metadata_bytes = 0;
    }

    fn retained_memory_bytes(&self) -> usize {
        if self.visit_state_memory.is_none() { return self.retained_source_bytes; }
        if std::env::var_os("VIEM_HISTORY_MEMORY_BREAKDOWN").is_some() {
            eprintln!("  ledger allocations {}", self.memory.allocation_count());
        }
        self.memory.bytes()
            .saturating_add(self.live_memory.bookkeeping_bytes())
            .saturating_add(self.live_memory_roots.capacity() * std::mem::size_of::<AllocationId>())
            .saturating_add(self.nodes.capacity() * std::mem::size_of::<Node<T, M>>())
            .saturating_add(self.node_indexes.capacity() * 48)
            .saturating_add(self.change_indexes.capacity() * 48)
            .saturating_add(self.memory_metadata_bytes)
    }

    #[cfg(test)]
    pub(super) fn assert_memory_matches_full_recount(&mut self) {
        self.refresh_node_memory(self.current);
        self.assert_memory_matches_full_recount_without_refresh();
    }

    #[cfg(test)]
    pub(super) fn assert_memory_matches_full_recount_without_refresh(&self) {
        let mut fresh = RetainedMemory::default();
        for node in &self.nodes {
            fresh.capture(|visitor| {
                visitor.arc(&node.state, |_| {});
                self.visit_state_memory.unwrap()(&node.state, visitor);
                if let Some(edge) = &node.incoming {
                    self.visit_map_memory.unwrap()(&edge.map, visitor);
                }
            });
        }
        self.memory.assert_same_allocations(&fresh);
        let mut live = RetainedMemory::default();
        let state = &self.nodes[self.current].state;
        live.capture(|visitor| {
            visitor.arc(state, |_| {});
            self.visit_state_memory.unwrap()(state, visitor);
        });
        self.live_memory.assert_same_allocations(&live);
    }

    #[cfg(test)]
    pub(super) fn assert_current_only_memory_baseline(&self) {
        assert_eq!(self.nodes.len(), 1);
        let node = &self.nodes[0];
        assert!(node.incoming.is_none());
        self.memory.assert_same_allocations(&self.live_memory);
        // Independent hash tables can have different usable capacities after
        // replacement, even with identical allocation graphs. The live-state
        // allowance estimates a second table with the live ledger's capacity;
        // this is the only variable ledger difference in a current-only tree.
        let capacity_difference = self.memory.bookkeeping_bytes() as i128
            - self.live_memory.bookkeeping_bytes() as i128;
        let metadata_difference = (self.nodes.capacity() - 1) * std::mem::size_of::<Node<T, M>>()
            + (self.node_indexes.capacity() + self.change_indexes.capacity()) * 48
            - self.singleton_index_bytes
            + node.children.capacity() * std::mem::size_of::<usize>();
        let root_difference = (node.memory_roots.capacity() as i128
            - self.live_memory_roots.capacity() as i128) * std::mem::size_of::<AllocationId>() as i128;
        assert_eq!(
            self.retained_memory_bytes() as i128 - self.live_state_memory_bytes() as i128,
            capacity_difference + metadata_difference as i128 + root_difference,
        );
    }

    /// Install initial projection-only configuration without changing the
    /// root/savepoint identities or creating a history unit.
    pub(crate) fn initialize_projection(&mut self, state: T) {
        assert!(self.nodes.len() == 1 && self.current == 0 && self.next_change_number == 1);
        if let Some(accounting) = &self.accounting {
            assert_eq!(
                (accounting.digest)(&self.nodes[0].state),
                (accounting.digest)(&state)
            );
        }
        let previous = std::mem::replace(&mut self.nodes[0].state, Arc::new(state));
        self.refresh_node_memory(0);
        drop(previous);
    }

    pub(crate) fn current(&self) -> &Arc<T> {
        &self.nodes[self.current].state
    }

    pub(crate) fn has_command_checkpoint(&self) -> bool {
        !self.command_checkpoints.is_empty()
    }

    pub(crate) fn command_checkpoint_depth(&self) -> usize {
        self.command_checkpoints.len()
    }

    pub(crate) fn begin_command_checkpoint(&mut self) {
        self.command_checkpoints.push(HistoryCommandCheckpoint {
            node_count: self.nodes.len(),
            nodes_capacity: self.nodes.capacity(),
            node_indexes_capacity: self.node_indexes.capacity(),
            change_indexes_capacity: self.change_indexes.capacity(),
            current: self.current,
            group_node: self.group_node,
            next_change_number: self.next_change_number,
            next_write_number: self.next_write_number,
            save_point: self.save_point,
            save_point_digest: self.save_point_digest,
            save_point_source: self.save_point_source,
            retention: self.retention,
            retained_source_bytes: self.retained_source_bytes,
            retention_requested: false,
            nodes: HashMap::new(),
        });
    }

    fn checkpoint_node(&mut self, index: usize) {
        for checkpoint in &mut self.command_checkpoints {
            if index >= checkpoint.node_count {
                continue;
            }
            checkpoint.nodes.entry(index).or_insert_with(|| {
                let node = &self.nodes[index];
                NodeCommandCheckpoint {
                    children_len: node.children.len(),
                    children_capacity: node.children.capacity(),
                    preferred_child: node.preferred_child,
                    saved_write: node.saved_write,
                    content: None,
                }
            });
        }
    }

    fn checkpoint_node_content(&mut self, index: usize)
    where
        M: Clone,
    {
        self.checkpoint_node(index);
        for checkpoint in &mut self.command_checkpoints {
            let Some(saved) = checkpoint.nodes.get_mut(&index) else {
                continue;
            };
            if saved.content.is_some() {
                continue;
            }
            let node = &mut self.nodes[index];
            saved.content = Some(NodeContentCheckpoint {
                state: node.state.clone(),
                retained_buffers_capacity: node.retained_buffers.capacity(),
                edge: node.incoming.as_mut().map(|edge| {
                    // Keep the original map allocation alive until its ledger
                    // roots are released. A clone alone would not preserve those
                    // allocation identities when a grouped edit replaces it.
                    let working_map = edge.map.clone();
                    EdgeCommandCheckpoint {
                        map: std::mem::replace(&mut edge.map, working_map),
                        semantic_len: edge.record.semantic.changes.len(),
                        semantic_capacity: edge.record.semantic.changes.capacity(),
                        transactions_len: edge.record.transactions.len(),
                        transactions_capacity: edge.record.transactions.capacity(),
                        restoration: edge.record.restoration.clone(),
                        command_restoration_attached: edge.record.command_restoration_attached,
                    }
                }),
            });
        }
    }

    pub(crate) fn commit_command_checkpoint(&mut self) {
        let checkpoint = self
            .command_checkpoints
            .pop()
            .expect("command checkpoint active");
        if !self.command_checkpoints.is_empty() {
            // Every outer frame has already journaled these mutations. Its
            // original allocations remain alive until outer publication.
            return;
        }
        for index in checkpoint.nodes.keys().copied() {
            self.refresh_node_memory(index);
        }
        for index in checkpoint.node_count..self.nodes.len() {
            self.refresh_node_memory(index);
        }
        if checkpoint.retention_requested {
            self.enforce_retention();
        }
        // Old snapshots/maps must outlive replacement of their ledger roots.
        drop(checkpoint);
    }

    pub(crate) fn rollback_command_checkpoint(&mut self) {
        let checkpoint = self
            .command_checkpoints
            .pop()
            .expect("command checkpoint active");
        for node in &self.nodes[checkpoint.node_count..] {
            self.node_indexes.remove(&node.id);
            self.change_indexes.remove(&node.change);
        }
        self.nodes.truncate(checkpoint.node_count);
        for (index, saved) in checkpoint.nodes {
            let node = &mut self.nodes[index];
            node.children.truncate(saved.children_len);
            node.children.shrink_to(saved.children_capacity);
            node.preferred_child = saved.preferred_child;
            node.saved_write = saved.saved_write;
            if let Some(content) = saved.content {
                node.state = content.state;
                node.retained_buffers =
                    collect_retained_buffers(self.accounting.as_ref(), &node.state);
                node.retained_buffers
                    .shrink_to(content.retained_buffers_capacity);
                if node.retained_buffers.capacity() < content.retained_buffers_capacity {
                    node.retained_buffers.reserve_exact(
                        content.retained_buffers_capacity - node.retained_buffers.len(),
                    );
                }
                if let Some(edge) = content.edge {
                    let current = node
                        .incoming
                        .as_mut()
                        .expect("existing history edge retained");
                    current.map = edge.map;
                    current.record.semantic.changes.truncate(edge.semantic_len);
                    current
                        .record
                        .semantic
                        .changes
                        .shrink_to(edge.semantic_capacity);
                    current.record.transactions.truncate(edge.transactions_len);
                    current
                        .record
                        .transactions
                        .shrink_to(edge.transactions_capacity);
                    current.record.restoration = edge.restoration;
                    current.record.command_restoration_attached = edge.command_restoration_attached;
                }
            }
        }
        self.nodes.shrink_to(checkpoint.nodes_capacity);
        self.node_indexes
            .shrink_to(checkpoint.node_indexes_capacity);
        self.change_indexes
            .shrink_to(checkpoint.change_indexes_capacity);
        self.current = checkpoint.current;
        self.group_node = checkpoint.group_node;
        self.next_change_number = checkpoint.next_change_number;
        self.next_write_number = checkpoint.next_write_number;
        self.save_point = checkpoint.save_point;
        self.save_point_digest = checkpoint.save_point_digest;
        self.save_point_source = checkpoint.save_point_source;
        self.retention = checkpoint.retention;
        self.retained_source_bytes = checkpoint.retained_source_bytes;
        // Both ledgers are frozen while a command checkpoint is active. Their
        // roots already describe the restored state; refreshing here would
        // publish lazy caches created by the failed command into its status.
    }

    pub(crate) fn begin_group(&mut self) {
        // The first commit creates the group node. Nesting is handled by the
        // Document facade, so this is intentionally idempotent.
        if self.group_node == Some(self.current) {
            return;
        }
        self.group_node = None;
    }

    pub(crate) fn end_group(&mut self) {
        self.group_node = None;
        self.enforce_retention();
    }

    #[cfg(test)]
    pub(crate) fn undo_exact(&mut self) -> Result<HistoryNavigation, HistoryError> {
        let Some(parent) = self.nodes[self.current].parent else {
            return Err(HistoryError::Boundary(HistoryBoundary::Oldest));
        };
        self.group_node = None;
        let navigation = self.move_to_index(parent);
        self.enforce_retention();
        Ok(navigation)
    }

    #[cfg(test)]
    pub(crate) fn redo_exact(&mut self) -> Result<HistoryNavigation, HistoryError> {
        let Some(child) = self.nodes[self.current].preferred_child else {
            return Err(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo));
        };
        self.group_node = None;
        let navigation = self.move_to_index(child);
        self.enforce_retention();
        Ok(navigation)
    }

    #[cfg(test)]
    pub(crate) fn undo(&mut self) -> bool {
        self.undo_exact().is_ok()
    }

    #[cfg(test)]
    pub(crate) fn redo(&mut self) -> bool {
        self.redo_exact().is_ok()
    }

    pub(crate) fn status(&self) -> HistoryStatus {
        let current = &self.nodes[self.current];
        HistoryStatus {
            current: self.location(self.current),
            save_point: self.save_point,
            parent: current.parent.map(|parent| self.location(parent)),
            preferred_redo: current.preferred_child.map(|child| self.location(child)),
            node_count: self.nodes.len(),
            redo_branch_count: current.children.len(),
            can_undo: current.parent.is_some(),
            can_redo: current.preferred_child.is_some(),
            is_dirty: match (&self.accounting, self.save_point_source) {
                (Some(accounting), Some(saved)) => (accounting.source_identity)(&current.state) != saved,
                _ => current.id != self.save_point,
            },
            undo_summary: current
                .incoming
                .as_ref()
                .map(|edge| edge.record.semantic.clone()),
            redo_summary: current.preferred_child.and_then(|child| {
                self.nodes[child]
                    .incoming
                    .as_ref()
                    .map(|edge| edge.record.semantic.clone())
            }),
            retention: self.retention,
            retained_source_bytes: self.retained_source_bytes,
            retained_memory_bytes: self.retained_memory_bytes(),
            live_state_memory_bytes: self.live_state_memory_bytes(),
            additional_history_memory_bytes: self.retained_memory_bytes().saturating_sub(self.live_state_memory_bytes()),
            save_point_retained: self.node_indexes.contains_key(&self.save_point),
            save_point_digest: self.save_point_digest,
        }
    }

    /// Resolve an ancestor without mutating the history cursor or preferred
    /// redo path. This is the validation half of an atomic counted undo.
    pub(crate) fn undo_target(&self, steps: usize) -> Result<HistoryLocation, HistoryError> {
        let mut target = self.current;
        for _ in 0..steps {
            target = self.nodes[target]
                .parent
                .ok_or(HistoryError::Boundary(HistoryBoundary::Oldest))?;
        }
        Ok(self.location(target))
    }

    /// Resolve a chain of preferred children without changing any branch
    /// preference. This is the validation half of an atomic counted redo.
    pub(crate) fn redo_target(&self, steps: usize) -> Result<HistoryLocation, HistoryError> {
        let mut target = self.current;
        for _ in 0..steps {
            target = self.nodes[target]
                .preferred_child
                .ok_or(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo))?;
        }
        Ok(self.location(target))
    }

    pub(crate) fn redo_branches(&self) -> Vec<HistoryBranch> {
        let node = &self.nodes[self.current];
        node.children
            .iter()
            .enumerate()
            .map(|(ordinal, child)| HistoryBranch {
                ordinal,
                destination: self.location(*child),
                preferred: node.preferred_child == Some(*child),
            })
            .collect()
    }

    pub(crate) fn redo_branch_count(&self) -> usize {
        self.nodes[self.current].children.len()
    }

    /// Resolve a retained node without changing the current history path.
    /// Used by document transaction preparation so projection verification and
    /// change summaries can be completed before publication.
    pub(crate) fn state_at_node(&self, node: HistoryNodeId) -> Option<&Arc<T>> {
        self.node_indexes
            .get(&node)
            .map(|index| &self.nodes[*index].state)
    }

    pub(crate) fn location_for_node(&self, node: HistoryNodeId) -> Option<HistoryLocation> {
        self.node_indexes
            .get(&node)
            .map(|index| self.location(*index))
    }

    pub(crate) fn location_for_change(
        &self,
        change: HistoryChangeNumber,
    ) -> Option<HistoryLocation> {
        self.change_indexes
            .get(&change)
            .map(|index| self.location(*index))
    }

    pub(crate) fn node_details(&self, node: HistoryNodeId) -> Option<HistoryNodeDetails> {
        let index = *self.node_indexes.get(&node)?;
        let node = &self.nodes[index];
        let incoming = node.incoming.as_ref();
        Some(HistoryNodeDetails {
            location: self.location(index),
            parent: node.parent.map(|parent| self.location(parent)),
            semantic: incoming.map(|edge| edge.record.semantic.clone()),
            transactions: incoming
                .map(|edge| edge.record.transactions.clone())
                .unwrap_or_default(),
            restoration: incoming.and_then(|edge| edge.record.restoration.clone()),
        })
    }

    /// Return the restoration state associated with the last edge traversed
    /// while moving between two retained nodes.
    pub(crate) fn restoration_between(
        &self,
        from: HistoryNodeId,
        to: HistoryNodeId,
    ) -> Result<Option<HistoryRestorationSnapshot>, HistoryError> {
        let from = *self
            .node_indexes
            .get(&from)
            .ok_or(HistoryError::NodeNotFound(from))?;
        let to = *self
            .node_indexes
            .get(&to)
            .ok_or(HistoryError::NodeNotFound(to))?;
        if from == to {
            return Ok(None);
        }

        let from_path = self.root_path(from);
        let to_path = self.root_path(to);
        let common = from_path
            .iter()
            .zip(&to_path)
            .take_while(|(left, right)| left == right)
            .count();

        let snapshot = if common < to_path.len() {
            let last_descendant = *to_path
                .last()
                .expect("a root path always contains its destination");
            self.nodes[last_descendant]
                .incoming
                .as_ref()
                .and_then(|edge| edge.record.restoration.as_ref())
                .map(|restoration| restoration.after.clone())
        } else {
            // The final upward edge enters `to`. The child immediately below
            // the common ancestor owns the matching before snapshot.
            self.nodes[from_path[common]]
                .incoming
                .as_ref()
                .and_then(|edge| edge.record.restoration.as_ref())
                .map(|restoration| restoration.before.clone())
        };
        Ok(snapshot)
    }

    pub(crate) fn attach_current_command_restoration(
        &mut self,
        expected: HistoryNodeId,
        restoration: HistoryRestoration,
    ) -> Result<(), HistoryError>
    where
        M: Clone,
    {
        self.checkpoint_node_content(self.current);
        let current = &mut self.nodes[self.current];
        if current.id != expected {
            return Err(HistoryError::NodeNotFound(expected));
        }
        let Some(edge) = current.incoming.as_mut() else {
            // Retention can promote a committed state to the root before the
            // coordinator publishes its final command restoration (including
            // when Insert closes its undo group). That state has no retained
            // undo edge, so there is no restoration record left to update.
            return Ok(());
        };
        edge.record.attach_command_restoration(restoration);
        self.enforce_retention();
        Ok(())
    }

    pub(crate) fn prefer_redo_branch(
        &mut self,
        branch: usize,
    ) -> Result<HistoryBranch, HistoryError> {
        let available = self.nodes[self.current].children.len();
        let Some(&child) = self.nodes[self.current].children.get(branch) else {
            return Err(HistoryError::RedoBranchNotFound {
                requested: branch,
                available,
            });
        };
        self.checkpoint_node(self.current);
        self.nodes[self.current].preferred_child = Some(child);
        Ok(HistoryBranch {
            ordinal: branch,
            destination: self.location(child),
            preferred: true,
        })
    }

    pub(crate) fn select_redo_branch(&mut self, branch: usize) -> bool {
        self.prefer_redo_branch(branch).is_ok()
    }

    pub(crate) fn select_node(
        &mut self,
        node: HistoryNodeId,
    ) -> Result<HistoryNavigation, HistoryError> {
        let Some(&target) = self.node_indexes.get(&node) else {
            return Err(HistoryError::NodeNotFound(node));
        };
        self.group_node = None;
        let navigation = self.move_to_index(target);
        self.enforce_retention();
        Ok(navigation)
    }

    pub(crate) fn select_change(
        &mut self,
        change: HistoryChangeNumber,
    ) -> Result<HistoryNavigation, HistoryError> {
        let Some(&target) = self.change_indexes.get(&change) else {
            return Err(HistoryError::ChangeNotFound(change));
        };
        self.group_node = None;
        let navigation = self.move_to_index(target);
        self.enforce_retention();
        Ok(navigation)
    }

    pub(crate) fn mark_saved(&mut self) -> HistoryNodeId {
        // Saving establishes an exact immutable history state. If a group was
        // open, a later grouped commit must create a new node rather than
        // overwrite the state represented by this save point.
        self.end_group();
        self.annotate_saved(self.current);
        self.save_point = self.nodes[self.current].id;
        self.save_point_digest = self
            .accounting
            .as_ref()
            .map(|accounting| (accounting.digest)(&self.nodes[self.current].state));
        self.save_point_source = self.accounting.as_ref()
            .map(|accounting| (accounting.source_identity)(&self.nodes[self.current].state));
        self.save_point
    }

    /// Establish a retained immutable node as the persisted save point without
    /// navigating history. This is used when an asynchronous write completes
    /// after the user has already committed newer edits.
    pub(crate) fn mark_location_saved(
        &mut self,
        location: HistoryLocation,
        digest: SourceArtifactDigest,
        source_identity: SourceSnapshotIdentity,
    ) -> HistoryLocation {
        // A prepared write is an immutable, trusted capture. Its node may have
        // been pruned while host I/O was running; saving still establishes that
        // exact snapshot identity without resurrecting a navigable node.
        if let Some(&index) = self.node_indexes.get(&location.node) {
            debug_assert_eq!(self.location(index), location);
            self.annotate_saved(index);
        } else {
            self.next_write_number = self.next_write_number.saturating_add(1);
        }
        self.save_point = location.node;
        self.save_point_digest = Some(digest);
        self.save_point_source = Some(source_identity);
        location
    }

    pub(crate) fn retention_policy(&self) -> HistoryRetentionPolicy {
        self.retention
    }

    pub(crate) fn set_retention_policy(&mut self, retention: HistoryRetentionPolicy) {
        self.retention = retention;
        self.enforce_retention();
    }

    fn location(&self, index: usize) -> HistoryLocation {
        let node = &self.nodes[index];
        HistoryLocation {
            node: node.id,
            change: node.change,
        }
    }

    fn move_to_index(&mut self, target: usize) -> HistoryNavigation {
        self.refresh_node_memory(self.current);
        let from = self.location(self.current);
        if target == self.current {
            return HistoryNavigation { from, to: from };
        }

        // Record both sides of a direct branch traversal. For an ordinary
        // undo this makes redo return to the child just left. For a jump to a
        // different branch, the target path overwrites the preference at the
        // fork so subsequent undo/redo follows the explicitly selected path.
        let current_path = self.root_path(self.current);
        let target_path = self.root_path(target);
        let common = current_path
            .iter()
            .zip(&target_path)
            .take_while(|(left, right)| left == right)
            .count();
        self.prefer_path(&current_path[common.saturating_sub(1)..]);
        self.prefer_path(&target_path[common.saturating_sub(1)..]);

        self.current = target;
        HistoryNavigation {
            from,
            to: self.location(target),
        }
    }

    fn root_path(&self, mut node: usize) -> Vec<usize> {
        let mut path = vec![node];
        while let Some(parent) = self.nodes[node].parent {
            path.push(parent);
            node = parent;
        }
        path.reverse();
        path
    }

    fn prefer_path(&mut self, path: &[usize]) {
        for edge in path.windows(2) {
            self.checkpoint_node(edge[0]);
            self.nodes[edge[0]].preferred_child = Some(edge[1]);
        }
    }

    fn exceeds_retention(&self, node_count: usize, retained_bytes: usize) -> bool {
        let retained_bytes = if self.retention.additional_to_live_state {
            retained_bytes.saturating_sub(self.live_state_memory_bytes())
        } else { retained_bytes };
        node_count > self.retention.node_budget
            || retained_bytes > self.retention.retained_byte_budget
    }

    fn enforce_retention(&mut self) {
        if !self.command_checkpoints.is_empty() {
            for checkpoint in &mut self.command_checkpoints {
                checkpoint.retention_requested = true;
            }
            return;
        }
        self.refresh_node_memory(self.current);
        if self.nodes.len() <= 1
            && !self.exceeds_retention(self.nodes.len(), self.retained_memory_bytes())
        {
            return;
        }

        let mut retained = vec![true; self.nodes.len()];
        let mut retained_count = retained.len();
        let (mut buffer_references, mut retained_bytes) = self.buffer_reference_counts(&retained);
        let mut protected = vec![false; self.nodes.len()];
        protected[self.current] = true;
        let promotion_floor = self.group_node.and_then(|group| {
            protected[group] = true;
            let parent = self.nodes[group].parent;
            if let Some(parent) = parent {
                protected[parent] = true;
            }
            parent
        });

        // Repeatedly removing the oldest eligible leaf deterministically
        // discards abandoned branches without touching the active ancestry.
        while self.exceeds_retention(
            retained_count,
            if self.visit_state_memory.is_some() {
                self.retained_memory_bytes()
            } else {
                retained_bytes
            },
        ) {
            let candidate = (0..self.nodes.len())
                .filter(|index| retained[*index] && !protected[*index])
                .filter(|index| {
                    self.nodes[*index]
                        .children
                        .iter()
                        .all(|child| !retained[*child])
                })
                .min_by_key(|index| (self.nodes[*index].change, self.nodes[*index].id));
            let Some(candidate) = candidate else {
                break;
            };
            retained[candidate] = false;
            retained_count -= 1;
            self.release_node_memory(candidate);
            remove_buffer_references(
                &self.nodes[candidate].retained_buffers,
                &mut buffer_references,
                &mut retained_bytes,
            );
        }

        // Once no abandoned leaf remains, only current ancestry (and any
        // explicitly protected states) can keep the tree over budget. Promote
        // one oldest ancestor at a time. An open unit stops promotion at its
        // fixed parent so both its before-state and result remain available.
        while self.exceeds_retention(
            retained_count,
            if self.visit_state_memory.is_some() {
                self.retained_memory_bytes()
            } else {
                retained_bytes
            },
        ) {
            let active_path: Vec<_> = self
                .root_path(self.current)
                .into_iter()
                .filter(|index| retained[*index])
                .collect();
            let Some(&oldest) = active_path.first() else {
                unreachable!("the current history node is always retained")
            };
            if oldest == self.current || Some(oldest) == promotion_floor {
                break;
            }

            // The leaf pass removed every non-protected side branch. Refuse to
            // create a forest if a future protected-node kind is added without
            // also defining its promotion policy.
            let next = active_path[1];
            debug_assert!(self.nodes[oldest]
                .children
                .iter()
                .filter(|child| retained[**child])
                .all(|child| *child == next));
            retained[oldest] = false;
            retained_count -= 1;
            self.release_node_memory(oldest);
            remove_buffer_references(
                &self.nodes[oldest].retained_buffers,
                &mut buffer_references,
                &mut retained_bytes,
            );
        }

        if retained_count != self.nodes.len() {
            self.compact_to_retained(&retained);
        } else {
            self.retained_source_bytes = retained_bytes;
        }
    }

    fn buffer_reference_counts(
        &self,
        retained: &[bool],
    ) -> (HashMap<usize, (usize, usize)>, usize) {
        let mut references = HashMap::<usize, (usize, usize)>::new();
        let mut total = 0_usize;
        for (index, node) in self.nodes.iter().enumerate() {
            if !retained[index] {
                continue;
            }
            for buffer in &node.retained_buffers {
                match references.entry(buffer.identity) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((buffer.bytes, 1));
                        total = total.saturating_add(buffer.bytes);
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        debug_assert_eq!(entry.get().0, buffer.bytes);
                        entry.get_mut().1 += 1;
                    }
                }
            }
        }
        (references, total)
    }

    fn compact_to_retained(&mut self, retained: &[bool]) {
        let old_current = self.current;
        let old_group = self.group_node;
        let old_nodes = std::mem::take(&mut self.nodes);
        let mut old_to_new = vec![None; old_nodes.len()];
        let mut compacted = Vec::with_capacity(retained.iter().filter(|keep| **keep).count());
        for (old_index, node) in old_nodes.into_iter().enumerate() {
            if retained[old_index] {
                old_to_new[old_index] = Some(compacted.len());
                compacted.push(node);
            }
        }

        let mut removed_edges = Vec::new();
        for node in &mut compacted {
            node.parent = node.parent.and_then(|parent| old_to_new[parent]);
            node.children = node
                .children
                .iter()
                .filter_map(|child| old_to_new[*child])
                .collect();
            node.preferred_child = node
                .preferred_child
                .and_then(|child| old_to_new[child])
                .or_else(|| node.children.last().copied());
            if node.parent.is_none() {
                removed_edges.push(node.incoming.take());
            }
        }

        self.current = old_to_new[old_current].expect("the active history node is retained");
        self.group_node = old_group
            .map(|group| old_to_new[group].expect("an open undo unit's result is retained"));
        self.nodes = compacted;
        self.node_indexes.clear();
        self.change_indexes.clear();
        for (index, node) in self.nodes.iter().enumerate() {
            self.node_indexes.insert(node.id, index);
            self.change_indexes.insert(node.change, index);
        }
        let all_retained = vec![true; self.nodes.len()];
        self.retained_source_bytes = self.buffer_reference_counts(&all_retained).1;
        // Keep removed edges alive until their old allocation identities have
        // been released, preventing allocator address reuse during accounting.
        for index in 0..self.nodes.len() { self.refresh_node_memory(index); }
        drop(removed_edges);
    }

    #[cfg(test)]
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

fn collect_retained_buffers<T>(
    accounting: Option<&HistoryAccounting<T>>,
    state: &T,
) -> Vec<RetainedBuffer> {
    let Some(accounting) = accounting else {
        return Vec::new();
    };
    let mut unique = HashMap::<usize, usize>::new();
    (accounting.visit_buffers)(state, &mut |identity, bytes| match unique.entry(identity) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(bytes);
        }
        std::collections::hash_map::Entry::Occupied(entry) => {
            debug_assert_eq!(*entry.get(), bytes);
        }
    });
    let mut buffers: Vec<_> = unique
        .into_iter()
        .map(|(identity, bytes)| RetainedBuffer { identity, bytes })
        .collect();
    buffers.sort_unstable_by_key(|buffer| buffer.identity);
    buffers
}

fn remove_buffer_references(
    buffers: &[RetainedBuffer],
    references: &mut HashMap<usize, (usize, usize)>,
    retained_bytes: &mut usize,
) {
    for buffer in buffers {
        let remove = {
            let entry = references
                .get_mut(&buffer.identity)
                .expect("every retained node buffer has a reference count");
            debug_assert_eq!(entry.0, buffer.bytes);
            entry.1 -= 1;
            entry.1 == 0
        };
        if remove {
            references.remove(&buffer.identity);
            *retained_bytes = retained_bytes.saturating_sub(buffer.bytes);
        }
    }
}

#[cfg(test)]
impl<T> History<T> {
    /// Commit a state for history users which do not retain edge metadata.
    /// Document history uses [`History::commit_with_maps`] instead.
    pub(crate) fn commit(&mut self, state: T, grouping: bool) {
        self.refresh_node_memory(self.current);
        let retained_buffers = collect_retained_buffers(self.accounting.as_ref(), &state);
        if grouping {
            if let Some(group) = self.group_node {
                debug_assert_eq!(group, self.current);
                self.checkpoint_node_content(group);
                let previous = std::mem::replace(&mut self.nodes[group].state, Arc::new(state));
                self.nodes[group].retained_buffers = retained_buffers;
                self.enforce_retention();
                drop(previous);
                return;
            }
        }
        self.push_node(state, retained_buffers, grouping, None);
    }

    fn push_node(
        &mut self,
        state: T,
        retained_buffers: Vec<RetainedBuffer>,
        grouping: bool,
        incoming: Option<HistoryEdge<()>>,
    ) {
        let parent = self.current;
        self.checkpoint_node(parent);
        let index = self.nodes.len();
        let id = next_node_id();
        let change = HistoryChangeNumber(self.next_change_number);
        self.next_change_number = self
            .next_change_number
            .checked_add(1)
            .expect("history change numbers exhausted");
        self.nodes.push(Node {
            id,
            change,
            timestamp: self.next_timestamp(),
            creation_write: self.next_write_number - 1,
            saved_write: None,
            state: Arc::new(state),
            parent: Some(parent),
            children: Vec::new(),
            preferred_child: None,
            incoming,
            retained_buffers,
            memory_roots: Vec::new(),
            memory_metadata_bytes: 0,
        });
        self.node_indexes.insert(id, index);
        self.change_indexes.insert(change, index);
        self.nodes[parent].children.push(index);
        self.refresh_node_memory(parent);
        self.nodes[parent].preferred_child = Some(index);
        self.current = index;
        self.group_node = grouping.then_some(index);
        self.enforce_retention();
    }
}

impl<T> History<T, PositionMap> {
    /// Commit a document state together with both exact directions of the
    /// formatted transition from the current node.
    ///
    /// Updating an open group replaces the group's state but composes its
    /// original parent edge through the new transition. All fallible map work
    /// completes before either the state or graph is mutated.
    pub(crate) fn commit_with_maps(
        &mut self,
        state: T,
        grouping: bool,
        forward: PositionMap,
        transaction: HistoryTransactionSummary,
        restoration: HistoryRestoration,
    ) -> Result<(), PositionError> {
        self.refresh_node_memory(self.current);
        // Check that the two directions meet at the same revision and length.
        // They need not be mathematical inverses for inserted/deleted content,
        // whose identity is intentionally unrecoverable.
        let reverse = forward.inverted()?;
        let _ = forward.then(&reverse)?;
        let _ = reverse.then(&forward)?;
        let retained_buffers = collect_retained_buffers(self.accounting.as_ref(), &state);

        if grouping {
            if let Some(group) = self.group_node {
                debug_assert_eq!(group, self.current);
                let edge = self.nodes[group]
                    .incoming
                    .as_ref()
                    .expect("a non-root group node has an incoming edge");
                let composed_forward = edge.map.then(&forward)?;
                self.checkpoint_node_content(group);
                let previous_state = std::mem::replace(&mut self.nodes[group].state, Arc::new(state));
                self.nodes[group].retained_buffers = retained_buffers;
                let edge = self.nodes[group]
                    .incoming
                    .as_mut()
                    .expect("a non-root group node has an incoming edge");
                let previous_map = std::mem::replace(&mut edge.map, composed_forward);
                edge.record.append(transaction, Some(restoration));
                self.enforce_retention();
                drop((previous_state, previous_map));
                return Ok(());
            }
        }

        let parent = self.current;
        self.checkpoint_node(parent);
        let index = self.nodes.len();
        let id = next_node_id();
        let change = HistoryChangeNumber(self.next_change_number);
        self.next_change_number = self
            .next_change_number
            .checked_add(1)
            .expect("history change numbers exhausted");
        self.nodes.push(Node {
            id,
            change,
            timestamp: self.next_timestamp(),
            creation_write: self.next_write_number - 1,
            saved_write: None,
            state: Arc::new(state),
            parent: Some(parent),
            children: Vec::new(),
            preferred_child: None,
            incoming: Some(HistoryEdge {
                map: forward,
                record: HistoryUnitRecord::new(transaction, Some(restoration)),
            }),
            retained_buffers,
            memory_roots: Vec::new(),
            memory_metadata_bytes: 0,
        });
        self.node_indexes.insert(id, index);
        self.change_indexes.insert(change, index);
        self.nodes[parent].children.push(index);
        self.refresh_node_memory(parent);
        self.nodes[parent].preferred_child = Some(index);
        self.current = index;
        self.group_node = grouping.then_some(index);
        self.enforce_retention();
        Ok(())
    }

    /// Compose the exact edge maps between any two retained nodes.
    pub(crate) fn map_between(
        &self,
        from: HistoryNodeId,
        to: HistoryNodeId,
        identity: PositionMap,
    ) -> Result<PositionMap, PositionError> {
        let from = *self
            .node_indexes
            .get(&from)
            .expect("history map source node is retained");
        let to = *self
            .node_indexes
            .get(&to)
            .expect("history map target node is retained");
        if from == to {
            return Ok(identity);
        }

        let from_path = self.root_path(from);
        let to_path = self.root_path(to);
        let common = from_path
            .iter()
            .zip(&to_path)
            .take_while(|(left, right)| left == right)
            .count();

        let mut map = identity;
        for child in from_path[common..].iter().rev() {
            let edge = self.nodes[*child]
                .incoming
                .as_ref()
                .expect("every non-root document history node has an edge");
            map = map.then(&edge.map.inverted()?)?;
        }
        for child in &to_path[common..] {
            let edge = self.nodes[*child]
                .incoming
                .as_ref()
                .expect("every non-root document history node has an edge");
            map = map.then(&edge.map)?;
        }
        Ok(map)
    }
}

fn next_node_id() -> HistoryNodeId {
    HistoryNodeId(NEXT_HISTORY_NODE_ID.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::source::SourceSnapshot;

    #[derive(Clone)]
    struct ByteState(SourceSnapshot);

    fn visit_byte_state(state: &ByteState, visitor: &mut dyn FnMut(usize, usize)) {
        state.0.visit_retained_buffers(visitor);
    }

    fn digest_byte_state(state: &ByteState) -> SourceArtifactDigest {
        state.0.artifact_digest()
    }

    fn byte_history(
        state: SourceSnapshot,
        retention: HistoryRetentionPolicy,
    ) -> History<ByteState> {
        History::new_accounted(
            ByteState(state),
            retention,
            visit_byte_state,
            digest_byte_state,
            |state| state.0.identity(),
        )
    }

    #[test]
    fn command_checkpoint_journals_only_touched_history_nodes() {
        let mut history = History::new(0usize);
        for value in 1..128 {
            history.commit(value, false);
        }
        let before = history.status();
        history.begin_command_checkpoint();
        assert!(history.command_checkpoints.last().unwrap().nodes.is_empty());
        history.commit(128, true);
        history.commit(129, true);
        assert_eq!(history.command_checkpoints.last().unwrap().nodes.len(), 1);
        assert!(history
            .command_checkpoints
            .last()
            .unwrap()
            .nodes
            .values()
            .all(|node| node.content.is_none()));
        history.rollback_command_checkpoint();
        assert_eq!(history.status(), before);
        assert_eq!(**history.current(), 127);
    }

    #[test]
    fn retains_branches_and_selects_redo_path() {
        let mut history = History::new("root");
        history.commit("first", false);
        let first = history.status().current;
        assert!(history.undo());
        history.commit("second", false);
        let second = history.status().current;
        assert!(history.undo());
        assert_eq!(history.redo_branch_count(), 2);
        assert!(history.select_redo_branch(0));
        assert!(history.redo());
        assert_eq!(**history.current(), "first");
        assert_eq!(history.status().current, first);

        assert!(history.undo());
        assert_eq!(history.redo_branches()[0].destination, first);
        assert!(history.select_redo_branch(1));
        assert!(history.redo());
        assert_eq!(history.status().current, second);
    }

    #[test]
    fn undo_makes_the_child_just_left_the_preferred_redo() {
        let mut history = History::new("root");
        history.commit("first", false);
        let first = history.status().current;
        assert!(history.undo());
        history.commit("second", false);
        assert!(history.undo());
        history.select_node(first.node).unwrap();
        history.undo_exact().unwrap();

        let branches = history.redo_branches();
        assert!(branches[0].preferred);
        assert!(!branches[1].preferred);
        assert_eq!(history.redo_exact().unwrap().to, first);
    }

    #[test]
    fn exact_navigation_by_node_and_change_updates_the_preferred_path() {
        let mut history = History::new("root");
        let root = history.status().current;
        history.commit("a", false);
        let a = history.status().current;
        history.commit("ab", false);
        let ab = history.status().current;
        history.select_node(root.node).unwrap();
        history.commit("x", false);
        let x = history.status().current;

        let movement = history.select_change(ab.change).unwrap();
        assert_eq!(movement.from, x);
        assert_eq!(movement.to, ab);
        assert_eq!(**history.current(), "ab");
        history.undo_exact().unwrap();
        assert_eq!(history.status().current, a);
        history.undo_exact().unwrap();
        assert_eq!(history.redo_exact().unwrap().to, a);

        let missing = HistoryChangeNumber::from_u64(999);
        let before = history.status().current;
        assert_eq!(
            history.select_change(missing),
            Err(HistoryError::ChangeNotFound(missing))
        );
        assert_eq!(history.status().current, before);
    }

    #[test]
    fn grouped_commits_use_one_node_and_one_change_number() {
        let mut history = History::new(0);
        history.begin_group();
        history.commit(1, true);
        let group = history.status().current;
        history.commit(2, true);
        history.end_group();
        assert_eq!(history.node_count(), 2);
        assert_eq!(history.status().current, group);
        assert_eq!(group.change, HistoryChangeNumber::from_u64(1));
        assert!(history.undo());
        assert_eq!(**history.current(), 0);
    }

    #[test]
    fn status_and_invalid_branch_selection_are_non_destructive() {
        let mut history = History::<i32>::new(0);
        assert_eq!(
            history.undo_exact(),
            Err(HistoryError::Boundary(HistoryBoundary::Oldest))
        );
        assert_eq!(
            history.redo_exact(),
            Err(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo))
        );
        let before = history.status();
        assert_eq!(
            history.prefer_redo_branch(3),
            Err(HistoryError::RedoBranchNotFound {
                requested: 3,
                available: 0
            })
        );
        assert_eq!(history.status(), before);
    }

    #[test]
    fn save_point_tracks_node_identity_across_branches() {
        let mut history = History::new("root");
        assert!(!history.status().is_dirty);
        history.commit("saved", false);
        let saved = history.mark_saved();
        assert!(!history.status().is_dirty);
        history.commit("later", false);
        assert!(history.status().is_dirty);
        history.undo_exact().unwrap();
        assert_eq!(history.status().current.node, saved);
        assert!(!history.status().is_dirty);
    }

    #[test]
    fn node_budget_prunes_oldest_non_current_leaf_branches_first() {
        let mut history = History::new("root");
        let root = history.status().current;
        history.commit("a", false);
        let a = history.status().current;
        history.commit("a1", false);
        let a1 = history.status().current;
        history.select_node(root.node).unwrap();
        history.commit("b", false);
        let b = history.status().current;
        history.select_node(root.node).unwrap();
        history.commit("c", false);
        let c = history.status().current;

        history.set_retention_policy(HistoryRetentionPolicy::new(3, usize::MAX));

        assert_eq!(history.status().node_count, 3);
        assert_eq!(history.status().current, c);
        assert!(history.location_for_node(root.node).is_some());
        assert!(history.location_for_node(b.node).is_some());
        assert!(history.location_for_node(a.node).is_none());
        assert!(history.location_for_node(a1.node).is_none());
        assert_eq!(
            history.select_change(a1.change),
            Err(HistoryError::ChangeNotFound(a1.change))
        );
    }

    #[test]
    fn current_ancestry_promotes_the_oldest_retained_state_to_root() {
        let mut history = History::new("root");
        let root = history.status().current;
        history.commit("a", false);
        let a = history.status().current;
        history.commit("b", false);
        history.commit("c", false);
        history.set_retention_policy(HistoryRetentionPolicy::new(3, usize::MAX));

        assert_eq!(history.status().node_count, 3);
        assert_eq!(
            history.select_node(root.node),
            Err(HistoryError::NodeNotFound(root.node))
        );
        assert!(history.undo());
        assert!(history.undo());
        assert_eq!(history.status().current, a);
        assert_eq!(
            history.undo_exact(),
            Err(HistoryError::Boundary(HistoryBoundary::Oldest))
        );
    }

    #[test]
    fn open_group_parent_and_result_can_temporarily_exceed_budget() {
        let mut history = History::new(0);
        history.set_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
        history.begin_group();
        history.commit(1, true);
        let result = history.status().current;
        history.commit(2, true);
        assert_eq!(history.status().node_count, 2);
        assert!(history.status().can_undo);

        history.end_group();
        assert_eq!(history.status().node_count, 1);
        assert_eq!(history.status().current, result);
        assert_eq!(**history.current(), 2);
        assert!(!history.status().can_undo);
    }

    #[test]
    fn pruned_saved_node_keeps_identity_and_dirty_comparison() {
        let mut history = History::new("root");
        let root = history.status().current;
        history.commit("saved", false);
        let saved = history.status().current;
        history.mark_saved();
        history.select_node(root.node).unwrap();
        history.commit("other", false);
        history.set_retention_policy(HistoryRetentionPolicy::new(2, usize::MAX));

        let status = history.status();
        assert_eq!(status.save_point, saved.node);
        assert!(!status.save_point_retained);
        assert!(status.is_dirty);
        assert_eq!(
            history.select_node(saved.node),
            Err(HistoryError::NodeNotFound(saved.node))
        );
    }

    #[test]
    fn source_identity_keeps_configuration_nodes_clean_across_branches_and_pruning() {
        let source = SourceSnapshot::new(b"saved".to_vec());
        let mut history = byte_history(source.clone(), HistoryRetentionPolicy::unlimited());
        let root = history.status().current;
        history.commit(ByteState(source.clone()), false);
        let configuration = history.status().current;
        assert_ne!(configuration, root);
        assert!(!history.status().is_dirty);
        history.mark_saved();
        history.select_node(root.node).unwrap();
        assert!(!history.status().is_dirty);
        history.commit(ByteState(source.clone()), false);
        assert!(!history.status().is_dirty);
        history.set_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
        assert!(!history.status().save_point_retained);
        assert!(!history.status().is_dirty);
        // Identical serialized bytes from a new source edit are still a new
        // snapshot; configuration-only sharing must not become byte equality.
        let replaced = source.replace(0, 5, b"saved".to_vec()).unwrap();
        history.commit(ByteState(replaced), false);
        assert!(history.status().is_dirty);
    }

    #[test]
    fn checking_dirty_state_does_not_digest_the_source() {
        static DIGEST_CALLS: AtomicU64 = AtomicU64::new(0);
        fn counted_digest(state: &ByteState) -> SourceArtifactDigest {
            DIGEST_CALLS.fetch_add(1, Ordering::Relaxed);
            state.0.artifact_digest()
        }
        let source = SourceSnapshot::new(vec![b'x'; 1_000_000]);
        let mut history: History<ByteState> = History::new_accounted(
            ByteState(source.clone()), HistoryRetentionPolicy::unlimited(),
            visit_byte_state, counted_digest, |state| state.0.identity(),
        );
        let initial = DIGEST_CALLS.load(Ordering::Relaxed);
        history.commit(ByteState(source), false);
        for _ in 0..1_000 { assert!(!history.status().is_dirty); }
        assert_eq!(DIGEST_CALLS.load(Ordering::Relaxed), initial);
    }

    #[test]
    fn byte_accounting_charges_shared_source_allocations_once() {
        let original = SourceSnapshot::new(b"abcdef".to_vec());
        let changed = original.replace(2, 4, b"XYZ".to_vec()).unwrap();
        let mut history = byte_history(original, HistoryRetentionPolicy::unlimited());
        assert_eq!(history.status().retained_source_bytes, 6);

        history.commit(ByteState(changed), false);
        assert_eq!(history.status().retained_source_bytes, 9);
    }

    #[test]
    fn retained_byte_budget_prunes_branches_before_current_ancestry() {
        let original = SourceSnapshot::new(b"seed".to_vec());
        let branch_a = original.replace(4, 4, b"A".to_vec()).unwrap();
        let branch_b = original.replace(4, 4, b"B".to_vec()).unwrap();
        let mut history = byte_history(original, HistoryRetentionPolicy::unlimited());
        history.commit(ByteState(branch_a), false);
        let a = history.status().current;
        history.undo_exact().unwrap();
        history.commit(ByteState(branch_b), false);

        assert_eq!(history.status().retained_source_bytes, 6);
        history.set_retention_policy(HistoryRetentionPolicy::new(10, 5));
        assert_eq!(history.status().retained_source_bytes, 5);
        assert_eq!(history.status().node_count, 2);
        assert!(history.location_for_node(a.node).is_none());
        assert!(history.status().can_undo);
    }

    #[test]
    fn additional_budget_preserves_shared_live_state_but_prunes_excess_history() {
        let original = SourceSnapshot::new(vec![b'a'; 1024 * 1024]);
        let mut history = byte_history(original.clone(), HistoryRetentionPolicy::additional_history(10, 2));
        assert!(HistoryRetentionPolicy::default().is_additional_to_live_state());
        let edited = original.replace(2, 3, vec![b'b']).unwrap();
        history.commit(ByteState(edited.clone()), false);
        assert!(history.status().can_undo, "the live megabyte must not consume history headroom");
        assert_eq!(history.status().additional_history_memory_bytes, 0);
        // Entirely replacing the live content makes the old megabyte history.
        let replaced = edited.replace(0, edited.len(), vec![b'c'; 1024 * 1024]).unwrap();
        history.commit(ByteState(replaced), false);
        assert!(!history.status().can_undo);
        assert_eq!(history.status().node_count, 1);
    }

    #[test]
    fn retained_byte_budget_promotes_ancestry_but_never_drops_current() {
        let original = SourceSnapshot::new(b"AAAA".to_vec());
        let replacement = original.replace(0, 4, b"BBBB".to_vec()).unwrap();
        let mut history = byte_history(original, HistoryRetentionPolicy::new(usize::MAX, 4));
        history.commit(ByteState(replacement), false);

        assert_eq!(history.status().node_count, 1);
        assert_eq!(history.status().retained_source_bytes, 4);
        assert!(!history.status().can_undo);
    }
}
