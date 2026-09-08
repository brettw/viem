use evim_core::command::{CommandStatus, InputEvent, Key, LineMode};
use evim_core::document::{Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

fn editor(document: Document, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 200.0);
    (core, view)
}
fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    let status = result.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending),
        "{key:?}: {status:?}"
    );
}
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for c in input.chars() {
        key(core, view, Key::Char(c));
    }
}
#[test]
fn visual_row_end_delete_counts_repeat_and_exact_undo() {
    let original = "abcdefghijklmno\nsecond paragraph";
    let (mut core, view) = editor(Document::new(original), 55.0);
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .clone();
    assert!(row.end < original.find('\n').unwrap());
    keys(&mut core, view, "$");
    assert_eq!(core.command_state(view).unwrap().cursor(), row.end - 1);
    keys(&mut core, view, "0\"add");
    assert_eq!(core.document().text(), &original[row.end..]);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        &original[row.clone()]
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
    keys(&mut core, view, "2dd");
    let after = core.document().text().to_owned();
    assert!(after.len() < original.len() - row.len());
    keys(&mut core, view, ".");
    assert!(core.document().text().len() < after.len());
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), after);
}
#[test]
fn physical_html_deletes_authoritative_line_and_replays_without_touching_neighbors() {
    let source = "<p>first</p>\n<!-- keep --><p>second</p>\n<p>third</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let (mut core, view) = editor(document, 50.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "\"add");
    assert_eq!(
        core.document().source_bytes(),
        b"<!-- keep --><p>second</p>\n<p>third</p>"
    );
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        "<p>first</p>\n"
    );
    keys(&mut core, view, ".");
    assert_eq!(core.document().source_bytes(), b"<p>third</p>");
    keys(&mut core, view, "u");
    assert_eq!(
        core.document().source_bytes(),
        b"<!-- keep --><p>second</p>\n<p>third</p>"
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn mode_is_view_local_and_rtf_rejects_physical() {
    let (mut core, first) = editor(Document::new("abc\ndef"), 50.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 50.0, 200.0);
    core.handle(first, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    assert_eq!(
        core.command_state(second).unwrap().line_mode(),
        LineMode::Visual
    );
    let (mut rich, view) = editor(
        Document::from_bytes(br"{\rtf1 Text}".to_vec(), Encoding::Utf8, Format::Rtf).unwrap(),
        50.0,
    );
    assert!(rich
        .handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .is_err());
    assert_eq!(
        rich.command_state(view).unwrap().line_mode(),
        LineMode::Visual
    );
}

#[test]
fn row_change_and_physical_change_have_one_undo_unit() {
    for mode in [LineMode::Visual, LineMode::PhysicalSource] {
        let original = "abcdefghijklmno\nsecond";
        let (mut core, view) = editor(Document::new(original), 55.0);
        core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
        let end = if mode == LineMode::Visual {
            core.layout(view).unwrap().snapshot().unwrap().rows[0]
                .text_range
                .end
        } else {
            original.find('\n').unwrap()
        };
        keys(&mut core, view, "llC");
        core.handle(view, CoreEvent::Input(InputEvent::Text("XYZ".into())))
            .unwrap();
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), format!("abXYZ{}", &original[end..]));
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), original);
    }
}
#[test]
fn physical_line_register_put_retains_html_source_syntax() {
    let source = "<p>one</p>\n<p>two</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let (mut core, view) = editor(document, 70.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "\"ayyj\"a2p");
    assert_eq!(
        core.document().source_bytes(),
        b"<p>one</p>\n<p>two</p>\n<p>one</p>\n<p>one</p>"
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn macro_line_delete_relayouts_each_event_and_is_one_undo_unit() {
    let original = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\nTail";
    let (mut core, view) = editor(Document::new(original), 55.0);
    keys(&mut core, view, "qaddq");
    let recorded = core.document().text().to_owned();
    keys(&mut core, view, "2@a");
    let played = core.document().text().to_owned();
    assert!(played.len() < recorded.len());
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), recorded);
    keys(&mut core, view, "@@");
    assert!(core.document().text().len() < recorded.len());
}
#[test]
fn location_and_width_changes_keep_large_document_layout_local() {
    let original = (0..20_000)
        .map(|n| format!("row {n} abcdefghijklmnopqrstuvwxyz\n"))
        .collect::<String>();
    let (mut core, view) = editor(Document::new(original), 80.0);
    keys(&mut core, view, "G");
    let before = core.layout(view).unwrap().regional_cache_statistics();
    let location = core.line_location(view).unwrap();
    assert_eq!(location.line, None);
    assert_eq!(location.hard_line, 20_001);
    assert_eq!(location.fragment, 1);
    assert_eq!(
        core.layout(view).unwrap().regional_cache_statistics(),
        before
    );
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    assert_eq!(core.line_location(view).unwrap().line, Some(20_001));
    assert!(
        core.layout(view)
            .unwrap()
            .regional_cache_statistics()
            .hard_line_count()
            < 500
    );
    core.handle(view, CoreEvent::SetLineMode(LineMode::Visual))
        .unwrap();
    keys(&mut core, view, "gg");
    assert_eq!(core.line_location(view).unwrap().line, Some(1));
    keys(&mut core, view, "dd");
    assert!(
        core.layout(view)
            .unwrap()
            .regional_cache_statistics()
            .hard_line_count()
            < 500
    );
}
#[test]
fn physical_navigation_can_address_invisible_comment_lines() {
    let source = "<p>one</p>\n<!-- invisible -->\n<p>two</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let (mut core, view) = editor(document, 90.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "j");
    assert_eq!(core.line_location(view).unwrap().line, Some(2));
    keys(&mut core, view, "dd");
    assert_eq!(core.document().source_bytes(), b"<p>one</p>\n<p>two</p>");
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
#[test]
fn physical_columns_indent_and_format_switch_policy() {
    let (mut core, view) = editor(Document::new("abcde\nx\nabcde"), 90.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "lljj");
    assert_eq!(core.line_location(view).unwrap().column, 3);
    keys(&mut core, view, "gg2>>");
    assert_eq!(core.document().text(), "    abcde\n    x\nabcde");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "abcde\nx\nabcde");
    core.handle(
        view,
        CoreEvent::SetFormat {
            document: core.document().id(),
            revision: core.document().revision(),
            target: Format::Rtf,
        },
    )
    .unwrap();
    assert_eq!(
        core.command_state(view).unwrap().line_mode(),
        LineMode::Visual
    );
}
#[test]
fn width_change_invalidates_visual_command_rows_but_not_source_lines() {
    let (mut core, view) = editor(Document::new("abcdefghijklmnopqrstuvwxyz"), 100.0);
    let old = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .end;
    core.handle(
        view,
        CoreEvent::Resize {
            width: 45.0,
            height: 200.0,
        },
    )
    .unwrap();
    let new = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .end;
    assert!(new < old);
    keys(&mut core, view, "$");
    assert_eq!(core.command_state(view).unwrap().cursor(), new - 1);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "$");
    assert_eq!(core.command_state(view).unwrap().cursor(), 25);
}

