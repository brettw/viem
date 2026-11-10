//! The live line diff between a base and an edited new side.
//!
//! The diff is a list of chunks, alternating between runs of equal lines and
//! changed regions. An edit re-diffs only a window: the edited lines plus any
//! changed chunk they touch, cut out of the surrounding equal runs. Because
//! the window follows the edit, an inserted line is reported exactly where it
//! was typed, which may differ from a from-scratch diff of the same texts.
//! Every result is exact: replaying the chunks over the base reproduces the
//! new side.

use super::inline::{inline_spans, similarity, InlineSpans};
use super::presentation::Side;
use super::sequence::{diff, Run, WorkBudget};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

/// Read access to one side's lines. A line excludes its terminator.
pub trait LineSource {
    fn line_count(&self) -> usize;
    fn line(&self, index: usize) -> Cow<'_, str>;
}

/// An owned text split at LF. A final LF ends its line and is followed by an
/// empty last line, matching hard-line numbering.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextLines {
    text: String,
    starts: Vec<usize>,
}

impl TextLines {
    pub fn new(text: String) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(offset, _)| offset + 1));
        Self { text, starts }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl LineSource for TextLines {
    fn line_count(&self) -> usize {
        self.starts.len()
    }

    fn line(&self, index: usize) -> Cow<'_, str> {
        let start = self.starts[index];
        let end = self.starts.get(index + 1).map_or(self.text.len(), |next| next - 1);
        Cow::Borrowed(&self.text[start..end])
    }
}

/// Bounds on the work one diff operation may do. Exceeding a bound makes the
/// affected region one changed chunk, which is still exact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffLimits {
    /// Most lines (both sides together) an edit re-diffs line by line.
    pub window_lines: usize,
    /// Comparison work per line diff.
    pub work_units: u64,
    /// Most base-line by new-line similarity comparisons used to pair the
    /// lines of one changed chunk; larger chunks pair lines in order.
    pub pairing_cells: usize,
    /// Changed chunks with more lines than this on either side get no
    /// within-line detail.
    pub inline_lines: usize,
}

impl Default for DiffLimits {
    fn default() -> Self {
        Self { window_lines: 20_000, work_units: 20_000_000, pairing_cells: 400, inline_lines: 2_000 }
    }
}

/// Two corresponding lines inside a changed chunk, with their changed spans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinePair {
    /// Line offsets within the chunk.
    pub base: usize,
    pub new: usize,
    pub spans: InlineSpans,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Chunk {
    base: usize,
    new: usize,
    equal: bool,
    pairs: Arc<[LinePair]>,
}

/// The part of a changed chunk between its last unaffected line pair before
/// an edit and its first unaffected pair after it, in chunk-local coordinates.
struct PairWindow {
    base: Range<usize>,
    new: Range<usize>,
    pairs: Range<usize>,
}

impl Chunk {
    fn equal(length: usize) -> Self {
        Self { base: length, new: length, equal: true, pairs: Arc::from([]) }
    }

    /// An existing pair whose base line is the only candidate between its
    /// surviving neighbours. A one-line edit can retain that correspondence.
    fn bounded_pair(&self, offset: usize) -> Option<usize> {
        let index = self.pairs.partition_point(|pair| pair.new < offset);
        let pair = self.pairs.get(index).filter(|pair| pair.new == offset)?;
        let preceding = index.checked_sub(1).map_or(0, |i| self.pairs[i].base + 1);
        let following = self.pairs.get(index + 1).map_or(self.base, |next| next.base);
        (preceding == pair.base && following == pair.base + 1).then_some(index)
    }

    fn pair_window(&self, edit: Range<usize>) -> Option<PairWindow> {
        let first = self.pairs.partition_point(|pair| pair.new < edit.start);
        let last = self.pairs.partition_point(|pair| pair.new < edit.end);
        let before = first.checked_sub(1).map(|i| &self.pairs[i]);
        let after = self.pairs.get(last);
        if before.is_none() && after.is_none() { return None; }
        Some(PairWindow {
            base: before.map_or(0, |p| p.base + 1)..after.map_or(self.base, |p| p.base),
            new: before.map_or(0, |p| p.new + 1)..after.map_or(self.new, |p| p.new),
            pairs: first..last,
        })
    }

