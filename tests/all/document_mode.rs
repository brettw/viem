use viem_core::{Core, CoreEvent, Document, DocumentMode, Format, ViewId};
use viem_core::document::{BoundaryAffinity, Encoding, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;

type Editor = Core<MockTextMeasurementProvider>;
fn editor(source: &[u8], format: Format, filename: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap());
    core.initialize_code_detection(filename, true).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    (core, view)
}
fn select(core: &mut Editor, view: ViewId, mode: DocumentMode, formatted: bool) {
    core.handle(view, CoreEvent::SetDocumentMode { document: core.document().id(), revision: core.document().revision(), mode, formatted_markdown: formatted }).unwrap();
}

#[test]
fn all_view_formats_preserve_bytes_dirty_state_shared_cursors_and_history() {
    let bytes = "\u{feff}# Heading\r\n\r\n**café** tail\r\n".as_bytes();
    for from in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        for (mode, target, formatted) in [
            (DocumentMode::PlainText, Format::PlainText, false),
            (DocumentMode::Code("rust".into()), Format::Code, false),
            (DocumentMode::Markdown, Format::MarkdownSource, false),
            (DocumentMode::Markdown, Format::Markdown, true),
        ] {
            let (mut core, first) = editor(bytes, from, "");
            let second = core.add_view(MockTextMeasurementProvider::new(), 180., 90.);
            let at = core.document().text().find("café").unwrap();
            for view in [first, second] {
                core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(), text_offset: at,
                    affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
            }
            select(&mut core, first, mode, formatted);
            assert_eq!(core.document().format(), target, "{from:?} -> {target:?}");
            assert_eq!(core.document().source_bytes(), bytes);
            assert!(!core.document().is_dirty());
            let expected = core.document().text().find("café").unwrap();
            for view in [first, second] { assert_eq!(core.command_state(view).unwrap().cursor(), expected, "{from:?} -> {target:?}"); }
            if from != target {
                core.handle(first, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                assert_eq!(core.document().format(), from);
                assert_eq!(core.document().source_bytes(), bytes);
                core.handle(first, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                assert_eq!(core.document().format(), target);
            }
        }
    }
}

#[test]
fn overrides_preserve_detection_and_auto_falls_back_to_plain_text() {
    let (mut core, view) = editor(b"text", Format::PlainText, "notes.rs");
    assert_eq!(core.document().format(), Format::Code);
    assert_eq!(core.document_mode_state().detected_name, "Rust");
    select(&mut core, view, DocumentMode::PlainText, false);
    assert_eq!(core.document().format(), Format::PlainText);
    assert!(!core.document_mode_state().automatic);
    select(&mut core, view, DocumentMode::Code("python".into()), false);
    assert_eq!(core.document_mode_state().language.as_deref(), Some("python"));
    assert_eq!(core.document_mode_state().detected_name, "Rust");
    select(&mut core, view, DocumentMode::Automatic, false);
    assert_eq!(core.document().format(), Format::Code);
    assert!(core.document_mode_state().automatic);
    assert_eq!(core.code_language_detection().unwrap().language.as_deref(), Some("rust"));
    core.redetect_code_language(Some("notes.unknown"));
    select(&mut core, view, DocumentMode::Automatic, false);
    assert_eq!(core.document().format(), Format::PlainText);
    assert_eq!(core.document_mode_state().detected_name, "Plain Text");
    assert!(core.document_mode_state().automatic);
}

#[test]
fn code_auto_on_markdown_remains_code_and_top_level_markdown_uses_preference() {
    let (mut core, view) = editor(b"# heading", Format::Markdown, "notes.md");
    select(&mut core, view, DocumentMode::Automatic, true);
    assert_eq!(core.document().format(), Format::Code);
    assert_eq!(core.document().text(), "# heading");
    assert_eq!(core.document_mode_state().detected_name, "Markdown");
    select(&mut core, view, DocumentMode::Markdown, true);
    assert_eq!(core.document().format(), Format::Markdown);
    assert_eq!(core.document().text(), "heading");
    select(&mut core, view, DocumentMode::Markdown, false);
    assert_eq!(core.document().format(), Format::MarkdownSource);
}

#[test]
fn rejected_modes_and_stale_requests_leave_document_and_override_unchanged() {
    let (mut core, view) = editor(b"text", Format::PlainText, "notes.rs");
    let revision = core.document().revision();
    let before = serde_json::to_value(core.document_mode_state()).unwrap();
    assert!(core.handle(view, CoreEvent::SetDocumentMode { document: core.document().id(), revision,
        mode: DocumentMode::Code("not-a-bundled-language".into()), formatted_markdown: false }).is_err());
    assert_eq!(serde_json::to_value(core.document_mode_state()).unwrap(), before);
    select(&mut core, view, DocumentMode::Markdown, false);
    let before = serde_json::to_value(core.document_mode_state()).unwrap();
    assert!(core.handle(view, CoreEvent::SetDocumentMode { document: core.document().id(), revision,
        mode: DocumentMode::Code("rust".into()), formatted_markdown: false }).is_err());
    assert_eq!(serde_json::to_value(core.document_mode_state()).unwrap(), before);
}

#[test]
fn catalogue_is_complete_unique_sorted_and_excludes_runtime_utilities() {
    use viem_core::document::syntax::languages::supported_languages;
    let languages = supported_languages();
    assert!(languages.len() > 600);
    assert!(languages.windows(2).all(|pair| pair[0].name.to_lowercase() <= pair[1].name.to_lowercase()));
    let ids = languages.iter().map(|language| language.id.as_str()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), languages.len());
    for id in ["rust", "swift", "c_sharp", "tsx", "vim", "markdown", "python", "bash"] { assert!(ids.contains(id), "{id}"); }
    for id in ["2html", "syntax", "synload", "manual", "nosyntax", "syncolor"] { assert!(!ids.contains(id), "{id}"); }
}

