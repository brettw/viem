//! Bare URLs and email addresses entered in Markdown Preview. GFM shows a
//! backslash escape or character reference inside an extended autolink
//! literally, so every input path keeps the address unescaped there. Pasted
//! plain text is inserted like typed text: the destination decides whether a
//! URL is a link, and the paste never rewrites it as an explicit link.
use crate::support::{assert_exact_undo_redo, assert_regional, assert_reopens_as_displayed, Editor};
use viem_core::document::{Document, Encoding, Format, StyleApplication};

const URLS: [&str; 6] = [
    "https://example.com/foo_bar/baz",
    "https://en.wikipedia.org/wiki/Snake_case",
    "www.example.com/a_b",
    "https://example.com/a*b~c",
    "https://example.com/?q=a_b&r=1",
    r"https://example.com/a\b",
];

#[derive(Clone, Copy, Debug)]
enum Input {
    Typed,
    Paste,
    PasteMatchingStyle,
}

fn enter(editor: &mut Editor, input: Input, text: &str) {
    match input {
        Input::Typed => editor.type_keys(text),
        Input::Paste => editor.paste_plain(text),
        Input::PasteMatchingStyle => editor.paste_matching_style(text),
    }
}

fn links(document: &Document) -> Vec<String> {
    let text = document.text();
    document
        .projection()
        .style_spans()
        .iter()
        .filter(|span| span.application == StyleApplication::Automatic("Link".into()))
        .map(|span| text[span.range.clone()].to_owned())
        .collect()
}

/// Open `source` in Insert at the boundary between its two adjacent spaces.
fn between_spaces(source: &str) -> Editor {
    let mut editor = Editor::open(source, Format::Markdown);
    let at = editor.document().text().find("  ").expect("two spaces") + 1;
    editor.place(at);
    editor.type_keys("i");
    editor
}

#[test]
fn bare_addresses_stay_unescaped_through_every_input_path_and_context() {
    let email = ["a_b@example.com"];
    for source in ["see  here", "- see  here", "# see  here", "> see  here", "**see  here**"] {
        for input in [Input::Typed, Input::Paste, Input::PasteMatchingStyle] {
            // A typed email first exists as a prefix that is not an address.
            let addresses = URLS.iter().chain(if matches!(input, Input::Typed) { &[][..] } else { &email[..] });
            for address in addresses {
                let context = format!("{address:?} {input:?} into {source:?}");
                let mut editor = between_spaces(source);
                enter(&mut editor, input, address);
                let at = source.find("  ").unwrap() + 1;
                let expected = format!("{}{address}{}", &source[..at], &source[at..]);
                if matches!(input, Input::Typed) && source.starts_with("**") && address.contains('*') {
                    // While typed, `a*` ends the URL with a closing delimiter
                    // that cannot be escaped there. Strong is respelled as
                    // passive HTML to keep the requested text and styles.
                    let body = &expected[2..expected.len() - 2];
                    assert_eq!(editor.source(), format!("<strong>{body}</strong>"), "{context}");
                } else {
                    assert_eq!(editor.source(), expected, "{context}");
                }
                assert_eq!(links(editor.document()), [*address], "{context}");
                assert_reopens_as_displayed(editor.document(), &context);
                if !matches!(input, Input::Typed) {
                    assert_exact_undo_redo(&mut editor, source, &expected, &context);
                }
            }
        }
    }
}

#[test]
fn pasted_plain_text_is_inserted_like_typed_text_beside_words() {
    let url = "https://example.com/foo_bar";
    for (source, keys, expected, linked) in [
        // A URL cannot begin after a word: it is ordinary text there.
        ("hello world", "A", "hello worldhttps://example.com/foo_bar", None),
        // Following word characters belong to the URL, as in the source.
        ("more text", "i", "https://example.com/foo_barmore text", Some("https://example.com/foo_barmore")),
    ] {
        for input in [Input::Paste, Input::PasteMatchingStyle] {
            let context = format!("{input:?} into {source:?}");
            let mut editor = Editor::open(source, Format::Markdown);
            editor.type_keys(keys);
            enter(&mut editor, input, url);
            assert_eq!(editor.source(), expected, "{context}");
            assert_eq!(links(editor.document()), linked.into_iter().collect::<Vec<_>>(), "{context}");
            assert_reopens_as_displayed(editor.document(), &context);
            assert_exact_undo_redo(&mut editor, source, expected, &context);
        }
    }
}

