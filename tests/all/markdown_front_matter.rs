use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    syntax::{
        service::{SyntaxProvider, SyntaxRequest, SyntaxResult},
        Coverage, SyntaxRun, SyntaxStyleName,
    },
    Association, BoundaryAffinity, DeletionRecovery, Encoding, FileFormat, Format,
    HistoryNavigationRequest, MappingOutcome, ModelRequest, StyleId,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn encode(source: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        Encoding::Latin1 => source.chars().map(|ch| ch as u8).collect(),
    }
}

#[test]
fn leading_metadata_is_literal_yaml_code_with_hidden_delimiters() {
    for (source, expected) in [
        ("---\nname: example\n---\n\n# Heading", "name: example\nHeading"),
        ("---\nname: example\n---", "name: example"),
        ("--- \t\nname: example\n---\t \n", "name: example"),
        (
            "---\n# comment\nvalue: '*literal* &amp; <b>x</b>'\nitems:\n  - one\n  - two\n---\n\n**Prose**",
            "# comment\nvalue: '*literal* &amp; <b>x</b>'\nitems:\n  - one\n  - two\nProse",
        ),
        // Incomplete YAML remains editable as literal metadata while typing.
        ("---\nvalue: [unfinished\n---", "value: [unfinished"),
        ("---\n---", ""),
    ] {
        let document = open(source);
        assert_eq!(document.text(), expected, "{source:?}");
        let block = &document.projection().blocks()[0];
        assert!(block.is_front_matter(), "{source:?}");
        assert_eq!(block.style.0, "Code Block");
        assert_eq!(block.code_language.as_deref(), Some("yaml"));
        assert!(document.projection().blocks()[1..]
            .iter()
            .all(|block| !block.is_front_matter()));
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.is_dirty());
    }
}

#[test]
fn unclosed_nonleading_and_other_delimiters_keep_ordinary_markdown_behavior() {
    for source in [
        "---\nname: example",
        "---\nname: example\n...",
        "---\nname: example\n----",
        " ---\nname: example\n---",
        "\n---\nname: example\n---",
        "Prose\n\n---\nname: example\n---",
        "> ---\n> name: example\n> ---",
        "```yaml\n---\nname: example\n---\n```",
    ] {
        let document = open(source);
        assert!(
            document
                .projection()
                .blocks()
                .iter()
                .all(|block| !block.is_front_matter()),
            "{source:?}"
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn body_edits_preserve_bom_encoding_line_endings_and_exact_history() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            for bom in [false, true] {
                if bom && encoding == Encoding::Latin1 {
                    continue;
                }
                let ending = match file_format {
                    FileFormat::Unix => "\n",
                    FileFormat::Dos => "\r\n",
                    FileFormat::Mac => "\r",
                };
                let source = format!(
                    "{}--- \t\n# untouched comment\nname: 'café'\n---\t\n\n__Untouched prose__\n",
                    if bom { "\u{feff}" } else { "" },
                )
                .replace('\n', ending);
                let bytes = encode(&source, encoding);
                let mut document = Document::from_bytes_with_file_format(
                    bytes.clone(),
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                assert_eq!(document.source_bytes(), bytes);
                assert!(document.projection().blocks()[0].is_front_matter());
                let at = document.text().find("café").unwrap();
                document.replace(at..at + "café".len(), "thé").unwrap();
                let expected = encode(&source.replace("café", "thé"), encoding);
                assert_eq!(
                    document.source_bytes(),
                    expected,
                    "{encoding:?} {file_format:?} bom={bom}"
                );
                let reopened = Document::from_bytes_with_file_format(
                    expected.clone(),
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                assert_eq!(reopened.text(), document.text());
                assert!(reopened.projection().blocks()[0].is_front_matter());
                assert_eq!(
                    reopened.projection().style_spans(),
                    document.projection().style_spans()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), bytes);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), expected);
            }
        }
    }
}

#[test]
fn native_insert_into_metadata_retains_literal_punctuation_and_undo_group() {
    let source = "---\nname: example\n---\n\nTail";
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    let at = core.document().text().find("example").unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    for key in [
        Key::Char('i'),
        Key::Char('*'),
        Key::Char('x'),
        Key::Char('*'),
        Key::Escape,
    ] {
        let outcome = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
        if let Some(command) = outcome.command {
            assert!(
                matches!(
                    command.status,
                    CommandStatus::Complete | CommandStatus::Pending
                ),
                "{:?}",
                command.status
            );
        }
    }
    let expected = source.replace("example", "*x*example");
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_eq!(core.document().text(), "name: *x*example\nTail");
    assert_eq!(open(&expected).text(), core.document().text());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
}

