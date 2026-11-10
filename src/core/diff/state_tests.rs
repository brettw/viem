use super::*;

fn lines(text: &str) -> TextLines {
    TextLines::new(text.to_owned())
}

fn hunks(state: &DiffState) -> Vec<(Range<usize>, Range<usize>)> {
    state.hunks().map(|hunk| (hunk.base, hunk.new)).collect()
}

/// Replace `lines[start..end]` with `replacement` and describe the edit.
fn edit(current: &mut Vec<String>, start: usize, end: usize, replacement: &[&str]) -> LineEdit {
    current.splice(start..end, replacement.iter().map(|line| line.to_string()));
    LineEdit { start, old_end: end, new_end: start + replacement.len() }
}

fn joined(current: &[String]) -> TextLines {
    lines(&current.join("\n"))
}

#[test]
fn from_scratch_diff_reports_changed_regions() {
    let base = lines("a\nb\nc\nd");
    let new = lines("a\nB\nc\nd\ne");
    let state = DiffState::compute(&base, &new, DiffLimits::default());
    assert_eq!(hunks(&state), vec![(1..2, 1..2), (4..4, 4..5)]);
    assert!(state.reproduces(&base, &new));
}

#[test]
fn identical_sides_have_no_hunks() {
    let text = lines("one\ntwo\n");
    let state = DiffState::compute(&text, &text, DiffLimits::default());
    assert_eq!(state.hunk_count(), 0);
    assert_eq!(state.new_line_count(), 3);
}

#[test]
fn a_typed_line_is_reported_where_it_was_inserted() {
    // Inserting a duplicate in a run of equal lines is ambiguous to a
    // from-scratch diff; the edit says exactly which line is new.
    let base = lines("x\nx\nx\nx");
    let mut current: Vec<String> = ["x", "x", "x", "x"].map(String::from).to_vec();
    let mut state = DiffState::compute(&base, &joined(&current), DiffLimits::default());
    let change = edit(&mut current, 2, 2, &["x"]);
    let new = joined(&current);
    state.apply_edit(&base, &new, change);
    assert_eq!(hunks(&state), vec![(2..2, 2..3)]);
    assert!(state.reproduces(&base, &new));
}

#[test]
fn typing_inside_a_line_marks_it_changed_and_undoing_restores_equality() {
    let base = lines("alpha\nbeta\ngamma");
    let mut current: Vec<String> = ["alpha", "beta", "gamma"].map(String::from).to_vec();
    let mut state = DiffState::compute(&base, &joined(&current), DiffLimits::default());
    let change = edit(&mut current, 1, 2, &["betax"]);
    state.apply_edit(&base, &joined(&current), change);
    assert_eq!(hunks(&state), vec![(1..2, 1..2)]);
    let hunk = state.hunks().next().unwrap();
    assert_eq!(hunk.pairs.len(), 1);
    assert_eq!(hunk.pairs[0].spans.new, vec![4..5]);
    let change = edit(&mut current, 1, 2, &["beta"]);
    state.apply_edit(&base, &joined(&current), change);
    assert_eq!(state.hunk_count(), 0);
}

#[test]
fn editing_next_to_a_hunk_merges_with_it() {
    let base = lines("a\nb\nc\nd");
    let mut current: Vec<String> = ["a", "B", "c", "d"].map(String::from).to_vec();
    let mut state = DiffState::compute(&base, &joined(&current), DiffLimits::default());
    let change = edit(&mut current, 2, 3, &["C"]);
    let new = joined(&current);
    state.apply_edit(&base, &new, change);
    assert_eq!(hunks(&state), vec![(1..3, 1..3)]);
    assert!(state.reproduces(&base, &new));
}

#[test]
fn deleting_lines_next_to_a_deletion_hunk_includes_its_base_lines() {
    let base = lines("a\nb\nc\nd\ne");
    let mut current: Vec<String> = ["a", "d", "e"].map(String::from).to_vec();
    let mut state = DiffState::compute(&base, &joined(&current), DiffLimits::default());
    assert_eq!(hunks(&state), vec![(1..3, 1..1)]);
    let change = edit(&mut current, 1, 2, &[]);
    let new = joined(&current);
    state.apply_edit(&base, &new, change);
    assert_eq!(hunks(&state), vec![(1..4, 1..1)]);
    assert!(state.reproduces(&base, &new));
}

