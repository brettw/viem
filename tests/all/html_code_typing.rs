use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn event(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: CoreEvent) {
    let update = core.handle(view, input).unwrap();
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
fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    event(core, view, CoreEvent::Input(InputEvent::Key(key)));
}
fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn return_then_scalar_indentation_keeps_pre_source_compact_and_one_undo_unit() {
    for source in [
        "<pre>first_code()\n\nlast_code()</pre><!--keep-->",
        "<pre><code>first_code()\n\nlast_code()</code></pre><!--keep-->",
    ] {
        let mut core = Core::new(html(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
        key(&mut core, view, Key::Char('A'));
        key(&mut core, view, Key::Enter);
        for ch in "    added_line()".chars() {
            event(
                &mut core,
                view,
                CoreEvent::Input(InputEvent::text(ch.to_string())),
            );
        }
        key(&mut core, view, Key::Escape);
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(saved.contains("<br>    added_line()"), "{saved}");
        assert!(!saved.contains("white-space"), "{saved}");
        assert_eq!(html(&saved).text(), core.document().text());
        assert_eq!(
            core.document().text(),
            "first_code()\n    added_line()\n\nlast_code()"
        );
        assert!(saved.ends_with("<!--keep-->"));
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), saved.as_bytes());
    }
}

#[test]
fn literal_spaces_and_tabs_obey_pre_inheritance_and_normal_overrides() {
    for (source, at, literal) in [
        ("<pre></pre>", 0, true),
        ("<pre><code></code></pre>", 0, true),
        ("<pre>AB</pre>", 0, true),
        ("<pre>AB</pre>", 2, true),
        ("<pre><code>AB</code></pre>", 1, true),
        (
            "<pre><span style='white-space:normal'>AB</span></pre>",
            1,
            false,
        ),
        (
            "<pre><span style='white-space:normal'><code>AB</code></span></pre>",
            1,
            false,
        ),
        ("<p><code>AB</code></p>", 1, false),
    ] {
        let document = html(source);
        let before = document.text().to_owned();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
        key(&mut core, view, Key::Char('i'));
        let revision = core.document().revision();
        event(
            &mut core,
            view,
            CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: at,
                affinity: if at == before.len() {
                    BoundaryAffinity::Upstream
                } else {
                    BoundaryAffinity::Downstream
                },
                extend_selection: false,
            },
        );
        for ch in "  \t  ".chars() {
            event(
                &mut core,
                view,
                CoreEvent::Input(InputEvent::text(ch.to_string())),
            );
            let saved = String::from_utf8(core.document().source_bytes()).unwrap();
            assert_eq!(
                html(&saved).text(),
                core.document().text(),
                "{source}: {saved}"
            );
        }
        key(&mut core, view, Key::Escape);
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        if literal {
            assert!(saved.contains("  \t  "), "{saved}");
            assert!(!saved.contains("white-space"), "{saved}");
        } else {
            assert!(saved.contains("&nbsp;"), "{saved}");
            assert!(!saved.contains("white-space: pre-wrap"), "{saved}");
        }
        let mut expected = before;
        expected.insert_str(at, if literal { "  \t  " } else { "     " });
        assert_eq!(core.document().text().replace('\u{a0}', " "), expected);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn whitespace_insert_in_large_pre_has_one_local_patch_and_bounded_projection_work() {
    let source =
        "<pre><code>".to_owned() + &"code body\n".repeat(10_000) + "last</code></pre><!--keep-->";
    let mut document = html(&source);
    let at = "code body\n".len() * 5_000 + 4;
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "  \t  ")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 256);
    document.commit_model_transaction(prepared).unwrap();
    let mut expected = source.clone();
    expected.insert_str("<pre><code>".len() + at, "  \t  ");
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
