//! Persistent index of restart checkpoints by snapshot-local position. Each
//! entry stores its distance from the previous one, so an edit shifts every
//! later checkpoint by changing one distance instead of walking them.
use super::Restart;
use crate::persistent::{at, Item, LeafSplit, Seek, Side, Summary, Tree, View, Work};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct Entry {
    /// Distance from the preceding checkpoint, or from zero for the first.
    gap: usize,
    value: Arc<Restart>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Extent {
    count: usize,
    span: usize,
    bytes: usize,
}

impl Summary for Extent {
    fn add(&mut self, right: &Self) {
        self.count += right.count;
        self.span += right.span;
        self.bytes += right.bytes;
    }
}

impl Item for Entry {
    type Summary = Extent;
    type Tag = ();

    fn summary(&self) -> Extent {
        Extent {
            count: 1,
            span: self.gap,
            bytes: std::mem::size_of::<Self>() + 64 + self.value.memory_usage(),
        }
    }
}

/// The first checkpoint at or after a position.
struct NotBefore(usize);

impl Seek<Extent> for NotBefore {
    fn side(&self, left: &Extent) -> Side {
        if left.span >= self.0 { Side::Left } else { Side::Right }
    }

    fn advance(&mut self, left: &Extent) {
        self.0 -= left.span;
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Checkpoints {
    entries: Tree<Entry>,
    pub visits: usize,
}

type Node = crate::persistent::Node<Entry>;

impl Checkpoints {
    pub fn len(&self) -> usize {
        self.entries.measure(|extent| extent.count)
    }

    pub fn memory_usage(&self) -> usize {
        self.entries.measure(|extent| extent.bytes)
    }

    /// The last checkpoint at or before `at`.
    pub fn before(&mut self, at: usize) -> Option<(usize, Arc<Restart>)> {
        fn last<'a>(node: &'a Node, visits: &mut usize) -> &'a Entry {
            *visits += 1;
            match node.view() {
                View::Leaf(entry) => entry,
                View::Branch(_, right) => last(right, visits),
            }
        }
        fn search<'a>(node: &'a Node, base: usize, at: usize, visits: &mut usize) -> Option<(usize, &'a Entry)> {
            *visits += 1;
            match node.view() {
                View::Leaf(entry) => (base + entry.gap <= at).then_some((base + entry.gap, entry)),
                View::Branch(left, right) => {
                    let middle = base + left.summary().span;
                    if middle > at {
                        return search(left, base, at, visits);
                    }
                    search(right, middle, at, visits).or_else(|| Some((middle, last(left, visits))))
                }
            }
        }
        let (key, entry) = search(self.entries.root()?, 0, at, &mut self.visits)?;
        Some((key, entry.value.clone()))
    }

    pub fn exact(&mut self, at: usize) -> Option<Arc<Restart>> {
        self.before(at).filter(|(k, _)| *k == at).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: usize, value: Restart) {
        let mut work = Work::default();
        let (before, from) = split(&self.entries, key, &mut work);
        let after = match from.first() {
            Some(first) if first.gap == key => drop_first(&from, &mut work),
            _ => from,
        };
        let entry = Tree::from_items([Entry { gap: key, value: Arc::new(value) }], &mut work);
        self.entries = join(&join(&before, &entry, &mut work), &after, &mut work);
        self.visits += work.visited;
    }

    /// Drop the checkpoints in `start..=old_end` and move the later ones by
    /// the edit's change in length.
    pub fn edit(&mut self, start: usize, old_end: usize, new_end: usize) {
        let mut work = Work::default();
        let (before, from) = split(&self.entries, start, &mut work);
        let (_, after) = split(&from, old_end.saturating_add(1), &mut work);
        let delta = new_end as isize - old_end as isize;
        let after = shift_first(&after, |gap| gap.checked_add_signed(delta).expect("validated checkpoint shift"), &mut work);
        self.entries = join(&before, &after, &mut work);
        self.visits += work.visited;
    }

    pub fn evict_first(&mut self) {
        let mut work = Work::default();
        self.entries = drop_first(&self.entries, &mut work);
        self.visits += work.visited;
    }
}

fn shift_first(tree: &Tree<Entry>, shift: impl FnOnce(usize) -> usize, work: &mut Work) -> Tree<Entry> {
    if tree.is_empty() {
        return tree.clone();
    }
    tree.update(
        &mut at(0, |extent: &Extent| extent.count),
        |entry, _| Entry { gap: shift(entry.gap), value: entry.value.clone() },
        work,
    )
}

/// Checkpoints before `key`, and those from it on with the first one's
/// distance measured from zero again.
fn split(tree: &Tree<Entry>, key: usize, work: &mut Work) -> (Tree<Entry>, Tree<Entry>) {
    let (before, from) = tree.split(
        &mut NotBefore(key),
        |entry, seek| if entry.gap >= seek.0 { LeafSplit::Before } else { LeafSplit::After },
        work,
    );
    let base = before.measure(|extent| extent.span);
    let from = shift_first(&from, |gap| gap + base, work);
    (before, from)
}

/// Concatenate trees whose first distances are both measured from zero.
fn join(left: &Tree<Entry>, right: &Tree<Entry>, work: &mut Work) -> Tree<Entry> {
    let base = left.measure(|extent| extent.span);
    left.concat(&shift_first(right, |gap| gap - base, work), work)
}

fn drop_first(tree: &Tree<Entry>, work: &mut Work) -> Tree<Entry> {
    let Some(first) = tree.first().map(|entry| entry.gap) else { return tree.clone() };
    let (_, rest) = tree.split_at(1, |extent| extent.count, |_, _| unreachable!("entries are whole"), work);
    shift_first(&rest, |gap| gap + first, work)
}
