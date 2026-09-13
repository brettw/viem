use std::ops::Range;
use viem_core::document::{Document, DocumentError, Encoding, Format, StyleApplication};
use viem_core::layout::DocumentLayoutStyles;

fn document(text: &str, format: Format) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn link_ranges(document: &Document) -> Vec<Range<usize>> {
    document
        .projection()
        .style_spans()
        .iter()
        .filter(|span| span.application == StyleApplication::Automatic("Link".into()))
        .map(|span| span.range.clone())
        .collect()
}

fn destination(document: &Document, offset: usize) -> Option<String> {
    document
        .link_at(document.text_point(offset).unwrap())
        .unwrap()
}

#[test]
fn markdown_source_styles_the_entire_construct_and_resolves_each_part() {
    let source = "before [café **bold**](https://example.test/a(b)?x=1&amp;y=2 \"title\") after";
    let document = document(source, Format::MarkdownSource);
    let start = source.find('[').unwrap();
    let end = source.find(" after").unwrap();
    assert_eq!(document.text(), source);
    assert_eq!(link_ranges(&document), vec![start..end]);
    for (at, _) in source[start..end].char_indices() {
        assert_eq!(
            destination(&document, start + at).as_deref(),
            Some("https://example.test/a(b)?x=1&y=2"),
            "offset {}",
            start + at
        );
    }
    assert_eq!(destination(&document, start - 1), None);
    assert_eq!(destination(&document, end), None);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn markdown_wysiwyg_shows_only_label_with_link_and_inline_styles() {
    let document = document(
        "before [**bold** and `code`](https://example.test/) after",
        Format::Markdown,
    );
    assert_eq!(document.text(), "before bold and code after");
    assert_eq!(link_ranges(&document), vec![7..20]);
    for offset in 7..20 {
        assert_eq!(
            destination(&document, offset).as_deref(),
            Some("https://example.test/")
        );
    }
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 7, false)
            .unwrap()
            .bold
    );
    assert_eq!(destination(&document, 6), None);
    assert_eq!(destination(&document, 20), None);
}

