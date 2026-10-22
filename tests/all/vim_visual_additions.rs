//! Visual commands and text objects added after the command audit: Visual
//! `Y`, Visual number changes, tag blocks and the cursor a Visual yank leaves.
//! Each case enters through the core coordinator with mock layout.

use viem_core::command::{CommandOutput, CommandStatus, InputEvent, Key, LineMode, Mode, RegisterKind};
use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn editor(text: &str, format: Format) -> (TestCore, ViewId) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    (core, view)
}

fn text_editor(text: &str) -> (TestCore, ViewId) {
    editor(text, Format::PlainText)
}

/// Parse `<Esc>`, `<CR>`, `<BS>`, `<C-x>` and `<Text:...>` tokens; every
/// other character is one key.
fn events(mut notation: &str) -> Vec<InputEvent> {
    let mut events = Vec::new();
    while !notation.is_empty() {
        if let Some(rest) = notation.strip_prefix("<Text:") {
            let end = rest.find('>').expect("terminated text token");
            events.push(InputEvent::Text(rest[..end].to_owned()));
            notation = &rest[end + 1..];
            continue;
        }
        let special = [("<Esc>", Key::Escape), ("<CR>", Key::Enter), ("<BS>", Key::Backspace)]
            .into_iter()
            .find(|(spelling, _)| notation.starts_with(spelling));
        if let Some((spelling, key)) = special {
            events.push(InputEvent::Key(key));
            notation = &notation[spelling.len()..];
            continue;
        }
        if let Some(rest) = notation.strip_prefix("<C-") {
            let character = rest.chars().next().expect("control key");
            assert_eq!(rest[character.len_utf8()..].chars().next(), Some('>'));
            events.push(InputEvent::Key(Key::Ctrl(character)));
            notation = &rest[character.len_utf8() + 1..];
            continue;
        }
        let character = notation.chars().next().unwrap();
        events.push(InputEvent::Key(Key::Char(character)));
        notation = &notation[character.len_utf8()..];
    }
    events
}

/// Send every event and return the final command output, failing on any
/// unsupported or failed step.
fn run(core: &mut TestCore, view: ViewId, notation: &str) -> CommandOutput {
    let mut last = None;
    for event in events(notation) {
        let outcome = core.handle_with_layout(view, CoreEvent::Input(event.clone())).unwrap();
        let output = outcome.command.expect("input produces command output");
        assert!(
            matches!(output.status, CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled),
            "{notation:?} at {event:?}: {:?}",
            output.status
        );
        last = Some(output);
    }
    last.expect("notation has events")
}

/// Send every event and return the final status without asserting success.
fn attempt(core: &mut TestCore, view: ViewId, notation: &str) -> CommandStatus {
    let mut last = None;
    for event in events(notation) {
        let outcome = core.handle_with_layout(view, CoreEvent::Input(event)).unwrap();
        last = Some(outcome.command.expect("input produces command output").status);
    }
    last.expect("notation has events")
}

fn cursor(core: &TestCore, view: ViewId) -> usize {
    core.command_state(view).unwrap().cursor()
}

fn mode(core: &TestCore, view: ViewId) -> Mode {
    core.command_state(view).unwrap().mode()
}



fn register(core: &TestCore, view: ViewId, name: char) -> (String, RegisterKind) {
    let value = core.command_state(view).unwrap().register(name).expect("register is set");
    (value.text.clone(), value.kind)
}

#[test]
fn visual_capital_y_yanks_lines_and_visual_block_y_yanks_the_block() {
    let (mut core, view) = text_editor("one\ntwo\nthree");
    run(&mut core, view, "lvjY");
    assert_eq!(register(&core, view, '"'), ("one\ntwo\n".to_owned(), RegisterKind::Linewise));
    assert_eq!(mode(&core, view), Mode::Normal);
    assert_eq!(core.document().text(), "one\ntwo\nthree", "Y never edits");
    run(&mut core, view, "ggjV\"aY");
    assert_eq!(register(&core, view, 'a'), ("two\n".to_owned(), RegisterKind::Linewise));
    run(&mut core, view, "gg<C-v>jlY");
    assert_eq!(register(&core, view, '"'), ("on\ntw".to_owned(), RegisterKind::Blockwise));
    assert_eq!(mode(&core, view), Mode::Normal);
    assert_eq!(core.document().text(), "one\ntwo\nthree");
}

