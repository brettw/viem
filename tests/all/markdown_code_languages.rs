use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
use viem_core::document::{
    syntax::{
        service::{SyntaxProvider, SyntaxRequest, SyntaxResult},
        Coverage, SyntaxRun, SyntaxStyleName,
    },
    Encoding, FileFormat, Format, StyleApplication, StyleId,
};
use viem_core::layout::{DecorationKind, MockTextMeasurementProvider};
use viem_core::{Core, Document};

#[test]
fn language_mutation_is_local_verified_undoable_and_snapshot_checked() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let source = "before\r\n\r\n> ~~~~  js extra\r\n> const value = 1;\r\n> ~~~~\r\n\r\nafter";
        let bytes = if encoding == Encoding::Utf16Le {
            source.encode_utf16().flat_map(u16::to_le_bytes).collect()
        } else {
            source.as_bytes().to_vec()
        };
        let mut document = Document::from_bytes_with_file_format(
            bytes.clone(),
            encoding,
            Format::Markdown,
            FileFormat::Dos,
        )
        .unwrap();
        let at = document.text().find("const").unwrap();
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .find(|block| block.range.contains(&at))
                .unwrap()
                .code_language
                .as_deref(),
            Some("javascript")
        );
        let revision = document.revision();
        let prepared = document
            .prepare_code_block_language(document.id(), revision, at, Some("rust"))
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        let expected = source.replace("js extra", "rust extra");
        let expected = if encoding == Encoding::Utf16Le {
            expected
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        } else {
            expected.into_bytes()
        };
        assert_eq!(document.source_bytes(), expected);
        assert!(document
            .prepare_code_block_language(document.id(), revision, at, None)
            .is_err());
        let prepared = document
            .prepare_code_block_language(document.id(), document.revision(), at, None)
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        let current = document.source_bytes();
        let reopened = Document::from_bytes_with_file_format(
            current,
            encoding,
            Format::Markdown,
            FileFormat::Dos,
        )
        .unwrap();
        assert_eq!(reopened.text(), document.text());
        assert_eq!(
            reopened
                .projection()
                .blocks()
                .iter()
                .find(|block| block.range.contains(&at))
                .unwrap()
                .code_language,
            None
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), expected);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected);
    }
}

#[test]
fn first_code_block_has_none_and_new_blocks_inherit_nearest_existing_language() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (source, needle, expected) in [
            ("first", "first", "```\nfirst\n```"),
            (
                "first\n\n```python\nx\n```",
                "first",
                "```python\nfirst\n```",
            ),
            (
                "```rust\nx\n```\n\nsecond\n\n```python\ny\n```",
                "second",
                "```rust\nsecond\n```",
            ),
            (
                "```\nx\n```\n\nsecond\n\n```python\ny\n```",
                "second",
                "```\nsecond\n```",
            ),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let at = document.text().find(needle).unwrap();
            document
                .set_paragraph_style(at..at, StyleId::from("Code Block"))
                .unwrap();
            assert!(String::from_utf8(document.source_bytes())
                .unwrap()
                .contains(expected));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn changing_code_language_preserves_scrolled_viewports_with_offscreen_carets() {
    use viem_core::CoreEvent;

    let source = format!(
        "{}\n\n```\n{}```\n\n{}",
        format!("before {}\n\n", "long text ".repeat(80)).repeat(100),
        format!("let value = 1; // {}\n", "long code ".repeat(80)).repeat(12),
        format!("after {}\n\n", "long text ".repeat(80)).repeat(100)
    );
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    // Language-specific Code metrics intentionally differ from the unhighlighted
    // block, so viewport preservation covers the resulting layout refresh too.
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let code = defaults["block_styles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|style| style["id"] == "Code Block")
        .unwrap();
    code["character"]["size"] = 36.into();
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let at = document.text().find("let value").unwrap();
    let code_line = document.physical_line_index_at_source(source.find("let value").unwrap()).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut core = Core::new(document);
    core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
    let active = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    let other = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    let on_screen = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    for view in [active, other, on_screen] {
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
    }
    core.handle(
        active,
        CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: code_line as u64 + 1,
        },
    )
    .unwrap();
    let row = core
        .layout(active)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.text_range.contains(&at))
        .unwrap();
    let code_top = row.y - 60.;
    core.handle(
        active,
        CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: 1,
        },
    )
    .unwrap();
    for (view, left, top) in [(active, 27., code_top), (other, 41., code_top / 2.)] {
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left,
                top: Some(top),
            },
        )
        .unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert!(core.layout(view).unwrap().viewport_top() > 100.);
        assert_eq!(core.layout(view).unwrap().viewport_left(), left);
    }
    core.handle(
        on_screen,
        CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: code_line as u64 + 1,
        },
    )
    .unwrap();
    let body_top = core
        .layout(on_screen)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.text_range.contains(&at))
        .unwrap()
        .y;
    core.handle(
        on_screen,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(body_top - 60.),
        },
    )
    .unwrap();
    assert_eq!(core.command_state(on_screen).unwrap().cursor(), at);
    let before = [active, other, on_screen].map(|view| core.viewport_state(view).unwrap());
    let assert_unchanged = |core: &Core<MockTextMeasurementProvider>| {
        for (view, old) in [active, other, on_screen].into_iter().zip(&before) {
            let actual = core.viewport_state(view).unwrap();
            assert_eq!(
                actual.top(),
                old.top(),
                "language choice must preserve vertical scroll"
            );
            assert_eq!(
                actual.left(),
                old.left(),
                "language choice must preserve horizontal scroll"
            );
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                if view == on_screen { at } else { 0 }
            );
            assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 100);
        }
    };
    for language in [Some("rust"), None] {
        let outcome = core
            .set_markdown_code_language(
                active,
                core.document().id(),
                core.document().revision(),
                at,
                language,
            )
            .unwrap();
        assert!(outcome.document_changed);
        assert_unchanged(&core);
        if language.is_some() {
            wait_for_fixture_syntax(&mut core, &calls, at);
        } else {
            core.poll_syntax();
        }
        for view in [active, other, on_screen] {
            core.handle(
                view,
                CoreEvent::Resize {
                    width: 500.,
                    height: 180.,
                },
            )
            .unwrap();
        }
        assert_unchanged(&core);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(
        core.document().is_dirty(),
        "both language choices retain ordinary undo history"
    );
}

