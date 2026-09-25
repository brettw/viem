use viem_core::command::{
    CommandContext, CommandModelRequest, CommandResolution, CommandStatus, InputEvent, Key,
};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, Format, FormattedPayloadEdit, FormattedTextPayload,
    HistoryRetentionPolicy, PreparedModelTransaction, ProjectionWorkScope,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let update = core.handle(view, CoreEvent::Input(event)).unwrap();
    if let Some(command) = update.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{:?}",
            command.status
        );
    }
}

fn prepare_typing(
    core: &Core<MockTextMeasurementProvider>,
    view: ViewId,
    text: &str,
) -> PreparedModelTransaction {
    let document = core.document();
    let plan = match core
        .command_state(view)
        .unwrap()
        .resolve(&CommandContext::new(document), InputEvent::text(text))
        .unwrap()
    {
        CommandResolution::Planned(plan) => plan,
        CommandResolution::Legacy(_) => panic!("HTML typing must resolve to a typed plan"),
    };
    let Some(CommandModelRequest::FormattedPayload(request)) = plan.model_request() else {
        panic!("HTML typing must use the normalized formatted payload");
    };
    document
        .prepare_formatted_payload_request(request.clone())
        .unwrap()
}

fn assert_regional(prepared: &PreparedModelTransaction) {
    let work = prepared.summary().projection_work();
    assert_eq!(
        work.scope(),
        ProjectionWorkScope::RegionalHardLines,
        "{work:?}"
    );
    assert!(work.decoded_source_bytes() < 256, "{work:?}");
    assert_eq!(work.projected_hard_lines(), 1);
    assert_eq!(work.full_text_bytes_materialized(), 0);
}

#[test]
fn completing_a_protected_space_in_a_large_document_uses_one_local_patch() {
    let prefix = "<p data-keep='yes'>Unchanged words</p>\n".repeat(2_500);
    let suffix = "\n<p title='tail'>Unchanged tail&#33;</p>".repeat(2_500);
    let original = format!("{prefix}<p>Hello,</p>{suffix}");
    let document = html(&original);
    let original_text = document.text().to_owned();
    let at = "Unchanged words\n".len() * 2_500 + "Hello,".len();
    let source_at = prefix.len() + "<p>Hello,".len();
    let first_block = document.projection().blocks()[0].id;
    let last_block = document.projection().blocks().last().unwrap().id;
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
    input(&mut core, view, InputEvent::key('i'));
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Upstream,
            extend_selection: false,
        },
    )
    .unwrap();

    let prepared = prepare_typing(&core, view, " ");
    assert_regional(&prepared);
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].range(), source_at..source_at);
    assert_eq!(patches[0].replacement(), b"&nbsp;");
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    drop(prepared);
    input(&mut core, view, InputEvent::text(" "));
    assert_eq!(core.command_state(view).unwrap().cursor(), at + 2);
    let protected = core.document().source_bytes();
    assert_eq!(
        protected,
        format!("{prefix}<p>Hello,&nbsp;</p>{suffix}").as_bytes()
    );

    let prepared = prepare_typing(&core, view, "world!");
    assert_regional(&prepared);
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].range(), source_at..source_at + "&nbsp;".len());
    assert_eq!(&protected[patches[0].range()], b"&nbsp;");
    assert_eq!(patches[0].replacement(), b" world!");
    assert_eq!(core.document().source_bytes(), protected);
    drop(prepared);

    input(&mut core, view, InputEvent::text("world!"));
    let expected_source = format!("{prefix}<p>Hello, world!</p>{suffix}");
    assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
    let mut expected_text = original_text.clone();
    expected_text.insert_str(at, " world!");
    assert_eq!(core.document().text(), expected_text);
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        at + " world!".len()
    );
    assert_eq!(core.document().projection().blocks()[0].id, first_block);
    assert_eq!(
        core.document().projection().blocks().last().unwrap().id,
        last_block
    );
    assert_eq!(html(&expected_source).text(), expected_text);

    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    assert_eq!(core.document().text(), original_text);
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
    assert_eq!(core.document().text(), expected_text);
}

fn append_typed(document: &mut Document, text: &str) -> usize {
    let at = document.text().len();
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), text, vec![]).unwrap();
    document
        .insert_with_typing_properties(
            FormattedPayloadEdit::new(at..at, payload)
                .with_boundary_affinity(BoundaryAffinity::Upstream),
            &[],
        )
        .unwrap()
}

#[test]
fn generated_nbsp_remains_simplifiable_after_its_creation_history_is_pruned() {
    let mut document = html("<p>Hello,</p><!--keep-->");
    assert_eq!(append_typed(&mut document, " "), "Hello,\u{a0}".len());
    let protected = document.source_bytes();
    assert_eq!(protected, b"<p>Hello,&nbsp;</p><!--keep-->");
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    assert_eq!(document.history_status().node_count, 1);
    assert!(!document.history_status().can_undo);
    document.set_history_retention_policy(HistoryRetentionPolicy::unlimited());

    assert_eq!(append_typed(&mut document, "world!"), "Hello, world!".len());
    let expected = "<p>Hello, world!</p><!--keep-->";
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert_eq!(document.text(), "Hello, world!");
    assert_eq!(html(expected).text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), protected);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
}

#[test]
fn reopening_generated_nbsp_makes_its_nonbreaking_semantics_authoritative() {
    let mut generated = html("<p>Hello,</p><!--keep-->");
    append_typed(&mut generated, " ");
    let protected = String::from_utf8(generated.source_bytes()).unwrap();
    assert_eq!(protected, "<p>Hello,&nbsp;</p><!--keep-->");
    let mut reopened = html(&protected);
    assert_eq!(
        append_typed(&mut reopened, "world!"),
        "Hello,\u{a0}world!".len()
    );
    assert_eq!(reopened.text(), "Hello,\u{a0}world!");
    assert_eq!(
        reopened.source_bytes(),
        b"<p>Hello,&nbsp;world!</p><!--keep-->"
    );
    assert!(reopened.undo());
    assert_eq!(reopened.source_bytes(), protected.as_bytes());
    assert!(reopened.redo());
    assert_eq!(
        reopened.source_bytes(),
        b"<p>Hello,&nbsp;world!</p><!--keep-->"
    );
}
