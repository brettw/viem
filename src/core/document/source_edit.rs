//! Shared reverse-edit boundary policy. Empty ranges name a visible insertion
//! location; nonempty ranges name only their actual source contributors.
//! Hidden syntax is never included merely to make a text edit contiguous.
use super::projection::VisibleSourceRun;
use super::{BoundaryAffinity, DocumentError, FormattedDocument};
use std::ops::Range;

/// The source context for escaping/whitespace is the beginning of the minimal
/// complete contributor. A structural selection needs only this context here;
/// its complete extent is still translated and verified by its adapter.
pub(super) fn text_context(
    document: &super::Document,
    edit: &super::TextEdit,
    affinity: Option<BoundaryAffinity>,
) -> Result<usize, DocumentError> {
    let edit = complete_contributors(document.projection(), edit)?;
    if edit.range.is_empty() && document.projection().text_tree().byte_len() != 0 {
        return insertion_point(document.projection(), edit.range.start, affinity)
            .ok_or(DocumentError::AmbiguousProjection);
    }
    let runs = match super::rich_text::text_source_runs(document, &edit.range) {
        Err(DocumentError::AmbiguousProjection) if !edit.range.is_empty() => {
            super::rich_text::text_source_runs(document, &(edit.range.start..edit.range.start))?
        }
        result => result?,
    };
    Ok(runs[0].start)
}

/// Text, paste/IME payloads and replay use one rich-source translation path.
/// Structural intentions are prepared by their adapter before reaching here.
pub(super) fn rich_text_patches(
    document: &super::Document,
    edit: &super::TextEdit,
    affinity: Option<BoundaryAffinity>,
) -> Result<Vec<super::SourcePatch>, DocumentError> {
    use super::{Format, SourcePatch};
    let original = edit;
    let mut edit = complete_contributors(document.projection(), edit)?;
    let html = document.format() == Format::Html;
    let mut runs = super::rich_text::text_source_runs(document, &edit.range)?;
    let suffix = if runs.len() > 1 {
        let suffix = document
            .projection()
            .text_tree()
            .slice(original.range.end..edit.range.end)
            .map_err(DocumentError::FormattedTextStorage)?;
        edit.replacement
            .truncate(edit.replacement.len() - suffix.len());
        suffix
    } else {
        String::new()
    };
    if edit.range.is_empty() {
        if document.projection().text_tree().byte_len() != 0 {
            let at = insertion_point(document.projection(), edit.range.start, affinity)
                .ok_or(DocumentError::AmbiguousProjection)?;
            runs = vec![at..at];
        }
        if document.format() == Format::Rtf {
            let at = super::rtf::advance_past_fallback_scope(document, runs[0].start)?;
            runs = vec![at..at];
        }
    }
    let syntax = if html {
        super::rich_text::escape_html_text_edit(document, runs[0].start, &edit)?
    } else if document.source_byte_len() == 0 {
        format!("{{\\rtf1\\ansi {}}}", super::rtf::escape(&edit.replacement))
    } else {
        super::rtf::escape_insertion(document, runs[0].start, &edit.replacement)?
    };
    let syntax = if html && runs.len() == 1 {
        if let Some((range, compact)) =
            super::rich_text::compact_generated_html_space(document, &edit, runs[0].start)?
        {
            runs[0] = range;
            compact
        } else {
            syntax
        }
    } else {
        syntax
    };
    let replacement = document.encoding().encode_fragment(&syntax)?;
    let mut patches = Vec::new();
    if html {
        for range in super::html_whitespace::exposed_whitespace(document, &edit, runs[0].start)? {
            patches.push(SourcePatch::primary(range, Vec::new()));
        }
    }
    let last = runs.len() - 1;
    for (index, range) in runs.into_iter().enumerate() {
        let value = if index == 0 {
            replacement.clone()
        } else if index == last && !suffix.is_empty() {
            let syntax = if html {
                super::rich_text::escape_html_source_edit(document, range.start, &suffix)?
            } else {
                super::rtf::escape_insertion(document, range.start, &suffix)?
            };
            document.encoding().encode_fragment(&syntax)?
        } else if html {
            document
                .encoding()
                .encode_fragment(&super::rich_text::escape_html_source_edit(
                    document,
                    range.start,
                    "",
                )?)?
        } else {
            Vec::new()
        };
        patches.push(SourcePatch::primary(range, value).with_generated_text(html));
    }
    Ok(patches)
}

