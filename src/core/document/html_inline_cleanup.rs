//! Remove character-only scopes emptied by the current logical edit.
//! Empty authored siblings, paragraph owners, and metadata remain source authority.
use super::html::{self, Tag, Token, TokenKind};
use super::{Document, DocumentError, Format, SourcePatch, TextEdit};
use std::ops::Range;

fn character_scope(tag: &Tag) -> bool {
    matches!(tag.name.as_str(),
        "span" | "b" | "strong" | "i" | "em" | "u" | "s" | "strike" | "del"
        | "font" | "small" | "big" | "code" | "sup" | "sub" | "tt" | "mark"
        | "kbd" | "samp" | "var" | "a")
        // These attributes belong to the character treatment. Other attributes
        // can retain an anchor, application metadata, or unsupported semantics.
        && tag.attributes.iter().all(|(name, value)| matches!(name.as_str(),
            "style" | "class" | "lang" | "dir" | "color" | "face" | "size"
            | "href" | "title" | "target" | "rel")
            || name == "data-viem-character" && value == "none")
}

fn intersects(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

pub(super) fn remove_empty_edited_scopes(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    if document.format() != Format::Html || edits.iter().all(|edit| edit.range.is_empty()) {
        return Ok(());
    }
    let selected = edits.iter().filter(|edit| !edit.range.is_empty()).flat_map(|edit| {
        document.projection().provenance_for_region(&edit.range).into_iter()
            .filter(|span| intersects(&span.formatted, &edit.range) && !span.source.is_empty())
            .map(|span| span.source)
    }).collect::<Vec<_>>();
    if selected.is_empty() { return Ok(()); }
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let mapper = super::rich_text::Builder::new(&input, document.revision());
    let original_patch_count = patches.len();
    let mut stack = Vec::<Token>::new();
    for token in html::tokenize(&input.text) {
        let TokenKind::Tag(tag) = &token.kind else { continue };
        if !tag.end {
            if !html::void(&tag.name) { stack.push(token); }
            continue;
        }
        let Some(index) = stack.iter().rposition(|opening| {
            matches!(&opening.kind, TokenKind::Tag(open) if open.name == tag.name)
        }) else { continue };
        let opening = stack[index].clone();
        stack.truncate(index);
        let TokenKind::Tag(open) = &opening.kind else { unreachable!() };
        if !character_scope(open) { continue; }
        let body = mapper.source_range(opening.range.end..token.range.start);
        if !selected.iter().any(|range| intersects(range, &body)) { continue; }
        let opening = mapper.source_range(opening.range);
        let closing = mapper.source_range(token.range);
        // An adapter replacing a delimiter owns its new spelling. Deleting
        // uncovered pieces of a rewritten tag could corrupt that transaction.
        if patches.iter().any(|patch| !patch.replacement().is_empty()
            && (intersects(&patch.range(), &opening) || intersects(&patch.range(), &closing))) {
            continue;
        }
        let remaining = super::markdown_code::edited_source_fragment(document, &body, patches)?;
        let remaining = document.encoding().decode(&remaining)?;
        // Comments have no character-style context; keep their exact bytes
        // while dropping the now-unused delimiters around them. Whitespace,
        // hidden elements, unknown elements, and atomic objects are retained.
        if !html::tokenize(&remaining.text).iter().all(|token| {
            matches!(token.kind, TokenKind::Opaque)
                && remaining.text[token.range.clone()].starts_with("<!--")
        }) { continue; }
        let mut support = Vec::new();
        super::source_edit::append_uncovered_deletions(&opening, patches, &mut support);
        super::source_edit::append_uncovered_deletions(&closing, patches, &mut support);
        patches.extend(support);
    }
    if patches.len() != original_patch_count {
        super::html_paragraph::preserve_after_inline_cleanup(document, edits, patches)?;
    }
    Ok(())
}
