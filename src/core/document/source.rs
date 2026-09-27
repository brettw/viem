//! Persistent byte storage for authoritative source snapshots.
//!
//! Leaves reference immutable byte buffers.  Splitting a leaf creates two
//! slices of the same buffer and concatenation builds a balanced persistent
//! rope, so a revision shares every untouched byte with its predecessor.

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

const SOURCE_BUFFER_BYTES: usize = 64 * 1024;

static NEXT_SOURCE_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);

/// Identity of one immutable source revision, retained independently of any
/// undo node or projection. Clones preserve it; real source edits replace it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceSnapshotIdentity(u64);

impl SourceSnapshotIdentity {
    fn fresh() -> Self {
        Self(
            NEXT_SOURCE_SNAPSHOT_ID
                .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |id| {
                    id.checked_add(1)
                })
                .expect("source snapshot identity exhausted"),
        )
    }
}

/// Deterministic digest of the exact serialized bytes in one source artifact.
///
/// History keeps this small value after a saved history node is pruned.  Dirty
/// state still uses the exact saved snapshot identity; the digest is retained
/// for persistence diagnostics and future persistent-history validation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceArtifactDigest([u8; 32]);

impl SourceArtifactDigest {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SourceSnapshot {
    identity: SourceSnapshotIdentity,
    root: Option<Arc<Node>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceDiffStats {
    pub nodes_visited: usize,
    pub bytes_compared: usize,
}

#[derive(Debug)]
enum Node {
    Leaf(Piece),
    Branch {
        left: Arc<Node>,
        right: Arc<Node>,
        len: usize,
        height: u8,
    },
}

#[derive(Debug)]
struct Piece {
    bytes: Arc<[u8]>,
    start: usize,
    len: usize,
}

impl SourceSnapshot {
    pub(super) fn visit_retained_memory(
        &self,
        visitor: &mut super::history_memory::MemoryVisitor<'_>,
    ) {
        fn visit(node: &Arc<Node>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
            visitor.arc(node, |visitor| match node.as_ref() {
                Node::Leaf(piece) => visitor.arc(&piece.bytes, |_| {}),
                Node::Branch { left, right, .. } => {
                    visit(left, visitor);
                    visit(right, visitor);
                }
            });
        }
        if let Some(root) = &self.root {
            visit(root, visitor);
        }
    }

    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        let root = tree_from_bytes(&bytes);
        Self {
            identity: SourceSnapshotIdentity::fresh(),
            root,
        }
    }

    pub(crate) fn identity(&self) -> SourceSnapshotIdentity {
        self.identity
    }

    pub(crate) fn len(&self) -> usize {
        self.root.as_deref().map(Node::len).unwrap_or(0)
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        super::work_statistics::record(|stats| {
            stats.source_full_materializations += 1;
            stats.source_full_materialized_bytes += self.len();
        });
        let mut result = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            root.append_to(&mut result);
        }
        result
    }

    /// Compare persistent artifacts without serializing their unchanged
    /// prefixes/suffixes. Split pieces retain their backing-buffer identity.
    pub(crate) fn changed_extent(&self, next: &Self) -> Option<(Range<usize>, Range<usize>)> {
        self.changed_extent_with_stats(next).0
    }

    pub(crate) fn changed_extent_with_stats(
        &self,
        next: &Self,
    ) -> (Option<(Range<usize>, Range<usize>)>, SourceDiffStats) {
        let mut stats = SourceDiffStats::default();
        let prefix = shared_source_edge(self, next, false, &mut stats);
        if prefix == self.len() && prefix == next.len() {
            return (None, stats);
        }
        let suffix = shared_source_edge(self, next, true, &mut stats)
            .min(self.len().min(next.len()) - prefix);
        (
            Some((prefix..self.len() - suffix, prefix..next.len() - suffix)),
            stats,
        )
    }