/// A source entity may encode several logical items. Rewrite only that
/// indivisible contributor, carrying unselected prefix/suffix text through the
/// same format encoder. Verification and position maps retain the original
/// logical edit; this expanded edit is solely a source-translation detail.
pub(super) fn complete_contributors(
    projection: &FormattedDocument,
    edit: &super::TextEdit,
) -> Result<super::TextEdit, DocumentError> {
    let mut range = edit.range.clone();
    for at in [range.start, range.end] {
        for span in projection.provenance_touching(&(at..at)) {
            if span.formatted.start < at && at < span.formatted.end {
                if span.source.is_empty() {
                    return Err(DocumentError::AmbiguousProjection);
                }
                range.start = range.start.min(span.formatted.start);
                range.end = range.end.max(span.formatted.end);
            }
        }
    }
    if range == edit.range {
        return Ok(edit.clone());
    }
    let text = projection.text_tree();
    let prefix = text
        .slice(range.start..edit.range.start)
        .map_err(DocumentError::FormattedTextStorage)?;
    let suffix = text
        .slice(edit.range.end..range.end)
        .map_err(DocumentError::FormattedTextStorage)?;
    let mut result = edit.clone();
    result.range = range;
    result.replacement = format!("{prefix}{}{suffix}", edit.replacement);
    for at in &mut result.html_protective_spaces {
        *at += prefix.len();
    }
    Ok(result)
}

pub(super) fn insertion_point(
    projection: &FormattedDocument,
    at: usize,
    affinity: Option<BoundaryAffinity>,
) -> Option<usize> {
    let line = projection.hard_line_range(projection.hard_line_at_offset(at)?)?;
    let spans = projection.provenance_touching(&(at..at));
    // A retained empty inline/paragraph context is an editable location, not
    // an alternative range through surrounding opening or closing syntax.
    if let Some(seed) = spans
        .iter()
        .rev()
        .find(|span| span.formatted == (at..at) && span.source.is_empty())
    {
        return Some(seed.source.start);
    }
    let preceding = spans
        .iter()
        .rev()
        .find(|span| {
            !span.formatted.is_empty() && !span.source.is_empty() && span.formatted.end == at
        })
        .map(|span| span.source.end);
    let following = spans
        .iter()
        .find(|span| {
            !span.formatted.is_empty() && !span.source.is_empty() && span.formatted.start == at
        })
        .map(|span| span.source.start);
    // Paragraph/line ownership wins at its edges. Affinity still chooses the
    // adjacent inline context at an interior split caret.
    let downstream = if at == line.start {
        true
    } else if at == line.end {
        false
    } else {
        affinity != Some(BoundaryAffinity::Upstream)
    };
    let point = if downstream {
        following.or(preceding)
    } else {
        preceding.or(following)
    };
    point.or_else(|| projection.source_insertion_point(at, downstream))
}

