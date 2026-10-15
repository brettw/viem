//! External text pasted into Markdown Preview is interpreted as Markdown.
//! Complete pasted blocks keep their own structure at the document's top
//! level; only a leading or trailing partial paragraph joins the destination
//! paragraph's inline content. Every step is prepared on a scratch document
//! and the composed source patches are verified as one transaction.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{Block, BlockKind, ContainerKind, Encoding};
use pulldown_cmark::{Event, Options, Parser, Tag};

/// One top-level pasted block. `paragraph` blocks may join the destination's
/// inline content; every other block retains its own structure.
#[derive(Clone, Debug)]
struct Unit {
    range: Range<usize>,
    paragraph: bool,
}

#[derive(Debug)]
struct Paste {
    text: String,
    units: Vec<Unit>,
    leading_break: bool,
    terminated: bool,
}

impl Paste {
    fn parse(input: &str) -> Self {
        let text = input.replace("\r\n", "\n").replace('\r', "\n");
        let mut units = Vec::new();
        let mut covered = 0;
        let mut depth = 0usize;
        let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
        for (event, range) in Parser::new_ext(&text, options).into_offset_iter() {
            let block = match &event {
                Event::Start(tag) => {
                    depth += 1;
                    if depth > 1 { continue; }
                    Some(matches!(tag, Tag::Paragraph))
                }
                Event::End(_) => { depth -= 1; continue; }
                _ if depth == 0 => Some(false),
                _ => None,
            };
            let Some(paragraph) = block else { continue };
            if range.start < covered { continue; }
            push_gap(&text, covered..range.start, &mut units);
            covered = range.end.max(covered);
            push_unit(&text, range, paragraph, &mut units);
        }
        push_gap(&text, covered..text.len(), &mut units);
        let trimmed = text.trim_end_matches([' ', '\t']);
        Self {
            leading_break: text.trim_start_matches([' ', '\t']).starts_with('\n'),
            terminated: trimmed.ends_with('\n'),
            text,
            units,
        }
    }

    fn source(&self, units: &[Unit]) -> &str {
        match (units.first(), units.last()) {
            (Some(first), Some(last)) => &self.text[first.range.start..last.range.end],
            _ => "",
        }
    }
}

/// Reference definitions produce no parser events but remain authored blocks.
fn push_gap(text: &str, gap: Range<usize>, units: &mut Vec<Unit>) {
    if !text[gap.clone()].trim().is_empty() {
        push_unit(text, gap, false, units);
    }
}

/// Whole physical lines, without surrounding blank lines or the final ending.
fn push_unit(text: &str, mut range: Range<usize>, paragraph: bool, units: &mut Vec<Unit>) {
    let line_start = text[..range.start].rfind('\n').map_or(0, |at| at + 1);
    if text[line_start..range.start].chars().all(|c| c == ' ' || c == '\t') {
        range.start = line_start;
    }
    while range.start < range.end {
        let line_end = text[range.start..range.end].find('\n').map_or(range.end, |at| range.start + at + 1);
        if !text[range.start..line_end].trim().is_empty() { break; }
        range.start = line_end;
    }
    let mut end = range.end;
    loop {
        let body = text[range.start..end].trim_end_matches('\n');
        let last_line = body.rfind('\n').map_or(range.start, |at| range.start + at + 1);
        end = range.start + body.len();
        if last_line == range.start || !text[last_line..end].trim().is_empty() { break; }
        end = last_line;
    }
    range.end = end;
    if !range.is_empty() {
        units.push(Unit { range, paragraph });
    }
}

