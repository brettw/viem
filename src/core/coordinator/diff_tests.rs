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
fn line_totals_count_physical_lines_and_final_terminators() {
    for (old, live, expected) in [
        ("", "", (0, 0)), ("", "\n", (1, 0)), ("one\n", "", (0, 1)),
        ("a", "a\n", (1, 1)), ("a\n", "a", (1, 1)), ("a\n", "a\nb\n", (1, 0)),
        ("a", "a\nb", (2, 1)), ("x\ny\n", "z\n", (1, 2)),
    ] {
        let (base, _) = core(old);
        let (new, _) = core(live);
        assert_eq!(DiffSession::new(base.document(), new.document(), true).line_totals(), expected, "{old:?} -> {live:?}");
    }
}

#[test]
fn sessions_mark_changed_lines_on_both_sides() {
    let (mut base, base_view) = core("a\nb\nc\n");
    let (mut new, new_view) = core("a\nB\nc\n");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Unchanged);
    assert_eq!(background_lines(&base, base_view), vec![1]);
    assert_eq!(background_lines(&new, new_view), vec![1]);
    for (core, view, side) in [(&mut base, base_view, Side::Base), (&mut new, new_view, Side::New)] {
        core.set_number(view, true).unwrap();
        let bands = core.line_number_regions(view).unwrap();
        let changed: Vec<_> = bands.iter().filter_map(|band| band.diff_side.map(|side| (band.source_line, side))).collect();
        assert_eq!(changed, [(2, side)]);
    }
    // Source, history and dirty state are untouched by presentation.
    assert_eq!(new.document().source_bytes(), b"a\nB\nc\n");
    assert!(!new.document().is_dirty());
    DiffSession::clear(&mut new, new_view).unwrap();
    assert!(background_lines(&new, new_view).is_empty());
    assert!(new.line_number_regions(new_view).unwrap().iter().all(|band| band.diff_side.is_none()));
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
fn typing_into_changed_lines_keeps_the_first_live_layout_with_verified_diff_styles() {
    let live = "head\nchanged value\nkeep\nlater change\ntail";
    for earlier in ["head\nkeep\nlater\ntail", "head\nchanged values\nkeep\nlater\ntail", "head\n!!!\nkeep\nlater\ntail"] {
        let (mut base, base_view) = core(earlier);
        let (mut new, new_view) = core(live);
        place(&mut new, new_view, 1);
        let mut session = DiffSession::new(base.document(), new.document(), true);
        session.sync(&mut base, base_view, &mut new, new_view).unwrap();
        type_text(&mut new, new_view, "Ax");
        let first = new.layout(new_view).unwrap().snapshot().unwrap().revision;
        assert_eq!(background_lines(&new, new_view), vec![1, 3]);
        assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Incremental);
        assert_eq!(new.layout(new_view).unwrap().snapshot().unwrap().revision, first);
        let fresh = DiffState::compute(&DocumentLines(base.document()), &DocumentLines(new.document()), DiffLimits::default());
        assert_eq!(session.state(), &fresh);
        for (line, span) in fresh.presentation(Side::New, true).text {
            let start = new.document().projection().hard_line_range(line).unwrap().start;
            assert!(has_text_background(&new, new_view, start + span.start));
        }
    }
}

#[test]
fn a_same_count_replacement_across_lines_does_not_reuse_the_typing_line() {
    let original = "first\nmiddle\nlast";
    let (mut base, base_view) = core(original);
    let (mut new, new_view) = core("first changed\nmiddle\nlast changed");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(new.views[&new_view].diff_typing_line.is_some());
    type_text(&mut new, new_view, ":%s/ changed//");
    key(&mut new, new_view, Key::Enter);
    assert_eq!(new.document().source_bytes(), original.as_bytes());
    assert!(new.layout(new_view).unwrap().presentation_overlays(new.document().id(), new.document().revision()).diff.is_none());
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(session.state().hunk_count(), 0);
}