#[test]
fn language_change_retains_wrapped_prefix_heights_and_visible_scroll_coverage() {
    use viem_core::CoreEvent;

    let prefix = "A long preceding paragraph wraps over several rows in this view.\n\n".repeat(32);
    for (source, needle, width, height) in [
        (
            format!("{prefix}~~~\nint value = 1;\n~~~\n\nafter"),
            "int value",
            120.,
            180.,
        ),
        (
            format!("{prefix}    int value = 1;\n\nafter"),
            "int value",
            120.,
            180.,
        ),
        (
            include_str!("../../docs/markdown_demo.md").to_owned(),
            "Four spaces start",
            80.,
            350.,
        ),
    ] {
        let original = source.as_bytes().to_vec();
        let mut core = Core::new(
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap(),
        );
        core.configure_syntax(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/vim/runtime/syntax"
        ));
        let view = core.add_view(MockTextMeasurementProvider::new(), width, height);
        let at = core.document().text().find(needle).unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: viem_core::document::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let code_top = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.text_range.contains(&at))
            .unwrap()
            .y;
        core.handle(
            view,
            CoreEvent::GoToLine {
                document: core.document().id(),
                revision: core.document().revision(),
                line: 1,
            },
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(code_top - 60.),
            },
        )
        .unwrap();
        let origin = core.layout(view).unwrap().viewport_top();
        let visible_rows = |core: &Core<MockTextMeasurementProvider>| {
            let layout = core.layout(view).unwrap();
            layout
                .snapshot()
                .unwrap()
                .rows
                .iter()
                .filter(|row| {
                    row.y + row.height() >= layout.viewport_top()
                        && row.y <= layout.viewport_top() + layout.height()
                })
                .count()
        };
        assert!(visible_rows(&core) > 0);
        core.set_markdown_code_language(
            view,
            core.document().id(),
            core.document().revision(),
            at,
            Some("c"),
        )
        .unwrap();
        for _ in 0..30 {
            core.poll_syntax();
            core.handle(view, CoreEvent::Resize { width, height })
                .unwrap();
            assert_eq!(core.layout(view).unwrap().viewport_top(), origin);
            assert!(core
                .viewport_state(view)
                .unwrap()
                .layout_revision()
                .is_some());
            assert!(
                visible_rows(&core) > 0,
                "language changes must not strand the viewport beyond estimated document end"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        for top in [origin - height, origin] {
            core.handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 0.,
                    top: Some(top),
                },
            )
            .unwrap();
            assert!(visible_rows(&core) > 0);
        }
        let changed = core.document().source_bytes();
        for (navigation, expected) in [
            (
                viem_core::document::HistoryNavigationRequest::Undo,
                &original,
            ),
            (
                viem_core::document::HistoryNavigationRequest::Redo,
                &changed,
            ),
        ] {
            core.handle(view, CoreEvent::NavigateHistory(navigation))
                .unwrap();
            assert_eq!(core.document().source_bytes(), *expected);
            core.handle(
                view,
                CoreEvent::SetViewportOrigin {
                    left: 0.,
                    top: Some(origin),
                },
            )
            .unwrap();
            assert_eq!(core.layout(view).unwrap().viewport_top(), origin);
            assert!(visible_rows(&core) > 0);
        }
    }
}

