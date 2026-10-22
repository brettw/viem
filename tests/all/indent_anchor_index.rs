use viem_core::layout::{IndentAnchorIndex, IndentAnchorWork, LineIndentation};

fn index(columns: &[usize]) -> IndentAnchorIndex {
    IndentAnchorIndex::from_lines(columns.iter().copied().map(LineIndentation::new))
}

fn oracle_anchor(lines: &[LineIndentation], line: usize) -> Option<usize> {
    (0..line).rev().find(|&candidate| {
        lines[candidate].anchor_eligible && lines[candidate].columns <= lines[line].columns
    })
}

fn verify(tree: &IndentAnchorIndex, lines: &[LineIndentation]) {
    assert_eq!(tree.len(), lines.len());
    for line in 0..lines.len() {
        assert_eq!(tree.line(line), Some(lines[line]), "line {line}");
        assert_eq!(
            tree.anchor(line),
            oracle_anchor(lines, line),
            "anchor of {line}"
        );
        let children: Vec<_> = (line + 1..lines.len())
            .filter(|&child| oracle_anchor(lines, child) == Some(line))
            .collect();
        assert_eq!(
            tree.direct_dependents(line).collect::<Vec<_>>(),
            children,
            "children of {line}"
        );
        let range = tree.dependent_range(line).unwrap();
        for child in line + 1..lines.len() {
            let mut ancestor = oracle_anchor(lines, child);
            let mut dependent = false;
            while let Some(parent) = ancestor {
                if parent == line {
                    dependent = true;
                    break;
                }
                if parent < line {
                    break;
                }
                ancestor = oracle_anchor(lines, parent);
            }
            if lines.iter().all(|line| line.anchor_eligible) {
                assert_eq!(
                    range.contains(&child),
                    dependent,
                    "descendant {child} of {line}"
                );
            } else if dependent {
                assert!(range.contains(&child));
            }
        }
    }
    assert_eq!(tree.line(lines.len()), None);
    assert_eq!(tree.anchor(lines.len()), None);
    assert_eq!(tree.dependent_range(lines.len()), None);
    assert_eq!(tree.preceding_with_max(lines.len() + 1, usize::MAX), None);
    assert_eq!(tree.following_with_max(lines.len() + 1, usize::MAX), None);
}

#[test]
fn anchor_and_dependents_match_nested_equal_and_distant_lines() {
    for columns in [
        vec![],
        vec![0],
        vec![0, 0, 0, 0],
        vec![8, 6, 4, 2, 0],
        vec![0, 2, 6, 8, 6, 4, 6, 2, 4, 0],
        vec![3, 9, 8, 7, 6, 5, 4, 3],
        vec![usize::MAX, usize::MAX, 0, usize::MAX],
    ] {
        let lines: Vec<_> = columns.into_iter().map(LineIndentation::new).collect();
        verify(&IndentAnchorIndex::from_lines(lines.clone()), &lines);
    }
    let tree = index(&[2, 6, 4, 7, 6, 2, 0]);
    assert_eq!(tree.direct_dependents(0).collect::<Vec<_>>(), [1, 2, 5]);
    assert_eq!(tree.dependent_range(0), Some(1..6));
}

#[test]
fn future_anchor_eligibility_can_skip_lines_without_changing_ordinals() {
    let mut lines: Vec<_> = [0, 9, 1, 8, 7, 3, 5, 0, 4, 3]
        .into_iter()
        .map(LineIndentation::new)
        .collect();
    for skipped in [1, 2, 4, 7] {
        lines[skipped].anchor_eligible = false;
    }
    verify(&IndentAnchorIndex::from_lines(lines.clone()), &lines);
    for line in &mut lines {
        line.anchor_eligible = false;
    }
    verify(&IndentAnchorIndex::from_lines(lines.clone()), &lines);
}

#[test]
fn snapshots_and_invalid_splices_preserve_old_dependencies() {
    let mut tree = index(&[0, 2, 4, 8, 2, 6, 0]);
    let snapshot = tree.clone();
    tree.splice(1..2, [LineIndentation::new(9)]).unwrap();
    assert_eq!(snapshot.anchor(2), Some(1));
    assert_eq!(tree.anchor(2), Some(0));
    assert!(tree.splice(3..2, [LineIndentation::new(0)]).is_err());
    assert!(tree.splice(0..100, [LineIndentation::new(0)]).is_err());
    assert_eq!(tree.anchor(2), Some(0));
    tree.splice(0..tree.len(), []).unwrap();
    assert!(tree.is_empty());
    tree.splice(0..0, [LineIndentation::new(10), LineIndentation::new(12)])
        .unwrap();
    assert_eq!(tree.anchor(1), Some(0));
    assert_eq!(snapshot.len(), 7);
}

#[test]
fn exhaustive_small_sequences_cover_anchor_and_dependency_boundaries() {
    // Every indentation/eligibility combination of four lines. This catches
    // equal-indent ties and excluded lines both above and below the threshold.
    for mut encoded in 0..6usize.pow(4) {
        let mut lines = Vec::new();
        for _ in 0..4 {
            let digit = encoded % 6;
            encoded /= 6;
            lines.push(LineIndentation {
                columns: digit / 2,
                anchor_eligible: digit % 2 == 0,
            });
        }
        verify(&IndentAnchorIndex::from_lines(lines.clone()), &lines);
    }
}

fn random(state: &mut u64, limit: usize) -> usize {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as usize) % limit
}

