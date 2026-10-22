use super::encoding::DecodingDiagnostic;
#[path = "markdown_html.rs"]
mod markdown_html;
#[path = "provenance_runs.rs"]
mod provenance_runs;
use provenance_runs::{ProvenanceIndex, ProvenanceRun};

use super::formatted_text::{
    FormattedTextError, FormattedTextTree, LeafBoundarySide, LogicalGraphemeSnapshot,
};
use super::line_endings::{LogicalUnit, NormalizedText};
use super::position::ProjectedAnchorBacking;
use super::range_index::{IntervalRangeStore, OrderedRangeStore, RangeSpliceStats, RangedItem};
use super::style::{
    BlockProperties, CharacterProperties, DocumentStyleAssignment, SemanticInlineStyle,
    StyleApplication, StyleId, StyleSheet,
};
use super::{
    Association, BoundaryAffinity, DeletionRecovery, DocumentId, MappingOutcome, PositionDomain,
    PositionError, PositionMap, ProjectedBlockBoundary, ProjectedLeafBoundary, Revision,
    SourceAnchorProvenance, SourcePartId, TextAnchor, TextEdit, TextPoint, UnresolvableAnchor,
};
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    PlainText,
    /// Markdown presented as formatted text (Markdown Preview). Its source is
    /// edited literally as Code with the Markdown language.
    Markdown,
    /// Literal source text with disposable, asynchronously computed syntax styles.
    Code,
}

impl Format {
    pub const fn is_code(self) -> bool { matches!(self, Self::Code) }

    /// Formats with identity semantics after decoding and physical line endings.
    pub const fn is_literal(self) -> bool { matches!(self, Self::PlainText | Self::Code) }

    /// Markdown presented as formatted text.
    pub const fn is_markdown(self) -> bool {
        matches!(self, Self::Markdown)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    ListItem {
        ordered: bool,
        ordinal: u64,
        level: u8,
        container_start: bool,
        item_start: bool,
        /// WYSIWYG labels are layout decorations with no logical text range.
        /// Source-visible modes retain their literal marker syntax in text.
        marker_is_decoration: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListStyle {
    Bullet,
    Numbered,
}

#[derive(Clone, Debug)]
pub struct Block {
    /// Stable within the owning document across projection revisions. The
    /// projector initially emits zero as a private provisional value; a
    /// [`super::Document`] assigns or reconciles a globally fresh identity
    /// before publishing the projection.
    pub id: u64,
    pub range: Range<usize>,
    /// Non-positional declarations share one immutable allocation, including
    /// one static allocation for the default paragraph in literal documents.
    pub attributes: Arc<BlockAttributes>,
}

/// Semantic code owner; presentation and authoring policy come from this kind.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CodeBlockKind {
    #[default]
    Ordinary,
    FrontMatter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodeClipboardRepresentation { Source, FencedYaml }

impl CodeBlockKind {
    pub fn language_editable(self) -> bool { self == Self::Ordinary }
    pub fn treatment_removable(self) -> bool { self == Self::Ordinary }
    pub fn supplies_language_default(self) -> bool { self == Self::Ordinary }
    pub fn creatable_in_preview(self) -> bool { self == Self::Ordinary }
    pub fn clipboard_representation(self) -> CodeClipboardRepresentation {
        match self { Self::Ordinary => CodeClipboardRepresentation::Source,
            Self::FrontMatter => CodeClipboardRepresentation::FencedYaml }
    }
    pub fn fixed_label(self) -> Option<&'static str> {
        match self { Self::Ordinary => None, Self::FrontMatter => Some("front matter") }
    }
}

#[derive(Clone, Debug)]
pub struct BlockAttributes {
    /// Adapter support retained by the parser, including semantic item starts.
    pub(super) list_editing: ListEditing,
    pub kind: BlockKind,
    /// Independent Markdown containers; a heading/code style does not erase them.
    pub quote_depth: usize,
    /// Structural ancestry, ordered outermost first. Paragraphs remain leaves.
    pub containers: Arc<[super::ContainerMembership]>,
    pub list_loose: bool,
    pub thematic_break: bool,
    pub markdown_html: bool,
    /// Semantic owner policy, independent of its Code Block appearance.
    pub code_kind: CodeBlockKind,
    /// Authored fenced-code language; empty means unspecified.
    pub code_language: Option<String>,
    pub(crate) code_language_label: bool,
    pub style: StyleId,
    /// Absent for the overwhelmingly common inherited paragraph. Nonempty
    /// declarations are immutable and shared by projection/history clones.
    pub direct_formatting: Option<Arc<BlockDirectFormatting>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ListEditing {
    pub item_start: bool,
    pub indent: bool,
    pub unindent: bool,
}

impl Block {
    pub fn is_front_matter(&self) -> bool { self.code_kind == CodeBlockKind::FrontMatter }

    pub(crate) fn language_label(&self) -> String {
        self.code_kind.fixed_label().map(str::to_owned).unwrap_or_else(||
            self.code_language.as_deref().map(super::syntax::languages::display_name)
                .unwrap_or_else(|| "None".into()))
    }

    pub fn new(id: u64, range: Range<usize>, kind: BlockKind, style: StyleId,
        direct_formatting: Option<Arc<BlockDirectFormatting>>) -> Self {
        if kind == BlockKind::Paragraph && style.0 == "Paragraph" && direct_formatting.is_none() {
            return Self::paragraph(id, range);
        }
        let list_editing = match kind {
            BlockKind::ListItem { item_start, .. } => ListEditing { item_start, indent: true, unindent: true },
            _ => ListEditing::default(),
        };
        Self { id, range, attributes: Arc::new(BlockAttributes { kind, style, direct_formatting, list_editing, quote_depth: 0, containers: Arc::from([]), list_loose: false, thematic_break: false, markdown_html: false, code_kind: CodeBlockKind::Ordinary, code_language: None, code_language_label: false }) }
    }

    pub fn paragraph(id: u64, range: Range<usize>) -> Self {
        static DEFAULT: OnceLock<Arc<BlockAttributes>> = OnceLock::new();
        Self { id, range, attributes: DEFAULT.get_or_init(|| Arc::new(BlockAttributes {
            kind: BlockKind::Paragraph, style: "Paragraph".into(), direct_formatting: None, quote_depth: 0, containers: Arc::from([]), list_loose: false, thematic_break: false, markdown_html: false, code_kind: CodeBlockKind::Ordinary, code_language: None, code_language_label: false,
            list_editing: ListEditing::default(),
        })).clone() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockDirectFormatting {
    pub direct_paragraph: BlockProperties,
    pub direct_default_character: CharacterProperties,
}

impl BlockDirectFormatting {
    pub fn shared(
        direct_paragraph: BlockProperties,
        direct_default_character: CharacterProperties,
    ) -> Option<Arc<Self>> {
        if direct_paragraph == BlockProperties::default()
            && direct_default_character == CharacterProperties::default()
        {
            None
        } else {
            Some(Arc::new(Self { direct_paragraph, direct_default_character }))
        }
    }
}

impl std::ops::Deref for Block {
    type Target = BlockAttributes;

    fn deref(&self) -> &Self::Target { &self.attributes }
}

impl std::ops::DerefMut for Block {
    fn deref_mut(&mut self) -> &mut Self::Target { Arc::make_mut(&mut self.attributes) }
}

impl std::ops::Deref for BlockAttributes {
    type Target = BlockDirectFormatting;

    fn deref(&self) -> &Self::Target {
        static EMPTY: OnceLock<BlockDirectFormatting> = OnceLock::new();
        self.direct_formatting.as_deref().unwrap_or_else(|| EMPTY.get_or_init(Default::default))
    }
}

impl std::ops::DerefMut for BlockAttributes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(self.direct_formatting.get_or_insert_with(|| Arc::new(BlockDirectFormatting::default())))
    }
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.range == other.range && self.attributes == other.attributes
    }
}

impl PartialEq for BlockAttributes {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.containers.len() == other.containers.len()
            && self.containers.iter().zip(other.containers.iter()).all(|(a, b)| {
                a.starts_here == b.starts_here && a.ends_here == b.ends_here
                    && a.container.kind == b.container.kind && a.container.style == b.container.style
                    && a.container.direct_formatting == b.container.direct_formatting
            }) && self.list_loose == other.list_loose && self.quote_depth == other.quote_depth && self.thematic_break == other.thematic_break && self.markdown_html == other.markdown_html && self.code_kind == other.code_kind && self.code_language == other.code_language && self.code_language_label == other.code_language_label && self.style == other.style && self.list_editing == other.list_editing && **self == **other
    }
}

impl RangedItem for Block {
    fn navigation_summary(&self) -> super::range_index::NavigationSummary {
        // Zero is a non-list boundary; list levels use one through 256.
        let key = self.list_navigation_level().map_or(0, |level| u16::from(level) + 1);
        let owned_paragraph = key != 0 && !matches!(self.kind, BlockKind::ListItem { .. });
        super::range_index::NavigationSummary {
            minimum: key, maximum: key,
            minimum_start: if self.list_editing.item_start { key } else { u16::MAX },
            flags: u16::from(!self.list_editing.indent && !owned_paragraph)
                | (u16::from(!self.list_editing.unindent && !owned_paragraph) << 1)
                | if self.style.0 == "Code Block" && self.code_kind.supplies_language_default() { 4 } else { 0 }
                | if matches!(self.kind, BlockKind::Heading(_)) || self.style.0.strip_prefix("Heading")
                    .and_then(|level| level.parse::<u8>().ok()).is_some_and(|level| (1..=6).contains(&level)) { 8 } else { 0 }
                // A list grammar restart may cross ordinary empty paragraphs,
                // but must stop at every other unrelated owner.
                | if key == 0 && !(self.range.is_empty() && self.kind == BlockKind::Paragraph
                    && self.containers.is_empty() && !self.thematic_break
                    && !self.markdown_html && self.style.0 != "Code Block") { 16 } else { 0 },
        }
    }

    fn visit_shared_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        // Immutable payloads alone own their strings; one entry charges both.
        let owned = self.style.0.capacity() + self.code_language.as_ref().map_or(0, String::capacity) + 16;
        visitor.arc_owning_once(&self.attributes, owned, |visitor| {
            if !self.containers.is_empty() {
                visitor.arc(&self.containers, |visitor| {
                    for member in self.containers.iter() {
                        visitor.arc_owning(&member.container, member.container.style.0.capacity() + 16, |visitor| {
                            if let Some(properties) = &member.container.direct_formatting {
                                visitor.arc_owning(properties, properties.direct_default_character.owned_heap_bytes(), |_| {});
                            }
                        });
                    }
                });
            }
            if let Some(properties) = &self.direct_formatting {
                visitor.arc_owning(properties, properties.direct_default_character.owned_heap_bytes(), |_| {});
            }
        });
    }

    fn range(&self) -> &Range<usize> { &self.range }

    fn with_range(&self, range: Range<usize>) -> Self {
        let mut shifted = self.clone();
        shifted.range = range;
        shifted
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockIdentityError {
    Exhausted,
    InvalidProjection,
}

/// Origin of one line in a source-backed hard-line transfer candidate.
/// Existing lines retain identity; copied lines receive a fresh identity but
/// inherit sparse direct declarations from the source line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransferredLineOrigin {
    Existing(usize),
    Copied(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub application: StyleApplication,
}

/// Passive image metadata. The source remains the only persistence authority;
/// these immutable records never load or retain native image resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineImage {
    pub range: Range<usize>,
    pub text: String,
    pub destination: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub(super) title: String,
    pub(super) source: Range<usize>,
    pub(super) inline: bool,
    pub(super) html: bool,
}
impl RangedItem for InlineImage {
    fn range(&self) -> &Range<usize> { &self.range }
    fn owned_heap_bytes(&self) -> usize { self.text.capacity() + self.destination.capacity() + self.title.capacity() }
    fn with_range(&self, range: Range<usize>) -> Self { Self { range, ..self.clone() } }
    fn with_transform(&self, range: Range<usize>, auxiliary_shift: i128, _: Option<u64>) -> Option<Self> {
        Some(Self { range, source: shift_range_i128(&self.source, auxiliary_shift)?, ..self.clone() })
    }
}

/// Markdown code blocks resolve syntax names through the Code definitions
/// merged into the layout sheet under their display names.
fn markdown_code_style_names(sheet: &StyleSheet) -> std::collections::BTreeMap<&str, &StyleId> {
    sheet.character_styles()
        .filter(|style| style.id.0.starts_with("__code/"))
        .filter_map(|style| sheet.character_style_metadata(&style.id)
            .map(|metadata| (metadata.display_name.as_str(), &style.id))).collect()
}

/// Character-style display names of a Code sheet, for run-name resolution.
fn code_style_names(sheet: &StyleSheet) -> std::collections::BTreeMap<&str, &StyleId> {
    sheet
        .character_styles()
        .filter_map(|style| {
            sheet
                .character_style_metadata(&style.id)
                .map(|metadata| (metadata.display_name.as_str(), &style.id))
        })
        .collect()
}

impl RangedItem for StyleSpan {
    fn owned_heap_bytes(&self) -> usize { self.application.owned_heap_bytes() }

    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self {
            range,
            application: self.application.clone(),
        }
    }
}

/// One authoritative formatted hard line. The range excludes its explicit
/// hard-break item; ordinary U+000A content can therefore remain inside it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HardLine {
    id: u64,
    range: Range<usize>,
    separator_length: usize,
}

impl RangedItem for HardLine {
    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self {
            id: self.id,
            range,
            separator_length: self.separator_length,
        }
    }
}

impl HardLine {
    fn separator_range(&self) -> Option<Range<usize>> {
        (self.separator_length != 0).then(|| {
            self.range.end
                ..self
                    .range
                    .end
                    .checked_add(self.separator_length)
                    .expect("formatted hard-line separator is representable")
        })
    }
}

/// One immutable hard-line record in an explicitly identified document
/// projection. All ranges are formatted UTF-8 byte ranges in `revision`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardLineInfo {
    document: DocumentId,
    revision: Revision,
    index: usize,
    id: u64,
    content_range: Range<usize>,
    separator_range: Option<Range<usize>>,
}

impl HardLineInfo {
    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Formatted content excluding the explicit hard-break item.
    pub fn content_range(&self) -> Range<usize> {
        self.content_range.clone()
    }

    /// The normalized hard-break item following this line, when present.
    pub fn separator_range(&self) -> Option<Range<usize>> {
        self.separator_range.clone()
    }

    /// Content plus its following hard-break item, when one exists.
    pub fn linewise_range(&self) -> Range<usize> {
        self.content_range.start
            ..self
                .separator_range
                .as_ref()
                .map_or(self.content_range.end, |separator| separator.end)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HardLineQueryError {
    InvalidLineRange {
        start: usize,
        end: usize,
        line_count: usize,
    },
    FormattedOffsetOutOfBounds {
        offset: usize,
        text_length: usize,
    },
    NotCharacterBoundary {
        offset: usize,
    },
}

impl fmt::Display for HardLineQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineRange {
                start,
                end,
                line_count,
            } => write!(
                formatter,
                "hard-line range {start}..{end} is invalid for {line_count} lines"
            ),
            Self::FormattedOffsetOutOfBounds {
                offset,
                text_length,
            } => write!(
                formatter,
                "formatted offset {offset} exceeds text length {text_length}"
            ),
            Self::NotCharacterBoundary { offset } => {
                write!(
                    formatter,
                    "formatted offset {offset} is not a UTF-8 boundary"
                )
            }
        }
    }
}

impl std::error::Error for HardLineQueryError {}

/// Validation failure while creating or capturing a structured formatted-text
/// payload. A marked break offset always names the single U+000A byte that
/// represents one semantic hard-break item in the payload text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormattedPayloadError {
    InvalidRange {
        start: usize,
        end: usize,
        text_length: usize,
    },
    NotCharacterBoundary {
        offset: usize,
    },
    NotGraphemeBoundary {
        offset: usize,
    },
    BoundaryResolutionFailed {
        offset: usize,
    },
    BreakOffsetOutOfBounds {
        offset: usize,
        text_length: usize,
    },
    BreakOffsetIsNotLineFeed {
        offset: usize,
    },
    BreakOffsetsNotStrictlyIncreasing {
        previous: usize,
        offset: usize,
    },
}

impl fmt::Display for FormattedPayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange {
                start,
                end,
                text_length,
            } => write!(
                formatter,
                "formatted payload range {start}..{end} is invalid for text length {text_length}"
            ),
            Self::NotCharacterBoundary { offset } => {
                write!(
                    formatter,
                    "payload boundary {offset} is not a UTF-8 boundary"
                )
            }
            Self::NotGraphemeBoundary { offset } => {
                write!(formatter, "payload boundary {offset} splits a grapheme")
            }
            Self::BoundaryResolutionFailed { offset } => write!(
                formatter,
                "payload boundary {offset} could not be resolved in formatted text storage"
            ),
            Self::BreakOffsetOutOfBounds {
                offset,
                text_length,
            } => write!(
                formatter,
                "payload break offset {offset} is outside text length {text_length}"
            ),
            Self::BreakOffsetIsNotLineFeed { offset } => write!(
                formatter,
                "payload break offset {offset} does not name U+000A"
            ),
            Self::BreakOffsetsNotStrictlyIncreasing { previous, offset } => write!(
                formatter,
                "payload break offsets are not strictly increasing at {previous}, {offset}"
            ),
        }
    }
}

impl std::error::Error for FormattedPayloadError {}

/// Revision-bound formatted UTF-8 plus explicit semantic hard-break markers.
///
/// Unmarked U+000A values are ordinary text. Markers are sorted unique byte
/// offsets into `text`, and each points at one U+000A scalar. Source spelling
/// is deliberately absent: insertion delegates marked breaks to the target
/// document's line-ending component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedTextPayload {
    document: DocumentId,
    revision: Revision,
    text: Arc<str>,
    break_offsets: Arc<[usize]>,
}

impl FormattedTextPayload {
    /// Creates payload content bound to an exact hard-line snapshot.
    pub fn new(
        snapshot: &HardLineSnapshot,
        text: impl Into<String>,
        break_offsets: Vec<usize>,
    ) -> Result<Self, FormattedPayloadError> {
        let text: Arc<str> = text.into().into();
        validate_payload_break_offsets(text.as_ref(), &break_offsets)?;
        Ok(Self {
            document: snapshot.document,
            revision: snapshot.revision,
            text,
            break_offsets: break_offsets.into(),
        })
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn break_offsets(&self) -> &[usize] {
        &self.break_offsets
    }
}

fn validate_payload_break_offsets(
    text: &str,
    break_offsets: &[usize],
) -> Result<(), FormattedPayloadError> {
    let mut previous = None;
    for &offset in break_offsets {
        if offset >= text.len() {
            return Err(FormattedPayloadError::BreakOffsetOutOfBounds {
                offset,
                text_length: text.len(),
            });
        }
        if let Some(previous) = previous {
            if previous >= offset {
                return Err(FormattedPayloadError::BreakOffsetsNotStrictlyIncreasing {
                    previous,
                    offset,
                });
            }
        }
        if text.as_bytes()[offset] != b'\n' {
            return Err(FormattedPayloadError::BreakOffsetIsNotLineFeed { offset });
        }
        previous = Some(offset);
    }
    Ok(())
}

fn validate_payload_capture_range(
    snapshot: &HardLineSnapshot,
    range: &Range<usize>,
) -> Result<(), FormattedPayloadError> {
    if range.start > range.end || range.end > snapshot.text_length() {
        return Err(FormattedPayloadError::InvalidRange {
            start: range.start,
            end: range.end,
            text_length: snapshot.text_length(),
        });
    }
    for offset in [range.start, range.end] {
        let is_char_boundary = snapshot
            .text_tree
            .is_char_boundary(offset)
            .map_err(|_| FormattedPayloadError::BoundaryResolutionFailed { offset })?;
        if !is_char_boundary {
            return Err(FormattedPayloadError::NotCharacterBoundary { offset });
        }
        if !snapshot.is_grapheme_boundary(offset) {
            return Err(FormattedPayloadError::NotGraphemeBoundary { offset });
        }
    }
    Ok(())
}

/// Cheap owned read snapshot of the authoritative formatted hard-line index.
///
/// Cloning this value shares the persistent range tree and text allocation. A
/// snapshot remains queryable after the live document advances; callers can
/// compare its document/revision identity or ask the document to validate it
/// before applying a result.
#[derive(Clone)]
pub struct HardLineSnapshot {
    document: DocumentId,
    revision: Revision,
    flat_text: Arc<OnceLock<Arc<str>>>,
    text_tree: FormattedTextTree,
    hard_lines: OrderedRangeStore<HardLine>,
    inline_images: IntervalRangeStore<InlineImage>,
    pub(super) comment_context_cache: super::comment_context::SharedCommentContextCache,
}

impl fmt::Debug for HardLineSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HardLineSnapshot")
            .field("document", &self.document)
            .field("revision", &self.revision)
            .field("text_length", &self.text_tree.byte_len())
            .field("line_count", &self.hard_lines.len())
            .finish()
    }
}

impl HardLineSnapshot {
    fn new(document: DocumentId, projection: &FormattedDocument) -> Self {
        Self {
            document,
            revision: projection.revision,
            flat_text: projection.flat_text.clone(),
            text_tree: projection.text.clone(),
            hard_lines: projection.hard_lines.clone(),
            inline_images: projection.inline_images.clone(),
            comment_context_cache: projection.comment_context_cache.clone(),
        }
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn text_length(&self) -> usize {
        self.text_tree.byte_len()
    }

    /// Borrow the remainder of one bounded tree leaf without flattening.
    pub(crate) fn byte_chunk_at(&self, offset: usize) -> &[u8] {
        self.text_tree.byte_chunk_at(offset)
    }

    /// Borrow the prefix of one leaf for reverse snapshot traversal.
    pub(crate) fn byte_chunk_before(&self, offset: usize) -> &[u8] {
        self.text_tree.byte_chunk_before(offset)
    }

    /// Number of UTF-16 code units in this exact formatted snapshot.
    ///
    /// This is read from the persistent text tree's root aggregate and does
    /// not materialize the compatibility flat string.
    pub fn utf16_length(&self) -> usize {
        self.text_tree.utf16_len()
    }

    /// Copy one scalar-aligned UTF-8 range from this exact snapshot.
    ///
    /// Only tree leaves intersecting `range` are visited. The returned string
    /// is independent of the snapshot and the compatibility flat string stays
    /// unmaterialized.
    pub fn slice_utf8(&self, range: Range<usize>) -> Result<String, FormattedTextError> {
        self.text_tree.slice(range)
    }

    /// Convert an exact UTF-8 scalar boundary to a UTF-16 code-unit boundary.
    /// Prefix work is logarithmic apart from decoding the one bounded leaf
    /// containing the requested boundary.
    pub fn utf16_offset_for_utf8(&self, offset: usize) -> Result<usize, FormattedTextError> {
        self.text_tree.utf16_offset_for_byte(offset)
    }

    /// Convert an exact UTF-16 code-unit boundary to a UTF-8 scalar boundary.
    /// A boundary splitting a surrogate pair is rejected rather than rounded.
    pub fn utf8_offset_for_utf16(&self, offset: usize) -> Result<usize, FormattedTextError> {
        self.text_tree.byte_offset_for_utf16(offset)
    }

    /// Exact formatted UTF-8 backing for the ranges returned by this snapshot.
    pub fn text(&self) -> &str {
        self.flat_text
            .get_or_init(|| Arc::from(self.text_tree.flatten()))
            .as_ref()
    }

    pub fn line_count(&self) -> usize {
        self.hard_lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hard_lines.is_empty()
    }

    /// Whether `offset` is a legal logical formatted-content boundary.
    ///
    /// Unicode segmentation is applied within text items, while both sides of
    /// every marked semantic hard break are forced boundaries. Thus a literal
    /// CR immediately followed by a marked U+000A remains a distinct content
    /// grapheme followed by a distinct hard-break item. An unmarked literal
    /// CRLF inside line content retains normal UAX #29 behavior.
    pub fn is_grapheme_boundary(&self, offset: usize) -> bool {
        logical_is_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset).unwrap_or(false)
    }

    /// Return the next logical grapheme/item boundary after `offset`.
    pub fn next_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        logical_next_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset)
            .ok()
            .flatten()
    }

    /// Return the preceding logical grapheme/item boundary before `offset`.
    pub fn previous_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        logical_previous_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset)
            .ok()
            .flatten()
    }

    /// Return the logical item beginning at a legal non-EOF boundary.
    pub fn grapheme_range_at(&self, offset: usize) -> Option<Range<usize>> {
        if offset >= self.text_length() || !self.is_grapheme_boundary(offset) {
            return None;
        }
        self.next_grapheme_boundary(offset).map(|end| offset..end)
    }

    /// Materialize the logical grapheme/item ranges in one aligned range.
    pub fn grapheme_ranges(&self, range: Range<usize>) -> Option<Vec<Range<usize>>> {
        if range.start > range.end
            || range.end > self.text_length()
            || !self.is_grapheme_boundary(range.start)
            || !self.is_grapheme_boundary(range.end)
        {
            return None;
        }
        let mut ranges = Vec::new();
        let mut start = range.start;
        while start < range.end {
            let end = self.next_grapheme_boundary(start)?;
            if end > range.end {
                return None;
            }
            ranges.push(start..end);
            start = end;
        }
        Some(ranges)
    }

    /// Count logical grapheme/item ranges without flattening the snapshot.
    pub fn grapheme_count(&self, range: Range<usize>) -> Option<usize> {
        if range.start > range.end
            || range.end > self.text_length()
            || !self.is_grapheme_boundary(range.start)
            || !self.is_grapheme_boundary(range.end)
        {
            return None;
        }
        let mut count = 0usize;
        let mut offset = range.start;
        while offset < range.end {
            offset = self.next_grapheme_boundary(offset)?;
            if offset > range.end {
                return None;
            }
            count = count.checked_add(1)?;
        }
        Some(count)
    }

    /// Capture a display location for explicit recovery in a replacement
    /// document. Unlike a snapshot point, these zero-based line/grapheme
    /// coordinates are only a presentation hint, never an editing target.
    pub fn recovery_location(&self, offset: usize) -> Option<(usize, usize)> {
        let line = self.line_at_offset(offset).ok()?;
        let column = self.grapheme_count(line.content_range.start..offset)?;
        Some((line.index, column))
    }

    /// Recover the nearest valid boundary on the requested line, or the final
    /// line if it disappeared. Work touches only the local line and never
    /// materializes a flat copy of the document.
    pub fn recover_location(&self, line: usize, column: usize) -> Option<usize> {
        let line = self.line(line.min(self.line_count().checked_sub(1)?))?;
        let mut offset = line.content_range.start;
        for _ in 0..column {
            if offset == line.content_range.end { break; }
            offset = self.next_grapheme_boundary(offset)?;
        }
        Some(offset)
    }

    /// Advance exactly `count` logical items, returning `None` when that would
    /// pass EOF or when `offset` is not itself a legal boundary.
    pub fn advance_graphemes(&self, offset: usize, count: usize) -> Option<usize> {
        if !self.is_grapheme_boundary(offset) {
            return None;
        }
        let mut result = offset;
        for _ in 0..count {
            result = self.next_grapheme_boundary(result)?;
        }
        Some(result)
    }

    /// Captures any valid grapheme-aligned formatted range, retaining exactly
    /// which included U+000A items are semantic hard breaks. Literal U+000A
    /// content remains unmarked.
    pub fn capture(
        &self,
        range: Range<usize>,
    ) -> Result<FormattedTextPayload, FormattedPayloadError> {
        validate_payload_capture_range(self, &range)?;
        let break_offsets = self
            .hard_lines
            .query_touching(&range)
            .into_iter()
            .filter_map(|line| line.separator_range())
            .filter(|separator| range.start <= separator.start && separator.end <= range.end)
            .map(|separator| {
                separator
                    .start
                    .checked_sub(range.start)
                    .expect("a captured separator follows the payload start")
            })
            .collect::<Vec<_>>();
        let text = self.text_tree.slice(range).map_err(|_| {
            FormattedPayloadError::BoundaryResolutionFailed {
                offset: self.text_tree.byte_len(),
            }
        })?;
        FormattedTextPayload::new(self, text, break_offsets)
    }

    /// Looks up one zero-based line in `O(log n)`.
    pub fn line(&self, index: usize) -> Option<HardLineInfo> {
        self.hard_lines
            .get(index)
            .map(|line| self.info(index, line))
    }

    /// Looks up a half-open ordinal line range in `O(log n + k)`.
    pub fn lines(&self, indices: Range<usize>) -> Result<Vec<HardLineInfo>, HardLineQueryError> {
        let lines =
            self.hard_lines
                .get_range(&indices)
                .ok_or(HardLineQueryError::InvalidLineRange {
                    start: indices.start,
                    end: indices.end,
                    line_count: self.line_count(),
                })?;
        Ok(lines
            .into_iter()
            .enumerate()
            .map(|(relative, line)| {
                self.info(
                    indices
                        .start
                        .checked_add(relative)
                        .expect("validated hard-line indices are representable"),
                    line,
                )
            })
            .collect())
    }

    /// Resolves a valid formatted UTF-8 boundary in `O(log n)` and returns
    /// both its line index and all ranges needed by command/layout consumers.
    /// A separator's leading boundary belongs to the preceding line, its
    /// trailing boundary belongs to the following line, and EOF belongs to the
    /// final line (including a trailing empty line).
    pub fn line_at_offset(&self, offset: usize) -> Result<HardLineInfo, HardLineQueryError> {
        if offset > self.text_tree.byte_len() {
            return Err(HardLineQueryError::FormattedOffsetOutOfBounds {
                offset,
                text_length: self.text_tree.byte_len(),
            });
        }
        if !self
            .text_tree
            .is_char_boundary(offset)
            .map_err(|_| HardLineQueryError::NotCharacterBoundary { offset })?
        {
            return Err(HardLineQueryError::NotCharacterBoundary { offset });
        }
        let index = self
            .hard_lines
            .index_touching_point(offset)
            .expect("every formatted projection contains a hard line");
        let line = self
            .hard_lines
            .get(index)
            .expect("a hard-line index resolves to an existing record");
        Ok(self.info(index, line))
    }

    /// Returns the formatted extent for a half-open ordinal line span. Every
    /// selected line contributes its following stored separator when present.
    /// An empty span resolves to an empty boundary at that line's start, or at
    /// EOF for `line_count..line_count`.
    pub fn linewise_extent(
        &self,
        indices: Range<usize>,
    ) -> Result<Range<usize>, HardLineQueryError> {
        if indices.start > indices.end || indices.end > self.line_count() {
            return Err(HardLineQueryError::InvalidLineRange {
                start: indices.start,
                end: indices.end,
                line_count: self.line_count(),
            });
        }
        if indices.is_empty() {
            let boundary = if indices.start == self.line_count() {
                self.text_tree.byte_len()
            } else {
                self.line(indices.start)
                    .expect("a validated line index exists")
                    .content_range
                    .start
            };
            return Ok(boundary..boundary);
        }
        let first = self
            .line(indices.start)
            .expect("a validated first line exists");
        let last = self
            .line(indices.end - 1)
            .expect("a validated final line exists");
        Ok(first.content_range.start..last.linewise_range().end)
    }

    fn info(&self, index: usize, line: HardLine) -> HardLineInfo {
        HardLineInfo {
            document: self.document,
            revision: self.revision,
            index,
            id: line.id,
            content_range: line.range.clone(),
            separator_range: line.separator_range(),
        }
    }

    #[cfg(test)]
    fn lines_with_stats(
        &self,
        indices: Range<usize>,
    ) -> Result<(Vec<HardLineInfo>, super::range_index::QueryStats), HardLineQueryError> {
        let (lines, stats) = self.hard_lines.get_range_with_stats(&indices).ok_or(
            HardLineQueryError::InvalidLineRange {
                start: indices.start,
                end: indices.end,
                line_count: self.line_count(),
            },
        )?;
        Ok((
            lines
                .into_iter()
                .enumerate()
                .map(|(relative, line)| {
                    self.info(
                        indices
                            .start
                            .checked_add(relative)
                            .expect("validated hard-line indices are representable"),
                        line,
                    )
                })
                .collect(),
            stats,
        ))
    }
}

impl LogicalGraphemeSnapshot for HardLineSnapshot {
    fn text_len(&self) -> usize {
        self.text_length()
    }

    fn is_logical_grapheme_boundary(&self, offset: usize) -> Result<bool, FormattedTextError> {
        logical_is_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset)
    }

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_next_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset)
    }

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_previous_grapheme_boundary(&self.text_tree, &self.hard_lines, &self.inline_images, offset)
    }
}

fn nearby_hard_line_indices(
    hard_lines: &OrderedRangeStore<HardLine>,
    offset: usize,
) -> Option<Range<usize>> {
    let center = hard_lines.index_touching_point(offset)?;
    let start = center.saturating_sub(1);
    let end = center.saturating_add(2).min(hard_lines.len());
    Some(start..end)
}

fn is_semantic_item_boundary(hard_lines: &OrderedRangeStore<HardLine>, offset: usize) -> bool {
    let Some(indices) = nearby_hard_line_indices(hard_lines, offset) else {
        return false;
    };
    indices
        .filter_map(|index| hard_lines.get(index))
        .any(|line| {
            line.separator_range()
                .is_some_and(|separator| offset == separator.start || offset == separator.end)
        })
}

fn next_semantic_item_boundary(
    hard_lines: &OrderedRangeStore<HardLine>,
    offset: usize,
) -> Option<usize> {
    let indices = nearby_hard_line_indices(hard_lines, offset)?;
    indices
        .filter_map(|index| hard_lines.get(index))
        .filter_map(|line| line.separator_range())
        .flat_map(|separator| [separator.start, separator.end])
        .filter(|boundary| *boundary > offset)
        .min()
}

fn previous_semantic_item_boundary(
    hard_lines: &OrderedRangeStore<HardLine>,
    offset: usize,
) -> Option<usize> {
    let indices = nearby_hard_line_indices(hard_lines, offset)?;
    indices
        .filter_map(|index| hard_lines.get(index))
        .filter_map(|line| line.separator_range())
        .flat_map(|separator| [separator.start, separator.end])
        .filter(|boundary| *boundary < offset)
        .max()
}

fn logical_is_grapheme_boundary(
    text: &FormattedTextTree,
    hard_lines: &OrderedRangeStore<HardLine>,
    inline_images: &IntervalRangeStore<InlineImage>,
    offset: usize,
) -> Result<bool, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Ok(false);
    }
    // Only semantic image objects split adjacent combining/prepend text;
    // an authored U+FFFC in a literal document retains Unicode segmentation.
    if inline_images.query_touching(&(offset..offset)).iter().any(|image|
        image.range.start==offset || image.range.end==offset) { return Ok(true); }
    if is_semantic_item_boundary(hard_lines, offset) {
        return Ok(true);
    }
    text.is_grapheme_boundary(offset)
}

fn logical_next_grapheme_boundary(
    text: &FormattedTextTree,
    hard_lines: &OrderedRangeStore<HardLine>,
    inline_images: &IntervalRangeStore<InlineImage>,
    offset: usize,
) -> Result<Option<usize>, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Err(FormattedTextError::NotCharBoundary(offset));
    }
    let mut unicode = text.next_grapheme_boundary(offset)?;
    if let Some(end)=unicode {
        for image in inline_images.query_touching(&(offset..end)).iter() {
            for boundary in [image.range.start,image.range.end] { if offset<boundary && boundary<=end {unicode=Some(unicode.unwrap().min(boundary));} }
        }
    }
    let semantic = next_semantic_item_boundary(hard_lines, offset);
    Ok(match (unicode, semantic) {
        (Some(unicode), Some(semantic)) => Some(unicode.min(semantic)),
        (Some(unicode), None) => Some(unicode),
        (None, Some(semantic)) => Some(semantic),
        (None, None) => None,
    })
}

