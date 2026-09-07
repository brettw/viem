//! Bounded, revision-checked wrap checkpoints. Unchanged text and resolved
//! style prefixes can survive a later edit in the same long hard line.
use super::engine::{LongLineLayoutCheckpoint, SHAPING_CONTEXT_BYTES};
use super::{DocumentLayoutStyles, LayoutProviderRequirements, ViewLayout};
use crate::document::{Document, MappingOutcome, PositionMap, TextRange, WritingDirection};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Default)]
pub(crate) struct LongLineCheckpointCache {
    entries: BTreeMap<usize, Entry>,
}
struct Entry {
    checkpoint: LongLineLayoutCheckpoint,
    dependency: Option<Dependency>,
}
struct Dependency {
    prefix: TextRange,
    styles: DocumentLayoutStyles,
    direction_determined: bool,
}
impl LongLineCheckpointCache {
    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &LongLineLayoutCheckpoint> {
        self.entries.values().map(|entry| &entry.checkpoint)
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn discard_stale(
        &mut self,
        document: &Document,
        view: &ViewLayout,
        requirements: LayoutProviderRequirements,
    ) {
        self.entries.retain(|_, entry| {
            let checkpoint = &entry.checkpoint;
            checkpoint.document_id() == document.id()
                && checkpoint.document_revision() == document.revision()
                && checkpoint.configuration_generation() == view.configuration_generation()
                && checkpoint.measurement_environment_id()
                    == requirements.measurement_environment_id
                && checkpoint.metrics_generation() == requirements.metrics_generation
        });
    }
    pub(crate) fn before(
        &self,
        range: Range<usize>,
        at: usize,
    ) -> Option<LongLineLayoutCheckpoint> {
        if at < range.start {
            return None;
        }
        self.entries
            .range(range.start..=at)
            .next_back()
            .map(|(_, entry)| entry.checkpoint.clone())
    }
    pub(crate) fn before_height(
        &self,
        range: Range<usize>,
        height: f32,
    ) -> Option<LongLineLayoutCheckpoint> {
        self.entries
            .range(range)
            .rev()
            .find(|(_, entry)| entry.checkpoint.completed_height() <= height)
            .map(|(_, entry)| entry.checkpoint.clone())
    }
    pub(crate) fn insert(&mut self, document: &Document, checkpoint: LongLineLayoutCheckpoint) {
        const MAX_CHECKPOINTS: usize = 256;
        let dependency = capture_dependency(document, &checkpoint);
        self.entries.insert(
            checkpoint.next_text_offset(),
            Entry {
                checkpoint,
                dependency,
            },
        );
        while self.entries.len() > MAX_CHECKPOINTS {
            let oldest = *self.entries.keys().next().expect("cache is nonempty");
            self.entries.remove(&oldest);
        }
    }
    pub(crate) fn rebase(&mut self, document: &Document, map: &PositionMap, flow: bool) {
        self.entries.retain(|_, entry| {
            let Some(dependency) = &mut entry.dependency else {
                return false;
            };
            if !dependency.direction_determined
                || entry.checkpoint.document_revision() != map.source_revision()
            {
                return false;
            }
            let Ok(MappingOutcome::Exact(mapped)) = map.map_text_range(dependency.prefix) else {
                return false;
            };
            let Some(prefix) = mapped
                .segments()
                .first()
                .copied()
                .filter(|_| mapped.segments().len() == 1)
            else {
                return false;
            };
            let range = prefix.start().offset()..prefix.end().offset();
            let Some(styles) = clipped_styles(document, range) else {
                return false;
            };
            if styles != dependency.styles {
                return false;
            }
            let checkpoint = &mut entry.checkpoint;
            let Some(line) = document
                .projection()
                .presentation_line_range(checkpoint.hard_line_index, flow)
            else {
                return false;
            };
            if line.start != checkpoint.hard_line_range.start {
                return false;
            }
            let end = line.end;
            if end < prefix.end().offset() {
                return false;
            }
            checkpoint.document_revision = map.target_revision();
            checkpoint.hard_line_range.end = end;
            dependency.prefix = prefix;
            true
        });
    }
}
fn capture_dependency(
    document: &Document,
    checkpoint: &LongLineLayoutCheckpoint,
) -> Option<Dependency> {
    let start = checkpoint.hard_line_range.start;
    let mut end = checkpoint
        .next_text_offset
        .saturating_add(SHAPING_CONTEXT_BYTES * 4)
        .min(checkpoint.hard_line_range.end);
    while end > checkpoint.next_text_offset && document.text_point(end).is_err() {
        end -= 1;
    }
    let prefix = TextRange::new(
        document.text_point(start).ok()?,
        document.text_point(end).ok()?,
    )
    .ok()?;
    let styles = clipped_styles(document, start..end)?;
    // A paragraph with an automatic base direction and only weak/neutral
    // prefix text can change direction when a later strong character is
    // inserted. Retain across edits only when direction is already fixed.
    let mut probe_end = (start + 128).min(end);
    while probe_end > start && document.text_point(probe_end).is_err() {
        probe_end -= 1;
    }
    let prefix_text = document
        .projection()
        .text_tree()
        .slice(start..probe_end)
        .ok()?;
    let direction_determined = styles
        .paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.base_direction != WritingDirection::Natural)
        || prefix_text
            .chars()
            .find(|character| !character.is_ascii_whitespace())
            .is_some_and(|character| character.is_ascii_alphabetic());
    Some(Dependency {
        prefix,
        styles,
        direction_determined,
    })
}
fn clipped_styles(document: &Document, range: Range<usize>) -> Option<DocumentLayoutStyles> {
    let mut styles =
        DocumentLayoutStyles::resolve_region(document.projection(), range.clone()).ok()?;
    // Dense style documents can still reuse same-revision checkpoints without
    // retaining an unbounded copy of all preceding style runs per checkpoint.
    if styles.shaping_runs.len() + styles.paint_runs.len() + styles.paragraphs.len() > 128 {
        return None;
    }
    for paragraph in &mut styles.paragraphs {
        paragraph.text_range =
            paragraph.text_range.start.max(range.start)..paragraph.text_range.end.min(range.end);
    }
    for run in &mut styles.shaping_runs {
        run.text_range = run.text_range.start.max(range.start)..run.text_range.end.min(range.end);
    }
    for run in &mut styles.paint_runs {
        run.text_range = run.text_range.start.max(range.start)..run.text_range.end.min(range.end);
    }
    Some(styles)
}