    /// Return disjoint byte changes, keeping unchanged immutable pieces between
    /// distant edits out of history-navigation replacement buffers. Shared
    /// backing identity, rather than coincidentally equal text, aligns interior
    /// ranges. Crossing/repeated pieces may produce a conservative larger gap.
    /// Only piece metadata inside the changed hull is retained temporarily.
    pub(crate) fn changed_extents(&self, next: &Self) -> Vec<(Range<usize>, Range<usize>)> {
        let Some((old, new)) = self.changed_extent(next) else {
            return Vec::new();
        };
        if old.is_empty() || new.is_empty() {
            return vec![(old, new)];
        }

        #[derive(Clone, Copy)]
        struct PieceRange {
            buffer: usize,
            start: usize,
            end: usize,
            document: usize,
        }
        fn collect(node: &Node, at: usize, range: &Range<usize>, into: &mut Vec<PieceRange>) {
            if at >= range.end || at + node.len() <= range.start {
                return;
            }
            match node {
                Node::Leaf(piece) => {
                    let begin = at.max(range.start);
                    let end = (at + piece.len).min(range.end);
                    into.push(PieceRange {
                        buffer: Arc::as_ptr(&piece.bytes) as *const u8 as usize,
                        start: piece.start + begin - at,
                        end: piece.start + end - at,
                        document: begin,
                    });
                }
                Node::Branch { left, right, .. } => {
                    collect(left, at, range, into);
                    collect(right, at + left.len(), range, into);
                }
            }
        }
        let mut previous = Vec::new();
        let mut following = Vec::new();
        collect(
            self.root.as_deref().expect("nonempty source"),
            0,
            &old,
            &mut previous,
        );
        collect(
            next.root.as_deref().expect("nonempty source"),
            0,
            &new,
            &mut following,
        );
        // A repeated/moved buffer can overlap many pieces. Bound the work and
        // scratch metadata in that unusual case; one exact hull remains valid.
        let match_limit = (previous.len() + following.len()).saturating_mul(4);
        let mut by_buffer = std::collections::HashMap::<usize, Vec<(PieceRange, usize)>>::new();
        for piece in previous {
            by_buffer.entry(piece.buffer).or_default().push((piece, 0));
        }
        for pieces in by_buffer.values_mut() {
            pieces.sort_unstable_by_key(|(piece, _)| piece.start);
            let mut maximum_end = 0;
            for (piece, prefix_end) in pieces {
                maximum_end = maximum_end.max(piece.end);
                *prefix_end = maximum_end;
            }
        }
        let mut changes = Vec::new();
        let mut old_at = old.start;
        let mut new_at = new.start;
        let mut visited_matches = 0;
        for piece in following {
            let Some(candidates) = by_buffer.get(&piece.buffer) else {
                continue;
            };
            let begin = candidates.partition_point(|(_, prefix_end)| *prefix_end <= piece.start);
            let end = candidates.partition_point(|(previous, _)| previous.start < piece.end);
            let mut matches = Vec::new();
            for (previous, _) in &candidates[begin..end] {
                visited_matches += 1;
                if visited_matches > match_limit {
                    return vec![(old, new)];
                }
                let start = previous.start.max(piece.start);
                let end = previous.end.min(piece.end);
                if start < end {
                    matches.push((
                        piece.document + start - piece.start,
                        previous.document + start - previous.start,
                        end - start,
                    ));
                }
            }
            matches.sort_unstable();
            for (new_start, old_start, length) in matches {
                let skip = old_at
                    .saturating_sub(old_start)
                    .max(new_at.saturating_sub(new_start));
                if skip >= length {
                    continue;
                }
                let old_start = old_start + skip;
                let new_start = new_start + skip;
                if old_at < old_start || new_at < new_start {
                    changes.push((old_at..old_start, new_at..new_start));
                }
                old_at = old_start + length - skip;
                new_at = new_start + length - skip;
            }
        }
        if old_at < old.end || new_at < new.end {
            changes.push((old_at..old.end, new_at..new.end));
        }
        changes
    }

    /// Materialize only one validated byte range. Splitting a persistent rope
    /// clones the logarithmic path while retaining its immutable buffers, so a
    /// ranged write does not first flatten an unrelated multi-megabyte source.
    pub(crate) fn bytes_in(&self, range: Range<usize>) -> Option<Vec<u8>> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }
        super::work_statistics::record(|stats| {
            stats.source_range_materializations += 1;
            stats.source_range_materialized_bytes += range.len();
        });
        let (_, suffix) = split(self.root.clone(), range.start);
        let (selected, _) = split(suffix, range.end - range.start);
        let mut result = Vec::with_capacity(range.end - range.start);
        if let Some(selected) = selected {
            selected.append_to(&mut result);
        }
        Some(result)
    }

    pub(crate) fn artifact_digest(&self) -> SourceArtifactDigest {
        // Digesting is currently needed only when establishing a save point,
        // not on every edit.  Keeping the implementation here makes the
        // physical byte sequence, rather than a derived projection, the sole
        // authority for the result.
        let mut digest = Sha256::new();
        if let Some(root) = &self.root {
            root.visit_bytes(&mut |bytes| digest.update(bytes));
        }
        SourceArtifactDigest(digest.finish())
    }

    /// Visit every immutable byte-buffer allocation reachable from this
    /// snapshot. A buffer can appear through several sliced pieces; callers
    /// that aggregate snapshots deduplicate the allocation identity.
    pub(crate) fn visit_retained_buffers(&self, visitor: &mut dyn FnMut(usize, usize)) {
        if let Some(root) = &self.root {
            root.visit_retained_buffers(visitor);
        }
    }

    pub(crate) fn replace(&self, start: usize, end: usize, replacement: Vec<u8>) -> Option<Self> {
        if start > end || end > self.len() {
            return None;
        }
        if start == end && replacement.is_empty() {
            // An empty splice is the identity map. Preserve the exact root so
            // stable piece identity and constant-time snapshot sharing do not
            // depend on a caller filtering no-op patch records first.
            return Some(self.clone());
        }

        let (before, rest) = split(self.root.clone(), start);
        let (_, after) = split(rest, end - start);
        let inserted = tree_from_bytes(&replacement);
        Some(Self {
            identity: SourceSnapshotIdentity::fresh(),
            root: concat(concat(before, inserted), after),
        })
    }

    #[cfg(test)]
    fn height(&self) -> u8 {
        self.root.as_deref().map(Node::height).unwrap_or(0)
    }
}