#[test]
fn visual_row_delete_keeps_list_marker_when_item_continues_and_handles_nested_formatting() {
    for (format, source) in [
        (
            Format::Html,
            "<ul><li><b>abcdefgh ijklmno</b> pqrst uvwxyz</li></ul><!-- keep -->",
        ),
        (
            Format::Rtf,
            r"{\rtf1{\*\unknown keep}{\pn\pnlvlblt}{\b abcdefgh ijklmno} pqrst uvwxyz\par}",
        ),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (mut core, view) = editor(document, 180.0);
        let original = core.document().text().to_owned();
        let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
            .text_range
            .clone();
        assert!(row.end < original.len(), "{format:?} row must wrap");
        keys(&mut core, view, "dd");
        assert_eq!(core.document().text(), &original[row.end..], "{format:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
#[test]
fn visual_line_selection_and_insert_placements_follow_row_boundaries() {
    let original = "abcdefgh ijklmnop qrstuvwxyz\nTail";
    let (mut core, view) = editor(Document::new(original), 90.0);
    let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .clone();
    keys(&mut core, view, "Vd");
    assert_eq!(core.document().text(), &original[row.end..]);
    keys(&mut core, view, "uA");
    core.handle(view, CoreEvent::Input(InputEvent::Text("!".into())))
        .unwrap();
    key(&mut core, view, Key::Escape);
    assert_eq!(
        core.document().text(),
        format!("{}!{}", &original[..row.end], &original[row.end..])
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
}
#[test]
fn narrow_list_row_contains_body_text_and_partial_deletion_retains_decoration() {
    let source = "<ul><li>abcdefghijklmnopqrstuvwxyz</li></ul>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let (mut core, view) = editor(document, 80.0);
    let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
    let end = row.text_range.end;
    assert!(!row.text_range.is_empty());
    assert_eq!(row.decorations.len(), 1);
    assert_eq!(row.clusters[0].text_range.start, 0);
    keys(&mut core, view, "dd");
    assert_eq!(core.document().text(), &"abcdefghijklmnopqrstuvwxyz"[end..]);
    assert_eq!(core.document().projection().list_structure().lists.len(), 1);
    assert_eq!(
        core.layout(view).unwrap().snapshot().unwrap().rows[0]
            .decorations
            .len(),
        1
    );
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn counted_visual_line_and_hidden_source_visual_line_use_the_selected_domain() {
    let original = "abcdefgh ijklmnop qrstuv wxyz\nTail";
    let (mut core, view) = editor(Document::new(original), 90.0);
    let end = core.layout(view).unwrap().snapshot().unwrap().rows[1]
        .text_range
        .end;
    keys(&mut core, view, "2Vd");
    assert_eq!(core.document().text(), &original[end..]);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
    let source = "<p>one</p>\n<!-- invisible -->\n<p>two</p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let (mut core, view) = editor(document, 90.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "jVd");
    assert_eq!(core.document().source_bytes(), b"<p>one</p>\n<p>two</p>");
}
#[test]
fn change_complete_rich_line_keeps_following_paragraph_and_one_undo() {
    for (format, source) in [
        (Format::Html, "<p><b>one</b></p><p>two</p>"),
        (Format::Rtf, r"{\rtf1{\b one}\par two}"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let (mut core, view) = editor(document, 300.0);
        keys(&mut core, view, "cc");
        core.handle(view, CoreEvent::Input(InputEvent::Text("New".into())))
            .unwrap();
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), "New\ntwo", "{format:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
#[test]
fn first_nonblank_operator_preserves_leading_indent() {
    for mode in [LineMode::Visual, LineMode::PhysicalSource] {
        let (mut core, view) = editor(Document::new("  abcdef"), 200.0);
        core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
        keys(&mut core, view, "5|d^");
        assert_eq!(core.document().text(), "  cdef");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "  abcdef");
    }
}
#[test]
fn indentation_of_visual_rows_is_batched_counted_and_repeatable() {
    let original = "abcdefgh ijklmnop qrstuv wxyz";
    let (mut core, view) = editor(Document::new(original), 100.0);
    let second = core.layout(view).unwrap().snapshot().unwrap().rows[1]
        .text_range
        .start;
    keys(&mut core, view, "2>>");
    assert_eq!(
        core.document().text(),
        format!("    {}    {}", &original[..second], &original[second..])
    );
    keys(&mut core, view, "uV2>");
    assert!(core.document().text().starts_with("        abcdefgh"));
    let after = core.document().text().to_owned();
    keys(&mut core, view, ".");
    assert!(core
        .document()
        .text()
        .starts_with("                abcdefgh"));
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), after);
}
#[test]
fn physical_utf16_dos_lines_preserve_encoding_bom_and_undo() {
    let source = "αβ\r\nγδ\r\n終";
    let mut bytes = vec![0xff, 0xfe];
    for unit in source.encode_utf16() {
        bytes.extend(unit.to_le_bytes());
    }
    let document = Document::from_bytes_with_file_format(
        bytes.clone(),
        Encoding::Utf16Le,
        Format::PlainText,
        evim_core::document::FileFormat::Dos,
    )
    .unwrap();
    let (mut core, view) = editor(document, 100.0);
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "jdd");
    assert_eq!(core.document().text(), "αβ\n終");
    assert_eq!(&core.document().source_bytes()[..2], &[0xff, 0xfe]);
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), bytes);
}
#[test]
fn joins_use_actual_boundaries_in_the_selected_line_domain() {
    let original = "abcdefgh ijklmnop\nqrstuv\nwxyz";
    let (mut core, view) = editor(Document::new(original), 85.0);
    keys(&mut core, view, "J");
    assert_eq!(
        core.document().text(),
        original,
        "soft wrap adds no separator to remove"
    );
    core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource))
        .unwrap();
    keys(&mut core, view, "J");
    assert_eq!(core.document().text(), "abcdefgh ijklmnop qrstuv\nwxyz");
    keys(&mut core, view, ".");
    assert_eq!(core.document().text(), "abcdefgh ijklmnop qrstuv wxyz");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "abcdefgh ijklmnop qrstuv\nwxyz");
    keys(&mut core, view, "ugJ");
    assert_eq!(core.document().text(), "abcdefgh ijklmnopqrstuv\nwxyz");
}

