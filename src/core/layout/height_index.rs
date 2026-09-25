use std::ops::Range;
use std::sync::Arc;

/// Height plus the certainty of every hard line contributing to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightMeasurement {
    height: f64,
    exact: bool,
}

impl HeightMeasurement {
    pub fn height(self) -> f64 {
        self.height
    }

    pub fn is_exact(self) -> bool {
        self.exact
    }
}

/// Result of locating a hard line in the unpaginated vertical coordinate
/// space. `prefix_is_exact` describes `line_top`, independently of whether the
/// located line's own height is exact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HardLineHeightHit {
    hard_line: usize,
    line_top: f64,
    line_height: f64,
    prefix_is_exact: bool,
    line_is_exact: bool,
}

impl HardLineHeightHit {
    pub fn hard_line(self) -> usize {
        self.hard_line
    }

    pub fn line_top(self) -> f64 {
        self.line_top
    }

    pub fn line_height(self) -> f64 {
        self.line_height
    }

    pub fn prefix_is_exact(self) -> bool {
        self.prefix_is_exact
    }

    pub fn line_is_exact(self) -> bool {
        self.line_is_exact
    }
}

/// Constant-time structural diagnostics. One tree node represents one run of
/// adjacent hard lines with identical height and certainty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewHeightIndexStatistics {
    hard_line_count: usize,
    run_count: usize,
    tree_depth: usize,
}

impl ViewHeightIndexStatistics {
    pub fn hard_line_count(self) -> usize {
        self.hard_line_count
    }

    pub fn run_count(self) -> usize {
        self.run_count
    }

