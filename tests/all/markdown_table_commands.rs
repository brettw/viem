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
#[test]
fn line_commands_crossing_cells_report_the_supported_table_actions() {
    for prefix in ["", "> ", "- "] {
        let source = format!("{prefix}| A | B |\n{}| - | - |\n{}| x | y |",
            if prefix == "- " { "  " } else { prefix },
            if prefix == "- " { "  " } else { prefix });
        for keys in ["J", "gJ", "2dd"] {
            let (mut core,view) = editor(&source);
            let mut status = None;
            for ch in keys.chars() {
                status = core.handle_with_layout(view,CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
                    .unwrap().command.map(|output|output.status);
            }
            assert!(matches!(status,Some(viem_core::command::CommandStatus::Unsupported(ref message))
                if message.contains("cell selection or a row action")),"{keys}: {status:?}");
            assert_eq!(core.document().source_bytes(),source.as_bytes());
        }
    }
}
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
fn editing_only_header_cell_retains_table_structure() {
    for source in ["| a |\n| --- |", "|a|\n|-|\n\ntext"] {
        for (keys, expected) in [
            (vec![Key::Char('x')], ""),
            (vec![Key::Char('A'), Key::Backspace, Key::Escape], ""),
            (vec![Key::Char('c'), Key::Char('i'), Key::Char('w'), Key::Char('z'), Key::Escape], "z"),
            (vec![Key::Char('~')], "A"),
            (vec![Key::Char('r'), Key::Char('z')], "z"),
        ] {
            let (mut core, view) = editor(source);
            for key in keys {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            }
            let tables = core.document().projection().tables();
            assert_eq!(tables.len(), 1, "{source:?}, expected {expected:?}");
            assert_eq!(tables[0].rows.len(), 1);
            assert_eq!(tables[0].columns.len(), 1);
            assert_eq!(&core.document().text()[tables[0].rows[0].cells[0].range.clone()], expected);
            let bytes = core.document().source_bytes();
            let fresh = Document::from_bytes(bytes.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(fresh.text(), core.document().text());
            assert_eq!(fresh.projection().tables().len(), 1);
            core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), bytes);
        }
    }
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

#[test]
fn code_span_boundary_affinity_matches_toolbar_and_typing_in_prose_and_cells() {
    use viem_core::document::BoundaryAffinity;
    for source in [
        "prefix `c|d` tail",
        "prefix `c|d`\n\nAfter",
        "prefix `c|d`",
        "| H | V |\n| - | - |\n| prefix `c\\|d` | z |\n\nAfter",
        "| H | V |\n| - | - |\n| z | prefix `c\\|d` |",
    ] {
        let code = if source.starts_with('|') {
            "`c\\|d`"
        } else {
            "`c|d`"
        };
        for at_end in [false, true] {
            for affinity in [BoundaryAffinity::Downstream, BoundaryAffinity::Upstream] {
                let (mut core, view) = editor(source);
                let start = core.document().text().find("c|d").unwrap();
                let at = start + if at_end { 3 } else { 0 };
                core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                    .unwrap();
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: at,
                        affinity,
                        extend_selection: false,
                    },
                )
                .unwrap();
                let inside = at_end == (affinity == BoundaryAffinity::Upstream);
                assert_eq!(
                    core.selected_named_styles(view)
                        .unwrap()
                        .character
                        .as_ref()
                        .map(|id| id.0.as_str()),
                    inside.then_some("Code"),
                    "before input: {source:?}, end={at_end}, {affinity:?}"
                );
                assert_eq!(core.document().is_code_at(at, affinity).unwrap(), inside);
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                for text in ["X", "Y"] {
                    core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                        .unwrap();
                }
                let replacement = match (at_end, inside) {
                    (true, false) => format!("{code}XY"),
                    (true, true) => format!("{}XY`", &code[..code.len() - 1]),
                    (false, false) => format!("XY{code}"),
                    (false, true) => format!("`XY{}", &code[1..]),
                };
                let expected = source.replacen(code, &replacement, 1);
                assert_eq!(
                    core.document().source_bytes(),
                    expected.as_bytes(),
                    "{source:?}, end={at_end}, {affinity:?}"
                );
                assert_eq!(
                    core.selected_named_styles(view)
                        .unwrap()
                        .character
                        .as_ref()
                        .map(|id| id.0.as_str()),
                    inside.then_some("Code")
                );
                let reopened = Document::from_bytes(
                    core.document().source_bytes(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                assert_eq!(reopened.text(), core.document().text());
                assert_eq!(
                    reopened
                        .is_code_at(at, BoundaryAffinity::Downstream)
                        .unwrap(),
                    inside
                );
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                    .unwrap();
                core.handle(
                    view,
                    CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
                )
                .unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle(
                    view,
                    CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
                )
                .unwrap();
                assert_eq!(core.document().source_bytes(), expected.as_bytes());
            }
        }
    }
}

#[test]
fn append_to_code_span_uses_upstream_typing_context_in_toolbar() {
    use viem_core::document::BoundaryAffinity;
    for source in [
        "prefix `c|d`\n\nAfter",
        "| H | V |\n| - | - |\n| prefix `c\\|d` | z |\n\nAfter",
    ] {
        for keys in ["a", "$a", "A"] {
            let (mut core, view) = editor(source);
            let end = core.document().text().find("c|d").unwrap() + 3;
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: end - 1,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            for key in keys.chars() {
                core.handle(view, CoreEvent::Input(InputEvent::key(key)))
                    .unwrap();
            }
            assert_eq!(
                core.selected_named_styles(view)
                    .unwrap()
                    .character
                    .unwrap()
                    .0,
                "Code",
                "{source:?}, {keys}"
            );
            core.handle(view, CoreEvent::Input(InputEvent::text("X")))
                .unwrap();
            let expected = source.replacen("d`", "dX`", 1);
            assert_eq!(core.document().source_bytes(), expected.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn downstream_code_exit_keeps_table_pipes_literal_and_following_input_outside_code() {
    use viem_core::document::BoundaryAffinity;
    let source = "| H | V |\n| - | - |\n| prefix `c\\|d` | z |";
    let (mut core, view) = editor(source);
    let end = core.document().text().find("c|d").unwrap() + 3;
    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: end,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text("|*")))
        .unwrap();
    let expected = source.replacen("d`", "d`\\|\\*", 1);
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_eq!(core.document().projection().tables()[0].columns.len(), 2);
    assert!(!core
        .document()
        .is_code_at(end, BoundaryAffinity::Downstream)
        .unwrap());
    assert_eq!(
        core.document().text()[end..]
            .chars()
            .take(2)
            .collect::<String>(),
        "|*"
    );
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn audit_table_duplicate_then_type_keeps_ordered_metadata() {
    for command in ["t.", "t0"] {
        let (mut core, view) = editor("| a |\n| --- |\n\nx");
        for ch in format!(":{command}").chars() {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(ch))).unwrap();
        }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        let tables = core.document().projection().tables();
        assert_ne!(tables[0].id, tables[1].id);
        assert!(tables.iter().all(|table| table.id != 0));
        for ch in ['i','q'] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(ch))).unwrap();
        }
        let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), core.document().text());
    }
}

