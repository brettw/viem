use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    assert!(matches!(
        result.command.unwrap().status,
        CommandStatus::Complete | CommandStatus::Pending
    ));
}

#[test]
fn every_wrapped_visual_row_can_be_deleted_with_exact_undo() {
    for body in [
        "alpha beta gamma delta epsilon zeta",
        "alpha beta <b>gamma delta</b> epsilon zeta",
        "<b>alpha beta</b> gamma <a href='target'>delta epsilon</a> zeta",
        "alpha&#32;beta <span style='color:red'>gamma delta</span> epsilon zeta",
        "alpha beta <b> </b> <!--keep--> gamma delta epsilon zeta",
        "alpha &fjlig; beta gamma delta epsilon zeta",
        "a&fjlig;b c d e f g h",
        "alpha &NotEqualTilde; beta gamma delta epsilon zeta",
        "alpha beta <code>gamma delta</code> epsilon zeta",
        "alpha beta <img src='kept'> gamma delta epsilon zeta",
    ] {
        for width in [1., 25., 95., 180.] {
            let source = format!("<p data-keep='yes'>{body}</p><!--tail-->");
            let mut initial = Core::new(html(&source));
            let view = initial.add_view(MockTextMeasurementProvider::new(), width, 3000.);
            let rows = initial
                .layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows
                .clone();
            let original = initial.document().text().to_owned();
            for row in rows.iter() {
                let mut core = Core::new(html(&source));
                let view = core.add_view(MockTextMeasurementProvider::new(), width, 3000.);
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: row.text_range.start,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                input(&mut core, view, Key::Char('V'));
                core.handle(view, CoreEvent::Input(InputEvent::key('d')))
                    .unwrap_or_else(|error| {
                        panic!(
                            "{source:?} width={width} row={:?}: {error:?}",
                            row.text_range
                        )
                    });
                let expected = format!(
                    "{}{}",
                    &original[..row.text_range.start],
                    &original[row.text_range.end..]
                );
                // At a newly exposed paragraph edge HTML needs a protective
                // NBSP for an existing visible space; no visible text is lost.
                assert_eq!(
                    core.document().text().replace('\u{a0}', " "),
                    expected,
                    "{source}: {:?}",
                    row.text_range
                );
                let register = &core
                    .command_state(view)
                    .unwrap()
                    .register('"')
                    .unwrap()
                    .text;
                assert_eq!(
                    register.trim_end_matches('\n'),
                    &original[row.text_range.clone()]
                );
                let changed = core.document().source_bytes();
                assert!(changed.starts_with(b"<p data-keep='yes'>"));
                assert!(changed.ends_with(b"</p><!--tail-->"));
                assert_eq!(
                    html(std::str::from_utf8(&changed).unwrap()).text(),
                    core.document().text()
                );
                input(&mut core, view, Key::Char('u'));
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                input(&mut core, view, Key::Ctrl('r'));
                assert_eq!(core.document().source_bytes(), changed);
            }
        }
    }
}

#[test]
fn final_wrapped_visual_row_delete_across_structural_contexts() {
    for source in [
        "<p>alpha beta gamma delta epsilon zeta</p><p>tail</p>",
        "<ul><li>alpha beta gamma delta epsilon zeta</li><li>tail</li></ul>",
        "<p>alpha beta gamma delta epsilon zeta<br>tail</p>",
        "<p>alpha beta gamma delta epsilon zeta<br></p><p>tail</p>",
        "<pre>alpha beta gamma delta epsilon zeta\ntail</pre>",
    ] {
        let mut core = Core::new(html(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 95., 500.);
        let original = core.document().text().to_owned();
        let paragraph_end = original.find('\n').unwrap();
        let row = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .filter(|row| row.text_range.start < paragraph_end)
            .last()
            .unwrap()
            .text_range
            .clone();
        assert!(row.start > 0);
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: row.start,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        input(&mut core, view, Key::Char('V'));
        core.handle(view, CoreEvent::Input(InputEvent::key('d')))
            .unwrap_or_else(|error| panic!("{source:?} row={row:?}: {error:?}"));
        assert!(core.document().text().ends_with("tail"));
        let changed = core.document().source_bytes();
        assert_eq!(
            html(std::str::from_utf8(&changed).unwrap()).text(),
            core.document().text()
        );
        input(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        input(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), changed);
    }
}

#[test]
fn visual_row_endpoint_inside_html_entity_preserves_unselected_characters() {
    for (range, expected) in [(1..2, "ajb"), (2..3, "afb"), (0..2, "jb"), (2..4, "af")] {
        let source = "<p>a&fjlig;b</p><!--tail-->";
        let mut document = html(source);
        document
            .delete_visual_text(range.clone())
            .unwrap_or_else(|error| panic!("{range:?}: {error:?}"));
        assert_eq!(document.text(), expected);
        assert_eq!(
            html(std::str::from_utf8(&document.source_bytes()).unwrap()).text(),
            expected
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
