//! Enter at an implicitly closed paragraph inside a list must use the
//! recovered HTML structure, including the following authored paragraph.
use evim_core::command::{CommandStatus, InputEvent, Key, Mode};
use evim_core::document::{BoundaryAffinity, Document, Encoding, Format, SourceArtifactDigest};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent, ViewId};

fn open(source: &str, initial_format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, initial_format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 700.);
    if initial_format == Format::HtmlSource {
        core.handle(
            view,
            CoreEvent::SetFormat {
                document: core.document().id(),
                revision: core.document().revision(),
                target: Format::Html,
            },
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let at = core.document().text().find("P>\n").unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    let cursor = core.command_state(view).unwrap().cursor();
    let outcome = core
        .handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| {
            panic!(
                "{event:?} at {cursor} in {:?}: {error:?}",
                String::from_utf8_lossy(&before)
            )
        });
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Complete | CommandStatus::Cancelled
    ));
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
    if revision == core.document().revision() {
        return;
    }
    let details = core
        .document()
        .history_node_details(core.document().history_status().current.node)
        .unwrap();
    let Some(transaction) = details
        .transactions
        .last()
        .filter(|transaction| transaction.before_revision() == revision)
    else {
        return;
    };
    let after = core.document().source_bytes();
    let mut old_at = 0;
    let mut new_at = 0;
    for patch in transaction.source_patches() {
        let range = patch.range();
        let unchanged = range.start - old_at;
        assert_eq!(
            &before[old_at..range.start],
            &after[new_at..new_at + unchanged]
        );
        new_at += unchanged;
        assert_eq!(
            patch.replacement_digest(),
            SourceArtifactDigest::from_bytes(&after[new_at..new_at + patch.replacement_len()])
        );
        new_at += patch.replacement_len();
        old_at = range.end;
    }
    assert_eq!(&before[old_at..], &after[new_at..]);
}

fn assert_reopens(core: &Core<MockTextMeasurementProvider>, expected: &str) {
    assert_eq!(core.document().text(), expected);
    let fresh =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(fresh.text(), expected);
    let actual_blocks = core.document().projection().blocks();
    let reopened_blocks = fresh.projection().blocks();
    assert_eq!(actual_blocks.len(), reopened_blocks.len());
    for (actual, reopened) in actual_blocks.iter().zip(reopened_blocks) {
        assert_eq!(actual.range, reopened.range);
        assert_eq!(actual.kind, reopened.kind);
        assert_eq!(actual.style, reopened.style);
        assert_eq!(actual.direct_paragraph, reopened.direct_paragraph);
        assert_eq!(
            actual.direct_default_character,
            reopened.direct_default_character
        );
        assert_eq!(
            DocumentLayoutStyles::character_at(
                core.document().projection(),
                actual.range.start,
                false
            )
            .unwrap(),
            DocumentLayoutStyles::character_at(fresh.projection(), reopened.range.start, false)
                .unwrap(),
        );
    }
    let actual_lines = core.document().hard_line_snapshot();
    let reopened_lines = fresh.hard_line_snapshot();
    assert_eq!(actual_lines.line_count(), reopened_lines.line_count());
    for index in 0..actual_lines.line_count() {
        let actual = actual_lines.line(index).unwrap();
        let reopened = reopened_lines.line(index).unwrap();
        assert_eq!(actual.content_range(), reopened.content_range());
        assert_eq!(actual.separator_range(), reopened.separator_range());
    }
}

fn exercise(source: &str, initial_format: Format) {
    let (mut core, view) = open(source, initial_format);
    let original_text = core.document().text().to_owned();
    let boundary = original_text.find("P>\n").unwrap() + "P>".len();
    let following_style =
        DocumentLayoutStyles::character_at(core.document().projection(), boundary + 1, false)
            .unwrap();
    let following_paragraph = core
        .document()
        .projection()
        .blocks()
        .iter()
        .find(|block| block.range.start == boundary + 1)
        .unwrap()
        .direct_paragraph
        .clone();
    let boundary_style = core
        .document()
        .projection()
        .blocks()
        .iter()
        .find(|block| block.range.contains(&(boundary - 1)))
        .unwrap()
        .style
        .clone();
    let expected = original_text.replacen("P>\n", "P>-\n\n", 1);
    assert_reopens(&core, &original_text);

    // Exact reported suffix, including the earlier insertion and its deletion.
    input(&mut core, view, InputEvent::key('A'));
    assert_eq!(core.command_state(view).unwrap().cursor(), boundary);
    input(&mut core, view, InputEvent::text("a"));
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.command_state(view).unwrap().cursor(), boundary);
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('a'));
    input(&mut core, view, InputEvent::text("-"));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(view).unwrap().cursor(), boundary + 1);
    assert_reopens(&core, &original_text.replacen("P>\n", "P>-\n", 1));
    input(&mut core, view, InputEvent::Key(Key::Enter));
    assert_eq!(core.command_state(view).unwrap().cursor(), boundary + 2);
    assert_reopens(&core, &expected);
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.start == boundary + 2)
            .unwrap()
            .style,
        boundary_style
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(core.document().projection(), boundary + 3, false)
            .unwrap(),
        following_style
    );
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.start == boundary + 3)
            .unwrap()
            .direct_paragraph,
        following_paragraph
    );
    let after = core.document().source_bytes();
    let following = source.find("<P").unwrap();
    assert!(after.ends_with(&source.as_bytes()[following..]));
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_reopens(&core, &original_text);
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), after);
    assert_reopens(&core, &expected);
}

#[test]
fn enter_after_implicitly_closed_list_paragraph_retains_one_new_hard_line() {
    // This reproduces the saved failure with too many generated newlines.
    // Opening directly as Html also failed, so conversion is not required.
    let source = "<ul data-keep='x'><li><p>before<p>1. العربية- 中\nP><P>&AMP;العربية&LT;BR&GT;A</P></li></ul><!--keep-->";
    for initial_format in [Format::HtmlSource, Format::Html] {
        exercise(source, initial_format);
    }
}

#[test]
fn enter_before_first_explicit_list_paragraph_preserves_its_boundary() {
    // Further minimization exposed the complementary case: the new list item
    // starts with <P>, so its authored paragraph separator must not disappear.
    for source in [
        "<ul data-keep='x'><li>P><P>&AMP;العربية</P></li></ul><!--keep-->",
        "<ul data-keep='x'><li class='evim-p-506172616772617068' data-keep='item'>P><P>&AMP;العربية</P></li></ul><!--keep-->",
    ] {
        for initial_format in [Format::HtmlSource, Format::Html] {
            exercise(source, initial_format);
        }
    }
}

#[test]
fn ordered_list_enter_preserves_authored_attributes_and_following_paragraph_styles() {
    let source = "<ol start='4' data-keep='list'><li data-keep='item'><p>before<p>P><P style='margin-left:18pt;text-align:right;font-size:21pt'><i>next</i></P></li></ol><!--keep-->";
    exercise(source, Format::Html);
}

#[test]
fn list_enter_does_not_reopen_inline_descendants_of_implicitly_closed_paragraph() {
    let source = "<ul data-keep='list'><li><p>before<span data-keep='closed' style='color:red'> text<p>P><P>next</P></li></ul><!--keep-->";
    exercise(source, Format::Html);
}