#[test]
fn audit_replace_enter_backspace_restores_table_quote_and_code_bytes() {
    for (source, format, at) in [
        ("| a | b |\n| - | - |\n| 1 | 2 |", Format::Markdown, 0),
        ("- | a | b |\n  | - | - |\n  | 1 | 2 |", Format::Markdown, 0),
        (">", Format::Markdown, 0),
        ("fn main() {\n    println!(\"hi\");\n}\n", Format::Code, 13),
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: viem_core::document::BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        for key in [Key::Char('R'), Key::Enter, Key::Backspace, Key::Escape] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source}");
    }
}

#[test]
fn audit_gutter_rows_clear_or_replace_cells_and_delete_paragraph_owners() {
    for (source, expected) in [("# T\n\nend", "end"), ("> q\n\nz", "z")] {
        let (mut core, view) = editor(source);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let row = &snapshot.rows[0];
        core.handle(view, CoreEvent::SelectPointerRow { document_revision: core.document().revision(),
            layout_revision: snapshot.revision, y: row.y + row.height()/2., extend_selection: false }).unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Backspace))).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let source = "| a | b |\n| - | - |\n| 1 | 2 |\n\nend";
    for key in [Key::Backspace, Key::Delete, Key::Char('x')] {
        let (mut core, view) = editor(source);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let row = snapshot.rows.iter().find(|row| row.table_cell.as_ref().is_some_and(|cell| cell.row == 1)).unwrap();
        core.handle(view, CoreEvent::SelectPointerRow { document_revision: core.document().revision(),
            layout_revision: snapshot.revision, y: row.y + row.height()/2., extend_selection: false }).unwrap();
        assert!(core.table_selection(view).unwrap().is_some());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        let table = &core.document().projection().tables()[0];
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[1].cells[1].range.is_empty());
        assert_eq!(table.rows[1].cells[0].range.len(), usize::from(key == Key::Char('x')));
        assert!(core.document().text().ends_with("end"));
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn audit_insert_enter_backspace_restores_original_emphasis_spelling() {
    for source in ["*it*", "_it_", "*a\nb*"] {
        let (mut core, view) = editor(source);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: 1, affinity: viem_core::document::BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        for key in [Key::Enter, Key::Backspace] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source}");
    }
}