#[test]
fn edits_inside_an_autolink_keep_it_automatic() {
    for inserted in ["_", "*", "~", "_x", r"\", "&amp;", "`"] {
        for input in [Input::Typed, Input::Paste, Input::PasteMatchingStyle] {
            if matches!(input, Input::Paste) && inserted == "*" {
                // Pasted alone, `*` is Markdown for an empty list item.
                continue;
            }
            let context = format!("{inserted:?} {input:?}");
            let source = "see https://example.com/foobar here";
            let mut editor = Editor::open(source, Format::Markdown);
            editor.place_at("bar");
            editor.type_keys("i");
            enter(&mut editor, input, inserted);
            // Pasted Markdown decodes its reference; other paths keep text.
            let shown = if matches!(input, Input::Paste) && inserted == "&amp;" { "&" } else { inserted };
            let url = format!("https://example.com/foo{shown}bar");
            assert_eq!(editor.source(), format!("see {url} here"), "{context}");
            assert_eq!(links(editor.document()), [url], "{context}");
            assert_reopens_as_displayed(editor.document(), &context);
        }
    }
}

#[test]
fn multiline_pastes_keep_addresses_unescaped() {
    let literal = "see https://example.com/a_b\nhttps://example.com/c_d end";
    let mut editor = between_spaces("hello  world");
    editor.paste_matching_style(literal);
    let expected = "hello see https://example.com/a_b\n\nhttps://example.com/c_d end world";
    assert_eq!(editor.source(), expected);
    assert_eq!(links(editor.document()), ["https://example.com/a_b", "https://example.com/c_d"]);
    assert_reopens_as_displayed(editor.document(), "literal lines");
    assert_exact_undo_redo(&mut editor, "hello  world", expected, "literal lines");

    // The destination splits before a merged head is inserted, so the head's
    // URL never absorbs the following text into its link.
    let markdown = "https://example.com/a_b\n\n- item\n\nhttps://example.com/c_d";
    for (source, expected, tail) in [
        ("hello  world", "hello https://example.com/a_b\n\n- item\n\nhttps://example.com/c_d world", "https://example.com/c_d"),
        ("hello world", "hello https://example.com/a_b\n\n- item\n\nhttps://example.com/c_dworld", "https://example.com/c_dworld"),
    ] {
        let mut editor = if source.contains("  ") {
            between_spaces(source)
        } else {
            let mut editor = Editor::open(source, Format::Markdown);
            editor.place_at("world");
            editor.type_keys("i");
            editor
        };
        editor.paste_plain(markdown);
        assert_eq!(editor.source(), expected, "{source:?}");
        assert_eq!(links(editor.document()), ["https://example.com/a_b", tail], "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
        assert_exact_undo_redo(&mut editor, source, expected, source);
    }
}

#[test]
fn an_address_before_protected_leading_whitespace_respells_that_reference() {
    let source = "before\n\n&#32;world";
    for input in [Input::Typed, Input::Paste, Input::PasteMatchingStyle] {
        let mut editor = Editor::open(source, Format::Markdown);
        let at = editor.document().text().find(" world").unwrap();
        editor.place(at);
        editor.type_keys("i");
        enter(&mut editor, input, "https://example.com/a_b");
        let expected = "before\n\nhttps://example.com/a_b world";
        assert_eq!(editor.source(), expected, "{input:?}");
        assert_eq!(links(editor.document()), ["https://example.com/a_b"], "{input:?}");
        assert_reopens_as_displayed(editor.document(), &format!("{input:?}"));
        if !matches!(input, Input::Typed) {
            assert_exact_undo_redo(&mut editor, source, expected, &format!("{input:?}"));
        }
    }
    // A literal reference typed inside the URL is text, not old whitespace.
    let mut editor = Editor::open("see https://example.com/ab here", Format::Markdown);
    editor.place_at(" here");
    editor.type_keys("i&#32;");
    assert_eq!(editor.source(), "see https://example.com/ab&#32; here");
    assert_reopens_as_displayed(editor.document(), "literal reference");
}

#[test]
fn code_with_addresses_becomes_prose_without_visible_escapes() {
    for (source, expected, linked) in [
        ("```\nsee https://example.com/a_b here\n```", "see https://example.com/a_b here", vec!["https://example.com/a_b"]),
        ("    https://example.com/a_b/c*d", "https://example.com/a_b/c*d", vec!["https://example.com/a_b/c*d"]),
        ("```\nfirst\nhttps://example.com/a_b\n```", "first  \nhttps://example.com/a_b", vec!["https://example.com/a_b"]),
        ("> ```\n> https://example.com/a_b\n> ```", "> https://example.com/a_b", vec!["https://example.com/a_b"]),
        // After a `<br>` or protected indentation a URL is ordinary text,
        // where escapes keep applying.
        ("```\nfirst\n\nhttps://example.com/a_b\n```", r"first<br><br>https://example.com/a\_b", vec![]),
        ("```\n  https://example.com/a_b\n```", r"&#32;&#32;https://example.com/a\_b", vec![]),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let text = document.text().to_owned();
        let at = text.find("http").unwrap();
        document.set_paragraph_style(at..at, "Paragraph".into()).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(String::from_utf8(document.source_bytes()).unwrap(), expected, "{source:?}");
        assert_eq!(document.text(), text, "{source:?}");
        assert_eq!(links(&document), linked, "{source:?}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn typing_an_address_in_a_large_document_stays_regional() {
    let filler = (0..2000).map(|index| format!("- item {index} text\n\nparagraph {index} text\n\n")).collect::<String>();
    let url = "https://example.com/foo_bar";
    for target in ["item 1000", "paragraph 1000"] {
        let open = || {
            let mut editor = Editor::open(&filler, Format::Markdown);
            let at = editor.document().text().find(&format!("{target} text")).unwrap() + target.len();
            editor.place(at);
            editor.type_keys("i ");
            editor
        };
        let mut editor = open();
        for character in url.chars() {
            assert_regional(&format!("{target:?} typing {character:?}"), || editor.type_keys(&character.to_string()));
        }
        assert!(editor.document().text().contains(&format!("{target} {url} text")), "{target:?}");
        assert!(links(editor.document()).iter().any(|link| link == url), "{target:?}");
        let mut editor = open();
        assert_regional(&format!("{target:?} paste"), || editor.paste_plain(url));
        assert!(editor.source().contains(&format!("{target} {url} text")), "{target:?}");
        assert!(links(editor.document()).iter().any(|link| link == url), "{target:?}");
    }
}

#[test]
fn addresses_stay_unescaped_in_every_encoding_and_line_ending() {
    use crate::markdown_test_support::EncodedFixture;
    use viem_core::document::FileFormat;
    let url = "https://example.com/a_b/c*d";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos] {
            let fixture = EncodedFixture { encoding, file_format, bom: !matches!(encoding, Encoding::Latin1) };
            let source = "first line\n\nsee  here";
            for input in [Input::Typed, Input::Paste, Input::PasteMatchingStyle] {
                let context = format!("{fixture:?} {input:?}");
                let mut core = viem_core::Core::new(fixture.open(source));
                let view = core.add_view(viem_core::layout::MockTextMeasurementProvider::new(), 800., 2000.);
                let mut editor = Editor { core, view };
                let at = editor.document().text().find("  ").unwrap() + 1;
                editor.place(at);
                editor.type_keys("i");
                enter(&mut editor, input, url);
                assert_eq!(editor.document().source_bytes(), fixture.bytes(&format!("first line\n\nsee {url} here")), "{context}");
                assert_eq!(links(editor.document()), [url], "{context}");
                assert_reopens_as_displayed(editor.document(), &context);
            }
        }
    }
}