    pub fn tree_depth(self) -> usize {
        self.tree_depth
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewHeightIndexError {
    InvalidHeight,
    InvalidVerticalCoordinate,
    HardLineOutsideIndex {
        hard_line: usize,
        hard_line_count: usize,
    },
    RangeOutsideIndex {
        start: usize,
        end: usize,
        hard_line_count: usize,
    },
    LineCountOverflow,
    HeightOverflow,
    InconsistentLayoutSnapshot(&'static str),
}

/// A coordinator-owned height summary for one view.
///
/// The tree is indexed implicitly by aggregate hard-line counts rather than by
/// stored document ordinals. Inserting or deleting hard lines therefore
/// changes only the logarithmic split/join paths; following entries are not
/// renumbered or shifted. Clones share immutable nodes, so staging a viewport
/// update does not copy the heights learned while visiting earlier regions.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewHeightIndex {
    root: Link,
    estimated_line_height: f64,
}

impl ViewHeightIndex {
    pub fn new_estimated(
        hard_line_count: usize,
        estimated_line_height: f64,
    ) -> Result<Self, ViewHeightIndexError> {
        validate_height(estimated_line_height)?;
        checked_height_product(estimated_line_height, hard_line_count)?;
        let mut result = Self {
            root: None,
            estimated_line_height,
        };
        if hard_line_count > 0 {
            let run = HeightRun {
                line_count: hard_line_count,
                height: estimated_line_height,
                exact: false,
            };
            result.root = Some(Node::new(run));
        }
        Ok(result)
    }

    pub fn hard_line_count(&self) -> usize {
        subtree_lines(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn estimated_line_height(&self) -> f64 {
        self.estimated_line_height
    }

    pub fn total_height(&self) -> HeightMeasurement {
        HeightMeasurement {
            height: subtree_height(&self.root),
            exact: subtree_exact(&self.root),
        }
    }

    pub fn statistics(&self) -> ViewHeightIndexStatistics {
        ViewHeightIndexStatistics {
            hard_line_count: self.hard_line_count(),
            run_count: subtree_runs(&self.root),
            tree_depth: subtree_depth(&self.root),
        }
    }

    /// Height of `[0, hard_line_end)` in O(log n).
    pub fn prefix_height(
        &self,
        hard_line_end: usize,
    ) -> Result<HeightMeasurement, ViewHeightIndexError> {
        let count = self.hard_line_count();
        if hard_line_end > count {
            return Err(ViewHeightIndexError::RangeOutsideIndex {
                start: 0,
                end: hard_line_end,
                hard_line_count: count,
            });
        }
        Ok(measure_range(&self.root, 0, hard_line_end).public())
    }

    /// Height and certainty of an arbitrary ordered half-open hard-line range.
    pub fn range_height(
        &self,
        range: Range<usize>,
    ) -> Result<HeightMeasurement, ViewHeightIndexError> {
        self.validate_range(&range)?;
        Ok(measure_range(&self.root, range.start, range.end).public())
    }

    /// Locate the hard line containing `y`. The exact total-height boundary is
    /// outside the index and returns `None`; negative and nonfinite values are
    /// rejected instead of being clamped.
    pub fn hard_line_at_y(
        &self,
        y: f64,
    ) -> Result<Option<HardLineHeightHit>, ViewHeightIndexError> {
        if !y.is_finite() || y < 0.0 {
            return Err(ViewHeightIndexError::InvalidVerticalCoordinate);
        }
        let total = subtree_height(&self.root);
        if self.root.is_none() || y >= total {
            return Ok(None);
        }

        let mut node = self.root.as_deref();
        let mut remaining_y = y;
        let mut hard_line = 0usize;
        let mut line_top = 0.0;
        let mut prefix_is_exact = true;

        while let Some(current) = node {
            let left_height = subtree_height(&current.left);
            if remaining_y < left_height {
                node = current.left.as_deref();
                continue;
            }

            remaining_y -= left_height;
            line_top = checked_height_sum(line_top, left_height)
                .expect("a prefix cannot exceed the validated total height");
            hard_line = hard_line
                .checked_add(subtree_lines(&current.left))
                .expect("a prefix cannot exceed the validated line count");
            prefix_is_exact &= subtree_exact(&current.left);

            let run_height = current.run.total_height();
            if remaining_y < run_height {
                let offset = ((remaining_y / current.run.height).floor() as usize)
                    .min(current.run.line_count - 1);
                let offset_height = checked_height_product(current.run.height, offset)
                    .expect("a run prefix cannot exceed its validated height");
                hard_line = hard_line
                    .checked_add(offset)
                    .expect("a run prefix cannot exceed the validated line count");
                line_top = checked_height_sum(line_top, offset_height)
                    .expect("a run prefix cannot exceed the validated total height");
                if offset > 0 {
                    prefix_is_exact &= current.run.exact;
                }
                return Ok(Some(HardLineHeightHit {
                    hard_line,
                    line_top,
                    line_height: current.run.height,
                    prefix_is_exact,
                    line_is_exact: current.run.exact,
                }));
            }

            remaining_y -= run_height;
            line_top = checked_height_sum(line_top, run_height)
                .expect("a prefix cannot exceed the validated total height");
            hard_line = hard_line
                .checked_add(current.run.line_count)
                .expect("a prefix cannot exceed the validated line count");
            prefix_is_exact &= current.run.exact;
            node = current.right.as_deref();
        }

        // `y < total` guarantees a result when tree aggregates are sound.
        unreachable!("height aggregates did not contain an in-range coordinate")
    }

    /// Mark one hard line exact. This is a constant-size replacement bracketed
    /// by two AVL-tree splits and joins, hence O(log n).
    pub fn set_exact_height(
        &mut self,
        hard_line: usize,
        height: f64,
    ) -> Result<(), ViewHeightIndexError> {
        let count = self.hard_line_count();
        if hard_line >= count {
            return Err(ViewHeightIndexError::HardLineOutsideIndex {
                hard_line,
                hard_line_count: count,
            });
        }
        let end = hard_line
            .checked_add(1)
            .ok_or(ViewHeightIndexError::LineCountOverflow)?;
        self.set_exact_range(hard_line..end, height)
    }

    /// Mark an ordered range exact with one common hard-line height.
    pub fn set_exact_range(
        &mut self,
        range: Range<usize>,
        height: f64,
    ) -> Result<(), ViewHeightIndexError> {
        self.validate_range(&range)?;
        validate_height(height)?;
        if range.is_empty() {
            return Ok(());
        }
        let line_count = range.end - range.start;
        let replacement_height = checked_height_product(height, line_count)?;
        self.validate_replacement(&range, line_count, replacement_height)?;
        let replacement = Some(Node::new(HeightRun {
            line_count,
            height,
            exact: true,
        }));
        self.replace_range(range, replacement);
        Ok(())
    }

    /// Batch exact heights are compacted into equal adjacent runs before one
    /// structural replacement. Work is O(k + log n) for `k` supplied lines.
    pub fn set_exact_heights(
        &mut self,
        start: usize,
        heights: &[f64],
    ) -> Result<(), ViewHeightIndexError> {
        let end = start
            .checked_add(heights.len())
            .ok_or(ViewHeightIndexError::LineCountOverflow)?;
        let range = start..end;
        self.validate_range(&range)?;
        if heights.is_empty() {
            return Ok(());
        }

        let mut replacement_height = 0.0;
        for height in heights {
            validate_height(*height)?;
            replacement_height = checked_height_sum(replacement_height, *height)
                .ok_or(ViewHeightIndexError::HeightOverflow)?;
        }
        self.validate_replacement(&range, heights.len(), replacement_height)?;

        let mut runs: Vec<HeightRun> = Vec::new();
        for height in heights {
            if let Some(last) = runs.last_mut() {
                if last.height.to_bits() == height.to_bits() {
                    last.line_count = last
                        .line_count
                        .checked_add(1)
                        .expect("the validated input length bounds each run");
                    continue;
                }
            }
            runs.push(HeightRun {
                line_count: 1,
                height: *height,
                exact: true,
            });
        }
        let replacement = self.tree_from_runs(runs);
        self.replace_range(range, replacement);
        Ok(())
    }

    /// Revert a range to this index's estimate in O(log n).
    pub fn invalidate(&mut self, range: Range<usize>) -> Result<(), ViewHeightIndexError> {
        self.validate_range(&range)?;
        if range.is_empty() {
            return Ok(());
        }
        let line_count = range.end - range.start;
        let replacement_height = checked_height_product(self.estimated_line_height, line_count)?;
        self.validate_replacement(&range, line_count, replacement_height)?;
        let replacement = Some(Node::new(HeightRun {
            line_count,
            height: self.estimated_line_height,
            exact: false,
        }));
        self.replace_range(range, replacement);
        Ok(())
    }

    /// Delete `removed` and insert `inserted_hard_lines` estimated entries at
    /// its start. The implicit tree avoids touching or renumbering its suffix.
    pub fn splice(
        &mut self,
        removed: Range<usize>,
        inserted_hard_lines: usize,
    ) -> Result<(), ViewHeightIndexError> {
        self.validate_range(&removed)?;
        let removed_count = removed.end - removed.start;
        let outside_count = self
            .hard_line_count()
            .checked_sub(removed_count)
            .expect("a validated range cannot remove too many lines");
        outside_count
            .checked_add(inserted_hard_lines)
            .ok_or(ViewHeightIndexError::LineCountOverflow)?;
        let replacement_height =
            checked_height_product(self.estimated_line_height, inserted_hard_lines)?;
        self.validate_replacement(&removed, inserted_hard_lines, replacement_height)?;

        if removed.is_empty() && inserted_hard_lines == 0 {
            return Ok(());
        }
        let replacement = (inserted_hard_lines > 0).then(|| {
            Node::new(HeightRun {
                line_count: inserted_hard_lines,
                height: self.estimated_line_height,
                exact: false,
            })
        });
        self.replace_range(removed, replacement);
        Ok(())
    }

    fn validate_range(&self, range: &Range<usize>) -> Result<(), ViewHeightIndexError> {
        let count = self.hard_line_count();
        if range.start > range.end || range.end > count {
            return Err(ViewHeightIndexError::RangeOutsideIndex {
                start: range.start,
                end: range.end,
                hard_line_count: count,
            });
        }
        Ok(())
    }

    fn validate_replacement(
        &self,
        range: &Range<usize>,
        replacement_count: usize,
        replacement_height: f64,
    ) -> Result<(), ViewHeightIndexError> {
        let left = measure_range(&self.root, 0, range.start);
        let right = measure_range(&self.root, range.end, self.hard_line_count());
        left.line_count
            .checked_add(replacement_count)
            .and_then(|count| count.checked_add(right.line_count))
            .ok_or(ViewHeightIndexError::LineCountOverflow)?;
        checked_height_sum(left.height, replacement_height)
            .and_then(|height| checked_height_sum(height, right.height))
            .ok_or(ViewHeightIndexError::HeightOverflow)?;
        Ok(())
    }

    fn replace_range(&mut self, range: Range<usize>, replacement: Link) {
        let root = self.root.take();
        let (left, remainder) = split(root, range.start);
        let (_, right) = split(remainder, range.end - range.start);
        let joined = join(left, replacement);
        self.root = join(joined, right);
    }

    fn tree_from_runs(&mut self, runs: Vec<HeightRun>) -> Link {
        balanced_tree_from_runs(&runs)
    }
}

type Link = Option<Arc<Node>>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct HeightRun {
    line_count: usize,
    height: f64,
    exact: bool,
}

impl HeightRun {
    fn total_height(self) -> f64 {
        checked_height_product(self.height, self.line_count)
            .expect("tree runs have validated finite aggregate heights")
    }

    fn can_merge(self, other: Self) -> bool {
        self.exact == other.exact && self.height.to_bits() == other.height.to_bits()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    run: HeightRun,
    left: Link,
    right: Link,
    subtree_lines: usize,
    subtree_height: f64,
    subtree_exact: bool,
    subtree_runs: usize,
    subtree_depth: usize,
}

impl Node {
    fn new(run: HeightRun) -> Arc<Self> {
        Arc::new(Self::leaf(run))
    }

    fn leaf(run: HeightRun) -> Self {
        Self {
            run,
            left: None,
            right: None,
            subtree_lines: run.line_count,
            subtree_height: run.total_height(),
            subtree_exact: run.exact,
            subtree_runs: 1,
            subtree_depth: 1,
        }
    }

    fn refresh(&mut self) {
        self.subtree_lines = subtree_lines(&self.left)
            .checked_add(self.run.line_count)
            .and_then(|count| count.checked_add(subtree_lines(&self.right)))
            .expect("public mutation validation prevents line-count overflow");
        self.subtree_height =
            checked_height_sum(subtree_height(&self.left), self.run.total_height())
                .and_then(|height| checked_height_sum(height, subtree_height(&self.right)))
                .expect("public mutation validation prevents aggregate-height overflow");
        self.subtree_exact =
            subtree_exact(&self.left) && self.run.exact && subtree_exact(&self.right);
        self.subtree_runs = subtree_runs(&self.left)
            .checked_add(1)
            .and_then(|runs| runs.checked_add(subtree_runs(&self.right)))
            .expect("run count cannot exceed the validated line count");
        self.subtree_depth = subtree_depth(&self.left)
            .max(subtree_depth(&self.right))
            .checked_add(1)
            .expect("tree depth cannot exceed the validated line count");
    }
}

#[derive(Clone, Copy)]
struct InternalMeasurement {
    line_count: usize,
    height: f64,
    exact: bool,
}

impl InternalMeasurement {
    const EMPTY: Self = Self {
        line_count: 0,
        height: 0.0,
        exact: true,
    };

    fn append(&mut self, other: Self) {
        self.line_count = self
            .line_count
            .checked_add(other.line_count)
            .expect("a measured range cannot exceed the tree's line count");
        self.height = checked_height_sum(self.height, other.height)
            .expect("a measured range cannot exceed the tree's total height");
        self.exact &= other.exact;
    }

    fn public(self) -> HeightMeasurement {
        HeightMeasurement {
            height: self.height,
            exact: self.exact,
        }
    }
}

fn subtree_lines(tree: &Link) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_lines)
}

fn subtree_height(tree: &Link) -> f64 {
    tree.as_ref().map_or(0.0, |node| node.subtree_height)
}

fn subtree_exact(tree: &Link) -> bool {
    tree.as_ref().map_or(true, |node| node.subtree_exact)
}

fn subtree_runs(tree: &Link) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_runs)
}

fn subtree_depth(tree: &Link) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_depth)
}

