use viem_core::{Document, Encoding, Format};
use std::time::Instant;

#[test]
#[ignore = "manual timing of the current repository specification"]
fn time_repository_markdown_switches() {
    let path = std::env::var("VIEM_PROFILE_MARKDOWN_PATH").unwrap_or_else(|_| "AGENTS.md".into());
    let source = std::fs::read(path).unwrap();
    let start = Instant::now();
    let mut document =
        Document::from_bytes(source.clone(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    eprintln!(
        "OPEN {:?}, {} bytes, {} blocks",
        start.elapsed(),
        source.len(),
        document.projection().blocks().len()
    );
    for format in [
        Format::Markdown,
        Format::MarkdownSource,
        Format::Markdown,
        Format::MarkdownSource,
    ] {
        let start = Instant::now();
        document
            .set_format(format, viem_core::document::FormatOperation::Reinterpret)
            .unwrap();
        eprintln!(
            "SWITCH {format:?} {:?}, {} text bytes, {} blocks",
            start.elapsed(),
            document.text().len(),
            document.projection().blocks().len()
        );
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn large_switches_preserve_source_anchors_styles_and_refresh_after_edit() {
    use viem_core::document::{
        Association, BoundaryAffinity, DeletionRecovery, MappingOutcome, ModelRequest,
    };
    let source = (0..1_500).map(|index| format!("## Section {index}\n\nA **strong** word and e\u{301} 👩🏽‍💻.\nContinuation {index}.\n\n")).collect::<String>();
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let token = "Continuation 1499";
    for format in [Format::Markdown, Format::MarkdownSource, Format::Markdown] {
        let at = document.text().find(token).unwrap();
        let anchor = document
            .text_anchor(
                document.text_point(at).unwrap(),
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::SetFormat {
                document: document.id(),
                revision: document.revision(),
                target: format,
                operation: viem_core::document::FormatOperation::Reinterpret,
            })
            .unwrap();
        assert!(prepared.summary().source_patches().is_empty());
        let mapped = match prepared
            .text_position_map()
            .map_text_anchor(anchor)
            .unwrap()
        {
            MappingOutcome::Exact(anchor) | MappingOutcome::Moved(anchor) => anchor,
            outcome => panic!("unchanged source word lost its identity: {outcome:?}"),
        };
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(mapped.offset(), document.text().find(token).unwrap());
        assert_eq!(document.source_bytes(), source.as_bytes());
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), fresh.text());
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
    }
    let at = document.text().find(token).unwrap();
    document.insert(at, "Authored ").unwrap();
    let edited_source = document.source_bytes();
    document
        .set_format(
            Format::MarkdownSource,
            viem_core::document::FormatOperation::Reinterpret,
        )
        .unwrap();
    assert!(document.text().contains("Authored Continuation 1499"));
    assert_eq!(document.source_bytes(), edited_source);
    document
        .set_format(
            Format::Markdown,
            viem_core::document::FormatOperation::Reinterpret,
        )
        .unwrap();
    assert!(document.text().contains("Authored Continuation 1499"));
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.format(), Format::Markdown);
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(!document.text().contains("Authored"));
    for _ in 0..3 {
        assert!(document.redo());
    }
    assert_eq!(document.source_bytes(), edited_source);
}

#[test]
fn same_source_mode_switch_keeps_encoding_and_grapheme_boundaries() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let source = "# café\r\n\r\n__tail__ and text\r\n";
        let bytes = match encoding {
            Encoding::Utf8 => source.as_bytes().to_vec(),
            Encoding::Latin1 => source.chars().map(|ch| ch as u8).collect(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        };
        let mut document =
            Document::from_bytes(bytes.clone(), encoding, Format::MarkdownSource).unwrap();
        document
            .set_format(
                Format::Markdown,
                viem_core::document::FormatOperation::Reinterpret,
            )
            .unwrap();
        assert_eq!(document.text(), "café\ntail and text");
        document
            .set_format(
                Format::MarkdownSource,
                viem_core::document::FormatOperation::Reinterpret,
            )
            .unwrap();
        assert_eq!(document.text(), "# café\n__tail__ and text\n");
        assert_eq!(document.source_bytes(), bytes);
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
    }
}
