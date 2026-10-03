use viem_core::command::{InputEvent, Key};
use viem_core::document::{BlockKind, BoundaryAffinity, ContainerKind, Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

const ORIGINAL: &str = "```mermaid\ngraph LR\n    Writing --> Editing\n    Editing --> Saving\n```\n\n### Other GitHub features\n\nFollowing prose.";

fn assert_split_fences(document: &Document) {
    let projection = document.projection();
    for (text, code) in [("graph LR", false), ("Writing --> Editing", false), ("### Other GitHub features", true), ("Following prose.", true)] {
        let at = document.text().find(text).unwrap();
        let block = projection.blocks().iter()
            .find(|block| block.range.contains(&at)).unwrap();
        assert_eq!(block.style.0 == "Code Block", code, "{text}");
        assert_eq!(block.quote_depth, 0, "{text}");
        assert_eq!(block.kind, BlockKind::Paragraph, "{text}");
    }
    assert!(projection.blocks().iter().all(|block| block.containers.iter()
        .filter(|member| member.container.kind == ContainerKind::CodeBlock).count() <= 1),
        "sibling fences cannot become overlapping nested code containers");
    let structure = projection.container_structure();
    let code: Vec<_> = structure.containers.iter().filter(|node| node.attributes.kind == ContainerKind::CodeBlock).collect();
    assert_eq!(code.len(), 2);
    assert!(code[0].range.end <= code[1].range.start);
    assert!(code[0].parent.is_some());
    assert!(code[1].parent.is_none());
}

#[test]
fn a_quote_on_only_the_opening_fence_ends_before_unquoted_body() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for marker in [">", "> ", "> > "] {
            for ending in ["\n", "\r\n"] {
                let source = format!("{marker}{ORIGINAL}").replace('\n', ending);
                let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
                assert_split_fences(&document);
            }
        }
    }
}

#[test]
fn typing_a_quote_before_a_fence_matches_reopening_and_exact_history() {
    let document = Document::from_bytes(ORIGINAL.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: 0,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::text(">"))).unwrap();
    assert_eq!(core.document().source_bytes(), format!(">{ORIGINAL}").as_bytes());
    assert_split_fences(core.document());
    for format in [Format::Markdown, Format::MarkdownSource] {
        assert_split_fences(&Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, format).unwrap());
    }
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), ORIGINAL.as_bytes());
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), format!(">{ORIGINAL}").as_bytes());
    assert_split_fences(core.document());
}

#[test]
fn an_indented_block_after_an_unclosed_quoted_fence_has_its_own_owner() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = ">```\n    code\n```\n# literal heading";
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let at = document.text().find("code").unwrap();
        let block = document.projection().blocks().iter().find(|block| block.range.contains(&at)).unwrap();
        assert_eq!(block.style.0, "Code Block");
        assert_eq!(block.quote_depth, 0);
        assert_eq!(block.containers.len(), 1);
        assert_eq!(block.containers[0].container.kind, ContainerKind::CodeBlock);
        assert_eq!(document.projection().container_structure().containers.iter()
            .filter(|node| node.attributes.kind == ContainerKind::CodeBlock).count(), 3);
    }
}

#[test]
fn quoting_a_fence_preserves_trailing_blank_code_lines() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for blank_lines in 0..=2 {
            for ending in ["\n", "\r\n"] {
                let body = format!("code{}", "\n".repeat(blank_lines));
                let source = format!("Before\n\n```\n{body}\n```\n\nAfter").replace('\n', ending);
                let quoted = format!("Before\n\n> ```\n> {}\n> ```\n\nAfter", body.replace('\n', "\n> ")).replace('\n', ending);
                let a = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                let b = Document::from_bytes(quoted.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                if format == Format::Markdown { assert_eq!(a.text(), b.text()); }
                let mut core = Core::new(a);
                let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
                let at = core.document().text().find("code").unwrap();
                core.handle(view, CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(), text_offset: at,
                    affinity: BoundaryAffinity::Downstream, extend_selection: false,
                }).unwrap();
                let expected = core.list_selection_identity(view).unwrap();
                core.handle(view, CoreEvent::SetBlockQuote { expected, enabled: true }).unwrap();
                assert_eq!(core.document().source_bytes(), quoted.as_bytes());
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                assert_eq!(core.document().source_bytes(), quoted.as_bytes());
                let expected = core.list_selection_identity(view).unwrap();
                core.handle(view, CoreEvent::SetBlockQuote { expected, enabled: false }).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn opening_a_paragraph_inside_quoted_code_keeps_its_closer_in_the_quote() {
    for source in [
        ">     code\n>     next",
        "> ```\n> code\n> next\n> ```",
        ">   ```\n>   code\n>   next\n>   ```",
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('o'))).unwrap();
        assert_eq!(core.document().text(), "code\n\nnext", "{source}");
        let saved = core.document().source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert_eq!(reopened.projection().blocks()[0].quote_depth, 1);
        assert_eq!(reopened.projection().blocks()[0].style.0, "Code Block");
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}
