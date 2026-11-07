use super::*;
use crate::command::{InputEvent, Key};
use crate::layout::{DecorationKind, MockTextMeasurementProvider};

type TestCore = Core<MockTextMeasurementProvider>;

fn core(text: &str) -> (TestCore, ViewId) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}

fn key(core: &mut TestCore, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
}

fn type_text(core: &mut TestCore, view: ViewId, text: &str) {
    for character in text.chars() {
        key(core, view, Key::Char(character));
    }
}

/// Hard lines whose rows carry a block background.
fn background_lines(core: &TestCore, view: ViewId) -> Vec<usize> {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let mut lines: Vec<usize> = snapshot
        .rows
        .iter()
        .filter(|row| row.decorations.iter().any(|decoration| decoration.kind == DecorationKind::BlockBackground))
        .map(|row| row.hard_line_index)
        .collect();
    lines.dedup();
    lines
}

fn row_top(core: &TestCore, view: ViewId, line: usize) -> f32 {
    core.layout(view).unwrap().snapshot().unwrap().rows.iter().find(|row| row.hard_line_index == line).unwrap().y
}

fn has_text_background(core: &TestCore, view: ViewId, offset: usize) -> bool {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    snapshot.paint_runs.iter().any(|run| run.text_range.contains(&offset) && run.paint.background.is_some())
}

#[test]
fn sessions_mark_changed_lines_on_both_sides() {
    let (mut base, base_view) = core("a\nb\nc\n");
    let (mut new, new_view) = core("a\nB\nc\n");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Unchanged);
    assert_eq!(background_lines(&base, base_view), vec![1]);
    assert_eq!(background_lines(&new, new_view), vec![1]);
    // Source, history and dirty state are untouched by presentation.
    assert_eq!(new.document().source_bytes(), b"a\nB\nc\n");
    assert!(!new.document().is_dirty());
    DiffSession::clear(&mut new, new_view).unwrap();
    assert!(background_lines(&new, new_view).is_empty());
}

#[test]
fn typing_updates_the_diff_incrementally_and_marks_the_typed_text() {
    let (mut base, base_view) = core("alpha\nbeta\n");
    let (mut new, new_view) = core("alpha\nbeta\n");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(background_lines(&new, new_view).is_empty());
    // `A` appends at the end of the first line.
    key(&mut new, new_view, Key::Char('A'));
    type_text(&mut new, new_view, "x");
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Incremental);
    assert_eq!(background_lines(&new, new_view), vec![0]);
    assert_eq!(background_lines(&base, base_view), vec![0]);
    assert!(has_text_background(&new, new_view, 5), "the typed x is Added text");
    assert!(!has_text_background(&new, new_view, 0));
    key(&mut new, new_view, Key::Backspace);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(background_lines(&new, new_view).is_empty());
    assert!(background_lines(&base, base_view).is_empty());
}

