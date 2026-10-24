//! Preview formatting selects displayed text but changes only source wrapping.
//! A source line can span many displayed rows: rounding a selection back out to
//! that source line would silently format content outside the requested motion.
use std::ops::Range;
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, FileFormat};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

fn markdown(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn open(source: &str, width: f32, text_width: u32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(markdown(source));
    core.set_text_width_default(text_width);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 2000.);
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert!(outcome.command.as_ref().is_none_or(|command| matches!(
        command.status, CommandStatus::Complete | CommandStatus::Pending
    )), "{outcome:?}");
}

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for character in text.chars() { input(core, view, Key::Char(character)); }
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
}

fn rows(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> Vec<Range<usize>> {
    core.layout(view).unwrap().snapshot().unwrap().rows.iter()
        .map(|row| row.text_range.clone()).collect()
}

fn source(core: &Core<MockTextMeasurementProvider>) -> String {
    String::from_utf8(core.document().source_bytes()).unwrap()
}

fn assert_same_preview(expected: &Document, actual: &Document) {
    assert_eq!(actual.text(), expected.text());
    assert_eq!(actual.projection().style_spans(), expected.projection().style_spans());
    assert_eq!(actual.projection().blocks().len(), expected.projection().blocks().len());
    for (before, after) in expected.projection().blocks().iter().zip(actual.projection().blocks()) {
        assert_eq!(after.range, before.range);
        // Equal visible text is insufficient: an accidental heading, quote,
        // list restart, or lost code owner must fail this assertion as well.
        assert_eq!(after.attributes, before.attributes);
    }
    assert_eq!(actual.hard_line_snapshot().capture(0..actual.text().len()).unwrap().break_offsets(),
        expected.hard_line_snapshot().capture(0..expected.text().len()).unwrap().break_offsets());
}

const LONG: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu";

#[test]
fn motions_doubled_forms_and_counts_select_preview_rows_not_the_source_line() {
    for (command, count, keep_cursor) in [
        ("gqq", 1, false), ("gqgq", 1, false), ("gww", 1, true), ("gwgw", 1, true),
        ("gqj", 2, false), ("gwj", 2, true), ("2gqq", 2, false),
        ("gq2q", 2, false), ("2gq2q", 4, false), ("gq2j", 3, false),
        ("Vjgq", 2, false),
    ] {
        let (mut core, view) = open(LONG, 180., 9);
        let displayed = rows(&core, view);
        assert!(displayed.len() > count, "fixture must have an unselected suffix");
        let untouched = &LONG[displayed[count].start..];
        keys(&mut core, view, command);
        let after = source(&core);
        assert_ne!(after, LONG, "{command} should format eligible selected prose");
        assert!(after.ends_with(untouched), "{command} expanded past the selected rows: {after:?}");
        assert_same_preview(&markdown(LONG), core.document());
        assert_eq!(core.command_state(view).unwrap().cursor(),
            if keep_cursor { 0 } else { displayed[count - 1].start }, "{command}");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    }
}

#[test]
fn paragraph_formatting_is_source_only_and_repeated_formatting_is_stable() {
    let initial = "aaa bbb\nccc ddd eee fff\n\nUnchanged paragraph";
    let (mut core, view) = open(initial, 300., 7);
    let before = markdown(initial);
    keys(&mut core, view, "gqip");
    assert_eq!(source(&core), "aaa bbb\nccc ddd\neee fff\n\nUnchanged paragraph");
    assert_same_preview(&before, core.document());
    let revision = core.document().revision();
    keys(&mut core, view, "gqip");
    assert_eq!(core.document().revision(), revision, "an already pretty paragraph is not an edit");
}

#[test]
fn paragraph_operator_caret_uses_preview_rows_instead_of_new_source_breaks() {
    for command in ["gqip", "gwip"] {
        let (mut core, view) = open(LONG, 180., 9);
        let displayed = rows(&core, view);
        let original = displayed[1].start;
        place(&mut core, view, original);
        keys(&mut core, view, command);
        assert_eq!(core.command_state(view).unwrap().cursor(),
            if command == "gwip" { original } else { displayed.last().unwrap().start },
            "new source breaks are invisible and must not choose the Preview caret");
        assert_same_preview(&markdown(LONG), core.document());
    }
}

#[test]
fn formatting_an_empty_final_preview_row_does_not_select_preceding_prose() {
    let initial = "alpha beta gamma delta epsilon\n\n\n\n";
    let (mut core, view) = open(initial, 500., 7);
    let displayed = rows(&core, view);
    let last = displayed.last().unwrap();
    assert!(last.is_empty(), "fixture needs a final empty paragraph: {displayed:?}");
    place(&mut core, view, last.start);
    keys(&mut core, view, "gqq");
    assert_eq!(source(&core), initial);
    assert_eq!(core.command_state(view).unwrap().cursor(), last.start);
}

#[test]
fn reflow_finishes_on_an_explicitly_selected_empty_row() {
    let initial = format!("{LONG}\n\n\n\n");
    for command in ["gqj", "2gqq", "Vjgq"] {
        let (mut core, view) = open(&initial, 180., 7);
        let displayed = rows(&core, view);
        let nonempty = displayed.iter().rposition(|row| !row.is_empty()).unwrap();
        let empty = &displayed[nonempty + 1];
        assert!(empty.is_empty());
        place(&mut core, view, displayed[nonempty].start);
        keys(&mut core, view, command);
        assert_eq!(core.command_state(view).unwrap().cursor(), empty.start,
            "{command}: an empty selected row is a destination, not the previous grapheme");
        assert_same_preview(&markdown(&initial), core.document());
    }
}

#[test]
fn partial_fragment_accounts_for_the_existing_source_column_without_expansion() {
    let initial = "alpha beta gamma delta epsilon zeta tail";
    let mut document = markdown(initial);
    // The selected fragment starts at column 11. It cannot be formatted as
    // a new paragraph or acquire a break in the unselected preceding space.
    let start = initial.find("gamma").unwrap();
    let end = initial.find(" zeta").unwrap();
    assert!(document.reflow_markdown(start..end, 12, 2).unwrap());
    assert_eq!(document.source_bytes(), b"alpha beta gamma\ndelta\nepsilon zeta tail");
    assert_same_preview(&markdown(initial), &document);
}

#[test]
fn quote_and_list_continuations_preserve_their_owners_and_neighboring_items() {
    for (initial, expected) in [
        ("> aaa bbb ccc ddd eee fff", "> aaa bbb\n> ccc ddd\n> eee fff"),
        ("- aaa bbb ccc ddd eee fff\n- untouched", "- aaa bbb\n  ccc ddd\n  eee fff\n- untouched"),
        ("> - aaa bbb ccc ddd eee fff", "> - aaa\n>   bbb\n>   ccc\n>   ddd\n>   eee\n>   fff"),
    ] {
        let (mut core, view) = open(initial, 500., 9);
        keys(&mut core, view, "gqq");
        assert_eq!(source(&core), expected, "{initial:?}");
        assert_same_preview(&markdown(initial), core.document());
        assert_same_preview(core.document(), &markdown(&source(&core)));
    }
}

#[test]
fn existing_lazy_quote_and_list_continuations_join_without_leaving_hidden_prefixes() {
    for (initial, expected) in [
        ("> aaa\nbbb ccc ddd", "> aaa bbb ccc ddd"),
        ("> aaa\n> bbb\n> ccc ddd", "> aaa bbb ccc ddd"),
        ("- aaa\n  bbb ccc\n  ddd", "- aaa bbb ccc ddd"),
        ("3. aaa\n   bbb ccc\n   ddd", "3. aaa bbb ccc ddd"),
        ("> - aaa\n>   bbb ccc\n>   ddd", "> - aaa bbb ccc ddd"),
    ] {
        let (mut core, view) = open(initial, 500., 80);
        keys(&mut core, view, "gqip");
        assert_eq!(source(&core), expected, "{initial:?}");
        assert_same_preview(&markdown(initial), core.document());
        assert_same_preview(core.document(), &markdown(&source(&core)));
        keys(&mut core, view, "u");
        assert_eq!(source(&core), initial, "undo restores the author's prefix spelling");
    }
    // A lazy continuation can be joined, then wrapped elsewhere. The inserted
    // source line must receive the quote prefix from ownership, not from the
    // unprefixed physical line on which the selected words happened to live.
    let initial = "> aaa\nbbb ccc ddd";
    let (mut core, view) = open(initial, 500., 9);
    keys(&mut core, view, "gqip");
    assert_eq!(source(&core), "> aaa bbb\n> ccc ddd");
    assert_same_preview(&markdown(initial), core.document());
}

#[test]
fn authored_space_runs_tabs_and_entities_are_not_normalized_by_reflow() {
    for (initial, expected) in [
        ("alpha  beta gamma delta", "alpha  beta\ngamma delta"),
        ("alpha\tbeta gamma delta", "alpha\tbeta\ngamma delta"),
        ("alpha&#32;beta gamma delta", "alpha&#32;beta\ngamma delta"),
        ("alpha\u{a0}beta gamma delta", "alpha\u{a0}beta\ngamma delta"),
        // The accent belongs to its space grapheme. Replacing only the ASCII
        // scalar would move that accent to another character after reparsing.
        ("alpha beta \u{301} gamma delta", "alpha beta \u{301}\ngamma delta"),
    ] {
        let (mut core, view) = open(initial, 500., 12);
        keys(&mut core, view, "gqip");
        assert_eq!(source(&core), expected, "{initial:?}");
        assert_same_preview(&markdown(initial), core.document());
    }
}

#[test]
fn mixed_selection_formats_prose_without_altering_fixed_blocks_or_blank_paragraphs() {
    let fixed = "\n\n```\ncode words stay on their original line\n```\n\n\n\n# Long heading stays on one source line\n\n| alpha | beta |\n| --- | --- |\n| one | two |\n\n";
    let initial = format!("aaa bbb ccc ddd{fixed}eee fff ggg hhh");
    let (mut core, view) = open(&initial, 500., 7);
    keys(&mut core, view, "gqG");
    assert_eq!(source(&core), format!("aaa bbb\nccc ddd{fixed}eee fff\nggg hhh"));
    assert_same_preview(&markdown(&initial), core.document());
}

#[test]
fn linewise_reflow_can_cross_a_table_boundary_and_format_the_selected_prose() {
    let table = "| name |\n| --- |\n| cell |";
    for (initial, start_word, expected) in [
        (format!("{table}\n\nalpha beta gamma delta"), "cell",
            format!("{table}\n\nalpha beta\ngamma delta")),
        (format!("alpha beta gamma delta\n\n{table}"), "alpha",
            format!("alpha beta\ngamma delta\n\n{table}")),
    ] {
        let (mut core, view) = open(&initial, 500., 11);
        let at = core.document().text().find(start_word).unwrap();
        place(&mut core, view, at);
        // A table cell owns special motion boundaries for other operators.
        // Reflow must still cover the checked displayed rows, skip the table,
        // and format eligible prose rather than failing the entire selection.
        keys(&mut core, view, "gqj");
        assert_eq!(source(&core), expected);
        assert_same_preview(&markdown(&initial), core.document());
    }
}

#[test]
fn protected_owners_are_successful_noops_without_history() {
    for initial in [
        "```\nalpha beta gamma delta\n```", "    alpha beta gamma delta",
        "---\nname: alpha beta gamma delta\n---",
        "<pre>alpha beta gamma delta</pre>", "# alpha beta gamma delta",
        "| alpha beta | gamma delta |\n| --- | --- |\n| epsilon | zeta |",
        "\n\n\n\n",
    ] {
        let (mut core, view) = open(initial, 500., 5);
        let revision = core.document().revision();
        keys(&mut core, view, "gqG");
        assert_eq!(source(&core), initial);
        assert_eq!(core.document().revision(), revision, "{initial:?}");
    }
}

#[test]
fn inline_syntax_and_line_start_punctuation_keep_their_meaning() {
    for initial in [
        "alpha beta # heading gamma delta",
        "alpha beta 1. item gamma delta",
        "alpha beta --- gamma delta",
        "alpha [linked words](https://example.test/a/b \"link title\") beta gamma",
        "alpha `literal code words` beta gamma",
        "alpha **bold words** beta &amp; gamma delta",
        "alpha ![image words](local.png) beta gamma delta",
    ] {
        let (mut core, view) = open(initial, 500., 12);
        keys(&mut core, view, "gqG");
        assert_same_preview(&markdown(initial), core.document());
        assert_same_preview(core.document(), &markdown(&source(&core)));
        for protected in ["[linked words](https://example.test/a/b \"link title\")", "`literal code words`", "![image words](local.png)", "&amp;"] {
            if initial.contains(protected) { assert!(source(&core).contains(protected), "{protected}"); }
        }
        if initial.contains("linked words") {
            let at = core.document().text().find("linked").unwrap();
            assert_eq!(core.document().link_at(core.document().text_point(at).unwrap()).unwrap(),
                Some("https://example.test/a/b".into()));
        }
    }
}

#[test]
fn valid_inline_syntax_can_start_a_wrapped_source_line() {
    for token in ["**beta**", "*beta*", "_beta_", "~~beta~~", "`beta`", "[beta](url)"] {
        let initial = format!("alpha {token}");
        let (mut core, view) = open(&initial, 500., 12);
        keys(&mut core, view, "gqip");
        if initial.len() > 12 {
            assert_eq!(source(&core), format!("alpha\n{token}"));
        }
        assert_same_preview(&markdown(&initial), core.document());
    }
}

#[test]
fn explicit_hard_breaks_and_surplus_blank_paragraphs_survive() {
    for break_spelling in ["  \n", "\\\n", "<br>"] {
        let initial = format!("aaa bbb ccc{break_spelling}ddd eee fff\n\n\n\nlast paragraph words");
        let (mut core, view) = open(&initial, 500., 7);
        keys(&mut core, view, "gqG");
        assert!(source(&core).contains(break_spelling));
        assert!(source(&core).contains("\n\n\n\n"));
        assert_same_preview(&markdown(&initial), core.document());
    }
}

#[test]
fn hard_break_spelling_survives_when_both_adjacent_prose_runs_rewrap() {
    for break_spelling in ["  \n", "\\\n"] {
        let initial = format!("aaa bbb ccc{break_spelling}ddd eee fff");
        let (mut core, view) = open(&initial, 500., 7);
        keys(&mut core, view, "gqip");
        assert_eq!(source(&core), format!("aaa bbb\nccc{break_spelling}ddd eee\nfff"));
        assert_same_preview(&markdown(&initial), core.document());
    }
}

#[test]
fn source_rewrapping_is_one_undo_unit_and_leaves_registers_and_other_view_carets_alone() {
    let initial = "aaa bbb ccc ddd eee fff";
    let (mut core, view) = open(initial, 180., 7);
    let other = core.add_view(MockTextMeasurementProvider::new(), 400., 2000.);
    place(&mut core, other, 12);
    keys(&mut core, view, "\"ayiw");
    let unnamed = core.command_state(view).unwrap().register('"').cloned();
    let named = core.command_state(view).unwrap().register('a').cloned();
    place(&mut core, view, 4);
    keys(&mut core, view, "\"bgwip");
    let after = source(&core);
    assert_eq!(after, "aaa bbb\nccc ddd\neee fff");
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    assert_eq!(core.command_state(other).unwrap().cursor(), 12);
    assert_eq!(core.command_state(view).unwrap().register('"').cloned(), unnamed);
    assert_eq!(core.command_state(view).unwrap().register('a').cloned(), named);
    assert!(core.command_state(view).unwrap().register('b').is_none());
    keys(&mut core, view, "u");
    assert_eq!(source(&core), initial);
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    input(&mut core, view, Key::Ctrl('r'));
    assert_eq!(source(&core), after);
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
}

#[test]
fn unicode_dos_endings_and_encoding_survive_rewrap_and_exact_undo() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Latin1] {
        let initial = "éé aaa bbb ccc\r\nddd\r\n\r\nuntouched\r\n";
        let bytes = match encoding {
            Encoding::Utf16Le => initial.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Latin1 => initial.chars().map(|character| character as u8).collect(),
            _ => initial.as_bytes().to_vec(),
        };
        let before = Document::from_bytes_with_file_format(bytes.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        let mut core = Core::new(Document::from_bytes_with_file_format(bytes.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap());
        core.set_text_width_default(6);
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 2000.);
        keys(&mut core, view, "gqip");
        assert_ne!(core.document().source_bytes(), bytes);
        assert_eq!(core.document().encoding(), encoding);
        let after = core.document().source_bytes();
        let reopened = Document::from_bytes_with_file_format(after, encoding, Format::Markdown, FileFormat::Dos).unwrap();
        assert_same_preview(&before, &reopened);
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), bytes, "{encoding:?}");
    }
}