#[test]
fn shrinking_final_code_block_keeps_scroll_origin_and_real_blank_tail() {
    use viem_core::CoreEvent;

    let source = format!(
        "{}```\nlet first = 1;\nlet second = 2;\n```",
        "before\n\n".repeat(10)
    );
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let code = defaults["block_styles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|style| style["id"] == "Code Block")
        .unwrap();
    code["character"]["size"] = 36.into();
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let at = document.text().find("let first").unwrap();
    let line = document.physical_line_index_at_source(source.find("let first").unwrap()).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut core = Core::new(document);
    core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    core.handle(
        view,
        CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: line as u64 + 1,
        },
    )
    .unwrap();
    let label = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .flat_map(|row| &row.decorations)
        .find(|item| item.kind == DecorationKind::CodeLanguage)
        .unwrap();
    let requested_top = (label.typographic_bounds.y - 5.).max(0.);
    core.handle(
        view,
        CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: 1,
        },
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(requested_top),
        },
    )
    .unwrap();
    let before = core.viewport_state(view).unwrap();
    assert!(before.top() > 0.);
    core.set_markdown_code_language(
        view,
        core.document().id(),
        core.document().revision(),
        at,
        Some("rust"),
    )
    .unwrap();
    assert_eq!(core.viewport_state(view).unwrap().top(), before.top());
    wait_for_fixture_syntax(&mut core, &calls, at);
    core.handle(
        view,
        CoreEvent::Resize {
            width: 500.,
            height: 180.,
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(
        snapshot.maximum_viewport_top(180.).unwrap() < before.top(),
        "the fixture must shrink beyond the ordinary end-of-document scroll clamp"
    );
    assert_eq!(core.viewport_state(view).unwrap().top(), before.top());
    assert!(core.viewport_state(view).unwrap().maximum_top().unwrap() >= before.top());
    assert_eq!(
        snapshot
            .rows
            .iter()
            .filter(|row| row.text_range.start >= at)
            .count(),
        2,
        "retained blank tail must not manufacture text or caret rows"
    );
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(
        core.document().source_bytes(),
        source.replace("```\n", "```rust\n").as_bytes()
    );
}

#[test]
fn source_language_metadata_includes_code_closing_lines() {
    for (source, ending) in [
        ("```rust\ncode\n```\n\nplain", "```"),
        ("> ```rust\n> code\n> ```\n\nplain", "```"),
        ("```rust\n```\n\nplain", "```"),
        ("<pre lang=rust>code\n</pre>\n\nplain", "</pre>"),
        ("> <pre lang='rust'>\n> code\n> </pre>\n\nplain", "</pre>"),
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let closing = document.text().rfind(ending).unwrap();
        let owner = document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&closing))
            .unwrap();
        assert_eq!(owner.style.0, "Code Block", "{source}");
        assert_eq!(owner.code_language.as_deref(), Some("rust"), "{source}");
    }
    let source = "<div>\n<p>before</p>\n<pre lang=rust>fn main() {}</pre>\n<p>after</p>\n</div>";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        for (needle, language) in [("before", None), ("fn main", Some("rust")), ("after", None)] {
            let at = document.text().find(needle).unwrap();
            let owner = document.projection().blocks().iter().find(|block| block.range.contains(&at)).unwrap();
            assert_eq!(owner.code_language.as_deref(), language, "{needle}, {format:?}");
            assert_eq!(owner.style.0 == "Code Block", language.is_some(), "{needle}, {format:?}");
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.is_dirty());
        let unclosed = "<pre lang=rust>code\n";
        let document = Document::from_bytes(unclosed.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let final_owner = document.projection().blocks().last().unwrap();
        assert_eq!(final_owner.style.0, "Code Block", "terminal pre row, {format:?}");
        assert_eq!(final_owner.code_language.as_deref(), Some("rust"), "terminal pre row, {format:?}");
        assert!(document.text().ends_with('\n'));
        assert_eq!(document.source_bytes(), unclosed.as_bytes());
    }
}

