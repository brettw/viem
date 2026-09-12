//! `gq`/`gw`/`textwidth`: command grammar, cursor policy, Visual forms,
//! registers, dot/macro replay, undo grouping, options, and locality.
use viem_core::command::ex_execute::{
    ExFrontendRequest, ExInfoRequest, ExOptionName, ExOptionValue,
};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::syntax::detection::LanguageSelection;
use viem_core::document::{Document, Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandStatus {
    let mut status = CommandStatus::Complete;
    for ch in text.chars() {
        status = c.handle(d, InputEvent::Key(Key::Char(ch))).unwrap().status;
    }
    status
}

fn ex(
    c: &mut CommandInterpreter,
    d: &mut Document,
    command: &str,
) -> viem_core::command::CommandOutput {
    keys(c, d, command);
    c.handle(d, InputEvent::Key(Key::Enter)).unwrap()
}

fn escape(c: &mut CommandInterpreter, d: &mut Document) -> CommandStatus {
    c.handle(d, InputEvent::Key(Key::Escape)).unwrap().status
}

fn text(input: &str) -> (CommandInterpreter, Document) {
    (CommandInterpreter::new(), Document::new(input))
}

fn code(input: &str, language: &str) -> (CommandInterpreter, Document) {
    let mut c = CommandInterpreter::new();
    c.set_reflow_language(Some(language.to_owned()));
    (
        c,
        Document::from_bytes(input.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    )
}

const PROSE: &str = "aaa bbb ccc\nddd eee\nfff\n\nggg hhh iii jjj\nkkk\n";

#[test]
fn doubled_forms_and_counts_format_hard_lines() {
    for command in ["gqq", "gqgq", "gww", "gwgw"] {
        let (mut c, mut d) = text(PROSE);
        c.set_text_width_default(8);
        assert!(
            matches!(keys(&mut c, &mut d, command), CommandStatus::Complete),
            "{command}"
        );
        assert_eq!(
            d.text(),
            "aaa bbb\nccc\nddd eee\nfff\n\nggg hhh iii jjj\nkkk\n",
            "{command}"
        );
    }
    let (mut c, mut d) = text(PROSE);
    c.set_text_width_default(80);
    keys(&mut c, &mut d, "2gqq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "3gqgq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gq2q");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "2gq2gq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
}

#[test]
fn motions_text_objects_and_paragraph_forward_compose_with_the_operator() {
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gqj");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gq2j");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "wgq}");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "jgqip");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gqap");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "GgqG");
    assert_eq!(d.text(), PROSE);
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "4Ggqgg");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    // A characterwise motion ending at a line start excludes that line.
    let (mut c, mut d) = text("aaa\nbbb\nccc\n");
    keys(&mut c, &mut d, "gqw");
    assert_eq!(d.text(), "aaa\nbbb\nccc\n");
    let (mut c, mut d) = text("aaa\nbbb\nccc\n");
    keys(&mut c, &mut d, "gq2e");
    assert_eq!(d.text(), "aaa bbb\nccc\n");
    let (mut c, mut d) = text("aaa\nbbb\nccc\n");
    keys(&mut c, &mut d, "gq/bbb");
    c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
    assert_eq!(
        d.text(),
        "aaa\nbbb\nccc\n",
        "an exclusive end at a line start excludes it"
    );
    // `gq]` alone stays incomplete grammar; Escape cancels it.
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gq");
    let status = c
        .handle(&mut d, InputEvent::Key(Key::Char(']')))
        .unwrap()
        .status;
    assert!(!matches!(status, CommandStatus::Complete), "{status:?}");
    assert_eq!(d.text(), PROSE);
}