#[test]
fn audit_native_typing_into_quoted_empty_cells() {
    for prefix in ["> ", "> > "] {
        let source = format!("{prefix}|  |  |\n{prefix}| - | - |\n{prefix}|  |  |");
        for row in 0..2 {
            for column in 0..2 {
                let (mut core, view) = editor(&source);
                core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
                let at = core.document().projection().tables()[0].rows[row].cells[column].range.start;
                core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                    text_offset: at, affinity: viem_core::document::BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("x"))).unwrap();
                assert!(String::from_utf8(core.document().source_bytes()).unwrap().contains("| x |"));
            }
        }
    }
}

const WHOLE_TABLE_SOURCES: [&str; 4] = [
    SOURCE,
    "before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter",
    "- item\n\n  | A | B |\n  | - | - |\n  | x | y |\n\nafter",
    "> | A | B |\n> | - | - |\n> | x | y |\n\nafter",
];

/// Remove the table's text exactly as an ordinary selection of it would.
fn assert_table_replaced(core: &mut Editor, view: ViewId, source: &str, replacement: &str, label: &str) {
    let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut expected = original.text().to_owned();
    expected.replace_range(original.projection().tables()[0].range.clone(), replacement);
    assert_eq!(core.document().text(), expected, "{label}: {source:?}");
    assert!(core.document().projection().tables().is_empty(), "{label}: {source:?}");
    // The text left in the table's place stays in the table's owners.
    let owners = |document: &Document, at: usize| document.projection().blocks().iter()
        .find(|block| block.range.start <= at && at <= block.range.end)
        .map(|block| (block.quote_depth, block.containers.iter()
            .map(|member| member.container.kind).collect::<Vec<_>>()));
    let start = original.projection().tables()[0].range.start;
    assert_eq!(owners(core.document(), start), owners(&original, start), "{label}: {source:?}");
    assert!(core.table_selection(view).unwrap().is_none(), "{label}: {source:?}");
    let edited = core.document().source_bytes();
    let fresh = Document::from_bytes(edited.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(fresh.text(), expected, "{label}: {source:?}");
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes(), "{label}: {source:?}");
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), edited, "{label}: {source:?}");
}

/// Name the whole rectangle by one of its four pairs of opposite corners.
fn select_every_cell(core: &mut Editor, view: ViewId, corners: usize) {
    let table = &core.document().projection().tables()[0];
    let (rows, columns) = (table.rows.len() - 1, table.columns.len() - 1);
    let (a, b) = [((0, 0), (rows, columns)), ((rows, columns), (0, 0)),
        ((0, columns), (rows, 0)), ((rows, 0), (0, columns))][corners];
    select(core, view, a, b);
}

#[test]
fn every_cell_selected_deletion_and_typing_replace_the_whole_table() {
    for source in WHOLE_TABLE_SOURCES {
        for corners in 0..4 {
            for (input, replacement) in [(InputEvent::Key(Key::Delete), ""), (InputEvent::text("new"), "new")] {
                let (mut core, view) = editor(source);
                select_every_cell(&mut core, view, corners);
                let label = format!("{input:?} corners={corners}");
                core.handle_with_layout(view, CoreEvent::Input(input.clone()))
                    .unwrap_or_else(|error| panic!("{label}: {source:?}: {error:?}"));
                if !replacement.is_empty() {
                    assert_eq!(core.command_state(view).unwrap().mode(), viem_core::command::Mode::Insert, "{label}");
                    let table_start = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                        .unwrap().projection().tables()[0].range.start;
                    assert_eq!(core.command_state(view).unwrap().cursor(), table_start + replacement.len(), "{label}");
                }
                assert_table_replaced(&mut core, view, source, replacement, &label);
            }
        }
    }
}

