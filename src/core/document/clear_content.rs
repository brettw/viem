//! Explicit whole-document content deletion. Unlike deleting the last glyph,
//! this intention owns the surrounding content scopes and removes them too.
use super::*;

impl Document {
    pub(super) fn prepare_clear_document_content(
        &self,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.format() == Format::Html
            && self.projection().text_tree().byte_len() == 0
            && self.projection().blocks().iter().all(|block|
                block.style == StyleId::from("Paragraph")
                    && !matches!(block.kind, super::super::BlockKind::ListItem { .. }))
        {
            // There is no selected text or paragraph separator to delete.
            // Preserve an already empty paragraph and unrelated source scopes.
            return Ok(self.no_op_prepared());
        }
        let bytes = self.source_bytes();
        let decoded = self.encoding().decode(&bytes)?;
        let input = normalize(&decoded, self.file_format());
        let converter = super::super::rich_text::Builder::new(&input, Revision(0));
        let mut keep = vec![0..decoded.bom_len];
        match self.format() {
            Format::Html => {
                use super::super::html::{self, TokenKind};
                let tokens = html::tokenize(&input.text);
                keep.extend(untouched_empty_html_scopes(self, &tokens, &converter));
                let mut hidden: Option<(&str, usize, usize)> = None;
                let mut removed_owners: Vec<&str> = Vec::new();
                for token in &tokens {
                    if let Some((name, start, depth)) = hidden {
                        if let TokenKind::Tag(tag) = &token.kind {
                            if tag.name == name {
                                if tag.end && depth == 1 {
                                    keep.push(converter.source_range(start..token.range.end));
                                    hidden = None;
                                } else {
                                    hidden = Some((
                                        name,
                                        start,
                                        if tag.end { depth - 1 } else { depth + 1 },
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    // Raw-text bodies and comments inside a selected atomic
                    // object belong to that object. Keeping their opaque
                    // tokens after deleting its tags would expose text (for
                    // example an iframe's fallback) in the cleared document.
                    if let TokenKind::Tag(tag) = &token.kind {
                        if tag.end {
                            if let Some(index) = removed_owners
                                .iter()
                                .rposition(|name| *name == tag.name)
                            {
                                removed_owners.truncate(index);
                                continue;
                            }
                        } else if html::atomic(&tag.name)
                            || matches!(tag.name.as_str(), "xmp" | "noembed" | "noframes")
                        {
                            if !html::void(&tag.name)
                                && !(matches!(tag.name.as_str(), "svg" | "math")
                                    && input.text[token.range.clone()]
                                        .trim_end()
                                        .ends_with("/>"))
                            {
                                removed_owners.push(&tag.name);
                            }
                            continue;
                        }
                    }
                    if !removed_owners.is_empty() {
                        continue;
                    }
                    match &token.kind {
                        TokenKind::Tag(tag) if !tag.end && html::hidden(&tag.name) => {
                            hidden = Some((&tag.name, token.range.start, 1));
                        }
                        TokenKind::Tag(tag)
                            if matches!(
                                tag.name.as_str(),
                                "html" | "body" | "meta" | "link" | "base"
                            ) =>
                        {
                            keep.push(converter.source_range(token.range.clone()));
                        }
                        TokenKind::Opaque => keep.push(converter.source_range(token.range.clone())),
                        TokenKind::Text if input.text[token.range.clone()].chars()
                            .all(super::super::html_whitespace::collapsible) =>
                        {
                            // Source-only spacing between selected owners and
                            // document metadata does not belong to their text.
                            let source = converter.source_range(token.range.clone());
                            if !self.projection().provenance_contained_in_source(&source).iter()
                                .any(|span| !span.formatted.is_empty())
                            {
                                keep.push(source);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some((_, start, _)) = hidden {
                    keep.push(converter.source_range(start..input.text.len()));
                }
            }
            Format::Rtf => {
                use super::super::rtf::{self, Kind};
                let tokens = rtf::tokenize(&input);
                let mut stack = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    match &token.kind {
                        Kind::Open => {
                            if stack.is_empty() {
                                keep.push(converter.source_range(token.range.clone()));
                            }
                            stack.push(index);
                        }
                        Kind::Close => {
                            if let Some(open) = stack.pop() {
                                if stack.is_empty() {
                                    keep.push(converter.source_range(token.range.clone()));
                                } else if tokens[open + 1..index].iter().take(2).any(|token| {
                                    matches!(&token.kind, Kind::Symbol('*')) || matches!(&token.kind,
                                        Kind::Control(name, _) if matches!(name.as_str(),
                                            "fonttbl" | "colortbl" | "stylesheet" | "listtable" |
                                            "listoverridetable" | "info" | "generator"))
                                }) && !tokens[open + 1..index].iter().take(2).any(|token| {
                                    matches!(&token.kind, Kind::Control(name, _) if name == "pn")
                                }) {
                                    keep.push(converter.source_range(tokens[open].range.start..token.range.end));
                                }
                            }
                        }
                        Kind::Control(name, _)
                            if stack.len() <= 1
                                && matches!(
                                    name.as_str(),
                                    "rtf"
                                        | "ansi"
                                        | "mac"
                                        | "pc"
                                        | "pca"
                                        | "ansicpg"
                                        | "deff"
                                        | "deflang"
                                        | "deflangfe"
                                        | "adeflang"
                                        | "uc"
                                        | "deftab"
                                        | "paperw"
                                        | "paperh"
                                        | "margl"
                                        | "margr"
                                        | "margt"
                                        | "margb"
                                        | "landscape"
                                        | "viewkind"
                                        | "viewscale"
                                ) =>
                        {
                            keep.push(converter.source_range(token.range.clone()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        keep.sort_by_key(|range| (range.start, range.end));
        let mut patches = Vec::new();
        let mut at = 0;
        for retained in keep {
            if retained.start > at {
                patches.push(SourcePatch::primary(at..retained.start, Vec::new()));
            }
            at = at.max(retained.end);
        }
        if at < bytes.len() {
            patches.push(SourcePatch::primary(at..bytes.len(), Vec::new()));
        }
        if self.format() == Format::Html {
            // Whole-content deletion consumes the paragraph owners as well as
            // their text. Leave one explicit normal paragraph at the first
            // edited body gap, preserving document metadata and empty siblings.
            let paragraph = self.encoding().encode_fragment("<p></p>")?;
            if let Some(first) = patches.first_mut() {
                first.insert_replacement_syntax(&paragraph, true);
            } else {
                let position = super::super::html::tokenize(&input.text).iter()
                    .filter(|token| matches!(&token.kind, super::super::html::TokenKind::Tag(tag)
                        if tag.end && matches!(tag.name.as_str(), "body" | "html")))
                    .map(|token| converter.source_range(token.range.clone()).start)
                    .next().unwrap_or(bytes.len()).max(decoded.bom_len);
                patches.push(SourcePatch::primary(position..position, paragraph));
            }
        }
        patches.retain(|patch| bytes.get(patch.range()) != Some(patch.replacement()));
        let prepared = self.prepare_text_edits_with_patches(
            vec![TextEdit::new(
                0..self.projection().text_tree().byte_len(),
                "",
            )],
            Some(patches),
        )?;
        let projection = match &prepared.publication {
            PreparedPublication::State(state) => &state.projection,
            PreparedPublication::NoOp => self.projection(),
            _ => return Err(DocumentError::VerificationFailed.into()),
        };
        if projection.text_tree().byte_len() != 0
            || projection.blocks().iter().any(|block| {
                (block.style == StyleId::from("Block quote") || block.quote_depth > 0)
                    || block.style.is_internal_list()
                    || matches!(block.kind, super::super::BlockKind::ListItem { .. })
            })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }
}

/// Whole-document selection owns all visible paragraphs, including empty ones.
/// A transparent source scope with no paragraph or visible contributor is an
/// unrelated empty sibling (possibly containing metadata), not selected text.
fn untouched_empty_html_scopes(
    document: &Document,
    tokens: &[super::super::html::Token],
    mapper: &super::super::rich_text::Builder<'_>,
) -> Vec<Range<usize>> {
    use super::super::html::{self, TokenKind};
    let mut visible = document.projection().provenance().iter()
        .filter(|span| !span.formatted.is_empty() && !span.source.is_empty())
        .map(|span| span.source.clone()).collect::<Vec<_>>();
    visible.sort_by_key(|range| range.start);
    let mut maximum = 0;
    let ends = visible.iter().map(|range| {
        maximum = maximum.max(range.end);
        maximum
    }).collect::<Vec<_>>();
    let mut paragraphs = Vec::new();
    let mut hidden: Option<(&str, usize)> = None;
    for token in tokens {
        let TokenKind::Tag(tag) = &token.kind else { continue; };
        if let Some((name, depth)) = hidden {
            if tag.name == name {
                hidden = if tag.end && depth == 1 { None } else {
                    Some((name, if tag.end { depth - 1 } else { depth + 1 }))
                };
            }
            continue;
        }
        if !tag.end && html::hidden(&tag.name) {
            hidden = Some((&tag.name, 1));
        } else if !tag.end && (html::owns_paragraph(tag, document.projection().style_sheet())
            || tag.name == "blockquote") {
            // Markup inside a template or another hidden body is metadata,
            // not a paragraph selected in the formatted document.
            paragraphs.push(token.range.start);
        }
    }
    let mut open = Vec::new();
    let mut retained = Vec::new();
    let mut hidden: Option<(&str, usize)> = None;
    for (index, token) in tokens.iter().enumerate() {
        let TokenKind::Tag(tag) = &token.kind else { continue; };
        if let Some((name, depth)) = hidden {
            if tag.name == name {
                hidden = if tag.end && depth == 1 { None } else {
                    Some((name, if tag.end { depth - 1 } else { depth + 1 }))
                };
            }
            continue;
        }
        if !tag.end && html::hidden(&tag.name) {
            // Hidden-body syntax cannot close an enclosing visible scope.
            // Its complete source is retained separately by the clear plan.
            hidden = Some((&tag.name, 1));
            continue;
        }
        if !tag.end {
            if !html::void(&tag.name) { open.push(index); }
            continue;
        }
        let Some(depth) = open.iter().rposition(|&index|
            matches!(&tokens[index].kind, TokenKind::Tag(other) if other.name == tag.name)) else { continue; };
        let start = tokens[open[depth]].range.start;
        open.truncate(depth);
        if !html::block(&tag.name) && open.iter().any(|&index|
            matches!(&tokens[index].kind, TokenKind::Tag(ancestor)
                if !matches!(ancestor.name.as_str(), "html" | "body" | "head")))
        {
            // An empty inline scope inside deleted content belongs to that
            // content. An untouched empty container may retain it as part of
            // its own complete source range instead.
            continue;
        }
        let paragraph = paragraphs.partition_point(|&at| at < start);
        if paragraphs.get(paragraph).is_some_and(|&at| at < token.range.end) { continue; }
        let source = mapper.source_range(start..token.range.end);
        let count = visible.partition_point(|range| range.start < source.end);
        if count == 0 || ends[count - 1] <= source.start {
            retained.push(source);
        }
    }
    retained
}
