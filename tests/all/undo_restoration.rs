use viem_core::document::{
    BomPolicy, Document, Encoding, FileFormat, Format, HistorySemanticChangeKind,
    SourceArtifactDigest,
};

#[test]
fn grouped_history_retains_ordered_exact_transactions_without_copying_patch_payloads() {
    let mut document = Document::new("base");
    document.begin_edit_group();
    document.insert(4, "-one").unwrap();
    document.insert(8, "-two").unwrap();
    document.end_edit_group();

    let node = document.history_status().current;
    let details = document.history_node_details(node.node).unwrap();
    assert_eq!(details.location, node);
    assert_eq!(
        details.semantic.unwrap().changes(),
        [
            HistorySemanticChangeKind::Text,
            HistorySemanticChangeKind::Text
        ]
    );
    assert_eq!(details.transactions.len(), 2);
    assert_eq!(details.transactions[0].before_revision().0, 0);
    assert_eq!(details.transactions[0].after_revision().0, 1);
    assert_eq!(details.transactions[1].before_revision().0, 1);
    assert_eq!(details.transactions[1].after_revision().0, 2);

    let first_patch = &details.transactions[0].source_patches()[0];
    assert_eq!(first_patch.range(), 4..4);
    assert_eq!(first_patch.replacement_len(), 4);
    assert_eq!(
        first_patch.replacement_digest(),
        SourceArtifactDigest::from_bytes(b"-one")
    );
    let second_patch = &details.transactions[1].source_patches()[0];
    assert_eq!(second_patch.range(), 8..8);
    assert_eq!(second_patch.replacement_len(), 4);
    assert_eq!(
        second_patch.replacement_digest(),
        SourceArtifactDigest::from_bytes(b"-two")
    );

    let restoration = details.restoration.unwrap();
    assert_eq!(restoration.before().cursor().revision().0, 0);
    assert_eq!(restoration.after().cursor().revision().0, 2);
}

#[test]
fn history_navigation_restores_exact_source_and_interpretation_metadata() {
    let original = b"**one**\r\n\r\ntwo".to_vec();
    let mut document = Document::from_bytes_with_file_format(
        original.clone(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    let original_evidence = document.line_ending_evidence();
    let original_origin = document.file_format_origin();

    document.set_file_format(FileFormat::Mac).unwrap();
    let converted = document.source_bytes();
    let converted_evidence = document.line_ending_evidence();
    assert_ne!(converted, original);
    assert_eq!(document.file_format(), FileFormat::Mac);
    assert_eq!(document.text(), "one\ntwo");

    document.try_undo().unwrap();
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.file_format(), FileFormat::Dos);
    assert_eq!(document.file_format_origin(), original_origin);
    assert_eq!(document.line_ending_evidence(), original_evidence);
    assert_eq!(document.encoding(), Encoding::Utf8);
    assert_eq!(document.format(), Format::Markdown);

    document.try_redo().unwrap();
    assert_eq!(document.source_bytes(), converted);
    assert_eq!(document.file_format(), FileFormat::Mac);
    assert_eq!(document.line_ending_evidence(), converted_evidence);
    assert_eq!(document.text(), "one\ntwo");
}

#[test]
fn source_metadata_changes_have_typed_history_and_restore_the_exact_bom_state() {
    let mut document = Document::new("abc");
    document.set_bom_policy(BomPolicy::Always).unwrap();
    let node = document.history_status().current;
    let details = document.history_node_details(node.node).unwrap();
    assert_eq!(
        details.semantic.unwrap().changes(),
        [HistorySemanticChangeKind::SourceMetadata]
    );
    assert_eq!(details.transactions.len(), 1);
    assert_eq!(details.transactions[0].source_patches()[0].range(), 0..0);
    assert_eq!(
        details.transactions[0].source_patches()[0].replacement_digest(),
        SourceArtifactDigest::from_bytes(&[0xef, 0xbb, 0xbf])
    );

    document.try_undo().unwrap();
    assert!(!document.has_bom());
    assert_eq!(document.source_bytes(), b"abc");
    document.try_redo().unwrap();
    assert!(document.has_bom());
    assert_eq!(document.source_bytes(), b"\xef\xbb\xbfabc");
}

#[test]
fn branch_menu_summaries_follow_the_preferred_redo_child() {
    let mut document = Document::new("x");
    document.insert(1, "a").unwrap();
    let first = document.history_status().current;
    document.try_undo().unwrap();
    document.set_bom_policy(BomPolicy::Always).unwrap();
    document.try_undo().unwrap();

    assert_eq!(
        document.history_status().redo_summary.unwrap().changes(),
        [HistorySemanticChangeKind::SourceMetadata]
    );
    document.prefer_redo_branch(0).unwrap();
    assert_eq!(document.redo_branches()[0].destination, first);
    assert_eq!(
        document.history_status().redo_summary.unwrap().changes(),
        [HistorySemanticChangeKind::Text]
    );
}
