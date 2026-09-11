mod defaults;
pub use defaults::StyleDefaultsError;
pub mod code;

use std::collections::{BTreeMap, BTreeSet};

/// Opaque stable identity of a block or character style. The string is a
/// serialization-friendly token, not the user-visible style name.
#[derive(
    Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct StyleId(pub String);

impl From<&str> for StyleId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl StyleId {
    /// Structural list families are editable definitions, distinct from source
    /// syntax coloring styles. They are assigned through list commands.
    pub fn is_internal_list(&self) -> bool {
        self.list_family_level().is_some()
    }

    pub fn list_family_level(&self) -> Option<(bool, u8)> {
        for (prefix, ordered) in [("BulletedList", false), ("NumberedList", true)] {
            if let Some(level) = self
                .0
                .strip_prefix(prefix)
                .and_then(|n| n.parse::<u8>().ok())
            {
                if (1..=4).contains(&level) && self.0 == format!("{prefix}{level}") {
                    return Some((ordered, level));
                }
            }
        }
        None
    }

    pub(crate) fn legacy_list_level(&self) -> Option<u16> {
        self.0
            .strip_prefix("List")
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|level| (1..=256).contains(level) && self.0 == format!("List{level}"))
    }

    pub fn is_internal(&self) -> bool {
        matches!(
            self.0.as_str(),
            "* HTML Brackets"
                | "* HTML Tag name"
                | "* HTML Attribute key"
                | "* HTML Attribute value"
                | "* HTML Equals"
                | "* HTML Entity"
                | "* HTML Uninterpreted"
        )
    }
}

/// Provenance/editability class of one normalized style definition.
///
/// This is immutable metadata on the definition itself. Model requests do not
/// get to assert their own authority: callers name a stable style ID and the
/// document derives whether it is editable from this core-owned value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleDefinitionOrigin {
    SourceBacked,
    GeneratedConfiguration,
    SyntheticReadOnly,
}

/// Core-owned presentation and authority metadata for one style definition.
/// Stable IDs remain opaque serialization tokens and are deliberately distinct
/// from the user-visible display name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleDefinitionMetadata {
    pub display_name: String,
    pub origin: StyleDefinitionOrigin,
}