#[test]
fn aligned_sides_keep_corresponding_lines_level() {
    let (mut base, base_view) = core("a\nx\ny\nz\nb\n");
    let (mut new, new_view) = core("a\nb\n");
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(row_top(&base, base_view, 4), row_top(&new, new_view, 1), "b lines up");
    // Turning alignment off removes the filler.
    session.set_aligned(false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(row_top(&new, new_view, 1) < row_top(&base, base_view, 4));
    assert_eq!(row_top(&new, new_view, 1), row_top(&base, base_view, 1));
}

#[test]
fn trailing_filler_follows_the_shorter_side() {
    let (mut base, base_view) = core("a");
    let (mut new, new_view) = core("a\nb\nc");
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let base_extent = core_height(&base, base_view);
    let new_extent = core_height(&new, new_view);
    assert_eq!(base_extent, new_extent);
}

fn core_height(core: &TestCore, view: ViewId) -> f32 {
    core.layout(view).unwrap().snapshot().unwrap().total_height
}

#[test]
fn undo_recomputes_the_diff() {
    let (mut base, base_view) = core("one\n");
    let (mut new, new_view) = core("one\n");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    type_text(&mut new, new_view, "Otwo");
    key(&mut new, new_view, Key::Escape);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(background_lines(&new, new_view), vec![0]);
    key(&mut new, new_view, Key::Char('u'));
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(background_lines(&new, new_view).is_empty());
    assert!(session.state().reproduces(&DocumentLines(base.document()), &DocumentLines(new.document())));
}

#[test]
fn corresponding_tops_follow_lines_through_the_diff() {
    let lines: Vec<String> = (0..200).map(|n| format!("line {n}")).collect();
    let mut base_lines = lines.clone();
    base_lines.insert(10, "extra".into());
    let (mut base, base_view) = core(&base_lines.join("\n"));
    let (mut new, new_view) = core(&lines.join("\n"));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    new.handle(new_view, CoreEvent::SetViewportOrigin { left: 0., top: Some(row_top(&new, new_view, 0) + 1_000.) }).unwrap();
    let top = new.viewport_state(new_view).unwrap().top();
    let new_line = new.layout(new_view).unwrap().hard_line_at_y(f64::from(top)).unwrap().unwrap().hard_line();
    let mirrored = session.corresponding_top(&new, new_view, true, &base, base_view).unwrap();
    let base_line = base.layout(base_view).unwrap().hard_line_at_y(f64::from(mirrored)).unwrap().unwrap().hard_line();
    assert!(new_line > 10);
    assert_eq!(base_line, new_line + 1);
}

fn lines(count: usize, edit: impl Fn(usize) -> Option<String>) -> String {
    (0..count).filter_map(|line| edit(line)).map(|line| line + "\n").collect()
}

fn cursor_line(core: &TestCore, view: ViewId) -> usize {
    core.document().hard_line_at_offset(core.views[&view].commands.cursor()).unwrap()
}

fn top(core: &TestCore, view: ViewId) -> f64 {
    f64::from(core.views[&view].layout.viewport_top())
}

fn prefix(core: &TestCore, view: ViewId, line: usize) -> f64 {
    core.layout(view).unwrap().hard_line_prefix_height(line).unwrap().height()
}

/// Base lines 0..100; the new side changes line 20, drops 50..52 and adds a
/// line after 80.
fn three_changes(aligned: bool) -> (TestCore, ViewId, TestCore, ViewId, DiffSession) {
    let (mut base, base_view) = core(&lines(100, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(100, |line| match line {
        20 => Some("changed".into()),
        50 | 51 => None,
        80 => Some("line 80\nadded".into()),
        _ => Some(format!("line {line}")),
    }));
    let mut session = DiffSession::new(base.document(), new.document(), aligned);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    (base, base_view, new, new_view, session)
}

#[test]
fn navigation_visits_each_change_in_order_and_wraps_within_the_file() {
    let (mut base, base_view, mut new, new_view, mut session) = three_changes(true);
    let mut go = |direction| {
        session.navigate_change(&mut base, base_view, &mut new, new_view, direction).unwrap().unwrap()
    };
    let first = go(ChangeDirection::Next);
    assert_eq!((first.change, first.change_count, first.caret_line), (0, 3, 20));
    // A pure removal puts the caret on the line after the removed lines.
    assert_eq!(go(ChangeDirection::Next).caret_line, 50);
    assert_eq!(go(ChangeDirection::Next).caret_line, 79);
    assert_eq!(go(ChangeDirection::Next).caret_line, 20, "next wraps to the first change");
    assert_eq!(go(ChangeDirection::Previous).caret_line, 79, "previous wraps to the last change");
    assert_eq!(cursor_line(&new, new_view), 79);
}

#[test]
fn navigation_centres_a_change_that_fits_and_lines_up_the_base() {
    let (mut base, base_view, mut new, new_view, mut session) = three_changes(false);
    // From the top, the last change (the line added after base line 80) is
    // off screen.
    let moved = session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Previous).unwrap().unwrap();
    assert_eq!(moved.caret_line, 79);
    assert!(moved.scrolled);
    let height = f64::from(new.views[&new_view].layout.height());
    let (start, end) = (prefix(&new, new_view, 79), prefix(&new, new_view, 80));
    assert!((top(&new, new_view) - ((start + end) / 2.0 - height / 2.0)).abs() < 0.5);
    // Unaligned sides are still placed as if aligned: the base's insertion
    // point (after base line 80) is level with the added line, although the
    // removed lines above it make the base two lines longer.
    let base_offset = prefix(&base, base_view, 81) - top(&base, base_view);
    assert!((base_offset - (start - top(&new, new_view))).abs() < 0.5);
    assert!(top(&base, base_view) > top(&new, new_view));
}

#[test]
fn navigation_to_a_visible_change_moves_only_the_caret() {
    let (mut base, base_view) = core("a\nb\nc\nd\n");
    let (mut new, new_view) = core("a\nb\nC\nd\n");
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let moved = session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap().unwrap();
    assert!(!moved.scrolled);
    assert_eq!((top(&new, new_view), top(&base, base_view)), (0.0, 0.0));
    assert_eq!(cursor_line(&new, new_view), 2);
}

#[test]
fn identical_sides_have_no_change_to_visit() {
    let (mut base, base_view) = core("a\nb\n");
    let (mut new, new_view) = core("a\nb\n");
    let mut session = DiffSession::new(base.document(), new.document(), true);
    assert_eq!(session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap(), None);
}

fn tall_change(view_rows: f32) -> (TestCore, ViewId, TestCore, ViewId, DiffSession, f64) {
    let (mut base, base_view) = core(&lines(100, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(100, |line| match line {
        40 => Some((0..60).map(|added| format!("added {added}")).collect::<Vec<_>>().join("\n")),
        _ => Some(format!("line {line}")),
    }));
    let row = new.diff_default_row(new_view).unwrap().0;
    new.handle(new_view, CoreEvent::Resize { width: 600., height: row * view_rows }).unwrap();
    base.handle(base_view, CoreEvent::Resize { width: 600., height: row * view_rows }).unwrap();
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    (base, base_view, new, new_view, session, f64::from(row))
}

#[test]
fn a_change_taller_than_the_view_starts_below_the_context_lines() {
    let (mut base, base_view, mut new, new_view, mut session, row) = tall_change(20.0);
    let moved = session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap().unwrap();
    assert_eq!(moved.caret_line, 40, "the first added line");
    let expected = prefix(&new, new_view, 40) - DIFF_CONTEXT_LINES as f64 * row;
    assert!((top(&new, new_view) - expected).abs() < 0.5);
}

#[test]
fn a_view_of_four_rows_shows_a_tall_change_from_its_top() {
    let (mut base, base_view, mut new, new_view, mut session, _) = tall_change(3.5);
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap();
    assert!((top(&new, new_view) - prefix(&new, new_view, 40)).abs() < 0.5);
}

#[test]
fn scrollbar_marks_show_own_changes_and_the_other_sides_filler() {
    let (mut base, base_view, mut new, new_view, session) = three_changes(true);
    let (marks, extent) = session.scrollbar_marks(&mut new, new_view, Side::New).unwrap();
    assert!((f64::from(extent) - prefix(&new, new_view, new.document().line_count())).abs() < 0.5);
    let added: Vec<_> = marks.iter().filter(|mark| mark.added).collect();
    let removed: Vec<_> = marks.iter().filter(|mark| !mark.added).collect();
    // Line 20 and the added line 80 (new line 79) are the new side's own.
    assert_eq!(added.len(), 2);
    assert!((f64::from(added[0].top) - prefix(&new, new_view, 20)).abs() < 0.5);
    // The removal shows at its filler, which has height when aligned.
    assert_eq!(removed.len(), 1);
    assert!(removed[0].bottom > removed[0].top);
    let (base_marks, _) = session.scrollbar_marks(&mut base, base_view, Side::Base).unwrap();
    assert_eq!(base_marks.iter().filter(|mark| !mark.added).count(), 2, "line 20 and the removed lines");
    assert_eq!(base_marks.iter().filter(|mark| mark.added).count(), 1, "filler for the added line");
}

#[test]
fn unaligned_scrollbar_marks_show_the_other_side_as_a_point() {
    let (_, _, mut new, new_view, session) = three_changes(false);
    let (marks, _) = session.scrollbar_marks(&mut new, new_view, Side::New).unwrap();
    let removed: Vec<_> = marks.iter().filter(|mark| !mark.added).collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].top, removed[0].bottom);
    assert!((f64::from(removed[0].top) - prefix(&new, new_view, 50)).abs() < 0.5);
}

