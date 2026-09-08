//! Declarative conformance matrix for the Vim command surface.
//!
//! This is intentionally a black-box integration suite. Every case enters
//! through `Core`, so layout-dependent commands exercise the same mock-shaped
//! coordinator path that a frontend uses. A valid case may contain pending
//! prefix states, but it must finish and no event may be reported unsupported.

use evim_core::command::{CommandStatus, InputEvent, Key};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

const PROSE: &str =
    "alpha beta (gamma). Next sentence!\n\nparagraph two delta epsilon\nthird target alpha beta\nfourth line";

#[derive(Clone, Copy, Debug)]
struct CommandCase {
    family: &'static str,
    name: &'static str,
    initial: &'static str,
    setup: &'static str,
    command: &'static str,
    width: f32,
}

impl CommandCase {
    const fn new(
        family: &'static str,
        name: &'static str,
        initial: &'static str,
        setup: &'static str,
        command: &'static str,
    ) -> Self {
        Self {
            family,
            name,
            initial,
            setup,
            command,
            width: 500.0,
        }
    }

    const fn narrow(mut self) -> Self {
        self.width = 55.0;
        self
    }
}

fn parse_events(mut notation: &str) -> Vec<InputEvent> {
    const SPECIALS: &[(&str, Key)] = &[
        ("<Esc>", Key::Escape),
        ("<Enter>", Key::Enter),
        ("<Tab>", Key::Tab),
        ("<BS>", Key::Backspace),
        ("<Del>", Key::Delete),
        ("<Left>", Key::Left),
        ("<Right>", Key::Right),
        ("<Up>", Key::Up),
        ("<Down>", Key::Down),
        ("<Home>", Key::Home),
        ("<End>", Key::End),
        ("<PageUp>", Key::PageUp),
        ("<PageDown>", Key::PageDown),
        ("<C-[>", Key::Ctrl('[')),
        ("<C-V>", Key::Ctrl('v')),
        ("<C-R>", Key::Ctrl('r')),
        ("<C-O>", Key::Ctrl('o')),
        ("<C-I>", Key::Ctrl('i')),
        ("<C-F>", Key::Ctrl('f')),
        ("<C-B>", Key::Ctrl('b')),
        ("<C-D>", Key::Ctrl('d')),
        ("<C-U>", Key::Ctrl('u')),
        ("<C-E>", Key::Ctrl('e')),
        ("<C-Y>", Key::Ctrl('y')),
        ("<C-W>", Key::Ctrl('w')),
    ];

    let mut events = Vec::new();
    while !notation.is_empty() {
        if let Some(payload) = notation.strip_prefix("<Text:") {
            let end = payload
                .find('>')
                .unwrap_or_else(|| panic!("unterminated text token in {notation:?}"));
            events.push(InputEvent::Text(payload[..end].to_owned()));
            notation = &payload[end + 1..];
            continue;
        }
        if let Some((spelling, key)) = SPECIALS
            .iter()
            .find(|(spelling, _)| notation.starts_with(*spelling))
        {
            events.push(InputEvent::Key(*key));
            notation = &notation[spelling.len()..];
            continue;
        }
        let character = notation.chars().next().expect("notation is nonempty");
        events.push(InputEvent::Key(Key::Char(character)));
        notation = &notation[character.len_utf8()..];
    }
    events
}

fn new_core(text: &str, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 2_000.0);
    (core, view)
}

