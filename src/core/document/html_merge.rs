//! Removing a paragraph separator keeps the preceding structural owner.
//! Splice the two visible boundaries, then restore the following paragraph's
//! original context. Text outside the edit and opaque metadata stay verbatim.
use super::html::{self, Token, TokenKind};
use super::{BoundaryAffinity, Document, DocumentError, TextEdit};
use std::ops::Range;

/// HTML recovery can place a table's source children before the table in the
/// logical paragraph. Structural splices need that same order in source too.
/// Move only those contributors (and their original inline scopes), retaining
/// every byte of the atomic owner. The transaction verifies this supporting
/// materialization before applying the user's original logical edit.
pub(super) fn recovered_source_patches(
    document: &Document,
    edits: &[TextEdit],
) -> Result<Vec<super::SourcePatch>, DocumentError> {
    let mut blocks = Vec::new();
    for edit in edits {
        if edit.range.is_empty() {
            continue;
        }
        let merged = super::edit_boundary::merged_paragraphs(document, &edit.range);
        if merged.is_empty() {
            blocks.extend(
                document
                    .projection()
                    .blocks_for_region(&edit.range)
                    .into_iter()
                    .filter(|block| {
                        edit.range.start <= block.range.start && block.range.end <= edit.range.end
                    }),
            );
        } else {
            blocks.extend(merged);
        }
    }
    blocks.sort_by_key(|block| (block.range.start, block.range.end));
    blocks.dedup_by_key(|block| (block.range.start, block.range.end));
    let mut patches = Vec::new();
    for block in blocks {
        let spans = document.projection().provenance_for_region(&block.range);
        let mut moved = Vec::new();
        for span in &spans {
            if document
                .projection()
                .text_tree()
                .slice(span.formatted.clone())
                .map_err(DocumentError::FormattedTextStorage)?
                != "\u{fffc}"
            {
                continue;
            }
            let Some(plan) = super::source_edit::overlapping_text_plan(document, &span.formatted)?
            else {
                continue;
            };
            let bytes = document.source_bytes();
            let mut retained = Vec::new();
            let mut owner = Vec::new();
            let mut at = span.source.start;
            for range in plan.ranges {
                if at < range.start {
                    let (open, close) = super::html_typing::recovered_inline_wrapper(
                        document,
                        span.source.clone(),
                        at..range.start,
                    )?;
                    retained.extend(document.encoding().encode_fragment(&open)?);
                    retained.extend_from_slice(&bytes[at..range.start]);
                    retained.extend(document.encoding().encode_fragment(&close)?);
                }
                owner.extend_from_slice(&bytes[range.clone()]);
                at = range.end;
            }
            retained.extend_from_slice(&bytes[at..span.source.end]);
            if retained.is_empty() {
                continue;
            }
            retained.extend(owner);
            moved.push(super::SourcePatch::primary(span.source.clone(), retained));
        }
        if moved.is_empty() {
            continue;
        }
        let bytes = document.source_bytes();
        moved.sort_by_key(|patch| patch.range().start);
        let start = spans
            .iter()
            .filter(|span| !span.source.is_empty())
            .map(|span| span.source.start)
            .min()
            .ok_or(DocumentError::AmbiguousProjection)?;
        let end = spans
            .iter()
            .map(|span| span.source.end)
            .max()
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut replacement = Vec::new();
        let mut at = start;
        for patch in moved {
            replacement.extend_from_slice(&bytes[at..patch.range().start]);
            replacement.extend_from_slice(patch.replacement());
            at = patch.range().end;
        }
        replacement.extend_from_slice(&bytes[at..end]);
        // An anonymous paragraph still exists when all its content is erased.
        // An explicit owner supplies that recoverable empty boundary.
        let input = super::line_endings::normalize(
            &document.encoding().decode(&bytes)?,
            document.file_format(),
        );
        let normalized = input
            .units
            .get(input.units.partition_point(|unit| unit.source.end <= start))
            .map_or(input.text.len(), |unit| unit.normalized.start);
        let tokens = html::tokenize(&input.text);
        let owned = super::html_paragraph::stack_at(&tokens, normalized).iter().any(|token| {
            matches!(&token.kind, TokenKind::Tag(tag) if html::owns_paragraph(tag, document.projection().style_sheet()))
        });
        if !owned {
            let class = super::html_styles::class_name(&block.style, false);
            let mut wrapped = document
                .encoding()
                .encode_fragment(&format!("<div class=\"{class}\">"))?;
            wrapped.extend(replacement);
            wrapped.extend(document.encoding().encode_fragment("</div>")?);
            replacement = wrapped;
        }
        patches.push(super::SourcePatch::primary(start..end, replacement));
    }
    Ok(patches)
}