fn balanced_tree_from_runs(runs: &[HeightRun]) -> Link {
    if runs.is_empty() {
        return None;
    }
    let middle = runs.len() / 2;
    let mut root = Node::leaf(runs[middle]);
    let left_runs = &runs[..middle];
    let right_runs = &runs[middle + 1..];
    root.left = balanced_tree_from_runs(left_runs);
    root.right = balanced_tree_from_runs(right_runs);
    root.refresh();
    Some(Arc::new(root))
}

fn split(mut tree: Link, at: usize) -> (Link, Link) {
    let Some(root) = tree.take() else {
        debug_assert_eq!(at, 0);
        return (None, None);
    };
    // Only the split path is copied when a prior snapshot still owns it.
    let mut root = Arc::unwrap_or_clone(root);
    debug_assert!(at <= root.subtree_lines);
    let left_lines = subtree_lines(&root.left);
    let run_end = left_lines
        .checked_add(root.run.line_count)
        .expect("node aggregates validate run boundaries");

    if at < left_lines {
        let (left, right_of_split) = split(root.left.take(), at);
        let right = join_with_run(right_of_split, root.run, root.right.take());
        return (left, right);
    }
    if at > run_end {
        let (left_of_split, right) = split(root.right.take(), at - run_end);
        let left = join_with_run(root.left.take(), root.run, left_of_split);
        return (left, right);
    }
    if at == left_lines {
        let left = root.left.take();
        let right = join_with_run(None, root.run, root.right.take());
        return (left, right);
    }
    if at == run_end {
        let right = root.right.take();
        let left = join_with_run(root.left.take(), root.run, None);
        return (left, right);
    }

    let left_count = at - left_lines;
    let right_count = root.run.line_count - left_count;
    let left_run = HeightRun {
        line_count: left_count,
        ..root.run
    };
    let right_run = HeightRun {
        line_count: right_count,
        ..root.run
    };
    (
        join_with_run(root.left.take(), left_run, None),
        join_with_run(None, right_run, root.right.take()),
    )
}

