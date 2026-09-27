//! Fenced paragraph assignment shared by Markdown's two presentations.
use super::*;

impl Document {
    pub(super) fn prepare_markdown_code_style(
        &self,
        range: &Range<usize>,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(range)?;
        let blocks = structural_style::selected_blocks(self, range);
        if !enabled && self.format() == Format::Markdown {
            let mut indented = Vec::new();
            for block in &blocks {
                if super::super::markdown_indented_code::source_block(self, block)?.is_some() { indented.push(block.clone()); }
            }
            if !indented.is_empty() {
                return self.prepare_with_fenced_indented_code(&indented, |doc| doc.prepare_markdown_code_style(range, enabled));
            }
        }
        let mut patches = Vec::new();
        let mut expected_text = self.text().to_owned();
        let mut edits = Vec::new();
        let mut targets = Vec::new();
        for block in blocks {
            let is_code = block.style.0 == "Code Block"
                || super::super::markdown_quotes::is_fenced_block(self, &block)?;
            if is_code == enabled {
                continue;
            }
            let visible = self
                .projection()
                .text_tree()
                .slice(block.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let (source, raw) = self.markdown_paragraph_source(&block, !enabled)?;
            let (syntax, body_range, body_syntax, opening, closing) = if enabled {
                let body = if self.format().is_source_view() {
                    raw.as_str()
                } else {
                    &visible
                };
                // A fence must be longer than every possible closing run in
                // its literal body, including pasted backtick-only lines.
                let width = body
                    .split(|ch| ch != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1;
                let fence = "`".repeat(width);
                (
                    format!("{fence}\n{body}\n{fence}"),
                    0..raw.len(),
                    body.to_owned(),
                    format!("{fence}\n"),
                    format!("\n{fence}"),
                )
            } else if let Some(code) = super::super::markdown_indented_code::source_block(self, &block)? {
                let mut lines = Vec::new();
                for line in code.lines {
                    let bytes = self.state().source.bytes_in(line.content_start..line.source.end)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    lines.push(format!("{}{}", " ".repeat(line.padding), self.encoding().decode_region(&bytes, line.content_start)?.text));
                }
                let prose = prose_from_code(&lines.join("\n"));
                (prose.clone(), 0..raw.len(), prose, String::new(), String::new())
            } else {
                let first_end = raw.find('\n').unwrap_or(raw.len());
                let opening = &raw[..first_end];
                let quote = super::super::markdown_quotes::prefix(opening);
                let marker = super::super::markdown_blocks::marker_prefix_length(&opening[quote..]).unwrap_or(0);
                let (delimiter, width) = super::super::projection::markdown_fence(opening[quote + marker..].trim_start())
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                let last_start = raw.rfind('\n').map_or(raw.len(), |at| at + 1);
                let closing = &raw[last_start..];
                let prefix = super::super::markdown_quotes::prefix(closing);
                let closed = super::super::markdown_syntax::fence_close(&closing[prefix..], delimiter, width);
                let body_end = if closed { last_start - 1 } else { raw.len() };
                let body = &raw[(first_end + 1).min(body_end)..body_end];
                let prose = prose_from_code(if self.format().is_source_view() { body } else { &visible });
                (
                    prose.clone(),
                    (first_end + 1).min(body_end)..body_end,
                    prose,
                    String::new(),
                    String::new(),
                )
            };
            let encode = |text: &str| {
                self.encoding()
                    .encode_fragment(&text.replace('\n', self.file_format().spelling()))
            };
            let body_start = source.start + encode(&raw[..body_range.start])?.len();
            let body_end = source.start + encode(&raw[..body_range.end])?.len();
            if body_start != source.start || !opening.is_empty() {
                patches.push(SourcePatch::primary(
                    source.start..body_start,
                    encode(&opening)?,
                ));
            }
            if raw[body_range] != body_syntax {
                patches.push(SourcePatch::primary(
                    body_start..body_end,
                    encode(&body_syntax)?,
                ));
            }
            if body_end != source.end || !closing.is_empty() {
                patches.push(SourcePatch::primary(
                    body_end..source.end,
                    encode(&closing)?,
                ));
            }
            if self.format().is_source_view() {
                edits.push(TextEdit::new(block.range.clone(), syntax));
            }
            targets.push(block.range);
        }
        if !enabled {
            if let (Some(first), Some(last)) = (targets.first(), targets.last()) {
                patches.extend(markdown_block_styles::code_removal_patches(
                    self,
                    &(first.start..last.end),
                )?);
            }
        }
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        patches.dedup();
        let mut combined: Vec<SourcePatch> = Vec::new();
        for patch in patches {
            if let Some(previous) = combined.last_mut().filter(|previous| {
                previous.range.is_empty() && previous.range.start == patch.range.start
            }) {
                previous.range.end = patch.range.end;
                previous.replacement.extend(patch.replacement);
            } else {
                combined.push(patch);
            }
        }
        for edit in edits.iter().rev() {
            expected_text.replace_range(edit.range.clone(), &edit.replacement);
        }
        let mut prepared = if self.format().is_source_view() {
            self.prepare_reprojected_source_patches(combined)?
        } else {
            self.prepare_text_edits_with_patches(Vec::new(), Some(combined))?
        };
        if prepared.is_no_op() {
            return Ok(prepared);
        }
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        if candidate.projection.text() != expected_text
            || candidate.projection.blocks().len() != self.projection().blocks().len()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        // Text verification alone cannot detect a paragraph being swallowed
        // by its neighbor. Verify every assigned block through the change map.
        for range in targets {
            let at = prepared
                .text_position_map()
                .map_text_point(
                    self.text_point(range.start)?,
                    Association::BeforeInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            let expected = if enabled { "Code Block" } else { "Paragraph" };
            if !candidate
                .projection
                .blocks_for_region(&(at..at))
                .iter()
                .any(|block| block.range.start == at && block.style.0 == expected)
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        prepared.summary.kind = ModelChangeKind::SemanticStyle;
        Ok(prepared)
    }

    fn markdown_paragraph_source(
        &self,
        block: &super::super::Block,
        code: bool,
    ) -> Result<(Range<usize>, String), DocumentError> {
        let projection = self.projection();
        let body = projection
            .source_range(block.range.clone())
            .or_else(|| {
                projection
                    .source_insertion_point(block.range.start, true)
                    .map(|at| at..at)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        let lines = &self.state().source_hard_lines;
        let first = lines
            .line_at_offset(body.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last = lines
            .line_at_offset(body.end.saturating_sub(1).max(body.start))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let source = if code && !self.format().is_source_view() {
            let fence = super::super::markdown_code::fenced_source(self, block)?
                .ok_or(DocumentError::UnsupportedFormatting)?;
            fence.opening.start..fence.closing_content_end.unwrap_or(body.end)
        } else {
            let start = lines
                .get(first)
                .ok_or(DocumentError::AmbiguousProjection)?
                .start;
            let line = lines.get(last).ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = self
                .state()
                .source
                .bytes_in(line.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, line.start)?;
            let normalized = normalize(&decoded, self.file_format());
            let end = normalized
                .endings
                .last()
                .filter(|ending| ending.source.end == line.end)
                .map_or(line.end, |ending| ending.source.start);
            start..end
        };
        let bytes = self
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, source.start)?;
        Ok((source, normalize(&decoded, self.file_format()).text))
    }
}

fn prose_from_code(body: &str) -> String {
    let separator = if body.split('\n').any(str::is_empty) {
        "<br>"
    } else {
        "  \n"
    };
    body.split('\n')
        .map(|line| {
            let mut prose = String::new();
            let start = line.len() - line.trim_start_matches([' ', '\t']).len();
            let end = line.trim_end_matches([' ', '\t']).len();
            let prefix = super::super::projection::markdown_block_prefix(line, 0, line.len())
                .0
                .max(super::super::markdown_quotes::prefix(line))
                .max(
                    super::super::projection::markdown_fence(line)
                        .map_or(0, |(_, width)| start + width),
                );
            for (offset, character) in line.char_indices() {
                if (offset < start || offset >= end) && matches!(character, ' ' | '\t') {
                    prose.push_str(if character == ' ' { "&#32;" } else { "&#9;" });
                    continue;
                }
                if matches!(character, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '&')
                    || offset < prefix && character.is_ascii_punctuation()
                {
                    prose.push('\\');
                }
                prose.push(character);
            }
            prose
        })
        .collect::<Vec<_>>()
        .join(separator)
}