#[test]
fn html_source_styles_only_anchor_contents_including_nested_markup() {
    let source = "<p>before <a href='https://example.test/?x=1&amp;y=2' title='x'><em>café</em> tail</a> after</p>";
    let document = document(source, Format::HtmlSource);
    let start = source.find("<em>").unwrap();
    let end = source.find("</a>").unwrap();
    assert_eq!(document.text(), source);
    let ranges = link_ranges(&document);
    for (at, _) in source.char_indices() {
        let styled = ranges.iter().any(|range| range.contains(&at));
        assert_eq!(styled, (start..end).contains(&at), "offset {at}");
        assert_eq!(
            destination(&document, at).as_deref(),
            if styled {
                Some("https://example.test/?x=1&y=2")
            } else {
                None
            },
            "offset {at}"
        );
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn html_wysiwyg_resolves_visible_label_with_decoded_attributes() {
    let document = document("<p>before <A HREF='https://example.test/a?x=1&#38;y=2'><b>bold</b> &amp; café</A> after</p>", Format::Html);
    assert_eq!(document.text(), "before bold & café after");
    for at in [7, 8, 12, 14, 17] {
        assert_eq!(
            destination(&document, at).as_deref(),
            Some("https://example.test/a?x=1&y=2"),
            "offset {at}"
        );
    }
    assert_eq!(destination(&document, 6), None);
    assert_eq!(destination(&document, 19), None);
}

#[test]
fn escaped_balanced_and_angle_destinations_are_decoded_without_execution() {
    for (source, expected) in [
        (
            r"[label](https://example.test/a(b(c)))",
            "https://example.test/a(b(c))",
        ),
        (
            r"[label](https://example.test/a\(b\)\*x)",
            "https://example.test/a(b)*x",
        ),
        (
            r"[a\]b](<https://example.test/a b> 'title')",
            "https://example.test/a b",
        ),
        (
            r"[label](https://example.test/?x=1&amp;y=&#50;)",
            "https://example.test/?x=1&y=2",
        ),
        (
            r"[label](<https://example.test/$(touch /tmp/viem-link-injection);`whoami`>)",
            "https://example.test/$(touch /tmp/viem-link-injection);`whoami`",
        ),
    ] {
        for format in [Format::MarkdownSource, Format::Markdown] {
            let document = document(source, format);
            assert_eq!(
                destination(&document, 1).as_deref(),
                Some(expected),
                "{source:?} {format:?}"
            );
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn code_images_escaped_openers_and_invalid_links_remain_literal() {
    for source in [
        "`[label](https://example.test/)`",
        "```\n[label](https://example.test/)\n```",
        "![label](https://example.test/)",
        r"\[label](https://example.test/)",
        "[label](unterminated",
        "[label](bad space)",
        "[label](bad(nesting)",
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let document = document(source, format);
            assert!(link_ranges(&document).is_empty(), "{source:?} {format:?}");
            for (at, _) in document.text().char_indices() {
                assert_eq!(destination(&document, at), None, "{source:?} at {at}");
            }
        }
    }
    for format in [Format::PlainText, Format::Code] {
        let document = document(
            "[label](https://example.test/) <a href='https://example.test/'>label</a>",
            format,
        );
        assert!(link_ranges(&document).is_empty());
        assert_eq!(destination(&document, 3), None);
    }
}

#[test]
fn html_comments_raw_text_and_anchors_without_href_are_not_links() {
    let source = "<p><!-- <a href='https://comment.test/'>fake</a> --><a name='bookmark'>named</a></p><script>\"<a href='https://script.test/'>fake</a>\"</script>";
    for format in [Format::Html, Format::HtmlSource] {
        let document = document(source, format);
        assert!(link_ranges(&document).is_empty(), "{format:?}");
        for (at, _) in document.text().char_indices() {
            assert_eq!(destination(&document, at), None);
        }
    }
}

#[test]
fn links_validate_snapshot_and_document_identity_before_lookup() {
    let mut document = document("[label](https://example.test/)", Format::MarkdownSource);
    let point = document.text_point(1).unwrap();
    document.insert(0, "prefix ").unwrap();
    assert!(matches!(
        document.link_at(point),
        Err(DocumentError::WrongSnapshot { .. })
    ));
    let other = self::document("other", Format::PlainText);
    assert!(matches!(
        document.link_at(other.text_point(0).unwrap()),
        Err(DocumentError::WrongDocument)
    ));
    assert_eq!(
        destination(&document, 8).as_deref(),
        Some("https://example.test/")
    );
    assert_eq!(destination(&document, document.text().len()), None);
}

#[test]
fn link_queries_map_normalized_newlines_and_non_utf8_source_bytes() {
    for format in [
        Format::MarkdownSource,
        Format::Markdown,
        Format::HtmlSource,
        Format::Html,
    ] {
        let source = if format.is_markdown() {
            "é prefix\r\n[café](https://example.test/café?x=1&amp;y=2)"
        } else {
            "<p>é prefix</p>\r\n<p><a href='https://example.test/café?x=1&amp;y=2'>café</a></p>"
        };
        for encoding in [Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes: Vec<u8> = match encoding {
                Encoding::Latin1 => source
                    .chars()
                    .map(|ch| u8::try_from(ch as u32).unwrap())
                    .collect(),
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => unreachable!(),
            };
            let document = Document::from_bytes(bytes.clone(), encoding, format).unwrap();
            let at = document.text().rfind("café").unwrap();
            assert_eq!(
                destination(&document, at).as_deref(),
                Some("https://example.test/café?x=1&y=2"),
                "{format:?} {encoding:?}"
            );
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn malformed_prior_paragraph_cannot_override_the_actual_link_destination() {
    let document = document(
        "[unclosed\n\n[real](https://real.test/)](https://wrong.test/)",
        Format::Markdown,
    );
    let at = document.text().find("real").unwrap();
    assert_eq!(
        destination(&document, at).as_deref(),
        Some("https://real.test/")
    );
}

#[test]
fn link_style_defaults_are_blue_underlined_and_not_applied_to_neighbors() {
    for (source, format) in [
        (
            "before [label](https://example.test/) after",
            Format::MarkdownSource,
        ),
        (
            "before [label](https://example.test/) after",
            Format::Markdown,
        ),
        (
            "before <a href='https://example.test/'>label</a> after",
            Format::HtmlSource,
        ),
        (
            "before <a href='https://example.test/'>label</a> after",
            Format::Html,
        ),
    ] {
        let document = document(source, format);
        let at = document.text().find("label").unwrap();
        let style = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
        assert!(style.underline, "{format:?}");
        assert_eq!(
            (
                style.foreground.red,
                style.foreground.green,
                style.foreground.blue
            ),
            (0.0, 0.0, 1.0),
            "{format:?}"
        );
        assert!(
            !DocumentLayoutStyles::character_at(document.projection(), 0, false)
                .unwrap()
                .underline
        );
    }
}

#[test]
fn changing_destination_in_source_and_undo_refresh_the_on_demand_result() {
    for (source, format) in [
        (
            "before [label](https://old.test/) after",
            Format::MarkdownSource,
        ),
        (
            "before <a href='https://old.test/'>label</a> after",
            Format::HtmlSource,
        ),
    ] {
        let mut document = document(source, format);
        let at = document.text().find("old").unwrap();
        document.replace(at..at + 3, "new").unwrap();
        let label = document.text().find("label").unwrap();
        assert_eq!(
            destination(&document, label).as_deref(),
            Some("https://new.test/")
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            destination(&document, label).as_deref(),
            Some("https://old.test/")
        );
        assert!(document.redo());
        assert_eq!(
            destination(&document, label).as_deref(),
            Some("https://new.test/")
        );
    }
}

#[test]
fn code_delimiters_cannot_create_an_outer_link_during_destination_lookup() {
    for source in [
        "`[fake` [real](https://good.test/)](https://wrong.test/)",
        "```\n[unclosed\n```\n[real](https://good.test/)](https://wrong.test/)",
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let document = document(source, format);
            let at = document.text().find("real").unwrap();
            assert_eq!(
                destination(&document, at).as_deref(),
                Some("https://good.test/"),
                "{source:?} {format:?}"
            );
        }
    }
}

#[test]
fn nested_links_reject_the_outer_construct_but_code_labels_do_not() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in [
            "[outer [inner](https://inner.test/) rest](https://outer.test/)",
            "[[inner](https://inner.test/)](https://outer.test/)",
        ] {
            let document = document(source, format);
            let at = document.text().find("inner").unwrap();
            assert_eq!(
                destination(&document, at).as_deref(),
                Some("https://inner.test/"),
                "{source:?} {format:?}"
            );
            if let Some(at) = document.text().find("outer ") {
                assert_eq!(destination(&document, at).as_deref(), None);
            }
        }
        let document = document(
            "[`[fake](https://wrong.test/)`](https://real.test/)",
            format,
        );
        let at = document.text().find("fake").unwrap();
        assert_eq!(
            destination(&document, at).as_deref(),
            Some("https://real.test/"),
            "{format:?}"
        );
    }
}

#[test]
fn physical_multiline_link_labels_resolve_in_both_markdown_modes() {
    let source = "before [multi\nline](https://example.test/) after";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let document = document(source, format);
        let at = document.text().find("multi").unwrap();
        assert_eq!(
            destination(&document, at).as_deref(),
            Some("https://example.test/"),
            "{format:?}"
        );
        let at = document.text().find("line").unwrap();
        assert_eq!(
            destination(&document, at).as_deref(),
            Some("https://example.test/"),
            "{format:?}"
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn link_paint_change_reuses_shaping_but_font_change_invalidates_it() {
    use viem_core::document::{
        Color, ConfigurationStyleIntent, StyleDefinitionEdit, StyleInvalidationEffect,
        StyleModelIntent, StyleModelRequest,
    };
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let mut document = document(
        "before [label](https://example.test/) after\n\nunchanged paragraph",
        Format::Markdown,
    );
    let source = document.source_bytes();
    let mut view = ViewLayout::new(700.0, 200.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.relayout(&document, &mut view).unwrap();
    let calls = engine.provider().request_calls();
    let geometry = |view: &ViewLayout| {
        view.snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| (row.text_range.clone(), row.y, row.line_advance))
            .collect::<Vec<_>>()
    };
    let before = geometry(&view);
    let mut link = document
        .projection()
        .style_sheet()
        .character_style(&"Link".into())
        .unwrap()
        .clone();
    link.properties.foreground = Some(Color {
        red: 0.8,
        green: 0.2,
        blue: 0.1,
        alpha: 1.0,
    });
    link.properties.underline = Some(false);
    let request = StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
            StyleDefinitionEdit::UpdateCharacter(link.clone()),
        )),
    );
    let change = document.apply_style_request(request).unwrap();
    assert!(change
        .summary()
        .style_change()
        .unwrap()
        .invalidation_effects()
        .contains(&StyleInvalidationEffect::Paint));
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(engine.provider().request_calls(), calls);
    assert_eq!(geometry(&view), before);
    let paint = DocumentLayoutStyles::character_at(document.projection(), 7, false).unwrap();
    assert_eq!(paint.foreground.red, 0.8);
    assert!(!paint.underline);
    link.properties.size = Some(28.0);
    let request = StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
            StyleDefinitionEdit::UpdateCharacter(link),
        )),
    );
    let change = document.apply_style_request(request).unwrap();
    assert!(change
        .summary()
        .style_change()
        .unwrap()
        .invalidation_effects()
        .contains(&StyleInvalidationEffect::Shaping));
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() > calls);
    assert_ne!(geometry(&view), before);
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn large_link_document_captures_and_shapes_only_requested_viewports() {
    use viem_core::layout::{
        compute_layout_job, inspect_layout_provider, prepare_layout_job, LayoutCancellationToken,
        LayoutEngine, LayoutExecutionContext, LayoutJobId, LayoutJobPriority, LayoutJobRegion,
        MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    };
    let source = (0..2_000)
        .map(|line| format!("[link {line}](https://example.test/{line}) prose {line}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let document = document(&source, Format::Markdown);
    assert_eq!(document.projection().hard_line_count(), 2_000);
    let mut view = ViewLayout::new(700.0, 120.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    for (job, range) in [0..4, 1996..2000].into_iter().enumerate() {
        let before = engine.provider().request_calls();
        let request = prepare_layout_job(
            &document,
            &mut view,
            inspect_layout_provider(&engine),
            LayoutJobId(job as u64 + 1),
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(
                ViewportLayoutRegion::new(range.clone(), range.start as f32 * 28.0, 120.0).unwrap(),
            ),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let capture = request.capture_statistics();
        assert!(capture.regional_text_bytes() < 512);
        assert!(capture.document_paragraph_styles() <= 6);
        assert!(capture.document_paint_style_runs() <= 16);
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let region = candidate.regional_snapshot();
        assert_eq!(region.hard_lines(), range);
        assert!(region
            .paint_runs()
            .iter()
            .any(|run| run.paint.underline && run.paint.foreground.blue == 1.0));
        assert!(engine.provider().request_calls() - before <= 8);
    }
    let at = document.text().find("link 1999").unwrap();
    assert_eq!(
        destination(&document, at).as_deref(),
        Some("https://example.test/1999")
    );
}

#[test]
fn authored_html_direct_and_named_styles_override_link_defaults() {
    use viem_core::document::{
        CharacterProperties, CharacterStyle, Color, ModelRequest, PersistedStyleIntent,
        StyleDefinitionEdit, StyleDefinitionMetadata, StyleDefinitionOrigin, StyleModelIntent,
        StyleModelRequest, TextRange,
    };

    // Author the named style through the supported API so the fixture has the
    // same sparse property metadata and class spelling as an actual saved file.
    let mut named = document("<a href='https://example.test/'>label</a>", Format::Html);
    named
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: named.id(),
            revision: named.revision(),
            enabled: true,
        })
        .unwrap();
    let definition = StyleModelRequest::new(
        named.id(),
        named.revision(),
        StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit: StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: "Authored Link".into(),
                    based_on: None,
                    properties: CharacterProperties {
                        foreground: Some(Color {
                            red: 1.0,
                            green: 0.0,
                            blue: 0.0,
                            alpha: 1.0,
                        }),
                        underline: Some(false),
                        ..Default::default()
                    },
                },
                metadata: StyleDefinitionMetadata {
                    display_name: "Authored Link".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            },
        }),
    );
    named.apply_style_request(definition).unwrap();
    let range = TextRange::new(named.text_point(0).unwrap(), named.text_point(5).unwrap()).unwrap();
    let assignment = StyleModelRequest::new(
        named.id(),
        named.revision(),
        StyleModelIntent::Persisted(PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: "Authored Link".into(),
        }),
    );
    named.apply_style_request(assignment).unwrap();
    let named_source = String::from_utf8(named.source_bytes()).unwrap();

    for source in [
        "<a href='https://example.test/' style='color: red; text-decoration: none'>label</a>",
        "<a href='https://example.test/'><span style='color: red; text-decoration: none'>label</span></a>",
        &named_source,
    ] {
        for format in [Format::Html, Format::HtmlSource] {
            let document = document(source, format);
            let at = document.text().find("label").unwrap();
            let style = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
            assert!(!style.underline, "{source:?} {format:?}");
            assert_eq!((style.foreground.red, style.foreground.green, style.foreground.blue), (1.0, 0.0, 0.0), "{source:?} {format:?}");
            assert_eq!(destination(&document, at).as_deref(), Some("https://example.test/"));
        }
    }
}

