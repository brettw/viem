use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, FileFormat, Format};
fn command(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandStatus {
    for ch in text.chars() {
        c.handle(d, InputEvent::Key(Key::Char(ch))).unwrap();
    }
    c.handle(d, InputEvent::Key(Key::Enter)).unwrap().status
}
fn success(status: CommandStatus) {
    assert!(matches!(status, CommandStatus::Complete), "{status:?}");
}
fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandStatus {
    let mut status = CommandStatus::Complete;
    for ch in text.chars() {
        status = c.handle(d, InputEvent::Key(Key::Char(ch))).unwrap().status;
    }
    status
}
#[test]
fn directional_word_searches_preserve_counts_direction_and_repeat() {
    let mut d = Document::new("old old_name old older old");
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, r"/\<old\>"));
    assert_eq!(c.cursor(), 13);
    success(keys(&mut c, &mut d, "2n"));
    assert_eq!(c.cursor(), 0);
    success(keys(&mut c, &mut d, "N"));
    assert_eq!(c.cursor(), 23);
    success(command(&mut c, &mut d, r"?\<old\>"));
    assert_eq!(c.cursor(), 13);
    success(keys(&mut c, &mut d, "n"));
    assert_eq!(c.cursor(), 0);
    success(keys(&mut c, &mut d, "N"));
    assert_eq!(c.cursor(), 13);
    assert_eq!(d.text(), "old old_name old older old");
    assert!(!d.undo());
}
#[test]
fn directional_operator_search_uses_an_exclusive_extent_and_named_register() {
    let original = "one older old tail";
    let mut d = Document::new(original);
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, r#""ad/\<old\>"#));
    assert_eq!(d.text(), "old tail");
    assert_eq!(c.register('a').unwrap().text, "one older ");
    assert_eq!(c.cursor(), 0);
    assert!(d.undo());
    assert_eq!(d.text(), original);
    assert!(!d.undo());
}
#[test]
fn directional_whole_word_substitution_renames_identifiers_in_one_undo_unit() {
    let original = "old old_name older (old) old-old";
    let mut d = Document::new(original);
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, r":%s/\<old\>/new/g"));
    assert_eq!(d.text(), "new old_name older (new) new-new");
    assert!(d.undo());
    assert_eq!(d.text(), original);
    assert!(!d.undo());
    assert!(d.redo());
    assert_eq!(d.text(), "new old_name older (new) new-new");
}
#[test]
fn directional_substitution_checks_every_grapheme_endpoint_before_editing() {
    for (text, substitution) in [
        ("a a\u{301}", r":%s/\<a/b/g"),
        ("a !\u{301}", r":%s/\</X/g"),
        ("a 👩\u{200d}💻", r":%s/\>/X/g"),
    ] {
        let mut d = Document::new(text);
        let mut c = CommandInterpreter::new();
        let revision = d.revision();
        let source = d.source_bytes();
        let status = command(&mut c, &mut d, substitution);
        assert!(
            format!("{status:?}").contains("RegexMatchSplitsGraphemeCluster"),
            "{substitution:?} on {text:?}: {status:?}"
        );
        assert_eq!(d.source_bytes(), source);
        assert_eq!(d.revision(), revision);
        assert_eq!(c.cursor(), 0);
        assert!(!d.undo());
        let status = command(&mut c, &mut d, ":&");
        assert!(format!("{status:?}").contains("NoPreviousSubstitute"));
    }
}
#[test]
fn star_and_hash_keep_existing_boundaries_for_non_regex_word_numbers() {
    // Keyword extraction includes ², but the Unicode regex word class does
    // not. Replacing the generated \b²\b with \<²\> would change behavior.
    for (search, first, repeated) in [("*", 4, 9), ("#", 9, 4)] {
        let mut d = Document::new("² x²x y²y");
        let mut c = CommandInterpreter::new();
        success(keys(&mut c, &mut d, search));
        assert_eq!(c.cursor(), first, "{search}");
        success(keys(&mut c, &mut d, "n"));
        assert_eq!(c.cursor(), repeated, "{search}");
        success(command(&mut c, &mut d, ":%s//Q/g"));
        assert_eq!(d.text(), "² xQx yQy", "{search}");
    }
}
#[test]
fn multiline_matches_are_contained_and_selected_once_by_start_line() {
    let mut d = Document::new("a\nb a\nb");
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, r":%s/a\nb/X/g"));
    assert_eq!(d.text(), "X X");
    assert!(d.undo());
    assert_eq!(d.text(), "a\nb a\nb");
    assert!(!d.undo());
    success(command(&mut c, &mut d, r":1s/a\nb/X/e"));
    assert_eq!(d.text(), "a\nb a\nb");
}
#[test]
fn replacement_capture_preserves_break_kinds_and_named_optional_groups() {
    let mut d = Document::from_bytes_with_file_format(
        b"a\nb\rc".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    let mut c = CommandInterpreter::new();
    success(command(
        &mut c,
        &mut d,
        r":%s/(?P<first>a\nb)\n(c)(x)?/\2\r\g{first}\3/",
    ));
    assert_eq!(d.text(), "c\na\nb");
    assert_eq!(d.source_bytes(), b"c\ra\nb");
    assert_eq!(d.hard_line_snapshot().line_count(), 2);
}
#[test]
fn every_selected_grapheme_endpoint_is_checked_before_any_mutation() {
    let mut d = Document::new("a a\u{301}");
    let mut c = CommandInterpreter::new();
    let revision = d.revision();
    let source = d.source_bytes();
    let status = command(&mut c, &mut d, ":%s/a/b/g");
    assert!(format!("{status:?}").contains("RegexMatchSplitsGraphemeCluster"));
    assert_eq!(d.source_bytes(), source);
    assert_eq!(d.revision(), revision);
    assert_eq!(c.cursor(), 0);
    assert!(!d.undo());
    // Failed replacement must not become either substitute or command history.
    let status = command(&mut c, &mut d, ":&");
    assert!(format!("{status:?}").contains("NoPreviousSubstitute"));
    c.handle(&mut d, InputEvent::Key(Key::Char(':'))).unwrap();
    c.handle(&mut d, InputEvent::Key(Key::Up)).unwrap();
    assert_eq!(c.command_line(), Some(""));
}
#[test]
fn navigation_skips_unusable_scalar_starts_without_rounding_them() {
    let mut d = Document::new("a\u{301} \u{301}");
    let mut c = CommandInterpreter::new();
    let status = command(&mut c, &mut d, "/\u{301}");
    assert!(matches!(status, CommandStatus::SearchNotFound));
    assert_eq!(c.cursor(), 0);
    let mut d = Document::new("a\u{301} \u{301}\na");
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, "/a"));
    assert_eq!(c.cursor(), d.text().len() - 1);
}
#[test]
fn search_options_validate_atomically_and_apply_to_substitution() {
    let mut d = Document::new("cat CAT Cat");
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, ":set ic sc nows"));
    success(command(&mut c, &mut d, "/cat"));
    assert_eq!(c.cursor(), 4);
    success(command(&mut c, &mut d, "/Cat"));
    assert_eq!(c.cursor(), 8);
    assert!(matches!(
        command(&mut c, &mut d, "/cat"),
        CommandStatus::SearchNotFound
    ));
    assert_eq!(c.cursor(), 8);
    assert!(
        format!("{:?}", command(&mut c, &mut d, ":set noic unknown")).contains("UnsupportedOption")
    );
    success(command(&mut c, &mut d, ":%s/cat/dog/g"));
    assert_eq!(d.text(), "dog dog dog");
    assert!(d.undo());
    success(command(&mut c, &mut d, ":%s/cat/dog/gI"));
    assert_eq!(d.text(), "dog CAT Cat");
    assert!(d.undo());
    success(command(&mut c, &mut d, ":%s/(?i:CAT)/dog/gI"));
    assert_eq!(d.text(), "dog dog dog");
}
#[test]
fn invalid_and_undeclared_replacements_fail_before_editing() {
    for replacement in [
        r"\1",
        r"\g{missing}",
        "$1",
        "${name}",
        "~",
        r"\q",
        r"\U",
        r"\=1",
    ] {
        let mut d = Document::new("a");
        let mut c = CommandInterpreter::new();
        let status = command(&mut c, &mut d, &format!(":s/a/{replacement}/"));
        assert!(
            format!("{status:?}").contains("UnsupportedReplacementAtom")
                || format!("{status:?}").contains("Parse"),
            "{replacement:?}: {status:?}"
        );
        assert_eq!(d.text(), "a");
        assert!(!d.undo());
    }
}
#[test]
fn captures_across_style_boundaries_keep_rich_formatting_and_one_undo() {
    let mut d = Document::from_bytes(
        b"**one** *two*".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let before = d.source_bytes();
    let mut c = CommandInterpreter::new();
    success(command(&mut c, &mut d, r":s/(one) (two)/\2 \1/"));
    assert_eq!(d.text(), "two one");
    let styles = d.projection().style_spans();
    assert!(styles.iter().any(|span| span.range.start == 0));
    assert!(d.undo());
    assert_eq!(d.source_bytes(), before);
    assert!(!d.undo());
}
#[test]
fn failed_word_search_keeps_previous_search_in_normal_and_operator_forms() {
    for keys in ["*", "#", "g*", "g#", "d*", "d#", "dg*", "dg#"] {
        let text = format!("{}\nok ok", "a".repeat(17_000));
        let mut d = Document::new(&text);
        let mut c = CommandInterpreter::new();
        success(command(&mut c, &mut d, "/ok"));
        for ch in "gg".chars() {
            c.handle(&mut d, InputEvent::Key(Key::Char(ch))).unwrap();
        }
        let mut last = None;
        for ch in keys.chars() {
            last = Some(c.handle(&mut d, InputEvent::Key(Key::Char(ch))).unwrap());
        }
        assert!(
            format!("{:?}", last.unwrap().status).contains("RegexResourceLimit"),
            "{keys}"
        );
        assert_eq!(d.text(), text);
        assert_eq!(c.cursor(), 0);
        let output = c.handle(&mut d, InputEvent::Key(Key::Char('n'))).unwrap();
        success(output.status);
        assert_eq!(c.cursor(), 17_001, "{keys}");
    }
}