#[test]
fn gq_moves_to_the_last_formatted_line_and_gw_restores_its_location() {
    let (mut c, mut d) = text("  aaa bbb\n  ccc\n  ddd\n");
    keys(&mut c, &mut d, "wgqj");
    assert_eq!(d.text(), "  aaa bbb ccc\n  ddd\n");
    assert_eq!(c.cursor(), 2, "first nonblank of the last formatted line");
    let (mut c, mut d) = text("aaa bbb\nccc\nddd\n");
    keys(&mut c, &mut d, "gqj");
    assert_eq!(c.cursor(), 0);
    let (mut c, mut d) = text("aaa\nbbb\nccc\n");
    keys(&mut c, &mut d, "2jgqgg");
    assert_eq!(d.text(), "aaa bbb ccc\n");
    assert_eq!(c.cursor(), 0);
    // Unchanged text still moves gq to the last covered line.
    let (mut c, mut d) = text("aaa bbb\n\nccc\n");
    keys(&mut c, &mut d, "gq2j");
    assert_eq!(d.text(), "aaa bbb\n\nccc\n");
    assert_eq!(c.cursor(), 9);
    // gw keeps the caret on its word through the change map.
    let (mut c, mut d) = text("aaa bbb\nccc ddd\neee\n");
    keys(&mut c, &mut d, "jwgwip");
    assert_eq!(d.text(), "aaa bbb ccc ddd eee\n");
    assert_eq!(c.cursor(), 12, "`ddd` moved with the join");
    let (mut c, mut d) = text("aaa bbb ccc ddd\n");
    c.set_text_width_default(7);
    keys(&mut c, &mut d, "3wgww");
    assert_eq!(d.text(), "aaa bbb\nccc ddd\n");
    assert_eq!(c.cursor(), 12);
    // Deleted whitespace under gw's caret recovers to the following word.
    let (mut c, mut d) = text("aaa   \nbbb\n");
    keys(&mut c, &mut d, "4lgwj");
    assert_eq!(d.text(), "aaa bbb\n");
    assert_eq!(c.cursor(), 4);
}

#[test]
fn visual_forms_cover_hard_lines_and_leave_visual_mode() {
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "Vjgq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    assert_eq!(c.mode(), viem_core::command::Mode::Normal);
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "wvjgq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "wvjgw");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    assert_eq!(c.cursor(), 16, "the active Visual end (`eee`) is restored");
    assert_eq!(c.mode(), viem_core::command::Mode::Normal);
    // Visual Line in the physical-source line mode still expands to hard lines.
    let (mut c, mut d) = text(PROSE);
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource)
        .unwrap();
    keys(&mut c, &mut d, "Vjgq");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
    let (mut c, mut d) = text(PROSE);
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource)
        .unwrap();
    keys(&mut c, &mut d, "gqj");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee\nfff\n\nggg hhh iii jjj\nkkk\n"
    );
}

fn new_core(text: &str, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 2_000.0);
    (core, view)
}

fn run(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    events: &[InputEvent],
) -> CommandStatus {
    let mut status = CommandStatus::Complete;
    for event in events {
        let outcome = core.handle(view, CoreEvent::Input(event.clone())).unwrap();
        status = outcome
            .command
            .expect("input produces command output")
            .status;
    }
    status
}

fn chars(text: &str) -> Vec<InputEvent> {
    text.chars()
        .map(|ch| InputEvent::Key(Key::Char(ch)))
        .collect()
}

#[test]
fn visual_block_formats_each_intersected_hard_line_once() {
    let (mut core, view) = new_core("aaa bbb\nccc\nddd eee\nfff\n", 500.0);
    let mut events = vec![InputEvent::Key(Key::Ctrl('v'))];
    events.extend(chars("jgq"));
    assert!(matches!(
        run(&mut core, view, &events),
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "aaa bbb ccc\nddd eee\nfff\n");
    assert_eq!(
        core.command_state(view).unwrap().mode(),
        viem_core::command::Mode::Normal
    );
    // Dot repeat re-applies the block shape as hard lines.
    assert!(matches!(
        run(&mut core, view, &chars("j.")),
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "aaa bbb ccc\nddd eee fff\n");
    let (mut core, view) = new_core("aaa bbb\nccc\nddd\n", 500.0);
    let mut events = vec![InputEvent::Key(Key::Ctrl('v'))];
    events.extend(chars("jlgw"));
    run(&mut core, view, &events);
    assert_eq!(core.document().text(), "aaa bbb ccc\nddd\n");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        9,
        "the block caret moved with `ccc`"
    );
}

#[test]
fn escape_cancels_each_pending_stage_without_changes() {
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "gqq");
    let recipe_text = d.text().to_owned();
    for stage in ["g", "gq", "gqi", "gqf", "gq2", "gqg", "gw", "gwi"] {
        let (mut c2, mut d2) = text(PROSE);
        keys(&mut c2, &mut d2, stage);
        assert!(
            matches!(
                escape(&mut c2, &mut d2),
                CommandStatus::Cancelled | CommandStatus::Complete
            ),
            "{stage}"
        );
        assert_eq!(d2.text(), PROSE, "{stage}");
        assert!(!d2.undo(), "{stage}");
    }
    // A cancelled command does not replace the prior dot recipe.
    keys(&mut c, &mut d, "u");
    assert_eq!(d.text(), PROSE);
    keys(&mut c, &mut d, "gqi");
    escape(&mut c, &mut d);
    keys(&mut c, &mut d, ".");
    assert_eq!(d.text(), recipe_text);
}