#[test]
fn deleting_link_style_never_leaves_an_undefined_layout_reference() {
    use viem_core::document::{
        ConfigurationStyleIntent, StyleDefinitionEdit, StyleModelIntent, StyleModelRequest,
    };
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    for (source, format) in [
        ("[label](https://example.test/) tail", Format::Markdown),
        (
            "[label](https://example.test/) tail",
            Format::MarkdownSource,
        ),
        (
            "<p><a href='https://example.test/'>label</a> tail</p>",
            Format::Html,
        ),
        (
            "<p><a href='https://example.test/'>label</a> tail</p>",
            Format::HtmlSource,
        ),
    ] {
        let mut document = document(source, format);
        let request = StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                StyleDefinitionEdit::DeleteCharacter("Link".into()),
            )),
        );
        document.apply_style_request(request).unwrap();
        assert!(
            DocumentLayoutStyles::resolve(document.projection()).is_ok(),
            "{format:?}"
        );
        let mut view = ViewLayout::new(700.0, 120.0);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.undo());
        let at = document.text().find("label").unwrap();
        assert!(
            DocumentLayoutStyles::character_at(document.projection(), at, false)
                .unwrap()
                .underline
        );
        assert!(document.redo());
        let at = document.text().find("tail").unwrap();
        document.replace(at..at + 1, "T").unwrap();
        assert!(
            DocumentLayoutStyles::resolve(document.projection()).is_ok(),
            "after reprojection {format:?}"
        );
        engine.relayout(&document, &mut view).unwrap();
    }
}