    fn changed_part(base: Range<usize>, new: Range<usize>, pairs: &[LinePair]) -> Self {
        Self {
            base: base.len(), new: new.len(), equal: false,
            pairs: pairs.iter().map(|pair| LinePair { base: pair.base - base.start,
                new: pair.new - new.start, spans: pair.spans.clone() }).collect(),
        }
    }
}

/// Base context that proves a one-line edit can retain the surrounding diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DiffLineBasis<T> {
    Added,
    /// Exactly one base and one live line. Only equality changes the hunk.
    Single(T),
    Paired(T),
}

/// Inline spans for a retained changed-line pair. Equality can split the
/// hunk; insufficient similarity can require a different correspondence.
pub(crate) fn retained_pair_spans(base: &str, new: &str) -> Option<InlineSpans> {
    (base != new && similarity(base, new) >= PAIR_SIMILARITY).then(|| inline_spans(base, new))
}

/// One changed region in both sides' line numbering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hunk {
    pub base: Range<usize>,
    pub new: Range<usize>,
    pub pairs: Arc<[LinePair]>,
}

impl Hunk {
    pub fn side(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Base => &self.base,
            Side::New => &self.new,
        }
    }
}

/// Replace new-side lines `start..old_end` (numbered before the edit) with
/// lines `start..new_end` (numbered after it).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl LineEdit {
    /// Whether this edit turns a side of `old_count` lines into one of
    /// `new_count` lines.
    pub fn fits(self, old_count: usize, new_count: usize) -> bool {
        self.start <= self.old_end
            && self.start <= self.new_end
            && self.old_end <= old_count
            && old_count - (self.old_end - self.start) + (self.new_end - self.start) == new_count
    }

    fn shift(self, line: usize) -> usize {
        line - self.old_end + self.new_end
    }

    /// Where a line moves: lines before the edit keep their number and
    /// lines after it shift. A replaced line has none.
    pub fn map_line(self, line: usize) -> Option<usize> {
        if line < self.start {
            Some(line)
        } else if line >= self.old_end {
            Some(self.shift(line))
        } else {
            None
        }
    }

    /// Where the boundary before `line` moves. The edit's own start
    /// boundary moves with the lines after the edit when it inserts there;
    /// a boundary between replaced lines has none.
    pub fn map_boundary(self, line: usize) -> Option<usize> {
        if line >= self.old_end {
            Some(self.shift(line))
        } else if line <= self.start {
            Some(line)
        } else {
            None
        }
    }

    /// `lines` moved whole, or `None` when the edit replaces or splits any
    /// of it.
    pub fn map_range(self, lines: Range<usize>) -> Option<Range<usize>> {
        if lines.end <= self.start {
            Some(lines)
        } else if lines.start >= self.old_end {
            Some(self.shift(lines.start)..self.shift(lines.end))
        } else {
            None
        }
    }

    /// The nonempty parts of `lines` before and after the replaced lines,
    /// moved.
    pub fn map_surviving(self, lines: Range<usize>) -> impl Iterator<Item = Range<usize>> {
        let before = lines.start..lines.end.min(self.start);
        let after = lines.start.max(self.old_end)..lines.end;
        [before, after].into_iter().filter(|part| !part.is_empty()).filter_map(move |part| self.map_range(part))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffState {
    chunks: Vec<Chunk>,
    base_lines: usize,
    new_lines: usize,
    limits: DiffLimits,
}

impl DiffState {
    /// A from-scratch diff of the two sides.
    pub fn compute(base: &dyn LineSource, new: &dyn LineSource, limits: DiffLimits) -> Self {
        let base_lines = base.line_count();
        let new_lines = new.line_count();
        let chunks = diff_region(base, 0..base_lines, new, 0..new_lines, limits, false);
        Self { chunks, base_lines, new_lines, limits }
    }

    pub fn base_line_count(&self) -> usize {
        self.base_lines
    }

    pub fn new_line_count(&self) -> usize {
        self.new_lines
    }

    pub(crate) fn line_basis(&self, line: usize) -> Option<DiffLineBasis<usize>> {
        let (mut x, mut y) = (0, 0);
        for chunk in &self.chunks {
            if line < y + chunk.new {
                if chunk.equal { return None; }
                if chunk.base == 0 { return Some(DiffLineBasis::Added); }
                if chunk.base == 1 && chunk.new == 1 && self.limits.inline_lines > 0 { return Some(DiffLineBasis::Single(x)); }
                let offset = line - y;
                if chunk.pair_window(offset..offset + 1).is_some_and(|window| window.base.is_empty()) {
                    return Some(DiffLineBasis::Added);
                }
                let index = chunk.bounded_pair(offset)?;
                return Some(DiffLineBasis::Paired(x + chunk.pairs[index].base));
            }
            x += chunk.base;
            y += chunk.new;
        }
        None
    }

    /// Update the diff after one edit to the new side. `new` is the new side
    /// after the edit. An edit that doesn't fit the current line count is
    /// treated as unknown and recomputes from scratch.
    pub fn apply_edit(&mut self, base: &dyn LineSource, new: &dyn LineSource, edit: LineEdit) {
        let LineEdit { start, old_end, new_end } = edit;
        if !edit.fits(self.new_lines, new.line_count()) || base.line_count() != self.base_lines {
            *self = Self::compute(base, new, self.limits);
            return;
        }
        // Grow the window over every changed chunk it touches, including a
        // changed chunk ending where the window begins (or starting where it
        // ends), so changed chunks never end up adjacent.
        let (mut window_start, mut window_end) = (start, old_end);
        let (mut base_position, mut position) = (0, 0);
        for index in 0..self.chunks.len() {
            let end = position + self.chunks[index].new;
            if !self.chunks[index].equal && position <= start && old_end <= end
                && self.edit_in_hunk(index, (base_position, position), base, new, edit)
            {
                self.new_lines = new.line_count();
                return;
            }
            let chunk = &self.chunks[index];
            if !chunk.equal && position <= window_end && window_start <= end {
                window_start = window_start.min(position);
                window_end = window_end.max(end);
            }
            position = end;
            base_position += chunk.base;
        }
        // Map the window's edges to base lines and split the chunk list.
        let mut before = Vec::new();
        let mut after = Vec::new();
        let (mut base_start, mut base_end): (Option<usize>, Option<usize>) = (None, None);
        let (mut base_position, mut new_position) = (0, 0);
        for chunk in &self.chunks {
            let (new_chunk_end, base_chunk_end) = (new_position + chunk.new, base_position + chunk.base);
            if chunk.equal {
                // The parts before and after the window survive unchanged;
                // the window's edges map through the equal offset.
                if new_position < window_start {
                    before.push(Chunk::equal(window_start.min(new_chunk_end) - new_position));
                }
                if (new_position..=new_chunk_end).contains(&window_start) {
                    let mapped = base_position + (window_start - new_position);
                    base_start = Some(base_start.map_or(mapped, |current| current.min(mapped)));
                }
                if (new_position..=new_chunk_end).contains(&window_end) {
                    let mapped = base_position + (window_end - new_position);
                    base_end = Some(base_end.map_or(mapped, |current| current.max(mapped)));
                }
                if new_chunk_end > window_end {
                    after.push(Chunk::equal(new_chunk_end - window_end.max(new_position)));
                }
            } else if new_position <= window_end && window_start <= new_chunk_end {
                // Inside the window, including a pure deletion at its edge.
                base_start = Some(base_start.map_or(base_position, |current| current.min(base_position)));
                base_end = Some(base_end.map_or(base_chunk_end, |current| current.max(base_chunk_end)));
            } else if new_chunk_end < window_start {
                before.push(chunk.clone());
            } else {
                after.push(chunk.clone());
            }
            base_position = base_chunk_end;
            new_position = new_chunk_end;
        }
        let base_start = base_start.unwrap_or(base_position);
        let base_end = base_end.unwrap_or(base_position).max(base_start);
        let new_window = window_start..window_end + new_end - old_end;
        let middle = diff_region(base, base_start..base_end, new, new_window, self.limits, true);
        let mut chunks = Vec::with_capacity(before.len() + middle.len() + after.len());
        for chunk in before.into_iter().chain(middle).chain(after) {
            push_chunk(&mut chunks, chunk);
        }
        self.chunks = chunks;
        self.new_lines = new.line_count();
    }

    /// Apply an edit within a single changed hunk when its surviving
    /// correspondences bound the work. Otherwise re-diff the enclosing hunk.
    fn edit_in_hunk(&mut self, index: usize, origin: (usize, usize), base: &dyn LineSource,
        new: &dyn LineSource, edit: LineEdit) -> bool {
        let (base_start, new_start) = origin;
        let chunk = &mut self.chunks[index];
        let removed = edit.old_end - edit.start;
        let added = edit.new_end - edit.start;
        let new_length = chunk.new - removed + added;
        if chunk.base == 0 {
            // Eliminating the entire addition needs the equal runs joined.
            if new_length == 0 { return false; }
            chunk.new = new_length;
            return true;
        }
        if new_length > self.limits.inline_lines { return false; }
        let start = edit.start - new_start;
        if removed == 1 && added == 1 {
            if chunk.base == 1 && chunk.new == 1 {
                let a = base.line(base_start);
                let b = new.line(edit.start);
                if a != b {
                    chunk.pairs = retained_pair_spans(&a, &b).map(|spans| LinePair { base: 0, new: 0, spans })
                        .into_iter().collect();
                    return true;
                }
            } else if let Some(index) = chunk.bounded_pair(start) {
                let a = base.line(base_start + chunk.pairs[index].base);
                let b = new.line(edit.start);
                if let Some(spans) = retained_pair_spans(&a, &b) {
                    Arc::make_mut(&mut chunk.pairs)[index].spans = spans;
                    return true;
                }
            }
        }
        let Some(window) = chunk.pair_window(start..start + removed) else { return false };
        // The text of both neighbouring pairs survived. Retain them as
        // correspondences while rechecking possible equalities in the gap.
        if !window.base.is_empty() || removed != added {
            let new_end = new_start + window.new.end - removed + added;
            let local = diff_region(base, base_start + window.base.start..base_start + window.base.end,
                new, new_start + window.new.start..new_end, self.limits, true);
            self.replace_pair_window(index, window, local);
        }
        true
    }

    /// An equal line rediscovered in the gap splits the hunk. Retained spans
    /// follow their line pairs into the resulting left and right hunks.
    fn replace_pair_window(&mut self, index: usize, window: PairWindow, middle: Vec<Chunk>) {
        let chunk = &mut self.chunks[index];
        if let [local] = middle.as_slice() {
            if !local.equal && local.pairs.len() == window.pairs.len() {
                // Splitting a paired line often just adds an unpaired row.
                // Keep the allocations for every surviving pair and its spans.
                let pairs = Arc::make_mut(&mut chunk.pairs);
                for (slot, pair) in pairs[window.pairs.clone()].iter_mut().zip(local.pairs.iter()) {
                    *slot = LinePair { base: window.base.start + pair.base, new: window.new.start + pair.new,
                        spans: pair.spans.clone() };
                }
                if local.new != window.new.len() {
                    for pair in &mut pairs[window.pairs.end..] {
                        pair.new = pair.new - window.new.len() + local.new;
                    }
                    chunk.new = chunk.new - window.new.len() + local.new;
                }
                return;
            }
        }
        let prefix = Chunk::changed_part(0..window.base.start, 0..window.new.start, &chunk.pairs[..window.pairs.start]);
        let suffix = Chunk::changed_part(window.base.end..chunk.base, window.new.end..chunk.new, &chunk.pairs[window.pairs.end..]);
        let mut replacement = Vec::with_capacity(middle.len() + 4);
        // Include neighbouring equal runs so equalities at either edge merge.
        let start = index.saturating_sub(1);
        let end = (index + 2).min(self.chunks.len());
        if start < index { push_chunk(&mut replacement, self.chunks[start].clone()); }
        push_chunk(&mut replacement, prefix);
        for chunk in middle { push_chunk(&mut replacement, chunk); }
        push_chunk(&mut replacement, suffix);
        if index + 1 < end { push_chunk(&mut replacement, self.chunks[index + 1].clone()); }
        self.chunks.splice(start..end, replacement);
    }

    /// Count physical lines. The layout diff compares delimiter-free lines and
    /// includes the empty insertion boundary at EOF. Neither convention applies
    /// to patch statistics: even a change to the final LF replaces a line.
    pub fn line_totals(&self, base_ends_empty: bool, new_ends_empty: bool) -> (usize, usize) {
        let base_lines = self.base_lines.saturating_sub(usize::from(base_ends_empty));
        let new_lines = self.new_lines.saturating_sub(usize::from(new_ends_empty));
        let (mut base, mut new, mut equal) = (0, 0, 0);
        for chunk in &self.chunks {
            if chunk.equal {
                let matched = chunk.base.min(base_lines.saturating_sub(base)).min(new_lines.saturating_sub(new));
                let base_unterminated = !base_ends_empty && base + matched == base_lines;
                let new_unterminated = !new_ends_empty && new + matched == new_lines;
                equal += matched.saturating_sub(usize::from(base_unterminated != new_unterminated));
            }
            base += chunk.base;
            new += chunk.new;
        }
        (new_lines - equal, base_lines - equal)
    }

    /// The changed regions in order.
    pub fn hunks(&self) -> impl Iterator<Item = Hunk> + '_ {
        let (mut base, mut new) = (0, 0);
        self.chunks.iter().filter_map(move |chunk| {
            let hunk = (!chunk.equal).then(|| Hunk {
                base: base..base + chunk.base,
                new: new..new + chunk.new,
                pairs: Arc::clone(&chunk.pairs),
            });
            base += chunk.base;
            new += chunk.new;
            hunk
        })
    }

    /// Every region in order, unchanged or changed, as its base lines, its
    /// new lines and whether they are equal.
    pub fn regions(&self) -> impl Iterator<Item = (Range<usize>, Range<usize>, bool)> + '_ {
        let (mut base, mut new) = (0, 0);
        self.chunks.iter().map(move |chunk| {
            let region = (base..base + chunk.base, new..new + chunk.new, chunk.equal);
            base += chunk.base;
            new += chunk.new;
            region
        })
    }

    pub fn hunk_count(&self) -> usize {
        self.chunks.iter().filter(|chunk| !chunk.equal).count()
    }

    /// The line on the other side that corresponds to `line`, for keeping
    /// the sides' scroll positions together. Inside a changed region the
    /// offset is kept, clamped to the other side's extent.
    pub fn corresponding_line(&self, from: Side, line: usize) -> usize {
        let (mut base, mut new) = (0, 0);
        for chunk in &self.chunks {
            let (own_start, own_len, other_start, other_len) = match from {
                Side::New => (new, chunk.new, base, chunk.base),
                Side::Base => (base, chunk.base, new, chunk.new),
            };
            if line < own_start + own_len {
                let offset = line - own_start;
                return other_start + if chunk.equal { offset } else { offset.min(other_len.saturating_sub(1)) };
            }
            base += chunk.base;
            new += chunk.new;
        }
        let (own, other) = match from { Side::New => (new, base), Side::Base => (base, new) };
        other + line.saturating_sub(own)
    }

    /// Whether replaying the diff over `base` reproduces `new` exactly.
    pub fn reproduces(&self, base: &dyn LineSource, new: &dyn LineSource) -> bool {
        if base.line_count() != self.base_lines || new.line_count() != self.new_lines {
            return false;
        }
        let (mut x, mut y) = (0, 0);
        for chunk in &self.chunks {
            if chunk.equal && (0..chunk.base).any(|offset| base.line(x + offset) != new.line(y + offset)) {
                return false;
            }
            x += chunk.base;
            y += chunk.new;
        }
        x == self.base_lines && y == self.new_lines
    }
}

