use viem_core::command::ex_execute::{
    ExFileRequest, ExFrontendRequest, ExPostWriteDisposition, HardLineRange,
    PreparedExArtifactWrite, PreparedExFileRequest,
};
use viem_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactStorageProvider,
    ArtifactWriteCompletion, ArtifactWriteCompletionStatus, ArtifactWritePurpose,
    ArtifactWriteToken, Document, Encoding, FileFormat, Format, HardLineSourceRangeError,
    InMemoryArtifactStorage, LineEndingOpenPolicy, PersistenceError,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{command::InputEvent, command::Key};
use viem_core::{Core, CoreError, CoreEvent};

fn core_from(document: Document) -> Core<MockTextMeasurementProvider> {
    Core::new(document)
}

fn artifact_write(prepared: PreparedExFileRequest) -> PreparedExArtifactWrite {
    match prepared {
        PreparedExFileRequest::ArtifactWrite(write) => write,
        PreparedExFileRequest::Host(request) => {
            panic!(
                "expected artifact write, got host request {:?}",
                request.request()
            )
        }
    }
}

fn encode(text: &str, encoding: Encoding, bom: bool) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => {
            let mut bytes = if bom {
                vec![0xef, 0xbb, 0xbf]
            } else {
                Vec::new()
            };
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::Latin1 => text
            .chars()
            .map(|character| {
                let value = character as u32;
                assert!(value <= 0xff);
                value as u8
            })
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = if bom { vec![0xff, 0xfe] } else { Vec::new() };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = if bom { vec![0xfe, 0xff] } else { Vec::new() };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            bytes
        }
    }
}

#[test]
fn ordinary_current_write_replaces_existing_and_marks_only_its_captured_state_saved() {
    let path = ArtifactPath::from("current.md");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"# old\r\n".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::Markdown,
        LineEndingOpenPolicy::forced(FileFormat::Dos),
    )
    .unwrap();
    document.replace(0..3, "new").unwrap();
    let captured = document.history_status().current;
    let mut core = core_from(document);

    let prepared = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: false,
            range: None,
        })
        .unwrap(),
    );
    assert_eq!(prepared.tag().document(), core.document().id());
    assert_eq!(prepared.tag().revision(), core.document().revision());
    assert_eq!(prepared.write().purpose(), ArtifactWritePurpose::Save);
    assert_eq!(
        prepared.write().storage_request().overwrite(),
        ArtifactOverwrite::ReplaceExisting
    );
    assert_eq!(prepared.write().storage_request().bytes(), b"# new\r\n");

    let receipt = execute_prepared_artifact_write(&mut storage, prepared.write()).unwrap();
    let completion = prepared.write().succeeded(receipt);
    let completed = core
        .complete_ex_artifact_write(&prepared, completion)
        .unwrap();
    assert!(matches!(
        completed.status(),
        ArtifactWriteCompletionStatus::Succeeded {
            save_point: Some(saved),
            document_is_dirty: false,
            ..
        } if *saved == captured
    ));
    assert_eq!(
        completed.post_write(),
        &ExPostWriteDisposition::NotRequested
    );
    assert_eq!(storage.bytes(&path), Some(b"# new\r\n".as_slice()));
}