#[test]
fn language_labels_are_nontext_furniture_inside_code_border_including_empty_blocks() {
    for (source, compact) in [
        ("```rust\nlet x = 1;\n```", false),
        ("```\n```", false),
        ("> ```python\n> x\n> ```", false),
        ("<pre lang=rust>let x = 1;</pre>", false),
        ("<pre></pre>", false),
        ("```\nx\n```", true),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        if compact {
            let mut defaults: serde_json::Value = serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
            let code = defaults["block_styles"].as_array_mut().unwrap().iter_mut()
                .find(|style| style["id"] == "Code Block").unwrap();
            code["character"]["size"] = 2.into();
            for property in ["padding_top", "padding_bottom", "border_top_width", "border_bottom_width"] {
                code["block"][property] = 0.into();
            }
            document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap();
        }
        let text = document.text().to_owned();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 200.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let labels = snapshot
            .rows
            .iter()
            .flat_map(|row| &row.decorations)
            .filter(|item| item.kind == DecorationKind::CodeLanguage)
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 1, "{source}");
        assert!(labels[0].text.ends_with(" ▾"));
        let row = snapshot
            .rows
            .iter()
            .find(|row| {
                row.decorations
                    .iter()
                    .any(|item| item.kind == DecorationKind::CodeLanguage)
            })
            .unwrap();
        if compact {
            assert!(labels[0].typographic_bounds.y + labels[0].typographic_bounds.height <= row.y + row.line_advance);
            assert!(row.line_advance < 16., "the fixture must be too short for a full selector");
        } else {
            assert!(labels[0].typographic_bounds.y < row.y + row.line_advance);
            assert!(labels[0].typographic_bounds.y + labels[0].typographic_bounds.height > row.y);
        }
        let mut styles = viem_core::layout::DocumentLayoutStyles::resolve(core.document().projection()).unwrap();
        let mut engine = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut layout = viem_core::layout::ViewLayout::new(500., 200.);
        engine.relayout_styled_text(core.document().id(), core.document().revision(), &text,
            styles.clone(), &mut layout).unwrap();
        let reference = layout.snapshot().unwrap().clone();
        for paragraph in &mut styles.paragraphs {
            paragraph.block_box.language_label = None;
            for container in Arc::make_mut(&mut paragraph.containers) { container.style.language_label = None; }
        }
        engine.relayout_styled_text(core.document().id(), core.document().revision(), &text,
            styles, &mut layout).unwrap();
        let geometry = |snapshot: &viem_core::layout::LayoutSnapshot| {
            let mut rows = snapshot.rows.as_ref().clone();
            for row in &mut rows {
                row.decorations.retain(|item| item.kind != DecorationKind::CodeLanguage);
                for caret in &mut row.carets { caret.point.layout_revision = reference.revision; }
            }
            (rows, snapshot.total_height, snapshot.content_width)
        };
        assert_eq!(geometry(&reference), geometry(layout.snapshot().unwrap()),
            "selector presence cannot move glyphs, caret stops, borders or document extents: {source}");
        let styles = viem_core::layout::DocumentLayoutStyles::resolve(core.document().projection()).unwrap();
        engine.relayout_styled_text(core.document().id(), core.document().revision(), &text,
            styles, &mut layout).unwrap();
        assert_eq!(geometry(&reference), geometry(layout.snapshot().unwrap()),
            "restoring selector presentation after cache invalidation retains exact geometry");
        assert_eq!(layout.snapshot().unwrap().rows.iter().flat_map(|row| &row.decorations)
            .filter(|item| item.kind == DecorationKind::CodeLanguage).count(), 1);
        assert_eq!(core.document().text(), text);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(!core.document().is_dirty());
    }
    for source in ["```rust\nx\n```", "<pre lang=rust>x</pre>"] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 200.);
        assert!(!core.layout(view).unwrap().snapshot().unwrap().rows.iter()
            .flat_map(|row| &row.decorations)
            .any(|item| item.kind == DecorationKind::CodeLanguage), "{source}");
    }
}

struct FixtureSyntax(Arc<Mutex<Vec<(String, usize, std::ops::Range<usize>)>>>);
impl SyntaxProvider for FixtureSyntax {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        self.0.lock().unwrap().push((
            request.configuration.language.clone().unwrap(),
            request.input.byte_len(),
            request.range.clone(),
        ));
        // Recording a request is not publication. Keep those phases apart so
        // completion predicates cannot accidentally rely on a fast worker.
        std::thread::sleep(Duration::from_millis(5));
        SyntaxResult {
            input: request.input.identity(),
            configuration: request.configuration.clone(),
            range: request.range.clone(),
            runs: vec![SyntaxRun {
                range: request.range.clone(),
                name: SyntaxStyleName("Statement".into()),
                origin: "fixture".into(),
                priority: 0,
            }],
            coverage: Coverage::Exact,
            diagnostics: Vec::new(),
            continuation: false,
        }
    }
}

struct EdgeSyntax(FixtureSyntax);
impl SyntaxProvider for EdgeSyntax {
    fn analyze(&mut self, request: &SyntaxRequest, cancellation: &AtomicBool) -> SyntaxResult {
        assert_eq!(request.configuration.language.as_deref(), Some("rust"));
        assert_eq!(request.input.slice(0..request.input.byte_len()).unwrap(), "fn main() {}",
            "syntax sees the preformatted body without literal HTML attributes or code wrappers");
        let mut result = self.0.analyze(request, cancellation);
        let mut first = result.runs[0].clone();
        first.range = request.range.start..request.range.start + 2;
        let mut last = first.clone();
        last.range = request.range.end - 2..request.range.end;
        result.runs = vec![first, last];
        result
    }
}