/// Anonymous HTML paragraphs otherwise coalesce a terminal <br> with their
/// container separator. Give only the edited paragraph an explicit owner so
/// the inserted hard break and the existing paragraph boundary stay distinct.
pub(super) fn anonymous_break_patches(
    document: &Document,
    edit: &TextEdit,
) -> Result<Option<Vec<super::SourcePatch>>, DocumentError> {
    let Some(block) = super::edit_boundary::paragraph_at(document, edit.range.start)? else {
        return Ok(None);
    };
    let touches_separator = edit.range.start == block.range.start && block.range.start > 0
        || edit.range.start == block.range.end
            && block.range.end < document.projection().text_tree().byte_len();
    if !touches_separator {
        return Ok(None);
    }
    let input = super::line_endings::normalize(
        &document.encoding().decode(&document.source_bytes())?,
        document.file_format(),
    );
    let at = super::source_edit::insertion_point(document.projection(), edit.range.start, None)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let normalized = input
        .units
        .get(input.units.partition_point(|unit| unit.source.end <= at))
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let tokens = html::tokenize(&input.text);
    if super::html_paragraph::stack_at(&tokens, normalized)
        .iter()
        .any(|token| {
            matches!(&token.kind, TokenKind::Tag(tag) if html::owns_paragraph(tag, document.projection().style_sheet()))
        })
    {
        return Ok(None);
    }
    let start = super::source_edit::insertion_point(
        document.projection(),
        block.range.start,
        Some(BoundaryAffinity::Downstream),
    )
    .ok_or(DocumentError::AmbiguousProjection)?;
    let end = super::source_edit::insertion_point(
        document.projection(),
        block.range.end,
        Some(BoundaryAffinity::Upstream),
    )
    .ok_or(DocumentError::AmbiguousProjection)?;
    let mut patches = super::source_edit::rich_text_patches(document, edit, None)?;
    for (at, tag, before) in [(start, "<p>", true), (end, "</p>", false)] {
        let bytes = document.encoding().encode_fragment(tag)?;
        if let Some(patch) = patches.iter_mut().find(|patch| patch.range() == (at..at)) {
            let mut replacement = if before {
                bytes.clone()
            } else {
                patch.replacement().to_vec()
            };
            replacement.extend(if before { patch.replacement() } else { &bytes });
            *patch = super::SourcePatch::primary(at..at, replacement);
        } else {
            patches.push(super::SourcePatch::primary(at..at, bytes));
        }
    }
    patches.sort_by_key(|patch| (patch.range().start, patch.range().end));
    Ok(Some(patches))
}

