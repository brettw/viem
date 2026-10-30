//! Persistent indentation summaries, independent of text, fonts and geometry.
//!
//! Parent links are implicit: a line's anchor is the nearest preceding eligible
//! line with no greater indentation. A subtree minimum answers that predecessor
//! query without scanning intervening lines. An indentation edit can change a
//! million parent links, but changes only two tree paths here.
//!
//! Leaves pack 128 column counts and an eligibility bitset. Adjacent underfull
//! leaves are repaired at splice boundaries, so editing does not gradually turn
//! the index into a node per line. Snapshots share immutable AVL nodes.

use std::ops::Range;
use std::sync::Arc;

const BLOCK_LINES: usize = 128;
const MIN_BLOCK_LINES: usize = BLOCK_LINES / 2;

/// Logical indentation, measured in the caller's character/tab columns.
/// Eligibility is a policy hook; the code policy excludes blank lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineIndentation {
    pub columns: usize,
    pub anchor_eligible: bool,
}

impl LineIndentation {
    pub fn new(columns: usize) -> Self {
        Self {
            columns,
            anchor_eligible: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndentAnchorIndexError {
    RangeOutsideIndex {
        start: usize,
        end: usize,
        line_count: usize,
    },
    LineCountOverflow,
}

/// Structural counters, rather than elapsed-time assertions, expose locality.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndentAnchorWork {
    pub nodes_visited: usize,
    pub nodes_created: usize,
    pub lines_examined: usize,
    pub lines_copied: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IndentAnchorStatistics {
    pub line_count: usize,
    pub block_count: usize,
    pub tree_height: usize,
    /// Live node and column payload bytes, excluding allocator headers and Arc
    /// control blocks. Shared snapshots must not sum this as unique ownership.
    pub payload_bytes: usize,
}

#[derive(Clone, Debug, Default)]
pub struct IndentAnchorIndex {
    root: Link,
}

type Link = Option<Arc<Node>>;

#[derive(Debug)]
struct Node {
    summary: Summary,
    content: Content,
}

#[derive(Clone, Copy, Debug)]
struct Summary {
    lines: usize,
    blocks: usize,
    height: usize,
    min_eligible: Option<usize>,
    min_all: usize,
}

#[derive(Debug)]
enum Content {
    Block {
        columns: Box<[usize]>,
        eligible: u128,
    },
    Branch {
        left: Arc<Node>,
        right: Arc<Node>,
    },
}

impl IndentAnchorIndex {
    pub(crate) fn shares_root_with(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    pub fn from_lines(lines: impl IntoIterator<Item = LineIndentation>) -> Self {
        let mut work = IndentAnchorWork::default();
        Self {
            root: build(lines, &mut work),
        }
    }

    pub fn len(&self) -> usize {
        line_count(&self.root)
    }
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn statistics(&self) -> IndentAnchorStatistics {
        let Some(root) = &self.root else {
            return IndentAnchorStatistics::default();
        };
        let summary = root.summary;
        IndentAnchorStatistics {
            line_count: summary.lines,
            block_count: summary.blocks,
            tree_height: summary.height,
            payload_bytes: (summary.blocks * 2 - 1) * std::mem::size_of::<Node>()
                + summary.lines * std::mem::size_of::<usize>(),
        }
    }

    pub fn line(&self, line: usize) -> Option<LineIndentation> {
        self.line_with_work(line, &mut IndentAnchorWork::default())
    }

    fn line_with_work(
        &self,
        mut line: usize,
        work: &mut IndentAnchorWork,
    ) -> Option<LineIndentation> {
        if line >= self.len() {
            return None;
        }
        let mut node = self.root.as_deref()?;
        loop {
            work.nodes_visited += 1;
            match &node.content {
                Content::Block { columns, eligible } => {
                    work.lines_examined += 1;
                    return Some(LineIndentation {
                        columns: columns[line],
                        anchor_eligible: eligible & (1 << line) != 0,
                    });
                }
                Content::Branch { left, right } => {
                    if line < left.summary.lines {
                        node = left;
                    } else {
                        line -= left.summary.lines;
                        node = right;
                    }
                }
            }
        }
    }

    pub fn anchor(&self, line: usize) -> Option<usize> {
        self.anchor_with_work(line, &mut IndentAnchorWork::default())
    }

    pub fn anchor_with_work(&self, line: usize, work: &mut IndentAnchorWork) -> Option<usize> {
        let indentation = self.line_with_work(line, work)?;
        self.preceding_with_max_and_work(line, indentation.columns, work)
    }

    /// Nearest eligible line strictly before `before` whose indentation is at
    /// most `maximum`. A strict indentation query uses `columns.checked_sub(1)`.
    /// Out-of-range positions are rejected instead of being clamped.
    pub fn preceding_with_max(&self, before: usize, maximum: usize) -> Option<usize> {
        self.preceding_with_max_and_work(before, maximum, &mut IndentAnchorWork::default())
    }

    pub fn preceding_with_max_and_work(
        &self,
        before: usize,
        maximum: usize,
        work: &mut IndentAnchorWork,
    ) -> Option<usize> {
        if before > self.len() {
            return None;
        }
        search(
            self.root.as_deref()?,
            0,
            &(0..before),
            maximum,
            true,
            true,
            work,
        )
    }

    /// First eligible line at or after `from` with indentation at most maximum.
    pub fn following_with_max(&self, from: usize, maximum: usize) -> Option<usize> {
        self.following_with_max_and_work(from, maximum, &mut IndentAnchorWork::default())
    }

    pub fn following_with_max_and_work(
        &self,
        from: usize,
        maximum: usize,
        work: &mut IndentAnchorWork,
    ) -> Option<usize> {
        if from > self.len() {
            return None;
        }
        search(
            self.root.as_deref()?,
            0,
            &(from..self.len()),
            maximum,
            true,
            false,
            work,
        )
    }

    /// Contiguous span containing all transitive dependents. Without excluded
    /// lines this is exact. An excluded line with less indentation can be a
    /// hole inside the span, but cannot end it.
    /// Equal-indent lines remain dependent, so only a *strictly* smaller
    /// eligible line terminates the span. No dependents are enumerated here.
    pub fn dependent_range(&self, line: usize) -> Option<Range<usize>> {
        let indentation = self.line(line)?;
        let start = line + 1;
        if !indentation.anchor_eligible {
            return Some(start..start);
        }
        let end = indentation
            .columns
            .checked_sub(1)
            .and_then(|maximum| self.following_with_max(start, maximum))
            .unwrap_or(self.len());
        Some(start..end)
    }

    /// Enumerate immediate children without materializing an adjacency list.
    /// Each yielded eligible child lowers the next search's threshold; its
    /// entire more-indented subtree is skipped. Cost is O((children + 1) log n)
    /// with bounded leaf scans when no lines are excluded. Excluded low-indent
    /// lines, such as blank lines, are additionally visited.
    pub fn direct_dependents(&self, line: usize) -> IndentDependents<'_> {
        let indentation = self.line(line).filter(|line| line.anchor_eligible);
        IndentDependents {
            index: self,
            next: if indentation.is_some() {
                line + 1
            } else {
                self.len()
            },
            lower: indentation.map_or(0, |line| line.columns),
            maximum: Some(usize::MAX),
        }
    }

    /// Replace line summaries atomically. Work copies bounded boundary blocks
    /// and O(log n) nodes plus the replacement; following ordinals never shift
    /// in storage, and changed implicit parents require no rewrites.
    pub fn splice(
        &mut self,
        removed: Range<usize>,
        replacement: impl IntoIterator<Item = LineIndentation>,
    ) -> Result<(), IndentAnchorIndexError> {
        self.splice_with_work(removed, replacement, &mut IndentAnchorWork::default())
    }

    pub fn splice_with_work(
        &mut self,
        removed: Range<usize>,
        replacement: impl IntoIterator<Item = LineIndentation>,
        work: &mut IndentAnchorWork,
    ) -> Result<(), IndentAnchorIndexError> {
        if removed.start > removed.end || removed.end > self.len() {
            return Err(IndentAnchorIndexError::RangeOutsideIndex {
                start: removed.start,
                end: removed.end,
                line_count: self.len(),
            });
        }
        let inserted = build(replacement, work);
        (self.len() - removed.len())
            .checked_add(line_count(&inserted))
            .ok_or(IndentAnchorIndexError::LineCountOverflow)?;
        if removed.is_empty() && inserted.is_none() {
            return Ok(());
        }
        let (left, tail) = split(self.root.clone(), removed.start, work);
        let (_, right) = split(tail, removed.len(), work);
        let next = concat(left, inserted, work);
        self.root = concat(next, right, work);
        Ok(())
    }
}

pub struct IndentDependents<'a> {
    index: &'a IndentAnchorIndex,
    next: usize,
    lower: usize,
    maximum: Option<usize>,
}

impl Iterator for IndentDependents<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            let maximum = self.maximum?;
            let found = search(
                self.index.root.as_deref()?,
                0,
                &(self.next..self.index.len()),
                maximum,
                false,
                false,
                &mut IndentAnchorWork::default(),
            )?;
            let indentation = self.index.line(found)?;
            self.next = found + 1;
            if indentation.anchor_eligible {
                self.maximum = indentation.columns.checked_sub(1);
                if indentation.columns < self.lower {
                    self.maximum = None;
                    return None;
                }
            }
            if indentation.columns >= self.lower {
                return Some(found);
            }
        }
    }
}

