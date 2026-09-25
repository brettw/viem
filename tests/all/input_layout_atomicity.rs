use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardTarget};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::layout::{
    MeasurementEnvironmentId, MeasurementError, MetricsGeneration, MockTextMeasurementProvider,
    RenderRunPolicy, ShapeRequest, ShapedFragment, TextMeasurementProvider,
};
use viem_core::{Core, CoreEvent, Document, ViewId};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct FailableProvider {
    inner: MockTextMeasurementProvider,
    fail: Arc<AtomicBool>,
}

impl FailableProvider {
    fn new(fail: Arc<AtomicBool>) -> Self {
        Self {
            inner: MockTextMeasurementProvider::new(),
            fail,
        }
    }
}

impl TextMeasurementProvider for FailableProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.inner.measurement_environment_id()
    }

    fn metrics_generation(&self) -> MetricsGeneration {
        self.inner.metrics_generation()
    }

    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.inner.render_run_policy()
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        if self.fail.load(Ordering::Acquire) {
            Err(MeasurementError::Provider(
                "requested extension failure".into(),
            ))
        } else {
            self.inner.shape_batch(requests)
        }
    }
}

fn event(key: char) -> CoreEvent {
    CoreEvent::Input(InputEvent::key(key))
}

fn keys<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, input: &str) {
    for key in input.chars() {
        let outcome = core.handle(view, event(key)).unwrap();
        assert!(matches!(
            outcome.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ));
    }
}

fn contents() -> String {
    (0..2_000)
        .map(|line| format!("line {line:04}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn counted_register_delete_retries_once_and_records_one_macro_command() {
    let source = contents();
    let removed = source.find("line 0198").unwrap();
    let mut core = Core::new(Document::new(source.clone()));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 32.);
    keys(&mut core, view, "qz\"a2d99g");
    let revision = core.document().revision();
    let history = core.document().history_status();
    let partial = core.handle(view, event('j')).unwrap();
    assert!(
        matches!(
            partial.command.as_ref().unwrap().status,
            CommandStatus::NeedsMoreLayout(_)
        ),
        "{partial:?}"
    );
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert!(core.command_state(view).unwrap().register('a').is_none());

    let result = core.handle_with_layout(view, event('j')).unwrap();
    assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
    assert!(result.document_changed);
    assert!(result.layout_changed);
    assert_eq!(core.document().source_bytes(), source[removed..].as_bytes());
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        source[..removed]
    );
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    keys(&mut core, view, "qu");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().history_status().can_undo);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source[removed..].as_bytes());
    keys(&mut core, view, "u@z");
    assert_eq!(core.document().source_bytes(), source[removed..].as_bytes());
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        0,
        "a failed attempt must not add another j to the recorded macro"
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn extension_failure_keeps_operator_state_and_another_views_open_undo_group() {
    let source = contents();
    let removed = source.find("line 0198").unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let mut core = Core::new(Document::new(source.clone()));
    let reader = core.add_view(FailableProvider::new(fail.clone()), 300., 32.);
    let writer = core.add_view(
        FailableProvider::new(Arc::new(AtomicBool::new(false))),
        300.,
        32.,
    );
    keys(&mut core, reader, "\"a2d99g");
    keys(&mut core, writer, "i");
    core.handle(writer, CoreEvent::Input(InputEvent::text("X")))
        .unwrap();
    let partial = core.handle(reader, event('j')).unwrap();
    assert!(
        matches!(
            partial.command.as_ref().unwrap().status,
            CommandStatus::NeedsMoreLayout(_)
        ),
        "{partial:?}"
    );
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    let history = core.document().history_status();
    let cursor = core.command_state(reader).unwrap().cursor();
    fail.store(true, Ordering::Release);
    let result = core.handle_with_layout(reader, event('j'));
    assert!(
        result.is_err()
            || result
                .as_ref()
                .is_ok_and(
                    |outcome| outcome.command.as_ref().is_some_and(|command| matches!(
                        command.status,
                        CommandStatus::NeedsMoreLayout(_) | CommandStatus::Error(_)
                    ))
                ),
        "a provider failure must remain visible: {result:?}"
    );
    assert_eq!(core.document().source_bytes(), before);
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.command_state(reader).unwrap().cursor(), cursor);
    assert!(core.command_state(reader).unwrap().register('a').is_none());
    fail.store(false, Ordering::Release);
    core.handle(writer, CoreEvent::Input(InputEvent::text("Y")))
        .unwrap();
    core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    keys(&mut core, writer, "u");
    assert_eq!(
        core.document().source_bytes(),
        source.as_bytes(),
        "the failed reader command cannot split the writer's XY undo group"
    );

    let retried = core.handle_with_layout(reader, event('j')).unwrap();
    assert_eq!(retried.command.unwrap().status, CommandStatus::Complete);
    assert_eq!(core.document().source_bytes(), source[removed..].as_bytes());
    assert_eq!(
        core.command_state(reader)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        source[..removed]
    );
}

#[test]
fn clipboard_delete_publishes_only_the_successful_retry_write() {
    let source = contents();
    let removed = source.find("line 0198").unwrap();
    let mut core = Core::new(Document::new(source.clone()));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 32.);
    keys(&mut core, view, "\"+2d99g");
    let event = CoreEvent::InputWithClipboard {
        input: InputEvent::key('j'),
        clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
    };
    let pending = core.handle(view, event.clone()).unwrap();
    let command = pending.command.unwrap();
    assert!(matches!(command.status, CommandStatus::NeedsMoreLayout(_)));
    assert!(command.clipboard_writes.is_empty());
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    let result = core.handle_with_layout(view, event).unwrap();
    let command = result.command.unwrap();
    assert_eq!(command.status, CommandStatus::Complete);
    assert_eq!(command.clipboard_writes.len(), 1);
    assert_eq!(
        command.clipboard_writes[0].target(),
        ClipboardTarget::Clipboard
    );
    assert_eq!(
        command.clipboard_writes[0].content().plain_text(),
        &source[..removed]
    );
    assert_eq!(core.document().source_bytes(), source[removed..].as_bytes());
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
