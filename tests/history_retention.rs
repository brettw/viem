use evim_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactWriteIntent,
    Document, Encoding, Format, HistoryError, HistoryRetentionPolicy, InMemoryArtifactStorage, SourceArtifactDigest,
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

#[test]
fn byte_budget_charges_derived_unicode_projections_and_history_maps() {
    let source = "éπa".repeat(1000);
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut unbounded =
        Document::from_bytes(bytes.clone(), Encoding::Utf16Le, Format::PlainText).unwrap();
    unbounded.set_history_retention_policy(HistoryRetentionPolicy::unlimited());
    let initial = unbounded.history_status().retained_memory_bytes;
    for index in 0..80 {
        unbounded.replace(0..2, if index % 2 == 0 { "δ" } else { "é" }).unwrap();
    }
    let unbounded_status = unbounded.history_status();
    assert!(unbounded_status.retained_memory_bytes > initial);
    // Calibrate a genuinely constraining heap budget. Persistent sharing can
    // legitimately fit many history nodes into a fixed multiple of the first
    // snapshot; a prescribed node count would test the allocation strategy.
    let budget = initial + (unbounded_status.retained_memory_bytes - initial) / 2;
    assert!(unbounded_status.retained_source_bytes < budget);

    let mut document = Document::from_bytes(bytes, Encoding::Utf16Le, Format::PlainText).unwrap();
    let root = document.history_status().current;
    document.set_history_retention_policy(HistoryRetentionPolicy::new(usize::MAX, budget));
    for index in 0..80 {
        document.replace(0..2, if index % 2 == 0 { "δ" } else { "é" }).unwrap();
        let retained = document.history_status().retained_memory_bytes;
        assert!(retained <= budget, "edit {index}: {retained} > {budget}");
    }
    let status = document.history_status();
    assert!(status.retained_source_bytes < 10_000);
    assert!(status.retained_memory_bytes > status.retained_source_bytes * 10);
    assert!(status.retained_memory_bytes <= budget, "{} > {budget}", status.retained_memory_bytes);
    assert!(status.node_count < unbounded_status.node_count);
    assert!(status.can_undo);
    assert_eq!(document.select_history_node(root.node), Err(HistoryError::NodeNotFound(root.node)));
    let final_bytes = document.source_bytes();
    assert_eq!(final_bytes, unbounded.source_bytes());
    document.try_undo().unwrap();
    document.try_redo().unwrap();
    assert_eq!(document.source_bytes(), final_bytes);
}

#[test]
fn tiny_memory_budget_keeps_open_group_parent_and_result_until_finalized() {
    let mut document = Document::new("base");
    document.set_history_retention_policy(HistoryRetentionPolicy::new(usize::MAX, 1));
    document.begin_edit_group();
    document.insert(4, "!").unwrap();
    document.insert(5, "?").unwrap();
    assert_eq!(document.history_status().node_count, 2);
    assert!(document.history_status().retained_memory_bytes > 1);
    document.try_undo().unwrap();
    assert_eq!(document.text(), "base");
    assert_eq!(document.history_status().node_count, 1);
}

#[test]
fn local_edit_in_large_document_shares_retained_projection_allocations() {
    let mut document = Document::new("paragraph text\n".repeat(20_000));
    document.set_history_retention_policy(HistoryRetentionPolicy::unlimited());
    let before = document.history_status().retained_memory_bytes;
    document.replace(150_000..150_001, "x").unwrap();
    let after = document.history_status().retained_memory_bytes;
    assert!(after - before < before / 3, "shared projection charged repeatedly: {before} -> {after}");
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    assert!(document.history_status().retained_memory_bytes < after);
}


#[test]
fn async_save_completion_stays_clean_after_pruned_configuration_only_edits() {
    use evim_core::document::*;
    let source = b"<p>Words</p>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let saved = document.history_status().current;
    let prepared = document.prepare_artifact_write(ArtifactWriteIntent::SaveAs {
        destination: ArtifactPath::from("configuration-save"),
        overwrite: ArtifactOverwrite::ReplaceExisting,
    }).unwrap();
    for size in [18., 24.] {
        let mut style = document.projection().style_sheet().block_style(&"Document".into()).unwrap().clone();
        style.character.size = Some(size);
        document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(style),
            }),
        )).unwrap();
    }
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    assert!(!document.history_status().save_point_retained);
    assert_eq!(document.source_bytes(), source);
    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    document.complete_artifact_write(prepared.succeeded(receipt)).unwrap();
    assert_eq!(document.history_status().save_point, saved.node);
    assert!(!document.is_dirty());
    document.insert(0, "a").unwrap();
    assert!(document.is_dirty());
}