pub(super) fn patches(
    document: &Document,
    input: &super::line_endings::NormalizedText,
    edit: &TextEdit,
) -> Result<Option<Vec<super::SourcePatch>>, DocumentError> {
    let paragraphs = super::edit_boundary::merged_paragraphs(document, &edit.range);
    let Some(last) = paragraphs.last() else {
        return Ok(None);
    };
    let edit = super::source_edit::complete_contributors(document.projection(), edit)?;
    let source_at = |at, affinity| {
        super::source_edit::insertion_point(document.projection(), at, Some(affinity))
            .ok_or(DocumentError::AmbiguousProjection)
    };
    let normalized = |source| {
        let index = input
            .units
            .partition_point(|unit| unit.source.end <= source);
        input
            .units
            .get(index)
            .map_or(input.text.len(), |unit| unit.normalized.start)
    };
    let start_source = source_at(edit.range.start, BoundaryAffinity::Upstream)?;
    let start = normalized(start_source);
    let end_source = source_at(edit.range.end, BoundaryAffinity::Downstream)?;
    let end = normalized(end_source);
    let tail = normalized(source_at(last.range.end, BoundaryAffinity::Upstream)?);
    if start > end || end > tail {
        return Err(DocumentError::AmbiguousProjection);
    }
    let following_at = last.range.end.saturating_add(1);
    let following = document
        .projection()
        .blocks_for_region(&(following_at..following_at))
        .into_iter()
        .find(|block| block.range.start == following_at);
    let resume = following
        .as_ref()
        .map(|block| source_at(block.range.start, BoundaryAffinity::Downstream))
        .transpose()?
        .map_or(input.text.len(), normalized);
    if tail > resume {
        return Err(DocumentError::AmbiguousProjection);
    }
    let tokens = html::tokenize(&input.text);
    let left = super::html_paragraph::stack_at(&tokens, start);
    let right = super::html_paragraph::stack_at(&tokens, end);
    let right_tail = super::html_paragraph::stack_at(&tokens, tail);
    let following_stack = if following.is_some() {
        super::html_paragraph::stack_at(&tokens, resume)
    } else {
        Vec::new()
    };
    let owner_depth = |stack: &[&Token]| {
        stack.iter().rposition(|token| {
        matches!(&token.kind, TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name))
    }).map_or(0, |at| at + 1)
    };
    let left_depth = owner_depth(&left);
    let right_depth = owner_depth(&right);
    let tail_depth = owner_depth(&right_tail);
    let owner = &left[..left_depth];
    // A table is an atomic formatted item, but standards-mode HTML forbids it
    // inside p. Preserve the surviving paragraph's assignment on a flow-capable
    // div when its retained suffix contains that item.
    let flow_owner = owner.last().filter(|token| {
        matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "p")
            && document
                .projection()
                .provenance_for_region(&(edit.range.end..last.range.end))
                .iter()
                .any(|span| {
                    document
                        .projection()
                        .text_tree()
                        .slice(span.formatted.clone())
                        .ok()
                        .as_deref()
                        == Some("\u{fffc}")
                        && tokens.iter().any(|token| {
                            token.range.start == normalized(span.source.start)
                                && matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "table")
                        })
                })
    });
    let common = owner
        .iter()
        .zip(&following_stack)
        .take_while(|(left, right)| left.range.start == right.range.start)
        .count();
    let close = |stack: &[&Token]| -> String {
        stack
            .iter()
            .rev()
            .map(|token| {
                let TokenKind::Tag(tag) = &token.kind else {
                    unreachable!()
                };
                let name = if flow_owner.is_some_and(|owner| owner.range == token.range) {
                    "div"
                } else {
                    &tag.name
                };
                format!("</{name}>")
            })
            .collect()
    };
    let open = |stack: &[&Token]| -> String {
        stack
            .iter()
            .map(|token| input.text[token.range.clone()].to_owned())
            .collect()
    };
    let replacement = super::rich_text::escape_html_text_edit(document, start_source, &edit)?;
    // Direct character declarations belong to retained text, while the
    // paragraph assignment and paragraph geometry come from the left owner.
    let mut character = super::CharacterProperties::default();
    let retained_ancestors = owner
        .iter()
        .zip(&right[..right_depth])
        .take_while(|(left, right)| left.range.start == right.range.start)
        .count();
    // Removed containers can supply direct character context to the retained
    // suffix too. Overlay them in ancestry order, while common owners remain
    // in source and the new paragraph continues to use the left named style.
    for token in &right[retained_ancestors..right_depth] {
        let TokenKind::Tag(tag) = &token.kind else {
            unreachable!()
        };
        if let Some(css) = tag.attribute("style") {
            html::apply_css(css, &mut character, &mut Default::default());
        }
        if let Some(language) = tag.attribute("lang") {
            character.language = Some(language.to_owned());
        }
        if let Some(direction) = match tag.attribute("dir") {
            Some("rtl") => Some(super::WritingDirection::RightToLeft),
            Some("ltr") => Some(super::WritingDirection::LeftToRight),
            _ => None,
        } {
            character.direction = Some(direction);
        }
    }
    let (character_open, character_close) = if character == super::CharacterProperties::default() {
        (String::new(), String::new())
    } else {
        html::character_wrapper(&character)
    };
    let before_preserves = html::whitespace_after_source_gap(&open(&left), false);
    let after_preserves = html::whitespace_after_source_gap(&open(&right), false);
    let (whitespace_open, whitespace_close) = if before_preserves == after_preserves || end == tail
    {
        ("", "")
    } else if after_preserves {
        ("<span style=\"white-space: pre-wrap\">", "</span>")
    } else {
        ("<span style=\"white-space: normal\">", "</span>")
    };
    let empty_paragraph = edit.range.start == paragraphs[0].range.start
        && edit.range.end == last.range.end
        && edit.replacement.is_empty()
        && !owner.last().is_some_and(|token| {
            matches!(&token.kind, TokenKind::Tag(tag)
            if tag.name != "li" && html::owns_paragraph(tag, document.projection().style_sheet()))
        });
    let before_containers = unselected_empty_containers(document, input, &tokens, start..end);
    let after_containers = unselected_empty_containers(document, input, &tokens, tail..resume);
    let converter = super::rich_text::Builder::new(input, document.revision());
    let mut bridge = document.encoding().encode_fragment(&format!(
        "{replacement}{}", close(&left[left_depth..])))?;
    bridge.extend(retained_metadata(document, input, &tokens, start..end, &before_containers, false)?);
    bridge.extend(document.encoding().encode_fragment(&format!(
        "{}{whitespace_open}{character_open}{}",
        if empty_paragraph { "<p></p>" } else { "" }, open(&right[right_depth..])))?);
    let mut restore = document.encoding().encode_fragment(&format!(
        "{}{character_close}{whitespace_close}{}",
        close(&right_tail[tail_depth..]), close(&owner[common..])))?;
    for range in &before_containers {
        restore.extend(document.state().source.bytes_in(converter.source_range(range.clone()))
            .ok_or(DocumentError::AmbiguousProjection)?);
    }
    restore.extend(retained_metadata(document, input, &tokens, tail..resume, &after_containers, true)?);
    restore.extend(document.encoding().encode_fragment(&open(&following_stack[common..]))?);
    let mut patches = vec![
        super::SourcePatch::primary(converter.source_range(start..end), bridge),
        super::SourcePatch::primary(converter.source_range(tail..resume), restore),
    ];
    if let Some(token) = flow_owner {
        let TokenKind::Tag(tag) = &token.kind else {
            unreachable!()
        };
        let class = super::html_styles::class_name(&paragraphs[0].style, false);
        let classes = tag
            .attribute("class")
            .map_or(class.clone(), |existing| format!("{class} {existing}"));
        let opening = &input.text[token.range.clone()];
        let (range, replacement) =
            super::html_styles::class_patch(opening, 0..opening.len(), &classes);
        let mut opening = opening.to_owned();
        opening.replace_range(range, &replacement);
        opening.replace_range(1..2, "div");
        patches.push(super::SourcePatch::primary(converter.source_range(token.range.clone()),
            document.encoding().encode_fragment(&opening)?));
    }
    Ok(Some(patches))
}

