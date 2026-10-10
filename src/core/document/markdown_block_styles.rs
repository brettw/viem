//! Supporting syntax edits when a flowed Markdown paragraph becomes a heading
//! or list item, or when removing that syntax would join neighboring prose.
use super::{Document, DocumentError, SourcePatch, TextEdit};
use crate::document::{line_endings, paragraph_flow, BlockKind};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Removing literal text can expose punctuation as newly active Markdown:
/// deleting `!` from an unsupported image spelling, for example, must leave
/// the displayed brackets rather than silently turn them into a hidden link.
/// Reparse only the affected physical lines and escape only retained literal
/// contributors which that candidate would consume as syntax.
pub(super) fn preserve_retained_literals(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    preserve_retained_literals_in_regions(document, edits, &[], patches)
}

/// Repairing a paired inline scope can activate previously literal markers
/// anywhere within that scope. Those grammar dependencies extend inspection,
/// while only the original requested ranges count as selected content.
pub(super) fn preserve_retained_literals_in_regions(
    document: &Document,
    edits: &[TextEdit],
    regions: &[Range<usize>],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let mut support = Vec::new();
    for edit in edits {
        if document
            .projection()
            .markdown_replacement_begins_in_code(&edit.range)
            || document
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| block.markdown_html)
        {
            continue;
        }
        let text = document.projection().text_tree();
        let is_boundary = |ch: char| ch.is_ascii_punctuation() || matches!(ch, ' ' | '\t');
        let mut start = edit.range.start;
        let mut end = edit.range.end;
        for region in regions.iter().filter(|region| {
            region.start <= edit.range.end && edit.range.start <= region.end
        }) {
            start = start.min(region.start);
            end = end.max(region.end);
        }
        while let Some(previous) = document.previous_grapheme_boundary(start) {
            if !text
                .slice(previous..start)
                .map_err(DocumentError::FormattedTextStorage)?
                .chars()
                .all(is_boundary)
            {
                break;
            }
            start = previous;
        }
        while let Some(next) = document.next_grapheme_boundary(end) {
            if !text
                .slice(end..next)
                .map_err(DocumentError::FormattedTextStorage)?
                .chars()
                .all(is_boundary)
            {
                break;
            }
            end = next;
        }
        let spans = document
            .projection()
            .provenance_for_region(&(start..end))
            .into_iter()
            .filter(|span| {
                !span.formatted.is_empty()
                    && !span.source.is_empty()
                    && !edits.iter().any(|selected| {
                        selected.range.start < span.formatted.end
                            && span.formatted.start < selected.range.end
                    })
                    && !patches.iter().any(|patch| {
                        patch.range.start < span.source.end && span.source.start < patch.range.end
                    })
                    && text
                        .slice(span.formatted.clone())
                        .is_ok_and(|text| text.chars().all(|ch| ch.is_ascii_punctuation()))
            })
            .collect::<Vec<_>>();
        let Some(last) = spans.last() else {
            continue;
        };
        let Some(candidate) =
            project_local_candidate(document, &(start..end), &last.source, patches, false)?
        else {
            continue;
        };
        let projected = crate::document::projection::project(
            &candidate.normalized,
            super::Format::Markdown,
            document.revision(),
            0,
            candidate.source_len,
        );
        for span in spans {
            let visible = text
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if document.state().source.bytes_in(span.source.clone())
                != Some(document.encoding().encode_fragment(&visible)?)
            {
                continue;
            }
            let delta = candidate
                .patches
                .iter()
                .filter(|patch| patch.range.end <= span.source.start)
                .map(|patch| patch.replacement.len() as isize - patch.range.len() as isize)
                .sum::<isize>();
            let at = (span.source.start - candidate.source_start)
                .checked_add_signed(delta)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let after = at..at + span.source.len();
            if !projected
                .provenance_contained_in_source(&after)
                .iter()
                .any(|mapped| {
                    mapped.source == after
                        && !mapped.formatted.is_empty()
                        && projected
                            .text_tree()
                            .slice(mapped.formatted.clone())
                            .as_deref()
                            == Ok(visible.as_str())
                })
            {
                escape_retained_punctuation(document, span.formatted.start, &mut support)?;
            }
        }
    }
    support.sort_by_key(|patch| (patch.range.start, patch.range.end));
    support.dedup_by(|left, right| left.range == right.range);
    patches.extend(support);
    Ok(())
}