#[test]
fn changing_the_base_replaces_the_typing_line_proof_and_leaving_diff_retires_it() {
    let (mut previous, previous_view) = core("short");
    let (mut new, new_view) = core("shorter");
    let mut old = DiffSession::new(previous.document(), new.document(), false);
    old.sync(&mut previous, previous_view, &mut new, new_view).unwrap();
    let (mut base, base_view) = core("shorter!");
    let mut session = DiffSession::new(base.document(), new.document(), false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    type_text(&mut new, new_view, "A!");
    assert!(background_lines(&new, new_view).is_empty());
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(session.state().hunk_count(), 0);
    key(&mut new, new_view, Key::Backspace);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(new.views[&new_view].diff_typing_line.is_some());
    DiffSession::clear(&mut new, new_view).unwrap();
    assert!(new.views[&new_view].diff_typing_line.is_none());
}

#[test]
fn typing_on_a_new_row_between_replacement_pairs_reuses_layout_and_undo_restores_the_hunk() {
    let (mut base, base_view) = core("value 1\nvalue 2\nvalue 3");
    let initial = "value 1!\nvalue 2!\nvalue 3!";
    let (mut new, new_view) = core(initial);
    place(&mut new, new_view, 1);
    let mut session = DiffSession::new(base.document(), new.document(), false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let before = session.state().clone();
    key(&mut new, new_view, Key::Char('A'));
    key(&mut new, new_view, Key::Enter);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    key(&mut new, new_view, Key::Char('x'));
    let first_frame = new.layout(new_view).unwrap().snapshot().unwrap().revision;
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().snapshot().unwrap().revision, first_frame);
    assert_eq!(new.document().source_bytes(), b"value 1!\nvalue 2!\nx\nvalue 3!");
    key(&mut new, new_view, Key::Escape);
    key(&mut new, new_view, Key::Char('u'));
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.document().source_bytes(), initial.as_bytes());
    assert_eq!(session.state(), &before);
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
fn reloading_the_live_document_preserves_unaffected_base_layout() {
    let original = lines(100, |line| Some(format!("line {line}")));
    let (mut base, base_view) = core(&original);
    let (mut live, live_view) = core(&original);
    let mut session = DiffSession::new(base.document(), live.document(), false);
    session.sync(&mut base, base_view, &mut live, live_view).unwrap();
    base.materialize_requested_viewport(base_view, 0., f32::MAX).unwrap();
    base.materialize_requested_viewport(base_view, 0., 0.).unwrap();
    let distant_line_cached = |base: &TestCore| base.layout(base_view).unwrap()
        .regional_cached_ranges().iter().any(|range| range.contains(&99));
    assert!(distant_line_cached(&base));

    let (mut reloaded, reloaded_view) = core(&original.replacen("line 1\n", "new\n", 1));
    assert_eq!(session.sync(&mut base, base_view, &mut reloaded, reloaded_view).unwrap(), DiffSync::Full);
    assert_eq!(background_lines(&base, base_view), vec![1]);
    assert_eq!(background_lines(&reloaded, reloaded_view), vec![1]);
    assert!(distant_line_cached(&base));
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
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    let mirrored = base.viewport_state(base_view).unwrap().top();
    assert_eq!(mirrored, top);
    let base_line = base.layout(base_view).unwrap().hard_line_at_y(f64::from(mirrored)).unwrap().unwrap().hard_line();
    assert!(new_line > 10);
    assert_eq!(base_line, new_line + 1);
}

fn lines(count: usize, edit: impl Fn(usize) -> Option<String>) -> String {
    (0..count).filter_map(|line| edit(line)).map(|line| line + "\n").collect()
}

#[test]
fn aligned_scrollbars_share_the_wrapped_tail_from_either_side() {
    let long = "common\n".to_owned() + &"several words on a wrapped line ".repeat(30) + &"\nextra".repeat(85);
    for reverse in [false, true] {
        let (older, newer) = if reverse { (long.as_str(), "common") } else { ("common", long.as_str()) };
        let (mut base, bv) = core(older);
        let (mut new, nv) = core(newer);
        for (core, view) in [(&mut base, bv), (&mut new, nv)] {
            core.handle(view, CoreEvent::Resize { width: 120., height: 80. }).unwrap();
            core.handle(view, CoreEvent::SetWrap(true)).unwrap();
        }
        let mut session = DiffSession::new(base.document(), new.document(), true);
        session.sync(&mut base, bv, &mut new, nv).unwrap();
        for (from_new, top) in [(reverse, 300.), (!reverse, 770.), (reverse, 605.), (reverse, 0.), (!reverse, 470.), (reverse, f32::MAX)] {
            let (leader, id) = if from_new { (&mut new, nv) } else { (&mut base, bv) };
            leader.handle(id, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
            session.sync_scroll(&mut base, bv, &mut new, nv, if from_new { Side::New } else { Side::Base }).unwrap();
            let a = base.viewport_state(bv).unwrap();
            let b = new.viewport_state(nv).unwrap();
            assert_eq!(a.estimated_maximum_top(), b.estimated_maximum_top());
            assert_eq!(a.top(), b.top());
            assert_eq!(a.top(), top.min(a.estimated_maximum_top()));
            assert!(a.estimated_maximum_top() > 770.);
        }
        // Filler on the shorter side stands for every row of the wrapped
        // line, so its own range reaches the shared end.
        let (short, view) = if reverse { (&mut new, nv) } else { (&mut base, bv) };
        let own_end = short.layout(view).unwrap().own_vertical_bound().maximum;
        let shared_end = short.viewport_state(view).unwrap().estimated_maximum_top();
        assert!((shared_end - own_end).abs() < 0.5, "{own_end} {shared_end}");
        let blank = shared_end / 2.;
        short.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(blank) }).unwrap();
        session.sync_scroll(&mut base, bv, &mut new, nv, if reverse { Side::New } else { Side::Base }).unwrap();
        let (short, view) = if reverse { (&mut new, nv) } else { (&mut base, bv) };
        short.handle(view, CoreEvent::Resize { width: 110., height: 79. }).unwrap();
        assert_eq!(short.viewport_state(view).unwrap().top(), blank);
        session.set_aligned(false);
        session.sync(&mut base, bv, &mut new, nv).unwrap();
        let short = if reverse { new.viewport_state(nv) } else { base.viewport_state(bv) }.unwrap();
        assert_eq!(short.estimated_maximum_top(), 0.);
        assert_eq!(short.top(), 0.);
    }
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
    let row = new.default_row_height(new_view).unwrap();
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

// Collapsed context.

fn collapse_lines(line: usize) -> Option<String> {
    Some(if line == 10 || line == 30 { format!("changed {line}") } else { format!("line {line}") })
}

/// Base lines 0..40; the new side changes lines 10 and 30, so collapsed
/// context hides lines 0..7, 14..27 and 34..41 (with the empty last line)
/// on both sides.
fn collapsing(aligned: bool) -> (TestCore, ViewId, TestCore, ViewId, DiffSession) {
    collapsing_with(aligned, true)
}

fn collapsing_with(aligned: bool, wrap: bool) -> (TestCore, ViewId, TestCore, ViewId, DiffSession) {
    let (mut base, base_view) = core(&lines(40, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(40, collapse_lines));
    if !wrap {
        for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
            type_text(core, view, ":set nowrap");
            key(core, view, Key::Enter);
        }
    }
    let mut session = DiffSession::new(base.document(), new.document(), aligned);
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    (base, base_view, new, new_view, session)
}

/// The hard lines that have rows, in order.
fn shown_lines(core: &TestCore, view: ViewId) -> Vec<usize> {
    let mut lines: Vec<usize> = core.layout(view).unwrap().snapshot().unwrap().rows.iter().map(|row| row.hard_line_index).collect();
    lines.dedup();
    lines
}

/// Hard lines whose rows carry a collapsed-context divider band.
fn divider_lines(core: &TestCore, view: ViewId) -> Vec<usize> {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let mut lines: Vec<usize> = snapshot.rows.iter()
        .filter(|row| row.decorations.iter().any(|item| item.kind == DecorationKind::CollapsedContext { rule: false }))
        .map(|row| row.hard_line_index)
        .collect();
    lines.dedup();
    lines
}

fn ranges(ranges: &[Range<usize>]) -> Vec<usize> {
    ranges.iter().flat_map(Clone::clone).collect()
}

fn place(core: &mut TestCore, view: ViewId, line: usize) {
    type_text(core, view, &format!(":{}", line + 1));
    key(core, view, Key::Enter);
}

#[test]
fn collapsed_context_hides_the_same_equal_runs_on_both_sides_behind_one_divider_each() {
    for (aligned, wrap) in [(true, true), (false, true), (true, false)] {
        let (base, base_view, new, new_view, _) = collapsing_with(aligned, wrap);
        assert_eq!(new.layout(new_view).unwrap().wrap(), wrap);
        for (core, view) in [(&base, base_view), (&new, new_view)] {
            assert_eq!(shown_lines(core, view), ranges(&[7..14, 27..34]), "aligned {aligned}, wrap {wrap}");
            assert_eq!(divider_lines(core, view), vec![7, 13, 33]);
            let layout = core.layout(view).unwrap();
            for hidden in [0..7, 14..27, 34..41] {
                let height = layout.hard_line_range_height(hidden).unwrap();
                assert!(height.is_exact() && height.height() == 0.0);
            }
        }
        // Presentation only: the source, history and dirty state are untouched.
        assert_eq!(new.document().source_bytes(), lines(40, collapse_lines).as_bytes());
        assert!(!new.document().is_dirty());
        assert_eq!(row_top(&base, base_view, 27), row_top(&new, new_view, 27));
    }
}

#[test]
fn a_divider_is_one_default_row_in_the_margin_beside_the_hidden_lines() {
    let (mut base, base_view, mut new, new_view, mut session) = collapsing(true);
    let row = new.default_row_height(new_view).unwrap();
    let collapsed = (row_top(&new, new_view, 7), row_top(&new, new_view, 13), row_top(&new, new_view, 27));
    session.set_collapsed(false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let first = row_top(&new, new_view, 0);
    assert!((collapsed.0 - (first + row)).abs() < 0.01, "the top divider replaces lines 0..7");
    assert!((collapsed.2 - collapsed.1 - 2.0 * row).abs() < 0.01, "line 13, then the divider for 14..27");
    // The scrollbar marks follow the collapsed extent.
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let (marks, extent) = session.scrollbar_marks(&mut new, new_view, Side::New).unwrap();
    assert_eq!(marks[0].top, row_top(&new, new_view, 10));
    assert_eq!(extent, core_height(&new, new_view));
}

#[test]
fn clicking_a_divider_opens_its_run_on_both_sides_until_collapsing_is_toggled() {
    let (mut base, base_view, mut new, new_view, mut session) = collapsing(true);
    // The divider after line 13 opens 14..27; the top divider below line 7
    // opens 0..7.
    assert!(session.open_collapsed_beside(Side::Base, 13));
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(session.open_collapsed_beside(Side::New, 7));
    assert!(!session.open_collapsed_beside(Side::New, 20), "no divider borders a shown line");
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    for (core, view) in [(&base, base_view), (&new, new_view)] {
        assert_eq!(divider_lines(core, view), vec![33]);
        assert_eq!(core.layout(view).unwrap().elided().runs(), [34..41]);
    }
    session.set_collapsed(false);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert!(new.layout(new_view).unwrap().elided().runs().is_empty());
    assert!(divider_lines(&new, new_view).is_empty());
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 14..27, 34..41], "opened runs closed again");
}

