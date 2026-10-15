//! Source-local inline and paragraph scopes for replacement typing.
use super::*;

pub(super) fn replacement_insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    inherited: &super::super::ReplacementTypingContext,
) -> Result<Option<super::super::replacement_context::Insertion>, DocumentError> {
    if document.format() != Format::Markdown || text.is_empty() {
        return Ok(None);
    }
    if document.projection().text_tree().byte_len() == 0 && !text.contains('\n') {
        if let Some(prefix) = inherited.paragraph.markdown_prefix.as_ref() {
            let current = document.projection().blocks_for_region(&(at..at));
            let retained_owner = current.iter().any(|block| block.quote_depth > 0
                || matches!(block.kind, super::super::BlockKind::ListItem { .. } | super::super::BlockKind::Heading(_)));
            if !retained_owner {
                // A whole-document replacement removed its original owner.
                // Restore its ordered container prefix before inserting the
                // body, preserving list/quote nesting and heading treatment.
                let source = document.projection().source_insertion_point(at, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let mut body = document.escape_markdown_source_text(source, text)?;
                if inherited.paragraph.style.0.starts_with("Heading") {
                    let end = body.trim_end_matches([' ', '\t']).len();
                    let tail = body[end..].chars().map(|ch| if ch == ' ' { "&#32;" } else { "&#9;" }).collect::<String>();
                    body.truncate(end); body.push_str(&tail);
                }
                let syntax = format!("{prefix}{body}");
                return Ok(Some(super::super::replacement_context::Insertion {
                    source: source..source,
                    syntax,
                }));
            }
        }
    }
    if inherited.paragraph.style.0 == "Code Block"
        && document.projection().text_tree().byte_len() == 0
        && !document.projection().blocks_for_region(&(at..at)).iter()
            .any(|block| block.style.0 == "Code Block")
    {
        // Whole-document replacement cleared the original fence. Recreate
        // its paragraph role before inserting literal code, including syntax
        // that would otherwise be escaped as ordinary Markdown prose.
        let width = text
            .split(|ch| ch != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(3);
        let fence = "`".repeat(width);
        let newline = document.file_format().spelling();
        let source = document.source_byte_len();
        let body = text.replace('\n', newline);
        let language = inherited.paragraph.code_language.as_deref().unwrap_or("");
        let prefix = format!("{fence}{language}{newline}");
        return Ok(Some(super::super::replacement_context::Insertion {
            source: source..source,
            syntax: format!("{prefix}{body}{newline}{fence}")
                .split(newline)
                .map(|line| format!("{}{line}", "> ".repeat(inherited.paragraph.quote_depth)))
                .collect::<Vec<_>>()
                .join(newline),
        }));
    }
    if inherited.paragraph.quote_depth > 0 && document.projection().text_tree().byte_len() == 0
        && !document.projection().blocks_for_region(&(at..at)).iter()
            .any(|block| block.quote_depth >= inherited.paragraph.quote_depth)
    {
        let source = document.source_byte_len();
        let escaped = escape_markdown_insert_in_encoding(text, document.encoding());
        let prefix = "> ".repeat(inherited.paragraph.quote_depth);
        let syntax = escaped
            .split('\n')
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join(document.file_format().spelling());
        return Ok(Some(super::super::replacement_context::Insertion {
            source: source..source,
            syntax,
        }));
    }
    let Some(destination) = &inherited.link else {
        return Ok(None);
    };
    let source = document
        .projection()
        .source_insertion_point(
            at,
            affinity == BoundaryAffinity::Downstream && at < document.text().len(),
        )
        .ok_or(DocumentError::AmbiguousProjection)?;
    let current = if affinity == BoundaryAffinity::Upstream && at > 0 {
        document
            .hard_line_snapshot()
            .previous_grapheme_boundary(at)
            .unwrap_or(at)
    } else {
        at
    };
    if current < document.text().len()
        && document.link_at(document.text_point(current)?)?.as_ref() == Some(destination)
    {
        return Ok(None);
    }
    let destination = destination
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "&#92;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;");
    let destination =
        super::super::projection::markdown_character_references(&destination, document.encoding());
    let escaped = escape_markdown_insert_in_encoding(text, document.encoding());
    Ok(Some(super::super::replacement_context::Insertion {
        source: source..source,
        syntax: format!("[{escaped}](<{destination}>)"),
    }))
}

struct Scope {
    range: Range<usize>,
    marker: String,
    bold: bool,
    italic: bool,
    strike: bool,
}
/// Emphasis, strong and strikethrough scopes in one physical source line
/// whose content contains `at`, outermost first.
fn scopes(text: &str, at: usize) -> Vec<Scope> {
    use super::super::markdown_syntax::InlineKind;
    let mut result: Vec<Scope> = super::super::markdown_syntax::source_inlines(text)
        .into_values()
        .filter_map(|inline| {
            let (bold, italic, strike) = match inline.kind {
                InlineKind::Strong => (true, false, false),
                InlineKind::Emphasis => (false, true, false),
                InlineKind::Strike => (false, false, true),
                _ => return None,
            };
            if at < inline.inner.start || at > inline.inner.end {
                return None;
            }
            let marker = &text[inline.range.start..inline.inner.start];
            let uniform = |byte| marker.bytes().all(|b| b == byte);
            let valid = !marker.is_empty()
                && (uniform(b'*') || uniform(b'_') || strike && uniform(b'~'))
                && text[inline.inner.end..inline.range.end] == *marker;
            valid.then(|| Scope { range: inline.range, marker: marker.to_owned(), bold, italic, strike })
        })
        .collect();
    result.sort_by_key(|scope| (scope.range.start, std::cmp::Reverse(scope.range.end)));
    result
}

pub(super) fn insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    desired: &CharacterProperties,
) -> Result<Option<super::super::replacement_context::Insertion>, DocumentError> {
    if !document.format().is_markdown()
        || text.is_empty()
        || document.projection().text_tree().byte_len() == 0
        || text.contains('\n')
        || desired.bold != Some(false) && desired.slant != Some(crate::document::FontSlant::Upright) && desired.strikethrough != Some(false)
    {
        return Ok(None);
    }
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
        .unit_with_source_start(source_at)
        .map(|unit| unit.normalized.start)
        .or_else(|| {
            normalized
                .last_unit()
                .filter(|unit| unit.source.end == source_at)
                .map(|unit| unit.normalized.end)
        })
        // A final empty physical row has no scalar units, but its body
        // start is still an exact caret location with no inline scopes.
        .or_else(|| (!normalized.has_units() && source_at == source_line.start).then_some(0))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let scopes = scopes(&normalized.text, local_at);
    let Some(first) = scopes.iter().position(|scope| {
        scope.bold && desired.bold == Some(false)
            || scope.italic && desired.slant == Some(crate::document::FontSlant::Upright)
            || scope.strike && desired.strikethrough == Some(false)
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
    let strike = desired.strikethrough != Some(false) && active.iter().any(|scope| scope.strike);
    let preserved_open = format!("{}{preserved}", if strike { "~~" } else { "" });
    let preserved_close = format!("{preserved}{}", if strike { "~~" } else { "" });
    let mut prefix = String::new();
    let mut suffix = preserved_close;
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
    prefix.push_str(&preserved_open);
    let mut escaped = escape_markdown_insert_in_encoding(text, document.encoding());
    if active.iter().any(|scope| scope.marker.starts_with('_')) || preserved.starts_with('_') {
        let mut characters = escaped.chars();
        if let Some(first) = characters.next() {
            let rest = characters.as_str();
            let mut body = format!("&#{};", first as u32);
            if let Some((last_at, last)) = rest.char_indices().next_back() {
                body.push_str(&rest[..last_at]);
                body.push_str(&format!("&#{};", last as u32));
            }
            escaped = body;
        }
    }
    let source = normalized.source_at(point);
    Ok(Some(super::super::replacement_context::Insertion {
        source: source..source,
        syntax: format!("{prefix}{escaped}{suffix}"),
    }))
}
