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
        if !enabled && blocks.iter().any(|block| !block.code_kind.treatment_removable(block.range.is_empty())) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if !enabled && self.format() == Format::Markdown {
            let mut indented = Vec::new();
            for block in &blocks {
                if super::super::markdown_indented_code::source_block(self, block)?.is_some() {
                    indented.push(block.clone());
                }
            }
            if !indented.is_empty() {
                return self.prepare_with_fenced_indented_code(&indented, |doc| {
                    doc.prepare_markdown_code_style(range, enabled)
                });
            }
        }
        let mut patches = Vec::new();
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
            let (body_range, body_syntax, opening, closing) = if enabled {
                let container =
                    &raw[..super::super::markdown_quotes::prefix(raw.lines().next().unwrap_or(""))];
                let body = &visible;
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
                let language = self.projection().default_code_language(block.range.start).unwrap_or_default();
                let body = quote_lines(body, container);
                (
                    0..raw.len(),
                    body,
                    format!("{container}{fence}{language}\n"),
                    format!("\n{container}{fence}"),
                )
            } else if let Some(code) =
                super::super::markdown_indented_code::source_block(self, &block)?
            {
                let prefix_bytes = self.state().source
                    .bytes_in(code.source.start..code.lines[0].content_start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let prefix = self.encoding().decode_region(&prefix_bytes, code.source.start)?.text;
                let quote = super::super::markdown_quotes::prefix(&prefix);
                let marker = super::super::markdown_blocks::marker_prefix_length(&prefix[quote..])
                    .unwrap_or(0);
                let continuation = format!("{}{}", &prefix[..quote], " ".repeat(code.container_indent));
                let first = if marker > 0 { &prefix[..quote + marker] } else { &continuation };
                let mut lines = Vec::new();
                for line in code.lines {
                    let bytes = self
                        .state()
                        .source
                        .bytes_in(line.content_start..line.source.end)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    lines.push(format!(
                        "{}{}",
                        " ".repeat(line.padding),
                        self.encoding()
                            .decode_region(&bytes, line.content_start)?
                            .text
                    ));
                }
                let prose = prose_from_code(&lines.join("\n")).split('\n').enumerate()
                    .map(|(index, line)| format!("{}{line}", if index == 0 { first } else { &continuation }))
                    .collect::<Vec<_>>().join("\n");
                (
                    0..raw.len(),
                    prose,
                    String::new(),
                    String::new(),
                )
            } else {
                let first_end = raw.find('\n').unwrap_or(raw.len());
                let opening = &raw[..first_end];
                let quote = super::super::markdown_quotes::container_prefix(opening);
                let fence = super::super::markdown_code::fenced_source(self, &block)?
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                let last_start = raw.rfind('\n').map_or(raw.len(), |at| at + 1);
                let closed = fence.closing_content_end.is_some();
                let body_end = if closed { last_start - 1 } else { raw.len() };
                let prose = prose_from_code(&visible);
                let prose = if matches!(block.kind, super::super::BlockKind::ListItem { .. }) {
                    let continuation = fence.body_prefix;
                    prose.split('\n').enumerate().map(|(index, line)| {
                        format!("{}{line}", if index == 0 { &opening[..quote] } else { &continuation })
                    }).collect::<Vec<_>>().join("\n")
                } else {
                    quote_lines(&prose, &opening[..quote])
                };
                (
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
        let empty_removal = !enabled && targets.len() == 1 && targets[0].is_empty();
        let mut prepared = if empty_removal {
            // An empty owner's removal keeps formatted text unchanged. Its
            // explicit syntax patches still need a bounded verification band;
            // an unqualified source-only edit would rebuild the whole file.
            self.prepare_text_edits_with_patch_policy(
                vec![TextEdit::new(targets[0].clone(), "")], Some(combined), true,
            )?
        } else {
            self.prepare_text_edits_with_patches(Vec::new(), Some(combined))?
        };
        if prepared.is_no_op() {
            return Ok(prepared);
        }
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        // The authored empty edit already verified exact text equality over
        // its dependency band. Check owner count without materializing every
        // paragraph; the target's resulting treatment is checked below.
        if (!empty_removal && candidate.projection.text() != self.text())
            || candidate.projection.block_count() != self.projection().block_count()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        // Text verification alone cannot detect a paragraph being swallowed
        // by its neighbor. Verify every assigned block through the change map.
        for range in targets {
            let retained_list = (!enabled).then(|| self.projection().blocks_for_region(&range)
                .into_iter().find(|block| block.range == range)
                .and_then(|block| match block.kind {
                    super::super::BlockKind::ListItem { ordered, level, .. } => Some((
                        block.kind.clone(), block.quote_depth,
                        self.projection().style_sheet().list_style_id(ordered, level))),
                    _ => None,
                })).flatten();
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
                .any(|block| {
                    block.range.start == at
                        && (block.style.0 == expected
                            || retained_list.as_ref().is_some_and(|(kind, quote, style)|
                                block.kind == *kind && block.quote_depth == *quote && block.style == *style))
                })
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
        let source = if code {
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
        .enumerate()
        .map(|(index, line)| {
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
            // Text inside an extended autolink stays literal; its body
            // follows the separator and any protected indentation.
            let mut preceding = if index > 0 { separator.trim_end_matches('\n').to_owned() } else { String::new() };
            preceding.push_str(&"&#32;".repeat(start));
            let literal = super::super::markdown_syntax::autolink_literal_ranges(Some(&preceding), &line[start..]);
            for (offset, character) in line.char_indices() {
                if (offset < start || offset >= end) && matches!(character, ' ' | '\t') {
                    prose.push_str(if character == ' ' { "&#32;" } else { "&#9;" });
                    continue;
                }
                if literal.iter().any(|range| range.contains(&(offset - start))) {
                    prose.push(character);
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

fn quote_lines(body: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return body.to_owned();
    }
    body.split('\n')
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