#[test]
fn a_caret_moving_onto_collapsed_context_opens_it_on_both_sides() {
    // By `j`, `:N` and a search; each opens only its own run.
    let moves: [(&dyn Fn(&mut TestCore, ViewId), Range<usize>); 3] = [
        (&|core, view| { place(core, view, 13); key(core, view, Key::Char('j')); }, 14..27),
        (&|core, view| place(core, view, 36), 34..41),
        (&|core, view| { type_text(core, view, "/line 3$"); key(core, view, Key::Enter); }, 0..7),
    ];
    for (index, (motion, opened)) in moves.into_iter().enumerate() {
        let (mut base, base_view, mut new, new_view, mut session) = collapsing(true);
        motion(&mut new, new_view);
        assert!(!new.layout(new_view).unwrap().elided().runs().contains(&opened),
            "motion {index} opened its run before any caret geometry: {:?}, caret {}",
            new.layout(new_view).unwrap().elided().runs(), cursor_line(&new, new_view));
        assert!(new.has_opened_collapsed(new_view).unwrap());
        session.sync(&mut base, base_view, &mut new, new_view).unwrap();
        assert!(!new.has_opened_collapsed(new_view).unwrap());
        let expected: Vec<_> = [0..7, 14..27, 34..41].into_iter().filter(|run| *run != opened).collect();
        assert_eq!(new.layout(new_view).unwrap().elided().runs(), expected);
        assert_eq!(base.layout(base_view).unwrap().elided().runs(), expected);
        assert!(shown_lines(&new, new_view).contains(&cursor_line(&new, new_view)));
    }
    // The base side's caret opens runs too.
    let (mut base, base_view, mut new, new_view, mut session) = collapsing(true);
    place(&mut base, base_view, 20);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 34..41]);
}