fn line_count(link: &Link) -> usize {
    link.as_ref().map_or(0, |node| node.summary.lines)
}

fn block(lines: &[LineIndentation], work: &mut IndentAnchorWork) -> Arc<Node> {
    debug_assert!(!lines.is_empty() && lines.len() <= BLOCK_LINES);
    work.nodes_created += 1;
    work.lines_copied += lines.len();
    let mut eligible = 0u128;
    let mut min_eligible = None;
    let mut min_all = usize::MAX;
    let columns = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            min_all = min_all.min(line.columns);
            if line.anchor_eligible {
                eligible |= 1 << i;
                min_eligible = Some(min_eligible.unwrap_or(usize::MAX).min(line.columns));
            }
            line.columns
        })
        .collect();
    Arc::new(Node {
        summary: Summary {
            lines: lines.len(),
            blocks: 1,
            height: 1,
            min_eligible,
            min_all,
        },
        content: Content::Block { columns, eligible },
    })
}

fn branch(left: Arc<Node>, right: Arc<Node>, work: &mut IndentAnchorWork) -> Arc<Node> {
    work.nodes_created += 1;
    Arc::new(Node {
        summary: Summary {
            lines: left.summary.lines + right.summary.lines,
            blocks: left.summary.blocks + right.summary.blocks,
            height: left.summary.height.max(right.summary.height) + 1,
            min_eligible: match (left.summary.min_eligible, right.summary.min_eligible) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, None) | (None, a) => a,
            },
            min_all: left.summary.min_all.min(right.summary.min_all),
        },
        content: Content::Branch { left, right },
    })
}

