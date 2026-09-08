use viem_core::command::ex_execute::{ExFileRequest, ExFrontendRequest};
use viem_core::command::{
    CommandLineEditAction, CommandLineEditRequest, CommandLineSnapshot, InputEvent, Key,
};
use viem_core::document::{Document, HistoryStatus, Revision};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreError, CoreEvent, CoreOutcome, ViewId};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "viem-filename-completion-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) {
        fs::write(self.0.join(name), b"file contents").unwrap();
    }

    fn directory(&self, name: &str) {
        fs::create_dir_all(self.0.join(name)).unwrap();
    }

    fn path(&self, fragment: &str) -> String {
        format!("{}/{fragment}", self.0.to_str().unwrap())
    }

    fn command(&self, fragment: &str) -> String {
        format!("e {}", self.path(fragment))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Prompt {
    core: Core<MockTextMeasurementProvider>,
    view: ViewId,
    source: Vec<u8>,
    revision: Revision,
    history: HistoryStatus,
}

impl Prompt {
    fn new(input: &str) -> Self {
        // Retain an existing undo entry so completion must preserve history,
        // rather than merely leave a pristine document without an undo node.
        let mut document = Document::new("document");
        document.insert(0, "untouched ").unwrap();
        let source = document.source_bytes();
        let revision = document.revision();
        let history = document.history_status();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        let mut prompt = Self {
            core,
            view,
            source,
            revision,
            history,
        };
        prompt.key(Key::Char(':'));
        prompt.text(input);
        prompt
    }

    fn assert_document_unchanged(&self) {
        assert_eq!(self.core.document().source_bytes(), self.source);
        assert_eq!(self.core.document().revision(), self.revision);
        assert_eq!(self.core.document().history_status(), self.history);
    }

    fn event(&mut self, event: CoreEvent) -> Result<CoreOutcome, CoreError> {
        let result = self.core.handle(self.view, event);
        self.assert_document_unchanged();
        if let Ok(outcome) = &result {
            assert!(!outcome.document_changed);
        }
        result
    }

    fn key(&mut self, key: Key) -> CoreOutcome {
        self.event(CoreEvent::Input(InputEvent::Key(key))).unwrap()
    }

    fn text(&mut self, text: &str) {
        self.event(CoreEvent::Input(InputEvent::text(text)))
            .unwrap();
    }

    fn snapshot(&self) -> CommandLineSnapshot {
        self.core
            .command_state(self.view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
    }

    fn assert_text(&self, expected: &str) {
        let snapshot = self.snapshot();
        assert_eq!(snapshot.text, expected);
        assert!(snapshot.text.is_char_boundary(snapshot.active));
        assert!(snapshot.text.is_char_boundary(snapshot.anchor));
    }

    fn request(&self, action: CommandLineEditAction) -> CommandLineEditRequest {
        CommandLineEditRequest {
            document: self.core.document().id(),
            revision: self.revision,
            expected: self.snapshot(),
            action,
        }
    }

    fn edit(&mut self, action: CommandLineEditAction) {
        self.event(CoreEvent::EditCommandLine(self.request(action)))
            .unwrap();
    }
}

#[test]
fn forward_and_reverse_cycles_are_case_insensitive_sorted_and_wrap() {
    let fixture = Fixture::new();
    fixture.file("alpine.md");
    fixture.directory("ALTO");
    fixture.file("Alpha.txt");
    fixture.file("beta.txt");
    let original = fixture.command("aL");
    let mut prompt = Prompt::new(&original);
    for expected in ["Alpha.txt", "alpine.md", "ALTO/", "Alpha.txt"] {
        prompt.key(Key::Tab);
        prompt.assert_text(&fixture.command(expected));
    }
    for expected in ["ALTO/", "alpine.md", "Alpha.txt", "ALTO/"] {
        prompt.key(Key::BackTab);
        prompt.assert_text(&fixture.command(expected));
    }
    prompt.key(Key::Ctrl('e'));
    prompt.assert_text(&original);

    // A reverse cycle starts at the final match before any forward Tab.
    let mut reverse = Prompt::new(&original);
    for expected in ["ALTO/", "alpine.md", "Alpha.txt", "ALTO/"] {
        reverse.key(Key::BackTab);
        reverse.assert_text(&fixture.command(expected));
    }
}

#[test]
fn directory_slash_accepts_without_duplication_for_portable_and_native_input() {
    let fixture = Fixture::new();
    fixture.directory("Folder/Subfolder");
    fixture.file("Folder/Child.txt");
    for input_kind in 0..3 {
        let mut prompt = Prompt::new(&fixture.command("fo"));
        prompt.key(Key::Tab);
        let directory = prompt.snapshot();
        assert_eq!(directory.text, fixture.command("Folder/"));
        match input_kind {
            0 => prompt.text("/"),
            1 => {
                prompt.key(Key::Char('/'));
            }
            _ => prompt.edit(CommandLineEditAction::Replace {
                range: directory.active..directory.active,
                text: "/".into(),
            }),
        }
        assert_eq!(prompt.snapshot(), directory, "input kind {input_kind}");
        prompt.key(Key::Tab);
        prompt.assert_text(&fixture.command("Folder/Child.txt"));
        prompt.key(Key::Tab);
        prompt.assert_text(&fixture.command("Folder/Subfolder/"));
        prompt.key(Key::Ctrl('e'));
        assert_eq!(prompt.snapshot(), directory);
    }
}

#[test]
fn ordinary_movement_typing_and_backspace_accept_the_current_filename() {
    let fixture = Fixture::new();
    fixture.file("Alpha.txt");
    for event in [
        InputEvent::Key(Key::Left),
        InputEvent::Key(Key::Backspace),
        InputEvent::text(".bak"),
        InputEvent::Key(Key::Char('_')),
    ] {
        let mut prompt = Prompt::new(&fixture.command("al"));
        prompt.key(Key::Tab);
        prompt.event(CoreEvent::Input(event.clone())).unwrap();
        let accepted = match event {
            InputEvent::Key(Key::Left) => fixture.command("Alpha.txt"),
            InputEvent::Key(Key::Backspace) => fixture.command("Alpha.tx"),
            InputEvent::Text(_) => fixture.command("Alpha.txt.bak"),
            InputEvent::Key(Key::Char('_')) => fixture.command("Alpha.txt_"),
            _ => unreachable!(),
        };
        prompt.assert_text(&accepted);
        // Once accepted, Ctrl-E performs its ordinary end-of-line movement.
        prompt.key(Key::Ctrl('e'));
        prompt.assert_text(&accepted);
        assert_eq!(prompt.snapshot().active, accepted.len());
    }
}

#[test]
fn control_e_restores_exact_original_prefix_and_caret_while_control_y_accepts() {
    let fixture = Fixture::new();
    fixture.file("École Notes.txt");
    let original = fixture.command("é.keep");
    let mut prompt = Prompt::new(&original);
    let cursor = original.find(".keep").unwrap();
    prompt.edit(CommandLineEditAction::Select {
        anchor: cursor,
        active: cursor,
    });
    let before = prompt.snapshot();
    prompt.key(Key::Tab);
    prompt.assert_text(&fixture.command("École Notes.txt.keep"));
    assert_eq!(
        prompt.snapshot().active,
        fixture.command("École Notes.txt").len()
    );
    prompt.key(Key::Ctrl('e'));
    assert_eq!(prompt.snapshot(), before);

    prompt.key(Key::Tab);
    let suggestion = prompt.snapshot();
    prompt.key(Key::Ctrl('y'));
    assert_eq!(prompt.snapshot(), suggestion);
    prompt.key(Key::Ctrl('e'));
    prompt.assert_text(&suggestion.text);
    assert_eq!(prompt.snapshot().active, suggestion.text.len());
}

#[test]
fn successful_native_selection_and_replacement_accept_the_suggestion() {
    let fixture = Fixture::new();
    fixture.file("École Notes.txt");
    let original = fixture.command("éc");
    let completed = fixture.command("École Notes.txt");

    let mut prompt = Prompt::new(&original);
    prompt.key(Key::Tab);
    let start = completed.find("École").unwrap();
    prompt.edit(CommandLineEditAction::Select {
        anchor: start,
        active: start + "École".len(),
    });
    let selected = prompt.snapshot();
    prompt.key(Key::Tab);
    assert_eq!(prompt.snapshot(), selected, "selection prevents cycling");
    prompt.key(Key::Ctrl('e'));
    prompt.assert_text(&completed);
    assert_eq!(prompt.snapshot().active, completed.len());

    let mut prompt = Prompt::new(&original);
    prompt.key(Key::Tab);
    let end = prompt.snapshot().active;
    prompt.edit(CommandLineEditAction::Replace {
        range: end..end,
        text: ".bak".into(),
    });
    prompt.key(Key::Ctrl('e'));
    prompt.assert_text(&format!("{completed}.bak"));
}

#[test]
fn stale_and_invalid_native_edits_preserve_the_active_completion_cycle() {
    let fixture = Fixture::new();
    fixture.file("Éclair.txt");
    fixture.file("École Notes.txt");
    let original = fixture.command("é");
    let mut prompt = Prompt::new(&original);
    let stale = prompt.request(CommandLineEditAction::Replace {
        range: 0..0,
        text: "wrong".into(),
    });
    prompt.key(Key::Tab);
    prompt.assert_text(&fixture.command("Éclair.txt"));
    let suggested = prompt.snapshot();
    assert!(prompt.event(CoreEvent::EditCommandLine(stale)).is_err());
    assert_eq!(prompt.snapshot(), suggested);

    let interior = suggested.text.find('É').unwrap() + 1;
    for action in [
        CommandLineEditAction::Select {
            anchor: interior,
            active: interior,
        },
        CommandLineEditAction::Replace {
            range: interior..interior,
            text: "wrong".into(),
        },
    ] {
        let invalid = prompt.request(action);
        assert!(prompt.event(CoreEvent::EditCommandLine(invalid)).is_err());
        assert_eq!(prompt.snapshot(), suggested);
    }
    prompt.key(Key::Tab);
    prompt.assert_text(&fixture.command("École Notes.txt"));
    prompt.key(Key::Ctrl('e'));
    prompt.assert_text(&original);
}

#[test]
fn unmatched_missing_and_non_file_arguments_leave_the_prompt_unchanged() {
    let fixture = Fixture::new();
    fixture.file("File.txt");
    for input in [
        fixture.command("unmatched"),
        fixture.command("Missing/child"),
        format!("set {}", fixture.path("f")),
        format!("s {}", fixture.path("f")),
    ] {
        let mut prompt = Prompt::new(&input);
        let original = prompt.snapshot();
        prompt.key(Key::Tab);
        assert_eq!(prompt.snapshot(), original);
        prompt.key(Key::BackTab);
        assert_eq!(prompt.snapshot(), original);
    }
}

#[test]
fn enter_executes_the_selected_exact_filename_as_an_ex_frontend_request() {
    let fixture = Fixture::new();
    fixture.file("Read Me.txt");
    let mut prompt = Prompt::new(&fixture.command("rE"));
    prompt.key(Key::Tab);
    prompt.assert_text(&fixture.command("Read Me.txt"));
    let outcome = prompt.key(Key::Enter);
    assert_eq!(
        outcome
            .command
            .unwrap()
            .ex_outcome
            .unwrap()
            .frontend_requests,
        [ExFrontendRequest::File(ExFileRequest::Edit {
            path: Some(fixture.path("Read Me.txt")),
            force: false,
        })]
    );
    assert!(prompt
        .core
        .command_state(prompt.view)
        .unwrap()
        .command_line_snapshot()
        .is_none());
}
