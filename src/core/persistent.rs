//! One persistent balanced tree for the core's summarized sequences.
//!
//! A [`Tree`] is an immutable, `Arc`-shared AVL tree whose leaves hold items
//! and whose every node caches the combined [`Summary`] of its subtree.
//! Splitting and concatenating copy only a logarithmic path, so snapshots
//! share everything an edit did not touch. Owners choose the item (one value
//! or a small chunk) and the summary (lengths, counts, maxima); positions are
//! found by descending through one measure of the summary with a [`Seek`].

use std::sync::Arc;

/// Combined description of a run of items. `Default` describes no items.
pub(crate) trait Summary: Clone + Default {
    /// Extend this summary with the items that follow it.
    fn add(&mut self, right: &Self);
}

impl Summary for usize {
    fn add(&mut self, right: &Self) {
        *self += right;
    }
}

pub(crate) trait Item {
    type Summary: Summary;
    fn summary(&self) -> Self::Summary;
}

pub(crate) struct Node<T: Item> {
    summary: T::Summary,
    height: u8,
    kind: Kind<T>,
}

enum Kind<T: Item> {
    Leaf(T),
    Branch(Arc<Node<T>>, Arc<Node<T>>),
}

/// A node's contents, for traversals an owner writes itself (memory
/// accounting, structural diffs that skip pointer-equal subtrees).
pub(crate) enum View<'a, T: Item> {
    Leaf(&'a T),
    Branch(&'a Arc<Node<T>>, &'a Arc<Node<T>>),
}

impl<T: Item> Node<T> {
    pub(crate) fn summary(&self) -> &T::Summary {
        &self.summary
    }

    pub(crate) fn view(&self) -> View<'_, T> {
        match &self.kind {
            Kind::Leaf(item) => View::Leaf(item),
            Kind::Branch(left, right) => View::Branch(left, right),
        }
    }

    fn leaf(item: T, work: &mut Work) -> Arc<Self> {
        work.created += 1;
        Arc::new(Self { summary: item.summary(), height: 1, kind: Kind::Leaf(item) })
    }

    fn branch(left: Arc<Self>, right: Arc<Self>, work: &mut Work) -> Arc<Self> {
        work.created += 1;
        let mut summary = left.summary.clone();
        summary.add(&right.summary);
        Arc::new(Self {
            summary,
            height: left.height.max(right.height) + 1,
            kind: Kind::Branch(left, right),
        })
    }

    fn children(&self) -> (&Arc<Self>, &Arc<Self>) {
        match &self.kind {
            Kind::Branch(left, right) => (left, right),
            Kind::Leaf(_) => unreachable!("a leaf is never the taller side of a join"),
        }
    }
}

/// Nodes visited and allocated by an operation, for locality assertions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Work {
    pub(crate) visited: usize,
    pub(crate) created: usize,
}

/// Which child of a branch holds the position being sought.
pub(crate) enum Side {
    Left,
    /// Exactly between the two children.
    Boundary,
    Right,
}

/// A position expressed against subtree summaries.
pub(crate) trait Seek<S> {
    fn side(&self, left: &S) -> Side;
    /// Step past `left` before descending into its right sibling.
    fn advance(&mut self, left: &S);
}

/// The position `remaining` in one additive measure of the summary. After a
/// search, `remaining` is the offset within the leaf that was reached.
pub(crate) struct At<F> {
    pub(crate) remaining: usize,
    measure: F,
}

pub(crate) fn at<S, F: Fn(&S) -> usize>(position: usize, measure: F) -> At<F> {
    At { remaining: position, measure }
}

impl<S, F: Fn(&S) -> usize> Seek<S> for At<F> {
    fn side(&self, left: &S) -> Side {
        match self.remaining.cmp(&(self.measure)(left)) {
            std::cmp::Ordering::Less => Side::Left,
            std::cmp::Ordering::Equal => Side::Boundary,
            std::cmp::Ordering::Greater => Side::Right,
        }
    }

    fn advance(&mut self, left: &S) {
        self.remaining -= (self.measure)(left);
    }
}

/// How a split treats the leaf its position falls in.
pub(crate) enum LeafSplit<T> {
    /// The whole leaf follows the split.
    Before,
    /// The whole leaf precedes the split.
    After,
    Parts(T, T),
}

pub(crate) struct Tree<T: Item> {
    root: Option<Arc<Node<T>>>,
}

impl<T: Item> Clone for Tree<T> {
    fn clone(&self) -> Self {
        Self { root: self.root.clone() }
    }
}

impl<T: Item> Default for Tree<T> {
    fn default() -> Self {
        Self { root: None }
    }
}

impl<T: Item + std::fmt::Debug> std::fmt::Debug for Tree<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

