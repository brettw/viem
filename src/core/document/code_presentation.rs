//! Exact edit lineage for the disposable Code presentation. Analysis caches
//! may reuse its mapped appearance while workers analyze the new input; this
//! map never promotes old captures to current provider coverage.
use super::{Document, FormattedDocument, PositionMap};

const MAX_PRESENTATION_MAP_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub(super) struct CodePresentation {
    pub(super) projection: FormattedDocument,
    pub(super) transition: PositionMap,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        code_style,
        syntax::{SyntaxRun, SyntaxStyleName},
        Encoding, Format,
    };
    use std::sync::Arc;

    #[test]
    fn presentation_lineage_follows_commits_history_and_command_rollback() {
        let mut document =
            Document::from_bytes(b"comment text".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        document.install_code_presentation(
            Arc::new(code_style::default_sheet()),
            &[SyntaxRun {
                range: 0..7,
                name: SyntaxStyleName("Comment".into()),
                origin: "test".into(),
                priority: 0,
            }],
        );
        let published = document.revision();
        let checkpoint = document.begin_command_checkpoint();
        document.replace(0..0, "new ").unwrap();
        let map = document.code_presentation_change_map().unwrap();
        assert_eq!(map.source_revision(), published);
        assert_eq!(map.target_revision(), document.revision());
        document.rollback_command_checkpoint(checkpoint);
        assert_eq!(
            document
                .code_presentation_change_map()
                .unwrap()
                .target_revision(),
            published
        );
        assert_eq!(document.projection().style_spans().len(), 1);
        document.replace(0..0, "new ").unwrap();
        let changed = document.revision();
        document.try_undo().unwrap();
        assert_eq!(
            document
                .code_presentation_change_map()
                .unwrap()
                .target_revision(),
            published
        );
        document.try_redo().unwrap();
        assert_eq!(
            document
                .code_presentation_change_map()
                .unwrap()
                .target_revision(),
            changed
        );
        assert_eq!(
            document
                .code_presentation_change_map()
                .unwrap()
                .source_revision(),
            published
        );
    }

    #[test]
    fn unpolled_edit_bursts_evict_optional_lineage_before_it_grows_without_bound() {
        let mut document =
            Document::from_bytes("a ".repeat(1024).into_bytes(), Encoding::Utf8, Format::Code)
                .unwrap();
        document.install_code_presentation(Arc::new(code_style::default_sheet()), &[]);
        for index in 0..1024 {
            document.replace(index * 2..index * 2 + 1, "b").unwrap();
            let Some(map) = document.code_presentation_change_map() else {
                assert!(document.format().is_code());
                assert_eq!(document.projection().text_tree().byte_len(), 2048);
                return;
            };
            assert!(map.owned_heap_bytes() <= MAX_PRESENTATION_MAP_BYTES);
        }
        panic!("A long burst of independent edits must evict its optional presentation lineage");
    }
}

impl Document {
    pub(crate) fn code_presentation_change_map(&self) -> Option<&PositionMap> {
        self.code_presentation.as_ref().map(|p| &p.transition)
    }

    pub(super) fn advance_code_presentation(&mut self, map: &PositionMap) {
        if !self.format().is_code() {
            self.code_presentation = None;
            return;
        }
        let Some(presentation) = self.code_presentation.as_mut() else {
            return;
        };
        if presentation
            .transition
            .owned_heap_bytes()
            .saturating_add(map.owned_heap_bytes())
            > MAX_PRESENTATION_MAP_BYTES
        {
            self.code_presentation = None;
            return;
        }
        match presentation.transition.then(map) {
            Ok(transition) if transition.owned_heap_bytes() <= MAX_PRESENTATION_MAP_BYTES => {
                presentation.transition = transition
            }
            // No exact lineage means no presentation reuse. An old numeric
            // offset is never interpreted in an unrelated source revision.
            Ok(_) | Err(_) => self.code_presentation = None,
        }
    }
}