fn build(lines: impl IntoIterator<Item = LineIndentation>, work: &mut IndentAnchorWork) -> Link {
    let mut blocks = Vec::new();
    let mut pending = Vec::with_capacity(BLOCK_LINES);
    for line in lines {
        pending.push(line);
        if pending.len() == BLOCK_LINES {
            blocks.push(block(&pending, work));
            pending.clear();
        }
    }
    if !pending.is_empty() {
        blocks.push(block(&pending, work));
    }
    fn balanced(blocks: &[Arc<Node>], work: &mut IndentAnchorWork) -> Link {
        match blocks.len() {
            0 => None,
            1 => Some(blocks[0].clone()),
            count => {
                let middle = count / 2;
                Some(branch(
                    balanced(&blocks[..middle], work).unwrap(),
                    balanced(&blocks[middle..], work).unwrap(),
                    work,
                ))
            }
        }
    }
    balanced(&blocks, work)
}

fn children(node: &Node) -> (&Arc<Node>, &Arc<Node>) {
    let Content::Branch { left, right } = &node.content else {
        unreachable!("branch height");
    };
    (left, right)
}

fn join(left: Link, right: Link, work: &mut IndentAnchorWork) -> Link {
    let (Some(left), Some(right)) = (left.clone(), right.clone()) else {
        return left.or(right);
    };
    work.nodes_visited += 1;
    if left.summary.height > right.summary.height + 1 {
        let (a, b) = children(&left);
        let joined = join(Some(b.clone()), Some(right), work).unwrap();
        Some(balance(a.clone(), joined, work))
    } else if right.summary.height > left.summary.height + 1 {
        let (a, b) = children(&right);
        let joined = join(Some(left), Some(a.clone()), work).unwrap();
        Some(balance(joined, b.clone(), work))
    } else {
        Some(branch(left, right, work))
    }
}

fn balance(left: Arc<Node>, right: Arc<Node>, work: &mut IndentAnchorWork) -> Arc<Node> {
    if left.summary.height > right.summary.height + 1 {
        let (a, b) = children(&left);
        if a.summary.height >= b.summary.height {
            let nested = branch(b.clone(), right, work);
            branch(a.clone(), nested, work)
        } else {
            let (c, d) = children(b);
            let first = branch(a.clone(), c.clone(), work);
            let last = branch(d.clone(), right, work);
            branch(first, last, work)
        }
    } else if right.summary.height > left.summary.height + 1 {
        let (a, b) = children(&right);
        if b.summary.height >= a.summary.height {
            let nested = branch(left, a.clone(), work);
            branch(nested, b.clone(), work)
        } else {
            let (c, d) = children(a);
            let first = branch(left, c.clone(), work);
            let last = branch(d.clone(), b.clone(), work);
            branch(first, last, work)
        }
    } else {
        branch(left, right, work)
    }
}

