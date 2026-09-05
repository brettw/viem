use evim_core::document::{Document, HistoryBoundary, HistoryChangeNumber, HistoryError};

#[test]
fn public_history_model_retains_branches_and_tracks_preferred_redo() {
    let mut document = Document::new("base");
    let root = document.history_status().current;

    document.insert(4, "-old").unwrap();
    let old = document.history_status().current;
    document.try_undo().unwrap();
    document.insert(4, "-new").unwrap();
    let new = document.history_status().current;
    document.try_undo().unwrap();

    let branches = document.redo_branches();
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0].destination, old);
    assert_eq!(branches[1].destination, new);
    assert!(!branches[0].preferred);
    assert!(branches[1].preferred);

    document.prefer_redo_branch(0).unwrap();
    assert_eq!(document.try_redo().unwrap().to, old);
    assert_eq!(document.text(), "base-old");

    // Undo always records the child just left as this fork's preferred redo.
    assert_eq!(document.try_undo().unwrap().to, root);
    assert!(document.redo_branches()[0].preferred);
    assert_eq!(document.try_redo().unwrap().to, old);
}

#[test]
fn exact_history_selection_is_typed_and_non_destructive_on_failure() {
    let mut document = Document::new("a");
    let root = document.history_status().current;
    document.insert(1, "b").unwrap();
    let ab = document.history_status().current;
    document.insert(2, "c").unwrap();
    let abc = document.history_status().current;

    let by_node = document.select_history_node(root.node).unwrap();
    assert_eq!(by_node.from, abc);
    assert_eq!(by_node.to, root);
    assert_eq!(document.text(), "a");

    let by_change = document.select_history_change(ab.change).unwrap();
    assert_eq!(by_change.to, ab);
    assert_eq!(document.text(), "ab");

    let other_document = Document::new("unrelated");
    let foreign_node = other_document.history_status().current.node;
    let before_status = document.history_status();
    let before_revision = document.revision();
    assert_eq!(
        document.select_history_node(foreign_node),
        Err(HistoryError::NodeNotFound(foreign_node))
    );
    assert_eq!(
        document.select_history_change(HistoryChangeNumber::from_u64(999)),
        Err(HistoryError::ChangeNotFound(HistoryChangeNumber::from_u64(
            999
        )))
    );
    assert_eq!(document.history_status(), before_status);
    assert_eq!(document.revision(), before_revision);
    assert_eq!(document.text(), "ab");
}

#[test]
fn grouped_edits_get_one_change_number_and_no_ops_get_none() {
    let mut document = Document::new("");
    assert_eq!(
        document.history_status().current.change,
        HistoryChangeNumber::INITIAL
    );

    document.insert(0, "").unwrap();
    assert_eq!(document.history_status().node_count, 1);

    document.begin_edit_group();
    document.insert(0, "a").unwrap();
    let group = document.history_status().current;
    document.insert(1, "b").unwrap();
    document.replace(0..2, "ab").unwrap();
    document.end_edit_group();

    assert_eq!(group.change, HistoryChangeNumber::from_u64(1));
    assert_eq!(document.history_status().current, group);
    assert_eq!(document.history_status().node_count, 2);
    document.insert(2, "c").unwrap();
    assert_eq!(
        document.history_status().current.change,
        HistoryChangeNumber::from_u64(2)
    );
}

#[test]
fn save_point_uses_history_identity_and_becomes_clean_again_on_undo() {
    let mut document = Document::new("a");
    assert!(!document.is_dirty());
    document.insert(1, "b").unwrap();
    assert!(document.is_dirty());
    let saved = document.mark_saved();
    assert_eq!(document.history_status().save_point, saved);
    assert!(!document.is_dirty());

    document.insert(2, "c").unwrap();
    assert!(document.is_dirty());
    document.try_undo().unwrap();
    assert_eq!(document.history_status().current.node, saved);
    assert!(!document.is_dirty());
    document.try_undo().unwrap();
    assert!(document.is_dirty());
}