fn assert_key_edit(source: &str, at: usize, keys: &[Key], expected: &str, front_matter: bool) {
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    for key in keys {
        let outcome = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key.clone())))
            .unwrap_or_else(|error| panic!("{source:?}, at {at}, {keys:?}: {error:?}"));
        if let Some(command) = outcome.command {
            assert!(
                matches!(
                    command.status,
                    CommandStatus::Complete | CommandStatus::Pending
                ),
                "{source:?}, at {at}, {keys:?}: {:?}",
                command.status
            );
        }
    }
    assert_eq!(
        core.document().text(),
        expected,
        "{source:?}, at {at}, {keys:?}"
    );
    assert_eq!(
        core.document().projection().blocks()[0].is_front_matter(),
        front_matter
    );
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(
        reopened.text(),
        expected,
        "saved {:?}",
        String::from_utf8_lossy(&saved)
    );
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.attributes))
            .collect::<Vec<_>>(),
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.attributes))
            .collect::<Vec<_>>()
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), saved);
}

#[test]
fn empty_body_typing_splitting_joining_and_last_character_deletion_reopen_exactly() {
    assert_key_edit(
        "---\n---\n\nTail",
        0,
        &[Key::Char('i'), Key::Char('x'), Key::Escape],
        "x\nTail",
        true,
    );
    assert_key_edit(
        "---\nname: example\n---\n\nTail",
        6,
        &[Key::Char('i'), Key::Enter, Key::Escape],
        "name: \nexample\nTail",
        true,
    );
    assert_key_edit(
        "---\nfirst: 1\nsecond: 2\n---\n\nTail",
        "first: 1\n".len(),
        &[Key::Char('i'), Key::Backspace, Key::Escape],
        "first: 1second: 2\nTail",
        true,
    );
    assert_key_edit("---\nx\n---\n\nTail", 0, &[Key::Char('x')], "\nTail", true);
}

#[test]
fn empty_metadata_body_insertion_materializes_content_before_its_closer() {
    for suffix in ["", "\n\nTail"] {
        let source = format!("---\n---{suffix}");
        for replacement in ["X", "\nX", "X\n"] {
            let mut document = open(&source);
            document
                .insert(0, replacement)
                .unwrap_or_else(|error| panic!("{source:?}, {replacement:?}: {error:?}"));
            let expected = format!("---\n{replacement}\n---{suffix}");
            assert_eq!(document.source_bytes(), expected.as_bytes());
            assert_eq!(
                document.text(),
                format!(
                    "{replacement}{}",
                    if suffix.is_empty() { "" } else { "\nTail" }
                )
            );
            assert!(document.projection().blocks()[0].is_front_matter());
            assert_eq!(open(&expected).text(), document.text());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn replacing_the_complete_body_keeps_its_metadata_owner() {
    let source = "---\nfirst: 1\nsecond: 2\n---\n\nTail";
    for replacement in [
        "",
        "name: replacement",
        "key: [unfinished\nliteral: '*value*'",
    ] {
        let mut document = open(source);
        let range = document.projection().blocks()[0].range.clone();
        document.replace(range, replacement).unwrap();
        assert_eq!(document.text(), format!("{replacement}\nTail"));
        assert!(document.projection().blocks()[0].is_front_matter());
        let saved = document.source_bytes();
        assert!(saved.starts_with(b"---\n"));
        assert!(saved.ends_with(b"---\n\nTail"));
        let reopened =
            Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        assert!(reopened.projection().blocks()[0].is_front_matter());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn deleting_the_boundary_to_following_prose_keeps_first_block_context() {
    assert_key_edit(
        "---\nname: example\n---\n\nTail",
        "name: example".len() - 1,
        &[Key::Char('a'), Key::Delete, Key::Escape],
        "name: exampleTail",
        true,
    );
}

#[test]
fn linewise_deletion_of_complete_metadata_removes_its_delimiters() {
    for (source, keys) in [
        (
            "---\nname: example\n---\n\nTail",
            vec![Key::Char('d'), Key::Char('d')],
        ),
        (
            "---\nname: example\n---\n\nTail",
            vec![Key::Char('V'), Key::Char('d')],
        ),
        (
            "---\nname: example\nenabled: true\n---\n\nTail",
            vec![Key::Char('2'), Key::Char('d'), Key::Char('d')],
        ),
        (
            "---\nname: example\nenabled: true\n---\n\nTail",
            vec![Key::Char('V'), Key::Char('j'), Key::Char('d')],
        ),
    ] {
        assert_key_edit(source, 0, &keys, "Tail", false);
    }
}

#[test]
fn deleting_complete_metadata_preserves_the_document_bom() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for file_format in [FileFormat::Unix, FileFormat::Dos] {
            let source = "\u{feff}---\nname: example\n---\n\nTail".replace(
                '\n',
                if file_format == FileFormat::Dos {
                    "\r\n"
                } else {
                    "\n"
                },
            );
            let original = encode(&source, encoding);
            let document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
            for key in [Key::Char('d'), Key::Char('d')] {
                let outcome = core
                    .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                    .unwrap();
                if let Some(command) = outcome.command {
                    assert!(
                        matches!(
                            command.status,
                            CommandStatus::Complete | CommandStatus::Pending
                        ),
                        "{encoding:?}, {file_format:?}: {:?}",
                        command.status
                    );
                }
            }
            let expected = encode("\u{feff}Tail", encoding);
            assert_eq!(
                core.document().source_bytes(),
                expected,
                "{encoding:?}, {file_format:?}"
            );
            assert_eq!(core.document().text(), "Tail");
            assert!(!core.document().projection().blocks()[0].is_front_matter());
            let reopened = Document::from_bytes_with_file_format(
                expected.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            assert_eq!(reopened.text(), "Tail");
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), original);
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), expected);
        }
    }
}

