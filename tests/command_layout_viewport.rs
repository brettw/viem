use evim_core::command::{CommandStatus, InputEvent, Key, Mode};
use evim_core::document::{Document, Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn input(core: &mut TestCore, view: ViewId, keys: &str) {
    for key in keys.chars() {
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::key(key)))
            .unwrap_or_else(|error| panic!("{keys:?}, key {key:?}: {error:?}"));
        if let Some(command) = outcome.command {
            assert!(
                matches!(
                    command.status,
                    CommandStatus::Complete | CommandStatus::Pending
                ),
                "{keys:?}, key {key:?}: {:?}",
                command.status
            );
        }
    }
}

fn scrolled_source(mode: Mode) -> (TestCore, ViewId) {
    let mut source = String::from("\n\n\n");
    for line in 0..1_500 {
        source.push_str(&format!("  line {line:04}\n"));
    }
    let document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::HtmlSource).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 80.);
    match mode {
        Mode::Normal => {}
        Mode::VisualCharacter => input(&mut core, view, "v"),
        Mode::VisualLine => input(&mut core, view, "V"),
        _ => unreachable!(),
    }
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(8_000.),
        },
    )
    .unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(!snapshot.coverage.contains_text_offset(0));
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    (core, view)
}

fn visible_targets(core: &TestCore, view: ViewId) -> Vec<usize> {
    let layout = core.layout(view).unwrap();
    let top = layout.viewport_top();
    let bottom = top + layout.height();
    let targets: Vec<_> = layout
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .filter(|row| row.y < bottom && row.y + row.height() > top)
        .map(|row| {
            let text = &core.document().text()[row.text_range.clone()];
            row.text_range.start
                + text
                    .char_indices()
                    .find(|(_, character)| !character.is_whitespace())
                    .map_or(0, |(offset, _)| offset)
        })
        .collect();
    assert!(targets.len() >= 4, "fixture needs several visible rows");
    targets
}

fn target(targets: &[usize], motion: char, count: usize) -> usize {
    match motion {
        'H' => targets[(count - 1).min(targets.len() - 1)],
        'M' => targets[targets.len() / 2],
        'L' => targets[targets.len().saturating_sub(count)],
        _ => unreachable!(),
    }
}

#[test]
fn viewport_motions_keep_the_original_scrolled_target_with_an_offscreen_caret() {
    for mode in [Mode::Normal, Mode::VisualCharacter, Mode::VisualLine] {
        for (keys, motion, count) in [
            ("H", 'H', 1),
            ("2H", 'H', 2),
            ("M", 'M', 1),
            ("3M", 'M', 3),
            ("L", 'L', 1),
            ("2L", 'L', 2),
            ("\"a2H", 'H', 2),
            ("\"H2L", 'L', 2),
        ] {
            let (mut core, view) = scrolled_source(mode);
            let expected = target(&visible_targets(&core, view), motion, count);
            let source = core.document().source_bytes();
            let revision = core.document().revision();
            input(&mut core, view, keys);
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                expected,
                "{mode:?}: {keys} must target the viewport from before the command"
            );
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            assert_eq!(core.document().source_bytes(), source);
            assert_eq!(core.document().revision(), revision);
            assert!(
                core.layout(view)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .coverage
                    .hard_lines()
                    .len()
                    < 100
            );
        }
    }
}

#[test]
fn viewport_operator_motions_yank_through_the_original_scrolled_target() {
    for (keys, motion, count) in [
        ("\"ayH", 'H', 1),
        ("\"ayM", 'M', 1),
        ("\"ayL", 'L', 1),
        ("\"a2y2H", 'H', 4),
        ("\"a2y2M", 'M', 4),
        ("\"a2y2L", 'L', 4),
    ] {
        let (mut core, view) = scrolled_source(Mode::Normal);
        let expected = target(&visible_targets(&core, view), motion, count);
        let target_line = core
            .document()
            .hard_line_snapshot()
            .line_at_offset(expected)
            .unwrap();
        let expected_yank = core.document().text()[..target_line.linewise_range().end].to_owned();
        let source = core.document().source_bytes();
        let revision = core.document().revision();
        input(&mut core, view, keys);
        let state = core.command_state(view).unwrap();
        assert_eq!(state.register('a').unwrap().text, expected_yank, "{keys}");
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        assert!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .coverage
                .hard_lines()
                .len()
                < 100
        );
    }
}

#[test]
fn visual_text_object_prefixes_preserve_the_viewport_until_the_object_is_resolved() {
    for mode in [Mode::VisualCharacter, Mode::VisualLine] {
        for prefix in ["i", "a", "2i", "2a"] {
            let (mut core, view) = scrolled_source(mode);
            let original_top = core.layout(view).unwrap().viewport_top();
            let original_target = target(&visible_targets(&core, view), 'H', 1);
            let source = core.document().source_bytes();
            input(&mut core, view, prefix);
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                0,
                "{mode:?}: {prefix}"
            );
            assert_eq!(
                core.layout(view).unwrap().viewport_top(),
                original_top,
                "{mode:?}: {prefix}"
            );
            assert_eq!(core.document().source_bytes(), source);
            let cancelled = core
                .handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            assert!(matches!(
                cancelled.command.unwrap().status,
                CommandStatus::Cancelled
            ));
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            assert_eq!(core.layout(view).unwrap().viewport_top(), original_top);
            input(&mut core, view, "H");
            assert_eq!(core.command_state(view).unwrap().cursor(), original_target);
            assert_eq!(core.document().source_bytes(), source);
        }
    }
}

#[test]
fn invalid_visual_prefix_completions_preserve_the_viewport_and_following_motion_target() {
    for mode in [Mode::VisualCharacter, Mode::VisualLine] {
        for prefix in ["g", "z", "f", "T", "`", "'", "i", "a"] {
            let completions: &[Key] = if matches!(prefix, "f" | "T") {
                &[Key::Tab]
            } else {
                &[Key::Tab, Key::Char('!')]
            };
            for &completion in completions {
                let (mut core, view) = scrolled_source(mode);
                let original_top = core.layout(view).unwrap().viewport_top();
                let original_target = target(&visible_targets(&core, view), 'H', 1);
                let source = core.document().source_bytes();
                input(&mut core, view, prefix);
                let invalid = core
                    .handle(view, CoreEvent::Input(InputEvent::Key(completion)))
                    .unwrap();
                assert!(
                    matches!(
                        invalid.command.unwrap().status,
                        CommandStatus::Unsupported(_)
                    ),
                    "{mode:?}: {prefix}{completion:?}"
                );
                assert_eq!(
                    core.command_state(view).unwrap().cursor(),
                    0,
                    "{mode:?}: {prefix}{completion:?}"
                );
                assert_eq!(
                    core.layout(view).unwrap().viewport_top(),
                    original_top,
                    "{mode:?}: {prefix}{completion:?}"
                );
                assert_eq!(core.document().source_bytes(), source);
                // An unsupported completion can leave its grammar pending. Escape
                // explicitly clears that state before testing the next motion.
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                    .unwrap();
                assert_eq!(core.layout(view).unwrap().viewport_top(), original_top);
                input(&mut core, view, "H");
                assert_eq!(
                    core.command_state(view).unwrap().cursor(),
                    original_target,
                    "{mode:?}: {prefix}{completion:?}H"
                );
                assert_eq!(core.document().source_bytes(), source);
            }
        }
    }
}
