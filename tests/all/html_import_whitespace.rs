use viem_core::document::FontSlant;
use viem_core::document::{Document, DocumentError, Encoding, Format};
use viem_core::layout::DocumentLayoutStyles;

fn assert_projection(source: &str, expected: &str) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), expected, "source: {source:?}");
    assert_eq!(
        document.source_bytes(),
        source.as_bytes(),
        "no-op source preservation"
    );
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), expected);
    assert_eq!(
        reopened.projection().provenance(),
        document.projection().provenance()
    );
}

#[test]
fn normal_whitespace_collapses_across_inline_boundaries() {
    for source in [
        "<p>  a  b  </p>",
        "<p>\t a \t b \t</p>",
        "<p>\n a \n\n b \n</p>",
        "<p>\r\n a \r\n b \r\n</p>",
        "<p>\r a \r b \r</p>",
        "<p>a<span> </span>b</p>",
        "<p>a <span> </span> b</p>",
        "<p>a<span> \t\n </span>b</p>",
        "<p><span> a </span> <span> b </span></p>",
        "<p><b> a </b> \t<i> b </i></p>",
        "<p>a <!--keep--> <unknown data-x='1'> </unknown> b</p>",
        "<p>a&#32;&#9;&#10;&#13;b</p>",
    ] {
        assert_projection(source, "a b");
    }
}

#[test]
fn collapsed_space_retains_the_style_of_the_surviving_segment_break() {
    let source = "<p><b>a </b><i>\n </i><span>\n b</span></p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "a b");
    let space_style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
    assert!(!space_style.bold);
    assert_eq!(space_style.slant, FontSlant::Italic);
}

#[test]
fn block_and_hard_break_edges_trim_collapsible_whitespace() {
    for (source, expected) in [
        ("<p> \t\n </p>", ""),
        ("<p><span> </span></p>", ""),
        ("<p> a <br> b </p>", "a\nb"),
        ("<p> <br> a </p>", "\na"),
        ("<p> a <br> </p>", "a\n"),
        ("<p> a </p> \n <p> b </p>", "a\nb"),
        ("a <div> b </div> c", "a\nb\nc"),
        ("<ul> <li> a </li> <li> b </li> </ul>", "a\nb"),
    ] {
        assert_projection(source, expected);
    }
}

#[test]
fn preserved_spaces_do_not_collapse_with_neighboring_normal_spaces() {
    for (source, expected) in [
        (
            "<p>a <span style='white-space:pre-wrap'> </span> b</p>",
            "a   b",
        ),
        ("<p> <span style='white-space:pre'> </span> </p>", " "),
        ("<p>a <span style='white-space:pre'>\n</span> b</p>", "a\nb"),
        (
            "<p>a<span style='white-space:pre'> \n </span>b</p>",
            "a \n b",
        ),
    ] {
        assert_projection(source, expected);
    }
}

#[test]
fn whitespace_modes_inherit_and_support_explicit_overrides() {
    for mode in ["pre", "pre-wrap", "break-spaces"] {
        assert_projection(
            &format!("<p style='white-space:{mode}'> a \t\n b </p>"),
            " a \t\n b ",
        );
        assert_projection(
            &format!("<p style='white-space:{mode}'><span> a \t\n b </span></p>"),
            " a \t\n b ",
        );
    }
    for mode in ["normal", "nowrap"] {
        assert_projection(
            &format!("<pre style='white-space:{mode}'> a \t\n b </pre>"),
            "a b",
        );
    }
    assert_projection(
        "<p style='white-space:pre-line'> a \t\n \t b \n\n c \t</p>",
        "a\nb\n\nc",
    );
    assert_projection(
        "<p style='white-space:pre-line'>a<span> \t\n </span>b</p>",
        "a\nb",
    );
    assert_projection("<p style='white-space:pre-line'> \n a \n </p>", "\na\n");
    assert_projection(
        "<pre> a<span style='white-space:normal'> \t\n b </span> c </pre>",
        " a b  c ",
    );
    assert_projection("<pre style='white-space:inherit'> a \t\n b </pre>", "a b");
    assert_projection("<pre style='white-space:unset'> a \t\n b </pre>", "a b");
    assert_projection(
        "<pre><span style='white-space:initial'> a \t\n b </span></pre>",
        "a b",
    );
    assert_projection(
        "<p style='white-space:pre;white-space:normal'> a  b </p>",
        "a b",
    );
    assert_projection(
        "<p style='white-space:pre !important;white-space:normal'> a  b </p>",
        " a  b ",
    );
}

#[test]
fn nbsp_and_non_css_whitespace_characters_are_not_collapsed() {
    assert_projection(
        "<p>&nbsp; a&nbsp;&nbsp;b &nbsp;</p>",
        "\u{a0} a\u{a0}\u{a0}b \u{a0}",
    );
    for character in [
        '\u{c}', '\u{b}', '\u{85}', '\u{2002}', '\u{2003}', '\u{2028}', '\u{2029}', '\u{202f}',
        '\u{3000}',
    ] {
        assert_projection(
            &format!("<p>{character}a{character}{character}b{character}</p>"),
            &format!("{character}a{character}{character}b{character}"),
        );
    }
    assert_projection("<p>a&#12;b</p>", "a\u{c}b");
}

#[test]
fn html_newline_normalization_precedes_css_whitespace_processing() {
    assert_projection("<pre>\r\na\r\nb\rc</pre>", "a\nb\nc");
    assert_projection("<pre>a&#13;b</pre>", "a b");
    assert_projection(
        "<p style='white-space:pre-line'>a&#13;b&#10;c</p>",
        "a b\nc",
    );
}

#[test]
fn exact_cr_insertion_is_rejected_atomically_when_html_cannot_reproject_it() {
    for source in [
        "<p>ab</p>",
        "<pre>ab</pre>",
        "<p style='white-space:pre-wrap'>ab</p>",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let before = document.revision();
        assert_eq!(
            document.insert(1, "\r"),
            Err(DocumentError::UnrepresentableFormattedCharacter { format: Format::Html, character: '\r' })
        );
        assert_eq!(document.text(), "ab");
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision(), before);
        assert!(!document.undo());
    }
}

#[test]
fn inline_atomic_objects_separate_whitespace_runs() {
    for (source, expected) in [
        ("<p>a <img src='x'> b</p>", "a \u{fffc} b"),
        ("<p>a <img src='x'></p>", "a \u{fffc}"),
        ("<p> <img src='x'> </p>", "\u{fffc}"),
        ("<p>a <span><img src='x'></span> b</p>", "a \u{fffc} b"),
    ] {
        assert_projection(source, expected);
    }
}