#[test]
fn visual_ctrl_a_and_ctrl_x_change_each_selected_line() {
    let (mut core, view) = text_editor("1\n1\n1");
    run(&mut core, view, "Vj<C-a>");
    assert_eq!(core.document().text(), "2\n2\n1");
    assert_eq!((mode(&core, view), cursor(&core, view)), (Mode::Normal, 0));
    run(&mut core, view, "u");
    assert_eq!(core.document().text(), "1\n1\n1", "one Visual change is one undo unit");
    run(&mut core, view, "ggVjg<C-a>");
    assert_eq!(core.document().text(), "2\n3\n1", "g CTRL-A is progressive");
    run(&mut core, view, "u");
    run(&mut core, view, "ggVG3g<C-x>");
    assert_eq!(core.document().text(), "-2\n-5\n-8");

    let (mut core, view) = text_editor("1\n1\n1\n1");
    run(&mut core, view, "Vj<C-a>jj.");
    assert_eq!(core.document().text(), "2\n2\n2\n2", "dot repeats over the same number of lines");

    let (mut core, view) = text_editor("a1 b2\nc3 d4");
    run(&mut core, view, "fbvj<C-a>");
    assert_eq!(core.document().text(), "a1 b3\nc4 d4", "characterwise regions start at the selection");
    let (mut core, view) = text_editor("x-5 123");
    run(&mut core, view, "f5v<C-a>");
    assert_eq!(core.document().text(), "x-6 123", "an unselected minus sign is not part of the number");
    run(&mut core, view, "f1v<C-a>");
    assert_eq!(core.document().text(), "x-6 223", "digits after the selection are not read");
    let (mut core, view) = text_editor("none\n7");
    run(&mut core, view, "Vj<C-x>");
    assert_eq!(core.document().text(), "none\n6", "lines without a number are skipped");
}

#[test]
fn visual_block_number_changes_use_the_block_columns() {
    let (mut core, view) = text_editor("1 1\n1 1\n1 1");
    run(&mut core, view, "l<C-v>jjl<C-a>");
    assert_eq!(core.document().text(), "1 2\n1 2\n1 2");
    assert_eq!(mode(&core, view), Mode::Normal);
    run(&mut core, view, "u");
    run(&mut core, view, "gg0l<C-v>jjlg<C-a>");
    assert_eq!(core.document().text(), "1 2\n1 3\n1 4");
    run(&mut core, view, "u");
    run(&mut core, view, "gg0l<C-v>jl2<C-x>");
    assert_eq!(core.document().text(), "1 -1\n1 -1\n1 1");
    run(&mut core, view, "j.");
    assert_eq!(core.document().text(), "1 -1\n1 -1\n1 -1",
        "dot repeats the two-column block, which no longer reaches the second row's digit");
}

#[test]
fn tag_block_text_objects_select_inner_and_whole_blocks() {
    let text = "<div>\n  <p>Hello <b>bold</b> text</p>\n</div>";
    let (mut core, view) = editor(text, Format::Code);
    run(&mut core, view, "/bold<CR>yit");
    assert_eq!(register(&core, view, '"').0, "bold");
    run(&mut core, view, "yat");
    assert_eq!(register(&core, view, '"').0, "<b>bold</b>");
    run(&mut core, view, "y2it");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text");
    run(&mut core, view, "/Hello<CR>dit");
    assert_eq!(core.document().text(), "<div>\n  <p></p>\n</div>");
    run(&mut core, view, "u/Hello<CR>dat");
    assert_eq!(core.document().text(), "<div>\n  \n</div>");
    run(&mut core, view, "u/Hello<CR>d2at");
    assert_eq!(core.document().text(), "");
    run(&mut core, view, "u");

    run(&mut core, view, "gg/<p<CR>lyit");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text", "a start tag selects its block");
    run(&mut core, view, "/<.p<CR>lyit");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text", "an end tag selects its block");
    run(&mut core, view, "gg0jyat");
    assert_eq!(register(&core, view, '"').0, "<p>Hello <b>bold</b> text</p>", "indentation selects the next block");

    run(&mut core, view, "/bold<CR>vit");
    assert_eq!(mode(&core, view), Mode::VisualCharacter);
    run(&mut core, view, "ity");
    assert_eq!(register(&core, view, '"').0, "<b>bold</b>", "a repeated Visual it includes the tags");
    run(&mut core, view, "/bold<CR>vatat");
    run(&mut core, view, "y");
    assert_eq!(register(&core, view, '"').0, "<p>Hello <b>bold</b> text</p>", "a repeated Visual at selects the parent");

    for (source, expected) in [
        ("<p>a<br>b</p>", "a<br>b"),
        ("<p>a<br/>b</p>", "a<br/>b"),
        ("<P class=\"x\">a b</p>", "a b"),
    ] {
        let (mut core, view) = editor(source, Format::Code);
        run(&mut core, view, "$hyit");
        assert_eq!(register(&core, view, '"').0, expected, "{source}");
    }
    let (mut core, view) = text_editor("no tags");
    let revision = core.document().revision();
    assert!(!matches!(attempt(&mut core, view, "dit"), CommandStatus::Complete));
    assert_eq!(core.document().revision(), revision);
}