#[test]
fn a_starting_caret_moves_beside_collapsed_context_but_later_keeps_its_run_open() {
    // Both carets start on line 0, which collapses: they move after the run.
    let (base, base_view, new, new_view, _) = collapsing(true);
    assert_eq!(cursor_line(&new, new_view), 7);
    assert_eq!(cursor_line(&base, base_view), 7);
    // Collapsing later leaves the caret's own run open.
    let (mut base, base_view) = core(&lines(40, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(40, collapse_lines));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    place(&mut new, new_view, 20);
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(cursor_line(&new, new_view), 20);
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [34..41]);
}

#[test]
fn typing_beside_collapsed_context_reflows_it_incrementally_like_a_fresh_diff() {
    let (mut base, base_view, mut new, new_view, mut session) = collapsing(true);
    place(&mut new, new_view, 13);
    type_text(&mut new, new_view, "Ax");
    key(&mut new, new_view, Key::Escape);
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Incremental);
    // Line 13 changed, so 14..17 become its context.
    let expected = [0..7, 17..27, 34..41];
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), expected);
    assert_eq!(base.layout(base_view).unwrap().elided().runs(), expected);
    let (mut fresh_base, fresh_base_view) = core(&lines(40, |line| Some(format!("line {line}"))));
    let (mut fresh_new, fresh_new_view) = core(&String::from_utf8(new.document().source_bytes().to_vec()).unwrap());
    let mut fresh = DiffSession::new(fresh_base.document(), fresh_new.document(), true);
    fresh.set_collapsed(true);
    fresh.sync(&mut fresh_base, fresh_base_view, &mut fresh_new, fresh_new_view).unwrap();
    assert_eq!(fresh.collapsed_runs(), session.collapsed_runs());
    place(&mut fresh_new, fresh_new_view, 13);
    assert_eq!(shown_lines(&fresh_new, fresh_new_view), shown_lines(&new, new_view));
    assert_eq!(row_top(&fresh_new, fresh_new_view, 27), row_top(&new, new_view, 27));
    // An opened run stays open while its lines remain equal.
    place(&mut new, new_view, 20);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    place(&mut new, new_view, 8);
    type_text(&mut new, new_view, "Ay");
    key(&mut new, new_view, Key::Escape);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..5, 34..41], "17..27 stays open beside the new change");
}

#[test]
fn collapsed_lines_cost_no_layout_work_in_a_million_line_document() {
    const COUNT: usize = 1_000_000;
    let (mut base, base_view) = core(&lines(COUNT, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(COUNT, |line| Some(if line == 500_000 { "changed".into() } else { format!("line {line}") })));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
        assert_eq!(shown_lines(core, view), ranges(&[499_997..500_004]));
        let layout = core.layout(view).unwrap();
        assert_eq!(layout.elided().runs(), [0..499_997, 500_004..COUNT + 1], "the empty last line too");
        let cached: usize = layout.regional_cached_ranges().iter().map(Range::len).sum();
        assert!(cached <= 7, "only shown lines are cached: {cached}");
        // A region spanning the whole document captures only the shown text.
        let request = core.prepare_view_layout_job(view, LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::HardLines(crate::layout::HardLineLayoutRegion::new(0..COUNT).unwrap()),
            LayoutCancellationToken::new()).unwrap();
        assert!(request.captured_text_len() < 200, "captured {}", request.captured_text_len());
    }
}

