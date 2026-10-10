use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{
    Encoding, FileFormat, Format, ModelRequest, ProjectionWorkScope,
    SemanticInlineStyle, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        _ => text.as_bytes().to_vec(),
    }
}

#[test]
fn markdown_tree_sitter_highlighting_requires_code_mode_and_preserves_native_views() {
    use viem_core::DocumentMode;
    let text = "# Heading\r\n\r\n**café** and *italic* [link](https://example.com)\r\n";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for format in [Format::MarkdownSource, Format::Markdown] {
            for mode in [DocumentMode::Automatic, DocumentMode::Code("markdown".into())] {
                let bytes = encoded(text, encoding);
                let document = Document::from_bytes_with_file_format(
                    bytes.clone(), encoding, format, FileFormat::Dos,
                ).unwrap();
                let native_text = document.text().to_owned();
                let native_spans = document.projection().style_spans().to_vec();
                let mut core = Core::new(document);
                core.initialize_code_detection("notes.md", true).unwrap();
                let view = core.add_view(MockTextMeasurementProvider::new(), 900., 600.);
                assert_eq!(core.document().format(), format);
                assert_eq!(core.document().text(), native_text);
                core.poll_syntax();
                assert_eq!(core.syntax_statistics().requests, 0);
                assert!(core.syntax_style_names().is_empty());

                core.handle(view, CoreEvent::SetDocumentMode {
                    document: core.document().id(), revision: core.document().revision(),
                    mode, formatted_markdown: format == Format::Markdown,
                }).unwrap();
                assert_eq!(core.document().format(), Format::Code);
                assert_eq!(core.document().text(), text.replace("\r\n", "\n"));
                assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some("markdown"));
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let expected = ["Markup.heading.1", "Markup.strong", "Markup.italic", "Markup.link.url"];
                loop {
                    core.poll_syntax();
                    if expected.iter().all(|name| core.syntax_style_names().iter().any(|actual| actual == name)) {
                        break;
                    }
                    assert!(std::time::Instant::now() < deadline,
                        "{:?}: {}", core.syntax_style_names(), core.syntax_diagnostics());
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                assert_eq!(core.document().source_bytes(), bytes);
                assert!(!core.document().is_dirty());
                key(&mut core, view, Key::Char('u'));
                assert_eq!(core.document().format(), format);
                assert_eq!(core.document().text(), native_text);
                assert_eq!(core.document().projection().style_spans(), native_spans);
                let requests = core.syntax_statistics().requests;
                core.poll_syntax();
                assert_eq!(core.syntax_statistics().requests, requests);
                assert!(core.syntax_style_names().is_empty());
                assert_eq!(core.document().source_bytes(), bytes);
                assert!(!core.document().is_dirty());
            }
        }
    }
}

#[test]
fn json_and_jsonc_open_as_literal_code_with_their_distinct_bundled_languages() {
    for (filename, source, language) in [
        (
            "settings.json",
            "{\"enabled\": true, \"text\": \"a\\\"b\"}\n",
            "json",
        ),
        ("settings.jsonc", "// settings\n{\"enabled\": true}\n", "jsonc"),
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::PlainText,
        )
        .unwrap();
        let mut core = Core::<MockTextMeasurementProvider>::new(document);
        core.initialize_code_detection(filename, true).unwrap();
        assert_eq!(core.document().format(), Format::Code);
        assert_eq!(
            core.code_language_detection().unwrap().language.as_deref(),
            Some(language)
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(!core.document().is_dirty());
    }
}

