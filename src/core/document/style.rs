mod defaults;
#[cfg(test)]
mod percentage_tests;
pub use defaults::StyleDefaultsError;
pub mod code;

use std::collections::{BTreeMap, BTreeSet};

/// Initial generated text family, shared by document and emergency shaping
/// defaults. Authored font requests remain unchanged across platforms.
pub const DEFAULT_FONT_FAMILY: &str = if cfg!(target_os = "macos") {
    "SF Pro"
} else if cfg!(target_os = "windows") {
    "Segoe UI"
} else {
    "system-ui"
};

pub(super) const DEFAULT_FONT_SIZE: f32 = 14.0;

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
    /// Presentation-only character overlay used for search matches.
    pub fn incremental_match() -> Self {
        Self("* Incremental match".into())
    }

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
            "* Incremental match"
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

/// Structural kinds share the block namespace; compatible parents retain
/// the same kind while Base Paragraph supplies common text defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BlockRole {
    Document,
    Paragraph,
    Quote,
    CodeBlock,
    List,
    ListItem,
    Table,
}

impl BlockRole {
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Quote | Self::CodeBlock | Self::List | Self::ListItem | Self::Table)
    }
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

/// A sparse size declaration. Percentages are evaluated against the paragraph
/// parent for block styles and the underlying text for named character styles.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "StoredFontSize", into = "StoredFontSize")]
pub enum FontSize {
    Points(f32),
    Percentage(u16),
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum StoredFontSize {
    Points(f32),
    Percentage { percentage: u16 },
}

impl From<StoredFontSize> for FontSize {
    fn from(value: StoredFontSize) -> Self {
        match value {
            StoredFontSize::Points(value) => Self::Points(value),
            StoredFontSize::Percentage { percentage } => Self::Percentage(percentage),
        }
    }
}
impl From<FontSize> for StoredFontSize {
    fn from(value: FontSize) -> Self {
        match value {
            FontSize::Points(value) => Self::Points(value),
            FontSize::Percentage(percentage) => Self::Percentage { percentage },
        }
    }
}
impl From<f32> for FontSize {
    fn from(value: f32) -> Self { Self::Points(value) }
}
impl FontSize {
    pub fn resolve(self, underlying_points: f32) -> f32 {
        match self {
            Self::Points(value) => value,
            Self::Percentage(value) => (f64::from(underlying_points) * f64::from(value) / 100.0) as f32,
        }
    }
    pub fn is_valid(self) -> bool {
        match self {
            Self::Points(value) => value.is_finite() && value > 0.0,
            Self::Percentage(value) => (10..=1000).contains(&value),
        }
    }
}
impl From<FontSize> for StylePropertyValue {
    fn from(value: FontSize) -> Self {
        match value {
            FontSize::Points(value) => Self::Float(value),
            FontSize::Percentage(value) => Self::Percentage(value),
        }
    }
}

/// Sparse character declarations. `None` means inherit/leave unchanged.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CharacterProperties {
    pub font_families: Option<Vec<String>>,
    /// Human-readable OpenType subfamily of the primary family; empty selects automatically.
    pub font_face: Option<String>,
    /// Coordinates in the selected font's design space; inherited as a whole.
    pub font_axes: Option<BTreeMap<String, f32>>,
    pub size: Option<FontSize>,
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
}

impl CharacterProperties {
    pub(super) fn overlay(&mut self, layer: &Self) {
        if layer.font_families.is_some() {
            self.font_face = None;
            self.font_axes = None;
            self.weight = None;
        }
        if layer.weight.is_some() && layer.font_families.is_none() {
            self.bold = None;
        }
        self.merge_declarations(layer);
    }

    pub(super) fn owned_heap_bytes(&self) -> usize {
        self.font_families.as_ref().map_or(0, |families| {
            families.capacity() * std::mem::size_of::<String>()
                + families.iter().map(|name| name.capacity() + 16).sum::<usize>()
        }) + self.font_face.as_ref().map_or(0, |value| value.capacity() + 16)
            + self.language.as_ref().map_or(0, |value| value.capacity() + 16)
            + self.font_axes.as_ref().map_or(0, |axes| {
                style_map_heap_bytes(axes.len(), std::mem::size_of::<(String, f32)>())
                    + axes.keys().map(|tag| tag.capacity() + 16).sum::<usize>()
            })
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
    pub margin_top: Option<f32>,
    pub margin_right: Option<f32>,
    pub margin_left: Option<f32>,
    pub margin_bottom: Option<f32>,
    pub line_spacing: Option<LineSpacing>,
    pub first_line_indent: Option<f32>,
    pub leading_indent: Option<f32>,
    pub trailing_indent: Option<f32>,
    pub border_top_width: Option<f32>,
    pub border_top_color: Option<Color>,
    pub border_right_width: Option<f32>,
    pub border_right_color: Option<Color>,
    pub border_bottom_width: Option<f32>,
    pub border_bottom_color: Option<Color>,
    pub border_left_width: Option<f32>,
    pub border_left_color: Option<Color>,
    pub padding_top: Option<f32>,
    pub padding_right: Option<f32>,
    pub padding_bottom: Option<f32>,
    pub padding_left: Option<f32>,
    pub background: Option<Color>,
    pub alignment: Option<ParagraphAlignment>,
    pub base_direction: Option<WritingDirection>,
}

impl BlockProperties {
    /// CSS boxes belong to their own element; these values do not inherit down the document tree.
    pub fn clear_box(&mut self) {
        self.margin_top = None;
        self.margin_right = None;
        self.margin_bottom = None;
        self.margin_left = None;
        self.padding_top = None;
        self.padding_right = None;
        self.padding_bottom = None;
        self.padding_left = None;
        self.border_top_width = None;
        self.border_top_color = None;
        self.border_right_width = None;
        self.border_right_color = None;
        self.border_bottom_width = None;
        self.border_bottom_color = None;
        self.border_left_width = None;
        self.border_left_color = None;
        self.background = None;
    }
}

/// Normalized style assignment for the formatted document root.
///
/// The root defaults to Base Paragraph; source adapters may supply an authored
/// [`BlockRole::Document`] context. Direct declarations are sparse layers
/// applied after the root style chain: `direct_canvas`
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
    /// None inherits the containing paragraph's character properties.
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
/// bytes untouched. Content style assignments use Markdown syntax, and
/// synthetic definitions remain read-only.
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
    /// Session configuration identity; not a persisted style declaration.
    pub(crate) theme_generation: u64,
    pub base_paragraph: StyleId,
    block_styles: BTreeMap<StyleId, BlockStyle>,
    character_styles: BTreeMap<StyleId, CharacterStyle>,
    block_metadata: BTreeMap<StyleId, StyleDefinitionMetadata>,
    character_metadata: BTreeMap<StyleId, StyleDefinitionMetadata>,
    deleted_configuration_blocks: BTreeSet<StyleId>,
    deleted_configuration_characters: BTreeSet<StyleId>,
    // Settings-file seeds and assignment provenance. These are not cascade
    // layers: the editable maps above contain the complete own declarations.
    default_blocks: BTreeMap<StyleId, BlockStyle>,
    default_characters: BTreeMap<StyleId, CharacterStyle>,
    /// Code syntax definitions generated for emitted names and not yet edited.
    /// They are never persisted.
    implicit_characters: BTreeSet<StyleId>,
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
        let mut bytes = id(&self.base_paragraph);
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
                    &self.implicit_characters] {
            bytes += style_map_heap_bytes(set.len(), std::mem::size_of::<StyleId>())
                + set.iter().map(id).sum::<usize>();
        }
        bytes
    }
}

// Style equality compares semantic definitions and configuration state.
impl PartialEq for StyleSheet {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.base_paragraph == other.base_paragraph
            && self.block_styles == other.block_styles
            && self.character_styles == other.character_styles
            && self.block_metadata == other.block_metadata
            && self.character_metadata == other.character_metadata
            && self.deleted_configuration_blocks == other.deleted_configuration_blocks
            && self.deleted_configuration_characters == other.deleted_configuration_characters
            && self.default_blocks == other.default_blocks
            && self.default_characters == other.default_characters
            && self.implicit_characters == other.implicit_characters
    }
}

