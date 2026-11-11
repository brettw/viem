//! Persistent source-byte extents for physical hard lines.
//!
//! Line lengths, rather than absolute suffix offsets, are stored under a
//! prefix-sum tree. A local edit path-copies only the leaves containing changed
//! lines; every later line moves implicitly through the changed aggregate.

use crate::persistent::{at, Item, LeafSplit, Summary, Tree, Work};
use std::ops::Range;

const LEAF_LINES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceHardLineIndex {
    content_start: usize,
    /// Never empty: a document has at least one physical line.
    lines: Tree<Lengths>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceHardLineSpliceStats {
    pub(crate) nodes_visited: usize,
    pub(crate) nodes_copied: usize,
    pub(crate) leaves_copied: usize,
    pub(crate) records_copied: usize,
}

/// Source byte lengths of up to `LEAF_LINES` consecutive lines.
#[derive(Debug, Eq, PartialEq)]
struct Lengths(Box<[usize]>);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Extent {
    lines: usize,
    bytes: usize,
}

impl Summary for Extent {
    fn add(&mut self, right: &Self) {
        self.lines += right.lines;
        self.bytes = self
            .bytes
            .checked_add(right.bytes)
            .expect("validated source hard-line aggregate is representable");
    }
}

impl Item for Lengths {
    type Summary = Extent;

    fn summary(&self) -> Extent {
        Extent { lines: self.0.len(), bytes: self.0.iter().sum() }
    }
}

fn lines(extent: &Extent) -> usize {
    extent.lines
}

fn leaves(lengths: &[usize]) -> impl Iterator<Item = Lengths> + '_ {
    lengths.chunks(LEAF_LINES).map(|chunk| Lengths(chunk.into()))
}

/// Byte lengths of consecutive ranges that begin at `start`.
fn consecutive_lengths(start: usize, ranges: &[Range<usize>]) -> Option<Vec<usize>> {
    let mut expected = start;
    let mut lengths = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start != expected || range.start > range.end {
            return None;
        }
        lengths.push(range.len());
        expected = range.end;
    }
    Some(lengths)
}

impl SourceHardLineIndex {
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        visitor.tree(&self.lines, |lengths| std::mem::size_of_val(&*lengths.0), |_, _| {});
    }

    pub(crate) fn new(ranges: Vec<Range<usize>>) -> Option<Self> {
        let content_start = ranges.first()?.start;
        let lengths = consecutive_lengths(content_start, &ranges)?;
        Some(Self {
            content_start,
            lines: Tree::from_items(leaves(&lengths), &mut Work::default()),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.lines.measure(lines)
    }

    pub(crate) fn get(&self, line: usize) -> Option<Range<usize>> {
        if line >= self.len() {
            return None;
        }
        let mut seek = at(line, lines);
        let (leaf, before) = self.lines.find(&mut seek, &mut Work::default())?;
        let preceding = leaf.0[..seek.remaining]
            .iter()
            .try_fold(before.bytes, |sum, length| sum.checked_add(*length))?;
        let start = self.content_start.checked_add(preceding)?;
        Some(start..start.checked_add(leaf.0[seek.remaining])?)
    }

    pub(crate) fn source_end(&self) -> usize {
        self.content_start
            .checked_add(self.lines.measure(|extent| extent.bytes))
            .expect("validated source hard-line aggregate is representable")
    }

    /// Locate a physical source line through byte aggregates without binary
    /// searching repeatedly through the ordinal lookup API.
    pub(crate) fn line_at_offset(&self, offset: usize) -> Option<usize> {
        let relative = offset.checked_sub(self.content_start)?;
        if relative > self.lines.measure(|extent| extent.bytes) {
            return None;
        }
        let mut seek = at(relative, |extent: &Extent| extent.bytes);
        let (leaf, before) = self.lines.find(&mut seek, &mut Work::default())?;
        let mut remaining = seek.remaining;
        for (index, length) in leaf.0.iter().enumerate() {
            if remaining < *length || index + 1 == leaf.0.len() {
                return Some(before.lines + index);
            }
            remaining -= length;
        }
        None
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
        target: Range<usize>,
        ranges: &[Range<usize>],
    ) -> Option<(Self, SourceHardLineSpliceStats)> {
        if target.start >= target.end || target.end > self.len() || ranges.is_empty() {
            return None;
        }
        let lengths = consecutive_lengths(self.get(target.start)?.start, ranges)?;
        let mut stats = SourceHardLineSpliceStats::default();
        let mut work = Work::default();
        let replaced = if ranges.len() == target.len() {
            // The line count is unchanged: rewrite the affected leaves in
            // place so every other leaf stays shared.
            let mut tree = self.lines.clone();
            let mut done = 0;
            while done < lengths.len() {
                tree = tree.update(
                    &mut at(target.start + done, lines),
                    |leaf, seek| {
                        let mut values = leaf.0.to_vec();
                        let count = (values.len() - seek.remaining).min(lengths.len() - done);
                        values[seek.remaining..seek.remaining + count]
                            .copy_from_slice(&lengths[done..done + count]);
                        done += count;
                        stats.leaves_copied += 1;
                        stats.records_copied += values.len();
                        Lengths(values.into())
                    },
                    &mut work,
                );
            }
            tree
        } else {
            let mut split = |tree: &Tree<Lengths>, line: usize, work: &mut Work| {
                tree.split(
                    &mut at(line, lines),
                    |leaf, seek| match seek.remaining {
                        0 => LeafSplit::Before,
                        offset if offset == leaf.0.len() => LeafSplit::After,
                        offset => {
                            stats.leaves_copied += 2;
                            stats.records_copied += leaf.0.len();
                            LeafSplit::Parts(Lengths(leaf.0[..offset].into()), Lengths(leaf.0[offset..].into()))
                        }
                    },
                    work,
                )
            };
            let (prefix, tail) = split(&self.lines, target.start, &mut work);
            let (_, suffix) = split(&tail, target.len(), &mut work);
            let inserted = Tree::from_items(leaves(&lengths), &mut work);
            stats.records_copied += lengths.len();
            stats.leaves_copied += lengths.len().div_ceil(LEAF_LINES);
            prefix.concat(&inserted, &mut work).concat(&suffix, &mut work)
        };
        stats.nodes_visited = work.visited;
        stats.nodes_copied = work.created;
        Some((Self { content_start: self.content_start, lines: replaced }, stats))
    }

    #[cfg(test)]
    pub(crate) fn shared_leaf_count_with(&self, other: &Self) -> usize {
        self.lines.shared_leaf_count(&other.lines)
    }

    #[cfg(test)]
    pub(crate) fn leaf_count(&self) -> usize {
        self.lines.iter().count()
    }

    #[cfg(test)]
    pub(crate) fn invariant_holds(&self) -> bool {
        self.lines.invariant_holds()
            && self.lines.iter().all(|leaf| !leaf.0.is_empty() && leaf.0.len() <= LEAF_LINES)
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
