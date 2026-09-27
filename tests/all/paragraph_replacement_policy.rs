use std::ops::Range;
use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};
type Editor = Core<MockTextMeasurementProvider>;
fn fixture(source: &str, format: Format) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
    key(&mut core, view, Key::Char('i'));
    (core, view)
}
fn select(core: &mut Editor, view: ViewId, range: Range<usize>, reverse: bool) {
    let (anchor, active) = if reverse {
        (range.end, range.start)
    } else {
        (range.start, range.end)
    };
    for (text_offset, extend_selection) in [(anchor, false), (active, true)] {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection,
            },
        )
        .unwrap();
    }
    assert_eq!(core.list_selection_identity(view).unwrap().range(), range);
}
fn text(core: &mut Editor, view: ViewId, text: &str) {
    core.handle(view, CoreEvent::Input(InputEvent::text(text)))
        .unwrap_or_else(|error| {
            panic!(
                "{error:?}: {:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
}
fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

fn owner(document: &Document, at: usize) -> viem_core::document::Block {
    document
        .projection()
        .blocks()
        .iter()
        .find(|block| block.range.start <= at && at <= block.range.end)
        .unwrap()
        .clone()
}
fn character(
    document: &Document,
    at: usize,
    upstream: bool,
) -> viem_core::document::ResolvedCharacterStyle {
    DocumentLayoutStyles::semantic_character_at(document.projection(), at, upstream).unwrap()
}
fn ime(core: &mut Editor, view: ViewId, range: Range<usize>) {
    let target = CompositionTarget::at_offsets(core.document(), range).unwrap();
    for event in [
        CompositionEvent::Begin(target),
        CompositionEvent::Update(CompositionUpdate::new("X", 1..1)),
        CompositionEvent::Commit,
    ] {
        core.handle(view, CoreEvent::Composition(event)).unwrap();
    }
}

#[test]
fn replacement_preserves_the_first_paragraph_and_skips_leading_separators_for_characters() {
    for (format, source) in [
        (Format::Html, "<h1>A</h1><h2>BC</h2><p>D</p>"),
        (
            Format::Html,
            "<p style='font-size:20pt;text-align:right;padding:5pt'>A</p><p>BC</p><p>D</p>",
        ),
        (
            Format::Html,
            "<p style='text-align:right'><b>A</b></p><p style='text-align:center'>BC</p><p>D</p>",
        ),
        (Format::Markdown, "# A\n\n## BC\n\nD"),
        (Format::Markdown, "**A**\n\nBC\n\nD"),
        (Format::Rtf, r"{\rtf1 \qr {\b A}\par \qc BC\par \ql D}"),
    ] {
        for (range, sample, upstream) in [
            (2..4, 2, false),
            (2..5, 2, false),
            (1..4, 2, false),
            (1..2, 1, true),
            (0..6, 0, false),
        ] {
            for reverse in [false, true] {
                for composition in [false, true] {
                    let (mut core, view) = fixture(source, format);
                    let original = core.document().text().to_owned();
                    let expected = character(core.document(), sample, upstream);
                    let expected_owner = owner(core.document(), range.start);
                    select(&mut core, view, range.clone(), reverse);
                    if composition {
                        ime(&mut core, view, range.clone());
                    } else {
                        text(&mut core, view, "X");
                    }
                    text(&mut core, view, "Y");
                    assert_eq!(
                        core.document().text(),
                        format!("{}XY{}", &original[..range.start], &original[range.end..])
                    );
                    let reopened = Document::from_bytes(
                        core.document().source_bytes(),
                        Encoding::Utf8,
                        format,
                    )
                    .unwrap();
                    for document in [core.document(), &reopened] {
                        let actual_owner = owner(document, range.start);
                        assert_eq!(
                            actual_owner.style, expected_owner.style,
                            "{format:?} {range:?}, reverse={reverse}, ime={composition}"
                        );
                        assert_eq!(
                            actual_owner.direct_paragraph, expected_owner.direct_paragraph,
                            "{format:?} {range:?}"
                        );
                        assert_eq!(
                            actual_owner.direct_default_character,
                            expected_owner.direct_default_character,
                            "{format:?} {range:?}"
                        );
                        for at in range.start..range.start + 2 {
                            let mut actual = character(document, at, false);
                            let expected = expected.clone();
                            if format == Format::Markdown {
                                actual.size = expected.size;
                                actual.base_weight = expected.base_weight;
                                actual.weight = expected.weight;
                                actual.font_families = expected.font_families.clone();
                            }
                            assert_eq!(
                                actual,
                                expected,
                                "{format:?} {range:?}, reverse={reverse}, ime={composition}: {:?}",
                                String::from_utf8_lossy(&document.source_bytes())
                            );
                        }
                    }
                    key(&mut core, view, Key::Escape);
                    key(&mut core, view, Key::Char('u'));
                    if composition {
                        key(&mut core, view, Key::Char('u'));
                    }
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                }
            }
        }
    }
}

#[test]
fn separators_alone_do_not_extend_links_and_empty_paragraphs_supply_their_own_defaults() {
    for (format, source, range, sample, expected_link) in [
        (
            Format::Html,
            "<p><a href='https://example.test/'><b>A</b></a></p><p>BC</p>",
            1..2,
            1,
            None,
        ),
        (
            Format::Markdown,
            "[**A**](https://example.test/)\n\nBC",
            1..2,
            1,
            None,
        ),
        (
            Format::Html,
            "<h2 style='text-align:center'></h2><p>BC</p>",
            0..1,
            0,
            None,
        ),
        (
            Format::Html,
            "<h2></h2><p></p><p><a href='https://example.test/'>BC</a></p>",
            0..3,
            2,
            Some("https://example.test/"),
        ),
    ] {
        for reverse in [false, true] {
            let (mut core, view) = fixture(source, format);
            let expected = character(core.document(), sample, expected_link.is_none());
            let expected_owner = owner(core.document(), range.start);
            select(&mut core, view, range.clone(), reverse);
            text(&mut core, view, "X");
            text(&mut core, view, "Y");
            for at in range.start..range.start + 2 {
                assert_eq!(
                    character(core.document(), at, false),
                    expected,
                    "{source}, at={at}: {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
                assert_eq!(
                    core.document()
                        .link_at(core.document().text_point(at).unwrap())
                        .unwrap()
                        .as_deref(),
                    expected_link,
                    "{source}"
                );
            }
            assert_eq!(
                owner(core.document(), range.start).style,
                expected_owner.style
            );
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn vim_whole_final_paragraph_change_preserves_its_style() {
    for (format, source) in [
        (
            Format::Html,
            "<p>A</p><h2 style='text-align:center'>BC</h2>",
        ),
        (Format::Markdown, "A\n\n## BC"),
        (Format::Rtf, r"{\rtf1 A\par \qc\sb120 BC}"),
    ] {
        for native in [false, true] {
            let (mut core, view) = fixture(source, format);
            let expected_owner = owner(core.document(), 2);
            let expected_character = character(core.document(), 2, false);
            if native {
                select(&mut core, view, 2..4, true);
            } else {
                key(&mut core, view, Key::Escape);
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: 2,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                key(&mut core, view, Key::Char('V'));
                key(&mut core, view, Key::Char('c'));
            }
            text(&mut core, view, "X");
            text(&mut core, view, "Y");
            assert_eq!(
                core.document().text(),
                "A\nXY",
                "{format:?} native={native}"
            );
            let reopened =
                Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, format)
                    .unwrap();
            for document in [core.document(), &reopened] {
                assert_eq!(owner(document, 2).style, expected_owner.style);
                assert_eq!(
                    owner(document, 2).direct_paragraph,
                    expected_owner.direct_paragraph
                );
                assert_eq!(character(document, 2, false), expected_character);
            }
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn whole_document_replacement_keeps_structural_paragraph_styles() {
    for (format, source) in [
        (Format::Html, "<blockquote><p>A</p></blockquote>"),
        (Format::Markdown, "> A"),
        (Format::Html, "<ul><li>A</li></ul>"),
        (Format::Html, "<ul><li><ul><li>A</li></ul></li></ul>"),
        (
            Format::Html,
            "<ol start='5'><li><ol start='3'><li>A</li></ol></li></ol>",
        ),
        (Format::Markdown, "- A"),
        (Format::Html, "<pre>A</pre>"),
        (Format::Markdown, "```\nA\n```"),
    ] {
        let (mut core, view) = fixture(source, format);
        let expected = owner(core.document(), 0);
        key(&mut core, view, Key::SelectAll);
        text(&mut core, view, "X");
        text(&mut core, view, "Y");
        assert_eq!(core.document().text(), "XY", "{source}");
        assert_eq!(owner(core.document(), 0).style, expected.style, "{source}");
        assert_eq!(owner(core.document(), 0).kind, expected.kind, "{source}");
        let reopened =
            Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(owner(&reopened, 0).style, expected.style, "{source}");
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn changing_an_empty_styled_paragraph_keeps_its_assignment() {
    for source in [
        "<h2></h2>",
        "<p style='text-align:center;font-size:20pt'></p>",
    ] {
        let (mut core, view) = fixture(source, Format::Html);
        let expected = owner(core.document(), 0);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('V'));
        key(&mut core, view, Key::Char('c'));
        text(&mut core, view, "X");
        assert_eq!(owner(core.document(), 0).style, expected.style);
        assert_eq!(
            owner(core.document(), 0).direct_paragraph,
            expected.direct_paragraph
        );
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn replacing_from_an_empty_first_paragraph_retains_its_defaults_separately_from_character_traits() {
    for source in [
        "<p style='font-size:30pt;text-align:center'></p><p style='font-size:10pt'>B</p>",
        "<div style='font-size:30pt;text-align:center'><p></p></div><p style='font-size:10pt'>B</p>",
    ] {
        for reverse in [false, true] {
            for composition in [false, true] {
                let (mut core, view) = fixture(source, Format::Html);
                let expected = owner(core.document(), 0);
                select(&mut core, view, 0..2, reverse);
                if composition {
                    ime(&mut core, view, 0..2);
                } else {
                    text(&mut core, view, "X");
                }
                text(&mut core, view, "Y");
                assert_eq!(core.document().text(), "XY");
                let reopened = Document::from_bytes(
                    core.document().source_bytes(),
                    Encoding::Utf8,
                    Format::Html,
                )
                .unwrap();
                for document in [core.document(), &reopened] {
                    let actual = owner(document, 0);
                    assert_eq!(actual.style, expected.style);
                    assert_eq!(actual.direct_paragraph, expected.direct_paragraph);
                    assert_eq!(
                        actual.direct_default_character,
                        expected.direct_default_character
                    );
                    assert_eq!(character(document, 0, false).size, 10.);
                    assert_eq!(character(document, 1, false).size, 10.);
                }
                key(&mut core, view, Key::Enter);
                let paragraphs = core.document().projection().blocks();
                assert_eq!(paragraphs.len(), 2);
                assert_eq!(paragraphs[1].style, expected.style);
                // An empty inline scope supplies the visible typing seed; after text is
                // inserted the paragraph defaults and inline traits remain distinct.
                text(&mut core, view, "Z");
                assert_eq!(
                    owner(core.document(), 3).direct_default_character,
                    expected.direct_default_character,
                    "source={source}, reverse={reverse}, ime={composition}: {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes())
                );
                assert_eq!(character(core.document(), 3, false).size, 10.);
                key(&mut core, view, Key::Escape);
                key(&mut core, view, Key::Char('u'));
                if composition {
                    key(&mut core, view, Key::Char('u'));
                }
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn changing_the_only_word_keeps_its_existing_paragraph_owner() {
    for source in [
        "<p><b>bold</b></p>",
        "<h2 style='font-size:30pt'><span style='font-size:10pt'>word</span></h2>",
    ] {
        let (mut core, view) = fixture(source, Format::Html);
        let expected = owner(core.document(), 0);
        let character = character(core.document(), 0, false);
        key(&mut core, view, Key::Escape);
        text(&mut core, view, "cw");
        text(&mut core, view, "X");
        text(&mut core, view, "Y");
        assert_eq!(core.document().text(), "XY");
        assert_eq!(core.document().projection().blocks().len(), 1);
        assert_eq!(owner(core.document(), 0).style, expected.style);
        assert_eq!(
            owner(core.document(), 0).direct_default_character,
            expected.direct_default_character
        );
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
                .unwrap(),
            character
        );
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn replacing_a_code_block_keeps_new_markdown_syntax_literal() {
    let source = "```\nA\n```";
    let (mut core, view) = fixture(source, Format::Markdown);
    key(&mut core, view, Key::SelectAll);
    text(&mut core, view, "`*<");
    text(&mut core, view, "Y");
    assert_eq!(core.document().text(), "`*<Y");
    assert_eq!(owner(core.document(), 0).style.0, "Code Block");
    let reopened = Document::from_bytes(
        core.document().source_bytes(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    assert_eq!(reopened.text(), core.document().text());
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn replacement_clears_character_background_without_changing_paragraph_defaults() {
    let source = "<p style='background-color:#ff0000'>A</p><p>BC</p>";
    for reverse in [false, true] {
        for composition in [false, true] {
            let (mut core, view) = fixture(source, Format::Html);
            let expected = owner(core.document(), 0);
            select(&mut core, view, 1..4, reverse);
            if composition {
                ime(&mut core, view, 1..4);
            } else {
                text(&mut core, view, "X");
            }
            text(&mut core, view, "Y");
            let reopened =
                Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html)
                    .unwrap();
            for document in [core.document(), &reopened] {
                assert_eq!(document.text(), "AXY");
                assert_eq!(
                    owner(document, 0).direct_default_character,
                    expected.direct_default_character
                );
                assert_eq!(owner(document, 0).direct_paragraph.background.unwrap().alpha, 1.);
                assert!(character(document, 0, false).background.is_none());
                for at in 1..3 {
                    assert_eq!(
                        character(document, at, false)
                            .background
                            .map_or(0., |color| color.alpha),
                        0.
                    );
                }
            }
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            if composition {
                key(&mut core, view, Key::Char('u'));
            }
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}