impl StyleSheet {
    /// The literal-text baseline has no rich-content definitions.
    pub(crate) fn plain_text() -> Self {
        let paragraph: StyleId = "Paragraph".into();
        let mut block_styles = BTreeMap::new();
        block_styles.insert(
            paragraph.clone(),
            BlockStyle {
                id: paragraph.clone(),
                based_on: None,
                next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: CharacterProperties {
                    font_families: Some(vec![DEFAULT_FONT_FAMILY.to_owned()]),
                    font_face: Some(String::new()),
                    font_axes: Some(BTreeMap::new()),
                    size: Some(DEFAULT_FONT_SIZE.into()),
                    weight: Some(400),
                    slant: Some(FontSlant::Upright),
                    // Unspecified color follows the application theme.
                    foreground: None,
                    underline: Some(false),
                    strikethrough: Some(false),
                    direction: Some(WritingDirection::Natural),
                    open_type_features: Some(BTreeMap::new()),
                    letter_spacing: Some(0.0),
                    ..CharacterProperties::default()
                },
                block: BlockProperties {
                    margin_top: Some(0.0),
                    margin_bottom: Some(0.0),
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
        let mut block_metadata = BTreeMap::new();
        block_metadata.insert(paragraph.clone(), StyleDefinitionMetadata::generated("Base Paragraph"));
        let character_styles = BTreeMap::new();
        let character_metadata = BTreeMap::new();
        let mut sheet = Self {
            revision: StyleSheetRevision(1),
            base_paragraph: paragraph,
            block_styles,
            character_styles,
            block_metadata,
            character_metadata,
            deleted_configuration_blocks: BTreeSet::new(),
            deleted_configuration_characters: BTreeSet::new(),
            default_blocks: BTreeMap::new(),
            default_characters: BTreeMap::new(),
            implicit_characters: BTreeSet::new(),
            theme_generation: 0,
        };
        sheet.install_incremental_match_style();
        sheet
    }
}

impl Default for StyleSheet {
    fn default() -> Self {
        let mut sheet = Self::plain_text();
        let paragraph = sheet.base_paragraph.clone();
        let block_styles = &mut sheet.block_styles;
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
                        size: Some((26.0 - level as f32 * 2.0).into()),
                        weight: Some(700),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        margin_top: Some(10.0),
                        margin_bottom: Some(5.0),
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
                            leading_indent: Some(0.0),
                            first_line_indent: Some(0.0),
                            margin_top: Some(0.0),
                            margin_bottom: Some(0.0),
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
                next_paragraph_style: None,
                role: BlockRole::Quote,
                character: CharacterProperties::default(),
                block: BlockProperties {
                    margin_left: Some(14.0),
                    margin_right: Some(32.0),
                    padding_left: Some(16.0),
                    border_left_width: Some(2.0),
                    border_left_color: Some(Color { red: 0.72, green: 0.72, blue: 0.72, alpha: 1.0 }),
                    ..Default::default()
                },
            },
        );
        for (name, role) in [("Bulleted List", BlockRole::List), ("Numbered List", BlockRole::List), ("List item", BlockRole::ListItem)] {
            block_styles.insert(name.into(), BlockStyle {
                id: name.into(), based_on: Some(paragraph.clone()), next_paragraph_style: None,
                role, character: CharacterProperties::default(), block: BlockProperties {
                    padding_left: (role == BlockRole::List).then_some(32.0), ..Default::default()
                },
            });
        }
        for (name, parent, role) in [("Table cell", "Paragraph", BlockRole::Paragraph), ("Table header", "Table cell", BlockRole::Paragraph), ("Table", "Paragraph", BlockRole::Table)] {
            let cell = name == "Table cell";
            block_styles.insert(name.into(), BlockStyle {
                id: name.into(), based_on: Some(parent.into()), next_paragraph_style: None, role,
                character: CharacterProperties { bold: (name == "Table header").then_some(true), ..Default::default() },
                block: BlockProperties { padding_left: cell.then_some(10.0), padding_right: cell.then_some(10.0), padding_top: cell.then_some(6.0), padding_bottom: cell.then_some(6.0),
                    border_left_width: cell.then_some(1.0), border_right_width: cell.then_some(1.0), border_top_width: cell.then_some(1.0), border_bottom_width: cell.then_some(1.0), ..Default::default() },
            });
        }
        let character_styles = &mut sheet.character_styles;
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
                next_paragraph_style: None,
                role: BlockRole::CodeBlock,
                character: code_properties.clone(),
                block: BlockProperties::default(),
            },
        );
        character_styles.insert(
            "Code".into(),
            CharacterStyle {
                id: "Code".into(),
                based_on: None,
                properties: code_properties,
            },
        );
        let block_metadata = &mut sheet.block_metadata;
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
        for name in ["Bulleted List", "Numbered List", "List item", "Table", "Table cell", "Table header"] {
            block_metadata.insert(name.into(), StyleDefinitionMetadata::generated(name));
        }
        let character_metadata = &mut sheet.character_metadata;
        block_metadata.insert(
            "Code Block".into(),
            StyleDefinitionMetadata::generated("Code Block"),
        );
        character_metadata.insert("Code".into(), StyleDefinitionMetadata::generated("Code"));
        sheet
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
    pub font_face: String,
    pub font_axes: BTreeMap<String, f32>,
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
}