#[test]
fn git_messages_open_as_literal_code_preserving_bytes_and_explicit_formats() {
    let text = "Describe café\r\n\r\n# Changes to be committed:\r\n#\tmodified:   file.txt\r\n";
    for filename in ["COMMIT_EDITMSG", "MERGE_MSG", "SQUASH_MSG", "TAG_EDITMSG", "NOTES_EDITMSG", "EDIT_DESCRIPTION"] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let source = encoded(text, encoding);
            for (format, automatic, expected_format) in [
                (Format::PlainText, true, Format::Code),
                (Format::PlainText, false, Format::PlainText),
                (Format::MarkdownSource, true, Format::MarkdownSource),
            ] {
                let document = Document::from_bytes_with_file_format(
                    source.clone(), encoding, format, FileFormat::Dos,
                ).unwrap();
                let revision = document.revision();
                let mut core = Core::<MockTextMeasurementProvider>::new(document);
                core.initialize_code_detection(filename, automatic).unwrap();
                assert_eq!(core.document().format(), expected_format, "{filename}");
                assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some("gitcommit"), "{filename}");
                assert_eq!(core.document().source_bytes(), source);
                assert_eq!(core.document().revision(), revision);
                assert_eq!(core.document().file_format(), FileFormat::Dos);
                assert!(!core.document().is_dirty());
            }
        }
    }
}

#[test]
fn detected_git_commit_syntax_uses_custom_comments_after_crlf_decoding() {
    use viem_core::document::{FormattedTextTree, syntax::{
        Coverage, SyntaxInputIdentity, SyntaxInputSnapshot,
        vim::{VimBudget, VimLoadLimits, VimProgram, VimSession, VimSetupContext},
    }};
    let source = format!("Commit summary\r\n\r\n{}; Please enter a message.\r\n", "Body line\r\n".repeat(40));
    let document = Document::from_bytes_with_file_format(
        source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Dos,
    ).unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    let filename = r"C:\project\.git\COMMIT_EDITMSG";
    core.initialize_code_detection(filename, true).unwrap();
    let language = core.code_language_detection().unwrap().language.as_deref().unwrap();
    assert_eq!(language, "gitcommit");
    let text = core.document().text();
    assert_eq!(text, source.replace("\r\n", "\n"));
    let input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 1, revision: 1, generation: 1 },
        FormattedTextTree::try_from_text(text).unwrap(),
    );
    let mut context = VimSetupContext::from_input(&input);
    context.filename = Some(filename.into());
    let program = VimProgram::load_directory_with_context(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vim/runtime/syntax"),
        language, VimLoadLimits::default(), &context,
        &std::sync::atomic::AtomicBool::new(false),
    ).unwrap();
    let mut session = VimSession::new(program);
    let result = (0..256).find_map(|_| {
        let result = session.highlight(&input, 0..text.len(), VimBudget { allow_provisional: false, ..Default::default() });
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        (!result.stats.yielded).then_some(result)
    }).expect("commit syntax must finish within bounded slices");
    assert_eq!(result.coverage, Coverage::Exact);
    for (token, style) in [("Commit summary", "Keyword"), ("; Please enter", "Comment")] {
        let start = text.find(token).unwrap();
        assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == style), "{token}: {:?}", result.runs);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}

