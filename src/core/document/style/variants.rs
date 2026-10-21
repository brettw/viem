//! Immutable cross-variant theme ancestry. Foreign edges always name the same
//! displayed definition in the same namespace; effective values are never saved.
use super::*;
use serde_json::Value;
use std::sync::{Arc, OnceLock};

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVariant {
    Text,
    Markdown,
    Code,
}
impl ThemeVariant {
    pub fn token(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Code => "code",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "markdown" => Some(Self::Markdown),
            "code" => Some(Self::Code),
            _ => None,
        }
    }
    pub fn abi_value(self) -> u32 {
        match self {
            Self::Text => 1,
            Self::Markdown => 2,
            Self::Code => 7,
        }
    }
    pub fn from_abi(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Text),
            2 | 5 => Some(Self::Markdown),
            7 => Some(Self::Code),
            _ => None,
        }
    }
}
const VARIANTS: [ThemeVariant; 3] = [
    ThemeVariant::Text,
    ThemeVariant::Markdown,
    ThemeVariant::Code,
];

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct VariantInheritance {
    pub family: Option<ThemeVariant>,
    pub parents: BTreeMap<(StyleNamespace, StyleId), ThemeVariant>,
    pub authored: BTreeSet<(StyleNamespace, StyleId)>,
    pub context: Option<Arc<BTreeMap<ThemeVariant, StyleSheet>>>,
}
impl VariantInheritance {
    pub fn owned_heap_bytes(&self) -> usize {
        style_map_heap_bytes(
            self.parents.len(),
            std::mem::size_of::<((StyleNamespace, StyleId), ThemeVariant)>(),
        ) + self
            .parents
            .keys()
            .map(|(_, id)| id.0.capacity())
            .sum::<usize>()
            + self.context.as_ref().map_or(0, |context| {
                context.values().map(StyleSheet::owned_heap_bytes).sum()
            })
    }
}
fn builtin(variant: ThemeVariant) -> &'static StyleSheet {
    static SHEETS: OnceLock<BTreeMap<ThemeVariant, StyleSheet>> = OnceLock::new();
    &SHEETS.get_or_init(|| {
        [
            (ThemeVariant::Text, StyleSheet::plain_text()),
            (
                ThemeVariant::Markdown,
                StyleSheet::for_format(crate::Format::Markdown),
            ),
            (ThemeVariant::Code, code::default_sheet()),
        ]
        .into_iter()
        .collect()
    })[&variant]
}

/// The envelope is session-only. Each exported/persisted sheet remains sparse
/// and independent; all three variants are bounded immutable resolver inputs.
pub(crate) fn envelope(variant: ThemeVariant, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > super::super::theme::MAX_THEME_BYTES {
        return Err("Theme exceeds 20 MiB".into());
    }
    let styles: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let styles = styles.as_object().ok_or("Expected theme variants object")?;
    let own = styles
        .get(variant.token())
        .filter(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty));
    let mut own = match own {
        Some(value) => value.clone(),
        None => serde_json::json!({"version": if variant == ThemeVariant::Code { 3 } else { 1 }}),
    };
    let object = own.as_object_mut().ok_or("Expected stylesheet object")?;
    object.insert("_theme_variant".into(), serde_json::json!(variant));
    object.insert("_theme_styles".into(), Value::Object(styles.clone()));
    serde_json::to_vec(&own).map_err(|e| e.to_string())
}

pub(super) fn unpack(
    bytes: &[u8],
) -> Result<
    (
        Vec<u8>,
        Option<(ThemeVariant, Arc<BTreeMap<ThemeVariant, StyleSheet>>)>,
    ),
    String,
