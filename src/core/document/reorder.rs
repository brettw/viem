//! Verified source-backed row permutations. Sorting never serializes flattened
//! content: untouched syntax and rich declarations travel with their row.
use super::projection::TransferredLineOrigin;
use super::transfer::{self, HardLineTransferPlan, PlannedSourcePatch};
use super::{Document, DocumentError, Format, TextEdit};
use std::{collections::HashSet, ops::Range};

pub(super) fn plan(
    document: &Document,
    selected: Range<usize>,
    order: Vec<usize>,
) -> Result<Option<HardLineTransferPlan>, DocumentError> {
    let snapshot = document.hard_line_snapshot();
    let count = snapshot.line_count();
    let mut seen = HashSet::new();
    if selected.is_empty()
        || selected.end > count
        || order.is_empty()
        || order
            .iter()
            .any(|index| !selected.contains(index) || !seen.insert(*index))
    {
        return Err(DocumentError::InvalidHardLineTransferRange {
            start: selected.start,
            end: selected.end,
            line_count: count,
        });
    }
    if order.iter().copied().eq(selected.clone()) {
        return Ok(None);
    }
    if document.format() == Format::Markdown {
        for index in selected.clone() {
            let line = document.projection().hard_line_range(index)
                .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
            let blocks = document.projection().blocks_for_region(&line);
            let block = blocks.iter().find(|block| block.range.start <= line.start && line.end <= block.range.end)
                .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
            let source_line = document.physical_line_at_text(line.start)?;
            let indented = source_line.text.starts_with('\t') || source_line.text.starts_with("    ");
            // This operation moves independent source paragraph owners. A
            // list marker depends on its surrounding container, and a code
            // body depends on indentation/fences outside the visible row.
            // Moving those raw bodies as independent paragraphs is not a
            // verified structure-preserving permutation capability.
            if block.range != line || block.style.0 == "Code Block" || indented
                || matches!(block.kind, super::BlockKind::ListItem { .. })
            {
                return Err(DocumentError::UnsupportedFormatting);
            }
        }
    }
    let infos = snapshot
        .lines(0..count)
        .map_err(|_| DocumentError::HardLineTransferProjectionMismatch)?;
    let source_mode = matches!(
        document.format(),
        Format::MarkdownSource | Format::HtmlSource
    );
    let signatures = if source_mode {
        None
    } else {
        Some(transfer::projection_signatures(
            document.projection(),
            &infos,
        )?)
    };
    let source = document.source_bytes();
    let mut source_patches = match document.format() {
        Format::PlainText | Format::MarkdownSource | Format::HtmlSource => {
            let physical = transfer::physical_hard_lines(document, count)?;
            let mut replacement = Vec::new();
            for (slot, index) in order.iter().enumerate() {
                replacement.extend_from_slice(&source[physical[*index].content.clone()]);
                if slot + 1 < order.len() {
                    let separator = physical[selected.start + slot]
                        .separator
                        .clone()
                        .ok_or(DocumentError::HardLineTransferProjectionMismatch)?;
                    replacement.extend_from_slice(&source[separator]);
                }
            }
            vec![PlannedSourcePatch {
                range: physical[selected.start].content.start
                    ..physical[selected.end - 1].content.end,
                replacement,
            }]
        }
        Format::Html => rich_patches(
            document,
            selected.clone(),
            &order,
            &source,
            html_rows(document, selected.clone())?,
        )?,
        Format::Rtf => return Err(DocumentError::UnsupportedFormatting),
        Format::Markdown => rich_patches(
            document,
            selected.clone(),
            &order,
            &source,
            markdown_rows(document)?,
        )?,
    };
    let mut indices: Vec<_> = (0..selected.start).collect();
    indices.extend(order.iter().copied());
    indices.extend(selected.end..count);
    let origins = indices
        .iter()
        .map(|index| TransferredLineOrigin::Existing(*index))
        .collect::<Vec<_>>();
    let contents = infos
        .iter()
        .map(|info| snapshot.text()[info.content_range()].to_owned())
        .collect::<Vec<_>>();
    let expected_text = indices
        .iter()
        .map(|index| contents[*index].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let expected_hard_breaks = transfer::hard_break_offsets(&origins, &contents)?;
    transfer::preserve_source_blank_rows(document, &mut source_patches, &expected_text)?;
    let expected_signatures = signatures.map(|signatures| {
        indices
            .iter()
            .map(|index| signatures[*index].clone())
            .collect()
    });
    let text_edits = vec![TextEdit::new(
        infos[selected.start].content_range().start..infos[selected.end - 1].content_range().end,
        order
            .iter()
            .map(|index| contents[*index].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )];
    Ok(Some(HardLineTransferPlan {
        source_patches,
        text_edits,
        expected_text,
        expected_hard_breaks,
        origins,
        expected_signatures,
    }))
}

fn rich_patches(
    document: &Document,
    selected: Range<usize>,
    order: &[usize],
    source: &[u8],
    rows: Vec<Range<usize>>,
) -> Result<Vec<PlannedSourcePatch>, DocumentError> {
    if rows.len() != document.line_count()
        || rows[selected.clone()]
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start)
    {
        return Err(DocumentError::AmbiguousProjection);
    }
    let mut replacement = Vec::new();
    // Trivia between rows stays in its original slot, including comments when
    // unique removes a duplicate. Candidate verification catches visible gaps.
    for (slot, old) in selected.clone().enumerate() {
        if let Some(index) = order.get(slot) {
            replacement.extend_from_slice(&source[rows[*index].clone()]);
        }
        if old + 1 < selected.end {
            replacement.extend_from_slice(&source[rows[old].end..rows[old + 1].start]);
        }
    }
    Ok(vec![PlannedSourcePatch {
        range: rows[selected.start].start..rows[selected.end - 1].end,
        replacement,
    }])
}

fn normalized(document: &Document) -> Result<super::line_endings::NormalizedText, DocumentError> {
    let decoded = document.encoding().decode(&document.source_bytes())?;
    Ok(super::line_endings::normalize(
        &decoded,
        document.file_format(),
    ))
}

fn html_rows(
    document: &Document,
    selected: Range<usize>,
) -> Result<Vec<Range<usize>>, DocumentError> {
    use super::html::{self, TokenKind};
    let input = normalized(document)?;
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(&input, document.revision());
    let mut stack: Vec<usize> = Vec::new();
    let mut owners = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if tag.end {
            let Some(open_at) = stack.iter().rposition(|open| matches!(&tokens[*open].kind, TokenKind::Tag(open_tag) if open_tag.name == tag.name)) else { continue; };
            let opening = stack[open_at];
            if matches!(
                tag.name.as_str(),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                // Misnested recovered phrasing and nested paragraph owners are
                // not guessed. The HTML5 projection remains the authority.
                if open_at + 1 != stack.len() {
                    return Err(DocumentError::AmbiguousProjection);
                }
                owners.push((
                    converter.source_range(tokens[opening].range.start..token.range.end),
                    stack[..open_at].to_vec(),
                ));
            }
            stack.truncate(open_at);
        } else if !matches!(
            tag.name.as_str(),
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        ) {
            stack.push(index);
        }
    }
    owners.sort_by_key(|(range, _)| range.start);
    map_owners(document, owners, selected)
}

