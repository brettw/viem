//! Chronological navigation across retained undo branches. Queries resolve a
//! target first; the ordinary atomic history transaction installs its snapshot.
use super::{History, HistoryLocation};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryTimeAmount {
    Changes(u64),
    Seconds(u64),
    Writes(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryTimelineEntry {
    pub location: HistoryLocation,
    /// Number of parent edges from the currently retained root.
    pub changes: usize,
    /// Unix timestamp in seconds at the beginning of the undo unit.
    pub timestamp: u64,
    /// Most recent successful write of this exact history state.
    pub saved_write: Option<u64>,
}

pub(super) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl<T, M> History<T, M> {
    pub(super) fn next_timestamp(&self) -> u64 {
        // Wall-clock adjustments must not reorder the history timeline.
        now_seconds().max(self.nodes.last().map_or(0, |node| node.timestamp))
    }

    pub(super) fn annotate_saved(&mut self, index: usize) {
        self.checkpoint_node(index);
        self.nodes[index].saved_write = Some(self.next_write_number);
        self.next_write_number = self.next_write_number.saturating_add(1);
    }

    pub(crate) fn timeline_leaves(&self) -> Vec<HistoryTimelineEntry> {
        let mut entries = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.children.is_empty() && node.parent.is_some())
            .map(|(index, node)| HistoryTimelineEntry {
                location: self.location(index),
                changes: self.root_path(index).len().saturating_sub(1),
                timestamp: node.timestamp,
                saved_write: node.saved_write,
            })
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.location.change);
        entries
    }

    pub(crate) fn time_target(&self, later: bool, amount: HistoryTimeAmount) -> HistoryLocation {
        let current = &self.nodes[self.current];
        let mut ordered = (0..self.nodes.len()).collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|&index| self.nodes[index].change);
        let first = ordered[0];
        let last = *ordered.last().expect("history always has a root");
        let boundary = if later { last } else { first };
        let target = match amount {
            HistoryTimeAmount::Changes(0)
            | HistoryTimeAmount::Seconds(0)
            | HistoryTimeAmount::Writes(0) => self.current,
            HistoryTimeAmount::Changes(count) => {
                let desired = if later {
                    current.change.0.saturating_add(count)
                } else {
                    current.change.0.saturating_sub(count)
                };
                // Numbered changes remain meaningful even after pruning makes
                // holes in the timeline. Pick the closest retained state in
                // the requested direction rather than interpreting a number
                // as an index into the current branch.
                ordered
                    .iter()
                    .copied()
                    .filter(|&index| {
                        let change = self.nodes[index].change.0;
                        if later {
                            change > current.change.0 && change <= desired
                        } else {
                            change < current.change.0 && change >= desired
                        }
                    })
                    .min_by_key(|&index| self.nodes[index].change.0.abs_diff(desired))
                    .unwrap_or_else(|| {
                        if later {
                            ordered
                                .iter()
                                .copied()
                                .find(|&i| self.nodes[i].change > current.change)
                        } else {
                            ordered
                                .iter()
                                .rev()
                                .copied()
                                .find(|&i| self.nodes[i].change < current.change)
                        }
                        .unwrap_or(boundary)
                    })
            }
            HistoryTimeAmount::Seconds(seconds) => {
                let desired = if later {
                    current.timestamp.saturating_add(seconds)
                } else {
                    current.timestamp.saturating_sub(seconds)
                };
                if desired < self.nodes[first].timestamp {
                    first
                } else if desired > self.nodes[last].timestamp {
                    last
                } else {
                    ordered
                        .iter()
                        .copied()
                        .filter(|&index| {
                            if later {
                                self.nodes[index].change > current.change
                            } else {
                                self.nodes[index].change < current.change
                            }
                        })
                        .min_by_key(|&index| {
                            let node = &self.nodes[index];
                            // Equal timestamps are ordered by change number. Going
                            // earlier selects the oldest and later the newest.
                            (
                                node.timestamp.abs_diff(desired),
                                if later {
                                    u64::MAX - node.change.0
                                } else {
                                    node.change.0
                                },
                            )
                        })
                        .unwrap_or(boundary)
                }
            }
            HistoryTimeAmount::Writes(count) => {
                let reference = current.saved_write.unwrap_or(current.creation_write);
                let desired = if later {
                    reference.saturating_add(count)
                } else {
                    reference.saturating_sub(count - u64::from(current.saved_write.is_none()))
                };
                let saved = ordered
                    .iter()
                    .copied()
                    .filter(|&index| {
                        self.nodes[index].saved_write.is_some_and(|write| {
                            if later {
                                write > reference && write <= desired
                            } else {
                                write <= desired
                                    && (current.saved_write.is_none() || write < reference)
                            }
                        })
                    })
                    .min_by_key(|&index| self.nodes[index].saved_write.unwrap().abs_diff(desired));
                let last_write = self
                    .nodes
                    .iter()
                    .filter_map(|node| node.saved_write)
                    .max()
                    .unwrap_or(0);
                if later && desired > last_write {
                    last
                } else {
                    saved.unwrap_or(boundary)
                }
            }
        };
        self.location(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{HistoryChangeNumber, HistoryRetentionPolicy};

    fn move_time(
        history: &mut History<&'static str>,
        later: bool,
        amount: HistoryTimeAmount,
    ) -> u64 {
        let target = history.time_target(later, amount);
        history.select_node(target.node).unwrap();
        target.change.as_u64()
    }

    #[test]
    fn change_counts_walk_all_branches_and_clamp_at_retained_ends() {
        let mut h = History::new("root");
        h.commit("a", false);
        h.commit("ab", false);
        h.undo_exact().unwrap();
        h.commit("ac", false);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Changes(1)), 2);
        assert_eq!(**h.current(), "ab");
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Changes(1)), 1);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Changes(2)), 3);
        assert_eq!(
            move_time(&mut h, false, HistoryTimeAmount::Changes(u64::MAX)),
            0
        );
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Changes(1)), 0);
        assert_eq!(
            move_time(&mut h, true, HistoryTimeAmount::Changes(u64::MAX)),
            3
        );
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Changes(1)), 3);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Changes(0)), 3);
    }

    #[test]
    fn timestamps_resolve_nearest_states_with_directional_ties_without_sleeping() {
        let mut h = History::new("root");
        for text in ["a", "b", "c", "d"] {
            h.commit(text, false);
        }
        for (node, time) in h.nodes.iter_mut().zip([100, 110, 120, 120, 130]) {
            node.timestamp = time;
        }
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Seconds(10)), 2);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Seconds(1)), 3);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Seconds(1)), 2);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Seconds(100)), 0);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Seconds(100)), 4);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Seconds(0)), 4);
    }

    #[test]
    fn writes_include_latest_unsaved_state_and_saved_states_across_branches() {
        let mut h = History::new("root");
        h.commit("a", false);
        h.mark_saved();
        h.commit("b", false);
        h.mark_saved();
        h.commit("c", false);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Writes(1)), 2);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Writes(1)), 1);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Writes(1)), 0);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Writes(1)), 1);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Writes(1)), 2);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Writes(1)), 3);
        h.select_change(HistoryChangeNumber::from_u64(1)).unwrap();
        h.commit("branch", false);
        h.mark_saved();
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Writes(1)), 2);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Writes(1)), 4);
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Writes(99)), 0);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Writes(99)), 4);
    }

    #[test]
    fn leaf_listing_reports_depth_save_numbers_and_grouping() {
        let mut h = History::new("root");
        assert!(h.timeline_leaves().is_empty());
        h.commit("a", true);
        let started = h.nodes[h.current].timestamp;
        h.commit("ab", true);
        h.end_group();
        h.commit("abc", false);
        h.mark_saved();
        h.select_change(HistoryChangeNumber::INITIAL).unwrap();
        h.commit("x", false);
        let leaves = h.timeline_leaves();
        assert_eq!(
            leaves
                .iter()
                .map(|entry| (
                    entry.location.change.as_u64(),
                    entry.changes,
                    entry.saved_write
                ))
                .collect::<Vec<_>>(),
            [(2, 2, Some(1)), (3, 1, None)]
        );
        assert_eq!(h.nodes[1].timestamp, started);
        assert!(leaves.iter().all(|entry| entry.timestamp >= started));
    }

    #[test]
    fn save_annotations_rollback_with_command_checkpoint() {
        let mut h = History::new("root");
        h.commit("a", false);
        h.mark_saved();
        let before = h.timeline_leaves();
        h.begin_command_checkpoint();
        h.mark_saved();
        h.commit("b", false);
        h.mark_saved();
        h.rollback_command_checkpoint();
        assert_eq!(h.timeline_leaves(), before);
        h.commit("c", false);
        h.mark_saved();
        assert_eq!(h.timeline_leaves()[0].saved_write, Some(2));
    }

    #[test]
    fn pruned_states_are_never_resurrected_and_leaf_depth_uses_retained_root() {
        let mut h = History::new("root");
        for text in ["a", "b", "c", "d"] {
            h.commit(text, false);
        }
        h.set_retention_policy(HistoryRetentionPolicy::new(3, usize::MAX));
        assert_eq!(move_time(&mut h, false, HistoryTimeAmount::Changes(100)), 2);
        assert_eq!(move_time(&mut h, true, HistoryTimeAmount::Changes(100)), 4);
        assert_eq!(h.timeline_leaves()[0].changes, 2);
    }
}
