use evim_core::command::{
    CommandContext, CommandModelRequest, CommandResolution, InputEvent, Key, Mode,
};
use evim_core::document::{BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent, ViewId};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    core.handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| panic!("{event:?}: {error:?}"));
}

fn assert_reopen_equivalent(document: &Document) {
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), fresh.text());
    let blocks_without_ids = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .cloned()
            .map(|mut block| {
                block.id = 0;
                block
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks_without_ids(document), blocks_without_ids(&fresh));
    assert_eq!(
        document.projection().hard_line_count(),
        fresh.projection().hard_line_count()
    );
    for line in 0..document.projection().hard_line_count() {
        assert_eq!(
            document.projection().hard_line_range(line),
            fresh.projection().hard_line_range(line)
        );
    }
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
    for (offset, _) in document.text().char_indices() {
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), offset, false),
            DocumentLayoutStyles::semantic_character_at(fresh.projection(), offset, false),
            "character style at {offset}"
        );
    }
}

fn open_above_and_type(pre_insertion_source: &str, motion: char) {
    let insertion_at = pre_insertion_source.find("<br>").unwrap();
    let original_source = pre_insertion_source.replacen("<br>", "", 1);
    let document = Document::from_bytes(
        original_source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    let original_text = document.text().to_owned();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);

    // `$` leaves upstream affinity at the old line end. O must associate its
    // new empty line with the following <br>, independent of that old side.
    for event in [
        InputEvent::Key(Key::Escape),
        InputEvent::key(motion),
        InputEvent::Key(Key::Escape),
        InputEvent::Key(Key::Escape),
        InputEvent::key('O'),
    ] {
        input(&mut core, view, event);
    }
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.command_state(view).unwrap().visual_position(), None);
    assert_eq!(core.command_state(view).unwrap().desired_x(), None);
    assert_eq!(
        core.command_state(view).unwrap().boundary_affinity(),
        BoundaryAffinity::Downstream
    );
    assert_eq!(
        core.document().source_bytes(),
        pre_insertion_source.as_bytes()
    );
    assert_eq!(core.document().text(), format!("\n{original_text}"));

    // Preparation proves the typing operation is one local source insertion
    // and remains read-only until the command coordinator commits it.
    let document = core.document();
    let context = CommandContext::new(document);
    let plan = match core
        .command_state(view)
        .unwrap()
        .resolve(&context, InputEvent::text("a"))
        .unwrap()
    {
        CommandResolution::Planned(plan) => plan,
        CommandResolution::Legacy(_) => panic!("scalar insertion should resolve to a typed plan"),
    };
    let Some(CommandModelRequest::FormattedPayload(request)) = plan.model_request() else {
        panic!("HTML scalar insertion should use a formatted payload");
    };
    let prepared = document
        .prepare_formatted_payload_request(request.clone())
        .unwrap();
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].range(), insertion_at..insertion_at);
    assert_eq!(patches[0].replacement(), b"a");
    assert_eq!(document.source_bytes(), pre_insertion_source.as_bytes());
    drop(prepared);

    input(&mut core, view, InputEvent::text("a"));
    let mut expected_source = pre_insertion_source.to_owned();
    expected_source.insert(insertion_at, 'a');
    assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
    assert_eq!(core.document().text(), format!("a\n{original_text}"));
    assert_reopen_equivalent(core.document());

    input(&mut core, view, InputEvent::Key(Key::Escape));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), original_source.as_bytes());
    assert_eq!(core.document().text(), original_text);
    assert_reopen_equivalent(core.document());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
    assert_eq!(core.document().text(), format!("a\n{original_text}"));
    assert_reopen_equivalent(core.document());
}

#[test]
fn opening_a_leading_empty_html_line_resets_the_previous_cursor_side() {
    for source in [
        "<p><br>;", // Minimized from the captured malformed fixture.
        "<p><br>achanged</p>",
        "<p style='font-style:italic'><br>achanged</p>",
        "<p><br>achanged</p><!--keep--><unknown data-x='untouched'><!--tail--></unknown>",
    ] {
        for motion in ['$', '0'] {
            open_above_and_type(source, motion);
        }
    }
}

#[test]
fn leading_empty_line_in_captured_malformed_html_accepts_typing() {
    // Exact pre-insertion source from core fuzz seed 1, action 694. The fixture
    // deliberately retains NULs, malformed tags, misnesting, and unknown bytes.
    open_above_and_type(include_str!("fixtures/html-leading-empty-line.html"), '$');
}

#[test]
fn counted_open_above_and_dot_repeat_use_the_new_empty_line() {
    let source = "<p>achanged</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    for event in [
        InputEvent::key('$'),
        InputEvent::key('2'),
        InputEvent::key('O'),
        InputEvent::text("a"),
        InputEvent::Key(Key::Escape),
    ] {
        input(&mut core, view, event);
    }
    assert_eq!(core.document().text(), "a\na\nachanged");
    let once = core.document().source_bytes();
    assert_reopen_equivalent(core.document());
    input(&mut core, view, InputEvent::key('.'));
    assert_eq!(core.document().text(), "a\na\na\na\nachanged");
    assert_reopen_equivalent(core.document());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), once);
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn vertical_motion_after_open_above_starts_on_the_new_empty_line() {
    let document = Document::from_bytes(
        b"<p>achanged</p><p>second</p>".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    for event in [InputEvent::key('$'), InputEvent::key('O')] {
        input(&mut core, view, event);
    }
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    input(&mut core, view, InputEvent::Key(Key::Down));
    assert_eq!(core.command_state(view).unwrap().cursor(), 1);
    input(&mut core, view, InputEvent::Key(Key::Up));
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    input(&mut core, view, InputEvent::text("a"));
    assert_eq!(core.document().text(), "a\nachanged\nsecond");
    assert_reopen_equivalent(core.document());
}
