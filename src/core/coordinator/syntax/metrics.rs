//! Compare only retained syntax coverage, never the document text or blocks.
use crate::document::{
    DocumentStyleAssignment, FormattedDocument, ResolvedCharacterStyle, StyleApplication,
    StyleError, StyleId, StyleInvalidationEffect,
};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

struct MetricRun {
    range: Range<usize>,
    style: Arc<ResolvedCharacterStyle>,
}

fn same_metrics(a: &ResolvedCharacterStyle, b: &ResolvedCharacterStyle) -> bool {
    a.changed_properties(b)
        .iter()
        .all(|property| property.invalidation_effect() == StyleInvalidationEffect::Paint)
}

fn metric_runs(projection: &FormattedDocument) -> Result<Vec<MetricRun>, StyleError> {
    metric_runs_from(projection, projection.style_spans().iter().cloned())
}

/// The metric runs of `projection` clipped to `range`.
fn metric_runs_in(projection: &FormattedDocument, range: &Range<usize>) -> Result<Vec<MetricRun>, StyleError> {
    metric_runs_from(
        projection,
        projection.style_spans_for_region(range).into_iter().map(|mut span| {
            span.range = span.range.start.max(range.start)..span.range.end.min(range.end);
            span
        }),
    )
}

fn metric_runs_from(
    projection: &FormattedDocument,
    spans: impl Iterator<Item = crate::document::StyleSpan>,
) -> Result<Vec<MetricRun>, StyleError> {
    let sheet = projection.style_sheet();
    let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
    let resolve = |id: Option<&StyleId>| {
        sheet
            .resolve_assigned_paragraph_style(
                &assignment,
                &sheet.base_paragraph,
                &Default::default(),
                &Default::default(),
                id,
                &Default::default(),
            )
            .map(|style| style.character)
    };
    let default = resolve(None)?;
    let mut styles: BTreeMap<StyleId, Option<Arc<ResolvedCharacterStyle>>> = BTreeMap::new();
    let mut runs: Vec<MetricRun> = Vec::new();
    for span in spans {
        let StyleApplication::Automatic(id) = &span.application else {
            continue;
        };
        if !styles.contains_key(id) {
            let style = resolve(Some(id))?;
            styles.insert(
                id.clone(),
                (!same_metrics(&style, &default)).then(|| Arc::new(style)),
            );
        }
        let Some(style) = &styles[id] else {
            continue;
        };
        // Capture boundaries and names can change while their effective font
        // metrics remain identical. Compare the normalized metric coverage.
        if let Some(previous) = runs.last_mut() {
            if previous.range.end == span.range.start
                && (Arc::ptr_eq(&previous.style, style) || same_metrics(&previous.style, style))
            {
                previous.range.end = span.range.end;
                continue;
            }
        }
        runs.push(MetricRun {
            range: span.range.clone(),
            style: style.clone(),
        });
    }
    Ok(runs)
}

/// Whether any automatic (syntax) character style of this sheet resolves to
/// metrics other than the default. When none does, no run can change metrics,
/// so a publication need not compare every span of both projections.
pub(super) fn sheet_has_metric_styles(sheet: &crate::document::StyleSheet) -> Result<bool, StyleError> {
    let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
    let resolve = |id| {
        sheet
            .resolve_assigned_paragraph_style(
                &assignment,
                &sheet.base_paragraph,
                &Default::default(),
                &Default::default(),
                id,
                &Default::default(),
            )
            .map(|style| style.character)
    };
    let default = resolve(None)?;
    for style in sheet.character_styles() {
        if !same_metrics(&resolve(Some(&style.id))?, &default) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Where a publication changes font metrics, in the shared new revision.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::coordinator) enum MetricChange {
    None,
    /// The formatted hull of every differing metric run.
    Local(std::ops::Range<usize>),
    /// Unresolvable or sheet-wide: every line may have new metrics.
    Unbounded,
}

pub(super) fn metric_change(old: &FormattedDocument, new: &FormattedDocument) -> MetricChange {
    match (sheet_has_metric_styles(old.style_sheet()), sheet_has_metric_styles(new.style_sheet())) {
        (Ok(false), Ok(false)) => return MetricChange::None,
        (Ok(_), Ok(_)) => {}
        _ => return MetricChange::Unbounded,
    }
    let (Ok(old), Ok(new)) = (metric_runs(old), metric_runs(new)) else {
        return MetricChange::Unbounded;
    };
    let same = |a: &MetricRun, b: &MetricRun| a.range == b.range && same_metrics(&a.style, &b.style);
    let prefix = old.iter().zip(&new).take_while(|(a, b)| same(a, b)).count();
    if prefix == old.len() && prefix == new.len() {
        return MetricChange::None;
    }
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| same(a, b))
        .count();
    let hull = |runs: &[MetricRun]| {
        let changed = &runs[prefix..runs.len() - suffix];
        changed.first().map(|first| first.range.start..changed.last().unwrap().range.end)
    };
    match (hull(&old), hull(&new)) {
        (Some(a), Some(b)) => MetricChange::Local(a.start.min(b.start)..a.end.max(b.end)),
        (Some(a), None) | (None, Some(a)) => MetricChange::Local(a),
        (None, None) => MetricChange::None,
    }
}

