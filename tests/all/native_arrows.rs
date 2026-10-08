use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert!(matches!(result.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending));
}

#[test]
fn normal_arrows_cross_image_paragraphs_both_directions_without_changing_source() {
    let source = "a\u{301}b\n\n![diagram](local.png)\n\nz";
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let image_at = document.text().find('\u{fffc}').unwrap();
    let last_at = document.text().rfind('z').unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    key(&mut core, view, Key::End);
    assert_eq!(core.command_state(view).unwrap().cursor(), 3);
    key(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().cursor(), image_at);
    key(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().cursor(), last_at);
    key(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().cursor(), last_at);
    key(&mut core, view, Key::Left);
    assert_eq!(core.command_state(view).unwrap().cursor(), image_at);
    for key_value in [Key::Char('h'), Key::Char('l')] {
        key(&mut core, view, key_value);
        assert_eq!(core.command_state(view).unwrap().cursor(), image_at, "Vim h/l remain line-limited");
    }
    key(&mut core, view, Key::Left);
    assert_eq!(core.command_state(view).unwrap().cursor(), 3);
    key(&mut core, view, Key::Left);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn counted_native_arrows_keep_empty_lines_and_unicode_atomic() {
    let mut core = Core::new(Document::new("a\u{301}😀\n\nb"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    for (key_value, expected) in [(Key::Right, 3), (Key::Right, 8), (Key::Right, 9), (Key::Left, 8), (Key::Left, 3)] {
        key(&mut core, view, key_value);
        assert_eq!(core.command_state(view).unwrap().cursor(), expected);
    }
    key(&mut core, view, Key::Char('9'));
    key(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().cursor(), 9);
    key(&mut core, view, Key::Char('9'));
    key(&mut core, view, Key::Left);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
}

#[test]
fn operator_arrows_retain_vim_line_limited_edit_semantics() {
    let mut core = Core::new(Document::new("ab\ncd"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    key(&mut core, view, Key::End);
    key(&mut core, view, Key::Char('d'));
    key(&mut core, view, Key::Right);
    assert_eq!(core.document().text(), "ab\ncd", "operator Right must not cross the line boundary");
    key(&mut core, view, Key::Left);
    key(&mut core, view, Key::Char('d'));
    key(&mut core, view, Key::Right);
    assert_eq!(core.document().text(), "b\ncd");
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "ab\ncd");
}
