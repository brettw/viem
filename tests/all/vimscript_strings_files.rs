//! File/string regressions checked against Vim 9.2.0000's implementations and
//! upstream test_functions.vim assertions.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor() -> (Editor, ViewId) {
    let document = Document::from_bytes_with_file_format(b"text\n".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    (core, view)
}

fn ex(core: &mut Editor, view: ViewId, command: &str) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char(':')))).unwrap();
    for character in command.chars() {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char(character)))).unwrap();
    }
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap()
}

fn messages(outcome: &CoreOutcome) -> Vec<String> {
    assert_eq!(outcome.command.as_ref().unwrap().status, CommandStatus::Complete, "{outcome:?}");
    outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).into_iter().flat_map(|outcome| &outcome.frontend_requests).filter_map(|request| {
        match request { ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()), _ => None }
    }).collect()
}

fn check(command: &str, expected: &str) {
    let (mut core, view) = editor();
    assert_eq!(messages(&ex(&mut core, view, command)), vec![expected]);
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("viem-scripting-files-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().replace('\\', "/").replace('\'', "''")
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn upstream_string_assertions_pass_through_source() {
    let directory = Directory::new("upstream");
    let fixture = include_str!("fixtures/vimscript/upstream_string_file_assertions.vim");
    for (name, declaration) in [("legacy.vim", ""), ("vim9.vim", "vim9script\n")] {
        // The upstream assertions themselves are valid in both dialects.
        // Only the fixture's provenance comments use legacy quote syntax.
        let body = fixture.lines().filter(|line| !line.trim_start().starts_with('"')).collect::<Vec<_>>().join("\n");
        let script = format!("{declaration}{body}\n");
        let path = directory.file(name, script.as_bytes());
        let (mut core, view) = editor();
        assert_eq!(messages(&ex(&mut core, view, &format!("source {path}"))), vec!["[]"], "{name}");
    }
}

#[test]
fn readblob_uses_byte_offsets_and_lengths() {
    let directory = Directory::new("blob");
    let path = directory.file("bytes", b"01234567");
    check(&format!("echo readblob('{path}', 3, 3) readblob('{path}', -2) readblob('{path}', 100) readblob('{path}', 1, 0)"), "0z333435 0z3637 0z 0z");
}

#[test]
fn readfile_preserves_binary_bom_and_maps_nul() {
    let directory = Directory::new("readfile");
    let path = directory.file("bytes", b"\xef\xbb\xbfa\0b\r\r\nc\r");
    let command = format!(r#"call assert_equal(0zEFBBBF61.00620D0D.0A630D, readfile('{path}', 'B')) | call assert_equal(["a\nb", "c\r"], readfile('{path}')) | call assert_equal(["\ufeffa\nb\r\r", "c\r"], readfile('{path}', 'b')) | echo v:errors"#);
    check(&command, "[]");
    let empty = directory.file("empty", b"");
    check(&format!("echo readfile('{empty}') readfile('{empty}', 'b')"), "[] ['']");
}

#[test]
fn readdir_evaluates_predicates_and_restores_callback_value() {
    let directory = Directory::new("readdir");
    directory.file("one.vim", b"");
    directory.file("two.txt", b"");
    directory.file("three.vim", b"");
    let path = directory.0.to_string_lossy().replace('\\', "/").replace('\'', "''");
    check(&format!(r#"echo readdir('{path}', {{name -> name =~ '\.vim$'}}) readdir('{path}', 'v:val =~ "\.txt$"')"#), "['one.vim', 'three.vim'] ['two.txt']");
    check(&format!(r#"echo map(['outer'], 'len(readdir("{path}", {{name -> 1}})) > 0 ? v:val : "bad"')"#), "['outer']");
}

#[test]
fn filename_modifiers_expand_extensions_and_vim_patterns() {
    check(r#"echo fnamemodify('one.tar.gz', ':e:e') fnamemodify('one.tar.gz', ':e:e:e') fnamemodify('logs/rec12', ':s?\d\+?X?') fnamemodify('a12b34', ':gs?\d\+?(&)?')"#, "tar.gz tar.gz logs/recX a(12)b(34)");
}

#[test]
fn blob_conversions_preserve_lines_and_requested_encodings() {
    check(r#"call assert_equal(0zE900780A79, str2blob(["é\nx", 'y'], {'encoding': 'latin1'})) | call assert_equal(["é\nx", 'y'], blob2str(0zE900780A79, {'encoding': 'latin1'})) | call assert_equal(['a', 'b'], blob2str(0z61000A006200, {'encoding': 'utf16le'})) | call assert_equal(0zFF, str2blob(["\xff"])) | echo v:errors"#, "[]");
}

#[test]
fn blob_conversion_errors_report_invalid_encoding_without_substitution() {
    let (mut core, view) = editor();
    for assertion in [
        r#"call assert_fails("call blob2str(0z80)", 'E1515:')"#,
        r#"call assert_fails("call str2blob(['ŝ'], {'encoding': 'latin1'})", 'E1516:')"#,
        r#"call assert_fails("call blob2str(0z61, {'encoding': 'unknown'})", 'E1515:')"#,
    ] {
        assert!(messages(&ex(&mut core, view, assertion)).is_empty());
    }
    assert_eq!(messages(&ex(&mut core, view, "echo v:errors")), vec!["[]"]);
}

#[test]
fn strpart_keeps_byte_start_and_counts_composing_characters_in_length() {
    check(r#"echo strpart('éabc', 2, 1, 1) strpart('abcdefg', -2) strpart("a\u0301x", 0, 1, 1) strlen(strpart("\xffx", 0, 1, 1))"#, "a abcdefg a\u{301} 1");
    check("echo strpart('éabc', 0, 1, 0)", "é");
}

#[test]
fn unicode_indices_group_composing_marks_but_separate_regional_indicators() {
    check(r#"echo strutf16len("a\u0301x") strutf16len("a\u0301x", 1) byteidx('🇨🇦x', 1) byteidx("a\u0301x", 1) byteidxcomp("a\u0301x", 1) strchars('🇨🇦x', 1)"#, "2 3 4 3 1 3");
}

#[test]
fn list_match_functions_return_item_indices_and_items() {
    check("echo matchend(['abcd'], 'bc') match(['x','b','c','b'], 'b', -1) matchstr(['abcd'], 'bc')", "0 3 abcd");
}

#[test]
fn counted_matches_include_overlapping_occurrences() {
    check("echo match('ababa', 'aba', 0, 2) matchend('ababa', 'aba', 0, 2) matchstr('ababa', 'aba', 0, 2)", "2 5 aba");
}

#[test]
fn replacement_callbacks_publish_and_restore_submatches() {
    check(r#"echo substitute('hey', 'e', {g -> submatch(0) .. g[0]}, '') substitute('ab', '\(a\)', {g -> substitute('x', 'x', {inner -> submatch(0)}, '') .. submatch(1)}, '') submatch(0)"#, "heey xab ");
}
