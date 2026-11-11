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

use crate::persistent::{at, Item, Summary, Tree, View, Work};
use std::ops::Range;

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
    blocks: Tree<Block>,
}

type Node = crate::persistent::Node<Block>;

/// Column counts of up to `BLOCK_LINES` lines and their eligibility bitset.
#[derive(Debug)]
struct Block {
    columns: Box<[usize]>,
    eligible: u128,
}

#[derive(Clone, Copy, Debug)]
struct Minima {
    lines: usize,
    blocks: usize,
    min_eligible: Option<usize>,
    min_all: usize,
}

impl Default for Minima {
    fn default() -> Self {
        Self { lines: 0, blocks: 0, min_eligible: None, min_all: usize::MAX }
    }
}

impl Summary for Minima {
    fn add(&mut self, right: &Self) {
        self.lines += right.lines;
        self.blocks += right.blocks;
        self.min_eligible = match (self.min_eligible, right.min_eligible) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) | (None, a) => a,
        };
        self.min_all = self.min_all.min(right.min_all);
    }
}

impl Item for Block {
    type Summary = Minima;

    fn summary(&self) -> Minima {
        let mut summary = Minima { lines: self.columns.len(), blocks: 1, ..Minima::default() };
        for (index, &columns) in self.columns.iter().enumerate() {
            summary.min_all = summary.min_all.min(columns);
            if self.eligible & (1 << index) != 0 {
                summary.min_eligible = Some(summary.min_eligible.unwrap_or(usize::MAX).min(columns));
            }
        }
        summary
    }
}

impl Block {
    fn new(lines: &[LineIndentation], work: &mut IndentAnchorWork) -> Self {
        debug_assert!(!lines.is_empty() && lines.len() <= BLOCK_LINES);
        work.lines_copied += lines.len();
        let mut eligible = 0u128;
        let columns = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if line.anchor_eligible {
                    eligible |= 1 << index;
                }
                line.columns
            })
            .collect();
        Self { columns, eligible }
    }

    fn line(&self, index: usize) -> LineIndentation {
        LineIndentation {
            columns: self.columns[index],
            anchor_eligible: self.eligible & (1 << index) != 0,
        }
    }

    fn lines(&self) -> Vec<LineIndentation> {
        (0..self.columns.len()).map(|index| self.line(index)).collect()
    }
}

fn lines(summary: &Minima) -> usize {
    summary.lines
}

/// Run one tree operation and add its node counts to `work`.
fn counted<R>(work: &mut IndentAnchorWork, operation: impl FnOnce(&mut Work) -> R) -> R {
    let mut tree_work = Work::default();
    let result = operation(&mut tree_work);
    work.nodes_visited += tree_work.visited;
    work.nodes_created += tree_work.created;
    result
}

impl IndentAnchorIndex {
    pub(crate) fn shares_root_with(&self, other: &Self) -> bool {
        self.blocks.ptr_eq(&other.blocks)
    }

    pub fn from_lines(lines: impl IntoIterator<Item = LineIndentation>) -> Self {
        let mut work = IndentAnchorWork::default();
        Self { blocks: build(lines, &mut work) }
    }

    pub fn len(&self) -> usize {
        self.blocks.measure(lines)
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn statistics(&self) -> IndentAnchorStatistics {
        if self.blocks.is_empty() {
            return IndentAnchorStatistics::default();
        }
        let summary = self.blocks.summary();
        IndentAnchorStatistics {
            line_count: summary.lines,
            block_count: summary.blocks,
            tree_height: usize::from(self.blocks.height()),
            payload_bytes: (summary.blocks * 2 - 1) * std::mem::size_of::<Node>()
                + summary.lines * std::mem::size_of::<usize>(),
        }
    }

    pub fn line(&self, line: usize) -> Option<LineIndentation> {
        self.line_with_work(line, &mut IndentAnchorWork::default())
    }

    fn line_with_work(
        &self,
        line: usize,
        work: &mut IndentAnchorWork,
    ) -> Option<LineIndentation> {
        if line >= self.len() {
            return None;
        }
        let mut seek = at(line, lines);
        let (block, _) = counted(work, |work| self.blocks.find(&mut seek, work))?;
        work.lines_examined += 1;
        Some(block.line(seek.remaining))
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
            self.blocks.root()?,
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
            self.blocks.root()?,
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
            .checked_add(inserted.measure(lines))
            .ok_or(IndentAnchorIndexError::LineCountOverflow)?;
        if removed.is_empty() && inserted.is_empty() {
            return Ok(());
        }
        let (left, tail) = split(&self.blocks, removed.start, work);
        let (_, right) = split(&tail, removed.len(), work);
        self.blocks = concat(&concat(&left, &inserted, work), &right, work);
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
                self.index.blocks.root()?,
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

fn build(
    lines: impl IntoIterator<Item = LineIndentation>,
    work: &mut IndentAnchorWork,
) -> Tree<Block> {
    let lines = lines.into_iter().collect::<Vec<_>>();
    let blocks = lines.chunks(BLOCK_LINES).map(|chunk| Block::new(chunk, work)).collect::<Vec<_>>();
    counted(work, |work| Tree::from_items(blocks, work))
}

fn split(tree: &Tree<Block>, at: usize, work: &mut IndentAnchorWork) -> (Tree<Block>, Tree<Block>) {
    let mut cut_work = IndentAnchorWork::default();
    let parts = counted(work, |work| {
        tree.split_at(
            at,
            lines,
            |block, offset| {
                let all = block.lines();
                (Block::new(&all[..offset], &mut cut_work), Block::new(&all[offset..], &mut cut_work))
            },
            work,
        )
    });
    work.lines_copied += cut_work.lines_copied;
    parts
}

/// Concatenate, first repairing an underfull block at the seam so editing
/// does not gradually turn the index into a node per line.
fn concat(left: &Tree<Block>, right: &Tree<Block>, work: &mut IndentAnchorWork) -> Tree<Block> {
    let (Some(a), Some(b)) = (left.last(), right.first()) else {
        return if left.is_empty() { right.clone() } else { left.clone() };
    };
    let (a_count, b_count) = (a.columns.len(), b.columns.len());
    if a_count >= MIN_BLOCK_LINES && b_count >= MIN_BLOCK_LINES {
        return counted(work, |work| left.concat(right, work));
    }
    let mut seam = a.lines();
    seam.extend(b.lines());
    let (left, _) = split(left, left.measure(lines) - a_count, work);
    let (_, right) = split(right, b_count, work);
    let middle = if seam.len() <= BLOCK_LINES {
        vec![Block::new(&seam, work)]
    } else {
        let (first, last) = seam.split_at(seam.len() / 2);
        vec![Block::new(first, work), Block::new(last, work)]
    };
    counted(work, |work| {
        left.concat(&Tree::from_items(middle, work), work).concat(&right, work)
    })
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
    let summary = node.summary();
    if start >= range.end || start + summary.lines <= range.start {
        return None;
    }
    let minimum = if eligible_only { summary.min_eligible } else { Some(summary.min_all) };
    if minimum.is_none_or(|minimum| minimum > maximum) {
        return None;
    }
    match node.view() {
        View::Leaf(Block { columns, eligible }) => {
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
        View::Branch(left, right) => {
            let right_start = start + left.summary().lines;
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