#[test]
fn detected_languages_preserve_bytes_and_actual_bundled_highlighting() {
    use viem_core::document::{FormattedTextTree, syntax::{
        Coverage, SyntaxInputIdentity, SyntaxInputSnapshot,
        vim::{VimBudget, VimLoadLimits, VimProgram, VimSession, VimSetupContext},
    }};
    for (filename, source, language, token, style) in [
        (".zshrc", "autoload -Uz compinit\r\nsetopt auto_cd\r\n", "zsh", "auto_cd", "Constant"),
        (".zshrc.local", "repeat 3 do print hello; done\r\n", "zsh", "repeat", "Repeat"),
        ("script.sh", "#!/usr/bin/env zsh\r\nsetopt auto_cd\r\n", "zsh", "auto_cd", "Constant"),
        (".bashrc", "shopt -s nullglob\r\n", "bash", "shopt", "Statement"),
        (".profile", "printf 'hello'\r\n", "sh", "printf", "Statement"),
        (".kshrc", "autoload helper\r\n", "ksh", "autoload", "Statement"),
        ("script.mksh", "autoload helper\r\nbind '^L=clear-screen'\r\n", "mksh", "autoload", "Statement"),
        ("settings.jsonc", "// settings\r\n{\"enabled\": true}\r\n", "jsonc", "// settings", "Comment"),
        ("Main.scala", "object Main {\r\n  val answer = 42\r\n}\r\n", "scala", "object", "Keyword"),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let bytes = encoded(source, encoding);
            let document = Document::from_bytes_with_file_format(
                bytes.clone(), encoding, Format::PlainText, FileFormat::Dos,
            ).unwrap();
            let revision = document.revision();
            let mut core = Core::<MockTextMeasurementProvider>::new(document);
            core.initialize_code_detection(filename, true).unwrap();
            assert_eq!(core.document().format(), Format::Code, "{filename}");
            assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some(language));
            let text = core.document().text();
            let input = SyntaxInputSnapshot::new(
                SyntaxInputIdentity { document: 1, revision: 1, generation: 1 },
                FormattedTextTree::try_from_text(text).unwrap(),
            );
            let mut context = VimSetupContext::from_input(&input);
            context.filename = Some(filename.into());
            let program = VimProgram::load_directory_with_context(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vim/runtime/syntax"),
                language, VimLoadLimits::default(), &context,
                &std::sync::atomic::AtomicBool::new(false),
            ).unwrap_or_else(|diagnostics| panic!("{filename}: {diagnostics:?}"));
            let mut session = VimSession::new(program);
            let result = (0..256).find_map(|_| {
                let result = session.highlight(&input, 0..text.len(), VimBudget { allow_provisional: false, ..Default::default() });
                assert!(result.diagnostics.is_empty(), "{filename}: {:?}", result.diagnostics);
                (!result.stats.yielded).then_some(result)
            }).expect("detected syntax must finish within bounded slices");
            assert_eq!(result.coverage, Coverage::Exact);
            let start = text.find(token).unwrap();
            assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == style),
                "{filename}: {:?}", result.runs);
            assert_eq!(core.document().source_bytes(), bytes);
            assert_eq!(core.document().revision(), revision);
            assert!(!core.document().is_dirty());
        }
    }
}

#[test]
fn html_opens_as_literal_code_preserving_encoding_endings_and_explicit_text() {
    let text = "<!doctype html>\r\n<h1 title=\"literal\">&amp; text</h1>\r\n<script>alert('x')</script>\r\n";
    for filename in ["page.html", "page.htm", "page.xhtml", "PAGE.HTML", "PAGE.HTM", "PAGE.XHTML"] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
            let source = encoded(text, encoding);
            let document = Document::from_bytes_with_file_format(
                source.clone(), encoding, Format::PlainText, FileFormat::Dos,
            ).unwrap();
            let revision = document.revision();
            let mut core = Core::<MockTextMeasurementProvider>::new(document);
            core.initialize_code_detection(filename, true).unwrap();
            assert_eq!(core.document().format(), Format::Code, "{filename}");
            assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some("html"));
            assert_eq!(core.document().text(), text.replace("\r\n", "\n"));
            assert_eq!(core.document().source_bytes(), source);
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().file_format(), FileFormat::Dos);
            assert_eq!(core.document().encoding(), encoding);
            assert!(!core.document().is_dirty());
        }
    }
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new(text));
    core.initialize_code_detection("page.html", false).unwrap();
    assert_eq!(core.document().format(), Format::PlainText);
}

#[test]
fn html_code_typing_inserts_only_the_authored_text() {
    for batch in [false, true] {
        let mut core = Core::<MockTextMeasurementProvider>::new(Document::new(""));
        core.initialize_code_detection("page.html", true).unwrap();
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
        key(&mut core, view, Key::Char('i'));
        if batch {
            core.handle(view, CoreEvent::Input(InputEvent::text("<div>"))).unwrap();
        } else {
            let mut authored = String::new();
            for character in "<div>".chars() {
                authored.push(character);
                key(&mut core, view, Key::Char(character));
                assert_eq!(core.document().text(), authored);
            }
        }
        assert_eq!(core.document().source_bytes(), b"<div>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 5);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert!(core.document().source_bytes().is_empty());
    }
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    let status = output.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending)
            || key == Key::Escape && status == CommandStatus::Cancelled,
        "{key:?}: {status:?}"
    );
}

