use viem_core::command::ex::{parse_ex, AddressBase, ExAction, ExRange};
use viem_core::command::{CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key};
use viem_core::document::Document;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
    for ch in text.chars() {
        c.handle(d, InputEvent::Key(Key::Char(ch))).unwrap();
    }
}
fn ex(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandOutput {
    keys(c, d, text);
    c.handle(d, InputEvent::Key(Key::Enter)).unwrap()
}
fn success(output: CommandOutput) {
    assert_eq!(output.status, CommandStatus::Complete, "{output:?}");
}
fn core_keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for ch in text.chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
            .unwrap();
    }
}
fn core_ex(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    text: &str,
) -> CommandOutput {
    core_keys(core, view, text);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap()
}

#[test]
fn parses_mark_pattern_global_grammar_and_delimiter_escapes() {
    let command = parse_ex(r":'a+1,?foo\?bar?-2y a").unwrap();
    let Some(ExRange::Between { start, end, .. }) = command.range else {
        panic!("range")
    };
    assert_eq!(start.base, AddressBase::Mark('a'));
    assert_eq!(start.offset, 1);
    assert_eq!(
        end.base,
        AddressBase::Search {
            pattern: viem_core::command::search_regex::escape_literal("foo?bar"),
            forward: false
        }
    );
    assert_eq!(end.offset, -2);
    assert!(
        matches!(parse_ex(r":g#a\#b#normal A ").unwrap().action, ExAction::Global { command, .. } if command == "normal A ")
    );
    assert!(
        matches!(parse_ex(":v/foo/").unwrap().action, ExAction::Global { command, invert: true, .. } if command == "print")
    );
    for invalid in [
        ":g",
        ":globalx/foo/d",
        ":g|foo|d",
        ":'1d",
        ":''d",
        ":g\\foo\\d",
    ] {
        assert!(parse_ex(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn mark_addresses_and_offsets_resolve_at_execution_and_support_destinations() {
    let mut d = Document::new("zero\none\ntwo\nthree\nfour");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "jma2jmbgg");
    success(ex(&mut c, &mut d, ":'a,'by a"));
    assert_eq!(c.register('a').unwrap().text, "one\ntwo\nthree\n");
    success(ex(&mut c, &mut d, ":'a+1"));
    assert_eq!(c.cursor(), 9);
    success(ex(&mut c, &mut d, ":1copy 'b"));
    assert_eq!(d.text(), "zero\none\ntwo\nthree\nzero\nfour");
    let before = d.source_bytes();
    assert!(format!("{:?}", ex(&mut c, &mut d, ":'zd").status).contains("MarkNotSet"));
    assert_eq!(d.source_bytes(), before);
}

#[test]
fn pattern_addresses_distinguish_comma_semicolon_direction_wrap_and_empty_pattern() {
    let mut d = Document::new("end\nstart\nmiddle\nend\nlast");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, ":/start/;/end/y a"));
    assert_eq!(c.register('a').unwrap().text, "start\nmiddle\nend\n");
    success(ex(&mut c, &mut d, ":?start?"));
    assert_eq!(c.cursor(), 4);
    success(ex(&mut c, &mut d, "://+1"));
    assert_eq!(c.cursor(), 10);
    success(ex(&mut c, &mut d, ":set nowrapscan"));
    assert!(format!("{:?}", ex(&mut c, &mut d, ":/start/d").status).contains("PatternNotFound"));
    success(ex(&mut c, &mut d, ":1;/end/y b"));
    assert_eq!(c.register('b').unwrap().text, "end\nstart\nmiddle\nend\n");
    assert_eq!(d.text(), "end\nstart\nmiddle\nend\nlast");
}

#[test]
fn visual_prefill_history_and_explicit_marks_follow_the_latest_selection() {
    let mut d = Document::new("one\ntwo\nthree\nfour\nfive");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "Vj:");
    assert_eq!(c.command_line(), Some("'<,'>"));
    success(ex(&mut c, &mut d, "y a"));
    assert_eq!(c.register('a').unwrap().text, "one\ntwo\n");
    keys(&mut c, &mut d, "gg2jVj");
    c.handle(&mut d, InputEvent::Key(Key::Escape)).unwrap();
    keys(&mut c, &mut d, ":");
    c.handle(&mut d, InputEvent::Key(Key::Up)).unwrap();
    assert_eq!(c.command_line(), Some("'<,'>y a"));
    success(c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap());
    assert_eq!(c.register('a').unwrap().text, "three\nfour\n");
    keys(&mut c, &mut d, "gg'>");
    assert_eq!(c.cursor(), 14);
    success(ex(&mut c, &mut d, ":'<,'>y b"));
    assert_eq!(c.register('b').unwrap().text, "three\nfour\n");
    success(ex(&mut c, &mut d, ":delmarks <>"));
    assert!(format!("{:?}", ex(&mut c, &mut d, ":'<y").status).contains("MarkNotSet"));
}