fn run_notation(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    notation: &str,
) -> Result<CommandStatus, String> {
    let mut final_status = None;
    for event in parse_events(notation) {
        let outcome = core
            .handle(view, CoreEvent::Input(event.clone()))
            .map_err(|error| format!("core error for {event:?}: {error:?}"))?;
        let status = outcome
            .command
            .ok_or_else(|| format!("input {event:?} returned no command output"))?
            .status;
        if let CommandStatus::Unsupported(reason) = &status {
            return Err(format!("{event:?} returned Unsupported({reason:?})"));
        }
        if matches!(
            status,
            CommandStatus::Error(_)
                | CommandStatus::CountError(_)
                | CommandStatus::RegisterReadError(_)
                | CommandStatus::RegisterWriteError(_)
                | CommandStatus::ExError(_)
                | CommandStatus::VisualBlockError(_)
                | CommandStatus::NeedsMoreLayout(_)
                | CommandStatus::SearchNotFound
        ) {
            return Err(format!("{event:?} returned {status:?}"));
        }
        final_status = Some(status);
    }
    final_status.ok_or_else(|| "case contains no input events".to_owned())
}

fn exercise(case: CommandCase) -> Option<String> {
    let (mut core, view) = new_core(case.initial, case.width);
    if !case.setup.is_empty() {
        if let Err(reason) = run_notation(&mut core, view, case.setup) {
            return Some(format!(
                "{} / {}: invalid setup {:?}: {reason}",
                case.family, case.name, case.setup
            ));
        }
    }
    match run_notation(&mut core, view, case.command) {
        Ok(CommandStatus::Complete | CommandStatus::Cancelled) => None,
        Ok(status) => Some(format!(
            "{} / {}: {:?} finished as {status:?}",
            case.family, case.name, case.command
        )),
        Err(reason) => Some(format!(
            "{} / {}: {:?}: {reason}",
            case.family, case.name, case.command
        )),
    }
}

