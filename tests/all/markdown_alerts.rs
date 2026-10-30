//! GitHub alerts: top-level quotes whose first line is a hidden `[!TYPE]`
//! marker. Authoring changes only quote prefixes and the marker line.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{AlertKind, BoundaryAffinity, ContainerKind, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn document(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, viem_core::ViewId) {
    let mut core = Core::new(document(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
    (core, view)
}

/// The alert kind of each block's top-level quote, `None` for plain quotes,
/// and no entry character for unquoted blocks.
fn alerts(document: &Document) -> Vec<Option<Option<AlertKind>>> {
    document.projection().blocks().iter().map(|block| block.containers.first()
        .filter(|member| member.container.kind == ContainerKind::Quote)
        .map(|member| member.container.alert)).collect()
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, at: usize) {
    core.handle_with_layout(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
}

fn set_alert(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, kind: Option<AlertKind>) {
    let expected = core.list_selection_identity(view).unwrap();
    core.handle_with_layout(view, CoreEvent::SetAlert { expected, kind }).unwrap();
}

fn undo_all(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, source: &str) {
    while core.document().history_status().can_undo {
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn top_level_markers_are_hidden_case_insensitively_and_nested_markers_stay_literal() {
    let note = Some(Some(AlertKind::Note));
    for (source, text, expected) in [
        ("> [!NOTE]\n> body", "body", vec![note]),
        ("> [!note]  \n> lazy\ncontinued\n\nafter", "lazy continued\nafter", vec![note, None]),
        ("> [!Warning]\n> a\n>\n> b", "a\nb", vec![Some(Some(AlertKind::Warning)); 2]),
        ("> [!TIP]\n>\n> separated", "separated", vec![Some(Some(AlertKind::Tip))]),
        ("> [!CAUTION]\n>", "", vec![Some(Some(AlertKind::Caution))]),
        ("> [!IMPORTANT]", "", vec![Some(Some(AlertKind::Important))]),
        ("para\n\n> [!NOTE]\n\nafter", "para\n\nafter", vec![None, note, None]),
        // Trailing text, unknown kinds and nesting are ordinary quote text.
        ("> [!NOTE] title\n> body", "[!NOTE] title body", vec![Some(None)]),
        ("> [!INFO]\n> body", "[!INFO] body", vec![Some(None)]),
        ("> > [!NOTE]\n> > body", "[!NOTE] body", vec![Some(None)]),
        ("- > [!NOTE]\n  > body", "[!NOTE] body", vec![None]),
    ] {
        let document = document(source);
        assert_eq!(document.text(), text, "{source:?}");
        assert_eq!(alerts(&document), expected, "{source:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn alert_styles_derive_from_block_quote_and_fall_back_to_it() {
    let document = document("> [!TIP]\n> body");
    let sheet = document.projection().style_sheet();
    for kind in AlertKind::ALL {
        let style = sheet.block_style(&kind.style_id()).unwrap();
        assert_eq!(style.based_on.as_ref().unwrap().0, "Block quote", "{kind:?}");
    }
    let block = &document.projection().blocks()[0];
    assert_eq!(block.containers[0].container.style.0, "Tip");
}

#[test]
fn choosing_a_kind_quotes_prose_and_rewrites_or_removes_markers_with_exact_undo() {
    for (source, at, kind, after) in [
        ("first\n\nsecond", 0, Some(AlertKind::Note), "> [!NOTE]\n> first\n\nsecond"),
        ("> quoted\n> lines", 2, Some(AlertKind::Warning), "> [!WARNING]\n> quoted\n> lines"),
        ("> [!note]\n> body", 1, Some(AlertKind::Caution), "> [!CAUTION]\n> body"),
        ("> [!NOTE]\n> body", 1, Some(AlertKind::Note), "> [!NOTE]\n> body"),
        ("> [!NOTE]\n> body", 1, None, "> body"),
        ("> [!NOTE]\n>", 0, None, ">"),
        ("> [!NOTE]", 0, None, "> "),
        ("plain", 0, None, "> plain"),
    ] {
        let (mut core, view) = open(source);
        let text = core.document().text().to_owned();
        place(&mut core, view, at);
        set_alert(&mut core, view, kind);
        assert_eq!(core.document().source_bytes(), after.as_bytes(), "{source:?} {kind:?}");
        assert_eq!(core.document().text(), text, "{source:?}");
        let quote = core.document().projection().blocks()[0].containers[0].container.clone();
        assert_eq!((quote.kind, quote.alert), (ContainerKind::Quote, kind), "{source:?}");
        undo_all(&mut core, view, source);
    }
}

#[test]
fn new_markers_use_the_documents_line_endings() {
    let source = "first\r\nsecond\r\n";
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
    set_alert(&mut core, view, Some(AlertKind::Tip));
    assert_eq!(core.document().source_bytes(), b"> [!TIP]\r\n> first\r\n> second\r\n");
    undo_all(&mut core, view, source);
}

#[test]
fn alerts_cannot_be_created_inside_lists_or_nested_quotes() {
    for (source, at) in [("- item", 0), ("> > nested", 0), ("> - item", 0)] {
        let (mut core, view) = open(source);
        place(&mut core, view, at);
        assert!(!core.can_set_alert(view).unwrap(), "{source:?}");
        let expected = core.list_selection_identity(view).unwrap();
        assert!(core.handle_with_layout(view, CoreEvent::SetAlert { expected, kind: Some(AlertKind::Note) }).is_err());
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let (core, view) = open("prose");
    assert!(core.can_set_alert(view).unwrap());
}

#[test]
fn removing_quote_treatment_from_the_first_paragraph_removes_the_marker() {
    let unquote = |source: &str, at: usize| {
        let (mut core, view) = open(source);
        place(&mut core, view, at);
        let expected = core.list_selection_identity(view).unwrap();
        core.handle_with_layout(view, CoreEvent::SetBlockQuote { expected, enabled: false }).unwrap();
        assert_eq!(core.document().text(), "one\ntwo");
        let after = String::from_utf8(core.document().source_bytes().to_vec()).unwrap();
        undo_all(&mut core, view, source);
        after
    };
    let plain = "> one\n>\n> two";
    // The first paragraph takes the marker with it; a later one leaves the
    // alert's opening, so both match a plain quote's own result.
    assert_eq!(unquote(&format!("> [!NOTE]\n{plain}"), 0), unquote(plain, 0));
    assert_eq!(unquote(&format!("> [!NOTE]\n{plain}"), 4), format!("> [!NOTE]\n{}", unquote(plain, 4)));
}

#[test]
fn paragraph_styles_name_alert_kinds_and_block_quote_restores_a_plain_quote() {
    let (mut core, view) = open("body");
    let expected = core.list_selection_identity(view).unwrap();
    core.handle_with_layout(view, CoreEvent::SetParagraphStyle { expected, style: "Important".into() }).unwrap();
    assert_eq!(core.document().source_bytes(), b"> [!IMPORTANT]\n> body");
    let selected = core.selected_named_styles(view).unwrap();
    assert_eq!(selected.paragraph.unwrap().0, "Important");
    assert_eq!((selected.alerts, selected.has_plain_quote), (1 << (AlertKind::Important.ordinal() - 1), false));
    let expected = core.list_selection_identity(view).unwrap();
    core.handle_with_layout(view, CoreEvent::SetParagraphStyle { expected, style: "Block quote".into() }).unwrap();
    assert_eq!(core.document().source_bytes(), b"> body");
    let selected = core.selected_named_styles(view).unwrap();
    assert_eq!((selected.alerts, selected.has_plain_quote), (0, true));
    undo_all(&mut core, view, "body");
}

#[test]
fn typing_into_an_alert_body_keeps_the_marker_hidden() {
    for (source, after) in [
        ("> [!NOTE]\n> body", "> [!NOTE]\n> Xbody"),
        ("> [!NOTE]\n> ", "> [!NOTE]\n> X"),
        ("> [!NOTE]\n> one\n>\n> two", "> [!NOTE]\n> Xone\n>\n> two"),
        ("> [!NOTE]", "> [!NOTE]\n> X"),
    ] {
        let (mut core, view) = open(source);
        place(&mut core, view, 0);
        for key in [Key::Char('i'), Key::Char('X'), Key::Escape] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        assert_eq!(String::from_utf8_lossy(&core.document().source_bytes()), after, "{source:?}");
        assert!(alerts(core.document()).iter().all(|alert| *alert == Some(Some(AlertKind::Note))), "{source:?}");
        undo_all(&mut core, view, source);
    }
}

#[test]
fn alerts_reserve_a_title_band_with_an_icon_and_shaped_title() {
    use viem_core::layout::DecorationKind;
    let rows = |source: &str| {
        let (core, view) = open(source);
        core.layout(view).unwrap().snapshot().unwrap().rows.clone()
    };
    let plain = rows("> body");
    let alert = rows("> [!WARNING]\n> body");
    let band = alert[0].y - plain[0].y;
    assert!(band > 0.0, "the title band precedes the body: {band}");
    let decorations = &alert[0].decorations;
    let icon = decorations.iter().find(|d| d.kind == DecorationKind::AlertIcon { kind: AlertKind::Warning }).unwrap();
    let title = decorations.iter().find(|d| d.kind == DecorationKind::AlertTitle { kind: AlertKind::Warning }).unwrap();
    let border = decorations.iter().find(|d| d.kind == DecorationKind::BlockQuoteBorder).unwrap();
    let shaped: String = decorations.iter().filter(|d| d.kind == title.kind).map(|d| d.text.as_str()).collect();
    assert_eq!(shaped, "Warning");
    assert!(icon.typographic_bounds.y < title.baseline && title.baseline < alert[0].y, "the title sits on its own baseline in the band");
    assert_eq!(icon.typographic_bounds.width, icon.typographic_bounds.height);
    assert!(icon.typographic_bounds.y + icon.typographic_bounds.height <= alert[0].y, "{icon:?}");
    assert!(title.x > icon.typographic_bounds.x + icon.typographic_bounds.width);
    assert!(border.typographic_bounds.y <= icon.typographic_bounds.y, "the quote border spans the band");
    assert_eq!(icon.paint, title.paint);
    assert!(!plain[0].decorations.iter().any(|d| matches!(d.kind,
        DecorationKind::AlertIcon { .. } | DecorationKind::AlertTitle { .. })));
    // Rows placed below other content move their title baseline with them.
    let below = rows(&format!("{}> [!NOTE]\n> body", "Paragraph.\n\n".repeat(4)));
    let row = below.iter().find(|row| row.decorations.iter().any(|d| matches!(d.kind, DecorationKind::AlertTitle { .. }))).unwrap();
    for title in row.decorations.iter().filter(|d| matches!(d.kind, DecorationKind::AlertTitle { .. })) {
        let bounds = title.typographic_bounds;
        assert!(bounds.y <= title.baseline && title.baseline <= bounds.y + bounds.height && title.baseline < row.y, "{title:?}");
    }
    // A two-paragraph alert draws its heading once, above the first.
    let two = rows("> [!NOTE]\n> one\n>\n> two");
    assert_eq!(two.iter().flat_map(|row| &row.decorations)
        .filter(|d| matches!(d.kind, DecorationKind::AlertTitle { .. })).map(|d| d.text.as_str()).collect::<String>(), "Note");
}

#[test]
fn clipboard_fragments_keep_the_alert_marker_and_leave_the_donor_intact() {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent,
        ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::{CommandInterpreter, CommandStatus, RegisterValue};
    use viem_core::document::ClipboardFragment;
    // Paste joins a selection's leading paragraph to the target, so the
    // alert here is copied as complete blocks after one.
    let source = "before\n\n> [!TIP]\n> one\n>\n> two\n\nafter";
    let donor = document(source);
    let fragment = donor.clipboard_fragment(0..donor.text().find("\nafter").unwrap()).unwrap();
    let value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), value["plain_text"].as_str().unwrap()).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1),
        ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap()),
    ));
    let mut target = document("");
    let mut commands = CommandInterpreter::new();
    super::markdown_test_support::place_insert_boundary(&mut commands, &mut target, 0);
    let result = commands.handle_with_clipboard_context(&mut target,
        InputEvent::Key(Key::PasteClipboard), &context).unwrap();
    assert_eq!(result.status, CommandStatus::Complete);
    assert_eq!(target.text(), "before\none\ntwo");
    let tip = Some(Some(AlertKind::Tip));
    assert_eq!(alerts(&target), vec![None, tip, tip], "{:?}", String::from_utf8_lossy(&target.source_bytes()));
    let reopened = Document::from_bytes(target.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(alerts(&reopened), vec![None, tip, tip]);
    assert_eq!(donor.source_bytes(), source.as_bytes());
}

#[test]
fn html_export_titles_alerts_without_changing_source() {
    let source = "> [!CAUTION]\n> Careful\n\n> plain";
    let document = document(source);
    let html = String::from_utf8(document.prepare_html_export().render().unwrap()).unwrap();
    assert_eq!(html.matches("class=\"markdown-alert markdown-alert-caution\"").count(), 1);
    assert_eq!(html.matches("<p class=\"markdown-alert-title\"").count(), 1);
    assert!(html.contains(">Caution</p>") && html.contains("Careful") && html.contains("plain"));
    assert!(!html.contains("[!CAUTION]"));
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn preview_edits_inside_alerts_match_plain_quotes_without_stranding_the_marker() {
    let run = |source: &str, keys: &[Key]| {
        let (mut core, view) = open(source);
        place(&mut core, view, 0);
        for key in keys {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(*key))).unwrap();
        }
        let after = (String::from_utf8(core.document().source_bytes().to_vec()).unwrap(),
            core.document().text().to_owned(), alerts(core.document()));
        undo_all(&mut core, view, source);
        after
    };
    let note = Some(Some(AlertKind::Note));
    // Enter and `o` continue the alert with a new paragraph.
    for keys in [&[Key::Char('A'), Key::Enter, Key::Char('n'), Key::Escape][..], &[Key::Char('o'), Key::Char('n'), Key::Escape]] {
        assert_eq!(run("> [!NOTE]\n> body\n\nafter", keys),
            ("> [!NOTE]\n> body\n>\n> n\n\nafter".into(), "body\nn\nafter".into(), vec![note, note, None]));
    }
    // Deleting the alert's only paragraph takes its hidden marker too.
    assert_eq!(run("> [!NOTE]\n> body\n\nafter", &[Key::Char('d'), Key::Char('d')]),
        ("after".into(), "after".into(), vec![None]));
    // Deleting every character leaves an empty, still editable alert body.
    assert_eq!(run("> [!NOTE]\n> body\n\nafter", &[Key::Char('x'); 4]),
        ("> [!NOTE]\n> \n\nafter".into(), "\nafter".into(), vec![note, None]));
}

#[test]
fn alert_markers_round_trip_through_utf16_and_bom_with_exact_history() {
    let source = "\u{feff}Words\r\nMore\r\n";
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(source.trim_start_matches('\u{feff}').encode_utf16().flat_map(u16::to_le_bytes));
    let document = Document::from_bytes(bytes.clone(), Encoding::Utf16Le, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
    set_alert(&mut core, view, Some(AlertKind::Caution));
    let expected: Vec<u8> = [0xff, 0xfe].into_iter()
        .chain("> [!CAUTION]\r\n> Words\r\n> More\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(core.document().source_bytes(), expected);
    assert_eq!(alerts(core.document()), vec![Some(Some(AlertKind::Caution))]);
    while core.document().history_status().can_undo {
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), bytes);
}

#[test]
fn preview_switching_keeps_alert_source_and_shows_the_marker_only_in_code() {
    use viem_core::document::ModelRequest;
    let source = "> [!NOTE]\n> body";
    let mut document = document(source);
    for (target, text, expected) in [(Format::Code, source, vec![None, None]), (Format::Markdown, "body", vec![Some(Some(AlertKind::Note))])] {
        let prepared = document.prepare_model_request(ModelRequest::SetViewFormat {
            document: document.id(), revision: document.revision(), target }).unwrap();
        assert!(prepared.summary().source_patches().is_empty());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), text);
        assert_eq!(alerts(&document), expected);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn alert_edits_beside_a_large_suffix_reproject_locally() {
    use viem_core::document::measure_document_work;
    for count in [64, 640] {
        let source = format!("> [!NOTE]\n> one\n>\n> two\n\nGuard\n\n{}", "Unrelated paragraph.\n\n".repeat(count));
        let (mut core, view) = open(&source);
        let at = core.document().text().find("two").unwrap();
        place(&mut core, view, at);
        let (_, typing) = measure_document_work(|| {
            for key in [Key::Char('i'), Key::Char('X'), Key::Escape] {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            }
        });
        let (_, kind) = measure_document_work(|| set_alert(&mut core, view, Some(AlertKind::Tip)));
        for work in [typing, kind] {
            assert_eq!(work.full_projection_candidates, 0, "count={count}: {work:?}");
            assert_eq!(work.source_full_materializations, 0, "count={count}: {work:?}");
            assert!(work.markdown_block_parsed_bytes < 8192, "count={count}: {work:?}");
        }
        let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(alerts(core.document()), alerts(&fresh));
        assert_eq!(alerts(&fresh)[..2], [Some(Some(AlertKind::Tip)); 2]);
    }
}

#[test]
fn empty_alert_bodies_project_once_whatever_follows_the_marker() {
    let note = Some(Some(AlertKind::Note));
    for (source, text, expected) in [
        ("> [!NOTE]\n> \n", "", vec![note]),
        ("> [!NOTE]\n> \n\nafter", "\nafter", vec![note, None]),
        ("before\n\n> [!NOTE]\n> \n", "before\n", vec![None, note]),
        ("> [!NOTE]\n\n> [!TIP]\n> x", "\nx", vec![note, Some(Some(AlertKind::Tip))]),
    ] {
        let document = document(source);
        assert_eq!(document.text(), text, "{source:?}");
        assert_eq!(alerts(&document), expected, "{source:?}");
    }
}

#[test]
fn choosing_a_kind_on_a_blank_paragraph_creates_an_editable_empty_alert() {
    for (source, at, after) in [
        ("", 0, "> [!NOTE]\n> "),
        ("before\n\n\n\nafter", 7, "before\n\n> [!NOTE]\n> \n\nafter"),
        ("before\n\n\n", 7, "before\n\n> [!NOTE]\n> \n"),
        ("\n\nafter", 0, "> [!NOTE]\n> \n\nafter"),
    ] {
        let (mut core, view) = open(source);
        let text = core.document().text().to_owned();
        place(&mut core, view, at);
        set_alert(&mut core, view, Some(AlertKind::Note));
        assert_eq!(String::from_utf8_lossy(&core.document().source_bytes()), after, "{source:?}");
        assert_eq!(core.document().text(), text, "{source:?}");
        for key in [Key::Char('i'), Key::Char('X'), Key::Escape] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        let typed = core.document().projection().blocks().iter()
            .find(|block| core.document().text()[block.range.clone()] == *"X").unwrap();
        assert_eq!(typed.containers[0].container.alert, Some(AlertKind::Note), "{source:?}");
        undo_all(&mut core, view, source);
    }
}

#[test]
fn alert_titles_draw_as_a_heading_4_inside_the_alert() {
    use viem_core::layout::DecorationKind;
    let title = |defaults: serde_json::Value| {
        let mut document = document("> [!WARNING]\n> body");
        assert!(document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap().is_empty());
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
        let rows = core.layout(view).unwrap().snapshot().unwrap().rows.clone();
        rows[0].decorations.iter().find(|d| d.kind == DecorationKind::AlertTitle { kind: AlertKind::Warning }).unwrap().clone()
    };
    let red = serde_json::json!({"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0});
    let blue = serde_json::json!({"red": 0.0, "green": 0.0, "blue": 1.0, "alpha": 1.0});
    // Heading 4's own declarations win.
    let heading = title(serde_json::json!({"version": 1, "block_styles": [
        {"id": "Heading4", "name": "Heading 4", "role": "Paragraph", "based_on": "Paragraph",
            "character": {"size": 30, "foreground": red}, "block": {}},
        {"id": "Warning", "name": "Warning", "role": "Quote", "based_on": "Block quote",
            "character": {"foreground": blue}, "block": {}}]}));
    assert_eq!(heading.font_size, 30.0);
    assert_eq!((heading.paint.foreground.red, heading.paint.foreground.blue), (1.0, 0.0));
    assert!(heading.render_run.is_some(), "the native shaper supplies the title glyphs");
    // Otherwise the heading inherits the alert style's appearance.
    let inherited = title(serde_json::json!({"version": 1, "block_styles": [
        {"id": "Heading4", "name": "Heading 4", "role": "Paragraph", "based_on": "Paragraph",
            "character": {"size": 30}, "block": {}},
        {"id": "Warning", "name": "Warning", "role": "Quote", "based_on": "Block quote",
            "character": {"foreground": blue}, "block": {}}]}));
    assert_eq!((inherited.paint.foreground.red, inherited.paint.foreground.blue), (0.0, 1.0));
}