#[test]
fn location_in_resumed_first_hard_line_has_exact_global_and_fragment_ordinals() {
    use evim_core::command::CommandInterpreter;
    use evim_core::layout::{
        compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
        LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
        LayoutJobId, LayoutJobPriority, LayoutJobRegion, ViewLayout, ViewportLayoutRegion,
        MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
    };
    let document = Document::new("abcdef ".repeat(15_000));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(90.0, 200.0);
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 200.0).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let checkpoint = candidate.next_long_line_checkpoint().unwrap().clone();
    let first_fragment = checkpoint.completed_visual_rows();
    let source_offset = checkpoint.next_text_offset();
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(2),
        LayoutJobPriority::NewlyExposedRows,
        LayoutJobRegion::Viewport(
            ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 200.0).unwrap(),
        ),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 256);
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    install_layout_job(
        &mut view,
        LayoutInstallTarget {
            document_id: document.id(),
            document_revision: document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        },
        candidate,
    )
    .unwrap();
    let snapshot = view.snapshot().unwrap();
    assert_eq!(
        snapshot.rows.first().unwrap().fragment_index,
        first_fragment
    );
    assert!(first_fragment > 0);
    let mut commands = CommandInterpreter::new();
    assert!(commands.set_cursor(&document, source_offset));
    let location = commands.line_location(&document, Some(snapshot)).unwrap();
    assert_eq!(location.line, Some(first_fragment + 1));
    assert_eq!(location.hard_line, 1);
    assert_eq!(location.fragment, first_fragment + 1);
    assert_eq!(location.column, 1);
}