fn assert_matrix(cases: impl IntoIterator<Item = CommandCase>) {
    let failures: Vec<_> = cases.into_iter().filter_map(exercise).collect();
    assert!(
        failures.is_empty(),
        "required Vim command conformance failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn normal_motion_and_search_matrix() {
    let cases = [
        CommandCase::new("basic", "h", PROSE, "l", "h"),
        CommandCase::new("basic", "l", PROSE, "", "l"),
        CommandCase::new("basic", "j", PROSE, "", "j"),
        CommandCase::new("basic", "k", PROSE, "j", "k"),
        CommandCase::new("basic", "Left", PROSE, "l", "<Left>"),
        CommandCase::new("basic", "Right", PROSE, "", "<Right>"),
        CommandCase::new("basic", "Down", PROSE, "", "<Down>"),
        CommandCase::new("basic", "Up", PROSE, "j", "<Up>"),
        CommandCase::new("basic", "Space", PROSE, "", " "),
        CommandCase::new("basic", "Backspace", PROSE, "l", "<BS>"),
        CommandCase::new("visual-row", "gj", PROSE, "", "gj").narrow(),
        CommandCase::new("visual-row", "gk", PROSE, "gj", "gk").narrow(),
        CommandCase::new("visual-row", "g0", PROSE, "gj", "g0").narrow(),
        CommandCase::new("visual-row", "g^", PROSE, "gj", "g^").narrow(),
        CommandCase::new("visual-row", "g$", PROSE, "", "2g$").narrow(),
        CommandCase::new("hard-line", "0", PROSE, "w", "0"),
        CommandCase::new("hard-line", "^", "   alpha\nnext", "w", "^"),
        CommandCase::new("hard-line", "$", PROSE, "", "$"),
        CommandCase::new("hard-line", "g_", PROSE, "", "g_"),
        CommandCase::new("hard-line", "|", PROSE, "", "5|"),
        CommandCase::new("hard-line", "+", PROSE, "", "+"),
        CommandCase::new("hard-line", "-", PROSE, "j", "-"),
        CommandCase::new("hard-line", "Enter", PROSE, "", "<Enter>"),
        CommandCase::new("word", "w", PROSE, "", "3w"),
        CommandCase::new("word", "W", PROSE, "", "W"),
        CommandCase::new("word", "e", PROSE, "", "e"),
        CommandCase::new("word", "E", PROSE, "", "E"),
        CommandCase::new("word", "b", PROSE, "ww", "b"),
        CommandCase::new("word", "B", PROSE, "WW", "B"),
        CommandCase::new("word", "ge", PROSE, "ww", "ge"),
        CommandCase::new("word", "gE", PROSE, "WW", "gE"),
        CommandCase::new("find", "f", "abacadaba", "", "fa"),
        CommandCase::new("find", "F", "abacadaba", "$", "Fa"),
        CommandCase::new("find", "t", "abacadaba", "", "ta"),
        CommandCase::new("find", "T", "abacadaba", "$", "Ta"),
        CommandCase::new("find", ";", "abacadaba", "fa", ";"),
        CommandCase::new("find", ",", "abacadaba", "fa;", ","),
        CommandCase::new("document", "gg", PROSE, "G", "gg"),
        CommandCase::new("document", "G", PROSE, "", "G"),
        CommandCase::new("document", "count-G", PROSE, "", "3G"),
        CommandCase::new("document", "count-percent", PROSE, "", "50%"),
        CommandCase::new("structure", "%", "before (inside) after", "f(", "%"),
        CommandCase::new("structure", "(", PROSE, ")", "("),
        CommandCase::new("structure", ")", PROSE, "", ")"),
        CommandCase::new("structure", "{", PROSE, "}", "{"),
        CommandCase::new("structure", "}", PROSE, "", "}"),
        CommandCase::new("viewport", "H", PROSE, "G", "H"),
        CommandCase::new("viewport", "M", PROSE, "", "M"),
        CommandCase::new("viewport", "L", PROSE, "", "L"),
        CommandCase::new("viewport", "Ctrl-F", PROSE, "", "<C-F>"),
        CommandCase::new("viewport", "Ctrl-B", PROSE, "G", "<C-B>"),
        CommandCase::new("viewport", "Ctrl-D", PROSE, "", "2<C-D>"),
        CommandCase::new("viewport", "Ctrl-U", PROSE, "G", "2<C-U>"),
        CommandCase::new("viewport", "Ctrl-E", PROSE, "", "<C-E>"),
        CommandCase::new("viewport", "Ctrl-Y", PROSE, "G", "<C-Y>"),
        CommandCase::new("viewport", "zz", PROSE, "j", "zz"),
        CommandCase::new("viewport", "zt", PROSE, "j", "zt"),
        CommandCase::new("viewport", "zb", PROSE, "j", "zb"),
        CommandCase::new("mark", "m", PROSE, "", "ma"),
        CommandCase::new("mark", "backtick", PROSE, "wmaG", "`a"),
        CommandCase::new("mark", "apostrophe", PROSE, "wmaG", "'a"),
        CommandCase::new("jump", "Ctrl-O", PROSE, "G", "<C-O>"),
        CommandCase::new("jump", "Ctrl-I", PROSE, "G<C-O>", "<C-I>"),
        CommandCase::new("search", "/", PROSE, "", "/target<Enter>"),
        CommandCase::new("search", "?", PROSE, "G$", "?alpha<Enter>"),
        CommandCase::new("search", "n", PROSE, "/alpha<Enter>", "n"),
        CommandCase::new("search", "N", PROSE, "/alpha<Enter>", "N"),
        CommandCase::new("search", "*", PROSE, "", "*"),
        CommandCase::new("search", "#", PROSE, "", "#"),
        CommandCase::new("search", "g*", PROSE, "", "g*"),
        CommandCase::new("search", "g#", PROSE, "", "g#"),
    ];
    assert_matrix(cases);
}

#[test]
fn operator_shorthand_and_text_object_matrix() {
    let mut cases = vec![
        CommandCase::new("operator", "d", PROSE, "", "dw"),
        CommandCase::new("operator", "c", PROSE, "", "cwQ<Esc>"),
        CommandCase::new("operator", "y", PROSE, "", "yw"),
        CommandCase::new("operator", ">", PROSE, "", ">j"),
        CommandCase::new("operator", "<", PROSE, ">j", "<j"),
        CommandCase::new("operator", "=", PROSE, "", "=j"),
        CommandCase::new("operator", "g~", PROSE, "", "g~w"),
        CommandCase::new("operator", "gu", "ALPHA beta", "", "guw"),
        CommandCase::new("operator", "gU", PROSE, "", "gUw"),
        CommandCase::new(
            "grammar",
            "multiplied counts",
            "one two three four five six seven",
            "",
            "2d3w",
        ),
        CommandCase::new("grammar", "register and count", PROSE, "", "\"a2dd"),
        CommandCase::new("shorthand", "dd", PROSE, "", "dd"),
        CommandCase::new("shorthand", "D", PROSE, "w", "D"),
        CommandCase::new("shorthand", "cc", PROSE, "", "ccQ<Esc>"),
        CommandCase::new("shorthand", "C", PROSE, "w", "CQ<Esc>"),
        CommandCase::new("shorthand", "yy", PROSE, "", "yy"),
        CommandCase::new("shorthand", "Y", PROSE, "", "Y"),
        CommandCase::new("shorthand", ">>", PROSE, "", ">>"),
        CommandCase::new("shorthand", "<<", PROSE, ">>", "<<"),
        CommandCase::new("shorthand", "==", PROSE, "", "=="),
        CommandCase::new("shorthand", "x", PROSE, "", "2x"),
        CommandCase::new("shorthand", "X", PROSE, "l", "X"),
        CommandCase::new("shorthand", "s", PROSE, "", "sQ<Esc>"),
        CommandCase::new("shorthand", "S", PROSE, "", "SQ<Esc>"),
        CommandCase::new("shorthand", "r", PROSE, "", "rQ"),
        CommandCase::new("shorthand", "R", PROSE, "", "RQ<Esc>"),
        CommandCase::new("shorthand", "~", PROSE, "", "~"),
        CommandCase::new("shorthand", "J", PROSE, "", "J"),
        CommandCase::new("shorthand", "gJ", PROSE, "", "gJ"),
        CommandCase::new("put", "p", "one two", "\"ayw$", "\"ap"),
        CommandCase::new("put", "P", "one two", "\"ayw$", "\"aP"),
        CommandCase::new("put", "gp", "one two", "\"ayw$", "\"agp"),
        CommandCase::new("put", "gP", "one two", "\"ayw$", "\"agP"),
        CommandCase::new("insert entry", "i", PROSE, "", "iQ<Esc>"),
        CommandCase::new("insert entry", "I", PROSE, "", "IQ<Esc>"),
        CommandCase::new("insert entry", "a", PROSE, "", "aQ<Esc>"),
        CommandCase::new("insert entry", "A", PROSE, "", "AQ<Esc>"),
        CommandCase::new("insert entry", "o", PROSE, "", "oQ<Esc>"),
        CommandCase::new("insert entry", "O", PROSE, "", "OQ<Esc>"),
        CommandCase::new("unicode", "grapheme x", "a\u{301}b", "", "x"),
        CommandCase::new("unicode", "grapheme r", "xy", "", "r<Text:a\u{301}>"),
    ];

    for (name, command) in [
        ("iw", "diw"),
        ("aw", "daw"),
        ("iW", "diW"),
        ("aW", "daW"),
        ("is", "dis"),
        ("as", "das"),
        ("ip", "dip"),
        ("ap", "dap"),
    ] {
        cases.push(CommandCase::new("text object", name, PROSE, "", command));
    }
    for (name, initial, command) in [
        ("i-double-quote", "\"quoted words\" tail", "di\""),
        ("a-double-quote", "\"quoted words\" tail", "da\""),
        ("i-single-quote", "'quoted words' tail", "di'"),
        ("a-single-quote", "'quoted words' tail", "da'"),
        ("i-backtick", "`quoted words` tail", "di`"),
        ("a-backtick", "`quoted words` tail", "da`"),
        ("i-paren", "(quoted words) tail", "di("),
        ("a-paren", "(quoted words) tail", "da("),
        ("ib", "(quoted words) tail", "dib"),
        ("ab", "(quoted words) tail", "dab"),
        ("i-bracket", "[quoted words] tail", "di["),
        ("a-bracket", "[quoted words] tail", "da["),
        ("i-brace", "{quoted words} tail", "di{"),
        ("a-brace", "{quoted words} tail", "da{"),
        ("iB", "{quoted words} tail", "diB"),
        ("aB", "{quoted words} tail", "daB"),
        ("i-angle", "<quoted words> tail", "di<"),
        ("a-angle", "<quoted words> tail", "da<"),
    ] {
        cases.push(CommandCase::new("text object", name, initial, "", command));
    }
    assert_matrix(cases);
}

#[test]
fn pending_prefix_cancellation_matrix() {
    assert_matrix([
        CommandCase::new("cancel", "count", PROSE, "", "2<Esc>"),
        CommandCase::new("cancel", "register", PROSE, "", "\"<Esc>"),
        CommandCase::new("cancel", "operator", PROSE, "", "d<Esc>"),
        CommandCase::new("cancel", "g-prefix", PROSE, "", "g<Esc>"),
        CommandCase::new("cancel", "z-prefix", PROSE, "", "z<Esc>"),
        CommandCase::new("cancel", "find", PROSE, "", "f<Esc>"),
    ]);
}

#[test]
fn insert_and_replace_input_matrix() {
    let mut cases = Vec::new();
    for (mode, entry) in [("insert", "i"), ("replace", "R")] {
        for (name, initial, setup, body) in [
            ("Unicode text", "abc", "", "<Text:é><Esc>"),
            ("Escape", "abc", "", "<Esc>"),
            ("Ctrl-[", "abc", "", "<C-[>"),
            ("Enter", "abc", "", "<Enter><Esc>"),
            ("Tab", "abc", "", "<Tab><Esc>"),
            ("Backspace", "abc", "l", "<BS><Esc>"),
            ("Forward Delete", "abc", "", "<Del><Esc>"),
            ("Left/Right", "abc", "", "<Right><Left><Esc>"),
            ("Home/End", "abc", "l", "<Home><End><Esc>"),
            ("Page Up/Down", PROSE, "", "<PageDown><PageUp><Esc>"),
            ("Ctrl-W", "one two", "A", "<C-W><Esc>"),
            ("Ctrl-U", "one two", "A", "<C-U><Esc>"),
            ("Ctrl-R register", "one two", "\"aywA", "<C-R>a<Esc>"),
            ("Ctrl-O Normal", "one two", "", "<C-O>w<Esc>"),
        ] {
            let setup = Box::leak(format!("{setup}{entry}").into_boxed_str());
            cases.push(CommandCase::new(mode, name, initial, setup, body).narrow());
        }
    }
    assert_matrix(cases);
}

#[test]
fn command_line_editing_matrix() {
    assert_matrix([
        CommandCase::new(
            "command line",
            "Left/Right",
            PROSE,
            "",
            ":set wrapp<Left><BS><Right><Enter>",
        ),
        CommandCase::new(
            "command line",
            "Home/End",
            PROSE,
            "",
            ":set wrap<Home><End><Enter>",
        ),
        CommandCase::new(
            "command line",
            "Delete",
            PROSE,
            "",
            ":set wrapp<Left><Del><Enter>",
        ),
        CommandCase::new(
            "command line",
            "history Up/Down",
            PROSE,
            ":set wrap<Enter>:set nowrap<Enter>",
            ":<Up><Up><Down><Enter>",
        ),
        CommandCase::new("command line", "Escape", PROSE, "", ":discarded<Esc>"),
    ]);
}

#[test]
fn ex_command_surface_matrix() {
    assert_matrix([
        CommandCase::new("Ex files", "edit", PROSE, "", ":edit sample.txt<Enter>"),
        CommandCase::new("Ex files", "enew", PROSE, "", ":enew<Enter>"),
        CommandCase::new("Ex files", "write", PROSE, "", ":write<Enter>"),
        CommandCase::new("Ex files", "saveas", PROSE, "", ":saveas sample.txt<Enter>"),
        CommandCase::new("Ex files", "quit", PROSE, "", ":quit<Enter>"),
        CommandCase::new("Ex files", "qall!", PROSE, "", ":qall!<Enter>"),
        CommandCase::new("Ex files", "wq", PROSE, "", ":wq<Enter>"),
        CommandCase::new("Ex files", "xit", PROSE, "", ":xit<Enter>"),
        CommandCase::new("Ex files", "wall!", PROSE, "", ":wall!<Enter>"),
        CommandCase::new("Ex editing", "undo", PROSE, "x", ":undo<Enter>"),
        CommandCase::new("Ex editing", "redo", PROSE, "xu", ":redo<Enter>"),
        CommandCase::new("Ex editing", "delete", PROSE, "", ":delete<Enter>"),
        CommandCase::new("Ex editing", "yank", PROSE, "", ":yank<Enter>"),
        CommandCase::new("Ex editing", "put", PROSE, "yy", ":put<Enter>"),
        CommandCase::new("Ex editing", "join", PROSE, "", ":join<Enter>"),
        CommandCase::new("Ex editing", "copy", PROSE, "", ":1copy 3<Enter>"),
        CommandCase::new("Ex editing", "move", PROSE, "", ":1move 3<Enter>"),
        CommandCase::new("Ex editing", "normal", PROSE, "", ":normal x<Enter>"),
        CommandCase::new(
            "Ex change",
            "substitute",
            PROSE,
            "",
            ":%substitute/alpha/omega/g<Enter>",
        ),
        CommandCase::new(
            "Ex change",
            "ampersand",
            PROSE,
            ":%substitute/alpha/omega/g<Enter>u",
            ":&<Enter>",
        ),
        CommandCase::new(
            "Ex change",
            "tilde",
            PROSE,
            ":%substitute/alpha/omega/g<Enter>u/alpha<Enter>",
            ":~<Enter>",
        ),
        CommandCase::new("Ex navigation", "line address", PROSE, "", ":3<Enter>"),
        CommandCase::new("Ex navigation", "goto", PROSE, "", ":goto 2<Enter>"),
        CommandCase::new("Ex info", "marks", PROSE, "ma", ":marks<Enter>"),
        CommandCase::new("Ex info", "registers", PROSE, "yy", ":registers<Enter>"),
        CommandCase::new("Ex info", "jumps", PROSE, "G", ":jumps<Enter>"),
        CommandCase::new("Ex options", "set", PROSE, "", ":set<Enter>"),
        CommandCase::new(
            "Ex options",
            "setlocal",
            PROSE,
            "",
            ":setlocal nowrap<Enter>",
        ),
        CommandCase::new("Ex options", "wrap", PROSE, "", ":set wrap<Enter>"),
        CommandCase::new("Ex options", "nowrap", PROSE, "", ":set nowrap<Enter>"),
        CommandCase::new("Ex options", "query", PROSE, "", ":set wrap?<Enter>"),
    ]);
}

#[test]
fn repeat_macro_and_undo_grouping_conformance() {
    let (mut core, view) = new_core("one two three four", 500.0);
    run_notation(&mut core, view, "dw.").expect("delete and dot repeat are supported");
    assert_eq!(core.document().text(), "three four");
    run_notation(&mut core, view, "u").expect("dot repeat is independently undoable");
    assert_eq!(
        core.document().text(),
        "two three four",
        "one dot repeat must be one undo unit"
    );
    run_notation(&mut core, view, "<C-R>").expect("redo is supported");
    assert_eq!(core.document().text(), "three four");

    let (mut core, view) = new_core("one two three", 500.0);
    run_notation(&mut core, view, "cw<Text:\u{00e9}><Esc>")
        .expect("change plus Unicode Insert payload is supported");
    assert_ne!(core.document().text(), "one two three");
    run_notation(&mut core, view, "u").expect("change undo is supported");
    assert_eq!(
        core.document().text(),
        "one two three",
        "operator deletion and following Insert session must share one undo unit"
    );

    let (mut core, view) = new_core("one two three four", 500.0);
    run_notation(&mut core, view, "qadwq@a").expect("macro record and replay are supported");
    assert_eq!(core.document().text(), "three four");
    run_notation(&mut core, view, "u").expect("macro replay is independently undoable");
    assert_eq!(
        core.document().text(),
        "two three four",
        "one macro replay must be one undo unit"
    );
    run_notation(&mut core, view, "@@").expect("repeat-last-macro is supported");

    let (mut core, view) = new_core("one two\nthree", 500.0);
    run_notation(&mut core, view, "\"a2dd").expect("named-register counted delete is supported");
    let register = core
        .command_state(view)
        .and_then(|state| state.register('a'))
        .expect("successful named-register delete populates that register");
    assert_eq!(register.text, "one two\nthree\n");

    let (mut core, view) = new_core(PROSE, 500.0);
    let before = core.document().source_bytes();
    run_notation(&mut core, view, "\"ad<Esc>").expect("pending command cancellation is supported");
    assert_eq!(
        core.document().source_bytes(),
        before,
        "cancelling a prefixed operator must not mutate the source"
    );
    assert!(
        core.command_state(view).unwrap().register('a').is_none(),
        "cancelling before the operator commits must not update its register"
    );
}

#[test]
fn visual_mode_by_motion_matrix() {
    let motions = [
        ("h", "l", "h"),
        ("l", "", "l"),
        ("j", "", "j"),
        ("k", "j", "k"),
        ("Left", "l", "<Left>"),
        ("Right", "", "<Right>"),
        ("Down", "", "<Down>"),
        ("Up", "j", "<Up>"),
        ("Space", "", " "),
        ("Backspace", "l", "<BS>"),
        ("gj", "", "gj"),
        ("gk", "gj", "gk"),
        ("g0", "w", "g0"),
        ("g^", "w", "g^"),
        ("g$", "", "g$"),
        ("0", "w", "0"),
        ("^", "w", "^"),
        ("$", "", "$"),
        ("g_", "", "g_"),
        ("|", "", "5|"),
        ("+", "", "+"),
        ("-", "j", "-"),
        ("Enter", "", "<Enter>"),
        ("w", "", "w"),
        ("W", "", "W"),
        ("e", "", "e"),
        ("E", "", "E"),
        ("b", "ww", "b"),
        ("B", "WW", "B"),
        ("ge", "ww", "ge"),
        ("gE", "WW", "gE"),
        ("f", "", "fa"),
        ("F", "$", "Fa"),
        ("t", "", "ta"),
        ("T", "$", "Ta"),
        (";", "fa", ";"),
        (",", "fa;", ","),
        ("gg", "G", "gg"),
        ("G", "", "G"),
        ("count-G", "", "3G"),
        ("count-percent", "", "50%"),
        ("percent", "f(", "%"),
        ("sentence-back", ")", "("),
        ("sentence-forward", "", ")"),
        ("paragraph-back", "}", "{"),
        ("paragraph-forward", "", "}"),
        ("H", "G", "H"),
        ("M", "", "M"),
        ("L", "", "L"),
        ("Ctrl-F", "", "<C-F>"),
        ("Ctrl-B", "G", "<C-B>"),
        ("Ctrl-D", "", "<C-D>"),
        ("Ctrl-U", "G", "<C-U>"),
        ("Ctrl-E", "", "<C-E>"),
        ("Ctrl-Y", "G", "<C-Y>"),
        ("zz", "j", "zz"),
        ("zt", "j", "zt"),
        ("zb", "j", "zb"),
        ("mark-exact", "wmaG", "`a"),
        ("mark-line", "wmaG", "'a"),
        ("jump-back", "G", "<C-O>"),
        ("jump-forward", "G<C-O>", "<C-I>"),
        ("search-forward", "", "/target<Enter>"),
        ("search-backward", "G$", "?alpha<Enter>"),
        ("n", "/alpha<Enter>", "n"),
        ("N", "/alpha<Enter>", "N"),
        ("star", "", "*"),
        ("hash", "", "#"),
        ("g-star", "", "g*"),
        ("g-hash", "", "g#"),
    ];
    let modes = [("character", "v"), ("line", "V"), ("block", "<C-V>")];
    let mut cases = Vec::new();
    for (mode, entry) in modes {
        for (name, setup, motion) in motions {
            let setup = Box::leak(format!("{setup}{entry}").into_boxed_str());
            cases.push(CommandCase::new(mode, name, PROSE, setup, motion).narrow());
        }
    }
    assert_matrix(cases);
}

#[test]
fn visual_mode_command_matrix() {
    let mut cases = vec![
        CommandCase::new("visual entry", "v", PROSE, "", "v"),
        CommandCase::new("visual entry", "V", PROSE, "", "V"),
        CommandCase::new("visual entry", "Ctrl-V", PROSE, "", "<C-V>"),
        CommandCase::new("visual entry", "gv", PROSE, "vly", "gv"),
    ];
    for (mode, entry) in [("character", "v"), ("line", "V"), ("block", "<C-V>")] {
        for (name, setup, command) in [
            ("v", "", "v"),
            ("V", "", "V"),
            ("Ctrl-V", "", "<C-V>"),
            ("o", "l", "o"),
            ("O", "l", "O"),
            ("Escape", "", "<Esc>"),
            ("x", "l", "x"),
            ("s", "l", "sQ<Esc>"),
            ("r", "l", "rQ"),
            ("J", "j", "J"),
            ("toggle", "l", "~"),
            ("lower", "l", "u"),
            ("upper", "l", "U"),
            ("indent", "j", ">"),
            ("outdent", "j", "<"),
            ("yank", "l", "y"),
            ("delete", "l", "d"),
            ("change", "l", "cQ<Esc>"),
            ("paste", "l\"ayw0", "\"ap"),
            ("paste-before", "l\"ayw0", "\"aP"),
            ("reindent", "j", "="),
        ] {
            let setup = Box::leak(format!("{setup}{entry}").into_boxed_str());
            cases.push(CommandCase::new(mode, name, PROSE, setup, command));
        }
    }
    assert_matrix(cases);
}

#[test]
fn visual_block_join_uses_touched_hard_lines_and_is_one_undo_unit() {
    let original = "abcdefghij\nx\nthird";
    let (mut core, view) = new_core(original, 24.0);

    run_notation(&mut core, view, "<C-V>GJ")
        .expect("Visual Block J is supported with wrapped and short rows");
    assert_eq!(core.document().text(), "abcdefghij x third");

    run_notation(&mut core, view, "u").expect("Visual Block J is undoable");
    assert_eq!(core.document().text(), original);
}

#[test]
fn visual_block_join_is_line_mode_independent_and_dot_replays_its_block_extent() {
    use evim_core::command::LineMode;
    let original = "abcdefghij\nx\nthird";
    for mode in [LineMode::Visual, LineMode::PhysicalSource] {
        for (join, expected) in [("J", "abcdefghij x third"), ("gJ", "abcdefghijxthird")] {
            let (mut core, view) = new_core(original, 24.0);
            core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
            run_notation(&mut core, view, &format!("<C-V>G{join}")).unwrap();
            assert_eq!(core.document().text(), expected, "{mode:?} {join}");
            run_notation(&mut core, view, "ugg0.").unwrap();
            assert_eq!(core.document().text(), expected, "dot: {mode:?} {join}");
            run_notation(&mut core, view, "u").unwrap();
            assert_eq!(core.document().text(), original);
        }
    }
}
