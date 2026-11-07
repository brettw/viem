//! Bounded Myers difference of two interned sequences.
//!
//! The result is always an exact alignment: applying its runs to the base
//! reproduces the new sequence. When the work budget runs out, the remaining
//! subproblem becomes a single change run instead of a minimal one, so the
//! cost is bounded without ever producing an invalid diff.

/// One aligned run of a sequence difference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Run {
    /// Elements that are identical on both sides.
    Equal(usize),
    /// `base` elements replaced by `new` elements; either may be zero.
    Change { base: usize, new: usize },
}

impl Run {
    pub fn base_len(self) -> usize {
        match self {
            Run::Equal(length) => length,
            Run::Change { base, .. } => base,
        }
    }

    pub fn new_len(self) -> usize {
        match self {
            Run::Equal(length) => length,
            Run::Change { new, .. } => new,
        }
    }
}

/// Appends runs, merging neighbours of the same kind and dropping empty runs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunBuilder {
    runs: Vec<Run>,
}

impl RunBuilder {
    pub fn push(&mut self, run: Run) {
        match run {
            Run::Equal(0) | Run::Change { base: 0, new: 0 } => return,
            _ => {}
        }
        match (self.runs.last_mut(), run) {
            (Some(Run::Equal(length)), Run::Equal(more)) => *length += more,
            (Some(Run::Change { base, new }), Run::Change { base: more_base, new: more_new }) => {
                *base += more_base;
                *new += more_new;
            }
            _ => self.runs.push(run),
        }
    }

    pub fn finish(self) -> Vec<Run> {
        self.runs
    }
}

/// Remaining comparison work. Each element comparison or diagonal step spends
/// one unit; an exhausted budget turns the current subproblem into a change.
#[derive(Clone, Copy, Debug)]
pub struct WorkBudget {
    remaining: u64,
}

impl WorkBudget {
    pub fn new(units: u64) -> Self {
        Self { remaining: units }
    }

    fn spend(&mut self, units: u64) -> bool {
        if self.remaining < units {
            self.remaining = 0;
            false
        } else {
            self.remaining -= units;
            true
        }
    }

    pub fn exhausted(&self) -> bool {
        self.remaining == 0
    }
}

/// Exact difference of `base` and `new` within `budget`.
pub fn diff<T: Eq>(base: &[T], new: &[T], budget: &mut WorkBudget) -> Vec<Run> {
    let mut builder = RunBuilder::default();
    diff_into(base, new, budget, &mut builder);
    builder.finish()
}

pub fn diff_into<T: Eq>(base: &[T], new: &[T], budget: &mut WorkBudget, builder: &mut RunBuilder) {
    // Iterative over the split stack so deep recursion never depends on input.
    let mut stack = vec![(0..base.len(), 0..new.len())];
    // Splits are processed left to right: pushing the right half first keeps
    // the output ordered.
    while let Some((a, b)) = stack.pop() {
        let (a_slice, b_slice) = (&base[a.clone()], &new[b.clone()]);
        let prefix = common_prefix(a_slice, b_slice);
        if !budget.spend(prefix as u64 + 1) {
            builder.push(Run::Equal(prefix));
            builder.push(Run::Change { base: a.len() - prefix, new: b.len() - prefix });
            continue;
        }
        let a_rest = &a_slice[prefix..];
        let b_rest = &b_slice[prefix..];
        let suffix = common_suffix(a_rest, b_rest);
        budget.spend(suffix as u64);
        let a_mid = &a_rest[..a_rest.len() - suffix];
        let b_mid = &b_rest[..b_rest.len() - suffix];
        if a_mid.is_empty() || b_mid.is_empty() {
            builder.push(Run::Equal(prefix));
            builder.push(Run::Change { base: a_mid.len(), new: b_mid.len() });
            builder.push(Run::Equal(suffix));
            continue;
        }
        match middle_snake(a_mid, b_mid, budget) {
            Some((x, y)) if (x, y) != (0, 0) && (x, y) != (a_mid.len(), b_mid.len()) => {
                let a_start = a.start + prefix;
                let b_start = b.start + prefix;
                // Emit the prefix now; the two halves and the suffix follow.
                builder.push(Run::Equal(prefix));
                let suffix_start_a = a_start + a_mid.len();
                let suffix_start_b = b_start + b_mid.len();
                // Suffix is an equal run; represent it as a trivial frame.
                stack.push((suffix_start_a..suffix_start_a + suffix, suffix_start_b..suffix_start_b + suffix));
                stack.push((a_start + x..suffix_start_a, b_start + y..suffix_start_b));
                stack.push((a_start..a_start + x, b_start..b_start + y));
            }
            _ => {
                builder.push(Run::Equal(prefix));
                builder.push(Run::Change { base: a_mid.len(), new: b_mid.len() });
                builder.push(Run::Equal(suffix));
            }
        }
    }
}