#[test]
fn literal_lf_in_a_mac_source_line_never_resets_its_reflow_column() {
    // The parser tags only CR as a physical ending here. Raw LF is content
    // even though actual source endings also normalize to LF internally.
    for (initial, width, expected) in [
        ("aaaa\nbbbb cccc dddd", 10, "aaaa\nbbbb\rcccc dddd"),
        ("aa bb\ncccc dd", 8, "aa\rbb\ncccc\rdd"),
    ] {
        let mut document = Document::from_bytes_with_file_format(initial.as_bytes().to_vec(),
            Encoding::Utf8, Format::Markdown, FileFormat::Mac).unwrap();
        let before = document.text().to_owned();
        assert!(document.reflow_markdown(0..before.len(), width, 2).unwrap());
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(document.text(), before);
        let reopened = Document::from_bytes_with_file_format(document.source_bytes(),
            Encoding::Utf8, Format::Markdown, FileFormat::Mac).unwrap();
        assert_same_preview(&document, &reopened);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), initial.as_bytes());
    }
}

#[test]
fn dot_repeats_the_preview_extent_at_its_new_location() {
    let (mut core, view) = open(LONG, 180., 9);
    let displayed = rows(&core, view);
    keys(&mut core, view, "gqq");
    let first = source(&core);
    place(&mut core, view, displayed[2].start);
    keys(&mut core, view, ".");
    assert_ne!(source(&core), first);
    assert!(source(&core).ends_with(&LONG[displayed[3].start..]));
    assert_same_preview(&markdown(LONG), core.document());
}

