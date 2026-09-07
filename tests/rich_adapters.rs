use evim_core::document::{
    BlockKind, CharacterProperties, FileFormat, FontSlant, StyleApplication,
    TransformationStageRole,
};
use evim_core::{Document, Encoding, Format};

fn open(source: &[u8], format: Format) -> Document {
    Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap()
}
fn properties_at(document: &Document, at: usize) -> CharacterProperties {
    document
        .projection()
        .style_spans()
        .iter()
        .find_map(|span| {
            if span.range.contains(&at) {
                if let StyleApplication::Direct(value) = &span.application {
                    return Some(value.clone());
                }
            }
            None
        })
        .unwrap_or_default()
}
#[test]
fn html_is_passive_and_preserves_every_source_byte() {
    let source = br#"<!DOCTYPE html><HTML odd='a'><head><style>p { color: red; }</style><script>fetch('https://invalid.test/'); '</p>'</script></head><body><h1 TITLE=x title=y>Title &amp; title</h1><p onclick='run()'> Hello  <B foo="bar">brave</B>\n world&#33; </p><!-- tail --><template><p>hidden</p></template></body></HTML>"#;
    let document = open(source, Format::Html);
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.text(), "Title & title\nHello brave\\n world!");
    assert!(matches!(
        document.projection().blocks()[0].kind,
        BlockKind::Heading(1)
    ));
    assert_eq!(
        properties_at(&document, 19).bold,
        None,
        "space before B keeps its own context"
    );
    assert_eq!(properties_at(&document, 20).bold, Some(true));
}
#[test]
fn html_entities_collapsed_whitespace_and_text_edits_keep_tags() {
    let source = b"<p><b foo=\"bar\">hello &amp; &#38; &#x26;</b>  \t tail</p><!--opaque-->";
    let mut document = open(source, Format::Html);
    assert_eq!(document.text(), "hello & & & tail");
    document.replace(0..5, "new <text>").unwrap();
    assert_eq!(document.text(), "new <text> & & & tail");
    assert_eq!(
        document.source_bytes(),
        b"<p><b foo=\"bar\">new &lt;text&gt; &amp; &#38; &#x26;</b>  \t tail</p><!--opaque-->"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
    assert!(document.redo());
    assert_eq!(document.text(), "new <text> & & & tail");
}
#[test]
fn html_css_first_attributes_and_unsupported_declarations() {
    let source = b"<p style='margin-inline-start:12pt;text-align:center'><span style='font-face:bad;font-family: Georgia, serif;font-size:24px;font-weight:650;font-style:italic;color:#f00;letter-spacing:1pt;font-feature-settings:\"liga\" 0' style='font-size:99pt' lang='en-US' dir='rtl'>text</span></p>";
    let document = open(source, Format::Html);
    let properties = properties_at(&document, 0);
    assert_eq!(
        properties.font_families,
        Some(vec!["Georgia".into(), "serif".into()])
    );
    assert_eq!(properties.size, Some(18.0));
    assert_eq!(properties.weight, Some(650));
    assert_eq!(properties.slant, Some(FontSlant::Italic));
    assert_eq!(properties.letter_spacing, Some(1.0));
    assert_eq!(properties.open_type_features.unwrap().get("liga"), Some(&0));
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .leading_indent,
        Some(12.0)
    );
    assert_eq!(document.source_bytes(), source);
}
#[test]
fn html_recovers_optional_paragraphs_and_keeps_named_references() {
    let document = open(
        b"<p>one<p>two &copy; &NotEqualTilde; &notit;<br>three",
        Format::Html,
    );
    assert_eq!(document.text(), "one\ntwo © ≂\u{338} ¬it;\nthree");
    assert_eq!(document.projection().hard_line_count(), 3);
}
#[test]
fn rich_edits_cannot_consume_hidden_source_or_opaque_objects() {
    for (source, format) in [
        (
            b"<p>a<script>secret()</script>b</p>".as_slice(),
            Format::Html,
        ),
        (br"{\rtf1 a{\*\unknown hidden}b}".as_slice(), Format::Rtf),
    ] {
        let mut document = open(source, format);
        assert_eq!(document.text(), "ab");
        document.replace(0..2, "x").unwrap();
        assert_eq!(document.text(), "x");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
        document.replace(1..2, "c").unwrap();
        assert_eq!(document.text(), "ac");
        assert!(
            document.source_bytes().windows(6).any(|s| s == b"secret")
                || document.source_bytes().windows(6).any(|s| s == b"hidden")
        );
    }
    let mut document = open(
        b"<p>one<img src='https://invalid.test/picture'>two</p>",
        Format::Html,
    );
    assert_eq!(document.text(), "one\u{fffc}two");
    assert!(document.replace(3..6, "").is_err());
}
#[test]
fn rtf_scoped_formatting_tables_controls_and_breaks() {
    let source = br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0\fnil Arial;}{\f7\fnil Times New Roman;}}{\colortbl;\red255\green0\blue0;}\f7\fs36\cf1\li240 plain {\b bold {\i both} bold}\par \pard\plain next\line last}";
    let document = open(source, Format::Rtf);
    assert_eq!(document.text(), "plain bold both bold\nnext\nlast");
    let first = properties_at(&document, 0);
    assert_eq!(first.font_families, Some(vec!["Times New Roman".into()]));
    assert_eq!(first.size, Some(18.0));
    assert_eq!(first.foreground.unwrap().red, 1.0);
    assert_eq!(properties_at(&document, 6).bold, Some(true));
    assert_eq!(properties_at(&document, 11).slant, Some(FontSlant::Italic));
    assert_eq!(properties_at(&document, 16).slant, None);
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .leading_indent,
        Some(12.0)
    );
    assert_eq!(
        document.projection().blocks()[1]
            .direct_paragraph
            .leading_indent,
        None
    );
    assert_eq!(document.source_bytes(), source);
}
#[test]
fn rtf_unicode_fallback_hex_binary_and_ignorable_groups() {
    let source = br"{\rtf1\ansi\uc1 A\u233? \'e9 \u-10179?\u-8704?{\*\hidden\bin4 {\}x}Z}";
    let document = open(source, Format::Rtf);
    assert_eq!(document.text(), "Aé é 😀Z");
    assert_eq!(document.source_bytes(), source);
}
#[test]
fn rtf_text_edits_preserve_header_and_escape_new_unicode() {
    let source = br"{\rtf1\ansi\uc2{\info{\title Original}}{\b\unknown7 hello} tail}";
    let mut document = open(source, Format::Rtf);
    document.replace(0..5, "Hi {é} 😀").unwrap();
    assert_eq!(document.text(), "Hi {é} 😀 tail");
    assert_eq!(document.source_bytes(), br"{\rtf1\ansi\uc2{\info{\title Original}}{\b\unknown7 {\uc1 Hi \{\u233?\} \u-10179?\u-8704?}} tail}");
    assert_eq!(properties_at(&document, 0).bold, Some(true));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
    assert!(document.redo());
    assert_eq!(document.text(), "Hi {é} 😀 tail");
}
#[test]
fn rtf_line_endings_are_grammar_trivia_not_fileformat_capability() {
    let mut document = open(b"{\\rtf1 one\r\n two\\par three}", Format::Rtf);
    assert_eq!(document.text(), "one two\nthree");
    assert!(!document
        .transformation_pipeline_snapshot()
        .stages()
        .iter()
        .any(|s| s.role == TransformationStageRole::LineEndingInterpretation));
    assert!(document.set_file_format(FileFormat::Dos).is_err());
}
#[test]
fn deterministic_local_edits_reopen_identically_and_restore_exact_source() {
    for format in [Format::Html, Format::Rtf] {
        for size in 1..30 {
            let word = "x".repeat(size);
            let source = match format {
                Format::Html => format!("<p data-x='keep'>left <b foo='bar'>{word}</b> right</p>"),
                Format::Rtf => format!("{{\\rtf1 left {{\\b\\unknown4 {word}}} right}}"),
                _ => unreachable!(),
            };
            let mut document = open(source.as_bytes(), format);
            document.replace(5..5 + size, "changed").unwrap();
            let fresh = open(&document.source_bytes(), format);
            assert_eq!(fresh.text(), document.text());
            assert_eq!(
                fresh.projection().style_spans(),
                document.projection().style_spans()
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn canonical_bold_and_italic_changes_preserve_other_source() {
    use evim_core::document::SemanticInlineStyle;
    let mut html = open(
        b"<p><b foo='bar'>word</b><span data-untouched='yes'> tail</span></p>",
        Format::Html,
    );
    html.set_semantic_style(0..4, SemanticInlineStyle::Strong, false)
        .unwrap();
    assert_eq!(
        html.source_bytes(),
        b"<p>word<span data-untouched='yes'> tail</span></p>"
    );
    assert!(html.undo());
    html.set_semantic_style(1..3, SemanticInlineStyle::Strong, false)
        .unwrap();
    assert_eq!(html.source_bytes(), b"<p><b foo='bar'>w<span style=\"font-weight: 400; --evim-base-weight: 400; --evim-bold: false\">or</span>d</b><span data-untouched='yes'> tail</span></p>");
    let mut rtf = open(br"{\rtf1\ansi{\b\unknown4 word} tail}", Format::Rtf);
    rtf.set_semantic_style(1..3, SemanticInlineStyle::Strong, false)
        .unwrap();
    assert_eq!(
        rtf.source_bytes(),
        br"{\rtf1\ansi{\b\unknown4 w{\b0 or}d} tail}"
    );
    rtf.set_semantic_style(1..3, SemanticInlineStyle::Emphasis, true)
        .unwrap();
    assert_eq!(properties_at(&rtf, 1).slant, Some(FontSlant::Italic));
}

#[test]
fn rich_direct_formatting_appends_rtf_tables_without_renumbering() {
    use evim_core::document::{
        Color, PersistedStyleIntent, StyleModelIntent, StyleModelRequest, TextRange,
    };
    for format in [Format::Html, Format::Rtf] {
        let source = match format {
            Format::Html => b"<p>word</p>".as_slice(),
            Format::Rtf => {
                br"{\rtf1{\fonttbl{\f7\fnil Old Font;}}{\colortbl;\red0\green0\blue255;}word}"
                    .as_slice()
            }
            _ => unreachable!(),
        };
        let mut document = open(source, format);
        let properties = CharacterProperties {
            font_families: Some(vec!["Georgia".into()]),
            size: Some(18.0),
            weight: Some(700),
            foreground: Some(Color {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            }),
            ..CharacterProperties::default()
        };
        let request = StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                range: TextRange::new(
                    document.text_point(0).unwrap(),
                    document.text_point(4).unwrap(),
                )
                .unwrap(),
                properties: properties.clone(),
            }),
        );
        let prepared = document.prepare_style_request(request).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(properties_at(&document, 0), properties);
        if format == Format::Rtf {
            assert_eq!(document.source_bytes(), br"{\rtf1{\fonttbl{\f7\fnil Old Font;}{\f8\fnil Georgia;}}{\colortbl;\red0\green0\blue255;\red255\green0\blue0;}{\f8\fs36\b\evimweight700\cf2 word}}");
        } else {
            assert_eq!(document.source_bytes(), b"<p><span style=\"font-family: 'Georgia'; font-size: 18pt; font-weight: 700; color: #ff0000ff\">word</span></p>");
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn rtf_declared_single_byte_codepages_project_and_preserve_raw_bytes() {
    let source = br"{\rtf1\ansi\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2}";
    let document = open(source, Format::Rtf);
    assert_eq!(document.text(), "Привет");
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn html_empty_paragraphs_and_misnested_inline_formatting_recover() {
    let source = b"<p></p><p><b><i>a</b>b</i></p><p></p>";
    let document = open(source, Format::Html);
    assert_eq!(document.text(), "\nab\n");
    assert_eq!(properties_at(&document, 1).bold, Some(true));
    assert_eq!(properties_at(&document, 1).slant, Some(FontSlant::Italic));
    assert_eq!(properties_at(&document, 2).weight, None);
    assert_eq!(properties_at(&document, 2).slant, Some(FontSlant::Italic));
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rtf_multibyte_codepages_and_font_charsets_preserve_byte_provenance() {
    for source in [
        br"{\rtf1\ansi\ansicpg932 \'82\'a0\'82\'a2}".as_slice(),
        br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f4\fcharset128 Gothic;}}\f4 \'82\'a0\'82\'a2}"
            .as_slice(),
        br"{\rtf1\ansi\ansicpg65001 \'e3\'81\'82\'e3\'81\'84}".as_slice(),
    ] {
        let mut document = open(source, Format::Rtf);
        assert_eq!(document.text(), "あい");
        document.replace(0..3, "う").unwrap();
        assert_eq!(document.text(), "うい");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn empty_rich_documents_accept_first_text_and_enter_inside_the_source_body() {
    for (source, format, expected) in [
        (
            b"<p></p>".as_slice(),
            Format::Html,
            b"<p>hi<br>there</p>".as_slice(),
        ),
        (
            br"{\rtf1\ansi}".as_slice(),
            Format::Rtf,
            br"{\rtf1\ansi hi\par there}".as_slice(),
        ),
        (
            b"".as_slice(),
            Format::Rtf,
            br"{\rtf1\ansi hi\par there}".as_slice(),
        ),
    ] {
        let mut document = open(source, format);
        document.insert(0, "hi\nthere").unwrap();
        assert_eq!(document.text(), "hi\nthere");
        assert_eq!(document.source_bytes(), expected);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn html_deletion_and_change_preserve_spaces_and_empty_formatting_context() {
    let source = b"<p><b foo='bar'>Bold words</b> and <i>italic</i> &amp; cafe.</p>";
    let mut document = open(source, Format::Html);
    document.replace(0..10, "").unwrap();
    assert_eq!(document.text(), "\u{a0}and italic & cafe.");
    assert!(document
        .source_bytes()
        .starts_with(b"<p><b foo='bar'></b>&nbsp;"));
    document.insert(0, "New words").unwrap();
    assert_eq!(document.text(), "New words and italic & cafe.");
    assert_eq!(
        properties_at(&document, 0).bold,
        Some(true),
        "{} {:?}",
        String::from_utf8_lossy(&document.source_bytes()),
        document.projection().provenance()
    );
    let mut interior = open(b"<p>one words two</p>", Format::Html);
    interior.replace(4..9, "").unwrap();
    assert_eq!(interior.text(), "one \u{a0}two");
    interior.insert(4, "  ").unwrap();
    assert_eq!(interior.text(), "one \u{a0} \u{a0}two");
    assert_eq!(
        open(&interior.source_bytes(), Format::Html).text(),
        interior.text()
    );
}

#[test]
fn rich_inline_breaks_keep_one_paragraph_and_independent_stable_hard_lines() {
    for (format, source) in [
        (
            Format::Html,
            b"<p style='margin-bottom:18pt;text-indent:12pt'>one<br>two<br>three</p><p>tail</p>"
                .as_slice(),
        ),
        (
            Format::Rtf,
            br"{\rtf1\sa360\fi240 one\line two\line three\par tail}".as_slice(),
        ),
    ] {
        let mut document = open(source, format);
        assert_eq!(document.text(), "one\ntwo\nthree\ntail");
        assert_eq!(document.projection().blocks().len(), 2, "{format:?}");
        assert_eq!(document.projection().blocks()[0].range, 0..13);
        assert_eq!(document.projection().hard_line_count(), 4);
        let paragraph = document.projection().blocks()[0].id;
        let tail = document.projection().blocks()[1].id;
        let lines = (0..4)
            .map(|line| document.projection().hard_line_id(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            lines
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        document.replace(4..7, "second").unwrap();
        assert_eq!(document.text(), "one\nsecond\nthree\ntail");
        assert_eq!(document.projection().blocks()[0].id, paragraph);
        assert_eq!(document.projection().blocks()[1].id, tail);
        assert_eq!(
            (0..4)
                .map(|line| document.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>(),
            lines
        );
        let fresh = open(&document.source_bytes(), format);
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .map(|block| block.range.clone())
                .collect::<Vec<_>>(),
            fresh
                .projection()
                .blocks()
                .iter()
                .map(|block| block.range.clone())
                .collect::<Vec<_>>()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.projection().blocks()[0].id, paragraph);
        assert_eq!(
            (0..4)
                .map(|line| document.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>(),
            lines
        );
    }
}

#[test]
fn rich_typing_at_hard_line_end_inherits_preceding_inline_group() {
    for (format, source) in [
        (Format::Html, b"<p><b>one</b><br>two</p>".as_slice()),
        (Format::Rtf, br"{\rtf1{\b one}\line two}".as_slice()),
    ] {
        let mut document = open(source, format);
        document.insert(3, "X").unwrap();
        assert_eq!(document.text(), "oneX\ntwo");
        let fresh = open(&document.source_bytes(), format);
        assert_eq!(properties_at(&document, 3), properties_at(&fresh, 3));
        assert_eq!(properties_at(&document, 3).bold, Some(true), "{format:?}");
    }
}

#[test]
fn html_legacy_encoding_uses_exact_numeric_escapes_for_new_unicode() {
    let source = b"<p>caf\xe9</p><!--keep-->";
    let mut document =
        Document::from_bytes(source.to_vec(), Encoding::Latin1, Format::Html).unwrap();
    document.insert(5, " 😀").unwrap();
    assert_eq!(document.text(), "café 😀");
    assert_eq!(document.encoding(), Encoding::Latin1);
    let bytes = document.source_bytes();
    assert!(bytes.windows(9).any(|chunk| chunk == b"&#x1F600;"));
    assert!(bytes.windows(1).any(|chunk| chunk == b"\xe9"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rtf_last_empty_paragraph_remains_editable_before_preserved_opaque_destination() {
    let source = br"{\rtf1\ansi body\par{\*\unknown Opaque preserved}}";
    let mut document = open(source, Format::Rtf);
    assert_eq!(document.text(), "body\n");
    document.insert(5, "مرحبا").unwrap();
    assert_eq!(document.text(), "body\nمرحبا");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(r"{\*\unknown Opaque preserved}"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn complex_and_malformed_rich_sources_no_op_save_exactly_without_external_effects() {
    let corpus:[(Format,&[u8]);8]=[
        (Format::Html,br#"<!DOCTYPE html PUBLIC 'odd'><html><head><meta charset='windows-1252'><style>@import 'https://invalid.test/x'; p{unknown:var(--x)}</style><script src='file:///private/no-read'>fetch('https://invalid.test/')</script></head><body><p ID=a id=b x='unterminated>opaque"#),
        (Format::Html,b"<table><b>before<tr><td>A<p>B</table>after<!--bad--!><svg onload='execute()'><foreignObject><p>x</p></foreignObject></svg>"),
        (Format::Html,b"<p>nul\0text &bogus; &#0; &#xD800; &notit;<!broken><p><i><b>malformed</i>end"),
        (Format::Html,b"<p>invalid:\xff\xfe</p><object data='https://invalid.test/x'>fallback</object>"),
        (Format::Rtf,br"{\rtf1\ansi\deff0{\fonttbl{\f0\fnil\fcharset0 Aptos;}{\f47\froman Times New Roman;}}{\colortbl;\red7\green8\blue9;}{\stylesheet{\s0 Normal;}{\s9\sbasedon0\b Heading 1;}}{\info{\author Anonymous}}\viewkind4\uc1\pard\s9\f0\fs22 Word authored \u945?\par {\field{\*\fldinst INCLUDETEXT https://invalid.test/document}{\fldrslt never refreshed}}{\pict\pngblip\bin6 abc{}}{\*\data arbitrary}}"),
        (Format::Rtf,br"{\rtf1\uc0\u-10179\u-8704{\*\unknown\bin999999 truncated}"),
        (Format::Rtf,b"{\\rtf1\\ansi raw\xff{\\unknown-99 broken braces"),
        (Format::Rtf,br"}\rtf1 odd\bin-1 escaped\{\}\'ff\u999999?{\*\foo opaque}"),
    ];
    for (format, source) in corpus {
        let document = open(source, format);
        assert_eq!(document.source_bytes(), source, "{format:?}");
        assert!(std::str::from_utf8(document.text().as_bytes()).is_ok());
        let reopened = open(&document.source_bytes(), format);
        assert_eq!(document.text(), reopened.text());
        assert_eq!(
            document.projection().provenance(),
            reopened.projection().provenance()
        );
    }
}

#[test]
fn rich_body_replacement_uses_discontiguous_patches_preserving_all_intervening_syntax() {
    for (format, source) in [
        (
            Format::Html,
            "<p><b foo='bar'>bold</b> and <i>italic</i></p><!--keep-->",
        ),
        (
            Format::Rtf,
            r"{\rtf1{\b\unknown42 bold} and {\i italic}{\*\unknown keep}}",
        ),
    ] {
        let mut document = open(source.as_bytes(), format);
        document.replace(0..15, "replacement").unwrap();
        assert_eq!(document.text(), "replacement");
        let actual = String::from_utf8(document.source_bytes()).unwrap();
        if format == Format::Html {
            assert_eq!(
                actual,
                "<p><b foo='bar'>replacement</b><i></i></p><!--keep-->"
            );
        } else {
            assert!(actual.contains(r"{\b\unknown42 replacement}"));
            assert!(actual.contains(r"{\*\unknown keep}"));
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn visual_change_of_whole_middle_inline_run_retains_original_bold_context() {
    use evim_core::command::{CommandInterpreter, InputEvent, Key};
    let source="<!doctype html><p>Bold <b foo='keep'>words</b> and &#x26; text.</p><script>preserved()</script>";
    let mut document = open(source.as_bytes(), Format::Html);
    let mut commands = CommandInterpreter::new();
    for character in "wvec".chars() {
        commands
            .handle(&mut document, InputEvent::key(character))
            .unwrap();
    }
    for character in "ORDS".chars() {
        commands
            .handle(&mut document, InputEvent::Text(character.to_string()))
            .unwrap();
    }
    commands
        .handle(&mut document, InputEvent::Key(Key::Escape))
        .unwrap();
    assert_eq!(document.text(), "Bold ORDS and & text.");
    for at in 5..9 {
        assert_eq!(
            properties_at(&document, at).bold,
            Some(true),
            "at{at} source {}",
            String::from_utf8_lossy(&document.source_bytes())
        );
    }
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("<b foo='keep'>ORDS</b>"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn rich_counted_insert_and_dot_use_exact_format_escapes_in_legacy_encoding() {
    use evim_core::command::{CommandInterpreter, InputEvent, Key};
    for (format, source) in [
        (Format::Html, "<p>x</p><!--keep-->"),
        (Format::Rtf, r"{\rtf1\ansi x{\*\unknown keep}}"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Latin1, format).unwrap();
        let mut commands = CommandInterpreter::new();
        for character in "3i".chars() {
            commands
                .handle(&mut document, InputEvent::key(character))
                .unwrap();
        }
        commands
            .handle(&mut document, InputEvent::Text("😀".into()))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::Key(Key::Escape))
            .unwrap();
        assert_eq!(document.text(), "😀😀😀x", "{format:?}");
        commands
            .handle(&mut document, InputEvent::key('.'))
            .unwrap();
        assert_eq!(document.text(), "😀😀😀😀😀😀x", "{format:?}");
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Latin1, format).unwrap();
        assert_eq!(document.text(), reopened.text());
        assert!(document.undo());
        assert_eq!(document.text(), "😀😀😀x");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn html_edit_boundaries_cannot_complete_existing_character_references() {
    for (source, range, replacement, expected) in [
        ("<p>&a</p>", 2..2, "mp;", "&amp;"),
        ("<p>&amp</p>", 1..1, ";", "&;"),
        ("<p>&#65</p>", 1..1, "9", "A9"),
        ("<p>&not</p>", 2..2, "in;", "¬in;"),
        ("<p>&amXp;</p>", 3..4, "", "&amp;"),
    ] {
        let mut document = open(source.as_bytes(), Format::Html);
        document.replace(range, replacement).unwrap();
        assert_eq!(document.text(), expected, "{source}");
        let reopened = open(&document.source_bytes(), Format::Html);
        assert_eq!(document.text(), reopened.text());
        assert_eq!(
            document.projection().provenance(),
            reopened.projection().provenance()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