fn join(left: Link, right: Link) -> Link {
    let can_merge = left
        .as_ref()
        .zip(right.as_ref())
        .is_some_and(|(left, right)| rightmost_run(left).can_merge(leftmost_run(right)));
    if !can_merge {
        return concat_raw(left, right);
    }

    let (left, left_run) = pop_last(left.expect("both boundary runs exist"));
    let (right_run, right) = pop_first(right.expect("both boundary runs exist"));
    let combined = HeightRun {
        line_count: left_run
            .line_count
            .checked_add(right_run.line_count)
            .expect("public mutation validation prevents line-count overflow"),
        height: left_run.height,
        exact: left_run.exact,
    };
    join_with_run(left, combined, right)
}

fn concat_raw(left: Link, right: Link) -> Link {
    match (left, right) {
        (None, tree) | (tree, None) => tree,
        (Some(left), Some(right)) => {
            let (left, pivot) = pop_last(left);
            join_with_run(left, pivot, Some(right))
        }
    }
}

fn join_with_run(left: Link, run: HeightRun, right: Link) -> Link {
    let left_depth = subtree_depth(&left);
    let right_depth = subtree_depth(&right);
    if left_depth > right_depth.saturating_add(1) {
        let mut root = Arc::unwrap_or_clone(left.expect("a positive depth has a root"));
        root.right = join_with_run(root.right.take(), run, right);
        return Some(rebalance(root));
    }
    if right_depth > left_depth.saturating_add(1) {
        let mut root = Arc::unwrap_or_clone(right.expect("a positive depth has a root"));
        root.left = join_with_run(left, run, root.left.take());
        return Some(rebalance(root));
    }

    let mut root = Node::leaf(run);
    root.left = left;
    root.right = right;
    root.refresh();
    Some(Arc::new(root))
}

