use evim_core::document::{
    execute_prepared_artifact_write, load_artifact, ArtifactOverwrite, ArtifactPath,
    ArtifactStorageProvider, ArtifactWriteCompletion, ArtifactWriteCompletionStatus,
    ArtifactWriteIntent, ArtifactWritePurpose, ArtifactWriteReceipt, ArtifactWriteScope,
    ArtifactWriteToken, Document, Encoding, FileFormat, Format, InMemoryArtifactStorage,
    InMemoryStorageError, LineEndingOpenPolicy, PersistenceError,
};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{command::InputEvent, command::Key, Core, CoreEvent};

fn save_as(path: &str) -> ArtifactWriteIntent {
    ArtifactWriteIntent::SaveAs {
        destination: ArtifactPath::from(path),
        overwrite: ArtifactOverwrite::ReplaceExisting,
    }
}

#[test]
fn prepared_writes_retain_exact_source_bytes_for_all_initial_encodings_and_markdown() {
    let cases = vec![
        (
            b"alpha\r\n\xce\xb2".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Dos,
        ),
        (
            vec![b'c', b'a', b'f', 0xe9, b'\r', b'x'],
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        ),
        (
            vec![0xff, 0xfe, b'a', 0, b'\r', 0, b'b', 0],
            Encoding::Utf16Le,
            Format::PlainText,
            FileFormat::Mac,
        ),
        (
            vec![0xfe, 0xff, 0, b'#', 0, b' ', 0, b'H', 0, b'\n'],
            Encoding::Utf16Be,
            Format::Markdown,
            FileFormat::Unix,
        ),
        (
            b"# title\r\n**bold** and _italic_\r\n".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Dos,
        ),
    ];

    for (index, (original, encoding, format, file_format)) in cases.into_iter().enumerate() {
        let mut document =
            Document::from_bytes_with_file_format(original.clone(), encoding, format, file_format)
                .unwrap();
        let destination = ArtifactPath::from_utf8(format!("case-{index}"));
        let prepared = document
            .prepare_artifact_write(ArtifactWriteIntent::SaveAs {
                destination: destination.clone(),
                overwrite: ArtifactOverwrite::RefuseExisting,
            })
            .unwrap();

        assert_eq!(prepared.document(), document.id());
        assert_eq!(prepared.revision(), document.revision());
        assert_eq!(prepared.purpose(), ArtifactWritePurpose::SaveAs);
        assert_eq!(prepared.storage_request().bytes(), original);

        let mut storage = InMemoryArtifactStorage::new();
        let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
        let status = document
            .complete_artifact_write(prepared.succeeded(receipt))
            .unwrap();
        assert!(matches!(
            status,
            ArtifactWriteCompletionStatus::Succeeded {
                file_identity_changed: true,
                document_is_dirty: false,
                ..
            }
        ));
        assert_eq!(storage.bytes(&destination), Some(original.as_slice()));
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn async_save_completion_marks_the_captured_node_not_the_newer_edit() {
    let mut document = Document::new("base");
    document.insert(4, " one").unwrap();
    let captured = document.history_status().current;
    let prepared = document
        .prepare_artifact_write(save_as("draft.txt"))
        .unwrap();
    let captured_revision = prepared.revision();

    document.insert(document.text().len(), " two").unwrap();
    let newest = document.history_status().current;
    assert_ne!(captured, newest);

    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    let status = document
        .complete_artifact_write(prepared.succeeded(receipt))
        .unwrap();

    assert!(matches!(
        status,
        ArtifactWriteCompletionStatus::Succeeded {
            written_revision,
            save_point: Some(save_point),
            document_is_dirty: true,
            ..
        } if written_revision == captured_revision && save_point == captured
    ));
    assert_eq!(document.history_status().save_point, captured.node);
    assert_eq!(document.history_status().current, newest);
    assert!(document.is_dirty());
    assert_eq!(
        storage.bytes(&ArtifactPath::from("draft.txt")),
        Some(b"base one".as_slice())
    );

    document.try_undo().unwrap();
    assert_eq!(document.history_status().current, captured);
    assert!(!document.is_dirty());
}

#[test]
fn async_completion_does_not_split_a_newer_open_edit_group() {
    let mut document = Document::new("");
    document.insert(0, "a").unwrap();
    let saved_node = document.history_status().current;
    let prepared = document
        .prepare_artifact_write(save_as("async-group"))
        .unwrap();

    document.begin_edit_group();
    document.insert(1, "b").unwrap();
    let grouped_node = document.history_status().current;
    let node_count = document.history_status().node_count;

    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    document
        .complete_artifact_write(prepared.succeeded(receipt))
        .unwrap();
    document.insert(2, "c").unwrap();
    document.end_edit_group();

    assert_eq!(document.history_status().current, grouped_node);
    assert_eq!(document.history_status().node_count, node_count);
    assert_eq!(document.history_status().save_point, saved_node.node);
    document.try_undo().unwrap();
    assert_eq!(document.text(), "a");
    assert!(!document.is_dirty());
}

#[test]
fn a_failed_provider_write_is_recorded_without_clearing_dirty_or_rebinding() {
    let path = ArtifactPath::from("current.txt");
    let mut storage = InMemoryArtifactStorage::new();
    let original_binding = storage.insert(path.clone(), b"old".to_vec()).unwrap();
    let loaded = load_artifact(&mut storage, &path).unwrap();
    let mut document = Document::from_loaded_artifact(
        loaded,
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(0..3, "new").unwrap();
    assert!(document.is_dirty());

    let prepared = document
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    storage.fail_next_write();
    assert_eq!(
        execute_prepared_artifact_write(&mut storage, &prepared),
        Err(InMemoryStorageError::InjectedWriteFailure)
    );
    let status = document.complete_artifact_write(prepared.failed()).unwrap();
    assert!(matches!(
        status,
        ArtifactWriteCompletionStatus::Failed {
            document_is_dirty: true,
            ..
        }
    ));
    assert_eq!(document.artifact_binding(), Some(&original_binding));
    assert_eq!(storage.bytes(&path), Some(b"old".as_slice()));
    assert!(document.is_dirty());
}

#[test]
fn only_successful_save_as_changes_file_identity_and_alternate_ranges_do_not_save() {
    let current = ArtifactPath::from("current.txt");
    let alternate = ArtifactPath::from("selection.txt");
    let renamed = ArtifactPath::from("renamed.txt");
    let mut storage = InMemoryArtifactStorage::new();
    let initial_binding = storage.insert(current.clone(), b"abcdef".to_vec()).unwrap();
    let loaded = storage.read_artifact(&current).unwrap();
    let mut document = Document::from_loaded_artifact(
        loaded,
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(5..6, "Z").unwrap();

    let alternate_write = document
        .prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: alternate.clone(),
            scope: ArtifactWriteScope::PrimarySourceBytes(1..4),
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    let receipt = execute_prepared_artifact_write(&mut storage, &alternate_write).unwrap();
    let status = document
        .complete_artifact_write(alternate_write.succeeded(receipt))
        .unwrap();
    assert!(matches!(
        status,
        ArtifactWriteCompletionStatus::Succeeded {
            save_point: None,
            file_identity_changed: false,
            document_is_dirty: true,
            ..
        }
    ));
    assert_eq!(storage.bytes(&alternate), Some(b"bcd".as_slice()));
    assert_eq!(document.artifact_binding(), Some(&initial_binding));

    let failed_save_as = document
        .prepare_artifact_write(save_as("failed.txt"))
        .unwrap();
    storage.fail_next_write();
    assert_eq!(
        execute_prepared_artifact_write(&mut storage, &failed_save_as),
        Err(InMemoryStorageError::InjectedWriteFailure)
    );
    document
        .complete_artifact_write(failed_save_as.failed())
        .unwrap();
    assert_eq!(document.artifact_binding(), Some(&initial_binding));

    let save_as = document
        .prepare_artifact_write(ArtifactWriteIntent::SaveAs {
            destination: renamed.clone(),
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    let receipt = execute_prepared_artifact_write(&mut storage, &save_as).unwrap();
    let renamed_identity = receipt.identity().clone();
    let status = document
        .complete_artifact_write(save_as.succeeded(receipt))
        .unwrap();
    assert!(matches!(
        status,
        ArtifactWriteCompletionStatus::Succeeded {
            file_identity_changed: true,
            document_is_dirty: false,
            ..
        }
    ));
    assert_eq!(document.artifact_binding().unwrap().path(), &renamed);
    assert_eq!(
        document.artifact_binding().unwrap().identity(),
        &renamed_identity
    );
}

#[test]
fn completion_tokens_reject_wrong_unknown_duplicate_and_superseded_results() {
    let mut first = Document::new("one");
    let other = Document::new("other");
    let early = first.prepare_artifact_write(save_as("early")).unwrap();

    let wrong_receipt = ArtifactWriteReceipt::new(
        ArtifactPath::from("early"),
        evim_core::document::ArtifactIdentity::new(vec![7]),
    );
    assert!(matches!(
        first.complete_artifact_write(ArtifactWriteCompletion::succeeded(
            other.id(),
            early.token(),
            wrong_receipt,
        )),
        Err(PersistenceError::WrongDocument { .. })
    ));
    assert_eq!(
        first.complete_artifact_write(ArtifactWriteCompletion::failed(
            first.id(),
            ArtifactWriteToken::from_u64(u64::MAX),
        )),
        Err(PersistenceError::UnknownCompletion(
            ArtifactWriteToken::from_u64(u64::MAX)
        ))
    );

    let late = first.prepare_artifact_write(save_as("late")).unwrap();
    let mut storage = InMemoryArtifactStorage::new();
    let late_receipt = execute_prepared_artifact_write(&mut storage, &late).unwrap();
    let late_completion = late.succeeded(late_receipt);
    first
        .complete_artifact_write(late_completion.clone())
        .unwrap();
    assert_eq!(
        first.complete_artifact_write(late_completion),
        Err(PersistenceError::DuplicateCompletion(late.token()))
    );

    let early_receipt = execute_prepared_artifact_write(&mut storage, &early).unwrap();
    assert_eq!(
        first.complete_artifact_write(early.succeeded(early_receipt)),
        Err(PersistenceError::StaleCompletion(early.token()))
    );
    assert_eq!(
        first.artifact_binding().unwrap().path(),
        &ArtifactPath::from("late")
    );
    assert_eq!(
        first.complete_artifact_write(early.failed()),
        Err(PersistenceError::DuplicateCompletion(early.token()))
    );
}

#[test]
fn fake_provider_atomic_failure_preserves_prior_bytes_and_identity() {
    let path = ArtifactPath::new(vec![b'n', 0, 0xff]);
    let mut storage = InMemoryArtifactStorage::new();
    let binding = storage.insert(path.clone(), b"before".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(0..6, "after").unwrap();

    let failed = document
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    storage.fail_next_write();
    assert_eq!(
        execute_prepared_artifact_write(&mut storage, &failed),
        Err(InMemoryStorageError::InjectedWriteFailure)
    );
    assert_eq!(storage.bytes(&path), Some(b"before".as_slice()));
    assert_eq!(storage.identity(&path), Some(binding.identity()));
    document.complete_artifact_write(failed.failed()).unwrap();

    let successful = document
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    let receipt = execute_prepared_artifact_write(&mut storage, &successful).unwrap();
    assert_eq!(receipt.identity(), binding.identity());
    document
        .complete_artifact_write(successful.succeeded(receipt))
        .unwrap();
    assert_eq!(storage.bytes(&path), Some(b"after".as_slice()));
    assert!(!document.is_dirty());
}

#[test]
fn preparation_closes_open_groups_and_core_exposes_separate_prepare_complete_phases() {
    let mut document = Document::new("");
    document.begin_edit_group();
    document.insert(0, "a").unwrap();
    let first_edit = document.history_status().current;
    let prepared = document.prepare_artifact_write(save_as("grouped")).unwrap();
    document.insert(1, "b").unwrap();
    assert_ne!(document.history_status().current, first_edit);
    document.try_undo().unwrap();
    assert_eq!(document.text(), "a");
    document.complete_artifact_write(prepared.failed()).unwrap();

    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new(""));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Text("a".to_owned())))
        .unwrap();
    let prepared = core.prepare_artifact_write(save_as("core.txt")).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Text("b".to_owned())))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    let status = core
        .complete_artifact_write(prepared.succeeded(receipt))
        .unwrap();
    assert!(matches!(
        status,
        ArtifactWriteCompletionStatus::Succeeded {
            document_is_dirty: true,
            ..
        }
    ));
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert_eq!(core.document().text(), "a");
    assert!(!core.document().is_dirty());
}

#[test]
fn invalid_range_and_existing_destination_fail_without_partial_provider_mutation() {
    let mut document = Document::new("abc");
    assert_eq!(
        document.prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: ArtifactPath::from("bad"),
            scope: ArtifactWriteScope::PrimarySourceBytes(2..4),
            overwrite: ArtifactOverwrite::ReplaceExisting,
        }),
        Err(PersistenceError::InvalidSourceRange {
            start: 2,
            end: 4,
            source_length: 3,
        })
    );

    let path = ArtifactPath::from("exists");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"original".to_vec()).unwrap();
    let prepared = document
        .prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: path.clone(),
            scope: ArtifactWriteScope::WholeArtifact,
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    assert_eq!(
        execute_prepared_artifact_write(&mut storage, &prepared),
        Err(InMemoryStorageError::AlreadyExists(path.clone()))
    );
    assert_eq!(storage.bytes(&path), Some(b"original".as_slice()));
    document.complete_artifact_write(prepared.failed()).unwrap();
}

