use evim_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, HistoryNavigationRequest,
    MappingOutcome, ModelChangeKind, ModelRequest, ModelTransactionError, SemanticInlineStyle,
    TextEdit, TextRange,
};

fn edit_request(document: &Document, edits: Vec<TextEdit>) -> ModelRequest {
    ModelRequest::ApplyTextEdits {
        document: document.id(),
        revision: document.revision(),
        edits,
    }
}

#[test]
fn preparing_discontiguous_edits_is_nonmutating_and_retains_exact_maps() {
    let document = Document::new("aa MID zz");
    let before_source = document.source_bytes();
    let before_history = document.history_status();
    let middle = TextRange::new(
        document.text_point(3).unwrap(),
        document.text_point(6).unwrap(),
    )
    .unwrap();

    let prepared = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(0..2, "alpha"), TextEdit::new(7..9, "!")],
        ))
        .unwrap();

    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.text(), "aa MID zz");
    assert_eq!(document.history_status(), before_history);
    assert_eq!(prepared.before_revision(), document.revision());
    assert_ne!(prepared.after_revision(), document.revision());
    assert_eq!(prepared.summary().kind(), ModelChangeKind::TextEdits);
    assert_eq!(prepared.summary().source_patches().len(), 2);
    assert_eq!(prepared.summary().source_patches()[0].range(), 0..2);
    assert_eq!(
        prepared.summary().source_patches()[0].replacement(),
        b"alpha"
    );
    assert_eq!(prepared.summary().source_patches()[1].range(), 7..9);
    assert_eq!(prepared.summary().source_patches()[1].replacement(), b"!");
    let mut reconstructed = before_source.clone();
    for patch in prepared.summary().source_patches().iter().rev() {
        reconstructed.splice(patch.range(), patch.replacement().iter().copied());
    }
    assert_eq!(reconstructed, b"alpha MID !");
    assert_eq!(prepared.summary().formatted_splices().len(), 2);
    assert_eq!(prepared.summary().formatted_splices()[0].old_range(), 0..2);
    assert_eq!(prepared.summary().formatted_splices()[1].old_range(), 7..9);

    let mapped = prepared.text_position_map().map_text_range(middle).unwrap();
    let MappingOutcome::Moved(mapped) = mapped else {
        panic!("unchanged middle content should move, not collapse");
    };
    assert_eq!(mapped.segments().len(), 1);
    assert_eq!(mapped.segments()[0].start().offset(), 6);
    assert_eq!(mapped.segments()[0].end().offset(), 9);
}

#[test]
fn committing_a_prepared_batch_is_one_atomic_undo_unit() {
    let mut document = Document::new("aa MID zz");
    let prepared = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(0..2, "alpha"), TextEdit::new(7..9, "!")],
        ))
        .unwrap();
    let after_revision = prepared.after_revision();
    let committed = document.commit_model_transaction(prepared).unwrap();

    assert_eq!(document.text(), "alpha MID !");
    assert_eq!(document.revision(), after_revision);
    assert_eq!(committed.after_revision(), after_revision);
    assert_eq!(document.history_status().node_count, 2);
    assert!(document.undo());
    assert_eq!(document.text(), "aa MID zz");
    assert!(!document.undo());
}

#[test]
fn stale_and_wrong_document_commits_are_non_destructive() {
    let mut first = Document::new("first");
    let stale = first
        .prepare_model_request(edit_request(&first, vec![TextEdit::new(5..5, " prepared")]))
        .unwrap();
    first.insert(5, " live").unwrap();
    let after_live_source = first.source_bytes();
    let after_live_history = first.history_status();

    assert!(matches!(
        first.commit_model_transaction(stale),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(first.source_bytes(), after_live_source);
    assert_eq!(first.history_status(), after_live_history);

    let owner = Document::new("owner");
    let prepared = owner
        .prepare_model_request(edit_request(&owner, vec![TextEdit::new(0..5, "candidate")]))
        .unwrap();
    let mut other = Document::new("other");
    let other_source = other.source_bytes();
    let other_history = other.history_status();
    assert!(matches!(
        other.commit_model_transaction(prepared),
        Err(ModelTransactionError::WrongDocument { .. })
    ));
    assert_eq!(other.source_bytes(), other_source);
    assert_eq!(other.history_status(), other_history);
    assert_eq!(owner.text(), "owner");
}

#[test]
fn preparation_failures_do_not_change_source_revision_or_history() {
    let document = Document::from_bytes(
        vec![b'c', b'a', b'f', 0xe9],
        Encoding::Latin1,
        Format::PlainText,
    )
    .unwrap();
    let before_source = document.source_bytes();
    let before_revision = document.revision();
    let before_history = document.history_status();
    let error = document
        .prepare_model_request(edit_request(&document, vec![TextEdit::new(0..0, "😀")]))
        .unwrap_err();
    assert!(matches!(
        error,
        ModelTransactionError::Document(DocumentError::UnrepresentableCharacter { .. })
    ));
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.revision(), before_revision);
    assert_eq!(document.history_status(), before_history);

    let overlap = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(0..2, "x"), TextEdit::new(1..3, "y")],
        ))
        .unwrap_err();
    assert_eq!(
        overlap,
        ModelTransactionError::Document(DocumentError::OverlappingEdits)
    );
    assert_eq!(document.history_status(), before_history);
}