/// Marks every `fn`, `let` and `use` as a Keyword.
struct Keywords;
impl crate::document::syntax::service::SyntaxProvider for Keywords {
    fn analyze(
        &mut self,
        request: &crate::document::syntax::service::SyntaxRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> crate::document::syntax::service::SyntaxResult {
        let mut result = crate::document::syntax::service::SyntaxResult::missing(request, "keywords");
        result.coverage = crate::document::syntax::Coverage::Exact;
        result.continuation = false;
        let text = request.input.slice(request.range.clone()).unwrap();
        for word in ["fn", "let", "use"] {
            for (at, _) in text.match_indices(word) {
                let start = request.range.start + at;
                result.runs.push(crate::document::syntax::SyntaxRun {
                    range: start..start + word.len(),
                    name: crate::document::syntax::SyntaxStyleName("Keyword".into()),
                    origin: "keywords".into(),
                    priority: 0,
                });
            }
        }
        result.runs.sort_by_key(|run| run.range.start);
        result
    }
}

fn highlighted(text: &str) -> (TestCore, ViewId) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    core.initialize_code_detection("fixture.rs", false).unwrap();
    core.set_syntax_provider_factory(Arc::new(|| Box::new(Keywords)));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}

/// Lay out again after a publication, as frontends do when the snapshot
/// they hold is no longer current.
fn relayout(core: &mut TestCore, view: ViewId) {
    core.handle(view, CoreEvent::Resize { width: 600., height: 400. }).unwrap();
}