#[test]
fn first_code_token_uses_its_syntax_style_over_the_full_body_base() {
    for source in [
        "```rust\nfn main() {}\n```",
        "<pre lang=\"rust\">fn main() {}</pre>",
        "<pre lang=\"rust\"><code>fn main() {}</code></pre>",
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let mut core = Core::new(Document::from_bytes(
            source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown,
        ).unwrap());
        core.set_syntax_provider_factory(Arc::new(move || {
            Box::new(EdgeSyntax(FixtureSyntax(captured.clone())))
        }));
        core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
        let tail = core.document().text().len() - 1;
        wait_for_fixture_syntax(&mut core, &calls, tail);
        let first = viem_core::layout::DocumentLayoutStyles::character_at(
            core.document().projection(), 0, false,
        ).unwrap();
        let last = viem_core::layout::DocumentLayoutStyles::character_at(
            core.document().projection(), tail, false,
        ).unwrap();
        assert_eq!(first.foreground, last.foreground,
            "a leading token must receive the same accepted syntax style as a trailing token: {source}");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(!core.document().is_dirty());
    }
}

fn wait_for_fixture_syntax(
    core: &mut Core<MockTextMeasurementProvider>,
    calls: &Arc<Mutex<Vec<(String, usize, std::ops::Range<usize>)>>>,
    offset: usize,
) {
    let sheet = viem_core::document::code_style::snapshot();
    let statement = sheet
        .character_styles()
        .find(|style| {
            sheet
                .character_style_metadata(&style.id)
                .is_some_and(|metadata| metadata.display_name == "Statement")
        })
        .unwrap();
    let expected = sheet
        .resolve_assigned_paragraph_style(
            &viem_core::document::DocumentStyleAssignment::new(sheet.base_paragraph.clone()),
            &sheet.base_paragraph,
            &Default::default(),
            &Default::default(),
            Some(&statement.id),
            &Default::default(),
        )
        .unwrap()
        .character;
    let source = viem_core::layout::DocumentLayoutStyles::semantic_character_at(
        core.document().projection(),
        offset,
        false,
    )
    .unwrap();
    let base_foreground = sheet
        .block_style(&sheet.base_paragraph)
        .unwrap()
        .character
        .foreground
        .unwrap_or(source.foreground);
    assert_ne!(
        expected.foreground, base_foreground,
        "the fixture's Statement capture must distinguish an accepted result from base code paint"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        core.poll_syntax();
        let actual = viem_core::layout::DocumentLayoutStyles::character_at(
            core.document().projection(),
            offset,
            false,
        )
        .unwrap();
        if !calls.lock().unwrap().is_empty()
            && actual.foreground == expected.foreground
            && actual.foreground_is_default == expected.foreground_is_default
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "fixture syntax publication timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn markdown_syntax_uses_code_provider_preserves_prose_and_only_requests_visible_blocks() {
    let source = format!("**prose**\n\n```rust\nlet x = 1;\n```\n\n```\nunhighlighted\n```\n\n{}\n\n```python\nfar_away\n```", "paragraph\n\n".repeat(20_000));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let captured = calls.clone();
    core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    let revision = core.document().revision();
    let defaults = core.document().export_style_defaults().unwrap();
    wait_for_fixture_syntax(&mut core, &calls, 6);
    let calls = calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert!(calls
        .iter()
        .all(|(language, bytes, _)| language == "rust" && *bytes == "let x = 1;".len()));
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
    assert_eq!(core.document().export_style_defaults().unwrap(), defaults);
    assert!(!core
        .document()
        .projection()
        .style_sheet()
        .character_styles()
        .any(|style| style.id.0.starts_with("__code/")));
    assert!(core
        .document()
        .projection()
        .style_sheet()
        .block_style(&StyleId::from("Heading1"))
        .is_some());
    assert!(core
        .document()
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.range == (0..5)
            && matches!(&span.application, StyleApplication::Semantic(_))));
    let plain = core.document().text().find("unhighlighted").unwrap();
    assert!(!core.document().projection().style_spans().iter().any(|span| matches!(&span.application, StyleApplication::Automatic(id) if id.0.starts_with("__code/")) && span.range.contains(&plain)));
    assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 200);
}

