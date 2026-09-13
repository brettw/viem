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
    if let Some(plan) = overlapping_text_plan(document, &edit.range)? {
        return Ok(plan.insertion);
    }
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
    let plan = overlapping_text_plan(document, &edit.range)?;
    let mut runs = match &plan {
        Some(plan) => plan.ranges.clone(),
        None => super::rich_text::text_source_runs(document, &edit.range)?,
    };
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
    let source_at = plan.as_ref().map_or(runs[0].start, |plan| plan.insertion);
    let syntax = if html {
        super::rich_text::escape_html_text_edit(document, source_at, &edit)?
    } else if document.source_byte_len() == 0 {
        format!("{{\\rtf1\\ansi {}}}", super::rtf::escape(&edit.replacement))
    } else {
        super::rtf::escape_insertion(document, source_at, &edit.replacement)?
    };
    let syntax = if html && runs.len() == 1 && plan.is_none() {
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
    let syntax = if edit.range.is_empty() && !edit.replacement.is_empty() {
        let closing = document
            .projection()
            .provenance_touching(&edit.range)
            .into_iter()
            .find(|span| {
                span.formatted.end == edit.range.start
                    && span.source.end == runs[0].start
                    && document
                        .projection()
                        .text_tree()
                        .slice(span.formatted.clone())
                        .ok()
                        .as_deref()
                        == Some("\u{fffc}")
            })
            .map(|span| match document.format() {
                Format::Html => super::html_typing::opaque_closing_syntax(document, span.source),
                Format::Rtf => super::rtf::opaque_closing_syntax(document, span.source),
                _ => Ok(String::new()),
            })
            .transpose()?
            .unwrap_or_default();
        format!("{closing}{syntax}")
    } else {
        syntax
    };
    let replacement = document.encoding().encode_fragment(&syntax)?;
    let mut patches = Vec::new();
    if html {
        for range in super::html_whitespace::exposed_whitespace(document, &edit, source_at)? {
            patches.push(SourcePatch::primary(range, Vec::new()));
        }
    }
    let last = runs.len() - 1;
    let insertion_run = plan.as_ref().map_or(Some(0), TextSourcePlan::insertion_run);
    for (index, range) in runs.into_iter().enumerate() {
        let value = if Some(index) == insertion_run {
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
    if insertion_run.is_none() && !replacement.is_empty() {
        patches.push(
            SourcePatch::primary(source_at..source_at, replacement).with_generated_text(html),
        );
    }
    Ok(patches)
}

pub(super) struct TextSourcePlan {
    pub ranges: Vec<Range<usize>>,
    pub insertion: usize,
}

impl TextSourcePlan {
    /// A supporting removal can precede the logical insertion. If no removal
    /// contains that boundary, authored bytes need a separate empty patch.
    pub fn insertion_run(&self) -> Option<usize> {
        self.ranges
            .iter()
            .position(|range| range.start <= self.insertion && self.insertion <= range.end)
    }
}

/// Selected contributors can physically contain visible text which a parser
/// moved outside their logical owner. Subtract that unselected text and its
/// inline scopes, then place authored text at the selected logical boundary.
/// Supporting deletions before that boundary do not become insertion sites.
pub(super) fn overlapping_text_plan(
    document: &super::Document,
    range: &Range<usize>,
) -> Result<Option<TextSourcePlan>, DocumentError> {
    if range.is_empty() {
        return Ok(None);
    }
    let projection = document.projection();
    let spans: Vec<_> = projection
        .provenance_for_region(range)
        .into_iter()
        .filter(|span| !span.formatted.is_empty())
        .collect();
    let mut position = range.start;
    for span in &spans {
        if span.formatted.start != position
            || span.formatted.end > range.end
            || span.source.is_empty()
        {
            return Ok(None);
        }
        position = span.formatted.end;
    }
    if position != range.end {
        return Ok(None);
    }
    let mut retained = Vec::new();
    for selected in &spans {
        for other in projection.provenance_contained_in_source(&selected.source) {
            if other.formatted.end <= range.start || range.end <= other.formatted.start {
                retained.push(other.source);
            }
        }
    }
    if retained.is_empty()
        && spans
            .windows(2)
            .all(|pair| pair[0].source.end <= pair[1].source.start)
    {
        return Ok(None);
    }
    let mut selected: Vec<_> = spans.iter().map(|span| span.source.clone()).collect();
    merge_ranges(&mut selected);
    merge_ranges(&mut retained);
    if document.format() == super::Format::Html && !retained.is_empty() {
        let mut syntax = Vec::new();
        for source in &selected {
            syntax.extend(super::html_typing::retained_inline_syntax(
                document,
                source.clone(),
                &retained,
            )?);
        }
        retained.extend(syntax);
        merge_ranges(&mut retained);
    }
    let mut ranges = Vec::new();
    for selected in selected {
        let mut start = selected.start;
        for keep in &retained {
            if keep.end <= start || selected.end <= keep.start {
                continue;
            }
            if start < keep.start {
                ranges.push(start..keep.start);
            }
            start = start.max(keep.end);
        }
        if start < selected.end {
            ranges.push(start..selected.end);
        }
    }
    let insertion =
        insertion_point(projection, range.start, None).ok_or(DocumentError::AmbiguousProjection)?;
    if ranges.is_empty() {
        // Fully shared synthetic ownership has no independently editable
        // source bytes; an adapter must materialize that relation explicitly.
        return Err(DocumentError::UnsupportedFormatting);
    }
    Ok(Some(TextSourcePlan { ranges, insertion }))
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
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
    let inside_object = |source| spans.iter().any(|span| {
        span.source.start < source && source < span.source.end
            && projection.text_tree().slice(span.formatted.clone()).ok().as_deref() == Some("\u{fffc}")
    });
    // A retained empty inline/paragraph context is an editable location, not
    // an alternative range through surrounding opening or closing syntax.
    if let Some(seed) = spans
        .iter()
        .rev()
        .find(|span| span.formatted == (at..at) && span.source.is_empty()
            && !inside_object(span.source.start))
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
    // A format parser can move visible children ahead of their physical
    // container (HTML table foster parenting). The preceding text endpoint
    // remains the insertion location between that text and its outer owner.
    if let (Some(before), Some(after)) = (preceding, following) {
        if after < before {
            return Some(before);
        }
    }
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

/// Add only syntax bytes not already owned by translated source patches.
pub(super) fn append_uncovered_deletions(
    delimiter: &Range<usize>,
    patches: &[super::SourcePatch],
    support: &mut Vec<super::SourcePatch>,
) {
    let mut remaining = vec![delimiter.clone()];
    for patch in patches.iter().chain(support.iter()) {
        remaining = remaining
            .into_iter()
            .flat_map(|range| {
                let overlap =
                    range.start.max(patch.range().start)..range.end.min(patch.range().end);
                if overlap.start >= overlap.end {
                    return vec![range];
                }
                let mut parts = Vec::new();
                if range.start < overlap.start {
                    parts.push(range.start..overlap.start);
                }
                if overlap.end < range.end {
                    parts.push(overlap.end..range.end);
                }
                parts
            })
            .collect();
    }
    support.extend(
        remaining
            .into_iter()
            .map(|range| super::SourcePatch::primary(range, Vec::new())),
    );
}