fn rebalance(mut root: Node) -> Arc<Node> {
    root.refresh();
    let left_depth = subtree_depth(&root.left);
    let right_depth = subtree_depth(&root.right);
    if left_depth > right_depth.saturating_add(1) {
        let left = root.left.as_mut().expect("an imbalanced side exists");
        if subtree_depth(&left.right) > subtree_depth(&left.left) {
            let child = root.left.take().expect("an imbalanced side exists");
            root.left = Some(rotate_left(Arc::unwrap_or_clone(child)));
        }
        return rotate_right(root);
    }
    if right_depth > left_depth.saturating_add(1) {
        let right = root.right.as_mut().expect("an imbalanced side exists");
        if subtree_depth(&right.left) > subtree_depth(&right.right) {
            let child = root.right.take().expect("an imbalanced side exists");
            root.right = Some(rotate_right(Arc::unwrap_or_clone(child)));
        }
        return rotate_left(root);
    }
    Arc::new(root)
}

fn rotate_left(mut root: Node) -> Arc<Node> {
    let pivot = root
        .right
        .take()
        .expect("a left rotation requires a right child");
    let mut pivot = Arc::unwrap_or_clone(pivot);
    root.right = pivot.left.take();
    root.refresh();
    pivot.left = Some(Arc::new(root));
    pivot.refresh();
    Arc::new(pivot)
}

fn rotate_right(mut root: Node) -> Arc<Node> {
    let pivot = root
        .left
        .take()
        .expect("a right rotation requires a left child");
    let mut pivot = Arc::unwrap_or_clone(pivot);
    root.left = pivot.right.take();
    root.refresh();
    pivot.right = Some(Arc::new(root));
    pivot.refresh();
    Arc::new(pivot)
}

fn pop_last(root: Arc<Node>) -> (Link, HeightRun) {
    let mut root = Arc::unwrap_or_clone(root);
    if let Some(right) = root.right.take() {
        let (new_right, run) = pop_last(right);
        root.right = new_right;
        (Some(rebalance(root)), run)
    } else {
        let left = root.left.take();
        (left, root.run)
    }
}

fn pop_first(root: Arc<Node>) -> (HeightRun, Link) {
    let mut root = Arc::unwrap_or_clone(root);
    if let Some(left) = root.left.take() {
        let (run, new_left) = pop_first(left);
        root.left = new_left;
        (run, Some(rebalance(root)))
    } else {
        let right = root.right.take();
        (root.run, right)
    }
}

fn leftmost_run(mut node: &Node) -> HeightRun {
    while let Some(left) = node.left.as_deref() {
        node = left;
    }
    node.run
}

fn rightmost_run(mut node: &Node) -> HeightRun {
    while let Some(right) = node.right.as_deref() {
        node = right;
    }
    node.run
}

fn measure_range(tree: &Link, start: usize, end: usize) -> InternalMeasurement {
    if start == end {
        return InternalMeasurement::EMPTY;
    }
    let node = tree
        .as_ref()
        .expect("a nonempty validated range has a containing node");
    debug_assert!(start <= end && end <= node.subtree_lines);
    if start == 0 && end == node.subtree_lines {
        return InternalMeasurement {
            line_count: node.subtree_lines,
            height: node.subtree_height,
            exact: node.subtree_exact,
        };
    }

    let left_lines = subtree_lines(&node.left);
    let run_end = left_lines
        .checked_add(node.run.line_count)
        .expect("node aggregates validate run boundaries");
    let mut result = InternalMeasurement::EMPTY;

    if start < left_lines {
        result.append(measure_range(&node.left, start, end.min(left_lines)));
    }

    let run_start_in_query = start.max(left_lines);
    let run_end_in_query = end.min(run_end);
    if run_start_in_query < run_end_in_query {
        let line_count = run_end_in_query - run_start_in_query;
        result.append(InternalMeasurement {
            line_count,
            height: checked_height_product(node.run.height, line_count)
                .expect("a run subset cannot exceed its validated total"),
            exact: node.run.exact,
        });
    }

    if end > run_end {
        result.append(measure_range(
            &node.right,
            start.saturating_sub(run_end),
            end - run_end,
        ));
    }
    result
}

