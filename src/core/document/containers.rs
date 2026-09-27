//! Explicit structural owners around paragraph leaves. Paths live with the
//! persistent paragraph records, so a local splice retains all untouched owners.
use super::{Block, BlockDirectFormatting, FormattedDocument, Revision, StyleId};
use std::{collections::{BTreeMap, BTreeSet}, ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContainerIdentity { pub anchor: u64, pub slot: u32 }

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContainerKind { Quote, CodeBlock, List { ordered: bool }, ListItem }

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerAttributes {
    pub id: ContainerIdentity,
    pub kind: ContainerKind,
    pub style: StyleId,
    pub direct_formatting: Option<Arc<BlockDirectFormatting>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerMembership {
    pub container: Arc<ContainerAttributes>,
    pub starts_here: bool,
    pub ends_here: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerNode {
    pub attributes: Arc<ContainerAttributes>,
    pub parent: Option<ContainerIdentity>,
    pub paragraph_ids: Vec<u64>,
    pub children: Vec<ContainerIdentity>,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerStructure { pub revision: Revision, pub containers: Vec<ContainerNode> }

impl ContainerKind {
    pub fn style_role(self) -> super::BlockRole { match self {
        Self::Quote => super::BlockRole::Quote, Self::CodeBlock => super::BlockRole::CodeBlock,
        Self::List { .. } => super::BlockRole::List, Self::ListItem => super::BlockRole::ListItem,
    } }
    pub(super) fn default_style(self) -> StyleId { match self {
        Self::Quote => "Block quote", Self::CodeBlock => "Code Block",
        Self::List { ordered: false } => "Bulleted List", Self::List { ordered: true } => "Numbered List",
        Self::ListItem => "List item",
    }.into() }
}

/// A parser-local source scope. Its byte coordinates never survive publication.
#[derive(Clone, Debug)]
pub(super) struct SourceContainer {
    pub range: Range<usize>,
    pub owns_empty_end: bool,
    pub kind: ContainerKind,
    pub style: StyleId,
    pub direct_formatting: Option<Arc<BlockDirectFormatting>>,
}
impl SourceContainer {
    pub fn new(range: Range<usize>, kind: ContainerKind) -> Self {
        Self { range, owns_empty_end: false, kind, style: kind.default_style(), direct_formatting: None }
    }
}

/// Reconcile each owner through any surviving paragraph, including when its
/// former first paragraph was deleted. A split may retain the old identity only
/// once. New identities use an independent monotonic owner identity: a
/// paragraph can change container kind while an earlier owner survives in its
/// siblings, so paragraph identity plus depth would incorrectly alias them.
pub(super) fn reconcile(blocks: &mut [Block], previous: &[Block], regional: bool) {
    let old: BTreeMap<_, _> = previous.iter().map(|block| (block.id, block)).collect();
    let mut identities = BTreeMap::new();
    let mut used = BTreeSet::new();
    for block in blocks.iter() {
        let Some(old) = old.get(&block.id) else { continue; };
        for (depth, membership) in block.containers.iter().enumerate() {
            let key = membership.container.id;
            if identities.contains_key(&key) { continue; }
            if let Some(prior) = old.containers.get(depth).filter(|prior| prior.container.kind == membership.container.kind) {
                if used.insert(prior.container.id) { identities.insert(key, prior.container.id); }
            }
        }
    }
    let mut old_edges = BTreeMap::new();
    for block in previous { for member in block.containers.iter() {
        let edge = old_edges.entry(member.container.id).or_insert((false, false));
        edge.0 |= member.starts_here; edge.1 |= member.ends_here;
    } }
    let mut new_edges = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() { for member in block.containers.iter() {
        new_edges.entry(member.container.id).and_modify(|edge: &mut (usize, usize)| edge.1 = index).or_insert((index, index));
    } }
    let count = blocks.len();
    let mut attributes = BTreeMap::new();
    for block in blocks.iter_mut().filter(|block| !block.containers.is_empty()) {
        let path = block.containers.iter().enumerate().map(|(depth, member)| {
            let key = member.container.id;
            let id = *identities.entry(key).or_insert_with(|| {
                static NEXT_OWNER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                ContainerIdentity { anchor: NEXT_OWNER.fetch_add(1, std::sync::atomic::Ordering::Relaxed), slot: depth as u32 }
            });
            let container = attributes.entry(key).or_insert_with(|| {
                let mut value = (*member.container).clone(); value.id = id; Arc::new(value)
            }).clone();
            let mut result = ContainerMembership { container, starts_here: member.starts_here, ends_here: member.ends_here };
            if regional {
                if let Some((old_start, old_end)) = old_edges.get(&id) {
                    // Fragment edges can clip an enclosing owner. Interior
                    // scope boundaries are real even when this edit moved a
                    // former edge onto a different surviving paragraph.
                    let (first, last) = new_edges[&key];
                    result.starts_here &= *old_start || first > 0;
                    result.ends_here &= *old_end || last + 1 < count;
                }
            }
            result
        }).collect::<Vec<_>>();
        block.containers = path.into();
    }
}

pub(super) fn provisional_identity() -> ContainerIdentity {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    ContainerIdentity { anchor: u64::MAX, slot: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u32 }
}

pub(super) fn mark_edges(blocks: &mut [Block]) {
    let mut edges = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        for member in block.containers.iter() {
            edges.entry(member.container.id).and_modify(|edge: &mut (usize, usize)| edge.1 = index).or_insert((index, index));
        }
    }
    for (index, block) in blocks.iter_mut().enumerate().filter(|(_, block)| !block.containers.is_empty()) {
        let path = block.containers.iter().map(|member| {
            let (first, last) = edges[&member.container.id];
            ContainerMembership { container: member.container.clone(), starts_here: first == index, ends_here: last == index }
        }).collect::<Vec<_>>();
        block.containers = path.into();
    }
}

/// RTF carries list membership on paragraph records rather than source tags.
/// Lift that semantic hierarchy into the same owner paths used by markup.
pub(super) fn install_list_paths(blocks: &mut [Block]) {
    struct Frame { level: u8, ordered: bool, ordinal: u64, list: Arc<ContainerAttributes>, item: Arc<ContainerAttributes> }
    fn owner(kind: ContainerKind) -> Arc<ContainerAttributes> {
        Arc::new(ContainerAttributes { id: provisional_identity(), kind, style: kind.default_style(), direct_formatting: None })
    }
    let mut stack: Vec<Frame> = Vec::new();
    for block in blocks.iter_mut() {
        let super::BlockKind::ListItem { ordered, ordinal, level, container_start, item_start, .. } = block.kind else { stack.clear(); continue; };
        while stack.last().is_some_and(|frame| frame.level > level) { stack.pop(); }
        let continues = stack.last().is_some_and(|frame| frame.level == level && frame.ordered == ordered
            && !container_start && (!item_start || !ordered || frame.ordinal.saturating_add(1) == ordinal));
        if !continues {
            if stack.last().is_some_and(|frame| frame.level == level) { stack.pop(); }
            stack.push(Frame { level, ordered, ordinal, list: owner(ContainerKind::List { ordered }), item: owner(ContainerKind::ListItem) });
        } else if item_start {
            let frame = stack.last_mut().unwrap(); frame.item = owner(ContainerKind::ListItem); frame.ordinal = ordinal;
        }
        block.containers = stack.iter().flat_map(|frame| [&frame.list, &frame.item]).map(|container|
            ContainerMembership { container: container.clone(), starts_here: false, ends_here: false }).collect::<Vec<_>>().into();
    }
    // RTF list-level indents are absolute paragraph coordinates. Preserve
    // those declarations without adding the generated list's default inset.
    let native_indents: BTreeSet<_> = blocks.iter().filter(|block|
        block.direct_paragraph.leading_indent.is_some() || block.direct_paragraph.trailing_indent.is_some())
        .flat_map(|block| block.containers.iter().filter(|member| matches!(member.container.kind, ContainerKind::List { .. }))
            .map(|member| member.container.id)).collect();
    let mut owners = BTreeMap::new();
    for block in blocks.iter_mut() {
        if !block.containers.iter().any(|member| native_indents.contains(&member.container.id)) { continue; }
        block.containers = block.containers.iter().map(|member| {
            let mut member = member.clone();
            if native_indents.contains(&member.container.id) {
                member.container = owners.entry(member.container.id).or_insert_with(|| {
                    let mut owner = (*member.container).clone();
                    owner.direct_formatting = BlockDirectFormatting::shared(super::BlockProperties {
                        padding_left: Some(0.), padding_right: Some(0.), ..Default::default()
                    }, super::CharacterProperties::default());
                    Arc::new(owner)
                }).clone();
            }
            member
        }).collect::<Vec<_>>().into();
    }
    mark_edges(blocks);
}

impl FormattedDocument {
    /// Materialize a structural tree for inspection. Layout uses each
    /// paragraph's bounded container path and does not call this full query.
    pub fn container_structure(&self) -> ContainerStructure {
        let mut nodes: Vec<ContainerNode> = Vec::new();
        let mut indices: BTreeMap<ContainerIdentity, usize> = BTreeMap::new();
        for block in self.blocks() {
            let mut parent = None;
            for member in block.containers.iter() {
                let id = member.container.id;
                let index = if let Some(index) = indices.get(&id).copied() { index } else {
                    let index = nodes.len();
                    if let Some(parent) = parent { if let Some(parent_index) = indices.get(&parent).copied() { nodes[parent_index].children.push(id); } }
                    nodes.push(ContainerNode { attributes: member.container.clone(), parent, paragraph_ids: Vec::new(), children: Vec::new(), range: block.range.clone() });
                    indices.insert(id, index); index
                };
                nodes[index].paragraph_ids.push(block.id);
                nodes[index].range.end = block.range.end;
                parent = Some(id);
            }
        }
        ContainerStructure { revision: self.revision(), containers: nodes }
    }
}

#[cfg(test)]
mod tests {
    use crate::document::*;

    #[test]
    fn deleting_a_configured_container_style_reassigns_flow_paths_and_picker_context() {
        let mut doc = Document::from_bytes(b"> body".to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
        let id = StyleId::from("Block quote");
        assert!(doc.projection().has_block_style_assignment(&id));
        doc.apply_style_request(StyleModelRequest::new(doc.id(), doc.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::DeleteBlock(id.clone()))))).unwrap();
        let projection = doc.projection();
        assert!(!projection.has_block_style_assignment(&id));
        let flow = projection.blocks_for_region(&(0..doc.text().len()));
        assert!(flow.iter().flat_map(|block| block.containers.iter()).all(|member| member.container.style != id));
        let at = doc.text().find("bo").unwrap();
        assert_eq!(projection.selected_named_styles(at..at, BoundaryAffinity::Downstream).paragraph, Some("Paragraph".into()));
        assert_eq!(doc.source_bytes(), b"> body");
    }
}