fn push_chunk(chunks: &mut Vec<Chunk>, chunk: Chunk) {
    if chunk.base == 0 && chunk.new == 0 {
        return;
    }
    if let Some(last) = chunks.last_mut() {
        if last.equal == chunk.equal {
            if !last.equal && !chunk.pairs.is_empty() {
                last.pairs = last.pairs.iter().cloned().chain(chunk.pairs.iter().map(|pair| LinePair {
                    base: last.base + pair.base, new: last.new + pair.new, spans: pair.spans.clone(),
                })).collect();
            }
            last.base += chunk.base;
            last.new += chunk.new;
            return;
        }
    }
    chunks.push(chunk);
}

fn line_hash(line: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// Diff one region of both sides into chunks. Lines are compared by hash and
/// equal runs are then verified against the text, so a collision only costs
/// minimality, never exactness.
fn diff_region(
    base: &dyn LineSource,
    base_range: Range<usize>,
    new: &dyn LineSource,
    new_range: Range<usize>,
    limits: DiffLimits,
    edit_window: bool,
) -> Vec<Chunk> {
    if base_range.is_empty() || new_range.is_empty() {
        return if base_range.is_empty() && new_range.is_empty() { Vec::new() } else {
            vec![Chunk { base: base_range.len(), new: new_range.len(), equal: false, pairs: Arc::from([]) }]
        };
    }
    let mut chunks = Vec::new();
    let runs = if edit_window && base_range.len() + new_range.len() > limits.window_lines {
        vec![Run::Change { base: base_range.len(), new: new_range.len() }]
    } else {
        let base_ids: Vec<u64> = base_range.clone().map(|index| line_hash(&base.line(index))).collect();
        let new_ids: Vec<u64> = new_range.clone().map(|index| line_hash(&new.line(index))).collect();
        diff(&base_ids, &new_ids, &mut WorkBudget::new(limits.work_units))
    };
    let (mut x, mut y) = (base_range.start, new_range.start);
    let mut pending: Option<(usize, usize, usize, usize)> = None;
    let flush = |chunks: &mut Vec<Chunk>, pending: &mut Option<(usize, usize, usize, usize)>| {
        if let Some((bx, by, removed, added)) = pending.take() {
            let pairs = pair_lines(base, bx..bx + removed, new, by..by + added, limits);
            push_chunk(chunks, Chunk { base: removed, new: added, equal: false, pairs: pairs.into() });
        }
    };
    for run in runs {
        match run {
            Run::Equal(length) => {
                // Verify; a hash collision becomes a one-line change.
                let mut equal_run = 0;
                for offset in 0..length {
                    if base.line(x + offset) == new.line(y + offset) {
                        if equal_run == 0 {
                            flush(&mut chunks, &mut pending);
                        }
                        equal_run += 1;
                    } else {
                        if equal_run > 0 {
                            push_chunk(&mut chunks, Chunk::equal(equal_run));
                            equal_run = 0;
                        }
                        let entry = pending.get_or_insert((x + offset, y + offset, 0, 0));
                        entry.2 += 1;
                        entry.3 += 1;
                    }
                }
                if equal_run > 0 {
                    push_chunk(&mut chunks, Chunk::equal(equal_run));
                }
                x += length;
                y += length;
            }
            Run::Change { base: removed, new: added } => {
                let entry = pending.get_or_insert((x, y, 0, 0));
                entry.2 += removed;
                entry.3 += added;
                x += removed;
                y += added;
            }
        }
    }
    flush(&mut chunks, &mut pending);
    chunks
}

const PAIR_SIMILARITY: f32 = 0.5;

/// Choose corresponding lines within a changed chunk and compute their spans.
/// Small chunks pick the in-order pairing with the greatest total similarity;
/// larger ones pair lines in order.
fn pair_lines(
    base: &dyn LineSource,
    base_range: Range<usize>,
    new: &dyn LineSource,
    new_range: Range<usize>,
    limits: DiffLimits,
) -> Vec<LinePair> {
    let (n, m) = (base_range.len(), new_range.len());
    if n == 0 || m == 0 || n > limits.inline_lines || m > limits.inline_lines {
        return Vec::new();
    }
    let base_text: Vec<Cow<'_, str>> = base_range.clone().map(|index| base.line(index)).collect();
    let new_text: Vec<Cow<'_, str>> = new_range.clone().map(|index| new.line(index)).collect();
    let mut chosen = Vec::new();
    if n * m <= limits.pairing_cells {
        // Weighted in-order matching by dynamic programming.
        let score = |i: usize, j: usize| {
            let value = similarity(&base_text[i], &new_text[j]);
            if value >= PAIR_SIMILARITY { value } else { 0.0 }
        };
        let mut table = vec![vec![0f32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                let pair = score(i, j);
                let take = if pair > 0.0 { pair + table[i + 1][j + 1] } else { 0.0 };
                table[i][j] = take.max(table[i + 1][j]).max(table[i][j + 1]);
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            let pair = score(i, j);
            if pair > 0.0 && (table[i][j] - (pair + table[i + 1][j + 1])).abs() < 1e-6 {
                chosen.push((i, j));
                i += 1;
                j += 1;
            } else if (table[i][j] - table[i + 1][j]).abs() < 1e-6 {
                i += 1;
            } else {
                j += 1;
            }
        }
    } else {
        chosen.extend((0..n.min(m)).filter(|&i| similarity(&base_text[i], &new_text[i]) >= PAIR_SIMILARITY).map(|i| (i, i)));
    }
    chosen
        .into_iter()
        .map(|(i, j)| LinePair { base: i, new: j, spans: inline_spans(&base_text[i], &new_text[j]) })
        .collect()
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
