use evim_core::command::{InputEvent, Key};
use evim_core::document::{
    BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest, SemanticInlineStyle,
};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent, ViewId};

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| {
            panic!(
                "{:?} {key:?} {error:?}: {:?}",
                core.document().format(),
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
    assert!(matches!(
        output.command.unwrap().status,
        evim_core::command::CommandStatus::Complete | evim_core::command::CommandStatus::Pending
    ));
}
fn style(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    style: SemanticInlineStyle,
    enabled: bool,
) {
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style,
            enabled,
        },
    )
    .unwrap();
}
fn start(
    format: Format,
    source: Vec<u8>,
    encoding: Encoding,
    at: usize,
) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::from_bytes(source, encoding, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    key(&mut core, view, Key::Char('R'));
    let document_revision = core.document().revision();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision,
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    (core, view)
}
fn fixtures() -> [(Format, &'static str, usize); 5] {
    [
        (Format::Html, "<p>word</p><!--keep-->", 0),
        (Format::Rtf, r"{\rtf1 word}{\*\unknown keep}", 0),
        (Format::HtmlSource, "<p>word</p><!--keep-->", 3),
        (Format::Markdown, "word", 0),
        (Format::MarkdownSource, "word", 0),
    ]
}
#[test]
fn styled_replace_backspace_restores_each_original_grapheme_and_exact_source() {
    for (format, source, at) in fixtures() {
        let (mut core, view) = start(format, source.as_bytes().to_vec(), Encoding::Utf8, at);
        style(&mut core, view, SemanticInlineStyle::Emphasis, true);
        key(&mut core, view, Key::Char('a'));
        let after_first = core.document().source_bytes();
        let first_cursor = core.command_state(view).unwrap().cursor();
        key(&mut core, view, Key::Char('b'));
        let wysiwyg_format = match format {
            Format::HtmlSource => Format::Html,
            Format::MarkdownSource => Format::Markdown,
            other => other,
        };
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            wysiwyg_format,
        )
        .unwrap();
        assert_eq!(
            reopened.text(),
            "abrd",
            "{format:?}: source {:?}",
            String::from_utf8_lossy(&core.document().source_bytes())
        );
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().source_bytes(), after_first, "{format:?}");
        let fresh = Document::from_bytes(after_first.clone(), Encoding::Utf8, format).unwrap();
        for offset in 0..fresh.projection().text_tree().byte_len() {
            if fresh.text_point(offset).is_ok() {
                let actual =
                    DocumentLayoutStyles::character_at(core.document().projection(), offset, false);
                let expected =
                    DocumentLayoutStyles::character_at(fresh.projection(), offset, false);
                assert_eq!(
                    actual.as_ref().map(|s| (s.bold, s.slant)),
                    expected.as_ref().map(|s| (s.bold, s.slant)),
                    "{format:?} style after restoration at {offset}"
                );
            }
        }
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            first_cursor,
            "{format:?}"
        );
        key(&mut core, view, Key::Backspace);
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}"
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
        key(&mut core, view, Key::Char('c'));
        key(&mut core, view, Key::Escape);
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: undo exact"
        );
    }
}
#[test]
fn styled_replace_batch_is_atomic_but_backspace_restores_one_grapheme() {
    for (format, source, at) in fixtures() {
        let (mut core, view) = start(format, source.as_bytes().to_vec(), Encoding::Utf8, at);
        style(&mut core, view, SemanticInlineStyle::Emphasis, true);
        let before = core.document().revision();
        core.handle(view, CoreEvent::Input(InputEvent::text("é猫")))
            .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        assert_eq!(core.document().revision().0, before.0 + 1, "{format:?}");
        key(&mut core, view, Key::Backspace);
        let wysiwyg_format = match format {
            Format::HtmlSource => Format::Html,
            Format::MarkdownSource => Format::Markdown,
            other => other,
        };
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            wysiwyg_format,
        )
        .unwrap();
        assert_eq!(reopened.text(), "éord", "{format:?}");
        key(&mut core, view, Key::Backspace);
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}"
        );
    }
}
#[test]
fn replace_restoration_survives_pending_style_changes_without_overwriting_previous_run() {
    for (format, source, at) in fixtures() {
        let (mut core, view) = start(format, source.as_bytes().to_vec(), Encoding::Utf8, at);
        style(&mut core, view, SemanticInlineStyle::Emphasis, true);
        key(&mut core, view, Key::Char('a'));
        let after_first = core.document().source_bytes();
        style(&mut core, view, SemanticInlineStyle::Strong, true);
        key(&mut core, view, Key::Char('b'));
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().source_bytes(), after_first, "{format:?}");
        key(&mut core, view, Key::Backspace);
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}"
        );
    }
}
#[test]
fn exact_replace_restoration_preserves_encoded_bytes_and_source_trivia() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for (format, source, at) in fixtures() {
            // RTF owns its byte-oriented grammar; UTF-16 transport is not an editable RTF encoding.
            if format == Format::Rtf && encoding != Encoding::Latin1 {
                continue;
            }
            let bytes: Vec<u8> = match encoding {
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => source.as_bytes().to_vec(),
            };
            let (mut core, view) = start(format, bytes.clone(), encoding, at);
            style(&mut core, view, SemanticInlineStyle::Emphasis, true);
            core.handle(view, CoreEvent::Input(InputEvent::text("éa")))
                .unwrap_or_else(|error| panic!("{format:?} {encoding:?}: {error:?}"));
            key(&mut core, view, Key::Backspace);
            key(&mut core, view, Key::Backspace);
            assert_eq!(
                core.document().source_bytes(),
                bytes,
                "{format:?} {encoding:?}"
            );
        }
    }
}
#[test]
fn pointer_and_external_edits_invalidate_recorded_replace_frontiers() {
    let source = "<p>word</p>";
    let (mut core, view) = start(Format::Html, source.as_bytes().to_vec(), Encoding::Utf8, 0);
    style(&mut core, view, SemanticInlineStyle::Emphasis, true);
    key(&mut core, view, Key::Char('a'));
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    key(&mut core, other, Key::Char('i'));
    core.handle(other, CoreEvent::Input(InputEvent::text("X")))
        .unwrap();
    key(&mut core, view, Key::Backspace);
    assert!(core.document().text().starts_with('X'));
    assert!(!core.document().text().contains('w'));
    let document_revision = core.document().revision();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision,
            text_offset: 3,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    key(&mut core, view, Key::Backspace);
    assert!(!core.document().text().contains('w'));
}
#[test]
fn inherited_styled_replace_restoration_keeps_large_document_layout_local() {
    let mut source = "<p>line</p>".repeat(10_000);
    source.push_str("<p><i>word</i></p><!--keep-->");
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let at = document.projection().text_tree().byte_len() - 4;
    let (mut core, view) = start(Format::Html, source.as_bytes().to_vec(), Encoding::Utf8, at);
    let first_id = core.document().projection().hard_line_id(0);
    style(&mut core, view, SemanticInlineStyle::Emphasis, true);
    key(&mut core, view, Key::Char('a'));
    assert_eq!(core.document().projection().hard_line_id(0), first_id);
    assert!(
        DocumentLayoutStyles::character_at(core.document().projection(), at, false)
            .unwrap()
            .slant
            != evim_core::document::FontSlant::Upright
    );
    key(&mut core, view, Key::Backspace);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().projection().hard_line_id(0), first_id);
}
#[test]
fn styled_replace_counts_backspace_and_dot_keep_one_undo_unit() {
    for (format, source, at) in fixtures() {
        let (mut core, view) = start(format, source.as_bytes().to_vec(), Encoding::Utf8, at);
        // Establish a counted Replace session at the fixture's exact source point.
        key(&mut core, view, Key::Escape);
        let document_revision = core.document().revision();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision,
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        key(&mut core, view, Key::Char('3'));
        key(&mut core, view, Key::Char('R'));
        style(&mut core, view, SemanticInlineStyle::Emphasis, true);
        core.handle(view, CoreEvent::Input(InputEvent::text("ab")))
            .unwrap();
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        let wysiwyg_format = match format {
            Format::HtmlSource => Format::Html,
            Format::MarkdownSource => Format::Markdown,
            other => other,
        };
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            wysiwyg_format,
        )
        .unwrap();
        assert_eq!(reopened.text(), "aaad", "{format:?}");
        let after_count = core.document().source_bytes();
        key(&mut core, view, Key::Char('.'));
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            after_count,
            "{format:?}: dot undo"
        );
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: counted undo"
        );
    }
}
#[test]
fn failed_styled_replace_batch_never_commits_a_prefix_or_discards_its_frontier() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let (mut core, view) = start(format, b"word".to_vec(), Encoding::Latin1, 0);
        style(&mut core, view, SemanticInlineStyle::Emphasis, true);
        key(&mut core, view, Key::Char('a'));
        let before = core.document().source_bytes();
        let cursor = core.command_state(view).unwrap().cursor();
        assert!(core
            .handle(view, CoreEvent::Input(InputEvent::text("b猫")))
            .is_err());
        assert_eq!(core.document().source_bytes(), before);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().source_bytes(), b"word");
    }
}
