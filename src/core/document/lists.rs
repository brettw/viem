//! Portable list containers derived from paragraph identities. Source adapters
//! supply the structure; generated labels are explicit synthetic content.
use super::{BlockKind, FormattedDocument, ListStyle, Revision};
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

impl FormattedDocument {
    /// Enumerate list containers and their item/child relationships. Paragraph
    /// styles remain assigned to the paragraph inside each item.
    pub fn list_structure(&self) -> ListStructure {
        let mut lists: Vec<ListNode> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for block in self.blocks() {
            let BlockKind::ListItem {
                ordered,
                ordinal,
                level,
                container_start,
                item_start,
            } = block.kind
            else {
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
            if !item_start {
                if let Some(item) = lists[index].items.last_mut() {
                    item.paragraph_ids.push(block.id);
                    continue;
                }
            }
            let text = self
                .text_tree()
                .slice(block.range.clone())
                .unwrap_or_default();
            let (source_prefix, kind) =
                super::projection::markdown_block_prefix(&text, 0, text.len());
            let length = if matches!(kind, BlockKind::ListItem { .. }) {
                source_prefix
            } else if ordered {
                format!("{ordinal}. ").len()
            } else {
                "• ".len()
            };
            let marker_range = block.range.start..(block.range.start + length).min(block.range.end);
            let marker_is_synthetic = self
                .provenance_for_region(&marker_range)
                .iter()
                .any(|span| span.is_synthetic());
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