#[test]
fn alternate_and_saveas_overwrite_and_binding_rules_follow_bang() {
    let current = ArtifactPath::from("current");
    let alternate = ArtifactPath::from("alternate");
    let renamed = ArtifactPath::from("renamed");
    let mut storage = InMemoryArtifactStorage::new();
    let original_binding = storage.insert(current.clone(), b"old".to_vec()).unwrap();
    storage.insert(alternate.clone(), b"keep".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&current).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    let initial_save_point = document.history_status().save_point;
    document.replace(0..3, "new").unwrap();
    let dirty_node = document.history_status().current;
    let mut core = core_from(document);

    let refused = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: Some("alternate".to_owned()),
            force: false,
            range: None,
        })
        .unwrap(),
    );
    assert_eq!(
        refused.write().storage_request().overwrite(),
        ArtifactOverwrite::RefuseExisting
    );
    assert!(execute_prepared_artifact_write(&mut storage, refused.write()).is_err());
    core.complete_ex_artifact_write(&refused, refused.write().failed())
        .unwrap();
    assert_eq!(storage.bytes(&alternate), Some(b"keep".as_slice()));
    assert_eq!(core.document().artifact_binding(), Some(&original_binding));
    assert_eq!(
        core.document().history_status().save_point,
        initial_save_point
    );

    let forced = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: Some("alternate".to_owned()),
            force: true,
            range: None,
        })
        .unwrap(),
    );
    assert_eq!(
        forced.write().storage_request().overwrite(),
        ArtifactOverwrite::ReplaceExisting
    );
    let receipt = execute_prepared_artifact_write(&mut storage, forced.write()).unwrap();
    core.complete_ex_artifact_write(&forced, forced.write().succeeded(receipt))
        .unwrap();
    assert_eq!(storage.bytes(&alternate), Some(b"new".as_slice()));
    assert_eq!(core.document().artifact_binding(), Some(&original_binding));
    assert_eq!(core.document().history_status().current, dirty_node);
    assert!(core.document().is_dirty());

    let save_as = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::SaveAs {
            path: "renamed".to_owned(),
            force: false,
        })
        .unwrap(),
    );
    assert_eq!(save_as.write().purpose(), ArtifactWritePurpose::SaveAs);
    assert_eq!(
        save_as.write().storage_request().overwrite(),
        ArtifactOverwrite::RefuseExisting
    );
    assert_eq!(core.document().artifact_binding(), Some(&original_binding));
    let receipt = execute_prepared_artifact_write(&mut storage, save_as.write()).unwrap();
    core.complete_ex_artifact_write(&save_as, save_as.write().succeeded(receipt))
        .unwrap();
    assert_eq!(core.document().artifact_binding().unwrap().path(), &renamed);
    assert!(!core.document().is_dirty());
}

#[test]
fn ranged_writes_copy_exact_source_lines_in_every_encoding_and_line_mode() {
    struct Case {
        source: &'static str,
        selected: &'static str,
        encoding: Encoding,
        format: Format,
        file_format: FileFormat,
        lines: HardLineRange,
    }

    let cases = [
        Case {
            source: "# one\r\n**two**\n\n_three_\r\n\r\nlast",
            selected: "**two**\n\n_three_\r\n\r\n",
            encoding: Encoding::Utf8,
            format: Format::Markdown,
            file_format: FileFormat::Dos,
            lines: HardLineRange { start: 1, end: 2 },
        },
        Case {
            source: "café\nliteral\rtwo\rfinal",
            selected: "café\nliteral\r",
            encoding: Encoding::Latin1,
            format: Format::PlainText,
            file_format: FileFormat::Mac,
            lines: HardLineRange { start: 0, end: 0 },
        },
        Case {
            source: "a\nliteral\rb\rfinal",
            selected: "a\nliteral\rb\r",
            encoding: Encoding::Utf16Le,
            format: Format::PlainText,
            file_format: FileFormat::Mac,
            lines: HardLineRange { start: 0, end: 1 },
        },
        Case {
            source: "# a\r\n**b**\n\nlast",
            selected: "**b**\n\n",
            encoding: Encoding::Utf16Be,
            format: Format::Markdown,
            file_format: FileFormat::Dos,
            lines: HardLineRange { start: 1, end: 1 },
        },
        Case {
            source: "one\ntwo\nunterminated",
            selected: "unterminated",
            encoding: Encoding::Utf8,
            format: Format::PlainText,
            file_format: FileFormat::Unix,
            lines: HardLineRange { start: 2, end: 2 },
        },
    ];

    for (index, case) in cases.into_iter().enumerate() {
        let has_bom = matches!(case.encoding, Encoding::Utf16Le | Encoding::Utf16Be);
        let source = encode(case.source, case.encoding, has_bom);
        let document = Document::from_bytes_with_file_format(
            source,
            case.encoding,
            case.format,
            case.file_format,
        )
        .unwrap();
        let mut core = core_from(document);
        let prepared = artifact_write(
            core.prepare_ex_file_request(&ExFileRequest::Write {
                path: Some(format!("selection-{index}")),
                force: false,
                range: Some(case.lines),
            })
            .unwrap(),
        );
        assert_eq!(
            prepared.write().purpose(),
            ArtifactWritePurpose::WriteAlternate
        );
        assert_eq!(
            prepared.write().storage_request().bytes(),
            encode(case.selected, case.encoding, false),
            "wrong authoritative range for {:?}/{:?}/{:?}",
            case.format,
            case.encoding,
            case.file_format
        );
        core.complete_ex_artifact_write(&prepared, prepared.write().failed())
            .unwrap();
    }
}

