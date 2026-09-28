//! Explicit whole-document content deletion. Unlike deleting the last glyph,
//! this intention owns the surrounding content scopes and removes them too.
use super::*;

impl Document {
    pub(super) fn prepare_clear_document_content(
        &self,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let bytes = self.source_bytes();
        let decoded = self.encoding().decode(&bytes)?;
        let mut keep = vec![0..decoded.bom_len];
        keep.sort_by_key(|range| (range.start, range.end));
        let mut patches = Vec::new();
        let mut at = 0;
        for retained in keep {
            if retained.start > at {
                patches.push(SourcePatch::primary(at..retained.start, Vec::new()));
            }
            at = at.max(retained.end);
        }
        if at < bytes.len() {
            patches.push(SourcePatch::primary(at..bytes.len(), Vec::new()));
        }

        patches.retain(|patch| bytes.get(patch.range()) != Some(patch.replacement()));
        let prepared = self.prepare_text_edits_with_patches(
            vec![TextEdit::new(
                0..self.projection().text_tree().byte_len(),
                "",
            )],
            Some(patches),
        )?;
        let projection = match &prepared.publication {
            PreparedPublication::State(state) => &state.projection,
            PreparedPublication::NoOp => self.projection(),
            _ => return Err(DocumentError::VerificationFailed.into()),
        };
        if projection.text_tree().byte_len() != 0
            || projection.blocks().iter().any(|block| {
                (block.style == StyleId::from("Block quote") || block.quote_depth > 0)
                    || block.style.is_internal_list()
                    || matches!(block.kind, super::super::BlockKind::ListItem { .. })
            })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }
}
