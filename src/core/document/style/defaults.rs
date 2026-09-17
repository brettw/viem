//! User style defaults are an independent sparse layer, never source syntax.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum StyleDefaultsError {
    Json(String),
    UnsupportedVersion(u32),
    InvalidStyle(StyleError),
    NotPristine,
}
impl std::fmt::Display for StyleDefaultsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(message) => write!(f, "Invalid style defaults: {message}"),
            Self::UnsupportedVersion(v) => write!(f, "Unsupported style defaults version {v}"),
            Self::InvalidStyle(error) => write!(f, "Invalid style defaults: {error:?}"),
            Self::NotPristine => write!(
                f,
                "Style defaults must be loaded before editing or opening a view"
            ),
        }
    }
}
impl std::error::Error for StyleDefaultsError {}
impl From<StyleError> for StyleDefaultsError {
    fn from(e: StyleError) -> Self {
        Self::InvalidStyle(e)
    }
}

#[derive(Serialize, Deserialize)]
struct BlockDefault {
    name: String,
    #[serde(flatten)]
    style: BlockStyle,
}
#[derive(Serialize, Deserialize)]
struct CharacterDefault {
    name: String,
    #[serde(flatten)]
    style: CharacterStyle,
}
#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    block_styles: Vec<BlockDefault>,
    #[serde(default)]
    character_styles: Vec<CharacterDefault>,
}

impl StyleSheet {
    pub(crate) fn has_html_native_definitions(&self) -> bool {
        self.source_defined_blocks
            .iter()
            .any(|id| super::super::html_styles::is_native_style(self, id, false))
            || self
                .source_defined_characters
                .iter()
                .any(|id| super::super::html_styles::is_native_style(self, id, true))
    }

    /// Native HTML style edits may remain buffer configuration instead of CSS.
    pub(crate) fn keep_html_native_configuration(&mut self, id: &StyleId, character: bool) {
        if !super::super::html_styles::is_native_style(self, id, character) {
            return;
        }
        if character {
            self.html_configuration_characters.insert(id.clone());
            self.source_defined_characters.remove(id);
        } else {
            self.html_configuration_blocks.insert(id.clone());
            self.source_defined_blocks.remove(id);
        }
    }

    pub(crate) fn keep_all_html_native_configuration(&mut self) {
        let blocks = self.block_styles.keys().cloned().collect::<Vec<_>>();
        let characters = self.character_styles.keys().cloned().collect::<Vec<_>>();
        for id in blocks {
            self.keep_html_native_configuration(&id, false);
        }
        for id in characters {
            self.keep_html_native_configuration(&id, true);
        }
    }

    pub(crate) fn clear_html_native_configuration(&mut self) {
        self.html_configuration_blocks.clear();
        self.html_configuration_characters.clear();
    }

    pub(super) fn retain_html_native_configuration(&mut self, previous: &Self) {
        for id in &previous.html_configuration_blocks {
            if self.source_defined_blocks.contains(id) {
                continue;
            }
            self.html_configuration_blocks.insert(id.clone());
            if previous.deleted_source_blocks.contains(id) {
                self.deleted_source_blocks.insert(id.clone());
            } else {
                self.deleted_source_blocks.remove(id);
            }
            if let Some(style) = previous.block_styles.get(id) {
                self.block_styles.insert(id.clone(), style.clone());
                self.block_metadata
                    .insert(id.clone(), previous.block_metadata[id].clone());
            } else {
                self.block_styles.remove(id);
                self.block_metadata.remove(id);
            }
        }
        for id in &previous.html_configuration_characters {
            if self.source_defined_characters.contains(id) {
                continue;
            }
            self.html_configuration_characters.insert(id.clone());
            if let Some(style) = previous.character_styles.get(id) {
                self.character_styles.insert(id.clone(), style.clone());
                self.character_metadata
                    .insert(id.clone(), previous.character_metadata[id].clone());
            } else {
                self.character_styles.remove(id);
                self.character_metadata.remove(id);
            }
        }
    }

