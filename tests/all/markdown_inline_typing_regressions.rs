use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

const COMBINED: &[&str] = &[
    "*~~__a__~~* b\n",
    "*~~__emphasis strike strong__~~* ~~*__strike emphasis strong__*~~\n",
    "*~~__`emphasis strike strong`__~~* ~~*__`strike emphasis strong`__*~~\n",
];
const UNMATCHED: &[&str] = &[
    "**foo*\n",
    "*foo**",
    "__foo_",
    "____foo_",
    "_foo____",
    "****foo*bar*baz****",
    "**a.*.**a*.**.",
];

fn type_at(source: &str, at: usize, affinity: BoundaryAffinity, text: &str) -> Result<(), String> {
    type_encoded_at(source, Encoding::Utf8, at, affinity, text)
}

fn type_encoded_at(
    source: &str,
    encoding: Encoding,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
) -> Result<(), String> {
    let original_bytes = match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Latin1 => source
            .chars()
            .map(|ch| u8::try_from(ch as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    };
    let document =
        Document::from_bytes(original_bytes.clone(), encoding, Format::Markdown).unwrap();
    let before_text = document.text().to_owned();
    let before_styles = (0..before_text.len())
        .filter(|&at| before_text.is_char_boundary(at))
        .map(|at| {
            (
                at,
                DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 400.);
    let event = |core: &mut Core<MockTextMeasurementProvider>, input| -> Result<(), String> {
        let outcome = core
            .handle_with_layout(view, CoreEvent::Input(input))
            .map_err(|error| format!("{error:?}"))?;
        if let Some(command) = outcome.command {
            if !matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ) {
                return Err(format!("{:?}", command.status));
            }
        }
        Ok(())
    };
    event(&mut core, InputEvent::key('i'))?;
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity,
            extend_selection: false,
        },
    )
    .map_err(|error| format!("place: {error:?}"))?;
    event(&mut core, InputEvent::text(text))?;
    event(&mut core, InputEvent::Key(Key::Escape))?;
    let mut expected = before_text.clone();
    expected.insert_str(at, text);
    assert_eq!(core.document().text(), expected, "{source:?} at {at}");
    let fresh =
        Document::from_bytes(core.document().source_bytes(), encoding, Format::Markdown).unwrap();
    assert_eq!(fresh.text(), expected, "reopen: {source:?} at {at}");
    for (offset, _) in expected.char_indices() {
        let applications = |document: &Document| {
            document
                .projection()
                .style_spans()
                .iter()
                .filter(|span| span.range.contains(&offset))
                .map(|span| span.application.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            applications(core.document()),
            applications(&fresh),
            "reopen applications {source:?} at {offset}"
        );
        assert_eq!(
            DocumentLayoutStyles::character_at(core.document().projection(), offset, false)
                .unwrap(),
            DocumentLayoutStyles::character_at(fresh.projection(), offset, false).unwrap(),
            "reopen style {source:?} at {offset}"
        );
    }
    for (old_at, style) in before_styles {
        let new_at = old_at + if old_at >= at { text.len() } else { 0 };
        assert_eq!(
            DocumentLayoutStyles::character_at(fresh.projection(), new_at, false).unwrap(),
            style,
            "retained style {source:?} at {old_at}, typing at {at}"
        );
    }
    let committed = core.document().source_bytes();
    event(&mut core, InputEvent::key('u'))?;
    assert_eq!(core.document().source_bytes(), original_bytes);
    assert_eq!(core.document().text(), before_text);
    event(&mut core, InputEvent::Key(Key::Ctrl('r')))?;
    assert_eq!(core.document().source_bytes(), committed);
    assert_eq!(core.document().text(), expected);
    Ok(())
}

#[test]
fn typing_at_combined_emphasis_strike_and_strong_boundaries() {
    let mut failures = Vec::new();
    for &source in COMBINED {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=document.text().len() {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                if let Err(error) = type_at(source, at, affinity, "x") {
                    failures.push(format!("{source:?} at {at} {affinity:?}: {error}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_next_to_unmatched_delimiters_preserves_literal_punctuation() {
    let mut failures = Vec::new();
    for &source in UNMATCHED {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=document.text().len() {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                if let Err(error) = type_at(source, at, affinity, "x") {
                    failures.push(format!("{source:?} at {at} {affinity:?}: {error}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn model_insertions_use_the_same_verified_inline_repairs() {
    for &source in COMBINED.iter().chain(UNMATCHED) {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=original.text().len() {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let mut expected = original.text().to_owned();
            expected.insert_str(at, "x");
            document
                .insert(at, "x")
                .unwrap_or_else(|error| panic!("model {source:?} at {at}: {error:?}"));
            let committed = document.source_bytes();
            let reopened =
                Document::from_bytes(committed.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(document.text(), expected);
            assert_eq!(reopened.text(), expected);
            assert_eq!(
                document.projection().style_spans(),
                reopened.projection().style_spans()
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), committed);
        }
    }
}

#[test]
fn typing_at_list_item_inline_openers_keeps_marker_padding() {
    for source in [
        "- **word**\n- after",
        "- *word*\n- after",
        "- ~~word~~\n- after",
        "- `word`\n- after",
        "> - **word**\n- after",
        "- > **word**\n- after",
        "- [word](url)\n- after",
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                type_encoded_at(source, encoding, 0, affinity, "X").unwrap_or_else(|error| {
                    panic!("{source:?} {encoding:?} {affinity:?}: {error}")
                });
            }
        }
    }
}
