//! Local paragraph splitting over the original HTML tokens. The recovered
//! tree remains the semantic authority; the transaction verifies the projected
//! text and paragraph assignments before publishing these source patches.
use super::html::{self, Token, TokenKind};
use super::{Block, CharacterProperties, DocumentError, Revision, StyleId, StyleSheet};
use std::collections::BTreeMap;
use std::ops::Range;

fn paragraph(name: &str) -> bool {
    matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}
pub(super) fn structural(name: &str) -> bool {
    paragraph(name)
        || matches!(
            name,
            "html"
                | "head"
                | "body"
                | "div"
                | "section"
                | "article"
                | "blockquote"
                | "ul"
                | "ol"
                | "li"
                | "header"
                | "footer"
                | "main"
                | "nav"
                | "aside"
                | "dl"
                | "dt"
                | "dd"
                | "table"
                | "pre"
        )
}
fn void(name: &str) -> bool {
    matches!(
        name,
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
    )
}
pub(super) fn stack_at(tokens: &[Token], at: usize) -> Vec<&Token> {
    let mut open: Vec<&Token> = Vec::new();
    for token in tokens.iter().take_while(|token| token.range.end <= at) {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if tag.end {
            if let Some(index) = open
                .iter()
                .rposition(|token| matches!(&token.kind, TokenKind::Tag(t) if t.name == tag.name))
            {
                open.truncate(index);
            }
        } else if !void(&tag.name) {
            if structural(&tag.name) {
                if let Some(index) = open.iter().rposition(
                    |token| matches!(&token.kind, TokenKind::Tag(t) if paragraph(&t.name)),
                ) {
                    open.truncate(index);
                }
            }
            open.push(token);
        }
    }
    open
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Join two sibling paragraphs by removing their boundary syntax. The first
/// paragraph owns the result; the second paragraph's direct character values
/// stay attached to its original text in a local span.
pub(super) fn join_patches(
    document: &super::Document,
    input: &super::line_endings::NormalizedText,
    edit: &super::TextEdit,
    all_edits: &[super::TextEdit],
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    let blocks = document.projection().blocks();
    let boundaries = blocks
        .windows(2)
        .filter(|pair| edit.range.start <= pair[0].range.end && pair[0].range.end < edit.range.end)
        .collect::<Vec<_>>();
    if boundaries.len() != 1 {
        return Ok(None);
    }
    let pair = boundaries[0];
    let first = &pair[0];
    let second = &pair[1];
    if edit.range.start < first.range.start
        || second.range.end < edit.range.end
        || edit.replacement.contains('\n')
    {
        return Ok(None);
    }
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let text_source_at = |block: &Block| -> Result<usize, DocumentError> {
        document
            .projection()
            .provenance_for_region(&block.range)
            .into_iter()
            .find(|span| !span.source.is_empty())
            .map(|span| span.source.start)
            .or_else(|| {
                document
                    .projection()
                    .source_insertion_point(block.range.start, true)
            })
            .ok_or(DocumentError::AmbiguousProjection)
    };
    let normalized_at = |source: usize| -> Result<usize, DocumentError> {
        input
            .units
            .iter()
            .find(|unit| unit.source.start == source)
            .map(|unit| unit.normalized.start)
            .ok_or(DocumentError::AmbiguousProjection)
    };
    let first_stack = stack_at(&tokens, normalized_at(text_source_at(first)?)?);
    let second_stack = stack_at(&tokens, normalized_at(text_source_at(second)?)?);
    let first_open = first_stack
        .iter()
        .rev()
        .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if paragraph(&tag.name)))
        .or_else(|| {
            first_stack
                .iter()
                .rev()
                .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if tag.name == "li"))
        })
        .copied()
        .ok_or(DocumentError::UnsupportedFormatting)?;
    let second_open = second_stack
        .iter()
        .rev()
        .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if paragraph(&tag.name)))
        .copied()
        .ok_or(DocumentError::UnsupportedFormatting)?;
    let TokenKind::Tag(first_tag) = &first_open.kind else {
        unreachable!()
    };
    let TokenKind::Tag(second_tag) = &second_open.kind else {
        unreachable!()
    };
    let joins_boundary = |boundary: usize| {
        all_edits.iter().any(|edit| {
            edit.range.start <= boundary
                && boundary < edit.range.end
                && !edit.replacement.contains('\n')
        })
    };
    let first_index = blocks
        .iter()
        .position(|block| block.id == first.id)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut origin_index = first_index;
    while origin_index > 0 && joins_boundary(blocks[origin_index - 1].range.end) {
        origin_index -= 1;
    }
    let prior_join = origin_index != first_index;
    let following_join = joins_boundary(second.range.end);
    let origin_stack = stack_at(
        &tokens,
        normalized_at(text_source_at(&blocks[origin_index])?)?,
    );
    let origin = origin_stack
        .iter()
        .rev()
        .find_map(|token| match &token.kind {
            TokenKind::Tag(tag) if paragraph(&tag.name) => Some(tag.name.as_str()),
            _ => None,
        })
        .or_else(|| {
            matches!(blocks[origin_index].kind, super::BlockKind::ListItem { .. }).then_some("")
        })
        .ok_or(DocumentError::UnsupportedFormatting)?;
    let parent = |stack: &Vec<&Token>| {
        stack.iter().filter(|token|matches!(&token.kind,TokenKind::Tag(tag) if structural(&tag.name)&&!paragraph(&tag.name))).map(|token|token.range.start).collect::<Vec<_>>()
    };
    if parent(&first_stack) != parent(&second_stack) {
        return Err(DocumentError::AmbiguousProjection);
    }
    let mut patches = Vec::new();
    for span in document.projection().provenance_for_region(&edit.range) {
        if span.formatted == (first.range.end..second.range.start) {
            continue;
        }
        if span.source.is_empty() {
            return Err(DocumentError::AmbiguousProjection);
        } else {
            patches.push((span.source, String::new()));
        }
    }
    let source_at = document
        .projection()
        .source_insertion_point(edit.range.start, false)
        .ok_or(DocumentError::AmbiguousProjection)?;
    if !edit.replacement.is_empty() {
        patches.push((
            source_at..source_at,
            super::rich_text::escape_html_text_edit(document, source_at, edit)?,
        ));
    }
    // Whitespace previously trimmed at the two paragraph edges becomes inline
    // after joining. Keep it when it can collapse with an inserted ordinary
    // space; otherwise remove only the newly exposed whitespace, retaining
    // every intervening element and comment.
    let trim_before = edit
        .replacement
        .chars()
        .next()
        .map_or(true, |ch| !super::html_whitespace::collapsible(ch));
    let trim_after = edit
        .replacement
        .chars()
        .next_back()
        .map_or(true, |ch| !super::html_whitespace::collapsible(ch));
    if trim_before || trim_after {
        let before = document
            .projection()
            .provenance_for_region(&first.range)
            .into_iter()
            .rev()
            .find(|span| !span.formatted.is_empty() && !span.source.is_empty())
            .map(|span| span.source.end);
        let after = document
            .projection()
            .provenance_for_region(&second.range)
            .into_iter()
            .find(|span| !span.formatted.is_empty() && !span.source.is_empty())
            .map(|span| span.source.start);
        if let (Some(before), Some(after)) = (before, after) {
            let mut hidden = Vec::new();
            let mut whitespace: Vec<Range<usize>> = Vec::new();
            for token in &tokens {
                match &token.kind {
                    TokenKind::Tag(tag) => {
                        if tag.end {
                            if let Some(index) = hidden.iter().rposition(|name| name == &tag.name) {
                                hidden.truncate(index);
                            }
                        } else if !void(&tag.name)
                            && (html::hidden(&tag.name) || html::atomic(&tag.name))
                        {
                            hidden.push(tag.name.clone());
                        }
                    }
                    TokenKind::Text if hidden.is_empty() => {
                        let source = converter.source_range(token.range.clone());
                        if source.end <= before || source.start >= after {
                            continue;
                        }
                        let mut at = token.range.start;
                        while at < token.range.end {
                            let (value, length) =
                                html::reference(&input.text[at..token.range.end], false)
                                    .unwrap_or_else(|| {
                                        let ch = input.text[at..].chars().next().unwrap();
                                        (ch.to_string(), ch.len_utf8())
                                    });
                            let source = converter.source_range(at..at + length);
                            at += length;
                            if source.start < before
                                || source.end > after
                                || !value.chars().all(super::html_whitespace::collapsible)
                                || !(source.end <= source_at && trim_before
                                    || source.start >= source_at && trim_after)
                            {
                                continue;
                            }
                            if let Some(previous) = whitespace
                                .last_mut()
                                .filter(|previous| previous.end == source.start)
                            {
                                previous.end = source.end;
                            } else {
                                whitespace.push(source);
                            }
                        }
                    }
                    _ => {}
                }
            }
            patches.extend(whitespace.into_iter().map(|range| (range, String::new())));
        }
    }
    for token in tokens.iter().filter(|token| {
        first_open.range.end <= token.range.start && token.range.start < second_open.range.start
    }) {
        if let TokenKind::Tag(tag) = &token.kind {
            if tag.end && tag.name == first_tag.name {
                if !prior_join {
                    patches.push((converter.source_range(token.range.clone()), String::new()));
                }
            } else if structural(&tag.name) {
                return Err(DocumentError::AmbiguousProjection);
            }
        }
    }
    let mut character = CharacterProperties::default();
    if let Some(css) = second_tag.attribute("style") {
        html::apply_css(css, &mut character, &mut Default::default());
    }
    if let Some(language) = second_tag.attribute("lang") {
        character.language = Some(language.into());
    }
    if let Some(direction) = second_tag.attribute("dir") {
        character.direction = match direction {
            "rtl" => Some(super::WritingDirection::RightToLeft),
            "ltr" => Some(super::WritingDirection::LeftToRight),
            _ => None,
        };
    }
    let (opening, closing) = if character == CharacterProperties::default() {
        (String::new(), String::new())
    } else {
        html::character_wrapper(&character)
    };
    patches.push((converter.source_range(second_open.range.clone()), opening));
    let mut explicit = false;
    let mut end = input.text.len();
    for token in tokens
        .iter()
        .filter(|token| second_open.range.end <= token.range.start)
    {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if tag.end && tag.name == second_tag.name {
            patches.push((
                converter.source_range(token.range.clone()),
                format!(
                    "{closing}{}",
                    if following_join || origin.is_empty() {
                        String::new()
                    } else {
                        format!("</{origin}>")
                    }
                ),
            ));
            explicit = true;
            break;
        }
        if structural(&tag.name) {
            end = token.range.start;
            break;
        }
    }
    if !explicit {
        let value = format!(
            "{closing}{}",
            if following_join || origin.is_empty() {
                String::new()
            } else {
                format!("</{origin}>")
            }
        );
        if !value.is_empty() {
            patches.push((converter.source_range(end..end), value));
        }
    }
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    Ok(Some(patches))
}
fn opening(
    style: &StyleId,
    sheet: &StyleSheet,
    block: &Block,
    original: Option<&html::Tag>,
) -> (String, String) {
    let name = if style == &sheet.base_paragraph {
        "p".to_owned()
    } else if let Some(level) = style
        .0
        .strip_prefix("Heading")
        .filter(|level| matches!(*level, "1" | "2" | "3" | "4" | "5" | "6"))
    {
        format!("h{level}")
    } else {
        "p".to_owned()
    };
    let mut value = format!("<{name}");
    // Quote paragraphs inherit their native container when split. A class on
    // the new child would duplicate that assignment without any source rule.
    if style != &sheet.base_paragraph && style.0 != "Block quote" && name == "p" {
        value.push_str(&format!(
            " class=\"{}\"",
            super::html_styles::class_name(style, false)
        ));
    }
    let mut character = CharacterProperties::default();
    if let Some(css) = original.and_then(|tag| tag.attribute("style")) {
        html::apply_css(css, &mut character, &mut Default::default());
    }
    let css = [
        super::html_styles::block_css(&block.direct_paragraph),
        html::character_css(&character),
        original
            .and_then(|tag| tag.attribute("style"))
            .and_then(|css| {
                html::cascade_declarations(css)
                    .into_iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("white-space"))
                    .last()
            })
            .map_or_else(String::new, |(_, value)| format!("white-space: {value}")),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("; ");
    if !css.is_empty() {
        value.push_str(&format!(" style=\"{}\"", escape(&css)));
    }
    for key in ["lang", "dir"] {
        if let Some(attribute) = original.and_then(|tag| tag.attribute(key)) {
            value.push_str(&format!(" {key}=\"{}\"", escape(attribute)));
        }
    }
    value.push('>');
    (name, value)
}