#[test]
fn registers_are_untouched_and_named_registers_are_ignored() {
    let (mut c, mut d) = text(PROSE);
    keys(&mut c, &mut d, "\"ayyjyy");
    let unnamed = c.register('"').cloned();
    let named = c.register('a').cloned();
    keys(&mut c, &mut d, "\"bgqip");
    assert_eq!(
        d.text(),
        "aaa bbb ccc ddd eee fff\n\nggg hhh iii jjj\nkkk\n"
    );
    assert_eq!(c.register('"').cloned(), unnamed);
    assert_eq!(c.register('a').cloned(), named);
    assert!(c.register('b').is_none());
    assert!(c.register('1').is_none());
}

#[test]
fn dot_repeat_and_macros_replay_the_recorded_extent_and_effective_width() {
    let (mut c, mut d) = text("a b c d\ne f\n\ng h i j\nk l\n\nm n\no p\n");
    c.set_text_width_default(5);
    keys(&mut c, &mut d, "gqip");
    assert_eq!(d.text(), "a b c\nd e f\n\ng h i j\nk l\n\nm n\no p\n");
    keys(&mut c, &mut d, "}j.");
    assert_eq!(d.text(), "a b c\nd e f\n\ng h i\nj k l\n\nm n\no p\n");
    c.set_text_width_default(80);
    keys(&mut c, &mut d, "}j.");
    assert_eq!(d.text(), "a b c\nd e f\n\ng h i\nj k l\n\nm n o p\n");
    // Counted line forms repeat their count, and a dot count overrides it.
    let (mut c, mut d) = text("a\nb\nc\nd\ne\nf\n");
    keys(&mut c, &mut d, "2gqq");
    assert_eq!(d.text(), "a b\nc\nd\ne\nf\n");
    keys(&mut c, &mut d, "j.");
    assert_eq!(d.text(), "a b\nc d\ne\nf\n");
    keys(&mut c, &mut d, "j1.");
    assert_eq!(d.text(), "a b\nc d\ne\nf\n");
    keys(&mut c, &mut d, "2.");
    assert_eq!(d.text(), "a b\nc d\ne f\n");
    // Macro replay uses the same command path, including gw's cursor policy.
    let (mut c, mut d) = text("a\nb\nc\nd\ne\n");
    keys(&mut c, &mut d, "qagwjjq");
    assert_eq!(d.text(), "a b\nc\nd\ne\n");
    assert_eq!(c.cursor(), 4);
    keys(&mut c, &mut d, "@a");
    assert_eq!(d.text(), "a b\nc d\ne\n");
    assert_eq!(c.cursor(), 8);
    let (mut c, mut d) = text("a\nb\nc\nd\ne\nf\n");
    keys(&mut c, &mut d, "qagqjjq@a");
    assert_eq!(d.text(), "a b\nc d\ne\nf\n");
    keys(&mut c, &mut d, "@@");
    assert_eq!(d.text(), "a b\nc d\ne f\n");
}

#[test]
fn reflow_is_one_undo_unit_and_identical_results_leave_no_history() {
    let (mut c, mut d) = text("a\nb\nc\n\nx y\n");
    let original = d.source_bytes();
    let revision = d.revision();
    keys(&mut c, &mut d, "gqG");
    assert_eq!(d.text(), "a b c\n\nx y\n");
    let formatted = d.source_bytes();
    assert!(d.undo());
    assert_eq!(d.source_bytes(), original);
    assert!(!d.undo(), "one undo unit for several paragraphs");
    assert!(d.redo());
    assert_eq!(d.source_bytes(), formatted);
    assert!(!d.redo());
    keys(&mut c, &mut d, "u");
    assert_eq!(d.source_bytes(), original);
    assert_eq!(d.revision(), revision, "undo restores the recorded state");
    let (mut c, mut d) = text("already fits\n\nhere\n");
    let revision = d.revision();
    let history = d.history_status();
    assert!(!d.is_dirty());
    keys(&mut c, &mut d, "gqG");
    assert_eq!(d.revision(), revision);
    assert_eq!(
        d.history_status(),
        history,
        "a source-identical result adds no history node"
    );
    assert!(
        !d.is_dirty(),
        "a source-identical result does not dirty the buffer"
    );
    assert!(!d.undo());
}