/// Vim leaves the cursor at the start of the yanked region: the first
/// selected character, the block's top-left corner, or column 0 of the
/// anchor's line. A linewise yank keeps a cursor that is already on a line
/// above the anchor, column included.
#[test]
fn visual_yank_moves_the_cursor_to_the_region_start_like_vim() {
    // Selection keys, yank keys, and `Some(offset)` where the yank moves the
    // cursor or `None` where the cursor already starts the region.
    let cases: &[(&str, &str, Option<usize>)] = &[
        ("llVj", "y", Some(0)),
        ("llV", "y", Some(0)),
        ("ll2V", "y", Some(0)),
        ("llVj", "\"ay", Some(0)),
        ("llVj", "2y", Some(0)),
        ("jjlllVk", "y", None),
        ("jjlVk", "2y", None),
        ("jjlVk", "\"by", None),
        ("llvj", "y", Some(2)),
        ("llvj", "\"cy", Some(2)),
        ("jjlllvkh", "y", None),
        ("ll<C-v>jl", "y", Some(2)),
        ("ll<C-v>jl", "\"dy", Some(2)),
        ("jjlll<C-v>kkh", "y", None),
    ];
    for (format, source) in [
        (Format::PlainText, "alpha\nbravo\ncharlie\ndelta\n"),
        (Format::Code, "alpha\nbravo\ncharlie\ndelta\n"),
        (Format::Markdown, "alpha\n\nbravo\n\ncharlie\n\ndelta\n"),
    ] {
        for line_mode in [LineMode::Visual, LineMode::PhysicalSource] {
            for &(selection, yank, expected) in cases {
                let label = format!("{format:?} {line_mode:?} {selection}{yank}");
                let (mut core, view) = editor(source, format);
                core.handle(view, CoreEvent::SetLineMode(line_mode)).unwrap();
                run(&mut core, view, selection);
                let before = cursor(&core, view);
                let output = run(&mut core, view, yank);
                assert_eq!(mode(&core, view), Mode::Normal, "{label}");
                assert_eq!(cursor(&core, view), expected.unwrap_or(before), "{label}");
                assert!(output.cursor_moved || cursor(&core, view) == before, "{label}");
                assert!(!output.document_changed, "{label}");
                assert_eq!(core.document().source_bytes(), source.as_bytes(), "{label}");
                let name = yank.strip_prefix('"').and_then(|rest| rest.chars().next()).unwrap_or('"');
                let (text, kind) = register(&core, view, name);
                let expected_kind = if selection.contains("<C-v>") {
                    RegisterKind::Blockwise
                } else if selection.contains('V') {
                    RegisterKind::Linewise
                } else {
                    RegisterKind::Characterwise
                };
                assert_eq!(kind, expected_kind, "{label}: {text:?}");
                assert!(!text.is_empty(), "{label}");
            }
        }
    }
}

#[test]
fn visual_line_yank_cursor_keeps_vim_columns_in_visual_line_meaning() {
    for (format, source) in [
        (Format::PlainText, "alpha\nbravo\ncharlie\ndelta\n"),
        (Format::Markdown, "alpha\n\nbravo\n\ncharlie\n\ndelta\n"),
    ] {
        // Vim: `0llVjy` ends at (1, 0); `jjlllVky` at (2, 3); `jjlVk2y` at (2, 1).
        for (notation, expected, yanked) in [
            ("llVjy", 0, "alpha\nbravo\n"),
            ("jjlllVky", 9, "bravo\ncharlie\n"),
            ("jjlVk2y", 7, "bravo\ncharlie\n"),
        ] {
            let (mut core, view) = editor(source, format);
            run(&mut core, view, notation);
            assert_eq!(cursor(&core, view), expected, "{format:?} {notation}");
            assert_eq!(register(&core, view, '"'), (yanked.to_owned(), RegisterKind::Linewise), "{format:?} {notation}");
        }
    }
}

