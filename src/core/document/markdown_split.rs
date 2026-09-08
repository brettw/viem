//! Keep inline delimiter scopes valid when a formatted hard break splits them.
use super::*;

struct Scope {
    formatted: Range<usize>,
    opening: Range<usize>,
    closing: Range<usize>,
    marker: String,
}

pub(super) fn patches(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
    if document.format() != Format::Markdown
        || !replacement.contains('\n')
        || !document
            .projection()
            .hard_breaks_for_region(range)
            .is_empty()
    {
        return Ok(None);
    }
    let projection = document.projection();
    let Some(line) = projection
        .hard_line_at_offset(range.start)
        .and_then(|index| projection.hard_line_range(index))
    else {
        return Ok(None);
    };
    if range.end > line.end {
        return Ok(None);
    }
    let mut pending = projection
        .style_spans_for_region(
            &(range.start.saturating_sub(1)
                ..range
                    .end
                    .saturating_add(1)
                    .min(projection.text_tree().byte_len())),
        )
        .into_iter()
        .filter_map(|span| {
            let StyleApplication::Semantic(style) = span.application else {
                return None;
            };
            Some((span.range, style))
        })
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < pending.len() {
        let extent = pending[index].0.clone();
        for at in [extent.start, extent.end] {
            for span in projection.style_spans_for_region(
                &(at.saturating_sub(1)
                    ..at.saturating_add(1).min(projection.text_tree().byte_len())),
            ) {
                let StyleApplication::Semantic(style) = span.application else {
                    continue;
                };
                if extent.start <= span.range.start
                    && span.range.end <= extent.end
                    && !pending
                        .iter()
                        .any(|(range, old_style)| range == &span.range && *old_style == style)
                {
                    pending.push((span.range, style));
                }
            }
        }
        index += 1;
    }
    let mut scopes: Vec<Scope> = Vec::new();
    // Inner scopes can share a visible endpoint with outer scopes. Resolve the
    // adjacent delimiters first, then step over them to recover outer spelling.
    while !pending.is_empty() {
        let mut progress = false;
        for index in (0..pending.len()).rev() {
            let (formatted, style) = &pending[index];
            let Some(source) = projection.source_range(formatted.clone()) else {
                continue;
            };
            let mut opening_end = source.start;
            let mut closing_start = source.end;
            loop {
                let Some(inner) = scopes.iter().find(|scope| {
                    scope.opening.end == opening_end && scope.formatted.start == formatted.start
                }) else {
                    break;
                };
                opening_end = inner.opening.start;
            }
            loop {
                let Some(inner) = scopes.iter().find(|scope| {
                    scope.closing.start == closing_start && scope.formatted.end == formatted.end
                }) else {
                    break;
                };
                closing_start = inner.closing.end;
            }
            let mut matched = None;
            if *style == SemanticInlineStyle::Code {
                if let Some((opening, closing)) =
                    super::super::markdown_code::delimiter_ranges(document, &source)?
                {
                    let bytes = document
                        .state()
                        .source
                        .bytes_in(opening.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let marker = document
                        .encoding()
                        .decode_region(&bytes, opening.start)?
                        .text;
                    matched = Some((opening, closing, marker));
                }
            } else {
                for marker in markdown_style_markers(*style) {
                    let bytes = document.encoding().encode_fragment(marker)?;
                    let Some(start) = opening_end.checked_sub(bytes.len()) else {
                        continue;
                    };
                    let opening = start..opening_end;
                    let closing = closing_start..closing_start + bytes.len();
                    if document.state().source.bytes_in(opening.clone()).as_ref() == Some(&bytes)
                        && document.state().source.bytes_in(closing.clone()).as_ref()
                            == Some(&bytes)
                    {
                        matched = Some((opening, closing, (*marker).to_owned()));
                        break;
                    }
                }
            }
            if let Some((opening, closing, marker)) = matched {
                scopes.push(Scope {
                    formatted: formatted.clone(),
                    opening,
                    closing,
                    marker,
                });
                pending.remove(index);
                progress = true;
            }
        }
        if !progress {
            return Ok(None);
        }
    }
    let mut source = if range.is_empty() {
        let Some(at) = projection.source_insertion_point(range.start, true) else {
            return Ok(None);
        };
        at..at
    } else {
        let Some(source) = projection.source_range(range.clone()) else {
            return Ok(None);
        };
        source
    };
    scopes.retain(|scope| scope.opening.end <= source.end && source.start <= scope.closing.start);
    if scopes.is_empty() {
        return Ok(None);
    }
    scopes.sort_by_key(|scope| (scope.opening.start, std::cmp::Reverse(scope.closing.end)));
    let original_source = source.clone();
    for scope in &scopes {
        if range.start <= scope.formatted.start && scope.opening.end <= original_source.end {
            source.start = source.start.min(scope.opening.start);
        }
        if scope.formatted.end <= range.end && original_source.start <= scope.closing.start {
            source.end = source.end.max(scope.closing.end);
        }
    }
    let active = |at: usize| {
        scopes
            .iter()
            .enumerate()
            .filter_map(|(index, scope)| {
                (scope.opening.end <= at && at <= scope.closing.start).then_some(index)
            })
            .collect::<Vec<_>>()
    };
    let mut current = active(source.start);
    let right = active(source.end);
    let inherited = scopes
        .iter()
        .enumerate()
        .filter_map(|(index, scope)| {
            (scope.formatted.start <= range.start && range.start < scope.formatted.end)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let mut syntax = String::new();
    let mut supporting = Vec::new();
    let mut line_start = false;
    for (index, segment) in replacement.split('\n').enumerate() {
        if index > 0 {
            transition(&scopes, &mut current, &[], &mut syntax);
            syntax.push_str("\n\n");
            line_start = true;
        }
        if !segment.is_empty() {
            if line_start {
                avoid_list_prefix(
                    document,
                    &mut scopes,
                    &inherited,
                    segment,
                    source.end,
                    &mut supporting,
                )?;
            }
            transition(&scopes, &mut current, &inherited, &mut syntax);
            let in_code = inherited
                .iter()
                .any(|&index| scopes[index].marker.starts_with('`'));
            if in_code && segment.contains('`') {
                return Ok(None);
            }
            if in_code {
                syntax.push_str(segment);
            } else {
                syntax.push_str(&document.escape_markdown_source_text(source.start, segment)?);
            }
            line_start = false;
        }
    }
    if line_start && range.end < line.end {
        let tail = projection
            .text_tree()
            .slice(range.end..(range.end + 1).min(line.end))
            .unwrap_or_default();
        avoid_list_prefix(
            document,
            &mut scopes,
            &right,
            &tail,
            source.end,
            &mut supporting,
        )?;
    }
    transition(&scopes, &mut current, &right, &mut syntax);
    let syntax = spell_logical_breaks(&syntax, document.file_format());
    supporting.push(SourcePatch::primary(
        source,
        document.encoding().encode_fragment(&syntax)?,
    ));
    Ok(Some(supporting))
}

// A reopened `*` followed by whitespace at physical line start is a list
// label. Its alternate spelling keeps the inline role, including in malformed
// input where a second adjacent `*` was previously literal visible content.
fn avoid_list_prefix(
    document: &Document,
    scopes: &mut [Scope],
    active: &[usize],
    text: &str,
    source_end: usize,
    patches: &mut Vec<SourcePatch>,
) -> Result<(), DocumentError> {
    let [index] = active else {
        return Ok(());
    };
    let scope = &mut scopes[*index];
    if scope.marker == "*" && text.starts_with([' ', '\t']) {
        scope.marker = "_".to_owned();
        if source_end <= scope.closing.start {
            patches.push(SourcePatch::primary(
                scope.closing.clone(),
                document.encoding().encode_fragment("_")?,
            ));
        }
    }
    Ok(())
}

fn transition(scopes: &[Scope], current: &mut Vec<usize>, next: &[usize], syntax: &mut String) {
    let shared = current
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    for &index in current[shared..].iter().rev() {
        syntax.push_str(&scopes[index].marker);
    }
    for &index in &next[shared..] {
        syntax.push_str(&scopes[index].marker);
    }
    current.clear();
    current.extend_from_slice(next);
}