#[test]
fn global_delete_inverse_registers_and_one_undo_unit() {
    for command in [":g/x/d a", ":v!/x/d a"] {
        let original = "x1\nkeep\nx2\nx3\ntail";
        let mut d = Document::new(original);
        let mut c = CommandInterpreter::new();
        success(ex(&mut c, &mut d, command));
        assert_eq!(d.text(), "keep\ntail");
        assert_eq!(c.register('a').unwrap().text, "x3\n");
        assert!(d.undo());
        assert_eq!(d.text(), original);
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.text(), "keep\ntail");
    }
    let mut d = Document::new("x1\nkeep\nx2\nx3\ntail");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, ":2,4g!/x/d _"));
    assert_eq!(d.text(), "x1\nx2\nx3\ntail");
}

#[test]
fn global_rebases_moves_deletions_insertions_and_multiline_starts() {
    for (command, expected) in [
        (":g/x/m$", "keep\ntail\nx1\nx2\nx3"),
        (":g/x/d 2", "tail"),
        (":g/x/copy .", "x1\nx1\nkeep\nx2\nx2\nx3\nx3\ntail"),
        (r":g/x2\nx3/d", "x1\nkeep\nx3\ntail"),
        (":g/x/normal A!", "x1!\nkeep\nx2!\nx3!\ntail"),
    ] {
        let mut d = Document::new("x1\nkeep\nx2\nx3\ntail");
        let mut c = CommandInterpreter::new();
        success(ex(&mut c, &mut d, command));
        assert_eq!(d.text(), expected, "{command}");
        assert!(d.undo());
        assert_eq!(d.text(), "x1\nkeep\nx2\nx3\ntail");
        assert!(!d.undo());
    }
}

#[test]
fn global_nested_filters_empty_patterns_print_and_per_line_errors() {
    let mut d = Document::new("xkeep\nxdelete\nother\nxagain");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, ":g/x/v/keep/d"));
    assert_eq!(d.text(), "xkeep\nother");
    assert!(d.undo());
    success(ex(&mut c, &mut d, ":g/x/s//Q/g"));
    assert_eq!(d.text(), "Qkeep\nQdelete\nother\nQagain");
    assert!(d.undo());
    let result = ex(&mut c, &mut d, ":g/x/s/delete/gone/");
    assert!(format!("{:?}", result.status).contains("PatternNotFound"));
    assert_eq!(d.text(), "xkeep\nxgone\nother\nxagain");
    assert!(d.undo());
    assert_eq!(d.text(), "xkeep\nxdelete\nother\nxagain");
    let output = ex(&mut c, &mut d, ":g/x/");
    success(output.clone());
    assert!(output.ex_outcome.is_some());
}

#[test]
fn invalid_global_regex_and_host_or_confirm_commands_do_not_edit() {
    let mut d = Document::new("x\nx\ny");
    let mut c = CommandInterpreter::new();
    for command in [
        ":g/[/d",
        ":g/x/write somewhere",
        ":g/x/undo",
        ":g/x/s/x/y/c",
        ":g/x/1g/y/d",
    ] {
        let before = d.source_bytes();
        assert_ne!(
            ex(&mut c, &mut d, command).status,
            CommandStatus::Complete,
            "{command}"
        );
        assert_eq!(d.source_bytes(), before, "{command}");
        assert!(!d.undo());
    }
}

#[test]
fn coordinator_global_uses_stable_lines_and_restores_other_views_and_undo() {
    for (command, expected) in [
        (":g/x/m$", "keep\ntail\nx1\nx2\nx3"),
        (":g/x/normal A!", "x1!\nkeep\nx2!\nx3!\ntail"),
        (":g/x/v/2/d", "keep\nx2\ntail"),
    ] {
        let original = "x1\nkeep\nx2\nx3\ntail";
        let mut core = Core::new(Document::new(original));
        let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
        let other = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
        core_keys(&mut core, other, "G");
        success(core_ex(&mut core, view, command));
        assert_eq!(core.document().text(), expected, "{command}");
        assert_eq!(
            core.document()
                .hard_line_at_offset(core.command_state(other).unwrap().cursor())
                .unwrap(),
            expected.lines().position(|line| line == "tail").unwrap()
        );
        success(core_ex(&mut core, view, ":u"));
        assert_eq!(core.document().text(), original);
        assert!(format!("{:?}", core_ex(&mut core, view, ":u").status).contains("NoUndo"));
    }
}

