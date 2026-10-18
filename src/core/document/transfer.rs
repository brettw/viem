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
    quote_depth: usize,
    code_language: Option<String>,
    front_matter: bool,
    thematic_break: bool,
    containers: Vec<(super::ContainerKind, StyleId, Option<std::sync::Arc<super::BlockDirectFormatting>>)>,
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
    // Code has no source-backed styling: asynchronous syntax must not enter
    // transaction verification. Its exact text and hard breaks are verified.
    let signatures = if document.format().is_code() {
        None
    } else {
        Some(projection_signatures(document.projection(), &infos)?)
    };
    let physical: std::collections::BTreeMap<usize, PhysicalHardLine> = if document.format() == super::Format::Markdown {
        // Only transferred rows and their separator/destination boundaries
        // need reverse source mappings. An unrelated table or image cannot
        // make an otherwise local paragraph transfer unavailable.
        let mut required = (source_lines.start.saturating_sub(1)..source_lines.end.saturating_add(1).min(line_count))
            .collect::<std::collections::BTreeSet<_>>();
        if destination < line_count { required.insert(destination); }
        projected_markdown_rows(document, &required)?
    } else {
        physical_hard_lines(document, line_count)?.into_iter().enumerate().collect()
    };
    let source_bytes = document.source_bytes();
    let origin = infos[source_lines.start].content_range().start;
    let destination_at = if destination < line_count { infos[destination].content_range().start }
        else { snapshot.text_length() };
    let literal_boundary = document.format().is_markdown()
        && document.projection().blocks_for_region(&(origin..origin)).iter().any(|block|
            block.style.0 == "Code Block" && block.range.start <= origin && origin <= block.range.end
                && block.range.start <= destination_at && destination_at <= block.range.end);
    let mut boundary = document
        .encoding()
        .encode_fragment(document.file_format().spelling())?;
    if document.format() == super::Format::Markdown && !literal_boundary {
        boundary.extend_from_within(..);
    }

    let first_physical = &physical[&source_lines.start];
    let last_physical = &physical[&(source_lines.end - 1)];
    let source_body = first_physical.content.start..last_physical.content.end;
    let mut source_body_bytes = source_bytes[source_body.clone()].to_vec();
    let promoted_continuation = document.format() == super::Format::Markdown
        && promote_detached_list_continuation(
            document, &infos, source_lines.start, destination, source_body.start,
            &mut source_body_bytes,
        )?;
    let trailing_separator = last_physical.separator.clone();

    let first_formatted = &infos[source_lines.start];
    let last_formatted = &infos[source_lines.end - 1];
    let formatted_body = snapshot.text()
        [first_formatted.content_range().start..last_formatted.content_range().end]
        .to_owned();

    let mut source_patches = source_patches(
        operation,
        line_count,
        if destination == line_count { document.source_byte_len() } else { physical[&destination].content.start },
        source_lines.start.checked_sub(1).and_then(|index| physical.get(&index)).and_then(|line| line.separator.clone()),
        source_lines.clone(),
        destination,
        &source_bytes,
        source_body,
        source_body_bytes,
        trailing_separator,
        &boundary,
        document,
        literal_boundary,
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

    let promoted_line = promoted_continuation.then_some(source_lines.start);
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
    let expected_signatures: Option<Vec<LineSignature>> = signatures.map(|signatures| {
        origins
            .iter()
            .map(|origin| {
                let (index, transferred) = match origin {
                    TransferredLineOrigin::Existing(index) => (*index, operation == HardLineTransfer::Move),
                    TransferredLineOrigin::Copied(index) => (*index, true),
                };
                let mut signature = signatures[index].clone();
                if transferred && promoted_line == Some(index) {
                    if let super::BlockKind::ListItem { item_start, .. } = &mut signature.kind {
                        *item_start = true;
                    }
                }
                signature
            })
            .collect()
    });

    if apply_text_edits(snapshot.text(), &text_edits)? != expected_text {
        return Err(DocumentError::HardLineTransferProjectionMismatch);
    }
    preserve_leading_thematic_rule(document, &mut source_patches, expected_signatures.as_deref())?;
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

/// A transferred or newly exposed leading `---` rule can pair with another
/// rule and become YAML metadata. Change only that first marker's spelling,
/// and only when the tentative source would acquire that different owner.
fn preserve_leading_thematic_rule(
    document: &Document,
    patches: &mut Vec<PlannedSourcePatch>,
    signatures: Option<&[LineSignature]>,
) -> Result<(), DocumentError> {
    if !document.format().is_markdown()
        || signatures.and_then(|items| items.first()).is_none_or(|line| !line.thematic_break)
    {
        return Ok(());
    }
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    let mut candidate = document.source_bytes();
    for patch in patches.iter().rev() {
        candidate.splice(patch.range.clone(), patch.replacement.iter().copied());
    }
    let decoded = document.encoding().decode(&candidate)?;
    let normalized = normalize(&decoded, document.file_format());
    if super::markdown_front_matter::recognize(&super::markdown_syntax::grammar_text(&normalized)).is_none() {
        return Ok(());
    }
    let replacement = document.encoding().encode_fragment("***")?;
    let marker = decoded.bom_len..decoded.bom_len + replacement.len();
    let mut old_cursor = 0;
    let mut new_cursor = 0;
    for patch in patches.iter_mut() {
        let start = new_cursor + patch.range.start - old_cursor;
        let end = start + patch.replacement.len();
        if marker.end <= start { break; }
        if start <= marker.start && marker.end <= end {
            patch.replacement.splice(marker.start - start..marker.end - start, replacement);
            return Ok(());
        }
        if marker.start < end && start < marker.end {
            // Transfers own complete physical rows: a marker split across
            // candidate contributors cannot be safely rewritten here.
            return Err(DocumentError::HardLineTransferProjectionMismatch);
        }
        old_cursor = patch.range.end;
        new_cursor = end;
    }
    let start = old_cursor + marker.start - new_cursor;
    patches.push(PlannedSourcePatch { range: start..start + replacement.len(), replacement });
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    Ok(())
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
    if !document.format().is_markdown()
        || !expected.starts_with('\n') && !expected.ends_with('\n') && !expected.contains("\n\n")
    {
        return Ok(());
    }
    patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
    let mut bytes = document.source_bytes();
    for patch in patches.iter().rev() {
        bytes.splice(patch.range.clone(), patch.replacement.iter().copied());
    }
    let decoded = document.encoding().decode(&bytes)?;
    let physical = normalize(&decoded, document.file_format());
    let visible = Document::from_bytes_with_file_format(
        bytes.clone(), document.encoding(), document.format(), document.file_format(),
    )?;
    let actual = visible.text();
    if actual == expected {
        return Ok(());
    }
    let mut actual_at = 0;
    let mut missing = Vec::<(usize, usize)>::new();
    for ch in expected.chars() {
        if actual[actual_at..].starts_with(ch) {
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
    if actual_at != actual.len() {
        return Ok(());
    }
    let ending = document
        .encoding()
        .encode_fragment(document.file_format().spelling())?;
    for (at, count) in missing.into_iter().rev() {
        let source_at = visible.projection().source_insertion_point(at, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
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
    if document.format() == super::Format::Markdown {
        return projected_markdown_hard_lines(document);
    }
    let source = document.source_bytes();
    let decoded = document.encoding().decode(&source)?;
    let normalized = normalize(&decoded, document.file_format());
    let normalized = match document.format() {
        super::Format::Markdown => super::paragraph_flow::markdown(&normalized).0,
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

/// Hidden fence lines are not formatted hard lines. Use each visible row's
/// source owner when the pre-Markdown newline partition cannot describe it.
/// Full code owners include their fences; unrelated code never prevents a
/// prose transfer. The ordinary candidate verifier still checks the complete
/// text, break partition, and source-backed paragraph/inline appearance.
fn projected_markdown_hard_lines(document: &Document) -> Result<Vec<PhysicalHardLine>, DocumentError> {
    let required = (0..document.hard_line_snapshot().line_count()).collect();
    Ok(projected_markdown_rows(document, &required)?.into_values().collect())
}

/// A continuation has no marker of its own. When its new predecessor is
/// outside its item, carry the leaf list treatment with the smallest explicit
/// marker instead of letting its indentation turn into an ordinary paragraph.
fn promote_detached_list_continuation(
    document: &Document,
    infos: &[super::HardLineInfo],
    first: usize,
    destination: usize,
    source_start: usize,
    body: &mut Vec<u8>,
) -> Result<bool, DocumentError> {
    let projection = document.projection();
    let range = infos[first].content_range();
    let block = projection.blocks_for_region(&range).into_iter()
        .find(|block| block.range.start <= range.start && range.end <= block.range.end)
        .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
    if !matches!(block.kind, super::BlockKind::ListItem { item_start: false, .. }) {
        return Ok(false);
    }
    let item = block.containers.iter().rev()
        .find(|member| member.container.kind == super::ContainerKind::ListItem)
        .ok_or(DocumentError::HardLineTransferProjectionMismatch)?.container.id;
    if let Some(previous) = destination.checked_sub(1) {
        let previous = infos[previous].content_range();
        if projection.blocks_for_region(&previous).iter().any(|block|
            block.range.start <= previous.start && previous.end <= block.range.end
                && block.containers.iter().any(|member| member.container.id == item)) {
            return Ok(false);
        }
    }
    let owner = projection.blocks().iter().find(|block|
        block.containers.iter().any(|member| member.container.id == item && member.starts_here))
        .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
    let marker_line = document.physical_line_at_text(owner.range.start)?;
    let quote = super::markdown_quotes::prefix(&marker_line.text);
    let width = super::markdown_blocks::marker_prefix_length(&marker_line.text[quote..])
        .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
    let marker = marker_line.text[quote..quote + width].trim_start_matches([' ', '\t']);
    let decoded = document.encoding().decode_region(body, source_start)?;
    let quote = super::markdown_quotes::prefix(&decoded.text);
    let indent_end = quote + decoded.text[quote..].len()
        - decoded.text[quote..].trim_start_matches([' ', '\t']).len();
    let start = document.encoding().encode_fragment(&decoded.text[..quote])?.len();
    let end = document.encoding().encode_fragment(&decoded.text[..indent_end])?.len();
    body.splice(start..end, document.encoding().encode_fragment(marker)?);
    Ok(true)
}

fn projected_markdown_rows(document: &Document, required: &std::collections::BTreeSet<usize>)
    -> Result<std::collections::BTreeMap<usize, PhysicalHardLine>, DocumentError> {
    let projection = document.projection();
    let snapshot = document.hard_line_snapshot();
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let normalized = normalize(&decoded, document.file_format());
    let syntax = super::markdown_syntax::Blocks::parse(
        &super::markdown_syntax::grammar_text(&normalized)).to_source(&normalized);
    let mut result = std::collections::BTreeMap::<usize, PhysicalHardLine>::new();
    for &index in required {
        let info = snapshot.line(index).ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
        let range = info.content_range();
        let mut body = projection.source_range(range.clone())
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if let Ok(Some((source, bytes, _))) = document.wrapped_inline_source(&range, "", "") {
            if document.state().source.bytes_in(source.clone()).as_deref() == Some(bytes.as_slice()) {
                body = source;
            }
        }
        let first = document.physical_line_at_source(body.start)?;
        let last = document.physical_line_at_source(body.end.saturating_sub(1).max(body.start))?;
        let owner = projection.blocks_for_region(&range).into_iter()
            .find(|block| block.range.start <= range.start && range.end <= block.range.end)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let mut content = if range.start == owner.range.start { first.content_range.start } else { body.start }
            ..if range.end == owner.range.end { last.content_range.end } else { body.end };
        if owner.quote_depth > 0 && first.content_range.start < content.start
            && content.start - first.content_range.start <= 4096 {
            let prefix = document.state().source.bytes_in(first.content_range.start..content.start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = document.encoding().decode_region(&prefix, first.content_range.start)?.text;
            if prefix.contains('>') && super::markdown_quotes::prefix(&prefix) == prefix.len() {
                // Each physical continuation row owns its quote prefix. It
                // must travel with the row, not with its preceding separator.
                content.start = first.content_range.start;
            }
        }
        if matches!(owner.kind, super::BlockKind::Heading(_)) && range.end == owner.range.end {
            if let Some(heading) = syntax.blocks.iter().find(|heading|
                matches!(heading.role, super::markdown_syntax::BlockRole::Heading(_))
                    && heading.content.start <= body.start && body.end <= heading.content.end) {
                let final_row = document.physical_line_at_source(heading.range.end.saturating_sub(1))?;
                content.end = content.end.max(final_row.content_range.end);
            }
        }
        if let Some(block) = projection.blocks_for_region(&range).into_iter()
            .find(|block| block.range.start <= range.start && range.end <= block.range.end
                && block.style.0 == "Code Block") {
            if let Some(fence) = super::markdown_code::fenced_source(document, &block)? {
                if range.start == block.range.start { content.start = fence.opening.start; }
                if range.end == block.range.end {
                    content.end = fence.closing_content_end.unwrap_or(content.end);
                }
            }
        }
        if let Some(previous) = index.checked_sub(1).and_then(|index| result.get_mut(&index)) {
            if previous.content.end > content.start {
                // Two logical rows in one physical row (for example <br>)
                // need semantic splitting, never overlapping source patches.
                return Err(DocumentError::UnsupportedFormatting);
            }
            previous.separator = Some(previous.content.end..content.start);
        }
        result.insert(index, PhysicalHardLine { content, separator: None });
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn source_patches(
    operation: HardLineTransfer,
    line_count: usize,
    insertion: usize,
    previous_separator: Option<Range<usize>>,
    source_lines: Range<usize>,
    destination: usize,
    source: &[u8],
    body: Range<usize>,
    body_bytes: Vec<u8>,
    trailing_separator: Option<Range<usize>>,
    new_boundary: &[u8],
    document: &Document,
    literal_boundary: bool,
) -> Result<Vec<PlannedSourcePatch>, DocumentError> {
    let boundary = |range: &Range<usize>| -> Result<Vec<u8>, DocumentError> {
        let bytes = &source[range.clone()];
        if document.format() == super::Format::Markdown && !literal_boundary {
            let decoded = document.encoding().decode_region(bytes, range.start)?;
            let normalized = normalize(&decoded, document.file_format());
            if normalized.text == "\\\n"
                || normalized.text.ends_with('\n')
                    && normalized.text[..normalized.text.len() - 1].len() >= 2
                    && normalized.text[..normalized.text.len() - 1].bytes().all(|byte| byte == b' ') {
                // An inline hard break belongs to its old paragraph. At a
                // transfer boundary it would make the destination absorb the
                // copied row into a different quote/list owner.
                return Ok(new_boundary.to_vec());
            }
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
                let previous = previous_separator
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
            let style = match block.kind {
                super::BlockKind::ListItem { ordered, level, .. }
                    if block.style.0 == format!("{}{}", if ordered { "NumberedList" } else { "BulletedList" }, u16::from(level).min(3) + 1) =>
                    StyleId::from(if ordered { "NumberedList1" } else { "BulletedList1" }),
                _ => block.style.clone(),
            };
            Ok(LineSignature {
                // List numbering, nesting and generated ancestor containers
                // follow the destination. The explicit leaf marker, custom
                // declarations, paragraph treatment and inline styles travel
                // with the row and remain part of exact verification.
                kind: match block.kind.clone() {
                    super::BlockKind::ListItem { ordered, item_start, marker_is_decoration, .. } =>
                        super::BlockKind::ListItem { ordered, level: 0, item_start, marker_is_decoration,
                            ordinal: 0, container_start: false },
                    kind => kind,
                },
                quote_depth: block.quote_depth,
                code_language: block.code_language.clone(),
                front_matter: block.front_matter,
                thematic_break: block.thematic_break,
                containers: block.containers.iter().filter(|member| {
                    let container = &member.container;
                    !matches!(container.kind, super::ContainerKind::List { .. } | super::ContainerKind::ListItem)
                        || container.style != container.kind.default_style()
                        || container.direct_formatting.is_some()
                }).map(|member| (
                    member.container.kind, member.container.style.clone(), member.container.direct_formatting.clone(),
                )).collect(),
                style,
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