pub(super) fn enter_patches(
    input: &super::line_endings::NormalizedText,
    source_at: usize,
    source_extent: Range<usize>,
    block: &Block,
    next: &StyleId,
    sheet: &StyleSheet,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let normalized_at = |source: usize| -> Result<usize, DocumentError> {
        if let Some(unit) = input.units.iter().find(|unit| unit.source.start == source) {
            return Ok(unit.normalized.start);
        }
        if let Some(unit) = input.units.iter().find(|unit| unit.source.end == source) {
            return Ok(unit.normalized.end);
        }
        if input.units.is_empty() && source == 0 {
            return Ok(0);
        }
        Err(DocumentError::AmbiguousProjection)
    };
    let at = normalized_at(source_at)?;
    let start = normalized_at(source_extent.start)?;
    let end = normalized_at(source_extent.end)?;
    let tokens = html::tokenize(&input.text);
    let open = stack_at(&tokens, at);
    let current = open
        .iter()
        .rposition(|token| matches!(&token.kind, TokenKind::Tag(tag) if paragraph(&tag.name)));
    let original = current.map(|index| {
        let TokenKind::Tag(tag) = &open[index].kind else {
            unreachable!()
        };
        tag
    });
    let (new_name, new_open) = opening(next, sheet, block, original);
    let inner_start = current.map(|index| index + 1).unwrap_or_else(|| {
        open.iter()
            .rposition(|token| matches!(&token.kind,TokenKind::Tag(tag) if structural(&tag.name)))
            .map_or(0, |index| index + 1)
    });
    let inner = &open[inner_start..];
    if inner
        .iter()
        .any(|token| matches!(&token.kind,TokenKind::Tag(tag) if structural(&tag.name)))
    {
        return Err(DocumentError::AmbiguousProjection);
    }
    let mut split = String::new();
    for token in inner.iter().rev() {
        let TokenKind::Tag(tag) = &token.kind else {
            unreachable!()
        };
        split.push_str(&format!("</{}>", tag.name));
    }
    let old_name = original.map_or("p", |tag| tag.name.as_str());
    split.push_str(&format!("</{old_name}>{new_open}"));
    for token in inner {
        split.push_str(&input.text[token.range.clone()]);
    }
    let mut insertions: BTreeMap<usize, String> = BTreeMap::new();
    let mut patches = Vec::new();
    if current.is_none() {
        let before = stack_at(&tokens, start);
        let first_inline = before
            .iter()
            .rposition(|token| matches!(&token.kind,TokenKind::Tag(tag) if structural(&tag.name)))
            .map_or(0, |index| index + 1);
        let extent_start = before
            .get(first_inline)
            .map_or(start, |token| token.range.start);
        let (_, old_open) = opening(&block.style, sheet, block, None);
        insertions
            .entry(extent_start)
            .or_default()
            .push_str(&old_open);
        // Complete only the phrasing ancestors that enclosed this paragraph.
        // Comments and following structural elements remain outside the patch.
        let mut extent_end = end;
        let mut remaining = stack_at(&tokens, end);
        for token in tokens.iter().filter(|token| token.range.start >= end) {
            match &token.kind {
                TokenKind::Tag(tag) if tag.end && !structural(&tag.name) => {
                    if let Some(index) = remaining.iter().rposition(
                        |token| matches!(&token.kind,TokenKind::Tag(t) if t.name==tag.name),
                    ) {
                        remaining.truncate(index);
                        extent_end = token.range.end;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        insertions.entry(at).or_default().push_str(&split);
        insertions
            .entry(extent_end)
            .or_default()
            .push_str(&format!("</{new_name}>"));
    } else {
        insertions.entry(at).or_default().push_str(&split);
        let mut boundary = input.text.len();
        let mut found_close = false;
        for token in tokens.iter().filter(|token| token.range.start >= at) {
            let TokenKind::Tag(tag) = &token.kind else {
                continue;
            };
            if tag.end && tag.name == old_name {
                if tag.name != new_name {
                    patches.push((
                        converter.source_range(
                            token.range.start + 2..token.range.start + 2 + tag.name.len(),
                        ),
                        new_name.clone(),
                    ));
                }
                found_close = true;
                break;
            }
            if structural(&tag.name) {
                boundary = token.range.start;
                break;
            }
        }
        if !found_close {
            insertions
                .entry(boundary)
                .or_default()
                .push_str(&format!("</{new_name}>"));
        }
    }
    patches.extend(
        insertions
            .into_iter()
            .map(|(at, value)| (converter.source_range(at..at), value)),
    );
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    Ok(patches)
}

/// A paragraph split makes adjacent collapsible whitespace become leading or
/// trailing whitespace. Protect the visible space with NBSP in both source and
/// formatted text so verification and position maps include its UTF-8 growth.
pub(super) fn split_whitespace_protections(
    document: &super::Document,
    at: usize,
    block: &Block,
) -> Result<Vec<(Range<usize>, super::TextEdit)>, DocumentError> {
    let mut patches = Vec::new();
    let start = at.saturating_sub(1).max(block.range.start);
    let end = (at + 1).min(block.range.end);
    let text = document.projection().text_tree();
    for offset in start..end {
        // Only ASCII HTML whitespace can collapse; inspecting a byte here
        // does not create a range through a multibyte grapheme.
        if !text
            .is_char_boundary(offset)
            .map_err(DocumentError::FormattedTextStorage)?
            || !text
                .is_char_boundary(offset + 1)
                .map_err(DocumentError::FormattedTextStorage)?
        {
            continue;
        }
        let range = offset..offset + 1;
        let value = text
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        if !matches!(value.as_str(), " " | "\t" | "\r")
            || document
                .projection()
                .style_spans_for_region(&range)
                .iter()
                .any(|span| {
                    span.range.contains(&offset)
                        && span.application == super::StyleApplication::SourcePreservedWhitespace
                })
        {
            continue;
        }
        let source = super::rich_text::text_source_range(document, &range)?;
        let mut edit = super::TextEdit::new(range, "\u{a0}");
        edit.html_protective_spaces.push(0);
        patches.push((source, edit));
    }
    Ok(patches)
}

/// Deleting complete paragraphs explicitly consumes their structural boundary,
/// whose projection has no text byte. Unknown descendants remain untouched.
pub(super) fn deletion_patches(
    document: &super::Document,
    input: &super::line_endings::NormalizedText,
    range: &Range<usize>,
    whole_line: bool,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    if whole_line {
        if let Some(patches) = partial_paragraph_line_deletion(document, range)? {
            return Ok(Some(patches));
        }
    }
    if !whole_line
        && !document
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?
            .contains('\n')
    {
        return Ok(None);
    }
    let paragraphs = document
        .projection()
        .blocks_for_region(range)
        .into_iter()
        .filter(|block| range.start <= block.range.start && block.range.end <= range.end)
        .collect::<Vec<_>>();
    let Some(first) = paragraphs.first() else {
        return Ok(None);
    };
    let last = paragraphs.last().unwrap();
    let has_list = paragraphs
        .iter()
        .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }));
    let has_boundary = document.projection().blocks().iter().any(|block| {
        block.range.end < document.projection().text_tree().byte_len()
            && range.start <= block.range.end
            && block.range.end < range.end
    });
    if !has_boundary && !has_list {
        return Ok(None);
    }
    if !((first.range.start == range.start
        || (first.range.start == range.start + 1
            && document
                .projection()
                .text_tree()
                .slice(range.start..first.range.start)
                .map_err(DocumentError::FormattedTextStorage)?
                == "\n"))
        && (last.range.end == range.end
            || (last.range.end + 1 == range.end
                && document
                    .projection()
                    .text_tree()
                    .slice(last.range.end..range.end)
                    .map_err(DocumentError::FormattedTextStorage)?
                    == "\n")))
    {
        return Ok(None);
    }
    let selected = paragraphs
        .iter()
        .map(|block| block.id)
        .collect::<std::collections::BTreeSet<_>>();
    let structure = document.projection().list_structure();
    let mut owners = Vec::new();
    let mut preserve_ordinals = Vec::new();
    for list in &structure.lists {
        let mut previous_removed = false;
        for item in &list.items {
            let removed = item.paragraph_ids.iter().any(|id| selected.contains(id));
            let mut remove_owner = removed
                && item.paragraph_ids.iter().all(|id| selected.contains(id));
            if removed {
                let mut children = item.child_lists.clone();
                while let Some(child) = children.pop() {
                    let list = structure
                        .lists
                        .iter()
                        .find(|list| list.id == child)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    for item in &list.items {
                        if !item.paragraph_ids.iter().all(|id| selected.contains(id)) {
                            remove_owner = false;
                        }
                        children.extend(item.child_lists.iter().copied());
                    }
                }
                // The selected paragraph does not own surviving continuation
                // paragraphs or nested items. Keep their existing list owner;
                // deleting visible text must never implicitly delete children.
                if remove_owner {
                    owners.push(item.paragraph_id);
                }
            } else if previous_removed && list.style == super::ListStyle::Numbered {
                preserve_ordinals.push((item.paragraph_id, item.ordinal));
            }
            previous_removed = remove_owner;
        }
    }
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let source_stack = |block: &Block| -> Result<Vec<&Token>, DocumentError> {
        let source_at = super::rich_text::block_source_point(document.projection(), block)?;
        let at = input
            .units
            .iter()
            .find(|unit| unit.source.start == source_at)
            .map(|unit| unit.normalized.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        Ok(stack_at(&tokens, at))
    };
    let mut patches = Vec::new();
    for span in document.projection().provenance_for_region(range) {
        if span.formatted.is_empty() {
            continue;
        }
        if span.formatted.len() == 1
            && document
                .projection()
                .blocks_for_region(&span.formatted)
                .iter()
                .any(|block| block.range.end == span.formatted.start)
        {
            // A structural separator can map to an opening container tag,
            // such as the <ul> preceding a nested list. That tag belongs to
            // the surviving structure; owner patches below remove only the
            // containers whose content is completely selected.
            continue;
        }
        if span.source.is_empty() {
            if document
                .projection()
                .text_tree()
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?
                != "\n"
            {
                return Err(DocumentError::AmbiguousProjection);
            }
        } else {
            patches.push(span.source);
        }
    }
    let mut owner_tokens = std::collections::BTreeSet::new();
    for block in &paragraphs {
        let open = source_stack(block)?;
        let preserve_empty = range.start == 0
            && range.end == document.projection().text_tree().byte_len()
            && block.id == first.id
            && !has_list;
        for name in ["paragraph", "li"] {
            if (name == "paragraph" && preserve_empty)
                || (name == "li" && !owners.contains(&block.id))
            {
                continue;
            }
            if let Some(opening)=open.iter().rev().find(|token|matches!(&token.kind,TokenKind::Tag(tag) if if name=="li"{tag.name=="li"}else{paragraph(&tag.name)})) {
                if !owner_tokens.insert(opening.range.start){continue;}
                let TokenKind::Tag(original)=&opening.kind else {unreachable!()};
                patches.push(converter.source_range(opening.range.clone()));
                let mut depth=0usize;
                for token in tokens.iter().filter(|token|token.range.start>=opening.range.end) {
                    let TokenKind::Tag(tag)=&token.kind else {continue};
                    if tag.name==original.name {
                        if tag.end {if depth==0{patches.push(converter.source_range(token.range.clone()));break;}depth-=1;}else{depth+=1;}
                    }
                    if name=="paragraph"&&structural(&tag.name) {break;}
                }
            }
        }
    }
    patches.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for patch in patches {
        if let Some(last) = merged.last_mut().filter(|last| patch.start <= last.end) {
            last.end = last.end.max(patch.end);
        } else {
            merged.push(patch);
        }
    }
    let mut result = merged
        .into_iter()
        .map(|range| (range, String::new()))
        .collect::<Vec<_>>();
    for (id, ordinal) in preserve_ordinals {
        let block = document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.id == id)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let open = source_stack(block)?;
        let opening = open
            .iter()
            .rev()
            .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if tag.name=="li"))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let at = opening.range.start + 3;
        result.push((
            converter.source_range(at..at),
            format!(" value=\"{ordinal}\""),
        ));
    }
    result.sort_by_key(|(range, _)| (range.start, range.end));
    Ok(Some(result))
}

