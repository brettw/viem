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
    let assignment = DocumentStyleAssignment::new(sheet.base_document.clone());
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

pub(super) fn runs_change_metrics(old: &FormattedDocument, new: &FormattedDocument) -> bool {
    let (Ok(old), Ok(new)) = (metric_runs(old), metric_runs(new)) else {
        return true;
    };
    old.len() != new.len()
        || old
            .iter()
            .zip(&new)
            .any(|(a, b)| a.range != b.range || !same_metrics(&a.style, &b.style))
}

#[cfg(test)]
mod tests;