#[test]
fn document_edge_keys_ignore_line_policy_and_preserve_editing_modes() {
    use evim_core::command::Mode;
    let source = "αβ\nmiddle words\n👩‍💻z";
    for line_mode in [LineMode::Visual, LineMode::PhysicalSource] {
        for (entry, mode) in [
            (None, Mode::Normal),
            (Some(Key::Char('i')), Mode::Insert),
            (Some(Key::Char('R')), Mode::Replace),
            (Some(Key::Char('v')), Mode::VisualCharacter),
            (Some(Key::Char('V')), Mode::VisualLine),
            (Some(Key::Ctrl('v')), Mode::VisualBlock),
        ] {
            let (mut core, view) = editor(Document::new(source), 85.);
            core.handle(view, CoreEvent::SetLineMode(line_mode))
                .unwrap();
            keys(&mut core, view, "l");
            if let Some(entry) = entry {
                key(&mut core, view, entry);
            }
            let revision = core.document().revision();
            let history = core.document().history_status();
            key(&mut core, view, Key::DocumentEnd);
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            let expected = if matches!(mode, Mode::Insert | Mode::Replace) {
                source.len()
            } else {
                source.len() - 1
            };
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                expected,
                "{line_mode:?} {mode:?}"
            );
            core.document().text_point(expected).unwrap();
            key(&mut core, view, Key::DocumentStart);
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            let after = core.document().history_status();
            assert_eq!(
                (
                    after.current,
                    after.parent,
                    after.preferred_redo,
                    after.node_count,
                    after.redo_branch_count,
                    after.is_dirty
                ),
                (
                    history.current,
                    history.parent,
                    history.preferred_redo,
                    history.node_count,
                    history.redo_branch_count,
                    history.is_dirty
                )
            );
        }
    }
}

#[test]
fn document_edge_keys_use_prompt_boundaries_and_split_insert_undo_units() {
    let (mut core, view) = editor(Document::new("abcd\ntail"), 85.);
    keys(&mut core, view, "l:edit path");
    let document_cursor = core.command_state(view).unwrap().cursor();
    key(&mut core, view, Key::DocumentStart);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .active,
        0
    );
    key(&mut core, view, Key::DocumentEnd);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .active,
        "edit path".len()
    );
    assert_eq!(core.command_state(view).unwrap().cursor(), document_cursor);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    keys(&mut core, view, "iX");
    key(&mut core, view, Key::DocumentEnd);
    keys(&mut core, view, "Y");
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "aXbcd\ntailY");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "aXbcd\ntail");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "abcd\ntail");
}
