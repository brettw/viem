//! Saved defaults seed ordinary editable definitions. They are configuration,
//! not a second inheritance layer: removing an own declaration inherits from
//! the parent, and source-authored definitions remain authoritative.
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

#[derive(Deserialize)]
struct InputFile {
    version: u32,
    #[serde(default)]
    block_styles: serde_json::Value,
    #[serde(default)]
    character_styles: serde_json::Value,
}

fn input_entries(value: serde_json::Value, group: &str, diagnostics: &mut Vec<String>) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(entries) => entries,
        serde_json::Value::Null => Vec::new(),
        _ => {
            diagnostics.push(format!("Ignored {group}: expected an array of style definitions."));
            Vec::new()
        }
    }
}

/// Property records are sparse and each declaration can be checked in isolation.
fn valid_properties<T>(
    raw: serde_json::Value,
    id: &StyleId,
    group: &str,
    validate: impl Fn(&T) -> Result<(), StyleError>,
    diagnostics: &mut Vec<String>,
) -> T
where T: Default + Serialize + serde::de::DeserializeOwned {
    let known = serde_json::to_value(T::default()).expect("property schema serializes");
    let fields = match raw {
        serde_json::Value::Object(fields) => fields,
        serde_json::Value::Null => return T::default(),
        _ => {
            diagnostics.push(format!("Ignored style {:?}.{group}: expected a property object.", id.0));
            return T::default();
        }
    };
    let mut accepted = serde_json::Map::new();
    for (property, value) in fields {
        if value.is_null() || known.get(&property).is_none() { continue; }
        let reason = {
            let one = serde_json::Value::Object([(property.clone(), value.clone())].into_iter().collect());
            match serde_json::from_value::<T>(one) {
                Err(error) => Some(error.to_string()),
                Ok(properties) => validate(&properties).err().map(|_| "invalid value for this style".to_owned()),
            }
        };
        if let Some(reason) = reason {
            diagnostics.push(format!("Ignored style {:?}.{group}.{property}: {reason}.", id.0));
        } else {
            accepted.insert(property, value);
        }
    }
    serde_json::from_value(serde_json::Value::Object(accepted)).expect("individually validated sparse properties combine")
}

fn input_object(value: serde_json::Value, group: &str, index: usize, diagnostics: &mut Vec<String>)
    -> Option<serde_json::Map<String, serde_json::Value>> {
    match value {
        serde_json::Value::Object(object) => Some(object),
        _ => {
            diagnostics.push(format!("Ignored {group}[{index}]: expected a style definition object."));
            None
        }
    }
}

fn input_label(object: &serde_json::Map<String, serde_json::Value>, group: &str, index: usize) -> String {
    object.get("id").and_then(serde_json::Value::as_str)
        .map_or_else(|| format!("{group}[{index}]"), |id| format!("style {id:?}"))
}

