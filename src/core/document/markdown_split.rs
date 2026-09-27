//! Keep inline delimiter scopes valid when edits split or join formatted lines.
use super::*;

struct Scope {
    formatted: Range<usize>,
    opening: Range<usize>,
    closing: Range<usize>,
    marker: String,
    closing_marker: String,
}

pub(super) fn patches(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
    patches_with_separator(document, range, replacement, "\n\n", None)
}

pub(super) fn hard_break_patches(
    document: &Document,
    at: usize,
    source_at: usize,
    separator: &str,
) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
    patches_with_separator(document, &(at..at), "\n", separator, Some(source_at))
}

pub(super) fn patches_with_separator(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
    separator: &str,
    insertion_source: Option<usize>,
) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
    if document.format() != Format::Markdown {
        return Ok(None);
    }
    let projection = document.projection();
    let crosses_break = !projection.hard_breaks_for_region(range).is_empty();
    if !replacement.contains('\n') && !crosses_break {
        return Ok(None);
    }
    let Some(mut line) = projection
        .hard_line_at_offset(range.start)
        .and_then(|index| projection.hard_line_range(index))
    else {
        return Ok(None);
    };
    if crosses_break {
        // An inline hard break belongs to the same paragraph on both sides.
        // Its source hull can include an emphasis/link closing delimiter;
        // perform the same scope transition as a split so retained text on
        // either side keeps its original inline context.
        if !projection
            .blocks_for_region(range)
            .iter()
            .any(|block| block.range.start <= range.start && range.end <= block.range.end)
        {
            return Ok(None);
        }
        let Some(last) = projection
            .hard_line_at_offset(range.end)
            .and_then(|index| projection.hard_line_range(index))
        else {
            return Ok(None);
        };
        line.end = last.end;
    } else if range.end > line.end {
        return Ok(None);
    }
    let Some(mut scopes) = inline_scopes(document, range, &line)? else {
        return Ok(None);
    };
    let mut source = if range.is_empty() {
        let Some(at) =
            insertion_source.or_else(|| projection.source_insertion_point(range.start, true))
        else {
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
            syntax.push_str(separator);
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

fn inline_scopes(
    document: &Document,
    range: &Range<usize>,
    line: &Range<usize>,
) -> Result<Option<Vec<Scope>>, ModelTransactionError> {
    let projection = document.projection();
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
    let mut scopes = link_scopes(document, &line)?;
    for span in projection.style_spans_for_region(line) {
        if span.application != StyleApplication::Automatic("Strikethrough".into()) {
            continue;
        }
        let Some(body) = projection.source_range(span.range.clone()) else {
            continue;
        };
        for marker in ["~~", "~"] {
            let bytes = document.encoding().encode_fragment(marker)?;
            let Some(start) = body.start.checked_sub(bytes.len()) else {
                continue;
            };
            let opening = start..body.start;
            let closing = body.end..body.end + bytes.len();
            if document.state().source.bytes_in(opening.clone()).as_ref() == Some(&bytes)
                && document.state().source.bytes_in(closing.clone()).as_ref() == Some(&bytes)
            {
                scopes.push(Scope {
                    formatted: span.range.clone(),
                    opening,
                    closing,
                    marker: marker.into(),
                    closing_marker: marker.into(),
                });
                break;
            }
        }
    }

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
            let mut html_closing = None;
            if matched.is_none() {
                let tags: &[(&str, &str)] = match style {
                    SemanticInlineStyle::Strong => &[("<strong>", "</strong>"), ("<b>", "</b>")],
                    SemanticInlineStyle::Emphasis => &[("<em>", "</em>"), ("<i>", "</i>")],
                    _ => &[],
                };
                for (open, close) in tags {
                    let a = document.encoding().encode_fragment(open)?;
                    let b = document.encoding().encode_fragment(close)?;
                    if let Some(start) = opening_end.checked_sub(a.len()) {
                        let opening = start..opening_end;
                        let closing = closing_start..closing_start + b.len();
                        if document.state().source.bytes_in(opening.clone()).as_ref() == Some(&a)
                            && document.state().source.bytes_in(closing.clone()).as_ref()
                                == Some(&b)
                        {
                            matched = Some((opening, closing, (*open).to_owned()));
                            html_closing = Some((*close).to_owned());
                            break;
                        }
                    }
                }
            }
            if let Some((opening, closing, marker)) = matched {
                scopes.push(Scope {
                    formatted: formatted.clone(),
                    opening,
                    closing,
                    closing_marker: html_closing.unwrap_or_else(|| marker.clone()),
                    marker,
                });
                pending.remove(index);
                progress = true;
            }
        }
        if !progress {
            // Fenced code and other block-owned styles have no inline pair.
            // Their structural adapter owns them; keep the resolved inline scopes.
            break;
        }
    }
    Ok(Some(scopes))
}