#[test]
fn semantic_style_and_fileformat_requests_prepare_before_publication() {
    let mut markdown =
        Document::from_bytes(b"make bold".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let style = markdown
        .prepare_model_request(ModelRequest::SetSemanticStyle {
            document: markdown.id(),
            revision: markdown.revision(),
            range: 5..9,
            style: SemanticInlineStyle::Strong,
            enabled: true,
        })
        .unwrap();
    assert_eq!(markdown.source_bytes(), b"make bold");
    assert_eq!(style.summary().kind(), ModelChangeKind::SemanticStyle);
    assert_eq!(style.summary().source_patches().len(), 2);
    assert!(style.summary().formatted_splices().is_empty());
    markdown.commit_model_transaction(style).unwrap();
    assert_eq!(markdown.source_bytes(), b"make **bold**");

    let mut text = Document::from_bytes_with_file_format(
        b"one\ntwo\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();
    let conversion = text
        .prepare_model_request(ModelRequest::SetFileFormat {
            document: text.id(),
            revision: text.revision(),
            target: FileFormat::Dos,
        })
        .unwrap();
    assert_eq!(text.source_bytes(), b"one\ntwo\n");
    assert_eq!(conversion.summary().kind(), ModelChangeKind::FileFormat);
    assert_eq!(conversion.summary().source_patches().len(), 2);
    text.commit_model_transaction(conversion).unwrap();
    assert_eq!(text.source_bytes(), b"one\r\ntwo\r\n");
    assert_eq!(text.text(), "one\ntwo\n");
}

#[test]
fn history_navigation_is_prepared_without_rerunning_an_edit() {
    let mut document = Document::new("a");
    document.insert(1, "b").unwrap();
    let changed = document.history_status().current;
    let before_prepare = document.history_status();
    let prepared = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    assert_eq!(document.text(), "ab");
    assert_eq!(document.history_status(), before_prepare);
    assert_eq!(prepared.planned_history_navigation().unwrap().from, changed);

    let committed = document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "a");
    assert_eq!(
        committed.summary().kind(),
        ModelChangeKind::HistoryNavigation
    );
    let navigation = committed.history_navigation().unwrap();
    assert_eq!(navigation.from, changed);
    assert_eq!(navigation.to, document.history_status().current);

    let redo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Redo,
        })
        .unwrap();
    document.commit_model_transaction(redo).unwrap();
    assert_eq!(document.text(), "ab");
    assert_eq!(document.history_status().current, changed);
}

#[test]
fn fileformat_preparation_reports_literal_break_reinterpretation() {
    let mut document = Document::from_bytes_with_file_format(
        b"a\rb\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();
    assert_eq!(document.text(), "a\rb\n");
    let before_source = document.source_bytes();
    let before_history = document.history_status();
    let request = ModelRequest::SetFileFormat {
        document: document.id(),
        revision: document.revision(),
        target: FileFormat::Mac,
    };
    assert_eq!(
        document.prepare_model_request(request).unwrap_err(),
        ModelTransactionError::Document(DocumentError::LineEndingConversionWouldReinterpretContent)
    );
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.history_status(), before_history);

    assert_eq!(
        document.set_file_format(FileFormat::Mac),
        Err(DocumentError::LineEndingConversionWouldReinterpretContent)
    );
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.history_status(), before_history);
}