/// Only hidden lines have an exact zero height.
fn assert_only_hidden_lines_have_no_height(core: &TestCore, view: ViewId) {
    let layout = core.layout(view).unwrap();
    let count = core.document().projection().presentation_line_count();
    for line in 0..count {
        let height = layout.hard_line_range_height(line..line + 1).unwrap();
        let hidden = layout.elided().run_at(line).is_some();
        assert_eq!(hidden, height.is_exact() && height.height() == 0.0, "line {line}: {height:?}");
    }
}

/// Base lines 0..400; the new side changes lines 10 and 390, so lines
/// 14..387 collapse, far more than the view shows.
fn long_collapse() -> (TestCore, ViewId, TestCore, ViewId, DiffSession) {
    let changed = |line: usize| Some(if line == 10 || line == 390 { format!("changed {line}") } else { format!("line {line}") });
    let (mut base, base_view) = core(&lines(400, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(400, changed));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 14..387, 394..401]);
    (base, base_view, new, new_view, session)
}

#[test]
fn hidden_lines_that_an_edit_reaches_are_shown_again_with_a_height() {
    // Another view of the buffer edits hidden line 199 while the diff
    // view's caret stays on line 7: the diff view shows 14..387 again, with
    // estimated heights offscreen.
    let (mut base, base_view, mut new, new_view, mut session) = long_collapse();
    let other = new.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    type_text(&mut new, other, ":200s/line/LINE/");
    key(&mut new, other, Key::Enter);
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 394..401]);
    assert_only_hidden_lines_have_no_height(&new, new_view);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(cursor_line(&new, new_view), 7);
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 14..196, 203..387, 394..401]);
    assert_only_hidden_lines_have_no_height(&new, new_view);
    key(&mut new, other, Key::Char('u'));
    assert_only_hidden_lines_have_no_height(&new, new_view);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 14..387, 394..401]);
    assert_only_hidden_lines_have_no_height(&new, new_view);
    assert_only_hidden_lines_have_no_height(&base, base_view);
    // The diff view's own edit moves its caret onto line 199, which opens
    // that run on both sides while its lines stay equal.
    let (mut base, base_view, mut new, new_view, mut session) = long_collapse();
    type_text(&mut new, new_view, ":200s/line/LINE/");
    key(&mut new, new_view, Key::Enter);
    assert_only_hidden_lines_have_no_height(&new, new_view);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    assert_eq!(cursor_line(&new, new_view), 199);
    assert_eq!(new.layout(new_view).unwrap().elided().runs(), [0..7, 394..401]);
    assert_eq!(base.layout(base_view).unwrap().elided().runs(), [0..7, 394..401]);
    assert_only_hidden_lines_have_no_height(&base, base_view);
}

/// The top line of a view and how far into it the view starts.
fn top_position(core: &TestCore, view: ViewId) -> (usize, f64) {
    let layout = core.layout(view).unwrap();
    let top = f64::from(layout.viewport_top());
    let hit = layout.hard_line_at_y(top).unwrap().unwrap();
    (hit.hard_line(), top - hit.line_top())
}

#[test]
fn opening_a_line_inside_an_addition_only_scrolls_to_reveal_the_live_caret() {
    // The viewport starts in the addition. The base's first text below its
    // filler moves down when the addition grows, but the live text above
    // the edit stays put unless the next row is outside the viewport.
    let base_text = "before\nafter\n2\n3\n4\n5\n6\n7";
    let new_text = base_text.replace("before\n", "before\na\nb\nc\nd\ne\nf\ng\nh\n");
    for collapsed in [false, true] {
        for (edit_line, open_key, reveal) in [(3, Key::Char('o'), false), (3, Key::Enter, false), (6, Key::Enter, true)] {
            let (mut base, bv) = core(base_text);
            let (mut new, nv) = core(&new_text);
            let height = 5. * new.default_row_height(nv).unwrap();
            for (core, view) in [(&mut base, bv), (&mut new, nv)] {
                core.handle(view, CoreEvent::Resize { width: 600., height }).unwrap();
            }
            let mut session = DiffSession::new(base.document(), new.document(), true);
            session.set_collapsed(collapsed);
            session.sync(&mut base, bv, &mut new, nv).unwrap();
            place(&mut new, nv, edit_line);
            if open_key == Key::Enter { key(&mut new, nv, Key::Char('A')); }
            let anchor_line = 2;
            let initial_top = row_top(&new, nv, anchor_line) + 1.;
            new.handle(nv, CoreEvent::SetViewportOrigin { left: 0., top: Some(initial_top) }).unwrap();
            session.sync_scroll(&mut base, bv, &mut new, nv, Side::New).unwrap();
            let initial_anchor_y = row_top(&new, nv, anchor_line) - initial_top;

            key(&mut new, nv, open_key);
            session.sync(&mut base, bv, &mut new, nv).unwrap();
            // Publishing the base first must not turn its reflow into a scroll.
            session.sync_scroll(&mut base, bv, &mut new, nv, Side::Base).unwrap();
            assert_eq!(cursor_line(&new, nv), edit_line + 1);
            let layout = new.layout(nv).unwrap();
            let caret_row = layout.snapshot().unwrap().rows.iter().find(|r| r.hard_line_index == edit_line + 1).unwrap();
            let expected_top = initial_top.max(caret_row.reveal_bounds().end - height);
            assert_eq!(expected_top > initial_top, reveal);
            assert!((layout.viewport_top() - expected_top).abs() < 0.01, "collapsed {collapsed}, line {edit_line}");
            assert_eq!(top(&base, bv), top(&new, nv));
            if !reveal {
                assert!((row_top(&new, nv, anchor_line) - layout.viewport_top() - initial_anchor_y).abs() < 0.01);
            }
        }
    }
}

