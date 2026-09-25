//! Preserve mapped appearance while new analysis is pending. Retained runs
//! are deliberately separate from provider coverage and its cache-hit policy.
use super::{
    Coverage, SyntaxInputSnapshot, SyntaxResult, SyntaxRun, SyntaxService, MAX_CACHED_REGIONS,
    MAX_CACHED_RUN_BYTES,
};
use crate::document::{DocumentId, PositionDomain, PositionMap, Revision, TextPoint, TextRange};
use std::ops::Range;

pub(super) fn run_bytes(run: &SyntaxRun) -> usize {
    std::mem::size_of::<SyntaxRun>() + run.name.0.len() + run.origin.len()
}

impl SyntaxService {
    /// Advance only through an exact normalized-text map. Provider results
    /// remain bound to their original input and are still rejected if stale.
    ///
    /// `hull` is the exact formatted extent the transition replaced, before and
    /// after, when the document recorded one. Runs outside it keep their text,
    /// so they are kept or shifted directly instead of being mapped one by one.
    pub(crate) fn rebase_input(
        &mut self,
        input: SyntaxInputSnapshot,
        map: Option<&PositionMap>,
        hull: Option<(Range<usize>, Range<usize>)>,
    ) {
        if self.current == Some(input.identity()) {
            return;
        }
        let retained = match (&self.current_input, map) {
            (Some(old), Some(map)) if matches_transition(old, &input, map) => {
                let mut retained = Vec::new();
                let mut bytes: usize = 0;
                let shift = hull
                    .as_ref()
                    .filter(|(old_hull, new_hull)| {
                        old_hull.start == new_hull.start
                            && old_hull.end <= old.byte_len()
                            && new_hull.end <= input.byte_len()
                    })
                    .map(|(old_hull, new_hull)| {
                        (old_hull.clone(), new_hull.end as i128 - old_hull.end as i128)
                    });
                for run in self.runs(old.identity()) {
                    if let Some((old_hull, delta)) = &shift {
                        let untouched = if run.range.end <= old_hull.start {
                            Some(run.range.clone())
                        } else if run.range.start >= old_hull.end {
                            let start = usize::try_from(run.range.start as i128 + delta).ok();
                            let end = usize::try_from(run.range.end as i128 + delta).ok();
                            start.zip(end).map(|(start, end)| start..end)
                        } else {
                            None
                        };
                        if let Some(range) = untouched.filter(|range| range.end <= input.byte_len()) {
                            let size = run_bytes(&run);
                            if bytes.saturating_add(size) > MAX_CACHED_RUN_BYTES {
                                break;
                            }
                            let mut next = run.clone();
                            next.range = range;
                            retained.push(next);
                            bytes += size;
                            continue;
                        }
                    }
                    let Some(range) = logical_range(old, run.range.clone()) else {
                        continue;
                    };
                    let Ok(mapped) = map.map_text_range(range) else {
                        continue;
                    };
                    let Some(mapped) = mapped.value() else {
                        continue;
                    };
                    for range in mapped.segments() {
                        if range.is_empty() {
                            continue;
                        }
                        let size = run_bytes(&run);
                        if bytes.saturating_add(size) > MAX_CACHED_RUN_BYTES {
                            break;
                        }
                        let mut next = run.clone();
                        next.range = range.start().offset()..range.end().offset();
                        retained.push(next);
                        bytes += size;
                    }
                }
                retained
            }
            _ => Vec::new(),
        };
        self.cache.clear();
        self.retained = retained;
        self.current = Some(input.identity());
        self.current_input = Some(input);
    }

    pub(super) fn replace_presentation_coverage(&mut self, result: &SyntaxResult) {
        let ready = result.coverage != Coverage::Missing;
        if ready {
            self.retained = self
                .retained
                .iter()
                .flat_map(|run| outside(run, &result.range))
                .collect();
        }
        for old in &self.cache {
            if old.input != result.input || !overlaps(&old.range, &result.range) {
                continue;
            }
            // A query replaces only its declared coverage. Keep the other
            // displayed portions when regional cache entries overlap.
            if ready {
                self.retained
                    .extend(old.runs.iter().flat_map(|run| outside(run, &result.range)));
            } else {
                self.retained.extend(old.runs.iter().cloned());
            }
        }
        self.cache
            .retain(|old| !overlaps(&old.range, &result.range) && old.input == result.input);
    }

    pub(super) fn bound_presentation_cache(&mut self) {
        while self.cache.len() > MAX_CACHED_REGIONS {
            self.cache.pop_front();
        }
        let mut bytes = self.retained_result_bytes();
        let mut removed = 0;
        while bytes > MAX_CACHED_RUN_BYTES && removed < self.retained.len() {
            bytes -= run_bytes(&self.retained[removed]);
            removed += 1;
        }
        self.retained.drain(..removed);
        while bytes > MAX_CACHED_RUN_BYTES {
            let Some(old) = self.cache.pop_front() else {
                break;
            };
            bytes -= old.bytes();
        }
    }
}

fn matches_transition(
    old: &SyntaxInputSnapshot,
    next: &SyntaxInputSnapshot,
    map: &PositionMap,
) -> bool {
    old.identity().document == next.identity().document
        && old.identity().generation == next.identity().generation
        && map.document() == DocumentId(old.identity().document)
        && map.domain() == PositionDomain::FormattedText
        && map.source_revision() == Revision(old.identity().revision)
        && map.target_revision() == Revision(next.identity().revision)
        && map.source_len() == old.byte_len()
        && map.target_len() == next.byte_len()
}

fn logical_range(input: &SyntaxInputSnapshot, range: Range<usize>) -> Option<TextRange> {
    if range.start >= range.end || range.end > input.byte_len() {
        return None;
    }
    let ceil = |offset| {
        if input.text_tree().is_grapheme_boundary(offset).ok()? {
            Some(offset)
        } else {
            input.text_tree().next_grapheme_boundary(offset).ok()?
        }
    };
    let point = |offset| TextPoint {
        document: DocumentId(input.identity().document),
        revision: Revision(input.identity().revision),
        offset,
    };
    TextRange::new(point(ceil(range.start)?), point(ceil(range.end)?)).ok()
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn outside(run: &SyntaxRun, coverage: &Range<usize>) -> Vec<SyntaxRun> {
    if !overlaps(&run.range, coverage) {
        return vec![run.clone()];
    }
    let mut pieces = Vec::with_capacity(2);
    if run.range.start < coverage.start {
        let mut left = run.clone();
        left.range.end = coverage.start;
        pieces.push(left);
    }
    if run.range.end > coverage.end {
        let mut right = run.clone();
        right.range.start = coverage.end;
        pieces.push(right);
    }
    pieces
}

#[cfg(test)]
mod tests;