#[test]
fn delimiters_inside_a_url_never_pair_with_outer_emphasis() {
    for (source, text, emphasized) in [
        ("see *https://example.com/c*d* here", "see https://example.com/c*d here", "https://example.com/c*d"),
        ("*see https://example.com/c*d here*", "see https://example.com/c*d here", "see https://example.com/c*d here"),
        ("see ***https://example.com/c*d*** here", "see https://example.com/c*d here", "https://example.com/c*d"),
        ("*www.example.com/a*b*", "www.example.com/a*b", "www.example.com/a*b"),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(document.text(), text, "{source:?}");
        let url = text.split(' ').find(|word| word.contains("example.com")).unwrap();
        assert_eq!(links(&document), [url], "{source:?}");
        assert!(document.projection().style_spans().iter().any(|span| span.application
            == StyleApplication::Semantic(viem_core::document::SemanticInlineStyle::Emphasis)
            && document.text()[span.range.clone()] == *emphasized), "{source:?}");
    }
    // Brackets own their text, as GFM recognizes no autolink inside them.
    let document = Document::from_bytes(b"[*see https://example.com/c*d*](u)".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), "see https://example.com/cd*");
}

#[derive(Clone, Copy, Debug)]
enum Treatment {
    Semantic(viem_core::document::SemanticInlineStyle),
    Strikethrough,
    Property(viem_core::document::StyleProperty),
}

