//! Portable list containers derived from paragraph identities. Source adapters
//! supply the structure; WYSIWYG labels are non-text layout decorations.
use super::{Block, BlockKind, ContainerIdentity, ContainerKind, FormattedDocument, ListStyle, Revision};
use std::collections::HashMap;
use std::ops::Range;

/// A list container follows its first item identity through ordinary edits and
/// splits. When that item is deleted, surviving items retain their own stable
/// identities and the container is recovered through those item identities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ListIdentity(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListItemNode {
    pub paragraph_id: u64,
    pub paragraph_ids: Vec<u64>,
    pub ordinal: u64,
    /// An empty body-start boundary for layout decorations. Decorations never
    /// contribute editable characters.
    pub marker_range: Range<usize>,
    pub marker_is_synthetic: bool,
    pub child_lists: Vec<ListIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListNode {
    pub id: ListIdentity,
    pub level: u8,
    pub style: ListStyle,
    pub start: u64,
    pub parent_item: Option<u64>,
    pub items: Vec<ListItemNode>,
}

/// Every range in this structural query belongs to this exact projection.
/// Item and list identities, rather than ranges, are retained across edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListStructure {
    pub revision: Revision,
    pub lists: Vec<ListNode>,
}

/// The lists touching a region and the blocks they are built from: the run
/// of list-owned blocks around it, between the nearest blocks outside every
/// list. A list never continues across such a block, so these lists are
/// exactly those the complete structure contains there.
#[derive(Clone, Debug)]
pub(crate) struct ListWindow {
    pub(crate) structure: ListStructure,
    pub(crate) blocks: Vec<Block>,
}

impl Block {
    /// Separator paragraphs have no marker, but remain leaves of their parsed
    /// list item. Structural traversal must not mistake them for a list break.
    pub(super) fn list_level(&self) -> Option<u8> {
        if let BlockKind::ListItem { level, .. } = self.kind { return Some(level); }
        self.list_item_owner()?;
        self.containers.iter().filter(|member|
            matches!(member.container.kind, ContainerKind::List { .. }))
            .count().checked_sub(1).map(|level| level.min(u8::MAX as usize) as u8)
    }

    pub(super) fn list_item_owner(&self) -> Option<ContainerIdentity> {
        self.containers.iter().rev()
            .find(|member| matches!(member.container.kind, ContainerKind::ListItem | ContainerKind::List { .. }))
            .filter(|member| member.container.kind == ContainerKind::ListItem)
            .map(|member| member.container.id)
    }

    /// List-owned separator paragraphs have no item to edit, but sibling
    /// traversal must cross them at their containing list's depth.
    pub(super) fn list_navigation_level(&self) -> Option<u8> {
        self.list_level().or_else(|| {
            if self.kind != BlockKind::Paragraph || !self.range.is_empty() {
                return None;
            }
            self.containers.iter().filter(|member|
                matches!(member.container.kind, ContainerKind::List { .. }))
                .count().checked_sub(1).map(|level| level.min(u8::MAX as usize) as u8)
        })
    }
}

impl FormattedDocument {
    /// The label is a short prefix even when its paragraph spans megabytes.
    pub(crate) fn list_marker_range_for_block(&self, block: &super::Block) -> Option<Range<usize>> {
        let BlockKind::ListItem {
            ordered,
            ordinal,
            item_start: true,
            marker_is_decoration,
            ..
        } = block.kind
        else {
            return None;
        };
        if marker_is_decoration {
            return Some(block.range.start..block.range.start);
        }
        let mut end = (block.range.start + 128).min(block.range.end);
        while end > block.range.start && !self.text_tree().is_char_boundary(end).ok()? {
            end -= 1;
        }
        let text = self.text_tree().slice(block.range.start..end).ok()?;
        let prefix = super::markdown_blocks::marker_prefix_length(&text);
        let canonical = if ordered {
            format!("{ordinal}. ")
        } else {
            "• ".to_owned()
        };
        let length = if let Some(prefix) = prefix {
            prefix
        } else if text.starts_with(&canonical) {
            canonical.len()
        } else {
            return None;
        };
        Some(block.range.start..(block.range.start + length).min(block.range.end))
    }

    /// Enumerate list containers and their item/child relationships. Paragraph
    /// styles remain assigned to the paragraph inside each item.
    pub fn list_structure(&self) -> ListStructure {
        self.list_structure_of(self.blocks())
    }

