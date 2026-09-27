//! Command publication rollback over immutable model states and a bounded
//! history mutation journal. This extends the model commit boundary through
//! controller cursor/anchor validation without copying the undo tree.

use super::{
    ArtifactBinding, ArtifactWriteToken, Document, DocumentId, PendingArtifactWrite, PositionMap,
};
use std::collections::HashMap;

pub(crate) struct DocumentCommandCheckpoint {
    document: DocumentId,
    depth: usize,
    next_revision: u64,
    next_projected_block_id: u64,
    edit_group_depth: usize,
    edit_group_generation: u64,
    position_map_capture: Option<PositionMap>,
    code_presentation: Option<super::code_presentation::CodePresentation>,
    configuration_state: Option<super::DocumentState>,
    artifact_binding: Option<ArtifactBinding>,
    pending_artifact_writes: HashMap<ArtifactWriteToken, PendingArtifactWrite>,
    next_artifact_write_token: u64,
    next_save_sequence: u64,
    last_successful_save_sequence: u64,
    read_only: bool,
    recovered_dirty: bool,
}

impl Document {
    pub(crate) fn has_command_checkpoint(&self) -> bool {
        self.history.has_command_checkpoint()
    }

    /// Begin a publication boundary for one normalized input event. Nested
    /// replay commands retain independent rollback boundaries so a failed
    /// command can be discarded while preserving its successful prefix.
    pub(crate) fn begin_command_checkpoint(&mut self) -> DocumentCommandCheckpoint {
        self.history.begin_command_checkpoint();
        debug_assert!(self.has_command_checkpoint());
        DocumentCommandCheckpoint {
            document: self.id,
            depth: self.history.command_checkpoint_depth(),
            next_revision: self.next_revision,
            next_projected_block_id: self.next_projected_block_id,
            edit_group_depth: self.edit_group_depth,
            edit_group_generation: self.edit_group_generation,
            position_map_capture: self.position_map_capture.clone(),
            code_presentation: self.code_presentation.clone(),
            configuration_state: self.configuration_state.clone(),
            artifact_binding: self.artifact_binding.clone(),
            pending_artifact_writes: self.pending_artifact_writes.clone(),
            next_artifact_write_token: self.next_artifact_write_token,
            next_save_sequence: self.next_save_sequence,
            last_successful_save_sequence: self.last_successful_save_sequence,
            read_only: self.read_only,
            recovered_dirty: self.recovered_dirty,
        }
    }

    pub(crate) fn commit_command_checkpoint(&mut self, checkpoint: DocumentCommandCheckpoint) {
        assert_eq!(
            checkpoint.document, self.id,
            "checkpoint belongs to this document"
        );
        assert_eq!(
            checkpoint.depth,
            self.history.command_checkpoint_depth(),
            "checkpoints finish in stack order"
        );
        self.history.commit_command_checkpoint();
    }