#[test]
fn auto_detects_bounded_physical_source_in_every_view_and_encoding() {
    use viem_core::document::syntax::detection::DETECTION_BYTE_LIMIT;
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        for (source, language) in [
            ("#!/usr/bin/env **python**\r\n", None),
            ("#!/usr/bin/env python3\r\n", Some("python")),
            ("vim: ft=rust\r\n", Some("rust")),
        ] {
            for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
                let bytes = match encoding {
                    Encoding::Utf8 => source.as_bytes().to_vec(),
                    Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                    Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                    _ => unreachable!(),
                };
                let mut core = Editor::new(Document::from_bytes(bytes.clone(), encoding, format).unwrap());
                core.initialize_code_detection("unknown", false).unwrap();
                let view = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
                select(&mut core, view, DocumentMode::Automatic, false);
                assert_eq!(core.document_mode_state().detected_language.as_deref(), language, "{format:?} {encoding:?} {source}");
                assert_eq!(core.document().format(), if language.is_some() { Format::Code } else { Format::PlainText });
                assert_eq!(core.document().source_bytes(), bytes);
            }
        }
    }
    let source = format!("{} vim: ft=rust\n", "x".repeat(DETECTION_BYTE_LIMIT * 2));
    let (mut core, view) = editor(source.as_bytes(), Format::PlainText, "unknown");
    select(&mut core, view, DocumentMode::Automatic, false);
    assert_eq!(core.document().format(), Format::PlainText);
    assert!(core.code_language_detection().unwrap().bytes_inspected <= DETECTION_BYTE_LIMIT);

    let mut bytes = vec![0xe9; 30_000];
    bytes.push(b'\n');
    bytes.extend(std::iter::repeat_n(0xe9, 30_000));
    let mut core = Editor::new(Document::from_bytes(bytes, Encoding::Latin1, Format::PlainText).unwrap());
    core.initialize_code_detection("unknown", false).unwrap();
    assert!(core.code_language_detection().unwrap().bytes_inspected <= DETECTION_BYTE_LIMIT);
}
