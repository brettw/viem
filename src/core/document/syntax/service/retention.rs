//! Preserve mapped appearance while new analysis is pending. Retained runs
//! are deliberately separate from provider coverage and its cache-hit policy.
use super::{
    runs::{run_bytes, PublicationDelta, RunStore},
    Coverage, SyntaxInputSnapshot, SyntaxResult, SyntaxRun, SyntaxService, MAX_CACHED_REGIONS,
    MAX_CACHED_RUN_BYTES,
};
use crate::document::{DocumentId, PositionDomain, PositionMap, Revision, TextPoint, TextRange};
use crate::document::range_index::RangedItem as _;
use std::ops::Range;

impl SyntaxService {
    /// Advance only through an exact normalized-text map. Provider results
    /// remain bound to their original input and are still rejected if stale.
    ///
    /// `hull` is the exact formatted extent the transition replaced, before and
    /// after, when the document recorded one. Only affected runs need mapping;
    /// the untouched suffix shifts in one lazy coordinate change. New text
    /// inherits the preceding character's appearance until analysis replaces it.
    pub(crate) fn rebase_input(
        &mut self,
        input: SyntaxInputSnapshot,
        map: Option<&PositionMap>,
        hull: Option<(Range<usize>, Range<usize>)>,
    ) {
        if self.current == Some(input.identity()) {
            return;
        }
        match (&self.current_input, map) {
            (Some(old), Some(map)) if matches_transition(old, &input, map) => {
                let replacements = map.replacements().filter(|(_, new)| !new.is_empty()).collect::<Vec<_>>();
                let remap = |run: &SyntaxRun| retain_run(old, map, &replacements, run);
                let shift = hull.filter(|(old_hull, new_hull)| {
                    old_hull.start == new_hull.start
                        && old_hull.end <= old.byte_len()
                        && new_hull.end <= input.byte_len()
                        && old.byte_len() as i128 - old_hull.end as i128
                            == input.byte_len() as i128 - new_hull.end as i128
                });
                match shift.and_then(|(old_hull, new_hull)| {
                    let predecessor = if old_hull.start == 0 { 0 }
                        else { old.text_tree().previous_grapheme_boundary(old_hull.start).ok()?? };
                    self.runs.rebase_for_edit(&old_hull, &new_hull, predecessor, remap)
                }) {
                    Some((old_hull, new_hull)) => {
                        self.delta.note_hull(old_hull, new_hull.clone());
                        self.delta.note_replaced(new_hull);
                    }
                    _ => {
                        let mut retained = Vec::new();
                        let mut bytes: usize = 0;
                        for run in self.runs.runs() {
                            for run in remap(&run) {
                                let size = run_bytes(&run);
                                if bytes.saturating_add(size) > MAX_CACHED_RUN_BYTES {
                                    break;
                                }
                                retained.push(run);
                                bytes += size;
                            }
                        }
                        self.runs = RunStore::new(retained);
                        self.delta = PublicationDelta::unbounded();
                    }
                }
            }
            _ => {
                self.runs = RunStore::default();
                self.delta = PublicationDelta::unbounded();
            }
        }
        self.cache.clear();
        self.current = Some(input.identity());
        self.current_input = Some(input);
        self.bound_presentation_cache();
    }

    /// Install a completed result's runs over its declared coverage. Returns
    /// whether the displayed runs in that region changed.
    pub(super) fn replace_presentation_coverage(&mut self, result: &SyntaxResult) -> bool {
        let ready = result.coverage != Coverage::Missing;
        let mut changed = false;
        if ready {
            // A query replaces only its declared coverage. Runs crossing its
            // edges keep their outside pieces.
            let previous = self.runs.replace(&result.range, result.runs.clone());
            changed = previous != result.runs;
            if changed {
                self.delta.note_replaced(result.range.clone());
            }
        }
        self.cache
            .retain(|old| !overlaps(&old.range, &result.range) && old.input == result.input);
        changed
    }

    pub(super) fn bound_presentation_cache(&mut self) {
        while self.cache.len() > MAX_CACHED_REGIONS {
            self.cache.pop_front();
        }
        let cache_bytes = self.cache.iter().map(SyntaxResult::bytes).sum::<usize>();
        let protected = self.cache.iter().map(|old| old.range.clone()).collect::<Vec<_>>();
        let budget = MAX_CACHED_RUN_BYTES.saturating_sub(cache_bytes);
        if let Some(removed) = self.runs.evict_front(budget, &protected) {
            self.delta.note_replaced(removed);
        }
    }
}

fn retain_run(
    old: &SyntaxInputSnapshot,
    map: &PositionMap,
    replacements: &[(Range<usize>, Range<usize>)],
    run: &SyntaxRun,
) -> Vec<SyntaxRun> {
    let Some(range) = logical_range(old, run.range.clone()) else { return Vec::new() };
    let Ok(mapped) = map.map_text_range(range) else { return Vec::new() };
    let Some(mapped) = mapped.value() else { return Vec::new() };
    let mut ranges = mapped.segments().iter()
        .map(|range| range.start().offset()..range.end().offset()).collect::<Vec<_>>();
    // A style owns insertions at its end, but not at its start. In particular,
    // BOF and an unstyled predecessor must not borrow a following token's style.
    ranges.extend(replacements.iter()
        .filter(|(replaced, _)| range.start().offset() < replaced.start && replaced.start <= range.end().offset())
        .map(|(_, inserted)| inserted.clone()));
    ranges.sort_by_key(|range| range.start);
    let mut retained: Vec<SyntaxRun> = Vec::new();
    for range in ranges.into_iter().filter(|range| !range.is_empty()) {
        if let Some(previous) = retained.last_mut().filter(|previous| previous.range.end >= range.start) {
            previous.range.end = previous.range.end.max(range.end);
        } else {
            retained.push(run.with_range(range));
        }
    }
    retained
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

#[cfg(test)]
mod tests;