#[test]
fn code_is_literal_and_preserves_source_bytes() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        let text = "# Heading\r\n<b title=\"yes\">&amp;</b>\r\n  // comment\r\n\r\n";
        let bytes = encoded(text, encoding);
        let mut document = Document::from_bytes_with_file_format(
            bytes.clone(),
            encoding,
            Format::Code,
            FileFormat::Dos,
        )
        .unwrap();
        assert_eq!(document.text(), text.replace("\r\n", "\n"));
        assert_eq!(document.source_bytes(), bytes);
        assert_eq!(document.projection().hard_line_count(), 5);
        assert_eq!(document.projection().blocks().len(), 5);
        assert!(!document.is_dirty());
        assert!(document.projection().style_spans().is_empty());
        let initial_revision = document.revision();
        assert!(document
            .set_semantic_style(0..1, SemanticInlineStyle::Strong, true)
            .is_err());
        assert_eq!(document.revision(), initial_revision);

    }
}

#[test]
fn code_keeps_quotes_and_syntax_literal_in_insert_replace_and_counted_replace() {
    for (command, input, expected) in [
        (
            "i",
            "\"word\" isn't 'x'\n<b>\n- item\n# title\n",
            "\"word\" isn't 'x'\n<b>\n- item\n# title\nxxxxxx",
        ),
        ("r", "\"", "\"xxxxx"),
        ("3r", "\"", "\"\"\"xxx"),
        ("R", "\"a\"", "\"a\"xxx"),
    ] {
        let mut core = Core::new(
            Document::from_bytes(b"xxxxxx".to_vec(), Encoding::Utf8, Format::Code).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 320.0, 120.0);
        core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
        for ch in command.chars() {
            key(&mut core, view, Key::Char(ch));
        }
        core.handle(view, CoreEvent::Input(InputEvent::text(input)))
            .unwrap();
        assert_eq!(core.document().text(), expected, "{command}");
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"xxxxxx", "{command}");
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(
            core.document().source_bytes(),
            expected.as_bytes(),
            "{command}"
        );
    }
}

#[test]
fn code_composition_commits_supplied_quotes_as_one_undo_unit() {
    let mut core =
        Core::new(Document::from_bytes(b"xxxxxx".to_vec(), Encoding::Utf8, Format::Code).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 320.0, 120.0);
    core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
    key(&mut core, view, Key::Char('i'));
    let target = CompositionTarget::at_offsets(core.document(), 0..6).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
            "\"a\"",
            3..3,
        ))),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"xxxxxx");
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"\"a\"");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), b"xxxxxx");
}

#[test]
fn code_pastes_rich_clipboard_as_literal_plain_text_without_syntax_assignments() {
    let mut donor = Document::from_bytes(
        b"**\"word\"**".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    let mut copied: Option<ClipboardContent> = None;
    for ch in "v$\"+y".chars() {
        let result = commands
            .handle_with_clipboard_context(&mut donor, InputEvent::key(ch), &context)
            .unwrap();
        if let Some(write) = result.clipboard_writes.first() {
            copied = Some(write.content().clone());
        }
    }
    let content = copied.unwrap();
    assert_eq!(content.plain_text(), "\"word\"");
    assert!(content
        .portable_register()
        .unwrap()
        .clipboard_fragment()
        .is_some());
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        content,
    ));
    for enter in ['i', 'R'] {
        let mut target = Document::from_bytes(b"X".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.set_smart_quotes(true);
        for input in [
            InputEvent::key(enter),
            InputEvent::Key(Key::Ctrl('r')),
            InputEvent::key('+'),
            InputEvent::Key(Key::Escape),
        ] {
            let result = commands
                .handle_with_clipboard_context(&mut target, input, &context)
                .unwrap();
            assert!(
                matches!(
                    result.status,
                    CommandStatus::Complete | CommandStatus::Pending
                ),
                "{:?}",
                result.status
            );
        }
        assert_eq!(
            target.text(),
            if enter == 'i' {
                "\"word\"X"
            } else {
                "\"word\""
            }
        );
        assert!(target.projection().style_spans().is_empty());
        assert_eq!(target.source_bytes(), target.text().as_bytes());
        assert!(target.undo());
        assert_eq!(target.source_bytes(), b"X");
    }
}

