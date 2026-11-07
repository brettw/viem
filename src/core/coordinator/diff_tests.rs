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