#[test]
fn text_width_never_reformats_existing_text_or_hard_wraps_while_typing() {
    let (mut c, mut d) = text(
        "an existing line that is far longer than the width
second
",
    );
    let source = d.source_bytes();
    let revision = d.revision();
    ex(&mut c, &mut d, ":set tw=10");
    assert_eq!(
        d.source_bytes(),
        source,
        "changing textwidth reformats nothing"
    );
    assert_eq!(d.revision(), revision);
    // Typing, pasted text, and committed input-method text never hard wrap.
    keys(&mut c, &mut d, "A");
    c.handle(
        &mut d,
        InputEvent::Text(" typed words past the width".into()),
    )
    .unwrap();
    c.handle(&mut d, InputEvent::Key(Key::Escape)).unwrap();
    assert_eq!(
        d.text(),
        "an existing line that is far longer than the width typed words past the width
second
"
    );
    assert_eq!(d.hard_line_snapshot().line_count(), 3);
}

#[test]
fn unsupported_formats_return_a_non_destructive_error() {
    for format in [
        Format::MarkdownSource,
        Format::HtmlSource,
        Format::Markdown,
        Format::Html,
    ] {
        let mut d = Document::new("aaa\nbbb\n");
        d.set_format(format, viem_core::document::FormatOperation::Reinterpret)
            .unwrap();
        let revision = d.revision();
        let source = d.source_bytes();
        let mut c = CommandInterpreter::new();
        let status = keys(&mut c, &mut d, "gqj");
        assert!(
            matches!(status, CommandStatus::Error(_)),
            "{format:?}: {status:?}"
        );
        assert_eq!(d.revision(), revision, "{format:?}");
        assert_eq!(d.source_bytes(), source, "{format:?}");
    }
}

#[test]
fn code_uses_the_language_comment_profile_and_keeps_code_boundaries() {
    let (mut c, mut d) = code(
        "int x;\n// One paragraph split across\n// several short lines.\nint y; // trailing\n",
        "cpp",
    );
    c.set_text_width_default(40);
    keys(&mut c, &mut d, "jgqip");
    assert_eq!(
        d.text(),
        "int x;\n// One paragraph split across several\n// short lines.\nint y; // trailing\n"
    );
    let (mut c, mut d) = code("/* a\n * b\n */\n", "c");
    keys(&mut c, &mut d, "gqG");
    assert_eq!(d.text(), "/* a b\n */\n");
    // A language without a profile formats comments as plain text.
    let (mut c, mut d) = code("# a\n# b\n", "python");
    keys(&mut c, &mut d, "gqG");
    assert_eq!(d.text(), "# a # b\n");
}

#[test]
fn ex_textwidth_aliases_queries_overrides_and_inheritance() {
    let (mut c, mut d) = text("a\n");
    assert_eq!(c.text_width().effective(), 80);
    let output = ex(&mut c, &mut d, ":set tw?");
    let shown = output.ex_outcome.unwrap().frontend_requests;
    assert_eq!(
        shown,
        vec![ExFrontendRequest::Info(ExInfoRequest::Options(vec![
            viem_core::command::ex_execute::ExOptionDisplay {
                name: ExOptionName::TextWidth,
                value: ExOptionValue::Number(80),
            }
        ]))]
    );
    let output = ex(&mut c, &mut d, ":set textwidth=72");
    assert!(matches!(output.status, CommandStatus::Complete));
    let effects = output.ex_outcome.unwrap().option_effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].name, ExOptionName::TextWidth);
    assert_eq!(effects[0].old_value, ExOptionValue::OptionalNumber(None));
    assert_eq!(
        effects[0].new_value,
        ExOptionValue::OptionalNumber(Some(72))
    );
    assert_eq!(c.text_width().effective(), 72);
    assert_eq!(c.text_width().local_override(), Some(72));
    ex(&mut c, &mut d, ":setlocal tw=60");
    assert_eq!(c.text_width().effective(), 60);
    let output = ex(&mut c, &mut d, ":set textwidth?");
    assert!(matches!(
        output.ex_outcome.unwrap().frontend_requests.as_slice(),
        [ExFrontendRequest::Info(ExInfoRequest::Options(options))]
            if options[0].value == ExOptionValue::Number(60)
    ));
    // The application default is preserved behind the override.
    c.set_text_width_default(100);
    assert_eq!(c.text_width().effective(), 60);
    ex(&mut c, &mut d, ":setlocal tw<");
    assert_eq!(c.text_width().effective(), 100);
    assert_eq!(c.text_width().local_override(), None);
    ex(&mut c, &mut d, ":set tw=50");
    ex(&mut c, &mut d, ":setlocal textwidth<");
    assert_eq!(c.text_width().effective(), 100);
    ex(&mut c, &mut d, ":set tw=50");
    ex(&mut c, &mut d, ":set tw&");
    assert_eq!(c.text_width().effective(), 100);
    // Option changes never touch source, dirty state, or history.
    assert_eq!(d.text(), "a\n");
    assert!(!d.undo());
    // `:set` lists only an explicit override; `:set all` shows the effective value.
    ex(&mut c, &mut d, ":set tw=33");
    let output = ex(&mut c, &mut d, ":set");
    assert!(matches!(
        output.ex_outcome.unwrap().frontend_requests.as_slice(),
        [ExFrontendRequest::Info(ExInfoRequest::Options(options))]
            if options.iter().any(|option| option.name == ExOptionName::TextWidth
                && option.value == ExOptionValue::Number(33))
    ));
    ex(&mut c, &mut d, ":set tw<");
    let output = ex(&mut c, &mut d, ":set");
    assert!(matches!(
        output.ex_outcome.unwrap().frontend_requests.as_slice(),
        [ExFrontendRequest::Info(ExInfoRequest::Options(options))]
            if !options.iter().any(|option| option.name == ExOptionName::TextWidth)
    ));
    let output = ex(&mut c, &mut d, ":set all");
    assert!(matches!(
        output.ex_outcome.unwrap().frontend_requests.as_slice(),
        [ExFrontendRequest::Info(ExInfoRequest::Options(options))]
            if options.iter().any(|option| option.name == ExOptionName::TextWidth
                && option.value == ExOptionValue::Number(100))
    ));
}