#[test]
fn text_map_closes_over_a_new_grapheme_on_the_left() {
    let mut document = Document::new("ab");
    let prepared = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(1..1, "\u{301}")],
        ))
        .unwrap();

    // The audit summary remains the exact semantic insertion even though the
    // position map conservatively includes the preceding `a` grapheme.
    assert_eq!(prepared.summary().formatted_splices().len(), 1);
    assert_eq!(prepared.summary().formatted_splices()[0].old_range(), 1..1);
    assert_eq!(
        prepared.summary().formatted_splices()[0].inserted_len(),
        "\u{301}".len()
    );
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "a\u{301}b");
}

#[test]
fn text_map_closes_over_a_new_grapheme_on_the_right() {
    let regional_a = "\u{1f1e6}";
    let regional_b = "\u{1f1e7}";
    let mut document = Document::new(format!("{regional_b}x"));
    let prepared = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(0..0, regional_a)],
        ))
        .unwrap();

    // Inserting one regional indicator before another joins them into one
    // flag grapheme, so the target boundary immediately after the insertion
    // is not legal. Preparation widens only its internal map representation.
    assert_eq!(prepared.summary().formatted_splices()[0].old_range(), 0..0);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), format!("{regional_a}{regional_b}x"));
}

#[test]
fn grapheme_closure_keeps_stable_content_between_multiple_edits() {
    let regional_a = "\u{1f1e6}";
    let regional_b = "\u{1f1e7}";
    let original = format!("ab MID {regional_b}x");
    let mut document = Document::new(original.clone());
    let middle_start = original.find("MID").unwrap();
    let middle = TextRange::new(
        document.text_point(middle_start).unwrap(),
        document.text_point(middle_start + 3).unwrap(),
    )
    .unwrap();
    let flag_start = original.find(regional_b).unwrap();
    let prepared = document
        .prepare_model_request(edit_request(
            &document,
            vec![
                TextEdit::new(1..1, "\u{301}"),
                TextEdit::new(flag_start..flag_start, regional_a),
            ],
        ))
        .unwrap();

    assert_eq!(prepared.summary().formatted_splices().len(), 2);
    let mapped = prepared.text_position_map().map_text_range(middle).unwrap();
    let MappingOutcome::Moved(mapped) = mapped else {
        panic!("the stable middle must survive two grapheme-closing edits");
    };
    assert_eq!(mapped.segments().len(), 1);
    let mapped_middle = mapped.segments()[0];
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(
        &document.text()[mapped_middle.start().offset()..mapped_middle.end().offset()],
        "MID"
    );
}

#[test]
fn undo_and_redo_reuse_the_exact_discontiguous_edit_map() {
    let mut document = Document::new("aa MID zz");
    let edit = document
        .prepare_model_request(edit_request(
            &document,
            vec![TextEdit::new(0..2, "alpha"), TextEdit::new(7..9, "!")],
        ))
        .unwrap();
    document.commit_model_transaction(edit).unwrap();

    let changed_middle = TextRange::new(
        document.text_point(6).unwrap(),
        document.text_point(9).unwrap(),
    )
    .unwrap();
    let undo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    let MappingOutcome::Moved(mapped) = undo
        .text_position_map()
        .map_text_range(changed_middle)
        .unwrap()
    else {
        panic!("undo must retain unchanged middle identity");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 3);
    assert_eq!(mapped.segments()[0].end().offset(), 6);
    document.commit_model_transaction(undo).unwrap();
    assert_eq!(&document.text()[3..6], "MID");

    let original_middle = TextRange::new(
        document.text_point(3).unwrap(),
        document.text_point(6).unwrap(),
    )
    .unwrap();
    let redo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Redo,
        })
        .unwrap();
    let MappingOutcome::Moved(mapped) = redo
        .text_position_map()
        .map_text_range(original_middle)
        .unwrap()
    else {
        panic!("redo must retain unchanged middle identity");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 6);
    assert_eq!(mapped.segments()[0].end().offset(), 9);
    document.commit_model_transaction(redo).unwrap();
    assert_eq!(&document.text()[6..9], "MID");
}

