use evim_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactWriteIntent,
    Document, HistoryError, HistoryRetentionPolicy, InMemoryArtifactStorage, SourceArtifactDigest,
};

#[test]
fn document_accounts_unique_structurally_shared_source_buffers() {
    let mut document = Document::new("abcdef");
    assert_eq!(document.history_status().retained_source_bytes, 6);

    document.replace(2..4, "XYZ").unwrap();
    assert_eq!(document.source_bytes(), b"abXYZef");
    assert_eq!(
        document.history_status().retained_source_bytes,
        9,
        "the six-byte original allocation and three-byte insertion are each charged once"
    );
}

#[test]
fn public_node_budget_promotes_current_ancestry_and_keeps_typed_ids_stale() {
    let mut document = Document::new("");
    let root = document.history_status().current;
    document.insert(0, "a").unwrap();
    document.insert(1, "b").unwrap();
    document.insert(2, "c").unwrap();

    let policy = HistoryRetentionPolicy::new(3, usize::MAX);
    document.set_history_retention_policy(policy);
    assert_eq!(document.history_retention_policy(), policy);
    assert_eq!(document.history_status().node_count, 3);
    assert_eq!(document.text(), "abc");
    assert_eq!(
        document.select_history_node(root.node),
        Err(HistoryError::NodeNotFound(root.node))
    );
    document.try_undo().unwrap();
    document.try_undo().unwrap();
    assert_eq!(document.text(), "a");
}

#[test]
fn undo_can_install_an_open_units_parent_before_pruning_to_tiny_budget() {
    let mut document = Document::new("base");
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    document.begin_edit_group();
    document.insert(4, "!").unwrap();
    assert_eq!(document.history_status().node_count, 2);
    assert!(document.history_status().can_undo);

    document.try_undo().unwrap();
    assert_eq!(document.text(), "base");
    assert_eq!(document.history_status().node_count, 1);
    assert!(!document.history_status().can_redo);
}

#[test]
fn async_save_retains_identity_and_digest_after_its_node_is_pruned() {
    let mut document = Document::new("base");
    document.insert(4, " one").unwrap();
    let captured = document.history_status().current;
    let prepared = document
        .prepare_artifact_write(ArtifactWriteIntent::SaveAs {
            destination: ArtifactPath::from("pruned-save"),
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();

    document.try_undo().unwrap();
    document.insert(4, " two").unwrap();
    document.set_history_retention_policy(HistoryRetentionPolicy::new(2, usize::MAX));
    assert_eq!(
        document.select_history_node(captured.node),
        Err(HistoryError::NodeNotFound(captured.node))
    );

    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    document
        .complete_artifact_write(prepared.succeeded(receipt))
        .unwrap();

    let status = document.history_status();
    assert_eq!(status.save_point, captured.node);
    assert!(!status.save_point_retained);
    assert_eq!(
        status.save_point_digest,
        Some(SourceArtifactDigest::from_bytes(b"base one"))
    );
    assert!(status.is_dirty);
}