fn retained_metadata(
    document: &Document,
    input: &super::line_endings::NormalizedText,
    tokens: &[Token],
    range: Range<usize>,
    containers: &[Range<usize>],
    include_containers: bool,
) -> Result<Vec<u8>, DocumentError> {
    let mut result = Vec::new();
    let converter = super::rich_text::Builder::new(input, document.revision());
    let mut hidden = Vec::new();
    for token in tokens {
        if token.range.end <= range.start || token.range.start >= range.end {
            continue;
        }
        let container = containers.partition_point(|range| range.end <= token.range.start);
        if containers.get(container).is_some_and(|range|
            range.start <= token.range.start && token.range.end <= range.end) {
            if include_containers {
                result.extend(document.state().source.bytes_in(converter.source_range(token.range.clone()))
                    .ok_or(DocumentError::AmbiguousProjection)?);
            }
            continue;
        }
        let keep = match &token.kind {
            TokenKind::Opaque => true,
            TokenKind::Tag(tag) => {
                let keep = !hidden.is_empty() || html::hidden(&tag.name);
                if tag.end {
                    if let Some(index) = hidden.iter().rposition(|name| name == &tag.name) {
                        hidden.truncate(index);
                    }
                } else if html::hidden(&tag.name) {
                    hidden.push(tag.name.clone());
                }
                keep
            }
            _ => !hidden.is_empty(),
        };
        if keep {
            let retained = token.range.start.max(range.start)..token.range.end.min(range.end);
            result.extend(document.state().source.bytes_in(converter.source_range(retained))
                .ok_or(DocumentError::AmbiguousProjection)?);
        }
    }
    Ok(result)
}

