//! Normal commands added after the command audit: reports, quitting,
//! insert-position, change-list, match-selection, bracket, `z` and `g`
//! commands. Each case enters through the core coordinator with mock layout.

use viem_core::command::ex_execute::{ExFileRequest, ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandOutput, CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{
    ArtifactPath, ArtifactStorageProvider, Document, Encoding, FileFormat, Format,
    InMemoryArtifactStorage, LineEndingOpenPolicy,
};
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

fn messages(output: &CommandOutput) -> Vec<String> {
    output
        .ex_outcome
        .iter()
        .flat_map(|outcome| &outcome.frontend_requests)
        .filter_map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn file_requests(output: &CommandOutput) -> Vec<ExFileRequest> {
    output
        .ex_outcome
        .iter()
        .flat_map(|outcome| &outcome.frontend_requests)
        .filter_map(|request| match request {
            ExFrontendRequest::File(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn ga_and_g8_report_the_grapheme_under_the_cursor() {
    let (mut core, view) = text_editor("a\u{e9}e\u{301}\n\n\tx");
    let revision = core.document().revision();
    assert_eq!(messages(&run(&mut core, view, "ga")), ["<a>  97,  Hex 61,  Octal 141"]);
    assert_eq!(messages(&run(&mut core, view, "g8")), ["61 "]);
    run(&mut core, view, "l");
    assert_eq!(messages(&run(&mut core, view, "ga")), ["<\u{e9}> 233, Hex 00e9, Octal 351"]);
    assert_eq!(messages(&run(&mut core, view, "g8")), ["c3 a9 "]);
    run(&mut core, view, "l");
    assert_eq!(
        messages(&run(&mut core, view, "ga")),
        ["<e>  101,  Hex 65,  Octal 145 <\u{301}> 769, Hex 0301, Octal 1401"]
    );
    assert_eq!(messages(&run(&mut core, view, "g8")), ["65 + cc 81 "]);
    run(&mut core, view, "j");
    assert_eq!(messages(&run(&mut core, view, "ga")), ["NUL"]);
    assert_eq!(messages(&run(&mut core, view, "g8")), ["NUL"]);
    run(&mut core, view, "j0");
    assert_eq!(messages(&run(&mut core, view, "ga")), ["<^I>  9,  Hex 09,  Octal 011"]);
    assert_eq!(core.document().revision(), revision, "reports never edit");
    assert_eq!(mode(&core, view), Mode::Normal);
}

#[test]
fn ctrl_g_reports_name_state_and_position() {
    let path = ArtifactPath::from("notes.txt");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"one\ntwo\nthree\nfour\n".to_vec()).unwrap();
    let document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::forced(FileFormat::Unix),
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    assert_eq!(messages(&run(&mut core, view, "<C-g>")), ["\"notes.txt\" 4 lines --25%--"]);
    run(&mut core, view, "jjx");
    assert_eq!(messages(&run(&mut core, view, "<C-g>")), ["\"notes.txt\" [Modified] 4 lines --75%--"]);

    let (mut core, view) = text_editor("");
    assert_eq!(messages(&run(&mut core, view, "<C-g>")), ["\"[No Name]\" --No lines in buffer--"]);
    let (mut core, view) = text_editor("only");
    assert_eq!(messages(&run(&mut core, view, "<C-g>")), ["\"[No Name]\" 1 line --100%--"]);
}

#[test]
fn zq_quits_without_writing_and_ctrl_l_changes_nothing() {
    let (mut core, view) = text_editor("one two");
    run(&mut core, view, "x");
    let output = run(&mut core, view, "ZQ");
    assert_eq!(file_requests(&output), [ExFileRequest::Quit { force: true }]);
    assert_eq!(core.document().text(), "ne two", "ZQ never writes or reverts");
    assert!(matches!(attempt(&mut core, view, "ZX"), CommandStatus::Unsupported(_)));

    let revision = core.document().revision();
    let before = cursor(&core, view);
    let output = run(&mut core, view, "<C-l>");
    assert!(output.ex_outcome.is_none());
    assert_eq!((core.document().revision(), cursor(&core, view)), (revision, before));
    run(&mut core, view, "v<C-l>");
    assert_eq!(mode(&core, view), Mode::VisualCharacter, "CTRL-L keeps Visual mode");
}

#[test]
fn gi_resumes_insert_where_it_stopped_and_g_capital_i_uses_column_one() {
    let (mut core, view) = text_editor("  one\n  two\n");
    run(&mut core, view, "A!<Esc>");
    assert_eq!(cursor(&core, view), 5, "Escape steps back from the line end");
    run(&mut core, view, "jgi?<Esc>");
    assert_eq!(core.document().text(), "  one!?\n  two\n", "gi resumes past the last character");
    run(&mut core, view, "u");
    assert_eq!(core.document().text(), "  one!\n  two\n", "gi typing is one undo unit");

    let (mut core, view) = text_editor("abc");
    run(&mut core, view, "gix<Esc>");
    assert_eq!(core.document().text(), "xabc", "without '^ gi inserts at the cursor");

    let (mut core, view) = text_editor("  one\n  two");
    run(&mut core, view, "$gI#<Esc>");
    assert_eq!(core.document().text(), "#  one\n  two", "gI ignores indentation");
    run(&mut core, view, "j$.");
    assert_eq!(core.document().text(), "#  one\n#  two", "dot repeats gI at column one");
    run(&mut core, view, "u");
    run(&mut core, view, "2gI-<Esc>");
    assert_eq!(core.document().text(), "#  one\n--  two", "gI counts repeat the insertion");

    let (mut core, view) = text_editor("ab");
    run(&mut core, view, "ax<C-o>l");
    run(&mut core, view, "y<Esc>0gi!<Esc>");
    assert_eq!(core.document().text(), "axy!b", "the last Insert exit wins");
}

#[test]
fn ampersand_repeats_the_last_substitution_without_its_flags() {
    let (mut core, view) = text_editor("a a\na a\na a\na a");
    run(&mut core, view, ":s/a/b/g<CR>");
    assert_eq!(core.document().text(), "b b\na a\na a\na a");
    run(&mut core, view, "j&");
    assert_eq!(core.document().text(), "b b\nb a\na a\na a", "& drops the g flag");
    run(&mut core, view, "j2&");
    assert_eq!(core.document().text(), "b b\nb a\nb a\nb a", "[count]& covers count lines");
    run(&mut core, view, "u");
    assert_eq!(core.document().text(), "b b\nb a\na a\na a", "one & is one undo unit");

    let (mut core, view) = text_editor("x x\nx x\ny");
    run(&mut core, view, ":s/x/z/g<CR>/y<CR>");
    run(&mut core, view, "u");
    run(&mut core, view, "/x<CR>g&");
    assert_eq!(core.document().text(), "z z\nz z\ny", "g& uses the last search, replacement and flags everywhere");

    let (mut core, view) = text_editor("abc");
    let revision = core.document().revision();
    let status = attempt(&mut core, view, "&");
    assert!(!matches!(status, CommandStatus::Complete), "{status:?}");
    assert_eq!(core.document().revision(), revision);
}

#[test]
fn go_jumps_to_source_bytes_and_works_as_an_operator_motion() {
    let (mut core, view) = text_editor("ab\ncd\nef");
    run(&mut core, view, "5go");
    assert_eq!(cursor(&core, view), 4);
    run(&mut core, view, "<C-o>");
    assert_eq!(cursor(&core, view), 0, "go is a jump");
    run(&mut core, view, "99go");
    assert_eq!(cursor(&core, view), 7, "past the end lands on the last character");
    run(&mut core, view, "go");
    assert_eq!(cursor(&core, view), 0);
    run(&mut core, view, "d4go");
    assert_eq!(core.document().text(), "cd\nef", "dgo is exclusive");
    run(&mut core, view, "vgoy");

    let (mut core, view) = editor("**bold** text", Format::Markdown);
    run(&mut core, view, "10go");
    assert_eq!(cursor(&core, view), 5, "source bytes map through hidden Markdown syntax");
}

#[test]
fn g_capital_m_moves_by_percentage_of_line_width() {
    let (mut core, view) = text_editor("0123456789\n\tx");
    run(&mut core, view, "gM");
    assert_eq!(cursor(&core, view), 5);
    run(&mut core, view, "20gM");
    assert_eq!(cursor(&core, view), 2);
    run(&mut core, view, "100gM");
    assert_eq!(cursor(&core, view), 9, "beyond the text lands on the last character");
    run(&mut core, view, "0dgM");
    assert_eq!(core.document().text(), "56789\n\tx", "dgM is exclusive");
    run(&mut core, view, "jgM");
    assert_eq!(cursor(&core, view), 6, "tabs count as display columns");
}

#[test]
fn g_question_rot13_operator_covers_motions_lines_visual_and_repeat() {
    let (mut core, view) = text_editor("Hello, World!\nabc xyz");
    run(&mut core, view, "g?w");
    assert_eq!(core.document().text(), "Uryyb, World!\nabc xyz");
    run(&mut core, view, "u");
    run(&mut core, view, "g??");
    assert_eq!(core.document().text(), "Uryyb, Jbeyq!\nabc xyz", "g?? is linewise, not a search");
    run(&mut core, view, "j.");
    assert_eq!(core.document().text(), "Uryyb, Jbeyq!\nnop klm");
    run(&mut core, view, "g?g?");
    assert_eq!(core.document().text(), "Uryyb, Jbeyq!\nabc xyz");
    run(&mut core, view, "0vlg?");
    assert_eq!(core.document().text(), "Uryyb, Jbeyq!\nnoc xyz");
    assert_eq!(mode(&core, view), Mode::Normal);
    run(&mut core, view, "u");
    run(&mut core, view, "0<C-v>kllg?");
    assert_eq!(core.document().text(), "Helyb, Jbeyq!\nnop xyz", "Visual Block g? rotates the block");
}

#[test]
fn unmatched_delimiter_motions_move_jump_and_drive_operators() {
    let (mut core, view) = editor("f(a, (b), g(c))", Format::Code);
    run(&mut core, view, "fc[(");
    assert_eq!(cursor(&core, view), 11);
    run(&mut core, view, "2[(");
    assert_eq!(cursor(&core, view), 1, "a count beyond the levels keeps the outermost");
    run(&mut core, view, "<C-o>");
    assert_eq!(cursor(&core, view), 11, "bracket motions are jumps");
    run(&mut core, view, "])");
    assert_eq!(cursor(&core, view), 13);
    run(&mut core, view, "0");
    assert!(matches!(attempt(&mut core, view, "[("), CommandStatus::SearchNotFound));
    assert_eq!(cursor(&core, view), 0);
    run(&mut core, view, "fcd])");
    assert_eq!(core.document().text(), "f(a, (b), g())", "d]) is exclusive");
    run(&mut core, view, "u0fbd])");
    assert_eq!(core.document().text(), "f(a, (), g(c))");
    run(&mut core, view, "u0fbvl])");
    assert_eq!((mode(&core, view), cursor(&core, view)), (Mode::VisualCharacter, 14));
    assert!(matches!(attempt(&mut core, view, "<Esc>[x"), CommandStatus::Unsupported(_)));

    let (mut core, view) = editor("{ a { b } c }", Format::Code);
    run(&mut core, view, "fc[{");
    assert_eq!(cursor(&core, view), 0);
    run(&mut core, view, "fb]}");
    assert_eq!(cursor(&core, view), 8);
    run(&mut core, view, "0fbc]}X<Esc>");
    assert_eq!(core.document().text(), "{ a { X} c }");
}

#[test]
fn method_motions_follow_vim_brace_structure() {
    let text = "class A {\n  void f() {\n    x;\n  }\n\n  void g() {\n    y;\n  }\n}\n";
    let (mut core, view) = editor(text, Format::Code);
    let f = text.find("f() {").unwrap() + 4;
    let g = text.find("g() {").unwrap() + 4;
    run(&mut core, view, "/y<CR>[m");
    assert_eq!(cursor(&core, view), g);
    run(&mut core, view, "/y<CR>2[m");
    assert_eq!(cursor(&core, view), f);
    run(&mut core, view, "]m");
    assert_eq!(cursor(&core, view), g);
    run(&mut core, view, "/x<CR>]M");
    assert_eq!(cursor(&core, view), text.find("  }").unwrap() + 2);
    run(&mut core, view, "gg]m");
    assert_eq!(cursor(&core, view), 8, "outside the class ]m reaches its start");
}

#[test]
fn section_motions_stop_at_braces_and_nroff_macros() {
    let text = "intro\n{\none\n}\n.SH X\n  last";
    let (mut core, view) = text_editor(text);
    run(&mut core, view, "]]");
    assert_eq!(cursor(&core, view), 6);
    run(&mut core, view, "]]");
    assert_eq!(cursor(&core, view), text.find(".SH").unwrap());
    run(&mut core, view, "]]");
    assert_eq!(cursor(&core, view), text.find("last").unwrap(), "Normal ]] at the end goes to the first nonblank");
    run(&mut core, view, "[[");
    assert_eq!(cursor(&core, view), text.find(".SH").unwrap());
    run(&mut core, view, "gg][");
    assert_eq!(cursor(&core, view), text.find('}').unwrap());
    run(&mut core, view, "[]");
    assert_eq!(cursor(&core, view), 0, "no earlier section end reaches the first line");
    run(&mut core, view, "gg");
    run(&mut core, view, "d]]");
    assert_eq!(core.document().text(), "{\none\n}\n.SH X\n  last", "d]] is linewise from column one");
    run(&mut core, view, "d]]");
    assert_eq!(core.document().text(), ".SH X\n  last", "the operator form also stops after a }} line");
    run(&mut core, view, "u");
    run(&mut core, view, "u");
    run(&mut core, view, "G0d[[");
    assert_eq!(core.document().text(), "intro\n{\none\n}\n  last");
}

fn visual_range(core: &TestCore, view: ViewId) -> (usize, usize) {
    let state = core.command_state(view).unwrap();
    (state.visual_anchor().expect("Visual selection"), state.cursor())
}

#[test]
fn gn_selects_the_match_under_or_after_the_cursor_and_repeats_operators() {
    let (mut core, view) = text_editor("foo bar foo baz foo");
    run(&mut core, view, "/foo<CR>gg");
    run(&mut core, view, "gn");
    assert_eq!(mode(&core, view), Mode::VisualCharacter);
    assert_eq!(visual_range(&core, view), (0, 2), "a match under the cursor is selected");
    run(&mut core, view, "gn");
    assert_eq!(visual_range(&core, view), (0, 10), "Visual gn extends to the next match");
    run(&mut core, view, "<Esc>0fbgN");
    assert_eq!(visual_range(&core, view), (2, 0), "gN selects backward from the match end");
    run(&mut core, view, "<Esc>$gN");
    assert_eq!(visual_range(&core, view), (18, 16));
    run(&mut core, view, "<Esc>gg2gn");
    assert_eq!(visual_range(&core, view), (8, 10), "a count skips matches");

    run(&mut core, view, "<Esc>ggdgn");
    assert_eq!(core.document().text(), " bar foo baz foo");
    run(&mut core, view, ".");
    assert_eq!(core.document().text(), " bar  baz foo", "dot repeats dgn on the next match");
    run(&mut core, view, "u");
    assert_eq!(core.document().text(), " bar foo baz foo");

    let (mut core, view) = text_editor("x foo foo foo");
    run(&mut core, view, "/foo<CR>0cgnbar<Esc>");
    assert_eq!(core.document().text(), "x bar foo foo");
    run(&mut core, view, "..");
    assert_eq!(core.document().text(), "x bar bar bar", "cgn and dot replace successive matches");
    assert!(matches!(attempt(&mut core, view, "."), CommandStatus::SearchNotFound | CommandStatus::Complete));
    assert_eq!(core.document().text(), "x bar bar bar");

    let (mut core, view) = text_editor("no search");
    let status = attempt(&mut core, view, "gn");
    assert!(matches!(status, CommandStatus::Error(_)), "{status:?}");
    assert_eq!(mode(&core, view), Mode::Normal);
    attempt(&mut core, view, "/zzz<CR>");
    assert_eq!(attempt(&mut core, view, "gn"), CommandStatus::SearchNotFound);
    assert_eq!(mode(&core, view), Mode::Normal);
}

#[test]
fn change_list_navigates_older_and_newer_changes() {
    let (mut core, view) = text_editor("one\ntwo\nthree\nfour");
    assert!(matches!(attempt(&mut core, view, "g;"), CommandStatus::Error(message) if message.contains("E664")));
    run(&mut core, view, "x3Gx");
    run(&mut core, view, "GAend<Esc>");
    assert_eq!(core.document().text(), "ne\ntwo\nhree\nfourend");
    run(&mut core, view, "gg");
    run(&mut core, view, "g;");
    assert_eq!(cursor(&core, view), "ne\ntwo\nhree\nfour".len(), "g; first returns to the newest change");
    run(&mut core, view, "g;");
    assert_eq!(cursor(&core, view), "ne\ntwo\n".len());
    run(&mut core, view, "g;");
    assert_eq!(cursor(&core, view), 0);
    assert!(matches!(attempt(&mut core, view, "g;"), CommandStatus::Error(message) if message.contains("E662")));
    run(&mut core, view, "2g,");
    assert_eq!(cursor(&core, view), "ne\ntwo\nhree\nfour".len());
    assert!(matches!(attempt(&mut core, view, "g,"), CommandStatus::Error(message) if message.contains("E663")));
    assert_eq!(messages(&run(&mut core, view, ":echo len(getchangelist()[0]) getchangelist()[1]<CR>")), ["3 2"]);

    // Entries follow later edits above them.
    run(&mut core, view, "ggOnew<Esc>");
    run(&mut core, view, "jg;g;");
    assert_eq!(cursor(&core, view), "new\nne\ntwo\nhree\nfour".len(), "positions are rebased");
}

fn left(core: &TestCore, view: ViewId) -> f32 {
    core.layout(view).unwrap().viewport_left()
}

#[test]
fn horizontal_z_commands_scroll_unwrapped_views_and_keep_the_cursor_visible() {
    let line = "abcdefghij".repeat(20);
    let (mut core, view) = text_editor(&format!("{line}\nshort"));
    run(&mut core, view, "zl");
    assert_eq!((left(&core, view), cursor(&core, view)), (0., 0), "wrapped views do not scroll horizontally");
    core.handle(view, CoreEvent::SetWrap(false)).unwrap();
    run(&mut core, view, "zl");
    let one = left(&core, view);
    assert!(one > 0., "zl scrolls the text left");
    assert!(cursor(&core, view) > 0, "a cursor scrolled off the left edge moves onto the screen");
    run(&mut core, view, "4zl");
    assert!(left(&core, view) > one);
    run(&mut core, view, "99zh");
    assert_eq!(left(&core, view), 0.);
    run(&mut core, view, "0zL");
    let half = left(&core, view);
    assert!(half > one * 4., "zL scrolls half a screen");
    run(&mut core, view, "zH");
    assert_eq!(left(&core, view), 0.);

    run(&mut core, view, "100|zs");
    assert_eq!(cursor(&core, view), 99, "zs keeps the cursor");
    let start = left(&core, view);
    assert!(start > half);
    run(&mut core, view, "ze");
    assert_eq!(cursor(&core, view), 99);
    assert!(left(&core, view) < start && left(&core, view) > 0., "ze puts the cursor at the right edge");
    let before = left(&core, view);
    run(&mut core, view, "jzl");
    assert_eq!(cursor(&core, view), line.len() + 1 + 4, "the cursor stays on the short line's last grapheme");
    assert!(left(&core, view) < before, "a short line limits the scroll to its last grapheme");
    assert_eq!(core.document().text(), format!("{line}\nshort"), "scrolling never edits");
}

#[test]
fn z_alignment_variants_move_to_the_first_nonblank() {
    let text = (1..=60).map(|line| format!("    line {line}")).collect::<Vec<_>>().join("\n");
    let (mut core, view) = text_editor(&text);
    run(&mut core, view, "30G$zt");
    let top = core.layout(view).unwrap().viewport_top();
    run(&mut core, view, "gg30G$z<CR>");
    assert_eq!(core.layout(view).unwrap().viewport_top(), top, "z<CR> aligns like zt");
    let line_start = text.find("    line 30").unwrap();
    assert_eq!(cursor(&core, view), line_start + 4);
    run(&mut core, view, "$z.");
    assert_eq!(cursor(&core, view), line_start + 4);
    run(&mut core, view, "$z-");
    assert_eq!(cursor(&core, view), line_start + 4);
    run(&mut core, view, "40z<CR>");
    assert_eq!(cursor(&core, view), text.find("    line 40").unwrap() + 4, "a count names the line");
}

#[test]
fn gm_moves_to_the_middle_of_the_screen_row() {
    let (mut core, view) = text_editor(&"word ".repeat(200));
    run(&mut core, view, "gm");
    let middle = cursor(&core, view);
    assert!(middle > 10, "{middle}");
    run(&mut core, view, "g$");
    let row_end = cursor(&core, view);
    assert!(middle < row_end, "gm stays on the first row ({middle} < {row_end})");
    run(&mut core, view, "0dgm");
    assert_eq!(core.document().text().len(), 1000 - middle, "dgm is exclusive");

    let (mut core, view) = text_editor("abc");
    run(&mut core, view, "gm");
    assert_eq!(cursor(&core, view), 2, "a short row ends at its last grapheme");
}

#[test]
fn k_runs_an_ex_keywordprg_with_the_keyword_and_rejects_external_programs() {
    let (mut core, view) = text_editor("look up%word here");
    let revision = core.document().revision();
    let status = attempt(&mut core, view, "K");
    assert!(matches!(&status, CommandStatus::Error(message) if message.contains("external program")), "{status:?}");
    assert_eq!(core.document().revision(), revision, "the default man program never runs");

    run(&mut core, view, ":set keywordprg=:echo<CR>");
    // `:echo look` evaluates the keyword as an expression, as Vim does.
    run(&mut core, view, ":let look = 'found'<CR>");
    assert_eq!(messages(&run(&mut core, view, "K")), ["found"]);
    run(&mut core, view, ":command! -nargs=* Show echo <q-args><CR>:set kp=:Show<CR>");
    assert_eq!(messages(&run(&mut core, view, "3K")), ["3 look"], "a count precedes the keyword");
    assert_eq!(messages(&run(&mut core, view, "fwK")), ["word"], "K takes the identifier under the cursor");
    assert_eq!(messages(&run(&mut core, view, "0wv3lK")), ["up\\%w"], "the keyword is escaped like fnameescape()");
    assert_eq!(messages(&run(&mut core, view, "0vllK")), ["loo"], "Visual K uses the selection");
    assert_eq!(mode(&core, view), Mode::Normal);
}

#[test]
fn ctrl_caret_requests_the_alternate_file() {
    let (mut core, view) = text_editor("text");
    let output = run(&mut core, view, "<C-^>");
    assert_eq!(file_requests(&output), [ExFileRequest::EditAlternate { force: false }]);
    let output = run(&mut core, view, "<C-6>");
    assert_eq!(file_requests(&output), [ExFileRequest::EditAlternate { force: false }]);
    let status = attempt(&mut core, view, "2<C-^>");
    assert!(matches!(&status, CommandStatus::Error(message) if message.starts_with("E86")), "{status:?}");
    assert_eq!(core.document().text(), "text");
}
