//! Supporting syntax edits when a flowed Markdown paragraph becomes a heading
//! or list item, or when removing that syntax would join neighboring prose.
use super::{Document, DocumentError, SourcePatch, TextEdit};
use crate::document::{line_endings, paragraph_flow, BlockKind};
use std::collections::BTreeSet;
use std::ops::Range;

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
        let Some(boundary) = projection.source_range(formatted_boundary) else {
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
        let Some(start) = projection.source_insertion_point(edit.range.start, false) else {
            continue;
        };
        let source_lines = &document.state().source_hard_lines;
        let Some(first) = source_lines
            .line_at_offset(start)
            .and_then(|index| source_lines.get(index))
        else {
            continue;
        };
        let Some(following) = source_lines
            .line_at_offset(boundary.end)
            .and_then(|index| source_lines.get(index))
        else {
            continue;
        };
        let band = first.start..following.end;
        let mut local = patches
            .iter()
            .filter(|patch| patch.range.start <= band.end && band.start <= patch.range.end)
            .collect::<Vec<_>>();
        if local
            .iter()
            .any(|patch| patch.range.start < band.start || patch.range.end > band.end)
        {
            continue;
        }
        local.sort_by_key(|patch| (patch.range.start, patch.range.end));
        let delta = local
            .iter()
            .filter(|patch| patch.range.end <= boundary.start)
            .map(|patch| patch.replacement.len() as isize - patch.range.len() as isize)
            .sum::<isize>();
        let candidate_boundary_start = (boundary.start - band.start)
            .checked_add_signed(delta)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let candidate_boundary =
            candidate_boundary_start..candidate_boundary_start + boundary.len();
        let mut bytes = source
            .bytes_in(band.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        for patch in local.into_iter().rev() {
            bytes.splice(
                patch.range.start - band.start..patch.range.end - band.start,
                patch.replacement.iter().copied(),
            );
        }
        let decoded = document.encoding().decode_region(&bytes, 0)?;
        let normalized = line_endings::normalize(&decoded, document.file_format());
        let (flowed, _) = paragraph_flow::markdown(&normalized);
        let still_hard = flowed.units.iter().any(|unit| {
            unit.source.start <= candidate_boundary.start
                && candidate_boundary.end <= unit.source.end
                && &flowed.text[unit.normalized.clone()] == "\n"
        });
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

/// Splitting a structural line leaves its trailing text in a prose paragraph.
/// Its original single source ending must still separate following prose.
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
        let structural = projection.blocks_for_region(&line).iter().any(|block| {
            block.range.start <= line.start
                && line.end <= block.range.end
                && matches!(
                    block.kind,
                    BlockKind::Heading(_) | BlockKind::ListItem { .. }
                )
        });
        let Some(next) = projection.hard_line_range(index + 1) else {
            continue;
        };
        let following_prose = projection.blocks_for_region(&next).iter().any(|block| {
            block.range.start <= next.start
                && next.end <= block.range.end
                && block.kind == BlockKind::Paragraph
                && block.style.0 != "Code Block"
        });
        if !structural || !following_prose {
            continue;
        }
        let source = projection
            .source_range(line.end..line.end + 1)
            .ok_or(DocumentError::AmbiguousProjection)?;
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
        if line_endings::normalize(&decoded, document.file_format()).text == "\n" {
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
            .hard_line_at_offset(range.end.saturating_sub(1))
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