    pub(crate) fn rollback_command_checkpoint(&mut self, checkpoint: DocumentCommandCheckpoint) {
        assert_eq!(
            checkpoint.document, self.id,
            "checkpoint belongs to this document"
        );
        assert_eq!(
            checkpoint.depth,
            self.history.command_checkpoint_depth(),
            "checkpoints finish in stack order"
        );
        self.history.rollback_command_checkpoint();
        self.next_revision = checkpoint.next_revision;
        self.next_projected_block_id = checkpoint.next_projected_block_id;
        self.edit_group_depth = checkpoint.edit_group_depth;
        self.edit_group_generation = checkpoint.edit_group_generation;
        self.position_map_capture = checkpoint.position_map_capture;
        self.code_presentation = checkpoint.code_presentation;
        self.configuration_state = checkpoint.configuration_state;
        self.artifact_binding = checkpoint.artifact_binding;
        self.pending_artifact_writes = checkpoint.pending_artifact_writes;
        self.next_artifact_write_token = checkpoint.next_artifact_write_token;
        self.next_save_sequence = checkpoint.next_save_sequence;
        self.last_successful_save_sequence = checkpoint.last_successful_save_sequence;
        self.read_only = checkpoint.read_only;
        self.recovered_dirty = checkpoint.recovered_dirty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::HistoryRetentionPolicy;

    #[test]
    fn nested_command_checkpoint_rolls_back_failed_event_and_preserves_prefix() {
        let mut document = Document::new("alpha beta");
        let replay = document.begin_command_checkpoint();
        document.begin_edit_group();
        let first = document.begin_command_checkpoint();
        document.replace(0..5, "ALPHA").unwrap();
        document.commit_command_checkpoint(first);
        let prefix_revision = document.revision();
        let prefix_history = document.history_status();
        let prefix_details = document
            .history_node_details(prefix_history.current.node)
            .unwrap();
        let failed = document.begin_command_checkpoint();
        document.replace(6..10, "BETA").unwrap();
        document.close_edit_group();
        document.replace(0..5, "OMEGA").unwrap();
        document.rollback_command_checkpoint(failed);
        assert_eq!(document.text(), "ALPHA beta");
        assert_eq!(document.revision(), prefix_revision);
        assert_eq!(document.history_status(), prefix_history);
        assert_eq!(
            document
                .history_node_details(prefix_history.current.node)
                .unwrap(),
            prefix_details
        );
        assert_eq!(document.edit_group_depth, 1);
        document.end_edit_group();
        document.commit_command_checkpoint(replay);
        document.history.assert_memory_matches_full_recount();
        document.try_undo().unwrap();
        assert_eq!(document.text(), "alpha beta");
        document.try_redo().unwrap();
        assert_eq!(document.text(), "ALPHA beta");
    }

    #[test]
    fn outer_command_checkpoint_rolls_back_successful_nested_group_edits() {
        let mut document = Document::new("alpha beta");
        document.begin_edit_group();
        document.replace(0..5, "ALPHA").unwrap();
        let before_revision = document.revision();
        let before_history = document.history_status();
        let outer = document.begin_command_checkpoint();
        let inner = document.begin_command_checkpoint();
        document.replace(6..10, "BETA").unwrap();
        document.commit_command_checkpoint(inner);
        let failed = document.begin_command_checkpoint();
        document.replace(0..5, "OMEGA").unwrap();
        document.rollback_command_checkpoint(failed);
        assert_eq!(document.text(), "ALPHA BETA");
        document.rollback_command_checkpoint(outer);
        assert_eq!(document.text(), "ALPHA beta");
        assert_eq!(document.revision(), before_revision);
        assert_eq!(document.history_status(), before_history);
        document.history.assert_memory_matches_full_recount();
    }

    #[test]
    fn command_checkpoint_restores_history_navigation_and_redo_preferences() {
        let mut document = Document::new("a");
        document.replace(0..1, "b").unwrap();
        document.replace(0..1, "c").unwrap();
        let c = document.history_status().current;
        document.try_undo().unwrap();
        document.replace(0..1, "d").unwrap();
        document.try_undo().unwrap();
        let before = document.history_status();
        let branches = document.redo_branches();
        let details = branches
            .iter()
            .map(|branch| {
                document
                    .history_node_details(branch.destination.node)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let checkpoint = document.begin_command_checkpoint();
        document.select_history_node(c.node).unwrap();
        document.replace(0..1, "e").unwrap();
        document.mark_saved();
        document.set_history_retention_policy(HistoryRetentionPolicy::new(1, 1));
        document.rollback_command_checkpoint(checkpoint);
        assert_eq!(document.text(), "b");
        assert_eq!(document.history_status(), before);
        assert_eq!(document.redo_branches(), branches);
        for detail in details {
            assert_eq!(
                document.history_node_details(detail.location.node).unwrap(),
                detail
            );
        }
        document.history.assert_memory_matches_full_recount();
        document.try_redo().unwrap();
        assert_eq!(document.text(), "d");
    }

    #[test]
    fn command_checkpoint_restores_group_setup_and_position_capture_after_error() {
        let mut document = Document::new("é");
        let revision = document.revision();
        let status = document.history_status();
        let generation = document.edit_group_generation;
        let (result, map) = document.capture_position_maps(|document| {
            let checkpoint = document.begin_command_checkpoint();
            document.begin_edit_group();
            document.replace(0..2, "abc").unwrap();
            document.rollback_command_checkpoint(checkpoint);
            Err::<(), _>(super::super::DocumentError::NotGraphemeBoundary(1))
        });
        assert!(result.is_err());
        assert_eq!(document.text(), "é");
        assert_eq!(document.history_status(), status);
        assert_eq!(document.edit_group_depth, 0);
        assert_eq!(document.edit_group_generation, generation);
        assert_eq!(map.source_revision(), revision);
        assert_eq!(map.target_revision(), revision);
        document.history.assert_memory_matches_full_recount();
    }
}