#[test]
fn opening_a_line_below_deletion_filler_uses_the_viewport_before_the_edit() {
    let live = lines(20, |line| Some(format!("line {line}")));
    let removed = lines(12, |line| Some(format!("removed {line}")));
    let older = live.replace("line 8\n", &format!("{removed}line 8\n"));
    for scroll_after_input in [false, true] {
        let (mut base, bv) = core(&older);
        let (mut new, nv) = core(&live);
        for (core, view) in [(&mut base, bv), (&mut new, nv)] {
            core.handle(view, CoreEvent::Resize { width: 600., height: 120. }).unwrap();
        }
        let mut session = DiffSession::new(base.document(), new.document(), true);
        session.set_collapsed(true);
        session.sync(&mut base, bv, &mut new, nv).unwrap();
        place(&mut new, nv, 10);
        key(&mut new, nv, Key::Char('A'));
        let start = prefix(&new, nv, 9) as f32;
        new.handle(nv, CoreEvent::SetViewportOrigin { left: 0., top: Some(start) }).unwrap();
        session.sync_scroll(&mut base, bv, &mut new, nv, Side::New).unwrap();
        let screen_y = |core: &TestCore| row_top(core, nv, 9) - core.layout(nv).unwrap().viewport_top();
        let before = screen_y(&new);

        key(&mut new, nv, Key::Enter);
        if scroll_after_input {
            new.handle(nv, CoreEvent::SetViewportOrigin { left: 0., top: Some(0.) }).unwrap();
        }
        session.sync(&mut base, bv, &mut new, nv).unwrap();
        assert_eq!(top(&base, bv), top(&new, nv));
        if scroll_after_input {
            assert_eq!(top(&new, nv), 0., "an explicit scroll supersedes the earlier typing viewport");
        } else {
            assert!((screen_y(&new) - before).abs() < 0.01, "the row beside the deletion stays fixed");
            let layout = new.layout(nv).unwrap();
            let caret_line = cursor_line(&new, nv);
            let caret = layout.snapshot().unwrap().rows.iter().find(|r| r.hard_line_index == caret_line).unwrap();
            assert!(caret.reveal_bounds().end <= layout.viewport_top() + layout.height() + 0.01);
        }
    }
}

fn assert_level(base: &TestCore, base_view: ViewId, new: &TestCore, new_view: ViewId, session: &DiffSession) {
    let (new_line, new_offset) = top_position(new, new_view);
    let (base_line, base_offset) = top_position(base, base_view);
    assert_eq!(base_line, session.corresponding_line(Side::New, new_line), "the base lines up with the new side");
    assert!((base_offset - new_offset).abs() < 0.5, "base {base_offset} into its line, new {new_offset}");
}

#[test]
fn aligned_sides_stay_level_whatever_each_side_has_measured() {
    let mut text: Vec<String> = (0..2000).map(|line| format!("line {line}")).collect();
    let (mut base, base_view) = core(&text.join("\n"));
    text[1500] = "changed".into();
    let (mut new, new_view) = core(&text.join("\n"));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    // Only the new side measures the lines around 800.
    let around = prefix(&new, new_view, 800) as f32;
    new.materialize_requested_viewport(new_view, 0.0, around).unwrap();
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap();
    assert_eq!(cursor_line(&new, new_view), 1500);
    assert_level(&base, base_view, &new, new_view, &session);
}

#[test]
fn aligned_sides_stay_level_below_filler_neither_side_has_measured() {
    let (mut base, base_view) = core(&lines(2000, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(2000, |line| match line {
        500 => Some((0..40).map(|added| format!("added {added}\n")).collect::<String>() + "line 500"),
        1500 => Some("changed".into()),
        _ => Some(format!("line {line}")),
    }));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    // From the top, the last change is the previous one: the 40 added lines
    // and the base's filler for them are never laid out.
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Previous).unwrap();
    assert_eq!(cursor_line(&new, new_view), 1540);
    assert_level(&base, base_view, &new, new_view, &session);
    // An edit far above renumbers the lines and gives the base's first line
    // filler; the next presentation estimates both again.
    type_text(&mut new, new_view, "ggOinserted");
    key(&mut new, new_view, Key::Escape);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Previous).unwrap();
    assert_eq!(cursor_line(&new, new_view), 1541);
    assert_level(&base, base_view, &new, new_view, &session);
}