/// Apply a native character treatment to the Visual selection `keys` makes
/// from `anchor`.
fn treat(editor: &mut Editor, anchor: &str, keys: &str, treatment: Treatment, enabled: bool) {
    use viem_core::CoreEvent;
    editor.key(viem_core::command::Key::Escape);
    editor.place_at(anchor);
    editor.type_keys(keys);
    let expected = editor.core.list_selection_identity(editor.view).unwrap();
    let event = match treatment {
        Treatment::Semantic(style) => CoreEvent::SetSelectionSemanticStyle { expected, style, enabled },
        Treatment::Strikethrough => CoreEvent::SetStrikethrough { expected, enabled },
        Treatment::Property(property) => CoreEvent::SetInlineProperty { expected, property, enabled },
    };
    let outcome = editor.core.handle(editor.view, event)
        .unwrap_or_else(|error| panic!("{treatment:?} from {anchor:?} with {keys:?}: {error:?}"));
    assert!(!matches!(outcome.command.map(|command| command.status), Some(viem_core::command::CommandStatus::Error(_))),
        "{treatment:?} from {anchor:?} with {keys:?}");
}

#[test]
fn character_treatments_keep_bare_links() {
    use viem_core::document::{SemanticInlineStyle::{Emphasis, Strong}, StyleProperty};
    let treatments = [
        Treatment::Semantic(Strong),
        Treatment::Semantic(Emphasis),
        Treatment::Strikethrough,
        Treatment::Property(StyleProperty::CharacterUnderline),
        Treatment::Property(StyleProperty::CharacterSuperscript),
    ];
    for url in ["https://example.com/ab_cd", "https://example.com/c*d"] {
        let source = format!("see {url} here");
        // Whole URL, part of it, a phrase around it, and from inside it.
        for (anchor, keys) in [("https", "vE"), ("example", "ve"), ("see", "v$"), ("example", "v$")] {
            for treatment in treatments {
                let context = format!("{treatment:?} {url:?} from {anchor:?} with {keys:?}");
                let mut editor = Editor::open(&source, Format::Markdown);
                treat(&mut editor, anchor, keys, treatment, true);
                let document = editor.document();
                assert_eq!(document.text(), source, "{context}");
                let at = document.text_point(4).unwrap();
                assert_eq!(document.link_at(at).unwrap().as_deref(), Some(url), "{context}");
                assert_reopens_as_displayed(document, &context);
                let treated = editor.source();
                treat(&mut editor, anchor, keys, treatment, false);
                assert_eq!(editor.document().text(), source, "{context}: removed");
                assert_eq!(links(editor.document()), [url], "{context}: removed");
                editor.undo();
                assert_eq!(editor.source(), treated, "{context}: undo removal");
                editor.undo();
                assert_eq!(editor.source(), source, "{context}: undo");
            }
        }
    }
    for (treatments, expected) in [
        (&[Treatment::Semantic(Emphasis)][..], "see *https://example.com/c*d* here"),
        (&[Treatment::Semantic(Strong), Treatment::Semantic(Emphasis)][..], "see ***https://example.com/c*d*** here"),
    ] {
        let mut editor = Editor::open("see https://example.com/c*d here", Format::Markdown);
        for treatment in treatments {
            treat(&mut editor, "https", "vE", *treatment, true);
        }
        assert_eq!(editor.source(), expected);
    }
    // Part of a URL can only be styled inside an explicit link's label,
    // which becomes a bare URL again once its text is plain.
    let mut editor = Editor::open("see https://example.com/ab here", Format::Markdown);
    treat(&mut editor, "example", "ve", Treatment::Semantic(Strong), true);
    assert_eq!(editor.source(), "see [https://**example**.com/ab](<https://example.com/ab>) here");
    treat(&mut editor, "example", "ve", Treatment::Semantic(Strong), false);
    assert_eq!(editor.source(), "see https://example.com/ab here");
    // Punctuation that would pair with the treatment is escaped instead.
    let mut editor = Editor::open("see https://example.com/c*d here", Format::Markdown);
    treat(&mut editor, "example", "v$", Treatment::Semantic(Emphasis), true);
    assert_eq!(editor.source(), r"see [https\:\/\/*example\.com\/c\*d*](<https://example.com/c*d>)<em> here</em>");
}

