//! Prefer literal spelling at the edit frontier, without normalizing unrelated
//! source. Each simplification must preserve the local semantic projection.
use super::replacement::PatchComposition;
use super::*;

const MAX_CONTEXT_BYTES: usize = 16 * 1024;
const MAX_WHITESPACE_BYTES: usize = 256;
const MAX_TOKENS: usize = 128;
const MAX_OWNER_PARAGRAPHS: usize = 64;

impl Document {
    pub(super) fn simplify_markdown_edit_spelling(
        &self,
        edits: &[TextEdit],
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        if self.format() != Format::Markdown || patches.is_empty() {
            return Ok(());
        }
        validate_source_patches(patches)?;
        let mut scopes = Vec::new();
        let mut bands: Vec<Range<usize>> = Vec::new();
        for edit in edits {
            let text = self.projection().text_tree();
            let (mut start, mut end) = (edit.range.start, edit.range.end);
            while edit.range.start - start < MAX_WHITESPACE_BYTES {
                let Some(before) = self.previous_grapheme_boundary(start) else {
                    break;
                };
                if !text
                    .slice(before..start)
                    .map_err(DocumentError::FormattedTextStorage)?
                    .chars()
                    .all(|ch| matches!(ch, ' ' | '\t'))
                {
                    break;
                }
                start = before;
            }
            while end - edit.range.end < MAX_WHITESPACE_BYTES {
                let Some(after) = self.next_grapheme_boundary(end) else {
                    break;
                };
                if !text
                    .slice(end..after)
                    .map_err(DocumentError::FormattedTextStorage)?
                    .chars()
                    .all(|ch| matches!(ch, ' ' | '\t'))
                {
                    break;
                }
                end = after;
            }
            // Existing whitespace immediately beside the edit belongs to the
            // cleanup frontier, even if it came from an imported file.
            for span in self.projection().provenance_for_region(&(start..end)) {
                if patches
                    .iter()
                    .any(|patch| ranges_overlap(&patch.range, &span.source))
                {
                    continue;
                }
                let visible = text
                    .slice(span.formatted)
                    .map_err(DocumentError::FormattedTextStorage)?;
                if visible != " " && visible != "\t" {
                    continue;
                }
                let bytes = self
                    .state()
                    .source
                    .bytes_in(span.source.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if !bytes.starts_with(&self.encoding().encode_fragment("&")?) {
                    continue;
                }
                scopes.push(
                    rebase_source_boundary(span.source.start, patches, Association::AfterInsertion)?
                        ..rebase_source_boundary(
                            span.source.end,
                            patches,
                            Association::BeforeInsertion,
                        )?,
                );
            }
            for block in self.projection().blocks_for_region(&(start..end)) {
                let Some(first) = self
                    .projection()
                    .source_insertion_point(block.range.start, true)
                else {
                    continue;
                };
                let Some(last) = self
                    .projection()
                    .source_insertion_point(block.range.end, false)
                else {
                    continue;
                };
                let lines = &self.state().source_hard_lines;
                let Some(first_line) = lines.line_at_offset(first) else {
                    continue;
                };
                // Continuation indentation and passive HTML depend on the
                // preceding owner. Include its complete paragraph, rather than
                // proving a continuation as if it were standalone prose.
                let mut first_line = first_line;
                let context_blocks = self
                    .projection()
                    .blocks_for_region(&(block.range.start.saturating_sub(1)..block.range.start));
                let mut owners_complete = true;
                for context in context_blocks.iter().chain(std::iter::once(&block)) {
                    let Some(owner) = context.containers.iter().rev().find(|member| {
                        member.container.kind == super::super::ContainerKind::ListItem
                    }) else {
                        continue;
                    };
                    if owner.starts_here {
                        continue;
                    }
                    let mut cursor = context.range.start;
                    let mut found = false;
                    for _ in 0..MAX_OWNER_PARAGRAPHS {
                        let Some(preceding) = self
                            .projection()
                            .blocks_for_region(&(cursor.saturating_sub(1)..cursor))
                            .into_iter()
                            .filter(|paragraph| paragraph.range.start < cursor)
                            .max_by_key(|paragraph| paragraph.range.start)
                        else {
                            break;
                        };
                        cursor = preceding.range.start;
                        if preceding.containers.iter().any(|member| {
                            member.container.id == owner.container.id && member.starts_here
                        }) {
                            if let Some(at) = self.projection().source_insertion_point(cursor, true)
                            {
                                if let Some(line) = lines.line_at_offset(at) {
                                    first_line = first_line.min(line);
                                    found = true;
                                }
                            }
                            break;
                        }
                    }
                    owners_complete &= found;
                }
                if !owners_complete {
                    continue;
                }
                for preceding in context_blocks {
                    if preceding.range.start >= block.range.start {
                        continue;
                    }
                    if let Some(at) = self
                        .projection()
                        .source_insertion_point(preceding.range.start, true)
                    {
                        if let Some(line) = lines.line_at_offset(at) {
                            let owner_line =
                                if preceding.markdown_html || preceding.style.0 == "Code Block" {
                                    line.saturating_sub(1)
                                } else {
                                    line
                                };
                            first_line = first_line.min(owner_line);
                        }
                    }
                }
                if block.markdown_html || block.style.0 == "Code Block" {
                    first_line = first_line.saturating_sub(1);
                }
                let Some(last_line) = lines.line_at_offset(last) else {
                    continue;
                };
                // Complete paragraph context protects inline delimiter pairs;
                // the following row protects Setext and physical break syntax.
                let Some(first) = lines.get(first_line) else {
                    continue;
                };
                let Some(last) = lines.get((last_line + 1).min(lines.len() - 1)) else {
                    continue;
                };
                if last.end - first.start <= MAX_CONTEXT_BYTES {
                    bands.push(first.start..last.end);
                }
            }
        }
        let mut delta = 0isize;
        for patch in patches.iter() {
            let start = patch
                .range
                .start
                .checked_add_signed(delta)
                .ok_or(DocumentError::AmbiguousProjection)?;
            if patch
                .replacement
                .iter()
                .any(|byte| matches!(byte, b'&' | b'\\'))
            {
                scopes.push(start..start + patch.replacement.len());
            }
            delta += patch.replacement.len() as isize - patch.range.len() as isize;
        }
        if scopes.is_empty() {
            return Ok(());
        }
        bands.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for band in bands {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| band.start <= previous.end)
            {
                previous.end = previous.end.max(band.end);
            } else {
                merged.push(band);
            }
        }
        let mut cleanup = Vec::new();
        for band in merged {
            if band.len() > MAX_CONTEXT_BYTES
                || patches.iter().any(|patch| {
                    ranges_overlap(&band, &patch.range)
                        && (patch.range.start < band.start || patch.range.end > band.end)
                })
            {
                continue;
            }
            let mut bytes = self
                .state()
                .source
                .bytes_in(band.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            for patch in patches
                .iter()
                .rev()
                .filter(|patch| band.start <= patch.range.start && patch.range.end <= band.end)
            {
                bytes.splice(
                    patch.range.start - band.start..patch.range.end - band.start,
                    patch.replacement.iter().copied(),
                );
            }
            if bytes.len() > MAX_CONTEXT_BYTES {
                continue;
            }
            let origin = rebase_source_boundary(band.start, patches, Association::BeforeInsertion)?;
            let decoded = self.encoding().decode_region(&bytes, 0)?;
            if decoded.spans.iter().any(|span| span.diagnostic.is_some()) {
                continue;
            }
            let baseline = self.spelling_projection(&bytes)?;
            let mut tokens = Vec::new();
            let mut at = 0;
            while at < decoded.text.len() && tokens.len() < MAX_TOKENS {
                let tail = &decoded.text[at..];
                let token = if tail.starts_with('\\') {
                    tail[1..]
                        .chars()
                        .next()
                        .filter(char::is_ascii_punctuation)
                        .map(|ch| (1 + ch.len_utf8(), ch.to_string()))
                } else if tail.starts_with('&') {
                    super::super::html::reference(tail, false)
                        .filter(|(value, length)| {
                            tail[..*length].ends_with(';')
                                && value
                                    .chars()
                                    .all(|ch| ch.is_ascii_graphic() || matches!(ch, ' ' | '\t'))
                        })
                        .map(|(value, length)| (length, value))
                } else {
                    None
                };
                if let Some((length, value)) = token {
                    let start = self.encoding().encoded_text_len(&decoded.text[..at]);
                    let end = start + self.encoding().encoded_text_len(&tail[..length]);
                    let range = start..end;
                    // Only visible text can be simplified: never an attribute,
                    // destination, delimiter, or literal entity inside code.
                    let visible =
                        baseline
                            .provenance_contained_in_source(&range)
                            .iter()
                            .any(|span| {
                                span.source == range
                                    && baseline
                                        .text_tree()
                                        .slice(span.formatted.clone())
                                        .as_deref()
                                        == Ok(value.as_str())
                                    // HTML text still requires standard entity
                                    // spelling for markup delimiters, even if
                                    // a malformed bare '<' happens to render.
                                    && !(value.contains(['<', '&'])
                                        && baseline.blocks_for_region(&span.formatted).iter()
                                            .any(|block| block.markdown_html))
                            });
                    if visible && !super::markdown_autodetect::protected_reference(tail)
                        && scopes
                            .iter()
                            .any(|scope| scope.start <= origin + start && origin + end <= scope.end)
                    {
                        if let Ok(replacement) = self.encoding().encode_fragment(&value) {
                            tokens.push((range, replacement));
                        }
                    }
                    at += length;
                } else {
                    at += tail.chars().next().unwrap().len_utf8();
                }
            }
            if tokens.is_empty() {
                continue;
            }
            let mut simplified = bytes.clone();
            for (range, replacement) in tokens.iter().rev() {
                simplified.splice(range.clone(), replacement.iter().copied());
            }
            if same_projection(&baseline, &self.spelling_projection(&simplified)?) {
                cleanup.extend(tokens.into_iter().map(|(range, replacement)| {
                    (origin + range.start..origin + range.end, replacement)
                }));
            } else {
                // One necessary escape must not prevent other safe tokens in
                // this edit from returning to ordinary Markdown spelling.
                for (range, replacement) in tokens.into_iter().rev() {
                    let mut candidate = bytes.clone();
                    candidate.splice(range.clone(), replacement.iter().copied());
                    if same_projection(&baseline, &self.spelling_projection(&candidate)?) {
                        bytes = candidate;
                        cleanup.push((origin + range.start..origin + range.end, replacement));
                    }
                }
            }
        }
        if cleanup.is_empty() {
            return Ok(());
        }
        let mut composition = PatchComposition::new(self.source_byte_len());
        for patch in patches.iter().rev() {
            composition.splice(patch.range(), patch.replacement());
        }
        cleanup.sort_by_key(|(range, _)| range.start);
        for (range, replacement) in cleanup.into_iter().rev() {
            composition.splice(range, &replacement);
        }
        *patches = composition.source_patches();
        Ok(())
    }

    fn spelling_projection(&self, bytes: &[u8]) -> Result<FormattedDocument, DocumentError> {
        let decoded = self.encoding().decode_region(bytes, 0)?;
        let normalized = normalize(&decoded, self.file_format());
        Ok(project(
            &normalized,
            Format::Markdown,
            self.revision(),
            0,
            bytes.len(),
        ))
    }
}

fn same_projection(before: &FormattedDocument, after: &FormattedDocument) -> bool {
    before.text() == after.text()
        && before.style_spans() == after.style_spans()
        && before.hard_break_offsets() == after.hard_break_offsets()
        && before.blocks().len() == after.blocks().len()
        && before
            .blocks()
            .iter()
            .zip(after.blocks())
            .all(|(a, b)| a.range == b.range && a.attributes == b.attributes)
}
