use viem_core::command::{InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn document(format: Format, source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn every_visible_rich_caret_boundary_accepts_typing_with_either_affinity() {
    let fixtures = [
        (Format::Html, "<p><b><i></i></b></p>"),
        (Format::Html, "<p><b>A</b><i></i></p>"),
        (Format::Html, "<p><b>&fjlig;</b><i>B</i></p>"),
        (Format::Html, "<ul><li><p></p><ul><li>X</li></ul></li></ul>"),
        (Format::Html, "<pre><code>A\n</code></pre><p></p>"),
        (Format::Html, "<p><img src='keep'></p>"),
        (Format::Html, "<p>A<svg><text>keep</text></svg>B</p>"),
        (Format::Html, "<table><tr><td>keep</td></tr></table>"),
        (Format::Html, "<p><object><p>keep</p></object></p>"),
        (Format::Html, "<table>foster<tr><td>keep</td></tr></table>"),
        (
            Format::Html,
            "<p>A</p><table><b>B<tr><td>keep</td></tr></table><p>C</p>",
        ),
        (Format::Rtf, r"{\rtf1{\b {\i }}}"),
        (Format::Rtf, r"{\rtf1{\b A}{\i }\par B}"),
        (Format::Rtf, r"{\rtf1{\pict\pngblip keep}}"),
        (
            Format::Rtf,
            r"{\rtf1{\field{\*\fldinst HYPERLINK keep}{\fldrslt X}}}",
        ),
        (Format::Markdown, "```\n\n```"),
        (Format::Markdown, "> ```\n> \n> ```"),
        (Format::Markdown, "- ```\n  A\n  ```"),
    ];
    let mut failures = Vec::new();
    for (format, source) in fixtures {
        let original = document(format, source);
        for at in (0..=original.text().len()).filter(|at| original.text_point(*at).is_ok()) {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                for input in ["X", " ", "é"] {
                    let mut core = Core::new(document(format, source));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
                    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                        .unwrap();
                    let placement = core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset: at,
                            affinity,
                            extend_selection: false,
                        },
                    );
                    // Only the shaping sides that actually exist are visual
                    // caret locations (e.g. the first glyph has no upstream side).
                    if matches!(
                        placement,
                        Err(viem_core::CoreError::Layout(
                            viem_core::layout::LayoutError::NotACaretStop { .. }
                        ))
                    ) {
                        continue;
                    }
                    placement.unwrap();
                    if let Err(error) = core.handle(view, CoreEvent::Input(InputEvent::text(input)))
                    {
                        failures.push(format!(
                            "{format:?} {source:?} at {at} {affinity:?} {input:?}: {error:?}"
                        ));
                        continue;
                    }
                    let mut expected = original.text().to_owned();
                    expected.insert_str(at, input);
                    let actual = core.document().text();
                    // HTML's authored edge spaces intentionally use NBSP.
                    assert_eq!(
                        actual.replace('\u{a0}', " "),
                        expected,
                        "{format:?} {source:?} at {at} {affinity:?} {input:?}"
                    );
                    assert_eq!(
                        core.command_state(view).unwrap().cursor(),
                        at + actual.len() - original.text().len()
                    );
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                        .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn atomic_objects_own_only_their_complete_source_construct() {
    for (format, source, expected) in [
        (
            Format::Html,
            "<p>A<img src='keep'>B</p><!--tail-->",
            "<p>AXB</p><!--tail-->",
        ),
        (
            Format::Html,
            "<p>A<svg><text>keep</text></svg>B</p>",
            "<p>AXB</p>",
        ),
        (
            Format::Html,
            "<div><table><tr><td>keep</td></tr></table></div>",
            "<div>X</div>",
        ),
        (Format::Html, "<p>A<svg/>B</p>", "<p>AXB</p>"),
        (
            Format::Html,
            "<table>foster<tr><td>keep</td></tr></table>",
            "fosterX",
        ),
        (
            Format::Html,
            "<table><b>foster</b><tr><td>keep</td></tr></table>",
            "<b>fosterX</b>",
        ),
        (
            Format::Rtf,
            r"{\rtf1 A{\pict\pngblip keep}B}",
            r"{\rtf1 AXB}",
        ),
        (
            Format::Rtf,
            r"{\rtf1 A{\field{\*\fldinst keep}{\fldrslt X}}B}",
            r"{\rtf1 AXB}",
        ),
    ] {
        for replacement in ["X", ""] {
            let mut doc = document(format, source);
            let at = doc.text().find('\u{fffc}').unwrap();
            let mut text = doc.text().to_owned();
            text.replace_range(at..at + '\u{fffc}'.len_utf8(), replacement);
            doc.replace(at..at + '\u{fffc}'.len_utf8(), replacement)
                .unwrap();
            assert_eq!(doc.text(), text);
            assert_eq!(
                doc.source_bytes(),
                expected.replace('X', replacement).as_bytes()
            );
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn selected_fostered_text_and_its_atomic_owner_share_one_minimal_patch_plan() {
    let source = "<table data-keep='yes'><b>foster</b><tr><td>inside</td></tr></table><!--tail-->";
    for start in 0..=6 {
        let mut doc = document(Format::Html, source);
        doc.replace(start..9, "X").unwrap();
        assert_eq!(doc.text(), format!("{}X", &"foster"[..start]));
        let reopened =
            Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), doc.text());
        assert!(String::from_utf8(doc.source_bytes())
            .unwrap()
            .ends_with("<!--tail-->"));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn typing_after_unclosed_objects_materializes_only_missing_closing_syntax() {
    for (format, source, expected) in [
        (Format::Html, "<table>", "<table></table>X"),
        (
            Format::Html,
            "<p><svg><text>keep",
            "<p><svg><text>keep</text></svg>X",
        ),
        (Format::Rtf, r"{\rtf1{\pict keep", r"{\rtf1{\pict keep}X"),
    ] {
        let mut doc = document(format, source);
        let at = doc.text().len();
        doc.replace(at..at, "X").unwrap();
        assert_eq!(doc.text(), "\u{fffc}X");
        assert_eq!(doc.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn empty_code_bodies_preserve_existing_line_endings_and_literal_delimiters() {
    for (source, expected) in [
        ("```\n\n```", "```\nX\n```"),
        ("```\n```", "```\nX\n```"),
        ("```", "```\nX"),
        ("> ```\n> \n> ```", "> ```\n> X\n> ```"),
        ("> ```\n> ```", "> ```\n> X\n> ```"),
        ("```\r\n\r\n```", "```\r\nX\r\n```"),
    ] {
        let mut doc = document(Format::Markdown, source);
        doc.replace(0..0, "X").unwrap();
        assert_eq!(doc.text(), "X");
        assert_eq!(doc.source_bytes(), expected.as_bytes());
    }
    for source in ["```\n\n```", "> ```\n> \n> ```"] {
        let mut doc = document(Format::Markdown, source);
        doc.replace(0..0, "```").unwrap();
        assert_eq!(doc.text(), "```");
        assert!(String::from_utf8(doc.source_bytes()).unwrap().starts_with(
            if source.starts_with('>') {
                "> ````"
            } else {
                "````"
            }
        ));
    }
}

#[test]
fn pending_style_changes_keep_object_edge_carets_editable() {
    use viem_core::document::SemanticInlineStyle;
    for source in [
        "<p><b><img src='keep'></b></p>",
        "<p><b><object><p>keep</p></object></b></p>",
        "<p><b><svg><text>keep",
    ] {
        let mut core = Core::new(document(Format::Html, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 3,
                affinity: BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style: SemanticInlineStyle::Strong,
                enabled: false,
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(core.document().text(), "\u{fffc}X");
    }
}

#[test]
fn exact_fostered_child_source_boundaries_take_priority_over_outer_atomic_ownership() {
    let doc = document(
        Format::Html,
        "<table>foster<tr><td>inside</td></tr></table>",
    );
    for source_at in 7..=13 {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            let point = doc
                .projection()
                .map_source_boundary(doc.revision(), source_at, affinity)
                .unwrap();
            assert_eq!(point.formatted_offset, source_at - 7);
        }
    }
}

#[test]
fn empty_insertions_leave_unfinished_source_constructs_byte_exact() {
    for (format, source) in [
        (Format::Html, "<table>"),
        (Format::Rtf, r"{\rtf1{\pict keep"),
        (Format::Markdown, "```"),
        (Format::Markdown, "```\n```"),
    ] {
        let mut doc = document(format, source);
        let end = doc.text().len();
        doc.replace(end..end, "").unwrap();
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}
