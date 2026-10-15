use viem_core::command::ex_execute::{ExFileRequest, ExFrontendRequest, ExInsertionPoint};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn editor(source: &str, format: Format, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 240.);
    (core, view)
}
fn read_request(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    text: &str,
) -> ExInsertionPoint {
    for key in text.chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
    }
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
    output
        .ex_outcome
        .unwrap()
        .frontend_requests
        .into_iter()
        .find_map(|request| match request {
            ExFrontendRequest::File(ExFileRequest::Read { after, .. }) => Some(after),
            _ => None,
        })
        .unwrap()
}
fn complete(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    after: ExInsertionPoint,
    text: &str,
) {
    let output = core
        .handle(
            view,
            CoreEvent::ReadFile {
                document: core.document().id(),
                revision: core.document().revision(),
                after,
                bytes: text.as_bytes().to_vec(),
            },
        )
        .unwrap();
    assert!(output.document_changed, "{:?}", output.command);
}

#[test]
fn numeric_read_uses_physical_source_boundary_and_restores_exact_bytes() {
    let original = "first\r\nsecond\r\n\r\nlast";
    let (mut core, view) = editor(original, Format::Code, 55.);
    let after = read_request(&mut core, view, ":1read imported.txt");
    assert_eq!(
        after,
        ExInsertionPoint {
            source: true,
            offset: 7
        }
    );
    complete(&mut core, view, after, "  imported\r\n");
    assert_eq!(
        core.document().source_bytes(),
        b"first\r\n  imported\r\nsecond\r\n\r\nlast"
    );
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn relative_read_retains_the_exact_wrapped_row_boundary_across_reflow() {
    let original = "one two three four five six seven eight";
    let (mut core, view) = editor(original, Format::PlainText, 55.);
    let end = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .end;
    assert!(end < original.len());
    let after = read_request(&mut core, view, ":.read imported.txt");
    assert_eq!(
        after,
        ExInsertionPoint {
            source: false,
            offset: end
        }
    );
    core.handle(
        view,
        CoreEvent::Resize {
            width: 400.,
            height: 240.,
        },
    )
    .unwrap();
    complete(&mut core, view, after, "imported\n");
    assert_eq!(
        core.document().text(),
        format!("{}\nimported\n{}", &original[..end], &original[end..])
    );
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}