fn map_owners(
    document: &Document,
    owners: Vec<(Range<usize>, Vec<usize>)>,
    selected: Range<usize>,
) -> Result<Vec<Range<usize>>, DocumentError> {
    let mut rows = Vec::new();
    let mut parent = None;
    for (index, block) in document.projection().blocks().iter().enumerate() {
        if !selected.contains(&index) {
            rows.push(0..0);
            continue;
        }
        let provenance = document.projection().provenance_for_region(&block.range);
        let source_ranges: Vec<_> = provenance
            .iter()
            .filter(|span| !span.source.is_empty())
            .map(|span| span.source.clone())
            .collect();
        let at = source_ranges
            .first()
            .map(|range| range.start)
            .or_else(|| {
                document
                    .projection()
                    .source_insertion_point(block.range.start, true)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        let owner_index = owners
            .partition_point(|(owner, _)| owner.start <= at)
            .checked_sub(1)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let (owner, ancestry) = &owners[owner_index];
        if at > owner.end
            || source_ranges
                .iter()
                .any(|range| range.start < owner.start || range.end > owner.end)
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        if owner_index > 0 && owners[owner_index - 1].0.end >= at {
            return Err(DocumentError::AmbiguousProjection);
        }
        if let Some(expected) = &parent {
            if expected != ancestry {
                return Err(DocumentError::AmbiguousProjection);
            }
        } else {
            parent = Some(ancestry.clone());
        }
        rows.push(owner.clone());
    }
    Ok(rows)
}

fn markdown_rows(document: &Document) -> Result<Vec<Range<usize>>, DocumentError> {
    // A blank-line-separated Markdown paragraph owns all its physical source
    // lines. Delimiter-only syntax, contextual lists/fences and hard breaks are
    // accepted only if the full semantic signature reprojects unchanged.
    let mut rows = Vec::new();
    for block in document.projection().blocks() {
        let provenance = document.projection().provenance_for_region(&block.range);
        let start = provenance
            .iter()
            .filter(|span| !span.source.is_empty())
            .map(|span| span.source.start)
            .min()
            .or_else(|| {
                document
                    .projection()
                    .source_insertion_point(block.range.start, true)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        let end = provenance
            .iter()
            .filter(|span| !span.source.is_empty())
            .map(|span| span.source.end)
            .max()
            .unwrap_or(start);
        let first = document.physical_line_at_source(start)?;
        let last = document.physical_line_at_source(end.saturating_sub(1).max(start))?;
        rows.push(first.content_range.start..last.content_range.end);
    }
    Ok(rows)
}