#[test]
fn invalid_textwidth_values_are_rejected_without_changing_the_prior_value() {
    let (mut c, mut d) = text("a\n");
    ex(&mut c, &mut d, ":set tw=72");
    for command in [
        ":set tw=0",
        ":set tw=-1",
        ":set tw=1.5",
        ":set tw=abc",
        ":set tw=+5",
        ":set tw=",
        ":set tw=4294967296",
        ":set tw=99999999999999999999",
        ":set tw",
        ":set notw",
        ":set invtw",
        ":set tw!",
        ":set tw+=1",
        ":set tw-=1",
        ":set tw^=2",
        ":set tw=72 nows tw=0",
    ] {
        let output = ex(&mut c, &mut d, command);
        assert!(
            matches!(output.status, CommandStatus::ExError(_)),
            "{command}: {:?}",
            output.status
        );
        assert_eq!(c.text_width().effective(), 72, "{command}");
    }
    assert_eq!(c.text_width().effective(), 72);
    ex(&mut c, &mut d, ":set tw=4294967295");
    assert_eq!(c.text_width().effective(), u32::MAX);
}

#[test]
fn text_width_is_buffer_owned_and_shared_by_every_view() {
    let (mut core, first) = new_core("a b c d\ne f\n", 500.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    assert_eq!(core.text_width().effective(), 80);
    let mut events = chars(":set tw=5");
    events.push(InputEvent::Key(Key::Enter));
    run(&mut core, first, &events);
    assert_eq!(core.text_width().effective(), 5);
    assert_eq!(
        core.command_state(second).unwrap().text_width().effective(),
        5
    );
    run(&mut core, second, &chars("gqip"));
    assert_eq!(core.document().text(), "a b c\nd e f\n");
    // A new default reaches inheriting buffers only.
    core.set_text_width_default(3);
    assert_eq!(
        core.command_state(first).unwrap().text_width().effective(),
        5
    );
    let mut events = chars(":setlocal tw<");
    events.push(InputEvent::Key(Key::Enter));
    run(&mut core, second, &events);
    assert_eq!(
        core.command_state(first).unwrap().text_width().effective(),
        3
    );
    run(&mut core, first, &chars("gqip"));
    assert_eq!(core.document().text(), "a b\nc d\ne f\n");
    let third = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    assert_eq!(
        core.command_state(third).unwrap().text_width().effective(),
        3
    );
    // A fresh buffer inherits the application default rather than an override.
    let (mut other, view) = new_core("x\n", 500.0);
    other.set_text_width_default(40);
    assert_eq!(
        other
            .command_state(view)
            .unwrap()
            .text_width()
            .local_override(),
        None
    );
    assert_eq!(
        other.command_state(view).unwrap().text_width().effective(),
        40
    );
}

/// Opening a file of each C-family language must recognize its comment
/// leaders, rather than reflowing them as ordinary words and joining the
/// comment with the code that follows it.
#[test]
fn detected_c_family_files_reflow_their_comments_and_not_the_code_below() {
    for (filename, language, code_line) in [
        ("main.rs", "rust", "fn main() {}"),
        ("main.c", "c", "int main(void) {}"),
        ("main.cpp", "cpp", "int main() {}"),
        ("main.m", "objc", "int main(void) {}"),
        ("App.swift", "swift", "func main() {}"),
        ("Program.cs", "c_sharp", "class Program {}"),
        ("app.js", "javascript", "function main() {}"),
        ("app.ts", "typescript", "function main() {}"),
        ("app.tsx", "tsx", "function main() {}"),
        ("main.go", "go", "func main() {}"),
        ("Main.java", "java", "class Main {}"),
    ] {
        let source =
            format!("// One paragraph split across\n// several short lines.\n{code_line}\n");
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let mut core = Core::new(document);
        core.initialize_code_detection(filename, false).unwrap();
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
        assert_eq!(
            core.command_state(view).unwrap().reflow_language(),
            Some(language),
            "{filename}"
        );
        core.set_text_width_default(40);
        run(&mut core, view, &chars("gqip"));
        assert_eq!(
            core.document().text(),
            format!("// One paragraph split across several\n// short lines.\n{code_line}\n"),
            "{filename}"
        );
    }
}

#[test]
fn core_publishes_the_detected_language_to_every_view() {
    let document = Document::from_bytes(
        b"// One paragraph split across\n// several short lines.\n".to_vec(),
        Encoding::Utf8,
        Format::Code,
    )
    .unwrap();
    let mut core = Core::new(document);
    core.initialize_code_detection("main.cpp", false).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    assert_eq!(
        core.command_state(view).unwrap().reflow_language(),
        Some("cpp")
    );
    core.set_text_width_default(40);
    run(&mut core, view, &chars("gqip"));
    assert_eq!(
        core.document().text(),
        "// One paragraph split across several\n// short lines.\n"
    );
    core.set_code_language(LanguageSelection::Language("python".into()));
    assert_eq!(
        core.command_state(view).unwrap().reflow_language(),
        Some("python")
    );
    run(&mut core, view, &chars("gqip"));
    assert_eq!(
        core.document().text(),
        "// One paragraph split across several //\nshort lines.\n"
    );
}

#[test]
fn source_bytes_outside_the_span_are_untouched_and_encodings_survive() {
    let mut bytes = Vec::new();
    for index in 0..2_000 {
        bytes.extend_from_slice(format!("line {index} alpha beta\r\n").as_bytes());
    }
    let mut d = Document::from_bytes_with_file_format(
        bytes.clone(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "1000G3gqq");
    let after = d.source_bytes();
    let prefix: Vec<u8> = bytes.iter().copied().take_while(|_| true).collect();
    let first_changed = after
        .iter()
        .zip(prefix.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix_len = after
        .iter()
        .rev()
        .zip(prefix.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let changed_before = std::str::from_utf8(&bytes[..first_changed]).unwrap();
    assert_eq!(
        changed_before.matches("\r\n").count(),
        999,
        "changes start on line 1000"
    );
    let changed_after = std::str::from_utf8(&bytes[bytes.len() - suffix_len..]).unwrap();
    assert_eq!(
        changed_after.matches("\r\n").count(),
        999,
        "line 1001's terminator and every later line are untouched"
    );
    assert!(std::str::from_utf8(&after)
        .unwrap()
        .contains("line 999 alpha beta line 1000 alpha beta line 1001 alpha beta\r\n"));
    let utf16 = "éé ää\nöö\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut d = Document::from_bytes(utf16, Encoding::Utf16Le, Format::PlainText).unwrap();
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "gqG");
    assert_eq!(d.text(), "éé ää öö\n");
    assert_eq!(d.encoding(), Encoding::Utf16Le);
    assert_eq!(
        d.source_bytes(),
        "éé ää öö\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    );
}

#[test]
fn layout_caches_follow_reflow_and_undo_with_wrapping_on_and_off() {
    for wrap in [false, true] {
        let (mut core, view) = new_core("aaa bbb ccc\nddd\neee fff\n", 500.0);
        if wrap {
            let mut events = chars(":set wrap");
            events.push(InputEvent::Key(Key::Enter));
            run(&mut core, view, &events);
        }
        assert!(matches!(
            run(&mut core, view, &chars("gqip")),
            CommandStatus::Complete
        ));
        assert_eq!(core.document().text(), "aaa bbb ccc ddd eee fff\n");
        // A layout-dependent command materializes the current revision.
        assert!(matches!(
            run(&mut core, view, &chars("zz")),
            CommandStatus::Complete
        ));
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(
            snapshot.document_revision,
            core.document().revision(),
            "wrap={wrap}"
        );
        assert!(
            snapshot.rows.iter().any(|row| row.text_range.start == 0)
                && snapshot.rows.iter().all(|row| row.text_range.end <= 25),
            "wrap={wrap}: {:?}",
            snapshot
                .rows
                .iter()
                .map(|row| row.text_range.clone())
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            run(&mut core, view, &chars("u")),
            CommandStatus::Complete
        ));
        assert_eq!(core.document().text(), "aaa bbb ccc\nddd\neee fff\n");
        assert!(matches!(
            run(&mut core, view, &chars("zz")),
            CommandStatus::Complete
        ));
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(
            snapshot.document_revision,
            core.document().revision(),
            "wrap={wrap}"
        );
        assert!(
            snapshot.rows.iter().any(|row| row.text_range.start == 12),
            "wrap={wrap}: the restored `ddd` line has its own row"
        );
    }
}

#[test]
fn generated_text_never_passes_through_typing_assistance() {
    let mut core = Core::new(Document::new("say \"aaa bbb\"\n\"ccc\" ddd 'eee'\n"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
    run(&mut core, view, &chars("gqip"));
    assert_eq!(
        core.document().text(),
        "say \"aaa bbb\" \"ccc\" ddd 'eee'\n"
    );
    core.set_text_width_default(14);
    run(&mut core, view, &chars("gqip"));
    assert_eq!(
        core.document().text(),
        "say \"aaa bbb\"\n\"ccc\" ddd\n'eee'\n"
    );
}

#[test]
fn a_failed_reflow_keeps_the_prior_dot_recipe_and_registers() {
    let mut d = Document::new("aaa\nbbb\n\nccc\nddd\n\neee\nfff\n");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "yy");
    let yanked = c.register('"').cloned();
    keys(&mut c, &mut d, "gqip");
    assert_eq!(d.text(), "aaa bbb\n\nccc\nddd\n\neee\nfff\n");
    d.set_format(
        Format::MarkdownSource,
        viem_core::document::FormatOperation::Reinterpret,
    )
    .unwrap();
    let revision = d.revision();
    let status = keys(&mut c, &mut d, "gqip");
    assert!(matches!(status, CommandStatus::Error(_)), "{status:?}");
    assert_eq!(d.revision(), revision);
    assert_eq!(c.register('"').cloned(), yanked);
    d.set_format(
        Format::PlainText,
        viem_core::document::FormatOperation::Reinterpret,
    )
    .unwrap();
    keys(&mut c, &mut d, "3j.");
    assert_eq!(
        d.text(),
        "aaa bbb\n\nccc ddd\n\neee\nfff\n",
        "the earlier recipe replayed"
    );
    assert_eq!(c.register('"').cloned(), yanked);
}

#[test]
fn a_stale_snapshot_or_out_of_range_span_applies_nothing() {
    use viem_core::document::{reflow_edits, ReflowError, ReflowRequest};
    let mut d = Document::new("aaa bbb ccc\nddd\n");
    let stale = d.hard_line_snapshot();
    d.replace(0..3, "zzz").unwrap();
    assert!(
        d.validate_hard_line_snapshot(&stale).is_err(),
        "a stale snapshot is rejected"
    );
    let fresh = d.hard_line_snapshot();
    assert!(matches!(
        reflow_edits(&ReflowRequest {
            snapshot: &fresh,
            format: d.format(),
            lines: 0..9,
            text_width: 80,
            profile: None,
        }),
        Err(ReflowError::InvalidLineRange { .. })
    ));
    assert_eq!(d.text(), "zzz bbb ccc\nddd\n");
    assert!(d.undo());
    assert!(!d.undo(), "no reflow transaction was committed");
}

fn scrolled_code(lines: usize) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut source = String::new();
    for index in 0..lines {
        source.push_str(&format!("// comment line {index:05}\n"));
    }
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    core.initialize_code_detection("main.cpp", false).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 180.0, 80.0);
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(4_000.),
        },
    )
    .unwrap();
    (core, view)
}