#[test]
fn edits_that_create_an_early_metadata_closer_reject_atomically() {
    for (source, range, replacement) in [
        ("---\nname: example\n---\n\nTail", 0..0, "---\n"),
        ("---\n----\n---\n\nTail", 0..1, ""),
        ("---\nvalue\n---\n\nTail", 0..5, "---"),
    ] {
        let mut document = open(source);
        let revision = document.revision();
        let history = document.history_status();
        let text = document.text().to_owned();
        assert!(
            document.replace(range, replacement).is_err(),
            "{source:?}, {replacement:?}"
        );
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status(), history);
        assert_eq!(document.text(), text);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.is_dirty());
    }
}

#[test]
fn metadata_language_changes_are_rejected_without_mutation() {
    let source = "---\nname: example\n---\n\nTail";
    let mut document = open(source);
    for language in [None, Some("yaml"), Some("rust"), Some("front matter")] {
        let revision = document.revision();
        let history = document.history_status();
        let text = document.text().to_owned();
        assert!(document
            .prepare_code_block_language(document.id(), revision, 0, language)
            .is_err());
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status(), history);
        assert_eq!(document.text(), text);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.is_dirty());
    }
    assert!(!document.undo());
}

#[test]
fn new_code_blocks_do_not_inherit_language_from_front_matter() {
    for (source, expected) in [
        ("---\nname: example\n---\n\nProse", None),
        (
            "---\nname: example\n---\n\nProse\n\n```rust\nfn main() {}\n```",
            Some("rust"),
        ),
    ] {
        let mut document = open(source);
        let at = document.text().find("Prose").unwrap();
        document
            .set_paragraph_style(at..at, StyleId::from("Code Block"))
            .unwrap();
        let block = document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&at))
            .unwrap();
        assert!(!block.is_front_matter());
        assert_eq!(block.style.0, "Code Block");
        assert_eq!(block.code_language.as_deref(), expected);
        assert!(document
            .source_bytes()
            .starts_with(b"---\nname: example\n---\n\n"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn preview_switches_keep_source_and_metadata_body_anchors() {
    let source = "\u{feff}---\r\nname: example\r\n---\r\n\r\n# Heading\r\n";
    let mut document = Document::from_bytes_with_file_format(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Code,
        FileFormat::Dos,
    )
    .unwrap();
    for target in [Format::Markdown, Format::Code, Format::Markdown] {
        let at = document.text().find("example").unwrap();
        let anchor = document
            .text_anchor(
                document.text_point(at).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::SetViewFormat {
                document: document.id(),
                revision: document.revision(),
                target,
            })
            .unwrap();
        assert!(prepared.summary().source_patches().is_empty());
        let mapped = match prepared
            .text_position_map()
            .map_text_anchor(anchor)
            .unwrap()
        {
            MappingOutcome::Exact(anchor) | MappingOutcome::Moved(anchor) => anchor,
            other => panic!("metadata content lost its identity: {other:?}"),
        };
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(mapped.offset(), document.text().find("example").unwrap());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            document.projection().blocks()[0].is_front_matter(),
            target == Format::Markdown
        );
        if target == Format::Code {
            assert!(document.text().starts_with("---\n"));
        }
    }
    for _ in 0..3 {
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    assert_eq!(document.format(), Format::Code);
}

struct MetadataSyntax(Arc<Mutex<Vec<(String, String)>>>);

impl SyntaxProvider for MetadataSyntax {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        self.0.lock().unwrap().push((
            request.configuration.language.clone().unwrap(),
            request
                .input
                .slice(0..request.input.byte_len())
                .unwrap()
                .to_owned(),
        ));
        SyntaxResult {
            input: request.input.identity(),
            configuration: request.configuration.clone(),
            range: request.range.clone(),
            runs: vec![SyntaxRun {
                range: request.range.clone(),
                name: SyntaxStyleName("Statement".into()),
                origin: "front-matter-fixture".into(),
                priority: 0,
            }],
            coverage: Coverage::Exact,
            diagnostics: Vec::new(),
            continuation: false,
        }
    }
}

#[test]
fn syntax_provider_receives_yaml_body_without_metadata_delimiters() {
    let source = "---\nname: example\nenabled: true\n---\n\n# Heading";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut core = Core::new(open(source));
    core.set_syntax_provider_factory(Arc::new(move || Box::new(MetadataSyntax(captured.clone()))));
    core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.lock().unwrap().is_empty() {
        core.poll_syntax();
        assert!(
            Instant::now() < deadline,
            "metadata YAML syntax was not requested"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    core.poll_syntax();
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .all(|(language, body)| { language == "yaml" && body == "name: example\nenabled: true" }));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}

#[test]
fn small_metadata_edit_does_not_materialize_or_reproject_large_prose_suffix() {
    for count in [100, 10_000] {
        let source = format!(
            "---\ntitle: old\n---\n\n{}",
            "Unrelated **prose**.\n\n".repeat(count)
        );
        let mut document = open(&source);
        let (result, work) =
            viem_core::document::measure_document_work(|| document.replace(7..10, "new"));
        result.unwrap();
        assert_eq!(work.source_full_materializations, 0, "{count}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{count}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{count}: {work:?}");
        assert_eq!(
            document.source_bytes(),
            source.replace("title: old", "title: new").as_bytes()
        );
        assert!(document.projection().blocks()[0].is_front_matter());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn ex_rule_transfers_preserve_thematic_rules_without_creating_metadata() {
    for (source, at, command, expected_source, expected_text) in [
        ("a\n\n---\n\nb", 2, "t0", "***\n\na\n\n---\n\nb", "\na\n\nb"),
        ("a\n\n---\n\nb\n\n---\n\nc", 2, "m0", "***\n\na\n\nb\n\n---\n\nc", "\na\nb\n\nc"),
        ("a\n\n---\n\nb\n\n---\n\nc", 0, "m$", "***\n\nb\n\n---\n\nc\n\na", "\nb\n\nc\na"),
        ("---\n\na", 0, "t$", "***\n\na\n\n---", "\na\n"),
        // A lone leading rule needs no supporting syntax change.
        ("a\n\n---\n\nb", 2, "m0", "---\n\na\n\nb", "\na\nb"),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let source = format!("\u{feff}{source}");
            let expected_source = format!("\u{feff}{expected_source}");
            let original = encode(&source, encoding);
            let document = Document::from_bytes_with_file_format(
                original.clone(), encoding, Format::Markdown, FileFormat::Unix,
            ).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
            // Numeric Ex addresses name physical source rows. Select the
            // visible thematic-rule row and use the contextual current address.
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            }).unwrap();
            for key in std::iter::once(Key::Char(':'))
                .chain(command.chars().map(Key::Char)).chain(std::iter::once(Key::Enter))
            {
                let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                if let Some(result) = outcome.command {
                    assert!(matches!(result.status, CommandStatus::Complete | CommandStatus::Pending),
                        "{source:?}, :{command}, {encoding:?}: {:?}", result.status);
                }
            }
            assert_eq!(core.document().text(), expected_text, "{source:?}, :{command}");
            let saved = encode(&expected_source, encoding);
            assert_eq!(core.document().source_bytes(), saved, "{source:?}, :{command}");
            assert!(core.document().projection().blocks().iter().all(|block| !block.is_front_matter()));
            let reopened = Document::from_bytes_with_file_format(
                saved.clone(), encoding, Format::Markdown, FileFormat::Unix,
            ).unwrap();
            assert_eq!(reopened.text(), expected_text);
            assert!(reopened.projection().blocks().iter().all(|block| !block.is_front_matter()));
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), original);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}
