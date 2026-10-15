use viem_core::command::{CommandStatus, InputEvent, Key, LineMode};
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
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for key in text.chars() {
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
        assert!(matches!(
            output.command.unwrap().status,
            CommandStatus::Pending | CommandStatus::Complete
        ));
    }
}
fn ex(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) -> CommandStatus {
    keys(core, view, text);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap()
        .status
}

#[test]
fn numeric_substitute_selects_visible_contributors_from_physical_lines() {
    let original = "**aa** aa\nbb aa\n\ncc aa";
    for format in [Format::Markdown] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            let (mut core, view) = editor(original, format, 55.);
            core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
            assert_eq!(ex(&mut core, view, ":1,2s/aa/X/"), CommandStatus::Complete);
            assert_eq!(
                core.document().source_bytes(),
                b"**X** aa\nbb X\n\ncc aa",
                "{format:?} {mode:?}"
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), original.as_bytes());
            assert_eq!(ex(&mut core, view, ":2s/aa/Y/g"), CommandStatus::Complete);
            assert_eq!(core.document().source_bytes(), b"**aa** aa\nbb Y\n\ncc aa");
        }
    }
}

#[test]
fn numeric_substitution_does_not_match_hidden_markup_or_cross_its_source_range() {
    let original = "aa\nbb\n\ncc";
    let (mut core, view) = editor(original, Format::Markdown, 400.);
    assert!(matches!(
        ex(&mut core, view, ":1s/aa bb/X/"),
        CommandStatus::ExError(_)
    ));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    assert_eq!(
        ex(&mut core, view, ":1,2s/aa bb/X/"),
        CommandStatus::Complete
    );
    assert_eq!(core.document().text(), "X\ncc");
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), original.as_bytes());

    let (mut core, view) = editor("**aa**\nnext", Format::Markdown, 400.);
    assert!(matches!(
        ex(&mut core, view, ":1s/\\*\\*/X/"),
        CommandStatus::ExError(_)
    ));
    assert_eq!(core.document().source_bytes(), b"**aa**\nnext");
}

#[test]
fn relative_substitute_replaces_only_the_current_wrapped_row() {
    let original = "aa aa aa aa aa aa aa aa";
    let (mut core, view) = editor(original, Format::PlainText, 55.);
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[1]
        .text_range
        .clone();
    assert!(row.end < original.len());
    keys(&mut core, view, "j0");
    let expected = format!(
        "{}{}{}",
        &original[..row.start],
        original[row.clone()].replace("aa", "XX"),
        &original[row.end..]
    );
    assert_eq!(ex(&mut core, view, ":.s/aa/XX/g"), CommandStatus::Complete);
    assert_eq!(core.document().text(), expected);
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn current_visual_line_count_and_last_address_use_displayed_rows() {
    let original = "aa aa aa aa aa aa aa aa\nlast aa aa aa";
    let (mut core, view) = editor(original, Format::PlainText, 55.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let end = snapshot.rows[1].text_range.end;
    let expected = format!(
        "{}{}",
        original[..end].replace("aa", "XX"),
        &original[end..]
    );
    assert_eq!(
        ex(&mut core, view, ":.s/aa/XX/g 2"),
        CommandStatus::Complete
    );
    assert_eq!(core.document().text(), expected);
    keys(&mut core, view, "u");
    let last = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .last()
        .unwrap()
        .text_range
        .clone();
    let expected = format!(
        "{}{}{}",
        &original[..last.start],
        original[last.clone()].replace("aa", "XX"),
        &original[last.end..]
    );
    assert_eq!(ex(&mut core, view, ":$s/aa/XX/g"), CommandStatus::Complete);
    assert_eq!(core.document().text(), expected);
}
