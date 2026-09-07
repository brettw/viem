//! Source-local structural exits for pending Markdown emphasis changes.
use super::*;

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
    if !matches!(document.format(), Format::Markdown | Format::MarkdownSource)
        || text.is_empty()
        || text.contains('\n')
        || desired.bold != Some(false) && desired.slant != Some(crate::document::FontSlant::Upright)
    {
        return Ok(None);
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
    let escaped = if document.format() == Format::MarkdownSource {
        text.to_owned()
    } else {
        escape_markdown_insert(text)
    };
    let source = projection
        .source_insertion_point(point, true)
        .ok_or(DocumentError::AmbiguousProjection)?;
    Ok(Some(super::super::html_typing::Insertion {
        source: source..source,
        source_caret: point + prefix.len() + text.len(),
        syntax: format!("{prefix}{escaped}{suffix}"),
    }))
}
