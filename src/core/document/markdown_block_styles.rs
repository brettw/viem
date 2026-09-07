//! Supporting syntax edits when a flowed Markdown paragraph becomes a heading
//! or list item, or when removing that syntax would join neighboring prose.
use super::{Document, DocumentError, SourcePatch};
use crate::document::{line_endings, paragraph_flow, BlockKind};
use std::collections::BTreeSet;
use std::ops::Range;

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
            if !resulting_prose(index) || !resulting_prose(index + 1) {
                continue;
            }
            let end = projection.hard_line_range(index).unwrap().end;
            let source = projection
                .source_range(end..end + 1)
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