#[test]
fn language_assignment_supports_empty_and_indented_code_bodies() {
    for source in [
        "```\n```",
        "    code\n    next",
        "> ```\n> ```",
        "- ```\n  code\n  ```",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let block = document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.style.0 == "Code Block")
            .unwrap()
            .clone();
        let text = document.text().to_owned();
        let prepared = document
            .prepare_code_block_language(
                document.id(),
                document.revision(),
                block.range.start,
                Some("rust"),
            )
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), text, "{source}");
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .find(|block| block.style.0 == "Code Block")
                .unwrap()
                .code_language
                .as_deref(),
            Some("rust")
        );
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(reopened.text(), text, "{source}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn passive_html_language_edits_preserve_attribute_spelling_body_and_history() {
    fn encode(source: &str, encoding: Encoding) -> Vec<u8> {
        if encoding == Encoding::Utf16Le {
            source.encode_utf16().flat_map(u16::to_le_bytes).collect()
        } else {
            source.as_bytes().to_vec()
        }
    }
    fn language(document: &Document) -> Option<&str> {
        document.projection().blocks().iter()
            .find(|block| block.style.0 == "Code Block")
            .expect("pre must retain its code owner").code_language.as_deref()
    }
    let body = "literal &amp; café\r\n    next";
    for (opening, initial, c, rust, none, body) in [
        ("<pre data-keep='x>y'>", None,
            "<pre lang=\"c\" data-keep='x>y'>", "<pre lang=\"rust\" data-keep='x>y'>", "<pre data-keep='x>y'>", body),
        ("<PRE data-k='keep' LANG = 'r&#117;st'>", Some("rust"),
            "<PRE data-k='keep' LANG = 'c'>", "<PRE data-k='keep' LANG = 'rust'>", "<PRE data-k='keep'>", body),
        ("<pre lang=js data-k=\"keep\">", Some("javascript"),
            "<pre lang=c data-k=\"keep\">", "<pre lang=rust data-k=\"keep\">", "<pre data-k=\"keep\">", body),
        ("<pre lang='py' data-k=keep LANG=js>", Some("python"),
            "<pre lang='c' data-k=keep LANG=js>", "<pre lang='rust' data-k=keep LANG=js>", "<pre data-k=keep>", body),
        ("<pre lang=\"custom-lang\" title='keep'>", Some("custom-lang"),
            "<pre lang=\"c\" title='keep'>", "<pre lang=\"rust\" title='keep'>", "<pre title='keep'>", body),
        ("<pre lang=\"\">", None,
            "<pre lang=\"c\">", "<pre lang=\"rust\">", "<pre>", body),
        ("<pre lang=>", None,
            "<pre lang=\"c\">", "<pre lang=\"rust\">", "<pre>", body),
        ("<pre lang=\"\" data-k=keep LANG=rust>", None,
            "<pre lang=\"c\" data-k=keep LANG=rust>", "<pre lang=\"rust\" data-k=keep LANG=rust>", "<pre data-k=keep>", body),
        ("<pre lang data-k='keep'>", None,
            "<pre lang=\"c\" data-k='keep'>", "<pre lang=\"rust\" data-k='keep'>", "<pre data-k='keep'>", body),
        ("<pre\r\n  lang = \"py\"\r\n  data-k='keep'\r\n>", Some("python"),
            "<pre\r\n  lang = \"c\"\r\n  data-k='keep'\r\n>", "<pre\r\n  lang = \"rust\"\r\n  data-k='keep'\r\n>", "<pre\r\n\r\n  data-k='keep'\r\n>", body),
        ("<pre>", None, "<pre lang=\"c\">", "<pre lang=\"rust\">", "<pre>", ""),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let source = |opening: &str| format!("before\r\n\r\n{opening}{body}</pre>\r\n\r\nafter");
            let original = encode(&source(opening), encoding);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(), encoding, Format::Markdown, FileFormat::Dos,
            ).unwrap();
            let text = document.text().to_owned();
            let at = document.projection().blocks().iter()
                .find(|block| block.style.0 == "Code Block").unwrap().range.start;
            let check = |bytes: Vec<u8>, expected: Option<&str>| {
                for format in [Format::Markdown, Format::MarkdownSource] {
                    let reopened = Document::from_bytes_with_file_format(
                        bytes.clone(), encoding, format, FileFormat::Dos,
                    ).unwrap();
                    assert_eq!(language(&reopened), expected, "{opening}, {format:?}, {encoding:?}");
                    assert_eq!(reopened.source_bytes(), bytes);
                    if format == Format::Markdown {
                        assert_eq!(reopened.text(), text, "{opening}");
                    } else if !body.is_empty() {
                        assert!(reopened.text().contains("literal &amp; café"), "{opening}");
                    }
                }
            };
            check(original.clone(), initial);
            if initial.is_none() && opening.contains("lang") {
                let mut empty = Document::from_bytes_with_file_format(
                    original.clone(), encoding, Format::Markdown, FileFormat::Dos,
                ).unwrap();
                let prepared = empty.prepare_code_block_language(empty.id(), empty.revision(), at, None).unwrap();
                empty.commit_model_transaction(prepared).unwrap();
                let cleared = encode(&source(none), encoding);
                assert_eq!(empty.source_bytes(), cleared, "direct None: {opening}");
                check(cleared.clone(), None);
                assert!(empty.undo());
                assert_eq!(empty.source_bytes(), original);
                assert!(empty.redo());
                assert_eq!(empty.source_bytes(), cleared);
            }
            let mut states = vec![original];
            for (expected, opening) in [(Some("c"), c), (Some("rust"), rust), (None, none)] {
                let prepared = document.prepare_code_block_language(
                    document.id(), document.revision(), at, expected,
                ).unwrap();
                document.commit_model_transaction(prepared).unwrap();
                let bytes = encode(&source(opening), encoding);
                assert_eq!(document.source_bytes(), bytes, "{opening}");
                assert_eq!(document.text(), text);
                check(bytes.clone(), expected);
                states.push(bytes);
            }
            for expected in states[..states.len() - 1].iter().rev() {
                assert!(document.undo());
                assert_eq!(&document.source_bytes(), expected);
            }
            for expected in &states[1..] {
                assert!(document.redo());
                assert_eq!(&document.source_bytes(), expected);
            }
        }
    }
    for (source, cleared, may_reject_none) in [
        ("> <pre lang=\"rust\">a\r\n> b</pre>", "> <pre>a\r\n> b</pre>", false),
        ("- <pre lang=\"rust\">a\r\n  b</pre>", "- <pre>a\r\n  b</pre>", false),
        ("> <pre lang =\r\n> 'rust'>a\r\n> b</pre>", "> <pre \r\n> >a\r\n> b</pre>", true),
    ] {
        let mut document = Document::from_bytes_with_file_format(
            source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown, FileFormat::Dos,
        ).unwrap();
        let text = document.text().to_owned();
        let block = document.projection().blocks().iter().find(|block| block.style.0 == "Code Block").unwrap();
        let at = block.range.start;
        let ownership = (block.kind.clone(), block.quote_depth);
        let prepared = document.prepare_code_block_language(document.id(), document.revision(), at, Some("c")).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        let changed = source.replace("rust", "c");
        assert_eq!(document.source_bytes(), changed.as_bytes());
        assert_eq!(document.text(), text);
        let revision = document.revision();
        let dirty = document.is_dirty();
        let prepared = document.prepare_code_block_language(document.id(), revision, at, None);
        let accepted = match prepared {
            Ok(prepared) => {
                document.commit_model_transaction(prepared).unwrap();
                assert_eq!(document.source_bytes(), cleared.as_bytes());
                true
            }
            Err(_) if may_reject_none => {
                assert_eq!(document.source_bytes(), changed.as_bytes());
                assert_eq!(document.revision(), revision);
                assert_eq!(document.is_dirty(), dirty);
                false
            }
            Err(error) => panic!("{source}: {error:?}"),
        };
        assert_eq!(document.text(), text);
        let block = document.projection().blocks().iter().find(|block| block.style.0 == "Code Block").unwrap();
        assert_eq!((block.kind.clone(), block.quote_depth), ownership);
        for format in [Format::Markdown, Format::MarkdownSource] {
            let reopened = Document::from_bytes_with_file_format(
                document.source_bytes(), Encoding::Utf8, format, FileFormat::Dos,
            ).unwrap();
            assert_eq!(language(&reopened), if accepted { None } else { Some("c") });
            if format == Format::Markdown { assert_eq!(reopened.text(), text); }
        }
        if accepted {
            assert!(document.undo());
            assert_eq!(document.source_bytes(), changed.as_bytes());
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed.as_bytes());
        if accepted {
            assert!(document.redo());
            assert_eq!(document.source_bytes(), cleared.as_bytes());
        }
    }
}