/// Transparent empty containers have no paragraph separator to select. A merge
/// must retain their exact bytes outside the surviving paragraph: placing a
/// block container inside a p would cause HTML recovery to split it again.
fn unselected_empty_containers(
    document: &Document,
    input: &super::line_endings::NormalizedText,
    tokens: &[Token],
    range: Range<usize>,
) -> Vec<Range<usize>> {
    let mut retained = Vec::new();
    let converter = super::rich_text::Builder::new(input, document.revision());
    let mut open: Vec<(&Token, usize)> = Vec::new();
    let mut content_owners = 0;
    let mut hidden: Option<(&str, usize)> = None;
    for token in tokens.iter().filter(|token|
        range.start <= token.range.start && token.range.end <= range.end) {
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
            // A template may contain paragraph, break, or object syntax;
            // none makes its otherwise empty container selected content.
            hidden = Some((&tag.name, 1));
            continue;
        }
        if !tag.end {
            if html::owns_paragraph(tag, document.projection().style_sheet())
                || tag.name == "blockquote" || html::atomic(&tag.name) || tag.name == "br" {
                content_owners += 1;
            }
            if !html::void(&tag.name) { open.push((token, content_owners)); }
            continue;
        }
        let Some(depth) = open.iter().rposition(|(opening, _)|
            matches!(&opening.kind, TokenKind::Tag(open_tag) if open_tag.name == tag.name))
            else { continue; };
        let (opening, previous_owners) = open[depth];
        open.truncate(depth);
        let TokenKind::Tag(open_tag) = &opening.kind else { unreachable!(); };
        if previous_owners != content_owners || !html::block(&open_tag.name)
            || open_tag.name == "blockquote" || html::list_element(&open_tag.name)
            || html::owns_paragraph(open_tag, document.projection().style_sheet()) {
            continue;
        }
        let extent = opening.range.start..token.range.end;
        // The source query deliberately ignores empty caret anchors: only
        // real visible contributors make the container part of the selection.
        if document.projection().provenance_contained_in_source(
            &converter.source_range(extent.clone())).iter().any(|span| !span.formatted.is_empty()) {
            continue;
        }
        // A retained outer scope already includes its untouched descendants.
        while retained.last().is_some_and(|previous: &Range<usize>|
            extent.start <= previous.start) { retained.pop(); }
        retained.push(extent);
    }
    retained
}
