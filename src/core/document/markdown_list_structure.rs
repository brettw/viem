//! Local source ownership for list edits whose labels are layout decorations.
use super::*;

/// Joining a paragraph boundary owns the following block prefix, which has no
/// formatted characters. Inline delimiters remain attached to their content.
/// This applies equally to Backspace, Delete and Vim's spaced J.
pub(super) fn joining_patches(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown || range.is_empty() || replacement.contains('\n') {
        return Ok(None);
    }
    let projection = document.projection();
    let blocks = projection.blocks_for_region(range);
    let Some(first) = blocks.iter().find(|block| {
        block.range.start <= range.start && range.start <= block.range.end
    }) else { return Ok(None); };
    let Some(last) = blocks.iter().rev().find(|block| {
        block.range.start <= range.end && range.end <= block.range.end
    }) else { return Ok(None); };
    if first.id == last.id || range.end <= first.range.end { return Ok(None); }
    for block in [first, last] {
        if block.style.0 == "Code Block" || super::super::markdown_quotes::is_fenced_block(document, block)? {
            // The code adapter relocates fences and translates literal content.
            return Ok(None);
        }
    }
    let at = projection.source_insertion_point(last.range.start, true)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let line = document.state().source_hard_lines.line_at_offset(at)
        .and_then(|index| document.state().source_hard_lines.get(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let bytes = document.state().source.bytes_in(line.start..line.end.min(line.start + 512))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, line.start)?;
    let quote = super::super::markdown_quotes::prefix(&decoded.text);
    let body = &decoded.text[quote..];
    let mut prefix = quote + super::super::markdown_blocks::marker_prefix_length(body)
        .unwrap_or_else(|| super::super::projection::markdown_block_prefix(body, 0, body.len()).0);
    if matches!(last.kind, super::super::BlockKind::ListItem { item_start: false, .. }) {
        prefix += decoded.text[prefix..].len() - decoded.text[prefix..].trim_start_matches([' ', '\t']).len();
    }
    let start = projection.source_range(first.range.end..first.range.end + 1)
        .ok_or(DocumentError::AmbiguousProjection)?.start;
    let end = line.start + document.encoding().encode_fragment(&decoded.text[..prefix])?.len();
    let mut sources = vec![start..end];
    for selected in [range.start..first.range.end, last.range.start..range.end] {
        if !selected.is_empty() {
            sources.extend(super::super::source_edit::visible_runs(projection, &selected)?
                .into_iter().map(|run| run.source));
        }
    }
    sources.extend(super::super::markdown_code::selected_inline_delimiters(document, range)?);
    sources.sort_by_key(|source| (source.start, source.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for source in sources {
        if let Some(previous) = merged.last_mut().filter(|previous| source.start <= previous.end) {
            previous.end = previous.end.max(source.end);
        } else { merged.push(source); }
    }
    let syntax = if projection.markdown_replacement_begins_in_code(range) {
        replacement.to_owned()
    } else {
        document.escape_markdown_source_text(merged[0].start, replacement)?
    };
    let replacement = document.encoding().encode_fragment(&syntax)?;
    Ok(Some(merged.into_iter().enumerate().map(|(index, source)| {
        SourcePatch::primary(source, if index == 0 { replacement.clone() } else { Vec::new() })
    }).collect()))
}

pub(super) fn deletion_patches(
    document: &Document,
    range: &Range<usize>,
    whole_line: bool,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown || range.is_empty() && !whole_line {
        return Ok(None);
    }
    let projection = document.projection();
    if !whole_line {
        if let Some(patches) = joining_patches(document, range, "")? {
            return Ok(Some(patches));
        }
        for block in projection.blocks_for_region(range) {
            if block.style.0 == "Code Block"
                || super::super::markdown_quotes::is_fenced_block(document, &block)?
            {
                return Ok(None);
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
    // A hard break inside one item is still item content. Deleting that
    // content must retain the label; only a paragraph boundary or an explicit
    // whole-line operation owns the enclosing list structure.
    if !whole_line
        && paragraphs.len() == 1
        && range.start == first.range.start
        && range.end == first.range.end
    {
        return Ok(None);
    }
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
            block.range == *range && block.style.0 != "Code Block"
                && matches!(block.kind, super::super::BlockKind::ListItem { .. })
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
    let syntax = format!(" {}", document.escape_markdown_source_text(at, text)?);
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
        syntax.push_str(&escape_markdown_insert_in_encoding(
            &edit.payload.text()[start..boundary], document.encoding(),
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
    syntax.push_str(&escape_markdown_insert_in_encoding(&edit.payload.text()[start..], document.encoding()));
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