fn validate_height(height: f64) -> Result<(), ViewHeightIndexError> {
    if !height.is_finite() || height <= 0.0 {
        Err(ViewHeightIndexError::InvalidHeight)
    } else {
        Ok(())
    }
}

fn checked_height_product(height: f64, count: usize) -> Result<f64, ViewHeightIndexError> {
    let result = height * count as f64;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(ViewHeightIndexError::HeightOverflow)
    }
}

fn checked_height_sum(left: f64, right: f64) -> Option<f64> {
    let result = left + right;
    result.is_finite().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        let tolerance = expected.abs().max(1.0) * 1.0e-10;
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual}"
        );
    }

    #[derive(Clone, Copy)]
    struct CheckedSubtree {
        lines: usize,
        height: f64,
        exact: bool,
        runs: usize,
        depth: usize,
        first: Option<HeightRun>,
        last: Option<HeightRun>,
    }

    fn assert_valid_tree(tree: &Link) -> CheckedSubtree {
        let Some(node) = tree else {
            return CheckedSubtree {
                lines: 0,
                height: 0.0,
                exact: true,
                runs: 0,
                depth: 0,
                first: None,
                last: None,
            };
        };
        let left = assert_valid_tree(&node.left);
        let right = assert_valid_tree(&node.right);
        assert!(left.depth.abs_diff(right.depth) <= 1, "AVL imbalance");
        if let Some(run) = left.last {
            assert!(!run.can_merge(node.run), "adjacent runs were not compacted");
        }
        if let Some(run) = right.first {
            assert!(!node.run.can_merge(run), "adjacent runs were not compacted");
        }

        let lines = left
            .lines
            .checked_add(node.run.line_count)
            .and_then(|count| count.checked_add(right.lines))
            .unwrap();
        let height = checked_height_sum(left.height, node.run.total_height())
            .and_then(|value| checked_height_sum(value, right.height))
            .unwrap();
        let runs = left.runs + 1 + right.runs;
        let depth = left.depth.max(right.depth) + 1;
        assert_eq!(node.subtree_lines, lines);
        assert_eq!(node.subtree_height.to_bits(), height.to_bits());
        assert_eq!(
            node.subtree_exact,
            left.exact && node.run.exact && right.exact
        );
        assert_eq!(node.subtree_runs, runs);
        assert_eq!(node.subtree_depth, depth);
        CheckedSubtree {
            lines,
            height,
            exact: node.subtree_exact,
            runs,
            depth,
            first: left.first.or(Some(node.run)),
            last: right.last.or(Some(node.run)),
        }
    }

    fn assert_valid_index(index: &ViewHeightIndex) {
        let checked = assert_valid_tree(&index.root);
        assert_eq!(checked.lines, index.hard_line_count());
        assert_eq!(checked.runs, index.statistics().run_count());
        assert_eq!(checked.depth, index.statistics().tree_depth());
    }

    #[test]
    fn one_million_estimated_lines_stay_compact_through_splices() {
        let mut index = ViewHeightIndex::new_estimated(1_000_000, 16.0).unwrap();
        assert_valid_index(&index);
        assert_eq!(
            index.statistics(),
            ViewHeightIndexStatistics {
                hard_line_count: 1_000_000,
                run_count: 1,
                tree_depth: 1,
            }
        );
        assert_eq!(index.total_height().height(), 16_000_000.0);
        assert!(!index.total_height().is_exact());

        index.set_exact_height(500_000, 24.0).unwrap();
        assert_valid_index(&index);
        assert_eq!(index.statistics().run_count(), 3);
        assert_eq!(index.prefix_height(500_001).unwrap().height(), 8_000_024.0);
        index.invalidate(500_000..500_001).unwrap();
        assert_valid_index(&index);
        assert_eq!(index.statistics().run_count(), 1);

        index.splice(400_000..400_100, 50).unwrap();
        assert_valid_index(&index);
        assert_eq!(index.hard_line_count(), 999_950);
        assert_eq!(index.statistics().run_count(), 1);
        assert_eq!(index.statistics().tree_depth(), 1);
    }

    #[test]
    fn exact_ranges_and_y_lookup_report_independent_certainty() {
        let mut index = ViewHeightIndex::new_estimated(5, 10.0).unwrap();
        index.set_exact_heights(1, &[20.0, 20.0, 30.0]).unwrap();
        assert_eq!(index.statistics().run_count(), 4);
        assert_eq!(
            index.range_height(1..4).unwrap(),
            HeightMeasurement {
                height: 70.0,
                exact: true,
            }
        );
        assert!(!index.prefix_height(4).unwrap().is_exact());

        let first = index.hard_line_at_y(0.0).unwrap().unwrap();
        assert_eq!(first.hard_line(), 0);
        assert!(first.prefix_is_exact());
        assert!(!first.line_is_exact());

        let second = index.hard_line_at_y(10.0).unwrap().unwrap();
        assert_eq!(second.hard_line(), 1);
        assert_eq!(second.line_top(), 10.0);
        assert_eq!(second.line_height(), 20.0);
        assert!(!second.prefix_is_exact());
        assert!(second.line_is_exact());
        assert_eq!(index.hard_line_at_y(100.0).unwrap(), None);
    }

    #[test]
    fn splice_preserves_exact_suffix_without_ordinal_rewrites() {
        let mut index = ViewHeightIndex::new_estimated(6, 10.0).unwrap();
        index
            .set_exact_heights(0, &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0])
            .unwrap();
        index.splice(2..4, 3).unwrap();
        assert_valid_index(&index);
        assert_eq!(index.hard_line_count(), 7);
        assert_eq!(index.range_height(0..2).unwrap().height(), 30.0);
        assert_eq!(index.range_height(2..5).unwrap().height(), 30.0);
        assert!(!index.range_height(2..5).unwrap().is_exact());
        assert_eq!(index.range_height(5..7).unwrap().height(), 110.0);
        assert!(index.range_height(5..7).unwrap().is_exact());
        assert_eq!(index.hard_line_at_y(60.0).unwrap().unwrap().hard_line(), 5);
    }

    #[test]
    fn zero_line_and_extreme_height_boundaries_are_checked_atomically() {
        assert_eq!(
            ViewHeightIndex::new_estimated(0, 0.0).unwrap_err(),
            ViewHeightIndexError::InvalidHeight
        );
        let mut empty = ViewHeightIndex::new_estimated(0, 1.0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.total_height().height(), 0.0);
        assert!(empty.total_height().is_exact());
        assert_eq!(empty.prefix_height(0).unwrap().height(), 0.0);
        assert_eq!(empty.hard_line_at_y(0.0).unwrap(), None);

        empty.splice(0..0, 1).unwrap();
        assert_eq!(empty.hard_line_count(), 1);
        empty.set_exact_height(0, f64::MAX / 4.0).unwrap();
        let hit = empty.hard_line_at_y(f64::MAX / 8.0).unwrap().unwrap();
        assert_eq!(hit.hard_line(), 0);
        assert!(hit.line_is_exact());

        assert_eq!(
            ViewHeightIndex::new_estimated(2, f64::MAX).unwrap_err(),
            ViewHeightIndexError::HeightOverflow
        );
        let mut two = ViewHeightIndex::new_estimated(2, 1.0).unwrap();
        let pristine = two.total_height();
        assert_eq!(
            two.set_exact_height(0, f64::INFINITY),
            Err(ViewHeightIndexError::InvalidHeight)
        );
        assert_eq!(two.total_height(), pristine);
        two.set_exact_height(0, f64::MAX).unwrap();
        let before = two.total_height();
        assert_eq!(
            two.set_exact_height(1, f64::MAX),
            Err(ViewHeightIndexError::HeightOverflow)
        );
        assert_eq!(two.total_height(), before);
        assert_eq!(two.statistics().run_count(), 2);

        assert_eq!(
            two.set_exact_height(2, 1.0),
            Err(ViewHeightIndexError::HardLineOutsideIndex {
                hard_line: 2,
                hard_line_count: 2,
            })
        );
        let inverted_start = 2;
        let inverted_end = 1;
        assert_eq!(
            two.invalidate(inverted_start..inverted_end),
            Err(ViewHeightIndexError::RangeOutsideIndex {
                start: 2,
                end: 1,
                hard_line_count: 2,
            })
        );
        assert_eq!(
            two.hard_line_at_y(f64::NAN),
            Err(ViewHeightIndexError::InvalidVerticalCoordinate)
        );

        let mut maximum_count = ViewHeightIndex::new_estimated(usize::MAX, 1.0).unwrap();
        assert_eq!(
            maximum_count.splice(0..0, 1),
            Err(ViewHeightIndexError::LineCountOverflow)
        );
        assert_eq!(maximum_count.hard_line_count(), usize::MAX);
    }

    #[derive(Clone, Copy)]
    struct OracleLine {
        height: f64,
        exact: bool,
    }

    fn oracle_measure(lines: &[OracleLine], range: Range<usize>) -> HeightMeasurement {
        HeightMeasurement {
            height: lines[range.clone()].iter().map(|line| line.height).sum(),
            exact: lines[range].iter().all(|line| line.exact),
        }
    }

    fn oracle_hit(lines: &[OracleLine], y: f64) -> Option<HardLineHeightHit> {
        let mut top = 0.0;
        let mut prefix_is_exact = true;
        for (hard_line, line) in lines.iter().enumerate() {
            if y < top + line.height {
                return Some(HardLineHeightHit {
                    hard_line,
                    line_top: top,
                    line_height: line.height,
                    prefix_is_exact,
                    line_is_exact: line.exact,
                });
            }
            top += line.height;
            prefix_is_exact &= line.exact;
        }
        None
    }

    fn next_random(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *state
    }

    #[test]
    fn randomized_updates_splices_and_queries_match_flat_oracle() {
        const ESTIMATE: f64 = 10.0;
        let mut index = ViewHeightIndex::new_estimated(32, ESTIMATE).unwrap();
        let mut oracle = vec![
            OracleLine {
                height: ESTIMATE,
                exact: false,
            };
            32
        ];
        let mut random = 0x1234_5678_9abc_def0;

        for _ in 0..4_000 {
            let choice = next_random(&mut random) % 5;
            match choice {
                0 if !oracle.is_empty() => {
                    let line = next_random(&mut random) as usize % oracle.len();
                    let height = (next_random(&mut random) % 40 + 1) as f64;
                    index.set_exact_height(line, height).unwrap();
                    oracle[line] = OracleLine {
                        height,
                        exact: true,
                    };
                }
                1 if !oracle.is_empty() => {
                    let first = next_random(&mut random) as usize % (oracle.len() + 1);
                    let second = next_random(&mut random) as usize % (oracle.len() + 1);
                    let range = first.min(second)..first.max(second);
                    index.invalidate(range.clone()).unwrap();
                    for line in &mut oracle[range] {
                        *line = OracleLine {
                            height: ESTIMATE,
                            exact: false,
                        };
                    }
                }
                2 => {
                    let start = next_random(&mut random) as usize % (oracle.len() + 1);
                    let available = oracle.len() - start;
                    let removed = (next_random(&mut random) as usize % 4).min(available);
                    let inserted = next_random(&mut random) as usize % 4;
                    index.splice(start..start + removed, inserted).unwrap();
                    oracle.splice(
                        start..start + removed,
                        std::iter::repeat(OracleLine {
                            height: ESTIMATE,
                            exact: false,
                        })
                        .take(inserted),
                    );
                }
                3 if !oracle.is_empty() => {
                    let start = next_random(&mut random) as usize % oracle.len();
                    let count = (next_random(&mut random) as usize % 5)
                        .min(oracle.len() - start)
                        .max(1);
                    let heights: Vec<_> = (0..count)
                        .map(|_| (next_random(&mut random) % 30 + 1) as f64)
                        .collect();
                    index.set_exact_heights(start, &heights).unwrap();
                    for (line, height) in oracle[start..start + count].iter_mut().zip(heights) {
                        *line = OracleLine {
                            height,
                            exact: true,
                        };
                    }
                }
                _ => {}
            }

            assert_eq!(index.hard_line_count(), oracle.len());
            let end = next_random(&mut random) as usize % (oracle.len() + 1);
            let expected_prefix = oracle_measure(&oracle, 0..end);
            let actual_prefix = index.prefix_height(end).unwrap();
            assert_close(actual_prefix.height(), expected_prefix.height());
            assert_eq!(actual_prefix.is_exact(), expected_prefix.is_exact());

            let first = next_random(&mut random) as usize % (oracle.len() + 1);
            let second = next_random(&mut random) as usize % (oracle.len() + 1);
            let range = first.min(second)..first.max(second);
            let expected_range = oracle_measure(&oracle, range.clone());
            let actual_range = index.range_height(range).unwrap();
            assert_close(actual_range.height(), expected_range.height());
            assert_eq!(actual_range.is_exact(), expected_range.is_exact());

            let total = oracle_measure(&oracle, 0..oracle.len()).height();
            let y = if total == 0.0 {
                0.0
            } else {
                total * (next_random(&mut random) as u32 as f64 / u32::MAX as f64)
            };
            let actual_hit = index.hard_line_at_y(y).unwrap();
            let expected_hit = oracle_hit(&oracle, y);
            assert_eq!(
                actual_hit.map(HardLineHeightHit::hard_line),
                expected_hit.map(HardLineHeightHit::hard_line)
            );
            if let (Some(actual), Some(expected)) = (actual_hit, expected_hit) {
                assert_close(actual.line_top(), expected.line_top());
                assert_eq!(actual.line_is_exact(), expected.line_is_exact());
                assert_eq!(actual.prefix_is_exact(), expected.prefix_is_exact());
            }

            // Structural instrumentation guards the logarithmic shape in
            // addition to the recursive AVL invariant check below.
            let stats = index.statistics();
            assert!(stats.run_count() <= oracle.len().max(1));
            let logarithmic_bound = if stats.run_count() == 0 {
                0
            } else {
                2 * (usize::BITS as usize - stats.run_count().leading_zeros() as usize)
            };
            assert!(stats.tree_depth() <= logarithmic_bound);
            assert_valid_index(&index);
        }
    }
}