fn shared_source_edge(
    a: &SourceSnapshot,
    b: &SourceSnapshot,
    reverse: bool,
    stats: &mut SourceDiffStats,
) -> usize {
    fn expand<'a>(stack: &mut Vec<(&'a Node, usize)>, reverse: bool) -> bool {
        let Some((Node::Branch { left, right, .. }, _)) = stack.last() else {
            return false;
        };
        let (left, right) = (left.as_ref(), right.as_ref());
        stack.pop();
        if reverse {
            stack.push((left, 0));
            stack.push((right, 0));
        } else {
            stack.push((right, 0));
            stack.push((left, 0));
        }
        true
    }
    let mut left = a.root.as_deref().map(|n| vec![(n, 0)]).unwrap_or_default();
    let mut right = b.root.as_deref().map(|n| vec![(n, 0)]).unwrap_or_default();
    let mut count = 0;
    while let (Some(&(l, lo)), Some(&(r, ro))) = (left.last(), right.last()) {
        stats.nodes_visited += 1;
        if lo == 0 && ro == 0 && std::ptr::eq(l, r) {
            count += l.len();
            left.pop();
            right.pop();
            continue;
        }
        match (l, r) {
            (Node::Branch { .. }, Node::Branch { .. }) => {
                if l.len() >= r.len() {
                    expand(&mut left, reverse);
                } else {
                    expand(&mut right, reverse);
                }
                continue;
            }
            (Node::Branch { .. }, _) => {
                expand(&mut left, reverse);
                continue;
            }
            (_, Node::Branch { .. }) => {
                expand(&mut right, reverse);
                continue;
            }
            (Node::Leaf(l), Node::Leaf(r)) => {
                let n = (l.len - lo).min(r.len - ro);
                let (lb, rb) = if reverse {
                    (l.start + l.len - lo, r.start + r.len - ro)
                } else {
                    (l.start + lo, r.start + ro)
                };
                // Byte equality across unshared buffers does not establish
                // continuing identity. Repeated separately allocated pieces
                // could otherwise turn a local edit into a full suffix scan.
                // The reported change is deliberately conservative.
                if !Arc::ptr_eq(&l.bytes, &r.bytes) || lb != rb {
                    return count;
                }
                count += n;
                if lo + n == l.len {
                    left.pop();
                } else {
                    left.last_mut().unwrap().1 += n;
                }
                if ro + n == r.len {
                    right.pop();
                } else {
                    right.last_mut().unwrap().1 += n;
                }
            }
        }
    }
    count
}

impl Node {
    fn len(&self) -> usize {
        match self {
            Self::Leaf(piece) => piece.len,
            Self::Branch { len, .. } => *len,
        }
    }

    fn height(&self) -> u8 {
        match self {
            Self::Leaf(_) => 1,
            Self::Branch { height, .. } => *height,
        }
    }

    fn visit_bytes(&self, visitor: &mut dyn FnMut(&[u8])) {
        match self {
            Self::Leaf(piece) => visitor(&piece.bytes[piece.start..piece.start + piece.len]),
            Self::Branch { left, right, .. } => {
                left.visit_bytes(visitor);
                right.visit_bytes(visitor);
            }
        }
    }

    fn append_to(&self, output: &mut Vec<u8>) {
        match self {
            Self::Leaf(piece) => {
                output.extend_from_slice(&piece.bytes[piece.start..piece.start + piece.len]);
            }
            Self::Branch { left, right, .. } => {
                left.append_to(output);
                right.append_to(output);
            }
        }
    }

    fn visit_retained_buffers(&self, visitor: &mut dyn FnMut(usize, usize)) {
        match self {
            Self::Leaf(piece) => visitor(
                Arc::as_ptr(&piece.bytes).cast::<u8>() as usize,
                piece.bytes.len(),
            ),
            Self::Branch { left, right, .. } => {
                left.visit_retained_buffers(visitor);
                right.visit_retained_buffers(visitor);
            }
        }
    }
}

/// Bound backing allocations independently of rope pieces, so keeping a tiny
/// surviving slice does not pin a whole deleted or replaced artifact.
fn tree_from_bytes(bytes: &[u8]) -> Option<Arc<Node>> {
    fn balanced(nodes: &[Arc<Node>]) -> Option<Arc<Node>> {
        match nodes.len() {
            0 => None,
            1 => Some(nodes[0].clone()),
            length => {
                let middle = length / 2;
                Some(branch(
                    balanced(&nodes[..middle]).unwrap(),
                    balanced(&nodes[middle..]).unwrap(),
                ))
            }
        }
    }
    let leaves: Vec<_> = bytes
        .chunks(SOURCE_BUFFER_BYTES)
        .map(|bytes| {
            Arc::new(Node::Leaf(Piece {
                bytes: Arc::from(bytes),
                start: 0,
                len: bytes.len(),
            }))
        })
        .collect();
    balanced(&leaves)
}

