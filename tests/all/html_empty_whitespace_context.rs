use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn type_at_end(source: &str, typed: &str, expected_source: &str, expected_text: &str) {
    type_at(source, None, &[typed], expected_source, expected_text);
}

fn type_at(
    source: &str,
    point: Option<(usize, BoundaryAffinity)>,
    typed: &[&str],
    expected_source: &str,
    expected_text: &str,
) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let (text_offset, affinity) =
        point.unwrap_or((core.document().text().len(), BoundaryAffinity::Upstream));
    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset,
            affinity,
            extend_selection: false,
        },
    )
    .unwrap();
    for text in typed {
        core.handle(view, CoreEvent::Input(InputEvent::text(*text)))
            .unwrap();
    }
    let description = source.chars().take(100).collect::<String>();
    assert!(
        core.document().text() == expected_text,
        "formatted text differs: {description}"
    );
    assert!(
        core.document().source_bytes() == expected_source.as_bytes(),
        "source differs: {description}"
    );
    let reopened =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), expected_text);
    assert_eq!(
        reopened.projection().style_spans(),
        core.document().projection().style_spans()
    );
    for input in [InputEvent::Key(Key::Escape), InputEvent::key('u')] {
        core.handle(view, CoreEvent::Input(input)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
}

#[test]
fn normal_spaces_beside_preserved_spaces_need_no_nonbreaking_escape() {
    for (source, at, affinity, expected) in [
        (
            "<p><span style='white-space:pre'>A </span>B</p>",
            2,
            BoundaryAffinity::Downstream,
            "<p><span style='white-space:pre'>A </span> B</p>",
        ),
        (
            "<p>A<span style='white-space:pre'> B</span></p>",
            1,
            BoundaryAffinity::Upstream,
            "<p>A <span style='white-space:pre'> B</span></p>",
        ),
    ] {
        type_at(source, Some((at, affinity)), &[" "], expected, "A  B");
    }
}

#[test]
fn completing_a_word_after_preserved_space_compacts_only_the_generated_nbsp() {
    type_at(
        "<p><span style='white-space:pre'>A </span><b></b></p>",
        None,
        &[" ", "B"],
        "<p><span style='white-space:pre'>A </span><b> B</b></p>",
        "A  B",
    );
}

#[test]
fn empty_pre_css_overrides_use_the_computed_whitespace_mode() {
    for mode in [
        "normal", "nowrap", "pre-line", "initial", "inherit", "unset",
    ] {
        let source = format!("<pre style='white-space:{mode}'></pre>");
        let expected = format!("<pre style='white-space:{mode}'>&nbsp;</pre>");
        type_at_end(&source, " ", &expected, "\u{a0}");
    }
    for mode in ["inherit", "unset"] {
        let source =
            format!("<div style='white-space:pre'><pre style='white-space:{mode}'></pre></div>");
        let expected = format!(
            "<div style='white-space:pre'><pre style='white-space:{mode}'> \t </pre></div>"
        );
        type_at_end(&source, " \t ", &expected, " \t ");
    }
}

#[test]
fn empty_sibling_inline_scopes_recover_their_actual_ancestor_mode() {
    for (source, expected_source, expected_text) in [
        (
            "<pre><span>A</span><b></b></pre>",
            "<pre><span>A</span><b> </b></pre>",
            "A ",
        ),
        (
            "<p><span style='white-space:pre-wrap'>A</span><b></b></p>",
            "<p><span style='white-space:pre-wrap'>A</span><b>&nbsp;</b></p>",
            "A\u{a0}",
        ),
        (
            "<pre><span style='white-space:normal'>A</span><b></b></pre>",
            "<pre><span style='white-space:normal'>A</span><b> </b></pre>",
            "A ",
        ),
        (
            "<pre><span>A</span><b style='white-space:normal'></b></pre>",
            "<pre><span>A</span><b style='white-space:normal'>&nbsp;</b></pre>",
            "A\u{a0}",
        ),
        (
            "<p><span style='white-space:pre'></span><b></b></p>",
            "<p><span style='white-space:pre'> </span><b></b></p>",
            " ",
        ),
        (
            "<p><span></span><b style='white-space:pre'></b></p>",
            "<p><span>&nbsp;</span><b style='white-space:pre'></b></p>",
            "\u{a0}",
        ),
    ] {
        type_at_end(source, " ", expected_source, expected_text);
    }
}

#[test]
fn empty_inline_context_survives_long_ancestors_without_prefix_reparsing() {
    let text = "a".repeat(200_000);
    for (opening, closing, expected_space) in [
        ("<pre><span>", "</span><b></b></pre>", " "),
        (
            "<p><span style='white-space:pre-wrap'>",
            "</span><b></b></p>",
            "&nbsp;",
        ),
    ] {
        let source = format!("{opening}{text}{closing}");
        let expected = format!(
            "{opening}{text}{}",
            closing.replacen("<b></b>", &format!("<b>{expected_space}</b>"), 1)
        );
        let rendered_space = if expected_space == " " { " " } else { "\u{a0}" };
        type_at_end(&source, " ", &expected, &format!("{text}{rendered_space}"));
    }
}