#[test]
fn code_reflow_still_counts_hard_lines_when_the_view_soft_wraps() {
    let initial = format!("{LONG}\nsecond source line\nthird source line");
    let mut core = Core::new(Document::from_bytes(initial.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap());
    core.set_text_width_default(1000);
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 2000.);
    keys(&mut core, view, ":set wrap");
    input(&mut core, view, Key::Enter);
    assert!(rows(&core, view).len() > 3, "source lines must soft-wrap in this fixture");
    keys(&mut core, view, "gqj");
    assert_eq!(source(&core), format!("{LONG} second source line\nthird source line"));
}

#[test]
fn local_source_reflow_does_not_scale_with_an_unrelated_suffix_or_flatten_snapshots() {
    use viem_core::document::measure_document_work;
    let mut small_work = None;
    for paragraphs in [100, 10_000] {
        let first = "aaa bbb ccc ddd";
        let suffix = "\n\nUntouched words and **formatting**".repeat(paragraphs);
        let initial = format!("{first}{suffix}");
        let mut document = markdown(&initial);
        let (result, work) = measure_document_work(|| document.reflow_markdown(0..first.len(), 7, 2));
        assert!(result.unwrap());
        // Check before asking for full text/source in the assertions below.
        // Both historical and current snapshots must remain streaming data.
        assert_eq!(work.full_projection_candidates, 0, "{paragraphs}: {work:?}");
        assert_eq!(work.source_full_materialized_bytes, 0, "{paragraphs}: {work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{paragraphs}: {work:?}");
        assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{paragraphs}: {work:?}");
        assert_eq!(work.compatibility_record_materializations, 0, "{paragraphs}: {work:?}");
        assert!(work.source_decoded_bytes < 8192, "{paragraphs}: {work:?}");
        assert!(work.projected_formatted_bytes < 8192, "{paragraphs}: {work:?}");
        if let Some(previous) = small_work {
            assert!(work.source_decoded_bytes <= previous + 512,
                "growing an unrelated suffix increased local decoding: {work:?}");
        } else {
            small_work = Some(work.source_decoded_bytes);
        }
        assert_eq!(document.source_bytes(), format!("aaa bbb\nccc ddd{suffix}").as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), initial.as_bytes());
    }
}
