//! Local source ownership for list edits whose labels are layout decorations.
use super::*;
use crate::document::BlockKind;

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
    let Some(first) = blocks
        .iter()
        .find(|block| block.range.start <= range.start && range.start <= block.range.end)
    else {
        return Ok(None);
    };
    let Some(last) = blocks
        .iter()
        .rev()
        .find(|block| block.range.start <= range.end && range.end <= block.range.end)
    else {
        return Ok(None);
    };
    if first.id == last.id || range.end <= first.range.end {
        return Ok(None);
    }
    for block in [first, last] {
        if block.style.0 == "Code Block"
            || super::super::markdown_quotes::is_fenced_block(document, block)?
        {
            // The code adapter relocates fences and translates literal content.
            return Ok(None);
        }
    }
    let at = projection
        .source_insertion_point(last.range.start, true)
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
        .bytes_in(line.start..line.end.min((line.start + 512).max(at)))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, line.start)?;
    // One physical row can alternate quote and list owners more than once.
    // Consume their complete structural prefix, stopping before inline syntax.
    let containers = super::super::markdown_quotes::container_prefix(&decoded.text);
    let body = &decoded.text[containers..];
    let mut prefix = containers
        + super::super::projection::markdown_block_prefix(body, 0, body.len()).0;
    if last.task_checkbox.is_some() {
        prefix += super::super::markdown_blocks::task_prefix_length(&decoded.text[prefix..])
            .ok_or(DocumentError::AmbiguousProjection)?;
    }
    if last.range.is_empty() || matches!(
        last.kind,
        super::super::BlockKind::ListItem {
            item_start: false,
            ..
        }
    ) {
        prefix += decoded.text[prefix..].len()
            - decoded.text[prefix..].trim_start_matches([' ', '\t']).len();
    }
    let mut start = projection
        .source_range(first.range.end..first.range.end + 1)
        .ok_or(DocumentError::AmbiguousProjection)?
        .start;
    let mut empty_continuation = None;
    if first.range.is_empty() && first.kind == BlockKind::Paragraph
        && !first.markdown_html && !first.thematic_break
    {
        let physical = document.physical_line_at_source(start)?;
        let bytes = document.source_bytes_in(physical.content_range.clone())?;
        let decoded = document.encoding().decode_region(&bytes, physical.content_range.start)?;
        let quote = super::super::markdown_quotes::prefix(&decoded.text);
        if decoded.text[quote..].bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            if first.list_level().is_some() {
                // Separator paragraphs inherit a list owner even though
                // their own physical row has no marker or indentation.
                empty_continuation = Some(document.markdown_list_continuation_context(first.range.start)
                    .map_err(compat_document_error)?.2);
                start = physical.content_range.start;
            } else {
                // Retain harmless blank-row trivia. Only indentation that
                // would turn the joined prose into code needs to be removed.
                let columns = |text: &str| text.bytes().fold(0usize, |column, byte|
                    column + if byte == b'\t' { 4 - column % 4 } else { 1 });
                if columns(&decoded.text) - columns(&decoded.text[..quote]) >= 4 {
                    start = decoded.source_boundary(quote).ok_or(DocumentError::AmbiguousProjection)?;
                }
            }
        }
    }
    // A heading's closing sequence is hidden syntax before its ending. Once
    // another body is joined there, that sequence would become literal text.
    if first.style.0.starts_with("Heading") {
        if let Some(body) = projection.source_range(first.range.clone()) {
            let first_line = document.state().source_hard_lines.line_at_offset(body.start)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = document.state().source.bytes_in(
                first_line.start..first_line.end.min(first_line.start + 512),
            ).ok_or(DocumentError::AmbiguousProjection)?;
            let first_prefix = document.encoding().decode_region(&bytes, first_line.start)?.text;
            if atx_body_prefix(&first_prefix).is_some() {
                let bytes = document.state().source.bytes_in(body.end..start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded_suffix = document.encoding().decode_region(&bytes, body.end)?;
                let suffix = &decoded_suffix.text;
                let trimmed = suffix.trim_end_matches([' ', '\t']);
                let hashes = trimmed.bytes().rev().take_while(|byte| *byte == b'#').count();
                let closing = trimmed.len() - hashes;
                let retained = if hashes > 0 && (trimmed[..closing].ends_with([' ', '\t'])
                    || closing == 0 && first.range.is_empty()) {
                    trimmed[..closing].trim_end_matches([' ', '\t'])
                } else { trimmed };
                start = decoded_suffix.source_boundary(retained.len())
                    .ok_or(DocumentError::AmbiguousProjection)?;
            } else {
                start = start.min(body.end);
            }
        }
    }
    let end = if last.style.0.starts_with("Heading") {
        // Block syntax stops before inline opening delimiters. Text provenance
        // starts after those delimiters, so it cannot own the entire prefix.
        decoded.source_boundary(atx_body_prefix(&decoded.text).unwrap_or(prefix))
            .ok_or(DocumentError::AmbiguousProjection)?
    } else { line.start
        + document
            .encoding()
            .encode_fragment(&decoded.text[..prefix])?
            .len() };
    let mut sources = vec![start..end];
    for selected in [range.start..first.range.end, last.range.start..range.end] {
        if !selected.is_empty() {
            sources.extend(
                super::super::source_edit::visible_runs(projection, &selected)?
                    .into_iter()
                    .map(|run| run.source),
            );
        }
    }
    sources.extend(super::super::markdown_code::selected_inline_delimiters(
        document, range,
    )?);
    sources.sort_by_key(|source| (source.start, source.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for source in sources {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| source.start <= previous.end)
        {
            previous.end = previous.end.max(source.end);
        } else {
            merged.push(source);
        }
    }
    let mut syntax = if projection.markdown_replacement_begins_in_code(range) {
        replacement.to_owned()
    } else {
        document.escape_markdown_source_text(merged[0].start, replacement)?
    };
    if let Some(continuation) = empty_continuation
        .filter(|_| !syntax.is_empty() || range.end < last.range.end)
    {
        syntax.insert_str(0, &continuation);
    }
    if first.range.is_empty() && matches!(first.kind, super::super::BlockKind::ListItem { .. }) {
        let line = document
            .state()
            .source_hard_lines
            .line_at_offset(start)
            .and_then(|index| document.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = document
            .state()
            .source
            .bytes_in(line.start..start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = document.encoding().decode_region(&bytes, line.start)?.text;
        if !prefix.ends_with([' ', '\t']) {
            syntax.insert(0, ' ');
        }
    }
    let replacement = document.encoding().encode_fragment(&syntax)?;
    let mut patches = merged
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            SourcePatch::primary(
                source,
                if index == 0 {
                    replacement.clone()
                } else {
                    Vec::new()
                },
            )
        })
        .collect::<Vec<_>>();
    if let Some(next) = projection.blocks_for_region(&(last.range.end + 1..last.range.end + 1))
        .into_iter().find(|block| block.range.start == last.range.end + 1 && !block.range.is_empty())
    {
        // The joined text keeps the first paragraph's owner. Child lists of
        // any removed later owner still need that ancestor's marker, even
        // when its former body was merged into the first paragraph.
        let removed = blocks.iter().filter(|block| block.id != first.id
            && block.range.start <= last.range.start).cloned().collect::<Vec<_>>();
        if let Some((end, prefix)) = retained_descendant_prefix(document, &removed, &next)? {
            let line = document.state().source_hard_lines.line_at_offset(end)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            patches.push(SourcePatch::primary(line.start..end, document.encoding().encode_fragment(&prefix)?));
        }
    }
    if first.style.0 != "Block quote"
        && first.quote_depth == 0
        && (last.style.0 == "Block quote" || last.quote_depth > 0)
    {
        let lines = &document.state().source_hard_lines;
        let first_line = lines
            .line_at_offset(at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let end = projection
            .source_range(last.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?
            .end;
        let last_line = lines
            .line_at_offset(end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        for index in first_line + 1..=last_line {
            let row = lines.get(index).ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = document
                .state()
                .source
                .bytes_in(row.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let text = document.encoding().decode_region(&bytes, row.start)?.text;
            let prefix = super::super::markdown_quotes::prefix(&text);
            if prefix > 0 {
                patches.push(SourcePatch::primary(
                    row.start
                        ..row.start + document.encoding().encode_fragment(&text[..prefix])?.len(),
                    Vec::new(),
                ));
            }
        }
    }
    Ok(Some(patches))
}

fn atx_body_prefix(text: &str) -> Option<usize> {
    let mut at = 0;
    loop {
        at += super::super::markdown_quotes::prefix(&text[at..]);
        let Some(marker) = super::super::markdown_blocks::marker_prefix_length(&text[at..]) else {
            break;
        };
        at += marker;
    }
    let indent = text[at..].bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 { return None; }
    at += indent;
    let hashes = text[at..].bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) { return None; }
    at += hashes;
    if text.as_bytes().get(at).is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n')) {
        return None;
    }
    at += text[at..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
    Some(at)
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
        .filter(|block| range.start <= block.range.start && block.range.end <= range.end
            // A retained empty paragraph at the half-open edge owns no
            // selected text or boundary, even inside the preceding list item.
            && (range.is_empty() || block.range.start < range.end))
        .collect::<Vec<_>>();
    let (Some(first), Some(last)) = (paragraphs.first(), paragraphs.last()) else {
        return Ok(None);
    };
    if whole_line && range.end > last.range.end {
        let next = projection
            .blocks_for_region(&(range.end..range.end))
            .into_iter()
            .find(|block| block.range.start == range.end);
        if next.is_some_and(|block| {
            matches!(
                block.kind,
                super::super::BlockKind::ListItem {
                    item_start: false,
                    ..
                }
            )
        }) {
            // The selected paragraph is only the beginning of an item. Keep
            // its marker and promote the surviving continuation body into it.
            // Removing the physical marker line would expose that body's
            // indentation as literal text and abandon its list ownership.
            if let Some(mut patches) = joining_patches(document, range, "")? {
                super::markdown_split::remove_empty_emphasis(
                    document,
                    &[TextEdit::new(range.clone(), "")],
                    &mut patches,
                )
                .map_err(super::compat_document_error)?;
                return Ok(Some(patches));
            }
            return Ok(None);
        }
    }
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
        .any(|block| matches!(block.kind, super::super::BlockKind::ListItem { .. })
            || whole_line && matches!(block.kind, super::super::BlockKind::Heading(_)))
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
            if !at_end && block.style.0 == "Code Block" {
                if let Some(fence) = super::super::markdown_code::fenced_source(document, block)? {
                    return Ok(fence.opening);
                }
            }
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
    let retained_empty = projection
        .blocks_for_region(&(range.end..range.end))
        .into_iter()
        .find(|block| block.range == (range.end..range.end)
            && block.kind == BlockKind::Paragraph);
    let start = if range.start < first.range.start {
        projection
            .source_range(range.start..first.range.start)
            .ok_or(DocumentError::AmbiguousProjection)?
            .start
    } else {
        source_line(first, false)?.start
    };
    let mut end = if range.end > last.range.end {
        let next = projection
            .blocks_for_region(&(range.end..range.end))
            .into_iter()
            .find(|block| block.range.start == range.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let next_start = source_line(&next, false)?.start;
        let last_end = source_line(last, true)?.end;
        // Blank lines between the deleted lines and the next block separate
        // it from the content above. Keep them unless a blank line already
        // precedes the deletion (or they spell retained empty paragraphs);
        // otherwise a following paragraph would become a lazy continuation of
        // the preceding item.
        let lines = &document.state().source_hard_lines;
        let separated_above = start == 0 || lines.line_at_offset(start - 1)
            .and_then(|index| lines.get(index))
            .and_then(|line| document.state().source.bytes_in(line))
            .map(|bytes| document.encoding().decode_region(&bytes, 0).is_ok_and(|text| text.text.trim().is_empty()))
            .unwrap_or(true);
        let only_blank_between = last_end < next_start && document.state().source.bytes_in(last_end..next_start)
            .map(|bytes| document.encoding().decode_region(&bytes, last_end).is_ok_and(|text| text.text.trim().is_empty()))
            .unwrap_or(false);
        if only_blank_between && !separated_above && retained_empty.is_none() { last_end } else { next_start }
    } else {
        source_line(last, true)?.end
    };
    let mut replacement = Vec::new();
    if whole_line {
        if let Some(next) = projection.blocks_for_region(&(range.end..range.end)).into_iter()
            .find(|block| block.range.start == range.end && !block.range.is_empty())
        {
            if let Some((prefix_end, prefix)) = retained_descendant_prefix(document, &paragraphs, &next)? {
                // Empty ancestor items can share the descendant's marker row.
                // Retain those owners without inventing visible empty leaves,
                // and leave the descendant's own marker/body bytes untouched.
                end = prefix_end;
                replacement = document.encoding().encode_fragment(&prefix)?;
            }
        }
    }
    if retained_empty.is_some() && range.start == first.range.start && range.start > 0 {
        // An item may follow its parent or sibling after a single physical
        // ending. Removing its marker leaves that retained boundary beside
        // the empty paragraph's separator. Promote the single ending to a
        // pair so the unselected empty paragraph survives the new blank run.
        let boundary = range.start - 1..range.start;
        if let Some(source) = projection.provenance_for_region(&boundary).into_iter()
            .find(|span| span.formatted == boundary && !span.source.is_empty())
            .map(|span| span.source)
            .filter(|source| source.len() <= 4)
        {
            let bytes = document.state().source.bytes_in(source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document.encoding().decode_region(&bytes, source.start)?;
            if normalize(&decoded, document.file_format()).text == "\n" {
                replacement = document.encoding().encode_fragment(document.file_format().spelling())?;
            }
        }
    }
    Ok(Some(vec![SourcePatch::primary(start..end, replacement)]))
}

fn retained_descendant_prefix(
    document: &Document,
    selected: &[super::super::Block],
    next: &super::super::Block,
) -> Result<Option<(usize, String)>, DocumentError> {
    // Only deleting a marker-bearing paragraph removes an item owner. Later
    // paragraphs may share that owner, including the first retained owner of
    // a join, and must neither duplicate nor reintroduce its marker.
    let ancestors = selected.iter().filter(|block|
        matches!(block.kind, BlockKind::ListItem { item_start: true, .. }))
        .filter(|block| block.list_item_owner()
        .is_some_and(|owner| next.containers.iter().any(|member| member.container.id == owner)))
        .collect::<Vec<_>>();
    if ancestors.is_empty() { return Ok(None); }
    let prefix = |block: &super::super::Block| -> Result<Option<(super::super::encoding::DecodedText, usize, usize)>, DocumentError> {
        let body = document.projection().source_insertion_point(block.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let row = document.state().source_hard_lines.line_at_offset(body)
            .and_then(|index| document.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = document.state().source.bytes_in(row.start..body)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, row.start)?;
        let quote = super::super::markdown_quotes::prefix(&decoded.text);
        let rest = &decoded.text[quote..];
        let Some(marker_end) = super::super::markdown_blocks::marker_prefix_length(rest) else { return Ok(None); };
        // The parsed item establishes this structural marker. Its visible
        // caret can lie after inline openers, which remain with their body and
        // are not part of the retained ancestor prefix. Multiple marker owners
        // on this physical row still need a separate structural rule.
        let after = &rest[marker_end..];
        if super::super::markdown_blocks::marker_prefix_length(after)
            .is_some_and(|end| end < after.len() || after.ends_with([' ', '\t'])) {
            return Ok(None);
        }
        let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        Ok(Some((decoded, quote + indent, quote + marker_end)))
    };
    let mut retained = String::new();
    for (index, ancestor) in ancestors.iter().enumerate() {
        if ancestor.quote_depth != next.quote_depth { return Ok(None); }
        let Some((decoded, marker, end)) = prefix(ancestor)? else { return Ok(None); };
        retained.push_str(if index == 0 { &decoded.text[..end] } else { &decoded.text[marker..end] });
    }
    let Some((decoded, marker, _)) = prefix(next)? else { return Ok(None); };
    let end = decoded.source_boundary(marker).ok_or(DocumentError::AmbiguousProjection)?;
    Ok(Some((end, retained)))
}

pub(super) fn empty_insertion_patches(
    document: &Document,
    range: &Range<usize>,
    text: &str,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown
        || !range.is_empty()
        || text.is_empty()
        || text.contains('\n')
    {
        return Ok(None);
    }
    if !document
        .projection()
        .blocks_for_region(range)
        .iter()
        .any(|block| {
            block.range.start == range.start
                && block.style.0 != "Code Block"
                && matches!(
                    block.kind,
                    super::super::BlockKind::ListItem {
                        item_start: true,
                        ..
                    }
                )
        })
    {
        return Ok(None);
    }
    let at = super::super::source_edit::insertion_point(document.projection(), range.start, None)
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
    // A hidden inline opener is also before the first visible character, but
    // it is content syntax, not a bare list marker that needs padding.
    let mut remaining = prefix.as_str();
    let mut has_list_marker = false;
    loop {
        let quote = super::super::markdown_quotes::prefix(remaining);
        remaining = &remaining[quote..];
        if remaining.trim_matches([' ', '\t']).is_empty() {
            break;
        }
        let Some(marker) = super::super::markdown_blocks::marker_prefix_length(remaining) else {
            return Ok(None);
        };
        if marker == remaining.len() && !remaining.ends_with([' ', '\t']) && at < line.end {
            // A prefix ending in `*` is ambiguous in isolation: the next
            // source unit distinguishes an empty marker from `*word*`.
            let end = (at + document.encoding().scalar_source_width(' ')).min(line.end);
            let following = document
                .state()
                .source
                .bytes_in(at..end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let following = document.encoding().decode_region(&following, at)?.text;
            if !following.starts_with([' ', '\t', '\r', '\n']) {
                return Ok(None);
            }
        }
        has_list_marker = true;
        remaining = &remaining[marker..];
    }
    if !has_list_marker {
        return Ok(None);
    }
    let needs_padding = !prefix.ends_with([' ', '\t']);
    let needs_break = document.projection().hard_breaks_for_region(
        &(range.start..(range.start + 1).min(document.projection().text_tree().byte_len())),
    ).contains(&range.start) && document.projection().blocks_for_region(range).iter()
        .any(|block| block.range.start == range.start && range.start < block.range.end);
    let following_block = document
        .projection()
        .blocks_for_region(
            &(range.start..(range.start + 1).min(document.projection().text_tree().byte_len())),
        )
        .into_iter()
        .find(|block| block.range.start == range.start + 1
            && (block.style.0 == "Code Block"
                || matches!(block.kind, super::super::BlockKind::Paragraph)));
    let needs_separator = if let Some(block) = following_block {
        let source = document
            .projection()
            .source_range(block.range)
            .ok_or(DocumentError::AmbiguousProjection)?;
        document
            .state()
            .source_hard_lines
            .line_at_offset(source.start)
            .and_then(|index| document.state().source_hard_lines.get(index))
            .is_some_and(|code_line| code_line.start == line.end)
    } else {
        false
    };
    if !needs_padding && !needs_separator && !needs_break {
        return Ok(None);
    }
    let mut syntax = format!(
        "{}{}",
        if needs_padding { " " } else { "" },
        document.escape_markdown_source_text(at, text)?
    );
    // Prose or indented code can follow an empty item without becoming its
    // body. Filling the item makes that ending a lazy prose continuation;
    // keep the unselected owner through the smallest blank separator.
    if needs_separator {
        syntax.push_str(document.file_format().spelling());
    } else if needs_break {
        // Empty marker padding can contribute a retained hard break. Filling
        // the marker would consume that padding and turn its ending soft.
        syntax.push('\\');
    }
    Ok(Some(vec![SourcePatch::primary(
        at..at,
        document.encoding().encode_fragment(&syntax)?,
    )]))
}

/// An empty item cannot interrupt preceding prose, including its parent.
/// A dash can instead underline that prose as a Setext heading. Preserve the
/// item with a local blank separator when an edit creates that spelling.
pub(super) fn preserve_empty_item_boundaries(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    for edit in edits.iter().filter(|edit| edit.replacement.is_empty()) {
        for block in projection.blocks_for_region(&edit.range) {
            if block.range.is_empty() || edit.range != block.range
                || !matches!(block.kind, BlockKind::ListItem { item_start: true, .. }) {
                continue;
            }
            if block.containers.iter().rposition(|member|
                member.container.kind == super::super::ContainerKind::ListItem)
                .is_some_and(|item| block.containers[item + 1..].iter().any(|member|
                    member.container.kind == super::super::ContainerKind::Quote))
            {
                // A heading already supplies an empty leaf, and the sole
                // document paragraph has an empty fallback. Neither needs
                // synthetic body syntax that would survive later cc typing.
                if block.style.0.starts_with("Heading") || projection.block_count() == 1 {
                    continue;
                }
                // An empty quote nested inside an item otherwise has no body
                // leaf on reopen. Keep the selected paragraph as an explicit
                // passive empty body, without retaining any selected text.
                let end = projection.source_insertion_point(block.range.end, false)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if let Some(patch) = patches.iter_mut().find(|patch|
                    patch.range.start < end && end <= patch.range.end && patch.replacement.is_empty()) {
                    patch.replacement = document.encoding().encode_fragment("<span></span>")?;
                }
                continue;
            }
            let next = projection.blocks_for_region(&(block.range.end + 1..block.range.end + 1))
                .into_iter().find(|next| next.range.start == block.range.end + 1);
            if !next.as_ref().is_some_and(|next|
                matches!(next.kind, BlockKind::ListItem { item_start: false, .. })) {
                continue;
            }
            let at = projection.source_insertion_point(block.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let Some(row) = document.state().source_hard_lines.line_at_offset(at)
                .and_then(|index| document.state().source_hard_lines.get(index + 1)) else { continue; };
            let bytes = document.state().source.bytes_in(row.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document.encoding().decode_region(&bytes, row.start)?;
            let text = decoded.text.trim_end_matches(['\r', '\n']);
            if text[super::super::markdown_quotes::prefix(text)..].trim().is_empty()
                && !patches.iter().any(|patch| patch.range.start < row.end && row.start < patch.range.end) {
                if next.as_ref().unwrap().style.0 == "Code Block" {
                    // Literal code keeps a boundary after a bare marker.
                    patches.push(SourcePatch::primary(row, Vec::new()));
                } else {
                    // Prose needs its retained separator and an empty first
                    // body; otherwise it collapses or leaves the item's owner.
                    let end = projection.source_insertion_point(block.range.end, false)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    if let Some(patch) = patches.iter_mut().find(|patch|
                        patch.range.start < end && end <= patch.range.end && patch.replacement.is_empty()) {
                        patch.replacement = document.encoding().encode_fragment("<span></span>")?;
                    }
                }
            }
        }
    }
    let mut candidates = std::collections::BTreeMap::new();
    for edit in edits.iter().filter(|edit| {
        edit.replacement.is_empty() || edit.replacement.contains('\n')
    }) {
        for block in projection.blocks_for_region(&edit.range) {
            let BlockKind::ListItem { level, item_start: true, .. } = block.kind else {
                continue;
            };
            if edit.range.start > block.range.start || block.range.start == 0 {
                continue;
            }
            let Some(previous) = projection.blocks_for_region(&(block.range.start - 1..block.range.start))
                .into_iter().filter(|previous| previous.range.end < block.range.start)
                .max_by_key(|previous| previous.range.start) else { continue; };
            if previous.range.is_empty() || edit.range.start < previous.range.end
                || previous.style.0.starts_with("Heading") || previous.style.0 == "Code Block"
                || previous.markdown_html
                || !(previous.kind == BlockKind::Paragraph
                    || matches!(previous.kind, BlockKind::ListItem { level: parent, .. } if parent < level))
            {
                continue;
            }
            let boundary = previous.range.end..previous.range.end + 1;
            if let Some(boundary) = projection.provenance_for_region(&boundary).into_iter()
                .find(|span| span.formatted == boundary && !span.source.is_empty())
                .map(|span| span.source)
            {
                if markdown_block_styles::explicit_paragraph_separator(document, &boundary, patches)? {
                    continue;
                }
            }
            let at = super::super::source_edit::insertion_point(projection, block.range.start, None)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let line = document.state().source_hard_lines.line_at_offset(at)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = document.state().source.bytes_in(line.start..at.min(line.start + 2048))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = document.encoding().decode_region(&prefix, line.start)?.text;
            let prefix = &prefix[super::super::markdown_quotes::prefix(&prefix)..];
            let indent = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
            candidates.insert(line.start, (prefix[..indent].to_owned(), block.quote_depth));
        }
    }
    if candidates.is_empty() { return Ok(()); }
    let mut sorted = patches.clone();
    sorted.sort_by_key(|patch| (patch.range.start, patch.range.end));
    let mut source = document.state().source.clone();
    for patch in sorted.iter().rev() {
        source = source.replace(patch.range.start, patch.range.end, patch.replacement.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
    }
    let mut support = std::collections::BTreeMap::new();
    for (old_start, (indent, depth)) in candidates {
        let mut start = old_start;
        let mut inside = false;
        for patch in &sorted {
            if patch.range.end <= old_start && !patch.range.is_empty()
                || patch.range.end < old_start
            {
                start = start.checked_sub(patch.range.len())
                    .and_then(|start| start.checked_add(patch.replacement.len()))
                    .ok_or(DocumentError::AmbiguousProjection)?;
            } else if patch.range.start < old_start && old_start < patch.range.end {
                start -= old_start - patch.range.start;
                inside = true;
                break;
            }
        }
        let end = (start + 2048).min(source.len());
        let bytes = source.bytes_in(start..end).ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, start)?;
        let normalized = super::super::line_endings::normalize(&decoded, document.file_format());
        let (row_start, text_start) = if inside {
            let Some(ending) = normalized.endings.first() else { continue; };
            (ending.source.end, ending.normalized.end)
        } else { (start, 0) };
        let ending = normalized.endings.iter().find(|ending| text_start <= ending.normalized.start);
        if ending.is_none() && end < source.len() { continue; }
        let row_end = ending.map_or(normalized.text.len(), |ending| ending.normalized.start);
        let row = &normalized.text[text_start..row_end];
        let quote = super::super::markdown_quotes::prefix(row);
        let row = &row[quote..];
        if row.starts_with(&indent)
            && super::super::markdown_blocks::marker_prefix_geometry(&row[indent.len()..])
                .is_some_and(|(body, _)| row[indent.len() + body..].trim_matches([' ', '\t']).is_empty())
        {
            support.insert(row_start, depth);
        }
    }
    if support.is_empty() { return Ok(()); }
    let mut composition = super::replacement::PatchComposition::new(document.source_byte_len());
    for patch in sorted.iter().rev() { composition.splice(patch.range.clone(), &patch.replacement); }
    for (at, depth) in support.into_iter().rev() {
        let blank = document.encoding().encode_fragment(&format!("{}{}",
            "> ".repeat(depth as usize), document.file_format().spelling()))?;
        composition.splice(at..at, &blank);
    }
    *patches = composition.source_patches();
    Ok(())
}

/// Plain register paragraphs inherit the destination list, while their labels
/// remain absent from the payload. Existing source labels are never rewritten.
pub(super) fn insertion_patches(
    document: &Document,
    edit: &FormattedPayloadEdit,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    paragraph_insertion_patches(document, &edit.range, edit.payload.text(),
        edit.payload.break_offsets(), edit.boundary_affinity)
}

/// Native text intentions and captured payloads share list paragraph syntax.
/// Reusing the item's marker retains its continuation/code indentation even
/// when the existing body begins on the far side of an inserted paragraph.
pub(super) fn text_insertion_patches(
    document: &Document,
    edit: &TextEdit,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if !edit.replacement.contains('\n') { return Ok(None); }
    let breaks = edit.replacement.match_indices('\n').map(|(at, _)| at).collect::<Vec<_>>();
    paragraph_insertion_patches(document, &edit.range, &edit.replacement, &breaks, None)
}

fn paragraph_insertion_patches(
    document: &Document,
    range: &Range<usize>,
    text: &str,
    breaks: &[usize],
    affinity: Option<BoundaryAffinity>,
) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
    if document.format() != Format::Markdown
        || !range.is_empty()
        || breaks.is_empty()
    {
        return Ok(None);
    }
    let at = range.start;
    let block = document
        .projection()
        .blocks_for_region(&(at..at))
        .into_iter()
        .find(|block| block.range.start <= at && at <= block.range.end);
    let Some(block) = block.filter(|block| block.style.0 != "Code Block") else {
        return Ok(None);
    };
    let super::super::BlockKind::ListItem {
        ordered, ordinal, level, ..
    } = block.kind
    else {
        return Ok(None);
    };
    let blocks = document.projection().list_indentation_blocks();
    // Keep the first touching block selected above, including empty bodies at
    // a shared boundary. The usual point lookup instead selects the last one.
    let index = if at == 0 { 0 } else { blocks.partition_point_end(at - 1) };
    if blocks.get(index).is_none_or(|candidate| candidate.id != block.id) {
        return Err(DocumentError::AmbiguousProjection);
    }
    let item_owner = block.list_item_owner().ok_or(DocumentError::AmbiguousProjection)?;
    let key = u16::from(level) + 1;
    let owner_index = blocks.find_navigation(0..index + 1, true, |summary| {
        summary.minimum < key || summary.minimum_start <= key
    }).ok_or(DocumentError::AmbiguousProjection)?;
    let owner = blocks.get(owner_index).filter(|owner| {
        matches!(owner.kind, super::super::BlockKind::ListItem {
            level: owner_level, item_start: true, ..
        } if owner_level == level) && owner.list_item_owner() == Some(item_owner)
    }).ok_or(DocumentError::AmbiguousProjection)?;
    let body_at = document
        .projection()
        .source_insertion_point(owner.range.start, true)
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
    let quote = super::super::markdown_quotes::prefix(&decoded.text);
    let quote_prefix = &decoded.text[..quote];
    let Some(prefix_len) =
        super::super::markdown_blocks::marker_prefix_length(&decoded.text[quote..])
    else {
        return Ok(None);
    };
    let prefix = &decoded.text[quote..quote + prefix_len];
    let indent_len = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
    let indent = &prefix[..indent_len];
    let delimiter = prefix[indent_len..]
        .chars()
        .find(|ch| !ch.is_ascii_digit())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let insert_before_label = at == owner.range.start
        && breaks
            .last()
            .is_some_and(|last| *last + 1 == text.len());
    let mut syntax = String::new();
    if insert_before_label {
        syntax.push_str(quote_prefix);
        syntax.push_str(indent);
        if ordered {
            syntax.push_str(&ordinal.to_string());
        }
        syntax.push(delimiter);
        syntax.push(' ');
    }
    let mut start = 0;
    let mut number = ordinal;
    // Text after a written marker begins the item body, where whitespace
    // would extend the marker padding and markers would nest a list.
    let push_segment = |syntax: &mut String, segment: &str, after_marker: bool| {
        let escaped = escape_markdown_insert_in_encoding(segment, document.encoding());
        if after_marker {
            super::protect_markdown_line_start(&escaped, syntax);
        } else {
            syntax.push_str(&escaped);
        }
    };
    for &boundary in breaks {
        push_segment(&mut syntax, &text[start..boundary], start > 0 || insert_before_label);
        syntax.push_str(document.file_format().spelling());
        if !(insert_before_label && boundary + 1 == text.len()) {
            syntax.push_str(quote_prefix);
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
    push_segment(&mut syntax, &text[start..], true);
    let source_at = if insert_before_label {
        line.start
    } else {
        document
            .projection()
            .source_insertion_point(
                at,
                at == block.range.start || affinity != Some(BoundaryAffinity::Upstream),
            )
            .ok_or(DocumentError::AmbiguousProjection)?
    };
    if !insert_before_label && block.range.is_empty() && !prefix.ends_with([' ', '\t']) {
        syntax.insert(0, ' ');
    }
    Ok(Some(vec![SourcePatch::primary(
        source_at..source_at,
        document.encoding().encode_fragment(&syntax)?,
    )]))
}

/// Retire only a separator created by empty-item indentation. Its source identity
/// is recorded by the structural transaction, so authored loose lists stay loose.
pub(super) fn retire_supporting_separator(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let Some(separator) = &document.state().transient_list_separator else { return Ok(()); };
    if document.format() != Format::Markdown { return Ok(()); }
    for edit in edits {
        if !edit.range.is_empty() || edit.replacement.is_empty()
            || edit.replacement.chars().all(char::is_whitespace) { continue; }
        let Some(block) = document.projection().blocks_for_region(&edit.range).into_iter()
            .find(|block| block.range == edit.range
                && matches!(block.kind, BlockKind::ListItem { level: 1.., .. })) else { continue; };
        let at = document.projection().source_insertion_point(block.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let Some(row) = document.state().source_hard_lines.line_at_offset(at)
            .and_then(|index| document.state().source_hard_lines.get(index)) else { continue; };
        if row.start == separator.end && !patches.iter().any(|patch|
            patch.range.start < separator.end && separator.start < patch.range.end) {
            patches.push(SourcePatch::primary(separator.clone(), Vec::new()));
            break;
        }
    }
    Ok(())
}
