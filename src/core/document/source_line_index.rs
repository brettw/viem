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
    height: u32,
    kind: NodeKind,
}

#[derive(Debug, Eq, PartialEq)]
enum NodeKind {
    Leaf(Arc<[usize]>),
    Branch { left: Arc<Node>, right: Arc<Node> },
}

impl SourceHardLineIndex {
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        fn visit(node: &Arc<Node>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
            visitor.arc(node, |visitor| match &node.kind {
                NodeKind::Leaf(lengths) => visitor.arc(lengths, |_| {}),
                NodeKind::Branch { left, right } => {
                    visit(left, visitor);
                    visit(right, visitor);
                }
            });
        }
        visit(&self.root, visitor);
    }

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
        if lines.start >= lines.end || lines.end > self.len() || ranges.is_empty() {
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
        let root = if ranges.len() == lines.len() {
            replace_lengths(&self.root, 0, &lines, &lengths, &mut stats)
        } else {
            let (prefix, tail) = split_lines(Some(self.root.clone()), lines.start, &mut stats);
            let (_, suffix) = split_lines(tail, lines.len(), &mut stats);
            let leaves = lengths.chunks(LEAF_LINES).map(Node::leaf).collect::<Vec<_>>();
            stats.records_copied += lengths.len();
            stats.leaves_copied += leaves.len();
            stats.nodes_copied += leaves.len();
            join_lines(join_lines(prefix, build_balanced(&leaves, 0..leaves.len()), &mut stats), suffix, &mut stats)?
        };
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
            height: 1,
            kind: NodeKind::Leaf(lengths.into()),
        })
    }

    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            height: 1 + left.height.max(right.height),
            line_count: left.line_count + right.line_count,
            byte_len: left
                .byte_len
                .checked_add(right.byte_len)
                .expect("validated source hard-line aggregate is representable"),
            kind: NodeKind::Branch { left, right },
        })
    }
}

fn join_lines(left: Option<Arc<Node>>, right: Option<Arc<Node>>, stats: &mut SourceHardLineSpliceStats) -> Option<Arc<Node>> {
    let (left, right) = match (left, right) { (None, r) => return r, (l, None) => return l, (Some(l), Some(r)) => (l, r) };
    stats.nodes_visited += 1;
    let result = if left.height > right.height + 1 {
        let NodeKind::Branch { left: a, right: b } = &left.kind else { unreachable!() };
        balance_lines(a.clone(), join_lines(Some(b.clone()), Some(right), stats).unwrap(), stats)
    } else if right.height > left.height + 1 {
        let NodeKind::Branch { left: a, right: b } = &right.kind else { unreachable!() };
        balance_lines(join_lines(Some(left), Some(a.clone()), stats).unwrap(), b.clone(), stats)
    } else { stats.nodes_copied += 1; Node::branch(left, right) };
    Some(result)
}

fn balance_lines(left: Arc<Node>, right: Arc<Node>, stats: &mut SourceHardLineSpliceStats) -> Arc<Node> {
    stats.nodes_copied += 1;
    if left.height > right.height + 1 {
        let NodeKind::Branch { left: a, right: b } = &left.kind else { unreachable!() };
        if a.height >= b.height { stats.nodes_copied += 1; return Node::branch(a.clone(), Node::branch(b.clone(), right)); }
        let NodeKind::Branch { left: c, right: d } = &b.kind else { unreachable!() };
        stats.nodes_copied += 2;
        return Node::branch(Node::branch(a.clone(), c.clone()), Node::branch(d.clone(), right));
    }
    if right.height > left.height + 1 {
        let NodeKind::Branch { left: a, right: b } = &right.kind else { unreachable!() };
        if b.height >= a.height { stats.nodes_copied += 1; return Node::branch(Node::branch(left, a.clone()), b.clone()); }
        let NodeKind::Branch { left: c, right: d } = &a.kind else { unreachable!() };
        stats.nodes_copied += 2;
        return Node::branch(Node::branch(left, c.clone()), Node::branch(d.clone(), b.clone()));
    }
    Node::branch(left, right)
}

fn split_lines(node: Option<Arc<Node>>, at: usize, stats: &mut SourceHardLineSpliceStats) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
    let Some(node) = node else { return (None, None); };
    stats.nodes_visited += 1;
    if at == 0 { return (None, Some(node)); }
    if at == node.line_count { return (Some(node), None); }
    match &node.kind {
        NodeKind::Leaf(values) => {
            stats.nodes_copied += 2; stats.leaves_copied += 2; stats.records_copied += values.len();
            (Some(Node::leaf(&values[..at])), Some(Node::leaf(&values[at..])))
        }
        NodeKind::Branch { left, right } if at < left.line_count => {
            let (a, b) = split_lines(Some(left.clone()), at, stats);
            (a, join_lines(b, Some(right.clone()), stats))
        }
        NodeKind::Branch { left, right } => {
            let (a, b) = split_lines(Some(right.clone()), at - left.line_count, stats);
            (join_lines(Some(left.clone()), a, stats), b)
        }
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
                && node.height == 1
                && node.line_count == lengths.len()
                && node.byte_len == bytes)
                .then_some((lengths.len(), bytes, 1, 1))
        }
        NodeKind::Branch { left, right } => {
            let (left_lines, left_bytes, left_leaves, left_height) = validate_node(left)?;
            let (right_lines, right_bytes, right_leaves, right_height) = validate_node(right)?;
            (left_height.abs_diff(right_height) <= 1
                && node.height as usize == left_height.max(right_height)+1
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
    fn variable_line_splices_preserve_balance_identity_and_logarithmic_work() {
        let mut lengths=vec![3usize;10_000];
        let mut index=SourceHardLineIndex::new((0..lengths.len()).map(|i|3*i..3*i+3).collect()).unwrap();
        let mut seed=7u64;
        for iteration in 0..1000 {
            seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at=seed as usize%lengths.len();
            let removed=(1+iteration%7).min(lengths.len()-at);
            let added=1+iteration%11;
            let base=index.get(at).unwrap().start;
            let replacement=(0..added).map(|i|base+i*5..base+i*5+5).collect::<Vec<_>>();
            let (next,stats)=index.replace_ranges_with_stats(at..at+removed,&replacement).unwrap();
            assert!(stats.nodes_copied<200,"{stats:?}");
            assert!(stats.records_copied<600,"{stats:?}");
            lengths.splice(at..at+removed,std::iter::repeat_n(5,added));
            assert!(next.invariant_holds(),"splice {iteration}");
            assert_eq!(next.len(),lengths.len());
            let mut offset=0;
            for (line,length) in lengths.iter().enumerate() {assert_eq!(next.get(line),Some(offset..offset+length));offset+=length;}
            index=next;
        }
    }

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