fn settle_syntax(cores: &mut [&mut TestCore]) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut quiet = 0;
    while quiet < 20 {
        let mut changed = false;
        for core in cores.iter_mut() {
            changed |= core.poll_syntax();
        }
        quiet = if changed { 0 } else { quiet + 1 };
        assert!(std::time::Instant::now() < deadline, "syntax did not settle");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The foreground of every formatted byte, as laid out.
fn foregrounds(core: &TestCore, view: ViewId) -> Vec<Option<String>> {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let mut colors = vec![None; core.document().projection().text_tree().byte_len()];
    for run in &snapshot.paint_runs {
        if run.paint.foreground_is_default {
            continue;
        }
        for offset in run.text_range.clone() {
            if let Some(color) = colors.get_mut(offset) {
                *color = Some(format!("{:?}", run.paint.foreground));
            }
        }
    }
    colors
}

#[test]
fn diff_presentation_keeps_syntax_colors_after_highlighting_arrives() {
    let base_text = "use a;\nfn one() {}\nlet x = 1;\n";
    let new_text = "use a, b;\nfn one() {}\nfn two() { let y = 2; }\nlet x = 1;\n";
    let (mut plain, plain_view) = highlighted(new_text);
    let (mut new, new_view) = highlighted(new_text);
    let (mut base, base_view) = highlighted(base_text);
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    settle_syntax(&mut [&mut plain, &mut new, &mut base]);
    relayout(&mut plain, plain_view);
    relayout(&mut new, new_view);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let expected = foregrounds(&plain, plain_view);
    assert!(expected.iter().any(Option::is_some), "the provider colored keywords");
    assert_eq!(foregrounds(&new, new_view), expected);
}
