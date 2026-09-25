//! Preserve mapped appearance while new analysis is pending. Retained runs
//! are deliberately separate from provider coverage and its cache-hit policy.
use super::{
    runs::{run_bytes, PublicationDelta, RunStore},
    Coverage, SyntaxInputSnapshot, SyntaxResult, SyntaxService, MAX_CACHED_REGIONS,
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
    /// after, when the document recorded one. Runs outside it keep their text,
    /// so the store drops the runs inside it and shifts the rest in one lazy
    /// coordinate change instead of mapping every run.
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
                let shift = hull.filter(|(old_hull, new_hull)| {
                    old_hull.start == new_hull.start
                        && old_hull.end <= old.byte_len()
                        && new_hull.end <= input.byte_len()
                        && old.byte_len() as i128 - old_hull.end as i128
                            == input.byte_len() as i128 - new_hull.end as i128
                });
                match shift {
                    Some((old_hull, new_hull)) if self.runs.shift_for_edit(&old_hull, &new_hull) => {
                        self.delta.note_hull(old_hull, new_hull);
                    }
                    _ => {
                        let mut retained = Vec::new();
                        let mut bytes: usize = 0;
                        for run in self.runs.runs() {
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
                                retained.push(run.with_range(range.start().offset()..range.end().offset()));
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