/// Screen position of the visual row containing `offset`, relative to the
/// viewport top.
fn caret_screen_y(core: &Core<MockTextMeasurementProvider>, view: ViewId, offset: usize) -> f32 {
    let layout = core.layout(view).unwrap();
    let snapshot = layout
        .snapshot()
        .expect("a scrolled view has an exact local layout");
    let row = snapshot
        .rows
        .iter()
        .find(|row| row.text_range.contains(&offset) || row.text_range.start == offset)
        .expect("the caret row is materialized");
    row.y - layout.viewport_top()
}

#[test]
fn a_small_reflow_preserves_the_caret_baseline_and_never_lays_out_the_whole_document() {
    for command in ["2gqq", "2gww"] {
        let (mut core, view) = scrolled_code(20_000);
        core.set_text_width_default(60);
        // Put the caret on a visible row rather than the offscreen document start.
        run(&mut core, view, &chars("H"));
        let caret = core.command_state(view).unwrap().cursor();
        let baseline = caret_screen_y(&core, view, caret);
        let height = core.layout(view).unwrap().height();
        let covered = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .coverage
            .hard_lines()
            .len();
        assert!(
            covered < 500,
            "{command}: the scrolled layout starts partial: {covered}"
        );
        assert!(matches!(
            run(&mut core, view, &chars(command)),
            CommandStatus::Complete
        ));
        assert!(core.document().text()[caret..caret + 40].starts_with("// comment line"));
        // The edited row keeps its screen baseline: no recentering, and no
        // bottom alignment. The numeric origin may shift when a partial
        // layout's estimated heights above the anchor change.
        assert_eq!(caret_screen_y(&core, view, caret), baseline, "{command}");
        assert!(
            baseline >= 0. && baseline < height / 2.,
            "{command}: {baseline} of {height}"
        );
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(
            !snapshot.coverage.is_full_document(),
            "{command} laid out the whole document"
        );
        assert!(
            snapshot.coverage.hard_lines().len() < 500,
            "{command}: coverage grew to {}",
            snapshot.coverage.hard_lines().len()
        );
        assert!(snapshot.coverage.contains_text_offset(caret), "{command}");
    }
}