// SHA-256 consumes source pieces directly and retains one partial block.
struct Sha256 {
    state: [u32; 8],
    pending: [u8; 64],
    used: usize,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        const INITIAL: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        Self {
            state: INITIAL,
            pending: [0; 64],
            used: 0,
            length: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.length = self.length.wrapping_add(input.len() as u64);
        if self.used > 0 {
            let count = input.len().min(64 - self.used);
            self.pending[self.used..self.used + count].copy_from_slice(&input[..count]);
            self.used += count;
            input = &input[count..];
            if self.used == 64 {
                self.compress(&self.pending.clone());
                self.used = 0;
            }
        }
        while input.len() >= 64 {
            self.compress(&input[..64]);
            input = &input[64..];
        }
        if !input.is_empty() {
            self.pending[..input.len()].copy_from_slice(input);
            self.used = input.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        self.pending[self.used] = 0x80;
        self.pending[self.used + 1..].fill(0);
        if self.used >= 56 {
            self.compress(&self.pending.clone());
            self.pending.fill(0);
        }
        self.pending[56..].copy_from_slice(&self.length.wrapping_mul(8).to_be_bytes());
        self.compress(&self.pending.clone());
        let mut output = [0; 32];
        for (chunk, word) in output.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    fn compress(&mut self, block: &[u8]) {
        const K: [u32; 64] = [
            0x428a_2f98,
            0x7137_4491,
            0xb5c0_fbcf,
            0xe9b5_dba5,
            0x3956_c25b,
            0x59f1_11f1,
            0x923f_82a4,
            0xab1c_5ed5,
            0xd807_aa98,
            0x1283_5b01,
            0x2431_85be,
            0x550c_7dc3,
            0x72be_5d74,
            0x80de_b1fe,
            0x9bdc_06a7,
            0xc19b_f174,
            0xe49b_69c1,
            0xefbe_4786,
            0x0fc1_9dc6,
            0x240c_a1cc,
            0x2de9_2c6f,
            0x4a74_84aa,
            0x5cb0_a9dc,
            0x76f9_88da,
            0x983e_5152,
            0xa831_c66d,
            0xb003_27c8,
            0xbf59_7fc7,
            0xc6e0_0bf3,
            0xd5a7_9147,
            0x06ca_6351,
            0x1429_2967,
            0x27b7_0a85,
            0x2e1b_2138,
            0x4d2c_6dfc,
            0x5338_0d13,
            0x650a_7354,
            0x766a_0abb,
            0x81c2_c92e,
            0x9272_2c85,
            0xa2bf_e8a1,
            0xa81a_664b,
            0xc24b_8b70,
            0xc76c_51a3,
            0xd192_e819,
            0xd699_0624,
            0xf40e_3585,
            0x106a_a070,
            0x19a4_c116,
            0x1e37_6c08,
            0x2748_774c,
            0x34b0_bcb5,
            0x391c_0cb3,
            0x4ed8_aa4a,
            0x5b9c_ca4f,
            0x682e_6ff3,
            0x748f_82ee,
            0x78a5_636f,
            0x84c8_7814,
            0x8cc7_0208,
            0x90be_fffa,
            0xa450_6ceb,
            0xbef9_a3f7,
            0xc671_78f2,
        ];
        let mut words = [0_u32; 64];
        for (index, bytes) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..64 {
            let x = words[index - 15];
            let y = words[index - 2];
            let small_zero = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let small_one = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            words[index] = words[index - 16]
                .wrapping_add(small_zero)
                .wrapping_add(words[index - 7])
                .wrapping_add(small_one);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let big_one = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temporary_one = h
                .wrapping_add(big_one)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let big_zero = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary_two = big_zero.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary_one);
            d = c;
            c = b;
            b = a;
            a = temporary_one.wrapping_add(temporary_two);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn sha256(input: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(input);
    digest.finish()
}

fn branch(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    Arc::new(Node::Branch {
        len: left.len() + right.len(),
        height: left.height().max(right.height()).saturating_add(1),
        left,
        right,
    })
}

/// Join two AVL-like ropes without flattening either side.
fn concat(left: Option<Arc<Node>>, right: Option<Arc<Node>>) -> Option<Arc<Node>> {
    match (left, right) {
        (None, other) | (other, None) => other,
        (Some(left), Some(right)) => Some(join(left, right)),
    }
}

fn join(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    let left_height = left.height();
    let right_height = right.height();
    if left_height > right_height.saturating_add(1) {
        let Node::Branch {
            left: outer,
            right: inner,
            ..
        } = left.as_ref()
        else {
            unreachable!("a leaf cannot be more than one level taller")
        };
        return balance(outer.clone(), join(inner.clone(), right));
    }
    if right_height > left_height.saturating_add(1) {
        let Node::Branch {
            left: inner,
            right: outer,
            ..
        } = right.as_ref()
        else {
            unreachable!("a leaf cannot be more than one level taller")
        };
        return balance(join(left, inner.clone()), outer.clone());
    }
    branch(left, right)
}

fn balance(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height() > right.height().saturating_add(1) {
        let Node::Branch {
            left: left_outer,
            right: left_inner,
            ..
        } = left.as_ref()
        else {
            unreachable!("unbalanced left side must be a branch")
        };
        if left_outer.height() >= left_inner.height() {
            return branch(left_outer.clone(), branch(left_inner.clone(), right));
        }
        let Node::Branch {
            left: middle_left,
            right: middle_right,
            ..
        } = left_inner.as_ref()
        else {
            unreachable!("inner-heavy branch must contain a branch")
        };
        return branch(
            branch(left_outer.clone(), middle_left.clone()),
            branch(middle_right.clone(), right),
        );
    }

    if right.height() > left.height().saturating_add(1) {
        let Node::Branch {
            left: right_inner,
            right: right_outer,
            ..
        } = right.as_ref()
        else {
            unreachable!("unbalanced right side must be a branch")
        };
        if right_outer.height() >= right_inner.height() {
            return branch(branch(left, right_inner.clone()), right_outer.clone());
        }
        let Node::Branch {
            left: middle_left,
            right: middle_right,
            ..
        } = right_inner.as_ref()
        else {
            unreachable!("inner-heavy branch must contain a branch")
        };
        return branch(
            branch(left, middle_left.clone()),
            branch(middle_right.clone(), right_outer.clone()),
        );
    }

    branch(left, right)
}

fn split(root: Option<Arc<Node>>, at: usize) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
    let Some(root) = root else {
        return (None, None);
    };
    debug_assert!(at <= root.len());

    if at == 0 {
        return (None, Some(root));
    }
    if at == root.len() {
        return (Some(root), None);
    }

    match root.as_ref() {
        Node::Leaf(piece) => {
            let left = Arc::new(Node::Leaf(Piece {
                bytes: piece.bytes.clone(),
                start: piece.start,
                len: at,
            }));
            let right = Arc::new(Node::Leaf(Piece {
                bytes: piece.bytes.clone(),
                start: piece.start + at,
                len: piece.len - at,
            }));
            (Some(left), Some(right))
        }
        Node::Branch { left, right, .. } => match at.cmp(&left.len()) {
            Ordering::Less => {
                let (a, b) = split(Some(left.clone()), at);
                (a, concat(b, Some(right.clone())))
            }
            Ordering::Equal => (Some(left.clone()), Some(right.clone())),
            Ordering::Greater => {
                let (a, b) = split(Some(right.clone()), at - left.len());
                (concat(Some(left.clone()), a), b)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_splices_preserve_old_revision() {
        let original = SourceSnapshot::new(b"abcdef".to_vec());
        let changed = original.replace(2, 4, b"XYZ".to_vec()).unwrap();
        assert_eq!(original.bytes(), b"abcdef");
        assert_eq!(changed.bytes(), b"abXYZef");
    }

    #[test]
    fn empty_splice_preserves_the_exact_persistent_root() {
        let original = SourceSnapshot::new(b"abcdef".to_vec());
        let unchanged = original.replace(3, 3, Vec::new()).unwrap();

        assert_eq!(unchanged.bytes(), b"abcdef");
        assert_eq!(unchanged.identity(), original.identity());
        assert!(Arc::ptr_eq(
            original.root.as_ref().unwrap(),
            unchanged.root.as_ref().unwrap()
        ));
    }

    #[test]
    fn source_identity_distinguishes_empty_and_byte_equal_revisions() {
        let empty = SourceSnapshot::new(Vec::new());
        let another_empty = SourceSnapshot::new(Vec::new());
        assert_ne!(empty.identity(), another_empty.identity());
        assert_eq!(empty.identity(), empty.clone().identity());
        let changed = empty.replace(0, 0, b"x".to_vec()).unwrap();
        let emptied = changed.replace(0, 1, Vec::new()).unwrap();
        assert!(emptied.bytes().is_empty());
        assert_ne!(empty.identity(), emptied.identity());
    }

    #[test]
    fn bounded_materialization_crosses_piece_boundaries_without_flattening_semantics() {
        let original = SourceSnapshot::new(b"abcdefghij".to_vec());
        let changed = original.replace(3, 7, b"XYZ".to_vec()).unwrap();

        assert_eq!(changed.bytes(), b"abcXYZhij");
        assert_eq!(changed.bytes_in(2..8).unwrap(), b"cXYZhi");
        assert_eq!(changed.bytes_in(4..4).unwrap(), b"");
        assert!(changed.bytes_in(8..20).is_none());
        let reversed_end = changed.len() - 3;
        assert!(changed.bytes_in(reversed_end + 1..reversed_end).is_none());
    }

    #[test]
    fn large_deleted_source_releases_unreferenced_backing_chunks() {
        let source = SourceSnapshot::new(vec![b'x'; 16 * SOURCE_BUFFER_BYTES]);
        let survivor = source.replace(1, source.len() - 1, Vec::new()).unwrap();
        drop(source);
        let mut buffers = std::collections::HashMap::new();
        survivor.visit_retained_buffers(&mut |identity, bytes| {
            buffers.insert(identity, bytes);
        });
        assert_eq!(survivor.bytes(), b"xx");
        assert_eq!(buffers.values().sum::<usize>(), 2 * SOURCE_BUFFER_BYTES);
    }

    #[test]
    fn streaming_digest_matches_flat_bytes_across_piece_and_padding_boundaries() {
        for length in [0, 1, 55, 56, 63, 64, 65, 127, SOURCE_BUFFER_BYTES + 65] {
            let bytes: Vec<_> = (0..length).map(|at| (at % 251) as u8).collect();
            let expected = SourceArtifactDigest::from_bytes(&bytes);
            for chunk in [1, 7, 63, 64, 65, 1024] {
                let mut digest = Sha256::new();
                for slice in bytes.chunks(chunk) {
                    digest.update(slice);
                }
                assert_eq!(SourceArtifactDigest(digest.finish()), expected);
            }
            let source = SourceSnapshot::new(bytes);
            assert_eq!(source.artifact_digest(), expected);
            let changed = source
                .replace(length / 2, length / 2, b"inserted".to_vec())
                .unwrap();
            assert_eq!(
                changed.artifact_digest(),
                SourceArtifactDigest::from_bytes(&changed.bytes())
            );
        }
    }

    #[test]
    fn artifact_digest_uses_exact_serialized_bytes() {
        let digest = SourceArtifactDigest::from_bytes(b"abc");
        assert_eq!(
            digest.as_bytes(),
            &[
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
        assert_ne!(digest, SourceArtifactDigest::from_bytes(b"abc\n"));
    }

    #[test]
    fn artifact_digest_matches_sha256_padding_edge_vectors() {
        fn hex(bytes: &[u8]) -> String {
            use std::fmt::Write as _;

            let mut result = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                write!(&mut result, "{byte:02x}").unwrap();
            }
            result
        }

        let cases = [
            (
                0,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                55,
                "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
            ),
            (
                56,
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
            ),
            (
                64,
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
        ];
        for (length, expected) in cases {
            let input = vec![b'a'; length];
            assert_eq!(
                hex(SourceArtifactDigest::from_bytes(&input).as_bytes()),
                expected
            );
        }
    }

    #[test]
    fn sliced_pieces_report_the_underlying_buffer_once_by_identity() {
        let original = SourceSnapshot::new(b"abcdef".to_vec());
        let changed = original.replace(2, 4, b"XYZ".to_vec()).unwrap();
        let mut buffers = std::collections::HashMap::new();
        changed.visit_retained_buffers(&mut |identity, bytes| {
            buffers.insert(identity, bytes);
        });
        assert_eq!(buffers.len(), 2);
        assert_eq!(buffers.values().copied().sum::<usize>(), 9);
    }

    #[test]
    fn repeated_splices_stay_balanced() {
        let mut source = SourceSnapshot::new(Vec::new());
        for i in 0..2_000 {
            source = source
                .replace(source.len(), source.len(), vec![(i % 251) as u8])
                .unwrap();
        }
        assert_eq!(source.len(), 2_000);
        // The exact balancing strategy is private; this catches accidental
        // degeneration into a linear tree.
        assert!(source.height() < 40, "rope height was {}", source.height());
    }

    #[test]
    fn large_source_shapes_patch_by_sharing_unchanged_backing_chunks() {
        for bytes in ["x\n".repeat(1_000_000).into_bytes(), vec![b'x'; 2_000_000]] {
            let original = SourceSnapshot::new(bytes);
            let middle = 1_000_000;
            let changed = original
                .replace(middle, middle + 1, b"yz".to_vec())
                .unwrap();

            assert_eq!(original.bytes_in(middle..middle + 1).unwrap(), b"x");
            assert_eq!(changed.bytes_in(middle..middle + 2).unwrap(), b"yz");
            assert!(changed.height() < 8, "rope height was {}", changed.height());

            let mut original_buffers = std::collections::HashMap::new();
            original.visit_retained_buffers(&mut |identity, length| {
                original_buffers.insert(identity, length);
            });
            let mut changed_buffers = std::collections::HashMap::new();
            changed.visit_retained_buffers(&mut |identity, length| {
                changed_buffers.insert(identity, length);
            });
            assert!(original_buffers.len() > 1);
            for (identity, length) in &original_buffers {
                assert_eq!(changed_buffers.get(identity), Some(length));
                assert!(*length <= SOURCE_BUFFER_BYTES);
            }
            assert_eq!(changed_buffers.len(), original_buffers.len() + 1);
        }
    }

    #[test]
    fn hundred_mib_utf8_and_legacy_sources_patch_without_copying_the_artifact() {
        const SOURCE_BYTES: usize = 100 * 1024 * 1024;

        fn exercise(bytes: Vec<u8>, expected_at_patch: &[u8]) {
            assert_eq!(bytes.len(), SOURCE_BYTES);
            let original = SourceSnapshot::new(bytes);
            let middle = SOURCE_BYTES / 2;
            let changed = original
                .replace(middle, middle + expected_at_patch.len(), b"local".to_vec())
                .expect("a local source patch is representable");

            assert_eq!(
                original
                    .bytes_in(middle..middle + expected_at_patch.len())
                    .unwrap(),
                expected_at_patch
            );
            assert_eq!(changed.bytes_in(middle..middle + 5).unwrap(), b"local");
            let mut retained = std::collections::HashMap::new();
            changed.visit_retained_buffers(&mut |identity, length| {
                retained.insert(identity, length);
            });
            assert_eq!(retained.values().sum::<usize>(), SOURCE_BYTES + 5);
            assert!(retained
                .values()
                .all(|length| *length <= SOURCE_BUFFER_BYTES));
            assert_eq!(retained.values().filter(|length| **length == 5).count(), 1);
        }

        // Four-byte records keep the patch point aligned in both fixtures.
        // The first is valid mixed-width UTF-8; the second is representative
        // Latin-1 source containing a non-ASCII byte which is invalid alone
        // in UTF-8. Source storage remains encoding-agnostic in both cases.
        exercise(
            "x\u{00e9}\n".repeat(SOURCE_BYTES / 4).into_bytes(),
            b"x\xc3\xa9\n",
        );
        exercise(
            [b'x', 0xe9, b'y', b'\n'].repeat(SOURCE_BYTES / 4),
            b"x\xe9y\n",
        );
    }

    #[test]
    fn arbitrary_splices_match_a_flat_reference() {
        let mut source = SourceSnapshot::new(b"seed".to_vec());
        let mut reference = b"seed".to_vec();
        let mut random = 0x1234_5678_u64;
        for step in 0..1_000 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let start = (random as usize) % (reference.len() + 1);
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let end = start + (random as usize) % (reference.len() - start + 1);
            let replacement = vec![b'a' + (step % 26) as u8; step % 5];
            reference.splice(start..end, replacement.iter().copied());
            source = source.replace(start, end, replacement).unwrap();
            assert_eq!(source.bytes(), reference);
        }
        assert!(source.height() < 40, "rope height was {}", source.height());
    }

    fn assert_diff_reconstructs(old: &SourceSnapshot, new: &SourceSnapshot) {
        let old_bytes = old.bytes();
        let new_bytes = new.bytes();
        match old.changed_extent(new) {
            None => assert_eq!(old_bytes, new_bytes),
            Some((removed, inserted)) => {
                assert_eq!(removed.start, inserted.start);
                assert!(removed.start <= removed.end && removed.end <= old.len());
                assert!(inserted.start <= inserted.end && inserted.end <= new.len());
                assert_eq!(old_bytes[..removed.start], new_bytes[..inserted.start]);
                assert_eq!(old_bytes[removed.end..], new_bytes[inserted.end..]);
                let mut repaired = old_bytes;
                repaired.splice(removed, new.bytes_in(inserted).unwrap());
                assert_eq!(repaired, new_bytes);
            }
        }
        let mut repaired = old.bytes();
        let changes = old.changed_extents(new);
        for pair in changes.windows(2) {
            assert!(pair[0].0.end <= pair[1].0.start);
            assert!(pair[0].1.end <= pair[1].1.start);
        }
        for (removed, inserted) in changes.into_iter().rev() {
            repaired.splice(removed, new.bytes_in(inserted).unwrap());
        }
        assert_eq!(repaired, new_bytes);
    }

    #[test]
    fn sparse_source_diff_retains_shared_pieces_between_distant_changes() {
        let original = SourceSnapshot::new(b"unchanged source content\n".repeat(100_000));
        let right = original.len() - 23;
        for (removed, replacement) in [
            (0, b"insert".as_slice()),
            (3, b"Z".as_slice()),
            (7, b"".as_slice()),
        ] {
            let changed = original
                .replace(right, right + removed, replacement.to_vec())
                .unwrap()
                .replace(11, 11 + removed, replacement.to_vec())
                .unwrap();
            for (old, new) in [(&original, &changed), (&changed, &original)] {
                let changes = old.changed_extents(new);
                assert_eq!(changes.len(), 2, "{changes:?}");
                assert!(changes.iter().map(|(old, _)| old.len()).sum::<usize>() <= 14);
                assert!(changes.iter().map(|(_, new)| new.len()).sum::<usize>() <= 14);
                assert_diff_reconstructs(old, new);
            }
        }
    }

    #[test]
    fn sparse_source_diff_handles_shared_piece_reordering_and_repetition() {
        let original = SourceSnapshot::new(b"abcdEFGHijklMNOP".to_vec());
        let (first, rest) = split(original.root.clone(), 4);
        let (middle, last) = split(rest, 8);
        for root in [
            concat(concat(last.clone(), middle.clone()), first.clone()),
            concat(concat(first.clone(), middle.clone()), first.clone()),
        ] {
            let changed = SourceSnapshot {
                identity: SourceSnapshotIdentity::fresh(),
                root,
            };
            assert_diff_reconstructs(&original, &changed);
            assert_diff_reconstructs(&changed, &original);
        }
    }

    #[test]
    fn source_diff_reconstructs_random_edits_and_history_branches() {
        let mut source = SourceSnapshot::new((0..4096).map(|i| (i % 251) as u8).collect());
        let mut history = vec![source.clone()];
        let mut random = 0x61e7_c24b_u64;
        for step in 0..1200 {
            random = random
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let at = random as usize % (source.len() + 1);
            let end = (at + step % 19).min(source.len());
            let replacement = if step % 7 == 0 {
                // Equal bytes in a distinct allocation are a valid no-op.
                source.bytes_in(at..end).unwrap()
            } else {
                (0..step % 23)
                    .map(|i| random.rotate_right(i as u32) as u8)
                    .collect()
            };
            let next = source.replace(at, end, replacement).unwrap();
            assert_diff_reconstructs(&source, &next);
            assert_diff_reconstructs(&next, &source);
            let ancestor = history[random as usize % history.len()].clone();
            assert_diff_reconstructs(&ancestor, &next);
            assert_diff_reconstructs(&next, &ancestor);
            if step % 31 == 0 {
                history.push(next.clone());
            }
            source = if step % 47 == 0 { ancestor } else { next };
        }
        let empty = SourceSnapshot::new(Vec::new());
        assert_diff_reconstructs(&source, &empty);
        assert_diff_reconstructs(&empty, &source);
        // A source patch may start inside a UTF-8 scalar: this is a raw byte
        // artifact, and applying the complete patch must reproduce its bytes.
        let accented = SourceSnapshot::new("é".as_bytes().to_vec());
        let changed = accented.replace(1, 2, vec![0xaa]).unwrap();
        assert_eq!(accented.changed_extent(&changed), Some((1..2, 1..2)));
        assert_diff_reconstructs(&accented, &changed);
    }

    #[test]
    fn source_diff_skips_repeated_million_line_buffers_and_shared_piece_paths() {
        for line_count in [10_000, 1_000_000] {
            let row = b"same repeated line\n";
            let original = SourceSnapshot::new(row.repeat(line_count));
            for at in [0, row.len() * (line_count / 2), original.len() - row.len()] {
                for (removed, inserted) in [
                    (0, row.as_slice()),
                    (row.len(), &[][..]),
                    (1, b"S".as_slice()),
                ] {
                    let next = original
                        .replace(at, at + removed, inserted.to_vec())
                        .unwrap();
                    for (old, new) in [(&original, &next), (&next, &original)] {
                        let (extent, stats) = old.changed_extent_with_stats(new);
                        let (removed, inserted) =
                            extent.expect("the edit changes artifact length or bytes");
                        assert!(removed.len() <= row.len() * 2, "{removed:?}");
                        assert!(inserted.len() <= row.len() * 2, "{inserted:?}");
                        // Bounded backing chunks add a logarithmic tree path at each edge.
                        assert!(stats.nodes_visited < 128, "{line_count}: {stats:?}");
                        assert!(
                            stats.bytes_compared <= row.len() * 2,
                            "{line_count}: {stats:?}"
                        );
                        assert_eq!(
                            new.bytes_in(inserted.clone()).unwrap().len(),
                            inserted.len()
                        );
                    }
                    let mut original_buffers = std::collections::HashMap::new();
                    original.visit_retained_buffers(&mut |id, len| {
                        original_buffers.insert(id, len);
                    });
                    let mut next_buffers = std::collections::HashMap::new();
                    next.visit_retained_buffers(&mut |id, len| {
                        next_buffers.insert(id, len);
                    });
                    assert!(original_buffers
                        .iter()
                        .all(|(id, len)| next_buffers.get(id) == Some(len)));
                }
            }
        }
        for piece_count in [1024, 16_384] {
            let mut original = SourceSnapshot::new(Vec::new());
            for _ in 0..piece_count {
                original = original
                    .replace(original.len(), original.len(), b"same\n".to_vec())
                    .unwrap();
            }
            for at in [0, original.len() / 2, original.len() - 13] {
                let next = original.replace(at, at + 10, b"same\n".to_vec()).unwrap();
                for (old, new) in [(&original, &next), (&next, &original)] {
                    let (_, stats) = old.changed_extent_with_stats(new);
                    assert!(stats.nodes_visited < 256, "{piece_count}: {stats:?}");
                    assert!(stats.bytes_compared < 40, "{piece_count}: {stats:?}");
                    assert_diff_reconstructs(old, new);
                }
            }
            let (extent, stats) = original.changed_extent_with_stats(&original.clone());
            assert!(extent.is_none());
            assert_eq!(
                stats,
                SourceDiffStats {
                    nodes_visited: 1,
                    bytes_compared: 0
                }
            );
        }
    }

    #[test]
    fn code_history_navigation_materializes_only_the_changed_source_range() {
        use crate::document::{Document, Encoding, Format, HistoryNavigationRequest, ModelRequest};

        let mut document = Document::from_bytes(
            b"same repeated line\n".repeat(10_000),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap();
        document.replace(0..0, "x").unwrap();
        let baseline = document.state().source.clone();
        let baseline_text = document.projection().clone();
        for at in [1, 19 * 5000 + 1, document.source_byte_len() - 1] {
            document.replace(at..at, "λ\n").unwrap();
            let edited = document.state().source.clone();
            let edited_text = document.projection().clone();
            for (navigation, target) in [
                (HistoryNavigationRequest::Undo, &baseline),
                (HistoryNavigationRequest::Redo, &edited),
                (HistoryNavigationRequest::Undo, &baseline),
            ] {
                let before = document.state().source.clone();
                let before_text = document.projection().clone();
                let prepared = document
                    .prepare_model_request(ModelRequest::NavigateHistory {
                        document: document.id(),
                        revision: document.revision(),
                        navigation,
                    })
                    .unwrap();
                let patches = prepared.summary().source_patches();
                assert_eq!(patches.len(), 1);
                assert!(patches[0].range().len() <= 3);
                assert!(patches[0].replacement().len() <= 3);
                let reconstructed = before
                    .replace(
                        patches[0].range().start,
                        patches[0].range().end,
                        patches[0].replacement().to_vec(),
                    )
                    .unwrap();
                assert_eq!(reconstructed.bytes(), target.bytes());
                let changes = prepared.summary().formatted_splices();
                assert_eq!(changes.len(), 1);
                assert!(changes[0].old_range().len() <= 3);
                assert!(changes[0].inserted_len() <= 3);
                assert!(!before_text.compatibility_text_is_materialized());
                document.commit_model_transaction(prepared).unwrap();
                assert!(!document.projection().compatibility_text_is_materialized());
                assert_eq!(document.state().source.identity(), target.identity());
            }
            assert!(!baseline_text.compatibility_text_is_materialized());
            assert!(!edited_text.compatibility_text_is_materialized());
        }
    }
}
