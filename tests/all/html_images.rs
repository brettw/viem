use viem_core::document::{Document, Encoding, Format, ImageEditIntent};

fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn html_images_share_atomic_projection_and_literal_source_metadata() {
    let source = "before <IMG SRC='local&amp;file.png' ALT='a &quot;cat&quot;' WIDTH=320 HEIGHT=180 title='kept'> after";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(source, format);
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert_eq!(doc.text(), if format == Format::Markdown { "before \u{fffc} after" } else { source });
        let images = doc.projection().inline_images_for_region(&(0..doc.text().len()));
        assert_eq!(images.len(), 1);
        let image = &images[0];
        assert_eq!(image.destination, "local&file.png");
        assert_eq!(image.text, "a \"cat\"");
        assert_eq!((image.width, image.height), (Some(320), Some(180)));
        assert_eq!(image.source_view, format == Format::MarkdownSource);
        assert_eq!(image.range, if image.source_view { 7..source.len() - 6 } else { 7..10 });
        assert_eq!(doc.image_snapshot_at(doc.text_point(7).unwrap()).unwrap().unwrap().destination, image.destination);
        if format == Format::Markdown { assert!(doc.text_point(8).is_err()); }
    }
}

#[test]
fn html_images_project_inside_blocks_and_multiline_tags() {
    for (source, expected, count) in [
        ("<img src=local.png>", "\u{fffc}", 1),
        ("<img\n src=local.png\n width='200'\n height=100>", "\u{fffc}", 1),
        ("<div><p><img src='first.png'></p><p>text <img src='second.png'></p></div>", "\u{fffc}\ntext \u{fffc}", 2),
        ("before <img\n src='local.png' width='200'> after", "before \u{fffc} after", 1),
        ("<div>\n<img src='local.png'>\n</div>", "\u{fffc}", 1),
        ("# <img src='heading.png'>", "\u{fffc}", 1),
        ("- <img src='list.png'>", "\u{fffc}", 1),
        ("> <img src='quote.png'>", "\u{fffc}", 1),
        ("> <img\n> src='quote.png'\n> width='123'>", "\u{fffc}", 1),
        ("- <img\n  src='list.png'\n  width='123'>", "\u{fffc}", 1),
        ("- parent\n  - <img\n    src='nested.png'\n    width='123'>", "parent\n\u{fffc}", 1),
        ("> - <img\n>   src='quote-list.png'\n>   width='123'>", "\u{fffc}", 1),
        ("- > <img\n  > src='list-quote.png'\n  > width='123'>", "\u{fffc}", 1),
        ("- > <img alt='café'\n  > src='list-quote.png'\n  > width='123'>", "\u{fffc}", 1),
        ("<img src='local.png' title='<table>'>", "\u{fffc}", 1),
        ("<img src='local.png' data-value='<table>'>", "\u{fffc}", 1),
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert_eq!(doc.source_bytes(), source.as_bytes(), "{source} {format:?}");
            let images = doc.projection().inline_images_for_region(&(0..doc.text().len()));
            assert_eq!(images.len(), count, "{source} {format:?}");
            if source.contains("width='123'") { assert_eq!(images.last().unwrap().width, Some(123), "{source} {format:?}"); }
            if format == Format::Markdown { assert_eq!(doc.text(), expected, "{source}"); }
            else { assert_eq!(doc.text(), source, "{source}"); }
        }
    }
}

