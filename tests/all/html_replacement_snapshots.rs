//! Replacement context must follow the source snapshot, including speculative
//! composition, alternating views, and history restoration.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

fn select(core: &mut Editor, view: ViewId, range: Range<usize>, reverse: bool) {
    let ends = if reverse {
        [range.end, range.start]
    } else {
        [range.start, range.end]
    };
    for (index, text_offset) in ends.into_iter().enumerate() {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: index != 0,
            },
        )
        .unwrap();
    }
}

fn check_reopened(document: &Document) {
    let fresh =
        Document::from_bytes(document.source_bytes(), document.encoding(), Format::Html).unwrap();
    assert_eq!(document.text(), fresh.text());
    let blocks = |doc: &Document| {
        doc.projection()
            .blocks()
            .iter()
            .map(|block| {
                (
                    block.range.clone(),
                    block.style.clone(),
                    block.direct_paragraph.clone(),
                    block.direct_default_character.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(document), blocks(&fresh));
    for (at, _) in document.text().grapheme_indices(true) {
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap(),
            DocumentLayoutStyles::semantic_character_at(fresh.projection(), at, false).unwrap(),
            "character at {at}",
        );
        assert_eq!(
            document.link_at(document.text_point(at).unwrap()).unwrap(),
            fresh.link_at(fresh.text_point(at).unwrap()).unwrap(),
            "link at {at}"
        );
    }
}

#[test]
fn repeated_replacements_across_views_composition_and_history_match_fresh_html() {
    let source = "<blockquote style='color:navy'><P data-keep='first' DATA-KEEP=second>pré \
        <B title='猫'>bold</B> <a HREF='https://example.test/a?b=1&amp;c=2'>link</a> tail</P>\r\n\
        <p><i>other</i> paragraph &amp; entities</p></blockquote><!--keep-->"
        .to_owned();
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = match encoding {
            Encoding::Utf8 => source.as_bytes().to_vec(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        };
        let mut core = Core::new(Document::from_bytes(bytes, encoding, Format::Html).unwrap());
        let views = [
            core.add_view(MockTextMeasurementProvider::new(), 140., 200.),
            core.add_view(MockTextMeasurementProvider::new(), 600., 200.),
        ];
        let mut random = 0x983a_017bu32;
        for turn in 0..24 {
            let view = views[turn % 2];
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('i'));
            let original = core.document().source_bytes();
            let text = core.document().text().to_owned();
            let boundaries = text
                .grapheme_indices(true)
                .filter(|(_, value)| *value != "\n")
                .map(|(at, value)| at..at + value.len())
                .collect::<Vec<_>>();
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let range = boundaries[random as usize % boundaries.len()].clone();
            let expected = DocumentLayoutStyles::semantic_character_at(
                core.document().projection(),
                range.start,
                false,
            )
            .unwrap();
            let link = core
                .document()
                .link_at(core.document().text_point(range.start).unwrap())
                .unwrap();
            select(&mut core, view, range.clone(), turn % 2 == 0);
            if turn % 3 == 0 {
                let target = CompositionTarget::at_offsets(core.document(), range.clone()).unwrap();
                for event in [
                    CompositionEvent::Begin(target),
                    CompositionEvent::Update(CompositionUpdate::new("temporary", 9..9)),
                    CompositionEvent::Cancel,
                ] {
                    core.handle(view, CoreEvent::Composition(event)).unwrap();
                }
                assert_eq!(core.document().source_bytes(), original);
            }
            for value in ["猫", "&"] {
                core.handle(view, CoreEvent::Input(InputEvent::text(value)))
                    .unwrap();
            }
            assert_eq!(
                core.document().text(),
                format!("{}猫&{}", &text[..range.start], &text[range.end..])
            );
            for at in [range.start, range.start + "猫".len()] {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        core.document().projection(),
                        at,
                        false
                    )
                    .unwrap(),
                    expected
                );
                assert_eq!(
                    core.document()
                        .link_at(core.document().text_point(at).unwrap())
                        .unwrap(),
                    link
                );
            }
            check_reopened(core.document());
            let edited = core.document().source_bytes();
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), original);
            check_reopened(core.document());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), edited);
            check_reopened(core.document());
        }
    }
}

#[test]
fn source_attribute_edits_format_and_encoding_changes_rebuild_replacement_context() {
    use viem_core::document::{FileFormat, FontSlant, FormatOperation};
    let source = "<p>first</p>\r\n<p><a href='old'><b>word</b></a> tail</p>";
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
    core.handle(
        view,
        CoreEvent::SetFormat {
            document: core.document().id(),
            revision: core.document().revision(),
            target: Format::HtmlSource,
            operation: FormatOperation::Reinterpret,
        },
    )
    .unwrap();
    key(&mut core, view, Key::Char('i'));
    for (old, new) in [
        ("old", "https://example.test/new"),
        ("<b>", "<i>"),
        ("</b>", "</i>"),
    ] {
        let at = core.document().text().find(old).unwrap();
        select(&mut core, view, at..at + old.len(), false);
        core.handle(view, CoreEvent::Input(InputEvent::text(new)))
            .unwrap();
    }
    key(&mut core, view, Key::Escape);
    core.handle(
        view,
        CoreEvent::SetFormat {
            document: core.document().id(),
            revision: core.document().revision(),
            target: Format::Html,
            operation: FormatOperation::Reinterpret,
        },
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::SetEncoding {
            document: core.document().id(),
            revision: core.document().revision(),
            target: Encoding::Utf16Be,
        },
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::SetFileFormat {
            document: core.document().id(),
            revision: core.document().revision(),
            target: FileFormat::Unix,
        },
    )
    .unwrap();
    let before = core.document().source_bytes();
    key(&mut core, view, Key::Char('i'));
    let at = core.document().text().find("word").unwrap();
    select(&mut core, view, at..at + 4, true);
    for value in ["猫", "é"] {
        core.handle(view, CoreEvent::Input(InputEvent::text(value)))
            .unwrap();
    }
    assert_eq!(core.document().text(), "first\n猫é tail");
    for at in [at, at + "猫".len()] {
        let character =
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false)
                .unwrap();
        assert!(!character.bold);
        assert_eq!(character.slant, FontSlant::Italic);
        assert_eq!(
            core.document()
                .link_at(core.document().text_point(at).unwrap())
                .unwrap()
                .as_deref(),
            Some("https://example.test/new")
        );
    }
    check_reopened(core.document());
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), before);
    check_reopened(core.document());
    key(&mut core, view, Key::Ctrl('r'));
    check_reopened(core.document());
}