#[test]
fn million_line_code_newline_edits_keep_projection_work_regional_in_utf8_and_utf16_crlf() {
    for (encoding, file_format, ending) in [
        (Encoding::Utf8, FileFormat::Unix, "\n"),
        (Encoding::Utf16Le, FileFormat::Dos, "\r\n"),
    ] {
        let mut prior_work = None;
        for lines in [10_000, 1_000_000] {
            let source = format!("x{ending}").repeat(lines);
            let mut document = Document::from_bytes_with_file_format(
                encoded(&source, encoding),
                encoding,
                Format::Code,
                file_format,
            )
            .unwrap();
            let at = document
                .projection()
                .hard_line_range(lines / 2)
                .unwrap()
                .start
                + 1;
            let stable_suffix = document.projection().blocks()[lines - 10].id;
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at, "\n")],
                })
                .unwrap();
            let work = prepared.summary().projection_work();
            assert_eq!(
                work.scope(),
                ProjectionWorkScope::RegionalHardLines,
                "{encoding:?} {lines}: {work:?}"
            );
            assert_eq!(work.full_text_bytes_materialized(), 0);
            assert!(work.decoded_source_bytes() < 128, "{work:?}");
            assert!(work.projected_hard_lines() <= 4, "{work:?}");
            assert!(work.persistent_records_copied() < 4096, "{work:?}");
            if let Some(prior_nodes) = prior_work {
                assert!(
                    work.persistent_nodes_visited() < prior_nodes + 2048,
                    "{work:?}"
                );
            }
            prior_work = Some(work.persistent_nodes_visited());
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.projection().hard_line_count(), lines + 2);
            assert_eq!(document.projection().blocks()[lines - 9].id, stable_suffix);
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at + 1, "")],
                })
                .unwrap();
            let work = prepared.summary().projection_work();
            assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
            assert!(work.decoded_source_bytes() < 128, "{work:?}");
            assert_eq!(work.full_text_bytes_materialized(), 0);
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.projection().hard_line_count(), lines + 1);
            assert_eq!(document.projection().blocks()[lines - 10].id, stable_suffix);
        }
    }
}

#[test]
fn code_newline_edges_unicode_and_existing_highlights_match_a_fresh_projection() {
    use std::sync::Arc;
    use viem_core::document::{
        code_style,
        syntax::{SyntaxRun, SyntaxStyleName},
    };
    for (encoding, ending) in [(Encoding::Utf8, "\n"), (Encoding::Utf16Le, "\r\n")] {
        for logical in ["", "a", "a\n", "\na", "a\nb", "a\u{301}\n😀\n"] {
            let original = encoded(&logical.replace('\n', ending), encoding);
            let fixture =
                || Document::from_bytes(original.clone(), encoding, Format::Code).unwrap();
            let points = (0..=logical.len())
                .filter(|at| fixture().text_point(*at).is_ok())
                .collect::<Vec<_>>();
            let mut edits = points
                .into_iter()
                .map(|at| TextEdit::new(at..at, "\n"))
                .collect::<Vec<_>>();
            edits.extend(
                logical
                    .match_indices('\n')
                    .map(|(at, _)| TextEdit::new(at..at + 1, "")),
            );
            for edit in edits {
                let mut document = fixture();
                document.install_code_presentation(
                    Arc::new(code_style::default_sheet()),
                    &[SyntaxRun {
                        range: 0..logical.len(),
                        name: SyntaxStyleName("Keyword".into()),
                        origin: "test".into(),
                        priority: 0,
                    }],
                );
                let mut expected = logical.to_owned();
                expected.replace_range(edit.range.clone(), &edit.replacement);
                let prepared = document
                    .prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![edit.clone()],
                    })
                    .unwrap_or_else(|error| panic!("{encoding:?} {logical:?} {edit:?}: {error:?}"));
                assert_eq!(
                    prepared.summary().projection_work().scope(),
                    ProjectionWorkScope::RegionalHardLines,
                    "{logical:?} {edit:?}"
                );
                assert_eq!(
                    prepared
                        .summary()
                        .projection_work()
                        .full_text_bytes_materialized(),
                    0
                );
                document.commit_model_transaction(prepared).unwrap();
                assert_eq!(
                    document.text(),
                    expected,
                    "{encoding:?} {logical:?} {edit:?}"
                );
                assert!(
                    document.projection().style_spans().is_empty(),
                    "Old automatic spans cannot enter the authoritative edited projection"
                );
                let fresh =
                    Document::from_bytes(document.source_bytes(), encoding, Format::Code).unwrap();
                assert_eq!(document.text(), fresh.text());
                // Regional edits can split compact mapping runs differently
                // from cold construction; compare their exact relation.
                for source in 0..=document.source_byte_len() {
                    for affinity in [
                        viem_core::document::BoundaryAffinity::Upstream,
                        viem_core::document::BoundaryAffinity::Downstream,
                    ] {
                        let lookup = |document: &Document| {
                            document
                                .projection()
                                .map_source_boundary(document.revision(), source, affinity)
                                .map(|point| (point.formatted_offset, point.affinity, point.relation))
                        };
                        assert_eq!(
                            lookup(&document),
                            lookup(&fresh),
                            "source={source}, {affinity:?}"
                        );
                    }
                }
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }
}