fn logical_previous_grapheme_boundary(
    text: &FormattedTextTree,
    hard_lines: &OrderedRangeStore<HardLine>,
    inline_images: &IntervalRangeStore<InlineImage>,
    offset: usize,
) -> Result<Option<usize>, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Err(FormattedTextError::NotCharBoundary(offset));
    }
    let mut unicode = text.previous_grapheme_boundary(offset)?;
    if let Some(start)=unicode {
        for image in inline_images.query_touching(&(start..offset)).iter() {
            for boundary in [image.range.start,image.range.end] { if start<=boundary && boundary<offset {unicode=Some(unicode.unwrap().max(boundary));} }
        }
    }
    let semantic = previous_semantic_item_boundary(hard_lines, offset);
    Ok(match (unicode, semantic) {
        (Some(unicode), Some(semantic)) => Some(unicode.max(semantic)),
        (Some(unicode), None) => Some(unicode),
        (None, Some(semantic)) => Some(semantic),
        (None, None) => None,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceSpan {
    pub formatted: Range<usize>,
    pub source: Range<usize>,
}

impl ProvenanceSpan {
    /// Generated content retains its recoverable source boundary but has no
    /// source-byte interior. Reverse edits must use its structural intention.
    pub fn is_synthetic(&self) -> bool {
        !self.formatted.is_empty() && self.source.is_empty()
    }
}

/// One monotonic source-backed run of visible formatted content. Hidden
/// format syntax between adjacent runs is intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VisibleSourceRun {
    pub(crate) formatted: Range<usize>,
    pub(crate) source: Range<usize>,
}

impl RangedItem for ProvenanceSpan {
    fn range(&self) -> &Range<usize> {
        &self.formatted
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self {
            formatted: range,
            source: self.source.clone(),
        }
    }

    fn with_transform(
        &self,
        range: Range<usize>,
        auxiliary_shift: i128,
        _revision: Option<u64>,
    ) -> Option<Self> {
        Some(Self {
            formatted: range,
            source: shift_range_i128(&self.source, auxiliary_shift)?,
        })
    }
}

/// Reverse presentation lookup, independently ordered by physical source
/// boundary so recovered/reordered HTML does not require a monotonic view.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceTextBoundary {
    source: Range<usize>,
    formatted: usize,
    side: Side,
}
impl RangedItem for SourceTextBoundary {
    fn range(&self) -> &Range<usize> {
        &self.source
    }
    fn with_range(&self, source: Range<usize>) -> Self {
        Self {
            source,
            ..self.clone()
        }
    }
    fn with_transform(
        &self,
        source: Range<usize>,
        auxiliary_shift: i128,
        _revision: Option<u64>,
    ) -> Option<Self> {
        Some(Self {
            source,
            formatted: shift_range_i128(&(self.formatted..self.formatted), auxiliary_shift)?.start,
            side: self.side,
        })
    }
}
/// The per-contributor boundary list that `ProvenanceIndex` reproduces.
#[cfg(test)]
fn source_text_boundaries(
    spans: impl IntoIterator<Item = ProvenanceSpan>,
) -> Vec<SourceTextBoundary> {
    let mut boundaries = spans
        .into_iter()
        .flat_map(|span| {
            [
                SourceTextBoundary {
                    source: span.source.start..span.source.start,
                    formatted: span.formatted.start,
                    side: Side::Downstream,
                },
                SourceTextBoundary {
                    source: span.source.end..span.source.end,
                    formatted: span.formatted.end,
                    side: Side::Upstream,
                },
            ]
        })
        .collect::<Vec<_>>();
    normalize_source_boundaries(&mut boundaries);
    boundaries
}
fn normalize_source_boundaries(boundaries: &mut Vec<SourceTextBoundary>) {
    boundaries.sort_by_key(|boundary| {
        (
            boundary.source.start,
            boundary.formatted,
            matches!(boundary.side, Side::Downstream),
        )
    });
    boundaries.dedup();
}

impl RangedItem for DecodingDiagnostic {
    fn range(&self) -> &Range<usize> {
        &self.formatted_range
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        let mut shifted = self.clone();
        shifted.formatted_range = range;
        shifted
    }

    fn with_transform(
        &self,
        range: Range<usize>,
        auxiliary_shift: i128,
        revision: Option<u64>,
    ) -> Option<Self> {
        let mut shifted = self.clone();
        shifted.formatted_range = range;
        shifted.source_range = shift_range_i128(&shifted.source_range, auxiliary_shift)?;
        if let Some(revision) = revision {
            shifted.revision = Revision(revision);
        }
        Some(shifted)
    }
}

fn shift_range_i128(range: &Range<usize>, delta: i128) -> Option<Range<usize>> {
    fn shift(value: usize, delta: i128) -> Option<usize> {
        let shifted = i128::try_from(value).ok()?.checked_add(delta)?;
        usize::try_from(shifted).ok()
    }
    Some(shift(range.start, delta)?..shift(range.end, delta)?)
}

/// How an exact source boundary relates to its formatted projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceBoundaryRelation {
    Exact,
    DocumentStart,
    DocumentEnd,
    EmptyDocument,
}

/// A revision-bound mapping from one source byte boundary into formatted text.
///
/// The ordinal fields remain explicitly named by domain. They are temporary
/// values in the identified immutable revision, not persistent anchors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectedSourceBoundary {
    pub revision: Revision,
    pub source_offset: usize,
    pub formatted_offset: usize,
    pub affinity: BoundaryAffinity,
    pub relation: SourceBoundaryRelation,
}

/// Why an exact source byte boundary cannot be represented by one legal
/// formatted text point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceToTextError {
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    WrongSourcePart {
        expected: SourcePartId,
        actual: SourcePartId,
    },
    WrongSnapshot {
        expected: Revision,
        actual: Revision,
    },
    SourceOffsetOutOfBounds {
        offset: usize,
        length: usize,
    },
    InteriorBom {
        source_range: Range<usize>,
    },
    InteriorOpaqueUnit {
        source_range: Range<usize>,
        formatted_range: Range<usize>,
    },
    InteriorMappedUnit {
        source_range: Range<usize>,
        formatted_range: Range<usize>,
    },
    InteriorHiddenSyntax {
        source_range: Range<usize>,
        upstream_formatted: Option<usize>,
        downstream_formatted: Option<usize>,
    },
    AmbiguousBoundary {
        source_offset: usize,
        candidates: Vec<usize>,
    },
    UnmappedBoundary {
        source_offset: usize,
    },
    NotFormattedGraphemeBoundary {
        source_offset: usize,
        formatted_offset: usize,
    },
    FormattedText(FormattedTextError),
}

impl fmt::Display for SourceToTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongDocument { expected, actual } => write!(
                formatter,
                "source point belongs to document {}, expected {}",
                actual.0, expected.0
            ),
            Self::WrongSourcePart { expected, actual } => write!(
                formatter,
                "source point belongs to part {}, expected {}",
                actual.0, expected.0
            ),
            Self::WrongSnapshot { expected, actual } => write!(
                formatter,
                "source point belongs to revision {}; projection revision is {}",
                actual.0, expected.0
            ),
            Self::SourceOffsetOutOfBounds { offset, length } => write!(
                formatter,
                "source byte boundary {offset} exceeds source length {length}"
            ),
            Self::InteriorBom { source_range } => write!(
                formatter,
                "source byte boundary is inside BOM bytes {}..{}",
                source_range.start, source_range.end
            ),
            Self::InteriorOpaqueUnit {
                source_range,
                formatted_range,
            } => write!(
                formatter,
                "source byte boundary is inside opaque bytes {}..{} projected at {}..{}",
                source_range.start,
                source_range.end,
                formatted_range.start,
                formatted_range.end
            ),
            Self::InteriorMappedUnit {
                source_range,
                formatted_range,
            } => write!(
                formatter,
                "source byte boundary is inside indivisible mapped unit {}..{} projected at {}..{}",
                source_range.start,
                source_range.end,
                formatted_range.start,
                formatted_range.end
            ),
            Self::InteriorHiddenSyntax {
                source_range,
                ..
            } => write!(
                formatter,
                "source byte boundary is inside hidden syntax {}..{}",
                source_range.start, source_range.end
            ),
            Self::AmbiguousBoundary {
                source_offset,
                candidates,
            } => write!(
                formatter,
                "source byte boundary {source_offset} maps to multiple formatted boundaries {candidates:?}"
            ),
            Self::UnmappedBoundary { source_offset } => write!(
                formatter,
                "source byte boundary {source_offset} has no formatted provenance"
            ),
            Self::NotFormattedGraphemeBoundary {
                source_offset,
                formatted_offset,
            } => write!(
                formatter,
                "source byte boundary {source_offset} maps inside a formatted grapheme at {formatted_offset}"
            ),
            Self::FormattedText(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SourceToTextError {}

impl From<FormattedTextError> for SourceToTextError {
    fn from(value: FormattedTextError) -> Self {
        Self::FormattedText(value)
    }
}

/// Disposable automatic styles are separate from semantic spans and configuration.
#[derive(Clone, Debug)]
struct MarkdownCodePresentation {
    sheet: Arc<StyleSheet>,
    spans: IntervalRangeStore<StyleSpan>,
}

/// Immutable formatted projection consumed by commands and layout.
#[derive(Clone, Debug)]
pub struct FormattedDocument {
    revision: Revision,
    tables: super::TableRows<super::MarkdownTable>,
    table_lines: Option<OrderedRangeStore<HardLine>>,
    /// Canonical persistent formatted-text representation.
    text: FormattedTextTree,
    /// Lazily materialized compatibility view. Regional candidates can remain
    /// wholly tree-backed until a legacy caller explicitly requests `text()`.
    flat_text: Arc<OnceLock<Arc<str>>>,
    comment_context_cache: super::comment_context::SharedCommentContextCache,
    blocks: OrderedRangeStore<Block>,
    /// Authoritative hard-line structure. This is intentionally distinct from
    /// both the block tree and the text tree's lexical U+000A aggregates. The
    /// paragraphs may contain several explicit breaks, and flow-normalized
    /// paragraphs may consume several physical source lines. This index is
    /// populated independently from paragraph identity. A pipeline may also retain U+000A as ordinary
    /// content (for example forced legacy-Mac input).
    hard_lines: OrderedRangeStore<HardLine>,
    styles: IntervalRangeStore<StyleSpan>,
    inline_images: IntervalRangeStore<InlineImage>,
    /// Contributors and their source-keyed boundary index. Rich projections
    /// store one-to-one scalar text as runs that queries expand exactly.
    provenance: ProvenanceIndex,
    /// Literal runs are divisible encoding mappings; rich contributors retain
    /// their existing indivisible/relational meaning.
    literal_encoding: Option<super::Encoding>,
    source_ordered: bool,
    decoding_diagnostics: IntervalRangeStore<DecodingDiagnostic>,
    style_sheet: Arc<StyleSheet>,
    markdown_code_presentation: Option<Arc<MarkdownCodePresentation>>,
    document_style: DocumentStyleAssignment,
    source_content_start: usize,
    source_content_end: usize,
    source_insertion_end: usize,
}

impl PartialEq for FormattedDocument {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.text() == other.text()
            && self.tables == other.tables
            && self.blocks == other.blocks
            && self.hard_lines == other.hard_lines
            && self.inline_images == other.inline_images
            && self.styles == other.styles
            && self.provenance == other.provenance
            && self.literal_encoding == other.literal_encoding
            && self.decoding_diagnostics == other.decoding_diagnostics
            && self.style_sheet == other.style_sheet
            && self.document_style == other.document_style
            && self.source_content_start == other.source_content_start
            && self.source_content_end == other.source_content_end
            && self.source_insertion_end == other.source_insertion_end
    }
}

impl LogicalGraphemeSnapshot for FormattedDocument {
    fn text_len(&self) -> usize {
        self.text.byte_len()
    }

    fn is_logical_grapheme_boundary(&self, offset: usize) -> Result<bool, FormattedTextError> {
        logical_is_grapheme_boundary(&self.text, &self.hard_lines, &self.inline_images, offset)
    }

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_next_grapheme_boundary(&self.text, &self.hard_lines, &self.inline_images, offset)
    }

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_previous_grapheme_boundary(&self.text, &self.hard_lines, &self.inline_images, offset)
    }
}

impl FormattedDocument {
    pub(super) fn install_inline_images(&mut self, images: Vec<InlineImage>) {
        self.inline_images = IntervalRangeStore::new(images);
    }
    /// Indexed region query; does not scan or materialize unrelated images.
    pub fn inline_images_for_region(&self, range: &Range<usize>) -> Vec<InlineImage> {
        self.inline_images.query_overlapping(range)
    }
    /// Image ranges are disjoint, so existence is a logarithmic index query.
    pub fn has_inline_images_in_region(&self, range: &Range<usize>) -> bool {
        !self.inline_images.overlapping_index_span(range).is_empty()
    }

    /// Image is a content-derived paragraph appearance. A prose paragraph
    /// containing any ordinary text keeps its source-selected assignment;
    /// headings, tables and list/quote treatment remain independent.
    /// Blocks outside containers often have equal attributes (headings, code
    /// blocks, table cells). Share one allocation per distinct value, as plain
    /// paragraphs already do; a later change to one block copies on write.
    fn share_block_attributes(&mut self) {
        fn share(blocks: &OrderedRangeStore<Block>) -> Option<OrderedRangeStore<Block>> {
            const DISTINCT: usize = 64;
            let mut shared: Vec<Arc<BlockAttributes>> = Vec::new();
            let mut changed = false;
            // `to_vec` reads the tree once; slice access would retain a flat copy.
            let blocks = blocks.to_vec().into_iter().map(|mut block| {
                if block.containers.is_empty() {
                    if let Some(existing) = shared.iter().find(|candidate| ***candidate == *block.attributes) {
                        if !Arc::ptr_eq(existing, &block.attributes) {
                            block.attributes = existing.clone();
                            changed = true;
                        }
                    } else if shared.len() < DISTINCT {
                        shared.push(block.attributes.clone());
                    }
                }
                block
            }).collect::<Vec<_>>();
            changed.then(|| OrderedRangeStore::new(blocks))
        }
        if let Some(blocks) = share(&self.blocks) {
            self.blocks = blocks;
        }
    }

    fn assign_image_paragraph_styles(&mut self) {
        if self.inline_images.is_empty() { return; }
        let assign = |blocks: &OrderedRangeStore<Block>| {
            blocks.to_vec().into_iter().map(|mut block| {
                if block.kind != BlockKind::Paragraph || block.style.0 != "Paragraph" {
                    return block;
                }
                let images = self.inline_images.query_overlapping(&block.range);
                if images.is_empty() { return block; }
                let mut body = String::new();
                let mut at = block.range.start;
                for image in images {
                    if image.range.start < at || image.range.end > block.range.end {
                        return block;
                    }
                    let Ok(gap) = self.text.slice(at..image.range.start) else { return block; };
                    if gap.contains('\u{fffc}') { return block; }
                    body.push_str(&gap);
                    body.push('\u{fffc}');
                    at = image.range.end;
                }
                let Ok(tail) = self.text.slice(at..block.range.end) else { return block; };
                if tail.contains('\u{fffc}') { return block; }
                body.push_str(&tail);
                if body.chars().all(|ch| ch == '\u{fffc}' || ch.is_whitespace()) {
                    block.style = "Image".into();
                }
                block
            }).collect()
        };
        self.blocks = OrderedRangeStore::new(assign(&self.blocks));
    }
    pub fn tables(&self) -> &super::TableRows<super::MarkdownTable> { &self.tables }
    pub(super) fn extract_table_row(&self,index:usize,source_delta:i128,source:Range<usize>)
        -> Option<(Self,super::MarkdownTableRow,super::MarkdownTableSourceRow)> {
        let table=self.tables.first()?;let row=table.rows.get(index)?;let range=row.range.clone();
        let source_row=table.source_rows.get(if index==0 {0}else{index+1})?;
        let text=self.text.slice(range.clone()).ok()?;
        let mut blocks=self.blocks.query_touching(&range);blocks.retain(|block|range.start<=block.range.start && block.range.end<=range.end);
        for block in &mut blocks {block.range=shift_range_i128(&block.range,-(range.start as i128))?;}
        let mut styles=self.styles.query_touching(&range);styles.retain(|span|range.start<=span.range.start && span.range.end<=range.end);
        for span in &mut styles {span.range=shift_range_i128(&span.range,-(range.start as i128))?;}
        let mut provenance=self.provenance.query_touching(&range,&self.text);provenance.retain(|span|range.start<=span.formatted.start && span.formatted.end<=range.end);
        for span in &mut provenance {span.formatted=shift_range_i128(&span.formatted,-(range.start as i128))?;span.source=shift_range_i128(&span.source,source_delta)?;}
        let diagnostics=self.decoding_diagnostics.query_touching(&range).into_iter()
            .filter(|item|range.start<=item.formatted_range.start && item.formatted_range.end<=range.end)
            .map(|item|item.with_transform(shift_range_i128(&item.formatted_range,-(range.start as i128))?,source_delta,None))
            .collect::<Option<Vec<_>>>()?;
        // A row must own complete blocks. Otherwise reject this local
        // candidate so the caller reprojects the ordinary edit instead.
        validate_block_partition(&text,&blocks).ok()?;
        let mut projection=Self::from_parts(self.revision,text,blocks,styles,provenance,diagnostics,self.style_sheet.as_ref().clone(),source.start,source.end);
        projection.inline_images=IntervalRangeStore::new(self.inline_images.query_overlapping(&range).into_iter()
            .map(|image| image.with_transform(shift_range_i128(&image.range,-(range.start as i128))?,source_delta,None)).collect::<Option<Vec<_>>>()?);
        let lines=self.hard_lines.query_touching(&range).into_iter().filter(|line|range.start<=line.range.start && line.range.end<=range.end)
            .map(|line|shift_range_i128(&line.range,-(range.start as i128))).collect::<Option<Vec<_>>>()?;
        projection.install_hard_line_partition(lines);
        let mut row=row.with_transform(0..range.len(),source_delta,None)?;row.source_range=source.clone();
        let mut source_row=source_row.with_transform(0..range.len(),source_delta,None)?;source_row.source_range=source;
        Some((projection,row,source_row))
    }
    pub(super) fn replace_table_row_metadata(&mut self,table_id:u64,index:usize,mut row:super::MarkdownTableRow,
        source_row:super::MarkdownTableSourceRow) -> Option<RangeSpliceStats> {
        let table_index=self.tables.index_touching_point(row.range.start)?;
        let mut table=self.tables.get(table_index).filter(|table|table.id==table_id)?.clone();
        let old=table.rows.get(index)?;row.id=old.id;
        for (new,old) in row.cells.iter_mut().zip(&old.cells) {new.id=old.id;}
        let mut stats=RangeSpliceStats::default();
        table.rows=table.rows.splice_transformed(index..index+1,vec![row],0,0,0,0,&mut stats)?;
        let source_index=if index==0 {0}else{index+1};
        table.source_rows=table.source_rows.splice_transformed(source_index..source_index+1,vec![source_row],0,0,0,0,&mut stats)?;
        self.tables=self.tables.splice_transformed(table_index..table_index+1,vec![table],0,0,0,0,&mut stats)?;
        Some(stats)
    }
    pub(super) fn extend_table_terminal_source(&mut self, end:usize) {
        if let Some(table)=self.tables.iter_mut().last() {
            table.source_range.end=end;
            if let Some(row)=table.rows.iter_mut().last() {row.source_range.end=end;}
            if let Some(row)=table.source_rows.iter_mut().last() {row.source_range.end=end;}
        }
    }
    pub fn table_at(&self, offset: usize) -> Option<&super::MarkdownTable> {
        self.tables.get(self.tables.index_touching_point(offset)?)
    }
    pub fn table_cell_at(&self, offset: usize) -> Option<(&super::MarkdownTable, &super::MarkdownTableRow, &super::MarkdownTableCell)> {
        let table = self.table_at(offset)?;
        let row = table.rows.get(table.rows.index_touching_point(offset)?)?;
        let cell = row.cells.get(row.cells.partition_point(|cell| cell.range.start <= offset).checked_sub(1)?)?;
        (offset <= cell.range.end).then_some((table, row, cell))
    }
    /// Include source-only table syntax such as pipes and the delimiter row.
    pub fn range_intersects_table(&self, range: &Range<usize>) -> bool {
        if range.is_empty() { return self.table_at(range.start).is_some(); }
        self.tables.get(self.tables.partition_point_end(range.start))
            .is_some_and(|table| table.range.start < range.end)
    }
    fn rebuild_table_presentation_lines(&mut self) {
        if self.tables.is_empty() {self.table_lines=None;return;}
        let mut presentation=Vec::new();let mut index=0;
        for ordinal in 0..self.tables.len() {
            let table = self.tables.get_owned(ordinal).expect("table ordinal");
            while index<self.hard_lines.len() && self.hard_lines.get(index).unwrap().range.start<table.range.start {presentation.push(self.hard_lines.get(index).unwrap());index+=1;}
            // This full scan needs transient records, not a borrowed cache
            // entry and lookup path for every row in the snapshot.
            for row in table.rows.get_range(&(0..table.rows.len())).expect("table row range") {
                presentation.push(HardLine{id:row.id,range:row.range,separator_length:1});
            }
            while index<self.hard_lines.len() && self.hard_lines.get(index).unwrap().range.start<=table.range.end {index+=1;}
        }
        while index<self.hard_lines.len() {presentation.push(self.hard_lines.get(index).unwrap());index+=1;}
        self.table_lines=Some(OrderedRangeStore::new(presentation));
    }
    fn install_table_ids(&mut self, next_id:u64, previous:Option<&Self>) -> Result<u64,BlockIdentityError> {
        self.install_table_ids_in(next_id, previous.map(|previous|(previous,0..previous.tables.len())), 0..self.tables.len())
    }
    fn install_table_ids_in(&mut self, mut next_id:u64, previous:Option<(&Self,Range<usize>)>, indices:Range<usize>) -> Result<u64,BlockIdentityError> {
        let replacing_all = indices == (0..self.tables.len());
        let mut tables = if replacing_all { std::mem::take(&mut self.tables).into_vec() }
            else { self.tables.get_range(&indices).ok_or(BlockIdentityError::InvalidProjection)? };
        let mut previous_cells=std::collections::HashMap::new();let mut previous_rows=std::collections::HashMap::new();
        let mut previous_table_rows=Vec::new();
        // A structural edit can split one table into several tables whose
        // retained rows all name the same former owner. Only one successor
        // may inherit that table identity.
        let mut table_ids = tables.iter().filter(|table| table.rows.is_frozen())
            .map(|table| table.id).collect::<std::collections::HashSet<_>>();
        if let Some((previous,previous_indices))=previous {
            // Source and formatted coordinates can both move during an edit.
            // The caller supplies the old region; matching new offsets against
            // old tables would lose untouched row and cell identities.
            // Keep the borrowed identity maps tied to one-off row vectors so
            // reconciliation does not populate the previous snapshot's cache.
            for table in previous.tables.get_range(&previous_indices).ok_or(BlockIdentityError::InvalidProjection)? {
                let rows=table.rows.get_range(&(0..table.rows.len())).ok_or(BlockIdentityError::InvalidProjection)?;
                previous_table_rows.push((table.id,rows));
            }
            for (table_id,rows) in &previous_table_rows {
                for row in rows {previous_rows.insert(row.id,(*table_id,row));for cell in &row.cells {previous_cells.insert(cell.id,(*table_id,row));}}
            }
        }
        for table in &mut tables {
            if table.rows.is_frozen() { continue; }
            let mut table_previous=None;
            for row in &mut table.rows {
                let old_row=row.cells.iter().find_map(|cell|self.blocks.get(self.blocks.partition_point_start(cell.range.start+1).saturating_sub(1)).and_then(|block|previous_cells.get(&block.id).copied()));
                if let Some((old_table,_))=old_row {table_previous=Some(old_table);}
                for cell in &mut row.cells {
                    cell.id=self.blocks.get(self.blocks.partition_point_start(cell.range.start+1).saturating_sub(1)).map_or(0,|block|block.id);
                }
                row.id=old_row.map_or_else(||row.cells.first().map_or(0,|cell|cell.id),|(_,row)|row.id);
            }
            table.id=table_previous.filter(|id| !table_ids.contains(id))
                .unwrap_or_else(||table.rows.first().map_or(0,|row|row.id));
            if !table_ids.insert(table.id) {
                table.id=next_id;
                next_id=next_id.checked_add(1).ok_or(BlockIdentityError::Exhausted)?;
                table_ids.insert(table.id);
            }
            table.rows.freeze(); table.source_rows.freeze();
        }
        if replacing_all {
            self.tables = tables.into();
            self.tables.freeze();
        } else {
            self.tables=self.tables.splice_transformed(indices,tables,0,0,0,0,&mut RangeSpliceStats::default())
                .ok_or(BlockIdentityError::InvalidProjection)?;
        }
        Ok(next_id)
    }

