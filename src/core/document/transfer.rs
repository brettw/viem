//! Source-backed hard-line transfer planning.
//!
//! Transfers use formatted hard-line addresses. Each Markdown view carries
//! the complete mapped source separator, including hidden blank source rows.
//! WYSIWYG additionally carries wrapped prose within its semantic hard line.

use super::line_endings::normalize;
use super::projection::{FormattedDocument, TransferredLineOrigin};
use super::{Document, DocumentError, StyleApplication, StyleId, TextEdit};
use std::ops::Range;

/// Atomic source-backed hard-line operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardLineTransfer {
    Copy,
    Move,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlannedSourcePatch {
    pub(crate) range: Range<usize>,
    pub(crate) replacement: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LineSignature {
    kind: super::BlockKind,
    style: StyleId,
    paragraph: super::BlockProperties,
    character: super::CharacterProperties,
    inline_styles: Vec<(Range<usize>, StyleApplication)>,
}

#[derive(Clone, Debug)]
pub(crate) struct HardLineTransferPlan {
    pub(crate) source_patches: Vec<PlannedSourcePatch>,
    pub(crate) text_edits: Vec<TextEdit>,
    pub(crate) expected_text: String,
    pub(crate) expected_hard_breaks: Vec<usize>,
    pub(crate) origins: Vec<TransferredLineOrigin>,
    pub(super) expected_signatures: Option<Vec<LineSignature>>,
}

#[derive(Clone, Debug)]
pub(super) struct PhysicalHardLine {
    pub(super) content: Range<usize>,
    pub(super) separator: Option<Range<usize>>,
}

pub(crate) fn plan(
    document: &Document,
    operation: HardLineTransfer,
    source_lines: Range<usize>,
    destination: usize,
) -> Result<Option<HardLineTransferPlan>, DocumentError> {
    let snapshot = document.hard_line_snapshot();
    let line_count = snapshot.line_count();
    if source_lines.start >= source_lines.end || source_lines.end > line_count {
        return Err(DocumentError::InvalidHardLineTransferRange {
            start: source_lines.start,
            end: source_lines.end,
            line_count,
        });
    }
    if destination > line_count {
        return Err(DocumentError::InvalidHardLineTransferDestination {
            destination,
            line_count,
        });
    }
    if operation == HardLineTransfer::Move
        && source_lines.start < destination
        && destination < source_lines.end
    {
        return Err(DocumentError::HardLineTransferDestinationInsideSource {
            destination,
            source: source_lines,
        });
    }
    // Vim treats moving immediately before or after the selected line span as
    // a successful no-op. Preserve that distinction from the strict-interior
    // E134 case, including revision and undo history.
    if operation == HardLineTransfer::Move
        && (destination == source_lines.start || destination == source_lines.end)
    {
        return Ok(None);
    }

    let infos = snapshot
        .lines(0..line_count)
        .map_err(|_| DocumentError::HardLineTransferProjectionMismatch)?;
    // Source commands move literal syntax. A copied fence body can acquire a
    // different semantic style at its destination, just as a source edit can.
    let signatures = if document.format() == super::Format::MarkdownSource {
        None
    } else {
        Some(projection_signatures(document.projection(), &infos)?)
    };
    let physical = physical_hard_lines(document, line_count)?;
    let source_bytes = document.source_bytes();
    let mut boundary = document
        .encoding()
        .encode_fragment(document.file_format().spelling())?;
    if document.format() == super::Format::Markdown {
        boundary.extend_from_within(..);
    }

    let first_physical = &physical[source_lines.start];
    let last_physical = &physical[source_lines.end - 1];
    let source_body = first_physical.content.start..last_physical.content.end;
    let source_body_bytes = source_bytes[source_body.clone()].to_vec();
    let trailing_separator = last_physical.separator.clone();

    let first_formatted = &infos[source_lines.start];
    let last_formatted = &infos[source_lines.end - 1];
    let formatted_body = snapshot.text()
        [first_formatted.content_range().start..last_formatted.content_range().end]
        .to_owned();

    let mut source_patches = source_patches(
        operation,
        &physical,
        source_lines.clone(),
        destination,
        &source_bytes,
        source_body,
        source_body_bytes,
        trailing_separator,
        &boundary,
        document,
    )?;
    let mut text_edits = formatted_edits(
        operation,
        &infos,
        source_lines.clone(),
        destination,
        snapshot.text_length(),
        formatted_body,
    )?;
    text_edits.sort_by_key(|edit| (edit.range.start, edit.range.end));

    let origins = transferred_origins(operation, line_count, source_lines, destination);
    let contents = infos
        .iter()
        .map(|info| snapshot.text()[info.content_range()].to_owned())
        .collect::<Vec<_>>();
    let expected_text = origins
        .iter()
        .map(|origin| match origin {
            TransferredLineOrigin::Existing(index) | TransferredLineOrigin::Copied(index) => {
                contents[*index].as_str()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let expected_hard_breaks = hard_break_offsets(&origins, &contents)?;
    let expected_signatures = signatures.map(|signatures| {
        origins
            .iter()
            .map(|origin| match origin {
                TransferredLineOrigin::Existing(index) | TransferredLineOrigin::Copied(index) => {
                    signatures[*index].clone()
                }
            })
            .collect()
    });

    if apply_text_edits(snapshot.text(), &text_edits)? != expected_text {
        return Err(DocumentError::HardLineTransferProjectionMismatch);
    }
    preserve_source_blank_rows(document, &mut source_patches, &expected_text)?;

    Ok(Some(HardLineTransferPlan {
        source_patches,
        text_edits,
        expected_text,
        expected_hard_breaks,
        origins,
        expected_signatures,
    }))
}

/// Joining transferred separator bytes can hide an intended empty Source
/// row. Add only the missing physical endings at those joins; existing bytes
/// and the transferred syntax remain untouched. This is a linear comparison
/// and one normalization, regardless of the number of empty rows copied.
pub(super) fn preserve_source_blank_rows(
    document: &Document,
    patches: &mut Vec<PlannedSourcePatch>,
    expected: &str,
) -> Result<(), DocumentError> {
    if document.format() != super::Format::MarkdownSource {
        return Ok(());
    }
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    let mut bytes = document.source_bytes();
    for patch in patches.iter().rev() {
        bytes.splice(patch.range.clone(), patch.replacement.iter().copied());
    }
    let decoded = document.encoding().decode(&bytes)?;
    let physical = normalize(&decoded, document.file_format());
    let cooked = super::paragraph_flow::markdown_source(&physical).0;
    if cooked.text == expected {
        return Ok(());
    }
    let mut actual_at = 0;
    let mut missing = Vec::<(usize, usize)>::new();
    for ch in expected.chars() {
        if cooked.text[actual_at..].starts_with(ch) {
            actual_at += ch.len_utf8();
        } else if ch == '\n' {
            if let Some((_, count)) = missing.last_mut().filter(|(at, _)| *at == actual_at) {
                *count += 1;
            } else {
                missing.push((actual_at, 1));
            }
        } else {
            // Contextual syntax changes still use ordinary transaction
            // verification; do not guess a source rewrite for them.
            return Ok(());
        }
    }
    if actual_at != cooked.text.len() {
        return Ok(());
    }
    let ending = document
        .encoding()
        .encode_fragment(document.file_format().spelling())?;
    for (at, count) in missing.into_iter().rev() {
        let source_at = cooked
            .units
            .get(
                cooked
                    .units
                    .partition_point(|unit| unit.normalized.end <= at),
            )
            .filter(|unit| unit.normalized.start == at)
            .map_or(bytes.len(), |unit| unit.source.start);
        let mut end = source_at;
        let mut index = physical
            .endings
            .partition_point(|line| line.source.end <= end);
        let mut source_endings = 0usize;
        while index > 0 {
            let previous = &physical.endings[index - 1];
            let gap = document
                .encoding()
                .decode_region(&bytes[previous.source.end..end], previous.source.end)?;
            if !gap.text.trim().is_empty() {
                break;
            }
            source_endings += 1;
            end = previous.source.start;
            index -= 1;
        }
        // One raw terminal ending is visible already; every further visible
        // empty paragraph requires a pair. An odd third ending is trivia.
        let extra = count * 2 + usize::from(source_endings == 1)
            - usize::from(source_endings > 1 && source_endings % 2 == 1);
        let addition = ending.repeat(extra);
        let mut old_cursor = 0;
        let mut new_cursor = 0;
        let mut installed = false;
        for patch in patches.iter_mut() {
            let start = new_cursor + patch.range.start - old_cursor;
            let end = start + patch.replacement.len();
            if source_at < start {
                break;
            }
            if source_at <= end {
                patch.replacement.splice(
                    source_at - start..source_at - start,
                    addition.iter().copied(),
                );
                installed = true;
                break;
            }
            old_cursor = patch.range.end;
            new_cursor = end;
        }
        if !installed {
            let at = old_cursor + source_at - new_cursor;
            patches.push(PlannedSourcePatch {
                range: at..at,
                replacement: addition,
            });
            patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        }
    }
    Ok(())
}

pub(crate) fn verify_projection(
    candidate: &FormattedDocument,
    plan: &HardLineTransferPlan,
) -> Result<(), DocumentError> {
    let Some(expected) = &plan.expected_signatures else {
        return Ok(());
    };
    let snapshot = candidate.hard_line_snapshot(super::DocumentId(0));
    let infos = snapshot
        .lines(0..snapshot.line_count())
        .map_err(|_| DocumentError::HardLineTransferProjectionMismatch)?;
    let actual = projection_signatures(candidate, &infos)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(DocumentError::HardLineTransferProjectionMismatch)
    }
}

pub(super) fn physical_hard_lines(
    document: &Document,
    expected_count: usize,
) -> Result<Vec<PhysicalHardLine>, DocumentError> {
    let source = document.source_bytes();
    let decoded = document.encoding().decode(&source)?;
    let normalized = normalize(&decoded, document.file_format());
    let normalized = match document.format() {
        super::Format::Markdown => super::paragraph_flow::markdown(&normalized).0,
        super::Format::MarkdownSource => super::paragraph_flow::markdown_source(&normalized).0,
        _ => normalized,
    };
    if normalized.endings.len().checked_add(1) != Some(expected_count) {
        return Err(DocumentError::HardLineTransferProjectionMismatch);
    }
    let source_end = decoded
        .source_boundary(decoded.text.len())
        .unwrap_or(decoded.bom_len);
    let mut start = decoded.bom_len;
    let mut result = Vec::with_capacity(expected_count);
    for ending in &normalized.endings {
        if start > ending.source.start || ending.source.end > source_end {
            return Err(DocumentError::HardLineTransferProjectionMismatch);
        }
        result.push(PhysicalHardLine {
            content: start..ending.source.start,
            separator: Some(ending.source.clone()),
        });
        start = ending.source.end;
    }
    if start > source_end {
        return Err(DocumentError::HardLineTransferProjectionMismatch);
    }
    result.push(PhysicalHardLine {
        content: start..source_end,
        separator: None,
    });
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn source_patches(
    operation: HardLineTransfer,
    physical: &[PhysicalHardLine],
    source_lines: Range<usize>,
    destination: usize,
    source: &[u8],
    body: Range<usize>,
    body_bytes: Vec<u8>,
    trailing_separator: Option<Range<usize>>,
    new_boundary: &[u8],
    document: &Document,
) -> Result<Vec<PlannedSourcePatch>, DocumentError> {
    let line_count = physical.len();
    let insertion = if destination == line_count {
        source.len()
    } else {
        physical[destination].content.start
    };
    let boundary = |range: &Range<usize>| -> Result<Vec<u8>, DocumentError> {
        let bytes = &source[range.clone()];
        if document.format() == super::Format::Markdown {
            let decoded = document.encoding().decode_region(bytes, range.start)?;
            if matches!(decoded.text.as_str(), "\n" | "\r" | "\r\n") {
                return Ok(concat(bytes, bytes));
            }
        }
        Ok(bytes.to_vec())
    };

    match operation {
        HardLineTransfer::Copy => {
            let replacement = if destination == line_count {
                let separator = trailing_separator
                    .as_ref()
                    .map(&boundary)
                    .transpose()?
                    .unwrap_or_else(|| new_boundary.to_vec());
                concat(&separator, &body_bytes)
            } else if let Some(separator) = trailing_separator {
                concat(&body_bytes, &boundary(&separator)?)
            } else {
                concat(&body_bytes, new_boundary)
            };
            Ok(vec![PlannedSourcePatch {
                range: insertion..insertion,
                replacement,
            }])
        }
        HardLineTransfer::Move if destination < source_lines.start => {
            if let Some(separator) = trailing_separator {
                let transfer = concat(&body_bytes, &boundary(&separator)?);
                Ok(vec![
                    PlannedSourcePatch {
                        range: insertion..insertion,
                        replacement: transfer,
                    },
                    PlannedSourcePatch {
                        range: body.start..separator.end,
                        replacement: Vec::new(),
                    },
                ])
            } else {
                let previous = source_lines
                    .start
                    .checked_sub(1)
                    .and_then(|index| physical[index].separator.clone())
                    .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
                let transfer = concat(&body_bytes, &boundary(&previous)?);
                Ok(vec![
                    PlannedSourcePatch {
                        range: insertion..insertion,
                        replacement: transfer,
                    },
                    PlannedSourcePatch {
                        range: previous.start..body.end,
                        replacement: Vec::new(),
                    },
                ])
            }
        }
        HardLineTransfer::Move => {
            let separator =
                trailing_separator.ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
            let transfer = if destination == line_count {
                concat(&boundary(&separator)?, &body_bytes)
            } else {
                concat(&body_bytes, &boundary(&separator)?)
            };
            Ok(vec![
                PlannedSourcePatch {
                    range: body.start..separator.end,
                    replacement: Vec::new(),
                },
                PlannedSourcePatch {
                    range: insertion..insertion,
                    replacement: transfer,
                },
            ])
        }
    }
}

fn formatted_edits(
    operation: HardLineTransfer,
    infos: &[super::HardLineInfo],
    source_lines: Range<usize>,
    destination: usize,
    text_length: usize,
    body: String,
) -> Result<Vec<TextEdit>, DocumentError> {
    let line_count = infos.len();
    let insertion = if destination == line_count {
        text_length
    } else {
        infos[destination].content_range().start
    };
    let with_trailing_boundary = || format!("{body}\n");
    let with_leading_boundary = || format!("\n{body}");

    match operation {
        HardLineTransfer::Copy => Ok(vec![TextEdit::new(
            insertion..insertion,
            if destination == line_count {
                with_leading_boundary()
            } else {
                with_trailing_boundary()
            },
        )]),
        HardLineTransfer::Move if destination < source_lines.start => {
            let deletion = if source_lines.end < line_count {
                infos[source_lines.start].content_range().start
                    ..infos[source_lines.end - 1].linewise_range().end
            } else {
                let previous_separator = source_lines
                    .start
                    .checked_sub(1)
                    .and_then(|index| infos[index].separator_range())
                    .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
                previous_separator.start..text_length
            };
            Ok(vec![
                TextEdit::new(insertion..insertion, with_trailing_boundary()),
                TextEdit::new(deletion, ""),
            ])
        }
        HardLineTransfer::Move => {
            let deletion = infos[source_lines.start].content_range().start
                ..infos[source_lines.end - 1].linewise_range().end;
            Ok(vec![
                TextEdit::new(deletion, ""),
                TextEdit::new(
                    insertion..insertion,
                    if destination == line_count {
                        with_leading_boundary()
                    } else {
                        with_trailing_boundary()
                    },
                ),
            ])
        }
    }
}

fn transferred_origins(
    operation: HardLineTransfer,
    line_count: usize,
    source: Range<usize>,
    destination: usize,
) -> Vec<TransferredLineOrigin> {
    let mut origins = (0..line_count)
        .map(TransferredLineOrigin::Existing)
        .collect::<Vec<_>>();
    let transfer = (source.clone())
        .map(|index| match operation {
            HardLineTransfer::Copy => TransferredLineOrigin::Copied(index),
            HardLineTransfer::Move => TransferredLineOrigin::Existing(index),
        })
        .collect::<Vec<_>>();
    let insertion = match operation {
        HardLineTransfer::Copy => destination,
        HardLineTransfer::Move => {
            origins.drain(source.clone());
            if destination > source.end {
                destination - source.len()
            } else {
                destination
            }
        }
    };
    origins.splice(insertion..insertion, transfer);
    origins
}

pub(super) fn hard_break_offsets(
    origins: &[TransferredLineOrigin],
    contents: &[String],
) -> Result<Vec<usize>, DocumentError> {
    let mut offsets = Vec::with_capacity(origins.len().saturating_sub(1));
    let mut cursor = 0usize;
    for (position, origin) in origins.iter().enumerate() {
        let index = match origin {
            TransferredLineOrigin::Existing(index) | TransferredLineOrigin::Copied(index) => *index,
        };
        cursor = cursor
            .checked_add(contents[index].len())
            .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
        if position + 1 < origins.len() {
            offsets.push(cursor);
            cursor = cursor
                .checked_add(1)
                .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
        }
    }
    Ok(offsets)
}

pub(super) fn projection_signatures(
    projection: &FormattedDocument,
    infos: &[super::HardLineInfo],
) -> Result<Vec<LineSignature>, DocumentError> {
    infos
        .iter()
        .map(|line| {
            let content = line.content_range();
            let block = projection
                .blocks_for_region(&content)
                .into_iter()
                .find(|block| block.range.start <= content.start && content.end <= block.range.end)
                .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
            let mut inline_styles = Vec::new();
            for span in projection.style_spans_for_region(&content) {
                if span.range.end <= content.start || content.end <= span.range.start {
                    continue;
                }
                inline_styles.push((
                    span.range.start.max(content.start) - content.start
                        ..span.range.end.min(content.end) - content.start,
                    span.application.clone(),
                ));
            }
            Ok(LineSignature {
                kind: block.kind.clone(),
                style: block.style.clone(),
                paragraph: block.direct_paragraph.clone(),
                character: block.direct_default_character.clone(),
                inline_styles,
            })
        })
        .collect()
}

fn apply_text_edits(text: &str, edits: &[TextEdit]) -> Result<String, DocumentError> {
    let mut result = text.to_owned();
    for edit in edits.iter().rev() {
        if edit.range.start > edit.range.end || edit.range.end > result.len() {
            return Err(DocumentError::HardLineTransferProjectionMismatch);
        }
        result.replace_range(edit.range.clone(), &edit.replacement);
    }
    Ok(result)
}

fn concat(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(first.len().saturating_add(second.len()));
    result.extend_from_slice(first);
    result.extend_from_slice(second);
    result
}
