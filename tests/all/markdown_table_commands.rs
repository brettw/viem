use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{InputEvent, Key, RegisterValue, TableSelectionExtent};
use viem_core::document::{Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};
type Editor = Core<MockTextMeasurementProvider>;
const SOURCE: &str = "| A | B | C |\n| - | - | - |\n| x | **y** | z |\n";
fn editor(source: &str) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}
fn select(core: &mut Editor, view: ViewId, a: (usize, usize), b: (usize, usize)) {
    let table = core.document().projection().tables()[0].id;
    core.select_table_cells(
        view,
        core.document().id(),
        core.document().revision(),
        Some(TableSelectionExtent {
            table,
            anchor_row: a.0,
            anchor_column: a.1,
            active_row: b.0,
            active_column: b.1,
        }),
    )
    .unwrap();
}
fn matrix_context(
    source: &str,
    rows: std::ops::Range<usize>,
    cols: std::ops::Range<usize>,
) -> ClipboardCommandContext {
    let (core, _) = editor(source);
    let (fragment, _) = core
        .document()
        .table_clipboard_fragment(core.document().projection().tables()[0].id, rows, cols)
        .unwrap();
    let value = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        ClipboardContent::from_register(value),
    ))
}
fn prefix(core: &mut Editor, view: ViewId, context: &ClipboardCommandContext) {
    for input in [
        InputEvent::Key(Key::Ctrl('o')),
        InputEvent::key('"'),
        InputEvent::key('+'),
    ] {
        core.handle(
            view,
            CoreEvent::InputWithClipboard {
                input,
                clipboard: context.clone(),
            },
        )
        .unwrap();
    }
}
#[test]
fn native_copy_keeps_rectangular_selection_and_rich_matrix() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (1, 1), (0, 0));
    let before = core.table_selection(view).unwrap();
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    let outcome = core
        .handle(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::CopySelection),
                clipboard: context,
            },
        )
        .unwrap();
    let command = outcome.command.unwrap();
    let value = command.clipboard_writes[0]
        .content()
        .portable_register()
        .unwrap();
    assert_eq!(value.text, "A\tB\nx\ty");
    assert_eq!(
        value
            .clipboard_fragment()
            .unwrap()
            .table_cells()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(core.table_selection(view).unwrap(), before);
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}
#[test]
fn native_cut_publishes_register_after_preserving_table_shape() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (0, 0), (1, 1));
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    prefix(&mut core, view, &context);
    let outcome = core
        .handle(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::key('d'),
                clipboard: context,
            },
        )
        .unwrap();
    assert_eq!(core.document().text(), "\n\nC\n\n\nz");
    assert_eq!(
        outcome.command.unwrap().clipboard_writes[0]
            .content()
            .plain_text(),
        "A\tB\nx\ty"
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}
#[test]
fn native_matrix_paste_preserves_styles_and_requires_exact_dimensions() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (0, 1), (1, 2));
    let context = matrix_context("| m | n |\n| - | - |\n| **p** | q |\n", 0..2, 0..2);
    prefix(&mut core, view, &context);
    core.handle(
        view,
        CoreEvent::InputWithClipboard {
            input: InputEvent::key('p'),
            clipboard: context,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "A\nm\nn\nx\np\nq");
    assert!(String::from_utf8(core.document().source_bytes())
        .unwrap()
        .contains("**p**"));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
    select(&mut core, view, (0, 1), (1, 2));
    let context = matrix_context("| single |\n| - |\n", 0..1, 0..1);
    prefix(&mut core, view, &context);
    let before_selection = core.table_selection(view).unwrap();
    let before_register = core.command_state(view).unwrap().register('"').cloned();
    let before_revision = core.document().revision();
    let output = core
        .handle(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::key('p'),
                clipboard: context,
            },
        )
        .unwrap();
    assert!(matches!(output.command.unwrap().status,
        viem_core::command::CommandStatus::Error(message)
            if message.contains("1 × 1") && message.contains("2 × 2")));
    assert_eq!(core.document().revision(), before_revision);
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
    assert_eq!(core.table_selection(view).unwrap(), before_selection);
    assert_eq!(
        core.command_state(view).unwrap().register('"'),
        before_register.as_ref()
    );
}
#[test]
fn plain_multiline_paste_replaces_anchor_cell_without_interpreting_tsv() {
    let (mut core, view) = editor(SOURCE);
    select(&mut core, view, (1, 1), (0, 0));
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(2),
        ClipboardContent::from_plain_text("one\ttwo\nthree"),
    ));
    prefix(&mut core, view, &context);
    core.handle(
        view,
        CoreEvent::InputWithClipboard {
            input: InputEvent::key('p'),
            clipboard: context,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "\n\nC\n\none\ttwo\nthree\nz");
    assert_eq!(core.document().projection().tables()[0].columns.len(), 3);
}
#[test]
fn dd_clears_only_active_cell_line_and_undo_restores_exact_bytes() {
    let (mut core, view) = editor("| abc<br>def | neighbor |\n| - | - |\n| body | other |\n");
    let original = core.document().source_bytes();
    for input in [InputEvent::key('d'), InputEvent::key('d')] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert_eq!(core.document().text(), "def\nneighbor\nbody\nother");
    assert_eq!(core.document().projection().tables()[0].rows.len(), 2);
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), original);
}
#[test]
fn whole_table_visual_selection_delete_removes_hidden_delimiter() {
    let (mut core, view) = editor(SOURCE);
    for input in [
        InputEvent::key('g'),
        InputEvent::key('g'),
        InputEvent::key('V'),
        InputEvent::key('G'),
        InputEvent::key('d'),
    ] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), b"");
    assert!(core.document().projection().tables().is_empty());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}