/// A hard-line deletion inside a paragraph removes its text and its native
/// break, retaining the paragraph/list owner. At a paragraph's final hard line,
/// the preceding intra-paragraph break is the delimiter to remove: the following
/// synthetic separator belongs to the surviving adjacent paragraph.
fn partial_paragraph_line_deletion(
    document: &super::Document,
    range: &Range<usize>,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    let Some(range) = partial_paragraph_line_range(document, range) else {
        return Ok(None);
    };
    Ok(Some(
        super::rich_text::text_source_runs(document, &range)?
            .into_iter()
            .map(|source| (source, String::new()))
            .collect(),
    ))
}

/// Name the actual hard-break item removed by a line deletion. Commands still
/// capture their register from the original line extent; the transaction and
/// its position map retain the surviving paragraph boundary's identity.
pub(super) fn line_deletion_range(document: &super::Document, range: &Range<usize>) -> Range<usize> {
    partial_paragraph_line_range(document, range).unwrap_or_else(|| range.clone())
}

fn partial_paragraph_line_range(
    document: &super::Document,
    range: &Range<usize>,
) -> Option<Range<usize>> {
    let projection = document.projection();
    let Some(block) = projection
        .blocks_for_region(range)
        .into_iter()
        .find(|block| {
            block.range.start <= range.start
                && range.start <= block.range.end
                && range.end <= block.range.end.saturating_add(1)
        })
    else {
        return None;
    };
    let ends_in_break = block.range.end > block.range.start
        && projection
            .hard_breaks_for_region(&(block.range.end - 1..block.range.end))
            .contains(&(block.range.end - 1));
    let partial = range.start > block.range.start
        || range.end < block.range.end
        || (range.end == block.range.end && ends_in_break);
    if !partial {
        return None;
    }
    let end = range.end.min(block.range.end);
    let start = if range.end > block.range.end {
        let preceding = range.start.saturating_sub(1);
        if preceding < block.range.start
            || !projection
                .hard_breaks_for_region(&(preceding..range.start))
                .contains(&preceding)
        {
            return None;
        }
        preceding
    } else {
        range.start
    };
    if start >= end {
        return None;
    }
    Some(start..end)
}