#[test]
fn aligned_sides_stay_level_when_an_edited_side_is_resized_before_its_next_presentation() {
    let (mut base, base_view) = core(&lines(2000, |line| Some(format!("line {line}"))));
    // The new side lacks base lines 300..340, so its line 300 carries filler.
    let (mut new, new_view) = core(&lines(2000, |line| match line {
        300..340 => None,
        1500 => Some("changed".into()),
        _ => Some(format!("line {line}")),
    }));
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    // One edit renumbers the new side's lines, and a new width invalidates
    // every estimate before diff mode installs the next presentation, which
    // only re-lays out the lines whose appearance changed.
    key(&mut new, new_view, Key::Char('O'));
    key(&mut new, new_view, Key::Escape);
    new.handle(new_view, CoreEvent::Resize { width: 500., height: 400. }).unwrap();
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Incremental);
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Previous).unwrap();
    assert_eq!(cursor_line(&new, new_view), 1461);
    assert_level(&base, base_view, &new, new_view, &session);
}

/// Wrap `view` at `width`.
fn wrap_at(core: &mut TestCore, view: ViewId, width: f32) {
    core.handle(view, CoreEvent::Resize { width, height: 300. }).unwrap();
    core.handle(view, CoreEvent::SetWrap(true)).unwrap();
}

/// Each new-side line of `lines` starts at the same height as the base line
/// corresponding to it.
fn assert_lines_level(base: &TestCore, base_view: ViewId, new: &TestCore, new_view: ViewId, session: &DiffSession,
    lines: Range<usize>) {
    for line in lines {
        let other = session.corresponding_line(Side::New, line);
        let (base_top, new_top) = (prefix(base, base_view, other), prefix(new, new_view, line));
        assert!((base_top - new_top).abs() < 0.5, "new line {line} starts at {new_top}, base line {other} at {base_top}");
    }
}

fn long_line() -> String {
    "several words on a wrapped line ".repeat(12)
}

#[test]
fn a_change_that_wraps_to_more_rows_on_one_side_keeps_later_lines_level() {
    for long_side in [Side::New, Side::Base] {
        let text = |side: Side| lines(300, |line| Some(if line == 100 && side == long_side { long_line() } else { format!("line {line}") }));
        let (mut base, base_view) = core(&text(Side::Base));
        let (mut new, new_view) = core(&text(Side::New));
        for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
            wrap_at(core, view, 300.);
        }
        let mut session = DiffSession::new(base.document(), new.document(), true);
        session.sync(&mut base, base_view, &mut new, new_view).unwrap();
        let top = prefix(&new, new_view, 95) as f32;
        new.handle(new_view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
        session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
        let row = f64::from(new.default_row_height(new_view).unwrap());
        let (long, view) = if long_side == Side::New { (&new, new_view) } else { (&base, base_view) };
        assert!(long.layout(view).unwrap().hard_line_range_height(100..101).unwrap().height() > 3. * row, "the change wraps");
        assert_lines_level(&base, base_view, &new, new_view, &session, 0..300);
        assert_level(&base, base_view, &new, new_view, &session);
    }
}

#[test]
fn an_unchanged_line_wrapped_differently_on_each_side_keeps_later_lines_level() {
    let text = |changed: bool| lines(300, |line| Some(match line {
        50 => long_line(),
        250 if changed => "changed".into(),
        _ => format!("line {line}"),
    }));
    let (mut base, base_view) = core(&text(false));
    let (mut new, new_view) = core(&text(true));
    wrap_at(&mut base, base_view, 400.);
    wrap_at(&mut new, new_view, 250.);
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    let top = prefix(&new, new_view, 45) as f32;
    new.handle(new_view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    let rows = |core: &TestCore, view: ViewId| core.layout(view).unwrap().snapshot().unwrap().rows.iter()
        .filter(|row| row.hard_line_index == 50).count();
    assert!(rows(&new, new_view) > rows(&base, base_view), "the narrower side wraps to more rows");
    assert_lines_level(&base, base_view, &new, new_view, &session, 0..300);
    // The comparison follows the line when its width changes again.
    new.handle(new_view, CoreEvent::Resize { width: 400., height: 300. }).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    assert_lines_level(&base, base_view, &new, new_view, &session, 0..300);
}

#[test]
fn a_wrapped_line_only_one_side_has_measured_keeps_the_sides_level() {
    let text = |changed: bool| lines(2000, |line| Some(match line {
        300 => long_line(),
        1500 if changed => "changed".into(),
        _ => format!("line {line}"),
    }));
    let (mut base, base_view) = core(&text(false));
    let (mut new, new_view) = core(&text(true));
    for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
        wrap_at(core, view, 300.);
    }
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    // Only the new side measures the wrapped line.
    let around = prefix(&new, new_view, 295) as f32;
    new.materialize_requested_viewport(new_view, 0.0, around).unwrap();
    assert!(new.layout(new_view).unwrap().hard_line_range_height(300..301).unwrap().is_exact());
    session.navigate_change(&mut base, base_view, &mut new, new_view, ChangeDirection::Next).unwrap();
    assert_eq!(cursor_line(&new, new_view), 1500);
    assert!(!base.layout(base_view).unwrap().hard_line_range_height(300..301).unwrap().is_exact(),
        "the base never measured it");
    assert_level(&base, base_view, &new, new_view, &session);
}