fn common_prefix<T: Eq>(a: &[T], b: &[T]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix<T: Eq>(a: &[T], b: &[T]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}

/// Myers' bidirectional search for a split point on an optimal path. Both
/// inputs are nonempty and share no common prefix or suffix. `None` means
/// the budget ran out or the sequences share nothing.
fn middle_snake<T: Eq>(a: &[T], b: &[T], budget: &mut WorkBudget) -> Option<(usize, usize)> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    let max_d = (n + m + 1) / 2;
    let offset = max_d;
    let length = (2 * max_d + 2) as usize;
    let mut forward = vec![-1isize; length];
    let mut backward = vec![-1isize; length];
    forward[(offset + 1) as usize] = 0;
    backward[(offset + 1) as usize] = 0;
    let delta = n - m;
    let front = delta % 2 != 0;
    let (mut k1_start, mut k1_end, mut k2_start, mut k2_end) = (0isize, 0isize, 0isize, 0isize);
    for d in 0..max_d {
        if !budget.spend(2 * d as u64 + 2) {
            return None;
        }
        let mut k1 = -d + k1_start;
        while k1 <= d - k1_end {
            let index = (offset + k1) as usize;
            let mut x1 = if k1 == -d || (k1 != d && forward[index - 1] < forward[index + 1]) {
                forward[index + 1]
            } else {
                forward[index - 1] + 1
            };
            let mut y1 = x1 - k1;
            let start = x1;
            while x1 < n && y1 < m && a[x1 as usize] == b[y1 as usize] {
                x1 += 1;
                y1 += 1;
            }
            if !budget.spend((x1 - start) as u64) {
                return None;
            }
            forward[index] = x1;
            if x1 > n {
                k1_end += 2;
            } else if y1 > m {
                k1_start += 2;
            } else if front {
                let other = offset + delta - k1;
                if other >= 0 && (other as usize) < length && backward[other as usize] != -1 {
                    let x2 = n - backward[other as usize];
                    if x1 >= x2 {
                        return Some((x1 as usize, y1 as usize));
                    }
                }
            }
            k1 += 2;
        }
        let mut k2 = -d + k2_start;
        while k2 <= d - k2_end {
            let index = (offset + k2) as usize;
            let mut x2 = if k2 == -d || (k2 != d && backward[index - 1] < backward[index + 1]) {
                backward[index + 1]
            } else {
                backward[index - 1] + 1
            };
            let mut y2 = x2 - k2;
            let start = x2;
            while x2 < n && y2 < m && a[(n - x2 - 1) as usize] == b[(m - y2 - 1) as usize] {
                x2 += 1;
                y2 += 1;
            }
            if !budget.spend((x2 - start) as u64) {
                return None;
            }
            backward[index] = x2;
            if x2 > n {
                k2_end += 2;
            } else if y2 > m {
                k2_start += 2;
            } else if !front {
                let other = offset + delta - k2;
                if other >= 0 && (other as usize) < length && forward[other as usize] != -1 {
                    let x1 = forward[other as usize];
                    let y1 = offset + x1 - other;
                    if x1 >= n - x2 {
                        return Some((x1 as usize, y1 as usize));
                    }
                }
            }
            k2 += 2;
        }
    }
    None
}