#[test]
fn whole_current_range_saves_but_partial_current_range_requires_bang() {
    let path = ArtifactPath::from("current");
    let mut storage = InMemoryArtifactStorage::new();
    storage
        .insert(path.clone(), b"one\r\ntwo".to_vec())
        .unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::forced(FileFormat::Dos),
    )
    .unwrap();
    document.replace(0..3, "ONE").unwrap();
    let mut core = core_from(document);

    let whole = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: false,
            range: Some(HardLineRange { start: 0, end: 1 }),
        })
        .unwrap(),
    );
    assert_eq!(whole.write().purpose(), ArtifactWritePurpose::Save);
    assert_eq!(whole.write().storage_request().bytes(), b"ONE\r\ntwo");
    core.complete_ex_artifact_write(&whole, whole.write().failed())
        .unwrap();

    let partial_range = HardLineRange { start: 1, end: 1 };
    assert_eq!(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: false,
            range: Some(partial_range),
        }),
        Err(CoreError::PartialCurrentWriteRequiresForce {
            range: partial_range,
        })
    );

    let partial = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: true,
            range: Some(partial_range),
        })
        .unwrap(),
    );
    assert_eq!(
        partial.write().purpose(),
        ArtifactWritePurpose::WriteAlternate
    );
    assert_eq!(
        partial.write().storage_request().overwrite(),
        ArtifactOverwrite::ReplaceExisting
    );
    assert_eq!(partial.write().storage_request().bytes(), b"two");
    core.complete_ex_artifact_write(&partial, partial.write().failed())
        .unwrap();
}

#[test]
fn invalid_current_write_range_is_reported_before_partial_write_policy() {
    let path = ArtifactPath::from("current");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"one\ntwo".to_vec()).unwrap();
    let document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    let mut core = core_from(document);

    assert_eq!(
        core.prepare_ex_file_request(&ExFileRequest::Write {
            path: None,
            force: false,
            range: Some(HardLineRange { start: 1, end: 2 }),
        }),
        Err(CoreError::HardLineSourceRange(
            HardLineSourceRangeError::InvalidRange {
                start: 1,
                end: 3,
                line_count: 2,
            }
        ))
    );
}

#[test]
fn write_quit_waits_for_success_and_suppresses_quit_after_a_newer_edit() {
    let path = ArtifactPath::from("current");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"old".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(0..3, "captured").unwrap();
    let mut core = core_from(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    let prepared = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::WriteQuit {
            path: None,
            force: false,
            range: None,
        })
        .unwrap(),
    );
    assert!(matches!(
        prepared.after_success().unwrap().request(),
        ExFileRequest::Quit { force: false }
    ));

    let failed = core
        .complete_ex_artifact_write(&prepared, prepared.write().failed())
        .unwrap();
    assert!(matches!(
        failed.post_write(),
        ExPostWriteDisposition::WriteFailed(request)
            if matches!(request.request(), ExFileRequest::Quit { force: false })
    ));

    let delayed = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::WriteQuit {
            path: None,
            force: false,
            range: None,
        })
        .unwrap(),
    );
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('A'))))
        .unwrap();
    core.handle(
        view,
        CoreEvent::Input(InputEvent::Text(" newer".to_owned())),
    )
    .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();

    let receipt = execute_prepared_artifact_write(&mut storage, delayed.write()).unwrap();
    let completion = delayed.write().succeeded(receipt);
    let completed = core
        .complete_ex_artifact_write(&delayed, completion)
        .unwrap();
    assert!(matches!(
        completed.post_write(),
        ExPostWriteDisposition::DocumentChanged {
            request,
            current_revision,
            ..
        } if matches!(request.request(), ExFileRequest::Quit { .. })
            && *current_revision == core.document().revision()
    ));
    assert!(core.document().is_dirty());
    assert_eq!(storage.bytes(&path), Some(b"captured".as_slice()));
}

