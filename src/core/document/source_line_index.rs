//! Persistent source-byte extents for physical hard lines.
//!
//! Line lengths, rather than absolute suffix offsets, are stored under a
//! prefix-sum tree. A local edit path-copies only the leaves containing changed
//! lines; every later line moves implicitly through the changed aggregate.

use std::ops::Range;
use std::sync::Arc;

const LEAF_LINES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceHardLineIndex {
    content_start: usize,
    root: Arc<Node>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceHardLineSpliceStats {
    pub(crate) nodes_visited: usize,
    pub(crate) nodes_copied: usize,
    pub(crate) leaves_copied: usize,
    pub(crate) records_copied: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct Node {
    line_count: usize,
    byte_len: usize,
    kind: NodeKind,
}

#[derive(Debug, Eq, PartialEq)]
enum NodeKind {
    Leaf(Arc<[usize]>),
    Branch { left: Arc<Node>, right: Arc<Node> },
}

impl SourceHardLineIndex {
    pub(crate) fn new(ranges: Vec<Range<usize>>) -> Option<Self> {
        let content_start = ranges.first()?.start;
        let mut expected = content_start;
        let mut lengths = Vec::with_capacity(ranges.len());
        for range in ranges {
            if range.start != expected || range.start > range.end {
                return None;
            }
            lengths.push(range.len());
            expected = range.end;
        }
        let leaves = lengths
            .chunks(LEAF_LINES)
            .map(Node::leaf)
            .collect::<Vec<_>>();
        let root = build_balanced(&leaves, 0..leaves.len())?;
        Some(Self {
            content_start,
            root,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.root.line_count
    }

    pub(crate) fn get(&self, line: usize) -> Option<Range<usize>> {
        let (relative_start, length) = lookup(&self.root, line, 0)?;
        let start = self.content_start.checked_add(relative_start)?;
        Some(start..start.checked_add(length)?)
    }

    pub(crate) fn source_end(&self) -> usize {
        self.content_start
            .checked_add(self.root.byte_len)
            .expect("validated source hard-line aggregate is representable")
    }

    /// Locate a physical source line through byte aggregates without binary
    /// searching repeatedly through the ordinal lookup API.
    pub(crate) fn line_at_offset(&self, offset: usize) -> Option<usize> {
        let mut remaining = offset.checked_sub(self.content_start)?;
        if remaining > self.root.byte_len {
            return None;
        }
        let mut node = self.root.as_ref();
        let mut first_line = 0;
        loop {
            match &node.kind {
                NodeKind::Leaf(lengths) => {
                    for (index, length) in lengths.iter().enumerate() {
                        if remaining < *length || index + 1 == lengths.len() {
                            return Some(first_line + index);
                        }
                        remaining -= length;
                    }
                    return None;
                }
                NodeKind::Branch { left, right } => {
                    if remaining < left.byte_len {
                        node = left;
                    } else {
                        remaining -= left.byte_len;
                        first_line += left.line_count;
                        node = right;
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn replace_ranges(
        &self,
        lines: Range<usize>,
        ranges: &[Range<usize>],
    ) -> Option<Self> {
        self.replace_ranges_with_stats(lines, ranges)
            .map(|(index, _)| index)
    }

    pub(crate) fn replace_ranges_with_stats(
        &self,
        lines: Range<usize>,
        ranges: &[Range<usize>],
    ) -> Option<(Self, SourceHardLineSpliceStats)> {
        if lines.start >= lines.end || lines.end > self.len() || ranges.len() != lines.len() {
            return None;
        }
        let expected_start = self.get(lines.start)?.start;
        let mut expected = expected_start;
        let mut lengths = Vec::with_capacity(ranges.len());
        for range in ranges {
            if range.start != expected || range.start > range.end {
                return None;
            }
            lengths.push(range.len());
            expected = range.end;
        }
        let mut stats = SourceHardLineSpliceStats::default();
        let root = replace_lengths(&self.root, 0, &lines, &lengths, &mut stats);
        Some((
            Self {
                content_start: self.content_start,
                root,
            },
            stats,
        ))
    }

    #[cfg(test)]
    pub(crate) fn shared_leaf_count_with(&self, other: &Self) -> usize {
        shared_leaf_count(&self.root, &other.root)
    }

    #[cfg(test)]
    pub(crate) fn leaf_count(&self) -> usize {
        leaf_count(&self.root)
    }

    #[cfg(test)]
    pub(crate) fn invariant_holds(&self) -> bool {
        validate_node(&self.root).is_some_and(|(lines, bytes, _, _)| {
            lines == self.len()
                && bytes == self.root.byte_len
                && self.source_end() == self.content_start + bytes
        })
    }
}

impl Node {
    fn leaf(lengths: &[usize]) -> Arc<Self> {
        Arc::new(Self {
            line_count: lengths.len(),
            byte_len: lengths.iter().sum(),
            kind: NodeKind::Leaf(lengths.into()),
        })
    }

    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            line_count: left.line_count + right.line_count,
            byte_len: left
                .byte_len
                .checked_add(right.byte_len)
                .expect("validated source hard-line aggregate is representable"),
            kind: NodeKind::Branch { left, right },
        })
    }
}

fn build_balanced(leaves: &[Arc<Node>], range: Range<usize>) -> Option<Arc<Node>> {
    match range.len() {
        0 => None,
        1 => Some(leaves[range.start].clone()),
        _ => {
            let middle = range.start + range.len() / 2;
            Some(Node::branch(
                build_balanced(leaves, range.start..middle)?,
                build_balanced(leaves, middle..range.end)?,
            ))
        }
    }
}

fn lookup(node: &Node, line: usize, prefix: usize) -> Option<(usize, usize)> {
    if line >= node.line_count {
        return None;
    }
    match &node.kind {
        NodeKind::Leaf(lengths) => {
            let preceding = lengths[..line]
                .iter()
                .try_fold(prefix, |sum, length| sum.checked_add(*length))?;
            Some((preceding, lengths[line]))
        }
        NodeKind::Branch { left, right } => {
            if line < left.line_count {
                lookup(left, line, prefix)
            } else {
                lookup(
                    right,
                    line - left.line_count,
                    prefix.checked_add(left.byte_len)?,
                )
            }
        }
    }
}

fn replace_lengths(
    node: &Arc<Node>,
    base_line: usize,
    target: &Range<usize>,
    replacements: &[usize],
    stats: &mut SourceHardLineSpliceStats,
) -> Arc<Node> {
    stats.nodes_visited = stats.nodes_visited.saturating_add(1);
    let node_end = base_line + node.line_count;
    if node_end <= target.start || base_line >= target.end {
        return node.clone();
    }
    match &node.kind {
        NodeKind::Leaf(current) => {
            let overlap_start = base_line.max(target.start);
            let overlap_end = node_end.min(target.end);
            let mut lengths = current.to_vec();
            let destination = overlap_start - base_line..overlap_end - base_line;
            let source = overlap_start - target.start..overlap_end - target.start;
            lengths[destination].copy_from_slice(&replacements[source]);
            stats.nodes_copied = stats.nodes_copied.saturating_add(1);
            stats.leaves_copied = stats.leaves_copied.saturating_add(1);
            stats.records_copied = stats.records_copied.saturating_add(lengths.len());
            Node::leaf(&lengths)
        }
        NodeKind::Branch { left, right } => {
            let left_after = replace_lengths(left, base_line, target, replacements, stats);
            let right_after = replace_lengths(
                right,
                base_line + left.line_count,
                target,
                replacements,
                stats,
            );
            if Arc::ptr_eq(&left_after, left) && Arc::ptr_eq(&right_after, right) {
                node.clone()
            } else {
                stats.nodes_copied = stats.nodes_copied.saturating_add(1);
                Node::branch(left_after, right_after)
            }
        }
    }
}

#[cfg(test)]
fn shared_leaf_count(left: &Arc<Node>, right: &Arc<Node>) -> usize {
    if Arc::ptr_eq(left, right) {
        return leaf_count(left);
    }
    match (&left.kind, &right.kind) {
        (
            NodeKind::Branch {
                left: left_left,
                right: left_right,
            },
            NodeKind::Branch {
                left: right_left,
                right: right_right,
            },
        ) if left_left.line_count == right_left.line_count => {
            shared_leaf_count(left_left, right_left) + shared_leaf_count(left_right, right_right)
        }
        _ => 0,
    }
}

#[cfg(test)]
fn leaf_count(node: &Node) -> usize {
    match &node.kind {
        NodeKind::Leaf(_) => 1,
        NodeKind::Branch { left, right } => leaf_count(left) + leaf_count(right),
    }
}

#[cfg(test)]
fn validate_node(node: &Node) -> Option<(usize, usize, usize, usize)> {
    match &node.kind {
        NodeKind::Leaf(lengths) => {
            let bytes = lengths
                .iter()
                .try_fold(0usize, |sum, length| sum.checked_add(*length))?;
            (!lengths.is_empty()
                && lengths.len() <= LEAF_LINES
                && node.line_count == lengths.len()
                && node.byte_len == bytes)
                .then_some((lengths.len(), bytes, 1, 1))
        }
        NodeKind::Branch { left, right } => {
            let (left_lines, left_bytes, left_leaves, left_height) = validate_node(left)?;
            let (right_lines, right_bytes, right_leaves, right_height) = validate_node(right)?;
            (left_height.abs_diff(right_height) <= 1
                && node.line_count == left_lines + right_lines
                && node.byte_len == left_bytes.checked_add(right_bytes)?)
            .then_some((
                node.line_count,
                node.byte_len,
                left_leaves + right_leaves,
                left_height.max(right_height) + 1,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_length_replacement_shifts_suffix_without_rebuilding_it() {
        let line_count = 1_000_000usize;
        let ranges = (0..line_count)
            .scan(3usize, |start, line| {
                let length = line % 7 + 1;
                let range = *start..*start + length;
                *start = range.end;
                Some(range)
            })
            .collect::<Vec<_>>();
        let original = SourceHardLineIndex::new(ranges).unwrap();
        let target = line_count / 2;
        let old = original.get(target).unwrap();
        let replacement = old.start..old.end + 9;
        let changed = original
            .replace_ranges(target..target + 1, std::slice::from_ref(&replacement))
            .unwrap();
        assert!(original.invariant_holds());
        assert!(changed.invariant_holds());
        assert_eq!(changed.get(target - 1), original.get(target - 1));
        assert_eq!(
            changed.get(target + 1).unwrap().start,
            original.get(target + 1).unwrap().start + 9
        );
        assert_eq!(changed.leaf_count(), original.leaf_count());
        assert_eq!(
            changed.shared_leaf_count_with(&original),
            original.leaf_count() - 1
        );
    }
}