#[test]
fn every_cell_selected_native_cut_and_vim_d_remove_the_whole_table() {
    for source in WHOLE_TABLE_SOURCES {
        let (mut core, view) = editor(source);
        select_every_cell(&mut core, view, 2);
        let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
        prefix(&mut core, view, &context);
        let outcome = core.handle(view, CoreEvent::InputWithClipboard {
            input: InputEvent::key('d'), clipboard: context,
        }).unwrap();
        assert_eq!(outcome.command.unwrap().clipboard_writes.len(), 1, "{source:?}");
        assert_table_replaced(&mut core, view, source, "", "native Cut");

        let (mut core, view) = editor(source);
        for key in ":set noautoselect\r".chars() {
            let key = if key == '\r' { Key::Enter } else { Key::Char(key) };
            core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        select_every_cell(&mut core, view, 3);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('d')))
            .unwrap_or_else(|error| panic!("Vim d: {source:?}: {error:?}"));
        assert_table_replaced(&mut core, view, source, "", "Vim d");
    }
}

fn place(core: &mut Editor, view: ViewId, at: usize, extend: bool) {
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(),
        text_offset: at,
        affinity: viem_core::document::BoundaryAffinity::Downstream,
        extend_selection: extend,
    })
    .unwrap();
}

#[test]
fn selection_reaching_into_a_table_from_outside_selects_the_whole_table() {
    for source in [
        "before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter",
        "- before\n\n  | A | B |\n  | - | - |\n  | x | y |\n\n  after",
        "> before\n>\n> | A | B |\n> | - | - |\n> | x | y |\n>\n> after",
    ] {
        let (mut core, view) = editor(source);
        let table = core.document().projection().tables()[0].range.clone();
        let text = core.document().text().to_owned();
        let (before, after) = (text.find("fore").unwrap(), text.find("ter").unwrap());
        let inside = text.find('B').unwrap();
        for (anchor, active, expected) in [
            (before, inside, before..table.end),
            (after, inside, table.start..after),
            (inside, before, before..table.end),
            (inside, after, table.start..after),
            (before, after, before..after),
            (before, table.start, before..table.start),
            (after, table.end + 1, table.end + 1..after),
        ] {
            place(&mut core, view, anchor, false);
            place(&mut core, view, active, true);
            let selection = core.list_selection_identity(view).unwrap();
            assert_eq!(selection.range(), expected, "{source:?}: {anchor} -> {active}");
            assert_eq!(selection.active(), active, "the caret stays where it was placed");
        }
        // A selection within one table keeps its exact cell-local extent.
        let last = text[..table.end].rfind('y').unwrap();
        place(&mut core, view, inside, false);
        place(&mut core, view, last, true);
        assert_eq!(core.list_selection_identity(view).unwrap().range(), inside..last, "{source:?}");
    }
    // Deleting or replacing such a selection removes the text and the table.
    let source = "before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter";
    for (input, expected) in [(InputEvent::Key(Key::Delete), "be\n\nafter"), (InputEvent::text("z"), "bez\n\nafter")] {
        let (mut core, view) = editor(source);
        let text = core.document().text().to_owned();
        place(&mut core, view, text.find("fore").unwrap(), false);
        place(&mut core, view, text.find('B').unwrap(), true);
        core.handle_with_layout(view, CoreEvent::Input(input.clone())).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{input:?}");
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{input:?}");
    }
    // Reaching from one table into another selects both completely.
    let source = "| A | B |\n| - | - |\n| x | y |\n\nbetween\n\n| C | D |\n| - | - |\n| z | w |";
    let (mut core, view) = editor(source);
    let tables = core.document().projection().tables().iter().map(|table| table.range.clone()).collect::<Vec<_>>();
    place(&mut core, view, tables[0].start + 2, false);
    place(&mut core, view, tables[1].start + 2, true);
    assert_eq!(core.list_selection_identity(view).unwrap().range(), tables[0].start..tables[1].end);
}