#[test]
fn grouped_history_edge_composes_every_commit_in_the_unit() {
    let mut document = Document::new("L MID R");
    let root = document.history_status().current;
    document.begin_edit_group();
    document.replace(0..1, "LEFT").unwrap();
    document.replace(9..10, "RIGHT").unwrap();
    document.end_edit_group();
    assert_eq!(document.text(), "LEFT MID RIGHT");
    assert_eq!(document.history_status().node_count, 2);

    let middle = TextRange::new(
        document.text_point(5).unwrap(),
        document.text_point(8).unwrap(),
    )
    .unwrap();
    let undo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    assert_eq!(undo.planned_history_navigation().unwrap().to, root);
    let MappingOutcome::Moved(mapped) = undo.text_position_map().map_text_range(middle).unwrap()
    else {
        panic!("the group edge must retain content unchanged by both commits");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 2);
    assert_eq!(mapped.segments()[0].end().offset(), 5);
    document.commit_model_transaction(undo).unwrap();
    assert_eq!(document.text(), "L MID R");

    let middle = TextRange::new(
        document.text_point(2).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    let redo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Redo,
        })
        .unwrap();
    let MappingOutcome::Moved(mapped) = redo.text_position_map().map_text_range(middle).unwrap()
    else {
        panic!("the grouped redo edge must be the composed forward map");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 5);
    assert_eq!(mapped.segments()[0].end().offset(), 8);
}

#[test]
fn branch_jump_composes_reverse_to_lca_then_forward_to_target() {
    let mut document = Document::new("aa MID zz");
    document.replace(0..2, "alpha").unwrap();
    let left_branch = document.history_status().current;
    document.try_undo().unwrap();
    document.replace(7..9, "!").unwrap();
    assert_eq!(document.text(), "aa MID !");

    let middle = TextRange::new(
        document.text_point(3).unwrap(),
        document.text_point(6).unwrap(),
    )
    .unwrap();
    let jump = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::SelectNode(left_branch.node),
        })
        .unwrap();
    let MappingOutcome::Moved(mapped) = jump.text_position_map().map_text_range(middle).unwrap()
    else {
        panic!("a branch jump must preserve identity through its common ancestor");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 6);
    assert_eq!(mapped.segments()[0].end().offset(), 9);
    document.commit_model_transaction(jump).unwrap();
    assert_eq!(document.text(), "alpha MID zz");
    assert_eq!(&document.text()[6..9], "MID");
}

#[test]
fn style_and_fileformat_history_edges_are_revision_changing_identities() {
    let mut markdown =
        Document::from_bytes(b"make bold".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    markdown
        .set_semantic_style(5..9, SemanticInlineStyle::Strong, true)
        .unwrap();
    let styled_range = TextRange::new(
        markdown.text_point(5).unwrap(),
        markdown.text_point(9).unwrap(),
    )
    .unwrap();
    let undo_style = markdown
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: markdown.id(),
            revision: markdown.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    assert!(matches!(
        undo_style
            .text_position_map()
            .map_text_range(styled_range)
            .unwrap(),
        MappingOutcome::Exact(_)
    ));

    let mut text = Document::from_bytes_with_file_format(
        b"one\ntwo\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();
    text.set_file_format(FileFormat::Dos).unwrap();
    let all = TextRange::new(
        text.text_point(0).unwrap(),
        text.text_point(text.text().len()).unwrap(),
    )
    .unwrap();
    let undo_format = text
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: text.id(),
            revision: text.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    assert!(matches!(
        undo_format.text_position_map().map_text_range(all).unwrap(),
        MappingOutcome::Exact(_)
    ));
}

#[test]
fn stale_prepared_history_navigation_cannot_move_the_current_node() {
    let mut document = Document::new("a");
    document.insert(1, "b").unwrap();
    let stale_undo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();

    document.insert(2, "c").unwrap();
    let source = document.source_bytes();
    let history = document.history_status();
    assert!(matches!(
        document.commit_model_transaction(stale_undo),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.text(), "abc");
    assert_eq!(document.history_status(), history);
}

#[test]
fn undo_map_keeps_grapheme_closed_splices_legal_in_reverse() {
    let mut document = Document::new("ab");
    document.insert(1, "\u{301}").unwrap();
    assert_eq!(document.text(), "a\u{301}b");

    let b_start = "a\u{301}".len();
    let b = TextRange::new(
        document.text_point(b_start).unwrap(),
        document.text_point(b_start + 1).unwrap(),
    )
    .unwrap();
    let undo = document
        .prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation: HistoryNavigationRequest::Undo,
        })
        .unwrap();
    let MappingOutcome::Moved(mapped) = undo.text_position_map().map_text_range(b).unwrap() else {
        panic!("the unchanged trailing grapheme must survive the reverse map");
    };
    assert_eq!(mapped.segments()[0].start().offset(), 1);
    assert_eq!(mapped.segments()[0].end().offset(), 2);
    document.commit_model_transaction(undo).unwrap();
    assert_eq!(document.text(), "ab");
}
