//! Views of one buffer that present it differently: a diff's live view is
//! always Code, beside ordinary Text views of the same Literal structure.
use super::*;
use crate::document::{code_style, syntax::{SyntaxRun, SyntaxStyleName}, Encoding, ViewPresentation};
use crate::layout::MockTextMeasurementProvider;

type Editor = Core<MockTextMeasurementProvider>;

fn text_buffer_with_diff_view() -> (Editor, ViewId, ViewId) {
    let document = Document::from_bytes(b"first line\nsecond line\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Core::new(document);
    let text = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    let diff = core.try_add_view_with_presentation(MockTextMeasurementProvider::new(), ViewLayout::new(300., 120.),
        LayoutExecutionContext::WorkerPool, ViewPresentation::diff()).unwrap();
    (core, text, diff)
}

#[test]
fn a_diff_view_of_a_text_buffer_shows_code_beside_a_text_view() {
    let (core, text, diff) = text_buffer_with_diff_view();
    assert_eq!(core.view_format(text).unwrap(), Format::PlainText);
    assert_eq!(core.view_format(diff).unwrap(), Format::Code);
    assert_eq!(core.document.buffer_format(), Format::PlainText);
    let monospace = |core: &Editor| {
        let sheet = core.document.projection().style_sheet();
        sheet.block_style(&sheet.base_paragraph).and_then(|style| style.character.font_families.clone())
            == Some(vec!["monospace".into()])
    };
    core.focus_view(diff);
    assert_eq!(core.document.format(), Format::Code);
    assert!(monospace(&core), "the diff view uses the Code sheet");
    core.focus_view(text);
    assert_eq!(core.document.format(), Format::PlainText);
    assert!(!monospace(&core), "the Text view keeps the Text sheet");
}

#[test]
fn text_and_code_views_share_one_literal_text_tree() {
    let (core, text, diff) = text_buffer_with_diff_view();
    core.focus_view(text);
    let shown_as_text = core.document.projection().text_tree().snapshot_identity();
    core.focus_view(diff);
    assert_eq!(core.document.projection().text_tree().snapshot_identity(), shown_as_text);
}

#[test]
fn adding_a_diff_view_changes_no_revision_history_or_dirty_state() {
    let document = Document::from_bytes(b"text\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Editor::new(document);
    let text = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    let revision = core.document.revision();
    core.try_add_view_with_presentation(MockTextMeasurementProvider::new(), ViewLayout::new(300., 120.),
        LayoutExecutionContext::WorkerPool, ViewPresentation::diff()).unwrap();
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status().node_count, 1);
    assert!(!core.document.is_dirty());
    assert_eq!(core.view_format(text).unwrap(), Format::PlainText);
}

#[test]
fn only_a_view_showing_code_starts_the_code_layer() {
    let document = Document::from_bytes(b"text\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Editor::new(document);
    let text = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    assert!(!core.code_layer_wanted());
    core.set_view_presentation(text, ViewPresentation::diff()).unwrap();
    assert!(core.code_layer_wanted());
    core.set_view_presentation(text, ViewPresentation::default()).unwrap();
    assert!(!core.code_layer_wanted());
}

#[test]
fn a_code_publication_does_not_invalidate_a_text_view_of_the_buffer() {
    let (mut core, text, diff) = text_buffer_with_diff_view();
    let text_generation = core.views[&text].layout.configuration_generation();
    let diff_generation = core.views[&diff].layout.configuration_generation();
    core.document.set_focus(ViewPresentation::diff());
    core.publish_code_presentation_runs(code_style::snapshot(), &[SyntaxRun {
        range: 0..5, name: SyntaxStyleName("Comment".into()), origin: "test".into(), priority: 0,
    }]);
    assert_eq!(core.views[&text].layout.configuration_generation(), text_generation);
    assert_ne!(core.views[&diff].layout.configuration_generation(), diff_generation);
    core.focus_view(text);
    assert!(core.document.projection().style_spans().is_empty());
    core.focus_view(diff);
    assert_eq!(core.document.projection().style_spans().len(), 1);
}

#[test]
fn choosing_code_restyles_without_reprojecting_or_recording_history() {
    let document = Document::from_bytes(b"text\nmore\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Editor::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    let tree = core.document.projection().text_tree().snapshot_identity();
    let (document, revision) = (core.document.id(), core.document.revision());
    core.set_document_mode(view, document, revision, DocumentMode::CodeFormat).unwrap();
    assert_eq!(core.view_format(view).unwrap(), Format::Code);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status().node_count, 1);
    assert_eq!(core.document.projection().text_tree().snapshot_identity(), tree);
}

#[test]
fn a_view_presentation_change_keeps_the_cursor_on_the_same_text() {
    let document = Document::from_bytes(b"first line\nsecond line\n".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Editor::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 120.);
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document.revision(), text_offset: 13,
        affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    core.set_view_presentation(view, ViewPresentation::diff()).unwrap();
    assert_eq!(core.views[&view].commands.cursor(), 13);
    assert!(core.layout(view).unwrap().snapshot().is_some());
}


#[test]
fn undo_after_a_preview_switch_restores_the_caret_in_the_shown_projection() {
    let source = "# Title\n\nfirst *word* here\n";
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Editor::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let at = core.document.text().find("here").unwrap();
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    for key in ["i", "X"] {
        core.handle(view, CoreEvent::Input(crate::command::InputEvent::text(key))).unwrap();
    }
    core.handle(view, CoreEvent::Input(crate::command::InputEvent::Key(crate::command::Key::Escape))).unwrap();
    let (document, revision) = (core.document.id(), core.document.revision());
    core.set_document_mode(view, document, revision, DocumentMode::Code("markdown".into())).unwrap();
    core.handle(view, CoreEvent::NavigateHistory(crate::document::HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document.source_bytes(), source.as_bytes());
    assert_eq!(core.view_format(view).unwrap(), Format::Code);
    let cursor = core.views[&view].commands.cursor();
    assert_eq!(cursor, core.document.text().find("here").unwrap());
}

mod concurrent {
    use super::*;
    use crate::command::{InputEvent, Key};
    use crate::document::{measure_document_work, ProjectionKind};

    const SOURCE: &str = "# Title\n\nfirst *word* here\n\nsecond paragraph target\n";

    /// A Markdown buffer with one Preview view and one view of its source.
    fn preview_and_code() -> (Editor, ViewId, ViewId) {
        let document = Document::from_bytes(SOURCE.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Editor::new(document);
        let preview = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let code = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.set_view_presentation(code, ViewPresentation::literal()).unwrap();
        (core, preview, code)
    }

    fn place(core: &mut Editor, view: ViewId, offset: usize) {
        let revision = core.view_document(view).revision();
        core.handle(view, CoreEvent::PlaceCursor { document_revision: revision, text_offset: offset,
            affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    }

    fn type_text(core: &mut Editor, view: ViewId, text: &str) {
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text(text))).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    }

    fn assert_fresh(core: &Editor, preview: ViewId, code: ViewId) {
        let source = core.document.source_bytes();
        let fresh_preview = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        let fresh_literal = Document::from_bytes(source, Encoding::Utf8, Format::PlainText).unwrap();
        assert_eq!(core.view_document(preview).text(), fresh_preview.text());
        assert!(core.view_document(preview).projection().semantic_difference(fresh_preview.projection()).is_none());
        assert_eq!(core.view_document(code).text(), fresh_literal.text());
    }

    #[test]
    fn a_view_shows_preview_beside_a_view_of_its_source() {
        let (core, preview, code) = preview_and_code();
        assert_eq!(core.view_format(preview).unwrap(), Format::Markdown);
        assert_eq!(core.view_format(code).unwrap(), Format::Code);
        assert!(core.document.has_lineage(ProjectionKind::Preview));
        assert!(core.document.has_lineage(ProjectionKind::Literal));
        assert_eq!(core.document.history_status().node_count, 1);
        assert!(!core.document.is_dirty());
    }

    #[test]
    fn typing_in_the_source_view_advances_the_preview_view() {
        let (mut core, preview, code) = preview_and_code();
        let at = core.view_document(code).text().find("here").unwrap();
        place(&mut core, code, at);
        type_text(&mut core, code, "new ");
        assert!(core.view_document(preview).text().contains("word new here"));
        assert_fresh(&core, preview, code);
    }

    #[test]
    fn typing_in_preview_keeps_the_source_view_caret_on_its_text() {
        let (mut core, preview, code) = preview_and_code();
        let target = core.view_document(code).text().find("target").unwrap();
        place(&mut core, code, target);
        let at = core.view_document(preview).text().find("here").unwrap();
        place(&mut core, preview, at);
        type_text(&mut core, preview, "X");
        assert_fresh(&core, preview, code);
        let cursor = core.views[&code].commands.cursor();
        assert_eq!(cursor, core.view_document(code).text().find("target").unwrap());
    }

    #[test]
    fn undo_from_either_view_keeps_both_projections_current() {
        let (mut core, preview, code) = preview_and_code();
        let at = core.view_document(preview).text().find("here").unwrap();
        place(&mut core, preview, at);
        type_text(&mut core, preview, "X");
        core.handle(code, CoreEvent::NavigateHistory(crate::document::HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document.source_bytes(), SOURCE.as_bytes());
        assert_fresh(&core, preview, code);
        core.handle(preview, CoreEvent::NavigateHistory(crate::document::HistoryNavigationRequest::Redo)).unwrap();
        assert!(core.view_document(code).text().contains("Xhere"));
        assert_fresh(&core, preview, code);
    }

    #[test]
    fn a_mark_set_in_one_projection_is_used_in_the_other() {
        let (mut core, preview, code) = preview_and_code();
        let target = core.view_document(code).text().find("target").unwrap();
        place(&mut core, code, target);
        core.handle(code, CoreEvent::Input(InputEvent::key('m'))).unwrap();
        core.handle(code, CoreEvent::Input(InputEvent::key('a'))).unwrap();
        place(&mut core, preview, 0);
        core.handle(preview, CoreEvent::Input(InputEvent::key('`'))).unwrap();
        core.handle(preview, CoreEvent::Input(InputEvent::key('a'))).unwrap();
        assert_eq!(core.views[&preview].commands.cursor(), core.view_document(preview).text().find("target").unwrap());
    }

    #[test]
    fn a_change_list_entry_is_recorded_in_each_views_projection() {
        let (mut core, preview, code) = preview_and_code();
        let at = core.view_document(code).text().find("target").unwrap();
        place(&mut core, code, at);
        // Several keystrokes: each validates every view's change list.
        type_text(&mut core, code, "new ");
        place(&mut core, preview, 0);
        core.handle(preview, CoreEvent::Input(InputEvent::key('g'))).unwrap();
        core.handle(preview, CoreEvent::Input(InputEvent::key(';'))).unwrap();
        let cursor = core.views[&preview].commands.cursor();
        let changed = core.view_document(preview).text().find("new target").unwrap();
        assert!((changed..changed + 4).contains(&cursor), "{cursor} near {changed}");
    }

    #[test]
    fn a_mark_survives_the_release_of_the_projection_it_was_set_in() {
        let (mut core, preview, code) = preview_and_code();
        let target = core.view_document(preview).text().find("target").unwrap();
        place(&mut core, preview, target);
        core.handle(preview, CoreEvent::Input(InputEvent::key('m'))).unwrap();
        core.handle(preview, CoreEvent::Input(InputEvent::key('a'))).unwrap();
        core.set_view_presentation(preview, ViewPresentation::literal()).unwrap();
        assert!(!core.document.has_lineage(ProjectionKind::Preview));
        for view in [preview, code] {
            place(&mut core, view, 0);
            core.handle(view, CoreEvent::Input(InputEvent::key('`'))).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::key('a'))).unwrap();
            assert_eq!(core.views[&view].commands.cursor(), core.view_document(view).text().find("target").unwrap());
        }
    }

    #[test]
    fn one_projection_performs_no_follower_work() {
        let document = Document::from_bytes(SOURCE.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Editor::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        place(&mut core, view, 2);
        let (_, work) = measure_document_work(|| type_text(&mut core, view, "abc"));
        assert_eq!(work.follower_reprojections, 0);
        assert!(!core.document.has_lineage(ProjectionKind::Literal));
    }

    /// Typing in either view of a large Markdown buffer with Preview and
    /// Code views open follows the other projection regionally.
    #[test]
    fn a_follower_catches_up_without_complete_reprojection() {
        let source = (0..2000).map(|n| format!("Paragraph {n:04} with *some* words.\n\n")).collect::<String>();
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Editor::new(document);
        let preview = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let code = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.set_view_presentation(code, ViewPresentation::literal()).unwrap();
        for view in [preview, code] {
            let at = core.view_document(view).text().find("Paragraph 1000").unwrap();
            place(&mut core, view, at);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            let (_, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::text("x"))).unwrap());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            assert_eq!(work.follower_reprojections, 1, "{view:?}");
            assert_eq!(work.full_projection_candidates, 0, "{view:?}: {work:?}");
        }
    }

    #[test]
    fn the_last_view_leaving_a_projection_releases_its_lineage() {
        let (mut core, preview, _code) = preview_and_code();
        core.set_view_presentation(preview, ViewPresentation::literal()).unwrap();
        assert!(!core.document.has_lineage(ProjectionKind::Preview));
        assert_eq!(core.document.cached_projection_count(ProjectionKind::Preview), 0);
    }

    #[test]
    fn changing_one_views_presentation_changes_no_other_lineage_history_or_dirty_state() {
        let (mut core, preview, code) = preview_and_code();
        let preview_revision = core.view_document(preview).revision();
        let nodes = core.document.history_status().node_count;
        let other = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
        core.set_view_presentation(other, ViewPresentation::literal()).unwrap();
        assert_eq!(core.view_document(preview).revision(), preview_revision);
        assert_eq!(core.document.history_status().node_count, nodes);
        assert!(!core.document.is_dirty());
        assert_eq!(core.view_format(code).unwrap(), Format::Code);
    }

    #[test]
    fn a_diff_view_stays_literal_while_another_view_shows_preview() {
        let (mut core, preview, _code) = preview_and_code();
        let diff = core.try_add_view_with_presentation(MockTextMeasurementProvider::new(), ViewLayout::new(300., 120.),
            LayoutExecutionContext::WorkerPool, ViewPresentation::diff()).unwrap();
        let at = core.view_document(preview).text().find("here").unwrap();
        place(&mut core, preview, at);
        type_text(&mut core, preview, "\n\nnew paragraph");
        let lines = core.document.source_bytes().iter().filter(|byte| **byte == b'\n').count();
        assert_eq!(core.view_format(diff).unwrap(), Format::Code);
        assert_eq!(core.view_document(diff).projection().hard_line_count(), lines + 1);
    }
}



#[test]
fn a_diff_view_follows_typing_while_another_view_shows_preview() {
    use crate::command::{InputEvent, Key};
    let live = Document::from_bytes(b"# Title\n\ntext\n".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut live = Editor::new(live);
    let preview = live.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let diff_view = live.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    live.set_view_presentation(diff_view, ViewPresentation::diff()).unwrap();
    let base = Document::from_bytes(b"# Title\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut base = Editor::new(base);
    let base_view = base.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let literal = crate::document::ProjectionKind::Literal;
    let mut session = live.document.with_lineage_focus(literal, |document| DiffSession::new(&base.document, document, true));
    session.sync(&mut base, base_view, &mut live, diff_view).unwrap();
    assert_eq!(session.line_totals(), (2, 0));
    // Preview off and on again in the other view.
    for mode in [DocumentMode::Code("markdown".into()), DocumentMode::Markdown] {
        let (document, revision) = (live.document.id(), live.view_document(preview).revision());
        live.set_document_mode(preview, document, revision, mode).unwrap();
        session.sync(&mut base, base_view, &mut live, diff_view).unwrap();
    }
    assert_eq!(live.view_format(diff_view).unwrap(), Format::Code);
    let revision = live.view_document(diff_view).revision();
    live.handle(diff_view, CoreEvent::PlaceCursor { document_revision: revision, text_offset: 2,
        affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    session.sync(&mut base, base_view, &mut live, diff_view).unwrap();
    live.handle(diff_view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    live.handle(diff_view, CoreEvent::Input(InputEvent::text("x"))).unwrap();
    live.handle(diff_view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    let sync = session.sync(&mut base, base_view, &mut live, diff_view);
    eprintln!("sync {sync:?} totals {:?}", session.line_totals());
    assert_ne!(session.line_totals(), (2, 0));
}


mod markdown_theme {
    use super::*;
    use crate::document::{FontSize, ProjectionKind};

    const THEMED: &[u8] = br#"{"version":1,"block_styles":[{"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","character":{"size":27},"block":{}}]}"#;

    fn base_size(core: &Editor, view: ViewId) -> Option<FontSize> {
        let document = core.view_document(view);
        let sheet = document.projection().style_sheet();
        sheet.block_style(&sheet.base_paragraph).and_then(|style| style.character.size.clone())
    }

    fn add(core: &mut Editor, presentation: ViewPresentation) -> ViewId {
        core.try_add_view_with_presentation(MockTextMeasurementProvider::new(), ViewLayout::new(300., 120.),
            LayoutExecutionContext::WorkerPool, presentation).unwrap()
    }

    #[test]
    fn a_theme_loaded_while_the_buffer_shows_its_source_styles_a_later_preview_view() {
        let mut document = Document::from_bytes(b"# Title\n\nBody\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        document.set_markdown_buffer(true);
        let mut core = Editor::new(document);
        core.focus_buffer();
        core.initialize_style_defaults(THEMED).unwrap();
        assert!(!core.document.has_lineage(ProjectionKind::Preview));
        let source = add(&mut core, ViewPresentation::literal());
        assert_eq!(core.view_format(source).unwrap(), Format::Code);
        let preview = add(&mut core, ViewPresentation::preview());
        assert_eq!(base_size(&core, preview), Some(FontSize::Points(27.)));
    }

    #[test]
    fn a_rebuilt_preview_lineage_keeps_the_theme_loaded_at_open() {
        let document = Document::from_bytes(b"# Title\n\nBody\n".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Editor::new(document);
        core.initialize_style_defaults(THEMED).unwrap();
        let view = add(&mut core, ViewPresentation::preview());
        assert_eq!(base_size(&core, view), Some(FontSize::Points(27.)));
        core.set_view_presentation(view, ViewPresentation::literal()).unwrap();
        assert!(!core.document.has_lineage(ProjectionKind::Preview), "no view shows Preview");
        core.set_view_presentation(view, ViewPresentation::preview()).unwrap();
        assert_eq!(base_size(&core, view), Some(FontSize::Points(27.)));
    }

    #[test]
    fn a_theme_replaced_while_a_source_view_is_focused_restyles_the_preview_view() {
        let document = Document::from_bytes(b"# Title\n\nBody\n".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Editor::new(document);
        let preview = add(&mut core, ViewPresentation::preview());
        let source = add(&mut core, ViewPresentation::literal());
        core.document.set_default_presentation(ViewPresentation::literal());
        core.focus_view(source);
        core.focus_buffer();
        core.replace_style_defaults(THEMED).unwrap();
        assert_eq!(base_size(&core, preview), Some(FontSize::Points(27.)));
        assert!(core.document.style_defaults_installed(THEMED));
    }
}