impl Default for ResolvedCharacterStyle {
    fn default() -> Self {
        Self {
            font_families: vec![DEFAULT_FONT_FAMILY.to_owned()],
            font_face: String::new(),
            font_axes: BTreeMap::new(),
            size: DEFAULT_FONT_SIZE,
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
        compare!(font_face, StyleProperty::CharacterFontFace);
        compare!(font_axes, StyleProperty::CharacterFontAxes);
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
    pub margin_right: f32,
    pub margin_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_top_width: f32,
    pub border_top_color: Option<Color>,
    pub border_right_width: f32,
    pub border_right_color: Option<Color>,
    pub border_bottom_width: f32,
    pub border_bottom_color: Option<Color>,
    pub border_left_width: f32,
    pub border_left_color: Option<Color>,
    pub background: Option<Color>,
    pub margin_top: f32,
    pub margin_bottom: f32,
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
/// a source-format spelling such as a CSS property.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum StyleProperty {
    CanvasBackground,
    CanvasPaddingTop,
    CanvasPaddingRight,
    CanvasPaddingBottom,
    CanvasPaddingLeft,
    BlockMarginRight,
    BlockMarginLeft,
    BlockPaddingTop,
    BlockPaddingRight,
    BlockPaddingBottom,
    BlockPaddingLeft,
    BlockBorderTopWidth,
    BlockBorderTopColor,
    BlockBorderRightWidth,
    BlockBorderRightColor,
    BlockBorderBottomWidth,
    BlockBorderBottomColor,
    BlockBorderLeftWidth,
    BlockBorderLeftColor,
    BlockBackground,
    BlockMarginTop,
    BlockMarginBottom,
    ParagraphLineSpacing,
    ParagraphFirstLineIndent,
    ParagraphLeadingIndent,
    ParagraphTrailingIndent,
    ParagraphAlignment,
    ParagraphBaseDirection,
    CharacterFontFamilies,
    CharacterFontFace,
    CharacterFontAxes,
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
    Percentage(u16),
    FontWeight(u16),
    Boolean(bool),
    Color(Color),
    FontFamilies(Vec<String>),
    FontAxes(BTreeMap<String, f32>),
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
            | Self::BlockBorderTopColor
            | Self::BlockBorderRightColor
            | Self::BlockBorderBottomColor
            | Self::BlockBorderLeftColor
            | Self::BlockBackground
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
            Self::BlockMarginRight
            | Self::BlockMarginLeft
            | Self::BlockPaddingTop
            | Self::BlockPaddingRight
            | Self::BlockPaddingBottom
            | Self::BlockPaddingLeft
            | Self::BlockBorderTopWidth
            | Self::BlockBorderRightWidth
            | Self::BlockBorderBottomWidth
            | Self::BlockBorderLeftWidth
            | Self::BlockMarginTop
            | Self::BlockMarginBottom
            | Self::ParagraphLineSpacing
            | Self::ParagraphFirstLineIndent
            | Self::ParagraphLeadingIndent
            | Self::ParagraphTrailingIndent
            | Self::ParagraphAlignment => StyleInvalidationEffect::ParagraphLayout,
            Self::ParagraphBaseDirection => StyleInvalidationEffect::Shaping,
            Self::CharacterFontFamilies
            | Self::CharacterFontFace
            | Self::CharacterFontAxes
            | Self::CharacterSize
            | Self::CharacterWeight
            | Self::CharacterBold
            | Self::CharacterSlant
            | Self::CharacterLanguage
            | Self::CharacterDirection
            | Self::CharacterOpenTypeFeatures
            | Self::CharacterLetterSpacing => StyleInvalidationEffect::Shaping,
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

pub(crate) const PARAGRAPH_STYLE_PROPERTIES: [StyleProperty; 23] = [
    StyleProperty::BlockMarginRight,
    StyleProperty::BlockMarginLeft,
    StyleProperty::BlockPaddingTop,
    StyleProperty::BlockPaddingRight,
    StyleProperty::BlockPaddingBottom,
    StyleProperty::BlockPaddingLeft,
    StyleProperty::BlockBorderTopWidth,
    StyleProperty::BlockBorderTopColor,
    StyleProperty::BlockBorderRightWidth,
    StyleProperty::BlockBorderRightColor,
    StyleProperty::BlockBorderBottomWidth,
    StyleProperty::BlockBorderBottomColor,
    StyleProperty::BlockBorderLeftWidth,
    StyleProperty::BlockBorderLeftColor,
    StyleProperty::BlockBackground,

    StyleProperty::BlockMarginTop,
    StyleProperty::BlockMarginBottom,
    StyleProperty::ParagraphLineSpacing,
    StyleProperty::ParagraphFirstLineIndent,
    StyleProperty::ParagraphLeadingIndent,
    StyleProperty::ParagraphTrailingIndent,
    StyleProperty::ParagraphAlignment,
    StyleProperty::ParagraphBaseDirection,
];

pub(crate) const CHARACTER_STYLE_PROPERTIES: [StyleProperty; 15] = [
    StyleProperty::CharacterFontFamilies,
    StyleProperty::CharacterFontFace,
    StyleProperty::CharacterFontAxes,
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
            margin_right: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_top_width: 0.0,
            border_top_color: None,
            border_right_width: 0.0,
            border_right_color: None,
            border_bottom_width: 0.0,
            border_bottom_color: None,
            border_left_width: 0.0,
            border_left_color: None,
            background: None,

            margin_top: 0.0,
            margin_bottom: 0.0,
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
        compare!(margin_right, StyleProperty::BlockMarginRight);
        compare!(margin_left, StyleProperty::BlockMarginLeft);
        compare!(padding_top, StyleProperty::BlockPaddingTop);
        compare!(padding_right, StyleProperty::BlockPaddingRight);
        compare!(padding_bottom, StyleProperty::BlockPaddingBottom);
        compare!(padding_left, StyleProperty::BlockPaddingLeft);
        compare!(border_top_width, StyleProperty::BlockBorderTopWidth);
        compare!(border_top_color, StyleProperty::BlockBorderTopColor);
        compare!(border_right_width, StyleProperty::BlockBorderRightWidth);
        compare!(border_right_color, StyleProperty::BlockBorderRightColor);
        compare!(border_bottom_width, StyleProperty::BlockBorderBottomWidth);
        compare!(border_bottom_color, StyleProperty::BlockBorderBottomColor);
        compare!(border_left_width, StyleProperty::BlockBorderLeftWidth);
        compare!(border_left_color, StyleProperty::BlockBorderLeftColor);
        compare!(background, StyleProperty::BlockBackground);
        compare!(margin_top, StyleProperty::BlockMarginTop);
        compare!(margin_bottom, StyleProperty::BlockMarginBottom);
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
    fn install_incremental_match_style(&mut self) {
        let id = StyleId::incremental_match();
        self.character_styles.insert(id.clone(), CharacterStyle {
            id: id.clone(),
            based_on: None,
            properties: CharacterProperties {
                background: Some(Color { red: 1.0, green: 0.86, blue: 0.0, alpha: 0.40 }),
                ..Default::default()
            },
        });
        self.character_metadata.insert(id, StyleDefinitionMetadata::generated("Incremental match"));
    }

    /// Sparse presentation declarations, including saved user defaults, with
    /// no paragraph or emergency properties filled in for unspecified fields.
    pub fn incremental_match_properties(&self) -> Result<CharacterProperties, StyleError> {
        self.named_character_declarations(Some(&StyleId::incremental_match()))
    }

    /// Apply search presentation after the complete ordinary text cascade.
    /// Unspecified properties preserve the underlying content's appearance.
    pub fn overlay_incremental_match(
        &self,
        underlying: &ResolvedCharacterStyle,
    ) -> Result<ResolvedCharacterStyle, StyleError> {
        let mut result = underlying.clone();
        apply_character_properties(&mut result, &self.incremental_match_properties()?);
        Ok(result)
    }

    /// Look up one immutable block-style definition by its stable ID.
    pub fn block_style(&self, id: &StyleId) -> Option<&BlockStyle> {
        self.block_styles.get(id)
    }

    /// A generated Code syntax definition that has not been edited.
    pub fn is_implicit_character(&self, id: &StyleId) -> bool {
        self.implicit_characters.contains(id)
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

    /// Content-derived links retain their semantic interval when the user
    /// deletes their optional appearance. Other undefined style references
    /// remain errors; deletion does not silently repair an invalid sheet.
    pub(crate) fn automatic_character_properties(
        &self,
        id: &StyleId,
    ) -> Result<CharacterProperties, StyleError> {
        if id.0 == "Link"
            && self.character_style(id).is_none()
            && self.configuration_deleted(id, false)
        {
            return Ok(CharacterProperties::default());
        }
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            let style = self.character_style(id)
                .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
            chain.push(&style.properties);
            current = style.based_on.as_ref();
        }
        let mut result = CharacterProperties::default();
        for properties in chain.into_iter().rev() {
            result.merge_declarations(properties);
        }
        Ok(result)
    }

    /// Prose paragraphs use half-em block margins, collapsed by the block layout.
    /// Literal formats retain their adapter-specific defaults.
    pub(crate) fn for_format(format: super::Format) -> Self {
        if !format.is_markdown() {
            return if format == super::Format::Code { code::default_sheet() } else { Self::plain_text() };
        }
        let mut sheet = Self::default();
        if format.is_markdown() {
            sheet.character_styles.insert("Link".into(), CharacterStyle {
                id: "Link".into(), based_on: None,
                properties: CharacterProperties {
                    foreground: Some(Color { red: 0.0, green: 0.0, blue: 1.0, alpha: 1.0 }),
                    underline: Some(true), ..Default::default()
                },
            });
            sheet.character_metadata.insert("Link".into(), StyleDefinitionMetadata::generated("Link"));
            let paragraph = sheet.block_styles.get_mut(&sheet.base_paragraph).unwrap();
            paragraph.block.margin_top = Some(7.0);
            paragraph.block.margin_bottom = Some(7.0);
            if format.is_markdown() {
                for (name, properties) in [
                    ("Markdown reference", CharacterProperties { foreground: Some(Color { red: 0.65, green: 0.45, blue: 0.82, alpha: 1.0 }), ..Default::default() }),
                    ("Comment", CharacterProperties { foreground: Some(Color { red: 0.45, green: 0.48, blue: 0.51, alpha: 1.0 }), ..Default::default() }),
                    ("Strikethrough", CharacterProperties { strikethrough: Some(true), ..Default::default() }),
                ] {
                    sheet.character_styles.insert(name.into(), CharacterStyle { id: name.into(), based_on: None, properties });
                    sheet.character_metadata.insert(name.into(), StyleDefinitionMetadata::generated(name));
                }
                sheet
                    .block_styles
                    .get_mut(&StyleId("Code Block".into()))
                    .unwrap()
                    .block
                    .margin_left = Some(32.0);
            }
        }
        sheet
    }

    pub fn block_style_count(&self) -> usize {
        self.block_styles.len()
    }

    /// Select a structural family without manufacturing definitions for deep
    /// nesting.
    pub fn list_style_id(&self, ordered: bool, zero_based_level: u8) -> StyleId {
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
        let step = first.leading_indent.filter(|value| *value != 0.0).unwrap_or(32.0);
        for level in 1..=level {
            let id = StyleId(format!("List{level}"));
            if self.block_styles.contains_key(&id)
                || self.deleted_configuration_blocks.contains(&id)
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
                        margin_top: first.margin_top,
                        margin_bottom: first.margin_bottom,
                        ..Default::default()
                    },
                },
            );
            self.block_metadata.insert(
                id,
                StyleDefinitionMetadata {
                    display_name: format!("List Level {level}"),
                    origin: StyleDefinitionOrigin::GeneratedConfiguration,
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
        let metadata = match namespace {
            StyleNamespace::Block => self.block_style_metadata(id),
            StyleNamespace::Character => self.character_style_metadata(id),
        }
        .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
        if metadata.origin != StyleDefinitionOrigin::GeneratedConfiguration {
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

    /// Deleting a definition removes references in the same transaction.
    /// Children inherit from the deleted style's parent; following-paragraph
    /// references use its declared successor or Base Paragraph.
    pub(crate) fn rebase_definition_references_for_delete(
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

    pub fn insert_block_style(
        &mut self,
        style: BlockStyle,
        metadata: StyleDefinitionMetadata,
    ) -> Result<(), StyleError> {
        if style.id == self.base_paragraph {
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
        validate_definition_metadata(&style.id, &metadata)?;
        validate_character_properties(&style.id, &style.properties)?;
        if let Some(parent) = &style.based_on {
            if !self.character_styles.contains_key(parent) {
                return Err(StyleError::MissingParent(parent.clone()));
            }
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
        if id == &self.base_paragraph {
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
        let chain = self.document_chain(assigned)?;
        let mut resolved = ResolvedDocumentStyle::default();
        for style in chain {
            apply_document_properties(&mut resolved, &style.block);
            apply_character_properties(&mut resolved.character, &style.character);
        }
        apply_document_properties(&mut resolved, direct_canvas);
        apply_character_properties(&mut resolved.character, direct_character);
        validate_resolved_font_size(&resolved.character, assigned)?;
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

        let chain = self.document_chain(assigned)?;
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
        validate_resolved_font_size(&value.character, assigned)?;
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
        if style.role == BlockRole::CodeBlock { return Ok(&self.base_paragraph); }
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

    /// Resolve the generic cascade through Base Paragraph and source defaults,
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

    /// Base Paragraph appearance without source-authored or named overrides.
    pub fn default_paragraph_style(&self) -> Result<ResolvedParagraphStyle, StyleError> {
        self.resolve_paragraph_style(&self.base_paragraph, &self.base_paragraph, None,
            &BlockProperties::default(), &CharacterProperties::default())
    }

    /// The unscaled unit for native pane height commands, independent of text,
    /// font metrics, wrapping, and source-authored paragraph overrides.
    pub fn default_line_height(&self) -> Result<f32, StyleError> {
        let paragraph = self.default_paragraph_style()?;
        let size = paragraph.character.size;
        Ok(match paragraph.line_spacing {
            LineSpacing::Normal => size,
            LineSpacing::Multiplier(multiplier) => size * multiplier,
            LineSpacing::AtLeast(minimum) => size.max(minimum),
            LineSpacing::Exact(height) => height,
        })
    }

    /// Resolve the complete normalized cascade using the root and paragraph
    /// assignments stored by the formatted projection. The final character
    /// declarations are inline direct formatting, after an optional named
    /// character style.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_assigned_paragraph_style(
        &self, document: &DocumentStyleAssignment, paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties, paragraph_default_character: &CharacterProperties,
        character_style: Option<&StyleId>, direct_character: &CharacterProperties,
    ) -> Result<ResolvedParagraphStyle, StyleError> {
        self.resolve_assigned_paragraph_style_in_container(document, paragraph_style, direct_paragraph,
            &CharacterProperties::default(), paragraph_default_character, character_style, direct_character)
    }

    pub fn resolve_assigned_paragraph_style_in_container(
        &self,
        document: &DocumentStyleAssignment,
        paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties,
        container_character: &CharacterProperties,
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
        let document_chain = self.document_chain(&document.style)?;
        let role = self.block_style(paragraph_style).ok_or_else(|| StyleError::UnknownStyle(paragraph_style.clone()))?.role;
        if role == BlockRole::Document { return Err(StyleError::IncompatibleBlockRole { style: paragraph_style.clone(), role, parent_role: BlockRole::Paragraph }); }
        let paragraph_chain = self.block_chain(paragraph_style, role)?;
        let mut resolved = ResolvedParagraphStyle::default();

        for style in document_chain {
            apply_character_properties(&mut resolved.character, &style.character);
        }
        apply_character_properties(&mut resolved.character, &document.direct_default_character);
        apply_character_properties(&mut resolved.character, container_character);
        let mut parent_font_size = DEFAULT_FONT_SIZE;
        for style in paragraph_chain {
            if let Some(size) = style.character.size {
                parent_font_size = size.resolve(parent_font_size);
            }
            if style.role == BlockRole::Document || (role.is_container() && style.id == self.base_paragraph) {
                continue;
            }
            apply_paragraph_properties(&mut resolved, &style.block);
            if style.id != self.base_paragraph {
                apply_character_properties(&mut resolved.character, &style.character);
                if matches!(style.character.size, Some(FontSize::Percentage(_))) {
                    resolved.character.size = parent_font_size;
                }
            }
        }
        apply_paragraph_properties(&mut resolved, direct_paragraph);
        apply_character_properties(&mut resolved.character, paragraph_default_character);
        if let Some(id) = character_style {
            let underlying_size = resolved.character.size;
            for style in self.character_chain(id)? {
                apply_character_properties(&mut resolved.character, &style.properties);
                if let Some(FontSize::Percentage(percent)) = style.properties.size {
                    resolved.character.size = FontSize::Percentage(percent).resolve(underlying_size);
                }
            }
        }
        apply_character_properties(&mut resolved.character, direct_character);
        validate_resolved_font_size(&resolved.character, character_style.unwrap_or(paragraph_style))?;
        Ok(resolved)
    }

    /// Resolve a paragraph/character cascade with complete contribution
    /// metadata. This is the dependency-aware counterpart of
    /// [`Self::resolve_assigned_paragraph_style`].
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_assigned_paragraph_style_with_contributions(
        &self, document: &DocumentStyleAssignment, paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties, paragraph_default_character: &CharacterProperties,
        character_style: Option<&StyleId>, direct_character: &CharacterProperties,
    ) -> Result<ResolvedStyle<ResolvedParagraphStyle>, StyleError> {
        self.resolve_assigned_paragraph_style_in_container_with_contributions(document, paragraph_style, direct_paragraph,
            &CharacterProperties::default(), paragraph_default_character, character_style, direct_character)
    }

    pub fn resolve_assigned_paragraph_style_in_container_with_contributions(
        &self,
        document: &DocumentStyleAssignment,
        paragraph_style: &StyleId,
        direct_paragraph: &BlockProperties,
        container_character: &CharacterProperties,
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

        let document_chain = self.document_chain(&document.style)?;
        let role = self.block_style(paragraph_style).ok_or_else(|| StyleError::UnknownStyle(paragraph_style.clone()))?.role;
        if role == BlockRole::Document { return Err(StyleError::IncompatibleBlockRole { style: paragraph_style.clone(), role, parent_role: BlockRole::Paragraph }); }
        let paragraph_chain = self.block_chain(paragraph_style, role)?;
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
        apply_character_properties(&mut value.character, container_character);
        record_character_winners(&mut contributions, container_character, StyleContributionOrigin::DirectParagraphCharacter);
        let mut parent_font_size = DEFAULT_FONT_SIZE;
        for style in paragraph_chain {
            if let Some(size) = style.character.size {
                parent_font_size = size.resolve(parent_font_size);
            }
            if style.role == BlockRole::Document || (role.is_container() && style.id == self.base_paragraph) {
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
            if style.id != self.base_paragraph {
                apply_character_properties(&mut value.character, &style.character);
                if matches!(style.character.size, Some(FontSize::Percentage(_))) {
                    value.character.size = parent_font_size;
                }
                record_character_winners(&mut contributions, &style.character,
                    StyleContributionOrigin::BlockStyle(style.id.clone()));
            }
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
            let underlying_size = value.character.size;
            for style in self.character_chain(id)? {
                add_dependency(
                    &mut contributions,
                    CHARACTER_STYLE_PROPERTIES,
                    StyleDependency::Character(style.id.clone()),
                );
                apply_character_properties(&mut value.character, &style.properties);
                if let Some(FontSize::Percentage(percent)) = style.properties.size {
                    value.character.size = FontSize::Percentage(percent).resolve(underlying_size);
                }
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

        validate_resolved_font_size(&value.character, character_style.unwrap_or(paragraph_style))?;
        Ok(ResolvedStyle {
            value,
            contributions,
        })
    }

    pub fn resolve_container_style(&self, id: &StyleId) -> Result<ResolvedParagraphStyle, StyleError> {
        self.resolve_assigned_container_style(&DocumentStyleAssignment::new(self.base_paragraph.clone()),
            id, &BlockProperties::default(), &CharacterProperties::default())
    }

    pub fn resolve_assigned_container_style(&self, document: &DocumentStyleAssignment, id: &StyleId,
        direct: &BlockProperties, character: &CharacterProperties) -> Result<ResolvedParagraphStyle, StyleError> {
        let role = self.block_style(id).ok_or_else(|| StyleError::UnknownStyle(id.clone()))?.role;
        if !role.is_container() { return Err(StyleError::IncompatibleBlockRole { style: id.clone(), role, parent_role: BlockRole::Quote }); }
        self.resolve_assigned_paragraph_style(document, id, direct, character, None, &CharacterProperties::default())
    }

    /// Container text defaults compose down the document tree; box declarations do not.
    /// Base Paragraph is omitted because it already supplies the document's text defaults.
    pub fn container_character_declarations(&self, id: &StyleId, direct: &CharacterProperties)
        -> Result<CharacterProperties, StyleError> {
        let role = self.block_style(id).ok_or_else(|| StyleError::UnknownStyle(id.clone()))?.role;
        if !role.is_container() { return Err(StyleError::IncompatibleBlockRole { style: id.clone(), role, parent_role: BlockRole::Quote }); }
        let mut result = CharacterProperties::default();
        for style in self.block_chain(id, role)? {
            if style.id != self.base_paragraph { result.overlay(&style.character); }
        }
        result.overlay(direct);
        Ok(result)
    }

    /// CSS-inherited paragraph text defaults provided by one container style.
    /// Box geometry never participates in document-parent inheritance.
    pub fn container_paragraph_declarations(&self, id: &StyleId, direct: &BlockProperties)
        -> Result<BlockProperties, StyleError> {
        let role = self.block_style(id).ok_or_else(|| StyleError::UnknownStyle(id.clone()))?.role;
        if !role.is_container() { return Err(StyleError::IncompatibleBlockRole { style: id.clone(), role, parent_role: BlockRole::Quote }); }
        let mut result = BlockProperties::default();
        for style in self.block_chain(id, role)? {
            if style.id != self.base_paragraph { result.merge_declarations(&style.block); }
        }
        result.merge_declarations(direct);
        result.clear_box();
        result.leading_indent = None;
        result.trailing_indent = None;
        Ok(result)
    }

    pub fn apply_container_paragraph_defaults(&self, paragraph_style: &StyleId,
        direct: &BlockProperties, inherited: &BlockProperties, resolved: &mut ResolvedParagraphStyle)
        -> Result<(), StyleError> {
        let role = self.block_style(paragraph_style).ok_or_else(|| StyleError::UnknownStyle(paragraph_style.clone()))?.role;
        let chain = self.block_chain(paragraph_style, role)?;
        macro_rules! inherit { ($($field:ident),*) => {$(
            if direct.$field.is_none() && !chain.iter().any(|style| style.id != self.base_paragraph && style.block.$field.is_some()) {
                if let Some(value) = inherited.$field { resolved.$field = value; }
            }
        )*}; }
        inherit!(line_spacing, alignment, base_direction, first_line_indent);
        Ok(())
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
            (BlockRole::Document, BlockRole::Paragraph) => parent.id == self.base_paragraph,
            (BlockRole::Paragraph, BlockRole::Paragraph) => true,
            (role, parent_role) if role.is_container() => role == parent_role || parent.id == self.base_paragraph,
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
        if style.id == self.base_paragraph {
            if matches!(style.character.size, Some(FontSize::Percentage(_))) {
                return Err(invalid_style_value(&style.id, StyleProperty::CharacterSize));
            }
            if style.role != BlockRole::Paragraph
                || style.based_on.is_some()
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
        let metadata = self.character_metadata.get(&style.id).cloned()
            .ok_or_else(|| StyleError::InvalidDefinitionMetadata(style.id.clone()))?;
        self.insert_character_style(style, metadata)
    }

    fn validate_next_paragraph_style(&self, style: &BlockStyle) -> Result<(), StyleError> {
        let Some(next) = style.next_paragraph_style.as_ref() else {
            return Ok(());
        };
        if style.id == self.base_paragraph {
            return Err(StyleError::InvalidBaseStyleDefinition(style.id.clone()));
        }
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

    fn document_chain(&self, id: &StyleId) -> Result<Vec<&BlockStyle>, StyleError> {
        self.block_chain(id, if id == &self.base_paragraph { BlockRole::Paragraph } else { BlockRole::Document })
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

    /// Merge only the named character chain, retaining absent properties as
    /// contextual paragraph inheritance instead of filling them from one paragraph.
    pub(crate) fn named_character_declarations(
        &self,
        id: Option<&StyleId>,
    ) -> Result<CharacterProperties, StyleError> {
        let mut properties = CharacterProperties::default();
        if let Some(id) = id {
            for style in self.character_chain(id)? {
                properties.overlay(&style.properties);
            }
        }
        Ok(properties)
    }

    fn character_chain(&self, id: &StyleId) -> Result<Vec<&CharacterStyle>, StyleError> {
        if id.0.is_empty() { return Ok(Vec::new()); }
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
            current = style.based_on.as_ref();
        }
        chain.reverse();
        Ok(chain)
    }

    fn validate_block_cycles(&self) -> Result<(), StyleError> {
        for style in self.block_styles.values() {
            if style.id != self.base_paragraph {
                self.validate_block_parent(style)?;
            }
            self.validate_next_paragraph_style(style)?;
            let mut size = DEFAULT_FONT_SIZE;
            for parent in self.block_chain(&style.id, style.role)? {
                if let Some(value) = parent.character.size { size = value.resolve(size); }
                if !size.is_finite() || size <= 0.0 {
                    return Err(invalid_style_value(&style.id, StyleProperty::CharacterSize));
                }
            }
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

fn definition_block_property(role: BlockRole, property: StyleProperty) -> Result<StyleProperty, ()> {
    let canvas = matches!(property, StyleProperty::CanvasBackground | StyleProperty::CanvasPaddingTop
        | StyleProperty::CanvasPaddingRight | StyleProperty::CanvasPaddingBottom | StyleProperty::CanvasPaddingLeft);
    if canvas != (role == BlockRole::Document) { return Err(()); }
    Ok(match property {
        StyleProperty::CanvasBackground => StyleProperty::BlockBackground,
        StyleProperty::CanvasPaddingTop => StyleProperty::BlockPaddingTop,
        StyleProperty::CanvasPaddingRight => StyleProperty::BlockPaddingRight,
        StyleProperty::CanvasPaddingBottom => StyleProperty::BlockPaddingBottom,
        StyleProperty::CanvasPaddingLeft => StyleProperty::BlockPaddingLeft,
        property => property,
    })
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
                set_block_property(&style.id, &mut style.block, definition_block_property(style.role, *property)
                    .map_err(|_| inapplicable_style_property(&style.id, *property))?, value)
            }
        }
        StyleDefinitionFieldEdit::ClearDeclaration(property) => {
            if is_character_property(*property) {
                clear_character_property(&style.id, &mut style.character, *property)
            } else {
                clear_block_property(&style.id, &mut style.block, definition_block_property(style.role, *property)
                    .map_err(|_| inapplicable_style_property(&style.id, *property))?)
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
                    properties.$field = Some(value.clone().into());
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
    CharacterProperties, set_character_property_value, clear_character_property_value;
    font_families => CharacterFontFamilies(FontFamilies),
    font_face => CharacterFontFace(Text),
    font_axes => CharacterFontAxes(FontAxes),
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
}

pub(super) fn clear_character_property(
    style: &StyleId,
    properties: &mut CharacterProperties,
    property: StyleProperty,
) -> Result<(), StyleError> {
    clear_character_property_value(style, properties, property)?;
    if property == StyleProperty::CharacterFontFamilies {
        properties.font_face = None;
        properties.font_axes = None;
        properties.weight = None;
    }
    Ok(())
}

pub(super) fn set_character_property(
    style: &StyleId,
    properties: &mut CharacterProperties,
    property: StyleProperty,
    value: &StylePropertyValue,
) -> Result<(), StyleError> {
    if let (StyleProperty::CharacterSize, StylePropertyValue::Percentage(value)) = (property, value) {
        if !(10..=1000).contains(value) {
            return Err(invalid_style_value(style, property));
        }
        properties.size = Some(FontSize::Percentage(*value));
        Ok(())
    } else {
        set_character_property_value(style, properties, property, value)
    }
}

sparse_property_operations! {
    BlockProperties, set_block_property, clear_block_property;
    margin_right => BlockMarginRight(Float),
    margin_left => BlockMarginLeft(Float),
    border_top_width => BlockBorderTopWidth(Float),
    border_top_color => BlockBorderTopColor(Color),
    border_right_width => BlockBorderRightWidth(Float),
    border_right_color => BlockBorderRightColor(Color),
    border_bottom_width => BlockBorderBottomWidth(Float),
    border_bottom_color => BlockBorderBottomColor(Color),
    border_left_width => BlockBorderLeftWidth(Float),
    border_left_color => BlockBorderLeftColor(Color),
    margin_top => BlockMarginTop(Float),
    margin_bottom => BlockMarginBottom(Float),
    line_spacing => ParagraphLineSpacing(LineSpacing),
    first_line_indent => ParagraphFirstLineIndent(Float),
    leading_indent => ParagraphLeadingIndent(Float),
    trailing_indent => ParagraphTrailingIndent(Float),
    padding_top => BlockPaddingTop(Float),
    padding_right => BlockPaddingRight(Float),
    padding_bottom => BlockPaddingBottom(Float),
    padding_left => BlockPaddingLeft(Float),
    background => BlockBackground(Color),
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
        && properties.font_face.as_ref().map_or(true, |name| name.len() <= 1024 && !name.chars().any(char::is_control))
        && language_valid
        && features_valid
        && properties.font_axes.as_ref().map_or(true, |axes| {
            axes.len() <= 64
                && axes.iter().all(|(tag, value)| {
                    tag.len() == 4
                        && tag.bytes().all(|b| (0x20..=0x7e).contains(&b))
                        && value.is_finite()
                })
        })
        && properties
            .size
            .map_or(true, FontSize::is_valid)
        && properties
            .weight
            .map_or(true, |weight| (1..=1000).contains(&weight))
        && properties.foreground.map_or(true, valid_color)
        && properties.background.map_or(true, valid_color)
        && properties
            .letter_spacing
            .map_or(true, |value| value.is_finite())
;
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
    let invalid_internal = id.is_internal()
        && (metadata.origin != StyleDefinitionOrigin::GeneratedConfiguration
            || metadata.display_name != if *id == StyleId::incremental_match() {
                "Incremental match"
            } else {
                id.0.as_str()
            });
    if metadata.display_name.trim().is_empty() || metadata.display_name.contains('\0') || invalid_internal {
        Err(StyleError::InvalidDefinitionMetadata(id.clone()))
    } else {
        Ok(())
    }
}

fn validate_block_properties(style: &BlockStyle) -> Result<(), StyleError> {
    let block = &style.block;
    let paragraph_declared = declares_paragraph_properties(block);
    if style.role == BlockRole::Document && paragraph_declared
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
        block.margin_right,
        block.margin_left,
        block.margin_top,
        block.margin_bottom,
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
    let box_valid = [block.padding_top,block.padding_right,block.padding_bottom,block.padding_left,block.border_top_width,block.border_right_width,block.border_bottom_width,block.border_left_width].into_iter().flatten().all(|v| v.is_finite() && v >= 0.0)
        && [block.border_top_color,block.border_right_color,block.border_bottom_color,block.border_left_color].into_iter().flatten().all(valid_color);
    if finite && spacing_valid && box_valid {
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
    block.margin_right.is_some()
        || block.margin_left.is_some()
        || block.border_top_width.is_some()
        || block.border_top_color.is_some()
        || block.border_right_width.is_some()
        || block.border_right_color.is_some()
        || block.border_bottom_width.is_some()
        || block.border_bottom_color.is_some()
        || block.border_left_width.is_some()
        || block.border_left_color.is_some()
        || block.margin_top.is_some()
        || block.margin_bottom.is_some()
        || block.line_spacing.is_some()
        || block.first_line_indent.is_some()
        || block.leading_indent.is_some()
        || block.trailing_indent.is_some()
        || block.alignment.is_some()
        || block.base_direction.is_some()
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
    if properties.font_face.is_some() || properties.font_families.is_some() {
        record_winner(contributions, StyleProperty::CharacterFontFace, &origin);
    }
    if properties.font_axes.is_some() || properties.font_families.is_some() {
        record_winner(contributions, StyleProperty::CharacterFontAxes, &origin);
    }
    if properties.size.is_some() {
        record_winner(contributions, StyleProperty::CharacterSize, &origin);
    }
    if properties.bold.is_some() {
        record_winner(contributions, StyleProperty::CharacterBold, &origin);
    }
    if properties.weight.is_some() || properties.font_families.is_some() {
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
    if properties.margin_right.is_some() { record_winner(contributions, StyleProperty::BlockMarginRight, &origin); }
    if properties.margin_left.is_some() { record_winner(contributions, StyleProperty::BlockMarginLeft, &origin); }
    if properties.padding_top.is_some() { record_winner(contributions, StyleProperty::BlockPaddingTop, &origin); }
    if properties.padding_right.is_some() { record_winner(contributions, StyleProperty::BlockPaddingRight, &origin); }
    if properties.padding_bottom.is_some() { record_winner(contributions, StyleProperty::BlockPaddingBottom, &origin); }
    if properties.padding_left.is_some() { record_winner(contributions, StyleProperty::BlockPaddingLeft, &origin); }
    if properties.border_top_width.is_some() { record_winner(contributions, StyleProperty::BlockBorderTopWidth, &origin); }
    if properties.border_top_color.is_some() { record_winner(contributions, StyleProperty::BlockBorderTopColor, &origin); }
    if properties.border_right_width.is_some() { record_winner(contributions, StyleProperty::BlockBorderRightWidth, &origin); }
    if properties.border_right_color.is_some() { record_winner(contributions, StyleProperty::BlockBorderRightColor, &origin); }
    if properties.border_bottom_width.is_some() { record_winner(contributions, StyleProperty::BlockBorderBottomWidth, &origin); }
    if properties.border_bottom_color.is_some() { record_winner(contributions, StyleProperty::BlockBorderBottomColor, &origin); }
    if properties.border_left_width.is_some() { record_winner(contributions, StyleProperty::BlockBorderLeftWidth, &origin); }
    if properties.border_left_color.is_some() { record_winner(contributions, StyleProperty::BlockBorderLeftColor, &origin); }
    if properties.background.is_some() { record_winner(contributions, StyleProperty::BlockBackground, &origin); }

    if properties.margin_top.is_some() {
        record_winner(
            contributions,
            StyleProperty::BlockMarginTop,
            &origin,
        );
    }
    if properties.margin_bottom.is_some() {
        record_winner(contributions, StyleProperty::BlockMarginBottom, &origin);
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

fn validate_resolved_font_size(style: &ResolvedCharacterStyle, id: &StyleId) -> Result<(), StyleError> {
    if style.size.is_finite() && style.size > 0.0 { Ok(()) }
    else { Err(invalid_style_value(id, StyleProperty::CharacterSize)) }
}

fn apply_character_properties(
    resolved: &mut ResolvedCharacterStyle,
    properties: &CharacterProperties,
) {
    if let Some(value) = properties.font_families.as_ref() {
        resolved.font_families.clone_from(value);
        resolved.font_face.clear();
        resolved.font_axes.clear();
        resolved.base_weight = 400;
    }
    if let Some(value) = properties.font_face.as_ref() {
        resolved.font_face.clone_from(value);
    }
    if let Some(value) = properties.font_axes.as_ref() {
        resolved.font_axes.clone_from(value);
    }
    if let Some(value) = properties.size {
        resolved.size = value.resolve(resolved.size);
    }
    if let Some(value) = properties.weight {
        resolved.base_weight = value;
        if properties.font_families.is_none() {
            resolved.bold = false;
        }
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
    if let Some(value) = properties.margin_right { resolved.margin_right = value; }
    if let Some(value) = properties.margin_left { resolved.margin_left = value; }
    if let Some(value) = properties.padding_top { resolved.padding_top = value; }
    if let Some(value) = properties.padding_right { resolved.padding_right = value; }
    if let Some(value) = properties.padding_bottom { resolved.padding_bottom = value; }
    if let Some(value) = properties.padding_left { resolved.padding_left = value; }
    if let Some(value) = properties.border_top_width { resolved.border_top_width = value; }
    if let Some(value) = properties.border_top_color { resolved.border_top_color = Some(value); }
    if let Some(value) = properties.border_right_width { resolved.border_right_width = value; }
    if let Some(value) = properties.border_right_color { resolved.border_right_color = Some(value); }
    if let Some(value) = properties.border_bottom_width { resolved.border_bottom_width = value; }
    if let Some(value) = properties.border_bottom_color { resolved.border_bottom_color = Some(value); }
    if let Some(value) = properties.border_left_width { resolved.border_left_width = value; }
    if let Some(value) = properties.border_left_color { resolved.border_left_color = Some(value); }
    if let Some(value) = properties.background { resolved.background = Some(value); }

    if let Some(value) = properties.margin_top {
        resolved.margin_top = value;
    }
    if let Some(value) = properties.margin_bottom {
        resolved.margin_bottom = value;
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
    Direct(CharacterProperties),
    Semantic(SemanticInlineStyle),
}

impl StyleApplication {
    pub(super) fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Named(id) | Self::Automatic(id) => id.0.capacity() + 16,

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
            based_on: (!parent.0.is_empty()).then(|| parent.clone()),
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
    fn generated_and_emergency_fonts_follow_the_host_platform() {
        let expected = if cfg!(target_os = "macos") { "SF Pro" }
            else if cfg!(target_os = "windows") { "Segoe UI" } else { "system-ui" };
        assert_eq!(DEFAULT_FONT_FAMILY, expected);
        for format in [crate::document::Format::PlainText, crate::document::Format::Markdown,
            crate::document::Format::MarkdownSource,
] {
            let sheet = StyleSheet::for_format(format);
            assert_eq!(sheet.block_style(&sheet.base_paragraph).unwrap().character.font_families,
                Some(vec![expected.to_owned()]), "{format:?}");
        }
        assert_eq!(ResolvedCharacterStyle::default().font_families, [expected]);
        assert_eq!(crate::layout::ResolvedTextStyle::default().font_families, [expected]);
        let code = code::default_sheet();
        assert_eq!(code.block_style(&code.base_paragraph).unwrap().character.font_families,
            Some(vec!["monospace".into()]));
    }

    #[test]
    fn pane_line_units_use_base_size_and_spacing_without_font_metrics() {
        for format in [crate::document::Format::PlainText, crate::document::Format::Code,
            crate::document::Format::Markdown, crate::document::Format::MarkdownSource] {
            let mut sheet = StyleSheet::for_format(format);
            let base = sheet.block_styles.get_mut(&sheet.base_paragraph).unwrap();
            base.character.size = Some(FontSize::Points(20.0));
            for (spacing, expected) in [(LineSpacing::Normal, 20.0),
                (LineSpacing::Multiplier(1.5), 30.0), (LineSpacing::AtLeast(12.0), 20.0),
                (LineSpacing::AtLeast(24.0), 24.0), (LineSpacing::Exact(12.0), 12.0)] {
                sheet.block_styles.get_mut(&sheet.base_paragraph).unwrap().block.line_spacing = Some(spacing);
                assert_eq!(sheet.default_line_height().unwrap(), expected, "{format:?}: {spacing:?}");
            }
        }
    }

    #[test]
    fn base_paragraph_is_the_only_styling_root() {
        let sheet = StyleSheet::default();
        let paragraph = sheet.block_style(&sheet.base_paragraph).unwrap();
        assert_eq!(paragraph.based_on, None);
        assert_eq!(paragraph.character.size, Some(14.0.into()));
        assert_eq!(paragraph.character.font_families, Some(vec![DEFAULT_FONT_FAMILY.into()]));
        assert_eq!(sheet.block_style_metadata(&sheet.base_paragraph).unwrap().display_name, "Base Paragraph");
        assert!(sheet.block_style(&"Document".into()).is_none());
        assert!(sheet.character_style(&"Character".into()).is_none());
        assert_eq!(sheet.character_style(&"Code".into()).unwrap().based_on, None);
        assert!(paragraph.block.padding_top.is_none());
        let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        let plain = sheet.resolve_assigned_paragraph_style(&assignment, &sheet.base_paragraph,
            &Default::default(), &Default::default(), Some(&"Code".into()), &Default::default()).unwrap();
        let heading = sheet.resolve_assigned_paragraph_style(&assignment, &"Heading1".into(),
            &Default::default(), &Default::default(), Some(&"Code".into()), &Default::default()).unwrap();
        assert_eq!(plain.character.size, 14.0);
        assert_eq!(heading.character.size, 24.0);
        assert_eq!(heading.character.font_families, vec!["monospace"]);
        assert_eq!(heading.character.foreground, plain.character.foreground);
    }

    #[test]
    fn generated_field_edits_derive_authority_from_definition_metadata() {
        let mut sheet = StyleSheet::default();
        let computed_id = StyleId::from("computed-block-17");
        let computed_style = paragraph_style(&computed_id.0, &sheet.base_paragraph);
        sheet
            .insert_block_style(
                computed_style,
                StyleDefinitionMetadata {
                    display_name: "Computed Body".to_owned(),
                    origin: StyleDefinitionOrigin::SyntheticReadOnly,
                },
            )
            .unwrap();
        let synthetic_id = StyleId::from("synthetic-token-3");
        sheet
            .insert_character_style(
                character_style(
                    &synthetic_id.0,
                    &StyleId::from(""),
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
                &computed_id,
                &StyleDefinitionFieldEdit::SetDeclaration {
                    property: StyleProperty::BlockMarginBottom,
                    value: StylePropertyValue::Float(8.0),
                },
            ),
            Err(StyleError::DefinitionNotGeneratedConfiguration {
                style: computed_id,
                origin: StyleDefinitionOrigin::SyntheticReadOnly,
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
            based_on: Some(sheet.base_paragraph.clone()),
            next_paragraph_style: None,
            role: BlockRole::Document,
            character: CharacterProperties {
                font_families: Some(vec!["Writer Serif".to_owned(), "system-ui".to_owned()]),
                size: Some(19.0.into()),
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
        assert_eq!(resolved.margin_top, 0.0);
        assert_eq!(resolved.line_spacing, LineSpacing::Normal);
    }

    #[test]
    fn cascade_orders_document_paragraph_named_character_and_direct_layers() {
        let mut sheet = StyleSheet::default();
        let named = character_style(
            "Callout",
            &StyleId::from(""),
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
                &sheet.base_paragraph,
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
        let mut root = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        root.direct_canvas.margin_top = Some(1.0);
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
        let invalid_padding = BlockStyle {
            block: BlockProperties {
                padding_left: Some(-10.0),
                ..BlockProperties::default()
            },
            ..paragraph_style("BadParagraph", &sheet.base_paragraph)
        };
        assert!(matches!(
            sheet.insert_block_style(invalid_padding, generated()),
            Err(StyleError::InvalidBlockProperties(_))
        ));

        let document_child = BlockStyle {
            id: "DocumentChild".into(),
            based_on: Some(sheet.base_paragraph.clone()),
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
            based_on: Some(sheet.base_paragraph.clone()),
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
            sheet.remove_block_style(&sheet.base_paragraph.clone(), false),
            Err(StyleError::CannotRemoveBaseStyle(
                sheet.base_paragraph.clone()
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
            &StyleId::from(""),
            CharacterProperties {
                size: Some((f32::NAN).into()),
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
                based_on: None,
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
                &sheet.base_paragraph,
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
                &sheet.base_paragraph,
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
                    based_on: Some(sheet.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Document,
                    character: CharacterProperties {
                        size: Some(18.0.into()),
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
        assert_eq!(traced.contributions().len(),
            CANVAS_STYLE_PROPERTIES.len() + CHARACTER_STYLE_PROPERTIES.len()
        );
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
                StyleDependency::Block(sheet.base_paragraph.clone()),
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
                        size: Some(20.0.into()),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        margin_top: Some(9.0),
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
                    based_on: None,
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
        let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());

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
        assert_eq!(traced.contributions().len(), PARAGRAPH_STYLE_PROPERTIES.len() + CHARACTER_STYLE_PROPERTIES.len());
        assert_eq!(
            traced
                .contribution(StyleProperty::BlockMarginTop)
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
            weight_dependencies.contains(&StyleDependency::Block(sheet.base_paragraph.clone()))
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
                    &StyleId::from(""),
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
                            size: (bits & 1 != 0).then_some(FontSize::Points(10.0 + (bits % 18) as f32)),
                            weight: (bits & 2 != 0).then_some(300 + (bits % 6) as u16 * 100),
                            underline: (bits & 4 != 0).then_some(bits & 8 != 0),
                            ..CharacterProperties::default()
                        },
                        block: BlockProperties {
                            margin_top: (bits & 16 != 0).then_some((bits % 12) as f32),
                            leading_indent: (bits & 32 != 0).then_some((bits % 20) as f32),
                            ..BlockProperties::default()
                        },
                    },
                    generated(),
                )
                .unwrap();
            paragraph_ids.push(id);
        }

        let mut character_ids = vec![StyleId::from("Code")];
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

        let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        for _ in 0..256 {
            let paragraph = &paragraph_ids[next() as usize % paragraph_ids.len()];
            let character = &character_ids[next() as usize % character_ids.len()];
            let direct_paragraph = BlockProperties {
                trailing_indent: (next() & 1 != 0).then_some((next() % 16) as f32),
                ..BlockProperties::default()
            };
            let direct_character = CharacterProperties {
                letter_spacing: (next() & 1 != 0).then_some((next() % 4) as f32),
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
            assert_eq!(traced.contributions().len(), PARAGRAPH_STYLE_PROPERTIES.len() + CHARACTER_STYLE_PROPERTIES.len());
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
            &StyleId::from(""),
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
                &StyleId::from(""),
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

#[cfg(test)]
mod block_box_tests {
    use super::*;

    #[test]
    fn container_boxes_have_typed_roles_and_sparse_inherited_sides() {
        let mut sheet = StyleSheet::default();
        let color = Color { red: 0.6, green: 0.2, blue: 0.7, alpha: 0.5 };
        let mut quote = sheet.block_style(&"Block quote".into()).unwrap().clone();
        quote.block.margin_top = Some(-3.0);
        quote.block.padding_right = Some(11.0);
        quote.block.border_top_width = Some(2.5);
        quote.block.border_top_color = Some(color);
        quote.block.background = Some(color);
        sheet.insert_block_style(quote, StyleDefinitionMetadata::generated("Block quote")).unwrap();
        sheet.insert_block_style(BlockStyle {
            id: "Pull quote".into(), based_on: Some("Block quote".into()), next_paragraph_style: None,
            role: BlockRole::Quote, character: CharacterProperties::default(),
            block: BlockProperties { padding_right: Some(4.0), border_top_width: Some(0.0), ..Default::default() },
        }, StyleDefinitionMetadata::generated("Pull quote")).unwrap();
        let resolved = sheet.resolve_container_style(&"Pull quote".into()).unwrap();
        assert_eq!(resolved.margin_top, -3.0);
        assert_eq!(resolved.padding_right, 4.0);
        assert_eq!(resolved.border_top_width, 0.0);
        assert_eq!(resolved.border_top_color, Some(color));
        assert_eq!(resolved.background, Some(color));
        assert_eq!(resolved.margin_left + resolved.border_left_width + resolved.padding_left, 32.0);
        assert_eq!(sheet.block_style(&"Code Block".into()).unwrap().role, BlockRole::CodeBlock);
        assert_eq!(sheet.block_style(&"Bulleted List".into()).unwrap().role, BlockRole::List);
        assert_eq!(sheet.block_style(&"List item".into()).unwrap().role, BlockRole::ListItem);
        assert!(sheet.insert_block_style(BlockStyle {
            id: "Bad kind".into(), based_on: Some("Block quote".into()), next_paragraph_style: None,
            role: BlockRole::CodeBlock, character: CharacterProperties::default(), block: BlockProperties::default(),
        }, StyleDefinitionMetadata::generated("Bad kind")).is_err());
    }

    #[test]
    fn block_declarations_validate_clear_and_track_layout_or_paint() {
        let mut sheet = StyleSheet::default();
        let id = StyleId::from("Heading1");
        for (property, value) in [
            (StyleProperty::BlockPaddingBottom, StylePropertyValue::Float(9.0)),
            (StyleProperty::BlockMarginLeft, StylePropertyValue::Float(-5.0)),
            (StyleProperty::BlockBorderRightWidth, StylePropertyValue::Float(2.0)),
        ] {
            let edit = sheet.prepare_generated_field_edit(StyleNamespace::Block, &id,
                &StyleDefinitionFieldEdit::SetDeclaration { property, value }).unwrap();
            sheet.apply_configuration_edit(&edit, StyleSheetRevision(sheet.revision.0 + 1), false).unwrap();
        }
        let resolved = sheet.resolve_assigned_paragraph_style_with_contributions(
            &DocumentStyleAssignment::new(sheet.base_paragraph.clone()), &id, &BlockProperties::default(),
            &CharacterProperties::default(), None, &CharacterProperties::default()).unwrap();
        assert_eq!(resolved.value.padding_bottom, 9.0);
        assert_eq!(resolved.contribution(StyleProperty::BlockPaddingBottom).unwrap().winner,
            StyleContributionOrigin::BlockStyle(id.clone()));
        let invalid = sheet.prepare_generated_field_edit(StyleNamespace::Block, &id,
            &StyleDefinitionFieldEdit::SetDeclaration { property: StyleProperty::BlockPaddingBottom, value: StylePropertyValue::Float(-1.0) }).unwrap();
        assert!(sheet.apply_configuration_edit(&invalid, StyleSheetRevision(sheet.revision.0 + 1), false).is_err());
        let clear = sheet.prepare_generated_field_edit(StyleNamespace::Block, &id,
            &StyleDefinitionFieldEdit::ClearDeclaration(StyleProperty::BlockPaddingBottom)).unwrap();
        sheet.apply_configuration_edit(&clear, StyleSheetRevision(sheet.revision.0 + 1), false).unwrap();
        assert_eq!(sheet.block_style(&id).unwrap().block.padding_bottom, None);
        assert!(sheet.prepare_generated_field_edit(StyleNamespace::Character, &"Code".into(),
            &StyleDefinitionFieldEdit::SetDeclaration { property: StyleProperty::BlockPaddingBottom, value: StylePropertyValue::Float(1.0) }).is_err());
        assert_eq!(StyleProperty::BlockBorderLeftWidth.invalidation_effect(), StyleInvalidationEffect::ParagraphLayout);
        assert_eq!(StyleProperty::BlockBorderLeftColor.invalidation_effect(), StyleInvalidationEffect::Paint);
        assert_eq!(StyleProperty::BlockBackground.invalidation_effect(), StyleInvalidationEffect::Paint);
    }

    #[test]
    fn container_text_defaults_compose_without_inheriting_its_box_into_paragraphs() {
        let mut sheet = StyleSheet::default();
        let blue = Color { red: 0.0, green: 0.0, blue: 1.0, alpha: 1.0 };
        let mut quote = sheet.block_style(&"Block quote".into()).unwrap().clone();
        quote.character.foreground = Some(blue);
        quote.character.size = Some(18.0.into());
        quote.block.padding_top = Some(20.0);
        sheet.insert_block_style(quote, StyleDefinitionMetadata::generated("Block quote")).unwrap();
        let context = sheet.container_character_declarations(&"Block quote".into(), &CharacterProperties::default()).unwrap();
        let document = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        let paragraph = sheet.resolve_assigned_paragraph_style_in_container(&document, &sheet.base_paragraph,
            &BlockProperties::default(), &context, &CharacterProperties::default(), None, &CharacterProperties::default()).unwrap();
        assert_eq!(paragraph.character.foreground, blue);
        assert_eq!(paragraph.character.size, 18.0);
        assert_eq!(paragraph.padding_top, 0.0);
        let heading = sheet.resolve_assigned_paragraph_style_in_container(&document, &"Heading1".into(),
            &BlockProperties::default(), &context, &CharacterProperties::default(), None, &CharacterProperties::default()).unwrap();
        assert_eq!(heading.character.foreground, blue);
        assert_eq!(heading.character.size, 24.0);
    }
}

#[cfg(test)]
mod variable_font_tests {
    use super::*;
    #[test]
    fn coordinates_round_trip_and_inherit_as_one_font_face() {
        let base = CharacterProperties {
            font_families: Some(vec!["Variable Serif".into()]),
            font_face: Some("Condensed Light".into()),
            font_axes: Some(BTreeMap::from([
                ("wght".into(), 450.25),
                ("wdth".into(), 87.5),
            ])),
            weight: Some(450),
            ..Default::default()
        };
        let json = serde_json::to_string(&base).unwrap();
        assert_eq!(
            serde_json::from_str::<CharacterProperties>(&json).unwrap(),
            base
        );
        let mut sheet = StyleSheet::default();
        sheet
            .block_styles
            .get_mut(&sheet.base_paragraph.clone())
            .unwrap()
            .character = base.clone();
        let bytes = sheet
            .default_configuration_json(&DocumentStyleAssignment::new(sheet.base_paragraph.clone()))
            .unwrap();
        let (reopened, diagnostics) = StyleSheet::default().with_default_json(&bytes).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(
            reopened
                .block_style(&reopened.base_paragraph)
                .unwrap()
                .character
                .font_axes,
            base.font_axes
        );
        let mut resolved = ResolvedCharacterStyle::default();
        apply_character_properties(&mut resolved, &base);
        apply_character_properties(
            &mut resolved,
            &CharacterProperties {
                bold: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(resolved.font_face, "Condensed Light");
        assert_eq!(resolved.font_axes, base.font_axes.clone().unwrap());
        assert_eq!(resolved.weight, 750);
        apply_character_properties(
            &mut resolved,
            &CharacterProperties {
                bold: Some(false),
                ..Default::default()
            },
        );
        assert_eq!(resolved.weight, 450);
        apply_character_properties(
            &mut resolved,
            &CharacterProperties {
                font_families: Some(vec!["Other Font".into()]),
                ..Default::default()
            },
        );
        assert!(resolved.font_face.is_empty());
        assert!(resolved.font_axes.is_empty());
        assert_eq!(resolved.base_weight, 400);
        let traced = sheet.resolve_document_style_with_contributions(&sheet.base_paragraph, &BlockProperties::default(), &CharacterProperties { font_families: Some(vec!["Other Font".into()]), ..Default::default() }).unwrap();
        assert_eq!(traced.contribution(StyleProperty::CharacterFontAxes).unwrap().winner, StyleContributionOrigin::DirectDocumentCharacter);
        assert_eq!(traced.contribution(StyleProperty::CharacterWeight).unwrap().winner, StyleContributionOrigin::DirectDocumentCharacter);
    }
    #[test]
    fn clearing_face_preserves_independent_size_and_emphasis() {
        let mut properties = CharacterProperties {
            font_families: Some(vec!["Variable Serif".into()]),
            font_face: Some("Condensed Light".into()),
            font_axes: Some(BTreeMap::from([("wght".into(), 425.5)])),
            weight: Some(425),
            size: Some(18.0.into()),
            bold: Some(true),
            slant: Some(FontSlant::Italic),
            ..Default::default()
        };
        clear_character_property(
            &"Test".into(),
            &mut properties,
            StyleProperty::CharacterFontFamilies,
        )
        .unwrap();
        assert!(
            properties.font_families.is_none()
                && properties.weight.is_none()
                && properties.font_face.is_none()
                && properties.font_axes.is_none()
        );
        assert_eq!(properties.bold, Some(true));
        assert_eq!(properties.slant, Some(FontSlant::Italic));
        assert_eq!(properties.size, Some(18.0.into()));
    }
    #[test]
    fn invalid_coordinates_are_rejected_and_axis_changes_invalidate_shaping() {
        for (tag, value) in [("bad", 1.0), ("wght", f32::NAN), ("wght", f32::INFINITY)] {
            let properties = CharacterProperties {
                font_axes: Some(BTreeMap::from([(tag.into(), value)])),
                ..Default::default()
            };
            assert!(validate_character_properties(&"Test".into(), &properties).is_err());
        }
        let mut resolved = ResolvedCharacterStyle::default();
        let before = resolved.clone();
        resolved.font_face = "Condensed".into();
        assert!(resolved.changed_properties(&before).contains(&StyleProperty::CharacterFontFace));
        assert_eq!(StyleProperty::CharacterFontFace.invalidation_effect(), StyleInvalidationEffect::Shaping);
        resolved.font_axes.insert("wdth".into(), 75.0);
        assert!(before
            .changed_properties(&resolved)
            .contains(&StyleProperty::CharacterFontAxes));
        assert_eq!(
            StyleProperty::CharacterFontAxes.invalidation_effect(),
            StyleInvalidationEffect::Shaping
        );
        let shape = crate::layout::shaping_style(&resolved).unwrap();
        assert_eq!(shape.font_face, "Condensed");
        assert_eq!(shape.font_axes["wdth"], 75.0);
    }
}