/// Deliberate differences from Vim 9.2, fixtured so a future comparison
/// harness against the pinned binary reports them as intended, not as bugs.
#[test]
fn deliberate_differences_from_vim_are_fixtured() {
    // 1. `textwidth=0` does not fall back to the window width; it is invalid
    //    and leaves the prior value in place.
    let (mut c, mut d) = text("a\n");
    ex(&mut c, &mut d, ":set tw=40");
    let output = ex(&mut c, &mut d, ":set tw=0");
    assert!(matches!(output.status, CommandStatus::ExError(_)));
    assert_eq!(c.text_width().effective(), 40);

    // 2. The global `:set textwidth` form sets a buffer override rather than a
    //    separate global value; the application default lives in Settings.
    c.set_text_width_default(70);
    assert_eq!(c.text_width().effective(), 40);
    assert_eq!(c.text_width().default_width(), 70);

    // 3. Joined words always get exactly one space; there is no `joinspaces`
    //    two-space sentence policy.
    let (mut c, mut d) = text("End of sentence.\nNext one.\n");
    keys(&mut c, &mut d, "gqip");
    assert_eq!(d.text(), "End of sentence. Next one.\n");

    // 4. List items are recognized without a `formatoptions` flag or a
    //    configurable `formatlistpat`, using the fixed initial marker set.
    let (mut c, mut d) = text("- alpha beta\n  gamma\n");
    c.set_text_width_default(9);
    keys(&mut c, &mut d, "gqip");
    assert_eq!(d.text(), "- alpha\n  beta\n  gamma\n");

    // 5. Comment leaders come from the language's declarative profile, not the
    //    `comments` option, so an unprofiled language reflows them as prose.
    let (mut c, mut d) = code("-- alpha\n-- beta\n", "haskell");
    keys(&mut c, &mut d, "gqip");
    assert_eq!(d.text(), "-- alpha -- beta\n");

    // 6. `gq`/`gw` count hard lines even where this view's other line commands
    //    count visual rows.
    let (mut c, mut d) = text("aaa\nbbb\nccc\n");
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource)
        .unwrap();
    keys(&mut c, &mut d, "2gqq");
    assert_eq!(d.text(), "aaa bbb\nccc\n");

    // 7. Formatting is internal portable policy: no `formatexpr`, `formatprg`,
    //    or other Vimscript hook is consulted or configurable.
    let (mut c, mut d) = text("a\n");
    for command in [":set formatexpr=Foo()", ":set formatprg=fmt", ":set fo=tcq"] {
        let output = ex(&mut c, &mut d, command);
        assert!(
            matches!(output.status, CommandStatus::ExError(_)),
            "{command}"
        );
    }
}
