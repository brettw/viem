use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

fn open(source: &str) -> Core<MockTextMeasurementProvider> {
    Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    )
}

fn assert_rule_edit(
    source: &str,
    at: usize,
    keys: &[Key],
    expected_text: &str,
    expected_rules: (usize, usize),
) {
    let mut core = open(source);
    assert!(core
        .document()
        .projection()
        .blocks()
        .iter()
        .all(|block| !block.front_matter));
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .filter(|block| block.thematic_break)
            .count(),
        expected_rules.0,
        "original rule fixture: {source:?}"
    );
    let original_code_blocks = core
        .document()
        .projection()
        .blocks()
        .iter()
        .filter(|block| block.style.0 == "Code Block")
        .count();
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
            .unwrap_or_else(|error| panic!("{source:?}, {keys:?}: {error:?}"));
        if let Some(command) = outcome.command {
            assert!(
                matches!(
                    command.status,
                    CommandStatus::Complete | CommandStatus::Pending
                ),
                "{source:?}, {keys:?}, key {key:?}: {:?}",
                command.status
            );
        }
    }
    assert_eq!(
        core.document().text(),
        expected_text,
        "{source:?}, {keys:?}"
    );
    assert!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .all(|block| !block.front_matter),
        "editing thematic rules must not create metadata: {source:?}, {keys:?}"
    );
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .filter(|block| block.thematic_break)
            .count(),
        expected_rules.1,
        "after {keys:?}: saved {:?}, blocks {:?}",
        String::from_utf8_lossy(&core.document().source_bytes()),
        core.document().projection().blocks(),
    );
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .filter(|block| block.style.0 == "Code Block")
            .count(),
        original_code_blocks
    );
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(
        reopened.text(),
        expected_text,
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
fn creating_a_leading_rule_does_not_pair_with_a_later_rule_as_metadata() {
    assert_rule_edit(
        "start\n\n---\n\nTail",
        0,
        &[
            Key::Char('O'),
            Key::Char('-'),
            Key::Char('-'),
            Key::Char('-'),
            Key::Escape,
        ],
        "\nstart\n\nTail",
        (1, 2),
    );
}

#[test]
fn creating_a_later_rule_does_not_reinterpret_an_existing_leading_rule() {
    let source = "---\n\nTail";
    let at = open(source).document().text().find("Tail").unwrap();
    assert_rule_edit(
        source,
        at,
        &[
            Key::Char('o'),
            Key::Char('-'),
            Key::Char('-'),
            Key::Char('-'),
            Key::Escape,
        ],
        "\nTail\n",
        (1, 2),
    );
}

#[test]
fn editing_later_code_cannot_turn_an_existing_leading_rule_into_metadata() {
    let source = "---\n\n```\n----\n```\n\nTail";
    let at = open(source).document().text().find("----").unwrap();
    assert_rule_edit(source, at, &[Key::Char('x')], "\n---\nTail", (1, 1));
}

#[test]
fn small_edits_after_an_unclosed_leading_rule_keep_source_work_local() {
    for count in [100, 10_000] {
        let source = format!("---\n\n{}Tail", "Unrelated **prose**.\n\n".repeat(count));
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find("Tail").unwrap();
        let (result, work) = viem_core::document::measure_document_work(|| document.replace(at..at + 4, "End"));
        result.unwrap();
        assert_eq!(work.source_full_materializations, 0, "{count}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{count}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{count}: {work:?}");
        assert_eq!(document.source_bytes(), source.replace("Tail", "End").as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