#[test]
fn disjoint_code_batches_keep_work_local_and_adjacent_grapheme_edits_atomic() {
    use viem_core::document::{Association, BoundaryAffinity, DeletionRecovery};
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let logical = format!("a\n{}z", "middle\n".repeat(100_000));
        let physical = if encoding == Encoding::Utf16Le {
            logical.replace('\n', "\r\n")
        } else {
            logical.clone()
        };
        let original = encoded(&physical, encoding);
        let mut document = Document::from_bytes(original.clone(), encoding, Format::Code).unwrap();
        let anchor = document
            .text_anchor(
                document.text_point(500).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();
        let end = logical.len();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![
                    TextEdit::new(0..1, "😀"),
                    TextEdit::new(1..1, "\u{301}\n"),
                    TextEdit::new(end - 1..end, "last\n"),
                ],
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert!(work.decoded_source_bytes() < 256, "{encoding:?}: {work:?}");
        assert_eq!(work.full_text_bytes_materialized(), 0);
        assert!(work.projected_hard_lines() <= 8, "{work:?}");
        assert!(work.persistent_records_copied() < 4096, "{work:?}");
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document.text(),
            format!("😀\u{301}\n\n{}last\n", "middle\n".repeat(100_000))
        );
        assert!(document
            .resolve_text_anchor(anchor)
            .unwrap()
            .value()
            .is_some());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        assert!(!document.undo(), "One batch is one document undo step");
    }
}

#[test]
fn syntax_winning_unknown_names_and_grapheme_start_ownership_remain_presentation_only() {
    use std::sync::Arc;
    use viem_core::document::{
        code_style,
        syntax::{SyntaxRun, SyntaxStyleName},
    };
    use viem_core::layout::DocumentLayoutStyles;
    let mut document = Document::from_bytes(
        "a\u{301}😀x".as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Code,
    )
    .unwrap();
    let original_revision = document.revision();
    let run = |range, name: &str| SyntaxRun {
        range,
        name: SyntaxStyleName(name.into()),
        origin: "test".into(),
        priority: 0,
    };
    let sheet = Arc::new(code_style::default_sheet());
    document.install_code_presentation(
        sheet.clone(),
        &[
            run(0..1, "Keyword"),
            run(1..3, "String"),
            run(3..7, "Missing"),
        ],
    );
    let spans = document.projection().style_spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        spans[0].range,
        0..3,
        "The style at the grapheme start owns the whole combining cluster"
    );
    assert!(
        !DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap()
            .foreground_is_default
    );
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 3, false)
            .unwrap()
            .foreground_is_default
    );
    document.install_code_presentation(sheet, &[run(0..3, "Missing")]);
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap()
            .foreground_is_default,
        "An unknown winning style clears the prior color"
    );
    assert_eq!(document.revision(), original_revision);
    assert!(!document.is_dirty());
    assert_eq!(document.source_bytes(), "a\u{301}😀x".as_bytes());
    assert!(!document.undo());
}