impl StyleDefinitionMetadata {
    pub fn generated(display_name: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            origin: StyleDefinitionOrigin::GeneratedConfiguration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FontSlant {
    Upright,
    Italic,
    Oblique,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WritingDirection {
    Natural,
    LeftToRight,
    RightToLeft,
}

/// The schema domain a block style may declare. Future structural roles (for
/// example List and ListItem) can extend this enum without creating another
/// style namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BlockRole {
    Document,
    Paragraph,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LineSpacing {
    Normal,
    Multiplier(f32),
    AtLeast(f32),
    Exact(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ParagraphAlignment {
    Start,
    End,
    Center,
}

/// Sparse character declarations. `None` means inherit/leave unchanged.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CharacterProperties {
    pub font_families: Option<Vec<String>>,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    /// Semantic emphasis relative to the selected base face/weight.
    pub bold: Option<bool>,
    pub slant: Option<FontSlant>,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub language: Option<String>,
    pub direction: Option<WritingDirection>,
    pub open_type_features: Option<BTreeMap<String, u32>>,
    pub letter_spacing: Option<f32>,
    pub baseline_shift: Option<f32>,
}

impl CharacterProperties {
    pub(super) fn overlay(&mut self, layer: &Self) {
        if layer.weight.is_some() {
            self.bold = None;
        }
        self.merge_declarations(layer);
    }

    pub(super) fn owned_heap_bytes(&self) -> usize {
        self.font_families.as_ref().map_or(0, |families| {
            families.capacity() * std::mem::size_of::<String>()
                + families.iter().map(|name| name.capacity() + 16).sum::<usize>()
        }) + self.language.as_ref().map_or(0, |value| value.capacity() + 16)
            + self.open_type_features.as_ref().map_or(0, |features| {
                style_map_heap_bytes(features.len(), std::mem::size_of::<(String, u32)>())
                    + features.keys().map(|name| name.capacity() + 16).sum::<usize>()
            })
    }
}

/// Sparse paragraph/block declarations. Future list properties can be added
/// here without introducing a separate paragraph-style namespace.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BlockProperties {
    pub spacing_before: Option<f32>,
    pub spacing_after: Option<f32>,
    pub line_spacing: Option<LineSpacing>,
    pub first_line_indent: Option<f32>,
    pub leading_indent: Option<f32>,
    pub trailing_indent: Option<f32>,
    pub padding_top: Option<f32>,
    pub padding_right: Option<f32>,
    pub padding_bottom: Option<f32>,
    pub padding_left: Option<f32>,
    pub background: Option<Color>,
    pub alignment: Option<ParagraphAlignment>,
    pub base_direction: Option<WritingDirection>,
}

/// Normalized style assignment for the formatted document root.
///
/// The style must have the [`BlockRole::Document`] role. Direct declarations
/// are sparse layers applied after the assigned style chain: `direct_canvas`
/// affects the document surface, while `direct_default_character` supplies
/// inherited character defaults for every paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentStyleAssignment {
    pub style: StyleId,
    pub direct_canvas: BlockProperties,
    pub direct_default_character: CharacterProperties,
}

impl DocumentStyleAssignment {
    pub fn new(style: StyleId) -> Self {
        Self {
            style,
            direct_canvas: BlockProperties::default(),
            direct_default_character: CharacterProperties::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CharacterStyle {
    pub id: StyleId,
    pub based_on: Option<StyleId>,
    pub properties: CharacterProperties,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockStyle {
    pub id: StyleId,
    pub based_on: Option<StyleId>,
    /// Paragraph style assigned to a new paragraph created by splitting this
    /// paragraph at its terminal boundary. `None` means retain this style.
    /// Only [`BlockRole::Paragraph`] styles may declare this relationship.
    pub next_paragraph_style: Option<StyleId>,
    pub role: BlockRole,
    pub character: CharacterProperties,
    pub block: BlockProperties,
}

/// Format-independent payload for inserting, updating, or deleting one style
/// definition. The containing model intention supplies the authority and
/// provenance class; this payload alone cannot mutate a document.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleDefinitionEdit {
    InsertBlock {
        style: BlockStyle,
        metadata: StyleDefinitionMetadata,
    },
    UpdateBlock(BlockStyle),
    DeleteBlock(StyleId),
    InsertCharacter {
        style: CharacterStyle,
        metadata: StyleDefinitionMetadata,
    },
    UpdateCharacter(CharacterStyle),
    DeleteCharacter(StyleId),
    UpdateMetadata {
        namespace: StyleNamespace,
        id: StyleId,
        metadata: StyleDefinitionMetadata,
    },
}

impl StyleDefinitionEdit {
    pub fn style_id(&self) -> &StyleId {
        match self {
            Self::InsertBlock { style, .. } | Self::UpdateBlock(style) => &style.id,
            Self::DeleteBlock(id) => id,
            Self::InsertCharacter { style, .. } | Self::UpdateCharacter(style) => &style.id,
            Self::DeleteCharacter(id) => id,
            Self::UpdateMetadata { id, .. } => id,
        }
    }

    pub fn is_block(&self) -> bool {
        matches!(
            self,
            Self::InsertBlock { .. }
                | Self::UpdateBlock(_)
                | Self::DeleteBlock(_)
                | Self::UpdateMetadata {
                    namespace: StyleNamespace::Block,
                    ..
                }
        )
    }
}

/// Explicit operations on the generated/application configuration layer.
/// These create immutable, undoable configuration states while leaving source
/// bytes untouched. Source-backed edits use adapter-capability-checked
/// persisted intentions, and synthetic definitions remain read-only.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigurationStyleIntent {
    EditDefinition(StyleDefinitionEdit),
    /// Assign a generated/configuration Document-role style to the formatted
    /// root. This is not a persisted content assignment.
    AssignDocumentStyle(StyleId),
    SetDocumentCanvas(BlockProperties),
    ClearDocumentCanvas(BTreeSet<StyleProperty>),
    SetDocumentDefaultCharacter(CharacterProperties),
    ClearDocumentDefaultCharacter(BTreeSet<StyleProperty>),
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct StyleSheetRevision(pub u64);

#[derive(Clone, Debug)]
pub struct StyleSheet {
    pub revision: StyleSheetRevision,
    pub base_document: StyleId,
    pub base_paragraph: StyleId,
    pub base_character: StyleId,
    block_styles: BTreeMap<StyleId, BlockStyle>,
    character_styles: BTreeMap<StyleId, CharacterStyle>,
    block_metadata: BTreeMap<StyleId, StyleDefinitionMetadata>,
    character_metadata: BTreeMap<StyleId, StyleDefinitionMetadata>,
    deleted_configuration_blocks: BTreeSet<StyleId>,
    deleted_configuration_characters: BTreeSet<StyleId>,
    deleted_source_blocks: BTreeSet<StyleId>,
    source_defined_blocks: BTreeSet<StyleId>,
    source_defined_characters: BTreeSet<StyleId>,
    html_configuration_blocks: BTreeSet<StyleId>,
    html_configuration_characters: BTreeSet<StyleId>,
    source_character_defaults: BTreeMap<StyleId, CharacterProperties>,
    default_blocks: BTreeMap<StyleId, BlockStyle>,
    default_characters: BTreeMap<StyleId, CharacterStyle>,
}

// Maps do not expose node capacities. Charge a conservative estimate for their
// partially occupied nodes; strings and property payloads are added separately.
fn style_map_heap_bytes(len: usize, item_size: usize) -> usize {
    if len == 0 { 0 } else { (3 * len + 11) * item_size + (len + 1) * 128 }
}

impl StyleSheet {
    pub(super) fn owned_heap_bytes(&self) -> usize {
        fn id(value: &StyleId) -> usize { value.0.capacity() + 16 }
        fn character(value: &CharacterStyle) -> usize {
            id(&value.id) + value.based_on.as_ref().map_or(0, id) + value.properties.owned_heap_bytes()
        }
        fn block(value: &BlockStyle) -> usize {
            id(&value.id) + value.based_on.as_ref().map_or(0, id)
                + value.next_paragraph_style.as_ref().map_or(0, id) + value.character.owned_heap_bytes()
        }
        let mut bytes = id(&self.base_document) + id(&self.base_paragraph) + id(&self.base_character);
        for map in [&self.block_styles, &self.default_blocks] {
            bytes += style_map_heap_bytes(map.len(), std::mem::size_of::<(StyleId, BlockStyle)>())
                + map.iter().map(|(key, value)| id(key) + block(value)).sum::<usize>();
        }
        for map in [&self.character_styles, &self.default_characters] {
            bytes += style_map_heap_bytes(map.len(), std::mem::size_of::<(StyleId, CharacterStyle)>())
                + map.iter().map(|(key, value)| id(key) + character(value)).sum::<usize>();
        }
        for map in [&self.block_metadata, &self.character_metadata] {
            bytes += style_map_heap_bytes(map.len(), std::mem::size_of::<(StyleId, StyleDefinitionMetadata)>())
                + map.iter().map(|(key, value)| id(key) + value.display_name.capacity() + 16).sum::<usize>();
        }
        for set in [&self.deleted_configuration_blocks, &self.deleted_configuration_characters,
                    &self.deleted_source_blocks, &self.source_defined_blocks, &self.source_defined_characters,
                    &self.html_configuration_blocks, &self.html_configuration_characters] {
            bytes += style_map_heap_bytes(set.len(), std::mem::size_of::<StyleId>())
                + set.iter().map(id).sum::<usize>();
        }
        bytes + style_map_heap_bytes(self.source_character_defaults.len(), std::mem::size_of::<(StyleId, CharacterProperties)>())
            + self.source_character_defaults.iter().map(|(key, value)| id(key) + value.owned_heap_bytes()).sum::<usize>()
    }
}

// Imported-definition tracking is parser provenance, not a semantic declaration.
impl PartialEq for StyleSheet {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.base_document == other.base_document
            && self.base_paragraph == other.base_paragraph
            && self.base_character == other.base_character
            && self.block_styles == other.block_styles
            && self.character_styles == other.character_styles
            && self.block_metadata == other.block_metadata
            && self.character_metadata == other.character_metadata
            && self.deleted_configuration_blocks == other.deleted_configuration_blocks
            && self.deleted_configuration_characters == other.deleted_configuration_characters
            && self.deleted_source_blocks == other.deleted_source_blocks
            && self.default_blocks == other.default_blocks
            && self.default_characters == other.default_characters
            && self.html_configuration_blocks == other.html_configuration_blocks
            && self.html_configuration_characters == other.html_configuration_characters
    }
}

impl Default for StyleSheet {
    fn default() -> Self {
        let document: StyleId = "Document".into();
        let paragraph: StyleId = "Paragraph".into();
        let character: StyleId = "Character".into();
        let mut block_styles = BTreeMap::new();
        block_styles.insert(
            document.clone(),
            BlockStyle {
                id: document.clone(),
                based_on: None,
                next_paragraph_style: None,
                role: BlockRole::Document,
                character: CharacterProperties {
                    font_families: Some(vec!["SF Pro".to_owned()]),
                    size: Some(14.0),
                    weight: Some(400),
                    slant: Some(FontSlant::Upright),
                    // Unspecified color follows the application theme.
                    foreground: None,
                    underline: Some(false),
                    strikethrough: Some(false),
                    direction: Some(WritingDirection::Natural),
                    open_type_features: Some(BTreeMap::new()),
                    letter_spacing: Some(0.0),
                    baseline_shift: Some(0.0),
                    ..CharacterProperties::default()
                },
                block: BlockProperties {
                    // Product defaults remain a configuration decision. Zero
                    // keeps the generated base neutral while still making the
                    // canvas values explicit and overridable.
                    padding_top: Some(0.0),
                    padding_right: Some(0.0),
                    padding_bottom: Some(0.0),
                    padding_left: Some(0.0),
                    background: None,
                    ..BlockProperties::default()
                },
            },
        );
        block_styles.insert(
            paragraph.clone(),
            BlockStyle {
                id: paragraph.clone(),
                based_on: Some(document.clone()),
                next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: CharacterProperties::default(),
                block: BlockProperties {
                    spacing_before: Some(0.0),
                    spacing_after: Some(0.0),
                    line_spacing: Some(LineSpacing::Normal),
                    first_line_indent: Some(0.0),
                    leading_indent: Some(0.0),
                    trailing_indent: Some(0.0),
                    alignment: Some(ParagraphAlignment::Start),
                    base_direction: Some(WritingDirection::Natural),
                    ..BlockProperties::default()
                },
            },
        );
        for level in 1..=6 {
            let id: StyleId = format!("Heading{level}").as_str().into();
            block_styles.insert(
                id.clone(),
                BlockStyle {
                    id,
                    based_on: Some(paragraph.clone()),
                    next_paragraph_style: Some(paragraph.clone()),
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(26.0 - level as f32 * 2.0),
                        weight: Some(700),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        spacing_before: Some(10.0),
                        spacing_after: Some(5.0),
                        ..BlockProperties::default()
                    },
                },
            );
        }
        for ordered in [false, true] {
            for level in 1..=4 {
                let id = StyleId(format!(
                    "{}{level}",
                    if ordered {
                        "NumberedList"
                    } else {
                        "BulletedList"
                    }
                ));
                block_styles.insert(
                    id.clone(),
                    BlockStyle {
                        id,
                        based_on: Some(paragraph.clone()),
                        next_paragraph_style: None,
                        role: BlockRole::Paragraph,
                        character: CharacterProperties::default(),
                        block: BlockProperties {
                            leading_indent: Some(32.0 * level as f32),
                            first_line_indent: Some(0.0),
                            spacing_before: Some(0.0),
                            spacing_after: Some(0.0),
                            ..Default::default()
                        },
                    },
                );
            }
        }
        block_styles.insert(
            "Block quote".into(),
            BlockStyle {
                id: "Block quote".into(),
                based_on: Some(paragraph.clone()),
                next_paragraph_style: Some("Block quote".into()),
                role: BlockRole::Paragraph,
                character: CharacterProperties::default(),
                block: BlockProperties {
                    leading_indent: Some(32.0),
                    trailing_indent: Some(32.0),
                    ..Default::default()
                },
            },
        );
        let mut character_styles = BTreeMap::new();
        character_styles.insert(
            character.clone(),
            CharacterStyle {
                id: character.clone(),
                based_on: None,
                properties: CharacterProperties::default(),
            },
        );
        let code_properties = CharacterProperties {
            font_families: Some(vec!["monospace".to_owned()]),
            foreground: Some(Color {
                red: 0.0,
                green: 100.0 / 255.0,
                blue: 0.0,
                alpha: 1.0,
            }),
            ..CharacterProperties::default()
        };
        block_styles.insert(
            "Code Block".into(),
            BlockStyle {
                id: "Code Block".into(),
                based_on: Some(paragraph.clone()),
                next_paragraph_style: Some(paragraph.clone()),
                role: BlockRole::Paragraph,
                character: code_properties.clone(),
                block: BlockProperties::default(),
            },
        );
        character_styles.insert(
            "Code".into(),
            CharacterStyle {
                id: "Code".into(),
                based_on: Some(character.clone()),
                properties: code_properties,
            },
        );
        let mut block_metadata = BTreeMap::new();
        block_metadata.insert(
            document.clone(),
            StyleDefinitionMetadata::generated("Base Document"),
        );
        block_metadata.insert(
            paragraph.clone(),
            StyleDefinitionMetadata::generated("Base Paragraph"),
        );
        for level in 1..=6 {
            block_metadata.insert(
                StyleId(format!("Heading{level}")),
                StyleDefinitionMetadata::generated(format!("Heading {level}")),
            );
        }
        for ordered in [false, true] {
            for level in 1..=4 {
                let family = if ordered { "Numbered" } else { "Bulleted" };
                block_metadata.insert(
                    StyleId(format!("{family}List{level}")),
                    StyleDefinitionMetadata::generated(format!("{family} List {level}")),
                );
            }
        }
        block_metadata.insert(
            "Block quote".into(),
            StyleDefinitionMetadata::generated("Block quote"),
        );
        let mut character_metadata = BTreeMap::new();
        character_metadata.insert(
            character.clone(),
            StyleDefinitionMetadata::generated("Base Character"),
        );
        block_metadata.insert(
            "Code Block".into(),
            StyleDefinitionMetadata::generated("Code Block"),
        );
        character_metadata.insert("Code".into(), StyleDefinitionMetadata::generated("Code"));
        Self {
            revision: StyleSheetRevision(1),
            base_document: document,
            base_paragraph: paragraph,
            base_character: character,
            block_styles,
            character_styles,
            block_metadata,
            character_metadata,
            deleted_configuration_blocks: BTreeSet::new(),
            deleted_configuration_characters: BTreeSet::new(),
            deleted_source_blocks: BTreeSet::new(),
            source_defined_blocks: BTreeSet::new(),
            source_defined_characters: BTreeSet::new(),
            html_configuration_blocks: BTreeSet::new(),
            html_configuration_characters: BTreeSet::new(),
            source_character_defaults: BTreeMap::new(),
            default_blocks: BTreeMap::new(),
            default_characters: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StyleError {
    StyleSheetRevisionExhausted,
    StyleAlreadyExists(StyleId),
    CannotReplaceBaseStyle(StyleId),
    CannotRemoveBaseStyle(StyleId),
    InvalidBaseStyleDefinition(StyleId),
    StyleInUse(StyleId),
    UnknownStyle(StyleId),
    MissingParent(StyleId),
    InheritanceCycle(StyleId),
    IncompatibleBlockRole {
        style: StyleId,
        role: BlockRole,
        parent_role: BlockRole,
    },
    InapplicableBlockProperties {
        style: StyleId,
        role: BlockRole,
    },
    InapplicableNextParagraphStyle {
        style: StyleId,
        role: BlockRole,
    },
    InvalidNextParagraphStyle {
        style: StyleId,
        next: StyleId,
    },
    InvalidCharacterProperties(StyleId),
    InvalidBlockProperties(StyleId),
    InvalidDefinitionMetadata(StyleId),
    DefinitionNotGeneratedConfiguration {
        style: StyleId,
        origin: StyleDefinitionOrigin,
    },
    InapplicableStyleProperty {
        style: StyleId,
        property: StyleProperty,
    },
    InvalidStylePropertyValue {
        style: StyleId,
        property: StyleProperty,
    },
    InapplicableStyleRelationship(StyleId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCharacterStyle {
    pub font_families: Vec<String>,
    pub size: f32,
    pub weight: u16,
    pub base_weight: u16,
    pub bold: bool,
    pub slant: FontSlant,
    pub foreground: Color,
    /// No source or named layer declared a color; presentation uses its theme.
    pub foreground_is_default: bool,
    pub background: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
    pub language: Option<String>,
    pub direction: WritingDirection,
    pub open_type_features: BTreeMap<String, u32>,
    pub letter_spacing: f32,
    pub baseline_shift: f32,
}

impl Default for ResolvedCharacterStyle {
    fn default() -> Self {
        Self {
            font_families: vec!["SF Pro".to_owned()],
            size: 14.0,
            weight: 400,
            base_weight: 400,
            bold: false,
            slant: FontSlant::Upright,
            foreground: Color {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
            foreground_is_default: true,
            background: None,
            underline: false,
            strikethrough: false,
            language: None,
            direction: WritingDirection::Natural,
            open_type_features: BTreeMap::new(),
            letter_spacing: 0.0,
            baseline_shift: 0.0,
        }
    }
}

impl ResolvedCharacterStyle {
    /// Return exactly the normalized properties whose effective values differ.
    /// Callers can map these through [`StyleProperty::invalidation_effect`]
    /// instead of invalidating every layout layer for any style edit.
    pub fn changed_properties(&self, other: &Self) -> BTreeSet<StyleProperty> {
        let mut changed = BTreeSet::new();
        macro_rules! compare {
            ($field:ident, $property:expr) => {
                if self.$field != other.$field {
                    changed.insert($property);
                }
            };
        }
        compare!(font_families, StyleProperty::CharacterFontFamilies);
        compare!(size, StyleProperty::CharacterSize);
        compare!(weight, StyleProperty::CharacterWeight);
        compare!(bold, StyleProperty::CharacterBold);
        compare!(slant, StyleProperty::CharacterSlant);
        compare!(foreground, StyleProperty::CharacterForeground);
        compare!(foreground_is_default, StyleProperty::CharacterForeground);
        compare!(background, StyleProperty::CharacterBackground);
        compare!(underline, StyleProperty::CharacterUnderline);
        compare!(strikethrough, StyleProperty::CharacterStrikethrough);
        compare!(language, StyleProperty::CharacterLanguage);
        compare!(direction, StyleProperty::CharacterDirection);
        compare!(open_type_features, StyleProperty::CharacterOpenTypeFeatures);
        compare!(letter_spacing, StyleProperty::CharacterLetterSpacing);
        compare!(baseline_shift, StyleProperty::CharacterBaselineShift);
        changed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedDocumentStyle {
    pub background: Color,
    pub background_is_default: bool,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub character: ResolvedCharacterStyle,
}

impl Default for ResolvedDocumentStyle {
    fn default() -> Self {
        Self {
            background: Color {
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
            },
            background_is_default: true,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            character: ResolvedCharacterStyle::default(),
        }
    }
}

impl ResolvedDocumentStyle {
    pub fn changed_properties(&self, other: &Self) -> BTreeSet<StyleProperty> {
        let mut changed = self.character.changed_properties(&other.character);
        if self.background != other.background
            || self.background_is_default != other.background_is_default
        {
            changed.insert(StyleProperty::CanvasBackground);
        }
        if self.padding_top != other.padding_top {
            changed.insert(StyleProperty::CanvasPaddingTop);
        }
        if self.padding_right != other.padding_right {
            changed.insert(StyleProperty::CanvasPaddingRight);
        }
        if self.padding_bottom != other.padding_bottom {
            changed.insert(StyleProperty::CanvasPaddingBottom);
        }
        if self.padding_left != other.padding_left {
            changed.insert(StyleProperty::CanvasPaddingLeft);
        }
        changed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedParagraphStyle {
    pub spacing_before: f32,
    pub spacing_after: f32,
    pub line_spacing: LineSpacing,
    pub first_line_indent: f32,
    pub leading_indent: f32,
    pub trailing_indent: f32,
    pub alignment: ParagraphAlignment,
    pub base_direction: WritingDirection,
    pub character: ResolvedCharacterStyle,
}

/// A schema property whose resolved value participates in style dependency
/// tracking.  The key deliberately names the normalized property rather than
/// a source-format spelling such as a CSS property or RTF control word.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum StyleProperty {
    CanvasBackground,
    CanvasPaddingTop,
    CanvasPaddingRight,
    CanvasPaddingBottom,
    CanvasPaddingLeft,
    ParagraphSpacingBefore,
    ParagraphSpacingAfter,
    ParagraphLineSpacing,
    ParagraphFirstLineIndent,
    ParagraphLeadingIndent,
    ParagraphTrailingIndent,
    ParagraphAlignment,
    ParagraphBaseDirection,
    CharacterFontFamilies,
    CharacterSize,
    CharacterWeight,
    CharacterBold,
    CharacterSlant,
    CharacterForeground,
    CharacterBackground,
    CharacterUnderline,
    CharacterStrikethrough,
    CharacterLanguage,
    CharacterDirection,
    CharacterOpenTypeFeatures,
    CharacterLetterSpacing,
    CharacterBaselineShift,
}

/// Namespace of one normalized style definition. IDs are unique only within
/// their namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleNamespace {
    Block,
    Character,
}

/// Strongly typed value used by native single-property configuration edits.
#[derive(Clone, Debug, PartialEq)]
pub enum StylePropertyValue {
    Float(f32),
    FontWeight(u16),
    Boolean(bool),
    Color(Color),
    FontFamilies(Vec<String>),
    Text(String),
    FontSlant(FontSlant),
    WritingDirection(WritingDirection),
    OpenTypeFeatures(BTreeMap<String, u32>),
    LineSpacing(LineSpacing),
    ParagraphAlignment(ParagraphAlignment),
}

/// One field of an existing style definition. Each successful edit is a
/// standalone semantic intention and undo unit.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleDefinitionFieldEdit {
    SetDeclaration {
        property: StyleProperty,
        value: StylePropertyValue,
    },
    ClearDeclaration(StyleProperty),
    SetParent(Option<StyleId>),
    SetNextParagraphStyle(Option<StyleId>),
    SetDisplayName(String),
}

/// Smallest downstream layer invalidated when one normalized property changes.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum StyleInvalidationEffect {
    Paint,
    Shaping,
    ParagraphLayout,
    DocumentLayout,
    ViewUsableWidth,
}

impl StyleProperty {
    pub fn invalidation_effect(self) -> StyleInvalidationEffect {
        match self {
            Self::CanvasBackground
            | Self::CharacterForeground
            | Self::CharacterBackground
            | Self::CharacterUnderline
            | Self::CharacterStrikethrough => StyleInvalidationEffect::Paint,
            Self::CanvasPaddingLeft | Self::CanvasPaddingRight => {
                StyleInvalidationEffect::ViewUsableWidth
            }
            Self::CanvasPaddingTop | Self::CanvasPaddingBottom => {
                StyleInvalidationEffect::DocumentLayout
            }
            Self::ParagraphSpacingBefore
            | Self::ParagraphSpacingAfter
            | Self::ParagraphLineSpacing
            | Self::ParagraphFirstLineIndent
            | Self::ParagraphLeadingIndent
            | Self::ParagraphTrailingIndent
            | Self::ParagraphAlignment => StyleInvalidationEffect::ParagraphLayout,
            Self::ParagraphBaseDirection => StyleInvalidationEffect::Shaping,
            Self::CharacterFontFamilies
            | Self::CharacterSize
            | Self::CharacterWeight
            | Self::CharacterBold
            | Self::CharacterSlant
            | Self::CharacterLanguage
            | Self::CharacterDirection
            | Self::CharacterOpenTypeFeatures
            | Self::CharacterLetterSpacing
            | Self::CharacterBaselineShift => StyleInvalidationEffect::Shaping,
        }
    }
}

/// The three property groups below are the single definition of which
/// properties belong to which style scope. The C ABI exports the same grouping
/// and reads it from here rather than restating it.
pub(crate) const CANVAS_STYLE_PROPERTIES: [StyleProperty; 5] = [
    StyleProperty::CanvasBackground,
    StyleProperty::CanvasPaddingTop,
    StyleProperty::CanvasPaddingRight,
    StyleProperty::CanvasPaddingBottom,
    StyleProperty::CanvasPaddingLeft,
];

pub(crate) const PARAGRAPH_STYLE_PROPERTIES: [StyleProperty; 8] = [
    StyleProperty::ParagraphSpacingBefore,
    StyleProperty::ParagraphSpacingAfter,
    StyleProperty::ParagraphLineSpacing,
    StyleProperty::ParagraphFirstLineIndent,
    StyleProperty::ParagraphLeadingIndent,
    StyleProperty::ParagraphTrailingIndent,
    StyleProperty::ParagraphAlignment,
    StyleProperty::ParagraphBaseDirection,
];

pub(crate) const CHARACTER_STYLE_PROPERTIES: [StyleProperty; 14] = [
    StyleProperty::CharacterFontFamilies,
    StyleProperty::CharacterSize,
    StyleProperty::CharacterWeight,
    StyleProperty::CharacterBold,
    StyleProperty::CharacterSlant,
    StyleProperty::CharacterForeground,
    StyleProperty::CharacterBackground,
    StyleProperty::CharacterUnderline,
    StyleProperty::CharacterStrikethrough,
    StyleProperty::CharacterLanguage,
    StyleProperty::CharacterDirection,
    StyleProperty::CharacterOpenTypeFeatures,
    StyleProperty::CharacterLetterSpacing,
    StyleProperty::CharacterBaselineShift,
];

/// The normalized declaration layer which supplied a resolved property's
/// current value.  Source adapters can associate these normalized layers with
/// their own lossless syntax provenance without leaking format-specific types
/// into the generic resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StyleContributionOrigin {
    EngineEmergency,
    BlockStyle(StyleId),
    CharacterStyle(StyleId),
    DirectDocumentCanvas,
    DirectDocumentCharacter,
    DirectParagraph,
    DirectParagraphCharacter,
    DirectCharacter,
}

/// A stable style-definition dependency consulted while resolving a property.
/// The namespace is retained because block and character style IDs are opaque
/// and are permitted to contain the same token.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum StyleDependency {
    Block(StyleId),
    Character(StyleId),
}

/// Contribution metadata for one complete resolved property.
///
/// `dependencies` includes style definitions whose ancestry was consulted,
/// including definitions which currently omit this property.  Adding or
/// clearing a declaration in any such definition can therefore invalidate the
/// cached result without conservatively invalidating unrelated style trees.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleContribution {
    pub winner: StyleContributionOrigin,
    pub dependencies: Vec<StyleDependency>,
}

/// A resolved style together with complete, property-keyed cascade metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle<T> {
    pub value: T,
    contributions: BTreeMap<StyleProperty, StyleContribution>,
}

impl<T> ResolvedStyle<T> {
    pub fn contribution(&self, property: StyleProperty) -> Option<&StyleContribution> {
        self.contributions.get(&property)
    }

    pub fn contributions(
        &self,
    ) -> impl ExactSizeIterator<Item = (StyleProperty, &StyleContribution)> {
        self.contributions
            .iter()
            .map(|(property, contribution)| (*property, contribution))
    }
}

/// Immutable reverse-dependency closure for one exact style-sheet revision.
/// Queries visit only affected descendants after the index has been built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleDependencyIndex {
    pub revision: StyleSheetRevision,
    block_dependents: BTreeMap<StyleId, Vec<StyleId>>,
    character_dependents: BTreeMap<StyleId, Vec<StyleId>>,
}

impl StyleDependencyIndex {
    /// The changed block style and every transitive derived style, in stable ID
    /// order. Unknown IDs return `None` rather than implying a
    /// document-wide invalidation.
    pub fn block_dependents(&self, style: &StyleId) -> Option<&[StyleId]> {
        self.block_dependents.get(style).map(Vec::as_slice)
    }

    /// The changed character style followed by every transitive derived style,
    /// in stable ID order.
    pub fn character_dependents(&self, style: &StyleId) -> Option<&[StyleId]> {
        self.character_dependents.get(style).map(Vec::as_slice)
    }
}

impl Default for ResolvedParagraphStyle {
    fn default() -> Self {
        Self {
            spacing_before: 0.0,
            spacing_after: 0.0,
            line_spacing: LineSpacing::Normal,
            first_line_indent: 0.0,
            leading_indent: 0.0,
            trailing_indent: 0.0,
            alignment: ParagraphAlignment::Start,
            base_direction: WritingDirection::Natural,
            character: ResolvedCharacterStyle::default(),
        }
    }
}

impl ResolvedParagraphStyle {
    pub fn changed_properties(&self, other: &Self) -> BTreeSet<StyleProperty> {
        let mut changed = self.character.changed_properties(&other.character);
        macro_rules! compare {
            ($field:ident, $property:expr) => {
                if self.$field != other.$field {
                    changed.insert($property);
                }
            };
        }
        compare!(spacing_before, StyleProperty::ParagraphSpacingBefore);
        compare!(spacing_after, StyleProperty::ParagraphSpacingAfter);
        compare!(line_spacing, StyleProperty::ParagraphLineSpacing);
        compare!(first_line_indent, StyleProperty::ParagraphFirstLineIndent);
        compare!(leading_indent, StyleProperty::ParagraphLeadingIndent);
        compare!(trailing_indent, StyleProperty::ParagraphTrailingIndent);
        compare!(alignment, StyleProperty::ParagraphAlignment);
        compare!(base_direction, StyleProperty::ParagraphBaseDirection);
        changed
    }
}

impl StyleSheet {
    pub(crate) fn install_html_source_styles(&mut self) {
        for (name, rgb) in [
            ("Brackets", [0.48, 0.48, 0.52]),
            ("Tag name", [0.62, 0.36, 0.80]),
            ("Attribute key", [0.22, 0.57, 0.68]),
            ("Attribute value", [0.29, 0.58, 0.31]),
            ("Equals", [0.55, 0.48, 0.40]),
            ("Entity", [0.76, 0.47, 0.20]),
            ("Uninterpreted", [0.50, 0.52, 0.55]),
        ] {
            let id = StyleId(format!("* HTML {name}"));
            self.character_styles
                .entry(id.clone())
                .or_insert_with(|| CharacterStyle {
                    id: id.clone(),
                    based_on: None,
                    properties: CharacterProperties {
                        foreground: Some(Color {
                            red: rgb[0],
                            green: rgb[1],
                            blue: rgb[2],
                            alpha: 1.0,
                        }),
                        ..Default::default()
                    },
                });
            self.character_metadata
                .entry(id.clone())
                .or_insert_with(|| StyleDefinitionMetadata::generated(id.0));
        }
    }
    /// Look up one immutable block-style definition by its stable ID.
    pub fn block_style(&self, id: &StyleId) -> Option<&BlockStyle> {
        self.block_styles.get(id)
    }

    /// Iterate immutable block-style definitions in stable ID order.
    pub(crate) fn configuration_deleted(&self, id: &StyleId, block: bool) -> bool {
        if block {
            self.deleted_configuration_blocks.contains(id)
        } else {
            self.deleted_configuration_characters.contains(id)
        }
    }
    pub(crate) fn retain_internal_styles(&mut self, previous: &Self) {
        for style in previous
            .character_styles()
            .filter(|style| style.id.is_internal())
        {
            self.character_styles
                .insert(style.id.clone(), style.clone());
            if let Some(metadata) = previous.character_style_metadata(&style.id) {
                self.character_metadata
                    .insert(style.id.clone(), metadata.clone());
            }
        }
    }
    pub(crate) fn retain_configuration_deletions(&mut self, previous: &Self) {
        self.retain_defaults(previous);
        self.retain_internal_styles(previous);
        self.retain_html_native_configuration(previous);
        self.deleted_configuration_blocks = previous.deleted_configuration_blocks.clone();
        self.deleted_configuration_characters = previous.deleted_configuration_characters.clone();
        for id in &self.deleted_configuration_blocks {
            self.block_styles.remove(id);
            self.block_metadata.remove(id);
        }
        for id in &self.deleted_configuration_characters {
            self.character_styles.remove(id);
            self.character_metadata.remove(id);
        }
    }

    pub fn block_styles(&self) -> impl ExactSizeIterator<Item = &BlockStyle> + '_ {
        self.block_styles.values()
    }

    /// Prose defaults approximate the common one-em collapsed HTML
    /// paragraph gap using two half-em sides in Viem's additive spacing model.
    /// Plain text and RTF retain their adapter-specific defaults.
    pub(crate) fn for_format(format: super::Format) -> Self {
        let mut sheet = Self::default();
        if matches!(
            format,
            super::Format::Markdown
                | super::Format::MarkdownSource
                | super::Format::Html
                | super::Format::HtmlSource
        ) {
            let paragraph = sheet.block_styles.get_mut(&sheet.base_paragraph).unwrap();
            paragraph.block.spacing_before = Some(7.0);
            paragraph.block.spacing_after = Some(7.0);
            if format.is_markdown() {
                sheet
                    .block_styles
                    .get_mut(&StyleId("Code Block".into()))
                    .unwrap()
                    .block
                    .leading_indent = Some(32.0);
            }
        }
        sheet
    }

    pub fn block_style_count(&self) -> usize {
        self.block_styles.len()
    }

    /// Select a structural family without manufacturing definitions for deep
    /// nesting. Authored legacy list definitions retain their original role.
    pub fn list_style_id(&self, ordered: bool, zero_based_level: u8) -> StyleId {
        let legacy = StyleId(format!("List{}", u16::from(zero_based_level) + 1));
        if self.source_defined_blocks.contains(&legacy) {
            return legacy;
        }
        StyleId(format!(
            "{}{}",
            if ordered {
                "NumberedList"
            } else {
                "BulletedList"
            },
            u16::from(zero_based_level).min(3) + 1
        ))
    }

    /// Restore a legacy ListN baseline while reading a saved legacy rule.
    pub(crate) fn ensure_list_level(&mut self, level: u16) {
        let first = self
            .block_styles
            .get(&StyleId("List1".into()))
            .or_else(|| self.block_styles.get(&StyleId::from("BulletedList1")))
            .map(|style| style.block.clone())
            .unwrap_or_default();
        let step = first.leading_indent.unwrap_or(32.0);
        for level in 1..=level {
            let id = StyleId(format!("List{level}"));
            if self.block_styles.contains_key(&id)
                || self.deleted_configuration_blocks.contains(&id)
                || self.deleted_source_blocks.contains(&id)
            {
                continue;
            }
            self.block_styles.insert(
                id.clone(),
                BlockStyle {
                    id: id.clone(),
                    based_on: Some(self.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties::default(),
                    block: BlockProperties {
                        leading_indent: Some(step * f32::from(level)),
                        first_line_indent: Some(first.first_line_indent.unwrap_or(0.0)),
                        spacing_before: first.spacing_before,
                        spacing_after: first.spacing_after,
                        ..Default::default()
                    },
                },
            );
            self.block_metadata.insert(
                id,
                StyleDefinitionMetadata {
                    display_name: format!("List Level {level}"),
                    origin: self
                        .block_style_metadata(&self.base_document)
                        .filter(|metadata| metadata.origin == StyleDefinitionOrigin::SourceBacked)
                        .map_or(StyleDefinitionOrigin::GeneratedConfiguration, |metadata| {
                            metadata.origin
                        }),
                },
            );
        }
    }

    pub fn block_style_metadata(&self, id: &StyleId) -> Option<&StyleDefinitionMetadata> {
        self.block_metadata.get(id)
    }

    /// Look up one immutable character-style definition by its stable ID.
    pub fn character_style(&self, id: &StyleId) -> Option<&CharacterStyle> {
        self.character_styles.get(id)
    }

    /// Iterate immutable character-style definitions in stable ID order.
    pub fn character_styles(&self) -> impl ExactSizeIterator<Item = &CharacterStyle> + '_ {
        self.character_styles.values()
    }

    pub fn character_style_count(&self) -> usize {
        self.character_styles.len()
    }

    /// Read immutable core-owned metadata for a character definition.
    pub fn character_style_metadata(&self, id: &StyleId) -> Option<&StyleDefinitionMetadata> {
        self.character_metadata.get(id)
    }

    /// Build one immutable definition replacement for a core-authorized
    /// generated-configuration field edit. The current metadata, namespace,
    /// and property kind are validated before any candidate sheet is changed.
    pub fn prepare_generated_field_edit(
        &self,
        namespace: StyleNamespace,
        id: &StyleId,
        edit: &StyleDefinitionFieldEdit,
    ) -> Result<StyleDefinitionEdit, StyleError> {
        self.prepare_definition_field_edit(
            namespace,
            id,
            edit,
            StyleDefinitionOrigin::GeneratedConfiguration,
        )
    }

    pub(crate) fn prepare_source_field_edit(
        &self,
        namespace: StyleNamespace,
        id: &StyleId,
        edit: &StyleDefinitionFieldEdit,
    ) -> Result<StyleDefinitionEdit, StyleError> {
        self.prepare_definition_field_edit(namespace, id, edit, StyleDefinitionOrigin::SourceBacked)
    }

    fn prepare_definition_field_edit(
        &self,
        namespace: StyleNamespace,
        id: &StyleId,
        edit: &StyleDefinitionFieldEdit,
        required_origin: StyleDefinitionOrigin,
    ) -> Result<StyleDefinitionEdit, StyleError> {
        let metadata = match namespace {
            StyleNamespace::Block => self.block_style_metadata(id),
            StyleNamespace::Character => self.character_style_metadata(id),
        }
        .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
        if metadata.origin != required_origin {
            return Err(StyleError::DefinitionNotGeneratedConfiguration {
                style: id.clone(),
                origin: metadata.origin,
            });
        }
        if let StyleDefinitionFieldEdit::SetDisplayName(display_name) = edit {
            let metadata = StyleDefinitionMetadata {
                display_name: display_name.clone(),
                origin: metadata.origin,
            };
            validate_definition_metadata(id, &metadata)?;
            return Ok(StyleDefinitionEdit::UpdateMetadata {
                namespace,
                id: id.clone(),
                metadata,
            });
        }

        match namespace {
            StyleNamespace::Block => {
                let mut style = self
                    .block_style(id)
                    .cloned()
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                apply_block_field_edit(&mut style, edit)?;
                Ok(StyleDefinitionEdit::UpdateBlock(style))
            }
            StyleNamespace::Character => {
                let mut style = self
                    .character_style(id)
                    .cloned()
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                apply_character_field_edit(&mut style, edit)?;
                Ok(StyleDefinitionEdit::UpdateCharacter(style))
            }
        }
    }

    pub(crate) fn set_configuration_revision(&mut self, revision: StyleSheetRevision) {
        self.revision = revision;
    }

    /// Apply one explicit configuration edit atomically. `has_assignment`
    /// is consulted only for deletions and must describe assignments in the
    /// owning formatted snapshot. A successful no-op update retains the
    /// existing revision and returns `false`.
    pub(crate) fn apply_configuration_edit(
        &mut self,
        edit: &StyleDefinitionEdit,
        revision: StyleSheetRevision,
        has_assignment: bool,
    ) -> Result<bool, StyleError> {
        let changed = self.apply_definition_edit(
            edit,
            revision,
            has_assignment,
            StyleDefinitionOrigin::GeneratedConfiguration,
        )?;
        if changed {
            match edit {
                StyleDefinitionEdit::DeleteBlock(id) => {
                    self.deleted_configuration_blocks.insert(id.clone());
                }
                StyleDefinitionEdit::DeleteCharacter(id) => {
                    self.deleted_configuration_characters.insert(id.clone());
                }
                StyleDefinitionEdit::InsertBlock { style, .. } => {
                    self.deleted_configuration_blocks.remove(&style.id);
                }
                StyleDefinitionEdit::InsertCharacter { style, .. } => {
                    self.deleted_configuration_characters.remove(&style.id);
                }
                _ => {}
            }
        }
        Ok(changed)
    }

    pub(crate) fn apply_source_edit(
        &mut self,
        edit: &StyleDefinitionEdit,
        revision: StyleSheetRevision,
        has_assignment: bool,
    ) -> Result<bool, StyleError> {
        let changed = self.apply_definition_edit(
            edit,
            revision,
            has_assignment,
            StyleDefinitionOrigin::SourceBacked,
        )?;
        if changed {
            match edit {
                StyleDefinitionEdit::DeleteBlock(id) if Self::builtin_block(id) => {
                    self.deleted_source_blocks.insert(id.clone());
                }
                StyleDefinitionEdit::InsertBlock { style, .. } => {
                    self.deleted_source_blocks.remove(&style.id);
                }
                _ => {}
            }
        }
        Ok(changed)
    }

    fn apply_definition_edit(
        &mut self,
        edit: &StyleDefinitionEdit,
        revision: StyleSheetRevision,
        has_assignment: bool,
        required_origin: StyleDefinitionOrigin,
    ) -> Result<bool, StyleError> {
        if edit.style_id().is_internal() {
            let allowed = match edit {
                StyleDefinitionEdit::UpdateCharacter(style) => self
                    .character_style(&style.id)
                    .is_some_and(|old| old.based_on == style.based_on),
                _ => false,
            };
            if !allowed {
                return Err(StyleError::InvalidDefinitionMetadata(
                    edit.style_id().clone(),
                ));
            }
        }
        let (origin, id) = match edit {
            StyleDefinitionEdit::InsertBlock { style, metadata } => (metadata.origin, &style.id),
            StyleDefinitionEdit::InsertCharacter { style, metadata } => {
                (metadata.origin, &style.id)
            }
            StyleDefinitionEdit::UpdateBlock(style) => (
                self.block_style_metadata(&style.id)
                    .ok_or_else(|| StyleError::UnknownStyle(style.id.clone()))?
                    .origin,
                &style.id,
            ),
            StyleDefinitionEdit::DeleteBlock(id) => (
                self.block_style_metadata(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                    .origin,
                id,
            ),
            StyleDefinitionEdit::UpdateCharacter(style) => (
                self.character_style_metadata(&style.id)
                    .ok_or_else(|| StyleError::UnknownStyle(style.id.clone()))?
                    .origin,
                &style.id,
            ),
            StyleDefinitionEdit::DeleteCharacter(id) => (
                self.character_style_metadata(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                    .origin,
                id,
            ),
            StyleDefinitionEdit::UpdateMetadata { namespace, id, .. } => (
                match namespace {
                    StyleNamespace::Block => self.block_style_metadata(id),
                    StyleNamespace::Character => self.character_style_metadata(id),
                }
                .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                .origin,
                id,
            ),
        };
        if origin != required_origin {
            return Err(StyleError::DefinitionNotGeneratedConfiguration {
                style: id.clone(),
                origin,
            });
        }
        let mut candidate = self.clone();
        let changed = match edit {
            StyleDefinitionEdit::InsertBlock { style, metadata } => {
                if candidate.block_styles.contains_key(&style.id) {
                    return Err(StyleError::StyleAlreadyExists(style.id.clone()));
                }
                candidate.insert_block_style(style.clone(), metadata.clone())?;
                true
            }
            StyleDefinitionEdit::UpdateBlock(style) => {
                let old = candidate
                    .block_styles
                    .get(&style.id)
                    .ok_or_else(|| StyleError::UnknownStyle(style.id.clone()))?;
                if old == style {
                    false
                } else {
                    candidate.replace_block_style(style.clone())?;
                    true
                }
            }
            StyleDefinitionEdit::DeleteBlock(id) => {
                if !candidate.block_styles.contains_key(id) {
                    return Err(StyleError::UnknownStyle(id.clone()));
                }
                candidate.remove_block_style(id, has_assignment)?;
                true
            }
            StyleDefinitionEdit::InsertCharacter { style, metadata } => {
                if candidate.character_styles.contains_key(&style.id) {
                    return Err(StyleError::StyleAlreadyExists(style.id.clone()));
                }
                candidate.insert_character_style(style.clone(), metadata.clone())?;
                true
            }
            StyleDefinitionEdit::UpdateCharacter(style) => {
                let old = candidate
                    .character_styles
                    .get(&style.id)
                    .ok_or_else(|| StyleError::UnknownStyle(style.id.clone()))?;
                if old == style {
                    false
                } else {
                    candidate.replace_character_style(style.clone())?;
                    true
                }
            }
            StyleDefinitionEdit::DeleteCharacter(id) => {
                if !candidate.character_styles.contains_key(id) {
                    return Err(StyleError::UnknownStyle(id.clone()));
                }
                candidate.remove_character_style(id, has_assignment)?;
                true
            }
            StyleDefinitionEdit::UpdateMetadata {
                namespace,
                id,
                metadata,
            } => {
                if metadata.origin != origin {
                    return Err(StyleError::InvalidDefinitionMetadata(id.clone()));
                }
                validate_definition_metadata(id, metadata)?;
                let current = match namespace {
                    StyleNamespace::Block => candidate.block_metadata.get_mut(id),
                    StyleNamespace::Character => candidate.character_metadata.get_mut(id),
                }
                .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                if current == metadata {
                    false
                } else {
                    current.clone_from(metadata);
                    true
                }
            }
        };
        if !changed {
            return Ok(false);
        }
        candidate.revision = revision;
        *self = candidate;
        Ok(true)
    }

    /// Import a complete native definition graph before validating references,
    /// so source order never controls parent/next-style resolution.
    pub(crate) fn install_source_definitions(
        &mut self,
        definitions: &[StyleDefinitionEdit],
    ) -> Result<(), StyleError> {
        let mut candidate = self.clone();
        for definition in definitions {
            match definition {
                StyleDefinitionEdit::InsertBlock { style, metadata } => {
                    validate_definition_metadata(&style.id, metadata)?;
                    candidate
                        .block_styles
                        .insert(style.id.clone(), style.clone());
                    candidate
                        .block_metadata
                        .insert(style.id.clone(), metadata.clone());
                    candidate.deleted_source_blocks.remove(&style.id);
                    if metadata.origin == StyleDefinitionOrigin::SourceBacked {
                        candidate.source_defined_blocks.insert(style.id.clone());
                    }
                }
                StyleDefinitionEdit::InsertCharacter { style, metadata } => {
                    if metadata.origin == StyleDefinitionOrigin::SourceBacked {
                        candidate.source_defined_characters.insert(style.id.clone());
                    }
                    validate_definition_metadata(&style.id, metadata)?;
                    candidate
                        .character_styles
                        .insert(style.id.clone(), style.clone());
                    candidate
                        .character_metadata
                        .insert(style.id.clone(), metadata.clone());
                }
                StyleDefinitionEdit::DeleteBlock(id) => {
                    if id == &candidate.base_document || id == &candidate.base_paragraph {
                        return Err(StyleError::CannotRemoveBaseStyle(id.clone()));
                    }
                    candidate.block_styles.remove(id);
                    candidate.block_metadata.remove(id);
                    candidate.deleted_source_blocks.insert(id.clone());
                }
                _ => {
                    return Err(StyleError::InvalidDefinitionMetadata(
                        definition.style_id().clone(),
                    ))
                }
            }
        }
        for style in candidate.block_styles.values() {
            validate_character_properties(&style.id, &style.character)?;
            validate_block_properties(style)?;
            if style.id != candidate.base_document {
                candidate.validate_block_parent(style)?;
            }
            candidate.validate_next_paragraph_style(style)?;
        }
        for style in candidate.character_styles.values() {
            validate_character_properties(&style.id, &style.properties)?;
            if let Some(parent) = &style.based_on {
                if !candidate.character_styles.contains_key(parent) {
                    return Err(StyleError::UnknownStyle(parent.clone()));
                }
            }
        }
        candidate.validate_block_cycles()?;
        candidate.validate_character_cycles()?;
        *self = candidate;
        Ok(())
    }

    pub(crate) fn mark_html_base_styles_source_backed(&mut self) {
        if let Some(metadata) = self.character_metadata.get_mut(&self.base_character) {
            metadata.origin = StyleDefinitionOrigin::SourceBacked;
        }
        for (id, metadata) in &mut self.block_metadata {
            if id == &self.base_document
                || id == &self.base_paragraph
                || id.0.starts_with("Heading")
                || Self::builtin_block(id)
            {
                metadata.origin = StyleDefinitionOrigin::SourceBacked;
            }
        }
    }

    pub(crate) fn builtin_block(id: &StyleId) -> bool {
        id.is_internal_list()
            || id.0 == "Block quote"
            || ["Heading", "List"].into_iter().any(|prefix| {
                id.0.strip_prefix(prefix)
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some_and(|level| {
                        (1..=if prefix == "Heading" { 6 } else { 256 }).contains(&level)
                            && id.0 == format!("{prefix}{level}")
                    })
            })
    }
    pub(crate) fn deleted_source_blocks(&self) -> impl Iterator<Item = &StyleId> {
        self.deleted_source_blocks.iter()
    }

    /// A source-backed deletion removes references in the same transaction.
    /// Children inherit from the deleted style's parent; following-paragraph
    /// references use its declared successor or Base Paragraph.
    pub(crate) fn rebase_source_references_for_delete(
        &mut self,
        edit: &StyleDefinitionEdit,
        revision: StyleSheetRevision,
    ) -> Result<(), StyleError> {
        match edit {
            StyleDefinitionEdit::DeleteCharacter(id) => {
                let parent = self
                    .character_style(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?
                    .based_on
                    .clone();
                let changed = self
                    .character_styles()
                    .filter(|style| style.based_on.as_ref() == Some(id))
                    .cloned()
                    .collect::<Vec<_>>();
                for mut style in changed {
                    style.based_on = parent.clone();
                    let origin = self.character_style_metadata(&style.id).unwrap().origin;
                    self.apply_definition_edit(
                        &StyleDefinitionEdit::UpdateCharacter(style),
                        revision,
                        false,
                        origin,
                    )?;
                }
            }
            StyleDefinitionEdit::DeleteBlock(id) => {
                let deleted = self
                    .block_style(id)
                    .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                let parent = deleted.based_on.clone();
                let next = deleted
                    .next_paragraph_style
                    .clone()
                    .filter(|next| next != id)
                    .or_else(|| Some(self.base_paragraph.clone()));
                let changed = self
                    .block_styles()
                    .filter(|style| {
                        &style.id != id
                            && (style.based_on.as_ref() == Some(id)
                                || style.next_paragraph_style.as_ref() == Some(id))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                for mut style in changed {
                    if style.based_on.as_ref() == Some(id) {
                        style.based_on = parent.clone();
                    }
                    if style.next_paragraph_style.as_ref() == Some(id) {
                        style.next_paragraph_style = next.clone();
                    }
                    let origin = self.block_style_metadata(&style.id).unwrap().origin;
                    self.apply_definition_edit(
                        &StyleDefinitionEdit::UpdateBlock(style),
                        revision,
                        false,
                        origin,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Build the transitive reverse-inheritance index for this immutable style
    /// sheet revision.  A resolver cache can retain this value beside entries
    /// keyed by [`StyleSheetRevision`] and answer definition invalidations
    /// without scanning formatted content.
    pub fn dependency_index(&self) -> StyleDependencyIndex {
        let mut block_dependents = self
            .block_styles
            .keys()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for descendant in self.block_styles.keys() {
            let mut current = Some(descendant);
            while let Some(id) = current {
                block_dependents
                    .get_mut(id)
                    .expect("validated block ancestry remains in the style sheet")
                    .push(descendant.clone());
                current = self
                    .block_styles
                    .get(id)
                    .and_then(|style| style.based_on.as_ref());
            }
        }

        let mut character_dependents = self
            .character_styles
            .keys()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for descendant in self.character_styles.keys() {
            let mut current = Some(descendant);
            while let Some(id) = current {
                character_dependents
                    .get_mut(id)
                    .expect("validated character ancestry remains in the style sheet")
                    .push(descendant.clone());
                current = self
                    .character_styles
                    .get(id)
                    .and_then(|style| style.based_on.as_ref());
            }
        }

        StyleDependencyIndex {
            revision: self.revision,
            block_dependents,
            character_dependents,
        }
    }

    /// Insert a style defined by application or generated configuration. Source
    /// adapters may retain malformed native definitions separately, but the
    /// normalized sheet never accepts a cycle or an inapplicable declaration.
    pub fn insert_block_style(
        &mut self,
        style: BlockStyle,
        metadata: StyleDefinitionMetadata,
    ) -> Result<(), StyleError> {
        if style.id == self.base_document || style.id == self.base_paragraph {
            return Err(StyleError::CannotReplaceBaseStyle(style.id));
        }
        validate_definition_metadata(&style.id, &metadata)?;
        validate_character_properties(&style.id, &style.character)?;
        validate_block_properties(&style)?;
        self.validate_block_parent(&style)?;
        self.validate_next_paragraph_style(&style)?;
        let next_revision = self.next_revision()?;

        let old = self.block_styles.insert(style.id.clone(), style.clone());
        if let Err(error) = self.validate_block_cycles() {
            if let Some(old) = old {
                self.block_styles.insert(old.id.clone(), old);
            } else {
                self.block_styles.remove(&style.id);
            }
            return Err(error);
        }
        self.block_metadata.insert(style.id.clone(), metadata);
        self.revision = next_revision;
        Ok(())
    }

    pub fn insert_character_style(
        &mut self,
        style: CharacterStyle,
        metadata: StyleDefinitionMetadata,
    ) -> Result<(), StyleError> {
        if style.id == self.base_character {
            return Err(StyleError::CannotReplaceBaseStyle(style.id));
        }
        validate_definition_metadata(&style.id, &metadata)?;
        validate_character_properties(&style.id, &style.properties)?;
        if let Some(parent) = &style.based_on {
            if !self.character_styles.contains_key(parent) {
                return Err(StyleError::MissingParent(parent.clone()));
            }
        } else if !style.id.is_internal() {
            return Err(StyleError::MissingParent(style.id.clone()));
        }
        let next_revision = self.next_revision()?;
        let old = self
            .character_styles
            .insert(style.id.clone(), style.clone());
        if let Err(error) = self.validate_character_cycles() {
            if let Some(old) = old {
                self.character_styles.insert(old.id.clone(), old);
            } else {
                self.character_styles.remove(&style.id);
            }
            return Err(error);
        }
        self.character_metadata.insert(style.id.clone(), metadata);
        self.revision = next_revision;
        Ok(())
    }

    pub fn remove_block_style(
        &mut self,
        id: &StyleId,
        has_content_assignment: bool,
    ) -> Result<(), StyleError> {
        if id == &self.base_document || id == &self.base_paragraph {
            return Err(StyleError::CannotRemoveBaseStyle(id.clone()));
        }
        if has_content_assignment
            || self.block_styles.values().any(|style| {
                &style.id != id
                    && (style.based_on.as_ref() == Some(id)
                        || style.next_paragraph_style.as_ref() == Some(id))
            })
        {
            return Err(StyleError::StyleInUse(id.clone()));
        }
        if self.block_styles.contains_key(id) {
            let next_revision = self.next_revision()?;
            self.block_styles.remove(id);
            self.block_metadata.remove(id);
            self.revision = next_revision;
        }
        Ok(())
    }

    pub fn remove_character_style(
        &mut self,
        id: &StyleId,
        has_content_assignment: bool,
    ) -> Result<(), StyleError> {
        if id == &self.base_character {
            return Err(StyleError::CannotRemoveBaseStyle(id.clone()));
        }
        if has_content_assignment
            || self
                .character_styles
                .values()
                .any(|style| style.based_on.as_ref() == Some(id))
        {
            return Err(StyleError::StyleInUse(id.clone()));
        }
        if self.character_styles.contains_key(id) {
            let next_revision = self.next_revision()?;
            self.character_styles.remove(id);
            self.character_metadata.remove(id);
            self.revision = next_revision;
        }
        Ok(())
    }

    pub fn resolve_document_style(
        &self,
        assigned: &StyleId,
        direct_canvas: &BlockProperties,
        direct_character: &CharacterProperties,
    ) -> Result<ResolvedDocumentStyle, StyleError> {
        validate_character_properties(assigned, direct_character)?;
        validate_block_property_values(assigned, direct_canvas)?;
        if declares_paragraph_properties(direct_canvas) {
            return Err(StyleError::InapplicableBlockProperties {
                style: assigned.clone(),
                role: BlockRole::Document,
            });
        }
        let chain = self.block_chain(assigned, BlockRole::Document)?;
        let mut resolved = ResolvedDocumentStyle::default();
        for style in chain {
            apply_document_properties(&mut resolved, &style.block);
            apply_character_properties(&mut resolved.character, &style.character);
        }
        apply_document_properties(&mut resolved, direct_canvas);
        apply_character_properties(&mut resolved.character, direct_character);
        Ok(resolved)
    }

    /// Resolve the document root while retaining the winner and dependency
    /// chain for every complete canvas and character property.
    pub fn resolve_document_style_with_contributions(
        &self,
        assigned: &StyleId,
        direct_canvas: &BlockProperties,
        direct_character: &CharacterProperties,
    ) -> Result<ResolvedStyle<ResolvedDocumentStyle>, StyleError> {
        validate_character_properties(assigned, direct_character)?;
        validate_block_property_values(assigned, direct_canvas)?;
        if declares_paragraph_properties(direct_canvas) {
            return Err(StyleError::InapplicableBlockProperties {
                style: assigned.clone(),
                role: BlockRole::Document,
            });
        }

        let chain = self.block_chain(assigned, BlockRole::Document)?;
        let mut value = ResolvedDocumentStyle::default();
        let mut contributions = emergency_contributions(
            CANVAS_STYLE_PROPERTIES
                .into_iter()
                .chain(CHARACTER_STYLE_PROPERTIES),
        );
        for style in chain {
            add_dependency(
                &mut contributions,
                CANVAS_STYLE_PROPERTIES
                    .into_iter()
                    .chain(CHARACTER_STYLE_PROPERTIES),
                StyleDependency::Block(style.id.clone()),
            );
            apply_document_properties(&mut value, &style.block);
            record_document_winners(
                &mut contributions,
                &style.block,
                StyleContributionOrigin::BlockStyle(style.id.clone()),
            );
            apply_character_properties(&mut value.character, &style.character);
            record_character_winners(
                &mut contributions,
                &style.character,
                StyleContributionOrigin::BlockStyle(style.id.clone()),
            );
        }
        apply_document_properties(&mut value, direct_canvas);
        record_document_winners(
            &mut contributions,
            direct_canvas,
            StyleContributionOrigin::DirectDocumentCanvas,
        );
        apply_character_properties(&mut value.character, direct_character);
        record_character_winners(
            &mut contributions,
            direct_character,
            StyleContributionOrigin::DirectDocumentCharacter,
        );
        Ok(ResolvedStyle {
            value,
            contributions,
        })
    }

    /// Resolve the style for a paragraph created at the terminal boundary of
    /// `current`. An absent declaration means that the current style carries
    /// forward, so callers never need to encode that default independently.
    pub fn next_paragraph_style(&self, current: &StyleId) -> Result<&StyleId, StyleError> {
        let style = self
            .block_styles
            .get(current)
            .ok_or_else(|| StyleError::UnknownStyle(current.clone()))?;
        if style.role != BlockRole::Paragraph {
            return Err(StyleError::IncompatibleBlockRole {
                style: style.id.clone(),
                role: style.role,
                parent_role: BlockRole::Paragraph,
            });
        }
        Ok(style.next_paragraph_style.as_ref().unwrap_or(&style.id))
    }

    /// Resolve one normalized document-root assignment.
    pub fn resolve_document_assignment(
        &self,
        assignment: &DocumentStyleAssignment,
    ) -> Result<ResolvedDocumentStyle, StyleError> {
        self.resolve_document_style(
            &assignment.style,
            &assignment.direct_canvas,
            &assignment.direct_default_character,
        )
    }

    /// Resolve the generic cascade through Document, Base Character,
    /// Paragraph, an optional named character style, and finally direct
    /// formatting. Later layers win property-by-property.
    pub fn resolve_paragraph_style(
        &self,
        document_style: &StyleId,
        paragraph_style: &StyleId,
        character_style: Option<&StyleId>,
        direct_paragraph: &BlockProperties,
        direct_character: &CharacterProperties,
    ) -> Result<ResolvedParagraphStyle, StyleError> {
        self.resolve_assigned_paragraph_style(
            &DocumentStyleAssignment::new(document_style.clone()),
            paragraph_style,
            direct_paragraph,
            &CharacterProperties::default(),
            character_style,
            direct_character,
        )
    }

    /// Resolve the complete normalized cascade using the root and paragraph
    /// assignments stored by the formatted projection. The final character
    /// declarations are inline direct formatting, after an optional named
    /// character style.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_assigned_paragraph_style(
        &self,
        document: &DocumentStyleAssignment,
        paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties,
        paragraph_default_character: &CharacterProperties,
        character_style: Option<&StyleId>,
        direct_character: &CharacterProperties,
    ) -> Result<ResolvedParagraphStyle, StyleError> {
        validate_character_properties(&document.style, &document.direct_default_character)?;
        validate_block_property_values(&document.style, &document.direct_canvas)?;
        if declares_paragraph_properties(&document.direct_canvas) {
            return Err(StyleError::InapplicableBlockProperties {
                style: document.style.clone(),
                role: BlockRole::Document,
            });
        }
        validate_character_properties(paragraph_style, paragraph_default_character)?;
        validate_character_properties(
            character_style.unwrap_or(paragraph_style),
            direct_character,
        )?;
        validate_block_property_values(paragraph_style, direct_paragraph)?;
        if declares_canvas_properties(direct_paragraph) {
            return Err(StyleError::InapplicableBlockProperties {
                style: paragraph_style.clone(),
                role: BlockRole::Paragraph,
            });
        }
        let document_chain = self.block_chain(&document.style, BlockRole::Document)?;
        let paragraph_chain = self.block_chain(paragraph_style, BlockRole::Paragraph)?;
        let base_character_chain = self.character_chain(&self.base_character)?;
        let mut resolved = ResolvedParagraphStyle::default();

        for style in document_chain {
            apply_character_properties(&mut resolved.character, &style.character);
        }
        apply_character_properties(&mut resolved.character, &document.direct_default_character);
        for style in base_character_chain {
            apply_character_properties(&mut resolved.character, &style.properties);
        }
        for style in paragraph_chain {
            if style.role == BlockRole::Document {
                continue;
            }
            apply_paragraph_properties(&mut resolved, &style.block);
            apply_character_properties(&mut resolved.character, &style.character);
        }
        apply_paragraph_properties(&mut resolved, direct_paragraph);
        apply_character_properties(&mut resolved.character, paragraph_default_character);
        if let Some(id) = character_style {
            for style in self.character_chain(id)? {
                // Base Character was already applied before paragraph defaults.
                if style.id != self.base_character {
                    apply_character_properties(&mut resolved.character, &style.properties);
                }
            }
        }
        apply_character_properties(&mut resolved.character, direct_character);
        Ok(resolved)
    }

    /// Resolve a paragraph/character cascade with complete contribution
    /// metadata. This is the dependency-aware counterpart of
    /// [`Self::resolve_assigned_paragraph_style`].
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_assigned_paragraph_style_with_contributions(
        &self,
        document: &DocumentStyleAssignment,
        paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties,
        paragraph_default_character: &CharacterProperties,
        character_style: Option<&StyleId>,
        direct_character: &CharacterProperties,
    ) -> Result<ResolvedStyle<ResolvedParagraphStyle>, StyleError> {
        validate_character_properties(&document.style, &document.direct_default_character)?;
        validate_block_property_values(&document.style, &document.direct_canvas)?;
        if declares_paragraph_properties(&document.direct_canvas) {
            return Err(StyleError::InapplicableBlockProperties {
                style: document.style.clone(),
                role: BlockRole::Document,
            });
        }
        validate_character_properties(paragraph_style, paragraph_default_character)?;
        validate_character_properties(
            character_style.unwrap_or(paragraph_style),
            direct_character,
        )?;
        validate_block_property_values(paragraph_style, direct_paragraph)?;
        if declares_canvas_properties(direct_paragraph) {
            return Err(StyleError::InapplicableBlockProperties {
                style: paragraph_style.clone(),
                role: BlockRole::Paragraph,
            });
        }

        let document_chain = self.block_chain(&document.style, BlockRole::Document)?;
        let paragraph_chain = self.block_chain(paragraph_style, BlockRole::Paragraph)?;
        let base_character_chain = self.character_chain(&self.base_character)?;
        let mut value = ResolvedParagraphStyle::default();
        let mut contributions = emergency_contributions(
            PARAGRAPH_STYLE_PROPERTIES
                .into_iter()
                .chain(CHARACTER_STYLE_PROPERTIES),
        );

        for style in document_chain {
            add_dependency(
                &mut contributions,
                CHARACTER_STYLE_PROPERTIES,
                StyleDependency::Block(style.id.clone()),
            );
            apply_character_properties(&mut value.character, &style.character);
            record_character_winners(
                &mut contributions,
                &style.character,
                StyleContributionOrigin::BlockStyle(style.id.clone()),
            );
        }
        apply_character_properties(&mut value.character, &document.direct_default_character);
        record_character_winners(
            &mut contributions,
            &document.direct_default_character,
            StyleContributionOrigin::DirectDocumentCharacter,
        );
        for style in base_character_chain {
            add_dependency(
                &mut contributions,
                CHARACTER_STYLE_PROPERTIES,
                StyleDependency::Character(style.id.clone()),
            );
            apply_character_properties(&mut value.character, &style.properties);
            record_character_winners(
                &mut contributions,
                &style.properties,
                StyleContributionOrigin::CharacterStyle(style.id.clone()),
            );
        }
        for style in paragraph_chain {
            if style.role == BlockRole::Document {
                continue;
            }
            add_dependency(
                &mut contributions,
                PARAGRAPH_STYLE_PROPERTIES
                    .into_iter()
                    .chain(CHARACTER_STYLE_PROPERTIES),
                StyleDependency::Block(style.id.clone()),
            );
            apply_paragraph_properties(&mut value, &style.block);
            record_paragraph_winners(
                &mut contributions,
                &style.block,
                StyleContributionOrigin::BlockStyle(style.id.clone()),
            );
            apply_character_properties(&mut value.character, &style.character);
            record_character_winners(
                &mut contributions,
                &style.character,
                StyleContributionOrigin::BlockStyle(style.id.clone()),
            );
        }
        apply_paragraph_properties(&mut value, direct_paragraph);
        record_paragraph_winners(
            &mut contributions,
            direct_paragraph,
            StyleContributionOrigin::DirectParagraph,
        );
        apply_character_properties(&mut value.character, paragraph_default_character);
        record_character_winners(
            &mut contributions,
            paragraph_default_character,
            StyleContributionOrigin::DirectParagraphCharacter,
        );
        if let Some(id) = character_style {
            for style in self.character_chain(id)? {
                if style.id == self.base_character {
                    continue;
                }
                add_dependency(
                    &mut contributions,
                    CHARACTER_STYLE_PROPERTIES,
                    StyleDependency::Character(style.id.clone()),
                );
                apply_character_properties(&mut value.character, &style.properties);
                record_character_winners(
                    &mut contributions,
                    &style.properties,
                    StyleContributionOrigin::CharacterStyle(style.id.clone()),
                );
            }
        }
        apply_character_properties(&mut value.character, direct_character);
        record_character_winners(
            &mut contributions,
            direct_character,
            StyleContributionOrigin::DirectCharacter,
        );

        Ok(ResolvedStyle {
            value,
            contributions,
        })
    }

    fn validate_block_parent(&self, style: &BlockStyle) -> Result<(), StyleError> {
        let parent_id = style
            .based_on
            .as_ref()
            .ok_or_else(|| StyleError::MissingParent(style.id.clone()))?;
        let parent = self
            .block_styles
            .get(parent_id)
            .ok_or_else(|| StyleError::MissingParent(parent_id.clone()))?;
        let compatible = match (style.role, parent.role) {
            (BlockRole::Document, BlockRole::Document) => true,
            (BlockRole::Paragraph, BlockRole::Paragraph) => true,
            (BlockRole::Paragraph, BlockRole::Document) => style.id == self.base_paragraph,
            _ => false,
        };
        if compatible {
            Ok(())
        } else {
            Err(StyleError::IncompatibleBlockRole {
                style: style.id.clone(),
                role: style.role,
                parent_role: parent.role,
            })
        }
    }

    fn replace_block_style(&mut self, style: BlockStyle) -> Result<(), StyleError> {
        if style.id == self.base_document {
            if style.role != BlockRole::Document
                || style.based_on.is_some()
                || style.next_paragraph_style.is_some()
            {
                return Err(StyleError::InvalidBaseStyleDefinition(style.id));
            }
            validate_character_properties(&style.id, &style.character)?;
            validate_block_properties(&style)?;
        } else if style.id == self.base_paragraph {
            if style.role != BlockRole::Paragraph
                || style.based_on.as_ref() != Some(&self.base_document)
            {
                return Err(StyleError::InvalidBaseStyleDefinition(style.id));
            }
            validate_character_properties(&style.id, &style.character)?;
            validate_block_properties(&style)?;
            self.validate_next_paragraph_style(&style)?;
        } else {
            let metadata = self
                .block_metadata
                .get(&style.id)
                .cloned()
                .ok_or_else(|| StyleError::InvalidDefinitionMetadata(style.id.clone()))?;
            return self.insert_block_style(style, metadata);
        }

        let next_revision = self.next_revision()?;
        self.block_styles.insert(style.id.clone(), style);
        self.validate_block_cycles()?;
        self.revision = next_revision;
        Ok(())
    }

    fn replace_character_style(&mut self, style: CharacterStyle) -> Result<(), StyleError> {
        if style.id != self.base_character {
            let metadata = self
                .character_metadata
                .get(&style.id)
                .cloned()
                .ok_or_else(|| StyleError::InvalidDefinitionMetadata(style.id.clone()))?;
            return self.insert_character_style(style, metadata);
        }
        if style.based_on.is_some() {
            return Err(StyleError::InvalidBaseStyleDefinition(style.id));
        }
        validate_character_properties(&style.id, &style.properties)?;
        let next_revision = self.next_revision()?;
        self.character_styles.insert(style.id.clone(), style);
        self.validate_character_cycles()?;
        self.revision = next_revision;
        Ok(())
    }

    fn validate_next_paragraph_style(&self, style: &BlockStyle) -> Result<(), StyleError> {
        let Some(next) = style.next_paragraph_style.as_ref() else {
            return Ok(());
        };
        if style.role != BlockRole::Paragraph {
            return Err(StyleError::InapplicableNextParagraphStyle {
                style: style.id.clone(),
                role: style.role,
            });
        }
        if next == &style.id {
            return Ok(());
        }
        if self
            .block_styles
            .get(next)
            .is_some_and(|candidate| candidate.role == BlockRole::Paragraph)
        {
            Ok(())
        } else {
            Err(StyleError::InvalidNextParagraphStyle {
                style: style.id.clone(),
                next: next.clone(),
            })
        }
    }

    fn block_chain(
        &self,
        id: &StyleId,
        expected_role: BlockRole,
    ) -> Result<Vec<&BlockStyle>, StyleError> {
        if !self.block_styles.contains_key(id) {
            return Err(StyleError::UnknownStyle(id.clone()));
        }
        let mut chain = Vec::new();
        let mut current = Some(id);
        let mut seen = BTreeMap::new();
        while let Some(style_id) = current {
            if seen.insert(style_id.clone(), ()).is_some() {
                return Err(StyleError::InheritanceCycle(style_id.clone()));
            }
            let style = self
                .block_styles
                .get(style_id)
                .ok_or_else(|| StyleError::MissingParent(style_id.clone()))?;
            chain.push(style);
            if let Some(default) = self.default_blocks.get(style_id) {
                chain.push(default);
            }
            current = style.based_on.as_ref();
        }
        chain.reverse();
        if chain.last().map(|style| style.role) != Some(expected_role) {
            let style = chain.last().expect("a requested style produced a chain");
            return Err(StyleError::IncompatibleBlockRole {
                style: style.id.clone(),
                role: style.role,
                parent_role: expected_role,
            });
        }
        Ok(chain)
    }

    fn character_chain(&self, id: &StyleId) -> Result<Vec<&CharacterStyle>, StyleError> {
        if !self.character_styles.contains_key(id) {
            return Err(StyleError::UnknownStyle(id.clone()));
        }
        let mut chain = Vec::new();
        let mut current = Some(id);
        let mut seen = BTreeMap::new();
        while let Some(style_id) = current {
            if seen.insert(style_id.clone(), ()).is_some() {
                return Err(StyleError::InheritanceCycle(style_id.clone()));
            }
            let style = self
                .character_styles
                .get(style_id)
                .ok_or_else(|| StyleError::MissingParent(style_id.clone()))?;
            chain.push(style);
            if let Some(default) = self.default_characters.get(style_id) {
                chain.push(default);
            }
            current = style.based_on.as_ref();
        }
        chain.reverse();
        Ok(chain)
    }

    fn validate_block_cycles(&self) -> Result<(), StyleError> {
        for style in self.block_styles.values() {
            if style.id != self.base_document {
                self.validate_block_parent(style)?;
            }
            self.validate_next_paragraph_style(style)?;
            let _ = self.block_chain(&style.id, style.role)?;
        }
        Ok(())
    }

    fn validate_character_cycles(&self) -> Result<(), StyleError> {
        for style in self.character_styles.values() {
            let _ = self.character_chain(&style.id)?;
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<StyleSheetRevision, StyleError> {
        self.revision
            .0
            .checked_add(1)
            .map(StyleSheetRevision)
            .ok_or(StyleError::StyleSheetRevisionExhausted)
    }
}

fn apply_block_field_edit(
    style: &mut BlockStyle,
    edit: &StyleDefinitionFieldEdit,
) -> Result<(), StyleError> {
    match edit {
        StyleDefinitionFieldEdit::SetDeclaration { property, value } => {
            if is_character_property(*property) {
                set_character_property(&style.id, &mut style.character, *property, value)
            } else {
                set_block_property(&style.id, &mut style.block, *property, value)
            }
        }
        StyleDefinitionFieldEdit::ClearDeclaration(property) => {
            if is_character_property(*property) {
                clear_character_property(&style.id, &mut style.character, *property)
            } else {
                clear_block_property(&style.id, &mut style.block, *property)
            }
        }
        StyleDefinitionFieldEdit::SetParent(parent) => {
            style.based_on.clone_from(parent);
            Ok(())
        }
        StyleDefinitionFieldEdit::SetNextParagraphStyle(next) => {
            style.next_paragraph_style.clone_from(next);
            Ok(())
        }
        StyleDefinitionFieldEdit::SetDisplayName(_) => {
            Err(StyleError::InapplicableStyleRelationship(style.id.clone()))
        }
    }
}

fn apply_character_field_edit(
    style: &mut CharacterStyle,
    edit: &StyleDefinitionFieldEdit,
) -> Result<(), StyleError> {
    match edit {
        StyleDefinitionFieldEdit::SetDeclaration { property, value } => {
            set_character_property(&style.id, &mut style.properties, *property, value)
        }
        StyleDefinitionFieldEdit::ClearDeclaration(property) => {
            clear_character_property(&style.id, &mut style.properties, *property)
        }
        StyleDefinitionFieldEdit::SetParent(parent) => {
            style.based_on.clone_from(parent);
            Ok(())
        }
        StyleDefinitionFieldEdit::SetNextParagraphStyle(_) => {
            Err(StyleError::InapplicableStyleRelationship(style.id.clone()))
        }
        StyleDefinitionFieldEdit::SetDisplayName(_) => {
            Err(StyleError::InapplicableStyleRelationship(style.id.clone()))
        }
    }
}

pub(crate) fn is_character_property(property: StyleProperty) -> bool {
    CHARACTER_STYLE_PROPERTIES.contains(&property)
}

fn invalid_style_value(style: &StyleId, property: StyleProperty) -> StyleError {
    StyleError::InvalidStylePropertyValue {
        style: style.clone(),
        property,
    }
}

fn inapplicable_style_property(style: &StyleId, property: StyleProperty) -> StyleError {
    StyleError::InapplicableStyleProperty {
        style: style.clone(),
        property,
    }
}

// One registry relates sparse fields, normalized property keys, and their
// typed values. Keeping these operations together makes adding a property an
// explicit, exhaustive change instead of synchronizing independent match tables.
macro_rules! sparse_property_operations {
    ($type:ident, $set:ident, $clear:ident; $(
        $field:ident => $property:ident($value:ident)
    ),+ $(,)?) => {
        impl $type {
            /// Exact declaration keys which differ. An absent declaration and
            /// an explicit default value remain observably different.
            pub fn changed_properties(&self, other: &Self) -> BTreeSet<StyleProperty> {
                let mut changed = BTreeSet::new();
                $(if self.$field != other.$field {
                    changed.insert(StyleProperty::$property);
                })+
                changed
            }

            pub fn declared_properties(&self) -> BTreeSet<StyleProperty> {
                self.changed_properties(&Self::default())
            }

            pub(super) fn clear_declaration(&mut self, property: StyleProperty) -> bool {
                match property {
                    $(StyleProperty::$property => self.$field = None,)+
                    _ => return false,
                }
                true
            }

            /// Merge sparse declarations without changing undeclared fields.
            /// Cascade policy, such as weight overriding inherited bold, is
            /// applied separately by the caller.
            pub(super) fn merge_declarations(&mut self, layer: &Self) {
                $(if layer.$field.is_some() {
                    self.$field.clone_from(&layer.$field);
                })+
            }
        }

        pub(super) fn $set(
            style: &StyleId,
            properties: &mut $type,
            property: StyleProperty,
            value: &StylePropertyValue,
        ) -> Result<(), StyleError> {
            match (property, value) {
                $((StyleProperty::$property, StylePropertyValue::$value(value)) => {
                    properties.$field = Some(value.clone());
                })+
                $( (StyleProperty::$property, _) => {
                    return Err(invalid_style_value(style, property));
                })+
                _ => return Err(inapplicable_style_property(style, property)),
            }
            Ok(())
        }

        pub(super) fn $clear(
            style: &StyleId,
            properties: &mut $type,
            property: StyleProperty,
        ) -> Result<(), StyleError> {
            if !properties.clear_declaration(property) {
                return Err(inapplicable_style_property(style, property));
            }
            Ok(())
        }
    };
}

sparse_property_operations! {
    CharacterProperties, set_character_property, clear_character_property;
    font_families => CharacterFontFamilies(FontFamilies),
    size => CharacterSize(Float),
    weight => CharacterWeight(FontWeight),
    bold => CharacterBold(Boolean),
    slant => CharacterSlant(FontSlant),
    foreground => CharacterForeground(Color),
    background => CharacterBackground(Color),
    underline => CharacterUnderline(Boolean),
    strikethrough => CharacterStrikethrough(Boolean),
    language => CharacterLanguage(Text),
    direction => CharacterDirection(WritingDirection),
    open_type_features => CharacterOpenTypeFeatures(OpenTypeFeatures),
    letter_spacing => CharacterLetterSpacing(Float),
    baseline_shift => CharacterBaselineShift(Float),
}

sparse_property_operations! {
    BlockProperties, set_block_property, clear_block_property;
    spacing_before => ParagraphSpacingBefore(Float),
    spacing_after => ParagraphSpacingAfter(Float),
    line_spacing => ParagraphLineSpacing(LineSpacing),
    first_line_indent => ParagraphFirstLineIndent(Float),
    leading_indent => ParagraphLeadingIndent(Float),
    trailing_indent => ParagraphTrailingIndent(Float),
    padding_top => CanvasPaddingTop(Float),
    padding_right => CanvasPaddingRight(Float),
    padding_bottom => CanvasPaddingBottom(Float),
    padding_left => CanvasPaddingLeft(Float),
    background => CanvasBackground(Color),
    alignment => ParagraphAlignment(ParagraphAlignment),
    base_direction => ParagraphBaseDirection(WritingDirection),
}

pub(super) fn validate_character_properties(
    id: &StyleId,
    properties: &CharacterProperties,
) -> Result<(), StyleError> {
    let font_families_valid = properties.font_families.as_ref().map_or(true, |families| {
        !families.is_empty() && families.iter().all(|family| !family.is_empty())
    });
    let language_valid = properties
        .language
        .as_ref()
        .map_or(true, |language| !language.is_empty());
    let features_valid = properties
        .open_type_features
        .as_ref()
        .map_or(true, |features| {
            features
                .keys()
                .all(|tag| tag.len() == 4 && tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte)))
        });
    let valid = font_families_valid
        && language_valid
        && features_valid
        && properties
            .size
            .map_or(true, |size| size.is_finite() && size > 0.0)
        && properties
            .weight
            .map_or(true, |weight| (1..=1000).contains(&weight))
        && properties.foreground.map_or(true, valid_color)
        && properties.background.map_or(true, valid_color)
        && properties
            .letter_spacing
            .map_or(true, |value| value.is_finite())
        && properties
            .baseline_shift
            .map_or(true, |value| value.is_finite());
    if valid {
        Ok(())
    } else {
        Err(StyleError::InvalidCharacterProperties(id.clone()))
    }
}

fn validate_definition_metadata(
    id: &StyleId,
    metadata: &StyleDefinitionMetadata,
) -> Result<(), StyleError> {
    if metadata.display_name.trim().is_empty() || metadata.display_name.contains('\0') {
        Err(StyleError::InvalidDefinitionMetadata(id.clone()))
    } else {
        Ok(())
    }
}

fn validate_block_properties(style: &BlockStyle) -> Result<(), StyleError> {
    let block = &style.block;
    let paragraph_declared = declares_paragraph_properties(block);
    let canvas_declared = declares_canvas_properties(block);
    if (style.role == BlockRole::Document && paragraph_declared)
        || (style.role == BlockRole::Paragraph && canvas_declared)
    {
        return Err(StyleError::InapplicableBlockProperties {
            style: style.id.clone(),
            role: style.role,
        });
    }
    validate_block_property_values(&style.id, block)
}

pub(super) fn validate_block_property_values(
    id: &StyleId,
    block: &BlockProperties,
) -> Result<(), StyleError> {
    let finite = [
        block.spacing_before,
        block.spacing_after,
        block.first_line_indent,
        block.leading_indent,
        block.trailing_indent,
        block.padding_top,
        block.padding_right,
        block.padding_bottom,
        block.padding_left,
    ]
    .into_iter()
    .flatten()
    .all(f32::is_finite)
        && block.background.map_or(true, valid_color);
    let spacing_valid = block.line_spacing.map_or(true, |spacing| match spacing {
        LineSpacing::Normal => true,
        LineSpacing::Multiplier(value) => value.is_finite() && value > 0.0,
        LineSpacing::AtLeast(value) | LineSpacing::Exact(value) => {
            value.is_finite() && value >= 0.0
        }
    });
    if finite && spacing_valid {
        Ok(())
    } else {
        Err(StyleError::InvalidBlockProperties(id.clone()))
    }
}

fn valid_color(color: Color) -> bool {
    [color.red, color.green, color.blue, color.alpha]
        .into_iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
}

fn declares_paragraph_properties(block: &BlockProperties) -> bool {
    block.spacing_before.is_some()
        || block.spacing_after.is_some()
        || block.line_spacing.is_some()
        || block.first_line_indent.is_some()
        || block.leading_indent.is_some()
        || block.trailing_indent.is_some()
        || block.alignment.is_some()
        || block.base_direction.is_some()
}

fn declares_canvas_properties(block: &BlockProperties) -> bool {
    block.padding_top.is_some()
        || block.padding_right.is_some()
        || block.padding_bottom.is_some()
        || block.padding_left.is_some()
        || block.background.is_some()
}

fn emergency_contributions(
    properties: impl IntoIterator<Item = StyleProperty>,
) -> BTreeMap<StyleProperty, StyleContribution> {
    properties
        .into_iter()
        .map(|property| {
            (
                property,
                StyleContribution {
                    winner: StyleContributionOrigin::EngineEmergency,
                    dependencies: Vec::new(),
                },
            )
        })
        .collect()
}

fn add_dependency(
    contributions: &mut BTreeMap<StyleProperty, StyleContribution>,
    properties: impl IntoIterator<Item = StyleProperty>,
    dependency: StyleDependency,
) {
    for property in properties {
        let entry = contributions
            .get_mut(&property)
            .expect("the resolved style initialized every applicable property");
        if entry.dependencies.last() != Some(&dependency)
            && !entry.dependencies.contains(&dependency)
        {
            entry.dependencies.push(dependency.clone());
        }
    }
}

fn record_winner(
    contributions: &mut BTreeMap<StyleProperty, StyleContribution>,
    property: StyleProperty,
    origin: &StyleContributionOrigin,
) {
    contributions
        .get_mut(&property)
        .expect("the resolved style initialized every applicable property")
        .winner
        .clone_from(origin);
}

fn record_character_winners(
    contributions: &mut BTreeMap<StyleProperty, StyleContribution>,
    properties: &CharacterProperties,
    origin: StyleContributionOrigin,
) {
    if properties.font_families.is_some() {
        record_winner(contributions, StyleProperty::CharacterFontFamilies, &origin);
    }
    if properties.size.is_some() {
        record_winner(contributions, StyleProperty::CharacterSize, &origin);
    }
    if properties.bold.is_some() {
        record_winner(contributions, StyleProperty::CharacterBold, &origin);
    }
    if properties.weight.is_some() {
        record_winner(contributions, StyleProperty::CharacterWeight, &origin);
    }
    if properties.slant.is_some() {
        record_winner(contributions, StyleProperty::CharacterSlant, &origin);
    }
    if properties.foreground.is_some() {
        record_winner(contributions, StyleProperty::CharacterForeground, &origin);
    }
    if properties.background.is_some() {
        record_winner(contributions, StyleProperty::CharacterBackground, &origin);
    }
    if properties.underline.is_some() {
        record_winner(contributions, StyleProperty::CharacterUnderline, &origin);
    }
    if properties.strikethrough.is_some() {
        record_winner(
            contributions,
            StyleProperty::CharacterStrikethrough,
            &origin,
        );
    }
    if properties.language.is_some() {
        record_winner(contributions, StyleProperty::CharacterLanguage, &origin);
    }
    if properties.direction.is_some() {
        record_winner(contributions, StyleProperty::CharacterDirection, &origin);
    }
    if properties.open_type_features.is_some() {
        record_winner(
            contributions,
            StyleProperty::CharacterOpenTypeFeatures,
            &origin,
        );
    }
    if properties.letter_spacing.is_some() {
        record_winner(
            contributions,
            StyleProperty::CharacterLetterSpacing,
            &origin,
        );
    }
    if properties.baseline_shift.is_some() {
        record_winner(
            contributions,
            StyleProperty::CharacterBaselineShift,
            &origin,
        );
    }
}

fn record_document_winners(
    contributions: &mut BTreeMap<StyleProperty, StyleContribution>,
    properties: &BlockProperties,
    origin: StyleContributionOrigin,
) {
    if properties.background.is_some() {
        record_winner(contributions, StyleProperty::CanvasBackground, &origin);
    }
    if properties.padding_top.is_some() {
        record_winner(contributions, StyleProperty::CanvasPaddingTop, &origin);
    }
    if properties.padding_right.is_some() {
        record_winner(contributions, StyleProperty::CanvasPaddingRight, &origin);
    }
    if properties.padding_bottom.is_some() {
        record_winner(contributions, StyleProperty::CanvasPaddingBottom, &origin);
    }
    if properties.padding_left.is_some() {
        record_winner(contributions, StyleProperty::CanvasPaddingLeft, &origin);
    }
}

fn record_paragraph_winners(
    contributions: &mut BTreeMap<StyleProperty, StyleContribution>,
    properties: &BlockProperties,
    origin: StyleContributionOrigin,
) {
    if properties.spacing_before.is_some() {
        record_winner(
            contributions,
            StyleProperty::ParagraphSpacingBefore,
            &origin,
        );
    }
    if properties.spacing_after.is_some() {
        record_winner(contributions, StyleProperty::ParagraphSpacingAfter, &origin);
    }
    if properties.line_spacing.is_some() {
        record_winner(contributions, StyleProperty::ParagraphLineSpacing, &origin);
    }
    if properties.first_line_indent.is_some() {
        record_winner(
            contributions,
            StyleProperty::ParagraphFirstLineIndent,
            &origin,
        );
    }
    if properties.leading_indent.is_some() {
        record_winner(
            contributions,
            StyleProperty::ParagraphLeadingIndent,
            &origin,
        );
    }
    if properties.trailing_indent.is_some() {
        record_winner(
            contributions,
            StyleProperty::ParagraphTrailingIndent,
            &origin,
        );
    }
    if properties.alignment.is_some() {
        record_winner(contributions, StyleProperty::ParagraphAlignment, &origin);
    }
    if properties.base_direction.is_some() {
        record_winner(
            contributions,
            StyleProperty::ParagraphBaseDirection,
            &origin,
        );
    }
}

fn apply_character_properties(
    resolved: &mut ResolvedCharacterStyle,
    properties: &CharacterProperties,
) {
    if let Some(value) = properties.font_families.as_ref() {
        resolved.font_families.clone_from(value);
    }
    if let Some(value) = properties.size {
        resolved.size = value;
    }
    if let Some(value) = properties.weight {
        resolved.base_weight = value;
        resolved.bold = false;
    }
    if let Some(value) = properties.bold {
        resolved.bold = value;
    }
    resolved.weight = if resolved.bold {
        resolved.base_weight.saturating_add(300).min(1000)
    } else {
        resolved.base_weight
    };
    if let Some(value) = properties.slant {
        resolved.slant = value;
    }
    if let Some(value) = properties.foreground {
        resolved.foreground = value;
        resolved.foreground_is_default = false;
    }
    if let Some(value) = properties.background {
        resolved.background = Some(value);
    }
    if let Some(value) = properties.underline {
        resolved.underline = value;
    }
    if let Some(value) = properties.strikethrough {
        resolved.strikethrough = value;
    }
    if let Some(value) = properties.language.as_ref() {
        resolved.language = Some(value.clone());
    }
    if let Some(value) = properties.direction {
        resolved.direction = value;
    }
    if let Some(value) = properties.open_type_features.as_ref() {
        resolved.open_type_features.clone_from(value);
    }
    if let Some(value) = properties.letter_spacing {
        resolved.letter_spacing = value;
    }
    if let Some(value) = properties.baseline_shift {
        resolved.baseline_shift = value;
    }
}

fn apply_document_properties(resolved: &mut ResolvedDocumentStyle, properties: &BlockProperties) {
    if let Some(value) = properties.background {
        resolved.background = value;
        resolved.background_is_default = false;
    }
    if let Some(value) = properties.padding_top {
        resolved.padding_top = value;
    }
    if let Some(value) = properties.padding_right {
        resolved.padding_right = value;
    }
    if let Some(value) = properties.padding_bottom {
        resolved.padding_bottom = value;
    }
    if let Some(value) = properties.padding_left {
        resolved.padding_left = value;
    }
}

fn apply_paragraph_properties(resolved: &mut ResolvedParagraphStyle, properties: &BlockProperties) {
    if let Some(value) = properties.spacing_before {
        resolved.spacing_before = value;
    }
    if let Some(value) = properties.spacing_after {
        resolved.spacing_after = value;
    }
    if let Some(value) = properties.line_spacing {
        resolved.line_spacing = value;
    }
    if let Some(value) = properties.first_line_indent {
        resolved.first_line_indent = value;
    }
    if let Some(value) = properties.leading_indent {
        resolved.leading_indent = value;
    }
    if let Some(value) = properties.trailing_indent {
        resolved.trailing_indent = value;
    }
    if let Some(value) = properties.alignment {
        resolved.alignment = value;
    }
    if let Some(value) = properties.base_direction {
        resolved.base_direction = value;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticInlineStyle {
    Strong,
    Emphasis,
    Code,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StyleApplication {
    Named(StyleId),
    /// Adapter-owned syntax decoration; never a user character assignment.
    Automatic(StyleId),
    /// A source grammar extent, including unpainted whitespace inside a tag.
    SourceSyntax,
    /// Raw script/style-like contents where even the opening boundary is literal.
    SourceRawText,
    /// Adapter context only: these HTML characters preserve literal whitespace.
    /// An empty range records the same context at a matching empty source
    /// anchor. It has no appearance or user-assignment meaning.
    SourcePreservedWhitespace,
    /// The semantic paragraph underlying visible source syntax. Source hard
    /// lines may contain several paragraph elements, so this context is inline.
    SourceParagraph {
        style: StyleId,
        defaults: CharacterProperties,
    },
    Direct(CharacterProperties),
    Semantic(SemanticInlineStyle),
}

impl StyleApplication {
    pub(super) fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Named(id) | Self::Automatic(id) => id.0.capacity() + 16,
            Self::SourceParagraph { style, defaults } => style.0.capacity() + 16 + defaults.owned_heap_bytes(),
            Self::Direct(properties) => properties.owned_heap_bytes(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated() -> StyleDefinitionMetadata {
        StyleDefinitionMetadata::generated("Test style")
    }

    fn character_style(
        id: &str,
        parent: &StyleId,
        properties: CharacterProperties,
    ) -> CharacterStyle {
        CharacterStyle {
            id: id.into(),
            based_on: Some(parent.clone()),
            properties,
        }
    }

    fn paragraph_style(id: &str, parent: &StyleId) -> BlockStyle {
        BlockStyle {
            id: id.into(),
            based_on: Some(parent.clone()),
            next_paragraph_style: None,
            role: BlockRole::Paragraph,
            character: CharacterProperties::default(),
            block: BlockProperties::default(),
        }
    }

    #[test]
    fn built_in_styles_explicitly_define_the_defaults_they_own() {
        let sheet = StyleSheet::default();
        let document = sheet.block_style(&sheet.base_document).unwrap();
        let paragraph = sheet.block_style(&sheet.base_paragraph).unwrap();
        assert_eq!(
            sheet
                .block_style_metadata(&sheet.base_document)
                .unwrap()
                .display_name,
            "Base Document"
        );
        assert_eq!(
            sheet
                .block_style_metadata(&StyleId::from("Heading1"))
                .unwrap()
                .display_name,
            "Heading 1"
        );
        assert_eq!(
            sheet
                .character_style_metadata(&sheet.base_character)
                .unwrap()
                .origin,
            StyleDefinitionOrigin::GeneratedConfiguration
        );

        assert_eq!(
            document.character,
            CharacterProperties {
                font_families: Some(vec!["SF Pro".to_owned()]),
                size: Some(14.0),
                weight: Some(400),
                slant: Some(FontSlant::Upright),
                foreground: None,
                underline: Some(false),
                strikethrough: Some(false),
                direction: Some(WritingDirection::Natural),
                open_type_features: Some(BTreeMap::new()),
                letter_spacing: Some(0.0),
                baseline_shift: Some(0.0),
                ..CharacterProperties::default()
            }
        );
        assert_eq!(
            document.block,
            BlockProperties {
                padding_top: Some(0.0),
                padding_right: Some(0.0),
                padding_bottom: Some(0.0),
                padding_left: Some(0.0),
                background: None,
                ..BlockProperties::default()
            }
        );
        assert_eq!(
            paragraph.block,
            BlockProperties {
                spacing_before: Some(0.0),
                spacing_after: Some(0.0),
                line_spacing: Some(LineSpacing::Normal),
                first_line_indent: Some(0.0),
                leading_indent: Some(0.0),
                trailing_indent: Some(0.0),
                alignment: Some(ParagraphAlignment::Start),
                base_direction: Some(WritingDirection::Natural),
                ..BlockProperties::default()
            }
        );
        assert_eq!(
            sheet
                .character_style(&sheet.base_character)
                .unwrap()
                .properties,
            CharacterProperties::default()
        );

        assert_eq!(
            sheet
                .resolve_document_assignment(&DocumentStyleAssignment::new(
                    sheet.base_document.clone(),
                ))
                .unwrap(),
            ResolvedDocumentStyle::default()
        );
        assert_eq!(
            sheet
                .resolve_assigned_paragraph_style(
                    &DocumentStyleAssignment::new(sheet.base_document.clone()),
                    &sheet.base_paragraph,
                    &BlockProperties::default(),
                    &CharacterProperties::default(),
                    None,
                    &CharacterProperties::default(),
                )
                .unwrap(),
            ResolvedParagraphStyle::default()
        );
    }

    #[test]
    fn generated_field_edits_derive_authority_from_definition_metadata() {
        let mut sheet = StyleSheet::default();
        let source_id = StyleId::from("source-token-17");
        let source_style = paragraph_style(&source_id.0, &sheet.base_paragraph);
        sheet
            .insert_block_style(
                source_style,
                StyleDefinitionMetadata {
                    display_name: "Imported Body".to_owned(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            )
            .unwrap();
        let synthetic_id = StyleId::from("synthetic-token-3");
        sheet
            .insert_character_style(
                character_style(
                    &synthetic_id.0,
                    &sheet.base_character,
                    CharacterProperties::default(),
                ),
                StyleDefinitionMetadata {
                    display_name: "Computed Emphasis".to_owned(),
                    origin: StyleDefinitionOrigin::SyntheticReadOnly,
                },
            )
            .unwrap();
        let before = sheet.clone();

        assert_eq!(
            sheet.prepare_generated_field_edit(
                StyleNamespace::Block,
                &source_id,
                &StyleDefinitionFieldEdit::SetDeclaration {
                    property: StyleProperty::ParagraphSpacingAfter,
                    value: StylePropertyValue::Float(8.0),
                },
            ),
            Err(StyleError::DefinitionNotGeneratedConfiguration {
                style: source_id,
                origin: StyleDefinitionOrigin::SourceBacked,
            })
        );
        assert_eq!(
            sheet.prepare_generated_field_edit(
                StyleNamespace::Character,
                &synthetic_id,
                &StyleDefinitionFieldEdit::ClearDeclaration(StyleProperty::CharacterWeight),
            ),
            Err(StyleError::DefinitionNotGeneratedConfiguration {
                style: synthetic_id,
                origin: StyleDefinitionOrigin::SyntheticReadOnly,
            })
        );
        assert_eq!(sheet, before);
    }

    #[test]
    fn document_text_overrides_flow_through_sparse_character_and_paragraph_bases() {
        let mut sheet = StyleSheet::default();
        let foreground = Color {
            red: 0.15,
            green: 0.25,
            blue: 0.35,
            alpha: 1.0,
        };
        let features = BTreeMap::from([("liga".to_owned(), 0)]);
        let document_style = BlockStyle {
            id: "WriterDocument".into(),
            based_on: Some(sheet.base_document.clone()),
            next_paragraph_style: None,
            role: BlockRole::Document,
            character: CharacterProperties {
                font_families: Some(vec!["Writer Serif".to_owned(), "system-ui".to_owned()]),
                size: Some(19.0),
                weight: Some(525),
                slant: Some(FontSlant::Italic),
                foreground: Some(foreground),
                underline: Some(true),
                direction: Some(WritingDirection::RightToLeft),
                open_type_features: Some(features.clone()),
                letter_spacing: Some(0.5),
                ..CharacterProperties::default()
            },
            block: BlockProperties::default(),
        };
        let document_style_id = document_style.id.clone();
        sheet
            .insert_block_style(document_style, generated())
            .unwrap();

        let resolved = sheet
            .resolve_assigned_paragraph_style(
                &DocumentStyleAssignment::new(document_style_id),
                &sheet.base_paragraph,
                &BlockProperties::default(),
                &CharacterProperties::default(),
                None,
                &CharacterProperties::default(),
            )
            .unwrap();

        assert_eq!(
            resolved.character.font_families,
            ["Writer Serif", "system-ui"]
        );
        assert_eq!(resolved.character.size, 19.0);
        assert_eq!(resolved.character.weight, 525);
        assert_eq!(resolved.character.slant, FontSlant::Italic);
        assert_eq!(resolved.character.foreground, foreground);
        assert!(resolved.character.underline);
        assert_eq!(resolved.character.direction, WritingDirection::RightToLeft);
        assert_eq!(resolved.character.open_type_features, features);
        assert_eq!(resolved.character.letter_spacing, 0.5);
        assert_eq!(resolved.spacing_before, 0.0);
        assert_eq!(resolved.line_spacing, LineSpacing::Normal);
    }

    #[test]
    fn cascade_orders_document_paragraph_named_character_and_direct_layers() {
        let mut sheet = StyleSheet::default();
        let named = character_style(
            "Callout",
            &sheet.base_character,
            CharacterProperties {
                foreground: Some(Color {
                    red: 0.8,
                    green: 0.1,
                    blue: 0.2,
                    alpha: 1.0,
                }),
                weight: Some(600),
                ..CharacterProperties::default()
            },
        );
        let named_id = named.id.clone();
        sheet.insert_character_style(named, generated()).unwrap();

        let resolved = sheet
            .resolve_paragraph_style(
                &sheet.base_document,
                &"Heading1".into(),
                Some(&named_id),
                &BlockProperties {
                    leading_indent: Some(18.0),
                    line_spacing: Some(LineSpacing::Multiplier(1.25)),
                    ..BlockProperties::default()
                },
                &CharacterProperties {
                    weight: Some(900),
                    underline: Some(true),
                    ..CharacterProperties::default()
                },
            )
            .unwrap();

        assert_eq!(resolved.character.size, 24.0);
        assert_eq!(resolved.character.weight, 900);
        assert!(resolved.character.underline);
        assert_eq!(resolved.character.foreground.red, 0.8);
        assert_eq!(resolved.leading_indent, 18.0);
        assert_eq!(resolved.line_spacing, LineSpacing::Multiplier(1.25));
    }

    #[test]
    fn complete_paragraph_resolution_validates_the_root_assignment() {
        let sheet = StyleSheet::default();
        let mut root = DocumentStyleAssignment::new(sheet.base_document.clone());
        root.direct_canvas.spacing_before = Some(1.0);
        assert!(matches!(
            sheet.resolve_assigned_paragraph_style(
                &root,
                &sheet.base_paragraph,
                &BlockProperties::default(),
                &CharacterProperties::default(),
                None,
                &CharacterProperties::default(),
            ),
            Err(StyleError::InapplicableBlockProperties {
                role: BlockRole::Document,
                ..
            })
        ));
    }

    #[test]
    fn block_roles_reject_inapplicable_properties_and_broadened_parents() {
        let mut sheet = StyleSheet::default();
        let invalid_canvas = BlockStyle {
            block: BlockProperties {
                padding_left: Some(10.0),
                ..BlockProperties::default()
            },
            ..paragraph_style("BadParagraph", &sheet.base_paragraph)
        };
        assert!(matches!(
            sheet.insert_block_style(invalid_canvas, generated()),
            Err(StyleError::InapplicableBlockProperties { .. })
        ));

        let document_child = BlockStyle {
            id: "DocumentChild".into(),
            based_on: Some(sheet.base_document.clone()),
            next_paragraph_style: None,
            role: BlockRole::Document,
            character: CharacterProperties::default(),
            block: BlockProperties::default(),
        };
        let document_child_id = document_child.id.clone();
        sheet
            .insert_block_style(document_child, generated())
            .unwrap();
        assert!(matches!(
            sheet.insert_block_style(
                paragraph_style("BadParent", &document_child_id),
                generated()
            ),
            Err(StyleError::IncompatibleBlockRole { .. })
        ));
    }

    #[test]
    fn paragraph_next_style_is_typed_validated_and_defaults_to_self() {
        let mut sheet = StyleSheet::default();
        assert_eq!(
            sheet.next_paragraph_style(&sheet.base_paragraph).unwrap(),
            &sheet.base_paragraph
        );
        assert_eq!(
            sheet.next_paragraph_style(&"Heading1".into()).unwrap(),
            &sheet.base_paragraph
        );

        let self_id: StyleId = "SelfFollowing".into();
        let mut self_following = paragraph_style("SelfFollowing", &sheet.base_paragraph);
        self_following.next_paragraph_style = Some(self_id.clone());
        sheet
            .insert_block_style(self_following, generated())
            .unwrap();
        assert_eq!(sheet.next_paragraph_style(&self_id).unwrap(), &self_id);
        sheet.remove_block_style(&self_id, false).unwrap();
        assert!(!sheet.block_styles.contains_key(&self_id));

        let mut missing = paragraph_style("MissingNext", &sheet.base_paragraph);
        missing.next_paragraph_style = Some("NotAStyle".into());
        assert!(matches!(
            sheet.insert_block_style(missing, generated()),
            Err(StyleError::InvalidNextParagraphStyle { .. })
        ));

        let mut document_child = BlockStyle {
            id: "CanvasWithNext".into(),
            based_on: Some(sheet.base_document.clone()),
            next_paragraph_style: Some(sheet.base_paragraph.clone()),
            role: BlockRole::Document,
            character: CharacterProperties::default(),
            block: BlockProperties::default(),
        };
        assert!(matches!(
            sheet.insert_block_style(document_child.clone(), generated()),
            Err(StyleError::InapplicableNextParagraphStyle { .. })
        ));
        document_child.next_paragraph_style = None;
        sheet
            .insert_block_style(document_child, generated())
            .unwrap();
        assert!(matches!(
            sheet.next_paragraph_style(&"CanvasWithNext".into()),
            Err(StyleError::IncompatibleBlockRole { .. })
        ));
    }

    #[test]
    fn a_next_paragraph_reference_prevents_style_removal() {
        let mut sheet = StyleSheet::default();
        let target = paragraph_style("BodyAfterLead", &sheet.base_paragraph);
        let target_id = target.id.clone();
        sheet.insert_block_style(target, generated()).unwrap();

        let mut lead = paragraph_style("Lead", &sheet.base_paragraph);
        lead.next_paragraph_style = Some(target_id.clone());
        sheet.insert_block_style(lead, generated()).unwrap();

        assert_eq!(
            sheet.remove_block_style(&target_id, false),
            Err(StyleError::StyleInUse(target_id))
        );
    }

    #[test]
    fn cycles_are_rejected_without_corrupting_the_previous_definition() {
        let mut sheet = StyleSheet::default();
        let first = paragraph_style("First", &sheet.base_paragraph);
        let first_id = first.id.clone();
        sheet.insert_block_style(first, generated()).unwrap();
        let second = paragraph_style("Second", &first_id);
        let second_id = second.id.clone();
        sheet.insert_block_style(second, generated()).unwrap();
        let revision = sheet.revision;

        let cyclic = paragraph_style("First", &second_id);
        assert_eq!(
            sheet.insert_block_style(cyclic, generated()),
            Err(StyleError::InheritanceCycle(first_id.clone()))
        );
        assert_eq!(sheet.revision, revision);
        assert_eq!(
            sheet.block_styles[&first_id].based_on.as_ref(),
            Some(&sheet.base_paragraph)
        );
    }

    #[test]
    fn base_and_referenced_styles_cannot_be_removed() {
        let mut sheet = StyleSheet::default();
        assert_eq!(
            sheet.remove_block_style(&sheet.base_document.clone(), false),
            Err(StyleError::CannotRemoveBaseStyle(
                sheet.base_document.clone()
            ))
        );

        let parent = paragraph_style("Parent", &sheet.base_paragraph);
        let parent_id = parent.id.clone();
        sheet.insert_block_style(parent, generated()).unwrap();
        sheet
            .insert_block_style(paragraph_style("Child", &parent_id), generated())
            .unwrap();
        assert_eq!(
            sheet.remove_block_style(&parent_id, false),
            Err(StyleError::StyleInUse(parent_id))
        );
    }

    #[test]
    fn invalid_numeric_declarations_are_rejected_atomically() {
        let mut sheet = StyleSheet::default();
        let revision = sheet.revision;
        let invalid = character_style(
            "Broken",
            &sheet.base_character,
            CharacterProperties {
                size: Some(f32::NAN),
                ..CharacterProperties::default()
            },
        );
        let id = invalid.id.clone();
        assert_eq!(
            sheet.insert_character_style(invalid, generated()),
            Err(StyleError::InvalidCharacterProperties(id.clone()))
        );
        assert_eq!(sheet.revision, revision);
        assert!(!sheet.character_styles.contains_key(&id));
    }

    #[test]
    fn invalid_provider_facing_character_declarations_are_rejected_atomically() {
        let invalid_properties = [
            CharacterProperties {
                font_families: Some(Vec::new()),
                ..CharacterProperties::default()
            },
            CharacterProperties {
                font_families: Some(vec![String::new()]),
                ..CharacterProperties::default()
            },
            CharacterProperties {
                language: Some(String::new()),
                ..CharacterProperties::default()
            },
            CharacterProperties {
                open_type_features: Some(BTreeMap::from([("lig".to_owned(), 1)])),
                ..CharacterProperties::default()
            },
            CharacterProperties {
                open_type_features: Some(BTreeMap::from([("li\0a".to_owned(), 1)])),
                ..CharacterProperties::default()
            },
            CharacterProperties {
                // Two two-byte code points make this exactly four bytes, so
                // the printable-ASCII rule is independently exercised.
                open_type_features: Some(BTreeMap::from([("éé".to_owned(), 1)])),
                ..CharacterProperties::default()
            },
        ];

        for (index, properties) in invalid_properties.into_iter().enumerate() {
            let mut sheet = StyleSheet::default();
            let before = sheet.clone();
            let id = StyleId(format!("InvalidProviderInput{index}"));
            let invalid = CharacterStyle {
                id: id.clone(),
                based_on: Some(sheet.base_character.clone()),
                properties,
            };

            assert_eq!(
                sheet.insert_character_style(invalid, generated()),
                Err(StyleError::InvalidCharacterProperties(id))
            );
            assert_eq!(sheet, before);
        }
    }

    #[test]
    fn invalid_direct_character_declarations_fail_before_reaching_layout() {
        let sheet = StyleSheet::default();
        let invalid = CharacterProperties {
            font_families: Some(Vec::new()),
            ..CharacterProperties::default()
        };

        assert_eq!(
            sheet.resolve_paragraph_style(
                &sheet.base_document,
                &sheet.base_paragraph,
                None,
                &BlockProperties::default(),
                &invalid,
            ),
            Err(StyleError::InvalidCharacterProperties(
                sheet.base_paragraph.clone()
            ))
        );

        let valid = CharacterProperties {
            font_families: Some(vec!["Writer Serif".to_owned(), "system-ui".to_owned()]),
            language: Some("en-US".to_owned()),
            open_type_features: Some(BTreeMap::from([
                ("kern".to_owned(), 1),
                ("liga".to_owned(), 0),
            ])),
            ..CharacterProperties::default()
        };
        let resolved = sheet
            .resolve_paragraph_style(
                &sheet.base_document,
                &sheet.base_paragraph,
                None,
                &BlockProperties::default(),
                &valid,
            )
            .unwrap();
        assert_eq!(
            resolved.character.font_families,
            valid.font_families.unwrap()
        );
        assert_eq!(resolved.character.language.as_deref(), Some("en-US"));
        assert_eq!(resolved.character.open_type_features.len(), 2);
    }

    #[test]
    fn document_resolution_reports_every_winner_and_consulted_dependency() {
        let mut sheet = StyleSheet::default();
        let writer_id = StyleId::from("WriterDocument");
        sheet
            .insert_block_style(
                BlockStyle {
                    id: writer_id.clone(),
                    based_on: Some(sheet.base_document.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Document,
                    character: CharacterProperties {
                        size: Some(18.0),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        padding_left: Some(12.0),
                        ..BlockProperties::default()
                    },
                },
                generated(),
            )
            .unwrap();
        let direct_canvas = BlockProperties {
            padding_top: Some(7.0),
            ..BlockProperties::default()
        };
        let direct_character = CharacterProperties {
            foreground: Some(Color {
                red: 0.2,
                green: 0.3,
                blue: 0.4,
                alpha: 1.0,
            }),
            ..CharacterProperties::default()
        };

        let traced = sheet
            .resolve_document_style_with_contributions(
                &writer_id,
                &direct_canvas,
                &direct_character,
            )
            .unwrap();
        assert_eq!(traced.contributions().len(), 19);
        assert_eq!(traced.value.padding_left, 12.0);
        assert_eq!(traced.value.padding_top, 7.0);
        assert_eq!(traced.value.character.size, 18.0);
        assert_eq!(
            traced
                .contribution(StyleProperty::CanvasPaddingLeft)
                .unwrap()
                .winner,
            StyleContributionOrigin::BlockStyle(writer_id.clone())
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::CanvasPaddingTop)
                .unwrap()
                .winner,
            StyleContributionOrigin::DirectDocumentCanvas
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::CharacterForeground)
                .unwrap()
                .winner,
            StyleContributionOrigin::DirectDocumentCharacter
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::CharacterSize)
                .unwrap()
                .dependencies,
            vec![
                StyleDependency::Block(sheet.base_document.clone()),
                StyleDependency::Block(writer_id),
            ]
        );
    }

    #[test]
    fn paragraph_resolution_traces_independent_cascade_layers() {
        let mut sheet = StyleSheet::default();
        let lead_id = StyleId::from("Lead");
        sheet
            .insert_block_style(
                BlockStyle {
                    id: lead_id.clone(),
                    based_on: Some(sheet.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(20.0),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        spacing_before: Some(9.0),
                        ..BlockProperties::default()
                    },
                },
                generated(),
            )
            .unwrap();
        let emphasis_id = StyleId::from("NamedEmphasis");
        sheet
            .insert_character_style(
                CharacterStyle {
                    id: emphasis_id.clone(),
                    based_on: Some(sheet.base_character.clone()),
                    properties: CharacterProperties {
                        weight: Some(650),
                        ..CharacterProperties::default()
                    },
                },
                generated(),
            )
            .unwrap();
        let direct_paragraph = BlockProperties {
            leading_indent: Some(11.0),
            ..BlockProperties::default()
        };
        let paragraph_character = CharacterProperties {
            slant: Some(FontSlant::Italic),
            ..CharacterProperties::default()
        };
        let direct_character = CharacterProperties {
            underline: Some(true),
            ..CharacterProperties::default()
        };
        let assignment = DocumentStyleAssignment::new(sheet.base_document.clone());

        let traced = sheet
            .resolve_assigned_paragraph_style_with_contributions(
                &assignment,
                &lead_id,
                &direct_paragraph,
                &paragraph_character,
                Some(&emphasis_id),
                &direct_character,
            )
            .unwrap();
        let ordinary = sheet
            .resolve_assigned_paragraph_style(
                &assignment,
                &lead_id,
                &direct_paragraph,
                &paragraph_character,
                Some(&emphasis_id),
                &direct_character,
            )
            .unwrap();
        assert_eq!(traced.value, ordinary);
        assert_eq!(traced.contributions().len(), 22);
        assert_eq!(
            traced
                .contribution(StyleProperty::ParagraphSpacingBefore)
                .unwrap()
                .winner,
            StyleContributionOrigin::BlockStyle(lead_id)
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::ParagraphLeadingIndent)
                .unwrap()
                .winner,
            StyleContributionOrigin::DirectParagraph
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::CharacterSlant)
                .unwrap()
                .winner,
            StyleContributionOrigin::DirectParagraphCharacter
        );
        assert_eq!(
            traced
                .contribution(StyleProperty::CharacterWeight)
                .unwrap()
                .winner,
            StyleContributionOrigin::CharacterStyle(emphasis_id.clone())
        );
        let weight_dependencies = &traced
            .contribution(StyleProperty::CharacterWeight)
            .unwrap()
            .dependencies;
        assert!(
            weight_dependencies.contains(&StyleDependency::Character(sheet.base_character.clone()))
        );
        assert!(weight_dependencies.contains(&StyleDependency::Character(emphasis_id)));
        assert_eq!(
            traced
                .contribution(StyleProperty::CharacterUnderline)
                .unwrap()
                .winner,
            StyleContributionOrigin::DirectCharacter
        );
    }

    #[test]
    fn dependency_index_limits_definition_invalidation_to_transitive_descendants() {
        let mut sheet = StyleSheet::default();
        let parent_id = StyleId::from("Parent");
        let child_id = StyleId::from("Child");
        let separate_id = StyleId::from("Separate");
        sheet
            .insert_block_style(
                paragraph_style("Parent", &sheet.base_paragraph),
                generated(),
            )
            .unwrap();
        sheet
            .insert_block_style(paragraph_style("Child", &parent_id), generated())
            .unwrap();
        sheet
            .insert_block_style(
                paragraph_style("Separate", &sheet.base_paragraph),
                generated(),
            )
            .unwrap();

        let character_parent_id = StyleId::from("CharacterParent");
        let character_child_id = StyleId::from("CharacterChild");
        sheet
            .insert_character_style(
                character_style(
                    "CharacterParent",
                    &sheet.base_character,
                    CharacterProperties::default(),
                ),
                generated(),
            )
            .unwrap();
        sheet
            .insert_character_style(
                character_style(
                    "CharacterChild",
                    &character_parent_id,
                    CharacterProperties::default(),
                ),
                generated(),
            )
            .unwrap();

        let index = sheet.dependency_index();
        assert_eq!(index.revision, sheet.revision);
        let affected = index.block_dependents(&parent_id).unwrap();
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&parent_id));
        assert!(affected.contains(&child_id));
        assert!(!affected.contains(&separate_id));
        let affected = index.character_dependents(&character_parent_id).unwrap();
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&character_parent_id));
        assert!(affected.contains(&character_child_id));
        assert!(index.block_dependents(&StyleId::from("Missing")).is_none());
    }

    #[test]
    fn property_effects_separate_paint_shaping_geometry_and_width() {
        assert_eq!(
            StyleProperty::CharacterForeground.invalidation_effect(),
            StyleInvalidationEffect::Paint
        );
        assert_eq!(
            StyleProperty::CharacterFontFamilies.invalidation_effect(),
            StyleInvalidationEffect::Shaping
        );
        assert_eq!(
            StyleProperty::ParagraphLeadingIndent.invalidation_effect(),
            StyleInvalidationEffect::ParagraphLayout
        );
        assert_eq!(
            StyleProperty::CanvasPaddingTop.invalidation_effect(),
            StyleInvalidationEffect::DocumentLayout
        );
        assert_eq!(
            StyleProperty::CanvasPaddingLeft.invalidation_effect(),
            StyleInvalidationEffect::ViewUsableWidth
        );
    }

    #[test]
    fn resolved_differences_report_only_effective_property_changes() {
        let before_character = ResolvedCharacterStyle::default();
        let mut after_character = before_character.clone();
        after_character.foreground.red = 0.5;
        after_character.weight = 700;
        assert_eq!(
            before_character.changed_properties(&after_character),
            BTreeSet::from([
                StyleProperty::CharacterForeground,
                StyleProperty::CharacterWeight,
            ])
        );

        let before_paragraph = ResolvedParagraphStyle::default();
        let mut after_paragraph = before_paragraph.clone();
        after_paragraph.leading_indent = 8.0;
        after_paragraph.character.size = 16.0;
        assert_eq!(
            before_paragraph.changed_properties(&after_paragraph),
            BTreeSet::from([
                StyleProperty::ParagraphLeadingIndent,
                StyleProperty::CharacterSize,
            ])
        );

        let before_document = ResolvedDocumentStyle::default();
        let mut after_document = before_document.clone();
        after_document.padding_left = 12.0;
        after_document.character.underline = true;
        assert_eq!(
            before_document.changed_properties(&after_document),
            BTreeSet::from([
                StyleProperty::CanvasPaddingLeft,
                StyleProperty::CharacterUnderline,
            ])
        );
    }

    #[test]
    fn randomized_acyclic_cascades_match_with_and_without_contribution_tracing() {
        let mut sheet = StyleSheet::default();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 7;
            seed ^= seed >> 9;
            seed = seed.wrapping_mul(0x2545_f491_4f6c_dd1d);
            seed
        };

        let mut paragraph_ids = vec![sheet.base_paragraph.clone()];
        for index in 0..64 {
            let parent = paragraph_ids[next() as usize % paragraph_ids.len()].clone();
            let id = StyleId(format!("RandomParagraph{index:02}"));
            let bits = next();
            sheet
                .insert_block_style(
                    BlockStyle {
                        id: id.clone(),
                        based_on: Some(parent),
                        next_paragraph_style: None,
                        role: BlockRole::Paragraph,
                        character: CharacterProperties {
                            size: (bits & 1 != 0).then_some(10.0 + (bits % 18) as f32),
                            weight: (bits & 2 != 0).then_some(300 + (bits % 6) as u16 * 100),
                            underline: (bits & 4 != 0).then_some(bits & 8 != 0),
                            ..CharacterProperties::default()
                        },
                        block: BlockProperties {
                            spacing_before: (bits & 16 != 0).then_some((bits % 12) as f32),
                            leading_indent: (bits & 32 != 0).then_some((bits % 20) as f32),
                            ..BlockProperties::default()
                        },
                    },
                    generated(),
                )
                .unwrap();
            paragraph_ids.push(id);
        }

        let mut character_ids = vec![sheet.base_character.clone()];
        for index in 0..64 {
            let parent = character_ids[next() as usize % character_ids.len()].clone();
            let id = StyleId(format!("RandomCharacter{index:02}"));
            let bits = next();
            sheet
                .insert_character_style(
                    CharacterStyle {
                        id: id.clone(),
                        based_on: Some(parent),
                        properties: CharacterProperties {
                            slant: (bits & 1 != 0).then_some(FontSlant::Italic),
                            weight: (bits & 2 != 0).then_some(400 + (bits % 5) as u16 * 100),
                            letter_spacing: (bits & 4 != 0).then_some((bits % 7) as f32 / 4.0),
                            ..CharacterProperties::default()
                        },
                    },
                    generated(),
                )
                .unwrap();
            character_ids.push(id);
        }

        let assignment = DocumentStyleAssignment::new(sheet.base_document.clone());
        for _ in 0..256 {
            let paragraph = &paragraph_ids[next() as usize % paragraph_ids.len()];
            let character = &character_ids[next() as usize % character_ids.len()];
            let direct_paragraph = BlockProperties {
                trailing_indent: (next() & 1 != 0).then_some((next() % 16) as f32),
                ..BlockProperties::default()
            };
            let direct_character = CharacterProperties {
                baseline_shift: (next() & 1 != 0).then_some((next() % 8) as f32),
                ..CharacterProperties::default()
            };
            let ordinary = sheet
                .resolve_assigned_paragraph_style(
                    &assignment,
                    paragraph,
                    &direct_paragraph,
                    &CharacterProperties::default(),
                    Some(character),
                    &direct_character,
                )
                .unwrap();
            let traced = sheet
                .resolve_assigned_paragraph_style_with_contributions(
                    &assignment,
                    paragraph,
                    &direct_paragraph,
                    &CharacterProperties::default(),
                    Some(character),
                    &direct_character,
                )
                .unwrap();
            assert_eq!(traced.value, ordinary);
            assert_eq!(traced.contributions().len(), 22);
            assert!(traced
                .contributions()
                .all(|(_, contribution)| !contribution.dependencies.is_empty()));
        }
    }

    #[test]
    fn revision_exhaustion_is_typed_and_leaves_the_sheet_unchanged() {
        let mut sheet = StyleSheet {
            revision: StyleSheetRevision(u64::MAX),
            ..StyleSheet::default()
        };
        let before = sheet.clone();
        let style = character_style(
            "WouldWrap",
            &sheet.base_character,
            CharacterProperties::default(),
        );

        assert_eq!(
            sheet.insert_character_style(style, generated()),
            Err(StyleError::StyleSheetRevisionExhausted)
        );
        assert_eq!(sheet, before);

        let removable = StyleId::from("Removable");
        sheet.character_styles.insert(
            removable.clone(),
            character_style(
                "Removable",
                &sheet.base_character,
                CharacterProperties::default(),
            ),
        );
        let before_remove = sheet.clone();
        assert_eq!(
            sheet.remove_character_style(&removable, false),
            Err(StyleError::StyleSheetRevisionExhausted)
        );
        assert_eq!(sheet, before_remove);
    }
}
