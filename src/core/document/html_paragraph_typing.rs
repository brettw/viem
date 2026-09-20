//! Materialize only the anonymous paragraph receiving a text edit. Existing
//! source owners and every byte outside the two supporting delimiters survive.
use super::html::{self, Token, TokenKind};
use super::{BoundaryAffinity, Document, DocumentError, Format, SourcePatch, TextEdit};
use std::ops::Range;

pub(super) fn materialize(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    if document.format() != Format::Html || !edits.iter().any(|edit| !edit.replacement.is_empty()) {
        return Ok(());
    }
    // An entirely empty projection may have no contributor-derived anchor.
    // Use the rich insertion resolver, which places input inside an authored
    // body/envelope instead of treating the file's fallback offset as content.
    let empty_source = (document.projection().text_tree().byte_len() == 0)
        .then(|| super::rich_text::text_source_range(document, &(0..0)).map(|range| range.start))
        .transpose()?;
    let source_at = |at, affinity| empty_source.map(Ok).unwrap_or_else(||
        super::source_edit::insertion_point(document.projection(), at, Some(affinity))
            .ok_or(DocumentError::AmbiguousProjection));
    let mut affected = Vec::new();
    for edit in edits.iter().filter(|edit| !edit.replacement.is_empty()) {
        let Some(block) = super::edit_boundary::paragraph_at(document, edit.range.start)? else { continue; };
        let Ok(source) = source_at(block.range.start, BoundaryAffinity::Downstream) else { continue; };
        if !has_local_owner(document, source)? { affected.push((edit, block)); }
    }
    if affected.is_empty() { return Ok(()); }
    let input = super::line_endings::normalize(
        &document.encoding().decode(&document.source_bytes())?, document.file_format(),
    );
    let tokens = html::tokenize(&input.text);
    let mapper = super::rich_text::Builder::new(&input, document.revision());
    let normalized = |source| input.units.get(input.units.partition_point(|unit| unit.source.end <= source))
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let mut extents = Vec::<Range<usize>>::new();
    for (edit, block) in affected {
        let start = normalized(source_at(block.range.start, BoundaryAffinity::Downstream)?);
        let open = super::html_paragraph::stack_at(&tokens, start);
        if open.iter().any(|token| matches!(&token.kind, TokenKind::Tag(tag)
            if html::owns_paragraph(tag, document.projection().style_sheet()) || tag.name == "table")) {
            continue;
        }
        let merged = super::edit_boundary::merged_paragraphs(document, &edit.range);
        let Ok(end) = source_at(merged.last().map_or(block.range.end, |last| last.range.end), BoundaryAffinity::Upstream) else { continue; };
        let end = normalized(end);
        if end < start { continue; }
        let inline = open.iter().rposition(|token| matches!(&token.kind,
            TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name)))
            .map_or(0, |index| index + 1);
        // An inline source ancestor may enclose several projected paragraphs.
        // Keep that ancestor outside the new p; otherwise closing p also closes
        // the ancestor in HTML5 and changes the style of untouched later text.
        let inline = (inline..open.len()).rev()
            .find(|&index| encloses_structure(&tokens, open[index]))
            .map_or(inline, |index| index + 1);
        let first = open.get(inline).map_or(start, |token| token.range.start);
        let last = phrasing_end(&tokens, end, first);
        // Atomic tables and retained block containers cannot be children of p.
        // Their existing flow-capable container continues to own this content;
        // ordinary prose materialization must not change its block topology.
        if tokens.iter().filter(|token| first <= token.range.start && token.range.start < last)
            .any(|token| matches!(&token.kind, TokenKind::Tag(tag)
                if !tag.end && (super::html_paragraph::structural(&tag.name) || tag.name == "table"))) {
            continue;
        }
        let extent = mapper.source_range(first..last);
        // Enter, paragraph assignment and structured paste may already have
        // supplied the necessary owner in their explicit source patches.
        let supplies_owner = patches.iter().filter(|patch|
            extent.start <= patch.range().start && patch.range().start < extent.end
                || extent.is_empty() && patch.range().start == extent.start)
            .try_fold(false, |found, patch| -> Result<bool, DocumentError> {
                if found { return Ok(true); }
                let syntax = document.encoding().decode_region(patch.replacement(), 0)?.text;
                Ok(html::tokenize(&syntax).iter().any(|token| matches!(&token.kind,
                    TokenKind::Tag(tag) if !tag.end && html::owns_paragraph(tag, document.projection().style_sheet()))))
            })?;
        if supplies_owner { continue; }
        extents.push(extent);
    }
    extents.sort_by_key(|range| (range.start, range.end));
    let mut unique = Vec::<Range<usize>>::new();
    for extent in extents {
        if let Some(previous) = unique.last_mut().filter(|previous|
            extent.start < previous.end || **previous == extent) {
            previous.end = previous.end.max(extent.end);
        } else { unique.push(extent); }
    }
    for extent in unique {
        insert_delimiter(patches, extent.start, document.encoding().encode_fragment("<p>")?, true);
        insert_delimiter(patches, extent.end, document.encoding().encode_fragment("</p>")?, false);
    }
    Ok(())
}

