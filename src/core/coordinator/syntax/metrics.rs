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
    let sheet = projection.style_sheet();
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
    let mut styles: BTreeMap<&StyleId, Option<Arc<ResolvedCharacterStyle>>> = BTreeMap::new();
    let mut runs: Vec<MetricRun> = Vec::new();
    for span in projection.style_spans() {
        let StyleApplication::Automatic(id) = &span.application else {
            continue;
        };
        if !styles.contains_key(id) {
            let style = resolve(Some(id))?;
            styles.insert(
                id,
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
fn sheet_has_metric_styles(sheet: &crate::document::StyleSheet) -> Result<bool, StyleError> {
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
pub(super) enum MetricChange {
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

#[cfg(test)]
pub(super) fn runs_change_metrics(old: &FormattedDocument, new: &FormattedDocument) -> bool {
    metric_change(old, new) != MetricChange::None
}

#[cfg(test)]
mod tests;