    pub(super) fn visit_retained_memory(
        &self,
        visitor: &mut super::history_memory::MemoryVisitor<'_>,
    ) {
        let trace = std::env::var_os("VIEM_HISTORY_MEMORY_BREAKDOWN").is_some();
        let mut before = visitor.retained_bytes();
        self.text.visit_retained_memory(visitor);
        if trace { eprintln!("  text {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        visitor.arc(&self.flat_text, |_| {});
        if let Some(text) = self.flat_text.get() { visitor.arc(text, |_| {}); }
        if trace { eprintln!("  flat {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.blocks.visit_retained_memory(visitor);
        if trace { eprintln!("  blocks {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.hard_lines.visit_retained_memory(visitor);
        if trace { eprintln!("  hard_lines {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        if let Some(lines) = &self.table_lines { lines.visit_retained_memory(visitor); }
        self.tables.visit_retained_memory(visitor);
        self.inline_images.visit_retained_memory(visitor);
        self.styles.visit_retained_memory(visitor);
        if trace { eprintln!("  styles {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.provenance.visit_retained_memory(visitor);
        if trace { eprintln!("  provenance {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.decoding_diagnostics.visit_retained_memory(visitor);

        visitor.arc(&self.style_sheet, |visitor| {
            visitor.owned(
                Arc::as_ptr(&self.style_sheet) as usize,
                1,
                self.style_sheet.owned_heap_bytes(),
            );
        });
        visitor.owned(
            self as *const Self as usize,
            0,
            self.document_style.style.0.capacity()
                + 16
                + self
                    .document_style
                    .direct_default_character
                    .owned_heap_bytes(),
        );
        if trace { eprintln!("  sheet/doc {}", visitor.retained_bytes() - before); }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        revision: Revision,
        flat_text: String,
        blocks: Vec<Block>,
        styles: Vec<StyleSpan>,
        provenance: Vec<ProvenanceSpan>,
        decoding_diagnostics: Vec<DecodingDiagnostic>,
        style_sheet: StyleSheet,
        source_content_start: usize,
        source_content_end: usize,
    ) -> Self {
        Self::from_parts_with(revision, flat_text, blocks, styles, provenance, decoding_diagnostics,
            style_sheet, source_content_start, source_content_end, true)
    }

    /// `divisible` stores one-to-one scalar contributors as runs. Literal
    /// projections pass false: their contributors are already encoding runs.
    #[allow(clippy::too_many_arguments)]
    fn from_parts_with(
        revision: Revision,
        flat_text: String,
        blocks: Vec<Block>,
        styles: Vec<StyleSpan>,
        provenance: Vec<ProvenanceSpan>,
        decoding_diagnostics: Vec<DecodingDiagnostic>,
        style_sheet: StyleSheet,
        source_content_start: usize,
        source_content_end: usize,
        divisible: bool,
    ) -> Self {
        let flat_text: Arc<str> = flat_text.into();
        let text = FormattedTextTree::try_from_shared(flat_text.clone())
            .expect("a materialized Rust string has representable text-tree aggregates");
        // A caret seed contributes no character or source bytes. Its point
        // can lie inside the following break's contributor (an empty list
        // item's padding, for example) without reordering visible content.
        let mut previous_source = None;
        let source_ordered = provenance.iter()
            .filter(|span| !span.formatted.is_empty() || !span.source.is_empty())
            .all(|span| {
                let ordered = previous_source.is_none_or(|before| before <= span.source.start);
                previous_source = Some(span.source.start);
                ordered
            });
        let provenance = ProvenanceIndex::new(provenance, &flat_text, divisible);
        Self::from_indexed_parts(revision, flat_text, text, blocks, styles, provenance, source_ordered,
            decoding_diagnostics, style_sheet, source_content_start, source_content_end)
    }

    /// `from_parts` for an adapter that compressed its contributors while
    /// emitting them.
    #[allow(clippy::too_many_arguments)]
    fn from_run_parts(
        revision: Revision,
        flat_text: String,
        blocks: Vec<Block>,
        styles: Vec<StyleSpan>,
        provenance: provenance_runs::RunBuilder,
        decoding_diagnostics: Vec<DecodingDiagnostic>,
        style_sheet: StyleSheet,
        source_content_start: usize,
        source_content_end: usize,
    ) -> Self {
        let (runs, source_ordered) = provenance.finish(&flat_text);
        let flat_text: Arc<str> = flat_text.into();
        let text = FormattedTextTree::try_from_shared(flat_text.clone())
            .expect("a materialized Rust string has representable text-tree aggregates");
        Self::from_indexed_parts(revision, flat_text, text, blocks, styles, ProvenanceIndex::from_runs(runs),
            source_ordered, decoding_diagnostics, style_sheet, source_content_start, source_content_end)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_indexed_parts(
        revision: Revision,
        flat_text: Arc<str>,
        text: FormattedTextTree,
        blocks: Vec<Block>,
        styles: Vec<StyleSpan>,
        provenance: ProvenanceIndex,
        source_ordered: bool,
        decoding_diagnostics: Vec<DecodingDiagnostic>,
        style_sheet: StyleSheet,
        source_content_start: usize,
        source_content_end: usize,
    ) -> Self {
        let document_style = DocumentStyleAssignment::new(style_sheet.base_paragraph.clone());
        // Seed paragraph boundary records here. Adapters with explicit
        // intra-paragraph hard breaks install their complete hard-line
        // partition before publication through `Document`. Keeping this seed
        // checked rejects malformed paragraph separators in release builds.
        let hard_lines = seed_hard_lines_from_current_blocks(&blocks, flat_text.as_ref())
            .expect("private projectors emit a valid current-adapter hard-line partition");
        let compatibility_text = Arc::new(OnceLock::new());
        compatibility_text
            .set(flat_text)
            .expect("a fresh compatibility cell is empty");
        Self {
            revision,
            text,
            flat_text: compatibility_text,
            comment_context_cache: super::comment_context::fresh_cache(),
            tables: Default::default(),
            table_lines: None,
            blocks: OrderedRangeStore::new(blocks),
            hard_lines: OrderedRangeStore::new(hard_lines),
            styles: IntervalRangeStore::new(styles),
            inline_images: IntervalRangeStore::new(Vec::new()),
            provenance,
            literal_encoding: None,
            source_ordered,
            decoding_diagnostics: IntervalRangeStore::new(decoding_diagnostics),
            style_sheet: Arc::new(style_sheet),
            markdown_code_presentation: None,
            document_style,
            source_content_start,
            source_content_end,
            source_insertion_end: source_content_end,
        }
    }

    pub fn presentation_line_count(&self) -> usize {
        self.table_lines.as_ref().unwrap_or(&self.hard_lines).len()
    }

    /// O(1) proof that semantic line boundaries and their identities are
    /// unchanged. Equal UTF-8 alone is insufficient after line-ending policy
    /// or projection changes; a false result only declines cache reuse.
    pub(crate) fn shares_presentation_lines_with(&self, other: &Self) -> bool {
        self.table_lines.as_ref().unwrap_or(&self.hard_lines)
            .shares_storage_with(other.table_lines.as_ref().unwrap_or(&other.hard_lines))
    }

    pub fn presentation_line_range(&self, index: usize) -> Option<Range<usize>> {
        self.table_lines.as_ref().unwrap_or(&self.hard_lines).get(index).map(|line| line.range)
    }

    pub fn presentation_line_at_offset(&self, offset: usize) -> Option<usize> {
        if offset > self.text.byte_len() {
            return None;
        }
        let lines = self.table_lines.as_ref().unwrap_or(&self.hard_lines);
        // The persistent store answers this in logarithmic time; slicing it
        // would flatten every line of the document on the first lookup after
        // each edit.
        lines.partition_point_start(offset + 1).checked_sub(1)
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn text(&self) -> &str {
        self.flat_text
            .get_or_init(|| Arc::from(self.text.flatten()))
            .as_ref()
    }

    /// Canonical persistent UTF-8 tree for logarithmic text lookup. Its legacy
    /// U+000A aggregates are lexical only; pipeline-defined hard lines are
    /// exposed by [`Self::hard_line_range`] and related methods. The flat
    /// [`Self::text`] view remains available while APIs migrate to
    /// snapshot-bound tree positions.
    pub fn text_tree(&self) -> &FormattedTextTree {
        &self.text
    }

    /// Capture a persistent anchor backed by stable projected identities and,
    /// where the adapter exposes one, exact source provenance.
    pub fn capture_text_anchor(
        &self,
        document: DocumentId,
        point: TextPoint,
        association: Association,
        affinity: BoundaryAffinity,
        deletion_recovery: DeletionRecovery,
    ) -> Result<TextAnchor, PositionError> {
        if point.document() != document {
            return Err(PositionError::WrongDocument {
                expected: document,
                actual: point.document(),
            });
        }
        if point.revision() != self.revision {
            return Err(PositionError::WrongSnapshot {
                expected: self.revision,
                actual: point.revision(),
            });
        }
        let offset = point.offset();
        if offset > self.text.byte_len() {
            return Err(PositionError::InvalidBoundary {
                domain: PositionDomain::FormattedText,
                offset,
                length: self.text.byte_len(),
            });
        }
        if !self.is_logical_grapheme_boundary(offset).unwrap_or(false) {
            return Err(PositionError::InvalidUnicodeBoundary { offset });
        }
        let leaf = |side| {
            self.text.locate_byte(offset, side).map(|location| {
                location.map(|location| {
                    ProjectedLeafBoundary::new(
                        location.id,
                        location.revision,
                        location.buffer_id,
                        location.buffer_byte,
                        side,
                    )
                })
            })
        };
        let preceding_leaf = leaf(LeafBoundarySide::Preceding)
            .map_err(|_| PositionError::InvalidUnicodeBoundary { offset })?;
        let following_leaf = leaf(LeafBoundarySide::Following)
            .map_err(|_| PositionError::InvalidUnicodeBoundary { offset })?;
        let block = self.block_anchor_at(offset, affinity);
        let (preceding_text, following_text) = self.anchor_context_fingerprints(offset);
        let provenance = self
            .source_boundary(
                offset,
                match affinity {
                    BoundaryAffinity::Upstream => Side::Upstream,
                    BoundaryAffinity::Downstream => Side::Downstream,
                },
            )
            .map(|source_offset| {
                SourceAnchorProvenance::new(
                    SourcePartId::PRIMARY,
                    self.revision,
                    source_offset,
                    preceding_text,
                    following_text,
                )
            });
        Ok(
            TextAnchor::new(point, association, affinity, deletion_recovery).with_backing(
                ProjectedAnchorBacking {
                    preceding_leaf,
                    following_leaf,
                    block,
                    provenance,
                },
            ),
        )
    }

    /// Resolve a persistent anchor against this exact projection without ever
    /// interpreting its stale compatibility ordinal as current.
    pub fn resolve_text_anchor(
        &self,
        document: DocumentId,
        anchor: TextAnchor,
    ) -> Result<MappingOutcome<TextPoint>, PositionError> {
        if anchor.document() != document {
            return Err(PositionError::WrongDocument {
                expected: document,
                actual: anchor.document(),
            });
        }
        if anchor.revision() == self.revision
            && anchor.offset() <= self.text.byte_len()
            && self
                .is_logical_grapheme_boundary(anchor.offset())
                .unwrap_or(false)
        {
            return Ok(MappingOutcome::Exact(TextPoint {
                document,
                revision: self.revision,
                offset: anchor.offset(),
            }));
        }
        // Empty paragraphs have no owned content leaf. Their neighboring
        // separator allocations can follow other rows in a permutation, while
        // the retained paragraph identity names the empty owner itself.
        if let Some(identity) = anchor.projected_block_boundary().filter(|block| block.block_len() == 0) {
            if let Some(offset) = self.resolve_block_boundary(identity) {
                return Ok(MappingOutcome::Moved(TextPoint { document, revision: self.revision, offset }));
            }
        }
        if let Some((preceding, following)) = anchor.projected_leaf_boundaries() {
            let preceding = preceding.and_then(|identity| self.resolve_leaf_boundary(identity));
            let following = following.and_then(|identity| self.resolve_leaf_boundary(identity));
            let resolved = match (preceding, following, anchor.association()) {
                (Some(left), Some(right), _) if left == right => Some(left),
                (Some(left), Some(_), Association::BeforeInsertion) => Some(left),
                (Some(_), Some(right), Association::AfterInsertion) => Some(right),
                (Some(only), None, _) | (None, Some(only), _) => Some(only),
                (None, None, _) => None,
            };
            if let Some(offset) = resolved
                .filter(|offset| self.is_logical_grapheme_boundary(*offset).unwrap_or(false))
            {
                let point = TextPoint {
                    document,
                    revision: self.revision,
                    offset,
                };
                return Ok(
                    if anchor.revision() == self.revision && anchor.offset() == offset {
                        MappingOutcome::Exact(point)
                    } else {
                        MappingOutcome::Moved(point)
                    },
                );
            }
        }

        if let Some(identity) = anchor.projected_block_boundary() {
            if let Some(offset) = self.resolve_block_boundary(identity) {
                let point = TextPoint {
                    document,
                    revision: self.revision,
                    offset,
                };
                return Ok(
                    if anchor.revision() == self.revision && anchor.offset() == offset {
                        MappingOutcome::Exact(point)
                    } else {
                        MappingOutcome::Moved(point)
                    },
                );
            }
        }

        if let Some(provenance) = anchor
            .source_provenance()
            .filter(|provenance| provenance.part() == SourcePartId::PRIMARY)
        {
            match self.map_source_boundary(self.revision, provenance.offset(), anchor.affinity()) {
                Ok(mapped) => {
                    if self.anchor_context_fingerprints(mapped.formatted_offset)
                        == (
                            provenance.preceding_text_fingerprint(),
                            provenance.following_text_fingerprint(),
                        )
                    {
                        return Ok(MappingOutcome::RecoveredFromProvenance(TextPoint {
                            document,
                            revision: self.revision,
                            offset: mapped.formatted_offset,
                        }));
                    }
                }
                Err(SourceToTextError::AmbiguousBoundary { candidates, .. }) => {
                    return Ok(MappingOutcome::Ambiguous(
                        candidates
                            .into_iter()
                            .map(|offset| TextPoint {
                                document,
                                revision: self.revision,
                                offset,
                            })
                            .collect(),
                    ));
                }
                Err(_) => {}
            }
        }

        Ok(MappingOutcome::Unresolvable(
            UnresolvableAnchor::MissingProvenance,
        ))
    }

    fn resolve_leaf_boundary(&self, identity: ProjectedLeafBoundary) -> Option<usize> {
        self.text.resolve_stable_boundary(
            identity.leaf(),
            identity.leaf_revision(),
            identity.buffer(),
            identity.buffer_byte(),
            identity.side(),
        )
    }

    fn block_anchor_at(
        &self,
        offset: usize,
        affinity: BoundaryAffinity,
    ) -> Option<ProjectedBlockBoundary> {
        let touching = self.blocks.query_touching(&(offset..offset));
        let selected = match affinity {
            BoundaryAffinity::Upstream => touching
                .iter()
                .rev()
                .find(|block| block.range.start < offset || block.range.end == offset)
                .or_else(|| touching.first()),
            BoundaryAffinity::Downstream => touching
                .iter()
                .find(|block| block.range.start == offset || offset < block.range.end)
                .or_else(|| touching.last()),
        }?;
        let text = self.text.slice(selected.range.clone()).ok()?;
        Some(ProjectedBlockBoundary::new(
            selected.id,
            offset.checked_sub(selected.range.start)?,
            selected.range.len(),
            stable_text_fingerprint(&text),
        ))
    }

    fn resolve_block_boundary(&self, identity: ProjectedBlockBoundary) -> Option<usize> {
        let block = self
            .blocks
            .items()
            .find(|block| block.id != 0 && block.id == identity.block())?;
        if block.range.len() != identity.block_len() || identity.local_byte() > block.range.len() {
            return None;
        }
        let text = self.text.slice(block.range.clone()).ok()?;
        if stable_text_fingerprint(&text) != identity.content_fingerprint() {
            return None;
        }
        let offset = block.range.start.checked_add(identity.local_byte())?;
        self.is_logical_grapheme_boundary(offset)
            .ok()
            .filter(|boundary| *boundary)
            .map(|_| offset)
    }

    fn anchor_context_fingerprints(&self, offset: usize) -> (Option<u64>, Option<u64>) {
        let preceding = self
            .previous_logical_grapheme_boundary(offset)
            .ok()
            .flatten()
            .and_then(|start| self.text.slice(start..offset).ok())
            .map(|text| stable_text_fingerprint(&text));
        let following = self
            .next_logical_grapheme_boundary(offset)
            .ok()
            .flatten()
            .and_then(|end| self.text.slice(offset..end).ok())
            .map(|text| stable_text_fingerprint(&text));
        (preceding, following)
    }

    /// Install a persistent text-tree splice after either a full or regional
    /// candidate has passed semantic verification. This preserves untouched
    /// leaf identities for downstream caches.
    /// First source byte after any BOM.
    pub(crate) fn source_content_start(&self) -> usize {
        self.source_content_start
    }

    pub(crate) fn install_persistent_text_edits(
        &mut self,
        previous: &Self,
        edits: &[TextEdit],
    ) -> Result<(), FormattedTextError> {
        // Splicing keeps unchanged leaves, and the stable anchors that name
        // them, but adds about two small leaves per edit. A dense edit set,
        // such as a whole-document view switch, would fragment the rope into
        // nodes that outweigh its text; keep this fresh compact rope instead.
        // Anchors then recover through block identity and source provenance.
        const EDITS_PER_LEAF: usize = 4;
        const EDIT_ALLOWANCE: usize = 32;
        if edits.len() > self.text.leaf_count().saturating_mul(EDITS_PER_LEAF).saturating_add(EDIT_ALLOWANCE) {
            return Ok(());
        }
        let edits: Vec<_> = edits
            .iter()
            .map(|edit| (edit.range.clone(), edit.replacement.as_str()))
            .collect();
        let persistent = previous.text.splice_prevalidated_batch(&edits)?;
        if persistent.flatten() != self.text() {
            return Err(FormattedTextError::ResultTextMismatch);
        }
        let first_change = edits.iter().map(|(range, _)| range.start).min();
        let before = first_change.and_then(|at| previous.hard_lines.index_touching_point(at)).unwrap_or(0);
        self.comment_context_cache = super::comment_context::prefix_cache(&previous.comment_context_cache, before);
        self.text = persistent;
        Ok(())
    }

    /// Reuse original contributor storage after a verified source permutation.
    /// Source lineage proves ownership; the local text comparison guards the
    /// disposable projection against transplanting an interpreted contributor.
    pub(crate) fn install_relocated_text_storage(
        &mut self,
        previous: &Self,
        relocations: &[(Range<usize>, usize)],
    ) -> Result<(), FormattedTextError> {
        let mut text = self.text.clone();
        for (source, target) in relocations {
            if source.is_empty() { continue; }
            let end = target.checked_add(source.len()).ok_or(FormattedTextError::ArithmeticOverflow)?;
            if previous.text.slice(source.clone())? != text.slice(*target..end)? {
                return Err(FormattedTextError::ResultTextMismatch);
            }
            let original = previous.text.subrope(source.clone())?;
            text = text.splice_subrope(*target..end, &original)?;
        }
        self.text = text;
        Ok(())
    }

    /// Retain complete literal paragraph identities after exact source moves.
    /// Empty paragraphs use these identities because they have no body slice.
    /// Donor IDs can survive an ordinary join at a patch edge; retire those
    /// local duplicates before installing the explicit owner correspondence.
    pub(crate) fn install_relocated_literal_line_ids(
        &mut self,
        previous: &Self,
        relocations: &[(Range<usize>, usize)],
        changed: &[Range<usize>],
        mut next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        if self.literal_encoding.is_none() || previous.literal_encoding.is_none()
            || self.blocks.len() != self.hard_lines.len() {
            return Err(BlockIdentityError::InvalidProjection);
        }
        let mut assignments = std::collections::BTreeMap::new();
        let mut transferred = std::collections::HashSet::new();
        for (source, destination) in relocations {
            for old in previous.blocks.query_touching(source) {
                if old.range.start < source.start || old.range.end > source.end
                    || (!source.is_empty() && old.range.start == source.end) {
                    continue;
                }
                let start = destination.checked_add(old.range.start - source.start)
                    .ok_or(BlockIdentityError::Exhausted)?;
                let end = start.checked_add(old.range.len()).ok_or(BlockIdentityError::Exhausted)?;
                let Some(index) = self.blocks.index_touching_point(start) else { continue; };
                let Some(target) = self.blocks.get(index).filter(|block| block.range == (start..end)) else { continue; };
                if target.kind != old.kind { continue; }
                if !transferred.insert(old.id) || assignments.insert(index, old.id).is_some() {
                    return Err(BlockIdentityError::InvalidProjection);
                }
            }
        }
        if assignments.is_empty() { return Ok(next_id); }
        let mut affected = std::collections::BTreeMap::new();
        for range in changed {
            for block in self.blocks.query_touching(range) {
                let index = self.blocks.index_touching_point(block.range.start)
                    .ok_or(BlockIdentityError::InvalidProjection)?;
                affected.insert(index, block);
            }
        }
        for index in assignments.keys() {
            affected.entry(*index).or_insert(self.blocks.get(*index).ok_or(BlockIdentityError::InvalidProjection)?);
        }
        let mut stats = RangeSpliceStats::default();
        for (index, mut block) in affected {
            let old_id = block.id;
            if let Some(id) = assignments.get(&index) {
                block.id = *id;
            } else if transferred.contains(&block.id) {
                block.id = next_id;
                next_id = next_id.checked_add(1).ok_or(BlockIdentityError::Exhausted)?;
            }
            if block.id == old_id { continue; }
            let mut line = self.hard_lines.get(index).ok_or(BlockIdentityError::InvalidProjection)?;
            if line.range != block.range { return Err(BlockIdentityError::InvalidProjection); }
            line.id = block.id;
            self.blocks = self.blocks.splice(index..index + 1, vec![block], 0, 0, &mut stats)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            self.hard_lines = self.hard_lines.splice(index..index + 1, vec![line], 0, 0, &mut stats)
                .ok_or(BlockIdentityError::InvalidProjection)?;
        }
        super::work_statistics::record(|work| {
            work.projection_persistent_nodes_visited += stats.nodes_visited;
            work.projection_persistent_nodes_copied += stats.nodes_copied;
        });
        Ok(next_id)
    }

    /// Reuse both canonical rope and compatibility flat storage when a full
    /// projection verified that the formatted text is byte-identical.
    pub(crate) fn install_unchanged_text_storage(
        &mut self,
        previous: &Self,
    ) -> Result<(), FormattedTextError> {
        if self.text() != previous.text() {
            return Err(FormattedTextError::ResultTextMismatch);
        }
        self.text = previous.text.clone();
        self.flat_text = previous.flat_text.clone();
        if self.hard_lines == previous.hard_lines {
            self.comment_context_cache = super::comment_context::prefix_cache(&previous.comment_context_cache, usize::MAX);
        }
        Ok(())
    }

    /// Rich adapters supply an independent paragraph partition after seeding
    /// their explicit hard-line records. Literal source newline characters in
    /// other adapters are never reinterpreted by this operation.
    pub(crate) fn install_paragraph_partition(&mut self, paragraphs: Vec<Block>) {
        self.try_install_paragraph_partition(paragraphs)
            .expect("rich projectors emit a valid paragraph partition");
    }

    /// Installs a partition derived from an earlier projection, which is only
    /// a candidate until it is validated against this projection's text.
    pub(crate) fn try_install_paragraph_partition(
        &mut self,
        paragraphs: Vec<Block>,
    ) -> Result<(), BlockIdentityError> {
        validate_block_partition(self.text(), &paragraphs)?;
        self.blocks = OrderedRangeStore::new(paragraphs);
        Ok(())
    }

    pub(crate) fn install_hard_line_partition(&mut self, ranges: Vec<Range<usize>>) {
        self.comment_context_cache = super::comment_context::fresh_cache();
        let line_blocks = blocks_for_hard_line_ranges(&ranges);
        self.hard_lines = OrderedRangeStore::new(
            seed_hard_lines_from_current_blocks(&line_blocks, self.text())
                .expect("regional rich edits preserve a valid hard-line partition"),
        );
    }

    fn rebuild_hard_lines_from_blocks(
        &mut self,
        previous: Option<&Self>,
    ) -> Result<(), BlockIdentityError> {
        let blocks = self.blocks.to_vec();
        let hard_lines = seed_hard_lines_from_current_blocks(&blocks, self.text())?;
        self.hard_lines = OrderedRangeStore::new(hard_lines);
        if let Some(previous) = previous {
            self.hard_lines.reuse_equal_chunks(&previous.hard_lines);
        }
        Ok(())
    }

    fn assign_initial_hard_line_ids(
        &mut self,
        mut next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        if self.blocks.len() == self.hard_lines.len() {
            self.rebuild_hard_lines_from_blocks(None)?;
            return Ok(next_id);
        }
        let blocks = self.blocks.to_vec();
        let mut lines = self.hard_lines.to_vec();
        for line in &mut lines {
            if let Some(block) = blocks
                .get(blocks.partition_point(|block| block.range.start < line.range.start))
                .filter(|block| block.range.start == line.range.start)
            {
                line.id = block.id;
            } else {
                line.id = next_id;
                next_id = next_id
                    .checked_add(1)
                    .ok_or(BlockIdentityError::Exhausted)?;
            }
        }
        self.hard_lines = OrderedRangeStore::new(lines);
        Ok(next_id)
    }

    fn install_unchanged_hard_line_ids(
        &mut self,
        previous: &Self,
    ) -> Result<(), BlockIdentityError> {
        let mut lines = self.hard_lines.to_vec();
        let old_lines = previous.hard_lines.to_vec();
        if lines.len() != old_lines.len()
            || lines.iter().zip(&old_lines).any(|(new, old)| {
                new.range != old.range || new.separator_length != old.separator_length
            })
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        for (line, old) in lines.iter_mut().zip(old_lines) {
            line.id = old.id;
        }
        self.hard_lines = OrderedRangeStore::new(lines);
        self.hard_lines.reuse_equal_chunks(&previous.hard_lines);
        Ok(())
    }

    fn reconcile_hard_line_ids(
        &mut self,
        previous: &Self,
        edits: &[TextEdit],
        mut next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        if self.blocks.len() == self.hard_lines.len()
            && previous.blocks.len() == previous.hard_lines.len()
        {
            self.rebuild_hard_lines_from_blocks(Some(previous))?;
            return Ok(next_id);
        }
        let old_lines = previous.hard_lines.to_vec();
        let mut lines = self.hard_lines.to_vec();
        let mut line_blocks = blocks_for_hard_line_ranges(
            &lines
                .iter()
                .map(|line| line.range.clone())
                .collect::<Vec<_>>(),
        );
        let mappings = build_edit_mappings(previous.text().len(), self.text().len(), edits)?;
        for old in &old_lines {
            let block = Block::new(old.id, old.range.clone(), BlockKind::Paragraph, "Paragraph".into(), None);
            let target = if let Some(witness) =
                block_identity_witness(previous.text(), &block, &mappings)?
            {
                let mapped = map_surviving_byte(witness, &mappings)?;
                block_index_for_witness(
                    self.text(),
                    &line_blocks,
                    mapped,
                    witness_side(previous.text(), &block, witness),
                )?
            } else if let Some(mapped) =
                deleted_empty_join_boundary(previous.text(), &block, &mappings)
            {
                block_index_for_witness(
                    self.text(),
                    &line_blocks,
                    mapped,
                    BlockWitnessSide::PrecedingBreak,
                )?
            } else if old_lines.len() == 1 {
                0
            } else {
                continue;
            };
            if line_blocks[target].id == 0 {
                line_blocks[target].id = old.id;
            }
        }
        next_id = allocate_unassigned_block_ids(&mut line_blocks, next_id)?;
        for (line, block) in lines.iter_mut().zip(line_blocks) {
            line.id = block.id;
        }
        self.hard_lines = OrderedRangeStore::new(lines);
        self.hard_lines.reuse_equal_chunks(&previous.hard_lines);
        Ok(next_id)
    }

    /// Assign identities to an initial projection. Zero is reserved for the
    /// projector's unpublished provisional blocks.
    pub(crate) fn assign_initial_block_ids(
        &mut self,
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        let mut blocks = self.blocks.to_vec();
        validate_block_partition(self.text(), &blocks)?;
        let next_id = allocate_unassigned_block_ids(&mut blocks, next_id)?;
        super::containers::reconcile(&mut blocks, &[], false);
        self.blocks = OrderedRangeStore::new(blocks);
        let next_id=self.install_table_ids(next_id,None)?;
        let next_id = self.assign_initial_hard_line_ids(next_id)?;
        // Initial validation used the construction string. Literal snapshots
        // publish only bounded text buffers; a legacy flat read remains lazy.
        if self.literal_encoding.is_some() { self.flat_text = Arc::new(OnceLock::new()); }
        Ok(next_id)
    }

    /// Preserve every logical block identity when a fully verified
    /// reprojection has byte-identical formatted text and block boundaries.
    /// Block kind is deliberately not part of identity: a source-backed
    /// structural line that changes semantic interpretation may retain its ID.
    pub(crate) fn install_unchanged_block_ids(
        &mut self,
        previous: &Self,
    ) -> Result<(), BlockIdentityError> {
        let mut blocks = self.blocks.to_vec();
        let previous_blocks = previous.blocks.to_vec();
        if self.text() != previous.text()
            || blocks.len() != previous_blocks.len()
            || blocks
                .iter()
                .zip(&previous_blocks)
                .any(|(candidate, old)| candidate.range != old.range)
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        for (candidate, old) in blocks.iter_mut().zip(&previous_blocks) {
            candidate.id = old.id;
            if candidate.direct_formatting != old.direct_formatting {
                candidate.direct_formatting = old.direct_formatting.clone();
            }
            if previous
                .style_sheet
                .configuration_deleted(&candidate.style, true)
            {
                candidate.style = previous.style_sheet.base_paragraph.clone();
            }
        }
        self.style_sheet = previous.style_sheet.clone();
        self.document_style = previous.document_style.clone();
        super::containers::reconcile(&mut blocks, &previous_blocks, false);
        self.blocks = OrderedRangeStore::new(blocks);
        self.blocks.reuse_equal_chunks(&previous.blocks);
        self.install_table_ids(u64::MAX,Some(previous))?;
        self.install_unchanged_hard_line_ids(previous)?;
        self.inline_images.reuse_equal_chunks(&previous.inline_images);
        self.styles.reuse_equal_chunks(&previous.styles);
        Ok(())
    }

    /// Reconcile provisional block IDs after the candidate has been fully
    /// parsed and semantically verified. The exact position map establishes
    /// the snapshot transition; edit ranges decide which source-backed line
    /// structure survived. Surviving content is preferred as the witness; if
    /// none remains, a non-final block uses its following hard break and the
    /// final block uses its preceding break. This retains identity across
    /// complete content replacement while allowing a whole-line deletion to
    /// retire the deleted ID. An insertion at a block start leaves the old ID
    /// with the original content, a join gives a collision to the first
    /// surviving block in document order, and a split copies sparse direct
    /// declarations to each child without replacing projector-derived named
    /// styles. Parsing itself is still a full reprojection.
    pub(crate) fn install_reconciled_block_ids(
        &mut self,
        previous: &Self,
        edits: &[TextEdit],
        position_map: &PositionMap,
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        if position_map.domain() != PositionDomain::FormattedText
            || position_map.source_revision() != previous.revision
            || position_map.target_revision() != self.revision
            || position_map.source_len() != previous.text().len()
            || position_map.target_len() != self.text().len()
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        let previous_blocks = previous.blocks.to_vec();
        let mut blocks = self.blocks.to_vec();
        validate_block_partition(previous.text(), &previous_blocks)?;
        validate_block_partition(self.text(), &blocks)?;
        if next_id == 0
            || previous_blocks
                .iter()
                .any(|block| block.id == 0 || block.id >= next_id)
        {
            return Err(BlockIdentityError::InvalidProjection);
        }

        let edit_mappings = build_edit_mappings(previous.text().len(), self.text().len(), edits)?;
        for block in &mut blocks {
            block.id = 0;
        }
        let new_text = self.text().to_owned();

        for old in &previous_blocks {
            inherit_split_direct_assignments(&new_text, &mut blocks, old, &edit_mappings)?;
            let witness = block_identity_witness(previous.text(), old, &edit_mappings)?;
            let target = if let Some(old_offset) = witness {
                let mapped_offset = map_surviving_byte(old_offset, &edit_mappings)?;
                let side = witness_side(previous.text(), old, old_offset);
                block_index_for_witness(self.text(), &blocks, mapped_offset, side)?
            } else if let Some(mapped_break) =
                deleted_empty_join_boundary(previous.text(), old, &edit_mappings)
            {
                // Removing the separator after an empty block is still a
                // join. Preserve the first block's ID even though that block
                // has no content byte to serve as a witness.
                block_index_for_witness(
                    self.text(),
                    &blocks,
                    mapped_break,
                    BlockWitnessSide::PrecedingBreak,
                )?
            } else if previous_blocks.len() == 1 {
                // The document's sole structural block survives a complete
                // content replacement or deletion even though no old byte can
                // witness it. If it was split, it remains the first block.
                0
            } else {
                continue;
            };
            if blocks[target].id == 0 {
                // Multiple surviving blocks can map into one block after a
                // join. Iteration is in document order, so the first ID wins.
                let target = &mut blocks[target];
                target.id = old.id;
                if target.direct_formatting != old.direct_formatting {
                    target.direct_formatting = old.direct_formatting.clone();
                }
            }
        }

        self.style_sheet = previous.style_sheet.clone();
        self.document_style = previous.document_style.clone();
        for block in &mut blocks {
            if self.style_sheet.configuration_deleted(&block.style, true) {
                block.style = self.style_sheet.base_paragraph.clone();
            }
        }
        let next_id = allocate_unassigned_block_ids(&mut blocks, next_id)?;
        super::containers::reconcile(&mut blocks, &previous_blocks, false);
        self.blocks = OrderedRangeStore::new(blocks);
        self.blocks.reuse_equal_chunks(&previous.blocks);
        let next_id=self.install_table_ids(next_id,Some(previous))?;
        let next_id = self.reconcile_hard_line_ids(previous, edits, next_id)?;
        self.inline_images.reuse_equal_chunks(&previous.inline_images);
        self.styles.reuse_equal_chunks(&previous.styles);
        Ok(next_id)
    }

    /// Install move-aware block identities after a verified source-backed
    /// hard-line transfer. The initial adapters publish exactly one block per
    /// hard line; refusing any other shape keeps this temporary contract
    /// explicit until nested/multi-line block transfer is modeled directly.
    pub(crate) fn install_reconciled_format_block_ids(
        &mut self, _format: Format, previous: &Self, edits: &[TextEdit],
        position_map: &PositionMap, next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        self.install_reconciled_block_ids(previous, edits, position_map, next_id)
    }

    /// Reprojection may replace the format's complete style interpretation.
    /// Preserve newly parsed properties while recovering logical identities.
    pub(crate) fn install_reconciled_source_block_ids(
        &mut self,
        previous: &Self,
        edits: &[TextEdit],
        position_map: &PositionMap,
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        let parsed_blocks = self.blocks.to_vec();
        let parsed_sheet = self.style_sheet.clone();
        let parsed_document = self.document_style.clone();
        let next_id = self.install_reconciled_block_ids(previous, edits, position_map, next_id)?;
        let mut blocks = self.blocks.to_vec();
        for (block, parsed) in blocks.iter_mut().zip(parsed_blocks) {
            if block.direct_formatting != parsed.direct_formatting {
                block.direct_formatting = parsed.direct_formatting.clone();
            }
            if previous
                .style_sheet
                .configuration_deleted(&block.style, true)
            {
                block.style = previous.style_sheet.base_paragraph.clone();
            }
        }
        self.blocks = OrderedRangeStore::new(blocks);
        self.style_sheet = parsed_sheet;
        Arc::make_mut(&mut self.style_sheet)
            .retain_configuration_deletions(&previous.style_sheet);
        self.document_style = parsed_document;
        self.document_style.direct_canvas = previous.document_style.direct_canvas.clone();
        self.document_style.direct_default_character =
            previous.document_style.direct_default_character.clone();
        Ok(next_id)
    }

    pub(crate) fn install_transferred_block_ids(
        &mut self,
        previous: &Self,
        origins: &[TransferredLineOrigin],
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        let previous_blocks = previous.blocks.to_vec();
        let mut blocks = self.blocks.to_vec();
        if self.hard_lines.len() == origins.len()
            && (previous_blocks.len() != previous.hard_lines.len()
                || blocks.len() != self.hard_lines.len())
        {
            validate_block_partition(previous.text(), &previous_blocks)?;
            validate_block_partition(self.text(), &blocks)?;
            let old_lines = previous.hard_lines.to_vec();
            if next_id == 0
                || old_lines
                    .iter()
                    .any(|line| line.id == 0 || line.id >= next_id)
            {
                return Err(BlockIdentityError::InvalidProjection);
            }
            let mut next_id = next_id;
            let mut lines = self.hard_lines.to_vec();
            for (line, origin) in lines.iter_mut().zip(origins) {
                line.id = match origin {
                    TransferredLineOrigin::Existing(index) => {
                        old_lines
                            .get(*index)
                            .ok_or(BlockIdentityError::InvalidProjection)?
                            .id
                    }
                    TransferredLineOrigin::Copied(index) => {
                        if old_lines.get(*index).is_none() {
                            return Err(BlockIdentityError::InvalidProjection);
                        }
                        let id = next_id;
                        next_id = next_id
                            .checked_add(1)
                            .ok_or(BlockIdentityError::Exhausted)?;
                        id
                    }
                };
            }
            for block in &mut blocks {
                let first = lines.partition_point(|line| line.range.start < block.range.start);
                let line = lines
                    .get(first)
                    .filter(|line| line.range.start == block.range.start)
                    .ok_or(BlockIdentityError::InvalidProjection)?;
                let source_index = match origins[first] {
                    TransferredLineOrigin::Existing(index)
                    | TransferredLineOrigin::Copied(index) => index,
                };
                let source_line = &old_lines[source_index];
                let source_index = previous_blocks
                    .partition_point(|block| block.range.start <= source_line.range.start)
                    .saturating_sub(1);
                let source = previous_blocks
                    .get(source_index)
                    .filter(|block| {
                        block.range.start <= source_line.range.start
                            && source_line.range.end <= block.range.end
                    })
                    .ok_or(BlockIdentityError::InvalidProjection)?;
                block.id = line.id;
                block.direct_formatting = source.direct_formatting.clone();
            }
            self.style_sheet = previous.style_sheet.clone();
            self.document_style = previous.document_style.clone();
            self.blocks = OrderedRangeStore::new(blocks);
            self.blocks.reuse_equal_chunks(&previous.blocks);
            self.hard_lines = OrderedRangeStore::new(lines);
            self.hard_lines.reuse_equal_chunks(&previous.hard_lines);
            let next_id = self.install_table_ids(next_id, Some(previous))?;
            self.rebuild_table_presentation_lines();
            self.inline_images.reuse_equal_chunks(&previous.inline_images);
        self.styles.reuse_equal_chunks(&previous.styles);
            return Ok(next_id);
        }
        if previous_blocks.len() != previous.hard_lines.len()
            || blocks.len() != self.hard_lines.len()
            || blocks.len() != origins.len()
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        validate_block_partition(previous.text(), &previous_blocks)?;
        validate_block_partition(self.text(), &blocks)?;
        if next_id == 0
            || previous_blocks
                .iter()
                .any(|block| block.id == 0 || block.id >= next_id)
        {
            return Err(BlockIdentityError::InvalidProjection);
        }

        for (candidate, origin) in blocks.iter_mut().zip(origins) {
            let source_index = match origin {
                TransferredLineOrigin::Existing(index) | TransferredLineOrigin::Copied(index) => {
                    *index
                }
            };
            let source = previous_blocks
                .get(source_index)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            candidate.direct_formatting = source.direct_formatting.clone();
            candidate.id = match origin {
                TransferredLineOrigin::Existing(_) => source.id,
                TransferredLineOrigin::Copied(_) => 0,
            };
        }

        self.style_sheet = previous.style_sheet.clone();
        self.document_style = previous.document_style.clone();
        let next_id = allocate_unassigned_block_ids(&mut blocks, next_id)?;
        self.blocks = OrderedRangeStore::new(blocks);
        self.blocks.reuse_equal_chunks(&previous.blocks);
        self.rebuild_hard_lines_from_blocks(Some(previous))?;
        let next_id = self.install_table_ids(next_id, Some(previous))?;
        self.rebuild_table_presentation_lines();
        self.inline_images.reuse_equal_chunks(&previous.inline_images);
        self.styles.reuse_equal_chunks(&previous.styles);
        Ok(next_id)
    }

    #[cfg(test)]
    pub(crate) fn shares_flat_text_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flat_text, &other.flat_text)
    }

    #[cfg(test)]
    pub(crate) fn compatibility_text_is_materialized(&self) -> bool {
        self.flat_text.get().is_some()
    }

    /// Physical byte length of the single-part source snapshot represented by
    /// this projection. This includes a BOM and hidden format syntax.
    pub fn source_byte_len(&self) -> usize {
        self.source_content_end
    }

    /// Map one exact physical source-byte boundary into this formatted
    /// projection.
    ///
    /// Interior bytes of an encoding unit, CRLF token, opaque malformed unit,
    /// BOM, or hidden format delimiter are rejected rather than rounded. At a
    /// boundary with distinct preceding and following projections, `affinity`
    /// explicitly selects the corresponding side.
    pub fn map_source_boundary(
        &self,
        source_revision: Revision,
        source_offset: usize,
        affinity: BoundaryAffinity,
    ) -> Result<ProjectedSourceBoundary, SourceToTextError> {
        if source_revision != self.revision {
            return Err(SourceToTextError::WrongSnapshot {
                expected: self.revision,
                actual: source_revision,
            });
        }
        if source_offset > self.source_content_end {
            return Err(SourceToTextError::SourceOffsetOutOfBounds {
                offset: source_offset,
                length: self.source_content_end,
            });
        }

        let relation = if self.source_content_end == 0 {
            SourceBoundaryRelation::EmptyDocument
        } else if source_offset == 0 {
            SourceBoundaryRelation::DocumentStart
        } else if source_offset == self.source_content_end {
            SourceBoundaryRelation::DocumentEnd
        } else {
            SourceBoundaryRelation::Exact
        };
        if source_offset == 0 {
            return self.checked_source_mapping(source_offset, 0, affinity, relation);
        }
        if source_offset == self.source_content_end {
            return self.checked_source_mapping(
                source_offset,
                self.text.byte_len(),
                affinity,
                relation,
            );
        }
        if source_offset < self.source_content_start {
            return Err(SourceToTextError::InteriorBom {
                source_range: 0..self.source_content_start,
            });
        }

        if self.literal_encoding.is_some() {
            let span = self.literal_span_for_source(source_offset)
                .ok_or(SourceToTextError::UnmappedBoundary { source_offset })?;
            if let Some(formatted) = self.literal_text_in_span(&span, source_offset) {
                return self.checked_source_mapping(source_offset, formatted, affinity, relation);
            }
            let span = self.literal_source_unit(&span, source_offset);
            let opaque = self.has_decoding_diagnostic_overlapping(&span.formatted);
            return Err(if opaque {
                SourceToTextError::InteriorOpaqueUnit {
                    source_range: span.source, formatted_range: span.formatted,
                }
            } else {
                SourceToTextError::InteriorMappedUnit {
                    source_range: span.source, formatted_range: span.formatted,
                }
            });
        }

        // The source-ordered boundary index holds every contributor edge, so
        // an exact boundary is found without scanning the document's provenance.
        let mut upstream = Vec::new();
        let mut downstream = Vec::new();
        for boundary in self.provenance.boundaries_touching(&(source_offset..source_offset), &self.text) {
            if boundary.source.start != source_offset {
                continue;
            }
            match boundary.side {
                Side::Upstream => upstream.push(boundary.formatted),
                Side::Downstream => downstream.push(boundary.formatted),
            }
        }
        upstream.sort_unstable();
        upstream.dedup();
        downstream.sort_unstable();
        downstream.dedup();
        let (preferred, fallback) = match affinity {
            BoundaryAffinity::Upstream => (&upstream, &downstream),
            BoundaryAffinity::Downstream => (&downstream, &upstream),
        };
        // The opposite side is a fallback only when this affinity has no
        // contributor. Its ambiguity cannot invalidate a unique requested
        // side, and an ambiguous requested side must still be rejected.
        let selected = match unique_candidate(source_offset, preferred)? {
            Some(offset) => Some(offset),
            None => unique_candidate(source_offset, fallback)?,
        };
        if let Some(formatted_offset) = selected {
            return self.checked_source_mapping(
                source_offset,
                formatted_offset,
                affinity,
                relation,
            );
        }

        // Exact child boundaries remain editable inside a larger source owner
        // when a format parser has reordered its visible children. Only a
        // point with no exact contributor boundary is an interior source unit.
        if let Some(span) = self.provenance.containing_source(source_offset, &self.text) {
            let opaque = self.decoding_diagnostics.query_touching(&span.formatted).iter().any(|diagnostic| {
                diagnostic.source_range.start <= source_offset
                    && source_offset < diagnostic.source_range.end
            });
            return Err(if opaque {
                SourceToTextError::InteriorOpaqueUnit {
                    source_range: span.source.clone(),
                    formatted_range: span.formatted.clone(),
                }
            } else {
                SourceToTextError::InteriorMappedUnit {
                    source_range: span.source.clone(),
                    formatted_range: span.formatted.clone(),
                }
            });
        }

        if let Some((source_range, upstream_formatted, downstream_formatted)) =
            self.hidden_gap(source_offset)
        {
            if source_offset == source_range.start || source_offset == source_range.end {
                let formatted_offset = match affinity {
                    BoundaryAffinity::Upstream => upstream_formatted.or(downstream_formatted),
                    BoundaryAffinity::Downstream => downstream_formatted.or(upstream_formatted),
                };
                if let Some(formatted_offset) = formatted_offset {
                    return self.checked_source_mapping(
                        source_offset,
                        formatted_offset,
                        affinity,
                        relation,
                    );
                }
            }
            return Err(SourceToTextError::InteriorHiddenSyntax {
                source_range,
                upstream_formatted,
                downstream_formatted,
            });
        }

        Err(SourceToTextError::UnmappedBoundary { source_offset })
    }

    fn checked_source_mapping(
        &self,
        source_offset: usize,
        formatted_offset: usize,
        affinity: BoundaryAffinity,
        relation: SourceBoundaryRelation,
    ) -> Result<ProjectedSourceBoundary, SourceToTextError> {
        if !self.is_logical_grapheme_boundary(formatted_offset)? {
            return Err(SourceToTextError::NotFormattedGraphemeBoundary {
                source_offset,
                formatted_offset,
            });
        }
        Ok(ProjectedSourceBoundary {
            revision: self.revision,
            source_offset,
            formatted_offset,
            affinity,
            relation,
        })
    }

    /// The uncovered source range containing an offset that no contributor
    /// contains, with the formatted edges of its neighbors.
    fn hidden_gap(
        &self,
        source_offset: usize,
    ) -> Option<(Range<usize>, Option<usize>, Option<usize>)> {
        if self.source_ordered {
            let (preceding, following) = self.provenance.uncovered_neighbors(source_offset);
            let start = preceding.map_or(self.source_content_start, |(source, _)| source);
            let end = following.map_or(self.source_content_end, |(source, _)| source);
            return (start < end && start <= source_offset && source_offset <= end)
                .then(|| (start..end, preceding.map(|(_, formatted)| formatted), following.map(|(_, formatted)| formatted)));
        }
        // Reordered contributors: follow the formatted-order source frontier.
        let mut source_start = self.source_content_start;
        let mut upstream = None;
        // A divisible run is gap-free, so its hull has the same edges.
        for span in self.provenance.stored() {
            if source_start < span.source.start
                && source_start <= source_offset
                && source_offset <= span.source.start
            {
                return Some((
                    source_start..span.source.start,
                    upstream,
                    Some(span.formatted.start),
                ));
            }
            if span.source.end > source_start {
                source_start = span.source.end;
                upstream = Some(span.formatted.end);
            }
        }
        (source_start < self.source_content_end
            && source_start <= source_offset
            && source_offset <= self.source_content_end)
            .then_some((source_start..self.source_content_end, upstream, None))
    }

    /// Attach parser-owned scopes before publishing stable paragraph identities.
    /// Source positions select membership only during projection; retained paths
    /// contain identities and declarations, never mutable document offsets.
    pub(super) fn install_source_containers(&mut self, mut scopes: Vec<super::containers::SourceContainer>) {
        if scopes.is_empty() { return; }
        scopes.sort_by(|a, b| a.range.start.cmp(&b.range.start).then_with(|| b.range.end.cmp(&a.range.end)));
        let owners: Vec<_> = scopes.iter().map(|scope| {
            let style = if self.style_sheet.block_style(&scope.style).is_some() && !self.style_sheet.configuration_deleted(&scope.style, true) {
                scope.style.clone()
            } else { self.style_sheet.base_paragraph.clone() };
            Arc::new(super::ContainerAttributes { id: super::containers::provisional_identity(), kind: scope.kind,
                style, direct_formatting: scope.direct_formatting.clone() })
        }).collect();
        let mut blocks = self.blocks.to_vec();
        let mut active: Vec<usize> = Vec::new();
        let mut next = 0;
        for block in &mut blocks {
            let at = self.provenance_touching(&(block.range.start..block.range.start)).into_iter()
                .find(|span| span.formatted.start <= block.range.start &&
                    (block.range.start < span.formatted.end || span.formatted.is_empty()))
                .map(|span| span.source.start)
                .or_else(|| self.source_insertion_point(block.range.start, true));
            let Some(at) = at else { continue; };
            // Source-visible paragraphs include their outer marker bytes. A
            // nested owner's opener may occur after that first source point;
            // its scope still owns this same paragraph's visible body.
            let source_end = self.source_insertion_point(block.range.end, false).unwrap_or(at).max(at);
            let owns = |index: usize| {
                let scope = &scopes[index];
                at < scope.range.end || block.range.is_empty() && at == scope.range.end && (scope.owns_empty_end || match scope.kind {
                    super::ContainerKind::Quote => block.quote_depth > 0,
                    super::ContainerKind::CodeBlock => block.style.0 == "Code Block",
                    super::ContainerKind::List { .. } | super::ContainerKind::ListItem => matches!(block.kind, BlockKind::ListItem { .. }),
                })
            };
            active.retain(|index| owns(*index));
            while next < scopes.len() && scopes[next].range.start <= source_end {
                if owns(next) { active.push(next); }
                next += 1;
            }
            if active.is_empty() { continue; }
            let path = active.iter().map(|index| super::ContainerMembership {
                container: owners[*index].clone(), starts_here: false, ends_here: false,
            }).chain(block.containers.iter().cloned()).collect::<Vec<_>>();
            block.containers = path.into();
            block.quote_depth = block.containers.iter().filter(|member| member.container.kind == super::ContainerKind::Quote).count();
        }
        super::containers::mark_edges(&mut blocks);
        self.blocks = OrderedRangeStore::new(blocks);
    }

    /// Every block, as a flat compatibility copy that this snapshot then
    /// retains (and history with it). Editing paths use `blocks_for_region`,
    /// `block_count` or other bounded queries; work statistics count each copy.
    pub fn blocks(&self) -> &[Block] {
        self.blocks.compatibility_slice()
    }

    /// Every style span, as a retained flat compatibility copy; see `blocks`.
    pub fn style_spans(&self) -> &[StyleSpan] {
        self.styles.compatibility_slice()
    }

    /// Every block in order, streamed in bounded chunks that this snapshot
    /// does not retain, for complete verification passes.
    pub(crate) fn block_items(&self) -> impl Iterator<Item = Block> + '_ {
        self.blocks.items()
    }

    /// Every style span in order, streamed without a retained copy.
    pub(crate) fn style_span_items(&self) -> impl Iterator<Item = StyleSpan> + '_ {
        self.styles.items()
    }

    /// Whether both snapshots present the same formatted text, compared
    /// without flattening or retaining a compatibility copy of either.
    pub(crate) fn same_text(&self, other: &Self) -> bool {
        self.text.content_eq(&other.text)
    }

    /// A one-off copy of every block that this snapshot does not retain, for
    /// small temporary projections such as a regional parse.
    pub(crate) fn block_records(&self) -> Vec<Block> {
        self.blocks.to_vec()
    }

    /// A one-off copy of every style span; see `block_records`.
    pub(crate) fn style_span_records(&self) -> Vec<StyleSpan> {
        self.styles.to_vec()
    }

    /// Number of semantic hard lines in this exact projection. This is not
    /// derived from scalar values in [`Self::text`].
    pub fn hard_line_count(&self) -> usize {
        self.hard_lines.len()
    }

    /// Snapshot-absolute formatted range for one zero-based hard line in
    /// `O(log n)` time. The terminating hard-break item is excluded.
    pub fn hard_line_range(&self, line: usize) -> Option<Range<usize>> {
        self.hard_lines.get(line).map(|line| line.range)
    }

    /// Stable identity of one projected hard line, independent of the
    /// paragraph-bearing block which contains it.
    pub fn hard_line_id(&self, line: usize) -> Option<u64> {
        self.hard_lines.get(line).map(|line| line.id)
    }

    pub(crate) fn hard_line_snapshot(&self, document: DocumentId) -> HardLineSnapshot {
        HardLineSnapshot::new(document, self)
    }

    /// Resolve a valid formatted UTF-8 byte boundary to its zero-based hard
    /// line in `O(log n)`. A boundary immediately before an explicit hard break
    /// belongs to the preceding line; the boundary after it belongs to the
    /// following line. EOF belongs to the final line, including a trailing
    /// empty line. Literal U+000A content has no special treatment.
    pub fn hard_line_at_offset(&self, offset: usize) -> Option<usize> {
        if offset > self.text.byte_len() || !self.text.is_char_boundary(offset).ok()? {
            return None;
        }
        self.hard_lines.index_touching_point(offset)
    }

    pub(crate) fn hard_breaks_for_region(&self, range: &Range<usize>) -> Vec<usize> {
        self.hard_lines
            .query_touching(range)
            .iter()
            .filter_map(|line| line.separator_range())
            .filter(|separator| range.start <= separator.start && separator.end <= range.end)
            .map(|separator| separator.start)
            .collect()
    }

    /// First semantic difference from `other`, for shadow verification of a
    /// regional candidate against complete reprojection. Block identities,
    /// cached compatibility views and provenance are not compared, and the
    /// reads retain no flat copies in either snapshot.
    pub(crate) fn semantic_difference(&self, other: &Self) -> Option<String> {
        fn excerpt(text: &str, at: usize) -> &str {
            let mut start = at.saturating_sub(24);
            while !text.is_char_boundary(start) { start -= 1; }
            let mut end = (at + 24).min(text.len());
            while !text.is_char_boundary(end) { end += 1; }
            &text[start..end]
        }
        fn clipped(value: impl fmt::Debug) -> String {
            let text = format!("{value:?}");
            text.chars().take(600).collect()
        }
        let (left, right) = (self.text.flatten(), other.text.flatten());
        if left != right {
            let at = left.bytes().zip(right.bytes()).position(|(a, b)| a != b)
                .unwrap_or(left.len().min(right.len()));
            return Some(format!("text differs at byte {at}: regional {:?}, complete {:?}",
                excerpt(&left, at), excerpt(&right, at)));
        }
        // Configuration overlays (direct formatting and deleted-style
        // fallbacks) are installed identically for every strategy when block
        // identities are reconciled, so `other` is compared through this
        // snapshot's style sheet and without direct formatting.
        let (left, right) = (self.blocks.to_vec(), other.blocks.to_vec());
        let structure = |block: &Block, sheet: &StyleSheet| {
            let mut attributes = (*block.attributes).clone();
            attributes.direct_formatting = None;
            if sheet.configuration_deleted(&attributes.style, true) {
                attributes.style = sheet.base_paragraph.clone();
            }
            (block.range.clone(), attributes)
        };
        if let Some((index, (a, b))) = left.iter().zip(&right).enumerate()
            .find(|(_, (a, b))| structure(a, &self.style_sheet) != structure(b, &self.style_sheet))
        {
            let ((a_range, a), (b_range, b)) = (structure(a, &self.style_sheet), structure(b, &self.style_sheet));
            let mut fields = Vec::new();
            macro_rules! field {
                ($name:literal, $left:expr, $right:expr) => {
                    if $left != $right {
                        fields.push(format!("{}: {} vs {}", $name, clipped(&$left), clipped(&$right)));
                    }
                };
            }
            field!("range", a_range, b_range);
            field!("kind", a.kind, b.kind);
            field!("style", a.style, b.style);
            field!("quote_depth", a.quote_depth, b.quote_depth);
            field!("list_loose", a.list_loose, b.list_loose);
            field!("list_editing", a.list_editing, b.list_editing);
            field!("thematic_break", a.thematic_break, b.thematic_break);
            field!("markdown_html", a.markdown_html, b.markdown_html);
            field!("code_kind", a.code_kind, b.code_kind);
            field!("code_language", a.code_language, b.code_language);
            let containers = |attributes: &BlockAttributes| attributes.containers.iter()
                .map(|member| (member.container.kind.clone(), member.container.style.clone(), member.starts_here, member.ends_here))
                .collect::<Vec<_>>();
            field!("containers", containers(&a), containers(&b));
            if fields.is_empty() {
                fields.push(format!("attributes: {} vs {}", clipped(&a), clipped(&b)));
            }
            return Some(format!("block {index} differs (regional vs complete): {}", fields.join("; ")));
        }
        if left.len() != right.len() {
            return Some(format!("regional has {} blocks, complete has {}", left.len(), right.len()));
        }
        let (left, right) = (self.styles.items().collect::<Vec<_>>(), other.styles.items().collect::<Vec<_>>());
        if let Some((a, b)) = left.iter().zip(&right).find(|(a, b)| a != b) {
            return Some(format!("style span differs: regional {}, complete {}", clipped(a), clipped(b)));
        }
        if left.len() != right.len() {
            return Some(format!("regional has {} style spans, complete has {}", left.len(), right.len()));
        }
        let (left, right) = (self.hard_break_offsets(), other.hard_break_offsets());
        if left != right {
            let at = left.iter().zip(&right).position(|(a, b)| a != b).unwrap_or(left.len().min(right.len()));
            return Some(format!("hard breaks differ at index {at}: regional {:?}, complete {:?}",
                left.get(at), right.get(at)));
        }
        None
    }

    pub(crate) fn hard_break_offsets(&self) -> Vec<usize> {
        self.hard_lines
            .get_range(&(0..self.hard_lines.len()))
            .expect("the complete hard-line range is valid")
            .into_iter()
            .filter_map(|line| line.separator_range().map(|separator| separator.start))
            .collect()
    }

    pub(crate) fn has_same_hard_line_structure(&self, other: &Self) -> bool {
        if self.hard_line_count() != other.hard_line_count() {
            return false;
        }
        let indices = 0..self.hard_line_count();
        let left = self
            .hard_lines
            .get_range(&indices)
            .expect("the complete hard-line range is valid");
        let right = other
            .hard_lines
            .get_range(&indices)
            .expect("the complete hard-line range is valid");
        left.iter().zip(&right).all(|(left, right)| {
            left.range == right.range && left.separator_length == right.separator_length
        })
    }

    /// Blocks touching a formatted region, located in `O(log n + k)` time.
    /// Touching rather than strict overlap preserves empty-paragraph geometry
    /// and the established paragraph-boundary layout semantics.
    pub(crate) fn blocks_for_region(&self, range: &Range<usize>) -> Vec<Block> {
        self.blocks.query_touching(range)
    }

    /// Find the nearest code paragraph without traversing unrelated paragraphs.
    /// Front matter has a fixed language and never supplies a creation default.
    pub(crate) fn default_code_language(&self, at: usize) -> Option<String> {
        let index = self.blocks.partition_point_start(at);
        let prior = self.blocks.find_navigation(0..index, true, |summary| summary.flags & 4 != 0);
        let found = prior.or_else(|| self.blocks.find_navigation(index..self.blocks.len(), false, |summary| summary.flags & 4 != 0));
        found.and_then(|index| self.blocks.get(index)).and_then(|block| block.code_language.clone())
    }

    pub(super) fn list_indentation_blocks(&self) -> &OrderedRangeStore<Block> {
        &self.blocks
    }

    pub(super) fn heading_after(&self, index: usize) -> Option<(usize, Block)> {
        let blocks = self.list_indentation_blocks();
        let index = blocks.find_navigation(index..blocks.len(), false, |summary| summary.flags & 8 != 0)?;
        Some((index, blocks.get(index)?))
    }

    /// Style spans with a non-empty intersection with a formatted region,
    /// located in `O(log n + k)` time. The `k` returned records are materialized
    /// with snapshot-absolute ranges and remain ordered by normalized span
    /// start (and stable input order for equal starts).
    pub(crate) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub(crate) fn style_span_count(&self) -> usize {
        self.styles.len()
    }

    /// Style spans whose closed extent touches `range`, including empty ones.
    pub(crate) fn style_spans_touching(&self, range: &Range<usize>) -> Vec<StyleSpan> {
        self.styles.query_touching(range)
    }

    pub(crate) fn style_spans_for_region(&self, range: &Range<usize>) -> Vec<StyleSpan> {
        self.styles.query_overlapping(range)
    }

    pub(super) fn append_link_styles(&mut self, spans: Vec<StyleSpan>) {
        if spans.is_empty() { return; }
        let mut styles = self.styles.to_vec();
        styles.extend(spans);
        styles.sort_by_key(|span| span.range.start);
        self.styles = IntervalRangeStore::new(styles);
    }

    /// Every contributor in formatted order. This materializes the whole
    /// list; prefer region queries.
    pub fn provenance(&self) -> Vec<ProvenanceSpan> {
        self.provenance.spans(&self.text).collect()
    }

    pub(crate) fn provenance_for_region(&self, range: &Range<usize>) -> Vec<ProvenanceSpan> {
        self.provenance.query_overlapping(range, &self.text).into_iter()
            .map(|span| self.clip_literal_span(span, range)).collect()
    }

    pub(crate) fn provenance_touching(&self, range: &Range<usize>) -> Vec<ProvenanceSpan> {
        self.provenance.query_touching(range, &self.text).into_iter()
            .map(|span| self.clip_literal_span(span, range)).collect()
    }

    fn clip_literal_span(&self, span: ProvenanceSpan, range: &Range<usize>) -> ProvenanceSpan {
        if self.literal_encoding.is_none() { return span; }
        let start = span.formatted.start.max(range.start);
        let end = span.formatted.end.min(range.end);
        if let Some((source_start, source_end)) = self.literal_source_in_span(&span, start)
            .zip(self.literal_source_in_span(&span, end)) {
            ProvenanceSpan { formatted: start..end, source: source_start..source_end }
        } else { span }
    }

    /// A literal span's first UTF-16 offset and source bytes per UTF-16 code
    /// unit, when its encoded length agrees with its source range. Latin-1
    /// scalars are one byte and UTF-16 code units two, so boundaries map
    /// through the text tree's UTF-16 aggregates in logarithmic time instead
    /// of re-encoding the span for every query. UTF-8 maps bytes directly.
    fn literal_units(&self, span: &ProvenanceSpan) -> Option<(usize, usize)> {
        let width = match self.literal_encoding? {
            super::Encoding::Utf8 => return None,
            super::Encoding::Latin1 => 1,
            super::Encoding::Utf16Le | super::Encoding::Utf16Be => 2,
        };
        let first = self.text.utf16_offset_for_byte(span.formatted.start).ok()?;
        let last = self.text.utf16_offset_for_byte(span.formatted.end).ok()?;
        ((last - first).checked_mul(width)? == span.source.len()).then_some((first, width))
    }

    fn literal_source_in_span(&self, span: &ProvenanceSpan, at: usize) -> Option<usize> {
        if at == span.formatted.start { return Some(span.source.start); }
        if at == span.formatted.end { return Some(span.source.end); }
        if self.literal_encoding? == super::Encoding::Utf8 {
            // A UTF-8 span whose lengths differ holds undecodable bytes.
            return (span.formatted.len() == span.source.len() && span.formatted.contains(&at)
                && self.text.is_char_boundary(at).ok()?).then_some(span.source.start + at - span.formatted.start);
        }
        if !span.formatted.contains(&at) { return None; }
        let (first, width) = self.literal_units(span)?;
        let units = self.text.utf16_offset_for_byte(at).ok()?;
        Some(span.source.start + (units - first) * width)
    }

    fn literal_text_in_span(&self, span: &ProvenanceSpan, source: usize) -> Option<usize> {
        if source == span.source.start { return Some(span.formatted.start); }
        if source == span.source.end { return Some(span.formatted.end); }
        if self.literal_encoding? == super::Encoding::Utf8 {
            if span.formatted.len() != span.source.len() { return None; }
            let at = span.formatted.start.checked_add(source.checked_sub(span.source.start)?)?;
            return (source < span.source.end && self.text.is_char_boundary(at).ok()?).then_some(at);
        }
        if !span.source.contains(&source) { return None; }
        let (first, width) = self.literal_units(span)?;
        let offset = source - span.source.start;
        // An offset inside a code unit, or between a surrogate pair's units,
        // is not a scalar boundary.
        if offset % width != 0 { return None; }
        self.text.byte_offset_for_utf16(first + offset / width).ok()
    }

    fn literal_span_for_source(&self, source: usize) -> Option<ProvenanceSpan> {
        self.literal_encoding?;
        // Literal contributors are stored whole, so boundary items are points.
        let index = self.provenance.boundary_partition_start(source);
        let boundary = self.provenance.boundary_get(index).filter(|at| at.source.start == source)
            .or_else(|| index.checked_sub(1).and_then(|at| self.provenance.boundary_get(at)))?;
        let at = boundary.formatted.start;
        self.provenance.query_touching(&(at..at), &self.text).into_iter()
            .find(|span| span.source.start <= source && source <= span.source.end)
    }

    fn literal_source_unit(&self, span: &ProvenanceSpan, source: usize) -> ProvenanceSpan {
        let Some(encoding) = self.literal_encoding else { return span.clone(); };
        let Ok(text) = self.text.slice(span.formatted.clone()) else { return span.clone(); };
        if encoding.encoded_text_len(&text) != span.source.len() { return span.clone(); }
        let mut at = span.source.start;
        for (offset, ch) in text.char_indices() {
            let end = at + encoding.scalar_source_width(ch);
            if at < source && source < end {
                return ProvenanceSpan {
                    formatted: span.formatted.start + offset..span.formatted.start + offset + ch.len_utf8(),
                    source: at..end,
                };
            }
            at = end;
        }
        span.clone()
    }

    /// Expand a literal edit to existing bounded mapping-run boundaries. This
    /// projection window is independent of user grapheme selection boundaries.
    pub(crate) fn literal_projection_extent(&self, range: &Range<usize>) -> Option<(Range<usize>, Range<usize>)> {
        self.literal_encoding?;
        if range.start > range.end || range.end > self.text.byte_len() { return None; }
        if self.text.byte_len() == 0 {
            return Some((0..0, self.source_content_start..self.source_content_start));
        }
        let first = self.provenance.query_touching(&(range.start..range.start), &self.text).into_iter().next()?;
        let last = self.provenance.query_touching(&(range.end..range.end), &self.text).into_iter().last()?;
        Some((first.formatted.start..last.formatted.end, first.source.start..last.source.end))
    }

    /// Regional batches are applied from right to left. Assign their fresh
    /// paragraph identities in final document order, just as one projection
    /// does, while visiting only blocks touched by the inserted regions.
    pub(crate) fn order_new_literal_block_ids(
        &mut self, previous_length: usize, edits: &[TextEdit], first_id: u64,
    ) -> Result<(), BlockIdentityError> {
        if self.literal_encoding.is_none() || self.blocks.len() != self.hard_lines.len() {
            return Err(BlockIdentityError::InvalidProjection);
        }
        let mappings = build_edit_mappings(previous_length, self.text.byte_len(), edits)?;
        let mut fresh = std::collections::BTreeMap::new();
        for mapping in mappings {
            for block in self.blocks.query_touching(&mapping.new) {
                if block.id >= first_id {
                    let index = self.blocks.index_touching_point(block.range.start)
                        .ok_or(BlockIdentityError::InvalidProjection)?;
                    fresh.insert(index, block);
                }
            }
        }
        let mut stats = RangeSpliceStats::default();
        for (ordinal, (index, mut block)) in fresh.into_iter().enumerate() {
            let id = first_id.checked_add(ordinal as u64).ok_or(BlockIdentityError::Exhausted)?;
            if block.id == id { continue; }
            let mut line = self.hard_lines.get(index).ok_or(BlockIdentityError::InvalidProjection)?;
            if line.id != block.id || line.range != block.range {
                return Err(BlockIdentityError::InvalidProjection);
            }
            block.id = id;
            line.id = id;
            self.blocks = self.blocks.splice(index..index + 1, vec![block], 0, 0, &mut stats)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            self.hard_lines = self.hard_lines.splice(index..index + 1, vec![line], 0, 0, &mut stats)
                .ok_or(BlockIdentityError::InvalidProjection)?;
        }
        Ok(())
    }

    /// Source-contained contributors, including content reordered by a format
    /// parser. The source boundary index keeps this query local to the owner.
    pub(crate) fn provenance_contained_in_source(&self, range: &Range<usize>) -> Vec<ProvenanceSpan> {
        let mut result = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for boundary in self.provenance.boundaries_touching(range, &self.text) {
            for span in self.provenance_touching(&(boundary.formatted..boundary.formatted)) {
                if !span.formatted.is_empty() && !span.source.is_empty()
                    && range.start <= span.source.start && span.source.end <= range.end
                    && seen.insert((span.formatted.start, span.formatted.end, span.source.start, span.source.end))
                {
                    result.push(span);
                }
            }
        }
        result
    }

    /// Presentation recovery at the nearest real source boundary, in
    /// O(log n + k) for the boundaries sharing that exact source position.
    pub(crate) fn nearest_text_boundary_for_source(
        &self,
        source: usize,
        downstream: bool,
    ) -> Option<usize> {
        if self.literal_encoding.is_some() {
            if let Some(span) = self.literal_span_for_source(source) {
                if let Some(point) = self.literal_text_in_span(&span, source) { return Some(point); }
                // Presentation-only recovery from an interior encoding unit.
                let text = self.text.slice(span.formatted.clone()).ok()?;
                let encoding = self.literal_encoding?;
                if encoding.encoded_text_len(&text) == span.source.len() {
                    let mut at = span.source.start;
                    for (offset, ch) in text.char_indices() {
                        let end = at + encoding.scalar_source_width(ch);
                        if source < end {
                            return Some(span.formatted.start + offset + if downstream { ch.len_utf8() } else { 0 });
                        }
                        at = end;
                    }
                }
                return Some(if downstream { span.formatted.end } else { span.formatted.start });
            }
        }
        let following = self.provenance.following_boundary(source, &self.text);
        if following == Some(source) {
            let exact = self.provenance.boundaries_touching(&(source..source), &self.text);
            let side = if downstream {
                Side::Downstream
            } else {
                Side::Upstream
            };
            return exact
                .iter()
                .filter(|boundary| boundary.side == side)
                .min_by_key(|boundary| boundary.formatted)
                .or_else(|| exact.iter().min_by_key(|boundary| boundary.formatted))
                .map(|boundary| boundary.formatted);
        }
        let preceding = self.provenance.preceding_boundary(source, &self.text);
        let at = if downstream {
            following.or(preceding)
        } else {
            preceding.or(following)
        }?;
        let adjacent = self.provenance.boundaries_touching(&(at..at), &self.text);
        if at < source {
            adjacent.iter().map(|boundary| boundary.formatted).max()
        } else {
            adjacent.iter().map(|boundary| boundary.formatted).min()
        }
    }

    #[cfg(test)]
    pub(crate) fn source_boundary_query_work(&self, source: usize) -> (usize, usize) {
        self.provenance.boundary_query_work(&(source.saturating_sub(1)..source.saturating_add(1)))
    }

    /// Malformed source ranges represented by visible opaque replacement
    /// items in this exact formatted snapshot.
    pub fn decoding_diagnostics(&self) -> &[DecodingDiagnostic] {
        self.decoding_diagnostics.compatibility_slice()
    }

    pub(crate) fn has_decoding_diagnostic_overlapping(&self, range: &Range<usize>) -> bool {
        self.decoding_diagnostics
            .query_touching(range)
            .into_iter()
            .any(|diagnostic| {
                diagnostic.formatted_range.start < range.end
                    && range.start < diagnostic.formatted_range.end
            })
    }

    pub(crate) fn decoding_diagnostics_for_region(
        &self,
        range: &Range<usize>,
    ) -> Vec<DecodingDiagnostic> {
        self.decoding_diagnostics.query_touching(range)
    }

    pub fn style_sheet(&self) -> &StyleSheet {
        &self.style_sheet
    }

    pub(crate) fn layout_style_sheet(&self) -> &StyleSheet {
        self.layout_style_sheet_arc()
    }

    /// The shared layout sheet, whose identity can be retained and compared.
    pub(crate) fn layout_style_sheet_arc(&self) -> &Arc<StyleSheet> {
        self.markdown_code_presentation.as_ref().map_or(&self.style_sheet, |presentation| &presentation.sheet)
    }

    pub(crate) fn layout_style_spans_for_region(&self, range: &Range<usize>) -> Vec<StyleSpan> {
        let mut spans = self.style_spans_for_region(range);
        if let Some(presentation) = &self.markdown_code_presentation {
            spans.extend(presentation.spans.query_overlapping(range));
            spans.sort_by_key(|span| span.range.start);
        }
        spans
    }


    /// Install an already validated generated-configuration sheet on an
    /// otherwise unchanged projection candidate. The caller binds the
    /// candidate to the new document revision before atomic publication.
    pub(crate) fn reassign_deleted_style(&mut self, id: &StyleId, block: bool) {
        if block {
            let reassign = |block: &mut Block| {
                if &block.style == id { block.style = self.style_sheet.base_paragraph.clone(); }
                if block.containers.iter().any(|member| &member.container.style == id) {
                    block.containers = block.containers.iter().map(|member| {
                        let mut member = member.clone();
                        if &member.container.style == id {
                            let container = Arc::make_mut(&mut member.container);
                            let default = container.kind.default_style();
                            container.style = if &default != id && self.style_sheet.block_style(&default).is_some() { default } else { self.style_sheet.base_paragraph.clone() };
                        }
                        member
                    }).collect::<Vec<_>>().into();
                }
            };
            let mut blocks = self.blocks.to_vec();
            for block in &mut blocks { reassign(block); }
            self.blocks = OrderedRangeStore::new(blocks);

            if &self.document_style.style == id {
                self.document_style.style = self.style_sheet.base_paragraph.clone();
            }
        } else {
            let mut spans = self.styles.to_vec();
            spans.retain(|span| !matches!(&span.application, StyleApplication::Named(style) if style == id));
            self.styles = IntervalRangeStore::new(spans);
        }
    }

    pub(crate) fn install_configuration_styles(
        &mut self,
        revision: Revision,
        style_sheet: StyleSheet,
        document_style: DocumentStyleAssignment,
    ) {
        self.comment_context_cache = super::comment_context::prefix_cache(&self.comment_context_cache, usize::MAX);
        self.revision = revision;
        self.style_sheet = Arc::new(style_sheet);
        self.document_style = document_style;
    }

    pub(crate) fn install_code_styles(&mut self, sheet: Arc<StyleSheet>, runs: &[super::syntax::SyntaxRun]) {
        self.document_style = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        let names = code_style_names(&sheet);
        self.styles = IntervalRangeStore::new(
            runs.iter().filter_map(|run| self.automatic_span(&names, run)).collect(),
        );
        self.style_sheet = sheet;
    }

    pub(crate) fn install_markdown_code_styles(&mut self, sheet: Arc<StyleSheet>, runs: &[super::syntax::SyntaxRun]) {
        let spans = self.markdown_code_store(&sheet, runs);
        self.markdown_code_presentation = Some(Arc::new(MarkdownCodePresentation { sheet, spans }));
    }

    fn markdown_code_store(&self, sheet: &StyleSheet, runs: &[super::syntax::SyntaxRun]) -> IntervalRangeStore<StyleSpan> {
        let names = markdown_code_style_names(sheet);
        IntervalRangeStore::new(runs.iter().filter_map(|run| self.automatic_span(&names, run)).collect())
    }

    /// Whether the installed Markdown code spans equal a complete
    /// installation from `runs`. Shadow verification for incremental
    /// publication.
    #[cfg(debug_assertions)]
    pub(crate) fn markdown_code_styles_match(&self, sheet: &StyleSheet, runs: &[super::syntax::SyntaxRun]) -> bool {
        self.markdown_code_presentation.as_ref()
            .is_some_and(|presentation| presentation.spans == self.markdown_code_store(sheet, runs))
    }

    /// Markdown analog of [`Self::install_code_styles_incrementally`]: splice
    /// `previous`'s code-block spans across the text edit hull, shifting the
    /// rest lazily, then replace the spans of each `replaced` block region
    /// with the spans of its current runs. Returns false when the previous
    /// spans cannot be reused, in which case nothing changes.
    pub(crate) fn install_markdown_code_styles_incrementally(
        &mut self,
        sheet: Arc<StyleSheet>,
        previous: &FormattedDocument,
        hull: Option<&(Range<usize>, Range<usize>)>,
        replaced: &[Range<usize>],
        runs_in: &dyn Fn(&Range<usize>) -> Vec<super::syntax::SyntaxRun>,
    ) -> bool {
        let Some(old) = previous.markdown_code_presentation.as_ref()
            .filter(|presentation| Arc::ptr_eq(&presentation.sheet, &sheet)) else { return false };
        let old_len = previous.text.byte_len();
        let new_len = self.text.byte_len();
        match hull {
            Some((old_hull, new_hull)) => {
                if old_hull.start != new_hull.start
                    || old_hull.end > old_len
                    || new_hull.end > new_len
                    || old_len as i128 - old_hull.end as i128 != new_len as i128 - new_hull.end as i128
                {
                    return false;
                }
            }
            None if old_len != new_len => return false,
            None => {}
        }
        let mut spans = old.spans.clone();
        let mut stats = RangeSpliceStats::default();
        if let Some((old_hull, new_hull)) = hull {
            let indices = spans.overlapping_index_span(old_hull);
            let Some(items) = spans.get_range(&indices) else { return false };
            let kept = items.into_iter().filter(|span| span.range.end <= old_hull.start).collect();
            let Some(next) = spans.splice(indices, kept, old_hull.end, new_hull.end, &mut stats) else {
                return false;
            };
            spans = next;
        }
        let names = markdown_code_style_names(&sheet);
        for region in replaced {
            if region.end > new_len {
                return false;
            }
            let indices = spans.overlapping_index_span(region);
            let Some(items) = spans.get_range(&indices) else { return false };
            let mut replacement = Vec::with_capacity(items.len());
            for span in &items {
                if span.range.start >= region.end || span.range.end <= region.start {
                    replacement.push(span.clone());
                    continue;
                }
                if span.range.start < region.start {
                    replacement.push(span.with_range(span.range.start..region.start));
                }
                if span.range.end > region.end {
                    replacement.push(span.with_range(region.end..span.range.end));
                }
            }
            replacement.extend(runs_in(region).iter().filter_map(|run| self.automatic_span(&names, run))
                .filter(|span| span.range.start < region.end && region.start < span.range.end));
            let Some(next) = spans.splice(indices, replacement, 0, 0, &mut stats) else { return false };
            spans = next;
        }
        self.markdown_code_presentation = Some(Arc::new(MarkdownCodePresentation { sheet, spans }));
        true
    }

    /// Install a publication as a splice of the previous presentation's
    /// automatic spans: drop the spans inside the edit hull and shift the rest
    /// lazily, then replace the spans of each region whose runs changed.
    /// `runs_in` supplies the current runs overlapping a replaced region.
    /// Returns false when the previous spans cannot be reused, in which case
    /// nothing changes and the caller installs from every run.
    pub(crate) fn install_code_styles_incrementally(
        &mut self,
        sheet: Arc<StyleSheet>,
        previous: &FormattedDocument,
        hull: Option<&(Range<usize>, Range<usize>)>,
        replaced: &[Range<usize>],
        runs_in: &dyn Fn(&Range<usize>) -> Vec<super::syntax::SyntaxRun>,
    ) -> bool {
        // Code has no source-backed spans; anything else is not a Code base.
        if !self.styles.is_empty() || previous.style_sheet.revision != sheet.revision {
            return false;
        }
        let old_len = previous.text.byte_len();
        let new_len = self.text.byte_len();
        match hull {
            Some((old_hull, new_hull)) => {
                if old_hull.start != new_hull.start
                    || old_hull.end > old_len
                    || new_hull.end > new_len
                    || old_len as i128 - old_hull.end as i128 != new_len as i128 - new_hull.end as i128
                {
                    return false;
                }
            }
            None if old_len != new_len => return false,
            None => {}
        }
        let mut styles = previous.styles.clone();
        let mut stats = RangeSpliceStats::default();
        if let Some((old_hull, new_hull)) = hull {
            let indices = styles.overlapping_index_span(old_hull);
            let Some(items) = styles.get_range(&indices) else { return false };
            let kept = items.into_iter().filter(|span| span.range.end <= old_hull.start).collect();
            let Some(next) = styles.splice(indices, kept, old_hull.end, new_hull.end, &mut stats) else {
                return false;
            };
            styles = next;
        }
        let names = code_style_names(&sheet);
        for region in replaced {
            if region.end > new_len {
                return false;
            }
            let indices = styles.overlapping_index_span(region);
            let Some(items) = styles.get_range(&indices) else { return false };
            let mut replacement = Vec::with_capacity(items.len());
            for span in &items {
                if span.range.start >= region.end || span.range.end <= region.start {
                    replacement.push(span.clone());
                    continue;
                }
                if span.range.start < region.start {
                    replacement.push(span.with_range(span.range.start..region.start));
                }
                if span.range.end > region.end {
                    replacement.push(span.with_range(region.end..span.range.end));
                }
            }
            replacement.extend(runs_in(region).iter().filter_map(|run| self.automatic_span(&names, run))
                .filter(|span| span.range.start < region.end && region.start < span.range.end));
            let Some(next) = styles.splice(indices, replacement, 0, 0, &mut stats) else { return false };
            styles = next;
        }
        self.document_style = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        self.styles = styles;
        self.style_sheet = sheet;
        true
    }

    /// The automatic span one syntax run contributes to this projection's
    /// text, or none when it is empty or begins past the text.
    fn automatic_span(
        &self,
        names: &std::collections::BTreeMap<&str, &StyleId>,
        run: &super::syntax::SyntaxRun,
    ) -> Option<StyleSpan> {
        if run.range.start >= run.range.end || run.range.end > self.text.byte_len() { return None; }
        let ceil = |at| {
            // Two adjacent ASCII bytes other than CR LF always form a
            // grapheme boundary, so most run edges need no cluster search.
            let ascii_pair = at > 0 && at < self.text.byte_len() && {
                let before = self.text.byte_chunk_at(at - 1);
                let after = self.text.byte_chunk_at(at);
                before.first().is_some_and(|b| b.is_ascii() && *b != b'\r')
                    && after.first().is_some_and(|b| b.is_ascii())
            };
            if ascii_pair || self.is_logical_grapheme_boundary(at).ok()? { Some(at) }
            else { self.next_logical_grapheme_boundary(at).ok()? }
        };
        // A cluster's first character owns its appearance. A later
        // capture beginning inside it starts at the following cluster.
        let range = ceil(run.range.start)?..ceil(run.range.end)?;
        if range.is_empty() { return None; }
        // A name without a definition uses its nearest defined dotted
        // ancestor, the appearance its implicit definition will have.
        let mut name = run.name.as_str();
        let id = loop {
            if let Some(id) = names.get(name) { break id; }
            name = name.rsplit_once('.')?.0;
        };
        Some(StyleSpan { range, application: StyleApplication::Automatic((*id).clone()) })
    }

    pub(crate) fn has_block_style_assignment(&self, style: &StyleId) -> bool {
        (style.0 == "Image" && !self.inline_images.is_empty()) || self.document_style.style == *style || self.blocks.items().any(|block| block.style == *style || block.containers.iter().any(|member| member.container.style == *style))
    }

    pub(crate) fn has_character_style_assignment(&self, style: &StyleId) -> bool {
        self.styles
            .items()
            .any(|span| matches!(&span.application, StyleApplication::Named(id) if id == style))
    }

    /// Current formatted ranges whose resolved result depends on one of the
    /// supplied block or character style definitions. Ranges are normalized
    /// and adjacent ranges are coalesced because this summary describes cache
    /// invalidation, not selection row identity.
    pub(crate) fn style_dependency_ranges(
        &self,
        block_styles: &std::collections::BTreeSet<StyleId>,
        character_styles: &std::collections::BTreeSet<StyleId>,
    ) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        if block_styles.contains(&self.document_style.style) {
            ranges.push(0..self.text.byte_len());
        }
        if block_styles.contains(&StyleId::from("Image")) {
            ranges.extend(self.inline_images.items().map(|image| image.range));
        }
        ranges.extend(
            self.blocks
                .items()
                .filter(|block| block_styles.contains(&block.style) || block.containers.iter().any(|member| block_styles.contains(&member.container.style)))
                .map(|block| block.range.clone()),
        );
        ranges.extend(
            self.styles
                .items()
                .filter_map(|span| match &span.application {
                    StyleApplication::Named(id) | StyleApplication::Automatic(id) => {
                        character_styles.contains(id).then(|| span.range.clone())
                    }

                    _ => None,
                }),
        );
        ranges.sort_by_key(|range| (range.start, range.end));
        let mut normalized: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = normalized.last_mut() {
                if range.start <= previous.end {
                    previous.end = previous.end.max(range.end);
                    continue;
                }
            }
            normalized.push(range);
        }
        normalized
    }

    /// The normalized document-root style assignment for this projection.
    pub fn document_style(&self) -> &DocumentStyleAssignment {
        &self.document_style
    }

    pub(crate) fn has_monotonic_grapheme_provenance(&self) -> bool {
        // Scalars inside a divisible run are monotonic and gap-free, so the
        // stored contributors decide this exactly.
        let mut previous: Option<ProvenanceRun> = None;
        self.provenance.stored().all(|span| {
            let ordered = (!span.source.is_empty() || span.formatted.is_empty())
                && previous.as_ref().is_none_or(|before| before.source.end <= span.source.start
                    && before.formatted.end <= span.formatted.start);
            previous = Some(span);
            ordered
        })
    }

    /// Bounded lookup bands for explicitly relocated source intervals. These
    /// bands only locate contributors; their exact source mapping decides
    /// whether their identity can be retained. Whole graphemes at an edge may
    /// therefore be inspected without expanding the declared source edit.
    pub(crate) fn source_grapheme_lookup_ranges(
        &self,
        source_ranges: impl IntoIterator<Item = Range<usize>>,
    ) -> Result<Vec<Range<usize>>, FormattedTextError> {
        let mut ranges = Vec::new();
        for source in source_ranges {
            if source.is_empty() { continue; }
            // A parser may reorder visible children. Query every local source
            // contributor rather than assuming endpoint order describes all
            // of them; compact/partial runs still need the nearest-edge band.
            let contained = self.provenance_contained_in_source(&source);
            if contained.is_empty() {
                let Some(start) = self.nearest_text_boundary_for_source(source.start, true) else { continue; };
                let Some(end) = self.nearest_text_boundary_for_source(source.end, false) else { continue; };
                ranges.push(start.min(end)..start.max(end));
            } else {
                ranges.extend(contained.into_iter().map(|span| span.formatted));
            }
        }
        self.grapheme_lookup_ranges(ranges)
    }

    /// Merge local contributor inspection bands and include a whole logical
    /// grapheme at either edge. This does not widen a source edit or selection.
    pub(crate) fn grapheme_lookup_ranges(
        &self,
        ranges: impl IntoIterator<Item = Range<usize>>,
    ) -> Result<Vec<Range<usize>>, FormattedTextError> {
        let mut ranges: Vec<_> = ranges.into_iter().collect();
        for range in &mut ranges {
            if range.start > range.end || range.end > self.text.byte_len() {
                return Err(FormattedTextError::InvalidRange {
                    start: range.start, end: range.end, length: self.text.byte_len(),
                });
            }
            if !self.is_logical_grapheme_boundary(range.start)? {
                range.start = self.previous_logical_grapheme_boundary(range.start)?.unwrap_or(0);
            }
            if !self.is_logical_grapheme_boundary(range.end)? {
                range.end = self.next_logical_grapheme_boundary(range.end)?.unwrap_or(self.text.byte_len());
            }
        }
        ranges.retain(|range| !range.is_empty());
        ranges.sort_by_key(|range| (range.start, range.end));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = merged.last_mut().filter(|previous| range.start <= previous.end) {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        Ok(merged)
    }

    /// Relocation counterpart of `source_range`, restricted to a whole logical
    /// grapheme band. It reads canonical rope slices and indexed provenance,
    /// leaving compatibility text and unrelated provenance unmaterialized.
    pub(crate) fn source_grapheme_ranges_in(
        &self,
        range: &Range<usize>,
    ) -> Result<Vec<(usize, String, Option<Range<usize>>)>, FormattedTextError> {
        if range.start > range.end || range.end > self.text.byte_len() {
            return Err(FormattedTextError::InvalidRange {
                start: range.start, end: range.end, length: self.text.byte_len(),
            });
        }
        for at in [range.start, range.end] {
            if !self.is_logical_grapheme_boundary(at)? {
                return Err(FormattedTextError::NotGraphemeBoundary(at));
            }
        }
        let mut result = Vec::new();
        let mut at = range.start;
        while at < range.end {
            let end = self.next_logical_grapheme_boundary(at)?
                .ok_or(FormattedTextError::UnicodeBoundaryResolutionFailed)?;
            if end > range.end { return Err(FormattedTextError::NotGraphemeBoundary(range.end)); }
            let item = self.text.slice(at..end)?;
            result.push((at, item, self.source_range(at..end)));
            at = end;
        }
        super::work_statistics::record(|stats| stats.source_relocation_graphemes_visited += result.len());
        Ok(result)
    }

    /// Batch counterpart of `source_range` for a complete reprojection diff.
    /// Adjacent graphemes share an ordered provenance frontier, avoiding four
    /// independent interval-tree queries and temporary vectors per item.
    pub(crate) fn source_grapheme_ranges(
        &self,
    ) -> impl Iterator<Item = (usize, &str, Option<Range<usize>>)> {
        // Stream the contributors twice, for their starts and their ends,
        // rather than materializing one record per scalar.
        let mut previous_end = None;
        let ordered_ends = self.provenance.spans(&self.text).all(|span| {
            let ordered = previous_end.is_none_or(|end| end <= span.formatted.end);
            previous_end = Some(span.formatted.end);
            ordered
        });
        let has_spans = !self.provenance.is_empty();
        let mut starts = self.provenance.spans(&self.text).peekable();
        let mut ends = self.provenance.spans(&self.text).peekable();
        let mut ended: Option<ProvenanceSpan> = None;
        let mut boundary = move |at: usize| {
            while starts.next_if(|span| span.formatted.start < at).is_some() {}
            while let Some(span) = ends.next_if(|span| span.formatted.end <= at) {
                ended = Some(span);
            }
            let following = starts
                .peek()
                .filter(|span| span.formatted.start == at)
                .map(|span| span.source.start);
            let preceding = ended
                .as_ref()
                .filter(|span| span.formatted.end == at)
                .map(|span| span.source.end);
            (following, preceding)
        };
        // Literal projections map each boundary through the first touching
        // contributor, independently of side. Keep the touching contributors
        // as an ordered window and reuse a grapheme's end for the next start,
        // rather than querying the interval tree twice per grapheme.
        let literal_streaming = self.literal_encoding.is_some() && ordered_ends && has_spans;
        let mut upcoming = self.provenance.spans(&self.text).peekable();
        let mut window = std::collections::VecDeque::<ProvenanceSpan>::new();
        let mut last_boundary: Option<(usize, Option<usize>)> = None;
        let mut literal_boundary = move |at: usize| -> Option<usize> {
            if let Some((cached, value)) = last_boundary {
                if cached == at {
                    return value;
                }
            }
            while let Some(span) = upcoming.next_if(|span| span.formatted.start <= at) {
                window.push_back(span);
            }
            while window.front().is_some_and(|span| span.formatted.end < at) {
                window.pop_front();
            }
            let value = window
                .iter()
                .filter(|span| span.formatted.start <= at && at <= span.formatted.end)
                .find_map(|span| self.literal_source_in_span(span, at));
            last_boundary = Some((at, value));
            value
        };
        let text = self.text();
        let mut at = 0;
        // ASCII pairs cannot join across a grapheme boundary except CRLF.
        // Ask the Unicode segmenter at every non-ASCII adjacency, preserving
        // combining sequences, emoji ZWJ chains, and regional-indicator pairs.
        let graphemes = std::iter::from_fn(move || {
            if at == text.len() {
                return None;
            }
            let offset = at;
            let bytes = text.as_bytes();
            let length = if bytes[at].is_ascii() && bytes.get(at + 1).map_or(true, u8::is_ascii) {
                if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                }
            } else {
                text[at..].graphemes(true).next().unwrap().len()
            };
            at += length;
            Some((offset, &text[offset..at]))
        });
        graphemes.map(move |(offset, item)| {
            let source = if self.literal_encoding.is_none() && ordered_ends && has_spans {
                let (following, preceding) = boundary(offset);
                let start = following
                    .or(preceding)
                    .or_else(|| (offset == 0).then_some(self.source_content_start));
                let (following, preceding) = boundary(offset + item.len());
                let end = preceding.or(following).or_else(|| {
                    (offset + item.len() == text.len()).then_some(self.source_content_end)
                });
                start
                    .zip(end)
                    .and_then(|(start, end)| (start <= end).then_some(start..end))
            } else if literal_streaming {
                let range = literal_boundary(offset)
                    .zip(literal_boundary(offset + item.len()))
                    .and_then(|(start, end)| (start <= end).then_some(start..end));
                debug_assert_eq!(range, self.source_range(offset..offset + item.len()));
                range
            } else {
                self.source_range(offset..offset + item.len())
            };
            (offset, item, source)
        })
    }

    pub(crate) fn source_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start > range.end || range.end > self.text.byte_len() {
            return None;
        }
        if range.is_empty() {
            if self.literal_encoding.is_some() {
                return self.source_boundary(range.start, Side::Downstream).map(|at| at..at);
            }
            return super::source_edit::insertion_point(self, range.start, None).map(|at| at..at);
        }
        let start = self.source_boundary(range.start, Side::Downstream)?;
        let end = self.source_boundary(range.end, Side::Upstream)?;
        (start <= end).then_some(start..end)
    }

    /// Return the ordered source ranges which contribute visible content to a
    /// non-empty formatted range contained by one hard line. Hidden syntax
    /// between those ranges is deliberately excluded.
    ///
    /// Markdown text replacement uses this relational view instead of the
    /// contiguous source hull returned by [`Self::source_range`]. Replacing
    /// that hull would consume inline delimiters at adjacent style boundaries
    /// (for example the `**_` between `**ab**_cd_`). Keeping the ranges
    /// discontiguous lets a transaction replace the first visible run, delete
    /// the remaining selected runs, and leave every delimiter byte untouched.
    ///
    /// The initial adapters produce monotonic, non-overlapping scalar
    /// provenance. A future transform with a genuinely relational or
    /// non-monotonic mapping gets `None` here and must provide its own typed
    /// reverse rule rather than being flattened into an unsafe patch.
    pub(crate) fn line_local_visible_source_runs(
        &self,
        range: Range<usize>,
    ) -> Option<Vec<VisibleSourceRun>> {
        if range.is_empty() || range.end > self.text.byte_len() {
            return None;
        }
        if self.hard_line_at_offset(range.start)? != self.hard_line_at_offset(range.end)? {
            return None;
        }

        super::source_edit::visible_runs(self, &range).ok()
    }

    /// Whether replacement text inserted at the downstream side of this
    /// range begins inside a Markdown code span. This is intentionally based
    /// on the first selected item (or following item for an insertion), not on
    /// whether one code span contains the entire replacement range.
    pub(crate) fn markdown_replacement_begins_in_code(&self, range: &Range<usize>) -> bool {
        let at = range.start;
        if self.blocks_for_region(range).iter().any(|block| {
            block.style.0 == "Code Block" && block.range.start <= at && range.end <= block.range.end
        }) {
            return true;
        }
        self.styles.query_touching(&(at..at)).iter().any(|span| {
            span.range.start <= at
                && at < span.range.end
                && span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
        })
    }

    pub(crate) fn source_insertion_point(&self, at: usize, downstream: bool) -> Option<usize> {
        self.source_boundary(
            at,
            if downstream {
                Side::Downstream
            } else {
                Side::Upstream
            },
        )
    }

    fn source_boundary(&self, at: usize, side: Side) -> Option<usize> {
        if at > self.text.byte_len() || !self.text.is_char_boundary(at).ok()? {
            return None;
        }
        if self.provenance.is_empty() {
            return Some(self.source_content_start);
        }

        // At the outer document boundaries, downstream means outside trailing
        // syntax and upstream means outside leading syntax. At interior split
        // boundaries the adjacent provenance segments decide the side.
        if at == 0 && matches!(side, Side::Upstream) {
            return Some(self.source_content_start);
        }
        if at == self.text.byte_len() && matches!(side, Side::Downstream) {
            return Some(self.source_insertion_end);
        }

        let adjacent = self.provenance.query_touching(&(at..at), &self.text);
        if self.literal_encoding.is_some() {
            return adjacent.iter().find_map(|span| self.literal_source_in_span(span, at));
        }
        let preceding = adjacent
            .iter()
            .rev()
            .find(|span| span.formatted.end == at)
            .map(|span| span.source.end);
        let following = adjacent
            .iter()
            .find(|span| span.formatted.start == at)
            .map(|span| span.source.start);
        match side {
            Side::Upstream => preceding.or(following).or_else(|| {
                (at == 0)
                    .then_some(self.source_content_start)
                    .or((at == self.text.byte_len()).then_some(self.source_content_end))
            }),
            Side::Downstream => following.or(preceding).or_else(|| {
                (at == 0)
                    .then_some(self.source_content_start)
                    .or((at == self.text.byte_len()).then_some(self.source_content_end))
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Upstream,
    Downstream,
}

fn stable_text_fingerprint(text: &str) -> u64 {
    // FNV-1a is sufficient here: the stable block ID and byte length are also
    // checked, and this fingerprint is a stale-anchor guard rather than a
    // persistence or security digest.
    text.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[derive(Clone, Copy)]
enum BlockWitnessSide {
    Containing,
    PrecedingBreak,
    FollowingBreak,
}

#[derive(Clone, Debug)]
struct EditMapping {
    old: Range<usize>,
    new: Range<usize>,
}

fn allocate_unassigned_block_ids(
    blocks: &mut [Block],
    next_id: u64,
) -> Result<u64, BlockIdentityError> {
    if next_id == 0 {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let needed = blocks.iter().filter(|block| block.id == 0).count();
    let needed = u64::try_from(needed).map_err(|_| BlockIdentityError::Exhausted)?;
    let next_after = next_id
        .checked_add(needed)
        .ok_or(BlockIdentityError::Exhausted)?;
    let mut allocated = next_id;
    for block in blocks.iter_mut().filter(|block| block.id == 0) {
        block.id = allocated;
        allocated = allocated
            .checked_add(1)
            .ok_or(BlockIdentityError::Exhausted)?;
    }
    debug_assert_eq!(allocated, next_after);
    Ok(next_after)
}

/// Transitional seed used by the two current adapters, whose paragraph and
/// hard-line partitions coincide. The persistent hard-line index itself does
/// not depend on this equivalence; adapters with nested or multiline blocks
/// must provide their own explicit hard-line records.
fn seed_hard_lines_from_current_blocks(
    blocks: &[Block],
    text: &str,
) -> Result<Vec<HardLine>, BlockIdentityError> {
    validate_block_partition(text, blocks)?;
    Ok(blocks
        .iter()
        .map(|block| HardLine {
            id: block.id,
            range: block.range.clone(),
            separator_length: usize::from(block.range.end < text.len()),
        })
        .collect())
}

fn validate_block_partition(text: &str, blocks: &[Block]) -> Result<(), BlockIdentityError> {
    if blocks.is_empty() {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let mut expected_start = 0;
    for (index, block) in blocks.iter().enumerate() {
        if block.range.start != expected_start
            || block.range.start > block.range.end
            || block.range.end > text.len()
            || !text.is_char_boundary(block.range.start)
            || !text.is_char_boundary(block.range.end)
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        if block.range.end == text.len() {
            if index + 1 != blocks.len() {
                return Err(BlockIdentityError::InvalidProjection);
            }
            expected_start = block.range.end;
        } else {
            if text.as_bytes()[block.range.end] != b'\n' {
                return Err(BlockIdentityError::InvalidProjection);
            }
            expected_start = block
                .range
                .end
                .checked_add(1)
                .ok_or(BlockIdentityError::InvalidProjection)?;
        }
    }
    if expected_start != text.len() {
        return Err(BlockIdentityError::InvalidProjection);
    }
    Ok(())
}

fn build_edit_mappings(
    old_len: usize,
    new_len: usize,
    edits: &[TextEdit],
) -> Result<Vec<EditMapping>, BlockIdentityError> {
    let mut mappings = Vec::with_capacity(edits.len());
    let mut old_cursor: usize = 0;
    let mut new_cursor: usize = 0;
    for edit in edits {
        if edit.range.start < old_cursor
            || edit.range.start > edit.range.end
            || edit.range.end > old_len
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        let unchanged = edit
            .range
            .start
            .checked_sub(old_cursor)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        new_cursor = new_cursor
            .checked_add(unchanged)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let new_start = new_cursor;
        new_cursor = new_cursor
            .checked_add(edit.replacement.len())
            .ok_or(BlockIdentityError::InvalidProjection)?;
        mappings.push(EditMapping {
            old: edit.range.clone(),
            new: new_start..new_cursor,
        });
        old_cursor = edit.range.end;
    }
    new_cursor = new_cursor
        .checked_add(
            old_len
                .checked_sub(old_cursor)
                .ok_or(BlockIdentityError::InvalidProjection)?,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;
    if new_cursor != new_len {
        return Err(BlockIdentityError::InvalidProjection);
    }
    Ok(mappings)
}

/// Carry sparse direct declarations to every child created by splitting one
/// old paragraph. Named styles remain projector-owned because a child may have
/// distinct source-derived semantics (for example, Markdown Heading followed
/// by Paragraph).
fn inherit_split_direct_assignments(
    new_text: &str,
    new_blocks: &mut [Block],
    old: &Block,
    edits: &[EditMapping],
) -> Result<(), BlockIdentityError> {
    if old.direct_paragraph == BlockProperties::default()
        && old.direct_default_character == CharacterProperties::default()
    {
        return Ok(());
    }
    let first = edits.partition_point(|edit| edit.old.end < old.range.start);
    let last = edits.partition_point(|edit| edit.old.start <= old.range.end);
    let local_edits = &edits[first..last];
    let split_inside_old = local_edits.iter().any(|edit| {
        old.range.start <= edit.old.start
            && edit.old.end <= old.range.end
            && new_text.as_bytes()[edit.new.clone()].contains(&b'\n')
    });
    if !split_inside_old {
        return Ok(());
    }

    // An edit crossing a paragraph boundary does not have one unambiguous
    // parent assignment. The normal retained-ID rule still handles the
    // surviving block; only a self-contained split fans declarations out.
    if local_edits.iter().any(|edit| {
        (edit.old.start < old.range.start && old.range.start < edit.old.end)
            || (edit.old.start < old.range.end && old.range.end < edit.old.end)
    }) {
        return Ok(());
    }

    let mapped_start = map_old_boundary_before(old.range.start, edits)?;
    let mut mapped_end = map_old_boundary_before(old.range.end, edits)?;
    for edit in local_edits
        .iter()
        .filter(|edit| edit.old.is_empty() && edit.old.start == old.range.end)
    {
        mapped_end = mapped_end.max(edit.new.end);
    }
    if mapped_start > mapped_end || mapped_end > new_text.len() {
        return Err(BlockIdentityError::InvalidProjection);
    }

    let first = new_blocks.partition_point(|candidate| candidate.range.start < mapped_start);
    let last = new_blocks.partition_point(|candidate| candidate.range.start <= mapped_end);
    for candidate in new_blocks[first..last]
        .iter_mut()
        .filter(|candidate| candidate.range.end <= mapped_end)
    {
        candidate.direct_formatting = old.direct_formatting.clone();
    }
    Ok(())
}

fn map_old_boundary_before(
    old_offset: usize,
    edits: &[EditMapping],
) -> Result<usize, BlockIdentityError> {
    let index = edits.partition_point(|edit| edit.old.end < old_offset);
    if let Some(edit) = edits.get(index).filter(|edit| edit.old.start <= old_offset) {
        if old_offset == edit.old.start {
            return Ok(edit.new.start);
        }
        if old_offset == edit.old.end {
            return Ok(edit.new.end);
        }
        return Err(BlockIdentityError::InvalidProjection);
    }
    match index.checked_sub(1).and_then(|index| edits.get(index)) {
        Some(edit) => edit
            .new
            .end
            .checked_add(
                old_offset
                    .checked_sub(edit.old.end)
                    .ok_or(BlockIdentityError::InvalidProjection)?,
            )
            .ok_or(BlockIdentityError::InvalidProjection),
        None => Ok(old_offset),
    }
}

fn block_identity_witness(
    text: &str,
    block: &Block,
    edits: &[EditMapping],
) -> Result<Option<usize>, BlockIdentityError> {
    if let Some(content) = first_surviving_byte(&block.range, edits) {
        return Ok(Some(content));
    }
    let delimiter = if block.range.end < text.len() && text.as_bytes()[block.range.end] == b'\n' {
        block.range.end
            ..block
                .range
                .end
                .checked_add(1)
                .ok_or(BlockIdentityError::InvalidProjection)?
    } else if block.range.start > 0 && text.as_bytes()[block.range.start - 1] == b'\n' {
        block.range.start - 1..block.range.start
    } else {
        block.range.start..block.range.start
    };
    Ok(first_surviving_byte(&delimiter, edits))
}

fn first_surviving_byte(extent: &Range<usize>, edits: &[EditMapping]) -> Option<usize> {
    if extent.is_empty() {
        return None;
    }
    let mut cursor = extent.start;
    let first = edits.partition_point(|edit| edit.old.end <= cursor);
    for edit in &edits[first..] {
        if edit.old.start >= extent.end {
            break;
        }
        if edit.old.is_empty() || edit.old.end <= cursor {
            continue;
        }
        if edit.old.start > cursor {
            return Some(cursor);
        }
        cursor = cursor.max(edit.old.end);
        if cursor >= extent.end {
            return None;
        }
    }
    Some(cursor)
}

fn deleted_empty_join_boundary(text: &str, block: &Block, edits: &[EditMapping]) -> Option<usize> {
    if !block.range.is_empty()
        || block.range.end >= text.len()
        || text.as_bytes()[block.range.end] != b'\n'
    {
        return None;
    }
    edits
        .iter()
        .find(|edit| {
            !edit.old.is_empty()
                && edit.old.start <= block.range.end
                && block.range.end < edit.old.end
        })
        .map(|edit| edit.new.start)
}

fn map_surviving_byte(
    old_offset: usize,
    edits: &[EditMapping],
) -> Result<usize, BlockIdentityError> {
    let completed = edits.partition_point(|edit| edit.old.end <= old_offset);
    let Some(edit) = completed.checked_sub(1).and_then(|index| edits.get(index)) else {
        return Ok(old_offset);
    };
    edit.new
        .end
        .checked_add(
            old_offset
                .checked_sub(edit.old.end)
                .ok_or(BlockIdentityError::InvalidProjection)?,
        )
        .ok_or(BlockIdentityError::InvalidProjection)
}

fn witness_side(text: &str, block: &Block, offset: usize) -> BlockWitnessSide {
    if offset == block.range.end && offset < text.len() && text.as_bytes()[offset] == b'\n' {
        BlockWitnessSide::PrecedingBreak
    } else if offset
        .checked_add(1)
        .is_some_and(|after| after == block.range.start)
        && text.as_bytes().get(offset) == Some(&b'\n')
    {
        BlockWitnessSide::FollowingBreak
    } else {
        BlockWitnessSide::Containing
    }
}

fn block_index_for_witness(
    text: &str,
    blocks: &[Block],
    mapped_offset: usize,
    side: BlockWitnessSide,
) -> Result<usize, BlockIdentityError> {
    let probe = match side {
        BlockWitnessSide::Containing | BlockWitnessSide::PrecedingBreak => mapped_offset,
        BlockWitnessSide::FollowingBreak => mapped_offset
            .checked_add(1)
            .ok_or(BlockIdentityError::InvalidProjection)?,
    };
    if probe > text.len() {
        return Err(BlockIdentityError::InvalidProjection);
    }
    blocks
        .partition_point(|block| block.range.start <= probe)
        .checked_sub(1)
        .ok_or(BlockIdentityError::InvalidProjection)
}

fn unique_candidate(
    source_offset: usize,
    candidates: &[usize],
) -> Result<Option<usize>, SourceToTextError> {
    match candidates {
        [] => Ok(None),
        [candidate] => Ok(Some(*candidate)),
        _ => Err(SourceToTextError::AmbiguousBoundary {
            source_offset,
            candidates: candidates.to_vec(),
        }),
    }
}

pub(crate) fn project(
    normalized: &NormalizedText,
    format: Format,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
) -> FormattedDocument {
    project_with_start(normalized, format, revision, source_content_start, source_content_end, true)
}

/// Project a band of complete source lines beginning at `source_content_start`.
/// A band inside a document can follow earlier blocks, so its leading blank
/// lines are not surplus document-start separators; a band at the document's
/// own content start is. Callers still verify the band against the document's
/// own projection.
pub(crate) fn project_band(
    normalized: &NormalizedText,
    format: Format,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    document_start: bool,
) -> FormattedDocument {
    project_with_start(normalized, format, revision, source_content_start, source_content_end, document_start)
}

fn project_with_start(
    normalized: &NormalizedText,
    format: Format,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    document_start: bool,
) -> FormattedDocument {
    match format {
        Format::PlainText => project_plain(
            normalized,
            revision,
            source_content_start,
            source_content_end,
        ),
        Format::Code => {
            let mut projection = project_plain(normalized, revision, source_content_start, source_content_end);
            projection.install_code_styles(super::code_style::snapshot(), &[]);
            projection
        }
        Format::Markdown => project_markdown(
            normalized,
            revision,
            source_content_start,
            source_content_end,
            document_start,
        ),
    }
}

/// Merge a freshly projected, delimiter-bounded hard-line region into an
/// existing formatted snapshot. The current plain-text and Markdown adapters
/// are line-local, so a region containing complete source lines is a semantic
/// restart boundary. Callers must fall back to full projection whenever an
/// edit changes that boundary topology.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProjectionSpliceStatistics {
    range_indexes: RangeSpliceStats,
}

impl ProjectionSpliceStatistics {
    pub(super) fn add_range_statistics(&mut self,stats:RangeSpliceStats) {
        self.range_indexes.nodes_visited+=stats.nodes_visited;
        self.range_indexes.nodes_copied+=stats.nodes_copied;
        self.range_indexes.leaves_copied+=stats.leaves_copied;
        self.range_indexes.items_copied+=stats.items_copied;
    }


    pub(crate) fn range_index_nodes_visited(self) -> usize {
        self.range_indexes.nodes_visited
    }

    pub(crate) fn range_index_nodes_copied(self) -> usize {
        self.range_indexes.nodes_copied
    }

    pub(crate) fn range_index_leaves_copied(self) -> usize {
        self.range_indexes.leaves_copied
    }

    pub(crate) fn range_index_records_copied(self) -> usize {
        self.range_indexes.items_copied
    }
}

#[allow(clippy::too_many_arguments)]
/// How the rows of a regional Source reparse correspond to the rows it
/// replaces. Rows before and including the first edited row keep their
/// index, rows after the edited rows move by `delta`, and the rows between
/// are new.
struct RegionalRowMap {
    first_edited: usize,
    last_edited: usize,
    delta: isize,
}

impl RegionalRowMap {
    fn new(
        previous: &FormattedDocument,
        old_hard_lines: &Range<usize>,
        regional: &FormattedDocument,
        edits: &[TextEdit],
        source_paragraphs: bool,
    ) -> Result<Self, BlockIdentityError> {
        let delta = regional.hard_lines.len() as isize - old_hard_lines.len() as isize;
        if !source_paragraphs || delta == 0 {
            return Ok(Self { first_edited: 0, last_edited: 0, delta: 0 });
        }
        let (Some(first), Some(last)) = (edits.first(), edits.last()) else {
            return Err(BlockIdentityError::InvalidProjection);
        };
        let line_at = |at: usize| {
            previous
                .hard_line_at_offset(at)
                .and_then(|line| line.checked_sub(old_hard_lines.start))
                .filter(|&line| line < old_hard_lines.len())
                .ok_or(BlockIdentityError::InvalidProjection)
        };
        let (first_edited, last_edited) = (line_at(first.range.start)?, line_at(last.range.end)?);
        if last_edited.checked_add_signed(delta).is_none_or(|end| end < first_edited) {
            return Err(BlockIdentityError::InvalidProjection);
        }
        Ok(Self { first_edited, last_edited, delta })
    }

    /// The old region row for a regional row, or `None` for an added row.
    fn old_line(&self, line: usize) -> Option<usize> {
        if self.delta == 0 || line <= self.first_edited {
            Some(line)
        } else if (line as isize) > self.last_edited as isize + self.delta {
            line.checked_add_signed(-self.delta)
        } else {
            None
        }
    }
}

pub(crate) fn splice_line_local_projection(
    previous: &FormattedDocument,
    regional: FormattedDocument,
    revision: Revision,
    old_hard_lines: Range<usize>,
    old_formatted: Range<usize>,
    old_source: Range<usize>,
    new_source: Range<usize>,
    target_text: FormattedTextTree,
    new_source_content_end: usize,
    source_paragraphs: bool,
    literal_topology: bool,
    edits: &[TextEdit],
    source_patches: &[super::SourcePatch],
    retained_separator: Option<(Range<usize>, Range<usize>)>,
    next_projected_block_id: &mut u64,
) -> Result<(FormattedDocument, ProjectionSpliceStatistics), BlockIdentityError> {
    if regional.revision != revision
        || regional.source_content_start != new_source.start
        || regional.source_content_end != new_source.end
        || old_hard_lines.start >= old_hard_lines.end
        || old_hard_lines.end > previous.hard_lines.len()
        || old_formatted.start > old_formatted.end
        || old_formatted.end > previous.text.byte_len()
        || old_source.start > old_source.end
        || old_source.end > previous.source_content_end
        || new_source.start > new_source.end
        || new_source.end > new_source_content_end
    {
        return Err(BlockIdentityError::InvalidProjection);
    }

    let comment_prefix_end = old_hard_lines.start;
    let row_map = RegionalRowMap::new(previous, &old_hard_lines, &regional, edits, source_paragraphs)?;
    let first_block = previous
        .blocks
        .index_touching_point(old_formatted.start)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    let last_block = previous
        .blocks
        .index_touching_point(old_formatted.end)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    let old_block_indices = first_block..last_block + 1;
    let previous_region_blocks = previous
        .blocks
        .get_range(&old_block_indices)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    // Code blocks retain their multi-line paragraph inside quote containers.
    // A regional text edit must extend that same paragraph rather than demand
    // that the captured physical line cover its entire source extent.
    let partial_preserved_block = previous_region_blocks.len() == 1
        && matches!(previous_region_blocks[0].style.0.as_str(), "Code Block" | "Block quote")
        && previous_region_blocks[0].range.start <= old_formatted.start
        && old_formatted.end <= previous_region_blocks[0].range.end
        && regional.blocks.len() == 1
        && regional.blocks.get(0).is_some_and(|block| block.style == previous_region_blocks[0].style);
    if !partial_preserved_block && !literal_topology
        && !source_paragraphs
        && (previous_region_blocks
            .first()
            .map(|block| block.range.start)
            != Some(old_formatted.start)
            || previous_region_blocks.last().map(|block| block.range.end)
                != Some(old_formatted.end))
    {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let mut regional_blocks = regional.blocks.to_vec();
    if !source_paragraphs && !literal_topology && regional_blocks.len() != previous_region_blocks.len() {
        return Err(BlockIdentityError::InvalidProjection);
    }

    let new_formatted_end = old_formatted
        .start
        .checked_add(regional.text().len())
        .ok_or(BlockIdentityError::InvalidProjection)?;
    if new_formatted_end > target_text.byte_len()
        || target_text
            .slice(old_formatted.start..new_formatted_end)
            .map_err(|_| BlockIdentityError::InvalidProjection)?
            != regional.text()
        || target_text.byte_len()
            != previous
                .text
                .byte_len()
                .checked_sub(old_formatted.len())
                .and_then(|length| length.checked_add(regional.text().len()))
                .ok_or(BlockIdentityError::InvalidProjection)?
    {
        return Err(BlockIdentityError::InvalidProjection);
    }

    if literal_topology {
        for block in &mut regional_blocks {
            block.range = shift_region_range(&block.range, old_formatted.start)?;
            block.id = 0;
        }
        if let (Some(first), Some(old)) = (regional_blocks.first_mut(), previous_region_blocks.first()) {
            first.range.start = old.range.start;
        }
        if let (Some(last), Some(old)) = (regional_blocks.last_mut(), previous_region_blocks.last()) {
            last.range.end = old.range.end - old_formatted.end + new_formatted_end;
        }
        let mappings = build_edit_mappings(previous.text.byte_len(), target_text.byte_len(), edits)?;
        for old in &previous_region_blocks {
            if old.direct_formatting.is_some() {
                let local = mappings.iter().zip(edits).filter(|(mapping, _)| {
                    mapping.old.end >= old.range.start && mapping.old.start <= old.range.end
                }).collect::<Vec<_>>();
                if local.iter().any(|(mapping, edit)| old.range.start <= mapping.old.start
                    && mapping.old.end <= old.range.end && edit.replacement.contains('\n'))
                    && !local.iter().any(|(mapping, _)|
                        mapping.old.start < old.range.start && old.range.start < mapping.old.end
                        || mapping.old.start < old.range.end && old.range.end < mapping.old.end) {
                    let start = map_old_boundary_before(old.range.start, &mappings)?;
                    let mut end = map_old_boundary_before(old.range.end, &mappings)?;
                    for (mapping, _) in &local {
                        if mapping.old.is_empty() && mapping.old.start == old.range.end { end = end.max(mapping.new.end); }
                    }
                    for block in regional_blocks.iter_mut().filter(|block| start <= block.range.start && block.range.end <= end) {
                        block.direct_formatting = old.direct_formatting.clone();
                    }
                }
            }
            let follows_break = old.range.end < previous.text.byte_len()
                && previous.text.byte_chunk_at(old.range.end).first() == Some(&b'\n');
            let precedes_break = old.range.start > 0
                && previous.text.byte_chunk_at(old.range.start - 1).first() == Some(&b'\n');
            let witness = first_surviving_byte(&old.range, &mappings).map(|at| (at, false))
                .or_else(|| if follows_break {
                    first_surviving_byte(&(old.range.end..old.range.end + 1), &mappings).map(|at| (at, false))
                } else if precedes_break {
                    first_surviving_byte(&(old.range.start - 1..old.range.start), &mappings).map(|at| (at, true))
                } else { None });
            let at = if let Some((witness, after_break)) = witness {
                map_surviving_byte(witness, &mappings)? + usize::from(after_break)
            } else if old.range.is_empty() && follows_break {
                let Some(mapping) = mappings.iter().find(|mapping| !mapping.old.is_empty()
                    && mapping.old.start <= old.range.end && old.range.end < mapping.old.end) else { continue; };
                mapping.new.start
            } else if previous.blocks.len() == 1 { regional_blocks[0].range.start }
            else { continue; };
            let index = regional_blocks.partition_point(|block| block.range.start <= at).saturating_sub(1);
            if let Some(block) = regional_blocks.get_mut(index).filter(|block| block.id == 0 && at <= block.range.end) {
                block.id = old.id;
                block.direct_formatting = old.direct_formatting.clone();
            }
        }
        *next_projected_block_id = allocate_unassigned_block_ids(&mut regional_blocks, *next_projected_block_id)?;
    } else if source_paragraphs {
        let regional_lines = regional.hard_lines.to_vec();
        let mut used = std::collections::BTreeSet::new();
        let last = regional_blocks.len().saturating_sub(1);
        for (index, candidate) in regional_blocks.iter_mut().enumerate() {
            let line = regional_lines
                .partition_point(|line| line.range.start <= candidate.range.start)
                .saturating_sub(1);
            // A row a break adds takes its paragraph properties from the row
            // it was split from, and a fresh identity.
            let mapped = row_map.old_line(line);
            let old_line = previous
                .hard_lines
                .get(old_hard_lines.start + mapped.unwrap_or(row_map.first_edited))
                .ok_or(BlockIdentityError::InvalidProjection)?;
            let old = previous_region_blocks
                .iter()
                .find(|block| {
                    block.range.start <= old_line.range.start
                        && old_line.range.start <= block.range.end
                })
                .ok_or(BlockIdentityError::InvalidProjection)?;
            candidate.range = shift_region_range(&candidate.range, old_formatted.start)?;
            if index == 0 && old.range.start < old_formatted.start {
                candidate.range.start = old.range.start;
            }
            if index == last {
                let tail = previous_region_blocks
                    .last()
                    .ok_or(BlockIdentityError::InvalidProjection)?;
                if tail.range.end > old_formatted.end {
                    candidate.range.end = tail.range.end - old_formatted.end + new_formatted_end;
                }
            }
            candidate.id = if mapped.is_none() {
                0
            } else if used.insert(old.id) {
                old.id
            } else {
                old_line.id
            };
            used.insert(candidate.id);
            candidate.direct_formatting = old.direct_formatting.clone();
            if previous
                .style_sheet
                .configuration_deleted(&candidate.style, true)
            {
                candidate.style = previous.style_sheet.base_paragraph.clone();
            }
        }
        *next_projected_block_id = allocate_unassigned_block_ids(&mut regional_blocks, *next_projected_block_id)?;
    } else {
        for (candidate, old) in regional_blocks.iter_mut().zip(&previous_region_blocks) {
            candidate.range = shift_region_range(&candidate.range, old_formatted.start)?;
            if partial_preserved_block {
                candidate.range =
                    old.range.start..old.range.end - old_formatted.len() + regional.text().len();
            }
            candidate.id = old.id;
            candidate.direct_formatting = old.direct_formatting.clone();
            if previous
                .style_sheet
                .configuration_deleted(&candidate.style, true)
            {
                candidate.style = previous.style_sheet.base_paragraph.clone();
            }
        }
    }

    super::containers::reconcile(&mut regional_blocks, &previous_region_blocks, true);
    let mut range_stats = RangeSpliceStats::default();
    let blocks = previous
        .blocks
        .splice(
            old_block_indices,
            regional_blocks.clone(),
            old_formatted.end,
            new_formatted_end,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

    let old_lines = previous
        .hard_lines
        .get_range(&old_hard_lines)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    let regional_lines = regional.hard_lines.to_vec();
    if !literal_topology
        && Some(regional_lines.len()) != old_lines.len().checked_add_signed(row_map.delta)
    {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let regional_hard_lines = if literal_topology {
        regional_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let mut range = shift_region_range(&line.range, old_formatted.start)?;
                // A literal reparse can capture only a bounded piece of a
                // giant line. Keep its untouched outer extent, while retaining
                // the regional boundaries of newly inserted/internal breaks.
                if index == 0 {
                    range.start = old_lines.first().unwrap().range.start;
                }
                if index + 1 == regional_lines.len() {
                    range.end = old_lines.last().unwrap().range.end
                        .checked_sub(old_formatted.end)
                        .and_then(|tail| tail.checked_add(new_formatted_end))
                        .ok_or(BlockIdentityError::InvalidProjection)?;
                }
                let owner=regional_blocks.get(regional_blocks.partition_point(|block|block.range.start<=range.start).saturating_sub(1));
                let id=if let Some(block)=owner.filter(|block|block.range.start==range.start) {block.id} else {
                    let id=*next_projected_block_id;*next_projected_block_id=id.checked_add(1).ok_or(BlockIdentityError::Exhausted)?;id
                };
                Ok(HardLine {
                    id,
                    range,
                    separator_length: if index + 1 == regional_lines.len() {
                        old_lines.last().unwrap().separator_length
                    } else {
                        line.separator_length
                    },
                })
            })
            .collect::<Result<Vec<_>, BlockIdentityError>>()?
    } else if row_map.delta != 0 {
        // Rows kept from before the edit retain their identities. A row the
        // edit adds shares the identity of the paragraph it starts, and
        // otherwise gets its own.
        let mut lines = Vec::with_capacity(regional_lines.len());
        for (index, line) in regional_lines.iter().enumerate() {
            let range = shift_region_range(&line.range, old_formatted.start)?;
            let id = match row_map.old_line(index) {
                Some(old) => old_lines[old].id,
                None => match regional_blocks.iter().find(|block| block.range.start == range.start) {
                    Some(block) => block.id,
                    None => {
                        let id = *next_projected_block_id;
                        *next_projected_block_id =
                            id.checked_add(1).ok_or(BlockIdentityError::Exhausted)?;
                        id
                    }
                },
            };
            // A retained row identity does not retain its old EOF status.
            // Splitting the last row gives it a real separator; only the
            // region's outer boundary is inherited from the untouched suffix.
            let separator_length = if index + 1 == regional_lines.len() {
                old_lines.last().unwrap().separator_length
            } else {
                line.separator_length
            };
            lines.push(HardLine { id, range, separator_length });
        }
        lines
    } else {
        regional_lines
            .iter()
            .zip(&old_lines)
            .map(|(line, old)| {
                Ok(HardLine {
                    id: old.id,
                    range: shift_region_range(&line.range, old_formatted.start)?,
                    separator_length: old.separator_length,
                })
            })
            .collect::<Result<Vec<_>, BlockIdentityError>>()?
    };
    let hard_lines = previous
        .hard_lines
        .splice(
            old_hard_lines,
            regional_hard_lines,
            old_formatted.end,
            new_formatted_end,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

    let literal_mapping = previous.literal_encoding.is_some()
        && regional.literal_encoding == previous.literal_encoding;
    let style_indices = if literal_mapping {
        let first = previous
            .styles
            .query_overlapping(&old_formatted)
            .iter()
            .map(|span| span.range.start)
            .min()
            .unwrap_or(old_formatted.start);
        previous.styles.partition_point_start(first)
            ..previous.styles.partition_point_start(old_formatted.end)
    } else {
        contained_interval_indices(&previous.styles, &old_formatted)?
    };
    let mut regional_styles = regional
        .styles
        .items()
        .map(|span| {
            Ok(StyleSpan {
                range: shift_region_range(&span.range, old_formatted.start)?,
                application: span.application.clone(),
            })
        })
        .collect::<Result<Vec<_>, BlockIdentityError>>()?;
    if literal_mapping {
        for span in previous.styles.get_range(&style_indices).ok_or(BlockIdentityError::InvalidProjection)? {
            if span.range.start < old_formatted.start {
                regional_styles.push(StyleSpan {
                    range: span.range.start..span.range.end.min(old_formatted.start),
                    application: span.application.clone(),
                });
            }
            if span.range.end > old_formatted.end {
                let range = span.range.start.max(old_formatted.end)..span.range.end;
                regional_styles.push(StyleSpan {
                    range: shift_range_i128(&range, new_formatted_end as i128 - old_formatted.end as i128)
                        .ok_or(BlockIdentityError::InvalidProjection)?,
                    application: span.application,
                });
            }
        }
    }
    let styles = previous.styles
        .splice(
            style_indices,
            regional_styles,
            old_formatted.end,
            new_formatted_end,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

    let image_indices = contained_interval_indices(&previous.inline_images, &old_formatted)?;
    let regional_images = regional.inline_images.items().map(|image| {
        Ok(InlineImage { range: shift_region_range(&image.range, old_formatted.start)?, ..image })
    }).collect::<Result<Vec<_>, BlockIdentityError>>()?;
    let inline_images = previous.inline_images.splice_transformed(image_indices, regional_images,
        old_formatted.end, new_formatted_end, Some((old_source.end, new_source.end)), None, &mut range_stats)
        .ok_or(BlockIdentityError::InvalidProjection)?;

    // Stored contributors: runs stay within a hard line, so a hard-line
    // region contains every run it touches and splices whole items.
    let runs = &previous.provenance.runs;
    let mut provenance_indices = if literal_mapping {
        let mut first = runs.partition_point_start(old_formatted.start);
        if first > 0 && runs.get(first - 1).is_some_and(|span| span.formatted.end > old_formatted.start) {
            first -= 1;
        }
        first..runs.partition_point_start(old_formatted.end)
    } else { contained_interval_indices(runs, &old_formatted)? };
    // A table row can end in an empty cell. Its caret contributor belongs to
    // this row even though it occupies the half-open formatted endpoint.
    if literal_topology && previous.table_at(old_formatted.start).is_some() {
        while runs.get(provenance_indices.end).is_some_and(|span|
            span.formatted == (old_formatted.end..old_formatted.end)
                && old_source.start <= span.source.start && span.source.end <= old_source.end) {
            provenance_indices.end += 1;
        }
    }
    // A final empty paragraph's seed lies on its row before the document's
    // final line terminator, inside the region, at its half-open end.
    if !literal_mapping {
        while runs.get(provenance_indices.end).is_some_and(|span|
            span.formatted == (old_formatted.end..old_formatted.end) && span.source.is_empty()
                && old_source.start <= span.source.start && span.source.end < old_source.end
                && old_formatted.end == previous.text.byte_len()) {
            provenance_indices.end += 1;
        }
    }
    // A paragraph that ends inside inline markup, such as `<b>…</b></p>`,
    // records its following break as an empty source span at the end of its
    // last content, which lies inside the region. The break moves with that
    // content: replace it at the regional parse's content end.
    let content_end = |spans: &[ProvenanceRun]| {
        spans.iter().rev().find(|span| !span.source.is_empty()).map(|span| span.source.end)
    };
    let trailing_break = if literal_mapping {
        None
    } else {
        let old_content = runs
            .get_range(&provenance_indices)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        runs.get(provenance_indices.end).filter(|span| {
            span.source.is_empty()
                && span.formatted.start >= old_formatted.end
                && old_source.start < span.source.start
                && span.source.start < old_source.end
                && content_end(&old_content) == Some(span.source.start)
        })
    };
    if trailing_break.is_some() {
        provenance_indices.end += 1;
    }
    if let Some((old, new)) = &retained_separator {
        let boundary = runs.get(provenance_indices.end)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        if source_paragraphs || literal_topology
            || boundary.formatted != (old_formatted.end..old_formatted.end + 1)
            || boundary.source != *old || old.start < old_source.start || old.end > old_source.end
            || new.start < new_source.start || new.end > new_source.end
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        provenance_indices.end += 1;
    }
    let old_provenance = runs
        .get_range(&provenance_indices)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    if !literal_mapping && old_provenance
        .iter()
        .any(|span| span.source.start < old_source.start || span.source.end > old_source.end)
    {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let mapping_source = if literal_mapping {
        old_provenance.first().map_or(old_source.start, |span| span.source.start)
            ..old_provenance.last().map_or(old_source.end, |span| span.source.end)
    } else { old_source.clone() };
    validate_source_neighbors(
        runs,
        &provenance_indices,
        &mapping_source,
        |span| &span.source,
    )?;
    let mut regional_provenance = regional
        .provenance
        .stored()
        .map(|run| Ok(run.with_range(shift_region_range(&run.formatted, old_formatted.start)?)))
        .collect::<Result<Vec<_>, BlockIdentityError>>()?;
    if let Some(boundary) = trailing_break {
        let at = content_end(&regional_provenance).unwrap_or(new_source.start);
        let formatted = shift_range_i128(&boundary.formatted, new_formatted_end as i128 - old_formatted.end as i128)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        regional_provenance.push(ProvenanceRun::indivisible(ProvenanceSpan { formatted, source: at..at }));
    }
    if let Some((_, source)) = retained_separator {
        regional_provenance.push(ProvenanceRun::indivisible(ProvenanceSpan {
            formatted: new_formatted_end..new_formatted_end + 1,
            source,
        }));
    }
    if literal_mapping {
        if let Some(first) = old_provenance.first().filter(|span| span.formatted.start < old_formatted.start) {
            let prefix = previous.clip_literal_span(first.hull(), &(first.formatted.start..old_formatted.start));
            if prefix.formatted.end != old_formatted.start { return Err(BlockIdentityError::InvalidProjection); }
            regional_provenance.insert(0, ProvenanceRun::indivisible(prefix));
        }
        if let Some(last) = old_provenance.last().filter(|span| span.formatted.end > old_formatted.end) {
            let suffix = previous.clip_literal_span(last.hull(), &(old_formatted.end..last.formatted.end));
            if suffix.formatted.start != old_formatted.end { return Err(BlockIdentityError::InvalidProjection); }
            let formatted = shift_range_i128(&suffix.formatted, new_formatted_end as i128 - old_formatted.end as i128)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            let source = shift_range_i128(&suffix.source, new_source.end as i128 - old_source.end as i128)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            regional_provenance.push(ProvenanceRun::indivisible(ProvenanceSpan { formatted, source }));
        }
    }
    // A source-reordered projection needs full reprojection when an edit can
    // shift its two orderings differently. Ordinary text/rich paragraphs
    // splice both persistent indexes without walking the untouched suffix.
    if !previous.source_ordered || !regional.source_ordered {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let formatted_delta = new_formatted_end as i128 - old_formatted.end as i128;
    let source_delta = new_source.end as i128 - old_source.end as i128;
    let source_boundaries = if let Some(boundaries) = &previous.provenance.boundaries {
    let boundary_indices = boundaries.partition_point_start(mapping_source.start)
        ..mapping_source
            .end
            .checked_add(1)
            .map_or(boundaries.len(), |end| boundaries.partition_point_start(end));
    let old_boundaries = boundaries
        .get_range(&boundary_indices)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    // Replaced items include other contributors' edges at the region's source
    // endpoints; retain those outside the formatted region.
    let mut regional_boundaries = provenance_runs::boundary_items(regional_provenance.iter().cloned());
    regional_boundaries.extend(old_boundaries.iter()
        .filter(|item| item.source.start == mapping_source.start && item.formatted.end <= old_formatted.start)
        .cloned());
    regional_boundaries.extend(old_boundaries.iter()
        .filter(|item| item.source.start == mapping_source.end && item.formatted.start >= old_formatted.end)
        .filter_map(|item| item.with_transform(shift_range_i128(&item.source, source_delta)?, formatted_delta, None)));
    provenance_runs::normalize_boundary_items(&mut regional_boundaries);
    let (boundary_prefix, boundary_suffix) = shared_transformed_edges(
        &old_boundaries,
        &regional_boundaries,
        |old, new| old == new,
        |old, new| old.transformed_eq(new, source_delta, formatted_delta),
    );
    let narrowed_boundary_indices = boundary_indices.start + boundary_prefix
        ..boundary_indices.end - boundary_suffix;
    let narrowed_regional_boundaries = regional_boundaries
        [boundary_prefix..regional_boundaries.len() - boundary_suffix]
        .to_vec();
    Some(boundaries
        .splice_transformed(
            narrowed_boundary_indices,
            narrowed_regional_boundaries,
            old_source.end,
            new_source.end,
            Some((old_formatted.end, new_formatted_end)),
            None,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?)
    } else {
        // Reverse queries share the formatted tree only while source starts
        // stay ordered, including empty caret seeds and both splice seams.
        let before = provenance_indices.start.checked_sub(1).and_then(|at| runs.get(at));
        let after = runs.get(provenance_indices.end);
        let starts = before.as_ref().map(|run| run.source.start as i128)
            .into_iter().chain(regional_provenance.iter().map(|run| run.source.start as i128))
            .chain(after.as_ref().map(|run| run.source.start as i128 + source_delta));
        let mut previous = None;
        for start in starts {
            if previous.is_some_and(|previous| previous > start) {
                return Err(BlockIdentityError::InvalidProjection);
            }
            previous = Some(start);
        }
        None
    };
    let (provenance_prefix, provenance_suffix) = shared_transformed_edges(
        &old_provenance,
        &regional_provenance,
        |old, new| old == new,
        |old, new| {
            transformed_provenance_eq(
                old,
                new,
                old_formatted.end,
                new_formatted_end,
                old_source.end,
                new_source.end,
            )
        },
    );
    let narrowed_provenance_indices = provenance_indices.start + provenance_prefix
        ..provenance_indices.end - provenance_suffix;
    let narrowed_regional_provenance = regional_provenance
        [provenance_prefix..regional_provenance.len() - provenance_suffix]
        .to_vec();
    let provenance = ProvenanceIndex {
        runs: runs
            .splice_transformed(
                narrowed_provenance_indices,
                narrowed_regional_provenance,
                old_formatted.end,
                new_formatted_end,
                Some((old_source.end, new_source.end)),
                None,
                &mut range_stats,
            )
            .ok_or(BlockIdentityError::InvalidProjection)?,
        boundaries: source_boundaries,
    };

    let diagnostic_indices =
        contained_interval_indices(&previous.decoding_diagnostics, &old_formatted)?;
    let old_diagnostics = previous
        .decoding_diagnostics
        .get_range(&diagnostic_indices)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    if old_diagnostics.iter().any(|diagnostic| {
        diagnostic.source_range.start < old_source.start
            || diagnostic.source_range.end > old_source.end
    }) {
        return Err(BlockIdentityError::InvalidProjection);
    }
    validate_source_neighbors(
        &previous.decoding_diagnostics,
        &diagnostic_indices,
        &old_source,
        |diagnostic| &diagnostic.source_range,
    )?;
    let regional_diagnostics = regional
        .decoding_diagnostics
        .items()
        .map(|mut diagnostic| {
            diagnostic.revision = revision;
            diagnostic.formatted_range =
                shift_region_range(&diagnostic.formatted_range, old_formatted.start)?;
            Ok(diagnostic)
        })
        .collect::<Result<Vec<_>, BlockIdentityError>>()?;
    let decoding_diagnostics = previous
        .decoding_diagnostics
        .splice_transformed(
            diagnostic_indices,
            regional_diagnostics,
            old_formatted.end,
            new_formatted_end,
            Some((old_source.end, new_source.end)),
            Some(revision.0),
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

    let mut candidate = FormattedDocument {
        revision,
        text: target_text,
        flat_text: Arc::new(OnceLock::new()),
        comment_context_cache: super::comment_context::prefix_cache(&previous.comment_context_cache, comment_prefix_end),
        tables: Default::default(),
        table_lines: None,
        blocks,
        hard_lines,
        styles,
        inline_images,
        provenance,
        literal_encoding: regional.literal_encoding,
        source_ordered: true,
        decoding_diagnostics,
        markdown_code_presentation: None,
        style_sheet: {
            let mut sheet = previous.style_sheet.clone();
            if let Some(level) = regional
                .style_sheet
                .block_styles()
                .filter_map(|style| {
                    style
                        .id
                        .0
                        .strip_prefix("List")
                        .and_then(|number| number.parse::<u16>().ok())
                })
                .max()
            {
                let required = StyleId(format!("List{level}"));
                if sheet.block_style(&required).is_none() {
                    Arc::make_mut(&mut sheet).ensure_list_level(level);
                }
            }
            sheet
        },
        document_style: previous.document_style.clone(),
        source_content_start: previous.source_content_start,
        source_content_end: new_source_content_end,
        source_insertion_end: (previous.source_insertion_end as i128
            + new_source_content_end as i128
            - previous.source_content_end as i128) as usize,
    };
    // Table structure is retained for grammar-proven cell edits and untouched
    // tables outside this local window. Regional parsing replaces complete tables.
    let map_point = |point:usize, spans:&[(Range<usize>,usize)], after:bool| {
        let mut delta=0isize;
        for (range,length) in spans {
            if point < range.start || point == range.start && !after {break;}
            if point <= range.end {return (range.start as isize + delta) as usize + if after {*length} else {0};}
            delta+=*length as isize-range.len() as isize;
        }
        point.checked_add_signed(delta).unwrap()
    };
    let text_spans=edits.iter().map(|edit|(edit.range.clone(),edit.replacement.len())).collect::<Vec<_>>();
    let source_spans=source_patches.iter().map(|patch|(patch.range(),patch.replacement().len())).collect::<Vec<_>>();
    let map_range=|range:&Range<usize>,spans:&[(Range<usize>,usize)]| map_point(range.start,spans,false)..map_point(range.end,spans,true);
    let table_start = old_formatted.start.checked_sub(1)
        .map_or(0, |at| previous.tables.partition_point_end(at));
    let table_indices = table_start..previous.tables.partition_point_start(old_formatted.end.saturating_add(1));
    let mut tables=Vec::new();
    for old in previous.tables.get_range(&table_indices).ok_or(BlockIdentityError::InvalidProjection)? {
        if !regional.tables.is_empty() && old_formatted.start <= old.range.start && old.range.end <= old_formatted.end {continue;}
        let mut table=old.clone(); table.range=map_range(&table.range,&text_spans); table.source_range=map_range(&table.source_range,&source_spans);
        // Only records touching the changed interval are copied. Coordinates
        // on the persistent suffix shift together, including physical bytes.
        let changed_text=text_spans.first().map_or(old_formatted.start,|span|span.0.start)..text_spans.last().map_or(old_formatted.end,|span|span.0.end);
        let changed_source=source_spans.first().map_or(old_source.start, |span| span.0.start)
            ..source_spans.last().map_or(old_source.end, |span| span.0.end);
        let rows=table.rows.partition_point(|row|row.range.end<changed_text.start && row.source_range.end<changed_source.start)
            ..table.rows.partition_point(|row|row.range.start<=changed_text.end || row.source_range.start<=changed_source.end);
        let mut replacement=Vec::with_capacity(rows.len());
        for index in rows.clone() {let mut row=table.rows[index].clone();
            row.range=map_range(&row.range,&text_spans);row.source_range=map_range(&row.source_range,&source_spans);
            for cell in &mut row.cells {cell.range=map_range(&cell.range,&text_spans);cell.source_range=map_range(&cell.source_range,&source_spans);}
            replacement.push(row);
        }
        table.rows=table.rows.splice_transformed(rows,replacement,old_formatted.end,new_formatted_end,
            previous.source_content_end,new_source_content_end,&mut range_stats).ok_or(BlockIdentityError::InvalidProjection)?;
        let rows=table.source_rows.partition_point(|row|row.source_range.end<changed_source.start)
            ..table.source_rows.partition_point(|row|row.source_range.start<=changed_source.end);
        let mut replacement=Vec::with_capacity(rows.len());
        for index in rows.clone() {let mut row=table.source_rows[index].clone();
            row.range=map_range(&row.range,&text_spans);row.source_range=map_range(&row.source_range,&source_spans);row.source_body=map_range(&row.source_body,&source_spans);
            for cell in &mut row.source_cells {*cell=map_range(cell,&source_spans);}
            replacement.push(row);
        }
        table.source_rows=table.source_rows.splice_transformed(rows,replacement,old_formatted.end,new_formatted_end,
            previous.source_content_end,new_source_content_end,&mut range_stats).ok_or(BlockIdentityError::InvalidProjection)?;
        tables.push(table);
    }
    for mut table in regional.tables.iter().cloned() {
        table.range=shift_region_range(&table.range,old_formatted.start)?;
        for row in &mut table.rows {row.range=shift_region_range(&row.range,old_formatted.start)?;for cell in &mut row.cells {cell.range=shift_region_range(&cell.range,old_formatted.start)?;}}
        for row in &mut table.source_rows {row.range=shift_region_range(&row.range,old_formatted.start)?;}
        tables.push(table);
    }
    tables.sort_by_key(|table|table.range.start);
    let changed_tables = table_indices.start..table_indices.start + tables.len();
    candidate.tables=previous.tables.splice_transformed(table_indices.clone(), tables,
        old_formatted.end,new_formatted_end,previous.source_content_end,new_source_content_end,&mut range_stats)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    if !regional.tables.is_empty() {
        *next_projected_block_id=candidate.install_table_ids_in(*next_projected_block_id,Some((previous,table_indices)),changed_tables)?;
    }
    if let Some(previous_lines)=&previous.table_lines {
        let start=previous_lines.partition_point(|line|line.range.end<old_formatted.start);
        let end=previous_lines.partition_point(|line|line.range.start<=old_formatted.end);
        let expanded_start=previous_lines.get(start).map_or(old_formatted.start,|line|line.range.start);
        let expanded_end=previous_lines.get(end.saturating_sub(1)).map_or(old_formatted.end,|line|line.range.end);
        let new_start=map_point(expanded_start,&text_spans,false);
        let new_end=map_point(expanded_end,&text_spans,true);
        let mut index=candidate.hard_lines.partition_point_start(new_start);
        let mut lines=Vec::new();
        while let Some(line)=candidate.hard_lines.get(index).filter(|line|line.range.start<=new_end) {
            if let Some(table)=candidate.table_at(line.range.start) {
                let row_index=table.rows.partition_point_start(line.range.start.saturating_add(1)).saturating_sub(1);
                if let Some(row)=table.rows.get(row_index).filter(|row|line.range.start<=row.range.end) {
                    lines.push(HardLine{id:row.id,range:row.range.clone(),separator_length:1});
                    index=candidate.hard_lines.partition_point_start(row.range.end.saturating_add(1));continue;
                }
            }
            lines.push(line);index+=1;
        }
        candidate.table_lines=Some(previous_lines.splice(start..end,lines,old_formatted.end,new_formatted_end,&mut range_stats)
            .ok_or(BlockIdentityError::InvalidProjection)?);
    } else if regional.table_lines.is_some() {candidate.rebuild_table_presentation_lines();}
    Ok((
        candidate,
        ProjectionSpliceStatistics {
            range_indexes: range_stats,
        },
    ))
}

fn contained_interval_indices<T>(
    store: &IntervalRangeStore<T>,
    region: &Range<usize>,
) -> Result<Range<usize>, BlockIdentityError>
where
    T: RangedItem + Clone,
{
    if store
        .query_overlapping(region)
        .iter()
        .any(|item| item.range().start < region.start || item.range().end > region.end)
    {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let start = store.partition_point_start(region.start);
    let mut end = store.partition_point_start(region.end);
    if region.is_empty() {
        // An empty paragraph has real point annotations even though its
        // character extent is empty. Replacing that body consumes its caret
        // seed; retaining it would shift it past generated closing tags and
        // send continued typing outside the original inline formatting.
        while store.get(end).is_some_and(|item| item.range() == region) {
            end += 1;
        }
    }
    let indices = start..end;
    if store
        .get_range(&indices)
        .ok_or(BlockIdentityError::InvalidProjection)?
        .iter()
        .any(|item| item.range().start < region.start || item.range().end > region.end)
    {
        return Err(BlockIdentityError::InvalidProjection);
    }
    Ok(indices)
}

/// Find records which can remain on the persistent index's shared prefix and
/// lazily shifted suffix. A line-local parser deliberately sees complete hard
/// lines, but changing one scalar must not consequently replace every
/// per-scalar provenance record in a long line.
fn shared_transformed_edges<T>(
    old: &[T],
    new: &[T],
    prefix_matches: impl Fn(&T, &T) -> bool,
    suffix_matches: impl Fn(&T, &T) -> bool,
) -> (usize, usize) {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| prefix_matches(old, new))
        .count();
    let suffix_limit = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take(suffix_limit)
        .take_while(|(old, new)| suffix_matches(old, new))
        .count();
    (prefix, suffix)
}

fn transformed_provenance_eq(
    old: &ProvenanceRun,
    new: &ProvenanceRun,
    old_formatted_end: usize,
    new_formatted_end: usize,
    old_source_end: usize,
    new_source_end: usize,
) -> bool {
    let formatted_shift = new_formatted_end as i128 - old_formatted_end as i128;
    let source_shift = new_source_end as i128 - old_source_end as i128;
    old.is_divisible() == new.is_divisible()
        && old.with_transform(shift_range_i128(&old.formatted, formatted_shift).unwrap_or(0..0), source_shift, None)
            .is_some_and(|shifted| shifted == *new)
}

fn validate_source_neighbors<T>(
    store: &IntervalRangeStore<T>,
    replaced: &Range<usize>,
    old_source: &Range<usize>,
    source_range: impl Fn(&T) -> &Range<usize>,
) -> Result<(), BlockIdentityError>
where
    T: RangedItem + Clone,
{
    if replaced.start > 0 {
        let preceding = store
            .get(replaced.start - 1)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        if source_range(&preceding).end > old_source.start {
            return Err(BlockIdentityError::InvalidProjection);
        }
    }
    if replaced.end < store.len() {
        let following = store
            .get(replaced.end)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        if source_range(&following).start < old_source.end {
            return Err(BlockIdentityError::InvalidProjection);
        }
    }
    Ok(())
}

fn shift_region_range(
    range: &Range<usize>,
    destination_start: usize,
) -> Result<Range<usize>, BlockIdentityError> {
    Ok(destination_start
        .checked_add(range.start)
        .ok_or(BlockIdentityError::InvalidProjection)?
        ..destination_start
            .checked_add(range.end)
            .ok_or(BlockIdentityError::InvalidProjection)?)
}

pub(super) fn project_plain(
    normalized: &NormalizedText,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
) -> FormattedDocument {
    let hard_lines = normalized_hard_line_ranges(normalized);
    let provenance = normalized
        .unit_runs()
        .map(|unit| ProvenanceSpan {
            formatted: unit.normalized.clone(),
            source: unit.source.clone(),
        })
        .collect();
    let decoding_diagnostics = normalized
        .unit_runs()
        .filter_map(|unit| {
            unit.decoding_diagnostic.map(|kind| DecodingDiagnostic {
                revision,
                encoding: normalized.encoding,
                kind,
                source_range: unit.source.clone(),
                formatted_range: unit.normalized.clone(),
            })
        })
        .collect();
    let mut projection = FormattedDocument::from_parts_with(
        revision,
        normalized.text.clone(),
        blocks_for_hard_line_ranges(&hard_lines),
        Vec::new(),
        provenance,
        decoding_diagnostics,
        StyleSheet::plain_text(),
        source_content_start,
        source_content_end,
        false,
    );
    projection.literal_encoding = Some(normalized.encoding);
    projection
}

fn normalized_hard_line_ranges(normalized: &NormalizedText) -> Vec<Range<usize>> {
    let mut ranges = Vec::with_capacity(normalized.endings.len() + 1);
    let mut start = 0;
    for ending in &normalized.endings {
        debug_assert!(start <= ending.normalized.start);
        debug_assert!(ending.normalized.start < ending.normalized.end);
        ranges.push(start..ending.normalized.start);
        start = ending.normalized.end;
    }
    ranges.push(start..normalized.text.len());
    ranges
}

fn blocks_for_hard_line_ranges(ranges: &[Range<usize>]) -> Vec<Block> {
    ranges
        .iter()
        .cloned()
        .map(|range| Block::paragraph(0, range))
        .collect()
}

#[cfg(test)]
fn blocks_for_plain_text(text: &str) -> Vec<Block> {
    let mut ranges = Vec::new();
    let mut start = 0;
    loop {
        let end = text[start..]
            .find('\n')
            .map(|relative| start + relative)
            .unwrap_or(text.len());
        ranges.push(start..end);
        if end == text.len() {
            break;
        }
        start = end + 1;
    }
    blocks_for_hard_line_ranges(&ranges)
}

/// Fast, provenance-free fixture used only by large-document layout tests.
/// It preserves the real formatted text tree and paragraph partition while
/// avoiding the intentionally detailed decode/line-ending provenance setup,
/// which is tested independently and would dominate this test's runtime.
#[cfg(test)]
pub(crate) fn layout_test_plain_projection(text: String) -> FormattedDocument {
    let text_len = text.len();
    let mut blocks = blocks_for_plain_text(&text);
    for (index, block) in blocks.iter_mut().enumerate() {
        block.id = u64::try_from(index + 1).expect("fixture block identity is representable");
    }
    FormattedDocument::from_parts(
        Revision(0),
        text,
        blocks,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        StyleSheet::plain_text(),
        0,
        text_len,
    )
}

pub(super) fn markdown_html_open_pre(source: &str) -> bool {
    markdown_html::open_pre(source)
}

pub(super) fn markdown_html_pre_owners(source: &str, excluded: &[Range<usize>]) -> Vec<super::html::PreOwner> {
    markdown_html::pre_owners(source, excluded)
}

fn project_markdown(
    normalized: &NormalizedText,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    document_start: bool,
) -> FormattedDocument {
    let syntax = super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(normalized), if document_start { super::markdown_syntax::ParseContext::Document } else { super::markdown_syntax::ParseContext::Interior }).to_source(normalized);
    let definitions = super::markdown_syntax::definitions(&normalized.text, if document_start { super::markdown_syntax::ParseContext::Document } else { super::markdown_syntax::ParseContext::Interior }).0;
    let quote_context = super::markdown_quotes::source_context_with_syntax(normalized, &syntax);
    let indented = super::markdown_indented_code::classify_with_syntax(normalized, &syntax);
    let list_context = super::markdown_blocks::source_context_with_syntax(normalized, &syntax);
    let (cooked, explicit) = super::paragraph_flow::markdown_with_syntax(normalized, document_start, &syntax);
    let mut projected = project_markdown_lines(
        &cooked,
        revision,
        source_content_start,
        source_content_end,
        &list_context,
        &quote_context,
        &indented,
        &syntax,
        &definitions,
        &explicit,
    );
    if let Some(ending) = normalized
        .endings
        .last()
        .filter(|ending| ending.normalized.end == normalized.text.len())
    {
        if cooked
            .last_unit()
            .map_or(true, |unit| unit.source.end <= ending.source.start)
        {
            projected.source_insertion_end = ending.source.start;
        }
    }
    if !explicit.is_empty() {
        let mut paragraphs: Vec<Block> = Vec::new();
        // Read the tree once; slice access would retain a flat copy.
        for block in &projected.blocks.to_vec() {
            let join = paragraphs.last().is_some_and(|previous| {
                (previous.kind == BlockKind::Paragraph && block.kind == BlockKind::Paragraph
                    || matches!((&previous.kind, &block.kind),
                        (BlockKind::ListItem { ordered:a, ordinal:b, level:c, .. }, BlockKind::ListItem { ordered:d, ordinal:e, level:f, item_start:false, .. })
                        if a == d && b == e && c == f))
                    && previous.style.0 != "Code Block"
                    && block.style.0 != "Code Block"
                    && !projected.range_intersects_table(&previous.range)
                    && !projected.range_intersects_table(&block.range)
                    && previous.quote_depth == block.quote_depth && previous.style == block.style
                    && projected
                        .provenance_for_region(&(previous.range.end..block.range.start))
                        .iter()
                        .any(|span| explicit.contains(&span.source.end))
            });
            if join {
                paragraphs.last_mut().unwrap().range.end = block.range.end;
            } else {
                paragraphs.push(block.clone());
            }
        }
        projected.install_paragraph_partition(paragraphs);
    }
    projected.install_source_containers(syntax.owners.clone());
    projected.assign_image_paragraph_styles();
    projected.share_block_attributes();
    projected
}

fn markdown_presented_kind(mut kind: BlockKind) -> BlockKind {
    if let BlockKind::ListItem {
        marker_is_decoration,
        ..
    } = &mut kind
    {
        *marker_is_decoration = true;
    }
    kind
}

fn project_markdown_lines(
    normalized: &NormalizedText,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    list_context: &[(Range<usize>, super::markdown_blocks::ListLine)],
    quote_context: &[super::markdown_quotes::QuoteLine],
    indented: &[super::markdown_indented_code::CodeBlock],
    syntax: &super::markdown_syntax::Blocks,
    definitions: &str,
    explicit_breaks: &std::collections::BTreeSet<usize>,
) -> FormattedDocument {
    let mut builder = MarkdownBuilder::new(
        normalized,
        revision,
        syntax,
    );
    builder.reference_definitions = definitions.to_owned();
    let input_lines = normalized_hard_line_ranges(normalized);
    let mut hard_breaks = Vec::new();
    let mut tables = Vec::new();

    let mut line_index = 0;
    let mut container_index = 0;
    let mut containers: Vec<&super::markdown_syntax::Container> = Vec::new();
    let normalized_at = |at| normalized.normalized_at(at);
    while line_index < input_lines.len() {
        let line = &input_lines[line_index];
        let output_start = builder.output.len();
        // A final empty row has no unit. After a final line terminator it is
        // the last source row before that terminator (inside its quote, and
        // where typing belongs), not the boundary after the terminator.
        let final_row = quote_context.len().checked_sub(1).filter(|last| line.is_empty()
            && line_index + 1 == input_lines.len()
            && quote_context[*last].range == (source_content_end..source_content_end))
            .and_then(|last| quote_context.get(last.checked_sub(1)?))
            .map(|row| row.range.end);
        let source_at = Some(builder.unit_at(line.start).map_or_else(
            || final_row.unwrap_or(source_content_end), |unit| unit.source.start));
        let source_end = builder.unit_at(line.end).map_or_else(
            || final_row.unwrap_or(source_content_end), |unit| unit.source.start);
        let at = source_at.unwrap();
        containers.retain(|container| at < container.range.end);
        while let Some(container) = syntax.containers.get(container_index).filter(|container| container.range.start <= source_end) {
            if container.range.end > at { containers.push(container); }
            container_index += 1;
        }
        let quote_depth = containers.iter().map(|container| container.quote_depth).max().unwrap_or(0);
        let list = containers.iter().rev().find(|container| container.list.is_some()).copied();
        let block_syntax = syntax.blocks.get(syntax.blocks.partition_point(|block| block.range.end <= at))
            .filter(|block| block.range.start <= source_end);
        let quote = source_at.and_then(|at| quote_context
            .get(quote_context.partition_point(|line| line.range.start <= at).saturating_sub(1))
            .filter(|line| line.range.start <= at && at <= line.range.end));
        let quote_depth = quote_depth.max(quote.map_or(0, |line| line.depth));
        let quoted = quote_depth > 0;
        let semantic_start = line.start;
        let context = source_at.map(|at| quote.map_or(at, |quote| at.max(quote.content_start))).and_then(|at| {
            list_context
                .get(
                    list_context
                        .partition_point(|(range, _)| range.start <= at)
                        .saturating_sub(1),
                )
                .filter(|(range, _)| range.start <= at && at <= range.end)
                .map(|(_, context)| context)
        });
        let presented_kind = |fallback: BlockKind| {
            let kind = if let Some(context) = context { context.kind.clone() }
            else if quote_depth > 0 && !normalized.text[line.clone()].trim().is_empty() {
                list.and_then(|container| container.list.clone()).map(|mut kind| {
                    if let BlockKind::ListItem { item_start, container_start, .. } = &mut kind {
                        *item_start = at <= list.unwrap().range.start;
                        *container_start &= *item_start;
                    }
                    kind
                }).unwrap_or(fallback)
            } else { fallback };
            markdown_presented_kind(kind)
        };
        if let Some(table) = syntax.tables.get(syntax.tables.partition_point(|table| table.range.end <= at)).filter(|table| table.range.start <= source_end && at < table.range.end) {
            // All table rows belong to the same enclosing list item. Only
            // its first projected block can own an authored list marker.
            let table_kind = |first: bool| {
                let mut kind = presented_kind(BlockKind::Paragraph);
                if !first {
                    if let BlockKind::ListItem { item_start, container_start, .. } = &mut kind {
                        *item_start = false;
                        *container_start = false;
                    }
                }
                kind
            };
            let mut projected = super::MarkdownTable { id: 0, range: output_start..output_start, source_range: table.range.clone(), columns: table.columns.clone(), rows: Default::default(), source_rows: Default::default() };
            builder.in_table = true;
            for source_row in &table.rows {
                let row_start = builder.output.len();
                let raw_end = normalized_at(source_row.body.end);
                let style = if source_row.delimiter || projected.rows.is_empty() { "Table header" } else { "Table cell" };
                let mut cells = Vec::new();
                if !source_row.delimiter {
                    for column in 0..table.columns.len() {
                        if column > 0 { hard_breaks.push(builder.output.len()); builder.output.push('\n'); }
                        let start = builder.output.len();
                        let raw = source_row.cells.get(column).cloned().unwrap_or(source_row.body.end..source_row.body.end);
                        let a = normalized_at(raw.start).min(raw_end); let b = normalized_at(raw.end).min(raw_end);
                        let value = &normalized.text[a..b];
                        let left = a + value.len() - value.trim_start_matches([' ', '\t']).len();
                        let right = a + value.trim_end_matches([' ', '\t']).len();
                        builder.parse_inline(left, right.max(left));
                        let source = if left < right { builder.unit_at(left).unwrap().source.start..builder.unit_at(normalized.text[..right].char_indices().next_back().unwrap().0).unwrap().source.end } else { raw.start..raw.start };
                        if start == builder.output.len() { builder.push_provenance(ProvenanceSpan { formatted:start..start, source:source.clone() }); }
                        let range = start..builder.output.len();
                        let mut block = Block::new(0,range.clone(),table_kind(row_start == output_start && column == 0),style.into(),None); block.quote_depth=quote_depth; builder.blocks.push(block);
                        cells.push(super::MarkdownTableCell { id:0,range,source_range:source,missing:column>=source_row.cells.len() });
                    }
                }
                let row_end = builder.output.len();
                projected.source_rows.push(super::MarkdownTableSourceRow { range:row_start..row_end,source_range:source_row.range.clone(),source_body:source_row.body.clone(),source_cells:source_row.cells.clone(),delimiter:source_row.delimiter });
                if !source_row.delimiter { projected.rows.push(super::MarkdownTableRow { id:0,range:row_start..row_end,source_range:source_row.range.clone(),cells }); }
                if !source_row.delimiter {
                    let last = source_row.range.end == table.rows.iter().rev().find(|row| !row.delimiter).unwrap().range.end;
                    if !last { hard_breaks.push(builder.output.len()); builder.output.push('\n'); }
                }
            }
            builder.in_table = false;
            projected.range.end=builder.output.len();
            while line_index < input_lines.len() && builder.unit_at(input_lines[line_index].start).is_some_and(|unit| unit.source.start < table.range.end) { line_index+=1; }
            // The table's final physical ending separates it from subsequent prose.
            if let Some(ending) = line_index.checked_sub(1).and_then(|index| normalized.endings.get(index)) { hard_breaks.push(builder.output.len()); builder.emit_unit_at(ending.normalized.start); }
            tables.push(projected);
            continue;
        }
        if let Some(html) = block_syntax.filter(|block| matches!(block.role, super::markdown_syntax::BlockRole::Html)
            && markdown_inline_break_length(&normalized.text[normalized_at(block.range.start).min(line.end)..line.end]).is_none()) {
            let first = line_index;
            while input_lines.get(line_index + 1).is_some_and(|next| builder.unit_at(next.start).is_some_and(|unit| unit.source.start < html.range.end)) { line_index += 1; }
            let range = input_lines[first].start..input_lines[line_index].end;
            let contains_table = markdown_html::contains_table(&normalized.text[range.clone()]);
            let pre_ending = !contains_table && normalized.endings.get(line_index).is_some_and(|ending|
                ending.source.end == source_content_end && markdown_html::open_pre(&normalized.text[range.clone()]));
            let body_start = normalized_at(html.range.start).max(range.start);
            let comment = normalized.text[body_start..range.end].starts_with("<!--");
            if comment || contains_table {
                // Comments are visible literal source, including whitespace
                // and hard lines. The paragraph-flow pass has already removed
                // surrounding Markdown owner prefixes.
                let start = if comment { body_start } else { range.start };
                builder.emit_range(start, range.end);
                let first_ending = normalized.endings.partition_point(|ending| ending.normalized.start < start);
                for ending in normalized.endings[first_ending..].iter()
                    .take_while(|ending| ending.normalized.start < range.end) {
                    hard_breaks.push(output_start + ending.normalized.start - start);
                }
                builder.blocks.push(Block::new(0, output_start..builder.output.len(), presented_kind(BlockKind::Paragraph), "Paragraph".into(), None));
                let block = builder.blocks.last_mut().unwrap();
                block.quote_depth = quote_depth;
                block.markdown_html = comment && body_start != range.start;
                if comment && output_start < builder.output.len() {
                    builder.styles.push(StyleSpan { range: output_start..builder.output.len(), application: StyleApplication::Automatic("Comment".into()) });
                }
            } else {
                // The grammar's HTML range begins after an enclosing list
                // marker; that source prefix remains a list decoration.
                let end = if pre_ending { normalized.endings[line_index].normalized.end } else { range.end };
                let range = body_start..end;
                let fragment = markdown_html::block(normalized, range, revision);
                builder.output.push_str(fragment.text());
                builder.blocks.extend(fragment.blocks().iter().cloned().map(|mut block| {
                    block.range = block.range.start + output_start..block.range.end + output_start;
                    block.kind = presented_kind(block.kind.clone()); block.quote_depth = quote_depth; block.markdown_html = true; block
                }));
                builder.styles.extend(fragment.style_spans().iter().cloned().map(|mut span| { span.range = span.range.start + output_start..span.range.end + output_start; span }));
                builder.inline_images.extend(fragment.inline_images_for_region(&(0..fragment.text().len())).into_iter().map(|mut image| {
                    image.range = image.range.start + output_start..image.range.end + output_start; image
                }));
                for mut span in fragment.provenance() { span.formatted = span.formatted.start + output_start..span.formatted.end + output_start; builder.push_provenance(span); }
                for index in 0..fragment.hard_line_count().saturating_sub(1) { hard_breaks.push(output_start + fragment.hard_line_range(index).unwrap().end); }
            }
            if !pre_ending {
                if let Some(ending) = normalized.endings.get(line_index) { hard_breaks.push(builder.output.len()); builder.emit_unit_at(ending.normalized.start); }
            }
            // A still-open pre scope owns the final ending and its empty
            // trailing line; the fragment already supplied that paragraph.
            line_index = if pre_ending { input_lines.len() } else { line_index + 1 };
            continue;
        }
        if let Some(code) = source_at.and_then(|at| indented
            .get(indented.partition_point(|block| block.source.start <= at).saturating_sub(1))
            .filter(|block| block.source.start <= at && at <= block.source.end))
        {
            let first = line_index;
            while line_index < input_lines.len() {
                let row = &input_lines[line_index];
                let at = builder.unit_at(row.start).map_or(source_content_end, |unit| unit.source.start);
                if at > code.source.end { break; }
                let owner = &code.lines[(line_index - first).min(code.lines.len() - 1)];
                let content = normalized.unit_at_or_after_source(owner.content_start)
                    .map_or(row.end, |unit| unit.normalized.start).min(row.end);
                let output = builder.output.len();
                if owner.padding > 0 {
                    let index = normalized.unit_partition_point(|unit| unit.source.start < owner.content_start);
                    if let Some(unit) = index.checked_sub(1).and_then(|index| normalized.unit(index)) {
                        builder.output.push_str(&" ".repeat(owner.padding));
                        builder.push_provenance(ProvenanceSpan { formatted: output..builder.output.len(), source: unit.source.clone() });
                    }
                }
                builder.emit_range(content, row.end);
                if output == builder.output.len() {
                    builder.push_provenance(ProvenanceSpan { formatted: output..output, source: owner.content_start..owner.content_start });
                }
                builder.push_semantic_style(output, SemanticInlineStyle::Code);
                line_index += 1;
                if line_index - first >= code.lines.len() { break; }
                if let Some(ending) = normalized.endings.get(line_index - 1) {
                    hard_breaks.push(builder.output.len());
                    builder.emit_unit_at(ending.normalized.start);
                }
            }
            builder.blocks.push(Block::new(0, output_start..builder.output.len(), markdown_presented_kind(
                context.map_or(BlockKind::Paragraph, |context| context.kind.clone())),
                "Code Block".into(), None));
            if let Some(block) = builder.blocks.last_mut() {
                block.kind = presented_kind(block.kind.clone());
                block.quote_depth = quote_depth;
                block.code_language_label = true;
            }
            if let Some(ending) = normalized.endings.get(line_index - 1) {
                hard_breaks.push(builder.output.len());
                builder.emit_unit_at(ending.normalized.start);
            }
            continue;
        }
        let fence_start = context.filter(|context| context.code && context.marker.is_some()).map_or(semantic_start, |context| {
            normalized.unit_at_or_after_source(context.content_start)
                .map_or(line.end, |unit| unit.normalized.start).min(line.end)
        });
        let fence = syntax.code.get(syntax.code.partition_point(|code| code.range.end <= at))
            .filter(|code| code.fenced && code.range.start <= source_end && at < code.range.end);
        let front_matter = syntax.front_matter.as_ref().filter(|front| front.range.start == at);
        let fence_marker = front_matter.map(|_| (b'-', 3))
            .or_else(|| markdown_fence(&normalized.text[fence_start..line.end]));
        if let Some(((delimiter, length), fence)) = fence_marker.zip(fence) {
            let mut closing = line_index + 1;
            let mut closed = false;
            while closing < input_lines.len() {
                let source = builder.unit_at(input_lines[closing].start).map_or(source_content_end, |unit| unit.source.start);
                // The parser's half-open range omits the editable empty row
                // after a terminal ending in an unclosed fence.
                let terminal_body = source == source_content_end
                    && source == fence.range.end && input_lines[closing].is_empty();
                if source >= fence.range.end && !terminal_body { break; }
                let raw = &normalized.text[input_lines[closing].clone()];
                if front_matter.map_or_else(|| super::markdown_syntax::fence_close(raw, delimiter, length),
                    |front| source == front.body.end) {
                    closed = true;
                    break;
                }
                closing += 1;
            }
            let language = if front_matter.is_some() { Some("yaml".to_owned()) } else {
                normalized.text[fence_start..line.end].trim_start_matches([' ', '\t'])
                    [length..].split_whitespace().next().filter(|value| !value.is_empty())
                    .map(super::syntax::detection::canonical_language)
            };
            let after = (closing + usize::from(closed)).min(input_lines.len());
            let body_start = line_index + 1;
            let body_end = closing;
            if body_start < body_end {
                for index in body_start..body_end {
                    let output_start = builder.output.len();
                    let row = &input_lines[index];
                    let indent = normalized.text[fence_start..line.end].bytes().take_while(|byte| *byte == b' ').count();
                    let trim = normalized.text[row.clone()].bytes().take_while(|byte| *byte == b' ').take(indent).count();
                    builder.emit_range(row.start + trim, row.end);
                    if input_lines[index].is_empty() {
                        let at = builder
                            .unit_at(input_lines[index].start)
                            .map_or(source_content_end, |unit| unit.source.start);
                        builder.push_provenance(ProvenanceSpan {
                            formatted: output_start..output_start,
                            source: at..at,
                        });
                    }
                    builder.push_semantic_style(output_start, SemanticInlineStyle::Code);
                    if index + 1 < body_end {
                        if let Some(ending) = normalized.endings.get(index) {
                            hard_breaks.push(builder.output.len());
                            builder.emit_unit_at(ending.normalized.start);
                        }
                    }
                }
                builder.blocks.push(Block::new(0, output_start..builder.output.len(), markdown_presented_kind(
                        context.map_or(BlockKind::Paragraph, |context| context.kind.clone()),
                    ), "Code Block".into(), None));
            } else {
                let body_at = input_lines
                    .get(line_index + 1)
                    .map_or(line.end, |line| line.start);
                let source_at = builder
                    .unit_at(body_at)
                    .map_or(source_content_end, |unit| unit.source.start);
                builder.push_provenance(ProvenanceSpan {
                    formatted: output_start..output_start,
                    source: source_at..source_at,
                });
                builder.blocks.push(Block::new(0, output_start..output_start, markdown_presented_kind(
                        context.map_or(BlockKind::Paragraph, |context| context.kind.clone()),
                    ), "Code Block".into(), None));
            }
            if let Some(block) = builder.blocks.last_mut() {
                block.kind = presented_kind(block.kind.clone());
                block.quote_depth = quote_depth;
                block.code_language = language;
                block.code_language_label = true;
                block.code_kind = if front_matter.is_some() { CodeBlockKind::FrontMatter } else { CodeBlockKind::Ordinary };
            }
            if let Some(ending) = normalized.endings.get(after - 1) {
                hard_breaks.push(builder.output.len());
                builder.emit_unit_at(ending.normalized.start);
            }
            line_index = after;
            continue;
        }
        let (mut content_start, mut kind) =
            markdown_block_prefix(&normalized.text, semantic_start, line.end);
        if let Some(context) = context {
            kind = context.kind.clone();
            content_start = normalized
                .unit_at_or_after_source(context.content_start)
                .map_or(line.end, |unit| unit.normalized.start)
                .min(line.end);
        } else if matches!(kind, BlockKind::ListItem { .. }) {
            kind = BlockKind::Paragraph;
            content_start = semantic_start;
        }
        if context.is_none() {
            if let Some(paragraph) = syntax.paragraphs.get(syntax.paragraphs.partition_point(|range| range.end <= at))
                .filter(|range| at <= range.start && range.start <= source_end) {
                content_start = content_start.max(normalized_at(paragraph.start).min(line.end));
            }
        }
        let mut content_end = line.end;
        let mut heading = match kind { BlockKind::Heading(level) => Some(level), _ => None };
        let mut thematic_break = false;
        if let Some(syntax) = block_syntax {
            match syntax.role {
                super::markdown_syntax::BlockRole::Heading(level) => {
                    heading = Some(level);
                    content_start = normalized_at(syntax.content.start).min(line.end);
                    content_end = normalized_at(syntax.content.end).min(line.end);
                    if !matches!(kind, BlockKind::ListItem { .. }) { kind = BlockKind::Heading(level); }
                }
                super::markdown_syntax::BlockRole::Rule => {
                    thematic_break = true;
                    kind = BlockKind::Paragraph;
                    content_end = content_start;
                }
                _ => {}
            }
        }
        // Quote markers after a list prefix belong to the container too.
        if quoted && heading.is_none() { content_start += super::markdown_quotes::prefix(&normalized.text[content_start..content_end]); }
        kind = presented_kind(kind);
        let definition = syntax.definitions.get(syntax.definitions.partition_point(|range| range.end <= at)).is_some_and(|range| range.start <= source_end && at < range.end);
        if definition {
            let start = builder.output.len();
            builder.emit_range(content_start, content_end);
            if start < builder.output.len() { builder.styles.push(StyleSpan { range: start..builder.output.len(), application: StyleApplication::Automatic("Markdown reference".into()) }); }
        } else {
            if quoted {
                let limit = containers.iter().map(|container| normalized_at(container.range.end)).min().unwrap_or(content_end);
                let mut start = content_start;
                while let Some(image) = builder.multiline_image(start, content_end, limit, quote_context) {
                    let last = image.range.end;
                    while input_lines.get(line_index + 1).is_some_and(|next| next.start < last) { line_index += 1; }
                    content_end = input_lines[line_index].end;
                    start = last;
                }
            }
            // Inline grammar spans the whole prose paragraph, including
            // physical hard breaks. Parsing one visual hard line at a time
            // would turn valid emphasis, links and code spans into literals.
            if !thematic_break {
                while normalized.endings.get(line_index).is_some_and(|ending| {
                    let html = syntax.inline_html.get(syntax.inline_html.partition_point(|range| range.end <= ending.source.start))
                        .is_some_and(|range| range.start <= ending.source.start && ending.source.end < range.end);
                    html || explicit_breaks.contains(&ending.source.end)
                }) {
                    let Some(next) = input_lines.get(line_index + 1) else { break; };
                    line_index += 1;
                    content_end = next.end;
                }
            }
            let inline_start = builder.output.len();
            let provenance_start = builder.provenance.mark(&builder.output);
            builder.parse_inline(content_start, content_end);
            for span in builder.provenance.indivisible_since(provenance_start) {
                if span.formatted.start < inline_start { continue; }
                if span.formatted.len() == 1
                    && explicit_breaks.contains(&span.source.end)
                    && builder.output.as_bytes().get(span.formatted.start) == Some(&b'\n') {
                    hard_breaks.push(span.formatted.start);
                }
            }
        }
        // Setext underlines are part of the same heading; the formatted view
        // hides the whole annotation.
        if let Some(syntax) = block_syntax.filter(|syntax| matches!(syntax.role, super::markdown_syntax::BlockRole::Heading(_))) {
            while input_lines.get(line_index + 1).is_some_and(|next| {
                builder.unit_at(next.start).is_some_and(|unit| unit.source.start < syntax.range.end)
            }) {
                line_index += 1;
            }
        }
        if thematic_break {
            builder.push_provenance(ProvenanceSpan { formatted: output_start..output_start, source: at..at });
        }
        let output_end = builder.output.len();
        if output_start == output_end && (matches!(kind, BlockKind::ListItem { .. } | BlockKind::Heading(_)) || quoted) {
            // Structural prefixes are source syntax. Empty blocks retain the
            // boundary after their prefix, including an empty ATX heading.
            let content_at = builder.unit_at(content_start).map_or_else(
                || block_syntax.map_or_else(|| final_row.unwrap_or(source_content_end), |syntax| syntax.content.start),
                |unit| unit.source.start);
            let at = if heading.is_some() || quoted { content_at } else {
                context.map_or_else(|| quote.map_or(content_at, |quote| quote.content_start), |context| context.content_start)
            };
            builder.push_provenance(ProvenanceSpan {
                formatted: output_start..output_start,
                source: at..at,
            });
        }
        let style = if let Some(level) = heading { StyleId(format!("Heading{level}")) } else { match kind {
            BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
            BlockKind::Paragraph => "Paragraph".into(),
            BlockKind::ListItem { ordered, level, .. } => {
                StyleId(format!("{}{}", if ordered { "NumberedList" } else { "BulletedList" }, u16::from(level).min(3) + 1))
            }
        }};
        let loose = matches!(kind, BlockKind::ListItem { .. }) && list.is_some_and(|container| container.loose);
        let mut block = Block::new(0, output_start..output_end, kind, style, None);
        block.list_loose = loose;
        block.quote_depth = quote_depth;
        block.thematic_break = thematic_break;
        let first_rule = builder.inline_rules.partition_point(|at| *at < output_start);
        let rules: Vec<_> = builder.inline_rules[first_rule..].iter().copied()
            .take_while(|at| *at <= output_end).collect();
        if rules.is_empty() {
            builder.blocks.push(block);
        } else {
            let mut start = output_start;
            for at in rules {
                if start < at {
                    let mut before = block.clone();
                    before.range = start..at - 1;
                    builder.blocks.push(before);
                }
                let mut rule = Block::new(0, at..at, BlockKind::Paragraph, "Paragraph".into(), None);
                rule.quote_depth = quote_depth;
                rule.thematic_break = true;
                builder.blocks.push(rule);
                start = at + 1;
            }
            if start < output_end {
                block.range = start..output_end;
                builder.blocks.push(block);
            }
        }
        if let Some(ending) = normalized.endings.get(line_index) {
            hard_breaks.push(builder.output.len());
            builder.emit_unit_at(ending.normalized.start);
        }
        line_index += 1;
    }

    hard_breaks.append(&mut builder.inline_hard_breaks);
    hard_breaks.sort_unstable();
    hard_breaks.dedup();
    let text_len = builder.output.len();
    builder.styles.retain(|span| !span.range.is_empty() || !matches!(span.application, StyleApplication::Direct(_)));
    let mut projection = FormattedDocument::from_run_parts(
        revision,
        builder.output,
        builder.blocks,
        builder.styles,
        builder.provenance,
        builder.decoding_diagnostics,
        StyleSheet::for_format(Format::Markdown),
        source_content_start,
        source_content_end,
    );
    projection.inline_images = IntervalRangeStore::new(builder.inline_images);
    let mut start = 0;
    let mut lines = Vec::with_capacity(hard_breaks.len() + 1);
    for at in hard_breaks {
        lines.push(start..at);
        start = at + 1;
    }
    lines.push(start..text_len);
    projection.install_hard_line_partition(lines);
    if !tables.is_empty() {
        let mut presentation = Vec::new(); let mut index=0;
        for table in &tables {
            while index < projection.hard_lines.len() && projection.hard_lines.get(index).unwrap().range.start < table.range.start { presentation.push(projection.hard_lines.get(index).unwrap()); index+=1; }
            for row in &table.rows { presentation.push(HardLine { id:0,range:row.range.clone(),separator_length:1 }); }
            while index < projection.hard_lines.len() && projection.hard_lines.get(index).unwrap().range.start <= table.range.end { index+=1; }
        }
        while index < projection.hard_lines.len() { presentation.push(projection.hard_lines.get(index).unwrap()); index+=1; }
        projection.table_lines=Some(OrderedRangeStore::new(presentation));
    }
    // The construction vector can have nearly twice the final table count.
    // Release that spare storage before later projection passes, as the
    // previous exact-length table slice did.
    tables.shrink_to_fit();
    projection.tables=tables.into();
    projection
}

pub(super) fn markdown_fence(line: &str) -> Option<(u8, usize)> {
    let indent = line.bytes().take_while(|c| *c == b' ').count();
    if indent > 3 {
        return None;
    }
    let tail = &line[indent..];
    let delimiter = *tail.as_bytes().first()?;
    if !matches!(delimiter, b'`' | b'~') {
        return None;
    }
    let length = tail.bytes().take_while(|c| *c == delimiter).count();
    (length >= 3 && (delimiter != b'`' || !tail[length..].contains('`')))
        .then_some((delimiter, length))
}

pub(crate) fn markdown_block_prefix(text: &str, start: usize, end: usize) -> (usize, BlockKind) {
    let line = &text.as_bytes()[start..end];
    let hashes = line.iter().take_while(|byte| **byte == b'#').count();
    if (1..=6).contains(&hashes) && line.get(hashes) == Some(&b' ') {
        (start + hashes + 1, BlockKind::Heading(hashes as u8))
    } else {
        let indent = line.iter().take_while(|byte| **byte == b' ').count();
        let body = &line[indent..];
        let level = u8::try_from(indent / 2).unwrap_or(u8::MAX);
        if matches!(body.first(), Some(b'-' | b'+' | b'*')) && body.get(1) == Some(&b' ') {
            return (
                start + indent + 2,
                BlockKind::ListItem {
                    ordered: false,
                    ordinal: 1,
                    level,
                    container_start: false,
                    item_start: true,
                    marker_is_decoration: false,
                },
            );
        }
        let digits = body.iter().take_while(|byte| byte.is_ascii_digit()).count();
        if digits > 0
            && matches!(body.get(digits), Some(b'.' | b')'))
            && body.get(digits + 1) == Some(&b' ')
        {
            if let Ok(ordinal) = text[start + indent..start + indent + digits].parse() {
                return (
                    start + indent + digits + 2,
                    BlockKind::ListItem {
                        ordered: true,
                        ordinal,
                        level,
                        container_start: false,
                        item_start: true,
                        marker_is_decoration: false,
                    },
                );
            }
        }
        (start, BlockKind::Paragraph)
    }
}

struct MarkdownBuilder<'a> {
    in_table: bool,
    html_stack: Vec<(String, usize, StyleApplication, Option<usize>)>,
    html_raw_text: Option<String>,
    inline_syntax: std::collections::BTreeMap<usize, super::markdown_syntax::Inline>,
    reference_definitions: String,
    source_syntax: &'a super::markdown_syntax::Blocks,
    source_text: &'a str,
    input: &'a NormalizedText,
    output: String,
    inline_hard_breaks: Vec<usize>,
    inline_rules: Vec<usize>,
    inline_paragraph_start: usize,
    blocks: Vec<Block>,
    styles: Vec<StyleSpan>,
    inline_images: Vec<InlineImage>,
    provenance: provenance_runs::RunBuilder,
    decoding_diagnostics: Vec<DecodingDiagnostic>,
    encoding: super::Encoding,
    revision: Revision,
}

impl<'a> MarkdownBuilder<'a> {
    fn new(
        input: &'a NormalizedText,
        revision: Revision,
        source_syntax: &'a super::markdown_syntax::Blocks,
    ) -> Self {
        Self {
            in_table: false,
            html_stack: Vec::new(),
            html_raw_text: None,
            inline_syntax: Default::default(),
            reference_definitions: String::new(),
            source_syntax,
            source_text: &input.text,
            input,
            output: String::new(),
            inline_hard_breaks: Vec::new(),
            inline_rules: Vec::new(),
            inline_paragraph_start: 0,
            blocks: Vec::new(),
            styles: Vec::new(),
            inline_images: Vec::new(),
            provenance: Default::default(),
            decoding_diagnostics: Vec::new(),
            encoding: input.encoding,
            revision,
        }
    }

    fn parse_inline(&mut self, start: usize, end: usize) {
        self.html_raw_text = None;
        self.inline_paragraph_start = self.output.len();
        self.inline_syntax = super::markdown_syntax::inlines(self.source_text, start..end, &self.reference_definitions);
        let first_style = self.styles.len();
        let first_rule = self.inline_rules.len();
        self.parse_inline_depth(start, end, 0);
        // A terminal rule owns its empty paragraph without manufacturing a
        // following empty paragraph or an implicit final newline.
        if self.inline_rules.len() > first_rule && self.inline_rules.last().is_some_and(|rule| *rule + 1 == self.output.len()) {
            self.output.pop();
            self.inline_hard_breaks.pop();
            self.provenance.pop();
            let end = self.output.len();
            for span in &mut self.styles[first_style..] {
                span.range.start = span.range.start.min(end);
                span.range.end = span.range.end.min(end);
            }
            for (_, start, _, _) in &mut self.html_stack { *start = (*start).min(end); }
        }
        let entries: Vec<_> = self.html_stack.drain(..).collect();
        for (_, start, application, slot) in entries {
            self.finish_html_scope(start, self.output.len(), application, slot);
        }
    }

    fn parse_inline_depth(&mut self, start: usize, end: usize, depth: usize) {
        if depth >= 64 {
            self.emit_range_with_escapes(start, end);
            return;
        }
        let mut at = start;
        while at < end {
            if self.in_table && self.source_text[at..].starts_with('\\') {
                let slashes = self.source_text.as_bytes()[at..end].iter()
                    .take_while(|byte| **byte == b'\\').count();
                let pipe = at + slashes;
                if pipe < end && self.source_text.as_bytes()[pipe] == b'|' {
                    // Table grammar removes the final pipe escape first.
                    // Ordinary inline parsing pairs the remaining slashes;
                    // an odd remainder also escapes the literal pipe.
                    let pairs = (slashes - 1) / 2;
                    for pair in 0..pairs {
                        self.emit_escaped(at + pair * 2, at + pair * 2 + 1);
                    }
                    self.emit_escaped(at + pairs * 2, pipe);
                    at = pipe + 1;
                    continue;
                }
            }
            if self.source_text[at..].starts_with('\\') {
                if let Some(next) = self.next_boundary(at) {
                    if next < end {
                        let escaped = self.source_text[next..].chars().next().unwrap();
                        if escaped.is_ascii_punctuation() {
                            self.emit_escaped(at, next);
                            at = next + escaped.len_utf8();
                            continue;
                        }
                    }
                }
            }

            if let Some(inline) = self.inline_syntax.get(&at).filter(|inline| {
                if inline.range.end > end { return false; }
                if matches!(inline.kind, super::markdown_syntax::InlineKind::Link) {
                    // Prose folding can turn an invalid destination newline
                    // into a legal space. Only original grammar intervals can
                    // authorize a link in the disposable cooked projection.
                    let first = self.unit_at(inline.range.start).unwrap().source.start;
                    let last = self.unit_at(inline.range.end - 1).unwrap().source.end;
                    return self.source_syntax.recognizes_inline_link(&(first..last));
                }
                true
            }).cloned() {
                use super::markdown_syntax::InlineKind;
                let output = self.output.len();
                match inline.kind {
                    InlineKind::Image { destination, text, title, inline: is_inline } => {
                        let source = self.unit_at(at).unwrap().source.start..self.unit_at(inline.range.end - 1).unwrap().source.end;
                        self.output.push('\u{fffc}');
                        self.push_provenance(ProvenanceSpan { formatted: output..self.output.len(), source: source.clone() });
                        self.inline_images.push(InlineImage { range: output..self.output.len(), text, destination, title,
                            source, inline: is_inline,
                            width: None, height: None, html: false });
                    }
                    InlineKind::Reference => {
                        self.emit_range(at, inline.range.end);
                        self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Markdown reference".into()) });
                    }
                    InlineKind::Autolink => {
                        self.emit_range(inline.inner.start, inline.inner.end);
                        self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Link".into()) });
                    }
                    InlineKind::Link => {
                        self.parse_inline_depth(inline.inner.start, inline.inner.end, depth + 1);
                        if output < self.output.len() {
                            self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Link".into()) });
                        }
                    }
                    InlineKind::Emphasis | InlineKind::Strong | InlineKind::Strike => {
                        self.parse_inline_depth(inline.inner.start, inline.inner.end, depth + 1);
                        match inline.kind {
                            InlineKind::Emphasis => self.push_semantic_style(output, SemanticInlineStyle::Emphasis),
                            InlineKind::Strong => self.push_semantic_style(output, SemanticInlineStyle::Strong),
                            _ => self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Strikethrough".into()) }),
                        }
                    }
                    InlineKind::Html if markdown_inline_break_length(&self.source_text[inline.range.clone()]).is_some() => {
                        let first = self.unit_at(at).unwrap().source.start;
                        let last = self.unit_at(inline.range.end - 1).unwrap().source.end;
                        self.inline_hard_breaks.push(output); self.output.push('\n');
                        self.push_provenance(ProvenanceSpan { formatted: output..self.output.len(), source: first..last });
                    
                    }
                    InlineKind::Html => {
                        // Comments remain editable, visible source by product policy.
                        if self.source_text[inline.range.clone()].starts_with("<!--") {
                            self.emit_range(at, inline.range.end);
                            // A newline inside a visible inline comment is
                            // literal comment content and a hard line, just as
                            // in a comment recognized as an HTML block.
                            let first = self.input.endings.partition_point(|ending| ending.normalized.end <= at);
                            self.inline_hard_breaks.extend(self.input.endings[first..].iter()
                                .take_while(|ending| ending.normalized.start < inline.range.end)
                                .map(|ending| output + ending.normalized.start - at));
                            self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Comment".into()) });
                        } else {
                            self.html_tag(inline.range.clone());
                        }
                    }
                }
                at = inline.range.end;
                continue;
            }

            if let Some((length, character)) = markdown_inline_character_reference(&self.source_text[at..end]) {
                if let (Some(first), Some(last)) = (self.unit_at(at), self.unit_at(at + length - 1)) {
                    let source = first.source.start..last.source.end;
                    let output_start = self.output.len();
                    self.output.push_str(&character);
                    self.push_provenance(ProvenanceSpan {
                        formatted: output_start..self.output.len(), source,
                    });
                    at += length;
                    continue;
                }
            }
            if let Some(length) = markdown_inline_break_length(&self.source_text[at..end]) {
                if let (Some(first), Some(last)) = (self.unit_at(at), self.unit_at(at + length - 1)) {
                    let source = first.source.start..last.source.end;
                    let output_start = self.output.len();
                    self.inline_hard_breaks.push(output_start);
                    self.output.push('\n');
                    self.push_provenance(ProvenanceSpan {
                        formatted: output_start..self.output.len(), source,
                    });
                    at += length;
                    continue;
                }
            }
        

            if self.source_text[at..].starts_with('`') {
                let length = self.source_text.as_bytes()[at..end]
                    .iter()
                    .take_while(|byte| **byte == b'`')
                    .count();
                let mut probe = at + length;
                let mut close = None;
                while probe < end {
                    if self.source_text.as_bytes()[probe] == b'`' {
                        let run = self.source_text.as_bytes()[probe..end]
                            .iter()
                            .take_while(|byte| **byte == b'`')
                            .count();
                        if run == length {
                            close = Some(probe);
                            break;
                        }
                        probe += run;
                    } else {
                        probe += self.source_text[probe..].chars().next().unwrap().len_utf8();
                    }
                }
                if let Some(close) = close {
                    let output_start = self.output.len();
                    let mut body = at + length..close;
                    let text = &self.source_text[body.clone()];
                    if text.starts_with(' ') && text.ends_with(' ') && text.bytes().any(|byte| byte != b' ') {
                        body.start += 1;
                        body.end -= 1;
                    }
                    if self.in_table {
                        let mut point=body.start;
                        while point<body.end { if self.source_text[point..body.end].starts_with("\\|") { self.emit_escaped(point,point+1); point+=2; } else { self.emit_unit_at(point); point=self.next_boundary(point).unwrap_or(body.end); } }
                    } else { self.emit_range(body.start, body.end); }
                
                    self.push_semantic_style(output_start, SemanticInlineStyle::Code);
                    if output_start < self.output.len() {
                        self.styles.push(StyleSpan {
                            range: output_start..self.output.len(),
                            application: StyleApplication::Named("Code".into()),
                        });
                    }
                    at = close + length;
                    continue;
                }
                self.emit_range(at, at + length);
                at += length;
                continue;
            }

            if let Some((finish, _)) = super::markdown_syntax::autolink(self.source_text, at, end) {
                let output = self.output.len();
                self.emit_range(at, finish);
                self.styles.push(StyleSpan { range: output..self.output.len(), application: StyleApplication::Automatic("Link".into()) });
                at = finish;
                continue;
            }

            self.emit_unit_at(at);
            at = self.next_boundary(at).unwrap_or(end);
        }
    }

    fn emit_range_with_escapes(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            if self.source_text[at..].starts_with('\\') {
                if let Some(next) = self.next_boundary(at) {
                    if next < end {
                        let escaped = self.source_text[next..].chars().next().unwrap();
                        if escaped.is_ascii_punctuation() {
                            self.emit_escaped(at, next);
                            at = next + escaped.len_utf8();
                            continue;
                        }
                    }
                }
            }
            self.emit_unit_at(at);
            at = self.next_boundary(at).unwrap_or(end);
        }
    }

    fn emit_range(&mut self, start: usize, end: usize) {
        let mut at = start;
        while at < end {
            self.emit_unit_at(at);
            at = self.next_boundary(at).unwrap_or(end);
        }
    }

    fn emit_escaped(&mut self, slash: usize, escaped: usize) {
        let Some(slash_source_start) = self.unit_at(slash).map(|unit| unit.source.start) else {
            return;
        };
        let Some((normalized, source_end)) = self
            .unit_at(escaped)
            .map(|unit| (unit.normalized.clone(), unit.source.end))
        else {
            return;
        };
        let output_start = self.output.len();
        self.output.push_str(&self.source_text[normalized]);
        self.push_provenance(ProvenanceSpan {
            formatted: output_start..self.output.len(),
            source: slash_source_start..source_end,
        });
    }

    fn emit_unit_at(&mut self, normalized: usize) {
        let Some((normalized_range, source, decoding_diagnostic)) =
            self.unit_at(normalized).map(|unit| {
                (
                    unit.normalized.clone(),
                    unit.source.clone(),
                    unit.decoding_diagnostic,
                )
            })
        else {
            return;
        };
        let output_start = self.output.len();
        self.output.push_str(&self.source_text[normalized_range]);
        let formatted = output_start..self.output.len();
        self.push_provenance(ProvenanceSpan {
            formatted: formatted.clone(),
            source: source.clone(),
        });
        if let Some(kind) = decoding_diagnostic {
            self.decoding_diagnostics.push(DecodingDiagnostic {
                revision: self.revision,
                encoding: self.encoding,
                kind,
                source_range: source,
                formatted_range: formatted,
            });
        }
    }

    fn push_semantic_style(&mut self, output_start: usize, style: SemanticInlineStyle) {
        if output_start != self.output.len() {
            self.styles.push(StyleSpan {
                range: output_start..self.output.len(),
                application: StyleApplication::Semantic(style),
            });
        }
    }

    fn push_provenance(&mut self, span: ProvenanceSpan) {
        self.provenance.push(span, &self.output);
    }

    fn unit_at(&self, normalized: usize) -> Option<LogicalUnit> {
        self.input.unit_starting_at(normalized)
    }

    fn next_boundary(&self, normalized: usize) -> Option<usize> {
        self.unit_at(normalized).map(|unit| unit.normalized.end)
    }

}

/// The native inline HTML spelling for a hard break. Escapes and code spans
/// are consumed before their contents reach this recognizer; attributes and
/// other HTML remain literal source content in this Markdown adapter.
pub(super) fn markdown_inline_break_length(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if !bytes.get(..3)?.eq_ignore_ascii_case(b"<br") { return None; }
    let mut at = 3;
    while matches!(bytes.get(at), Some(b' ' | b'\t')) { at += 1; }
    if bytes.get(at) == Some(&b'/') {
        at += 1;
        while matches!(bytes.get(at), Some(b' ' | b'\t')) { at += 1; }
    }
    if bytes.get(at) == Some(&b'>') { return Some(at + 1); }
    if !matches!(bytes.get(3), Some(b' ' | b'\t' | b'\n' | b'\r')) { return None; }
    let tokens = super::html::tokenize(text);
    tokens.first().and_then(|token| match &token.kind {
        super::html::TokenKind::Tag(tag) if tag.name == "br" && !tag.end => Some(token.range.end),
        _ => None,
    })
}

/// Markdown references require the semicolon (unlike HTML text recovery).
fn markdown_inline_character_reference(text: &str) -> Option<(usize, String)> {
    super::html::markdown_reference(text).map(|(value, length)| (length, value))
}

pub(crate) fn escape_markdown_insert(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '_' | '`' | '#' | '>' | '~' | '[' | ']' | '<' | '&') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Markdown prose and link destinations can represent an otherwise
/// unencodable scalar with a character reference. Code spans, code blocks and
/// source-visible text must not use this: references are literal there.
pub(crate) fn markdown_character_references(text: &str, encoding: super::Encoding) -> String {
    if encoding != super::Encoding::Latin1 {
        return text.to_owned();
    }
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch as u32 > 0xff {
            escaped.push_str(&format!("&#x{:X};", ch as u32));
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

pub(crate) fn escape_markdown_insert_in_encoding(text: &str, encoding: super::Encoding) -> String {
    markdown_character_references(&escape_markdown_insert(text), encoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::history::History;
    use crate::document::line_endings::{normalize, FileFormat};
    use crate::document::{Encoding, Splice};

    #[test]
    fn indexed_interior_and_hidden_source_queries_match_contributor_scans() {
        let sources = [
            "# Title\n\nPara **bold** and _it_ &amp; `code` \\* x\n> quote\n> more\n\n- a\n- b\n  continued\n\n1. one\n2. two\n",
            "a  \nb\\\nc<br>d\n\n\n\n***\n\n[ref]: https://e.com\n\n[link](https://e.com) ![img](p.png)\n",
            "| a | b |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn x() {}\n```\n\n    indented\n\n<b>html</b> &#x41; &copy;\n",
            "\u{e9}t\u{e9} \u{4e2d}\u{6587} **\u{1f600}**\r\n\r\nnext\r\n",
        ];
        let mut gaps = 0;
        for source in sources {
            for (encoding, format) in [(Encoding::Utf8, Format::Markdown), (Encoding::Utf16Le, Format::Markdown)] {
                let bytes = encoding.encode_fragment(source).unwrap();
                let document = crate::document::Document::from_bytes(bytes.clone(), encoding, format).unwrap();
                let projection = document.projection();
                let spans = projection.provenance();
                for at in 0..=bytes.len() {
                    let interior = spans.iter()
                        .find(|span| span.source.start < at && at < span.source.end).cloned();
                    assert_eq!(projection.provenance.containing_source(at, &projection.text), interior,
                        "{source:?} {format:?} {encoding:?} at {at}");
                    let edge = spans.iter().any(|span| span.source.start == at || span.source.end == at);
                    if interior.is_some() || edge || !projection.source_ordered { continue; }
                    // The formatted-order frontier scan this index replaces.
                    let mut start = projection.source_content_start;
                    let mut upstream = None;
                    let mut expected = None;
                    for span in projection.provenance.stored() {
                        if start < span.source.start && start <= at && at <= span.source.start {
                            expected = Some((start..span.source.start, upstream, Some(span.formatted.start)));
                            break;
                        }
                        if span.source.end > start { start = span.source.end; upstream = Some(span.formatted.end); }
                    }
                    let expected = expected.or_else(|| (start < projection.source_content_end && start <= at
                        && at <= projection.source_content_end).then_some((start..projection.source_content_end, upstream, None)));
                    assert_eq!(projection.hidden_gap(at), expected, "{source:?} {format:?} {encoding:?} at {at}");
                    gaps += 1;
                }
            }
        }
        assert!(gaps > 100, "{gaps} hidden positions compared");
    }

    #[test]
    fn literal_grapheme_ranges_stream_long_latin1_and_utf16_lines() {
        // A literal line is one contributor. Each boundary maps through the
        // text tree's UTF-16 aggregates; re-encoding the line's prefix for
        // every grapheme made this walk quadratic in the line length.
        for encoding in [Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            let unit = if encoding == Encoding::Latin1 { "aé" } else { "aé😀" };
            let line = unit.repeat(40_000);
            let bytes = encoding.encode_fragment(&line).unwrap();
            let decoded = encoding.decode(&bytes).unwrap();
            let normalized = super::super::line_endings::normalize_literal(&decoded, FileFormat::Unix);
            let projection = project_plain(&normalized, Revision(1), decoded.bom_len, bytes.len());
            let mut source = 0;
            let mut graphemes = 0;
            for (_, item, range) in projection.source_grapheme_ranges() {
                let width = encoding.encoded_text_len(item);
                assert_eq!(range, Some(source..source + width), "{encoding:?} at source {source}");
                source += width;
                graphemes += 1;
            }
            assert_eq!((graphemes, source), (line.chars().count(), bytes.len()), "{encoding:?}");
        }
    }

    #[test]
    fn compact_literal_mapping_matches_dense_oracle_at_every_source_boundary() {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            let body = if encoding == Encoding::Latin1 { "aé\r\nz\rb\n" } else { "aé😀e\u{301}\r\nz\rb\n" };
            let mut bytes = encoding.bom_bytes().to_vec();
            bytes.extend(encoding.encode_fragment(body).unwrap());
            match encoding {
                Encoding::Utf8 => bytes.extend([0xff, b'x', 0xe2, 0x82]),
                Encoding::Utf16Le => bytes.extend([0x00, 0xd8, b'x', 0, 0x91]),
                Encoding::Utf16Be => bytes.extend([0xd8, 0x00, 0, b'x', 0x91]),
                Encoding::Latin1 => (),
            }
            let decoded = encoding.decode(&bytes).unwrap();
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let compact = super::super::line_endings::normalize_literal(&decoded, format);
                let dense = normalize(&decoded, format);
                let compact = project_plain(&compact, Revision(7), decoded.bom_len, bytes.len());
                let dense = project_plain(&dense, Revision(7), decoded.bom_len, bytes.len());
                assert_eq!(compact.text(), dense.text());
                for source in 0..=bytes.len() {
                    for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                        assert_eq!(compact.map_source_boundary(Revision(7), source, affinity),
                            dense.map_source_boundary(Revision(7), source, affinity), "{encoding:?} {format:?} source={source}");
                    }
                }
                let boundaries = compact.text().char_indices().map(|(at, _)| at)
                    .chain(std::iter::once(compact.text().len())).collect::<Vec<_>>();
                for &start in &boundaries {
                    for &end in boundaries.iter().filter(|&&end| end >= start) {
                        assert_eq!(compact.source_range(start..end), dense.source_range(start..end));
                    }
                }
            }
        }
    }

    #[test]
    fn many_literal_hard_lines_do_not_create_scalar_or_line_mapping_tables() {
        let source = "x\n".repeat(100_000);
        let document = crate::document::Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        let projection = document.projection();
        assert_eq!(projection.hard_line_count(), 100_001);
        assert!(projection.provenance.runs.len() <= source.len().div_ceil(super::super::encoding::MAPPING_CHUNK_BYTES));
        assert!(projection.provenance.boundaries.as_ref().is_none_or(|boundaries| boundaries.len() <= projection.provenance.runs.len() * 2));
        assert_eq!(projection.source_range(12345..12346), Some(12345..12346));
    }

    #[test]
    fn compact_literal_edit_windows_match_dense_source_patches_across_chunks() {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            for file_format in [FileFormat::Unix, FileFormat::Dos] {
                let body = if encoding == Encoding::Latin1 { "éabcd\r\n" } else { "é😀e\u{301}abcd\r\n" };
                let mut source = encoding.bom_bytes().to_vec();
                source.extend(encoding.encode_fragment(&body.repeat(900)).unwrap());
                let mut document = crate::document::Document::from_bytes_with_file_format(
                    source.clone(), encoding, Format::PlainText, file_format).unwrap();
                let mut seed = 31_u64;
                for step in 0..24 {
                    let decoded = encoding.decode(&source).unwrap();
                    let normalized = normalize(&decoded, file_format);
                    let oracle = project_plain(&normalized, document.revision(), decoded.bom_len, source.len());
                    let boundaries = oracle.text().char_indices().map(|(at, _)| at)
                        .chain(std::iter::once(oracle.text().len()))
                        .filter(|&at| oracle.is_logical_grapheme_boundary(at).unwrap()).collect::<Vec<_>>();
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let index = seed as usize % boundaries.len();
                    let end = (index + step % 3).min(boundaries.len() - 1);
                    let range = boundaries[index]..boundaries[end];
                    let replacement = if step % 5 == 0 { "\n" } else { "é" };
                    let patch = oracle.source_range(range.clone()).unwrap();
                    let physical = if replacement == "\n" && file_format == FileFormat::Dos { "\r\n" } else { replacement };
                    source.splice(patch, encoding.encode_fragment(physical).unwrap());
                    document.replace(range, replacement).unwrap();
                    assert_eq!(document.source_bytes(), source, "{encoding:?} {file_format:?} step={step}");
                    let decoded = encoding.decode(&source).unwrap();
                    let normalized = normalize(&decoded, file_format);
                    assert_eq!(document.text(), normalized.text);
                    let source_at = source.len() / 2;
                    let reopened = project_plain(&normalized, document.revision(), decoded.bom_len, source.len());
                    assert_eq!(document.projection().map_source_boundary(document.revision(), source_at, BoundaryAffinity::Downstream),
                        reopened.map_source_boundary(document.revision(), source_at, BoundaryAffinity::Downstream));
                }
            }
        }
    }

    fn markdown_at(source: &str, revision: Revision) -> FormattedDocument {
        let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
        let normalized = normalize(&decoded, FileFormat::Unix);
        project(&normalized, Format::Markdown, revision, 0, source.len())
    }

    fn markdown(source: &str) -> FormattedDocument {
        markdown_at(source, Revision(1))
    }

    #[test]
    fn markdown_front_matter_projects_literal_yaml_with_hidden_delimiters() {
        let source = "---\nname: **literal**\ndescription: [unfinished\n\n----\n> literal\n---\n\n# Heading";
        let projected = markdown(source);
        let expected = "name: **literal**\ndescription: [unfinished\n\n----\n> literal\nHeading";
        assert_eq!(projected.text(), expected);
        assert_eq!(projected.blocks().len(), 2);
        let metadata = &projected.blocks()[0];
        assert!(metadata.is_front_matter());
        assert_eq!(metadata.style.0, "Code Block");
        assert_eq!(metadata.code_language.as_deref(), Some("yaml"));
        assert!(metadata.code_language_label);
        assert_eq!(metadata.quote_depth, 0);
        assert!(!projected.blocks()[1].is_front_matter());
        assert_eq!(projected.blocks()[1].kind, BlockKind::Heading(1));
        assert!(projected.default_code_language(0).is_none());
        assert_eq!(projected.map_source_boundary(Revision(1), 4, BoundaryAffinity::Downstream)
            .unwrap().formatted_offset, 0);
    }

    #[test]
    fn markdown_front_matter_empty_body_has_an_owned_caret_boundary() {
        for source in ["---\n---", "---\n\n---"] {
            let projected = markdown(source);
            assert_eq!(projected.text(), "");
            assert_eq!(projected.blocks().len(), 1);
            assert!(projected.blocks()[0].is_front_matter());
            assert_eq!(projected.blocks()[0].range, 0..0);
            assert_eq!(projected.map_source_boundary(Revision(1), 4, BoundaryAffinity::Downstream)
                .unwrap().formatted_offset, 0);
        }
    }

    #[test]
    fn markdown_front_matter_recognition_is_disabled_in_nonleading_bands() {
        let source = "---\nname: Example\n---";
        let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
        let normalized = normalize(&decoded, FileFormat::Unix);
        let projected = project_band(&normalized, Format::Markdown, Revision(1), 0, source.len(), false);
        assert!(projected.blocks().iter().all(|block| !block.is_front_matter()));
        assert!(projected.blocks().iter().any(|block| block.thematic_break));
        assert!(projected.blocks().iter().any(|block| block.kind == BlockKind::Heading(2)));
    }

    #[test]
    fn markdown_front_matter_preserves_normalized_encoded_body_provenance() {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let ending = match file_format { FileFormat::Unix => "\n", FileFormat::Dos => "\r\n", FileFormat::Mac => "\r" };
            let source = ["---", "name: café", "second: true", "---"].join(ending);
            let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
            let normalized = normalize(&decoded, file_format);
            let projected = project(&normalized, Format::Markdown, Revision(1), 0, source.len());
            assert_eq!(projected.text(), "name: café\nsecond: true");
            assert!(projected.blocks()[0].is_front_matter());
            assert_eq!(projected.map_source_boundary(Revision(1), 3 + ending.len(), BoundaryAffinity::Downstream)
                .unwrap().formatted_offset, 0);
        }
    }

    #[test]
    fn empty_caret_seeds_do_not_reorder_real_source_contributors() {
        let projected = markdown("- foo\n-   \n- bar\n");
        assert!(projected.source_ordered);
        for provenance in [
            vec![
                ProvenanceSpan { formatted: 0..1, source: 2..3 },
                ProvenanceSpan { formatted: 1..2, source: 0..1 },
            ],
            vec![
                ProvenanceSpan { formatted: 0..1, source: 2..3 },
                ProvenanceSpan { formatted: 1..1, source: 0..1 },
                ProvenanceSpan { formatted: 1..2, source: 3..4 },
            ],
        ] {
            let projected = FormattedDocument::from_parts(
                Revision(1), "ab".to_owned(), blocks_for_plain_text("ab"), Vec::new(),
                provenance, Vec::new(), StyleSheet::default(), 0, 4,
            );
            assert!(!projected.source_ordered);
        }
    }

    fn plain(text: String, revision: Revision) -> FormattedDocument {
        let length = text.len();
        FormattedDocument::from_parts(
            revision,
            text.clone(),
            blocks_for_plain_text(&text),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            StyleSheet::default(),
            0,
            length,
        )
    }

    fn add_direct_assignments(document: &mut FormattedDocument) {
        document.document_style.direct_canvas.padding_left = Some(17.0);
        document.document_style.direct_default_character.size = Some(16.0.into());
        let mut blocks = document.blocks.to_vec();
        blocks[0].direct_paragraph.margin_bottom = Some(9.0);
        blocks[0].direct_default_character.font_families = Some(vec!["Assigned Serif".to_owned()]);
        document.blocks = OrderedRangeStore::new(blocks);
    }

    #[test]
    fn many_separate_paragraph_splits_inherit_direct_properties_locally() {
        const COUNT: usize = 5_000;
        let old_text = "a\n".repeat(COUNT);
        let new_text = "a\nb\n".repeat(COUNT);
        let mut before = plain(old_text.clone(), Revision(1));
        let next_id = before.assign_initial_block_ids(1).unwrap();
        let mut blocks = before.blocks().to_vec();
        for block in &mut blocks {
            block.direct_paragraph.margin_bottom = Some(9.0);
        }
        before.install_paragraph_partition(blocks);
        let mut after = plain(new_text.clone(), Revision(2));
        let edits = (0..COUNT)
            .map(|index| TextEdit::new(index * 2..index * 2 + 1, "a\nb"))
            .collect::<Vec<_>>();
        let map = PositionMap::for_text_snapshots(
            DocumentId(1),
            Revision(1),
            Revision(2),
            &before,
            &after,
            edits
                .iter()
                .map(|edit| {
                    super::super::Splice::new(edit.range.clone(), edit.replacement.len()).unwrap()
                })
                .collect(),
        )
        .unwrap();
        after
            .install_reconciled_block_ids(&before, &edits, &map, next_id)
            .unwrap();
        assert_eq!(after.blocks().len(), COUNT * 2 + 1);
        assert!(after
            .blocks()
            .iter()
            .all(|block| block.direct_paragraph.margin_bottom == Some(9.0)));
    }

    #[test]
    fn batched_grapheme_provenance_matches_checked_point_queries() {
        for (format, source) in [
            (
                Format::PlainText,
                "A\r\nB e\u{301} क्‍ष 👩🏽‍💻 🇺🇸🇨🇦\n\u{301}end",
            ),
            (
                Format::Markdown,
                "# e\u{301} **bold**\n\n- item\n  continuation\n\n```\ncode\n\n```",
            ),

        ] {
            let document = super::super::Document::from_bytes(
                source.as_bytes().to_vec(),
                super::super::Encoding::Utf8,
                format,
            )
            .unwrap();
            let projection = document.projection();
            let expected = projection
                .text()
                .grapheme_indices(true)
                .map(|(at, item)| (at, item, projection.source_range(at..at + item.len())))
                .collect::<Vec<_>>();
            assert_eq!(
                projection.source_grapheme_ranges().collect::<Vec<_>>(),
                expected,
                "{format:?}"
            );
        }
        for spans in [
            vec![],
            vec![
                ProvenanceSpan {
                    formatted: 0..2,
                    source: 0..2,
                },
                ProvenanceSpan {
                    formatted: 1..1,
                    source: 1..1,
                },
            ],
        ] {
            let base = plain("ab".into(), Revision(0));
            let projection = FormattedDocument::from_parts(
                Revision(0),
                "ab".into(),
                base.blocks().to_vec(),
                vec![],
                spans,
                vec![],
                StyleSheet::default(),
                0,
                2,
            );
            let expected = projection
                .text()
                .grapheme_indices(true)
                .map(|(at, item)| (at, item, projection.source_range(at..at + item.len())))
                .collect::<Vec<_>>();
            assert_eq!(
                projection.source_grapheme_ranges().collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn local_contributor_lookup_includes_whole_graphemes_and_reordered_children() {
        let document = super::super::Document::from_bytes("before\n\ne\u{301} after".as_bytes().to_vec(),
            super::super::Encoding::Utf8, Format::Markdown).unwrap();
        let projection = document.projection();
        let ranges = projection.source_grapheme_lookup_ranges([9..11]).unwrap();
        assert_eq!(ranges.len(), 1);
        let items = projection.source_grapheme_ranges_in(&ranges[0]).unwrap();
        assert_eq!(items, vec![(7, "e\u{301}".into(), Some(8..11))]);

        let base = plain("ABC".into(), Revision(0));
        let projection = FormattedDocument::from_parts(Revision(0), "ABC".into(), base.blocks().to_vec(),
            vec![], vec![ProvenanceSpan { formatted: 0..1, source: 2..3 },
                ProvenanceSpan { formatted: 1..2, source: 0..1 },
                ProvenanceSpan { formatted: 2..3, source: 1..2 }],
            vec![], StyleSheet::default(), 0, 3);
        assert_eq!(projection.source_grapheme_lookup_ranges([0..2]).unwrap(), vec![1..3]);
        assert_eq!(projection.source_grapheme_ranges_in(&(1..3)).unwrap(),
            vec![(1, "B".into(), Some(0..1)), (2, "C".into(), Some(1..2))]);
    }

    #[test]
    fn plain_and_markdown_project_default_root_and_paragraph_assignments() {
        let plain = plain("plain".to_owned(), Revision(1));
        let markdown = markdown("# heading\nbody");

        for projection in [&plain, &markdown] {
            assert_eq!(
                projection.document_style.style,
                projection.style_sheet.base_paragraph
            );
            assert_eq!(
                projection.document_style.direct_canvas,
                BlockProperties::default()
            );
            assert_eq!(
                projection.document_style.direct_default_character,
                CharacterProperties::default()
            );
            assert!(projection.blocks().iter().all(|block| {
                block.direct_paragraph == BlockProperties::default()
                    && block.direct_default_character == CharacterProperties::default()
            }));
        }
        assert_eq!(plain.blocks()[0].style, plain.style_sheet.base_paragraph);
        assert_eq!(markdown.blocks()[0].style, StyleId::from("Heading1"));
        assert_eq!(
            markdown.blocks()[1].style,
            markdown.style_sheet.base_paragraph
        );
    }

    #[test]
    fn unchanged_projection_and_history_preserve_all_direct_assignments() {
        let mut previous = plain("first\nsecond".to_owned(), Revision(1));
        previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_root = previous.document_style.clone();
        let expected_block = previous.blocks()[0].clone();

        let mut candidate = plain("first\nsecond".to_owned(), Revision(2));
        candidate.install_unchanged_block_ids(&previous).unwrap();
        assert_eq!(candidate.document_style, expected_root);
        assert_eq!(candidate.blocks()[0], expected_block);

        let mut history = History::new(previous);
        history.commit(candidate, false);
        assert!(history.undo());
        assert_eq!(history.current().document_style, expected_root);
        assert_eq!(history.current().blocks()[0], expected_block);
        assert!(history.redo());
        assert_eq!(history.current().document_style, expected_root);
        assert_eq!(history.current().blocks()[0], expected_block);
    }

    #[test]
    fn text_reconciliation_carries_assignments_with_the_retained_block() {
        let old_text = "first\nsecond".to_owned();
        let new_text = "prefix first\nsecond".to_owned();
        let mut previous = plain(old_text.clone(), Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_root = previous.document_style.clone();
        let expected_id = previous.blocks()[0].id;
        let expected_paragraph = previous.blocks()[0].direct_paragraph.clone();
        let expected_character = previous.blocks()[0].direct_default_character.clone();
        let mut candidate = plain(new_text.clone(), Revision(2));
        let edits = vec![TextEdit::new(0..0, "prefix ")];
        let map = PositionMap::for_text(
            DocumentId(1),
            Revision(1),
            Revision(2),
            &old_text,
            &new_text,
            vec![Splice::new(0..0, 7).unwrap()],
        )
        .unwrap();

        candidate
            .install_reconciled_block_ids(&previous, &edits, &map, next_id)
            .unwrap();
        assert_eq!(candidate.document_style, expected_root);
        assert_eq!(candidate.blocks()[0].id, expected_id);
        assert_eq!(candidate.blocks()[0].direct_paragraph, expected_paragraph);
        assert_eq!(
            candidate.blocks()[0].direct_default_character,
            expected_character
        );
    }

    #[test]
    fn paragraph_splits_inherit_direct_assignments_at_start_middle_and_end() {
        for (old_text, new_text, edit) in [
            ("styled", "\nstyled", TextEdit::new(0..0, "\n")),
            ("styled", "sty\nled", TextEdit::new(3..3, "\n")),
            ("styled", "styled\n", TextEdit::new(6..6, "\n")),
        ] {
            let mut previous = plain(old_text.to_owned(), Revision(1));
            let next_id = previous.assign_initial_block_ids(1).unwrap();
            add_direct_assignments(&mut previous);
            let old_id = previous.blocks()[0].id;
            let expected_paragraph = previous.blocks()[0].direct_paragraph.clone();
            let expected_character = previous.blocks()[0].direct_default_character.clone();
            let mut candidate = plain(new_text.to_owned(), Revision(2));
            let map = PositionMap::for_text(
                DocumentId(1),
                Revision(1),
                Revision(2),
                old_text,
                new_text,
                vec![Splice::new(edit.range.clone(), edit.replacement.len()).unwrap()],
            )
            .unwrap();

            candidate
                .install_reconciled_block_ids(&previous, &[edit], &map, next_id)
                .unwrap();
            assert_eq!(candidate.blocks.len(), 2);
            assert!(candidate.blocks().iter().all(|block| {
                block.direct_paragraph == expected_paragraph
                    && block.direct_default_character == expected_character
            }));
            let retained_index = if new_text.starts_with('\n') { 1 } else { 0 };
            assert_eq!(candidate.blocks()[retained_index].id, old_id);
        }
    }

    #[test]
    fn split_inheritance_does_not_replace_projected_markdown_block_styles() {
        let old_text = "title";
        let new_text = "title\nbody";
        let mut previous = markdown_at("# title", Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_paragraph = previous.blocks()[0].direct_paragraph.clone();
        let expected_character = previous.blocks()[0].direct_default_character.clone();
        let mut candidate = markdown_at("# title\nbody", Revision(2));
        let edit = TextEdit::new(5..5, "\nbody");
        let map = PositionMap::for_text(
            DocumentId(1),
            Revision(1),
            Revision(2),
            old_text,
            new_text,
            vec![Splice::new(edit.range.clone(), edit.replacement.len()).unwrap()],
        )
        .unwrap();

        candidate
            .install_reconciled_block_ids(&previous, &[edit], &map, next_id)
            .unwrap();
        assert_eq!(candidate.blocks()[0].style, StyleId::from("Heading1"));
        assert_eq!(candidate.blocks()[1].style, StyleId::from("Paragraph"));
        assert!(candidate.blocks().iter().all(|block| {
            block.direct_paragraph == expected_paragraph
                && block.direct_default_character == expected_character
        }));
    }

    #[test]
    fn unchanged_source_backed_block_may_retain_id_when_kind_changes() {
        let mut previous = plain("title\nbody".to_owned(), Revision(1));
        previous.assign_initial_block_ids(1).unwrap();
        let mut candidate = plain("title\nbody".to_owned(), Revision(2));
        let mut blocks = candidate.blocks.to_vec();
        blocks[0].kind = BlockKind::Heading(1);
        blocks[0].style = "Heading1".into();
        candidate.blocks = OrderedRangeStore::new(blocks);

        candidate.install_unchanged_block_ids(&previous).unwrap();
        assert_eq!(candidate.blocks()[0].id, previous.blocks()[0].id);
        assert_eq!(candidate.blocks()[0].kind, BlockKind::Heading(1));
    }

    #[test]
    fn large_block_reconciliation_preserves_all_shifted_ids_without_pairwise_diffing() {
        const LINES: usize = 100_000;
        let old_text = format!("{}tail", "x\n".repeat(LINES));
        let new_text = format!("new\n{old_text}");
        let mut previous = plain(old_text.clone(), Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        let previous_last = previous.blocks().last().unwrap().id;
        let mut candidate = plain(new_text.clone(), Revision(2));
        let edits = vec![TextEdit::new(0..0, "new\n")];
        let map = PositionMap::for_text(
            DocumentId(1),
            Revision(1),
            Revision(2),
            &old_text,
            &new_text,
            vec![Splice::new(0..0, 4).unwrap()],
        )
        .unwrap();

        let after = candidate
            .install_reconciled_block_ids(&previous, &edits, &map, next_id)
            .unwrap();
        assert_eq!(candidate.blocks.len(), LINES + 2);
        assert_eq!(candidate.blocks()[0].id, next_id);
        assert_eq!(candidate.blocks()[1].id, previous.blocks()[0].id);
        assert_eq!(candidate.blocks().last().unwrap().id, previous_last);
        assert_eq!(after, next_id + 1);
    }

    #[test]
    fn markdown_projects_headings_and_inline_styles() {
        let projected = markdown("# A **bold** and *soft* with `code`\nplain");
        assert_eq!(projected.text(), "A bold and soft with code\nplain");
        assert_eq!(projected.text_tree().flatten(), projected.text());
        assert_eq!(projected.text_tree().hard_line_count(), 2);
        assert_eq!(projected.text_tree().hard_line_start(1).unwrap(), 26);
        assert_eq!(projected.blocks()[0].kind, BlockKind::Heading(1));
        assert_eq!(projected.blocks()[1].kind, BlockKind::Paragraph);
        assert_eq!(projected.styles.len(), 4);
        assert_eq!(projected.style_spans()[0].range, 2..6);
        assert_eq!(
            projected.style_spans()[0].application,
            StyleApplication::Semantic(SemanticInlineStyle::Strong)
        );
    }

    #[test]
    fn escaped_markdown_is_visible_but_escape_has_provenance() {
        let projected = markdown("\\*literal\\*");
        assert_eq!(projected.text(), "*literal*");
        assert_eq!(projected.provenance()[0].source, 0..2);
    }

    #[test]
    fn formatted_range_maps_inside_delimiters() {
        let projected = markdown("**bold** next");
        assert_eq!(projected.source_range(0..4), Some(2..6));
        assert_eq!(projected.source_insertion_point(4, false), Some(6));
        assert_eq!(projected.source_insertion_point(4, true), Some(8));
    }

    #[test]
    fn markdown_visible_source_ranges_exclude_adjacent_style_delimiters() {
        let projected = markdown("**ab**_cd_ tail");
        assert_eq!(projected.text(), "abcd tail");
        assert_eq!(projected.source_range(1..3), Some(3..8));
        assert_eq!(
            projected.line_local_visible_source_runs(1..3),
            Some(vec![
                VisibleSourceRun {
                    formatted: 1..2,
                    source: 3..4,
                },
                VisibleSourceRun {
                    formatted: 2..3,
                    source: 7..8,
                },
            ])
        );
        assert!(!projected.markdown_replacement_begins_in_code(&(1..3)));
    }

    #[test]
    fn line_local_source_ranges_decline_cross_line_and_relational_mappings() {
        let projected = markdown("**a**\n\n_b_");
        assert_eq!(projected.line_local_visible_source_runs(0..3), None);

        let relational = FormattedDocument::from_parts(
            Revision(7),
            "ab".to_owned(),
            blocks_for_plain_text("ab"),
            Vec::new(),
            vec![
                ProvenanceSpan {
                    formatted: 0..1,
                    source: 0..2,
                },
                ProvenanceSpan {
                    formatted: 1..2,
                    source: 1..3,
                },
            ],
            Vec::new(),
            StyleSheet::default(),
            0,
            3,
        );
        assert_eq!(relational.line_local_visible_source_runs(0..2), None);
    }

    #[test]
    fn markdown_replacement_context_uses_the_downstream_start_style() {
        let projected = markdown("`ab`plain`cd`");
        assert!(projected.markdown_replacement_begins_in_code(&(1..3)));
        assert!(!projected.markdown_replacement_begins_in_code(&(2..4)));
        assert!(projected.markdown_replacement_begins_in_code(&(7..7)));
        assert!(!projected.markdown_replacement_begins_in_code(&(2..2)));
    }

    #[test]
    fn source_boundary_affinity_selects_distinct_projection_sides() {
        let projected = FormattedDocument::from_parts(
            Revision(7),
            "abc".to_owned(),
            blocks_for_plain_text("abc"),
            Vec::new(),
            vec![
                ProvenanceSpan {
                    formatted: 0..1,
                    source: 0..1,
                },
                ProvenanceSpan {
                    formatted: 2..3,
                    source: 1..2,
                },
            ],
            Vec::new(),
            StyleSheet::default(),
            0,
            2,
        );
        assert_eq!(
            projected
                .map_source_boundary(Revision(7), 1, BoundaryAffinity::Upstream)
                .unwrap()
                .formatted_offset,
            1
        );
        assert_eq!(
            projected
                .map_source_boundary(Revision(7), 1, BoundaryAffinity::Downstream)
                .unwrap()
                .formatted_offset,
            2
        );
    }

    #[test]
    fn source_boundary_unique_preferred_side_ignores_ambiguous_opposite_side() {
        for (source_ranges, source_offset, affinity, expected, ambiguous) in [
            ([0..1, 1..2, 1..3], 1, BoundaryAffinity::Upstream, 1, vec![1, 2]),
            ([0..2, 1..2, 2..3], 2, BoundaryAffinity::Downstream, 2, vec![1, 2]),
        ] {
            let projected = FormattedDocument::from_parts(
                Revision(7), "abc".to_owned(), blocks_for_plain_text("abc"), Vec::new(),
                source_ranges.into_iter().enumerate().map(|(at, source)| ProvenanceSpan {
                    formatted: at..at + 1, source,
                }).collect(),
                Vec::new(), StyleSheet::default(), 0, 3,
            );
            assert_eq!(projected.map_source_boundary(Revision(7), source_offset, affinity)
                .unwrap().formatted_offset, expected);
            let opposite = match affinity {
                BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
                BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
            };
            assert_eq!(projected.map_source_boundary(Revision(7), source_offset, opposite),
                Err(SourceToTextError::AmbiguousBoundary { source_offset, candidates: ambiguous }));
        }
    }

    #[test]
    fn implicit_html_paragraph_close_keeps_its_downstream_source_boundary() {
        for (source, boundary, expected) in [("<p>a<p>b", 4, 1), ("<p><p>b", 3, 0)] {
            let projected = markdown(source);
            assert_eq!(projected.map_source_boundary(projected.revision(), boundary,
                BoundaryAffinity::Downstream).unwrap().formatted_offset, expected, "{source}");
        }
    }

    #[test]
    fn source_boundary_mapping_reports_ambiguity_staleness_and_bounds() {
        let projected = FormattedDocument::from_parts(
            Revision(7),
            "abc".to_owned(),
            blocks_for_plain_text("abc"),
            Vec::new(),
            vec![
                ProvenanceSpan {
                    formatted: 0..1,
                    source: 1..2,
                },
                ProvenanceSpan {
                    formatted: 2..3,
                    source: 1..3,
                },
            ],
            Vec::new(),
            StyleSheet::default(),
            0,
            3,
        );
        assert_eq!(
            projected.map_source_boundary(Revision(7), 1, BoundaryAffinity::Downstream),
            Err(SourceToTextError::AmbiguousBoundary {
                source_offset: 1,
                candidates: vec![0, 2]
            })
        );
        assert!(matches!(
            projected.map_source_boundary(Revision(6), 1, BoundaryAffinity::Downstream),
            Err(SourceToTextError::WrongSnapshot { .. })
        ));
        assert_eq!(
            projected.map_source_boundary(Revision(7), 4, BoundaryAffinity::Downstream),
            Err(SourceToTextError::SourceOffsetOutOfBounds {
                offset: 4,
                length: 3
            })
        );
    }

    #[test]
    fn malformed_current_adapter_partition_is_rejected_before_line_index_rebuild() {
        let mut projection = plain("a\nb".to_owned(), Revision(1));
        let original_lines = projection.hard_lines.clone();
        let mut malformed = projection.blocks.to_vec();
        malformed[1].range = 3..3;
        projection.blocks = OrderedRangeStore::new(malformed);

        assert_eq!(
            projection.rebuild_hard_lines_from_blocks(None),
            Err(BlockIdentityError::InvalidProjection)
        );
        assert_eq!(projection.hard_lines, original_lines);
    }

    #[test]
    fn many_style_spans_are_queried_through_a_bounded_interval_frontier() {
        const SPANS: usize = 100_000;
        let text = "x ".repeat(SPANS);
        let styles = (0..SPANS)
            .map(|index| StyleSpan {
                range: index * 2..index * 2 + 1,
                application: StyleApplication::Semantic(SemanticInlineStyle::Strong),
            })
            .collect();
        let projection = FormattedDocument::from_parts(
            Revision(1),
            text.clone(),
            blocks_for_plain_text(&text),
            styles,
            Vec::new(),
            Vec::new(),
            StyleSheet::default(),
            0,
            text.len(),
        );
        let query_start = (SPANS - 2) * 2;
        let (matches, stats) = projection
            .styles
            .query_overlapping_with_stats(&(query_start..query_start + 1));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].range, query_start..query_start + 1);
        assert!(stats.nodes_visited <= 40, "{stats:?}");
        assert!(stats.items_examined <= 64, "{stats:?}");
        assert!(projection.styles.invariant_holds());
    }

    #[test]
    fn extremely_long_single_block_has_constant_size_regional_result() {
        let projection = plain("x".repeat(4 * 1024 * 1024), Revision(1));
        let middle = projection.text().len() / 2;
        let (blocks, stats) = projection
            .blocks
            .query_touching_with_stats(&(middle..middle));

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].range, 0..projection.text().len());
        assert_eq!(stats.nodes_visited, 1);
        assert_eq!(stats.items_examined, 1);
        assert!(projection.blocks.invariant_holds());
    }

    #[test]
    fn hard_line_snapshot_batches_match_a_random_flat_oracle() {
        const LINES: usize = 20_000;
        let mut seed = 0x6a09_e667_f3bc_c909_u64;
        let mut text = String::new();
        let mut expected = Vec::with_capacity(LINES);
        for line in 0..LINES {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let length = usize::try_from(seed % 9).unwrap();
            let start = text.len();
            text.extend(std::iter::repeat('x').take(length));
            expected.push(start..text.len());
            if line + 1 != LINES {
                text.push('\n');
            }
        }
        let projection = plain(text, Revision(7));
        let snapshot = HardLineSnapshot::new(DocumentId(91), &projection);

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let start = usize::try_from(seed % u64::try_from(LINES + 1).unwrap()).unwrap();
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let remaining = LINES - start;
            let count = usize::try_from(seed % u64::try_from(remaining + 1).unwrap()).unwrap();
            let end = start + count;
            let actual = snapshot.lines(start..end).unwrap();
            assert_eq!(actual.len(), count);
            for (relative, info) in actual.iter().enumerate() {
                let index = start + relative;
                assert_eq!(info.document(), DocumentId(91));
                assert_eq!(info.revision(), Revision(7));
                assert_eq!(info.index(), index);
                assert_eq!(info.content_range(), expected[index]);
                let separator =
                    (index + 1 != LINES).then(|| expected[index].end..expected[index].end + 1);
                assert_eq!(info.separator_range(), separator);
            }

            let expected_extent = if start == end {
                let boundary = expected
                    .get(start)
                    .map_or(snapshot.text_length(), |line| line.start);
                boundary..boundary
            } else {
                let final_end = if end == LINES {
                    expected[end - 1].end
                } else {
                    expected[end - 1].end + 1
                };
                expected[start].start..final_end
            };
            assert_eq!(
                snapshot.linewise_extent(start..end).unwrap(),
                expected_extent
            );
        }

        for _ in 0..2_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let offset =
                usize::try_from(seed % u64::try_from(snapshot.text_length() + 1).unwrap()).unwrap();
            let expected_index = expected
                .iter()
                .rposition(|line| line.start <= offset && offset <= line.end)
                .unwrap();
            let actual = snapshot.line_at_offset(offset).unwrap();
            assert_eq!(actual.index(), expected_index);
            assert_eq!(actual.content_range(), expected[expected_index]);
        }
    }

    #[test]
    fn million_line_snapshot_batch_visits_only_the_requested_frontier() {
        const LINES: usize = 1_000_000;
        const REQUESTED: usize = 7;
        let mut text = "x\n".repeat(LINES - 1);
        text.push('x');
        let hard_lines = (0..LINES)
            .map(|index| HardLine {
                id: u64::try_from(index + 1).unwrap(),
                range: index * 2..index * 2 + 1,
                separator_length: usize::from(index + 1 != LINES),
            })
            .collect();
        let snapshot = HardLineSnapshot {
            document: DocumentId(1),
            revision: Revision(2),
            flat_text: {
                let value = Arc::new(OnceLock::new());
                value.set(Arc::from(text.clone())).unwrap();
                value
            },
            comment_context_cache: crate::document::comment_context::fresh_cache(),
            text_tree: FormattedTextTree::try_from_text(text).unwrap(),
            hard_lines: OrderedRangeStore::new(hard_lines),
            inline_images: IntervalRangeStore::new(Vec::new()),
        };
        let start = LINES - REQUESTED;
        let (lines, stats) = snapshot.lines_with_stats(start..LINES).unwrap();

        assert_eq!(lines.len(), REQUESTED);
        assert_eq!(lines[0].index(), start);
        assert_eq!(lines.last().unwrap().index(), LINES - 1);
        assert!(stats.nodes_visited <= 48, "{stats:?}");
        assert_eq!(stats.items_examined, REQUESTED);
    }

    /// Pin the format vocabulary by membership. Adding a format must be a
    /// deliberate decision about every predicate, not an inherited default.
    #[test]
    fn format_predicates_have_exact_membership() {
        const ALL: [Format; 3] = [Format::PlainText, Format::Markdown, Format::Code];

        fn members(predicate: fn(Format) -> bool) -> Vec<Format> {
            ALL.into_iter().filter(|format| predicate(*format)).collect()
        }

        assert_eq!(members(Format::is_markdown), [Format::Markdown]);
        assert_eq!(members(Format::is_code), [Format::Code]);
        assert_eq!(members(Format::is_literal), [Format::PlainText, Format::Code]);
        // Every format is exactly one of literal or Markdown.
        for format in ALL {
            assert_ne!(format.is_markdown(), format.is_literal(), "{format:?}");
        }
    }
}
