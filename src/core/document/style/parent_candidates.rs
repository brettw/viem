//! Query the same local inheritance constraints used by style mutations.
use super::*;
impl StyleSheet {
    pub fn local_parent_candidates(
        &self,
        namespace: StyleNamespace,
        child: &StyleId,
    ) -> Vec<StyleId> {
        let metadata = match namespace {
            StyleNamespace::Block => self.block_style_metadata(child),
            StyleNamespace::Character => self.character_style_metadata(child),
        };
        if child.is_internal_in(namespace)
            || metadata.is_none_or(|m| m.origin != StyleDefinitionOrigin::GeneratedConfiguration)
            || namespace == StyleNamespace::Block && child == &self.base_paragraph
        {
            return Vec::new();
        }
        let parents: BTreeMap<_, _> = match namespace {
            StyleNamespace::Block => self
                .block_styles()
                .map(|s| (s.id.clone(), s.based_on.clone()))
                .collect(),
            StyleNamespace::Character => self
                .character_styles()
                .map(|s| (s.id.clone(), s.based_on.clone()))
                .collect(),
        };
        // Build the reverse dependency graph once; descendants cannot be parents.
        let mut children: BTreeMap<&StyleId, Vec<&StyleId>> = BTreeMap::new();
        for (id, parent) in &parents {
            if self.parent_variant(namespace, id).is_none() {
                if let Some(parent) = parent {
                    children.entry(parent).or_default().push(id);
                }
            }
        }
        let mut forbidden = BTreeSet::new();
        let mut queue = vec![child];
        while let Some(id) = queue.pop() {
            if forbidden.insert(id) {
                if let Some(next) = children.get(id) {
                    queue.extend(next);
                }
            }
        }
        parents
            .keys()
            .filter(|parent| !forbidden.contains(parent))
            .filter(|parent| match namespace {
                StyleNamespace::Character => true,
                StyleNamespace::Block => self.local_block_roles_compatible(
                    self.block_styles.get(child).unwrap().role,
                    self.block_styles.get(*parent).unwrap(),
                ),
            })
            .filter(|parent| {
                // A local link can close a cycle through a different theme variant.
                // The mutation validator follows those exact cross-variant edges.
                if self.variants.context.is_none() {
                    return true;
                }
                // Adding child -> parent is cyclic exactly when the parent's
                // existing chain reaches this child in this variant. Reuse the
                // mutation resolver without copying a stylesheet per option.
                self.variant_chain(namespace, parent).is_ok_and(|chain| {
                    !chain
                        .iter()
                        .any(|(sheet, id)| std::ptr::eq(*sheet, self) && id == child)
                })
            })
            .cloned()
            .collect()
    }
    pub(super) fn local_block_roles_compatible(
        &self,
        child: BlockRole,
        parent: &BlockStyle,
    ) -> bool {
        match (child, parent.role) {
            (BlockRole::Document, BlockRole::Document) => true,
            (BlockRole::Document, BlockRole::Paragraph) => parent.id == self.base_paragraph,
            (BlockRole::Paragraph, BlockRole::Paragraph) => true,
            (role, parent_role) if role.is_container() => {
                role == parent_role || parent.id == self.base_paragraph
            }
            _ => false,
        }
    }
}
