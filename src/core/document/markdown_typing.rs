//! Source-local inline and paragraph scopes for replacement typing.
use super::*;

pub(super) fn replacement_insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    inherited: &super::super::ReplacementTypingContext,
) -> Result<Option<super::super::html_typing::Insertion>, DocumentError> {
    if document.format() != Format::Markdown || text.is_empty() {
        return Ok(None);
    }
    if inherited.paragraph.style.0 == "Code Block" && document.projection().text_tree().byte_len() == 0 {
        // Whole-document replacement cleared the original fence. Recreate
        // its paragraph role before inserting literal code, including syntax
        // that would otherwise be escaped as ordinary Markdown prose.
        let width = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0).saturating_add(1).max(3);
        let fence = "`".repeat(width);
        let newline = document.file_format().spelling();
        let source = document.source_byte_len();
        let body = text.replace('\n', newline);
        let prefix = format!("{fence}{newline}");
        return Ok(Some(super::super::html_typing::Insertion {
            source: source..source,
            source_caret: source + prefix.len() + body.len(),
            syntax: format!("{prefix}{body}{newline}{fence}"),
        }));
    }
    let Some(destination) = &inherited.link else { return Ok(None) };
    let source = document.projection().source_insertion_point(
        at, affinity == BoundaryAffinity::Downstream && at < document.text().len(),
    ).ok_or(DocumentError::AmbiguousProjection)?;
    let current = if affinity == BoundaryAffinity::Upstream && at > 0 {
        document.hard_line_snapshot().previous_grapheme_boundary(at).unwrap_or(at)
    } else { at };
    if current < document.text().len()
        && document.link_at(document.text_point(current)?)?.as_ref() == Some(destination)
    {
        return Ok(None);
    }
    let destination = destination.replace('&', "&amp;").replace('<', "&lt;")
        .replace('>', "&gt;").replace('\\', "&#92;").replace('\n', "&#10;").replace('\r', "&#13;");
    let destination = super::super::projection::markdown_character_references(&destination, document.encoding());
    let escaped = escape_markdown_insert_in_encoding(text, document.encoding());
    Ok(Some(super::super::html_typing::Insertion {
        source: source..source,
        source_caret: source + 1 + escaped.len(),
        syntax: format!("[{escaped}](<{destination}>)"),
    }))
}

struct Scope {
    range: Range<usize>,
    marker: String,
    bold: bool,
    italic: bool,
}
fn scopes(projection: &FormattedDocument, at: usize) -> Result<Vec<Scope>, DocumentError> {
    let text = projection.text_tree();
    let spans = projection
        .style_spans_for_region(&(at.saturating_sub(1)..at.saturating_add(1).min(text.byte_len())));
    let mut result: Vec<Scope> = Vec::new();
    for span in &spans {
        let StyleApplication::Semantic(
            style @ (SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis),
        ) = span.application
        else {
            continue;
        };
        if result.iter().any(|scope| scope.range == span.range) {
            continue;
        }
        let bold = spans.iter().any(|s| {
            s.range == span.range
                && s.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)
        });
        let italic = spans.iter().any(|s| {
            s.range == span.range
                && s.application == StyleApplication::Semantic(SemanticInlineStyle::Emphasis)
        });
        let width = if bold && italic {
            3
        } else if style == SemanticInlineStyle::Strong {
            2
        } else {
            1
        };
        if span.range.len() < width * 2
            || at < span.range.start + width
            || at > span.range.end - width
        {
            continue;
        }
        let marker = text
            .slice(span.range.start..span.range.start + width)
            .map_err(DocumentError::FormattedTextStorage)?;
        if (!marker.bytes().all(|b| b == b'*') && !marker.bytes().all(|b| b == b'_'))
            || text
                .slice(span.range.end - width..span.range.end)
                .map_err(DocumentError::FormattedTextStorage)?
                != marker
        {
            continue;
        }
        result.push(Scope {
            range: span.range.clone(),
            marker,
            bold,
            italic,
        });
    }
    result.sort_by_key(|scope| (scope.range.start, std::cmp::Reverse(scope.range.end)));
    Ok(result)
}