#[test]
fn changing_markdown_style_defaults_refreshes_highlighted_code_without_source_or_provider_work() {
    let source = "```rust\nlet value = 1;\n```";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 200.);
    wait_for_fixture_syntax(&mut core, &calls, 0);
    let request_count = calls.lock().unwrap().len();
    let revision = core.document().revision();
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&core.document().export_style_defaults().unwrap()).unwrap();
    let code = defaults["block_styles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|style| style["id"] == "Code Block")
        .unwrap();
    code["character"]["foreground"] =
        serde_json::json!({"red":0.2,"green":0.3,"blue":0.4,"alpha":1.0});
    core.replace_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    assert!(core.poll_syntax());
    core.handle(
        view,
        viem_core::CoreEvent::Resize {
            width: 500.,
            height: 200.,
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let label = snapshot
        .rows
        .iter()
        .flat_map(|row| &row.decorations)
        .find(|item| item.kind == DecorationKind::CodeLanguage)
        .unwrap();
    assert_eq!(
        label.paint.foreground,
        viem_core::document::Color {
            red: 0.2,
            green: 0.3,
            blue: 0.4,
            alpha: 1.0
        }
    );
    assert_eq!(calls.lock().unwrap().len(), request_count);
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}

#[test]
fn giant_code_block_requests_and_layout_remain_regional_and_scrolling_retires_old_metrics() {
    use viem_core::layout::{
        compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
        LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
        LayoutJobId, LayoutJobPriority, LayoutJobRegion, ViewLayout, ViewportLayoutRegion,
    };
    let body = "let x = 1;\n".repeat(100_000);
    let source = format!("```rust\n{body}```\n\n{}", "after the code\n\n".repeat(80));
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let code = defaults["block_styles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|style| style["id"] == "Code Block")
        .unwrap();
    code["character"]["size"] = 36.into();
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut core = Core::new(document);
    core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
    wait_for_fixture_syntax(&mut core, &calls, 0);
    core.handle(
        view,
        viem_core::CoreEvent::Resize {
            width: 500.,
            height: 180.,
        },
    )
    .unwrap();
    {
        let calls = calls.lock().unwrap();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|(language, bytes, range)| language == "rust" && *bytes == body.len() - 1 && range.len() < 4_096),
            "providers receive a shared complete body snapshot and only visible regional requests: {calls:?}");
    }
    let assert_fresh = |core: &Core<MockTextMeasurementProvider>| {
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.coverage.hard_lines().len() < 100);
        let mut fresh_view = ViewLayout::new(500., 180.);
        let mut fresh_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&fresh_engine);
        let request = prepare_layout_job(
            core.document(),
            &mut fresh_view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(
                ViewportLayoutRegion::new(snapshot.coverage.hard_lines(), 0., 180.).unwrap(),
            ),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert!(request.capture_statistics().regional_text_bytes() < 4_096);
        assert!(request.capture_statistics().document_shaping_style_runs() < 10);
        assert_eq!(request.capture_statistics().document_paragraph_styles(), 1);
        let fresh = compute_layout_job(
            &mut fresh_engine,
            &request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        assert!(
            fresh
                .regional_snapshot()
                .work_statistics()
                .segmented_text_bytes()
                < 4_096
        );
        install_layout_job(
            &mut fresh_view,
            LayoutInstallTarget {
                document_id: core.document().id(),
                document_revision: core.document().revision(),
                measurement_environment_id: requirements.measurement_environment_id,
                metrics_generation: requirements.metrics_generation,
            },
            fresh,
        )
        .unwrap();
        let geometry = |row: &viem_core::layout::VisualRow| {
            (
                row.text_range.clone(),
                row.y,
                row.baseline,
                row.width,
                row.ascent,
                row.descent,
            )
        };
        let cached = snapshot.rows.iter().map(geometry).collect::<Vec<_>>();
        let recomputed = fresh_view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .map(geometry)
            .collect::<Vec<_>>();
        assert_eq!(
            cached, recomputed,
            "retained geometry must match a fresh shaping/wrapping cache"
        );
        let labels = |snapshot: &viem_core::layout::LayoutSnapshot| snapshot.rows.iter()
            .flat_map(|row| &row.decorations).filter(|item| item.kind == DecorationKind::CodeLanguage)
            .map(|item| (item.text.clone(), item.typographic_bounds)).collect::<Vec<_>>();
        assert_eq!(labels(snapshot), labels(fresh_view.snapshot().unwrap()),
            "regional cache rebuilds retain the selector overlay at the same code boundary");
    };
    assert_fresh(&core);
    // Retiring the block's visible service also retires its base Code metrics.
    // The old first-line cache must not survive when scrolling back to it.
    let last_line = core.document().physical_source_line_number_count() as u64;
    core.handle(
        view,
        viem_core::CoreEvent::GoToLine {
            document: core.document().id(),
            revision: core.document().revision(),
            line: last_line,
        },
    )
    .unwrap();
    core.poll_syntax();
    let retired = viem_core::layout::DocumentLayoutStyles::resolve_region(
        core.document().projection(),
        0..100,
    )
    .unwrap();
    assert!(
        retired.shaping_runs.iter().all(|run| run.style.size == 36.),
        "retiring an offscreen service restores the declared Code Block metrics"
    );
    core.handle(
        view,
        viem_core::CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(0.),
        },
    )
    .unwrap();
    assert_fresh(&core);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}