#[test]
fn html_image_dimensions_are_positive_integers_and_cannot_wrap() {
    for (value, expected) in [
        ("1", Some(1)), ("00120", Some(120)), ("&#49;00", Some(100)),
        ("4294967295", Some(u32::MAX)), ("999999999999999999999999999999999999", Some(u32::MAX)),
        ("0", None), ("", None), ("-1", None), ("+12", None), ("12.5", None),
        ("20px", None), ("50%", None), (" 100 ", None),
    ] {
        let source = format!("before <img src='local.png' width='{value}' height='{value}'>");
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(&source, format);
            let images = doc.projection().inline_images_for_region(&(0..doc.text().len()));
            assert_eq!((images[0].width, images[0].height), (expected, expected), "{value} {format:?}");
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
    let doc = document("![ordinary](local.png)", Format::Markdown);
    let images = doc.projection().inline_images_for_region(&(0..doc.text().len()));
    assert_eq!((images[0].width, images[0].height), (None, None));
}

#[test]
fn html_images_require_a_source_and_exclude_literal_or_opaque_contexts() {
    for source in [
        "<img>", "<img src>", "<img src=''>", "<img src='  '>", "</img>",
        "`<img src='local.png'>`", "```html\n<img src='local.png'>\n```",
        "    <img src='local.png'>", "\\<img src='local.png'>", "<!-- <img src='local.png'> -->",
        "<script><img src='local.png'></script>", "before <script><img src='local.png'></script> after",
        "<style><img src='local.png'></style>",
        "before <script>ignored\n<img src='local.png'></script> after",
        "before <textarea>ignored\n<img src='local.png'></textarea> after",
        "- > `<img\n  > src='local.png'>`",
        "- > `prefix\n  > <img\n  > src='local.png'>\n  > suffix`",
        "- > `prefix\n  > <img src='local.png'>\n  > suffix`",
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert!(doc.projection().inline_images_for_region(&(0..doc.text().len())).is_empty(), "{source} {format:?}");
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn raw_inline_html_context_ends_with_its_paragraph() {
    for tag in ["script", "textarea"] {
        let source = format!("before <{tag}>ignored\n\n</{tag}>\n\nafter <img src=after.png>");
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(&source, format);
            let images = doc.projection().inline_images_for_region(&(0..doc.text().len()));
            assert_eq!(images.len(), 1, "{tag} {format:?}");
            assert_eq!(images[0].destination, "after.png");
        }
    }
    for source in ["before `<script>`\n<img src=after.png>", "before \\<script>\n<img src=after.png>"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert_eq!(doc.projection().inline_images_for_region(&(0..doc.text().len())).len(), 1, "{source} {format:?}");
        }
    }
}

#[test]
fn html_image_appearance_does_not_override_structural_html_owners() {
    for source in ["<h1><img src='local.png'></h1>", "<pre><img src='local.png'></pre>", "<ul><li><img src='local.png'></li></ul>"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert_ne!(doc.projection().blocks()[0].style.0, "Image", "{source} {format:?}");
        }
    }
}

#[test]
fn html_image_only_paragraphs_use_image_appearance_in_both_views() {
    for source in ["<img src='local.png'>", "<p><img src='local.png'></p>", "<div>\n<img src='local.png'>\n</div>",
        "<img src='one.png'> <img src='two.png'>", "**<img src='local.png'>**"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            assert_eq!(doc.projection().blocks()[0].style.0, "Image", "{source} {format:?}");
        }
    }
}

#[test]
fn html_image_metadata_matches_fresh_projection_after_local_edits_and_undo() {
    let source = "before <img src='local.png' width=120 height=80> after\n\nuntouched";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document(source, format);
        doc.insert(0, "prefix ").unwrap();
        let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(doc.text(), fresh.text());
        assert_eq!(doc.projection().inline_images_for_region(&(0..doc.text().len())), fresh.projection().inline_images_for_region(&(0..fresh.text().len())));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.text(), fresh.text());
    }
}

#[test]
fn nested_list_quote_images_keep_owner_identity_during_location_edits() {
    let source = "- > <img\n  > src='nested.png'\n  > width='123' height=45>";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document(source, format);
        let image = doc.projection().inline_images_for_region(&(0..doc.text().len())).remove(0);
        assert_eq!(doc.projection().blocks().iter().find(|block| block.range.contains(&image.range.start)).unwrap().quote_depth, 1);
        let (edit, _) = doc.prepare_image_edit(doc.id(), doc.revision(), ImageEditIntent::Edit {
            range: image.range, text: image.text, destination: "updated.png".into(),
        }).unwrap();
        doc.commit_model_transaction(edit).unwrap();
        assert_eq!(doc.source_bytes(), source.replace("'nested.png'", "\"updated.png\"").as_bytes());
        let image = doc.projection().inline_images_for_region(&(0..doc.text().len())).remove(0);
        assert_eq!((image.width, image.height), (Some(123), Some(45)));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}