fn block_lines(node: &Node) -> Vec<LineIndentation> {
    let Content::Block { columns, eligible } = &node.content else {
        unreachable!("block");
    };
    columns
        .iter()
        .enumerate()
        .map(|(i, &columns)| LineIndentation {
            columns,
            anchor_eligible: eligible & (1 << i) != 0,
        })
        .collect()
}

fn split(root: Link, at: usize, work: &mut IndentAnchorWork) -> (Link, Link) {
    let Some(node) = root else {
        return (None, None);
    };
    work.nodes_visited += 1;
    if at == 0 {
        return (None, Some(node));
    }
    if at == node.summary.lines {
        return (Some(node), None);
    }
    match &node.content {
        Content::Block { .. } => {
            let lines = block_lines(&node);
            (
                Some(block(&lines[..at], work)),
                Some(block(&lines[at..], work)),
            )
        }
        Content::Branch { left, right } => {
            if at < left.summary.lines {
                let (before, after) = split(Some(left.clone()), at, work);
                (before, join(after, Some(right.clone()), work))
            } else {
                let (before, after) = split(Some(right.clone()), at - left.summary.lines, work);
                (join(Some(left.clone()), before, work), after)
            }
        }
    }
}

fn edge_block<'a>(mut node: &'a Node, last: bool, work: &mut IndentAnchorWork) -> &'a Node {
    loop {
        work.nodes_visited += 1;
        match &node.content {
            Content::Block { .. } => return node,
            Content::Branch { left, right } => node = if last { right } else { left },
        }
    }
}

fn concat(left: Link, right: Link, work: &mut IndentAnchorWork) -> Link {
    let (Some(a), Some(b)) = (left.as_deref(), right.as_deref()) else {
        return left.or(right);
    };
    let a = edge_block(a, true, work);
    let b = edge_block(b, false, work);
    if a.summary.lines >= MIN_BLOCK_LINES && b.summary.lines >= MIN_BLOCK_LINES {
        return join(left, right, work);
    }
    let (a_count, b_count) = (a.summary.lines, b.summary.lines);
    let mut lines = block_lines(a);
    lines.extend(block_lines(b));
    let left_count = line_count(&left);
    let (left, _) = split(left, left_count - a_count, work);
    let (_, right) = split(right, b_count, work);
    let middle = if lines.len() <= BLOCK_LINES {
        Some(block(&lines, work))
    } else {
        let middle = lines.len() / 2;
        let a = block(&lines[..middle], work);
        let b = block(&lines[middle..], work);
        Some(branch(a, b, work))
    };
    let joined = join(left, middle, work);
    join(joined, right, work)
}

fn search(
    node: &Node,
    start: usize,
    range: &Range<usize>,
    maximum: usize,
    eligible_only: bool,
    reverse: bool,
    work: &mut IndentAnchorWork,
) -> Option<usize> {
    work.nodes_visited += 1;
    if start >= range.end || start + node.summary.lines <= range.start {
        return None;
    }
    let minimum = if eligible_only {
        node.summary.min_eligible
    } else {
        Some(node.summary.min_all)
    };
    if minimum.is_none_or(|minimum| minimum > maximum) {
        return None;
    }
    match &node.content {
        Content::Block { columns, eligible } => {
            let lo = range.start.saturating_sub(start);
            let hi = (range.end - start).min(columns.len());
            let matches = |index: usize, work: &mut IndentAnchorWork| {
                work.lines_examined += 1;
                (!eligible_only || eligible & (1 << index) != 0) && columns[index] <= maximum
            };
            if reverse {
                (lo..hi)
                    .rev()
                    .find(|&i| matches(i, work))
                    .map(|i| start + i)
            } else {
                (lo..hi).find(|&i| matches(i, work)).map(|i| start + i)
            }
        }
        Content::Branch { left, right } => {
            let right_start = start + left.summary.lines;
            let (first, first_start, last, last_start) = if reverse {
                (right, right_start, left, start)
            } else {
                (left, start, right, right_start)
            };
            search(
                first,
                first_start,
                range,
                maximum,
                eligible_only,
                reverse,
                work,
            )
            .or_else(|| {
                search(
                    last,
                    last_start,
                    range,
                    maximum,
                    eligible_only,
                    reverse,
                    work,
                )
            })
        }
    }
}