#[test]
fn highlighted_code_line_transfers_and_sort_verify_source_without_syntax_decorations() {
    use viem_core::{CoreEvent, document::HistoryNavigationRequest};
    use viem_core::command::{CommandStatus, InputEvent, Key};
    for (command, expected) in [
        (":1t1", "c\nc\na\nb"),
        (":1m2", "a\nc\nb"),
        (":sort", "a\nb\nc"),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let mut core = Core::new(Document::from_bytes(b"c\na\nb".to_vec(), Encoding::Utf8, Format::Code).unwrap());
        core.initialize_code_detection("x.rs", true).unwrap();
        core.set_syntax_provider_factory(Arc::new(move || Box::new(FixtureSyntax(captured.clone()))));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
        wait_for_fixture_syntax(&mut core, &calls, 0);
        assert!(!core.document().projection().style_spans().is_empty());
        for ch in command.chars() {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(ch))).unwrap();
        }
        let output = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert_eq!(output.command.unwrap().status, CommandStatus::Complete, "{command}");
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), b"c\na\nb");
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}

#[test]
fn audit_clearing_language_removes_its_separator_whitespace() {
    for (source, expected) in [
        ("``` {.rust}\nx\n```", "```\nx\n```"),
        ("<pre lang=\"py\">x</pre>", "<pre>x</pre>"),
        ("<pre id=\"keep\"  lang=\"py\">x</pre>", "<pre id=\"keep\">x</pre>"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let prepared = document.prepare_code_block_language(document.id(), document.revision(), 0, None).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}
