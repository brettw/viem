//! Optional authored-Markdown recognition. Speculative typing and delimiter
//! removal compose into one verified source transaction, never a visible edit
//! followed by an unverified formatting operation.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{ReplacementTypingContext, StylePropertyValue};

const CONTEXT_BYTES: usize = 8192;

// Six-digit ASCII references deliberately retain literal-next intent across
// saves, view changes, subsequent typing and spelling cleanup.
pub(super) fn protected_reference(text: &str) -> bool {
    text.starts_with("&#x0000")
        && text.as_bytes().get(9) == Some(&b';')
        && text
            .as_bytes()
            .get(7..9)
            .is_some_and(|s| s.iter().all(u8::is_ascii_hexdigit))
}

struct Recognition {
    patches: Vec<SourcePatch>,
    edits: Vec<TextEdit>,
    caret: usize,
    exit_source: Option<usize>,
}

impl Document {
    pub(crate) fn markdown_pending_code_delimiter(
        &self,
        caret: usize,
    ) -> Result<Option<usize>, DocumentError> {
        let Some(block) = super::super::edit_boundary::paragraph_at(self, caret)? else {
            return Ok(None);
        };
        if block.markdown_html {
            return Ok(None);
        }
        if caret - block.range.start > CONTEXT_BYTES {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let text = self
            .projection()
            .text_tree()
            .slice(block.range.start..caret)
            .map_err(DocumentError::FormattedTextStorage)?;
        let mut active = None;
        let mut offset = 0;
        while let Some(relative) = text[offset..].find('`') {
            let start = offset + relative;
            let width = text[start..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            offset = start + width;
            if self
                .markdown_literal_syntax(block.range.start + start..block.range.start + offset)?
                .is_some()
            {
                if active == Some(width) {
                    active = None;
                } else if active.is_none() {
                    active = Some(width);
                }
            }
        }
        Ok(active)
    }

    pub(crate) fn prepare_markdown_typing_batch(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        inherited: Option<&ReplacementTypingContext>,
        literal: bool,
        exit: Option<crate::document::SourcePoint>,
    ) -> Result<(PreparedModelTransaction, usize, usize, Option<usize>), ModelTransactionError>
    {
        self.validate_typing_payload(&edit)?;
        if exit.is_some_and(|point| {
            point.document() != self.id() || point.revision() != self.revision()
        }) {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        if !literal
            && exit.is_none()
            && !edit
                .payload
                .text()
                .contains(['*', '_', '~', '`', ')', '-', '>', '#', '[', ']', '+'])
            && !edit.payload.text().starts_with(' ')
        {
            return self
                .prepare_insertion_with_typing_context(edit, named, values, inherited)
                .map(|(p, c, s)| (p, c, s, None));
        }
        if edit.payload.text().graphemes(true).count() == 1 {
            return self
                .prepare_markdown_typing_grapheme(edit, named, values, inherited, literal, exit);
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let mut caret = edit.range.start;
        let mut authored_start = caret;
        let mut exit_source = exit.map(|point| point.offset());
        for (index, grapheme) in edit.payload.text().grapheme_indices(true) {
            let lines = scratch.hard_line_snapshot();
            let breaks = if edit.payload.break_offsets().contains(&index) {
                vec![0]
            } else {
                vec![]
            };
            let payload = FormattedTextPayload::new(&lines, grapheme, breaks)
                .expect("grapheme slice preserves validated payload breaks");
            let range = if index == 0 {
                edit.range.clone()
            } else {
                caret..caret
            };
            let (prepared, next, start, closed) = scratch.prepare_markdown_typing_grapheme(
                FormattedPayloadEdit::new(range, payload).with_boundary_affinity(if index == 0 {
                    edit.boundary_affinity
                        .unwrap_or(BoundaryAffinity::Downstream)
                } else if exit_source.is_some() {
                    BoundaryAffinity::Downstream
                } else {
                    BoundaryAffinity::Upstream
                }),
                named,
                values,
                if index == 0 { inherited } else { None },
                literal,
                exit_source.map(|at| scratch.source_point(at)).transpose()?,
            )?;
            if index == 0 {
                authored_start = start;
            }
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            formatted.record_formatted(&prepared)?;
            scratch.commit_model_transaction(prepared)?;
            caret = next;
            exit_source = closed;
        }
        let edits = formatted.formatted_edits();
        let patches = sources.source_patches();
        let prepared = self.prepare_text_edits_with_patch_policy(edits, Some(patches), true)?;
        Ok((prepared, caret, authored_start.min(caret), exit_source))
    }

    pub(super) fn prepare_markdown_typing_grapheme(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        inherited: Option<&ReplacementTypingContext>,
        literal: bool,
        exit: Option<crate::document::SourcePoint>,
    ) -> Result<(PreparedModelTransaction, usize, usize, Option<usize>), ModelTransactionError>
    {
        let input = edit.payload.text().to_owned();
        let mut edit = edit;
        if input == ")" && named.is_none() && values.is_empty() {
            edit.boundary_affinity = Some(BoundaryAffinity::Downstream);
        }
        let (first, caret, start) = if let Some(point) = exit.filter(|_| {
            edit.range.is_empty()
                && named.is_none()
                && values.is_empty()
                && inherited.is_none()
                && !input.contains(['\n', '\r'])
        }) {
            if point.document() != self.id() || point.revision() != self.revision() {
                return Err(DocumentError::AmbiguousProjection.into());
            }
            let at = edit.range.start;
            let syntax = self.escape_markdown_source_text(point.offset(), &input)?;
            let patch = SourcePatch::primary(
                point.offset()..point.offset(),
                self.encoding().encode_fragment(&syntax)?,
            );
            let prepared = self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(vec![patch]),
                true,
            )?;
            (prepared, at + input.len(), at)
        } else if matches!(input.as_str(), "`" | "~")
            && edit.range.is_empty()
            && named.is_none()
            && values.is_empty()
            && inherited.is_none()
            && !self.is_code_at(
                edit.range.start,
                edit.boundary_affinity
                    .unwrap_or(BoundaryAffinity::Downstream),
            )?
            && !super::super::edit_boundary::paragraph_at(self, edit.range.start)?
                .is_some_and(|block| block.markdown_html)
        {
            let at = super::super::source_edit::insertion_point(
                self.projection(),
                edit.range.start,
                edit.boundary_affinity,
            )
            .ok_or(DocumentError::AmbiguousProjection)?;
            let patch = SourcePatch::primary(
                at..at,
                self.encoding().encode_fragment(&format!("\\{input}"))?,
            );
            match self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(vec![patch]),
                true,
            ) {
                Ok(prepared) => (prepared, edit.range.start + input.len(), edit.range.start),
                Err(ModelTransactionError::Document(
                    DocumentError::UnsupportedFormatting
                    | DocumentError::VerificationFailed
                    | DocumentError::FormattedPayloadCannotReproject,
                )) => self.prepare_insertion_with_typing_context(
                    edit.clone(),
                    named,
                    values,
                    inherited,
                )?,
                Err(error) => return Err(error),
            }
        } else {
            match self.prepare_insertion_with_typing_context(edit.clone(), named, values, inherited)
            {
                Ok(prepared) => prepared,
                Err(ModelTransactionError::Document(
                    DocumentError::FormattedPayloadCannotReproject
                    | DocumentError::VerificationFailed,
                )) if input.chars().all(|ch| ch.is_ascii_punctuation())
                    && named.is_none()
                    && values.is_empty()
                    && inherited.is_none() =>
                {
                    let range = if edit.range.is_empty() {
                        let at = super::super::source_edit::insertion_point(
                            self.projection(),
                            edit.range.start,
                            edit.boundary_affinity,
                        )
                        .ok_or(DocumentError::AmbiguousProjection)?;
                        at..at
                    } else {
                        self.projection()
                            .source_range(edit.range.clone())
                            .ok_or(DocumentError::AmbiguousProjection)?
                    };
                    let syntax = input
                        .chars()
                        .map(|ch| format!("\\{ch}"))
                        .collect::<String>();
                    let patch =
                        SourcePatch::primary(range, self.encoding().encode_fragment(&syntax)?);
                    let prepared = self.prepare_text_edits_with_patch_policy(
                        vec![edit.text_edit()],
                        Some(vec![patch]),
                        true,
                    )?;
                    (prepared, edit.range.start + input.len(), edit.range.start)
                }
                Err(error) => return Err(error),
            }
        };
        let mut scratch = self.scratch_document();
        let PreparedPublication::State(state) = &first.publication else {
            return Ok((first, caret, start, None));
        };
        scratch.history = super::super::history::History::transient(state.clone());
        scratch.next_revision = first
            .after_revision
            .0
            .checked_add(1)
            .ok_or(ModelTransactionError::RevisionExhausted)?;
        scratch.next_projected_block_id = first.next_projected_block_id_after;
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        for patch in first.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        formatted.record_formatted(&first)?;
        if scratch.is_code_at(caret, BoundaryAffinity::Upstream)?
            || super::super::edit_boundary::paragraph_at(&scratch, caret)?
                .is_some_and(|block| block.markdown_html)
        {
            return Ok((first, caret, start, None));
        }
        if literal {
            for (index, ch) in input.char_indices().rev() {
                if !ch.is_ascii_punctuation() && !ch.is_ascii_digit() && ch != ' ' {
                    continue;
                }
                let at = start + index;
                let source = scratch
                    .projection()
                    .source_range(at..at + ch.len_utf8())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                sources.splice(
                    source,
                    &self
                        .encoding()
                        .encode_fragment(&format!("&#x{:06X};", ch as u32))?,
                );
            }
            return Ok((
                self.prepare_text_edits_with_patch_policy(
                    formatted.formatted_edits(),
                    Some(sources.source_patches()),
                    true,
                )?,
                caret,
                start,
                None,
            ));
        }
        let Some(recognition) = scratch.recognize_markdown(caret, &input)? else {
            // Keep incomplete syntax escaped even when today's parser would
            // permit a bare marker. Later typing must not reinterpret an
            // earlier completed span before the new span is complete.
            let mut pending = false;
            for (index, ch) in input.char_indices().rev() {
                if !matches!(
                    ch,
                    '*' | '_' | '~' | '`' | '-' | '+' | '#' | '>' | '[' | ']'
                ) {
                    continue;
                }
                if let Some(mut patch) =
                    scratch.markdown_literal_syntax(start + index..start + index + ch.len_utf8())?
                {
                    patch.replacement = self.encoding().encode_fragment(&format!("\\{ch}"))?;
                    if scratch.state().source.bytes_in(patch.range())
                        == Some(patch.replacement.clone())
                    {
                        continue;
                    }
                    sources.splice(patch.range(), patch.replacement());
                    pending = true;
                }
            }
            if pending {
                let protected = self.prepare_text_edits_with_patch_policy(
                    formatted.formatted_edits(),
                    Some(sources.source_patches()),
                    true,
                );
                return Ok((
                    // Escaping is optional protection for future input. A
                    // source owner that cannot accept it keeps the already
                    // verified literal insertion available.
                    match protected {
                        Ok(prepared) => prepared,
                        Err(ModelTransactionError::Document(
                            DocumentError::UnsupportedFormatting
                            | DocumentError::VerificationFailed
                            | DocumentError::FormattedPayloadCannotReproject,
                        )) => first,
                        Err(error) => return Err(error),
                    },
                    caret,
                    start,
                    None,
                ));
            }
            return Ok((first, caret, start, None));
        };
        // Parser verification also enforces flanking, intraword underscores,
        // container ownership and code-space normalization. An incomplete or
        // unsupported construct remains the successfully typed literal text.
        match scratch.prepare_text_edits_with_patch_policy(
            recognition.edits,
            Some(recognition.patches),
            true,
        ) {
            Ok(converted) => {
                for patch in converted.summary.source_patches.iter().rev() {
                    sources.splice(patch.range(), patch.replacement());
                }
                formatted.record_formatted(&converted)?;
                let prepared = self.prepare_text_edits_with_patch_policy(
                    formatted.formatted_edits(),
                    Some(sources.source_patches()),
                    true,
                )?;
                Ok((
                    prepared,
                    recognition.caret,
                    start.min(recognition.caret),
                    recognition.exit_source,
                ))
            }
            Err(ModelTransactionError::Document(
                DocumentError::VerificationFailed
                | DocumentError::UnsupportedFormatting
                | DocumentError::AmbiguousProjection,
            )) => Ok((first, caret, start, None)),
            Err(error) => Err(error),
        }
    }

    /// Return a raw-syntax patch only for independently mapped literal input.
    /// Protected Control-Q references never qualify. Ordinary escaping chosen
    /// by the text-edit translator still represents recognizable punctuation.
    fn markdown_literal_syntax(
        &self,
        range: Range<usize>,
    ) -> Result<Option<SourcePatch>, DocumentError> {
        self.markdown_literal_source(range, false)
    }

    fn markdown_literal_source(
        &self,
        range: Range<usize>,
        protected_content: bool,
    ) -> Result<Option<SourcePatch>, DocumentError> {
        let visible = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let Some(source) = self.projection().source_range(range.clone()) else {
            return Ok(None);
        };
        let mut expected = String::new();
        for (index, ch) in visible.char_indices() {
            let Some(run) = self
                .projection()
                .source_range(range.start + index..range.start + index + ch.len_utf8())
            else {
                return Ok(None);
            };
            let bytes = self
                .state()
                .source
                .bytes_in(run)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let raw = self.encoding().decode_region(&bytes, 0)?.text;
            let ordinary_reference = (protected_content || ch.is_ascii_punctuation())
                && raw.starts_with('&')
                && raw.ends_with(';')
                && raw.len() <= 16
                && (protected_content || !protected_reference(&raw));
            if raw != ch.to_string()
                && !(ch.is_ascii_punctuation() && raw == format!("\\{ch}"))
                && !ordinary_reference
            {
                return Ok(None);
            }
            expected.push_str(&raw);
        }
        if self.state().source.bytes_in(source.clone())
            != Some(self.encoding().encode_fragment(&expected)?)
        {
            return Ok(None);
        }
        Ok(Some(SourcePatch::primary(
            source,
            self.encoding().encode_fragment(&visible)?,
        )))
    }

    fn recognize_markdown(
        &self,
        caret: usize,
        input: &str,
    ) -> Result<Option<Recognition>, DocumentError> {
        if self.format() != Format::Markdown
            || !input.ends_with([' ', '*', '_', '~', '`', ')', '-', '>'])
        {
            return Ok(None);
        }
        let lines = self.hard_line_snapshot();
        let line = lines
            .line_at_offset(caret)
            .map_err(|_| DocumentError::AmbiguousProjection)?
            .content_range();
        let mut start = line.start.max(caret.saturating_sub(CONTEXT_BYTES));
        while !self
            .projection()
            .text_tree()
            .is_char_boundary(start)
            .map_err(DocumentError::FormattedTextStorage)?
        {
            start += 1;
        }
        let prefix = self
            .projection()
            .text_tree()
            .slice(start..caret)
            .map_err(DocumentError::FormattedTextStorage)?;
        let in_table = self.projection().table_cell_at(caret).is_some();
        if !in_table && (prefix.ends_with("```") || prefix.ends_with("~~~")) {
            if let Some(fence) =
                self.recognize_markdown_fence(caret, prefix.chars().next_back().unwrap())?
            {
                return Ok(Some(fence));
            }
        }
        if !in_table && start == line.start {
            let marker = prefix.strip_suffix(' ');
            let heading =
                marker.is_some_and(|m| (1..=6).contains(&m.len()) && m.bytes().all(|b| b == b'#'));
            let bullet = matches!(marker, Some("-" | "*" | "+"));
            let quote = marker == Some(">");
            let ordered = marker.is_some_and(|m| {
                let digits = m
                    .strip_suffix('.')
                    .or_else(|| m.strip_suffix(')'))
                    .unwrap_or("");
                (1..=9).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
            });
            let rule =
                (prefix == "---" || matches!(marker, Some("___" | "***"))) && caret == line.end;
            if heading || bullet || quote || ordered || rule {
                if let Some(patch) = self.markdown_literal_syntax(start..caret)? {
                    return Ok(Some(Recognition {
                        patches: vec![patch],
                        edits: vec![TextEdit::new(start..caret, "")],
                        caret: start,
                        exit_source: None,
                    }));
                }
            }
        }
        let Some(last) = prefix.chars().next_back() else {
            return Ok(None);
        };
        if last != '`' && !matches!(self.markdown_pending_code_delimiter(caret), Ok(None)) {
            return Ok(None);
        }
        if matches!(last, '*' | '_' | '~' | '`') {
            let width = prefix
                .bytes()
                .rev()
                .take_while(|b| *b == last as u8)
                .count();
            if (last != '`' && width > 3) || prefix.len() <= width {
                return Ok(None);
            }
            let closing = caret - width..caret;
            let body_end = prefix.len() - width;
            let marker = last.to_string().repeat(width);
            let mut search = body_end;
            while let Some(open) = prefix[..search].rfind(last) {
                let run_start = prefix[..open + 1].trim_end_matches(last).len();
                let run_end = open + 1;
                let opening_width = run_end - run_start;
                search = run_start;
                if opening_width != width {
                    // Wait for the rest of a closing bold/combined run.
                    if opening_width > width {
                        return Ok(None);
                    }
                    continue;
                }
                if run_end == body_end {
                    continue;
                }
                let opening = start + run_start..start + run_end;
                let Some(mut open_patch) = self.markdown_literal_syntax(opening.clone())? else {
                    continue;
                };
                let Some(close_patch) = self.markdown_literal_syntax(closing.clone())? else {
                    return Ok(None);
                };
                // Touching delimiter runs are not always separate spans in
                // CommonMark. Join an identical, certified preceding scope
                // instead; its visible text and formatting remain unchanged.
                let bytes = self.encoding().encode_fragment(&marker)?;
                if let Some(before) = open_patch.range.start.checked_sub(bytes.len()) {
                    let spans = self
                        .projection()
                        .style_spans_for_region(&(opening.start.saturating_sub(1)..opening.start));
                    let has = |application| {
                        spans.iter().any(|span| {
                            span.range.end == opening.start && span.application == application
                        })
                    };
                    let bold = has(StyleApplication::Semantic(SemanticInlineStyle::Strong));
                    let italic = has(StyleApplication::Semantic(SemanticInlineStyle::Emphasis));
                    let code = has(StyleApplication::Semantic(SemanticInlineStyle::Code));
                    let strike = has(StyleApplication::Automatic("Strikethrough".into()));
                    let same = match last {
                        '*' | '_' => {
                            !code
                                && !strike
                                && bold == (width >= 2)
                                && italic == (width == 1 || width == 3)
                        }
                        '~' => strike && !bold && !italic && !code,
                        '`' => code && !bold && !italic && !strike,
                        _ => false,
                    };
                    let preceding_marker = before
                        .checked_sub(self.encoding().encode_fragment(&last.to_string())?.len())
                        .and_then(|start| self.state().source.bytes_in(start..before))
                        == Some(self.encoding().encode_fragment(&last.to_string())?);
                    if same
                        && !preceding_marker
                        && self.state().source.bytes_in(before..open_patch.range.start)
                            == Some(bytes)
                    {
                        open_patch.range.start = before;
                        open_patch.replacement.clear();
                    }
                }
                let patches = if last == '`' {
                    // Code owns literal body spelling. Do not preserve prose
                    // escapes as visible backslashes inside the code span.
                    let Some(literal) =
                        self.markdown_literal_source(opening.start..closing.end, true)?
                    else {
                        return Ok(None);
                    };
                    let source = literal.range;
                    self.reject_unsafe_opaque_mapping(&(opening.start..closing.end), &source)?;
                    let joined = open_patch.replacement.is_empty();
                    vec![SourcePatch::primary(
                        if joined {
                            open_patch.range.start..source.end
                        } else {
                            source
                        },
                        self.encoding().encode_fragment(&format!(
                            "{}{}{marker}",
                            if joined { "" } else { &marker },
                            &prefix[run_end..body_end]
                        ))?,
                    )]
                } else {
                    vec![open_patch, close_patch]
                };
                let end = patches.last().unwrap().range.end;
                let exit_source =
                    rebase_source_boundary(end, &patches, Association::AfterInsertion)
                        .map_err(|_| DocumentError::AmbiguousProjection)?;
                return Ok(Some(Recognition {
                    patches,
                    edits: vec![TextEdit::new(opening, ""), TextEdit::new(closing, "")],
                    caret: caret - width * 2,
                    exit_source: Some(exit_source),
                }));
            }
        } else if last == ')' {
            if let Some(close) = prefix.rfind("](") {
                if let Some(open) = prefix[..close]
                    .rfind('[')
                    .filter(|open| *open == 0 || prefix.as_bytes()[open - 1] != b'!')
                {
                    if close > open + 1 {
                        let opening = start + open..start + open + 1;
                        let closing = start + close..caret;
                        if let (Some(a), Some(b)) = (
                            self.markdown_literal_syntax(opening.clone())?,
                            self.markdown_literal_syntax(closing.clone())?,
                        ) {
                            let exit_source = rebase_source_boundary(
                                b.range.end,
                                &[a.clone(), b.clone()],
                                Association::AfterInsertion,
                            )
                            .map_err(|_| DocumentError::AmbiguousProjection)?;
                            return Ok(Some(Recognition {
                                patches: vec![a, b],
                                edits: vec![
                                    TextEdit::new(opening, ""),
                                    TextEdit::new(closing.clone(), ""),
                                ],
                                caret: caret - closing.len() - 1,
                                exit_source: Some(exit_source),
                            }));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    fn recognize_markdown_fence(
        &self,
        caret: usize,
        delimiter: char,
    ) -> Result<Option<Recognition>, DocumentError> {
        let Some(block) = super::super::edit_boundary::paragraph_at(self, caret)? else {
            return Ok(None);
        };
        if block.markdown_html || block.range.len() > CONTEXT_BYTES || caret < block.range.start + 3
        {
            return Ok(None);
        }
        let marker = caret - 3..caret;
        let Some(body_source) = self.projection().source_range(block.range.clone()) else {
            return Ok(None);
        };
        if body_source.len() > CONTEXT_BYTES * 2 {
            return Ok(None);
        }
        // Only a literal body can be rewritten as code without discarding
        // hidden inline content or opaque source contributors.
        if self.markdown_literal_syntax(marker.clone())?.is_none()
            || self
                .markdown_literal_source(block.range.clone(), true)?
                .is_none()
        {
            return Ok(None);
        }
        let lines = &self.state().source_hard_lines;
        let Some(first) = lines
            .line_at_offset(body_source.start)
            .and_then(|i| lines.get(i))
        else {
            return Ok(None);
        };
        let Some(last) = lines
            .line_at_offset(body_source.end.saturating_sub(1))
            .and_then(|i| lines.get(i))
        else {
            return Ok(None);
        };
        let physical = first.start..last.end;
        if physical.len() > CONTEXT_BYTES * 2 {
            return Ok(None);
        }
        let bytes = self
            .state()
            .source
            .bytes_in(physical.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, physical.start)?;
        if decoded.spans.iter().any(|span| span.diagnostic.is_some()) {
            return Ok(None);
        }
        let normalized = normalize(&decoded, self.file_format());
        let end = normalized
            .endings
            .last()
            .filter(|ending| ending.source.end == physical.end)
            .map_or(physical.end, |ending| ending.source.start);
        let opening = normalized.text.split('\n').next().unwrap_or("");
        let quote = super::super::markdown_quotes::prefix(opening);
        let tail = &opening[quote..];
        let list = super::super::markdown_blocks::marker_prefix_geometry(tail);
        let indentation = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        let first_prefix = &opening[..quote + list.map_or(indentation, |(bytes, _)| bytes)];
        let continued = if let Some((_, width)) = list {
            format!("{}{}", &opening[..quote], " ".repeat(width))
        } else {
            first_prefix.to_owned()
        };
        let mut body = self
            .projection()
            .text_tree()
            .slice(block.range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        body.replace_range(
            marker.start - block.range.start..marker.end - block.range.start,
            "",
        );
        let width = body
            .split(|ch| ch != delimiter)
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1;
        let fence = delimiter.to_string().repeat(width);
        let nl = self.file_format().spelling();
        let body = body
            .split('\n')
            .map(|line| format!("{continued}{line}"))
            .collect::<Vec<_>>()
            .join(nl);
        let language = self.projection().default_code_language(block.range.start).unwrap_or_default();
        let syntax = format!("{first_prefix}{fence}{language}{nl}{body}{nl}{continued}{fence}");
        Ok(Some(Recognition {
            patches: vec![SourcePatch::primary(
                physical.start..end,
                self.encoding().encode_fragment(&syntax)?,
            )],
            edits: vec![TextEdit::new(marker, "")],
            caret: block.range.start,
            exit_source: None,
        }))
    }
}