#[test]
fn randomized_splices_match_linear_oracle_and_keep_snapshots_valid() {
    let mut seed = 0xa817_a50d_27b4_f130;
    let mut lines = Vec::new();
    let mut tree = IndentAnchorIndex::default();
    for step in 0..2_000 {
        let saved_tree = tree.clone();
        let saved_lines = lines.clone();
        let start = random(&mut seed, lines.len() + 1);
        let removed = random(&mut seed, (lines.len() - start).min(25) + 1);
        let count = if lines.len() > 180 {
            0
        } else {
            random(&mut seed, 16)
        };
        let replacement: Vec<_> = (0..count)
            .map(|_| LineIndentation {
                columns: random(&mut seed, 20),
                anchor_eligible: random(&mut seed, 5) != 0,
            })
            .collect();
        tree.splice(start..start + removed, replacement.clone())
            .unwrap();
        lines.splice(start..start + removed, replacement);
        for line in 0..lines.len() {
            assert_eq!(tree.line(line), Some(lines[line]));
            assert_eq!(
                tree.anchor(line),
                oracle_anchor(&lines, line),
                "step {step}, line {line}"
            );
            for threshold in [0, 3, 19, usize::MAX] {
                assert_eq!(
                    tree.preceding_with_max(line, threshold),
                    (0..line)
                        .rev()
                        .find(|&i| lines[i].anchor_eligible && lines[i].columns <= threshold)
                );
                assert_eq!(
                    tree.following_with_max(line, threshold),
                    (line..lines.len())
                        .find(|&i| lines[i].anchor_eligible && lines[i].columns <= threshold)
                );
            }
        }
        if step % 100 == 0 {
            verify(&tree, &lines);
            verify(&saved_tree, &saved_lines);
        }
    }
}

#[test]
fn million_line_queries_and_parent_changes_have_structural_bounds() {
    let count = 1_000_000;
    let mut tree = IndentAnchorIndex::from_lines(
        (0..count).map(|line| LineIndentation::new(if line == 0 { 0 } else { 8 })),
    );
    let statistics = tree.statistics();
    assert!(statistics.block_count <= count / 128 + 1);
    assert!(statistics.tree_height <= 16);
    assert!(statistics.payload_bytes < 11 * count, "{statistics:?}");

    // A far dedent finds the first line without reading the intervening text.
    tree.splice(count - 1..count, [LineIndentation::new(2)])
        .unwrap();
    let mut work = IndentAnchorWork::default();
    assert_eq!(tree.anchor_with_work(count - 1, &mut work), Some(0));
    assert!(work.nodes_visited < 100, "{work:?}");
    assert!(work.lines_examined <= 257, "{work:?}");

    // Changing the first line changes the implicit ancestry of every other
    // line. No dependent entry is rewritten during publication.
    let original = tree.clone();
    let mut work = IndentAnchorWork::default();
    tree.splice_with_work(0..1, [LineIndentation::new(9)], &mut work)
        .unwrap();
    assert_eq!(tree.anchor(count - 1), None);
    assert_eq!(original.anchor(count - 1), Some(0));
    assert!(work.nodes_visited < 300, "{work:?}");
    assert!(work.nodes_created < 150, "{work:?}");
    assert!(work.lines_copied <= 1_024, "{work:?}");
    assert_eq!(original.dependent_range(0), Some(1..count));
}

#[test]
fn repeated_insert_delete_and_indent_changes_do_not_fragment_blocks() {
    let count = 32_768;
    let mut tree =
        IndentAnchorIndex::from_lines((0..count).map(|line| LineIndentation::new(line % 17)));
    let snapshot = tree.clone();
    let mut seed = 0x932e_186c_74af_52d9;
    for step in 0..10_000 {
        let at = random(&mut seed, count);
        let mut work = IndentAnchorWork::default();
        tree.splice_with_work(at..at + 1, [LineIndentation::new(step % 21)], &mut work)
            .unwrap();
        assert!(work.nodes_visited < 300, "{work:?}");
        assert!(work.nodes_created < 150, "{work:?}");
        assert!(work.lines_copied <= 1_024, "{work:?}");
        if step % 7 == 0 {
            tree.splice(at..at, [LineIndentation::new(99)]).unwrap();
            tree.splice(at..at + 1, []).unwrap();
        }
    }
    let statistics = tree.statistics();
    assert_eq!(statistics.line_count, count);
    assert!(statistics.block_count <= count / 64 + 2, "{statistics:?}");
    assert!(statistics.tree_height <= 15, "{statistics:?}");
    assert_eq!(snapshot.line(123), Some(LineIndentation::new(123 % 17)));
}

#[test]
fn append_truncate_and_reinsert_across_block_edges() {
    let mut tree = IndentAnchorIndex::default();
    let mut lines = Vec::new();
    for count in [1, 127, 128, 129, 3, 256, 11] {
        let replacement: Vec<_> = (0..count).map(|i| LineIndentation::new(i % 13)).collect();
        tree.splice(tree.len()..tree.len(), replacement.clone())
            .unwrap();
        lines.extend(replacement);
        for line in 0..lines.len() {
            assert_eq!(tree.anchor(line), oracle_anchor(&lines, line));
        }
    }
    for count in [1, 128, 3, 257, 100] {
        let end = tree.len() - count;
        tree.splice(end..tree.len(), []).unwrap();
        lines.truncate(end);
        for line in 0..lines.len() {
            assert_eq!(tree.anchor(line), oracle_anchor(&lines, line));
        }
    }
}
