use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use viem_core::command::{CommandStatus, InputEvent, Key, LineMode};
use viem_core::document::{
    measure_document_work, BoundaryAffinity, Document, Encoding, FileFormat, Format, ModelRequest,
};
use viem_core::layout::{
    DocumentLayoutStyles, MeasurementEnvironmentId, MeasurementError, MetricsGeneration,
    MockTextMeasurementProvider, RenderRunPolicy, ShapeRequest, ShapedFragment,
    TextMeasurementProvider,
};
use viem_core::{Core, CoreEvent, ViewId};

fn editor(
    document: Document,
    width: f32,
    mode: LineMode,
) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 240.);
    core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
    (core, view)
}

fn run<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, command: &str) {
    for key in command
        .chars()
        .map(Key::Char)
        .chain(std::iter::once(Key::Enter))
    {
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
        let status = outcome.command.unwrap().status;
        assert!(
            matches!(status, CommandStatus::Complete | CommandStatus::Pending),
            "{command}: {status:?}"
        );
    }
}

fn input<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, command: &str) {
    for key in command.chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
    }
}

fn restored<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, before: &[u8]) {
    let after = core.document().source_bytes();
    input(core, view, "u");
    assert_eq!(core.document().source_bytes(), before);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), after);
}

#[test]
fn numeric_delete_and_yank_do_not_expand_folded_source_lines() {
    let source = "before\nfirst\nsecond\n\nafter";
    for format in [Format::PlainText, Format::Markdown] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let (mut core, view) = editor(document, 55., mode);
            run(&mut core, view, ":2,3y a");
            assert_eq!(
                core.command_state(view)
                    .unwrap()
                    .register('a')
                    .unwrap()
                    .text,
                "first\nsecond\n"
            );
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            run(&mut core, view, ":2d");
            assert_eq!(
                core.document().source_bytes(),
                b"before\nsecond\n\nafter",
                "{format:?} {mode:?}"
            );
            assert_eq!(
                core.command_state(view)
                    .unwrap()
                    .register('1')
                    .unwrap()
                    .text,
                "first\n"
            );
            restored(&mut core, view, source.as_bytes());
        }
    }
}

#[test]
fn contextual_and_mixed_ranges_delete_only_selected_wrapped_rows() {
    let source = "abc def ghi jkl mno pqr stu vwx\nlast";
    for command in [":.,.+1d", ":1,.+1d"] {
        let (mut core, view) = editor(Document::new(source), 55., LineMode::Visual);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows[2].text_range.end < source.find('\n').unwrap());
        let end = snapshot.rows[1].text_range.end;
        run(&mut core, view, command);
        assert_eq!(
            core.document().source_bytes(),
            &source.as_bytes()[end..],
            "{command}"
        );
        restored(&mut core, view, source.as_bytes());
    }
}

#[test]
fn numeric_offsets_and_counts_keep_physical_units() {
    let source = "first\nsecond\nthird\nfourth\nfifth";
    let (mut core, view) = editor(Document::new(source), 20., LineMode::Visual);
    run(&mut core, view, ":2+1d 2");
    assert_eq!(core.document().source_bytes(), b"first\nsecond\nfifth");
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn numeric_join_changes_only_real_physical_endings() {
    let source = "outside\nalpha\n  beta\nlast";
    for format in [Format::PlainText, Format::Markdown] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (mut core, view) = editor(document, 55., LineMode::Visual);
        run(&mut core, view, ":2,3join");
        assert_eq!(
            core.document().source_bytes(),
            b"outside\nalpha beta\nlast",
            "{format:?}"
        );
        restored(&mut core, view, source.as_bytes());
    }
    let source = "abc def ghi jkl mno pqr stu vwx";
    let (mut core, view) = editor(Document::new(source), 55., LineMode::Visual);
    let revision = core.document().revision();
    run(&mut core, view, ":.,.+1join");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision(), revision);
}

#[test]
fn numeric_sort_reorders_exact_source_and_preserves_style_syntax() {
    let source = "before\n# Zebra\n# Apple\n\nafter";
    for format in [Format::Markdown] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (mut core, view) = editor(document, 55., LineMode::Visual);
        run(&mut core, view, ":2,3sort");
        assert_eq!(
            core.document().source_bytes(),
            b"before\n# Apple\n# Zebra\n\nafter"
        );
        restored(&mut core, view, source.as_bytes());
    }
}

