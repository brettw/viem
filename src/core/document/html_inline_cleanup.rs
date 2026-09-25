//! Remove character-only scopes emptied by the current logical edit.
//! Empty authored siblings, paragraph owners, and metadata remain source authority.
use super::html::{self, Tag, Token, TokenKind};
use super::html_scope_index::HtmlScope;
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
    let Some(index) = document.projection().html_scope_index() else {
        return remove_with_full_tokens(document, &selected, patches, edits);
    };
    let original_patch_count = patches.len();
    // Only scopes open inside an edited contributor can have an edited body.
    // Deeper scopes go first: removing an emptied child can empty its parent.
    let mut candidates = Vec::<(usize, usize, std::sync::Arc<HtmlScope>)>::new();
    for range in &selected {
        let scopes = index.scopes_at(range.start);
        for (depth, scope) in scopes.into_iter().enumerate() {
            if character_scope(&scope.tag) { candidates.push((depth, range.start, scope)); }
        }
    }
    candidates.sort_by_key(|(depth, at, _)| (std::cmp::Reverse(*depth), *at));
    let mut seen = Vec::<Range<usize>>::new();
    for (_, at, scope) in candidates {
        // The body is emptied only when everything between the delimiters is
        // already removed or is opaque source; stop at any retained content.
        let Some((opening, closing)) = index.element_delimiters(at, &scope, |piece, opaque| {
            opaque || covered(piece, patches)
        }) else { continue };
        if seen.contains(&opening) { continue; }
        seen.push(opening.clone());
        remove_if_empty(document, opening, closing, patches)?;
    }
    if patches.len() != original_patch_count {
        super::html_paragraph::preserve_after_inline_cleanup(document, edits, patches)?;
    }
    Ok(())
}

fn covered(piece: &Range<usize>, patches: &[SourcePatch]) -> bool {
    let mut at = piece.start;
    while at < piece.end {
        let Some(end) = patches.iter().filter(|patch| patch.range().start <= at && at < patch.range().end)
            .map(|patch| patch.range().end).max() else { return false };
        at = end;
    }
    true
}

fn remove_if_empty(
    document: &Document,
    opening: Range<usize>,
    closing: Range<usize>,
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    // An adapter replacing a delimiter owns its new spelling. Deleting
    // uncovered pieces of a rewritten tag could corrupt that transaction.
    if patches.iter().any(|patch| !patch.replacement().is_empty()
        && (intersects(&patch.range(), &opening) || intersects(&patch.range(), &closing))) {
        return Ok(());
    }
    let body = opening.end..closing.start;
    let remaining = super::markdown_code::edited_source_fragment(document, &body, patches)?;
    let remaining = document.encoding().decode(&remaining)?;
    // Comments have no character-style context; keep their exact bytes
    // while dropping the now-unused delimiters around them. Whitespace,
    // hidden elements, unknown elements, and atomic objects are retained.
    if !html::tokenize(&remaining.text).iter().all(|token| {
        matches!(token.kind, TokenKind::Opaque)
            && remaining.text[token.range.clone()].starts_with("<!--")
    }) { return Ok(()); }
    let mut support = Vec::new();
    super::source_edit::append_uncovered_deletions(&opening, patches, &mut support);
    super::source_edit::append_uncovered_deletions(&closing, patches, &mut support);
    for deletion in support {
        if !patches.iter_mut().any(|patch| patch.absorb_adjacent_deletion(&deletion.range())) {
            patches.push(deletion);
        }
    }
    Ok(())
}

/// Reference behavior for a projection without a lexical scope index.
fn remove_with_full_tokens(
    document: &Document,
    selected: &[Range<usize>],
    patches: &mut Vec<SourcePatch>,
    edits: &[TextEdit],
) -> Result<(), DocumentError> {
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
        remove_if_empty(document, mapper.source_range(opening.range), mapper.source_range(token.range), patches)?;
    }
    if patches.len() != original_patch_count {
        super::html_paragraph::preserve_after_inline_cleanup(document, edits, patches)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Document, Encoding, Format, SourcePatch, TextEdit};

    fn spelling(document: &Document, patches: &[SourcePatch]) -> Vec<u8> {
        let mut source = document.source_bytes();
        let mut ordered = patches.to_vec();
        ordered.sort_by_key(|patch| (patch.range().start, patch.range().end));
        for patch in ordered.iter().rev() {
            source.splice(patch.range(), patch.replacement().iter().copied());
        }
        source
    }

    /// The indexed walk must choose the same delimiters as the full-token
    /// reference, including nested, commented, and implicitly closed scopes.
    #[test]
    fn indexed_cleanup_matches_full_token_reference() {
        let sources = [
            "<p><b>x</b></p>",
            "<p><span class='Code'><b><i>x</i></b></span></p>",
            "<p><b><!--keep-->x<!--also--></b>tail</p>",
            "<p><b>x</b><i></i>y</p>",
            "<p><b>x</b>a<i>y</i>b<u>cd</u></p>",
            "<p><b>xy</b> z <i>w</i></p><p><s>q</s></p>",
            "<p><b>x<script>keep()</script></b></p>",
            "<p><b><i>A</b>B</i>C</p>",
            "<p><a href='/x'><sup>x</sup></a><b>y<br>z</b></p>",
            "<div><b>x<p>y</p></b></div>",
        ];
        for source in sources {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
            let length = document.text().len();
            let mut cases = Vec::new();
            for start in 0..length {
                for end in start + 1..=length {
                    cases.push(vec![TextEdit::new(start..end, "")]);
                    cases.push(vec![TextEdit::new(start..end, "Q")]);
                }
            }
            for start in 0..length {
                for other in start + 2..length {
                    cases.push(vec![TextEdit::new(start..start + 1, ""), TextEdit::new(other..other + 1, "")]);
                }
            }
            for edits in cases {
                let Ok(translated) = document.translate_source_edits(edits.iter().map(|edit| (edit, None))) else {
                    continue;
                };
                let mut indexed = translated.clone();
                super::remove_empty_edited_scopes(&document, &edits, &mut indexed).unwrap();
                let selected = edits.iter().flat_map(|edit| {
                    document.projection().provenance_for_region(&edit.range).into_iter()
                        .filter(|span| super::intersects(&span.formatted, &edit.range) && !span.source.is_empty())
                        .map(|span| span.source)
                }).collect::<Vec<_>>();
                let mut reference = translated.clone();
                if !selected.is_empty() {
                    super::remove_with_full_tokens(&document, &selected, &mut reference, &edits).unwrap();
                }
                assert_eq!(
                    String::from_utf8(spelling(&document, &indexed)).unwrap(),
                    String::from_utf8(spelling(&document, &reference)).unwrap(),
                    "{source:?} {:?}", edits.iter().map(|edit| edit.range.clone()).collect::<Vec<_>>(),
                );
            }
        }
    }
}
