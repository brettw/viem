use viem_core::command::ex_execute::{ExFileRequest, PreparedExFileRequest};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactStorageProvider,
    ArtifactWriteIntent, ArtifactWriteScope, Encoding, Format, InMemoryArtifactStorage,
    LineEndingOpenPolicy, PersistenceError,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreError, CoreEvent, Document};

fn loaded() -> (Document, InMemoryArtifactStorage) {
    let mut storage = InMemoryArtifactStorage::new();
    let path = ArtifactPath::from("file.txt");
    storage.insert(path.clone(), b"recovered".to_vec()).unwrap();
    let document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    (document, storage)
}
#[test]
fn readonly_is_buffer_policy_with_explicit_write_force_and_normal_edit_undo() {
    let (mut document, mut storage) = loaded();
    let revision = document.revision();
    let root = document.history_status().current;
    document.set_read_only(true);
    assert!(!document.is_dirty());
    assert_eq!(document.revision(), revision);
    assert_eq!(document.history_status().current, root);
    document.replace(0..9, "edited").unwrap();
    assert!(document.is_dirty());
    assert!(document.undo());
    assert!(!document.is_dirty());
    assert!(document.is_read_only());
    let intent = ArtifactWriteIntent::Save {
        overwrite: ArtifactOverwrite::ReplaceExisting,
    };
    assert_eq!(
        document.prepare_artifact_write(intent.clone()).unwrap_err(),
        PersistenceError::ReadOnly
    );
    assert_eq!(document.history_status().current, root);
    let write = document
        .prepare_artifact_write_with_force(intent, true)
        .unwrap();
    let completion = execute_prepared_artifact_write(&mut storage, &write);
    document
        .complete_artifact_write(write.succeeded(completion.unwrap()))
        .unwrap();
    assert!(document.is_read_only());
    assert!(!document.is_dirty());
}
#[test]
fn all_prepared_ex_write_forms_obey_readonly_before_creating_write_requests() {
    let (document, _) = loaded();
    let mut core: Core<MockTextMeasurementProvider> = Core::new(document);
    core.set_read_only(core.document().id(), core.document().revision(), true)
        .unwrap();
    for request in [
        ExFileRequest::Write {
            path: None,
            force: false,
            range: None,
        },
        ExFileRequest::Write {
            path: Some("other.txt".into()),
            force: false,
            range: None,
        },
        ExFileRequest::WriteQuit {
            path: None,
            force: false,
            range: None,
        },
        ExFileRequest::SaveAs {
            path: "other.txt".into(),
            force: false,
        },
        ExFileRequest::WriteAll { force: false },
    ] {
        assert_eq!(
            core.prepare_ex_file_request(&request).unwrap_err(),
            CoreError::Persistence(PersistenceError::ReadOnly)
        );
    }
    assert!(
        matches!(
            core.prepare_ex_file_request(&ExFileRequest::Xit {
                path: None,
                force: false
            })
            .unwrap(),
            PreparedExFileRequest::Host(_)
        ),
        "clean xit does not write"
    );
    core.mark_recovered(core.document().id(), core.document().revision())
        .unwrap();
    assert_eq!(
        core.prepare_ex_file_request(&ExFileRequest::Xit {
            path: None,
            force: false
        })
        .unwrap_err(),
        CoreError::Persistence(PersistenceError::ReadOnly)
    );
    assert!(matches!(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: true,
            range: None
        })
        .unwrap(),
        PreparedExFileRequest::ArtifactWrite(_)
    ));
}
#[test]
fn recovered_root_stays_dirty_through_undo_and_alternate_writes_until_successful_save() {
    let (mut document, mut storage) = loaded();
    let source = document.source_bytes();
    let revision = document.revision();
    let saved = document.history_status().current;
    document.mark_recovered();
    assert!(document.is_dirty());
    assert!(document.history_status().is_dirty);
    assert_eq!(document.history_status().current, saved);
    assert_eq!(document.revision(), revision);
    assert!(!document.history_status().can_undo);
    document.insert(0, "new ").unwrap();
    document.undo();
    assert_eq!(document.source_bytes(), source);
    assert!(document.is_dirty());
    let alternate = document
        .prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: ArtifactPath::from("alternate.txt"),
            scope: ArtifactWriteScope::WholeArtifact,
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    let completion = execute_prepared_artifact_write(&mut storage, &alternate);
    document
        .complete_artifact_write(alternate.succeeded(completion.unwrap()))
        .unwrap();
    assert!(document.is_recovered());
    assert!(document.is_dirty());
    let failed = document
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    document.complete_artifact_write(failed.failed()).unwrap();
    assert!(document.is_dirty());
    assert!(document.is_recovered());
    let save = document
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    let completion = execute_prepared_artifact_write(&mut storage, &save);
    document
        .complete_artifact_write(save.succeeded(completion.unwrap()))
        .unwrap();
    assert!(!document.is_recovered());
    assert!(!document.is_dirty());
    assert_eq!(document.source_bytes(), source);
}
#[test]
fn ex_readonly_gate_suppresses_host_write_effect_until_bang() {
    let (document, _) = loaded();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
    core.set_read_only(core.document().id(), core.document().revision(), true)
        .unwrap();
    for c in ":w".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(c))))
            .unwrap();
    }
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap();
    assert!(
        matches!(output.status,CommandStatus::ExError(viem_core::command::ExCommandError::Execute(ref error))if error.to_string().contains("E45")),
        "{:?}",
        output.status
    );
    for c in ":w!".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(c))))
            .unwrap();
    }
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
}