> {
    if bytes.len() > super::super::theme::MAX_THEME_BYTES {
        return Err("Theme exceeds 20 MiB".into());
    }
    let mut value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let object = value.as_object_mut().ok_or("Expected stylesheet object")?;
    let variant = object.remove("_theme_variant");
    let context = object.remove("_theme_styles");
    let Some(variant) = variant else {
        return Ok((bytes.to_vec(), None));
    };
    let variant: ThemeVariant = serde_json::from_value(variant).map_err(|e| e.to_string())?;
    let context = context
        .and_then(|v| v.as_object().cloned())
        .ok_or("Missing theme variant context")?;
    let mut sheets = BTreeMap::new();
    for family in VARIANTS {
        let sheet = match context
            .get(family.token())
            .filter(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
        {
            None => builtin(family).clone(),
            Some(raw) => {
                if raw.get("_theme_variant").is_some() || raw.get("_theme_styles").is_some() {
                    return Err("Nested theme context is invalid".into());
                }
                let bytes = serde_json::to_vec(raw).map_err(|e| e.to_string())?;
                if family == ThemeVariant::Code {
                    code::parse_json(&bytes)?
                } else {
                    builtin(family)
                        .with_default_json(&bytes)
                        .map_err(|e| e.to_string())?
                        .0
                }
            }
        };
        sheets.insert(family, sheet);
    }
    Ok((
        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        Some((variant, Arc::new(sheets))),
    ))
}

impl StyleSheet {
    pub fn parent_variant(&self, namespace: StyleNamespace, id: &StyleId) -> Option<ThemeVariant> {
        self.variants.parents.get(&(namespace, id.clone())).copied()
    }
    pub(super) fn load_variant_parents(&mut self, bytes: &[u8]) -> Result<(), String> {
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        self.variants.parents.clear();
        for (field, namespace) in [
            ("block_styles", StyleNamespace::Block),
            ("character_styles", StyleNamespace::Character),
        ] {
            for entry in value
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if entry.get("based_on_variant").is_some() {
                    if let Some(id) = entry.get("id").and_then(Value::as_str) {
                        self.variants
                            .authored
                            .insert((namespace, StyleId(id.into())));
                    }
                }
                let Some(parent) = entry.get("based_on_variant").filter(|v| !v.is_null()) else {
                    continue;
                };
                let variant: ThemeVariant =
                    serde_json::from_value(parent.clone()).map_err(|e| e.to_string())?;
                let id = StyleId(
                    entry
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("Missing style ID")?
                        .into(),
                );
                if self.metadata(namespace, &id).is_none() {
                    continue;
                }
                match namespace {
                    StyleNamespace::Block => {
                        self.block_styles.get_mut(&id).unwrap().based_on = None
                    }
                    StyleNamespace::Character => {
                        self.character_styles.get_mut(&id).unwrap().based_on = None
                    }
                };
                self.variants.parents.insert((namespace, id), variant);
            }
        }
        Ok(())
    }
    pub(super) fn export_variant_parents(&self, mut value: Value) -> Value {
        for (field, namespace) in [
            ("block_styles", StyleNamespace::Block),
            ("character_styles", StyleNamespace::Character),
        ] {
            for entry in value
                .get_mut(field)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let Some(id) = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| StyleId(id.into()))
                else {
                    continue;
                };
                let parent = self.parent_variant(namespace, &id);
                // Explicit null retires an authored edge in the native merger;
                // untouched definitions need no persisted relationship field.
                if parent.is_some() || self.variants.authored.contains(&(namespace, id)) {
                    entry["based_on_variant"] =
                        serde_json::to_value(parent).expect("variant serializes");
                }
            }
        }
        value
    }
    pub(super) fn attach_variant_context(
        &mut self,
        context: Option<(ThemeVariant, Arc<BTreeMap<ThemeVariant, StyleSheet>>)>,
    ) {
        if let Some((family, context)) = context {
            self.variants.family = Some(family);
            self.variants.context = Some(context);
        }
    }
    pub(super) fn metadata(
        &self,
        ns: StyleNamespace,
        id: &StyleId,
    ) -> Option<&StyleDefinitionMetadata> {
        match ns {
            StyleNamespace::Block => self.block_style_metadata(id),
            StyleNamespace::Character => self.character_style_metadata(id),
        }
    }
    fn variant_sheet(&self, family: ThemeVariant) -> Option<&StyleSheet> {
        if Some(family) == self.variants.family {
            Some(self)
        } else {
            self.variants.context.as_ref()?.get(&family)
        }
    }
    fn foreign_target<'a>(
        &'a self,
        sheet: &StyleSheet,
        ns: StyleNamespace,
        id: &StyleId,
        family: ThemeVariant,
    ) -> Result<(&'a StyleSheet, StyleId), StyleError> {
        let name = &sheet
            .metadata(ns, id)
            .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
            .display_name;
        let target = self
            .variant_sheet(family)
            .ok_or_else(|| StyleError::MissingParent(id.clone()))?;
        let metadata = match ns {
            StyleNamespace::Block => &target.block_metadata,
            StyleNamespace::Character => &target.character_metadata,
        };
        let compatible = |target_id: &StyleId, metadata: &StyleDefinitionMetadata| {
            &metadata.display_name == name
                && target_id.is_internal_in(ns) == id.is_internal_in(ns)
                && (ns != StyleNamespace::Block
                    || sheet.block_styles[id].role == target.block_styles[target_id].role)
        };
        let target_id = if metadata.get(id).is_some_and(|m| compatible(id, m)) {
            id.clone()
        } else {
            let mut matches = metadata.iter().filter(|(id, m)| compatible(id, m));
            let target_id = matches
                .next()
                .map(|(id, _)| id.clone())
                .ok_or_else(|| StyleError::MissingParent(id.clone()))?;
            if matches.next().is_some() {
                return Err(StyleError::InapplicableStyleRelationship(id.clone()));
            }
            target_id
        };
        Ok((target, target_id))
    }
    pub(super) fn variant_chain<'a>(
        &'a self,
        ns: StyleNamespace,
        id: &StyleId,
    ) -> Result<Vec<(&'a StyleSheet, StyleId)>, StyleError> {
        let mut family = self
            .variants
            .family
            .ok_or_else(|| StyleError::MissingParent(id.clone()))?;
        let mut sheet = self;
        let mut current = id.clone();
        let mut seen = BTreeSet::new();
        let mut chain = Vec::new();
        loop {
            if !seen.insert((family, current.clone())) {
                return Err(StyleError::InheritanceCycle(current));
            }
            if sheet.metadata(ns, &current).is_none() {
                return Err(StyleError::UnknownStyle(current));
            }
            chain.push((sheet, current.clone()));
            if let Some(next_family) = sheet.parent_variant(ns, &current) {
                let (next_sheet, next_id) =
                    self.foreign_target(sheet, ns, &current, next_family)?;
                sheet = next_sheet;
                current = next_id;
                family = next_family;
            } else {
                let parent = match ns {
                    StyleNamespace::Block => &sheet.block_styles[&current].based_on,
                    StyleNamespace::Character => &sheet.character_styles[&current].based_on,
                };
                let Some(parent) = parent else {
                    break;
                };
                current = parent.clone();
            }
        }
        chain.reverse();
        Ok(chain)
    }
    pub(super) fn variant_block_chain(
        &self,
        id: &StyleId,
        role: BlockRole,
    ) -> Result<Vec<&BlockStyle>, StyleError> {
        match self.variant_chain(StyleNamespace::Block, id) {
            Ok(chain) => {
                let styles: Vec<_> = chain
                    .into_iter()
                    .map(|(sheet, id)| &sheet.block_styles[&id])
                    .collect();
                if styles.last().is_none_or(|s| s.role != role) {
                    return Err(StyleError::InapplicableStyleRelationship(id.clone()));
                }
                Ok(styles)
            }
            Err(
                StyleError::InheritanceCycle(_)
                | StyleError::MissingParent(_)
                | StyleError::InapplicableStyleRelationship(_),
            ) => {
                let defaults = builtin(self.variants.family.unwrap());
                if defaults.block_styles.contains_key(id) {
                    defaults.block_chain(id, role)
                } else {
                    // A custom definition has no preset. Supply the immutable
                    // builtin root directly, never the active theme's root.
                    Ok(vec![&defaults.block_styles[&defaults.base_paragraph]])
                }
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn variant_character_chain(
        &self,
        id: &StyleId,
    ) -> Result<Vec<&CharacterStyle>, StyleError> {
        match self.variant_chain(StyleNamespace::Character, id) {
            Ok(chain) => Ok(chain
                .into_iter()
                .map(|(sheet, id)| &sheet.character_styles[&id])
                .collect()),
            Err(
                StyleError::InheritanceCycle(_)
                | StyleError::MissingParent(_)
                | StyleError::InapplicableStyleRelationship(_),
            ) => {
                let defaults = builtin(self.variants.family.unwrap());
                if defaults.character_styles.contains_key(id) {
                    defaults.character_chain(id)
                } else {
                    Ok(Vec::new())
                }
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn has_variant_dependents(&self, ns: StyleNamespace, id: &StyleId) -> bool {
        let (Some(family), Some(context)) = (self.variants.family, self.variants.context.as_ref())
        else {
            return false;
        };
        context
            .iter()
            .filter(|(source, _)| **source != family)
            .any(|(_, sheet)| {
                sheet
                    .variants
                    .parents
                    .iter()
                    .any(|((space, child), target)| {
                        *space == ns
                            && *target == family
                            && self
                                .foreign_target(sheet, ns, child, family)
                                .is_ok_and(|(_, target)| &target == id)
                    })
            })
    }

    pub(super) fn set_variant_parent(
        &mut self,
        ns: StyleNamespace,
        id: &StyleId,
        parent: Option<StyleId>,
        variant: Option<ThemeVariant>,
    ) -> Result<bool, StyleError> {
        if variant.is_some() && (variant == self.variants.family || self.variants.context.is_none())
        {
            return Err(StyleError::InapplicableStyleRelationship(id.clone()));
        }
        if ns == StyleNamespace::Block && id == &self.base_paragraph && parent.is_some() {
            return Err(StyleError::InapplicableStyleRelationship(id.clone()));
        }
        let old_variant = self.parent_variant(ns, id);
        if id.is_internal_in(ns) && variant.is_none() {
            let existing = match ns {
                StyleNamespace::Block => {
                    self.block_styles.get(id).and_then(|s| s.based_on.as_ref())
                }
                StyleNamespace::Character => self
                    .character_styles
                    .get(id)
                    .and_then(|s| s.based_on.as_ref()),
            };
            if parent.is_some() || old_variant.is_none() {
                if parent.as_ref() != existing {
                    return Err(StyleError::InvalidDefinitionMetadata(id.clone()));
                }
            }
        }
        let local = match ns {
            StyleNamespace::Block => {
                &mut self
                    .block_styles
                    .get_mut(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                    .based_on
            }
            StyleNamespace::Character => {
                &mut self
                    .character_styles
                    .get_mut(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                    .based_on
            }
        };
        if *local == parent && old_variant == variant {
            return Ok(false);
        }
        *local = parent;
        self.variants.authored.insert((ns, id.clone()));
        if let Some(variant) = variant {
            self.variants.parents.insert((ns, id.clone()), variant);
        } else {
            self.variants.parents.remove(&(ns, id.clone()));
        }
        if ns == StyleNamespace::Block && id != &self.base_paragraph {
            if variant.is_none() && self.block_styles[id].based_on.is_none() {
                self.block_styles.get_mut(id).unwrap().based_on = self
                    .variants
                    .family
                    .and_then(|family| builtin(family).block_style(id))
                    .and_then(|s| s.based_on.clone())
                    .or_else(|| Some(self.base_paragraph.clone()));
            }
            self.validate_block_parent(&self.block_styles[id])?;
        }
        if self.variants.context.is_some() {
            self.variant_chain(ns, id)?;
        } else if ns == StyleNamespace::Block {
            self.block_chain(id, self.block_styles[id].role)?;
        } else {
            self.character_chain(id)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exported(sheet: &StyleSheet) -> Value {
        serde_json::from_slice(&if sheet.variants.family == Some(ThemeVariant::Code) {
            code::export_snapshot(sheet).unwrap()
        } else {
            sheet
                .default_configuration_json(&DocumentStyleAssignment::new("Paragraph".into()))
                .unwrap()
        })
        .unwrap()
    }
    fn base(family: ThemeVariant) -> Value {
        let sheet = builtin(family);
        serde_json::from_slice(&if family == ThemeVariant::Code {
            code::export_snapshot(sheet).unwrap()
        } else {
            sheet
                .default_configuration_json(&DocumentStyleAssignment::new("Paragraph".into()))
                .unwrap()
        })
        .unwrap()
    }
    fn load(family: ThemeVariant, all: Value) -> StyleSheet {
        let bytes = envelope(family, &serde_json::to_vec(&all).unwrap()).unwrap();
        if family == ThemeVariant::Code {
            code::parse_json(&bytes).unwrap()
        } else {
            let (sheet, diagnostics) = builtin(family).with_default_json(&bytes).unwrap();
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            sheet
        }
    }
    fn block_mut<'a>(value: &'a mut Value, id: &str) -> &'a mut Value {
        value["block_styles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["id"] == id)
            .unwrap()
    }
    fn parent(
        sheet: &mut StyleSheet,
        ns: StyleNamespace,
        id: &str,
        edit: StyleDefinitionFieldEdit,
    ) -> Result<bool, StyleError> {
        let edit = sheet.prepare_generated_field_edit(ns, &id.into(), &edit)?;
        sheet.apply_configuration_edit(&edit, StyleSheetRevision(sheet.revision.0 + 1), false)
    }
    fn size(sheet: &StyleSheet, id: &str) -> f32 {
        sheet
            .resolve_assigned_paragraph_style(
                &DocumentStyleAssignment::new("Paragraph".into()),
                &id.into(),
                &Default::default(),
                &Default::default(),
                None,
                &Default::default(),
            )
            .unwrap()
            .character
            .size
    }

    #[test]
    fn theme_context_allows_new_code_character_definitions() {
        let mut sheet = load(
            ThemeVariant::Code,
            serde_json::json!({"code":base(ThemeVariant::Code),"markdown":base(ThemeVariant::Markdown),"text":base(ThemeVariant::Text)}),
        );
        let revision = sheet.revision;
        let parent_id = StyleId::from("custom-parent");
        let child_id = StyleId::from("custom-child");
        for (id, name, based_on, bold) in [
            (parent_id.clone(), "Parent", None, Some(true)),
            (child_id.clone(), "Child", Some(parent_id.clone()), None),
        ] {
            let edit = StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: id.clone(), based_on,
                    properties: CharacterProperties { bold, ..Default::default() },
                },
                metadata: StyleDefinitionMetadata::generated(name),
            };
            assert!(sheet.apply_configuration_edit(&edit, StyleSheetRevision(sheet.revision.0 + 1), false).unwrap());
            assert_eq!(sheet.character_style_metadata(&id).unwrap().display_name, name);
        }
        assert_eq!(sheet.revision, StyleSheetRevision(revision.0 + 2));
        assert!(sheet.resolve_paragraph_style(&sheet.base_paragraph, &sheet.base_paragraph,
            Some(&child_id), &Default::default(), &Default::default()).unwrap().character.bold);
        let before = sheet.clone();
        assert!(matches!(parent(&mut sheet, StyleNamespace::Character, &parent_id.0,
            StyleDefinitionFieldEdit::SetParent(Some(child_id.clone()))), Err(StyleError::InheritanceCycle(_))));
        assert_eq!(sheet, before, "rejected relationships retain metadata and revision");
        let reopened = load(ThemeVariant::Code, serde_json::json!({"code":exported(&sheet)}));
        assert_eq!(reopened.character_style(&child_id).unwrap().based_on.as_ref(), Some(&parent_id));
        assert_eq!(reopened.character_style_metadata(&child_id), sheet.character_style_metadata(&child_id));
    }

    #[test]
    fn theme_context_allows_new_paragraph_definitions() {
        let mut sheet = load(ThemeVariant::Markdown, serde_json::json!({"markdown":base(ThemeVariant::Markdown)}));
        let revision = sheet.revision;
        let id = StyleId::from("custom-paragraph");
        let edit = StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: id.clone(), based_on: Some(sheet.base_paragraph.clone()), next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: CharacterProperties { size: Some(FontSize::Points(28.0)), ..Default::default() },
                block: Default::default(),
            },
            metadata: StyleDefinitionMetadata::generated("Custom paragraph"),
        };
        assert!(sheet.apply_configuration_edit(&edit, StyleSheetRevision(revision.0 + 1), false).unwrap());
        assert_eq!(sheet.revision, StyleSheetRevision(revision.0 + 1));
        assert_eq!(sheet.block_style_metadata(&id).unwrap().display_name, "Custom paragraph");
        assert_eq!(size(&sheet, &id.0), 28.0);
        let reopened = load(ThemeVariant::Markdown, serde_json::json!({"markdown":exported(&sheet)}));
        assert_eq!(reopened.block_style(&id), sheet.block_style(&id));
        assert_eq!(reopened.block_style_metadata(&id), sheet.block_style_metadata(&id));
    }

    #[test]
    fn foreign_base_inherits_sparse_values_and_roundtrips_without_context() {
        let mut text = base(ThemeVariant::Text);
        let mut markdown = base(ThemeVariant::Markdown);
        block_mut(&mut text, "Paragraph")["character"]["size"] = serde_json::json!(31.0);
        block_mut(&mut markdown, "Paragraph")["character"]["size"] = Value::Null;
        let mut sheet = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        parent(
            &mut sheet,
            StyleNamespace::Block,
            "Paragraph",
            StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text),
        )
        .unwrap();
        assert_eq!(size(&sheet, "Paragraph"), 31.0);
        let saved = exported(&sheet);
        assert_eq!(
            block_mut(&mut saved.clone(), "Paragraph")["based_on_variant"],
            "text"
        );
        assert!(saved.get("_theme_styles").is_none());
        assert!(block_mut(&mut saved.clone(), "Paragraph")["character"]["size"].is_null());
        let again = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":saved}),
        );
        assert_eq!(size(&again, "Paragraph"), 31.0);
    }

    #[test]
    fn attempted_cycle_is_atomic_and_existing_cycle_resolves_builtin_base() {
        let mut text = base(ThemeVariant::Text);
        let markdown = base(ThemeVariant::Markdown);
        block_mut(&mut text, "Paragraph")["based_on_variant"] = "markdown".into();
        block_mut(&mut text, "Paragraph")["character"]["size"] = serde_json::json!(91.0);
        let mut sheet = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        let before = sheet.clone();
        assert!(matches!(
            parent(
                &mut sheet,
                StyleNamespace::Block,
                "Paragraph",
                StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text)
            ),
            Err(StyleError::InheritanceCycle(_))
        ));
        assert_eq!(sheet, before);
        let mut markdown = exported(&sheet);
        block_mut(&mut markdown, "Paragraph")["based_on_variant"] = "text".into();
        let cycle = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        assert_eq!(
            size(&cycle, "Paragraph"),
            size(builtin(ThemeVariant::Markdown), "Paragraph")
        );
        assert_eq!(
            block_mut(&mut exported(&cycle), "Paragraph")["based_on_variant"],
            "text"
        );
    }

    #[test]
    fn character_foreign_parent_uses_name_not_id_and_keeps_heading_context() {
        let mut text = base(ThemeVariant::Text);
        let mut markdown = base(ThemeVariant::Markdown);
        text["character_styles"].as_array_mut().unwrap().push(serde_json::json!({"id":"foreign-id","name":"Custom","based_on":null,"properties":{"bold":true}}));
        markdown["character_styles"].as_array_mut().unwrap().push(
            serde_json::json!({"id":"own-id","name":"Custom","based_on":null,"properties":{}}),
        );
        let mut sheet = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        parent(
            &mut sheet,
            StyleNamespace::Character,
            "own-id",
            StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text),
        )
        .unwrap();
        let styled = sheet
            .resolve_assigned_paragraph_style(
                &DocumentStyleAssignment::new("Paragraph".into()),
                &"Heading1".into(),
                &Default::default(),
                &Default::default(),
                Some(&"own-id".into()),
                &Default::default(),
            )
            .unwrap();
        assert!(styled.character.bold);
        assert_eq!(styled.character.size, size(&sheet, "Heading1"));
    }

    #[test]
    fn chrome_foreign_parent_and_clear_persist_explicit_null_for_code() {
        let mut sheet = load(
            ThemeVariant::Code,
            serde_json::json!({"text":base(ThemeVariant::Text)}),
        );
        let id = sheet
            .block_styles()
            .find(|s| sheet.builtin_line_number_kind(s) == Some(false))
            .unwrap()
            .id
            .0
            .clone();
        parent(
            &mut sheet,
            StyleNamespace::Block,
            &id,
            StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text),
        )
        .unwrap();
        assert_eq!(
            sheet.parent_variant(StyleNamespace::Block, &StyleId(id.clone())),
            Some(ThemeVariant::Text)
        );
        let saved = exported(&sheet);
        let mut again = load(
            ThemeVariant::Code,
            serde_json::json!({"text":base(ThemeVariant::Text),"code":saved}),
        );
        parent(
            &mut again,
            StyleNamespace::Block,
            &id,
            StyleDefinitionFieldEdit::SetParent(None),
        )
        .unwrap();
        assert!(block_mut(&mut exported(&again), &id)["based_on_variant"].is_null());
    }

    #[test]
    fn foreign_chrome_prefers_builtin_identity_over_same_named_content() {
        let mut text = base(ThemeVariant::Text);
        let defaults = builtin(ThemeVariant::Text);
        let id = defaults
            .block_styles()
            .find(|s| defaults.builtin_line_number_kind(s) == Some(false))
            .unwrap()
            .id
            .0
            .clone();
        let mut copy = block_mut(&mut text, &id).clone();
        copy["id"] = "AAA custom line number".into();
        copy["character"]["size"] = 77.0.into();
        text["block_styles"].as_array_mut().unwrap().push(copy);
        block_mut(&mut text, &id)["character"]["size"] = Value::Null;
        block_mut(&mut text, "Paragraph")["character"]["size"] = 23.0.into();
        let mut sheet = load(ThemeVariant::Code, serde_json::json!({"text":text}));
        parent(
            &mut sheet,
            StyleNamespace::Block,
            &id,
            StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text),
        )
        .unwrap();
        let edit = sheet
            .prepare_generated_field_edit(
                StyleNamespace::Block,
                &StyleId(id.clone()),
                &StyleDefinitionFieldEdit::ClearDeclaration(StyleProperty::CharacterSize),
            )
            .unwrap();
        sheet
            .apply_configuration_edit(&edit, StyleSheetRevision(99), false)
            .unwrap();
        assert_eq!(size(&sheet, &id), 23.0);
    }

    #[test]
    fn local_edge_closing_cross_variant_cycle_is_rejected() {
        let mut text = base(ThemeVariant::Text);
        let mut markdown = base(ThemeVariant::Markdown);
        for (value, id, parent, variant) in [
            (&mut text, "A", None, Some("markdown")),
            (&mut markdown, "A", Some("B"), None),
        ] {
            value["character_styles"].as_array_mut().unwrap().push(serde_json::json!({"id":id,"name":id,"based_on":parent,"based_on_variant":variant,"properties":{}}));
        }
        for value in [&mut text, &mut markdown] {
            value["character_styles"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"id":"B","name":"B","based_on":null,"properties":{}}));
        }
        let mut sheet = load(
            ThemeVariant::Markdown,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        parent(
            &mut sheet,
            StyleNamespace::Character,
            "B",
            StyleDefinitionFieldEdit::SetParentVariant(ThemeVariant::Text),
        )
        .unwrap();
        // Text B -> Text A -> Markdown A -> Markdown B -> Text B.
        let saved = exported(&sheet);
        let mut sheet = load(
            ThemeVariant::Text,
            serde_json::json!({"text":text,"markdown":saved}),
        );
        assert!(!sheet.local_parent_candidates(StyleNamespace::Character, &"B".into()).contains(&"A".into()));
        let before = sheet.clone();
        assert!(matches!(
            parent(
                &mut sheet,
                StyleNamespace::Character,
                "B",
                StyleDefinitionFieldEdit::SetParent(Some("A".into()))
            ),
            Err(StyleError::InheritanceCycle(_))
        ));
        assert_eq!(sheet, before);
    }
    #[test]
    fn cyclic_custom_block_uses_builtin_values_without_current_base() {
        let mut text = base(ThemeVariant::Text);
        let mut markdown = base(ThemeVariant::Markdown);
        block_mut(&mut text, "Paragraph")["character"]["size"] = 91.0.into();
        for (value, target) in [(&mut text, "markdown"), (&mut markdown, "text")] {
            value["block_styles"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "id":"A","name":"Custom","based_on":null,"based_on_variant":target,
                    "role":"Paragraph","character":{},"block":{}
                }));
        }
        let sheet = load(
            ThemeVariant::Text,
            serde_json::json!({"text":text,"markdown":markdown}),
        );
        assert_eq!(size(&sheet, "Paragraph"), 91.0);
        assert_eq!(size(&sheet, "A"), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn load_rejects_overflow_only_exposed_by_foreign_ancestry_before_install() {
        let mut text = base(ThemeVariant::Text);
        let mut markdown = base(ThemeVariant::Markdown);
        for (value, prefix) in [(&mut text, "T"), (&mut markdown, "U")] {
            for index in 0..20 {
                let id = format!("{prefix}{index}");
                let name = if prefix == "U" && index == 0 {
                    "T19".to_owned()
                } else {
                    id.clone()
                };
                let parent = if index == 19 {
                    "Paragraph".to_owned()
                } else {
                    format!("{prefix}{}", index + 1)
                };
                value["block_styles"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "id":id,"name":name,"based_on":parent,"role":"Paragraph",
                        "character":{"size":{"percentage":1000}},"block":{}
                    }));
            }
        }
        let mut document = crate::document::Document::new("unchanged");
        let before = document.projection().style_sheet().clone();
        block_mut(&mut text, "T19")["based_on_variant"] = "markdown".into();
        let bytes = envelope(
            ThemeVariant::Text,
            &serde_json::to_vec(&serde_json::json!({"text":text,"markdown":markdown})).unwrap(),
        )
        .unwrap();
        assert!(document.initialize_style_defaults(&bytes).is_err());
        assert_eq!(document.projection().style_sheet(), &before);
        assert_eq!(document.source_bytes(), b"unchanged");
        assert_eq!(document.revision().0, 0);
    }
}
