use viem_core::document::{Document, Encoding, Format, StyleApplication};

fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn destination(document: &Document, needle: &str) -> Option<String> {
    let at = document.text().find(needle).unwrap();
    document.link_at(document.text_point(at).unwrap()).unwrap()
}

#[test]
fn collapsed_reference_images_consume_the_complete_source_construct() {
    for (label, definition, alt) in [
        ("foo", "foo", "foo"),
        ("*foo* bar", "*foo* bar", "foo bar"),
        ("Foo", "foo", "Foo"),
        ("^1", "^1", "^1"),
    ] {
        let source = format!("First ![{label}][] Second\n\n[{definition}]: /url\n");
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(&source, format);
            if format == Format::Markdown {
                assert_eq!(
                    doc.text(),
                    format!("First \u{fffc} Second\n[{definition}]: /url")
                );
            }
            let images = doc
                .projection()
                .inline_images_for_region(&(0..doc.text().len()));
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].text, alt);
            assert_eq!(images[0].destination, "/url");
            if format == Format::MarkdownSource {
                assert_eq!(
                    &doc.text()[images[0].range.clone()],
                    format!("![{label}][]")
                );
            }
            assert_eq!(doc.source_bytes(), source.as_bytes());
            if label == "foo" && format == Format::Markdown {
                let mut edited = doc;
                edited.replace(images[0].range.clone(), "").unwrap();
                let saved = edited.source_bytes();
                assert_eq!(edited.text(), "First  Second\n[foo]: /url");
                assert_eq!(
                    edited.text(),
                    document(std::str::from_utf8(&saved).unwrap(), format).text()
                );
                assert!(edited.undo());
                assert_eq!(edited.source_bytes(), source.as_bytes());
                assert!(edited.redo());
                assert_eq!(edited.source_bytes(), saved);
            }
        }
    }
}

#[test]
fn inline_links_follow_raw_html_autolink_reference_and_line_ending_precedence() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in [
            "[a <b x=\"](c)\">",
            "[link](<foo\nbar>)",
            "[a[ref]](url)\n\n[ref]: inner",
        ] {
            let doc = document(source, format);
            assert!(
                doc.projection()
                    .style_spans()
                    .iter()
                    .all(|span| span.application != StyleApplication::Automatic("Link".into())),
                "{source:?} {format:?}"
            );
        }
        let source = "[foo<https://example.com/?search=](uri)>";
        let doc = document(source, format);
        assert_eq!(
            destination(&doc, "https"),
            Some("https://example.com/?search=](uri)".into()),
            "{format:?}"
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
        let doc = document("[a <b x=\"]\">c</b>](u)", format);
        assert_eq!(destination(&doc, "a "), Some("u".into()));
        if format == Format::Markdown {
            assert_eq!(doc.text(), "a c");
        }
    }
}

#[test]
fn markdown_reference_decoding_is_strict_in_prose_and_link_destinations() {
    for source in ["&#87654321;", "&#00000000;", "&#x0000000;"] {
        assert_eq!(document(source, Format::Markdown).text(), source);
    }
    for (raw, expected) in [
        ("foo&copy", "foo&copy"),
        ("foo&#123", "foo&#123"),
        ("foo&#87654321;", "foo&#87654321;"),
        ("foo&#x0000000;", "foo&#x0000000;"),
        ("foo&copy;&#123;", "foo©{"),
        ("foo&#0000123;&#x00007B;", "foo{{"),
    ] {
        let source = format!("[a]({raw})");
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(&source, format);
            assert_eq!(
                destination(&doc, "a"),
                Some(expected.into()),
                "{source} {format:?}"
            );
        }
    }
}

#[test]
fn parenthesized_link_titles_reject_unescaped_nested_openers() {
    for source in ["[a](url (tit(le))", "[a](url (tit(le)))"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert_eq!(doc.text(), source);
            assert_eq!(destination(&doc, "a"), None);
        }
    }
    for source in ["[a](url (title))", r"[a](url (tit\(le))"] {
        let doc = document(source, Format::Markdown);
        assert_eq!(doc.text(), "a");
        assert_eq!(destination(&doc, "a"), Some("url".into()));
    }
}

#[test]
fn code_span_edge_space_exception_counts_only_ascii_spaces() {
    for (source, expected) in [("` \t `", "\t"), ("` \u{a0} `", "\u{a0}"), ("`   `", "   ")] {
        let doc = document(source, Format::Markdown);
        assert_eq!(doc.text(), expected);
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn paragraph_splits_after_styled_folded_spaces_preserve_the_retained_body() {
    for source in [
        "one *two\nthree* [label](url) `x`",
        "one *two three* [label](url) `x`",
    ] {
        let mut doc = document(source, Format::Markdown);
        doc.replace(8..8, "\n").unwrap();
        assert_eq!(doc.text(), "one two \nthree label x");
        assert_eq!(
            doc.text(),
            document(
                std::str::from_utf8(&doc.source_bytes()).unwrap(),
                Format::Markdown
            )
            .text()
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn comment_replacement_breaks_preserve_adjacent_visible_spaces() {
    for range in [2..13, 12..13] {
        let source = "a <!--keep--> b";
        let mut doc = document(source, Format::Markdown);
        let mut expected = doc.text().to_owned();
        expected.replace_range(range.clone(), "\n");
        doc.replace(range, "\n").unwrap();
        assert_eq!(doc.text(), expected);
        assert_eq!(
            doc.text(),
            document(
                std::str::from_utf8(&doc.source_bytes()).unwrap(),
                Format::Markdown
            )
            .text()
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn extended_email_autolinks_allow_digits_and_internal_domain_underscores() {
    for address in ["a@b.c1", "a@foo_bar.baz", "a@foo.b_ar", "a@foo.bar_"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(address, format);
            let expected = (!address.ends_with('_')).then(|| format!("mailto:{address}"));
            assert_eq!(destination(&doc, "a"), expected, "{address:?} {format:?}");
        }
    }
}

#[test]
fn unmatched_url_parentheses_are_trimmed_in_bounded_opening_time() {
    let source = format!("http://example.com/{}", ")".repeat(128_000));
    let started = std::time::Instant::now();
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(&source, format);
        assert_eq!(
            destination(&doc, "http"),
            Some("http://example.com/".into())
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
