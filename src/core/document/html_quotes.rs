//! Split quote wrappers around selected paragraphs without rewriting bodies.
use super::html::{self, Token, TokenKind};
use super::line_endings::NormalizedText;
use super::{DocumentError, Revision};
use std::collections::BTreeMap;
use std::ops::Range;

pub(super) fn in_native_pre(document: &super::Document, at: usize) -> Result<bool, DocumentError> {
    let source_at = document
        .projection()
        .provenance_touching(&(at..at))
        .iter()
        .find(|span| span.formatted == (at..at))
        .map(|span| span.source.start)
        .or_else(|| {
            document
                .projection()
                .source_insertion_point(at, at < document.text().len())
        })
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let offset = input
        .units
        .get(
            input
                .units
                .partition_point(|unit| unit.source.end <= source_at),
        )
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let tokens = html::tokenize(&input.text);
    Ok(super::html_paragraph::stack_at(&tokens, offset)
        .iter()
        .any(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "pre")))
}

fn closing(tokens: &[Token], opening: &Token, end: usize) -> Range<usize> {
    let TokenKind::Tag(tag) = &opening.kind else {
        unreachable!()
    };
    let mut depth = 1;
    for token in tokens
        .iter()
        .filter(|token| token.range.start >= opening.range.end)
    {
        if let TokenKind::Tag(next) = &token.kind {
            if depth == 1
                && tag.name != "blockquote"
                && super::html_paragraph::structural(&next.name)
                && !(next.end && next.name == tag.name)
            {
                return token.range.start..token.range.start;
            }
            if next.name == tag.name {
                if next.end {
                    depth -= 1;
                } else {
                    depth += 1;
                }
                if depth == 0 {
                    return token.range.clone();
                }
            }
            if depth == 1
                && tag.name != "blockquote"
                && super::html_paragraph::structural(&next.name)
            {
                return token.range.start..token.range.start;
            }
        }
    }
    end..end
}

/// Add quote containers around native paragraphs while retaining their inner
/// heading, code, and list syntax. A bare list paragraph is wrapped inside li.
pub(super) fn wrap_patches(
    input: &NormalizedText,
    ranges: &[Range<usize>],
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let normalized_at = |source| {
        input
            .units
            .get(
                input
                    .units
                    .partition_point(|unit| unit.source.end <= source),
            )
            .map_or(input.text.len(), |unit| unit.normalized.start)
    };
    let mut extents = Vec::new();
    for source in ranges {
        let at = normalized_at(source.start);
        let stack = super::html_paragraph::stack_at(&tokens, at);
        if stack
            .iter()
            .any(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "blockquote"))
        {
            continue;
        }
        let paragraph = stack.iter().rev().find(|token| {
            matches!(&token.kind, TokenKind::Tag(tag)
            if matches!(tag.name.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre"))
        });
        let extent = if let Some(paragraph) = paragraph {
            paragraph.range.start..closing(&tokens, paragraph, input.text.len()).end
        } else {
            let mut extent = at..normalized_at(source.end);
            let first_inline = stack.iter().rev().take_while(|token| {
                matches!(&token.kind, TokenKind::Tag(tag) if !super::html_paragraph::structural(&tag.name))
            }).last();
            if let Some(first) = first_inline {
                extent.start = first.range.start;
            }
            for token in tokens
                .iter()
                .filter(|token| token.range.start >= normalized_at(source.end))
            {
                match &token.kind {
                    TokenKind::Tag(tag)
                        if tag.end && !super::html_paragraph::structural(&tag.name) =>
                    {
                        extent.end = token.range.end;
                    }
                    TokenKind::Opaque => continue,
                    _ => break,
                }
            }
            extent
        };
        extents.push(extent);
    }
    extents.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for extent in extents {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| extent.start <= previous.end)
        {
            previous.end = previous.end.max(extent.end);
        } else {
            merged.push(extent);
        }
    }
    let mut patches = Vec::new();
    for extent in merged {
        if extent.is_empty() {
            patches.push((
                converter.source_range(extent),
                "<blockquote></blockquote>".into(),
            ));
        } else {
            patches.push((
                converter.source_range(extent.start..extent.start),
                "<blockquote>".into(),
            ));
            patches.push((
                converter.source_range(extent.end..extent.end),
                "</blockquote>".into(),
            ));
        }
    }
    Ok(patches)
}