#[test]
fn an_inconsistent_edit_recomputes_from_scratch() {
    let base = lines("a\nb");
    let new = lines("a\nc\nd");
    let mut state = DiffState::compute(&base, &base, DiffLimits::default());
    state.apply_edit(&base, &new, LineEdit { start: 0, old_end: 1, new_end: 1 });
    assert_eq!(state, DiffState::compute(&base, &new, DiffLimits::default()));
}

#[test]
fn editing_a_changed_line_reads_only_its_pair_or_no_lines_for_an_addition() {
    struct Counted(TextLines, std::cell::Cell<usize>);
    impl LineSource for Counted {
        fn line_count(&self) -> usize { self.0.line_count() }
        fn line(&self, index: usize) -> Cow<'_, str> {
            self.1.set(self.1.get() + 1);
            self.0.line(index)
        }
    }
    let before = ["first", "value 1 old", "value 2 old", "value 3 old", "last"].join("\n");
    let added = ["first", "last"].join("\n");
    let changed = ["first", "value 1 new", "value 2 new", "value 3 new", "last"].join("\n");
    let after = changed.replace("value 2 new", "value 2 newer");
    for (original, reads) in [(added, 0), (before, 1)] {
        let base = Counted(lines(&original), Default::default());
        let mut state = DiffState::compute(&base, &lines(&changed), DiffLimits::default());
        base.1.set(0);
        let new = Counted(lines(&after), Default::default());
        state.apply_edit(&base, &new, LineEdit { start: 2, old_end: 3, new_end: 3 });
        assert_eq!((base.1.get(), new.1.get()), (reads, reads));
        assert_eq!(state, DiffState::compute(&base, &new, DiffLimits::default()));
    }
}

#[test]
fn a_split_and_typing_between_surviving_pairs_read_only_the_gap() {
    struct Window<'a>(&'a TextLines, Range<usize>);
    impl LineSource for Window<'_> {
        fn line_count(&self) -> usize { self.0.line_count() }
        fn line(&self, index: usize) -> Cow<'_, str> {
            assert!(self.1.contains(&index), "read line {index} outside {:?}", self.1);
            self.0.line(index)
        }
    }
    // The first square replacement larger than the usual weighted-pairing limit.
    let original: Vec<_> = (0..21).map(|i| format!("let variable_{i} = 10;")).collect();
    let mut current: Vec<_> = original.iter().map(|s| s.replace("10;", "20;")).collect();
    let base = joined(&original);
    let mut state = DiffState::compute(&base, &joined(&current), DiffLimits::default());
    let before = state.clone();
    let row = 10;
    let changed_line = current[row].clone();
    let change = edit(&mut current, row, row + 1, &[&changed_line, ""]);
    let new = joined(&current);
    state.apply_edit(&Window(&base, row..row + 1), &Window(&new, row..row + 2), change);
    assert!(state.reproduces(&base, &new));
    assert_eq!(state.line_basis(row + 1), Some(DiffLineBasis::Added));
    let hunk = state.hunks().next().unwrap();
    assert_eq!(hunk.pairs[row + 1].base, row + 1);
    assert_eq!(hunk.pairs[row + 1].new, row + 2);

    let change = edit(&mut current, row + 1, row + 2, &["x"]);
    let new = joined(&current);
    state.apply_edit(&Window(&base, 0..0), &Window(&new, 0..0), change);
    assert!(state.reproduces(&base, &new));
    // Reverse the split and the typing in it.
    let change = edit(&mut current, row, row + 2, &[&changed_line]);
    let new = joined(&current);
    state.apply_edit(&Window(&base, row..row + 1), &Window(&new, row..row + 1), change);
    assert_eq!(state, before);

    // Restoring equality splits the hunk and re-bases all suffix pair offsets.
    let change = edit(&mut current, row, row + 1, &[&original[row]]);
    let new = joined(&current);
    state.apply_edit(&Window(&base, row..row + 1), &Window(&new, row..row + 1), change);
    assert_eq!(hunks(&state), vec![(0..row, 0..row), (row + 1..21, row + 1..21)]);
    assert_eq!(state, DiffState::compute(&base, &new, DiffLimits::default()));
    assert!(state.reproduces(&base, &new));
}

#[test]
fn splitting_a_hunk_beyond_the_inline_limit_drops_its_pair_details() {
    let original = lines("value A\nvalue B");
    let mut current = vec![String::from("value A!"), String::from("value B!")];
    let limits = DiffLimits { inline_lines: 2, ..DiffLimits::default() };
    let mut state = DiffState::compute(&original, &joined(&current), limits);
    assert_eq!(state.hunks().next().unwrap().pairs.len(), 2);
    let change = edit(&mut current, 0, 1, &["value A!", ""]);
    let new = joined(&current);
    state.apply_edit(&original, &new, change);
    assert!(state.hunks().next().unwrap().pairs.is_empty());
    assert!(state.reproduces(&original, &new));
}