#[test]
fn alternate_write_quit_can_close_the_unchanged_captured_state_even_if_buffer_is_dirty() {
    let mut document = Document::new("old");
    document.replace(0..3, "new").unwrap();
    assert!(document.is_dirty());
    let mut core = core_from(document);
    let prepared = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::WriteQuit {
            path: Some("copy".to_owned()),
            force: false,
            range: None,
        })
        .unwrap(),
    );
    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, prepared.write()).unwrap();
    let completed = core
        .complete_ex_artifact_write(&prepared, prepared.write().succeeded(receipt))
        .unwrap();
    assert!(matches!(
        completed.status(),
        ArtifactWriteCompletionStatus::Succeeded {
            save_point: None,
            document_is_dirty: true,
            ..
        }
    ));
    assert!(matches!(
        completed.post_write(),
        ExPostWriteDisposition::Ready(request)
            if matches!(request.request(), ExFileRequest::Quit { force: false })
    ));
}

#[test]
fn xit_skips_a_clean_pathless_write_but_writes_when_dirty_or_given_a_path() {
    let document = Document::new("clean");
    let id = document.id();
    let revision = document.revision();
    let mut core = core_from(document);
    let clean = core
        .prepare_ex_file_request(&ExFileRequest::Xit {
            path: None,
            force: true,
        })
        .unwrap();
    let PreparedExFileRequest::Host(clean) = clean else {
        panic!("clean xit must skip storage");
    };
    assert_eq!(clean.tag().document(), id);
    assert_eq!(clean.tag().revision(), revision);
    assert!(matches!(
        clean.request(),
        ExFileRequest::Quit { force: true }
    ));

    let path = ArtifactPath::from("current");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"old".to_vec()).unwrap();
    let mut dirty = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    dirty.replace(0..3, "dirty").unwrap();
    let mut dirty_core = core_from(dirty);
    let dirty = artifact_write(
        dirty_core
            .prepare_ex_file_request(&ExFileRequest::Xit {
                path: None,
                force: false,
            })
            .unwrap(),
    );
    assert_eq!(dirty.write().purpose(), ArtifactWritePurpose::Save);
    dirty_core
        .complete_ex_artifact_write(&dirty, dirty.write().failed())
        .unwrap();

    let mut clean_core = core_from(Document::new("clean"));
    let alternate = artifact_write(
        clean_core
            .prepare_ex_file_request(&ExFileRequest::Xit {
                path: Some("copy".to_owned()),
                force: false,
            })
            .unwrap(),
    );
    assert_eq!(
        alternate.write().purpose(),
        ArtifactWritePurpose::WriteAlternate
    );
    clean_core
        .complete_ex_artifact_write(&alternate, alternate.write().failed())
        .unwrap();
}

#[test]
fn completion_must_match_prepared_document_and_token_and_stale_save_never_quits() {
    let path = ArtifactPath::from("current");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"old".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(0..3, "new").unwrap();
    let mut core = core_from(document);
    let wq = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::WriteQuit {
            path: None,
            force: false,
            range: None,
        })
        .unwrap(),
    );

    let mismatch = ArtifactWriteCompletion::failed(
        wq.write().document(),
        ArtifactWriteToken::from_u64(wq.write().token().as_u64() + 1),
    );
    assert!(matches!(
        core.complete_ex_artifact_write(&wq, mismatch),
        Err(CoreError::ExWriteCompletionMismatch { .. })
    ));
    let other = Document::new("other");
    assert!(matches!(
        core.complete_ex_artifact_write(
            &wq,
            ArtifactWriteCompletion::failed(other.id(), wq.write().token())
        ),
        Err(CoreError::ExWriteCompletionMismatch { .. })
    ));

    let newer = artifact_write(
        core.prepare_ex_file_request(&ExFileRequest::SaveAs {
            path: "new-name".to_owned(),
            force: false,
        })
        .unwrap(),
    );
    let newer_receipt = execute_prepared_artifact_write(&mut storage, newer.write()).unwrap();
    core.complete_ex_artifact_write(&newer, newer.write().succeeded(newer_receipt))
        .unwrap();
    let stale_receipt = execute_prepared_artifact_write(&mut storage, wq.write()).unwrap();
    assert!(matches!(
        core.complete_ex_artifact_write(&wq, wq.write().succeeded(stale_receipt)),
        Err(CoreError::Persistence(PersistenceError::StaleCompletion(token)))
            if token == wq.write().token()
    ));
}

