//! Persistent byte storage for authoritative source snapshots.
//!
//! Leaves reference immutable byte buffers.  Splitting a leaf creates two
//! slices of the same buffer and concatenation builds a balanced persistent
//! rope, so a revision shares every untouched byte with its predecessor.

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::Arc;

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
    root: Option<Arc<Node>>,
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
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        fn visit(node: &Arc<Node>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
            visitor.arc(node, |visitor| match node.as_ref() {
                Node::Leaf(piece) => visitor.arc(&piece.bytes, |_| {}),
                Node::Branch { left, right, .. } => {
                    visit(left, visitor);
                    visit(right, visitor);
                }
            });
        }
        if let Some(root) = &self.root { visit(root, visitor); }
    }

    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        let root = if bytes.is_empty() {
            None
        } else {
            let len = bytes.len();
            Some(Arc::new(Node::Leaf(Piece {
                bytes: Arc::from(bytes),
                start: 0,
                len,
            })))
        };
        Self { root }
    }

    pub(crate) fn len(&self) -> usize {
        self.root.as_deref().map(Node::len).unwrap_or(0)
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            root.append_to(&mut result);
        }
        result
    }

    /// Materialize only one validated byte range. Splitting a persistent rope
    /// clones the logarithmic path while retaining its immutable buffers, so a
    /// ranged write does not first flatten an unrelated multi-megabyte source.
    pub(crate) fn bytes_in(&self, range: Range<usize>) -> Option<Vec<u8>> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }
        let (_, suffix) = split(self.root.clone(), range.start);
        let (selected, _) = split(suffix, range.end - range.start);
        let selected = Self { root: selected };
        Some(selected.bytes())
    }

    pub(crate) fn artifact_digest(&self) -> SourceArtifactDigest {
        // Digesting is currently needed only when establishing a save point,
        // not on every edit.  Keeping the implementation here makes the
        // physical byte sequence, rather than a derived projection, the sole
        // authority for the result.
        SourceArtifactDigest::from_bytes(&self.bytes())
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
        let inserted = if replacement.is_empty() {
            None
        } else {
            let len = replacement.len();
            Some(Arc::new(Node::Leaf(Piece {
                bytes: Arc::from(replacement),
                start: 0,
                len,
            })))
        };
        Some(Self {
            root: concat(concat(before, inserted), after),
        })
    }

    #[cfg(test)]
    fn height(&self) -> u8 {
        self.root.as_deref().map(Node::height).unwrap_or(0)
    }
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

// Compact SHA-256 implementation used to retain an exact physical-artifact
// fingerprint without adding a persistence dependency to the portable core.
fn sha256(input: &[u8]) -> [u8; 32] {
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

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let block_count = (input.len() + 9).div_ceil(64);
    let mut padded = vec![0_u8; block_count * 64];
    padded[..input.len()].copy_from_slice(input);
    padded[input.len()] = 0x80;
    let length_offset = padded.len() - 8;
    padded[length_offset..].copy_from_slice(&bit_len.to_be_bytes());

    let mut state = INITIAL;
    for block in padded.chunks_exact(64) {
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

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
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
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut output = [0_u8; 32];
    for (chunk, word) in output.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    output
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
        assert!(Arc::ptr_eq(
            original.root.as_ref().unwrap(),
            unchanged.root.as_ref().unwrap()
        ));
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
    fn large_source_shapes_patch_by_sharing_the_original_allocation() {
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
            let original_identity = *original_buffers.keys().next().unwrap();
            let mut changed_buffers = std::collections::HashMap::new();
            changed.visit_retained_buffers(&mut |identity, length| {
                changed_buffers.insert(identity, length);
            });
            assert_eq!(changed_buffers.get(&original_identity), Some(&2_000_000));
            assert_eq!(changed_buffers.len(), 2);
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
            assert_eq!(
                retained
                    .values()
                    .filter(|length| **length == SOURCE_BYTES)
                    .count(),
                1
            );
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
}
