//! Supporting fence conversion for edits an indented block cannot represent,
//! such as an empty body or an authored blank first/last code line.
use super::*;

impl Document {
    pub(super) fn prepare_with_fenced_indented_code(
        &self,
        blocks: &[super::super::Block],
        operation: impl FnOnce(&Document) -> Result<PreparedModelTransaction, ModelTransactionError>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut patches = Vec::new();
        for block in blocks {
            let Some(code) = super::super::markdown_indented_code::source_block(self, block)?
            else {
                continue;
            };
            let body = self
                .projection()
                .text_tree()
                .slice(block.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let bytes = self
                .state()
                .source
                .bytes_in(code.source.start..code.lines[0].content_start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = self
                .encoding()
                .decode_region(&bytes, code.source.start)?
                .text;
            let quote = super::super::markdown_quotes::prefix(&prefix);
            let marker =
                super::super::markdown_blocks::marker_prefix_length(&prefix[quote..]).unwrap_or(0);
            // Keep list/quote ownership while replacing only code indentation.
            let rest = format!("{}{}", &prefix[..quote], " ".repeat(code.container_indent));
            let first = if marker > 0 {
                prefix[..quote + marker].to_owned()
            } else {
                rest.clone()
            };
            let fence = "`".repeat(
                body.split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1,
            );
            let ending = self.file_format().spelling();
            let syntax = format!(
                "{first}{fence}{ending}{rest}{}{ending}{rest}{fence}",
                body.replace('\n', &format!("{ending}{rest}"))
            );
            patches.push(SourcePatch::primary(
                code.source,
                self.encoding().encode_fragment(&syntax)?,
            ));
        }
        self.prepare_markdown_supporting_patches(patches, operation)
    }

    pub(super) fn prepare_indented_code_text_edits(
        &self,
        edits: &[TextEdit],
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let blocks = self.indented_code_requiring_fences(edits)?;
        if blocks.is_empty() {
            return Ok(None);
        }
        self.prepare_with_fenced_indented_code(&blocks, |doc| {
            doc.prepare_text_edits(edits.to_vec())
        })
        .map(Some)
    }

    pub(super) fn indented_code_requiring_fences(
        &self,
        edits: &[TextEdit],
    ) -> Result<Vec<super::super::Block>, ModelTransactionError> {
        if self.format() != Format::Markdown {
            return Ok(Vec::new());
        }
        let mut blocks = Vec::new();
        for edit in edits {
            for block in self.projection().blocks_for_region(&edit.range) {
                if block.style.0 != "Code Block"
                    || blocks
                        .iter()
                        .any(|old: &super::super::Block| old.id == block.id)
                {
                    continue;
                }
                let contained =
                    block.range.start <= edit.range.start && edit.range.end <= block.range.end;
                let body = self
                    .projection()
                    .text_tree()
                    .slice(block.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                let mut after = body.clone();
                if contained {
                    after.replace_range(
                        edit.range.start - block.range.start..edit.range.end - block.range.start,
                        &edit.replacement,
                    );
                }
                let needs_fence = !contained
                    || after.is_empty()
                    || after
                        .split('\n')
                        .next()
                        .unwrap()
                        .trim_matches([' ', '\t'])
                        .is_empty()
                    || after
                        .split('\n')
                        .next_back()
                        .unwrap()
                        .trim_matches([' ', '\t'])
                        .is_empty();
                if let Some(code) =
                    super::super::markdown_indented_code::source_block(self, &block)?
                {
                    let leading = |text: &str| text.bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t')).count();
                    let changes_container = contained
                        && block.kind == super::super::BlockKind::Paragraph
                        && block.range.start > 0
                        && body[..leading(&body)] != after[..leading(&after)]
                        && self.indented_code_can_enter_previous_item(&block, &code, &after)?;
                    if needs_fence || changes_container || code.lines.iter().any(|line| line.padding > 0) {
                        blocks.push(block);
                    }
                }
            }
        }
        Ok(blocks)
    }

    /// Increasing the first code row's indentation can pull an outside code
    /// block into the preceding list item. A fence retains both literal body
    /// whitespace and the existing container boundary in that case.
    fn indented_code_can_enter_previous_item(
        &self,
        block: &super::super::Block,
        code: &super::super::markdown_indented_code::CodeBlock,
        after: &str,
    ) -> Result<bool, DocumentError> {
        let Some(previous) = self.projection()
            .blocks_for_region(&(block.range.start - 1..block.range.start))
            .into_iter().filter(|previous| previous.range.end < block.range.start)
            .max_by_key(|previous| previous.range.start)
            .filter(|previous| previous.quote_depth == block.quote_depth
                && matches!(previous.kind, super::super::BlockKind::ListItem { .. }))
        else { return Ok(false); };
        let at = super::super::source_edit::insertion_point(self.projection(), previous.range.start, None)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let row = self.state().source_hard_lines.line_at_offset(at)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        if at - row.start > 1024 { return Ok(false); }
        let bytes = self.state().source.bytes_in(row.start..at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = self.encoding().decode_region(&bytes, row.start)?.text;
        let prefix = &prefix[super::super::markdown_quotes::prefix(&prefix)..];
        let Some((_, content_indent)) = super::super::markdown_blocks::marker_prefix_geometry(prefix)
        else { return Ok(false); };
        if code.lines[0].content_start - code.source.start > 1024 { return Ok(false); }
        let bytes = self.state().source.bytes_in(code.source.start..code.lines[0].content_start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = self.encoding().decode_region(&bytes, code.source.start)?.text;
        let prefix = &prefix[super::super::markdown_quotes::prefix(&prefix)..];
        let columns = prefix.bytes().chain(after.bytes())
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .fold(0, |column, byte| column + if byte == b'\t' { 4 - column % 4 } else { 1 });
        Ok(columns >= content_indent)
    }
}
