//! Bounded reprojection for independent prose and guarded structural owners.
use super::*;

pub(super) enum ListParagraphReprojection {
    Candidate(PreparedModelTransaction, Range<usize>, Range<usize>),
    TextMismatch,
}

/// Source diffing can describe the same visible result with different deleted
/// characters. For example, deleting the nonblank tail of "\nlast" code can
/// look like replacing the entire owner with an empty one. Preserve the caller's
/// semantic edit when the bounded parse proves exactly that result; otherwise
/// surviving empty rows lose their block/container identities and caret policy.
fn matching_requested_edits(
    expected: Option<(&[TextEdit], &FormattedTextTree)>,
    old: &Range<usize>,
    old_len: usize,
    regional: &FormattedDocument,
) -> Result<Option<Vec<TextEdit>>, DocumentError> {
    let Some((edits, target)) = expected else { return Ok(None); };
    if !edits.iter().all(|edit| old.start <= edit.range.start && edit.range.end <= old.end) {
        return Ok(None);
    }
    let Some(end) = target.byte_len().checked_sub(old_len - old.end) else { return Ok(None); };
    if end < old.start || regional.text_tree().byte_len() != end - old.start
        || target.slice(old.start..end).map_err(DocumentError::FormattedTextStorage)?
            != regional.text_tree().slice(0..regional.text_tree().byte_len()).map_err(DocumentError::FormattedTextStorage)? {
        return Ok(None);
    }
    Ok(Some(edits.to_vec()))
}

impl Document {
    pub(super) fn prepare_independent_paragraph_reprojection(
        &self,
        patches: &[SourcePatch],
        expected: Option<(&[TextEdit], &FormattedTextTree)>,
    ) -> Result<Option<(PreparedModelTransaction, Range<usize>, Range<usize>)>, ModelTransactionError> {
        if let Ok(Some(prepared)) = self.prepare_paragraph_reprojection(patches, false, expected) {
            return Ok(Some(prepared));
        }
        self.prepare_paragraph_reprojection(patches, true, expected)
    }

    fn independent_markdown_paragraph(&self, block: &super::super::Block) -> bool {
        // Source exposes whitespace in surplus separator rows. Such a body
        // does not stop blank-line pairing when a preceding ending is edited,
        // so it cannot prove that a regional parser is independent.
        if block.range.len() > 32768 || self.projection().text_tree().slice(block.range.clone())
            .is_ok_and(|text| text.trim().is_empty()) {
            return false;
        }
        // An ATX heading is one source line opened by its own marker; a setext
        // heading's underline is a separate line, so it is not independent.
        let atx = |block: &super::super::Block| -> bool {
            let Some(content) = self.projection().source_range(block.range.clone()) else { return false; };
            let Ok(line) = self.physical_line_at_source(content.start) else { return false; };
            self.source_bytes_in(line.source_range.start..content.start).is_ok_and(|prefix| {
                self.encoding().decode_region(&prefix, line.source_range.start)
                    .is_ok_and(|prefix| prefix.text.trim_start_matches(' ').starts_with('#'))
            })
        };
        (block.kind == super::super::BlockKind::Paragraph
                || matches!(block.kind, super::super::BlockKind::Heading(_)) && atx(block))
            && !block.range.is_empty() && block.containers.is_empty() && block.quote_depth == 0
            && !block.markdown_html && self.projection().table_at(block.range.start).is_none()
    }

    fn guarded_markdown_paragraphs(
        &self,
        mut run: Vec<super::super::Block>,
    ) -> Result<Option<(Vec<super::super::Block>, Vec<Range<usize>>)>, ModelTransactionError> {
        // Structural grammars may absorb a separator or a neighboring body.
        // Capture unchanged, independent paragraphs on both sides and prove
        // their complete semantics survive the edit. Walk bounded paragraph
        // records, never the document's full block array or a container index.
        let mut guards = Vec::new();
        if run.is_empty() { return Ok(None); }
        for preceding in [true, false] {
            loop {
                if run.len() >= 64 { return Ok(None); }
                let edge = if preceding { run[0].range.start } else { run.last().unwrap().range.end };
                let neighbor = if preceding { edge.checked_sub(1) }
                    else { (edge < self.projection().text_tree().byte_len()).then_some(edge + 1) };
                let Some(neighbor) = neighbor else { break; };
                let Some(block) = self.projection().blocks_for_region(&(neighbor..neighbor)).into_iter()
                    .find(|block| if preceding { block.range.end < edge } else { block.range.start > edge })
                    else { return Ok(None); };
                let guard = self.independent_markdown_paragraph(&block);
                if guard { guards.push(block.range.clone()); }
                if preceding { run.insert(0, block); } else { run.push(block); }
                if run.last().unwrap().range.end - run[0].range.start > 32768 { return Ok(None); }
                if guard { break; }
            }
        }
        Ok(Some((run, guards)))
    }