#[test]
fn emphasis_across_a_link_boundary_styles_each_side() {
    use viem_core::document::SemanticInlineStyle::Strong;
    for (anchor, keys, expected) in [
        ("text", "v$", "see [link **text**](u)<strong> here</strong>"),
        ("see", "vee", "<strong>see </strong>[**link** text](u) here"),
    ] {
        let source = "see [link text](u) here";
        let mut editor = Editor::open(source, Format::Markdown);
        treat(&mut editor, anchor, keys, Treatment::Semantic(Strong), true);
        assert_eq!(editor.source(), expected, "{anchor:?}");
        assert_eq!(links(editor.document()), ["link text"], "{anchor:?}");
        assert_reopens_as_displayed(editor.document(), anchor);
        assert_exact_undo_redo(&mut editor, source, expected, anchor);
    }
}

fn typed(editor: &mut Editor, text: &str) {
    for character in text.chars() {
        editor.core.handle(editor.view, viem_core::CoreEvent::Input(viem_core::command::InputEvent::key(character)))
            .unwrap_or_else(|error| panic!("typing {character:?} of {text:?}: {error:?}"));
    }
}

#[test]
fn whitespace_typed_at_a_bare_url_is_ordinary_text() {
    for (source, anchor, keys, input, expected, linked) in [
        // After a typed or pasted URL, the next word is not part of the link.
        ("see  here", "  ", "i", "https://example.com/a_b more", "see https://example.com/a_b more here", "https://example.com/a_b"),
        ("see https://example.com/ab", "ab", "A", " more", "see https://example.com/ab more", "https://example.com/ab"),
        // Inside a bare URL, a space splits it as Markdown does.
        ("see https://example.com/ab here", "b here", "i", " ", "see https://example.com/a b here", "https://example.com/a"),
        ("see https://example.com/ab here", "https", "i", "x ", "see x https://example.com/ab here", "https://example.com/ab"),
    ] {
        let mut editor = if anchor == "  " { between_spaces(source) } else {
            let mut editor = Editor::open(source, Format::Markdown);
            editor.place_at(anchor);
            editor.type_keys(keys);
            editor
        };
        typed(&mut editor, input);
        assert_eq!(editor.source(), expected, "{source:?} + {input:?}");
        assert_eq!(links(editor.document()), [linked], "{source:?} + {input:?}");
        assert_reopens_as_displayed(editor.document(), source);
    }
    // `<...>` autolinks keep their label and destination.
    let mut editor = Editor::open("see <https://example.com/ab> here", Format::Markdown);
    editor.place_at("b here");
    editor.type_keys("i");
    typed(&mut editor, " ");
    assert_eq!(editor.source(), "see [https://example.com/a b](<https://example.com/ab>) here");
}