/// Return None when the selection has no quotation container. The ordinary
/// paragraph-style path then handles the request as before.
pub(super) fn remove_patches(
    input: &NormalizedText,
    ranges: &[Range<usize>],
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let normalized_at = |source: usize| {
        input
            .units
            .get(
                input
                    .units
                    .partition_point(|unit| unit.source.end <= source),
            )
            .map_or(input.text.len(), |unit| unit.normalized.start)
    };
    let mut groups: BTreeMap<usize, (Range<usize>, Vec<Range<usize>>)> = BTreeMap::new();
    for source in ranges {
        let at = normalized_at(source.start);
        let stack = super::html_paragraph::stack_at(&tokens, at);
        let quotes = stack
            .iter()
            .enumerate()
            .filter(
                |(_, token)| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "blockquote"),
            )
            .collect::<Vec<_>>();
        for (index, quote) in quotes.into_iter().rev().take(1) {
            let end = closing(&tokens, quote, input.text.len());
            let inner = &stack[index + 1..];
            let paragraph = inner.iter().find(|token| {
                matches!(&token.kind, TokenKind::Tag(tag)
                    if matches!(tag.name.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "blockquote"))
            });
            let extent = if let Some(paragraph) = paragraph {
                paragraph.range.start..closing(&tokens, paragraph, end.start).end
            } else {
                let mut extent = at..normalized_at(source.end);
                if let Some(first) = inner.first() {
                    extent.start = first.range.start;
                }
                let content_end = extent.end;
                for token in tokens
                    .iter()
                    .filter(|token| token.range.start >= content_end)
                {
                    if matches!(&token.kind, TokenKind::Tag(tag) if tag.end && !html::block(&tag.name))
                    {
                        extent.end = token.range.end;
                    } else {
                        break;
                    }
                }
                extent
            };
            if extent.start < quote.range.end || extent.end > end.start {
                return Err(DocumentError::AmbiguousProjection);
            }
            groups
                .entry(quote.range.start)
                .or_insert_with(|| (end, Vec::new()))
                .1
                .push(extent);
        }
    }
    if groups.is_empty() {
        return Ok(None);
    }
    let mut patches = Vec::new();
    for (start, (end, mut selected)) in groups {
        let opening = tokens
            .iter()
            .find(|token| token.range.start == start)
            .unwrap();
        selected.sort_by_key(|range| (range.start, range.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in selected {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| range.start <= previous.end)
            {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        // A quote containing bare prose is itself the paragraph wrapper.
        // Retain that boundary as p when removing the quotation treatment.
        let containing_item = super::html_paragraph::stack_at(&tokens, start)
            .into_iter()
            .rev()
            .find(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "li"));
        let owns_item_body = containing_item.is_some_and(|item| {
            input.text[item.range.end..start].trim().is_empty()
                && tokens.iter().filter(|token| token.range.start >= end.end)
                    .find(|token| !matches!(&token.kind, TokenKind::Opaque)
                        && !(matches!(&token.kind, TokenKind::Text) && input.text[token.range.clone()].trim().is_empty()))
                    .map_or(true, |token| matches!(&token.kind, TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name)))
        });
        if merged.len() == 1
            && !owns_item_body
            && input.text[opening.range.end..merged[0].start]
                .trim()
                .is_empty()
            && input.text[merged[0].end..end.start].trim().is_empty()
            && !tokens.iter().any(|token| {
                token.range.start >= opening.range.end
                    && token.range.end <= end.start
                    && matches!(&token.kind, TokenKind::Tag(tag) if html::block(&tag.name))
            })
        {
            patches.push((
                converter.source_range(start + 1..opening.range.end.min(start + 11)),
                "p".into(),
            ));
            if !end.is_empty() {
                patches.push((
                    converter.source_range(end.start + 2..end.start + 12),
                    "p".into(),
                ));
            } else {
                patches.push((converter.source_range(end.clone()), "</p>".into()));
            }
            continue;
        }
        for range in merged {
            if input.text[opening.range.end..range.start].trim().is_empty() {
                patches.push((converter.source_range(opening.range.clone()), String::new()));
            } else {
                patches.push((
                    converter.source_range(range.start..range.start),
                    "</blockquote>".into(),
                ));
            }
            if input.text[range.end..end.start].trim().is_empty() {
                if !end.is_empty() {
                    patches.push((converter.source_range(end.clone()), String::new()));
                }
            } else {
                patches.push((
                    converter.source_range(range.end..range.end),
                    input.text[opening.range.clone()].into(),
                ));
            }
        }
    }
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    patches.dedup();
    Ok(Some(patches))
}