/// A consumed physical break also owns the following line's hidden quote or
/// list-continuation prefix. Otherwise that prefix becomes visible mid-line.
pub(super) fn preserve_deleted_source_prefixes(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let mut support = Vec::new();
    for edit in edits
        .iter()
        .filter(|edit| !edit.range.is_empty() || edit.replacement.contains('\n'))
    {
        let band = edit.range.start.saturating_sub(1)
            ..(edit.range.end + 1).min(document.projection().text_tree().byte_len());
        for span in document.projection().provenance_for_region(&band) {
            if span.source.is_empty() {
                continue;
            }
            let visible = document
                .projection()
                .text_tree()
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if visible != " " && visible != "\n" {
                continue;
            }
            let blocks = document.projection().blocks_for_region(&span.formatted);
            if blocks.iter().any(|block| block.style.0 == "Code Block") {
                continue;
            }
            let bytes = document
                .state()
                .source
                .bytes_in(span.source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document
                .encoding()
                .decode_region(&bytes, span.source.start)?;
            let normalized = line_endings::normalize(&decoded, document.file_format());
            for ending in normalized.endings {
                // Retained-break replacements keep their physical prefix.
                if !patches.iter().any(|patch| {
                    patch.range.start <= ending.source.start
                        && ending.source.end <= patch.range.end
                        && !document
                            .encoding()
                            .decode_region(&patch.replacement, patch.range.start)
                            .is_ok_and(|decoded| {
                                decoded.text.contains(document.file_format().spelling())
                            })
                }) {
                    continue;
                }
                let Some(line) = document
                    .state()
                    .source_hard_lines
                    .line_at_offset(ending.source.end)
                    .and_then(|index| document.state().source_hard_lines.get(index))
                else {
                    continue;
                };
                let bytes = document
                    .state()
                    .source
                    .bytes_in(line.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = document.encoding().decode_region(&bytes, line.start)?;
                let mut prefix = super::super::markdown_quotes::prefix(&decoded.text);
                if blocks
                    .iter()
                    .any(|block| matches!(block.kind, BlockKind::ListItem { .. }))
                {
                    let body = &decoded.text[prefix..];
                    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
                    let marker = super::super::markdown_blocks::marker_prefix_length(body);
                    let hidden_marker = if let Some(length) = marker {
                        let start = line.start + document.encoding().encode_fragment(&decoded.text[..prefix + indent])?.len();
                        let end = line.start + document.encoding().encode_fragment(&decoded.text[..prefix + length])?.len();
                        (!document.projection().provenance_contained_in_source(&(start..end))
                            .iter().any(|span| !span.formatted.is_empty())).then_some(length)
                    } else { None };
                    // A numbered-looking continuation can be literal prose.
                    // Its digits and punctuation survive deletion of the fold.
                    prefix += hidden_marker.unwrap_or(indent);
                }
                if prefix > 0 {
                    let end = line.start
                        + document
                            .encoding()
                            .encode_fragment(&decoded.text[..prefix])?
                            .len();
                    super::super::source_edit::append_uncovered_deletions(
                        &(line.start..end),
                        patches,
                        &mut support,
                    );
                }
            }
        }
    }
    patches.extend(support);
    Ok(())
}

/// Deleting a physical-line body can expose a formerly folded newline or
/// list-prefix whitespace. Preserve each retained visible space as an entity
/// only when the locally reparsed source would otherwise consume it.
pub(super) fn preserve_deleted_boundary_spaces(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    let mut seen = BTreeSet::new();
    for edit in edits
        .iter()
        .filter(|edit| !edit.range.is_empty() || edit.replacement.contains('\n'))
    {
        let band = edit.range.start.saturating_sub(1)
            ..(edit.range.end + 1).min(projection.text_tree().byte_len());
        for span in projection.provenance_for_region(&band) {
            let visible = projection
                .text_tree()
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if !seen.insert((span.source.start, span.source.end))
                || span.source.is_empty()
                || edits.iter().any(|edit| {
                    edit.range.start < span.formatted.end && span.formatted.start < edit.range.end
                })
                || visible != " " && visible != "\n"
                || projection.markdown_replacement_begins_in_code(&span.formatted)
                || patches.iter().any(|patch| {
                    patch.range.start < span.source.end && span.source.start < patch.range.end
                })
            {
                continue;
            }
            let replacement = if visible == "\n" {
                // A retained hard break also needs protection when deleting
                // all of its preceding body turns the source row into a blank
                // separator. The candidate below decides whether it survives.
                let bytes = document
                    .state()
                    .source
                    .bytes_in(span.source.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = document
                    .encoding()
                    .decode_region(&bytes, span.source.start)?;
                let raw = line_endings::normalize(&decoded, document.file_format());
                if raw.text != "  \n" && raw.text != "\\\n" {
                    continue;
                }
                "<br>".to_owned()
            } else {
                "&#32;".to_owned()
            };
            let Some(candidate) =
                project_local_candidate(document, &band, &span.source, patches, false)?
            else {
                continue;
            };
            let projected = super::super::projection::project(
                &candidate.normalized,
                super::Format::Markdown,
                document.revision(),
                0,
                candidate.source_len,
            );
            let delta = candidate
                .patches
                .iter()
                .filter(|patch| patch.range.end <= span.source.start)
                .map(|patch| patch.replacement.len() as isize - patch.range.len() as isize)
                .sum::<isize>();
            let start = (span.source.start - candidate.source_start)
                .checked_add_signed(delta)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let end = start + span.source.len();
            let survives = projected.provenance().iter().any(|unit| {
                unit.source.start == start
                    && end <= unit.source.end
                    && projected
                        .text_tree()
                        .slice(unit.formatted.clone())
                        .as_deref()
                        == Ok(visible.as_str())
            });
            if !survives {
                patches.push(SourcePatch::primary(
                    span.source,
                    document.encoding().encode_fragment(&replacement)?,
                ));
            }
        }
    }
    Ok(())
}

/// Once an entire continuation body is gone, its old indentation is no
/// longer list syntax and would become literal text on the remaining row.
pub(super) fn remove_empty_continuation_prefixes(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let mut seen = BTreeSet::new();
    let mut support = Vec::new();
    for edit in edits.iter().filter(|edit| !edit.range.is_empty()) {
        if !document
            .projection()
            .blocks_for_region(&edit.range)
            .iter()
            .any(|block| {
                matches!(block.kind, BlockKind::ListItem { .. }) && block.style.0 != "Code Block"
            })
        {
            continue;
        }
        for span in document.projection().provenance_for_region(&edit.range) {
            let Some(line) = document
                .state()
                .source_hard_lines
                .line_at_offset(span.source.start)
                .and_then(|index| document.state().source_hard_lines.get(index))
            else {
                continue;
            };
            if !seen.insert(line.start) {
                continue;
            }
            let bytes = document
                .state()
                .source
                .bytes_in(line.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document.encoding().decode_region(&bytes, line.start)?;
            let quote_prefix = super::super::markdown_quotes::prefix(&decoded.text);
            let body = &decoded.text[quote_prefix..];
            let prefix = quote_prefix + body.len() - body.trim_start_matches([' ', '\t']).len();
            if prefix == quote_prefix
                || super::super::markdown_blocks::marker_prefix_length(body).is_some()
            {
                continue;
            }
            let normalized = line_endings::normalize(&decoded, document.file_format());
            let end = normalized
                .endings
                .last()
                .map_or(line.end, |ending| ending.source.start);
            let body_start = line.start
                + document
                    .encoding()
                    .encode_fragment(&decoded.text[..prefix])?
                    .len();
            if body_start >= end
                || document
                    .projection()
                    .provenance_contained_in_source(&(line.start..body_start))
                    .iter()
                    .any(|span| !span.formatted.is_empty())
            {
                continue;
            }
            let mut uncovered = Vec::new();
            super::super::source_edit::append_uncovered_deletions(
                &(body_start..end),
                patches,
                &mut uncovered,
            );
            if uncovered.is_empty()
                && !patches.iter().any(|patch| {
                    !patch.replacement.is_empty()
                        && patch.range.start < end
                        && body_start < patch.range.end
                })
            {
                super::super::source_edit::append_uncovered_deletions(
                    &(line.start
                        + document
                            .encoding()
                            .encode_fragment(&decoded.text[..quote_prefix])?
                            .len()..body_start),
                    patches,
                    &mut support,
                );
            }
        }
    }
    patches.extend(support);
    Ok(())
}

/// An emptied continuation paragraph joins its two physical separators into
/// one run. Preserve both retained logical boundaries: Markdown represents two
/// paragraph boundaries with four physical endings, not three.
pub(super) fn preserve_empty_continuation_paragraphs(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    let mut seen = BTreeSet::new();
    let mut deletions = edits
        .iter()
        .filter(|edit| !edit.range.is_empty() && edit.replacement.is_empty())
        .collect::<Vec<_>>();
    deletions.sort_by_key(|edit| (edit.range.start, edit.range.end));
    for edit in &deletions {
        for block in projection.blocks_for_region(&edit.range) {
            if block.range.is_empty()
                || block.style.0 == "Code Block"
                || !matches!(
                    block.kind,
                    BlockKind::Paragraph |
                    BlockKind::ListItem {
                        item_start: false,
                        ..
                    }
                )
                || block.range.start != edit.range.start
                || block.range.start == 0
                || block.range.end >= projection.text_tree().byte_len()
                || !seen.insert(block.id)
            {
                continue;
            }
            let mut deleted_end = block.range.start;
            for deletion in &deletions {
                if deletion.range.start == deleted_end {
                    deleted_end = deletion.range.end;
                }
            }
            if deleted_end != block.range.end {
                continue;
            }
            let boundaries = [
                block.range.start - 1..block.range.start,
                block.range.end..block.range.end + 1,
            ];
            if edits.iter().any(|other| {
                boundaries.iter().any(|boundary| {
                    other.range.start < boundary.end && boundary.start < other.range.end
                })
            }) {
                continue;
            }
            let mut count = 0;
            let mut implicit_boundary = false;
            for boundary in boundaries {
                let Some(source) = hard_boundary_contributor(document, boundary) else {
                    implicit_boundary = true;
                    count = 4;
                    break;
                };
                let bytes = document
                    .state()
                    .source
                    .bytes_in(source.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = document.encoding().decode_region(&bytes, source.start)?;
                let normalized = line_endings::normalize(&decoded, document.file_format());
                if !normalized.text.split('\n').all(|line| {
                    line[super::super::markdown_quotes::prefix(line)..]
                        .chars()
                        .all(char::is_whitespace)
                }) {
                    implicit_boundary = true;
                    count = 4;
                    break;
                }
                count += normalized.endings.len();
            }
            if implicit_boundary && block.kind == BlockKind::Paragraph && !block.markdown_html {
                // Hidden structural syntax (for example a setext underline)
                // owns a boundary here. Retain the emptied paragraph's body
                // without changing that syntax or an unselected neighbor.
                let at = projection.source_insertion_point(block.range.end, false)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if let Some(patch) = patches.iter_mut().find(|patch| {
                    patch.range.end == at && patch.replacement.is_empty() && !patch.range.is_empty()
                }) {
                    patch.replacement = document.encoding().encode_fragment("<span></span>")?;
                    continue;
                }
            }
            if count >= 4 {
                continue;
            }
            let at = projection
                .source_insertion_point(block.range.end, false)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_line = document
                .state()
                .source_hard_lines
                .line_at_offset(at)
                .and_then(|index| document.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = document
                .state()
                .source
                .bytes_in(source_line.start..at)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document
                .encoding()
                .decode_region(&bytes, source_line.start)?;
            let prefix = &decoded.text[..super::super::markdown_quotes::prefix(&decoded.text)];
            let separator = format!("{}{prefix}", document.file_format().spelling());
            let added = document
                .encoding()
                .encode_fragment(&separator.repeat(4 - count))?;
            if let Some(patch) = patches.iter_mut().find(|patch| patch.range.end == at) {
                patch.replacement.extend(added);
            } else {
                patches.push(SourcePatch::primary(at..at, added));
            }
        }
    }
    Ok(())
}

/// A new source line must not reinterpret retained text as block syntax or
/// turn an existing folded source break into another hard boundary. Preserve
/// only those exposed contributors; authored text keeps its own translation.
pub(super) fn preserve_split_literals(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    let mut support = Vec::new();
    for edit in edits {
        if projection.markdown_replacement_begins_in_code(&edit.range)
            || projection
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| block.markdown_html)
        {
            continue;
        }
        // Inserting an escaped delimiter can split a previously literal run
        // into matching code/emphasis delimiters. Escape the adjacent visible
        // run as one grammatical contributor, leaving semantic markers alone.
        for character in edit
            .replacement
            .chars()
            .filter(|character| matches!(character, '`' | '*' | '_'))
            .collect::<BTreeSet<_>>()
        {
            let mut start = edit.range.start;
            while document.text()[..start].ends_with(character) {
                start -= character.len_utf8();
            }
            let mut end = edit.range.end;
            while document.text()[end..].starts_with(character) {
                end += character.len_utf8();
            }
            for at in (start..edit.range.start).chain(edit.range.end..end) {
                escape_retained_punctuation(document, at, &mut support)?;
            }
        }
        if !edit.replacement.contains('\n') {
            continue;
        }
        let list = projection
            .blocks_for_region(&edit.range)
            .into_iter()
            .find(|block| {
                block.range.start <= edit.range.start
                    && edit.range.end <= block.range.end
                    && matches!(block.kind, BlockKind::ListItem { .. })
            });
        // Splitting a list's inherited scope can expose indentation on any
        // continuation line of that paragraph, not only beside the caret.
        let band = list.map_or_else(
            || {
                document
                    .previous_grapheme_boundary(edit.range.start)
                    .unwrap_or(edit.range.start)
                    ..document
                        .next_grapheme_boundary(edit.range.end)
                        .unwrap_or(edit.range.end)
            },
            |block| block.range,
        );
        let mut folded = Vec::new();
        for span in projection.provenance_for_region(&band) {
            if span.formatted.len() != 1
                || span.source.is_empty()
                || document.text().as_bytes().get(span.formatted.start) != Some(&b' ')
                || edits.iter().any(|edit| {
                    edit.range.start < span.formatted.end && span.formatted.start < edit.range.end
                })
            {
                continue;
            }
            let bytes = document
                .state()
                .source
                .bytes_in(span.source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = document
                .encoding()
                .decode_region(&bytes, span.source.start)?;
            let normalized = line_endings::normalize(&decoded, document.file_format());
            if !normalized.endings.is_empty()
                && normalized.text.split('\n').all(|row| {
                    row[crate::document::markdown_quotes::prefix(row)..]
                        .chars()
                        .all(char::is_whitespace)
                })
            {
                folded.push(span);
            }
        }
        if let Some(last) = folded.last() {
            if let Some(candidate) =
                project_local_candidate(document, &band, &last.source, patches, false)?
            {
                for span in folded {
                    if !candidate.contributor_survives(&span.source, " ")? {
                        support.push(SourcePatch::primary(
                            span.source,
                            document.encoding().encode_fragment("&#32;")?,
                        ));
                    }
                }
            }
        }
        if !edit.replacement.ends_with('\n') {
            continue;
        }
        let tail = &document.text()[edit.range.end..];
        let end = tail.find('\n').unwrap_or(tail.len());
        let line = &tail[..end];
        let whitespace = line
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        for span in projection.provenance_for_region(&(edit.range.end..edit.range.end + whitespace))
        {
            if span.formatted.is_empty() || span.source.is_empty() {
                continue;
            }
            let body = projection
                .text_tree()
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if body.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
                let syntax = body
                    .bytes()
                    .map(|byte| if byte == b' ' { "&#32;" } else { "&#9;" })
                    .collect::<String>();
                support.push(SourcePatch::primary(
                    span.source,
                    document.encoding().encode_fragment(&syntax)?,
                ));
            }
        }
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let (_, kind) = crate::document::projection::markdown_block_prefix(line, 0, line.len());
        let punctuation = match kind {
            BlockKind::Heading(_) => Some(0),
            BlockKind::ListItem { ordered: true, .. } => Some(
                indent
                    + line[indent..]
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count(),
            ),
            BlockKind::ListItem { .. } => Some(indent),
            BlockKind::Paragraph
                if line[indent..].starts_with('>')
                    || crate::document::projection::markdown_fence(line).is_some() =>
            {
                Some(indent)
            }
            _ => None,
        };
        if let Some(offset) = punctuation {
            escape_retained_punctuation(document, edit.range.end + offset, &mut support)?;
        }
    }
    support.sort_by_key(|patch| (patch.range.start, patch.range.end));
    support.dedup_by(|left, right| left.range == right.range);
    for patch in support {
        if patches.iter().any(|existing| {
            existing.range.start < patch.range.end && patch.range.start < existing.range.end
        }) {
            continue;
        }
        patches.push(patch);
    }
    Ok(())
}

fn escape_retained_punctuation(
    document: &Document,
    at: usize,
    support: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let Some(end) = document.next_grapheme_boundary(at) else {
        return Ok(());
    };
    let text = document
        .projection()
        .text_tree()
        .slice(at..end)
        .map_err(DocumentError::FormattedTextStorage)?;
    let Some(character) = text.chars().next().filter(char::is_ascii_punctuation) else {
        return Ok(());
    };
    let range = at..at + character.len_utf8();
    let Some(span) = document
        .projection()
        .provenance_for_region(&range)
        .into_iter()
        .find(|span| span.formatted == range && !span.source.is_empty())
    else {
        return Ok(());
    };
    let literal = document
        .encoding()
        .encode_fragment(&character.to_string())?;
    if document
        .state()
        .source
        .bytes_in(span.source.clone())
        .as_deref()
        == Some(literal.as_slice())
    {
        support.push(SourcePatch::primary(
            span.source,
            document
                .encoding()
                .encode_fragment(&format!("\\{character}"))?,
        ));
    }
    Ok(())
}

/// Removing a boundary or indentation can make the joined line ordinary
/// flowed prose. Preserve its following hard boundary if the local candidate
/// would otherwise collapse it, without rewriting either paragraph's content.
pub(super) fn preserve_join_boundaries(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    let mut preserve = BTreeSet::new();
    for edit in edits {
        if edit.replacement.contains('\n')
            || edit.range.is_empty()
            || !projection
                .text_tree()
                .slice(edit.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
                .contains('\n')
        {
            continue;
        }
        let Some(last_line) = projection
            .hard_line_at_offset(edit.range.end)
            .and_then(|index| projection.hard_line_range(index))
        else {
            continue;
        };
        let formatted_boundary = last_line.end..last_line.end + 1;
        if formatted_boundary.end > projection.text_tree().byte_len()
            || edits.iter().any(|edit| {
                edit.range.start < formatted_boundary.end
                    && formatted_boundary.start < edit.range.end
            })
            || projection
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| block.style.0 == "Code Block")
        {
            continue;
        }
        let Some(boundary) = hard_boundary_contributor(document, formatted_boundary) else {
            continue;
        };
        if patches
            .iter()
            .any(|patch| patch.range.start < boundary.end && boundary.start < patch.range.end)
        {
            continue;
        }
        let source = &document.state().source;
        let spelling = source
            .bytes_in(boundary.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document
            .encoding()
            .decode_region(&spelling, boundary.start)?;
        if line_endings::normalize(&decoded, document.file_format()).text != "\n" {
            continue;
        }
        let include_preceding = edit.replacement.is_empty()
            && projection
                .blocks_for_region(&edit.range)
                .first()
                .is_some_and(|block| block.range.start == edit.range.start)
            && edit.range.end == last_line.end
            && edit.range.start > 0;
        let Some(still_hard) = source_contributor_survives(
            document,
            &edit.range,
            &boundary,
            patches,
            include_preceding,
            "\n",
        )?
        else {
            continue;
        };
        if !still_hard {
            preserve.insert(boundary.end);
        }
    }
    for at in preserve {
        let ending = document
            .encoding()
            .encode_fragment(document.file_format().spelling())?;
        if let Some(patch) = patches.iter_mut().find(|patch| patch.range.start == at) {
            patch.replacement.splice(0..0, ending);
        } else {
            patches.push(SourcePatch::primary(at..at, ending));
        }
    }
    Ok(())
}

// Delimiter preservation concerns the break's contributing bytes. Endpoint
// insertion associations can additionally encompass a neighboring empty block.
fn hard_boundary_contributor(document: &Document, range: Range<usize>) -> Option<Range<usize>> {
    document
        .projection()
        .provenance_for_region(&range)
        .into_iter()
        .find(|span| span.formatted == range && !span.source.is_empty())
        .map(|span| span.source)
}

/// A pair of physical endings separated only by blank quote prefixes is an
/// explicit prose separator. Its proof does not depend on the following body.
pub(super) fn explicit_paragraph_separator(
    document: &Document,
    boundary: &Range<usize>,
    patches: &[SourcePatch],
) -> Result<bool, DocumentError> {
    if boundary.len() > 32768 {
        return Ok(false);
    }
    let mut bytes = document.state().source.bytes_in(boundary.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut local = patches.iter().filter(|patch| {
        boundary.start < patch.range.start && patch.range.start <= boundary.end
            && patch.range.end <= boundary.end
    }).collect::<Vec<_>>();
    if boundary.len().saturating_add(local.iter()
        .map(|patch| patch.replacement.len()).sum::<usize>()) > 32768
        || patches.iter().any(|patch| !patch.range.is_empty()
            && patch.range.start < boundary.end && boundary.start < patch.range.end
            && (patch.range.start <= boundary.start || patch.range.end > boundary.end))
    {
        return Ok(false);
    }
    local.sort_by_key(|patch| (patch.range.start, patch.range.end));
    for patch in local.into_iter().rev() {
        bytes.splice(patch.range.start - boundary.start..patch.range.end - boundary.start,
            patch.replacement.iter().copied());
    }
    let decoded = document.encoding().decode_region(&bytes, boundary.start)?;
    let normalized = line_endings::normalize(&decoded, document.file_format());
    Ok(normalized.endings.len() >= 2 && normalized.text.split('\n').all(|line| {
        line[crate::document::markdown_quotes::prefix(line)..]
            .chars().all(|character| matches!(character, ' ' | '\t'))
    }))
}

/// Whether a retained source contributor still produces its original flowed
/// content after nearby syntax edits, without being absorbed into another unit.
fn source_contributor_survives(
    document: &Document,
    range: &Range<usize>,
    boundary: &Range<usize>,
    patches: &[SourcePatch],
    include_preceding: bool,
    expected: &str,
) -> Result<Option<bool>, DocumentError> {
    let Some(candidate) =
        project_local_candidate(document, range, boundary, patches, include_preceding)?
    else {
        return Ok(None);
    };
    Ok(Some(candidate.contributor_survives(boundary, expected)?))
}

struct LocalCandidate<'a> {
    source_start: usize,
    patches: Vec<&'a SourcePatch>,
    flowed: line_endings::NormalizedText,
    normalized: line_endings::NormalizedText,
    source_len: usize,
}

impl LocalCandidate<'_> {
    fn contributor_survives(
        &self,
        boundary: &Range<usize>,
        expected: &str,
    ) -> Result<bool, DocumentError> {
        let delta = self
            .patches
            .iter()
            .filter(|patch| patch.range.end <= boundary.start)
            .map(|patch| patch.replacement.len() as isize - patch.range.len() as isize)
            .sum::<isize>();
        let start = boundary
            .start
            .checked_sub(self.source_start)
            .and_then(|start| start.checked_add_signed(delta))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let end = start + boundary.len();
        Ok(self
            .flowed
            .units
            .binary_search_by_key(&start, |unit| unit.source.start)
            .ok()
            .and_then(|index| self.flowed.units.get(index))
            .is_some_and(|unit| {
                end <= unit.source.end && &self.flowed.text[unit.normalized.clone()] == expected
            }))
    }
}

fn project_local_candidate<'a>(
    document: &Document,
    range: &Range<usize>,
    boundary: &Range<usize>,
    patches: &'a [SourcePatch],
    include_preceding: bool,
) -> Result<Option<LocalCandidate<'a>>, DocumentError> {
    project_local_candidate_with_limit(document, range, boundary, patches, include_preceding, None)
}

fn project_local_candidate_with_limit<'a>(
    document: &Document,
    range: &Range<usize>,
    boundary: &Range<usize>,
    patches: &'a [SourcePatch],
    include_preceding: bool,
    byte_limit: Option<usize>,
) -> Result<Option<LocalCandidate<'a>>, DocumentError> {
    let projection = document.projection();
    let source = &document.state().source;
    let Some(start) = projection.source_insertion_point(range.start, false) else {
        return Ok(None);
    };
    let source_lines = &document.state().source_hard_lines;
    let Some(first) = source_lines
        .line_at_offset(start)
        .and_then(|index| source_lines.get(index))
    else {
        return Ok(None);
    };
    let Some(following) = source_lines
        .line_at_offset(boundary.end)
        .and_then(|index| source_lines.get(index))
    else {
        return Ok(None);
    };
    let mut band = first.start..following.end;
    // If all merged content disappears, the source delimiters on both
    // sides become one blank run. Include the preceding delimiter when
    // checking whether that run still represents two logical boundaries.
    if include_preceding {
        if let Some(before) = projection.source_range(range.start - 1..range.start) {
            band.start = before.start;
        }
    }
    // Supporting edits can touch an earlier continuation-line delimiter.
    // Include its whole physical line so the simulation has the same syntax
    // context instead of dropping the boundary check at a crossed edge.
    loop {
        let mut expanded = band.clone();
        for patch in patches
            .iter()
            .filter(|patch| patch.range.start <= band.end && band.start <= patch.range.end)
        {
            if patch.range.start < expanded.start {
                expanded.start = source_lines
                    .line_at_offset(patch.range.start)
                    .and_then(|index| source_lines.get(index))
                    .map_or(patch.range.start, |line| line.start);
            }
            if patch.range.end > expanded.end {
                expanded.end = source_lines
                    .line_at_offset(patch.range.end)
                    .and_then(|index| source_lines.get(index))
                    .map_or(patch.range.end, |line| line.end);
            }
        }
        if expanded == band {
            break;
        }
        band = expanded;
    }
    let mut local = patches
        .iter()
        .filter(|patch| patch.range.start <= band.end && band.start <= patch.range.end)
        .collect::<Vec<_>>();
    if byte_limit.is_some_and(|limit| band.len().saturating_add(local.iter()
        .map(|patch| patch.replacement.len()).sum::<usize>()) > limit)
    {
        return Ok(None);
    }
    local.sort_by_key(|patch| (patch.range.start, patch.range.end));
    let mut bytes = source
        .bytes_in(band.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    for patch in local.iter().rev() {
        bytes.splice(
            patch.range.start - band.start..patch.range.end - band.start,
            patch.replacement.iter().copied(),
        );
    }
    let decoded = document.encoding().decode_region(&bytes, 0)?;
    let normalized = line_endings::normalize(&decoded, document.file_format());
    let (flowed, _) = paragraph_flow::markdown(&normalized);
    Ok(Some(LocalCandidate {
        source_start: band.start,
        patches: local,
        flowed,
        normalized,
        source_len: bytes.len(),
    }))
}

/// Typing must retain paragraph boundaries which a neighboring line's grammar
/// can otherwise absorb, including empty prose and setext-like thematic rules.
pub(super) fn preserve_edited_paragraph_boundaries(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    // Empty paragraphs may be represented by a single physical ending before
    // a structural block. Once prose is typed there, that ending can fold or
    // become a setext underline boundary. Keep the untouched logical separator.
    let mut boundaries = BTreeMap::new();
    for edit in edits
        .iter()
        .filter(|edit| !edit.replacement.is_empty() && !edit.replacement.contains('\n'))
    {
        let Some(paragraph) = document
            .projection()
            .blocks_for_region(&edit.range)
            .into_iter()
            .find(|block| {
                block.range.start <= edit.range.start
                    && edit.range.end <= block.range.end
                    && block.kind == BlockKind::Paragraph
                    && !block.markdown_html
                    && !crate::document::edit_boundary::is_code_paragraph(document, block)
                        .unwrap_or(true)
            })
        else {
            continue;
        };
        let Some(index) = document.projection().hard_line_at_offset(edit.range.start) else {
            continue;
        };
        let Some(line) = document.projection().hard_line_range(index) else {
            continue;
        };
        if edit.range.end > line.end {
            continue;
        }
        let next_is_rule = document
            .projection()
            .hard_line_range(index + 1)
            .is_some_and(|next| {
                document
                    .projection()
                    .blocks_for_region(&next)
                    .iter()
                    .any(|block| block.thematic_break)
            });
        if !line.is_empty() && !next_is_rule {
            continue;
        }
        let Some(boundary) = hard_boundary_contributor(document, line.end..line.end + 1) else {
            continue;
        };
        if patches
            .iter()
            .any(|patch| patch.range.start < boundary.end && boundary.start < patch.range.end)
        {
            continue;
        }
        if explicit_paragraph_separator(document, &boundary, patches)? {
            continue;
        }
        let Some(candidate) = project_local_candidate_with_limit(
            document,
            &edit.range,
            &boundary,
            patches,
            false,
            Some(32768),
        )?
        else {
            // A double ending guarantees a prose boundary without parsing an
            // arbitrarily long following row merely to prove a single ending.
            boundaries.insert(boundary.end, paragraph.quote_depth);
            continue;
        };
        let delta = candidate
            .patches
            .iter()
            .filter(|patch| patch.range.end <= boundary.start)
            .map(|patch| patch.replacement.len() as isize - patch.range.len() as isize)
            .sum::<isize>();
        let start = (boundary.start - candidate.source_start)
            .checked_add_signed(delta)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mapped = start..start + boundary.len();
        let projected = crate::document::projection::project(
            &candidate.normalized,
            super::Format::Markdown,
            document.revision(),
            0,
            candidate.source_len,
        );
        if !projected
            .provenance()
            .iter()
            .any(|span| {
                span.source.start <= mapped.start && mapped.end <= span.source.end
                    && projected
                        .text_tree()
                        .slice(span.formatted.clone())
                        .as_deref()
                        == Ok("\n")
            })
        {
            boundaries.insert(boundary.end, paragraph.quote_depth);
        }
    }
    for (at, depth) in boundaries {
        let prefix = "> ".repeat(depth as usize);
        let suffix = document
            .encoding()
            .encode_fragment(&format!("{prefix}{}", document.file_format().spelling()))?;
        if let Some(patch) = patches.iter_mut().find(|patch| patch.range.start == at) {
            patch.replacement.splice(0..0, suffix);
        } else {
            patches.push(SourcePatch::primary(at..at, suffix));
        }
    }
    Ok(())
}
/// A split must preserve an existing following hard boundary even when the
/// inserted separator and that boundary become one source-whitespace run.
pub(super) fn preserve_split_boundaries<'a>(
    document: &Document,
    ranges: impl Iterator<Item = &'a Range<usize>>,
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let projection = document.projection();
    let mut boundaries = BTreeSet::new();
    for range in ranges {
        if projection.blocks_for_region(range).iter().any(|block| {
            block.range.start <= range.start && range.end <= block.range.end
                && crate::document::edit_boundary::is_code_paragraph(document, block)
                    .unwrap_or(false)
        }) {
            // Code owns literal internal endings. A paragraph-flow probe
            // without its opener would fold them and add spurious blank rows.
            continue;
        }
        let Some(index) = projection.hard_line_at_offset(range.end) else {
            continue;
        };
        let Some(line) = projection.hard_line_range(index) else {
            continue;
        };
        if range.start < line.start {
            continue;
        }
        // Opening a paragraph at its beginning also preserves the boundary
        // behind it. A preceding fence may contribute only one physical ending.
        if range.is_empty() && range.start == line.start && index > 0 {
            let previous = crate::document::edit_boundary::paragraph_at(document, line.start - 1)?;
            if let Some(previous) = previous.filter(|block| block.style.0 == "Code Block") {
                let fence = crate::document::markdown_code::fenced_source(document, &previous)?;
                let end = if let Some(fence) = fence {
                    fence.closing_content_end
                } else {
                    projection.source_insertion_point(previous.range.end, false)
                };
                {
                    if let Some(end) = end {
                        let at = projection
                            .source_insertion_point(range.start, true)
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        if let Some(patch) =
                            patches.iter_mut().find(|patch| patch.range == (at..at))
                        {
                            let bytes = document
                                .state()
                                .source
                                .bytes_in(end..at)
                                .ok_or(DocumentError::AmbiguousProjection)?;
                            let existing = document.encoding().decode_region(&bytes, end)?;
                            let added =
                                document.encoding().decode_region(&patch.replacement, at)?;
                            let endings =
                                line_endings::normalize(&existing, document.file_format())
                                    .endings
                                    .len()
                                    + line_endings::normalize(&added, document.file_format())
                                        .endings
                                        .len();
                            if existing.text.chars().all(char::is_whitespace)
                                && added.text.chars().all(char::is_whitespace) && endings < 4 {
                                patch
                                    .replacement
                                    .extend(document.encoding().encode_fragment(
                                        &document.file_format().spelling().repeat(4 - endings),
                                    )?);
                            }
                        }
                    }
                }
            }
        }
        if projection.hard_line_range(index + 1).is_none() {
            continue;
        }
        let Some(source) = hard_boundary_contributor(document, line.end..line.end + 1) else {
            continue;
        };
        if patches
            .iter()
            .any(|patch| patch.range.start < source.end && source.start < patch.range.end)
        {
            continue;
        }
        let bytes = document
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        let decoded = document.encoding().decode_region(&bytes, source.start)?;
        if line_endings::normalize(&decoded, document.file_format()).text == "\n"
            && source_contributor_survives(document, range, &source, patches, false, "\n")?
                == Some(false)
        {
            boundaries.insert(source.end);
        }
    }
    for at in boundaries {
        let suffix = document
            .encoding()
            .encode_fragment(document.file_format().spelling())?;
        // An edit at the following paragraph start must follow this separator.
        if let Some(patch) = patches.iter_mut().find(|patch| patch.range.start == at) {
            patch.replacement.splice(0..0, suffix);
        } else {
            patches.push(SourcePatch::primary(at..at, suffix));
        }
    }
    Ok(())
}

pub(super) fn support_patches(
    document: &Document,
    range: &Range<usize>,
    make_structural: bool,
    remove_structure: bool,
) -> Result<Vec<SourcePatch>, DocumentError> {
    support_patches_impl(document, range, make_structural, remove_structure, false)
}

pub(super) fn code_removal_patches(
    document: &Document,
    range: &Range<usize>,
) -> Result<Vec<SourcePatch>, DocumentError> {
    support_patches_impl(document, range, false, true, true)
}

fn support_patches_impl(
    document: &Document,
    range: &Range<usize>,
    make_structural: bool,
    remove_structure: bool,
    remove_code: bool,
) -> Result<Vec<SourcePatch>, DocumentError> {
    let projection = document.projection();
    let first = projection
        .hard_line_at_offset(range.start)
        .ok_or(DocumentError::VerificationFailed)?;
    let last = if range.is_empty() {
        first
    } else {
        projection
            .hard_line_at_offset(
                document
                    .previous_grapheme_boundary(range.end)
                    .ok_or(DocumentError::VerificationFailed)?,
            )
            .ok_or(DocumentError::VerificationFailed)?
    };
    let mut patches = Vec::new();
    if make_structural {
        for index in first..=last {
            let line = projection
                .hard_line_range(index)
                .ok_or(DocumentError::VerificationFailed)?;
            if line.is_empty() {
                continue;
            }
            let spans = projection.provenance_for_region(&line);
            let source_start = spans
                .first()
                .map(|span| span.source.start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_end = spans
                .last()
                .map(|span| span.source.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_lines = &document.state().source_hard_lines;
            let first_source = source_lines
                .line_at_offset(source_start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let last_source = source_lines
                .line_at_offset(source_end.saturating_sub(1).max(source_start))
                .ok_or(DocumentError::AmbiguousProjection)?;
            if first_source == last_source {
                continue;
            }
            let source = source_lines.get(first_source).unwrap().start
                ..source_lines.get(last_source).unwrap().end;
            let bytes = document
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = document.encoding().decode_region(&bytes, source.start)?;
            let normalized = line_endings::normalize(&decoded, document.file_format());
            let soft = paragraph_flow::markdown_soft_breaks(&normalized);
            for ending in normalized
                .endings
                .iter()
                .filter(|ending| soft.contains(&ending.normalized.start))
            {
                patches.push(SourcePatch::primary(
                    ending.source.clone(),
                    document.encoding().encode_fragment(" ")?,
                ));
            }
        }
    }
    if remove_structure {
        // Removing a quote/container does not yet remove a surviving fenced
        // body. Its literal internal endings must not be doubled as prose.
        let adjacent = projection
            .hard_line_range(first.saturating_sub(1))
            .unwrap()
            .start
            ..projection
                .hard_line_range((last + 1).min(projection.hard_line_count() - 1))
                .unwrap()
                .end;
        let mut fenced = BTreeSet::new();
        for block in projection.blocks_for_region(&adjacent) {
            if !remove_code
                && (block.style.0 == "Code Block"
                    || crate::document::markdown_quotes::is_fenced_block(document, &block)?)
            {
                fenced.insert(block.id);
            }
        }
        let mut boundaries = BTreeSet::new();
        for index in first..=last {
            if index > 0 {
                boundaries.insert(index - 1);
            }
            if index + 1 < projection.hard_line_count() {
                boundaries.insert(index);
            }
        }
        let resulting_prose = |index: usize| -> bool {
            let Some(line) = projection.hard_line_range(index) else {
                return false;
            };
            let Some(block) = projection
                .blocks_for_region(&line)
                .into_iter()
                .find(|block| block.range.start <= line.start && line.start <= block.range.end)
            else {
                return false;
            };
            if fenced.contains(&block.id) {
                return false;
            }
            if (first..=last).contains(&index) {
                matches!(
                    block.kind,
                    BlockKind::Paragraph | BlockKind::Heading(_) | BlockKind::ListItem { .. }
                )
            } else {
                block.kind == BlockKind::Paragraph && block.style.0 != "Code Block"
            }
        };
        for index in boundaries {
            let left = projection.hard_line_range(index).unwrap();
            let right = projection.hard_line_range(index + 1).unwrap();
            // A source continuation or literal code break is internal to one
            // paragraph. Only its outer boundaries need prose separators.
            if projection
                .blocks_for_region(&left)
                .iter()
                .any(|block| block.range.start <= left.start && right.end <= block.range.end)
            {
                continue;
            }
            let untouched_list = |at: usize| {
                !(first..=last).contains(&at)
                    && projection.hard_line_range(at).is_some_and(|line| {
                        projection
                            .blocks_for_region(&line)
                            .iter()
                            .any(|block| matches!(block.kind, BlockKind::ListItem { .. }))
                    })
            };
            if !(resulting_prose(index) && resulting_prose(index + 1)
                || (first..=last).contains(&index) && untouched_list(index + 1)
                || untouched_list(index) && (first..=last).contains(&(index + 1)))
            {
                continue;
            }
            let end = projection.hard_line_range(index).unwrap().end;
            let spans = projection.provenance_for_region(&(end..end + 1));
            let source = spans
                .iter()
                .find(|span| !span.formatted.is_empty())
                .zip(spans.iter().rev().find(|span| !span.formatted.is_empty()))
                .map(|(first, last)| first.source.start..last.source.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = document
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = document.encoding().decode_region(&bytes, source.start)?;
            let normalized = line_endings::normalize(&decoded, document.file_format());
            if normalized.text == "\n" {
                patches.push(SourcePatch::primary(
                    source.end..source.end,
                    document
                        .encoding()
                        .encode_fragment(document.file_format().spelling())?,
                ));
            }
        }
    }
    Ok(patches)
}
