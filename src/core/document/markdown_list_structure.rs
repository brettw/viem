//! Local source ownership for list edits whose labels are layout decorations.
use super::*;

pub(super) fn deletion_patches(
    document: &Document,
    range: &Range<usize>,
    whole_line: bool,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown || range.is_empty() && !whole_line {
        return Ok(None);
    }
    let projection = document.projection();
    if !whole_line && range.len() == 1
        && projection.hard_breaks_for_region(range).contains(&range.start)
    {
        let right = projection.blocks_for_region(&(range.end..range.end)).into_iter()
            .find(|block| block.range.start == range.end
                && matches!(block.kind, super::super::BlockKind::ListItem { item_start: true, .. }));
        if let Some(right) = right {
            let at = projection.source_insertion_point(right.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let line = document.state().source_hard_lines.line_at_offset(at)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = document.state().source.bytes_in(line.start..line.end.min(line.start + 512))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document.encoding().decode_region(&bytes, line.start)?;
            if let Some(prefix) = super::super::markdown_blocks::marker_prefix_length(&decoded.text) {
                let source_start = projection.source_range(range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?.start;
                let source_end = line.start + document.encoding().encode_fragment(&decoded.text[..prefix])?.len();
                // The following item's label has no text to delete. Joining
                // its boundary owns that syntax, while inline delimiters and
                // the item's following separator stay untouched.
                return Ok(Some(vec![SourcePatch::primary(source_start..source_end, Vec::new())]));
            }
        }
    }
    let paragraphs = projection
        .blocks_for_region(range)
        .into_iter()
        .filter(|block| range.start <= block.range.start && block.range.end <= range.end)
        .collect::<Vec<_>>();
    let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) else {
        return Ok(None);
    };
    if !paragraphs
        .iter()
        .any(|block| matches!(block.kind, super::super::BlockKind::ListItem { .. }))
        || !(range.start == first.range.start || range.start + 1 == first.range.start)
        || !(range.end == last.range.end || range.end == last.range.end + 1)
        || !whole_line
            && !projection
                .hard_breaks_for_region(range)
                .iter()
                .any(|at| range.contains(at))
    {
        return Ok(None);
    }
    let source_line =
        |block: &super::super::Block, at_end: bool| -> Result<Range<usize>, DocumentError> {
            let at = if at_end && !block.range.is_empty() {
                projection
                    .source_range(block.range.clone())
                    .map(|range| range.end.saturating_sub(1))
            } else {
                projection.source_insertion_point(block.range.start, true)
            }
            .ok_or(DocumentError::AmbiguousProjection)?;
            document
                .state()
                .source_hard_lines
                .line_at_offset(at)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)
        };
    let start = if range.start < first.range.start {
        projection
            .source_range(range.start..first.range.start)
            .ok_or(DocumentError::AmbiguousProjection)?
            .start
    } else {
        source_line(first, false)?.start
    };
    let end = if range.end > last.range.end {
        let next = projection
            .blocks_for_region(&(range.end..range.end))
            .into_iter()
            .find(|block| block.range.start == range.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        source_line(&next, false)?.start
    } else {
        source_line(last, true)?.end
    };
    Ok(Some(vec![SourcePatch::primary(start..end, Vec::new())]))
}

pub(super) fn empty_insertion_patches(
    document: &Document,
    range: &Range<usize>,
    text: &str,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown || !range.is_empty() || text.is_empty() {
        return Ok(None);
    }
    if !document
        .projection()
        .blocks_for_region(range)
        .iter()
        .any(|block| {
            block.range == *range && matches!(block.kind, super::super::BlockKind::ListItem { .. })
        })
    {
        return Ok(None);
    }
    let at = document
        .projection()
        .source_insertion_point(range.start, true)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let line = document
        .state()
        .source_hard_lines
        .line_at_offset(at)
        .and_then(|index| document.state().source_hard_lines.get(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let bytes = document
        .state()
        .source
        .bytes_in(line.start..at)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let prefix = document.encoding().decode_region(&bytes, line.start)?.text;
    if prefix.ends_with([' ', '\t']) {
        return Ok(None);
    }
    let syntax = format!(" {}", escape_markdown_insert(text));
    Ok(Some(vec![SourcePatch::primary(
        at..at,
        document.encoding().encode_fragment(&syntax)?,
    )]))
}

/// Plain register paragraphs inherit the destination list, while their labels
/// remain absent from the payload. Existing source labels are never rewritten.
pub(super) fn insertion_patches(
    document: &Document,
    edit: &FormattedPayloadEdit,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown
        || !edit.range.is_empty()
        || edit.payload.break_offsets().is_empty()
    {
        return Ok(None);
    }
    let at = edit.range.start;
    let block = document
        .projection()
        .blocks_for_region(&(at..at))
        .into_iter()
        .find(|block| block.range.start <= at && at <= block.range.end);
    let Some(block) = block.filter(|block| block.style.0 != "Code Block") else {
        return Ok(None);
    };
    let super::super::BlockKind::ListItem {
        ordered, ordinal, ..
    } = block.kind
    else {
        return Ok(None);
    };
    let body_at = document
        .projection()
        .source_insertion_point(block.range.start, true)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let line = document
        .state()
        .source_hard_lines
        .line_at_offset(body_at)
        .and_then(|index| document.state().source_hard_lines.get(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let bytes = document
        .state()
        .source
        .bytes_in(line.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, line.start)?;
    let Some(prefix_len) = super::super::markdown_blocks::marker_prefix_length(&decoded.text)
    else {
        return Ok(None);
    };
    let prefix = &decoded.text[..prefix_len];
    let indent_len = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
    let indent = &prefix[..indent_len];
    let delimiter = prefix[indent_len..]
        .chars()
        .find(|ch| !ch.is_ascii_digit())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let insert_before_label = at == block.range.start
        && edit
            .payload
            .break_offsets()
            .last()
            .is_some_and(|last| *last + 1 == edit.payload.text().len());
    let mut syntax = String::new();
    if insert_before_label {
        syntax.push_str(indent);
        if ordered {
            syntax.push_str(&ordinal.to_string());
        }
        syntax.push(delimiter);
        syntax.push(' ');
    }
    let mut start = 0;
    let mut number = ordinal;
    for &boundary in edit.payload.break_offsets() {
        syntax.push_str(&escape_markdown_insert(
            &edit.payload.text()[start..boundary],
        ));
        syntax.push_str(document.file_format().spelling());
        if !(insert_before_label && boundary + 1 == edit.payload.text().len()) {
            syntax.push_str(indent);
            number = number.saturating_add(1);
            if ordered {
                syntax.push_str(&number.to_string());
            }
            syntax.push(delimiter);
            syntax.push(' ');
        }
        start = boundary + 1;
    }
    syntax.push_str(&escape_markdown_insert(&edit.payload.text()[start..]));
    let source_at = if insert_before_label {
        line.start
    } else {
        document
            .projection()
            .source_insertion_point(
                at,
                edit.boundary_affinity != Some(BoundaryAffinity::Upstream),
            )
            .ok_or(DocumentError::AmbiguousProjection)?
    };
    Ok(Some(vec![SourcePatch::primary(
        source_at..source_at,
        document.encoding().encode_fragment(&syntax)?,
    )]))
}