    pub(crate) fn mark_html_export_definitions_source_backed(&mut self) {
        for id in self.block_styles.keys().cloned().collect::<Vec<_>>() {
            if super::super::html_styles::is_native_style(self, &id, false) {
                self.source_defined_blocks.insert(id.clone());
                self.block_metadata.get_mut(&id).unwrap().origin =
                    StyleDefinitionOrigin::SourceBacked;
            }
        }
        for id in self.character_styles.keys().cloned().collect::<Vec<_>>() {
            if super::super::html_styles::is_native_style(self, &id, true) {
                self.source_defined_characters.insert(id.clone());
                self.character_metadata.get_mut(&id).unwrap().origin =
                    StyleDefinitionOrigin::SourceBacked;
            }
        }
    }

    pub(crate) fn materialize_html_export_defaults(&mut self) {
        let blocks = self.block_styles.keys().cloned().collect::<Vec<_>>();
        let characters = self.character_styles.keys().cloned().collect::<Vec<_>>();
        for id in blocks {
            if !super::super::html_styles::is_native_style(self, &id, false) {
                continue;
            }
            let style = self.block_styles.get_mut(&id).unwrap();
            if let Some(default) = self.default_blocks.get(&id) {
                style.character = overlay(&default.character, &style.character);
                style.block = overlay(&default.block, &style.block);
            }
            self.block_metadata.get_mut(&id).unwrap().origin = StyleDefinitionOrigin::SourceBacked;
        }
        for id in characters {
            if !super::super::html_styles::is_native_style(self, &id, true) {
                continue;
            }
            let style = self.character_styles.get_mut(&id).unwrap();
            if let Some(default) = self.default_characters.get(&id) {
                style.properties = overlay(&default.properties, &style.properties);
            }
            self.character_metadata.get_mut(&id).unwrap().origin =
                StyleDefinitionOrigin::SourceBacked;
        }
        self.clear_html_native_configuration();
    }