#[test]
fn code_moves_into_link_text_and_out_of_urls() {
    use viem_core::document::SemanticInlineStyle::Code;
    for (source, anchor, keys, expected) in [
        ("see https://example.com/a_b here", "https", "vE", "see [`https://example.com/a_b`](<https://example.com/a_b>) here"),
        ("see https://example.com/a_b here", "example", "ve", "see [https://`example`.com/a_b](<https://example.com/a_b>) here"),
        ("see [link text](u) here", "text", "ve", "see [link `text`](u) here"),
        // Code cannot contain a link, so it is split at the link's edges.
        ("see [link text](u) here", "see", "v$", "`see `[`link text`](u)` here`"),
        // Code text is literal: escapes become the characters they show.
        (r"see foo\_bar here", "foo", "vE", "see `foo_bar` here"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let text = editor.document().text().to_owned();
        let shown = links(editor.document());
        treat(&mut editor, anchor, keys, Treatment::Semantic(Code), true);
        assert_eq!(editor.source(), expected, "{source:?}");
        assert_eq!(editor.document().text(), text, "{source:?}");
        assert_eq!(links(editor.document()), shown, "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
        assert_exact_undo_redo(&mut editor, source, expected, source);
    }
    // Removing code from a URL reveals its link; a code label collapses.
    for (source, anchor, expected) in [
        ("see `https://example.com/a_b/c*d` here", "https", "see https://example.com/a_b/c*d here"),
        ("see [`https://example.com/a_b`](<https://example.com/a_b>) here", "https", "see https://example.com/a_b here"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        treat(&mut editor, anchor, "vE", Treatment::Semantic(Code), false);
        assert_eq!(editor.source(), expected, "{source:?}");
        assert_eq!(links(editor.document()).len(), 1, "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
    }
}

#[test]
fn a_copied_url_put_before_a_word_keeps_its_destination() {
    for (target, keys, expected) in [
        ("line", "P", "see https://example.com/a_b here\n\nnext [https://example.com/a_b](<https://example.com/a_b>)line"),
        ("line", "$p", "see https://example.com/a_b here\n\nnext line[https://example.com/a_b](<https://example.com/a_b>)"),
    ] {
        let source = "see https://example.com/a_b here\n\nnext line";
        let mut editor = Editor::open(source, Format::Markdown);
        editor.place_at("https");
        editor.type_keys("yiW");
        editor.place_at(target);
        editor.type_keys(keys);
        assert_eq!(editor.source(), expected, "{keys:?}");
        assert_eq!(links(editor.document()), ["https://example.com/a_b", "https://example.com/a_b"], "{keys:?}");
        assert_reopens_as_displayed(editor.document(), keys);
    }
    // Pasted Markdown is inserted as written when its URL touches a word.
    for (input, expected, linked) in [
        ("[x](https://example.com/ab) and https://example.com/cd", "hello [x](https://example.com/ab) and https://example.com/cdworld", vec!["x", "https://example.com/cdworld"]),
        ("intro\n\n*it* and https://example.com/c_d", "hello intro\n\n*it* and https://example.com/c_dworld", vec!["https://example.com/c_dworld"]),
    ] {
        let mut editor = Editor::open("hello world", Format::Markdown);
        editor.place_at("world");
        editor.type_keys("i");
        editor.paste_plain(input);
        assert_eq!(editor.source(), expected, "{input:?}");
        assert_eq!(links(editor.document()), linked, "{input:?}");
        assert_reopens_as_displayed(editor.document(), input);
        assert_exact_undo_redo(&mut editor, "hello world", expected, input);
    }
}

#[test]
fn typed_intraword_underscores_need_no_escape() {
    for (input, expected) in [
        ("snake_case", "see snake_case here"),
        ("a_b_c", "see a_b_c here"),
        ("x_1", "see x_1 here"),
        // Pending protection remains where `_` can still delimit emphasis.
        ("foo_", r"see foo\_ here"),
        ("foo_ bar", r"see foo\_ bar here"),
        ("_y", r"see \_y here"),
        ("_x_", "see _x_ here"),
    ] {
        let mut editor = between_spaces("see  here");
        typed(&mut editor, input);
        assert_eq!(editor.source(), expected, "{input:?}");
        assert_reopens_as_displayed(editor.document(), input);
    }
}

#[test]
fn table_cell_whitespace_typed_before_text_is_plain() {
    for input in [Input::Typed, Input::Paste, Input::PasteMatchingStyle] {
        let source = "| a | b |\n| - | - |\n| x | y |";
        let mut editor = Editor::open(source, Format::Markdown);
        let at = editor.document().text().rfind("\ny").unwrap() - 1;
        editor.place(at);
        editor.type_keys("a ");
        enter(&mut editor, input, "https://example.com/a_b/c*d");
        assert_eq!(editor.source(), "| a | b |\n| - | - |\n| x https://example.com/a_b/c*d | y |", "{input:?}");
        assert_eq!(links(editor.document()), ["https://example.com/a_b/c*d"], "{input:?}");
        assert_reopens_as_displayed(editor.document(), &format!("{input:?}"));
    }
    let mut editor = Editor::open("| a | b |\n| - | - |\n| &#32;x | y |", Format::Markdown);
    let at = editor.document().text().find(" x").unwrap();
    editor.place(at);
    editor.type_keys("i");
    typed(&mut editor, "w");
    assert_eq!(editor.source(), "| a | b |\n| - | - |\n| w x | y |");
}

#[test]
fn link_text_contains_no_bare_url() {
    let document = Document::from_bytes(b"[https://example.com/a_b](u) and www.example.com".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(links(&document), ["https://example.com/a_b", "www.example.com"]);
    assert_eq!(document.link_at(document.text_point(0).unwrap()).unwrap().as_deref(), Some("u"));
}