    /// Recover a bounded HTML parser input from complete projected owners.
    /// The independent edge paragraphs and the original parse prove that no
    /// unobserved HTML or list state enters this band from outside it.
    pub(super) fn guarded_markdown_input(
        &self,
        range: Range<usize>,
    ) -> Result<Option<super::super::line_endings::NormalizedText>, ModelTransactionError> {
        let run = self.projection().blocks_for_region(&range);
        let Some((run, _)) = self.guarded_markdown_paragraphs(run)? else { return Ok(None); };
        let formatted = run[0].range.start..run.last().unwrap().range.end;
        if self.markdown_edit_needs_reference_context(&[TextEdit::new(formatted.clone(), "")]) {
            return Ok(None);
        }
        for boundary in [formatted.start.checked_sub(1),
            (formatted.end < self.projection().text_tree().byte_len()).then_some(formatted.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return Ok(None); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, &[])? { return Ok(None); }
        }
        let Some(contributors) = self.projection().source_range(formatted.clone()) else { return Ok(None); };
        let start = if formatted.start == 0 { self.projection().source_content_start() } else {
            self.physical_line_at_source(contributors.start)?.source_range.start
        };
        let end = if formatted.end == self.projection().text_tree().byte_len() { self.source_byte_len() } else {
            let Some(separator) = self.projection().source_range(formatted.end..formatted.end + 1) else { return Ok(None); };
            separator.start
        };
        if end - start > 32768 { return Ok(None); }
        let bytes = self.source_bytes_in(start..end)?;
        let decoded = self.encoding().decode_region(&bytes, start)?;
        let input = normalize(&decoded, self.file_format());
        let local = super::super::projection::project_band(&input, self.format(), self.revision(),
            start, end, start == self.projection().source_content_start());
        if local.text() != self.projection().text_tree().slice(formatted.clone()).map_err(DocumentError::FormattedTextStorage)?
            || markdown_region_signature(self.projection(), &formatted)
                != markdown_region_signature(&local, &(0..local.text_tree().byte_len())) {
            return Ok(None);
        }
        Ok(Some(input))
    }