#[test]
fn deleted_link_appearance_survives_core_viewport_capture_install_and_edit() {
    use viem_core::command::InputEvent;
    use viem_core::document::{
        BoundaryAffinity, ConfigurationStyleIntent, StyleDefinitionEdit, StyleModelIntent,
        StyleModelRequest,
    };
    use viem_core::layout::{
        compute_layout_job, LayoutCancellationToken, LayoutEngine, LayoutExecutionContext,
        LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider, ViewportLayoutRegion,
    };
    use viem_core::{Core, CoreEvent};

    for (source, format) in [
        ("[label](https://example.test/) tail", Format::Markdown),
        (
            "[label](https://example.test/) tail",
            Format::MarkdownSource,
        ),
        (
            "<p><a href='https://example.test/'>label</a> tail</p>",
            Format::Html,
        ),
        (
            "<p><a href='https://example.test/'>label</a> tail</p>",
            Format::HtmlSource,
        ),
    ] {
        let mut document = document(source, format);
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                    StyleDefinitionEdit::DeleteCharacter("Link".into()),
                )),
            ))
            .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 700.0, 120.0);
        assert!(core
            .document()
            .projection()
            .style_sheet()
            .character_style(&"Link".into())
            .is_none());
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        for edited in [false, true] {
            if edited {
                let tail = core.document().text().find("tail").unwrap();
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: tail,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::text("rT")))
                    .unwrap();
                assert!(core.document().text().contains("Tail"));
            }
            let lines = core.document().projection().hard_line_count();
            let request = core
                .prepare_view_layout_job(
                    view,
                    LayoutJobPriority::ChangedVisibleRows,
                    LayoutJobRegion::Viewport(
                        ViewportLayoutRegion::new(0..lines, 0.0, 120.0).unwrap(),
                    ),
                    LayoutCancellationToken::new(),
                )
                .unwrap_or_else(|error| panic!("capture {format:?} edited={edited}: {error:?}"));
            let candidate =
                compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap_or_else(|error| panic!("layout {format:?} edited={edited}: {error:?}"));
            core.install_view_layout_job(view, candidate).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(!snapshot.rows.is_empty());
            assert_eq!(snapshot.coverage.hard_lines(), 0..lines);
            let label = core.document().text().find("label").unwrap();
            assert_eq!(
                destination(core.document(), label).as_deref(),
                Some("https://example.test/")
            );
        }
    }
}