impl Document {
    /// Interpret external text as Markdown at `range` in Markdown Preview.
    /// `None` means the destination keeps literal text (code, tables and
    /// constructs this destination cannot represent); callers then insert the
    /// text literally. The returned offset is the caret after the paste.
    /// Without `inline`, a single partial paragraph also stays literal, so
    /// pending typing choices can apply to it.
    pub(crate) fn prepare_markdown_paste(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
        markdown: &str,
        inline: bool,
    ) -> Result<Option<(PreparedModelTransaction, usize)>, ModelTransactionError> {
        if self.format() != Format::Markdown {
            return Ok(None);
        }
        self.validate_range(&range)?;
        let paste = Paste::parse(markdown);
        if paste.units.is_empty() || !self.markdown_paste_destination(range.start, affinity)? {
            return Ok(None);
        }
        let partial = paste.units.len() == 1 && paste.units[0].paragraph
            && !paste.terminated && !paste.leading_break;
        if partial && !inline {
            return Ok(None);
        }
        if partial {
            let (fragment, text) = markdown_fragment(paste.source(&paste.units))?;
            return Ok(self.prepare_clipboard_fragment(range.clone(), &fragment, &text)
                .ok().flatten().map(|prepared| (prepared, range.start + text.len())));
        }
        if self.projection().table_cell_at(range.start).is_some()
            || self.projection().table_cell_at(range.end).is_some()
        {
            return Ok(None);
        }
        Ok(self.prepare_structural_markdown_paste(range, &paste).ok().flatten())
    }

    /// Code spans, code blocks, tables, rules and passive HTML keep literal text.
    fn markdown_paste_destination(&self, at: usize, affinity: BoundaryAffinity) -> Result<bool, DocumentError> {
        if self.is_code_at(at, affinity)? {
            return Ok(false);
        }
        Ok(self.projection().blocks_for_region(&(at..at)).iter().all(|block| {
            block.style.0 != "Code Block" && !block.thematic_break && !block.markdown_html
                && block.containers.iter().all(|membership| membership.container.kind != ContainerKind::CodeBlock)
        }))
    }

    fn prepare_structural_markdown_paste(
        &self,
        range: Range<usize>,
        paste: &Paste,
    ) -> Result<Option<(PreparedModelTransaction, usize)>, ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut apply = |scratch: &mut Document, prepared: PreparedModelTransaction| -> Result<(), ModelTransactionError> {
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };
        // The region of `self` whose text this paste may replace.
        let region_start = self.paragraph_at(range.start)?.range.start;
        let region_end = self.paragraph_at(range.end)?.range.end;
        let mut at = range.start;
        if !range.is_empty() {
            let deleted = scratch.prepare_text_edits(vec![TextEdit::new(range.clone(), "")])?;
            apply(&mut scratch, deleted)?;
        }
        let destination = scratch.paragraph_at(at)?;
        let before = at > destination.range.start;
        let after = at < destination.range.end;
        let units = &paste.units;
        let merge_head = units[0].paragraph && !paste.leading_break && before;
        let last = units.len() - 1;
        let merge_tail = units[last].paragraph && !paste.terminated && after
            && (last > 0 || !merge_head);
        let blocks = &units[usize::from(merge_head)..units.len() - usize::from(merge_tail)];