#[test]
fn a_divider_stays_level_below_a_line_wrapped_differently_on_each_side() {
    let text = |changed: bool| lines(41, |line| Some(match line {
        0 | 40 if changed => "changed".into(),
        3 => long_line(),
        _ => format!("line {line}"),
    }));
    let (mut base, base_view) = core(&text(false));
    let (mut new, new_view) = core(&text(true));
    wrap_at(&mut base, base_view, 400.);
    wrap_at(&mut new, new_view, 250.);
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.set_collapsed(true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    let divider = |core: &TestCore, view: ViewId| {
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        snapshot.rows.iter().flat_map(|row| row.decorations.iter())
            .find(|item| item.kind == DecorationKind::CollapsedContext { rule: false })
            .map(|item| item.typographic_bounds.y)
            .unwrap()
    };
    assert_eq!(divider_lines(&new, new_view), vec![3]);
    assert!((divider(&base, base_view) - divider(&new, new_view)).abs() < 0.5,
        "base divider at {}, new at {}", divider(&base, base_view), divider(&new, new_view));
    assert_lines_level(&base, base_view, &new, new_view, &session, 37..41);
}

#[test]
fn lines_after_additions_at_the_start_of_one_side_stay_level_with_padding() {
    let (mut base, base_view) = core(&lines(100, |line| Some(format!("line {line}"))));
    let (mut new, new_view) = core(&lines(103, |line| Some(if line < 3 { format!("added {line}") } else { format!("line {}", line - 3) })));
    for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
        core.set_view_insets(view, crate::layout::EdgeInsets { top: 20., left: 0., bottom: 30., right: 0. }).unwrap();
    }
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    // The first line's band starts above the padding and the filler.
    assert_eq!(row_top(&base, base_view, 0), row_top(&new, new_view, 3));
    assert_lines_level(&base, base_view, &new, new_view, &session, 4..103);
}

#[test]
fn a_line_whose_filler_changes_keeps_the_height_measured_for_its_text() {
    // The new side adds a line after the long line 9, so the base has filler
    // below it.
    let (mut base, base_view) = core(&lines(600, |line| Some(if line == 9 { long_line() } else { format!("line {line}") })));
    let (mut new, new_view) = core(&lines(601, |line| Some(match line {
        9 => long_line(),
        10 => "added".into(),
        11.. => format!("line {}", line - 1),
        _ => format!("line {line}"),
    })));
    for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
        wrap_at(core, view, 300.);
    }
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    let band = |core: &TestCore| core.layout(base_view).unwrap().hard_line_range_height(9..10).unwrap();
    let row = f64::from(base.default_row_height(base_view).unwrap());
    let measured = band(&base);
    assert!(measured.is_exact() && measured.height() > 3. * row);
    // With the base's long line far offscreen, a second added line grows the
    // filler below it by one row.
    let far = prefix(&new, new_view, 500) as f32;
    new.handle(new_view, CoreEvent::SetViewportOrigin { left: 0., top: Some(far) }).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    assert!(!base.layout(base_view).unwrap().snapshot().unwrap().rows.iter().any(|row| row.hard_line_index == 9));
    place(&mut new, new_view, 10);
    type_text(&mut new, new_view, "yyp");
    assert_eq!(session.sync(&mut base, base_view, &mut new, new_view).unwrap(), DiffSync::Incremental);
    assert!((band(&base).height() - (measured.height() + row)).abs() < 0.5,
        "measured {} grows by one row to {}", measured.height(), band(&base).height());
}

#[test]
fn padded_sides_share_one_extent_before_either_measures_its_end() {
    // The new side adds lines after the base's unterminated last line, so
    // only the base's last line is also the last of the unchanged run;
    // neither side lays it out.
    let text = |count: usize| (0..count).map(|line| if line < 2000 { format!("line {line}") } else { format!("added {line}") })
        .collect::<Vec<_>>().join("\n");
    let (mut base, base_view) = core(&text(2000));
    let (mut new, new_view) = core(&text(2003));
    for (core, view) in [(&mut base, base_view), (&mut new, new_view)] {
        core.set_view_insets(view, crate::layout::EdgeInsets { top: 20., left: 0., bottom: 30., right: 0. }).unwrap();
    }
    let mut session = DiffSession::new(base.document(), new.document(), true);
    session.sync(&mut base, base_view, &mut new, new_view).unwrap();
    session.sync_scroll(&mut base, base_view, &mut new, new_view, Side::New).unwrap();
    let end = |core: &TestCore, view: ViewId| core.layout(view).unwrap().hard_line_range_height(1990..core.document().line_count()).unwrap();
    assert!(!end(&base, base_view).is_exact() && !end(&new, new_view).is_exact());
    let total = |core: &TestCore, view: ViewId| core.layout(view).unwrap().content_height().height();
    assert!((total(&base, base_view) - total(&new, new_view)).abs() < 0.5,
        "base {} new {}", total(&base, base_view), total(&new, new_view));
    assert_lines_level(&base, base_view, &new, new_view, &session, 1..2000);
    // Estimates hold the padding as measurements do, so nothing needs
    // balancing.
    for side in [Side::Base, Side::New] {
        assert_eq!(session.installed(side).balance, Balance::default(), "{side:?}");
    }
}
