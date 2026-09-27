//! Explicit whole-document content deletion. Unlike deleting the last glyph,
//! this intention owns the surrounding content scopes and removes them too.
use super::*;

impl Document {
    pub(super) fn prepare_clear_document_content(
        &self,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let bytes = self.source_bytes();
        let decoded = self.encoding().decode(&bytes)?;
        let input = normalize(&decoded, self.file_format());
        let converter = super::super::rich_text::Builder::new(&input, Revision(0));
        let mut keep = vec![0..decoded.bom_len];
        match self.format() {
            Format::Rtf => {
                use super::super::rtf::{self, Kind};
                let tokens = rtf::tokenize(&input);
                let mut stack = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    match &token.kind {
                        Kind::Open => {
                            if stack.is_empty() {
                                keep.push(converter.source_range(token.range.clone()));
                            }
                            stack.push(index);
                        }
                        Kind::Close => {
                            if let Some(open) = stack.pop() {
                                if stack.is_empty() {
                                    keep.push(converter.source_range(token.range.clone()));
                                } else if tokens[open + 1..index].iter().take(2).any(|token| {
                                    matches!(&token.kind, Kind::Symbol('*')) || matches!(&token.kind,
                                        Kind::Control(name, _) if matches!(name.as_str(),
                                            "fonttbl" | "colortbl" | "stylesheet" | "listtable" |
                                            "listoverridetable" | "info" | "generator"))
                                }) && !tokens[open + 1..index].iter().take(2).any(|token| {
                                    matches!(&token.kind, Kind::Control(name, _) if name == "pn")
                                }) {
                                    keep.push(converter.source_range(tokens[open].range.start..token.range.end));
                                }
                            }
                        }
                        Kind::Control(name, _)
                            if stack.len() <= 1
                                && matches!(
                                    name.as_str(),
                                    "rtf"
                                        | "ansi"
                                        | "mac"
                                        | "pc"
                                        | "pca"
                                        | "ansicpg"
                                        | "deff"
                                        | "deflang"
                                        | "deflangfe"
                                        | "adeflang"
                                        | "uc"
                                        | "deftab"
                                        | "paperw"
                                        | "paperh"
                                        | "margl"
                                        | "margr"
                                        | "margt"
                                        | "margb"
                                        | "landscape"
                                        | "viewkind"
                                        | "viewscale"
                                ) =>
                        {
                            keep.push(converter.source_range(token.range.clone()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
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