/// The ordinary case needs only the short syntax prefix before the first
/// contributor. A complete owner followed solely by balanced phrasing syntax
/// proves ownership without scanning preceding paragraphs. An oversized or
/// unusual prefix falls back to the full lossless-token context above.
fn has_local_owner(document: &Document, source: usize) -> Result<bool, DocumentError> {
    let start = source.saturating_sub(4096);
    let bytes = document.state().source.bytes_in(start..source).ok_or(DocumentError::AmbiguousProjection)?;
    let prefix = document.encoding().decode_region(&bytes, start)?.text;
    let tokens = html::tokenize(&prefix);
    for (index, token) in tokens.iter().enumerate().rev() {
        let TokenKind::Tag(owner) = &token.kind else { continue; };
        if owner.end || !html::owns_paragraph(owner, document.projection().style_sheet()) { continue; }
        let mut open = vec![owner.name.as_str()];
        let mut valid = true;
        for token in &tokens[index + 1..] {
            match &token.kind {
                TokenKind::Tag(tag) if tag.end => {
                    if open.last().copied() != Some(tag.name.as_str()) { valid = false; break; }
                    open.pop();
                    if open.is_empty() { valid = false; break; }
                }
                TokenKind::Tag(tag) if !super::html_paragraph::structural(&tag.name)
                    && !html::hidden(&tag.name) && !html::atomic(&tag.name) => {
                    if !html::void(&tag.name) { open.push(&tag.name); }
                }
                TokenKind::Text if prefix[token.range.clone()].chars().all(super::html_whitespace::collapsible) => {}
                TokenKind::Opaque if prefix[token.range.clone()].starts_with("<!--")
                    && prefix[token.range.clone()].ends_with("-->") => {}
                _ => { valid = false; break; }
            }
        }
        if valid { return Ok(true); }
    }
    Ok(false)
}

fn encloses_structure(tokens: &[Token], opening: &Token) -> bool {
    let TokenKind::Tag(owner) = &opening.kind else { return false; };
    let mut depth = 0usize;
    for token in tokens.iter().filter(|token| token.range.start >= opening.range.end) {
        let TokenKind::Tag(tag) = &token.kind else { continue; };
        if tag.name == owner.name {
            if tag.end {
                if depth == 0 { return false; }
                depth -= 1;
            } else { depth += 1; }
        }
        if super::html_paragraph::structural(&tag.name) { return true; }
    }
    false
}

fn phrasing_end(tokens: &[Token], end: usize, first: usize) -> usize {
    let mut last = end;
    let mut remaining = super::html_paragraph::stack_at(tokens, end);
    for token in tokens.iter().filter(|token| token.range.start >= end) {
        match &token.kind {
            TokenKind::Tag(tag) if tag.end && !super::html_paragraph::structural(&tag.name) => {
                let Some(index) = remaining.iter().rposition(|token|
                    matches!(&token.kind, TokenKind::Tag(other) if other.name == tag.name)) else { break; };
                if remaining[index].range.start < first { break; }
                remaining.truncate(index);
                last = token.range.end;
            }
            TokenKind::Opaque => continue,
            _ => break,
        }
    }
    last
}

fn insert_delimiter(patches: &mut Vec<SourcePatch>, at: usize, delimiter: Vec<u8>, opening: bool) {
    let index = if opening {
        patches.iter().position(|patch| patch.range().start == at)
    } else {
        patches.iter().position(|patch| patch.range().end == at)
            .or_else(|| patches.iter().position(|patch| patch.range().start == at))
    };
    if let Some(index) = index {
        let patch = &patches[index];
        let before = opening || !patch.range().is_empty() && patch.range().start == at;
        patches[index].insert_replacement_syntax(&delimiter, before);
    } else {
        patches.push(SourcePatch::primary(at..at, delimiter));
    }
}