#[test]
fn an_edit_larger_than_the_window_is_one_exact_hunk() {
    let base = lines(&(0..100).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"));
    let mut current: Vec<String> = (0..100).map(|n| n.to_string()).collect();
    let limits = DiffLimits { window_lines: 10, ..DiffLimits::default() };
    let mut state = DiffState::compute(&base, &joined(&current), limits);
    let pasted: Vec<String> = (0..20).map(|n| format!("p{n}")).collect();
    let pasted: Vec<&str> = pasted.iter().map(String::as_str).collect();
    let change = edit(&mut current, 50, 50, &pasted);
    let new = joined(&current);
    state.apply_edit(&base, &new, change);
    assert_eq!(hunks(&state), vec![(50..50, 50..70)]);
    assert!(state.reproduces(&base, &new));
}

#[test]
fn corresponding_lines_follow_equal_runs_and_clamp_in_hunks() {
    let base = lines("a\nb\nc\nd");
    let new = lines("a\nx\ny\nz\nc\nd");
    let state = DiffState::compute(&base, &new, DiffLimits::default());
    assert_eq!(hunks(&state), vec![(1..2, 1..4)]);
    assert_eq!(state.corresponding_line(Side::New, 0), 0);
    assert_eq!(state.corresponding_line(Side::New, 3), 1);
    assert_eq!(state.corresponding_line(Side::New, 4), 2);
    assert_eq!(state.corresponding_line(Side::Base, 3), 5);
}

#[test]
fn random_edit_sequences_stay_exact() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let words = ["a", "b", "c", "", "dd"];
    for _ in 0..60 {
        let base_lines: Vec<String> = (0..1 + next() % 12).map(|_| words[(next() % 5) as usize].to_string()).collect();
        let base = joined(&base_lines);
        let mut current = base_lines.clone();
        let limits = DiffLimits { window_lines: 1 + (next() % 30) as usize, ..DiffLimits::default() };
        let mut state = DiffState::compute(&base, &joined(&current), limits);
        for _ in 0..25 {
            let start = (next() % (current.len() as u64 + 1)) as usize;
            let mut end = start + (next() % 3) as usize;
            end = end.min(current.len());
            // A document always keeps at least one line.
            let count = (next() % 3) as usize + usize::from(end - start == current.len());
            let replacement: Vec<&str> = (0..count).map(|_| words[(next() % 5) as usize]).collect();
            let change = edit(&mut current, start, end, &replacement);
            let new = joined(&current);
            state.apply_edit(&base, &new, change);
            assert!(state.reproduces(&base, &new), "{base_lines:?} -> {current:?}: {:?}", hunks(&state));
            let all: Vec<_> = state.hunks().collect();
            for pair in all.windows(2) {
                assert!(pair[0].new.end < pair[1].new.start || pair[0].base.end < pair[1].base.start,
                    "hunks are separated by equal lines: {:?}", hunks(&state));
            }
        }
    }
}

#[test]
fn an_edit_moves_lines_boundaries_and_ranges_outside_the_lines_it_replaces() {
    // Lines 3..5 become 3..8.
    let edit = LineEdit { start: 3, old_end: 5, new_end: 8 };
    assert_eq!([2, 3, 4, 5].map(|line| edit.map_line(line)), [Some(2), None, None, Some(8)]);
    assert_eq!([3, 4, 5].map(|line| edit.map_boundary(line)), [Some(3), None, Some(8)]);
    assert_eq!(edit.map_range(1..3), Some(1..3));
    assert_eq!(edit.map_range(5..7), Some(8..10));
    assert_eq!(edit.map_range(2..4), None);
    assert_eq!(edit.map_surviving(1..7).collect::<Vec<_>>(), [1..3, 8..10]);
    assert_eq!(edit.map_surviving(3..5).count(), 0);
    // An insertion's boundary moves with the following line.
    let insertion = LineEdit { start: 3, old_end: 3, new_end: 5 };
    assert_eq!(insertion.map_boundary(3), Some(5));
    assert_eq!(insertion.map_range(1..3), Some(1..3));
    assert_eq!(insertion.map_range(2..4), None);
    assert!(edit.fits(10, 13));
    assert!(!edit.fits(10, 12));
    assert!(!edit.fits(4, 7));
}