/// Trees are equal when they hold equal items in order, whatever their shape.
impl<T: Item + PartialEq> PartialEq for Tree<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.iter().eq(other.iter())
    }
}

impl<T: Item + Eq> Eq for Tree<T> {}

impl<T: Item> Tree<T> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_items(items: impl IntoIterator<Item = T>, work: &mut Work) -> Self {
        fn balanced<T: Item>(leaves: &[Arc<Node<T>>], work: &mut Work) -> Arc<Node<T>> {
            match leaves {
                [leaf] => leaf.clone(),
                _ => {
                    let (left, right) = leaves.split_at(leaves.len() / 2);
                    Node::branch(balanced(left, work), balanced(right, work), work)
                }
            }
        }
        let leaves = items.into_iter().map(|item| Node::leaf(item, work)).collect::<Vec<_>>();
        Self { root: (!leaves.is_empty()).then(|| balanced(&leaves, work)) }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub(crate) fn root(&self) -> Option<&Arc<Node<T>>> {
        self.root.as_ref()
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// The summary of every item, or the empty summary.
    pub(crate) fn summary(&self) -> T::Summary {
        self.root.as_ref().map_or_else(T::Summary::default, |root| root.summary.clone())
    }

    /// Read one measure of the whole tree without cloning its summary.
    pub(crate) fn measure<R>(&self, measure: impl FnOnce(&T::Summary) -> R) -> R {
        match &self.root {
            Some(root) => measure(&root.summary),
            None => measure(&T::Summary::default()),
        }
    }

    pub(crate) fn concat(&self, right: &Self, work: &mut Work) -> Self {
        Self { root: concat(self.root.clone(), right.root.clone(), work) }
    }

    /// The leaf holding the sought position and the summary of everything
    /// before that leaf. A position on a boundary belongs to the following leaf.
    pub(crate) fn find(
        &self,
        seek: &mut impl Seek<T::Summary>,
        work: &mut Work,
    ) -> Option<(&T, T::Summary)> {
        let mut node = self.root.as_ref()?;
        let mut before = T::Summary::default();
        loop {
            work.visited += 1;
            match &node.kind {
                Kind::Leaf(item) => return Some((item, before)),
                Kind::Branch(left, right) => match seek.side(&left.summary) {
                    Side::Left => node = left,
                    Side::Boundary | Side::Right => {
                        seek.advance(&left.summary);
                        before.add(&left.summary);
                        node = right;
                    }
                },
            }
        }
    }

    /// Split at the sought position. `leaf` divides the one leaf the position
    /// falls in, which includes a position at that leaf's start or end; it is
    /// not called when the position lies between two subtrees.
    pub(crate) fn split<S: Seek<T::Summary>>(
        &self,
        seek: &mut S,
        leaf: impl FnOnce(&T, &mut S) -> LeafSplit<T>,
        work: &mut Work,
    ) -> (Self, Self) {
        fn split<T: Item, S: Seek<T::Summary>>(
            node: &Arc<Node<T>>,
            seek: &mut S,
            leaf: impl FnOnce(&T, &mut S) -> LeafSplit<T>,
            work: &mut Work,
        ) -> (Option<Arc<Node<T>>>, Option<Arc<Node<T>>>) {
            work.visited += 1;
            match &node.kind {
                Kind::Leaf(item) => match leaf(item, seek) {
                    LeafSplit::Before => (None, Some(node.clone())),
                    LeafSplit::After => (Some(node.clone()), None),
                    LeafSplit::Parts(left, right) => {
                        (Some(Node::leaf(left, work)), Some(Node::leaf(right, work)))
                    }
                },
                Kind::Branch(left, right) => match seek.side(&left.summary) {
                    Side::Boundary => (Some(left.clone()), Some(right.clone())),
                    Side::Left => {
                        let (before, after) = split(left, seek, leaf, work);
                        (before, concat(after, Some(right.clone()), work))
                    }
                    Side::Right => {
                        seek.advance(&left.summary);
                        let (before, after) = split(right, seek, leaf, work);
                        (concat(Some(left.clone()), before, work), after)
                    }
                },
            }
        }
        let Some(root) = &self.root else { return (Self::new(), Self::new()) };
        let (left, right) = split(root, seek, leaf, work);
        (Self { root: left }, Self { root: right })
    }

    /// Split at `position` in one additive measure. `cut` divides an item at
    /// an offset strictly inside it; whole items are shared, and a split at
    /// either end shares this tree unchanged.
    pub(crate) fn split_at(
        &self,
        position: usize,
        measure: impl Fn(&T::Summary) -> usize + Copy,
        cut: impl FnOnce(&T, usize) -> (T, T),
        work: &mut Work,
    ) -> (Self, Self) {
        if position == 0 {
            return (Self::new(), self.clone());
        }
        if position == self.measure(measure) {
            return (self.clone(), Self::new());
        }
        self.split(
            &mut at(position, measure),
            |item, seek| match seek.remaining {
                0 => LeafSplit::Before,
                offset if offset == measure(&item.summary()) => LeafSplit::After,
                offset => {
                    let (left, right) = cut(item, offset);
                    LeafSplit::Parts(left, right)
                }
            },
            work,
        )
    }

    /// Replace the leaf holding the sought position, keeping the tree's shape.
    pub(crate) fn update<S: Seek<T::Summary>>(
        &self,
        seek: &mut S,
        replace: impl FnOnce(&T, &mut S) -> T,
        work: &mut Work,
    ) -> Self {
        fn update<T: Item, S: Seek<T::Summary>>(
            node: &Arc<Node<T>>,
            seek: &mut S,
            replace: impl FnOnce(&T, &mut S) -> T,
            work: &mut Work,
        ) -> Arc<Node<T>> {
            work.visited += 1;
            match &node.kind {
                Kind::Leaf(item) => Node::leaf(replace(item, seek), work),
                Kind::Branch(left, right) => match seek.side(&left.summary) {
                    Side::Left => {
                        Node::branch(update(left, seek, replace, work), right.clone(), work)
                    }
                    Side::Boundary | Side::Right => {
                        seek.advance(&left.summary);
                        Node::branch(left.clone(), update(right, seek, replace, work), work)
                    }
                },
            }
        }
        Self { root: self.root.as_ref().map(|root| update(root, seek, replace, work)) }
    }

    pub(crate) fn first(&self) -> Option<&T> {
        let mut node = self.root.as_ref()?;
        loop {
            match &node.kind {
                Kind::Leaf(item) => return Some(item),
                Kind::Branch(left, _) => node = left,
            }
        }
    }

    pub(crate) fn last(&self) -> Option<&T> {
        let mut node = self.root.as_ref()?;
        loop {
            match &node.kind {
                Kind::Leaf(item) => return Some(item),
                Kind::Branch(_, right) => node = right,
            }
        }
    }

    /// Every item in order.
    pub(crate) fn iter(&self) -> Iter<'_, T> {
        Iter { stack: self.root.iter().map(|root| root.as_ref()).collect() }
    }

