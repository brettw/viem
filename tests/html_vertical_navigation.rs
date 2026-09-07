use evim_core::command::{CommandStatus, InputEvent, Key, LineMode};
use evim_core::document::{BoundaryAffinity, Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let result = core
        .handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| {
            panic!(
                "{event:?}: {error:?}, source={:?}, cursor={}",
                String::from_utf8_lossy(&core.document().source_bytes()),
                core.command_state(view).unwrap().cursor()
            )
        });
    assert!(
        matches!(
            result.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ),
        "{event:?}: {:?}",
        core.document().text()
    );
}

fn current_row(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> usize {
    let state = core.command_state(view).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let (offset, affinity) = state
        .visual_position()
        .map_or((state.cursor(), state.boundary_affinity()), |position| {
            (position.text_offset, position.affinity)
        });
    let point = [
        affinity,
        BoundaryAffinity::Downstream,
        BoundaryAffinity::Upstream,
    ]
    .into_iter()
    .find_map(|affinity| snapshot.caret_point(offset, affinity).ok())
    .unwrap();
    snapshot.caret_geometry(point).unwrap().row_index
}

#[test]
fn html_up_and_down_cross_each_rendered_row_once_after_terminal_typing() {
    for source in [
        "<p>First paragraph.</p><p>Second paragraph.</p>",
        "<div>First line.<br>Second line.<br></div>",
        "<p>First paragraph.</p><p>Second paragraph.<br></p>",
        "<p>First paragraph.</p><p></p><p></p>",
        "<ol><li>First item.</li><li>Second item.</li></ol>",
        "<pre>first line\nsecond line\n</pre>",
        "<p>Words that continue onto several wrapped lines of prose with a final short line.</p>",
        "<ul><li>first</li></ul><p><br></p><ul></ul>",
    ] {
        for width in [110., 500.] {
            for returns in 0..=3 {
                let document =
                    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html)
                        .unwrap();
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), width, 1000.);
                input(&mut core, view, InputEvent::key('G'));
                input(&mut core, view, InputEvent::key('A'));
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: core.document().text().len(),
                        affinity: BoundaryAffinity::Upstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                for _ in 0..returns {
                    input(&mut core, view, InputEvent::Key(Key::Enter));
                }
                input(&mut core, view, InputEvent::text("-"));
                let last_row = current_row(&core, view);
                for expected in (0..last_row).rev() {
                    input(&mut core, view, InputEvent::Key(Key::Up));
                    assert_eq!(
                        current_row(&core, view),
                        expected,
                        "{source} width={width} returns={returns}"
                    );
                }
                for expected in 1..=last_row {
                    input(&mut core, view, InputEvent::Key(Key::Down));
                    assert_eq!(
                        current_row(&core, view),
                        expected,
                        "{source} width={width} returns={returns}"
                    );
                }
                input(&mut core, view, InputEvent::Key(Key::Escape));
                let last_row = current_row(&core, view);
                for expected in (0..last_row).rev() {
                    input(&mut core, view, InputEvent::Key(Key::Up));
                    assert_eq!(
                        current_row(&core, view),
                        expected,
                        "normal: {source} width={width} returns={returns}"
                    );
                }
                for expected in 1..=last_row {
                    input(&mut core, view, InputEvent::Key(Key::Down));
                    assert_eq!(
                        current_row(&core, view),
                        expected,
                        "normal: {source} width={width} returns={returns}"
                    );
                }
            }
        }
    }
}

#[test]
fn html_physical_line_policy_applies_to_normal_but_not_insert_arrow_navigation() {
    let source = "<p>First paragraph.</p>\n\n<p>Second paragraph.</p>\n\n<p>Last paragraph.</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 1000.);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    input(&mut core, view, InputEvent::key('G'));
    assert_eq!(current_row(&core, view), 2);
    for expected in [2, 1, 1, 0, 0] {
        input(&mut core, view, InputEvent::Key(Key::Up));
        assert_eq!(current_row(&core, view), expected);
    }
    core.handle(view, CoreEvent::SetLineMode(LineMode::Visual))
        .unwrap();
    input(&mut core, view, InputEvent::key('G'));
    for expected in [1, 0] {
        input(&mut core, view, InputEvent::Key(Key::Up));
        assert_eq!(current_row(&core, view), expected);
    }
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    input(&mut core, view, InputEvent::key('G'));
    input(&mut core, view, InputEvent::key('A'));
    assert_eq!(current_row(&core, view), 2);
    for expected in [1, 0] {
        input(&mut core, view, InputEvent::Key(Key::Up));
        assert_eq!(current_row(&core, view), expected);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