pub(super) fn insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    desired: &CharacterProperties,
) -> Result<Option<super::super::html_typing::Insertion>, DocumentError> {
    if !document.format().is_markdown()
        || text.is_empty()
        || document.projection().text_tree().byte_len() == 0
        || text.contains('\n')
        || desired.bold != Some(false) && desired.slant != Some(crate::document::FontSlant::Upright)
    {
        return Ok(None);
    }
    if document.format() == Format::MarkdownSource && text.chars().next().is_some_and(char::is_alphanumeric) {
        let span = document.projection().style_spans_for_region(&(at.saturating_sub(1)..at)).into_iter()
            .filter(|span| span.range.end == at && matches!(span.application, StyleApplication::Semantic(SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis)))
            .min_by_key(|span| span.range.start);
        if let Some(span) = span {
            let body = document.projection().text_tree().slice(span.range.clone()).map_err(DocumentError::FormattedTextStorage)?;
            let prefix = body.bytes().take_while(|b| matches!(b, b'*' | b'_')).count();
            let suffix = body.bytes().rev().take_while(|b| matches!(b, b'*' | b'_')).count();
            if prefix > 0 && prefix == suffix && body[..prefix].contains('_') {
                let source = document.projection().source_range(span.range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
                let marker = "*".repeat(prefix);
                return Ok(Some(super::super::html_typing::Insertion { source, source_caret: at + text.len(),
                    syntax: format!("{marker}{}{marker}{text}", &body[prefix..body.len() - suffix]) }));
            }
        }
    }
    let local;
    let (projection, local_at) = if document.format() == Format::MarkdownSource {
        (document.projection(), at)
    } else {
        let source_at = document
            .projection()
            .source_insertion_point(
                at,
                affinity == BoundaryAffinity::Downstream
                    && at < document.projection().text_tree().byte_len(),
            )
            .ok_or(DocumentError::AmbiguousProjection)?;
        let index = document
            .state()
            .source_hard_lines
            .line_at_offset(source_at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let source_line = document
            .state()
            .source_hard_lines
            .get(index)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = document
            .state()
            .source
            .bytes_in(source_line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document
            .encoding()
            .decode_region(&bytes, source_line.start)?;
        let normalized = normalize(&decoded, document.file_format());
        let local_at = normalized
            .units
            .iter()
            .find(|unit| unit.source.start == source_at)
            .map(|unit| unit.normalized.start)
            .or_else(|| {
                normalized
                    .units
                    .last()
                    .filter(|unit| unit.source.end == source_at)
                    .map(|unit| unit.normalized.end)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        local = project(
            &normalized,
            Format::MarkdownSource,
            document.revision(),
            source_line.start,
            source_line.end,
        );
        (&local, local_at)
    };
    let scopes = scopes(projection, local_at)?;
    let Some(first) = scopes.iter().position(|scope| {
        scope.bold && desired.bold == Some(false)
            || scope.italic && desired.slant == Some(crate::document::FontSlant::Upright)
    }) else {
        return Ok(None);
    };
    let active = &scopes[first..];
    let mut exit = local_at;
    let exact_exit = active.iter().rev().all(|scope| {
        if scope.range.end - scope.marker.len() != exit {
            return false;
        }
        exit = scope.range.end;
        true
    });
    let bold = desired.bold != Some(false) && active.iter().any(|scope| scope.bold);
    let italic = desired.slant != Some(crate::document::FontSlant::Upright)
        && active.iter().any(|scope| scope.italic);
    // A middle split can put three adjacent marker runs next to one another.
    // Use the other legal spelling for the retained role so those runs cannot
    // be mistaken for a combined-emphasis delimiter.
    let alternate = !exact_exit && active[0].marker.starts_with('*');
    let preserved = match (bold, italic, alternate) {
        (true, true, true) => "___",
        (true, false, true) => "__",
        (false, true, true) => "_",
        (true, true, false) => "***",
        (true, false, false) => "**",
        (false, true, false) => "*",
        _ => "",
    };
    let mut prefix = String::new();
    let mut suffix = preserved.to_owned();
    let point = if exact_exit {
        exit
    } else {
        for scope in active.iter().rev() {
            prefix.push_str(&scope.marker);
        }
        for scope in active {
            suffix.push_str(&scope.marker);
        }
        local_at
    };
    prefix.push_str(preserved);
    let mut escaped = if document.format() == Format::MarkdownSource {
        text.to_owned()
    } else {
        escape_markdown_insert_in_encoding(text, document.encoding())
    };
    if active.iter().any(|scope| scope.marker.starts_with('_')) || preserved.starts_with('_') {
        let mut characters = escaped.chars();
        if let Some(first) = characters.next() {
            let rest = characters.as_str();
            let mut body = format!("&#{};", first as u32);
            if let Some((last_at, last)) = rest.char_indices().next_back() {
                body.push_str(&rest[..last_at]); body.push_str(&format!("&#{};", last as u32));
            }
            escaped = body;
        }
    }
    let source = projection
        .source_insertion_point(point, true)
        .ok_or(DocumentError::AmbiguousProjection)?;
    Ok(Some(super::super::html_typing::Insertion {
        source: source..source,
        source_caret: point + prefix.len() + escaped.len(),
        syntax: format!("{prefix}{escaped}{suffix}"),
    }))
}