    pub(crate) fn materialize_html_default_definition(
        &mut self,
        id: &StyleId,
        character: bool,
        include_native: bool,
    ) {
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if !include_native && super::super::html_styles::is_native_style(self, &id, character) {
                break;
            }
            if character {
                let Some(style) = self.character_styles.get_mut(&id) else {
                    break;
                };
                current = style.based_on.clone();
                if let Some(default) = self.default_characters.get(&id) {
                    style.properties = overlay(&default.properties, &style.properties);
                }
                self.character_metadata.get_mut(&id).unwrap().origin =
                    StyleDefinitionOrigin::SourceBacked;
                self.source_defined_characters.insert(id);
            } else {
                let Some(style) = self.block_styles.get_mut(&id) else {
                    break;
                };
                current = style.based_on.clone();
                if let Some(default) = self.default_blocks.get(&id) {
                    style.character = overlay(&default.character, &style.character);
                    style.block = overlay(&default.block, &style.block);
                }
                self.block_metadata.get_mut(&id).unwrap().origin =
                    StyleDefinitionOrigin::SourceBacked;
                self.source_defined_blocks.insert(id);
            }
        }
    }

    pub fn has_user_default(&self, id: &StyleId, character: bool) -> bool {
        if character {
            self.default_characters.contains_key(id)
        } else {
            self.default_blocks.contains_key(id)
        }
    }
    pub(crate) fn materialize_default_definition(&mut self, id: &StyleId, character: bool) {
        let mut current = Some(id.clone());
        while let Some(id) = current {
            if character {
                let Some(style) = self.character_styles.get(&id) else {
                    break;
                };
                current = style.based_on.clone();
                if let Some(metadata) = self.character_metadata.get_mut(&id) {
                    metadata.origin = StyleDefinitionOrigin::SourceBacked;
                }
                self.source_defined_characters.insert(id);
            } else {
                let Some(style) = self.block_styles.get(&id) else {
                    break;
                };
                current = style.based_on.clone();
                if id == self.base_paragraph {
                    break;
                }
                if let Some(metadata) = self.block_metadata.get_mut(&id) {
                    metadata.origin = StyleDefinitionOrigin::SourceBacked;
                }
                self.source_defined_blocks.insert(id);
            }
        }
    }
    pub(crate) fn record_source_character_defaults(
        &mut self,
        id: StyleId,
        properties: CharacterProperties,
    ) {
        self.source_character_defaults.insert(id, properties);
    }
    pub(crate) fn with_default_json(&self, bytes: &[u8]) -> Result<Self, StyleDefaultsError> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(StyleDefaultsError::Json("file exceeds 4 MiB".into()));
        }
        let file: File =
            serde_json::from_slice(bytes).map_err(|e| StyleDefaultsError::Json(e.to_string()))?;
        if file.version != 1 {
            return Err(StyleDefaultsError::UnsupportedVersion(file.version));
        }
        if file.block_styles.len() + file.character_styles.len() > 4096 {
            return Err(StyleDefaultsError::Json(
                "too many style definitions".into(),
            ));
        }
        let mut defaults = StyleSheet::default();
        // Preserve adapter-specific built-ins (for example RTF's intrinsic
        // 12-point default) when a user file omits their definition.
        for (id, style) in &self.block_styles {
            if !self.source_defined_blocks.contains(id) {
                defaults.block_styles.insert(id.clone(), style.clone());
                if let Some(metadata) = self.block_metadata.get(id) {
                    defaults.block_metadata.insert(id.clone(), metadata.clone());
                }
            }
        }
        for (id, style) in &self.character_styles {
            if !self.source_defined_characters.contains(id) {
                defaults.character_styles.insert(id.clone(), style.clone());
                if let Some(metadata) = self.character_metadata.get(id) {
                    defaults
                        .character_metadata
                        .insert(id.clone(), metadata.clone());
                }
            }
        }
        defaults.install_html_source_styles();
        let mut seen = BTreeSet::new();
        let mut definitions = Vec::new();
        for entry in file.block_styles {
            if !seen.insert((0, entry.style.id.clone())) {
                return Err(StyleDefaultsError::Json("duplicate block style".into()));
            }
            definitions.push(StyleDefinitionEdit::InsertBlock {
                style: entry.style,
                metadata: StyleDefinitionMetadata::generated(entry.name),
            });
        }
        for entry in file.character_styles {
            if !seen.insert((1, entry.style.id.clone())) {
                return Err(StyleDefaultsError::Json("duplicate character style".into()));
            }
            definitions.push(StyleDefinitionEdit::InsertCharacter {
                style: entry.style,
                metadata: StyleDefinitionMetadata::generated(entry.name),
            });
        }
        defaults.install_source_definitions(&definitions)?;
        if defaults.block_styles[&defaults.base_paragraph].role != BlockRole::Paragraph
            || defaults.block_styles[&defaults.base_paragraph].based_on.is_some() {
            return Err(StyleDefaultsError::Json("invalid base paragraph relationship".into()));
        }
        let mut candidate = self.clone();
        candidate.default_blocks = defaults.block_styles;
        candidate.default_characters = defaults.character_styles;
        // Keep only internal styles available in this projection: the search
        // overlay is universal; HTML syntax styles require HTML Source.
        candidate
            .default_characters
            .retain(|id, _| !id.is_internal() || self.character_styles.contains_key(id));
        candidate.install_default_layer(&defaults.block_metadata, &defaults.character_metadata);
        candidate.validate_block_cycles()?;
        candidate.validate_character_cycles()?;
        candidate.resolve_document_style(
            &candidate.base_paragraph,
            &BlockProperties::default(),
            &CharacterProperties::default(),
        )?;
        Ok(candidate)
    }

    fn install_default_layer(
        &mut self,
        block_names: &BTreeMap<StyleId, StyleDefinitionMetadata>,
        character_names: &BTreeMap<StyleId, StyleDefinitionMetadata>,
    ) {
        for (id, default) in &self.default_blocks {
            if self.deleted_source_blocks.contains(id)
                || self.deleted_configuration_blocks.contains(id)
            {
                continue;
            }
            if !self.source_defined_blocks.contains(id) {
                let mut sparse = default.clone();
                sparse.character = self
                    .source_character_defaults
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
                sparse.block = BlockProperties::default();
                self.block_styles.insert(id.clone(), sparse);
                let origin = self
                    .block_metadata
                    .get(id)
                    .map(|m| m.origin)
                    .unwrap_or(StyleDefinitionOrigin::GeneratedConfiguration);
                let name = block_names
                    .get(id)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| id.0.clone());
                self.block_metadata.insert(
                    id.clone(),
                    StyleDefinitionMetadata {
                        display_name: name,
                        origin,
                    },
                );
            }
        }
        for (id, default) in &self.default_characters {
            if self.deleted_configuration_characters.contains(id) {
                continue;
            }
            if !self.source_defined_characters.contains(id) {
                let mut sparse = default.clone();
                sparse.properties = CharacterProperties::default();
                self.character_styles.insert(id.clone(), sparse);
                let origin = self
                    .character_metadata
                    .get(id)
                    .map(|m| m.origin)
                    .unwrap_or(StyleDefinitionOrigin::GeneratedConfiguration);
                let name = character_names
                    .get(id)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| id.0.clone());
                self.character_metadata.insert(
                    id.clone(),
                    StyleDefinitionMetadata {
                        display_name: name,
                        origin,
                    },
                );
            }
        }
    }

    pub(super) fn retain_defaults(&mut self, previous: &Self) {
        if previous.default_blocks.is_empty() && previous.default_characters.is_empty() {
            return;
        }
        self.default_blocks = previous.default_blocks.clone();
        self.default_characters = previous.default_characters.clone();
        self.install_default_layer(&previous.block_metadata, &previous.character_metadata);
        // Configuration-only overrides survive source reparsing too.
        for (id, style) in &previous.block_styles {
            if !self.source_defined_blocks.contains(id)
                && previous
                    .block_metadata
                    .get(id)
                    .is_some_and(|m| m.origin == StyleDefinitionOrigin::GeneratedConfiguration)
            {
                self.block_styles.insert(id.clone(), style.clone());
                self.block_metadata
                    .insert(id.clone(), previous.block_metadata[id].clone());
            }
        }
        for (id, style) in &previous.character_styles {
            if !self.source_defined_characters.contains(id)
                && previous
                    .character_metadata
                    .get(id)
                    .is_some_and(|m| m.origin == StyleDefinitionOrigin::GeneratedConfiguration)
            {
                self.character_styles.insert(id.clone(), style.clone());
                self.character_metadata
                    .insert(id.clone(), previous.character_metadata[id].clone());
            }
        }
    }

    pub fn default_configuration_json(
        &self,
        document: &DocumentStyleAssignment,
    ) -> Result<Vec<u8>, StyleDefaultsError> {
        let mut blocks = Vec::new();
        for explicit in self.block_styles.values() {
            let mut style = explicit.clone();
            if let Some(default) = self.default_blocks.get(&style.id) {
                style.character = overlay(&default.character, &style.character);
                style.block = overlay(&default.block, &style.block);
            }
            if style.id == document.style {
                style.character = overlay(&style.character, &document.direct_default_character);
                style.block = overlay(&style.block, &document.direct_canvas);
            }
            blocks.push(BlockDefault {
                name: self.block_metadata[&style.id].display_name.clone(),
                style,
            });
        }
        let characters = self
            .character_styles
            .values()
            .map(|explicit| {
                let mut style = explicit.clone();
                if let Some(default) = self.default_characters.get(&style.id) {
                    style.properties = overlay(&default.properties, &style.properties);
                }
                CharacterDefault {
                    name: self.character_metadata[&style.id].display_name.clone(),
                    style,
                }
            })
            .collect();
        serde_json::to_vec_pretty(&File {
            version: 1,
            block_styles: blocks,
            character_styles: characters,
        })
        .map_err(|e| StyleDefaultsError::Json(e.to_string()))
    }
}

// Property records are flat optional declarations. Null is absent, never an
// instruction to flatten an inherited value into the source layer.
fn overlay<T: Serialize + serde::de::DeserializeOwned>(base: &T, direct: &T) -> T {
    let mut base = serde_json::to_value(base).expect("validated style serializes");
    let direct = serde_json::to_value(direct).expect("validated style serializes");
    for (key, value) in direct.as_object().unwrap() {
        if !value.is_null() {
            base[key] = value.clone();
        }
    }
    serde_json::from_value(base).expect("same property schema")
}