/// Replays `runs` over `base` and checks that it reproduces `new` exactly.
pub fn runs_reproduce<T: Eq>(runs: &[Run], base: &[T], new: &[T]) -> bool {
    let (mut x, mut y) = (0, 0);
    for run in runs {
        match *run {
            Run::Equal(length) => {
                if x + length > base.len() || y + length > new.len() || base[x..x + length] != new[y..y + length] {
                    return false;
                }
                x += length;
                y += length;
            }
            Run::Change { base: removed, new: added } => {
                x += removed;
                y += added;
            }
        }
    }
    x == base.len() && y == new.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn edit_cost(runs: &[Run]) -> usize {
        runs.iter().map(|run| match run {
            Run::Equal(_) => 0,
            Run::Change { base, new } => base + new,
        }).sum()
    }

    #[test]
    fn identical_sequences_are_one_equal_run() {
        let a = chars("abcdef");
        assert_eq!(diff(&a, &a, &mut WorkBudget::new(1_000)), vec![Run::Equal(6)]);
    }

    #[test]
    fn empty_sides_are_pure_insertions_or_deletions() {
        let a = chars("abc");
        let empty: Vec<char> = Vec::new();
        assert_eq!(diff(&empty, &a, &mut WorkBudget::new(100)), vec![Run::Change { base: 0, new: 3 }]);
        assert_eq!(diff(&a, &empty, &mut WorkBudget::new(100)), vec![Run::Change { base: 3, new: 0 }]);
        assert_eq!(diff(&empty, &empty, &mut WorkBudget::new(100)), Vec::<Run>::new());
    }

    #[test]
    fn finds_a_minimal_alignment() {
        // The classic Myers example: ABCABBA -> CBABAC has distance 5.
        let a = chars("ABCABBA");
        let b = chars("CBABAC");
        let runs = diff(&a, &b, &mut WorkBudget::new(10_000));
        assert!(runs_reproduce(&runs, &a, &b));
        assert_eq!(edit_cost(&runs), 5);
    }

    #[test]
    fn exhausted_budget_still_reproduces_the_new_sequence() {
        let a = chars("the quick brown fox jumps over the lazy dog");
        let b = chars("a quick brown cat leaps over one lazy dog!");
        for units in [0, 1, 5, 20, 100, 10_000] {
            let runs = diff(&a, &b, &mut WorkBudget::new(units));
            assert!(runs_reproduce(&runs, &a, &b), "budget {units}: {runs:?}");
        }
    }

    #[test]
    fn deterministic_pseudo_random_inputs_reproduce_and_are_minimal_for_small_cases() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let a: Vec<u8> = (0..next() % 12).map(|_| (next() % 3) as u8).collect();
            let b: Vec<u8> = (0..next() % 12).map(|_| (next() % 3) as u8).collect();
            let runs = diff(&a, &b, &mut WorkBudget::new(u64::MAX));
            assert!(runs_reproduce(&runs, &a, &b), "{a:?} {b:?} {runs:?}");
            assert_eq!(edit_cost(&runs), a.len() + b.len() - 2 * lcs(&a, &b), "{a:?} {b:?} {runs:?}");
        }
    }

    fn lcs(a: &[u8], b: &[u8]) -> usize {
        let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 0..a.len() {
            for j in 0..b.len() {
                table[i + 1][j + 1] = if a[i] == b[j] { table[i][j] + 1 } else { table[i][j + 1].max(table[i + 1][j]) };
            }
        }
        table[a.len()][b.len()]
    }
}
