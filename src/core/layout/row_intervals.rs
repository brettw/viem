//! Immutable interval queries for materialized rows whose bottoms may overlap.
//! A centered tree visits O(log n + k) nodes/items for k intersecting rows.
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowIntervalIndex {
    data: Arc<Intervals>,
    offset: f32,
}

#[derive(Debug, PartialEq)]
struct Intervals {
    starts: Vec<f32>,
    ends: Vec<f32>,
    /// Furthest preceding bottom, with the last painted row breaking ties.
    prefix_end: Vec<(f32, usize)>,
    root: Option<Box<Node>>,
}

#[derive(Debug, PartialEq)]
struct Node {
    center: f32,
    crossing_start: Vec<usize>,
    crossing_end: Vec<usize>,
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl RowIntervalIndex {
    pub(super) fn new(bounds: impl Iterator<Item = (f32, f32)>) -> Self {
        let (starts, ends): (Vec<_>, Vec<_>) = bounds.unzip();
        debug_assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]));
        debug_assert!(starts.iter().zip(&ends).all(|(start, end)| start < end));
        let mut prefix_end = Vec::with_capacity(ends.len());
        let mut previous = (f32::NEG_INFINITY, 0);
        for (index, end) in ends.iter().copied().enumerate() {
            if end >= previous.0 { previous = (end, index); }
            prefix_end.push(previous);
        }
        let root = Node::build((0..starts.len()).collect(), &starts, &ends);
        Self { data: Arc::new(Intervals { starts, ends, prefix_end, root }), offset: 0. }
    }

    pub(super) fn translate(&mut self, delta: f32) { self.offset += delta; }

    pub(super) fn closest(&self, y: f32) -> Option<usize> {
        let y = y - self.offset;
        let mut painted = None;
        if let Some(root) = &self.data.root {
            root.point(y, &self.data, &mut |index| painted = Some(painted.map_or(index, |old: usize| old.max(index))));
        }
        if painted.is_some() { return painted; }
        let next = self.data.starts.partition_point(|start| *start <= y);
        let before = next.checked_sub(1).map(|index| self.data.prefix_end[index]);
        match (before, self.data.starts.get(next)) {
            (Some((end, index)), Some(start)) => Some(if y - end <= start - y { index } else { next }),
            (Some((_, index)), None) => Some(index),
            (None, Some(_)) => Some(next),
            (None, None) => None,
        }
    }

    pub(super) fn intersecting(&self, top: f32, bottom: f32, mut visit: impl FnMut(usize)) {
        if top >= bottom { return; }
        if let Some(root) = &self.data.root { root.range(top - self.offset, bottom - self.offset, &self.data, &mut visit); }
    }
}

impl Node {
    fn build(indices: Vec<usize>, starts: &[f32], ends: &[f32]) -> Option<Box<Self>> {
        if indices.is_empty() { return None; }
        let center = starts[indices[indices.len() / 2]];
        let (mut left, mut right, mut crossing_start) = (Vec::new(), Vec::new(), Vec::new());
        for index in indices {
            if ends[index] <= center { left.push(index); }
            else if starts[index] > center { right.push(index); }
            else { crossing_start.push(index); }
        }
        let mut crossing_end = crossing_start.clone();
        crossing_end.sort_unstable_by(|left, right| ends[*right].total_cmp(&ends[*left]).then(right.cmp(left)));
        Some(Box::new(Self { center, crossing_start, crossing_end,
            left: Self::build(left, starts, ends), right: Self::build(right, starts, ends) }))
    }

    fn point(&self, y: f32, data: &Intervals, visit: &mut impl FnMut(usize)) {
        if y < self.center {
            for &index in &self.crossing_start {
                if data.starts[index] > y { break; }
                visit(index);
            }
            if let Some(left) = &self.left { left.point(y, data, visit); }
        } else {
            for &index in &self.crossing_end {
                if data.ends[index] <= y { break; }
                visit(index);
            }
            if let Some(right) = &self.right { right.point(y, data, visit); }
        }
    }

    fn range(&self, top: f32, bottom: f32, data: &Intervals, visit: &mut impl FnMut(usize)) {
        if top >= self.center {
            for &index in &self.crossing_end {
                if data.ends[index] <= top { break; }
                visit(index);
            }
            if let Some(right) = &self.right { right.range(top, bottom, data, visit); }
        } else if bottom <= self.center {
            for &index in &self.crossing_start {
                if data.starts[index] >= bottom { break; }
                visit(index);
            }
            if let Some(left) = &self.left { left.range(top, bottom, data, visit); }
        } else {
            for &index in &self.crossing_start { visit(index); }
            if let Some(left) = &self.left { left.range(top, bottom, data, visit); }
            if let Some(right) = &self.right { right.range(top, bottom, data, visit); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_queries_match_exhaustive_geometry_and_translate_without_rebuilding() {
        let bounds = (0..10_000).map(|index| {
            let start = index as f32;
            (start, start + if index % 53 == 0 { 200. } else { 0.5 })
        }).collect::<Vec<_>>();
        let mut index = RowIntervalIndex::new(bounds.iter().copied());
        for top in [0.25, 17.75, 199.75, 5077.75, 10_010.] {
            let mut actual = Vec::new(); index.intersecting(top, top + 2., |row| actual.push(row)); actual.sort_unstable();
            let expected = bounds.iter().enumerate().filter(|(_, (start, end))| *start < top + 2. && *end > top).map(|(index, _)| index).collect::<Vec<_>>();
            assert_eq!(actual, expected);
            let painted = bounds.iter().enumerate().rfind(|(_, (start, end))| *start <= top && top < *end).map(|(index, _)| index);
            assert_eq!(index.closest(top), painted);
        }
        let before = Arc::clone(&index.data);
        let old = index.closest(199.75);
        index.translate(320.);
        assert_eq!(index.closest(519.75), old);
        assert!(Arc::ptr_eq(&before, &index.data));
        let gaps = RowIntervalIndex::new([(0., 90.), (5., 10.), (100., 110.)].into_iter());
        assert_eq!(gaps.closest(50.), Some(0));
        assert_eq!(gaps.closest(95.), Some(0));
        assert_eq!(gaps.closest(96.), Some(2));
    }
}