#[test]
fn newly_recognized_files_open_as_code_without_changing_source_or_explicit_format() {
    let source = b"literal 'text'\r\nsecond line\r\n";
    for (filename, language) in [
        ("build.bat", "dosbatch"), ("build.BaT", "dosbatch"), ("build.CMD", "dosbatch"),
        ("profile.PS1", "ps1"), ("main.kt", "kotlin"), ("main.scss", "scss"),
        ("module.cmake.in", "cmake"), ("meson.build", "meson"), (".gitignore", "gitignore"),
    ] {
        for (format, automatic, expected_format) in [
            (Format::PlainText, true, Format::Code),
            (Format::PlainText, false, Format::PlainText),
            (Format::MarkdownSource, true, Format::MarkdownSource),
        ] {
            let document = Document::from_bytes_with_file_format(
                source.to_vec(), Encoding::Utf8, format, FileFormat::Dos,
            ).unwrap();
            let revision = document.revision();
            let mut core = Core::<MockTextMeasurementProvider>::new(document);
            core.initialize_code_detection(filename, automatic).unwrap();
            assert_eq!(core.document().format(), expected_format, "{filename}");
            assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some(language), "{filename}");
            assert_eq!(core.document().source_bytes(), source);
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().file_format(), FileFormat::Dos);
            assert!(!core.document().is_dirty());
        }
    }
}

#[test]
fn detected_batch_language_highlights_bundled_commands_without_changing_source() {
    use viem_core::document::syntax::{
        vim::{VimBudget, VimLoadLimits, VimProgram, VimSession, VimSetupContext},
        Coverage, SyntaxInputIdentity, SyntaxInputSnapshot,
    };
    use viem_core::document::FormattedTextTree;

    let source = concat!(
        "@ECHO OFF\r\n",
        "REM Build comment\r\n",
        "SET NAME=world\r\n",
        "IF defined NAME (\r\n",
        "  echo \"Hello %NAME%\"\r\n",
        ")\r\n",
        "FOR %%F in (*.txt) do CALL :process %%F\r\n",
        "GOTO :done\r\n",
        ":process\r\n",
        "EXIT /b 0\r\n",
        ":done\r\n",
    );
    let document = Document::from_bytes_with_file_format(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    core.initialize_code_detection("build.bat", true).unwrap();
    let language = core.code_language_detection().unwrap().language.as_deref().unwrap();
    assert_eq!(language, "dosbatch");
    assert_eq!(core.document().format(), Format::Code);

    let text = core.document().text();
    let input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 1, revision: 1, generation: 1 },
        FormattedTextTree::try_from_text(text).unwrap(),
    );
    let mut context = VimSetupContext::from_input(&input);
    context.filename = Some("build.bat".into());
    let program = VimProgram::load_directory_with_context(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vim/runtime/syntax"),
        language,
        VimLoadLimits::default(),
        &context,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let mut session = VimSession::new(program);
    let budget = VimBudget { allow_provisional: false, ..Default::default() };
    let result = (0..256)
        .find_map(|_| {
            let result = session.highlight(&input, 0..text.len(), budget);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert!(result.stats.instructions <= budget.instructions);
            (!result.stats.yielded).then_some(result)
        })
        .expect("batch syntax must complete within bounded slices");
    assert_eq!(result.coverage, Coverage::Exact);
    assert_eq!(result.covered, 0..text.len());

    for (needle, expected) in [
        ("ECHO", "Function"),
        ("OFF", "Operator"),
        ("Build comment", "Comment"),
        ("SET", "Function"),
        ("IF", "Conditional"),
        ("Hello", "String"),
        ("%NAME%", "Identifier"),
        ("FOR", "Repeat"),
        ("%%F", "Identifier"),
        ("CALL", "Statement"),
        ("GOTO", "Statement"),
        (":done", "Label"),
        ("EXIT", "Statement"),
    ] {
        let start = text.find(needle).unwrap();
        for at in start..start + needle.len() {
            let name = result.runs.iter().find(|run| run.range.contains(&at))
                .map(|run| run.name.as_str());
            assert_eq!(name, Some(expected), "{needle} at byte {at}");
        }
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}
