use evim_core::command::{CommandStatus, InputEvent, Key, LineMode};
use evim_core::document::{Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for c in text.chars() {
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(c))))
            .unwrap();
        assert!(matches!(
            output.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ));
    }
}

#[test]
fn physical_line_motions_and_deletion_preserve_hidden_source_separator_coordinates() {
    let source = "One\r\n\r\nTwo\r\n\r\n3. Item\r\n";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_eq!(document.text(), "One\nTwo\n3. Item\n");
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 400.);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "gg2j");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        core.document().text().find("Two").unwrap()
    );
    keys(&mut core, view, "\"add");
    assert_eq!(
        core.document().source_bytes(),
        b"One\r\n\r\n\r\n3. Item\r\n"
    );
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        "Two\n"
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::SetLineMode(LineMode::Visual))
        .unwrap();
    keys(&mut core, view, "ggjx");
    assert_eq!(
        core.document().source_bytes(),
        b"One\r\n\r\nwo\r\n\r\n3. Item\r\n"
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