#[test]
fn visual_line_yank_cursor_starts_the_first_selected_row_or_physical_line() {
    let source = "one two three four five six seven eight nine ten\nnext line\n";
    for format in [Format::PlainText, Format::Code, Format::Markdown] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 120., 400.);
        core.handle(view, CoreEvent::SetLineMode(LineMode::Visual)).unwrap();
        run(&mut core, view, "gjg0");
        let row_start = cursor(&core, view);
        assert!(row_start > 0, "{format:?}: the first hard line wraps");
        run(&mut core, view, "llVjy");
        assert_eq!(cursor(&core, view), row_start, "{format:?}: Visual rows");

        core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource)).unwrap();
        run(&mut core, view, "ggllllV");
        let origin = cursor(&core, view);
        run(&mut core, view, "jy");
        assert_eq!(cursor(&core, view), 0, "{format:?}: physical lines from {origin}");
        run(&mut core, view, "jlVky");
        assert!(cursor(&core, view) < source.find('\n').unwrap(), "{format:?}: the cursor stays on the first line");
        run(&mut core, view, "ggVjy");
        assert_eq!(cursor(&core, view), 0, "{format:?}: physical lines");
        assert_eq!(register(&core, view, '"').1, RegisterKind::Linewise);
    }
}

/// `VjyP` puts the yanked lines above the first selected line, so they never
/// land inside the second row's block.
#[test]
fn visual_line_yank_then_put_above_lands_before_the_selection() {
    for (format, source, expected) in [
        (Format::PlainText, "Intro\nMid\nEnd\n", "Intro\nMid\nIntro\nMid\nEnd\n"),
        (Format::Code, "Intro\nMid\nEnd\n", "Intro\nMid\nIntro\nMid\nEnd\n"),
        (Format::Markdown, "Intro\n\nMid\n\nEnd\n", "Intro\n\nMid\n\nIntro\n\nMid\n\nEnd\n"),
        (
            Format::Markdown,
            "Intro\n\n```\ncode\n```\n\nEnd\n",
            "Intro\n\n```\ncode\n```\n\nIntro\n\n```\ncode\n```\n\nEnd\n",
        ),
    ] {
        let (mut core, view) = editor(source, format);
        run(&mut core, view, "Vjy");
        assert_eq!(cursor(&core, view), 0, "{format:?} {source:?}");
        run(&mut core, view, "P");
        assert_eq!(String::from_utf8_lossy(&core.document().source_bytes()), expected, "{format:?} {source:?}");
        run(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{format:?} {source:?}");
    }
}

/// (line, column), both 1-based, as Vim's `getpos(".")` reports them.
fn line_column(core: &TestCore, view: ViewId) -> (usize, usize) {
    let at = cursor(core, view);
    let text = core.document().text();
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    (text[..at].matches('\n').count() + 1, at - start + 1)
}

/// Expected positions and line counts were checked against Vim 9.2 with
/// `vim -Nu NONE`. `V` keeps the cursor column, a count moves it to the last
/// selected line at the same column, and Visual `o` swaps both ends, so the
/// operator still covers every selected line.
#[test]
fn visual_line_keeps_columns_and_o_swaps_the_whole_selection_in_both_line_meanings() {
    let source = "alpha one\nbravo two\ncharlie three\ndelta four\necho five\n";
    for format in [Format::PlainText, Format::Code] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            for (keys, position, lines) in [
                ("lllV", (1, 4), 5),
                ("3lll3V", (3, 6), 5),
                ("llllVj", (2, 5), 5),
                ("llllVjjk", (2, 5), 5),
                ("jlllVjjo", (2, 4), 5),
                ("jlllVjjoy", (2, 4), 5),
                ("jlllVjjy", (2, 1), 5),
                ("jlllVjjd", (2, 1), 2),
                ("jlllVjjod", (2, 1), 2),
                ("jlllVjjood", (2, 1), 2),
                ("3lll3Vd", (1, 1), 2),
            ] {
                let context = format!("{format:?} {mode:?} {keys}");
                let (mut core, view) = editor(source, format);
                core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
                run(&mut core, view, &format!("<Esc>{keys}"));
                assert_eq!(line_column(&core, view), position, "{context}");
                assert_eq!(core.document().text().lines().count(), lines, "{context}");
            }
        }
    }
}

#[test]
fn physical_visual_o_in_markdown_keeps_the_selected_source_lines() {
    let source = "alpha one\n\nbravo two\n\ncharlie three\n\ndelta four\n";
    for (keys, swapped) in [("jjVjjd", "jjVjjod"), ("jjjjVkkd", "jjjjVkkood")] {
        let result = |keys: &str| {
            let (mut core, view) = editor(source, Format::Markdown);
            core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource)).unwrap();
            run(&mut core, view, keys);
            core.document().source_bytes()
        };
        assert_eq!(result(swapped), result(keys), "{swapped} versus {keys}");
    }
}