    fn prepare_paragraph_reprojection(
        &self,
        patches: &[SourcePatch],
        guarded: bool,
        expected: Option<(&[TextEdit], &FormattedTextTree)>,
    ) -> Result<Option<(PreparedModelTransaction, Range<usize>, Range<usize>)>, ModelTransactionError> {
        if !self.format().is_markdown() || patches.is_empty()
            || !guarded && self.format() != Format::Markdown {
            return candidate::decline(Strategy::IndependentParagraph, "not formatted Markdown or no patches");
        }
        // The edited paragraphs: one for edits inside a body, two adjacent
        // ones when an edit joins them across their separator.
        let first = &patches[0];
        let last = &patches[patches.len() - 1];
        let point = self.visible_point_for_source(first.range.start, true)?;
        let end_point = self.visible_point_for_source(last.range.end, false)?.max(point);
        let run = self.projection().blocks_for_region(&(point..end_point)).into_iter()
            .filter(|block| block.range.start <= end_point && point <= block.range.end)
            .collect::<Vec<_>>();
        let (run, guards) = if guarded {
            let Some(band) = self.guarded_markdown_paragraphs(run)? else { return candidate::decline(Strategy::IndependentParagraph, "no guarded paragraph band"); };
            band
        } else { (run, Vec::new()) };
        let (Some(first_block), Some(last_block)) = (run.first(), run.last()) else { return candidate::decline(Strategy::IndependentParagraph, "no edited paragraphs"); };
        let block_range = first_block.range.start..last_block.range.end;
        if (!guarded && (run.len() > 2 || !run.iter().all(|block| self.independent_markdown_paragraph(block))))
            || run.iter().any(|block| self.projection().table_at(block.range.start).is_some())
            || !(first_block.range.start <= point && end_point <= last_block.range.end)
            || self.markdown_range_has_references(&block_range)
        { return candidate::decline(Strategy::IndependentParagraph, "edited blocks are not independent prose"); }
        // Blank rows beside a list can still belong to its final item; an
        // unclosed HTML block can consume all later source. Neither neighbor
        // can be retained independently merely because two endings remain.
        // Headings, rules, tables, code blocks and a following list end at
        // or begin with their own syntax: across an explicit separator they
        // neither absorb nor are absorbed by a paragraph, and a standalone
        // parse below rejects any edit that turns the paragraphs themselves
        // into code, a list or HTML.
        let neighbors = block_range.start.saturating_sub(1)
            ..block_range.end.saturating_add(1).min(self.projection().text_tree().byte_len());
        if !guarded && (self.projection().blocks_for_region(&neighbors).iter().any(|neighbor| {
            if run.iter().any(|block| block.id == neighbor.id) { return false; }
            let preceding = neighbor.range.end <= block_range.start;
            neighbor.range.is_empty() || neighbor.markdown_html || neighbor.quote_depth != 0
                || preceding && (matches!(neighbor.kind, super::super::BlockKind::ListItem { .. })
                    || neighbor.containers.iter().any(|member|
                        member.container.kind != super::super::ContainerKind::CodeBlock))
                || neighbor.containers.iter().any(|member|
                    member.container.kind == super::super::ContainerKind::Quote)
        }) || markdown_html_edit::edit_changes_literal_html_grammar(
                self, &[TextEdit::new(block_range.clone(), "")])?)
        { return candidate::decline(Strategy::IndependentParagraph, "a neighbor can absorb or own the edited paragraphs"); }
        // Both outer separators must be explicit paragraph boundaries before
        // and after the edit. Endings inserted into a single ending (after a
        // heading interrupting a paragraph) pair with that ending, which the
        // regional parse cannot see.
        for boundary in [block_range.start.checked_sub(1),
            (block_range.end < self.projection().text_tree().byte_len()).then_some(block_range.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return candidate::decline(Strategy::IndependentParagraph, "separator source not found"); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, &[])?
                || !markdown_block_styles::explicit_paragraph_separator(self, &source, patches)?
            {
                return candidate::decline(Strategy::IndependentParagraph, "an outer separator is not explicit before and after the edit");
            }
        }
        let content = self.projection().source_range(block_range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_line = if block_range.start == 0 { 0 } else {
            self.state().source_hard_lines.line_at_offset(content.start)
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let last_line = self.state().source_hard_lines.line_at_offset(content.end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let trailing = self.state().source_hard_lines.get(last_line).ok_or(DocumentError::AmbiguousProjection)?;
        let start = if block_range.start == 0 { self.projection().source_content_start() } else {
            self.state().source_hard_lines.get(first_line).ok_or(DocumentError::AmbiguousProjection)?.start
        };
        let end = if block_range.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.projection().source_range(block_range.end..block_range.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?.start
        };
        let old_source = start..end;
        if old_source.len() > 32768 || patches.iter().any(|patch|
            patch.part != SourcePartId::PRIMARY || patch.range.start < start || patch.range.end > end)
        { return candidate::decline(Strategy::IndependentParagraph, "owner source exceeds the regional limit or the patches"); }
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_source = start..rebase_source_boundary(end, patches, Association::AfterInsertion)?;
        if new_source.len() > 65536 { return candidate::decline(Strategy::IndependentParagraph, "new owner source exceeds the regional limit"); }
        let revision = Revision(self.next_revision);
        let parse = |source: &super::super::source::SourceSnapshot, range: Range<usize>|
            -> Result<(FormattedDocument, Vec<Range<usize>>, usize), ModelTransactionError> {
            let bytes = source.bytes_in(range.clone()).ok_or(DocumentError::VerificationFailed)?;
            let decoded = self.encoding().decode_region(&bytes, range.start)?;
            let normalized = normalize(&decoded, self.file_format());
            // Terminal trivia can become an empty paragraph, including when
            // a code exit adds the first final ending. A guarded band captures
            // every source row and owner through EOF; its old semantic match
            // and unchanged preceding guard prove that no outside owner is
            // affected. Reparse that complete tail rather than requiring the
            // final ending to keep its old blank/nonblank role. Unguarded
            // bodies still need the broader boundary proof.
            if !guarded && normalized.endings.last()
                .is_some_and(|ending| range.end == source.len() && ending.source.end == range.end) {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            // A newly blank edge row joins the retained separator outside
            // this band. Its endings must be paired with that separator, not
            // interpreted as standalone leading/trailing trivia. Retry with
            // unchanged guard paragraphs so the complete blank run is parsed.
            if !guarded && (range.start > self.projection().source_content_start()
                && normalized.endings.first().is_some_and(|ending|
                    normalized.text[..ending.normalized.start].bytes()
                        .all(|byte| matches!(byte, b' ' | b'\t')))
                || range.end < source.len() && normalized.endings.last().is_some_and(|ending|
                    normalized.text[ending.normalized.end..].bytes()
                        .all(|byte| matches!(byte, b' ' | b'\t'))))
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            if !guarded && normalized.text.contains('<')
                || super::super::markdown_syntax::source_needs_reference_context(&normalized.text)
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let mut rows = Vec::new();
            let mut cursor = self.state().source_hard_lines.get(first_line)
                .ok_or(DocumentError::VerificationFailed)?.start;
            for ending in &normalized.endings {
                rows.push(cursor..ending.source.end);
                cursor = ending.source.end;
            }
            // Source row records own their ending even though the unedited
            // paragraph separator remains outside the regional parser input.
            let trailing_end = rebase_source_boundary(trailing.end, patches, Association::AfterInsertion)?;
            rows.push(cursor..trailing_end.max(range.end));
            let projection = super::super::projection::project_band(&normalized, self.format(), revision, range.start, range.end,
                range.start == self.projection().source_content_start());
            Ok((projection, rows, bytes.len()))
        };
        let (old_local, _, old_bytes) = parse(&self.state().source, old_source.clone())?;
        let (regional, source_rows, new_bytes) = parse(&source, new_source.clone())?;
        // The splice's half-open contributor range deliberately retains an
        // endpoint caret. If a structural owner becomes wholly empty, that
        // old caret can conflict with the new owner's insertion boundary.
        // Let the complete owner projection replace those empty identities.
        if guarded && regional.text_tree().byte_len() == 0 { return candidate::decline(Strategy::IndependentParagraph, "a structural owner becomes empty"); }
        let inherited_styles = self.projection().style_spans_for_region(&block_range);
        if inherited_styles.iter().any(|span|
            span.range.start < block_range.start || block_range.end < span.range.end)
        { return candidate::decline(Strategy::IndependentParagraph, "an inherited style crosses the band"); }
        let local_styles = inherited_styles.into_iter().map(|mut span| {
            span.range = span.range.start - block_range.start..span.range.end - block_range.start;
            span
        }).collect::<Vec<_>>();
        // Standalone parsing must reproduce the owner's complete original
        // semantics. New list/code/HTML owners require their broader grammar.
        if old_local.text() != self.projection().text_tree().slice(block_range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
            || old_local.style_span_records() != local_styles
            || old_local.block_records().len() != run.len()
            || old_local.block_records().iter().zip(&run).any(|(local, block)|
                local.kind != block.kind || local.style != block.style)
            || !guarded && regional.block_records().iter().any(|block| block.markdown_html
                || !block.containers.is_empty() || matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || block.style.0 == "Code Block")
        { return candidate::decline(Strategy::IndependentParagraph, "standalone parse differs from the document or creates structure"); }
        if guarded {
            // Matching the original standalone parse establishes the captured
            // owners' full context, including HTML scopes and list looseness.
            if markdown_region_signature(self.projection(), &block_range)
                != markdown_region_signature(&old_local, &(0..old_local.text_tree().byte_len())) {
                return candidate::decline(Strategy::IndependentParagraph, "guarded owner context differs");
            }
            for guard in guards {
                let old = guard.start - block_range.start..guard.end - block_range.start;
                let new = if guard.start == block_range.start { old.clone() } else {
                    let Some(start) = regional.text_tree().byte_len().checked_sub(guard.len()) else { return candidate::decline(Strategy::IndependentParagraph, "a guard paragraph changed"); };
                    start..regional.text_tree().byte_len()
                };
                if old_local.text_tree().slice(old.clone()).map_err(DocumentError::FormattedTextStorage)?
                    != regional.text_tree().slice(new.clone()).map_err(DocumentError::FormattedTextStorage)?
                    || markdown_region_signature(&old_local, &old)
                        != markdown_region_signature(&regional, &new) {
                    return candidate::decline(Strategy::IndependentParagraph, "a guard paragraph changed");
                }
            }
        }
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches)
            .into_iter().map(|edit| TextEdit::new(
                edit.range.start + block_range.start..edit.range.end + block_range.start,
                edit.replacement)).collect::<Vec<_>>();
        let edits = matching_requested_edits(expected, &block_range,
            self.projection().text_tree().byte_len(), &regional)?.unwrap_or(edits);
        let persistent = edits.iter().map(|edit| (edit.range.clone(), edit.replacement.as_str())).collect::<Vec<_>>();
        let (target_text, text_work) = self.projection().text_tree()
            .splice_prevalidated_batch_with_stats(&persistent).map_err(DocumentError::FormattedTextStorage)?;
        let first_hard = self.projection().hard_line_at_offset(block_range.start).ok_or(DocumentError::VerificationFailed)?;
        let last_hard = self.projection().hard_line_at_offset(block_range.end).ok_or(DocumentError::VerificationFailed)?;
        let new_formatted = block_range.start..block_range.start + regional.text_tree().byte_len();
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (projection, range_work) = splice_line_local_projection(self.projection(), regional, revision,
            first_hard..last_hard + 1, block_range.clone(), old_source, new_source,
            target_text, source.len(), false, true, &edits, patches, None, &mut next_id)
            .map_err(super::super::block_identity_document_error)?;
        let (source_hard_lines, source_work) = self.state().source_hard_lines
            .replace_ranges_with_stats(first_line..if end == self.source_byte_len() {
                self.state().source_hard_lines.len()
            } else { last_line + 1 }, &source_rows)
            .ok_or(DocumentError::VerificationFailed)?;
        if source_hard_lines.source_end() != source.len() { return Err(DocumentError::VerificationFailed.into()); }
        let splices = grapheme_closed_snapshot_map_splices(self.projection(), &projection, &edits)?;
        let map = PositionMap::for_text_snapshots(self.id, self.revision(), revision,
            self.projection(), &projection, splices)?;
        let formatted_splices = edits.iter().map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let state = DocumentState {
            transient_list_separator: None, revision, source, projection, source_hard_lines,
            encoding: self.encoding(), format: self.format(), file_format: self.file_format(),
            file_format_origin: self.file_format_origin(), line_ending_evidence: self.line_ending_evidence(),
            has_bom: self.state().has_bom,
        };
        Ok(Some((self.prepared(revision, ModelChangeSummary {
            kind: ModelChangeKind::TextEdits, source_patches: patches.to_vec(), formatted_splices,
            projection_work: ProjectionWorkStatistics::regional(old_bytes + new_bytes,
                projected_bytes, projected_lines, text_work, range_work, source_work),
            style_change: None, conversion_warnings: Vec::new(),
        }, map, None, next_id, PreparedPublication::State(state)), block_range, new_formatted)))
    }


    /// Reparse a complete list when its paragraph topology changes.
    /// Tight/loose spacing depends on all siblings, so a single-item restart
    /// cannot prove that the untouched siblings retain their presentation. The
    /// actual container bounds the work; a large list must not cause unrelated
    /// document content to be parsed by falling back at an arbitrary size cap.
    pub(super) fn prepare_list_paragraph_reprojection(
        &self,
        patches: &[SourcePatch],
        target_text: &FormattedTextTree,
        expected_edits: &[TextEdit],
    ) -> Result<Option<ListParagraphReprojection>, ModelTransactionError> {
        if self.format() != Format::Markdown || patches.is_empty() {
            return candidate::decline(Strategy::ListParagraph, "not formatted Markdown or no patches");
        }
        let point = self.visible_point_for_source(patches[0].range.start, true)?;
        let blocks = self.projection().list_indentation_blocks();
        let Some(index) = blocks.index_touching_point(point) else { return candidate::decline(Strategy::ListParagraph, "edit is not in a list"); };
        let Some(block) = blocks.get(index) else { return candidate::decline(Strategy::ListParagraph, "edit is not in a list"); };
        // Enclosing quotes and HTML may extend beyond the list. They require
        // their own complete restart, rather than borrowing an outside owner.
        let Some(root) = block.containers.first().filter(|member|
            matches!(member.container.kind, super::super::ContainerKind::List { .. }))
        else { return candidate::decline(Strategy::ListParagraph, "edit block has no enclosing list root"); };
        let owner = root.container.id;
        let owned = |block: &super::super::Block| block.containers.first()
            .is_some_and(|member| member.container.id == owner);
        let mut first = index;
        while first > 0 && blocks.get(first - 1).is_some_and(|block| owned(&block)) {
            first -= 1;
        }
        let mut end_index = index + 1;
        while end_index < blocks.len() && blocks.get(end_index).is_some_and(|block| owned(&block)) {
            end_index += 1;
        }
        let run = blocks.get_range(&(first..end_index)).ok_or(DocumentError::VerificationFailed)?;
        let block_range = run.first().unwrap().range.start..run.last().unwrap().range.end;
        let supported_body = |block: &super::super::Block| {
            (matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || block.kind == super::super::BlockKind::Paragraph && block.range.is_empty()
                    && block.containers.first().is_some_and(|member|
                        matches!(member.container.kind, super::super::ContainerKind::List { .. })))
                && !block.markdown_html && block.quote_depth == 0 && block.style.0 != "Code Block"
                && !block.thematic_break
        };
        if run.iter().any(|block| !supported_body(block)
            || self.projection().table_at(block.range.start).is_some())
        { return candidate::decline(Strategy::ListParagraph, "list contains unsupported bodies or tables"); }
        // A bounded list must begin/end at its own container edges. An
        // adjacent unclosed HTML/quote owner cannot be retained independently.
        if !run.first().unwrap().containers[0].starts_here
            || !run.last().unwrap().containers[0].ends_here
            || first.checked_sub(1).and_then(|i| blocks.get(i)).is_some_and(|block|
                block.markdown_html || block.quote_depth != 0)
            || blocks.get(end_index).is_some_and(|block| block.markdown_html || block.quote_depth != 0)
        { return candidate::decline(Strategy::ListParagraph, "list owner bounds are not independent"); }
        for boundary in [block_range.start.checked_sub(1),
            (block_range.end < self.projection().text_tree().byte_len()).then_some(block_range.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return candidate::decline(Strategy::ListParagraph, "separator source not found"); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, &[])?
                || !markdown_block_styles::explicit_paragraph_separator(self, &source, patches)?
            { return candidate::decline(Strategy::ListParagraph, "an outer separator is not explicit before and after the edit"); }
        }
        let content = self.projection().source_range(block_range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_line = self.state().source_hard_lines.line_at_offset(content.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last_line = self.state().source_hard_lines.line_at_offset(content.end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let trailing = self.state().source_hard_lines.get(last_line).ok_or(DocumentError::AmbiguousProjection)?;
        let start = self.state().source_hard_lines.get(first_line).ok_or(DocumentError::AmbiguousProjection)?.start;
        let end = if block_range.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.projection().source_range(block_range.end..block_range.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?.start
        };
        let old_source = start..end;
        if patches.iter().any(|patch|
            patch.part != SourcePartId::PRIMARY || patch.range.start < start || patch.range.end > end)
            || self.markdown_range_has_references(&block_range)
        { return candidate::decline(Strategy::ListParagraph, "patches extend beyond the list or need references"); }
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_source = start..rebase_source_boundary(end, patches, Association::AfterInsertion)?;
        let revision = Revision(self.next_revision);
        let parse = |source: &super::super::source::SourceSnapshot, range: Range<usize>|
            -> Result<(FormattedDocument, Vec<Range<usize>>, usize), ModelTransactionError> {
            let bytes = source.bytes_in(range.clone()).ok_or(DocumentError::VerificationFailed)?;
            let decoded = self.encoding().decode_region(&bytes, range.start)?;
            let normalized = normalize(&decoded, self.file_format());
            // The authoring transaction uses an empty passive span to retain
            // an editable empty paragraph. Other HTML/reference grammar may
            // depend on source outside this complete list and stays excluded.
            if normalized.text.replace("<span>", "").replace("</span>", "").contains('<')
                || super::super::markdown_syntax::source_needs_reference_context(&normalized.text)
            { return Err(DocumentError::UnsupportedFormatting.into()); }
            let mut rows = Vec::new();
            let mut cursor = range.start;
            for ending in &normalized.endings {
                rows.push(cursor..ending.source.end);
                cursor = ending.source.end;
            }
            let trailing_end = rebase_source_boundary(trailing.end, patches, Association::AfterInsertion)?;
            // A complete list at EOF also owns terminal trivia and its empty
            // final source row. Include that row in the same replacement so
            // inserting a paragraph preserves final-terminator ownership.
            rows.push(cursor..trailing_end.max(range.end));
            let projection = super::super::projection::project_band(&normalized, self.format(), revision, range.start, range.end,
                range.start == self.projection().source_content_start());
            Ok((projection, rows, bytes.len()))
        };
        let (old_local, _, old_bytes) = parse(&self.state().source, old_source.clone())?;
        let (regional, source_rows, new_bytes) = parse(&source, new_source.clone())?;
        let inherited_styles = self.projection().style_spans_for_region(&block_range);
        if inherited_styles.iter().any(|span|
            span.range.start < block_range.start || block_range.end < span.range.end)
        { return candidate::decline(Strategy::ListParagraph, "an inherited style crosses the list"); }
        let local_styles = inherited_styles.into_iter().map(|mut span| {
            span.range = span.range.start - block_range.start..span.range.end - block_range.start;
            span
        }).collect::<Vec<_>>();
        if old_local.text() != self.projection().text_tree().slice(block_range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
            || old_local.style_span_records() != local_styles
            || old_local.block_records().len() != run.len()
            || old_local.block_records().iter().zip(&run).any(|(local, block)|
                local.kind != block.kind || local.style != block.style
                || local.task_checkbox != block.task_checkbox || local.checkbox_insertable != block.checkbox_insertable
                || local.list_loose != block.list_loose || local.quote_depth != block.quote_depth
                || local.containers.len() != block.containers.len()
                || local.containers.iter().zip(block.containers.iter()).any(|(a, b)|
                    a.container.kind != b.container.kind || a.container.style != b.container.style
                    || a.starts_here != b.starts_here || a.ends_here != b.ends_here))
            || regional.block_records().iter().any(|block| !supported_body(block)
                && !(block.kind == super::super::BlockKind::Paragraph
                    && block.containers.is_empty() && block.quote_depth == 0
                    && !block.markdown_html && !block.thematic_break
                    && block.style.0 != "Code Block"))
        { return candidate::decline(Strategy::ListParagraph, "standalone list parse differs or creates structure"); }
        // Pasted paragraphs may divide this complete owner into several
        // lists. Container starts follow that parsed partition; the authored
        // item kinds, nesting and numbering must otherwise remain unchanged.
        let item_kind = |block: &super::super::Block| {
            let mut kind = block.kind.clone();
            if let super::super::BlockKind::ListItem { container_start, .. } = &mut kind {
                *container_start = false;
            }
            kind
        };
        // Speculative payloads can introduce an extra item while losing their
        // visible leading whitespace. The unchanged outer items and marker
        // prefixes still prove this complete parse; a text mismatch can be
        // rejected locally before enforcing the successful-candidate policy.
        let outer_markers_unchanged = || -> Result<bool, ModelTransactionError> {
            for block in [run.first().unwrap(), run.last().unwrap()] {
                let Some(content) = self.projection().source_range(block.range.clone()) else {
                    return Ok(false);
                };
                // Only the source-row start is needed; decoding its body
                // would add work proportional to an untouched long item.
                let line = self.state().source_hard_lines.line_at_offset(content.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let start = self.state().source_hard_lines.get(line)
                    .ok_or(DocumentError::AmbiguousProjection)?.start;
                let prefix = start..content.start;
                if patches.iter().any(|patch| patch.range.start < prefix.end
                    && prefix.start < patch.range.end
                    || patch.range.is_empty() && prefix.start <= patch.range.start
                        && patch.range.start < prefix.end) {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        // A changed outer marker could merge with a neighboring list despite
        // its blank separator. Keep the existing boundary syntax for both a
        // successful splice and a definitive speculative rejection.
        if !outer_markers_unchanged()? { return candidate::decline(Strategy::ListParagraph, "an outer list marker changed"); }
        if expected_edits.iter().all(|edit|
            block_range.start <= edit.range.start && edit.range.end <= block_range.end)
            && regional.block_records().first().is_some_and(|block| item_kind(block) == item_kind(run.first().unwrap()))
            && regional.block_records().last().is_some_and(|block| item_kind(block) == item_kind(run.last().unwrap()))
        {
            let retained_suffix = self.projection().text_tree().byte_len() - block_range.end;
            let expected_end = target_text.byte_len().checked_sub(retained_suffix)
                .ok_or(DocumentError::VerificationFailed)?;
            if expected_end < block_range.start
                || regional.text_tree().byte_len() != expected_end - block_range.start
                || regional.text_tree().slice(0..regional.text_tree().byte_len())
                    .map_err(DocumentError::FormattedTextStorage)?
                    != target_text.slice(block_range.start..expected_end)
                        .map_err(DocumentError::FormattedTextStorage)? {
                return Ok(Some(ListParagraphReprojection::TextMismatch));
            }
        }
        let item_kinds = |projection: &FormattedDocument| projection.block_records().iter()
            .filter(|block| matches!(block.kind, super::super::BlockKind::ListItem { item_start: true, .. }))
            .map(item_kind).collect::<Vec<_>>();
        if item_kinds(&old_local) != item_kinds(&regional) { return candidate::decline(Strategy::ListParagraph, "list item kinds changed"); }
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches)
            .into_iter().map(|edit| TextEdit::new(
                edit.range.start + block_range.start..edit.range.end + block_range.start,
                edit.replacement)).collect::<Vec<_>>();
        let edits = matching_requested_edits(Some((expected_edits, target_text)), &block_range,
            self.projection().text_tree().byte_len(), &regional)?.unwrap_or(edits);
        let persistent = edits.iter().map(|edit| (edit.range.clone(), edit.replacement.as_str())).collect::<Vec<_>>();
        let (target_text, text_work) = self.projection().text_tree()
            .splice_prevalidated_batch_with_stats(&persistent).map_err(DocumentError::FormattedTextStorage)?;
        let first_hard = self.projection().hard_line_at_offset(block_range.start).ok_or(DocumentError::VerificationFailed)?;
        let last_hard = self.projection().hard_line_at_offset(block_range.end).ok_or(DocumentError::VerificationFailed)?;
        let new_formatted = block_range.start..block_range.start + regional.text_tree().byte_len();
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (projection, range_work) = splice_line_local_projection(self.projection(), regional, revision,
            first_hard..last_hard + 1, block_range.clone(), old_source, new_source,
            target_text, source.len(), false, true, &edits, patches, None, &mut next_id)
            .map_err(super::super::block_identity_document_error)?;
        let (source_hard_lines, source_work) = self.state().source_hard_lines
            .replace_ranges_with_stats(first_line..if end == self.source_byte_len() {
                self.state().source_hard_lines.len()
            } else { last_line + 1 }, &source_rows)
            .ok_or(DocumentError::VerificationFailed)?;
        if source_hard_lines.source_end() != source.len() { return Err(DocumentError::VerificationFailed.into()); }
        let splices = grapheme_closed_snapshot_map_splices(self.projection(), &projection, &edits)?;
        let map = PositionMap::for_text_snapshots(self.id, self.revision(), revision,
            self.projection(), &projection, splices)?;
        let formatted_splices = edits.iter().map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let state = DocumentState {
            transient_list_separator: None, revision, source, projection, source_hard_lines,
            encoding: self.encoding(), format: self.format(), file_format: self.file_format(),
            file_format_origin: self.file_format_origin(), line_ending_evidence: self.line_ending_evidence(),
            has_bom: self.state().has_bom,
        };
        Ok(Some(ListParagraphReprojection::Candidate(self.prepared(revision, ModelChangeSummary {
            kind: ModelChangeKind::TextEdits, source_patches: patches.to_vec(), formatted_splices,
            projection_work: ProjectionWorkStatistics::regional(old_bytes + new_bytes,
                projected_bytes, projected_lines, text_work, range_work, source_work),
            style_change: None, conversion_warnings: Vec::new(),
        }, map, None, next_id, PreparedPublication::State(state)), block_range, new_formatted)))
    }
}