#[test]
fn coordinator_global_continues_errors_keeps_outer_history_and_reuses_marks() {
    let mut core = Core::new(Document::new("xkeep\nxdelete\nxagain"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    let result = core_ex(&mut core, view, ":g/x/s/delete/gone/");
    assert!(format!("{:?}", result.status).contains("PatternNotFound"));
    assert_eq!(core.document().text(), "xkeep\nxgone\nxagain");
    core_keys(&mut core, view, ":");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Up)))
        .unwrap();
    assert_eq!(
        core.command_state(view).unwrap().command_line(),
        Some("g/x/s/delete/gone/")
    );
}

#[test]
fn coordinator_visual_marks_are_shared_rebased_and_valid_after_visual_deletion() {
    let mut core = Core::new(Document::new("one\ntwo\nthree\nfour"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    let second = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    core_keys(&mut core, first, "jVj");
    core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core_keys(&mut core, second, "Oextra");
    core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    success(core_ex(&mut core, first, ":'<,'>y a"));
    assert_eq!(
        core.command_state(first)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        "two\nthree\n"
    );
    core_keys(&mut core, first, "ggVjd");
    success(core_ex(&mut core, first, ":'<,'>y b"));
    assert!(!core
        .command_state(first)
        .unwrap()
        .register('b')
        .unwrap()
        .text
        .is_empty());
}

#[test]
fn pattern_addresses_use_pending_search_and_preserve_literal_backward_delimiter() {
    let mut d = Document::new("foo?bar\nother\nfoo?bar\ntail");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, r":?foo\?bar?s//new/"));
    assert_eq!(d.text(), "foo?bar\nother\nnew\ntail");
    assert!(d.undo());
    keys(&mut c, &mut d, "G");
    assert!(
        format!("{:?}", ex(&mut c, &mut d, ":/other/,/foo/y").status).contains("InvertedRange")
    );
    success(ex(&mut c, &mut d, ":/other/;/foo/y a"));
    assert_eq!(c.register('a').unwrap().text, "other\nfoo?bar\n");
}

#[test]
fn global_restores_one_jump_origin_and_rejects_inherited_confirmation() {
    let mut core = Core::new(Document::new("origin\nx1\nx2\nx3"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    success(core_ex(&mut core, view, ":g/x/s/x/y/"));
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
        .unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    let mut d = Document::new("x\nx\nx");
    let mut c = CommandInterpreter::new();
    assert_eq!(
        ex(&mut c, &mut d, ":s/x/y/c").status,
        CommandStatus::Pending
    );
    c.handle(&mut d, InputEvent::Key(Key::Char('q'))).unwrap();
    let output = ex(&mut c, &mut d, ":g/x/&&");
    assert_ne!(output.status, CommandStatus::Complete);
    assert!(c.substitute_confirmation_prompt().is_none());
    assert_eq!(d.text(), "x\nx\nx");
    assert!(!d.undo());
}

#[test]
fn incremental_substitute_resolves_visual_and_pattern_addresses_without_changing_search_state() {
    let mut d = Document::new("xoutside\nxinside\nxinside\nxoutside");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, ":set incsearch"));
    keys(&mut c, &mut d, "jVj:s/x");
    let preview = c.search_presentation(&d);
    assert_eq!(preview.search_range, Some(9..24));
    assert_eq!(preview.incremental_match.unwrap().matched_range, 9..10);
    c.handle(&mut d, InputEvent::Key(Key::Escape)).unwrap();
    keys(&mut c, &mut d, "gg:/inside/;/outside/s//Q");
    let preview = c.search_presentation(&d);
    assert_eq!(preview.pattern.as_deref(), Some("outside"));
    assert_eq!(preview.incremental_match.unwrap().matched_range, 26..33);
    c.handle(&mut d, InputEvent::Key(Key::Escape)).unwrap();
    assert_eq!(c.search_presentation(&d).pattern, None);
    assert_eq!(d.text(), "xoutside\nxinside\nxinside\nxoutside");
}

#[test]
fn global_delimiter_escaping_respects_regex_extended_mode_and_backslash_pairs() {
    let mut d = Document::new("other\na#b\na\\\na");
    let mut c = CommandInterpreter::new();
    success(ex(&mut c, &mut d, r":g#(?x)^a\#b$#y a"));
    assert_eq!(c.register('a').unwrap().text, "a#b\n");
    success(ex(&mut c, &mut d, r":g/a\\/y a"));
    assert_eq!(c.register('a').unwrap().text, "a\\\n");
}