    /// Lists touching `range`, built in `O(log n + k)` from the surrounding run
    /// of list-owned blocks without copying the complete block store.
    pub(crate) fn list_window(&self, range: &Range<usize>) -> ListWindow {
        let store = self.list_indentation_blocks();
        let count = store.len();
        if count == 0 {
            return ListWindow { structure: ListStructure { revision: self.revision(), lists: Vec::new() }, blocks: Vec::new() };
        }
        // The block containing `range.start` (or the one before it), through
        // every block starting at or before `range.end`.
        let first = store.partition_point_start(range.start).saturating_sub(1);
        let last = store.partition_point_start(range.end.saturating_add(1)).max(first + 1).min(count);
        // Navigation key zero marks a block outside every list.
        let start = store.find_navigation(0..first, true, |summary| summary.minimum == 0).map_or(0, |index| index + 1);
        let end = store.find_navigation(last..count, false, |summary| summary.minimum == 0).unwrap_or(count);
        let blocks = store.get_range(&(start..end)).unwrap_or_default();
        super::work_statistics::record(|stats| stats.list_capability_blocks_visited += blocks.len());
        ListWindow { structure: self.list_structure_of(&blocks), blocks }
    }

    fn list_structure_of<'a>(&self, blocks: impl IntoIterator<Item = &'a Block>) -> ListStructure {
        let mut lists: Vec<ListNode> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut items_by_owner: HashMap<ContainerIdentity, (usize, usize)> = HashMap::new();
        let mut lists_by_owner: HashMap<ContainerIdentity, usize> = HashMap::new();
        for block in blocks {
            let BlockKind::ListItem {
                ordered,
                ordinal,
                level,
                container_start,
                item_start,
                ..
            } = block.kind
            else {
                // An empty item can leave separator paragraphs owned by the
                // list itself, without an item owner. They do not split the
                // list or become additional paragraphs of either sibling.
                if let Some(owner) = block.containers.iter().rev().find(|member|
                    matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem))
                    .filter(|member| matches!(member.container.kind, ContainerKind::List { .. }))
                {
                    if let Some(&list) = lists_by_owner.get(&owner.container.id) {
                        if let Some(depth) = stack.iter().position(|index| *index == list) {
                            stack.truncate(depth + 1);
                            continue;
                        }
                    }
                    stack.clear();
                    continue;
                }
                if let Some(&(list, item)) = block.list_item_owner()
                    .and_then(|owner| items_by_owner.get(&owner))
                {
                    if let Some(depth) = stack.iter().position(|index| *index == list) {
                        stack.truncate(depth + 1);
                        lists[list].items[item].paragraph_ids.push(block.id);
                        continue;
                    }
                }
                stack.clear();
                continue;
            };
            let style = if ordered {
                ListStyle::Numbered
            } else {
                ListStyle::Bullet
            };
            while stack
                .last()
                .is_some_and(|index| lists[*index].level > level)
            {
                stack.pop();
            }
            let current = stack.last().copied().filter(|index| {
                let list = &lists[*index];
                !container_start
                    && list.level == level
                    && list.style == style
                    && (!item_start
                        || !ordered
                        || list
                            .items
                            .last()
                            .is_some_and(|item| item.ordinal.saturating_add(1) == ordinal))
            });
            let index = if let Some(index) = current {
                index
            } else {
                if stack
                    .last()
                    .is_some_and(|index| lists[*index].level == level)
                {
                    stack.pop();
                }
                let parent = stack.last().copied();
                let parent_item = parent
                    .and_then(|index| lists[index].items.last().map(|item| item.paragraph_id));
                let id = ListIdentity(block.id);
                if let Some(item) = parent.and_then(|index| lists[index].items.last_mut()) {
                    item.child_lists.push(id);
                }
                let index = lists.len();
                lists.push(ListNode {
                    id,
                    level,
                    style,
                    start: ordinal,
                    parent_item,
                    items: Vec::new(),
                });
                stack.push(index);
                index
            };
            if let Some(owner) = block.containers.iter().rev().find(|member|
                matches!(member.container.kind, ContainerKind::List { .. }))
            {
                lists_by_owner.insert(owner.container.id, index);
            }
            if !item_start {
                if let Some(item) = lists[index].items.last_mut() {
                    item.paragraph_ids.push(block.id);
                    continue;
                }
            }
            let marker_range = self
                .list_marker_range_for_block(block)
                .unwrap_or(block.range.start..block.range.start);
            let marker_is_synthetic = matches!(
                block.kind,
                BlockKind::ListItem {
                    marker_is_decoration: true,
                    ..
                }
            ) || self
                .provenance_for_region(&marker_range)
                .iter()
                .any(|span| span.is_synthetic());
            if let Some(owner) = block.list_item_owner() {
                items_by_owner.insert(owner, (index, lists[index].items.len()));
            }
            lists[index].items.push(ListItemNode {
                paragraph_id: block.id,
                paragraph_ids: vec![block.id],
                ordinal,
                marker_range,
                marker_is_synthetic,
                child_lists: Vec::new(),
            });
        }
        ListStructure {
            revision: self.revision(),
            lists,
        }
    }
}