#[test]
fn one_column_dd_consumes_complete_row_and_promotes_header() {
    let source = "| header |\n| :--: |\n| body |\n| last |";
    let (mut core, view) = editor(source);
    for input in [InputEvent::key('d'), InputEvent::key('d')] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert_eq!(core.document().text(), "body\nlast");
    assert_eq!(core.document().projection().tables()[0].rows.len(), 2);
    assert_eq!(
        core.document().projection().tables()[0].columns[0],
        viem_core::document::TableAlignment::Center
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn insert_register_matrix_grows_right_and_down_with_exact_undo() {
    let source = "| H |\n| :-: |";
    let (mut core, view) = editor(source);
    let context = matrix_context("| A | B |\n| - | - |\n| x | y |", 0..2, 0..2);
    for input in [
        InputEvent::key('i'),
        InputEvent::Key(Key::Ctrl('r')),
        InputEvent::key('+'),
        InputEvent::Key(Key::Escape),
    ] {
        core.handle(
            view,
            CoreEvent::InputWithClipboard {
                input,
                clipboard: context.clone(),
            },
        )
        .unwrap();
    }
    assert_eq!(core.document().text(), "A\nB\nx\ny");
    assert_eq!(
        core.document().projection().tables()[0].columns,
        vec![viem_core::document::TableAlignment::Center; 2]
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn header_only_tab_appends_row_and_shift_tab_at_first_cell_is_noop() {
    let source = "| H |\n| :-: |";
    let (mut core, view) = editor(source);
    for input in [InputEvent::key('i'), InputEvent::Key(Key::BackTab)] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Tab)))
        .unwrap();
    assert_eq!(core.document().projection().tables()[0].rows.len(), 2);
    core.handle(view, CoreEvent::Input(InputEvent::text("value")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(core.document().text(), "H\nvalue");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn native_cut_and_paste_mixed_rich_matrix_with_empty_cells_round_trips() {
    let source="# Table interaction test\n\n| Feature | Example | Status |\n| :--- | :---: | ---: |\n| Bold | **bold**<br>Hello, world! | ready |\n| Italic | *italic* | test |\n| Empty | | |\n\nAfter the table.\n";
    for insert in [false, true] {
        let (mut core, view) = editor(source);
        select(&mut core, view, (1, 1), (3, 2));
        let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
        prefix(&mut core, view, &context);
        let outcome = core
            .handle(
                view,
                CoreEvent::InputWithClipboard {
                    input: InputEvent::key('d'),
                    clipboard: context,
                },
            )
            .unwrap();
        let command = outcome.command.unwrap();
        let copied = command.clipboard_writes[0].content().clone();
        let text = copied.plain_text();
        let fragment = copied
            .portable_register()
            .unwrap()
            .clipboard_fragment()
            .unwrap();
        let validated =
            viem_core::document::ClipboardFragment::from_json(fragment.json(), text).unwrap();
        let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            ClipboardTarget::Clipboard,
            ClipboardGeneration(7),
            ClipboardContent::from_register(
                RegisterValue::from_clipboard_fragment(validated).unwrap(),
            ),
        ));
        let sequence = if insert {
            vec![
                InputEvent::key('i'),
                InputEvent::Key(Key::Ctrl('r')),
                InputEvent::key('+'),
            ]
        } else {
            vec![
                InputEvent::key('"'),
                InputEvent::key('+'),
                InputEvent::key('p'),
            ]
        };
        for input in sequence {
            core.handle(
                view,
                CoreEvent::InputWithClipboard {
                    input,
                    clipboard: context.clone(),
                },
            )
            .unwrap();
        }
        assert!(core
            .document()
            .text()
            .contains("bold\nHello, world!\nready"));
        assert!(core.document().text().contains("italic\ntest"));
        assert_eq!(core.document().projection().tables()[0].rows.len(), 4);
        for style in [
            viem_core::document::SemanticInlineStyle::Strong,
            viem_core::document::SemanticInlineStyle::Emphasis,
        ] {
            for enabled in [false, true] {
                let document = core.document();
                let range = document.projection().tables()[0].rows[1].cells[1]
                    .range
                    .clone();
                document
                    .prepare_model_request(viem_core::document::ModelRequest::SetSemanticStyle {
                        document: document.id(),
                        revision: document.revision(),
                        range,
                        style,
                        enabled,
                    })
                    .unwrap_or_else(|error| {
                        panic!(
                            "{style:?} {enabled}: {error:?}, source={:?}",
                            String::from_utf8_lossy(&document.source_bytes())
                        )
                    });
            }
        }
    }
}
#[test]
fn appending_with_tab_closes_previous_cell_typing_undo_group() {
    let source = "| H |\n| - |";
    let (mut core, view) = editor(source);
    for input in [
        InputEvent::key('i'),
        InputEvent::text("first"),
        InputEvent::Key(Key::Tab),
        InputEvent::text("second"),
        InputEvent::Key(Key::Escape),
    ] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "firstH");
    assert_eq!(core.document().projection().tables()[0].rows.len(), 1);
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn open_line_commands_create_internal_cell_breaks_without_adding_table_rows() {
    for key in ['o', 'O'] {
        let (mut core, view) = editor("| head | neighbor |\n| - | - |\n| body | other |\n");
        for input in [
            InputEvent::key(key),
            InputEvent::text("new"),
            InputEvent::Key(Key::Escape),
        ] {
            core.handle(view, CoreEvent::Input(input)).unwrap();
        }
        assert_eq!(core.document().projection().tables()[0].rows.len(), 2);
        assert_eq!(
            core.document().text(),
            if key == 'o' {
                "head\nnew\nneighbor\nbody\nother"
            } else {
                "new\nhead\nneighbor\nbody\nother"
            }
        );
    }
}

#[test]
fn counted_matrix_put_is_rejected_in_normal_mode_and_cell_selection() {
    for rectangle in [false, true] {
        let (mut core, view) = editor(SOURCE);
        if rectangle {
            select(&mut core, view, (0, 0), (1, 1));
        }
        let context = matrix_context("| m | n |\n| - | - |\n| p | q |", 0..2, 0..2);
        if rectangle {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
                .unwrap();
        }
        for key in ['2', '"', '+'] {
            core.handle(
                view,
                CoreEvent::InputWithClipboard {
                    input: InputEvent::key(key),
                    clipboard: context.clone(),
                },
            )
            .unwrap();
        }
        let selection = core.table_selection(view).unwrap();
        let cursor = core.command_state(view).unwrap().cursor();
        let mode = core.command_state(view).unwrap().mode();
        let history = core.document().history_status();
        let register = core.command_state(view).unwrap().register('"').cloned();
        let output = core
            .handle(
                view,
                CoreEvent::InputWithClipboard {
                    input: InputEvent::key('p'),
                    clipboard: context,
                },
            )
            .unwrap_or_else(|error| panic!("rectangle={rectangle}: {error:?}"));
        let status = output.command.unwrap().status;
        assert!(
            matches!(&status,
            viem_core::command::CommandStatus::Unsupported(message) if message.contains("Counted table matrix paste")),
            "rectangle={rectangle}, status={status:?}"
        );
        assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.table_selection(view).unwrap(), selection);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        assert_eq!(core.command_state(view).unwrap().mode(), mode);
        assert_eq!(
            core.command_state(view).unwrap().register('"'),
            register.as_ref()
        );
    }
}

#[test]
fn tab_selected_multiline_cell_exposes_and_applies_bold_and_italic() {
    use viem_core::document::SemanticInlineStyle;
    let source = "| Label | Value |\n| - | - |\n| row | **bold**<br>Hello, world! |";
    let (mut core, view) = editor(source);
    for input in [
        InputEvent::key('j'),
        InputEvent::key('i'),
        InputEvent::Key(Key::Tab),
    ] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert!(
        core.table_selection(view).unwrap().is_none(),
        "Tab selects text inside one cell"
    );
    for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
        let state = core
            .selection_semantic_style_presentation(view, style)
            .unwrap();
        assert!(state.can_set() && state.can_clear(), "{style:?}: {state:?}");
    }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style: SemanticInlineStyle::Emphasis,
            enabled: true,
        },
    )
    .unwrap();
    assert_eq!(
        core.document().text(),
        "Label\nValue\nrow\nbold\nHello, world!"
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn normal_matrix_put_replaces_existing_cell_scopes_and_keeps_outside_columns() {
    let (mut core, view) = editor(SOURCE);
    let context = matrix_context("| m | n |\n| - | - |\n| p | q |", 0..2, 0..2);
    for key in ['"', '+', 'p'] {
        core.handle(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::key(key),
                clipboard: context.clone(),
            },
        )
        .unwrap();
    }
    assert_eq!(core.document().text(), "m\nn\nC\np\nq\nz");
    assert!(!String::from_utf8(core.document().source_bytes())
        .unwrap()
        .contains("**"));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}

#[test]
fn typed_digit_replaces_insert_and_replace_origin_cell_rectangles() {
    for entry in ['i', 'R'] {
        let (mut core, view) = editor(SOURCE);
        core.handle(view, CoreEvent::Input(InputEvent::key(entry)))
            .unwrap();
        select(&mut core, view, (1, 1), (1, 2));
        core.handle(view, CoreEvent::Input(InputEvent::key('2')))
            .unwrap();
        assert_eq!(core.document().text(), "A\nB\nC\nx\n2\n");
        assert!(core.table_selection(view).unwrap().is_none());
        assert_eq!(
            core.command_state(view).unwrap().mode(),
            viem_core::command::Mode::Insert
        );
    }
}