#[test]
fn non_write_file_requests_remain_revision_tagged_host_work() {
    let document = Document::new("text");
    let id = document.id();
    let revision = document.revision();
    let mut core = core_from(document);
    for request in [
        ExFileRequest::Edit {
            path: Some("next".to_owned()),
            force: false,
        },
        ExFileRequest::New { force: true },
        ExFileRequest::Quit { force: false },
        ExFileRequest::QuitAll { force: true },
        ExFileRequest::WriteAll { force: false },
    ] {
        let prepared = core.prepare_ex_file_request(&request).unwrap();
        let PreparedExFileRequest::Host(tagged) = prepared else {
            panic!("non-write request unexpectedly became storage work");
        };
        assert_eq!(tagged.tag().document(), id);
        assert_eq!(tagged.tag().revision(), revision);
        assert_eq!(tagged.request(), &request);
    }
}

#[test]
fn source_line_query_rejects_empty_and_out_of_bounds_spans() {
    let document = Document::new("one\ntwo");
    assert_eq!(
        document.source_byte_range_for_hard_lines(1..1),
        Err(HardLineSourceRangeError::InvalidRange {
            start: 1,
            end: 1,
            line_count: 2,
        })
    );
    assert_eq!(
        document.source_byte_range_for_hard_lines(0..3),
        Err(HardLineSourceRangeError::InvalidRange {
            start: 0,
            end: 3,
            line_count: 2,
        })
    );
}

#[test]
fn outbound_ex_outcome_can_be_prepared_without_frontend_io_or_model_access() {
    let mut core = core_from(Document::new("text"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(':'))))
        .unwrap();
    core.handle(
        view,
        CoreEvent::Input(InputEvent::Text("write copy".to_owned())),
    )
    .unwrap();
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    let request = match &outcome
        .command
        .as_ref()
        .unwrap()
        .ex_outcome
        .as_ref()
        .unwrap()
        .frontend_requests[0]
    {
        ExFrontendRequest::File(request) => request.clone(),
        request => panic!("unexpected outbound request: {request:?}"),
    };

    let prepared = artifact_write(core.prepare_ex_file_request(&request).unwrap());
    assert_eq!(prepared.write().storage_request().bytes(), b"text");
    assert_eq!(prepared.tag().document(), core.document().id());
    assert_eq!(prepared.tag().revision(), core.document().revision());
    core.complete_ex_artifact_write(&prepared, prepared.write().failed())
        .unwrap();
}

#[test]
fn write_next_only_releases_navigation_after_the_exact_captured_state_is_saved() {
    use viem_core::command::argument_list::ExArgumentTarget;
    let path = ArtifactPath::from("argument.txt");
    let mut storage = InMemoryArtifactStorage::new();
    storage.insert(path.clone(), b"old".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(), Encoding::Utf8, Format::PlainText,
        LineEndingOpenPolicy::default(),
    ).unwrap();
    document.replace(0..3, "edited").unwrap();
    let mut core = core_from(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    let request = ExFileRequest::NavigateArgument {
        target: ExArgumentTarget::Previous(2), force: false,
        write_first: true, path: None, line: Some(12),
    };
    let continuation = ExFileRequest::NavigateArgument {
        target: ExArgumentTarget::Previous(2), force: false,
        write_first: false, path: None, line: Some(12),
    };
    let failed = artifact_write(core.prepare_ex_file_request(&request).unwrap());
    let result = core.complete_ex_artifact_write(&failed, failed.write().failed()).unwrap();
    assert!(matches!(result.post_write(), ExPostWriteDisposition::WriteFailed(_)));
    assert!(core.document().is_dirty());

    let delayed = artifact_write(core.prepare_ex_file_request(&request).unwrap());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('A')))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Text(" later".into()))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    let receipt = execute_prepared_artifact_write(&mut storage, delayed.write()).unwrap();
    let result = core.complete_ex_artifact_write(&delayed, delayed.write().succeeded(receipt)).unwrap();
    assert!(matches!(result.post_write(), ExPostWriteDisposition::DocumentChanged { .. }));
    assert!(core.document().is_dirty());

    let prepared = artifact_write(core.prepare_ex_file_request(&request).unwrap());
    assert_eq!(prepared.after_success().unwrap().request(), &continuation);
    let receipt = execute_prepared_artifact_write(&mut storage, prepared.write()).unwrap();
    let result = core.complete_ex_artifact_write(&prepared, prepared.write().succeeded(receipt)).unwrap();
    assert!(matches!(result.post_write(), ExPostWriteDisposition::Ready(ready)
        if ready.request() == &continuation));
    assert!(!core.document().is_dirty());
    assert_eq!(storage.bytes(&path), Some(b"edited later".as_slice()));
}