    pub(crate) fn height(&self) -> u8 {
        self.root.as_ref().map_or(0, |root| root.height)
    }

    /// Leaves this tree shares, by allocation, with `other`.
    #[cfg(test)]
    pub(crate) fn shared_leaf_count(&self, other: &Self) -> usize {
        let theirs = other.iter().map(std::ptr::from_ref).collect::<std::collections::HashSet<_>>();
        self.iter().filter(|item| theirs.contains(&std::ptr::from_ref(*item))).count()
    }

    /// Whether every branch is balanced and caches its children's summary.
    #[cfg(test)]
    pub(crate) fn invariant_holds(&self) -> bool
    where
        T::Summary: PartialEq,
    {
        fn check<T: Item>(node: &Node<T>) -> Option<u8>
        where
            T::Summary: PartialEq,
        {
            match &node.kind {
                Kind::Leaf(item) => (node.height == 1 && node.summary == item.summary()).then_some(1),
                Kind::Branch(left, right) => {
                    let (left_height, right_height) = (check(left)?, check(right)?);
                    let mut summary = left.summary.clone();
                    summary.add(&right.summary);
                    (left_height.abs_diff(right_height) <= 1
                        && node.height == left_height.max(right_height) + 1
                        && node.summary == summary)
                        .then_some(node.height)
                }
            }
        }
        self.root.as_ref().map_or(true, |root| check(root).is_some())
    }
}

pub(crate) struct Iter<'a, T: Item> {
    stack: Vec<&'a Node<T>>,
}

impl<'a, T: Item> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            match &self.stack.pop()?.kind {
                Kind::Leaf(item) => return Some(item),
                Kind::Branch(left, right) => {
                    self.stack.push(right);
                    self.stack.push(left);
                }
            }
        }
    }
}

type Link<T> = Option<Arc<Node<T>>>;

fn concat<T: Item>(left: Link<T>, right: Link<T>, work: &mut Work) -> Link<T> {
    match (left, right) {
        (None, other) | (other, None) => other,
        (Some(left), Some(right)) => Some(join(left, right, work)),
    }
}

/// Join two balanced trees of any heights into one balanced tree.
fn join<T: Item>(left: Arc<Node<T>>, right: Arc<Node<T>>, work: &mut Work) -> Arc<Node<T>> {
    work.visited += 1;
    if left.height > right.height + 1 {
        let (outer, inner) = left.children();
        return balance(outer.clone(), join(inner.clone(), right, work), work);
    }
    if right.height > left.height + 1 {
        let (inner, outer) = right.children();
        return balance(join(left, inner.clone(), work), outer.clone(), work);
    }
    Node::branch(left, right, work)
}

