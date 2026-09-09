//! Supporting syntax edits when a flowed Markdown paragraph becomes a heading
//! or list item, or when removing that syntax would join neighboring prose.
use super::{Document, DocumentError, SourcePatch, TextEdit};
use crate::document::{line_endings, paragraph_flow, BlockKind};
use std::collections::BTreeSet;
use std::ops::Range;

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
        if projection.markdown_replacement_begins_in_code(&edit.range) {
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
            if !normalized.endings.is_empty() && normalized.text.chars().all(char::is_whitespace) {
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
    let Some(character) = document.text()[at..]
        .chars()
        .next()
        .filter(char::is_ascii_punctuation)
    else {
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
    }))
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
        let Some(index) = projection.hard_line_at_offset(range.end) else {
            continue;
        };
        let Some(line) = projection.hard_line_range(index) else {
            continue;
        };
        if range.start < line.start {
            continue;
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
            if block.style.0 == "Code Block"
                || crate::document::markdown_quotes::is_fenced_block(document, &block)?
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
