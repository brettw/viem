//! Exact source saved by fuzz campaign 20260907T193459Z-15419, seed 1,
//! immediately before action 1100. JSON retains its NUL bytes without making
//! the fixture a binary file. The failing view had width 80 and cursor 294.
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format, SourceArtifactDigest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: InputEvent) {
    let outcome = core.handle(view, CoreEvent::Input(input)).unwrap();
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

#[test]
fn saved_html_open_above_case_commits_a_local_rewrite_with_a_valid_cursor() {
    let source: String =
        serde_json::from_str(include_str!("fixtures/html-open-line-saved-case.json")).unwrap();
    assert_eq!(source.len(), 994);
    assert_eq!(&source.as_bytes()[429..431], b"\n\n");
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let original_text = document.text().to_owned();
    assert!(original_text[..294].ends_with("العربية中中 "));
    assert!(original_text[294..].starts_with("3.中 &é"));
    document.text_point(294).unwrap();
    let revision = document.revision();
    let mut core = Core::new(document);
    // Keep the saved whitespace edit target at a real word-wrap boundary.
    // Without emergency word splitting, a slightly narrower row is required.
    let view = core.add_view(MockTextMeasurementProvider::new(), 70., 700.);
    assert!(core.layout(view).unwrap().snapshot().unwrap().rows.iter().any(|row| {
        row.text_range.start == 294
    }));
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: revision,
            text_offset: 294,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.command_state(view).unwrap().cursor(), 294);

    // O alone previously committed this support rewrite, then failed because
    // the old offset 294 was now inside the two-byte NBSP at 293..295.
    input(&mut core, view, InputEvent::key('O'));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(view).unwrap().cursor(), 295);
    assert_ne!(core.document().revision(), revision);
    let mut expected_source = source.as_bytes().to_vec();
    expected_source.splice(429..431, b"&nbsp;</p><p>".iter().copied());
    assert_eq!(expected_source.len(), 1005);
    assert_eq!(core.document().source_bytes(), expected_source);
    let expected_text = format!("{}\u{a0}\n{}", &original_text[..293], &original_text[294..]);
    assert_eq!(core.document().text(), expected_text);

    let details = core
        .document()
        .history_node_details(core.document().history_status().current.node)
        .unwrap();
    assert_eq!(details.transactions.len(), 1);
    let transaction = &details.transactions[0];
    assert_eq!(transaction.before_revision(), revision);
    assert_eq!(transaction.after_revision(), core.document().revision());
    let patches = transaction.source_patches();
    assert_eq!(patches.first().unwrap().range().start, 429);
    assert_eq!(patches.last().unwrap().range().end, 431);
    let mut before_at = 429;
    let mut after_at = 429;
    for patch in patches {
        assert_eq!(patch.range().start, before_at);
        let after_end = after_at + patch.replacement_len();
        assert_eq!(
            patch.replacement_digest(),
            SourceArtifactDigest::from_bytes(&expected_source[after_at..after_end])
        );
        before_at = patch.range().end;
        after_at = after_end;
    }
    assert_eq!(after_at, 442);
    assert_eq!(&expected_source[..429], &source.as_bytes()[..429]);
    assert_eq!(&expected_source[442..], &source.as_bytes()[431..]);
    let reopened =
        Document::from_bytes(expected_source.clone(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), expected_text);
    assert_eq!(
        reopened.projection().hard_line_count(),
        core.document().projection().hard_line_count()
    );

    input(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.document().source_bytes(), expected_source);
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().text(), original_text);
    assert_eq!(core.command_state(view).unwrap().cursor(), 294);
    assert!(!core.document().history_status().can_undo);
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), expected_source);
    assert_eq!(core.document().text(), expected_text);
    assert!(!core.document().history_status().can_redo);
}
