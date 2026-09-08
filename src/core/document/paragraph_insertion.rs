//! Native paragraph-menu insertion at the end of ordinary prose. Both the
//! split and style assignment are verified on scratch state, then published
//! as one local source transaction.
use super::replacement::PatchComposition;
use super::*;

impl Document {
    pub(crate) fn prepare_quote_after_paragraph(
        &self,
        at: usize,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !matches!(self.format(), Format::Html | Format::Markdown)
            || at != self.projection().text_tree().byte_len()
        {
            return Ok(None);
        }
        let Some(block) = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| block.range.start <= at && block.range.end == at)
        else {
            return Ok(None);
        };
        if block.range.is_empty()
            || block.style != self.projection().style_sheet().base_paragraph
            || !matches!(block.kind, super::super::BlockKind::Paragraph)
        {
            return Ok(None);
        }
        let mut scratch = structural_style::scratch_document(self);
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(at);
        // An empty final row within this same paragraph already supplies the
        // desired line. Replace its internal break with a paragraph boundary.
        if at > block.range.start
            && self
                .projection()
                .text_tree()
                .slice(at - 1..at)
                .is_ok_and(|text| text == "\n")
        {
            let prepared = scratch
                .prepare_text_edits_with_patches(vec![TextEdit::new(at - 1..at, "")], None)?;
            record(&mut scratch, prepared, &mut sources, &mut formatted)?;
        }
        let end = scratch.projection().text_tree().byte_len();
        let split = if self.format() == Format::Markdown {
            scratch.prepare_text_edits_with_patches(vec![TextEdit::new(end..end, "\n")], None)?
        } else {
            scratch.prepare_rich_list_enter(end)?
        };
        record(&mut scratch, split, &mut sources, &mut formatted)?;
        let end = scratch.projection().text_tree().byte_len();
        let styled = scratch.prepare_model_request(ModelRequest::AssignNamedStyle {
            document: scratch.id(),
            revision: scratch.revision(),
            range: end..end,
            namespace: super::super::StyleNamespace::Block,
            style: "Block quote".into(),
        })?;
        record(&mut scratch, styled, &mut sources, &mut formatted)?;
        if !scratch
            .projection()
            .blocks_for_region(&(end..end))
            .iter()
            .any(|block| block.range == (end..end) && block.style.0 == "Block quote")
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let patches = sources.source_patches(&scratch.state().source)?;
        let edits = formatted.formatted_edits(&scratch)?;
        // Replacing an internal break with a paragraph boundary deliberately
        // changes block ownership without changing flat text. A source-only
        // style transaction forbids that structural change, so verify this
        // explicit structural intention against the independently prepared
        // scratch result instead.
        let prepared = if scratch.text() == self.text() {
            self.prepare_reprojected_source_patches(patches)?
        } else {
            self.prepare_text_edits_with_patches(edits, Some(patches))?
        };
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let actual_blocks = candidate.projection.blocks();
        let expected_blocks = scratch.projection().blocks();
        if candidate.projection.text() != scratch.text()
            || candidate.projection.style_spans() != scratch.projection().style_spans()
            || !candidate
                .projection
                .has_same_hard_line_structure(scratch.projection())
            || actual_blocks.len() != expected_blocks.len()
            || actual_blocks
                .iter()
                .zip(expected_blocks)
                .any(|(actual, expected)| {
                    actual.range != expected.range
                        || actual.kind != expected.kind
                        || actual.style != expected.style
                        || actual.direct_paragraph != expected.direct_paragraph
                        || actual.direct_default_character != expected.direct_default_character
                })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        self.prepared_text_point(&prepared, end)?;
        Ok(Some(prepared))
    }
}

fn record(
    scratch: &mut Document,
    prepared: PreparedModelTransaction,
    sources: &mut PatchComposition,
    formatted: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    formatted.record_formatted(&prepared)?;
    scratch.commit_model_transaction(prepared)?;
    Ok(())
}