pub(super) fn visible_runs(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> Result<Vec<VisibleSourceRun>, DocumentError> {
    if range.is_empty() || range.start > range.end || range.end > projection.text_tree().byte_len()
    {
        return Err(DocumentError::AmbiguousProjection);
    }
    let mut position = range.start;
    let mut result: Vec<VisibleSourceRun> = Vec::new();
    for span in projection.provenance_for_region(range) {
        // Seeds carry insertion context only and cannot contribute selected text.
        if span.formatted.is_empty() {
            continue;
        }
        if span.formatted.start != position
            || span.formatted.end > range.end
            || span.source.is_empty()
        {
            return Err(DocumentError::AmbiguousProjection);
        }
        position = span.formatted.end;
        if let Some(last) = result.last_mut() {
            if span.source.start < last.source.end {
                return Err(DocumentError::AmbiguousProjection);
            }
            if last.source.end == span.source.start {
                last.source.end = span.source.end;
                last.formatted.end = span.formatted.end;
                continue;
            }
        }
        result.push(VisibleSourceRun {
            formatted: span.formatted,
            source: span.source,
        });
    }
    if position != range.end {
        return Err(DocumentError::AmbiguousProjection);
    }
    Ok(result)
}

pub(super) fn contiguous_range(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> Result<Range<usize>, DocumentError> {
    if range.is_empty() {
        return insertion_point(projection, range.start, None)
            .map(|at| at..at)
            .ok_or(DocumentError::AmbiguousProjection);
    }
    let mut runs = visible_runs(projection, range)?;
    if runs.len() != 1 {
        return Err(DocumentError::AmbiguousProjection);
    }
    Ok(runs.remove(0).source)
}

/// Independent logical edits may touch the same indivisible source entity.
/// Partition their source translations at that entity's boundaries and combine
/// its changes once. Unchanged gaps stay in their original contributor/style;
/// callers retain the original logical edits for verification and position maps.
pub(super) fn rich_text_batch_patches(
    document: &super::Document,
    edits: &[(super::TextEdit, Option<BoundaryAffinity>)],
) -> Result<Vec<super::SourcePatch>, DocumentError> {
    if edits.len() == 1 {
        return rich_text_patches(document, &edits[0].0, edits[0].1);
    }
    let mut edits = edits.to_vec();
    edits.sort_by_key(|(edit, _)| (edit.range.start, edit.range.end));
    for pair in edits.windows(2) {
        if pair[0].0.range.end > pair[1].0.range.start
            || pair[0].0.range.is_empty() && pair[0].0.range == pair[1].0.range
        {
            return Err(DocumentError::OverlappingEdits);
        }
    }
    let projection = document.projection();
    let mut boundaries = Vec::new();
    for (edit, _) in &edits {
        for at in [edit.range.start, edit.range.end] {
            for span in projection.provenance_touching(&(at..at)) {
                if span.formatted.start < at && at < span.formatted.end {
                    if span.source.is_empty() {
                        return Err(DocumentError::AmbiguousProjection);
                    }
                    boundaries.extend([span.formatted.start, span.formatted.end]);
                }
            }
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut pieces = Vec::new();
    for (edit, affinity) in edits {
        if edit.range.is_empty() {
            pieces.push((edit, affinity));
            continue;
        }
        let mut start = edit.range.start;
        let mut first = true;
        let from = boundaries.partition_point(|&at| at <= start);
        for end in boundaries[from..]
            .iter()
            .copied()
            .take_while(|&at| at < edit.range.end)
            .chain(std::iter::once(edit.range.end))
        {
            let mut piece = if first {
                edit.clone()
            } else {
                super::TextEdit::new(start..end, "")
            };
            piece.range = start..end;
            pieces.push((piece, affinity));
            first = false;
            start = end;
        }
    }
    let mut groups: Vec<(
        Range<usize>,
        Vec<(super::TextEdit, Option<BoundaryAffinity>)>,
    )> = Vec::new();
    for (edit, affinity) in pieces {
        let complete = complete_contributors(projection, &edit)?.range;
        if let Some((extent, group)) = groups
            .last_mut()
            .filter(|(extent, _)| complete.start < extent.end && extent.start < complete.end)
        {
            extent.end = extent.end.max(complete.end);
            group.push((edit, affinity));
        } else {
            groups.push((complete, vec![(edit, affinity)]));
        }
    }
    let mut patches = Vec::new();
    for (_, group) in groups {
        if group.len() == 1 {
            patches.extend(rich_text_patches(document, &group[0].0, group[0].1)?);
            continue;
        }
        let start = group[0].0.range.start;
        let end = group.last().unwrap().0.range.end;
        let mut combined = super::TextEdit::new(start..end, "");
        combined.html_normalized = group.iter().all(|(edit, _)| edit.html_normalized);
        let mut at = start;
        for (edit, _) in &group {
            combined.replacement.push_str(
                &projection
                    .text_tree()
                    .slice(at..edit.range.start)
                    .map_err(DocumentError::FormattedTextStorage)?,
            );
            let shift = combined.replacement.len();
            combined.html_protective_spaces.extend(
                edit.html_protective_spaces
                    .iter()
                    .map(|offset| shift + offset),
            );
            combined.replacement.push_str(&edit.replacement);
            at = edit.range.end;
        }
        patches.extend(rich_text_patches(document, &combined, group[0].1)?);
    }
    Ok(patches)
}