#[test]
fn one_destination_has_at_most_one_in_flight_write_and_malformed_receipts_are_retryable() {
    let path = ArtifactPath::from("same");
    let mut document = Document::new("contents");
    let prepared = document
        .prepare_artifact_write(ArtifactWriteIntent::SaveAs {
            destination: path.clone(),
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    assert_eq!(
        document.prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: path.clone(),
            scope: ArtifactWriteScope::WholeArtifact,
            overwrite: ArtifactOverwrite::ReplaceExisting,
        }),
        Err(PersistenceError::DestinationWritePending(path.clone()))
    );

    let malformed = ArtifactWriteReceipt::new(
        ArtifactPath::from("different"),
        evim_core::document::ArtifactIdentity::new(vec![1]),
    );
    assert_eq!(
        document.complete_artifact_write(prepared.succeeded(malformed)),
        Err(PersistenceError::ReceiptDestinationMismatch)
    );
    assert_eq!(
        document.prepare_artifact_write(save_as("same")),
        Err(PersistenceError::DestinationWritePending(path.clone()))
    );

    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    document
        .complete_artifact_write(prepared.succeeded(receipt))
        .unwrap();
    let next = document.prepare_artifact_write(save_as("same")).unwrap();
    document.complete_artifact_write(next.failed()).unwrap();

    let current = ArtifactPath::from("current-identity");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(current.clone(), b"old".to_vec()).unwrap();
    let mut bound = Document::from_loaded_artifact(
        storage.read_artifact(&current).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    bound.replace(0..3, "new").unwrap();
    let save = bound
        .prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        })
        .unwrap();
    let wrong_identity = ArtifactWriteReceipt::new(
        current.clone(),
        evim_core::document::ArtifactIdentity::new(vec![0xff]),
    );
    assert_eq!(
        bound.complete_artifact_write(save.succeeded(wrong_identity)),
        Err(PersistenceError::ReceiptIdentityMismatch)
    );
    assert_eq!(
        bound.prepare_artifact_write(ArtifactWriteIntent::Save {
            overwrite: ArtifactOverwrite::ReplaceExisting,
        }),
        Err(PersistenceError::DestinationWritePending(current.clone()))
    );
    let valid_receipt = execute_prepared_artifact_write(&mut storage, &save).unwrap();
    bound
        .complete_artifact_write(save.succeeded(valid_receipt))
        .unwrap();
    assert_eq!(storage.bytes(&current), Some(b"new".as_slice()));
}
