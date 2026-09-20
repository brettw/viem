use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn open(source: &str) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    (core, view)
}

fn input(core: &mut Editor, view: ViewId, event: InputEvent) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| panic!("{event:?}: {error:?}, {:?}", core.document().source_bytes()));
    assert!(result.command.as_ref().map_or(true, |command| matches!(
        command.status,
        CommandStatus::Complete | CommandStatus::Pending
    )));
    core.document().text_point(core.command_state(view).unwrap().cursor()).unwrap();
}

fn insert_at(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(),
        text_offset: at,
        affinity: BoundaryAffinity::Downstream,
        extend_selection: false,
    }).unwrap();
    input(core, view, InputEvent::key('i'));
}

fn saved_and_history(core: &mut Editor, view: ViewId, original: &str, expected: &str) {
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    let reopened = Document::from_bytes(expected.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), core.document().text());
    assert_eq!(
        reopened.projection().blocks().iter().map(|b| (&b.range, &b.style, &b.kind)).collect::<Vec<_>>(),
        core.document().projection().blocks().iter().map(|b| (&b.range, &b.style, &b.kind)).collect::<Vec<_>>()
    );
    if core.command_state(view).unwrap().mode() != Mode::Normal {
        input(core, view, InputEvent::Key(Key::Escape));
    }
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
}

#[test]
fn empty_list_exit_reuses_one_paragraph_and_preserves_source_neighbors() {
    for (source, at, expected) in [
        ("<ul><li></li></ul>", 0, "<p></p>"),
        ("<ul><li><p></p></li></ul>", 0, "<p></p>"),
        ("<div data-keep='x'></div><!--before--><ul><li><p></p></li></ul><!--after--><div></div>",
         0, "<div data-keep='x'></div><!--before--><p></p><!--after--><div></div>"),
        ("<p>prior</p><ol><li><p></p></li><li>later</li></ol>",
         6, "<p>prior</p><p></p><ol start=\"2\"><li>later</li></ol>"),
        ("<ol><li>prior</li><li><p></p></li></ol>",
         6, "<ol><li>prior</li></ol><p></p>"),
        ("<ul data-keep='list'><li>prior</li><li><p></p></li><li>later</li></ul>",
         6, "<ul data-keep='list'><li>prior</li></ul><p></p><ul data-keep='list'><li>later</li></ul>"),
        ("<ul><li data-keep='item'><p id='paragraph'></p></li></ul>",
         0, "<div data-keep='item'><p id='paragraph'></p></div>"),
    ] {
        for exit in [Key::Backspace, Key::Enter] {
            let (mut core, view) = open(source);
            let text = core.document().text().to_owned();
            let count = core.document().projection().blocks().len();
            insert_at(&mut core, view, at);
            input(&mut core, view, InputEvent::Key(exit));
            assert_eq!(core.document().text(), text, "{source}");
            assert_eq!(core.document().projection().blocks().len(), count, "{source}");
            assert_eq!(core.command_state(view).unwrap().cursor(), at, "{source}");
            saved_and_history(&mut core, view, source, expected);
        }
    }
}

#[test]
fn removing_list_treatment_keeps_intentional_continuations_and_nested_content() {
    for (source, expected) in [
        ("<ul><li><p></p><p></p></li></ul>", "<p></p><p></p>"),
        ("<ul><li><p></p><p>continuation</p></li></ul>", "<p></p><p>continuation</p>"),
        ("<ul><li><!--keep--><p id='body'>one</p><p>two</p></li></ul>",
         "<!--keep--><p id='body'>one</p><p>two</p>"),
        ("<ul><li><p>one</p><ul data-keep='nested'><li>two</li></ul></li></ul>",
         "<p>one</p><ul data-keep='nested'><li>two</li></ul>"),
        ("<ul><li data-keep='item'><p>one</p><p>two</p></li></ul>",
         "<div data-keep='item'><p>one</p><p>two</p></div>"),
        ("<ul data-keep='outer'><li data-keep='item'><!--keep--><ul><li><p>one</p></li></ul></li></ul>",
         "<div data-keep='outer'><div data-keep='item'><!--keep--><p>one</p></div></div>"),
    ] {
        let (mut core, view) = open(source);
        let text = core.document().text().to_owned();
        core.handle(view, CoreEvent::SetListStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style: None,
        }).unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_eq!(core.document().text(), text, "{source}");
        saved_and_history(&mut core, view, source, expected);
    }
}

#[test]
fn backspace_after_empty_list_exit_merges_only_the_deleted_boundary() {
    for source in [
        "<p></p><ul><li></li></ul>",
        "<p></p><ul><li><p></p></li></ul>",
    ] {
        let (mut core, view) = open(source);
        insert_at(&mut core, view, 1);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().source_bytes(), b"<p></p><p></p>");
        assert_eq!(core.document().text(), "\n");
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), "");
        assert_eq!(core.document().projection().blocks().len(), 1);
        saved_and_history(&mut core, view, source, "<p></p>");
    }
}

#[test]
fn exiting_an_empty_item_with_retained_containers_supplies_one_paragraph() {
    for (source, expected) in [
        ("<ul><li><div></div></li></ul>", "<div><p></p></div>"),
        ("<ul><li><div data-keep='child'></div></li></ul>",
         "<div data-keep='child'><p></p></div>"),
        ("<ul><li data-keep='item'><div data-keep='child'></div></li></ul>",
         "<div data-keep='item'><div data-keep='child'><p></p></div></div>"),
        ("<p>prior</p><ul><li><div></div></li></ul><p>after</p>",
         "<p>prior</p><div><p></p></div><p>after</p>"),
    ] {
        let (mut core, view) = open(source);
        let text = core.document().text().to_owned();
        let at = if source.starts_with("<p>") { 6 } else { 0 };
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), text);
        assert_eq!(expected.matches("<p></p>").count(), 1);
        saved_and_history(&mut core, view, source, expected);
    }
}