#[test]
fn physical_sort_keeps_bom_encoding_and_original_delimiter_slots() {
    let source = "before\r\nz\r\na\nmiddle\r\n";
    let expected = "before\r\na\r\nz\nmiddle\r\n";
    let encode = |text: &str| {
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        bytes
    };
    let original = encode(source);
    let document = Document::from_bytes_with_file_format(
        original.clone(),
        Encoding::Utf16Le,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let (mut core, view) = editor(document, 50., LineMode::Visual);
    run(&mut core, view, ":2,3sort");
    assert_eq!(core.document().source_bytes(), encode(expected));
    restored(&mut core, view, &original);
}

#[test]
fn numeric_source_copy_move_and_put_use_exact_destination_gaps() {
    let source = "head\n# title\n\nlast\n";
    for (command, expected) in [
        (":2copy 4", "head\n# title\n\nlast\n# title\n"),
        (":2move 4", "head\n\nlast\n# title\n"),
        (":2copy 0", "# title\nhead\n# title\n\nlast\n"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let (mut core, view) = editor(document, 80., LineMode::Visual);
        run(&mut core, view, command);
        assert_eq!(
            core.document().source_bytes(),
            expected.as_bytes(),
            "{command}"
        );
        restored(&mut core, view, source.as_bytes());
    }
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let (mut core, view) = editor(document, 80., LineMode::Visual);
    run(&mut core, view, ":2y a");
    run(&mut core, view, ":0put a");
    assert_eq!(
        core.document().source_bytes(),
        b"# title\nhead\n# title\n\nlast\n"
    );
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn numeric_source_whitespace_edits_preserve_unrelated_lines() {
    let source = "before\n  first\n\tsecond\n\nafter";
    let (mut core, view) = editor(Document::new(source), 35., LineMode::Visual);
    run(&mut core, view, ":set ts=2 sw=2 et");
    run(&mut core, view, ":2,3>");
    assert_eq!(
        core.document().source_bytes(),
        b"before\n    first\n    second\n\nafter"
    );
    restored(&mut core, view, source.as_bytes());
    input(&mut core, view, "u");
    run(&mut core, view, ":2,3left 1");
    assert_eq!(
        core.document().source_bytes(),
        b"before\n first\n second\n\nafter"
    );
}

#[test]
fn physical_retab_preserves_malformed_bytes_and_mixed_endings() {
    let original = b"keep\r\n\xff\tword\r\ntail\n".to_vec();
    let document = Document::from_bytes_with_file_format(
        original.clone(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let (mut core, view) = editor(document, 40., LineMode::Visual);
    run(&mut core, view, ":set ts=2 et");
    run(&mut core, view, ":2retab 4");
    assert_eq!(
        core.document().source_bytes(),
        b"keep\r\n\xff word\r\ntail\n"
    );
    restored(&mut core, view, &original);
}

#[test]
fn visual_copy_keeps_exact_wrapped_extent_and_character_styles() {
    let source = "**red red red red red** tail\n\nlast";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let (mut core, view) = editor(document, 45., LineMode::Visual);
    let original_text = core.document().text().to_owned();
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .clone();
    assert!(row.end < original_text.find("tail").unwrap());
    let copied = original_text[row.clone()].to_owned();
    run(&mut core, view, ":.copy $");
    assert_eq!(
        core.document().text(),
        format!("{original_text}\n{copied}\n")
    );
    let style = DocumentLayoutStyles::character_at(
        core.document().projection(),
        original_text.len() + 1,
        false,
    )
    .unwrap();
    assert!(style.bold);
    let reopened = Document::from_bytes(
        core.document().source_bytes(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    assert_eq!(reopened.text(), core.document().text());
    assert!(
        DocumentLayoutStyles::character_at(reopened.projection(), original_text.len() + 1, false)
            .unwrap()
            .bold
    );
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn physical_delete_final_unterminated_line_retains_final_terminator_policy() {
    let source = "first\nsecond\nlast";
    let (mut core, view) = editor(Document::new(source), 25., LineMode::Visual);
    run(&mut core, view, ":3d");
    assert_eq!(core.document().source_bytes(), b"first\nsecond");
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn mixed_physical_start_preserves_retained_bold_and_code_scope() {
    for source in [
        "**abc def ghi jkl mno pqr stu** tail\n\nlast",
        "```text\nabc def ghi jkl mno pqr stu\nretained\n```\n\nlast",
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let (mut core, view) = editor(document, 55., LineMode::Visual);
        let before_text = core.document().text().to_owned();
        let row = core.layout(view).unwrap().snapshot().unwrap().rows[0].clone();
        assert!(row.wraps_to_next);
        let first_retained = row.text_range.end;
        let before_style =
            DocumentLayoutStyles::character_at(core.document().projection(), first_retained, false)
                .unwrap();
        run(&mut core, view, ":1,.d");
        assert_eq!(
            core.document().text(),
            &before_text[first_retained..],
            "{source}"
        );
        let after_style =
            DocumentLayoutStyles::character_at(core.document().projection(), 0, false).unwrap();
        assert_eq!(after_style.bold, before_style.bold);
        assert_eq!(after_style.font_families, before_style.font_families);
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert_eq!(
            DocumentLayoutStyles::character_at(reopened.projection(), 0, false)
                .unwrap()
                .bold,
            before_style.bold
        );
        restored(&mut core, view, source.as_bytes());
    }
}

#[test]
fn mixed_table_edge_uses_structural_deletion_and_reopens_exactly() {
    let source =
        "| **alpha beta gamma delta** | other |\n| --- | --- |\n| body | retained |\n\nlast";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let (mut core, view) = editor(document, 100., LineMode::Visual);
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .clone();
    // Obtain the semantic reverse-rule result independently, then verify that a
    // physical first endpoint retains exactly that selection and table policy.
    let mut expected =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    expected
        .apply_model_request(ModelRequest::DeleteLines {
            document: expected.id(),
            revision: expected.revision(),
            range: row,
        })
        .unwrap();
    run(&mut core, view, ":1,.d");
    assert_eq!(core.document().text(), expected.text());
    let reopened = Document::from_bytes(
        core.document().source_bytes(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    assert_eq!(reopened.text(), expected.text());
    assert_eq!(
        reopened.projection().tables().len(),
        expected.projection().tables().len()
    );
    assert!(core.document().text().contains("retained"));
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn linewise_put_at_a_soft_row_keeps_both_real_boundaries() {
    let source = "abc def ghi jkl mno pqr stu vwx\nlast";
    let (mut core, view) = editor(Document::new(source), 55., LineMode::Visual);
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .clone();
    assert!(row.end < source.find('\n').unwrap());
    run(&mut core, view, ":.y a");
    run(&mut core, view, ":.put a");
    assert_eq!(
        core.document().text(),
        format!(
            "{}\n{}\n{}",
            &source[..row.end],
            &source[row.clone()],
            &source[row.end..]
        )
    );
    restored(&mut core, view, source.as_bytes());
}

#[test]
fn whole_file_commands_preserve_the_artifact_bom() {
    for command in [":%d", ":sort", ":retab"] {
        let bytes = [b"\xef\xbb\xbf".as_slice(), b"z\na\n".as_slice()].concat();
        let document =
            Document::from_bytes(bytes.clone(), Encoding::Utf8, Format::PlainText).unwrap();
        let (mut core, view) = editor(document, 30., LineMode::Visual);
        run(&mut core, view, command);
        assert!(
            core.document().source_bytes().starts_with(b"\xef\xbb\xbf"),
            "{command}"
        );
        if command != ":retab" {
            restored(&mut core, view, &bytes);
        }
    }
}

struct Measured {
    inner: MockTextMeasurementProvider,
    bytes: Arc<AtomicUsize>,
}
impl TextMeasurementProvider for Measured {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.inner.measurement_environment_id()
    }
    fn metrics_generation(&self) -> MetricsGeneration {
        self.inner.metrics_generation()
    }
    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.inner.render_run_policy()
    }
    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.bytes.fetch_add(
            requests
                .iter()
                .map(|request| request.text.len())
                .sum::<usize>(),
            Ordering::Relaxed,
        );
        self.inner.shape_batch(requests)
    }
}

fn late_editor() -> (Core<Measured>, ViewId, Arc<AtomicUsize>, String, usize) {
    let line = "abc def ghi jkl mno pqr stu vwx xyz";
    let source = format!("{line}\n").repeat(20_000);
    let late = (line.len() + 1) * 19_500;
    let bytes = Arc::new(AtomicUsize::new(0));
    let mut core = Core::new(Document::new(&source));
    let view = core.add_view(
        Measured {
            inner: MockTextMeasurementProvider::new(),
            bytes: bytes.clone(),
        },
        55.,
        80.,
    );
    core.handle(view, CoreEvent::SetLineMode(LineMode::Visual))
        .unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: late,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert!(
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .coverage
            .hard_lines()
            .start
            > 19_000
    );
    bytes.store(0, Ordering::Relaxed);
    (core, view, bytes, source, late)
}

#[test]
fn physical_numeric_late_mutation_uses_bounded_source_and_projection_work() {
    let (mut core, view, measured, source, late) = late_editor();
    run(&mut core, view, ":set et sw=2");
    measured.store(0, Ordering::Relaxed);
    let (_, work) = measure_document_work(|| run(&mut core, view, ":19501>"));
    assert_eq!(work.source_full_materializations, 0, "{work:?}");
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 20_000, "{work:?}");
    assert!(work.projected_formatted_bytes < 20_000, "{work:?}");
    assert!(measured.load(Ordering::Relaxed) < 20_000);
    assert_eq!(
        core.document().source_bytes(),
        format!("{}  {}", &source[..late], &source[late..]).into_bytes()
    );
    assert!(
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .coverage
            .hard_lines()
            .start
            > 19_000
    );
}

#[test]
fn late_visual_copy_does_not_materialize_or_layout_the_document_prefix() {
    let (mut core, view, measured, source, _) = late_editor();
    let rows = core.layout(view).unwrap().snapshot().unwrap().rows.clone();
    let cursor = core.command_state(view).unwrap().cursor();
    let row = rows
        .iter()
        .position(|row| row.text_range.start == cursor)
        .unwrap();
    let selected = rows[row].text_range.clone();
    let destination = rows[row + 1].text_range.end;
    assert!(rows[row].wraps_to_next && rows[row + 1].wraps_to_next);
    let (_, work) = measure_document_work(|| run(&mut core, view, ":.copy .+1"));
    assert_eq!(work.source_full_materializations, 0, "{work:?}");
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 20_000, "{work:?}");
    assert!(measured.load(Ordering::Relaxed) < 20_000);
    assert_eq!(
        core.document().source_bytes(),
        format!(
            "{}\n{}\n{}",
            &source[..destination],
            &source[selected],
            &source[destination..]
        )
        .into_bytes()
    );
    assert!(
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .coverage
            .hard_lines()
            .start
            > 19_000
    );
}

#[test]
fn physical_move_relocates_marks_and_inactive_view_cursors() {
    let source = "one\n**two**\nthree";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let (mut core, view) = editor(document, 80., LineMode::Visual);
    let other = core.add_view(MockTextMeasurementProvider::new(), 120., 120.);
    let before_offset = core.document().text().find("two").unwrap();
    core.handle(
        other,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: before_offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    input(&mut core, view, "2Gma");
    run(&mut core, view, ":2move 0");
    assert_eq!(core.document().source_bytes(), b"**two**\none\nthree");
    assert_eq!(core.command_state(other).unwrap().cursor(), 0);
    input(&mut core, view, "G`a");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    input(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.command_state(other).unwrap().cursor(), before_offset);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"**two**\none\nthree");
    assert_eq!(core.command_state(other).unwrap().cursor(), 0);
    input(&mut core, view, "G`a");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
}

#[test]
fn physical_sort_relocates_marks_and_inactive_cursors_with_original_bytes() {
    let source = "head\n**zebra**\n**apple**\ntail";
    for format in [Format::PlainText, Format::Markdown] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (mut core, view) = editor(document, 80., LineMode::Visual);
        let other = core.add_view(MockTextMeasurementProvider::new(), 120., 120.);
        let zebra = core.document().text().find("zebra").unwrap();
        let apple = core.document().text().find("apple").unwrap();
        for (target, offset) in [(view, zebra), (other, apple)] {
            core.handle(
                target,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: offset,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
        }
        input(&mut core, view, "ma");
        run(&mut core, view, ":2,3sort");
        assert_eq!(
            core.document().source_bytes(),
            b"head\n**apple**\n**zebra**\ntail",
            "{format:?}"
        );
        assert_eq!(
            core.command_state(other).unwrap().cursor(),
            core.document().text().find("apple").unwrap(),
            "{format:?}"
        );
        input(&mut core, view, "G`a");
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            core.document().text().find("zebra").unwrap(),
            "{format:?}"
        );
        input(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(
            core.command_state(other).unwrap().cursor(),
            apple,
            "{format:?}"
        );
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(
            core.command_state(other).unwrap().cursor(),
            core.document().text().find("apple").unwrap(),
            "{format:?}"
        );
        input(&mut core, view, "G`a");
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            core.document().text().find("zebra").unwrap(),
            "{format:?}"
        );
    }
}

#[test]
fn mixed_transfers_and_sort_reject_partial_formatting_owners_atomically() {
    let cases = [
        (
            "**zebra zebra zebra\napple apple apple retained** tail\n\nlast",
            "apple",
            vec![":1,.copy $", ":1,.move $", ":1,.sort!"],
        ),
        (
            "```text\nabc def ghi jkl mno pqr stu\nretained\n```\n\nlast",
            "abc",
            vec![":1,.copy $", ":1,.move $"],
        ),
        (
            "| **alpha beta gamma delta** | other |\n| --- | --- |\n| body | retained |\n\nlast",
            "alpha",
            vec![":1,.copy $", ":1,.move $"],
        ),
    ];
    for (source, target, commands) in cases {
        for command in commands {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let (mut core, view) = editor(document, 55., LineMode::Visual);
            let at = core.document().text().find(target).unwrap();
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            run(&mut core, view, ":.y a");
            let cursor = core.command_state(view).unwrap().cursor();
            let mode = core.command_state(view).unwrap().mode();
            let register = core.command_state(view).unwrap().register('a').cloned();
            let revision = core.document().revision();
            let history = core.document().history_status();
            let text = core.document().text().to_owned();
            input(&mut core, view, command);
            let output = core
                .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap();
            assert!(
                matches!(output.command.unwrap().status, CommandStatus::ExError(_)),
                "{command}: {source}"
            );
            assert_eq!(
                core.document().source_bytes(),
                source.as_bytes(),
                "{command}"
            );
            assert_eq!(core.document().text(), text);
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status(), history);
            assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            assert_eq!(
                core.command_state(view).unwrap().register('a').cloned(),
                register
            );
        }
    }
}

#[test]
fn numeric_goto_does_not_decode_a_giant_target_line_body() {
    use viem_core::command::ex::parse_ex;
    use viem_core::command::ex_execute::{
        execute_ex, ExExecutionContext, ExExecutionState, ExNavigation,
    };
    let source = format!("before\n  {}\nafter", "x".repeat(2_000_000));
    for format in [Format::PlainText, Format::Markdown] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (outcome, work) = measure_document_work(|| {
            execute_ex(
                &mut document,
                &mut ExExecutionState::default(),
                &ExExecutionContext::default(),
                &parse_ex(":2").unwrap(),
                &(),
            )
            .unwrap()
        });
        assert_eq!(
            outcome.navigation,
            Some(ExNavigation::PhysicalSourceOffset("before\n  ".len()))
        );
        assert_eq!(work.source_full_materializations, 0, "{format:?}: {work:?}");
        assert_eq!(
            work.formatted_full_materialized_bytes, 0,
            "{format:?}: {work:?}"
        );
        assert!(
            work.source_range_materialized_bytes < 4_096,
            "{format:?}: {work:?}"
        );
        assert!(work.source_decoded_bytes < 32, "{format:?}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{format:?}: {work:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn visual_row_copy_and_move_to_numeric_destination_keep_styles_and_breaks() {
    let source = "**red red red red red** tail\n\nlast";
    for moving in [false, true] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let (mut core, view) = editor(document, 45., LineMode::Visual);
        let text = core.document().text().to_owned();
        let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
            .text_range
            .clone();
        let copied = text[row.clone()].to_owned();
        assert!(row.end < text.find("tail").unwrap());
        run(
            &mut core,
            view,
            if moving { ":.move 3" } else { ":.copy 3" },
        );
        let retained = if moving { &text[row.end..] } else { &text };
        assert_eq!(core.document().text(), format!("{retained}\n{copied}\n"));
        let inserted = retained.len() + 1;
        assert!(
            DocumentLayoutStyles::character_at(core.document().projection(), inserted, false)
                .unwrap()
                .bold
        );
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert!(
            DocumentLayoutStyles::character_at(reopened.projection(), inserted, false)
                .unwrap()
                .bold
        );
        restored(&mut core, view, source.as_bytes());
    }
}

#[test]
fn visual_transfer_into_hidden_physical_boundary_rejects_atomically() {
    let source = "**red red red red red** tail\n\n**last**";
    for command in [":.copy 3", ":.move 3"] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let (mut core, view) = editor(document, 45., LineMode::Visual);
        let revision = core.document().revision();
        let history = core.document().history_status();
        let cursor = core.command_state(view).unwrap().cursor();
        input(&mut core, view, command);
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert!(matches!(
            output.command.unwrap().status,
            CommandStatus::ExError(_)
        ));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    }
}
