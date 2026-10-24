//! Source wrapping is a spelling-only edit: its parser result must retain the
//! complete Preview meaning, including empty owners and their boundaries.
use super::*;

impl Document {
    pub(super) fn prepare_markdown_reflow_patches(
        &self,
        mut patches: Vec<SourcePatch>,
        regions: &[Range<usize>],
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if patches.is_empty() {
            return Ok(self.no_op_prepared());
        }
        validate_source_patches(&mut patches)?;
        let mut points = Vec::with_capacity(regions.len().saturating_mul(2));
        for region in regions {
            self.validate_range(region)?;
            points.extend([region.start, region.end]);
        }
        points.sort_unstable();
        points.dedup();
        if points.is_empty() {
            return Err(DocumentError::VerificationFailed.into());
        }
        // These are verification windows, not replacement intentions. Replacing
        // an unchanged paragraph with itself would collapse anchors inside it;
        // empty edits retain the identity map while requesting regional parsing.
        let edits = points.into_iter().map(|at| TextEdit::new(at..at, "")).collect();
        let prepared = self.prepare_text_edits_with_patch_policy(
            edits,
            Some(patches.clone()),
            true,
        )?;
        // The reflow planner authorizes whitespace and continuation prefixes
        // only. Shared supporting-syntax cleanup must not silently widen that
        // authority or normalize unrelated spelling.
        if prepared.summary.source_patches != patches {
            return Err(DocumentError::VerificationFailed.into());
        }
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let before = self.projection();
        let after = &candidate.projection;
        if before.text_tree().byte_len() != after.text_tree().byte_len() {
            return Err(DocumentError::VerificationFailed.into());
        }
        let region = match &prepared.reprojected_region {
            Some((old, new)) if old == new => old.clone(),
            Some(_) => return Err(DocumentError::VerificationFailed.into()),
            // A complete candidate has no proof that records outside the
            // requested range were retained. Check everything, in bounded
            // chunks, rather than trusting only the selected visible fragment.
            None => 0..before.text_tree().byte_len(),
        };
        verify_unchanged_preview(before, after, region)?;
        Ok(prepared)
    }
}

fn verify_unchanged_preview(
    before: &FormattedDocument,
    after: &FormattedDocument,
    region: Range<usize>,
) -> Result<(), ModelTransactionError> {
    const CHUNK_BYTES: usize = 16 * 1024;
    let mut start = region.start;
    loop {
        let mut end = start.saturating_add(CHUNK_BYTES).min(region.end);
        while end > start
            && !before.text_tree().is_char_boundary(end)
                .map_err(DocumentError::FormattedTextStorage)?
        {
            end -= 1;
        }
        if !after.text_tree().is_char_boundary(end)
            .map_err(DocumentError::FormattedTextStorage)?
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let window = start..end;
        let old_text = before.text_tree().slice(window.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let new_text = after.text_tree().slice(window.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        if old_text != new_text
            || before.hard_breaks_for_region(&window) != after.hard_breaks_for_region(&window)
            || before.style_spans_touching(&window) != after.style_spans_touching(&window)
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        // Touching queries intentionally include empty paragraphs and both
        // sides of an owner boundary. Text equality alone cannot distinguish a
        // rule from an empty paragraph or retain a quote's containment policy.
        let old_blocks = before.blocks_for_region(&window);
        let new_blocks = after.blocks_for_region(&window);
        if old_blocks.len() != new_blocks.len()
            || old_blocks.iter().zip(&new_blocks).any(|(old, new)| {
                old.id != new.id || old.range != new.range || old.attributes != new.attributes
            })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let mut old_images = before.inline_images_for_region(&window);
        let mut new_images = after.inline_images_for_region(&window);
        // Source byte coordinates legitimately shift when earlier whitespace
        // changes length. Every displayed property and destination must match.
        for image in old_images.iter_mut().chain(new_images.iter_mut()) {
            image.source = 0..0;
        }
        if old_images != new_images {
            return Err(DocumentError::VerificationFailed.into());
        }
        if end == region.end {
            break;
        }
        start = end;
    }
    Ok(())
}
