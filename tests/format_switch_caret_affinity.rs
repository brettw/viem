use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const PARAGRAPH: &str = "A uniquely identified paragraph with café and 👩‍💻 followed by enough ordinary words to wrap across several rows. Anchor.";

fn editor(format: Format) -> (Editor, ViewId) {
    let source = if format == Format::Markdown {
        format!("# Heading\n\n{PARAGRAPH}\n\nFollowing paragraph.")
    } else {
        format!("<h1>Heading</h1><p>{PARAGRAPH}</p><p>Following paragraph.</p>")
    };
    let mut core =
        Core::new(Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 600.0);
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    (core, view)
}

fn place(core: &mut Editor, view: ViewId, offset: usize) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Upstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let state = core.command_state(view).unwrap();
    assert_eq!(state.cursor(), offset);
    assert_eq!(state.boundary_affinity(), BoundaryAffinity::Upstream);
    // Pointer placement stores the logical point. A visual-row round trip
    // additionally installs the layout-backed visual_position being reviewed.
    for key in [Key::Down, Key::Up] {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    let state = core.command_state(view).unwrap();
    assert_eq!(state.cursor(), offset);
    let visual = state
        .visual_position()
        .expect("row navigation retains its exact caret");
    assert_eq!(visual.text_offset, offset);
    assert_eq!(visual.affinity, BoundaryAffinity::Upstream);
}

fn targets(from: Format) -> [Format; 3] {
    if from == Format::Markdown {
        [Format::MarkdownSource, Format::Html, Format::HtmlSource]
    } else {
        [Format::HtmlSource, Format::Markdown, Format::MarkdownSource]
    }
}

fn switch_and_check(core: &mut Editor, view: ViewId, target: Format, retained_prefix: &str) {
    let original = core.document().source_bytes();
    core.handle(
        view,
        CoreEvent::SetFormat {
            document: core.document().id(),
            revision: core.document().revision(),
            target,
        },
    )
    .unwrap();
    let expected = core.document().text().find(retained_prefix).unwrap() + retained_prefix.len();
    let state = core.command_state(view).unwrap();
    assert_eq!(state.mode(), Mode::Insert);
    core.document().text_point(state.cursor()).unwrap();
    assert_eq!(
        state.cursor(),
        expected,
        "target {target:?}, text {:?}",
        core.document().text()
    );
    assert_eq!(state.boundary_affinity(), BoundaryAffinity::Upstream);
    let converted = core.document().source_bytes();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), original);
    // Native history deliberately returns to Normal mode, whose cursor at
    // hard-line end addresses the preceding grapheme rather than the Insert
    // boundary. Format switching itself above retains Insert mode exactly.
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), converted);
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

#[test]
fn insert_upstream_soft_wrap_caret_keeps_its_semantic_boundary_across_format_switch() {
    for from in [Format::Markdown, Format::Html] {
        for target in targets(from) {
            let (mut core, view) = editor(from);
            let paragraph_start = core.document().text().find(PARAGRAPH).unwrap();
            let boundary = core
                .layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows
                .iter()
                .find(|row| row.wraps_to_next && row.text_range.start >= paragraph_start)
                .unwrap()
                .text_range
                .end;
            let prefix = core.document().text()[paragraph_start..boundary].to_owned();
            place(&mut core, view, boundary);
            switch_and_check(&mut core, view, target, &prefix);
        }
    }
}

#[test]
fn insert_upstream_paragraph_end_caret_stays_before_generated_source_delimiters() {
    for from in [Format::Markdown, Format::Html] {
        for target in targets(from) {
            let (mut core, view) = editor(from);
            let boundary = core.document().text().find(PARAGRAPH).unwrap() + PARAGRAPH.len();
            place(&mut core, view, boundary);
            switch_and_check(&mut core, view, target, PARAGRAPH);
        }
    }
}