/// Where a local publication changes font metrics, comparing only the
/// regions the delta names: the edit hull and each replaced region, in the
/// previous presentation's and the new projection's own coordinates. Both
/// projections share the sheet; `has_metric_styles` is that sheet's answer.
pub(super) fn metric_change_local(
    old: Option<&FormattedDocument>,
    new: &FormattedDocument,
    delta: &crate::document::syntax::service::PublicationDelta,
    has_metric_styles: bool,
) -> MetricChange {
    if !has_metric_styles {
        return MetricChange::None;
    }
    let Some(old) = old else {
        return MetricChange::Unbounded;
    };
    let hull = delta.hull.as_ref();
    let shift = hull.map_or(0, |(old_hull, new_hull)| new_hull.end as i128 - old_hull.end as i128);
    // Map a new-coordinate offset back to the previous presentation.
    let to_old = |offset: usize, at_end: bool| -> usize {
        match hull {
            Some((old_hull, new_hull)) if offset >= new_hull.end => (offset as i128 - shift) as usize,
            Some((old_hull, new_hull)) if offset > new_hull.start => {
                if at_end { old_hull.end } else { old_hull.start }
            }
            _ => offset,
        }
    };
    let mut regions: Vec<Range<usize>> = delta.replaced.clone();
    if let Some((_, new_hull)) = hull {
        regions.push(new_hull.clone());
    }
    let mut changed: Option<Range<usize>> = None;
    for region in regions {
        let old_region = to_old(region.start, false)..to_old(region.end, true);
        let (Ok(old_runs), Ok(new_runs)) = (metric_runs_in(old, &old_region), metric_runs_in(new, &region)) else {
            return MetricChange::Unbounded;
        };
        let touches_hull = |range: &Range<usize>, hull: &Range<usize>| {
            range.start < hull.end && hull.start < range.end
                || (hull.is_empty() && range.start < hull.start && hull.start < range.end)
        };
        let differs = match hull {
            Some((old_hull, new_hull))
                if old_runs.iter().any(|run| touches_hull(&run.range, old_hull))
                    || new_runs.iter().any(|run| touches_hull(&run.range, new_hull)) =>
            {
                true
            }
            _ => {
                let translated = old_runs.iter().map(|run| {
                    let map = |offset: usize| match hull {
                        Some((old_hull, _)) if offset >= old_hull.end => (offset as i128 + shift) as usize,
                        _ => offset,
                    };
                    (map(run.range.start)..map(run.range.end), &run.style)
                });
                !translated.eq_by_metrics(new_runs.iter().map(|run| (run.range.clone(), &run.style)))
            }
        };
        if differs {
            changed = Some(match changed {
                Some(hull) => hull.start.min(region.start)..hull.end.max(region.end),
                None => region,
            });
        }
    }
    changed.map_or(MetricChange::None, MetricChange::Local)
}

trait MetricRunsEq<'a>: Iterator<Item = (Range<usize>, &'a Arc<ResolvedCharacterStyle>)> + Sized {
    fn eq_by_metrics(mut self, mut other: impl Iterator<Item = (Range<usize>, &'a Arc<ResolvedCharacterStyle>)>) -> bool {
        loop {
            match (self.next(), other.next()) {
                (None, None) => return true,
                (Some((a, x)), Some((b, y))) if a == b && same_metrics(x, y) => {}
                _ => return false,
            }
        }
    }
}

impl<'a, I: Iterator<Item = (Range<usize>, &'a Arc<ResolvedCharacterStyle>)>> MetricRunsEq<'a> for I {}

#[cfg(test)]
pub(super) fn runs_change_metrics(old: &FormattedDocument, new: &FormattedDocument) -> bool {
    metric_change(old, new) != MetricChange::None
}

#[cfg(test)]
mod tests;