/// Once every byte of an emphasis body is consumed, its paired delimiters
/// have no content to style. Remove them in the same local source transaction
/// so a following typing event cannot combine them with its new delimiters.
pub(super) fn remove_empty_emphasis(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), ModelTransactionError> {
    for edit in edits.iter().filter(|edit| !edit.range.is_empty()) {
        let projection = document.projection();
        let first = projection
            .hard_line_at_offset(edit.range.start)
            .and_then(|index| projection.hard_line_range(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last = projection
            .hard_line_at_offset(edit.range.end)
            .and_then(|index| projection.hard_line_range(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let Some(mut scopes) = inline_scopes(document, &edit.range, &(first.start..last.end))?
        else {
            continue;
        };
        scopes.sort_by_key(|scope| std::cmp::Reverse(scope.opening.start));
        for scope in scopes {
            if !scope.marker.starts_with(['*', '_', '~', '<']) {
                continue;
            }
            let body = scope.opening.end..scope.closing.start;
            if super::super::markdown_code::edited_source_fragment(document, &body, patches)?
                .is_empty()
            {
                let mut support = Vec::new();
                super::super::source_edit::append_uncovered_deletions(
                    &scope.opening,
                    patches,
                    &mut support,
                );
                super::super::source_edit::append_uncovered_deletions(
                    &scope.closing,
                    patches,
                    &mut support,
                );
                patches.extend(support);
            }
        }
    }
    Ok(())
}

/// A link label cannot contain a paragraph separator. Split its original
/// delimiters just like emphasis, retaining the destination's exact spelling
/// on both surviving labels and dropping wrappers around an empty label.
fn link_scopes(document: &Document, line: &Range<usize>) -> Result<Vec<Scope>, DocumentError> {
    let projection = document.projection();
    let spans = projection
        .style_spans_for_region(line)
        .into_iter()
        .filter(|span| span.application == StyleApplication::Automatic("Link".into()))
        .collect::<Vec<_>>();
    if spans.is_empty() {
        return Ok(Vec::new());
    }
    let source = projection
        .source_range(line.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let physical = &document.state().source_hard_lines;
    let first = physical
        .line_at_offset(source.start)
        .and_then(|at| physical.get(at))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let last = physical
        .line_at_offset(source.end.saturating_sub(1).max(source.start))
        .and_then(|at| physical.get(at))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let bytes = document
        .state()
        .source
        .bytes_in(first.start..last.end)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, first.start)?;
    let normalized = normalize(&decoded, document.file_format());
    let mapper = super::super::rich_text::Builder::new(&normalized, document.revision());
    let mut scopes = Vec::new();
    for link in super::super::links::markdown_links_in(&normalized.text, 0..normalized.text.len()) {
        let label = mapper.source_range(link.label.clone());
        let Some(span) = spans.iter().find(|span| {
            projection
                .source_range(span.range.clone())
                .is_some_and(|source| label.start <= source.start && source.end <= label.end)
        }) else {
            continue;
        };
        let opening = link.range.start..link.label.start;
        let closing = link.label.end..link.range.end;
        scopes.push(Scope {
            formatted: span.range.clone(),
            opening: mapper.source_range(opening.clone()),
            closing: mapper.source_range(closing.clone()),
            marker: normalized.text[opening].to_owned(),
            closing_marker: normalized.text[closing].to_owned(),
        });
    }
    Ok(scopes)
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
        scope.closing_marker = "_".to_owned();
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
        syntax.push_str(&scopes[index].closing_marker);
    }
    for &index in &next[shared..] {
        syntax.push_str(&scopes[index].marker);
    }
    current.clear();
    current.extend_from_slice(next);
}

/// Keep affected emphasis semantic when an edit invalidates GFM flanking.
/// Use passive HTML pairs when Markdown markers can no longer express it, and
/// compose those local repairs with the original patches in one transaction.
pub(super) fn repair_flanking(
    document: &Document,
    edits: &[TextEdit],
    patches: &mut Vec<SourcePatch>,
) -> Result<(), ModelTransactionError> {
    let mut scopes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for edit in edits {
        let projection = document.projection();
        let Some(first) = projection
            .hard_line_at_offset(edit.range.start)
            .and_then(|index| projection.hard_line_range(index))
        else {
            continue;
        };
        let Some(last) = projection
            .hard_line_at_offset(edit.range.end)
            .and_then(|index| projection.hard_line_range(index))
        else {
            continue;
        };
        if let Some(found) = inline_scopes(document, &edit.range, &(first.start..last.end))? {
            scopes.extend(found.into_iter().filter(|scope| {
                scope.marker.starts_with(['*', '_', '~'])
                    && seen.insert((scope.opening.start, scope.closing.end))
            }));
        }
    }
    if scopes.is_empty() {
        return Ok(());
    }
    let mut ordered = patches.clone();
    validate_source_patches(&mut ordered)?;
    let candidate = apply_source_patches(&document.state().source, &ordered)?;
    let map = |at, association| rebase_source_boundary(at, &ordered, association);
    let mut support = Vec::new();
    for scope in scopes {
        if ordered.iter().any(|patch| {
            [&scope.opening, &scope.closing]
                .iter()
                .any(|marker| patch.range.start < marker.end && marker.start < patch.range.end)
        }) {
            continue;
        }
        let opening = map(scope.opening.start, Association::AfterInsertion)?
            ..map(scope.opening.end, Association::BeforeInsertion)?;
        let closing = map(scope.closing.start, Association::AfterInsertion)?
            ..map(scope.closing.end, Association::BeforeInsertion)?;
        if opening.start > opening.end || closing.start > closing.end || opening.end > closing.start
        {
            continue;
        }
        let marker = document.encoding().encode_fragment(&scope.marker)?;
        if candidate.bytes_in(opening.clone()).as_ref() != Some(&marker)
            || candidate.bytes_in(closing.clone()).as_ref() != Some(&marker)
        {
            continue;
        }
        let body = candidate
            .bytes_in(opening.end..closing.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let body = document.encoding().decode_region(&body, opening.end)?.text;
        if body.is_empty() || body.contains(&document.file_format().spelling().repeat(2)) {
            continue;
        }
        // Recognize this exact scope in its edited physical context. Entities
        // and nested closing markers count as punctuation in GFM flanking;
        // whitespace escaping alone cannot keep every such scope representable.
        let first = document
            .state()
            .source_hard_lines
            .line_at_offset(scope.opening.start)
            .unwrap();
        let last = document
            .state()
            .source_hard_lines
            .line_at_offset(scope.closing.end)
            .unwrap();
        let context_boundary = |at, after| {
            let boundary = ordered
                .iter()
                .find(|patch| patch.range.start < at && at < patch.range.end)
                .map_or(at, |patch| {
                    if after {
                        patch.range.end
                    } else {
                        patch.range.start
                    }
                });
            map(
                boundary,
                if after {
                    Association::AfterInsertion
                } else {
                    Association::BeforeInsertion
                },
            )
        };
        let start = context_boundary(
            document.state().source_hard_lines.get(first).unwrap().start,
            false,
        )?
        .min(opening.start);
        let end = context_boundary(
            document.state().source_hard_lines.get(last).unwrap().end,
            true,
        )?
        .max(closing.end)
        .min(candidate.len());
        let bytes = candidate
            .bytes_in(start..end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, start)?;
        let input = normalize(&decoded, document.file_format());
        let normalized_at = |at| {
            input
                .units
                .get(input.units.partition_point(|unit| unit.source.start < at))
                .map_or(input.text.len(), |unit| unit.normalized.start)
        };
        let extent = normalized_at(opening.start)..normalized_at(closing.end);
        let recognized =
            super::super::markdown_syntax::inlines(&input.text, 0..input.text.len(), "");
        let hidden = |range: Range<usize>| {
            (range.start..range.end).all(|at| {
                recognized.values().any(|inline| {
                    matches!(
                        inline.kind,
                        super::super::markdown_syntax::InlineKind::Emphasis
                            | super::super::markdown_syntax::InlineKind::Strong
                            | super::super::markdown_syntax::InlineKind::Strike
                    ) && (inline.range.start <= at && at < inline.inner.start
                        || inline.inner.end <= at && at < inline.range.end)
                })
            })
        };
        if hidden(extent.start..normalized_at(opening.end))
            && hidden(normalized_at(closing.start)..extent.end)
        {
            continue;
        }
        if let Some(newline) = body.find(document.file_format().spelling()) {
            let tail = &body[newline + document.file_format().spelling().len()..];
            let quote = super::super::markdown_quotes::prefix(tail);
            let marker =
                super::super::markdown_blocks::marker_prefix_length(&tail[quote..]).unwrap_or(0);
            if marker > 0 && tail[quote + marker..].trim().is_empty() && !body[..newline].is_empty()
            {
                let at = opening.end + document.encoding().encode_fragment(&body[..newline])?.len();
                support.push((
                    at..at,
                    document.encoding().encode_fragment(&scope.closing_marker)?,
                ));
                support.push((closing, Vec::new()));
                continue;
            }
        }
        let (open, close) = match (scope.marker.as_bytes()[0], scope.marker.len()) {
            (b'~', _) => ("<del>", "</del>"),
            (_, 2) => ("<strong>", "</strong>"),
            _ => ("<em>", "</em>"),
        };
        support.push((opening, document.encoding().encode_fragment(open)?));
        support.push((closing, document.encoding().encode_fragment(close)?));
    }
    if support.is_empty() {
        return Ok(());
    }
    support.sort_by_key(|(range, _)| (range.start, range.end));
    support.dedup_by(|a, b| a.0 == b.0);
    let mut composition = replacement::PatchComposition::new(document.source_byte_len());
    for patch in ordered.iter().rev() {
        composition.splice(patch.range.clone(), &patch.replacement);
    }
    for (range, bytes) in support.into_iter().rev() {
        composition.splice(range, &bytes);
    }
    *patches = composition
        .patches()
        .into_iter()
        .map(|(range, bytes)| SourcePatch::primary(range, bytes))
        .collect();
    Ok(())
}