#[test]
fn all_small_recovered_html_replacements_match_requested_text_and_fresh_projection() {
    let mut cases = 0;
    for source in [
        "<p><b>A<a href='x'>B</b>C</a>D</p>",
        "<p><a href='x'>A<b>B</a>C</b>D</p>",
        "<p><b><i>A</b>B</i>C</p>",
        "<p>A<b>B<i>C</b>D</i>E</p>",
        "<p>A<a href='x'>B<a href='y'>C</a>D</a>E</p>",
        "<p>A <b>B</b> C</p>\r\n<p>D</p>",
        "<p>A <a\n href='x'>B</a> C</p>\n<p>D</p>",
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = match encoding {
                Encoding::Utf8 => source.as_bytes().to_vec(),
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => unreachable!(),
            };
            let original = Document::from_bytes(bytes.clone(), encoding, Format::Html).unwrap();
            let text = original.text().to_owned();
            let boundaries = text
                .grapheme_indices(true)
                .map(|(at, _)| at)
                .chain(std::iter::once(text.len()))
                .collect::<Vec<_>>();
            for (index, &start) in boundaries.iter().enumerate() {
                for &end in &boundaries[index + 1..] {
                    for replacement in ["X", "XY", " "] {
                        let mut core = Core::new(
                            Document::from_bytes(bytes.clone(), encoding, Format::Html).unwrap(),
                        );
                        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 160.);
                        key(&mut core, view, Key::Char('i'));
                        select(&mut core, view, start..end, false);
                        core.handle(view, CoreEvent::Input(InputEvent::text(replacement)))
                            .unwrap_or_else(|error| {
                                panic!(
                                    "{source:?}, {encoding:?}, {start}..{end}, \
                                     {replacement:?}: {error:?}"
                                )
                            });
                        assert_eq!(
                            // HTML typing preserves otherwise-collapsible
                            // spaces as NBSP. Exact spelling is independently
                            // checked by the fresh-projection oracle below.
                            core.document().text().replace('\u{a0}', " "),
                            format!("{}{replacement}{}", &text[..start], &text[end..]),
                            "{source:?}, {encoding:?}, {start}..{end}, {replacement:?}",
                        );
                        if source.starts_with("<p>A ") {
                            // Well-formed fixtures also prove that source and
                            // projection did not agree on an unintended style
                            // change to retained text. Compare grapheme ordinals
                            // because HTML's NBSP repair can change byte lengths.
                            let new_boundaries = core
                                .document()
                                .text()
                                .grapheme_indices(true)
                                .map(|(at, _)| at)
                                .collect::<Vec<_>>();
                            let end_index = boundaries.binary_search(&end).unwrap();
                            for old_index in
                                (0..index).chain(end_index..boundaries.len() - 1)
                            {
                                let new_index = if old_index < index {
                                    old_index
                                } else {
                                    old_index - (end_index - index)
                                        + replacement.graphemes(true).count()
                                };
                                assert_eq!(
                                    DocumentLayoutStyles::semantic_character_at(
                                        original.projection(),
                                        boundaries[old_index],
                                        false,
                                    ),
                                    DocumentLayoutStyles::semantic_character_at(
                                        core.document().projection(),
                                        new_boundaries[new_index],
                                        false,
                                    ),
                                    "retained glyph {old_index}: {source:?}, {encoding:?}, \
                                     {start}..{end}, {replacement:?}",
                                );
                            }
                        }
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            check_reopened(core.document());
                        }))
                        .unwrap_or_else(|_| {
                            panic!(
                                "fresh projection mismatch: {source:?}, {encoding:?}, \
                                 {start}..{end}, {replacement:?}"
                            )
                        });
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 1_008);
}

#[test]
fn visible_link_glyphs_ignore_empty_recovered_caret_contributors() {
    for label in ["X", "&#x1F431;&amp;X"] {
        let source = format!(
            "<p><a href='empty'><b></a></b><a href='visible'>{label}</a>D</p>"
        );
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes = match encoding {
                Encoding::Utf8 => source.as_bytes().to_vec(),
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => unreachable!(),
            };
            let document = Document::from_bytes(bytes, encoding, Format::Html).unwrap();
            assert!(document
                .projection()
                .provenance()
                .iter()
                .any(|span| span.formatted == (0..0)));
            for (at, glyph) in document.text().grapheme_indices(true) {
                assert_eq!(
                    document.link_at(document.text_point(at).unwrap()).unwrap().as_deref(),
                    (glyph != "D").then_some("visible"),
                    "{source:?}, {encoding:?}, glyph {at}",
                );
            }
        }
    }
}