fn parse_block_defaults(
    entries: Vec<serde_json::Value>,
    builtins: &StyleSheet,
    diagnostics: &mut Vec<String>,
) -> BTreeMap<StyleId, BlockDefault> {
    let mut accepted = BTreeMap::new();
    for (index, value) in entries.into_iter().enumerate() {
        let Some(mut object) = input_object(value, "block_styles", index, diagnostics) else { continue; };
        let label = input_label(&object, "block_styles", index);
        let block = object.insert("block".into(), serde_json::json!({})).unwrap_or_default();
        let character = object.insert("character".into(), serde_json::json!({})).unwrap_or_default();
        if let Some(next) = object.get("next_paragraph_style") {
            if serde_json::from_value::<Option<StyleId>>(next.clone()).is_err() {
                diagnostics.push(format!("Ignored {label}.next_paragraph_style: expected a style ID or null."));
                object.remove("next_paragraph_style");
            }
        }
        let mut entry: BlockDefault = match serde_json::from_value(serde_json::Value::Object(object)) {
            Ok(entry) => entry,
            Err(error) => { diagnostics.push(format!("Ignored {label}: {error}.")); continue; }
        };
        let id = &entry.style.id;
        if id.0.is_empty() || validate_definition_metadata(id, &StyleDefinitionMetadata::generated(&entry.name)).is_err() {
            diagnostics.push(format!("Ignored {label}: invalid style ID or display name."));
            continue;
        }
        if let Some(builtin) = builtins.block_styles.get(id) {
            if entry.style.role != builtin.role {
                diagnostics.push(format!("Ignored {label}: role {:?} is incompatible with built-in role {:?}.", entry.style.role, builtin.role));
                continue;
            }
        }
        if *id == builtins.base_paragraph && entry.style.based_on.is_some() {
            diagnostics.push(format!("Ignored {label}.based_on: Base Paragraph cannot inherit another style."));
            entry.style.based_on = None;
        }
        if accepted.contains_key(id) {
            diagnostics.push(format!("Ignored duplicate {label}."));
            continue;
        }
        entry.style.character = valid_properties(character, id, "character", |properties: &CharacterProperties| {
            validate_character_properties(id, properties)?;
            if *id == builtins.base_paragraph && matches!(properties.size, Some(FontSize::Percentage(_))) {
                return Err(invalid_style_value(id, StyleProperty::CharacterSize));
            }
            Ok(())
        }, diagnostics);
        entry.style.block = valid_properties(block, id, "block", |properties: &BlockProperties| {
            let mut style = entry.style.clone();
            style.block = properties.clone();
            validate_block_properties(&style)
        }, diagnostics);
        accepted.insert(id.clone(), entry);
    }
    accepted
}

fn parse_character_defaults(entries: Vec<serde_json::Value>, diagnostics: &mut Vec<String>)
    -> BTreeMap<StyleId, CharacterDefault> {
    let mut accepted = BTreeMap::new();
    for (index, value) in entries.into_iter().enumerate() {
        let Some(mut object) = input_object(value, "character_styles", index, diagnostics) else { continue; };
        let label = input_label(&object, "character_styles", index);
        let properties = object.insert("properties".into(), serde_json::json!({})).unwrap_or_default();
        let mut entry: CharacterDefault = match serde_json::from_value(serde_json::Value::Object(object)) {
            Ok(entry) => entry,
            Err(error) => { diagnostics.push(format!("Ignored {label}: {error}.")); continue; }
        };
        let id = &entry.style.id;
        if id.0.is_empty() || validate_definition_metadata(id, &StyleDefinitionMetadata::generated(&entry.name)).is_err()
            || (id.is_internal() && entry.style.based_on.is_some()) {
            diagnostics.push(format!("Ignored {label}: invalid style ID, display name, or internal-style relationship."));
            continue;
        }
        if accepted.contains_key(id) {
            diagnostics.push(format!("Ignored duplicate {label}."));
            continue;
        }
        entry.style.properties = valid_properties(properties, id, "properties",
            |properties| validate_character_properties(id, properties), diagnostics);
        accepted.insert(id.clone(), entry);
    }
    accepted
}

fn discard_block_dependency(
    sheet: &StyleSheet, start: &StyleId, entries: &mut BTreeMap<StyleId, BlockDefault>,
    reason: &str, diagnostics: &mut Vec<String>,
) -> bool {
    let mut current = Some(start.clone());
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) { break; }
        if entries.remove(&id).is_some() {
            diagnostics.push(format!("Ignored style {:?}: invalid parent relationship ({reason}).", id.0));
            return true;
        }
        current = sheet.block_styles.get(&id).and_then(|style| style.based_on.clone());
    }
    false
}

fn discard_character_dependency(
    sheet: &StyleSheet, start: &StyleId, entries: &mut BTreeMap<StyleId, CharacterDefault>,
    reason: &str, diagnostics: &mut Vec<String>,
) -> bool {
    let mut current = Some(start.clone());
    let mut seen = BTreeSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) { break; }
        if let Some(saved) = entries.get_mut(&id) {
            if saved.style.based_on.take().is_some() {
                diagnostics.push(format!("Ignored character style {:?}.based_on: {reason}.", id.0));
                return true;
            }
        }
        current = sheet.character_styles.get(&id).and_then(|style| style.based_on.clone());
    }
    false
}