        if merge_head {
            let (fragment, text) = markdown_fragment(paste.source(&units[..1]))?;
            let Some(prepared) = scratch.prepare_clipboard_fragment(at..at, &fragment, &text)? else {
                return Ok(None);
            };
            apply(&mut scratch, prepared)?;
            at += text.len();
        }
        let leading = at > destination.range.start;
        let following = after || merge_tail;
        let mut caret = at;
        if blocks.is_empty() {
            if merge_tail || paste.terminated && after {
                let split = scratch.prepare_paragraph_split(at)?;
                apply(&mut scratch, split)?;
                caret = at + 1;
            }
        } else {
            if leading && following {
                let split = scratch.prepare_paragraph_split(at)?;
                apply(&mut scratch, split)?;
            }
            let markdown = paste.source(blocks);
            let expected = Document::from_bytes(markdown.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)?;
            let start = if leading { at + 1 } else { scratch.paragraph_at(at)?.range.start };
            let markers = (
                list_marker(paste.source(&blocks[..1])),
                list_marker(paste.source(&blocks[blocks.len() - 1..])),
            );
            let Some(prepared) = scratch.prepare_top_level_block_insertion(at, leading, following, markdown, markers)? else {
                return Ok(None);
            };
            apply(&mut scratch, prepared)?;
            if !scratch.inserted_blocks_match(start, &expected)? {
                return Ok(None);
            }
            let end = start + expected.projection().text_tree().byte_len();
            caret = if following { end + 1 } else { end };
        }
        if merge_tail {
            let (fragment, text) = markdown_fragment(paste.source(&units[last..]))?;
            let Some(prepared) = scratch.prepare_clipboard_fragment(caret..caret, &fragment, &text)? else {
                return Ok(None);
            };
            apply(&mut scratch, prepared)?;
            caret += text.len();
        }
        let delta = scratch.projection().text_tree().byte_len() as isize - self.projection().text_tree().byte_len() as isize;
        let new_end = (region_end as isize + delta) as usize;
        let old = self.projection().text_tree().slice(region_start..region_end).map_err(DocumentError::FormattedTextStorage)?;
        let new = scratch.projection().text_tree().slice(region_start..new_end).map_err(DocumentError::FormattedTextStorage)?;
        let prefix = common_prefix(&old, &new);
        let suffix = common_suffix(&old[prefix..], &new[prefix..]);
        let edit = TextEdit::new(region_start + prefix..region_end - suffix, &new[prefix..new.len() - suffix]);
        let prepared = self.prepare_text_edits_with_patches(vec![edit], Some(sources.source_patches()))?;
        // Pasted lists keep their authored numbers; only a destination run
        // split around the pasted blocks restarts.
        let region = region_start..region_end;
        let prepared = if self.projection().blocks_for_region(&region).iter()
            .any(|block| matches!(block.kind, BlockKind::ListItem { ordered: true, .. }))
        {
            self.prepare_markdown_numbering(prepared, super::markdown_numbering::NumberingScope::for_range(region))?
        } else {
            prepared
        };
        Ok(Some((prepared, caret)))
    }

    /// Put complete copied Markdown blocks above (or below) the paragraph at
    /// `at`, outside its containers. The result must show exactly `text`.
    pub(super) fn prepare_markdown_block_put(
        &self,
        at: usize,
        before: bool,
        markdown: &str,
        text: &str,
    ) -> Result<Option<(PreparedModelTransaction, usize, usize)>, ModelTransactionError> {
        let paste = Paste::parse(markdown);
        if paste.units.is_empty() {
            return Ok(None);
        }
        let markdown = paste.source(&paste.units);
        let expected = Document::from_bytes(markdown.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)?;
        let length = expected.projection().text_tree().byte_len();
        let shown = expected.projection().text_tree().slice(0..length).map_err(DocumentError::FormattedTextStorage)?;
        if shown != text.strip_suffix('\n').unwrap_or(text) {
            return Ok(None);
        }
        let block = self.paragraph_at(at)?;
        let (insertion, start, inserted) = if before {
            (block.range.start, block.range.start, format!("{shown}\n"))
        } else {
            (block.range.end, block.range.end + 1, format!("\n{shown}"))
        };
        let markers = (
            list_marker(paste.source(&paste.units[..1])),
            list_marker(paste.source(&paste.units[paste.units.len() - 1..])),
        );
        let at = if before { block.range.start } else { block.range.end };
        let Some(reprojected) = self.prepare_top_level_block_insertion(at, !before, before, markdown, markers)? else {
            return Ok(None);
        };
        let candidate = self.prepared_candidate_document(&reprojected)?;
        if !candidate.inserted_blocks_match(start, &expected)? {
            return Ok(None);
        }
        let patches = reprojected.summary.source_patches.clone();
        let prepared = self.prepare_text_edits_with_patches(
            vec![TextEdit::new(insertion..insertion, inserted)], Some(patches))?;
        Ok(Some((prepared, start, start + length)))
    }

    /// The paragraph owning a text offset; a paragraph separator belongs to
    /// the paragraph it ends.
    fn paragraph_at(&self, at: usize) -> Result<Block, DocumentError> {
        self.projection().blocks_for_region(&(at..at)).into_iter()
            .find(|block| block.range.start <= at && at <= block.range.end)
            .ok_or(DocumentError::AmbiguousProjection)
    }

    /// Split the paragraph at `at` as Enter does, retaining its containers.
    fn prepare_paragraph_split(&self, at: usize) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let block = self.paragraph_at(at)?;
        if matches!(block.kind, BlockKind::ListItem { .. } | BlockKind::Heading(_)) || block.quote_depth > 0 {
            return self.prepare_model_request(ModelRequest::ContinueList {
                document: self.id(), revision: self.revision(), at,
            });
        }
        let payload = FormattedTextPayload::new(&self.hard_line_snapshot(), "\n", vec![0])
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        self.prepare_formatted_payload_edits(vec![FormattedPayloadEdit::new(at..at, payload)])
    }

    /// Insert complete Markdown blocks outside every container: after the
    /// paragraph ending at `at` when `leading`, otherwise before (or, when it
    /// is empty and nothing follows, in place of) the paragraph at `at`.
    fn prepare_top_level_block_insertion(
        &self,
        at: usize,
        leading: bool,
        following: bool,
        markdown: &str,
        markers: (Option<(bool, char)>, Option<(bool, char)>),
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let block = self.paragraph_at(at)?;
        let Some(source) = self.paragraph_source_lines(&block)? else { return Ok(None) };
        // Same-kind lists join across a blank line anyway; keep them tight.
        let neighbor = |at: Option<usize>| -> Result<Option<(bool, char)>, DocumentError> {
            let Some(block) = at.map(|at| self.paragraph_at(at)).transpose()? else { return Ok(None) };
            if block.quote_depth > 0 || !matches!(block.kind, BlockKind::ListItem { level: 0, .. }) {
                return Ok(None);
            }
            let Some(lines) = self.paragraph_source_lines(&block)? else { return Ok(None) };
            Ok(self.source_line_text(lines.start)?.and_then(|line| list_marker(&line)))
        };
        let text_len = self.projection().text_tree().byte_len();
        let (previous_block, next_block) = if leading {
            (Some(at), (at < text_len).then_some(at + 1))
        } else {
            (block.range.start.checked_sub(1), Some(block.range.start).filter(|_| following)
                .or_else(|| (block.range.end < text_len).then_some(block.range.end + 1)))
        };
        let join_previous = markers.0.is_some() && neighbor(previous_block)? == markers.0;
        let join_next = markers.1.is_some() && neighbor(next_block)? == markers.1;
        let lines = &self.state().source_hard_lines;
        let blank = |line: usize| -> Result<Option<bool>, DocumentError> {
            let Some(range) = lines.get(line) else { return Ok(None) };
            let bytes = self.state().source.bytes_in(range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
            Ok(Some(self.encoding().decode_region(&bytes, range.start)?.text.trim().is_empty()))
        };
        let ending = self.file_format().spelling();
        let body = markdown.replace('\n', ending);
        if !leading && following && join_previous
            && source.start.checked_sub(1).map(&blank).transpose()?.flatten() == Some(true)
        {
            // Join the same-kind list above directly, before its blank line,
            // so that list stays tight.
            let previous = self.paragraph_at(block.range.start - 1)?;
            let Some(above) = self.paragraph_source_lines(&previous)? else { return Ok(None) };
            let end = self.line_content_end(above.end - 1)?;
            let bytes = self.encoding().encode_fragment(&format!("{ending}{body}"))?;
            let mut patches = vec![SourcePatch::primary(end..end, bytes)];
            patches.extend(self.dedented_remainder(source.start)?);
            return self.prepare_reprojected_source_patches(patches).map(Some);
        }
        let (range, previous, next) = if leading {
            let end = self.line_content_end(source.end - 1)?;
            (end..end, Some(true), blank(source.end)?)
        } else if following {
            let start = lines.get(source.start).ok_or(DocumentError::AmbiguousProjection)?.start;
            (start..start, source.start.checked_sub(1).map(&blank).transpose()?.flatten(), Some(false))
        } else {
            let start = lines.get(source.start).ok_or(DocumentError::AmbiguousProjection)?.start;
            let end = self.line_content_end(source.end - 1)?;
            (start..end, source.start.checked_sub(1).map(&blank).transpose()?.flatten(), blank(source.end)?)
        };
        // A blank line separates the blocks from any adjacent nonblank line,
        // ending containers above and keeping following content distinct.
        let mut text = String::new();
        if leading {
            text.push_str(ending);
            if !join_previous { text.push_str(ending); }
        } else if previous == Some(false) && !join_previous {
            text.push_str(ending);
        }
        text.push_str(&body);
        if !leading && following {
            text.push_str(ending);
            if !join_next { text.push_str(ending); }
        } else if next == Some(false) && !join_next {
            text.push_str(ending);
        }
        let bytes = self.encoding().encode_fragment(&text)?;
        let rest = if leading || !following { source.end } else { source.start };
        let mut patches = vec![SourcePatch::primary(range, bytes)];
        patches.extend(self.dedented_remainder(rest)?);
        self.prepare_reprojected_source_patches(patches).map(Some)
    }

    /// Content nested at four or more columns after top-level blocks would
    /// become indented code. Shift that indented run left by its first line's
    /// indentation, retaining its relative structure, up to the next line
    /// that starts at column zero.
    fn dedented_remainder(&self, first: usize) -> Result<Vec<SourcePatch>, DocumentError> {
        let lines = &self.state().source_hard_lines;
        let mut run = Vec::new();
        let mut line = first;
        while let Some(range) = lines.get(line) {
            let bytes = self.state().source.bytes_in(range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
            let text = self.encoding().decode_region(&bytes, range.start)?.text;
            let content = text.trim_end_matches(['\n', '\r']);
            if !content.trim().is_empty() {
                let indent = content.len() - content.trim_start_matches(' ').len();
                if content[indent..].starts_with('\t') {
                    return Ok(Vec::new());
                }
                if indent == 0 { break; }
                run.push((range.start, indent));
            }
            line += 1;
        }
        let Some(&(_, shift)) = run.first() else { return Ok(Vec::new()) };
        if shift < 4 {
            return Ok(Vec::new());
        }
        run.into_iter().map(|(start, indent)| {
            let removed = self.encoding().encode_fragment(&" ".repeat(indent.min(shift)))?.len();
            Ok(SourcePatch::primary(start..start + removed, Vec::new()))
        }).collect()
    }

    /// Physical source lines contributing to a paragraph, including a setext
    /// heading underline.
    fn paragraph_source_lines(&self, block: &Block) -> Result<Option<Range<usize>>, DocumentError> {
        let lines = &self.state().source_hard_lines;
        let projection = self.projection();
        let source = projection.source_range(block.range.clone())
            .filter(|source| !source.is_empty())
            .or_else(|| projection.source_insertion_point(block.range.start, true).map(|at| at..at));
        let Some(source) = source else { return Ok(None) };
        let (Some(first), Some(mut last)) = (
            lines.line_at_offset(source.start),
            lines.line_at_offset(source.end.saturating_sub(1).max(source.start)),
        ) else { return Ok(None) };
        if matches!(block.kind, BlockKind::Heading(_)) {
            if let Some(next) = lines.get(last + 1) {
                let bytes = self.state().source.bytes_in(next.clone()).ok_or(DocumentError::AmbiguousProjection)?;
                let line = self.encoding().decode_region(&bytes, next.start)?.text;
                let underline = line.trim();
                if !underline.is_empty() && (underline.chars().all(|c| c == '=') || underline.chars().all(|c| c == '-')) {
                    last += 1;
                }
            }
        }
        Ok(Some(first..last + 1))
    }

    /// One physical source line without its delimiter.
    fn source_line_text(&self, line: usize) -> Result<Option<String>, DocumentError> {
        let Some(range) = self.state().source_hard_lines.get(line) else { return Ok(None) };
        let bytes = self.state().source.bytes_in(range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let text = self.encoding().decode_region(&bytes, range.start)?.text;
        Ok(Some(text.trim_end_matches(['\n', '\r']).to_owned()))
    }

    /// The source byte offset before a physical line's delimiter.
    fn line_content_end(&self, line: usize) -> Result<usize, DocumentError> {
        let range = self.state().source_hard_lines.get(line).ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let text = self.encoding().decode_region(&bytes, range.start)?.text;
        let content = text.trim_end_matches(['\n', '\r']);
        Ok(range.start + self.encoding().encode_fragment(content)?.len())
    }

    /// The pasted blocks reproject as authored: same text, kinds and
    /// containers, not merged into a neighboring list, quote or paragraph.
    fn inserted_blocks_match(&self, start: usize, expected: &Document) -> Result<bool, DocumentError> {
        let projection = expected.projection();
        let length = projection.text_tree().byte_len();
        let end = start + length;
        if end > self.projection().text_tree().byte_len() {
            return Ok(false);
        }
        let actual_text = self.projection().text_tree().slice(start..end).map_err(DocumentError::FormattedTextStorage)?;
        let expected_text = projection.text_tree().slice(0..length).map_err(DocumentError::FormattedTextStorage)?;
        if actual_text != expected_text {
            return Ok(false);
        }
        let shape = |block: &Block, origin: usize| (
            block.range.start - origin, block.range.end - origin, block.style.0.clone(), block.quote_depth,
            std::mem::discriminant(&block.kind), block.thematic_break, block.markdown_html,
            block.code_language.clone(),
            block.containers.iter().map(|membership| membership.container.kind).collect::<Vec<_>>(),
        );
        let actual = self.projection().blocks_for_region(&(start..end)).into_iter()
            .filter(|block| start <= block.range.start && block.range.end <= end)
            .map(|block| shape(&block, start)).collect::<Vec<_>>();
        let expected = projection.blocks_for_region(&(0..length)).iter().map(|block| shape(block, 0)).collect::<Vec<_>>();
        Ok(actual == expected)
    }
}

/// The list kind a line's unindented item marker starts: ordered with its
/// delimiter, or a bullet character. Thematic breaks are not items.
fn list_marker(line: &str) -> Option<(bool, char)> {
    let line = line.lines().next().unwrap_or_default();
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let (ordered, delimiter, rest) = if (1..=9).contains(&digits) {
        let delimiter = line[digits..].chars().next().filter(|c| matches!(c, '.' | ')'))?;
        (true, delimiter, &line[digits + 1..])
    } else {
        let bullet = line.chars().next().filter(|c| matches!(c, '-' | '+' | '*'))?;
        let compact = line.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        if compact.len() >= 3 && compact.chars().all(|c| c == bullet) && bullet != '+' {
            return None;
        }
        (false, bullet, &line[1..])
    };
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then_some((ordered, delimiter))
}

/// A rich fragment of one pasted paragraph, for inline insertion.
fn markdown_fragment(source: &str) -> Result<(crate::document::ClipboardFragment, String), DocumentError> {
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)?;
    let length = document.projection().text_tree().byte_len();
    let text = document.projection().text_tree().slice(0..length).map_err(DocumentError::FormattedTextStorage)?;
    Ok((document.clipboard_fragment(0..length)?, text))
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut length = 0;
    for ((at, a), b) in left.char_indices().zip(right.chars()) {
        if a != b { return at; }
        length = at + a.len_utf8();
    }
    length
}

fn common_suffix(left: &str, right: &str) -> usize {
    let mut length = 0;
    for (a, b) in left.chars().rev().zip(right.chars().rev()) {
        if a != b { break; }
        length += a.len_utf8();
    }
    length
}

#[cfg(test)]
mod tests {
    use super::Paste;

    #[test]
    fn units_are_top_level_blocks_with_whole_lines() {
        let paste = Paste::parse("## Title\n\nSome *text*\nmore\n\n- a\n- b\n\n[ref]: https://example.com\n\n> quote\n");
        let units = paste.units.iter().map(|unit| (&paste.text[unit.range.clone()], unit.paragraph)).collect::<Vec<_>>();
        assert_eq!(units, [
            ("## Title", false), ("Some *text*\nmore", true), ("- a\n- b", false),
            ("[ref]: https://example.com", false), ("> quote", false),
        ]);
        assert!(paste.terminated);
        assert!(!paste.leading_break);
        let paste = Paste::parse("\r\nword");
        assert!(paste.leading_break && !paste.terminated);
        assert_eq!(paste.source(&paste.units), "word");
    }
}
