use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{Document, HistoryRetentionPolicy};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: InputEvent) {
    let outcome = core.handle(view, CoreEvent::Input(input)).unwrap();
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Complete | CommandStatus::Pending
    ));
}

fn core(policy: HistoryRetentionPolicy) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut document = Document::new("abc");
    document.set_history_retention_policy(policy);
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    (core, view)
}

#[test]
fn finishing_insert_succeeds_when_retention_promotes_the_edit_to_history_root() {
    for policy in [
        HistoryRetentionPolicy::new(1, usize::MAX),
        HistoryRetentionPolicy::new(usize::MAX, 1),
    ] {
        let (mut core, view) = core(policy);
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::Text("x".into()));
        let edited_revision = core.document().revision();
        let edited_node = core.document().history_status().current.node;
        assert!(core.document().history_status().can_undo);
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_eq!(core.document().text(), "xabc");
        assert_eq!(core.document().source_bytes(), b"xabc");
        assert_eq!(core.document().revision(), edited_revision);
        assert_eq!(core.document().history_status().current.node, edited_node);
        assert_eq!(core.document().history_status().node_count, 1);
        assert!(!core.document().history_status().can_undo);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        // A later edit must remain usable after the old undo edge was pruned.
        input(&mut core, view, InputEvent::key('x'));
        assert_eq!(core.document().source_bytes(), b"abc");
    }
}

#[test]
fn standalone_edit_succeeds_when_its_history_edge_is_immediately_pruned() {
    let (mut core, view) = core(HistoryRetentionPolicy::new(1, usize::MAX));
    input(&mut core, view, InputEvent::key('x'));
    assert_eq!(core.document().source_bytes(), b"bc");
    assert_eq!(core.document().history_status().node_count, 1);
    assert!(!core.document().history_status().can_undo);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
}

#[test]
fn retained_insert_edge_keeps_authentic_cursor_restoration() {
    let (mut core, view) = core(HistoryRetentionPolicy::new(2, usize::MAX));
    input(&mut core, view, InputEvent::key('l'));
    input(&mut core, view, InputEvent::key('a'));
    input(&mut core, view, InputEvent::Text("XY".into()));
    input(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().source_bytes(), b"abXYc");
    let edited_cursor = core.command_state(view).unwrap().cursor();
    assert_eq!(edited_cursor, 3);
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), b"abc");
    assert_eq!(core.command_state(view).unwrap().cursor(), 1);
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), b"abXYc");
    assert_eq!(core.command_state(view).unwrap().cursor(), edited_cursor);
}