/// Pair two subtrees whose heights differ by at most two, rotating if needed.
fn balance<T: Item>(left: Arc<Node<T>>, right: Arc<Node<T>>, work: &mut Work) -> Arc<Node<T>> {
    if left.height > right.height + 1 {
        let (outer, inner) = left.children();
        if outer.height >= inner.height {
            return Node::branch(outer.clone(), Node::branch(inner.clone(), right, work), work);
        }
        let (middle_left, middle_right) = inner.children();
        return Node::branch(
            Node::branch(outer.clone(), middle_left.clone(), work),
            Node::branch(middle_right.clone(), right, work),
            work,
        );
    }
    if right.height > left.height + 1 {
        let (inner, outer) = right.children();
        if outer.height >= inner.height {
            return Node::branch(Node::branch(left, inner.clone(), work), outer.clone(), work);
        }
        let (middle_left, middle_right) = inner.children();
        return Node::branch(
            Node::branch(left, middle_left.clone(), work),
            Node::branch(middle_right.clone(), outer.clone(), work),
            work,
        );
    }
    Node::branch(left, right, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Run(usize);

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Total {
        runs: usize,
        length: usize,
    }

    impl Summary for Total {
        fn add(&mut self, right: &Self) {
            self.runs += right.runs;
            self.length += right.length;
        }
    }

    impl Item for Run {
        type Summary = Total;
        fn summary(&self) -> Total {
            Total { runs: 1, length: self.0 }
        }
    }

    fn lengths(tree: &Tree<Run>) -> Vec<usize> {
        tree.iter().map(|run| run.0).collect()
    }

    fn split_length(tree: &Tree<Run>, position: usize) -> (Tree<Run>, Tree<Run>) {
        tree.split(
            &mut at(position, |total: &Total| total.length),
            |run, seek| match seek.remaining {
                0 => LeafSplit::Before,
                offset if offset == run.0 => LeafSplit::After,
                offset => LeafSplit::Parts(Run(offset), Run(run.0 - offset)),
            },
            &mut Work::default(),
        )
    }

    #[test]
    fn splits_and_concatenations_stay_balanced_and_keep_item_order() {
        let mut work = Work::default();
        let tree = Tree::from_items((1..=200).map(Run), &mut work);
        assert!(tree.invariant_holds());
        for runs in [0, 1, 7, 100, 199, 200] {
            let (left, right) = tree.split(
                &mut at(runs, |total: &Total| total.runs),
                |_, seek| if seek.remaining == 0 { LeafSplit::Before } else { LeafSplit::After },
                &mut work,
            );
            assert!(left.invariant_holds() && right.invariant_holds());
            assert_eq!(left.summary().runs, runs);
            let joined = left.concat(&right, &mut work);
            assert!(joined.invariant_holds());
            assert_eq!(joined, tree);
        }
        let lopsided = (0..300).fold(Tree::new(), |tree, run| {
            tree.concat(&Tree::from_items([Run(run)], &mut work), &mut work)
        });
        assert!(lopsided.invariant_holds());
        assert_eq!(lengths(&lopsided), (0..300).collect::<Vec<_>>());
    }

    #[test]
    fn a_position_inside_a_leaf_splits_that_leaf_only() {
        let tree = Tree::from_items([Run(4), Run(6), Run(5)], &mut Work::default());
        let mut seek = at(7, |total: &Total| total.length);
        let (run, before) = tree.find(&mut seek, &mut Work::default()).unwrap();
        assert_eq!((run, before.length, seek.remaining), (&Run(6), 4, 3));
        let (left, right) = split_length(&tree, 7);
        assert_eq!(left.summary().runs + right.summary().runs, 4);
        let (left, right) = split_length(&tree, 4);
        assert_eq!((lengths(&left), lengths(&right)), (vec![4], vec![6, 5]));
    }

    #[test]
    fn a_local_edit_shares_untouched_subtrees_and_visits_a_logarithmic_path() {
        let tree = Tree::from_items((0..4096).map(Run), &mut Work::default());
        let mut work = Work::default();
        let updated = tree.update(&mut at(1000, |total: &Total| total.runs), |_, _| Run(7), &mut work);
        assert!(updated.invariant_holds());
        assert_eq!(updated.summary().runs, 4096);
        assert!(work.visited <= 13 && work.created <= 13, "{work:?}");
        let (View::Branch(_, old_right), View::Branch(_, new_right)) =
            (tree.root().unwrap().view(), updated.root().unwrap().view())
        else {
            panic!("a tree of 4096 leaves has a branch root");
        };
        assert!(Arc::ptr_eq(old_right, new_right));
    }
}