/// Each repair removes a saved definition or one optional declaration. Rebuild
/// from the valid baseline afterwards so omitted built-ins regain their actual
/// definitions, and forward references never depend on file order.
fn repair_default_graph(
    sheet: &StyleSheet,
    blocks: &mut BTreeMap<StyleId, BlockDefault>,
    characters: &mut BTreeMap<StyleId, CharacterDefault>,
    diagnostics: &mut Vec<String>,
) -> bool {
    for style in sheet.block_styles.values() {
        if style.id != sheet.base_paragraph {
            if let Err(error) = sheet.validate_block_parent(style) {
                if discard_block_dependency(sheet, &style.id, blocks, &format!("{error:?}"), diagnostics) { return true; }
            }
        }
    }
    // Reject missing immediate edges before walking any complete chains. This
    // keeps a long chain ending in a missing parent bounded during recovery.
    for style in sheet.character_styles.values() {
        if let Some(parent) = &style.based_on {
            if !sheet.character_styles.contains_key(parent) {
                if discard_character_dependency(sheet, &style.id, characters, &format!("missing parent {:?}", parent.0), diagnostics) { return true; }
            }
        }
    }
    for style in sheet.block_styles.values() {
        if let Err(error) = sheet.validate_next_paragraph_style(style) {
            if let Some(saved) = blocks.get_mut(&style.id).filter(|saved| saved.style.next_paragraph_style.is_some()) {
                saved.style.next_paragraph_style = None;
                diagnostics.push(format!("Ignored style {:?}.next_paragraph_style: {error:?}.", style.id.0));
                return true;
            }
            if let Some(next) = &style.next_paragraph_style {
                if discard_block_dependency(sheet, next, blocks, &format!("invalid next style for {:?}", style.id.0), diagnostics) { return true; }
            }
        }
        match sheet.block_chain(&style.id, style.role) {
            Err(StyleError::InheritanceCycle(id)) => {
                if discard_block_dependency(sheet, &id, blocks, "inheritance cycle", diagnostics) { return true; }
            }
            Ok(chain) => {
                let mut size = sheet.intrinsic_character_defaults.size
                    .map_or(DEFAULT_FONT_SIZE, |value| value.resolve(DEFAULT_FONT_SIZE));
                let mut saved_size = None;
                for ancestor in chain {
                    if let Some(value) = ancestor.character.size {
                        if blocks.get(&ancestor.id).is_some_and(|saved| saved.style.character.size.is_some()) {
                            saved_size = Some(ancestor.id.clone());
                        }
                        size = value.resolve(size);
                    }
                    if !size.is_finite() || size <= 0.0 {
                        if let Some(id) = saved_size {
                            blocks.get_mut(&id).unwrap().style.character.size = None;
                            diagnostics.push(format!("Ignored style {:?}.character.size: inherited font size is not finite and positive.", id.0));
                            return true;
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    for style in sheet.character_styles.values() {
        match sheet.character_chain(&style.id) {
            Err(StyleError::InheritanceCycle(id)) => {
                if discard_character_dependency(sheet, &id, characters, "inheritance cycle", diagnostics) { return true; }
            }
            _ => {}
        }
    }
    false
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
            self.block_metadata.get_mut(&id).unwrap().origin = StyleDefinitionOrigin::SourceBacked;
        }
        for id in characters {
            if !super::super::html_styles::is_native_style(self, &id, true) {
                continue;
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
                self.character_metadata.get_mut(&id).unwrap().origin =
                    StyleDefinitionOrigin::SourceBacked;
                self.source_defined_characters.insert(id);
            } else {
                let Some(style) = self.block_styles.get_mut(&id) else {
                    break;
                };
                current = style.based_on.clone();
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
    pub(crate) fn with_default_json(&self, bytes: &[u8]) -> Result<(Self, Vec<String>), StyleDefaultsError> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(StyleDefaultsError::Json("file exceeds 4 MiB".into()));
        }
        let file: InputFile =
            serde_json::from_slice(bytes).map_err(|e| StyleDefaultsError::Json(e.to_string()))?;
        if file.version != 1 {
            return Err(StyleDefaultsError::UnsupportedVersion(file.version));
        }
        let mut diagnostics = Vec::new();
        let block_entries = input_entries(file.block_styles, "block_styles", &mut diagnostics);
        let character_entries = input_entries(file.character_styles, "character_styles", &mut diagnostics);
        if block_entries.len() + character_entries.len() > 4096 {
            return Err(StyleDefaultsError::Json("too many style definitions".into()));
        }
        let builtins = StyleSheet::default();
        let mut blocks = parse_block_defaults(block_entries, &builtins, &mut diagnostics);
        let mut characters = parse_character_defaults(character_entries, &mut diagnostics);
        let mut baseline = builtins;
        // Keep adapter-specific defaults and already installed configuration.
        // Source-owned definitions remain authoritative in the final candidate.
        for (id, style) in &self.block_styles {
            if !self.source_defined_blocks.contains(id) {
                baseline.block_styles.insert(id.clone(), style.clone());
                if let Some(metadata) = self.block_metadata.get(id) {
                    baseline.block_metadata.insert(id.clone(), metadata.clone());
                }
            }
        }
        for (id, style) in &self.character_styles {
            if !self.source_defined_characters.contains(id) {
                baseline.character_styles.insert(id.clone(), style.clone());
                if let Some(metadata) = self.character_metadata.get(id) {
                    baseline.character_metadata.insert(id.clone(), metadata.clone());
                }
            }
        }
        baseline.install_html_source_styles();
        // A repair removes an entry, next-style declaration, parent edge, or
        // size declaration. This finite bound also guards future repair changes.
        let repair_limit = 4 * (blocks.len() + characters.len());
        for _ in 0..=repair_limit {
            let mut defaults = baseline.clone();
            for (id, entry) in &blocks {
                defaults.block_styles.insert(id.clone(), entry.style.clone());
                defaults.block_metadata.insert(id.clone(), StyleDefinitionMetadata::generated(&entry.name));
            }
            for (id, entry) in &characters {
                defaults.character_styles.insert(id.clone(), entry.style.clone());
                defaults.character_metadata.insert(id.clone(), StyleDefinitionMetadata::generated(&entry.name));
            }
            if repair_default_graph(&defaults, &mut blocks, &mut characters, &mut diagnostics) { continue; }
            defaults.validate_block_cycles()?;
            defaults.validate_character_cycles()?;
            let mut candidate = self.clone();
            candidate.default_blocks = defaults.block_styles;
            candidate.default_characters = defaults.character_styles;
            candidate.default_characters.retain(|id, _| !id.is_internal() || self.character_styles.contains_key(id));
            candidate.install_default_definitions(&defaults.block_metadata, &defaults.character_metadata);
            if repair_default_graph(&candidate, &mut blocks, &mut characters, &mut diagnostics) { continue; }
            candidate.validate_block_cycles()?;
            candidate.validate_character_cycles()?;
            candidate.resolve_document_style(
                &candidate.base_paragraph,
                &BlockProperties::default(),
                &CharacterProperties::default(),
            )?;
            return Ok((candidate, diagnostics));
        }
        Err(StyleDefaultsError::Json("style relationship validation did not converge".into()))
    }

    fn install_default_definitions(
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
                let mut definition = default.clone();
                if let Some(source) = self.source_character_defaults.get(id) {
                    definition.character.overlay(source);
                }
                self.block_styles.insert(id.clone(), definition);
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
                self.character_styles.insert(id.clone(), default.clone());
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
        if !previous.default_blocks.is_empty() || !previous.default_characters.is_empty() {
            self.default_blocks = previous.default_blocks.clone();
            self.default_characters = previous.default_characters.clone();
            self.install_default_definitions(&previous.block_metadata, &previous.character_metadata);
        }
        // Configuration-only overrides survive source reparsing even when the
        // user has never loaded a saved default sheet. A generated style may
        // also be the parent of a source-backed or native configuration style.
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
                let style = explicit.clone();
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