#[test]
fn matching_bytes_on_an_unsaved_branch_do_not_impersonate_the_save_point() {
    let mut document = Document::new("a");
    let saved = document.history_status().current;
    document.insert(1, "b").unwrap();
    document.delete(1..2).unwrap();
    assert_eq!(document.source_bytes(), b"a");
    assert!(document.is_dirty());

    document.select_history_node(saved.node).unwrap();
    assert!(!document.is_dirty());
}

#[test]
fn structured_history_boundaries_do_not_change_the_document() {
    let mut document = Document::new("a");
    let status = document.history_status();
    assert_eq!(
        document.try_undo(),
        Err(HistoryError::Boundary(HistoryBoundary::Oldest))
    );
    assert_eq!(
        document.try_redo(),
        Err(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo))
    );
    assert_eq!(document.history_status(), status);
    assert_eq!(document.text(), "a");
}

#[test]
fn counted_history_navigation_validates_the_complete_path_atomically() {
    let mut document = Document::new("a");
    document.insert(1, "b").unwrap();
    document.insert(2, "c").unwrap();

    let before = document.history_status();
    let before_bytes = document.source_bytes();
    let before_revision = document.revision();
    assert_eq!(
        document.try_undo_count(3),
        Err(HistoryError::Boundary(HistoryBoundary::Oldest))
    );
    assert_eq!(document.history_status(), before);
    assert_eq!(document.source_bytes(), before_bytes);
    assert_eq!(document.revision(), before_revision);

    document.try_undo_count(2).unwrap();
    assert_eq!(document.text(), "a");
    let before = document.history_status();
    assert_eq!(
        document.try_redo_count(3),
        Err(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo))
    );
    assert_eq!(document.history_status(), before);
    assert_eq!(document.text(), "a");

    document.try_redo_count(2).unwrap();
    assert_eq!(document.text(), "abc");
}

#[test]
fn failed_counted_redo_preserves_branch_preference_and_an_open_group() {
    let mut document = Document::new("root");
    let root = document.history_status().current;
    document.insert(4, "-old").unwrap();
    document.insert(8, "-leaf").unwrap();
    let old_leaf = document.history_status().current;
    document.select_history_node(root.node).unwrap();
    document.insert(4, "-new").unwrap();
    document.select_history_node(root.node).unwrap();
    document.prefer_redo_branch(0).unwrap();

    let branches = document.redo_branches();
    let before = document.history_status();
    assert_eq!(
        document.try_redo_count(3),
        Err(HistoryError::Boundary(HistoryBoundary::NoPreferredRedo))
    );
    assert_eq!(document.history_status(), before);
    assert_eq!(document.redo_branches(), branches);
    assert_eq!(document.text(), "root");
    assert_eq!(document.try_redo_count(2).unwrap().to, old_leaf);
    assert_eq!(document.text(), "root-old-leaf");

    let mut grouped = Document::new("");
    grouped.begin_edit_group();
    grouped.insert(0, "a").unwrap();
    grouped.insert(1, "b").unwrap();
    assert_eq!(
        grouped.try_undo_count(2),
        Err(HistoryError::Boundary(HistoryBoundary::Oldest))
    );
    grouped.insert(2, "c").unwrap();
    grouped.end_edit_group();
    assert_eq!(grouped.history_status().node_count, 2);
    grouped.try_undo().unwrap();
    assert_eq!(grouped.text(), "");
}

#[test]
fn failed_exact_selection_does_not_break_an_open_undo_unit() {
    let mut document = Document::new("");
    let foreign = Document::new("").history_status().current.node;
    document.begin_edit_group();
    assert_eq!(
        document.select_history_node(foreign),
        Err(HistoryError::NodeNotFound(foreign))
    );
    document.insert(0, "a").unwrap();
    document.insert(1, "b").unwrap();
    document.end_edit_group();

    assert_eq!(document.text(), "ab");
    assert_eq!(document.history_status().node_count, 2);
    document.try_undo().unwrap();
    assert_eq!(document.text(), "");
}
