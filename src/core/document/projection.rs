use super::encoding::DecodingDiagnostic;
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
    Markdown,
    /// Editable Markdown source with formatting applied to visible syntax.
    MarkdownSource,
    Html,
    HtmlSource,
    Rtf,
    /// Literal source text with disposable, asynchronously computed syntax styles.
    Code,
}

impl Format {
    pub const fn is_code(self) -> bool { matches!(self, Self::Code) }

    /// Formats with identity semantics after decoding and physical line endings.
    pub const fn is_literal(self) -> bool { matches!(self, Self::PlainText | Self::Code) }
    /// The semantic editing view for this persistence format.
    pub const fn wysiwyg(self) -> Self {
        match self {
            Self::MarkdownSource => Self::Markdown,
            Self::HtmlSource => Self::Html,
            format => format,
        }
    }

    /// Markdown, in either its WYSIWYG or its source view.
    pub const fn is_markdown(self) -> bool {
        matches!(self, Self::Markdown | Self::MarkdownSource)
    }

    /// HTML, in either its WYSIWYG or its source view.
    pub const fn is_html(self) -> bool {
        matches!(self, Self::Html | Self::HtmlSource)
    }

    /// A source view: the format's own markup is visible, editable text.
    pub const fn is_source_view(self) -> bool {
        matches!(self, Self::HtmlSource | Self::MarkdownSource)
    }

    /// A WYSIWYG view: block structure and named styles are presented instead
    /// of the syntax which spells them. Plain text has no such structure and
    /// source views deliberately show the syntax, so neither qualifies.
    pub const fn is_wysiwyg(self) -> bool {
        matches!(self, Self::Html | Self::Markdown | Self::Rtf)
    }

    /// A WYSIWYG view whose source persists arbitrary character and paragraph
    /// declarations. Markdown carries structure but only a fixed inline
    /// vocabulary, so it is structured without being rich text.
    pub const fn is_rich_text(self) -> bool {
        matches!(self, Self::Html | Self::Rtf)
    }

    /// Backed by rich style markup in either view. Equivalent to
    /// [`Self::is_rich_text`] on this format's [`Self::wysiwyg`] view.
    pub const fn has_rich_source(self) -> bool {
        self.wysiwyg().is_rich_text()
    }

    /// `Enter` continues an enclosing list structure rather than inserting the
    /// marker text literally. Markdown Source qualifies because its list
    /// syntax is still structural, unlike HTML Source's tags.
    pub const fn has_structural_lists(self) -> bool {
        self.is_wysiwyg() || matches!(self, Self::MarkdownSource)
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

#[derive(Clone, Debug)]
pub struct BlockAttributes {
    pub kind: BlockKind,
    pub style: StyleId,
    /// Absent for the overwhelmingly common inherited paragraph. Nonempty
    /// declarations are immutable and shared by projection/history clones.
    pub direct_formatting: Option<Arc<BlockDirectFormatting>>,
}

impl Block {
    pub fn new(id: u64, range: Range<usize>, kind: BlockKind, style: StyleId,
        direct_formatting: Option<Arc<BlockDirectFormatting>>) -> Self {
        if kind == BlockKind::Paragraph && style.0 == "Paragraph" && direct_formatting.is_none() {
            return Self::paragraph(id, range);
        }
        Self { id, range, attributes: Arc::new(BlockAttributes { kind, style, direct_formatting }) }
    }

    pub fn paragraph(id: u64, range: Range<usize>) -> Self {
        static DEFAULT: OnceLock<Arc<BlockAttributes>> = OnceLock::new();
        Self { id, range, attributes: DEFAULT.get_or_init(|| Arc::new(BlockAttributes {
            kind: BlockKind::Paragraph, style: "Paragraph".into(), direct_formatting: None,
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
        self.kind == other.kind && self.style == other.style && **self == **other
    }
}

impl RangedItem for Block {
    fn visit_shared_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        visitor.arc_once(&self.attributes, |visitor| {
            visitor.owned(Arc::as_ptr(&self.attributes) as usize, 0, self.style.0.capacity() + 16);
            if let Some(properties) = &self.direct_formatting {
                visitor.arc(properties, |visitor| {
                    visitor.owned(Arc::as_ptr(properties) as usize, 0,
                        properties.direct_default_character.owned_heap_bytes());
                });
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
        logical_is_grapheme_boundary(&self.text_tree, &self.hard_lines, offset).unwrap_or(false)
    }

    /// Return the next logical grapheme/item boundary after `offset`.
    pub fn next_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        logical_next_grapheme_boundary(&self.text_tree, &self.hard_lines, offset)
            .ok()
            .flatten()
    }

    /// Return the preceding logical grapheme/item boundary before `offset`.
    pub fn previous_grapheme_boundary(&self, offset: usize) -> Option<usize> {
        logical_previous_grapheme_boundary(&self.text_tree, &self.hard_lines, offset)
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
        logical_is_grapheme_boundary(&self.text_tree, &self.hard_lines, offset)
    }

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_next_grapheme_boundary(&self.text_tree, &self.hard_lines, offset)
    }

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_previous_grapheme_boundary(&self.text_tree, &self.hard_lines, offset)
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
    offset: usize,
) -> Result<bool, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Ok(false);
    }
    if is_semantic_item_boundary(hard_lines, offset) {
        return Ok(true);
    }
    text.is_grapheme_boundary(offset)
}

fn logical_next_grapheme_boundary(
    text: &FormattedTextTree,
    hard_lines: &OrderedRangeStore<HardLine>,
    offset: usize,
) -> Result<Option<usize>, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Err(FormattedTextError::NotCharBoundary(offset));
    }
    let unicode = text.next_grapheme_boundary(offset)?;
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
    offset: usize,
) -> Result<Option<usize>, FormattedTextError> {
    if !text.is_char_boundary(offset)? {
        return Err(FormattedTextError::NotCharBoundary(offset));
    }
    let unicode = text.previous_grapheme_boundary(offset)?;
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

/// Immutable formatted projection consumed by commands and layout.
#[derive(Clone, Debug)]
pub struct FormattedDocument {
    revision: Revision,
    /// Canonical persistent formatted-text representation.
    text: FormattedTextTree,
    /// Lazily materialized compatibility view. Regional candidates can remain
    /// wholly tree-backed until a legacy caller explicitly requests `text()`.
    flat_text: Arc<OnceLock<Arc<str>>>,
    blocks: OrderedRangeStore<Block>,
    /// Authoritative hard-line structure. This is intentionally distinct from
    /// both the block tree and the text tree's lexical U+000A aggregates. The
    /// paragraphs may contain several explicit breaks, and flow-normalized
    /// paragraphs may consume several physical source lines. This index is
    /// populated independently from paragraph identity. A pipeline may also retain U+000A as ordinary
    /// content (for example forced legacy-Mac input).
    hard_lines: OrderedRangeStore<HardLine>,
    flow_lines: Option<OrderedRangeStore<HardLine>>,
    /// Source-visible structural paragraphs, used only by paragraph flow.
    /// Their ranges retain every source character, including surrounding tags.
    flow_blocks: Option<OrderedRangeStore<Block>>,
    styles: IntervalRangeStore<StyleSpan>,
    provenance: IntervalRangeStore<ProvenanceSpan>,
    /// Literal runs are divisible encoding mappings; rich contributors retain
    /// their existing indivisible/relational meaning.
    literal_encoding: Option<super::Encoding>,
    source_boundaries: IntervalRangeStore<SourceTextBoundary>,
    source_ordered: bool,
    decoding_diagnostics: IntervalRangeStore<DecodingDiagnostic>,
    style_sheet: Arc<StyleSheet>,
    document_style: DocumentStyleAssignment,
    source_content_start: usize,
    source_content_end: usize,
    source_insertion_end: usize,
}

impl PartialEq for FormattedDocument {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.text() == other.text()
            && self.blocks == other.blocks
            && self.hard_lines == other.hard_lines
            && self.flow_lines == other.flow_lines
            && self.flow_blocks == other.flow_blocks
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
        logical_is_grapheme_boundary(&self.text, &self.hard_lines, offset)
    }

    fn next_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_next_grapheme_boundary(&self.text, &self.hard_lines, offset)
    }

    fn previous_logical_grapheme_boundary(
        &self,
        offset: usize,
    ) -> Result<Option<usize>, FormattedTextError> {
        logical_previous_grapheme_boundary(&self.text, &self.hard_lines, offset)
    }
}

impl FormattedDocument {
    pub(super) fn visit_retained_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
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
        if let Some(lines) = &self.flow_lines { lines.visit_retained_memory(visitor); }
        if let Some(blocks) = &self.flow_blocks { blocks.visit_retained_memory(visitor); }
        self.styles.visit_retained_memory(visitor);
        if trace { eprintln!("  styles {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.provenance.visit_retained_memory(visitor);
        if trace { eprintln!("  provenance {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
        self.source_boundaries.visit_retained_memory(visitor);
        if trace { eprintln!("  boundaries {}", visitor.retained_bytes() - before); before = visitor.retained_bytes(); }
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
        let document_style = DocumentStyleAssignment::new(style_sheet.base_paragraph.clone());
        let flat_text: Arc<str> = flat_text.into();
        let text = FormattedTextTree::try_from_shared(flat_text.clone())
            .expect("a materialized Rust string has representable text-tree aggregates");
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
        let source_ordered = provenance
            .windows(2)
            .all(|pair| pair[0].source.start <= pair[1].source.start);
        let source_boundaries =
            IntervalRangeStore::new(source_text_boundaries(provenance.iter().cloned()));
        Self {
            revision,
            text,
            flat_text: compatibility_text,
            blocks: OrderedRangeStore::new(blocks),
            hard_lines: OrderedRangeStore::new(hard_lines),
            flow_lines: None,
            flow_blocks: None,
            styles: IntervalRangeStore::new(styles),
            provenance: IntervalRangeStore::new(provenance),
            literal_encoding: None,
            source_boundaries,
            source_ordered,
            decoding_diagnostics: IntervalRangeStore::new(decoding_diagnostics),
            style_sheet: Arc::new(style_sheet),
            document_style,
            source_content_start,
            source_content_end,
            source_insertion_end: source_content_end,
        }
    }

    pub(crate) fn install_flow_ranges(&mut self, ranges: Vec<Range<usize>>) {
        let ends = ranges
            .iter()
            .skip(1)
            .map(|range| range.start)
            .chain(std::iter::once(self.text.byte_len()))
            .collect::<Vec<_>>();
        self.flow_lines = Some(OrderedRangeStore::new(
            ranges
                .into_iter()
                .zip(ends)
                .map(|(range, next_start)| HardLine {
                    id: 0,
                    separator_length: next_start - range.end,
                    range,
                })
                .collect(),
        ));
    }

    pub(crate) fn install_flow_blocks(&mut self, blocks: Vec<Block>) {
        self.flow_blocks = Some(OrderedRangeStore::new(blocks));
    }

    pub(crate) fn flow_blocks_for_region(&self, range: &Range<usize>) -> Option<Vec<Block>> {
        self.flow_blocks
            .as_ref()
            .map(|blocks| blocks.query_touching(range))
    }

    pub(crate) fn flow_ranges_for_region(&self, range: &Range<usize>) -> Option<Vec<Range<usize>>> {
        self.flow_lines.as_ref().map(|lines| {
            lines
                .query_touching(range)
                .into_iter()
                .map(|line| line.range)
                .collect()
        })
    }

    pub fn presentation_line_count(&self, flow: bool) -> usize {
        if flow {
            self.flow_lines
                .as_ref()
                .map_or(self.hard_lines.len(), OrderedRangeStore::len)
        } else {
            self.hard_lines.len()
        }
    }

    pub fn presentation_line_range(&self, index: usize, flow: bool) -> Option<Range<usize>> {
        let lines = if flow {
            self.flow_lines.as_ref().unwrap_or(&self.hard_lines)
        } else {
            &self.hard_lines
        };
        lines.get(index).map(|line| line.range)
    }

    pub fn presentation_line_at_offset(&self, offset: usize, flow: bool) -> Option<usize> {
        if offset > self.text.byte_len() {
            return None;
        }
        let lines = if flow {
            self.flow_lines.as_ref().unwrap_or(&self.hard_lines)
        } else {
            &self.hard_lines
        };
        lines
            .partition_point(|line| line.range.start <= offset)
            .checked_sub(1)
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
            .iter()
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
    pub(crate) fn install_persistent_text_edits(
        &mut self,
        previous: &Self,
        edits: &[TextEdit],
    ) -> Result<(), FormattedTextError> {
        let edits: Vec<_> = edits
            .iter()
            .map(|edit| (edit.range.clone(), edit.replacement.as_str()))
            .collect();
        let persistent = previous.text.splice_prevalidated_batch(&edits)?;
        if persistent.flatten() != self.text() {
            return Err(FormattedTextError::ResultTextMismatch);
        }
        self.text = persistent;
        Ok(())
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
        Ok(())
    }

    /// Rich adapters supply an independent paragraph partition after seeding
    /// their explicit hard-line records. Literal source newline characters in
    /// other adapters are never reinterpreted by this operation.
    pub(crate) fn install_paragraph_partition(&mut self, paragraphs: Vec<Block>) {
        validate_block_partition(self.text(), &paragraphs)
            .expect("rich projectors emit a valid paragraph partition");
        self.blocks = OrderedRangeStore::new(paragraphs);
    }

    pub(crate) fn install_hard_line_partition(&mut self, ranges: Vec<Range<usize>>) {
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
        self.blocks = OrderedRangeStore::new(blocks);
        let next_id = self.assign_initial_hard_line_ids(next_id)?;
        let next_id = self.reconcile_flow_block_ids(None, &[], next_id)?;
        // Initial validation used the construction string. Literal snapshots
        // publish only bounded text buffers; a legacy flat read remains lazy.
        if self.literal_encoding.is_some() { self.flat_text = Arc::new(OnceLock::new()); }
        Ok(next_id)
    }

    fn reconcile_flow_block_ids(
        &mut self,
        previous: Option<&Self>,
        edits: &[TextEdit],
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        let Some(flow) = &self.flow_blocks else {
            return Ok(next_id);
        };
        let mut blocks = flow.to_vec();
        if let Some(previous) = previous {
            let mappings = build_edit_mappings(previous.text().len(), self.text().len(), edits)?;
            if let Some(old) = &previous.flow_blocks {
                for block in old.as_slice() {
                    if let Some(witness) =
                        block_identity_witness(previous.text(), block, &mappings)?
                    {
                        let at = map_surviving_byte(witness, &mappings)?;
                        let index = block_index_for_witness(
                            self.text(),
                            &blocks,
                            at,
                            witness_side(previous.text(), block, witness),
                        )?;
                        if let Some(candidate) = blocks.get_mut(index).filter(|block| block.id == 0)
                        {
                            candidate.id = block.id;
                        }
                    }
                }
            }
        }
        let next_id = allocate_unassigned_block_ids(&mut blocks, next_id)?;
        self.flow_blocks = Some(OrderedRangeStore::new(blocks));
        if let Some(old) = previous.and_then(|previous| previous.flow_blocks.as_ref()) {
            self.flow_blocks.as_mut().unwrap().reuse_equal_chunks(old);
        }
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
            candidate.direct_formatting = old.direct_formatting.clone();
            if previous
                .style_sheet
                .configuration_deleted(&candidate.style, true)
            {
                candidate.style = previous.style_sheet.base_paragraph.clone();
            }
        }
        self.style_sheet = previous.style_sheet.clone();
        self.document_style = previous.document_style.clone();
        self.blocks = OrderedRangeStore::new(blocks);
        self.blocks.reuse_equal_chunks(&previous.blocks);
        self.install_unchanged_hard_line_ids(previous)?;
        if let (Some(flow), Some(old)) = (&mut self.flow_blocks, &previous.flow_blocks) {
            let mut blocks = flow.to_vec();
            if blocks.len() != old.len() || blocks.iter().zip(old.as_slice())
                .any(|(block, old)| block.range != old.range) {
                return Err(BlockIdentityError::InvalidProjection);
            }
            for (block, old) in blocks.iter_mut().zip(old.as_slice()) { block.id = old.id; }
            *flow = OrderedRangeStore::new(blocks);
            flow.reuse_equal_chunks(old);
        }
        self.styles.reuse_equal_chunks(&previous.styles);
        Ok(())
    }

    /// Retain block identity while publishing properties and definitions newly
    /// parsed from an authoritative rich source rather than configuration.
    pub(crate) fn install_source_block_ids(
        &mut self,
        previous: &Self,
    ) -> Result<(), BlockIdentityError> {
        let parsed_blocks = self.blocks.to_vec();
        let parsed_sheet = self.style_sheet.clone();
        let parsed_document = self.document_style.clone();
        self.install_unchanged_block_ids(previous)?;
        let mut blocks = self.blocks.to_vec();
        for (block, parsed) in blocks.iter_mut().zip(parsed_blocks) {
            block.direct_formatting = parsed.direct_formatting.clone();
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
        // Source adapters do not author these configuration-only root layers.
        // Retain them when preserving source-backed parsed definitions.
        self.document_style.direct_canvas = previous.document_style.direct_canvas.clone();
        self.document_style.direct_default_character =
            previous.document_style.direct_default_character.clone();
        if self
            .style_sheet
            .block_style(&previous.document_style.style)
            .is_some()
        {
            self.document_style.style = previous.document_style.style.clone();
        }
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
                target.direct_formatting = old.direct_formatting.clone();
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
        self.blocks = OrderedRangeStore::new(blocks);
        self.blocks.reuse_equal_chunks(&previous.blocks);
        let next_id = self.reconcile_hard_line_ids(previous, edits, next_id)?;
        let next_id = self.reconcile_flow_block_ids(Some(previous), edits, next_id)?;
        self.styles.reuse_equal_chunks(&previous.styles);
        Ok(next_id)
    }

    /// Install move-aware block identities after a verified source-backed
    /// hard-line transfer. The initial adapters publish exactly one block per
    /// hard line; refusing any other shape keeps this temporary contract
    /// explicit until nested/multi-line block transfer is modeled directly.
    pub(crate) fn install_reconciled_format_block_ids(
        &mut self,
        format: Format,
        previous: &Self,
        edits: &[TextEdit],
        position_map: &PositionMap,
        next_id: u64,
    ) -> Result<u64, BlockIdentityError> {
        if !format.has_rich_source() {
            return self.install_reconciled_block_ids(previous, edits, position_map, next_id);
        }
        self.install_reconciled_source_block_ids(previous, edits, position_map, next_id)
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
            block.direct_formatting = parsed.direct_formatting.clone();
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
            self.styles.reuse_equal_chunks(&previous.styles);
            return self.reconcile_flow_block_ids(None, &[], next_id);
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
        self.styles.reuse_equal_chunks(&previous.styles);
        self.reconcile_flow_block_ids(None, &[], next_id)
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

        let mut upstream: Vec<_> = self
            .provenance
            .iter()
            .filter(|span| span.source.end == source_offset)
            .map(|span| span.formatted.end)
            .collect();
        let mut downstream: Vec<_> = self
            .provenance
            .iter()
            .filter(|span| span.source.start == source_offset)
            .map(|span| span.formatted.start)
            .collect();
        upstream.sort_unstable();
        upstream.dedup();
        downstream.sort_unstable();
        downstream.dedup();
        let (preferred, fallback) = match affinity {
            BoundaryAffinity::Upstream => (&upstream, &downstream),
            BoundaryAffinity::Downstream => (&downstream, &upstream),
        };
        if let Some(formatted_offset) = unique_candidate(source_offset, preferred)?
            .or(unique_candidate(source_offset, fallback)?)
        {
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
        if let Some(span) = self
            .provenance
            .iter()
            .find(|span| span.source.start < source_offset && source_offset < span.source.end)
        {
            let opaque = self.decoding_diagnostics.iter().any(|diagnostic| {
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

    fn hidden_gap(
        &self,
        source_offset: usize,
    ) -> Option<(Range<usize>, Option<usize>, Option<usize>)> {
        let mut source_start = self.source_content_start;
        let mut upstream = None;
        for span in self.provenance.as_slice() {
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

    pub fn blocks(&self) -> &[Block] {
        self.blocks.as_slice()
    }

    pub fn style_spans(&self) -> &[StyleSpan] {
        self.styles.as_slice()
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

    /// Authoritative line ranges touching a formatted text region, found in
    /// `O(log n + k)` time and materialized with absolute snapshot offsets.
    pub(crate) fn hard_lines_for_region(&self, text_range: &Range<usize>) -> Vec<Range<usize>> {
        self.hard_lines
            .query_touching(text_range)
            .into_iter()
            .map(|line| line.range)
            .collect()
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

    /// Style spans with a non-empty intersection with a formatted region,
    /// located in `O(log n + k)` time. The `k` returned records are materialized
    /// with snapshot-absolute ranges and remain ordered by normalized span
    /// start (and stable input order for equal starts).
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

    /// Includes point annotations at an empty editable boundary, located in
    /// `O(log n + k)` without materializing the complete style collection.
    pub(crate) fn style_spans_touching(&self, range: &Range<usize>) -> Vec<StyleSpan> {
        self.styles.query_touching(range)
    }

    pub fn provenance(&self) -> &[ProvenanceSpan] {
        self.provenance.as_slice()
    }

    /// A synthetic paragraph separator can recover from the end of its last
    /// visible character, inside a now-redundant HTML whitespace wrapper.
    /// Move only that zero-byte recovery point past the removed closing syntax
    /// before applying the ordinary source shift to the untouched suffix.
    pub(crate) fn relocate_synthetic_hard_line_source_boundary(
        &self,
        formatted_at: usize,
        source_from: usize,
        source_to: usize,
    ) -> Result<Option<(Self, ProjectionSpliceStatistics)>, BlockIdentityError> {
        let Some(end) = formatted_at
            .checked_add(1)
            .filter(|end| *end <= self.text.byte_len())
        else {
            return Ok(None);
        };
        if self.text.slice(formatted_at..end).as_deref() != Ok("\n") {
            return Ok(None);
        }
        let index = self.provenance.partition_point_start(formatted_at);
        let Some(old) = self.provenance.get(index) else {
            return Ok(None);
        };
        if old.formatted != (formatted_at..end) || old.source != (source_from..source_from) {
            return Ok(None);
        }
        if source_to < source_from
            || source_to > self.source_content_end
            || index > 0
                && self
                    .provenance
                    .get(index - 1)
                    .is_some_and(|span| span.source.end > source_to)
            || self
                .provenance
                .get(index + 1)
                .is_some_and(|span| span.source.start < source_to)
        {
            return Err(BlockIdentityError::InvalidProjection);
        }
        let mut relocated = old.clone();
        relocated.source = source_to..source_to;
        let mut stats = RangeSpliceStats::default();
        let provenance = self
            .provenance
            .splice(
                index..index + 1,
                vec![relocated.clone()],
                end,
                end,
                &mut stats,
            )
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let boundary_end = source_to
            .checked_add(1)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let indices = self.source_boundaries.partition_point_start(source_from)
            ..self.source_boundaries.partition_point_start(boundary_end);
        let mut boundaries = self
            .source_boundaries
            .get_range(&indices)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let old_boundaries = source_text_boundaries([old]);
        let old_count = boundaries.len();
        boundaries.retain(|boundary| !old_boundaries.contains(boundary));
        if old_count - boundaries.len() != old_boundaries.len() {
            return Err(BlockIdentityError::InvalidProjection);
        }
        boundaries.extend(source_text_boundaries([relocated]));
        normalize_source_boundaries(&mut boundaries);
        let source_boundaries = self
            .source_boundaries
            .splice(indices, boundaries, source_to, source_to, &mut stats)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let mut projection = self.clone();
        projection.provenance = provenance;
        projection.source_boundaries = source_boundaries;
        Ok(Some((
            projection,
            ProjectionSpliceStatistics {
                range_indexes: stats,
            },
        )))
    }

    pub(crate) fn provenance_for_region(&self, range: &Range<usize>) -> Vec<ProvenanceSpan> {
        self.provenance.query_overlapping(range).into_iter()
            .map(|span| self.clip_literal_span(span, range)).collect()
    }

    pub(crate) fn provenance_touching(&self, range: &Range<usize>) -> Vec<ProvenanceSpan> {
        self.provenance.query_touching(range).into_iter()
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

    fn literal_source_in_span(&self, span: &ProvenanceSpan, at: usize) -> Option<usize> {
        if at == span.formatted.start { return Some(span.source.start); }
        if at == span.formatted.end { return Some(span.source.end); }
        let encoding = self.literal_encoding?;
        if encoding == super::Encoding::Utf8 && span.formatted.len() == span.source.len() {
            return (span.formatted.contains(&at) && self.text.is_char_boundary(at).ok()?)
                .then_some(span.source.start + at - span.formatted.start);
        }
        let text = self.text.slice(span.formatted.clone()).ok()?;
        if encoding.encoded_text_len(&text) != span.source.len() { return None; }
        let prefix = text.get(..at.checked_sub(span.formatted.start)?)?;
        Some(span.source.start + encoding.encoded_text_len(prefix))
    }

    fn literal_text_in_span(&self, span: &ProvenanceSpan, source: usize) -> Option<usize> {
        if source == span.source.start { return Some(span.formatted.start); }
        if source == span.source.end { return Some(span.formatted.end); }
        let encoding = self.literal_encoding?;
        if encoding == super::Encoding::Utf8 && span.formatted.len() == span.source.len() {
            let at = span.formatted.start.checked_add(source.checked_sub(span.source.start)?)?;
            return (source < span.source.end && self.text.is_char_boundary(at).ok()?).then_some(at);
        }
        let text = self.text.slice(span.formatted.clone()).ok()?;
        if encoding.encoded_text_len(&text) != span.source.len() { return None; }
        let mut at = span.source.start;
        for (offset, ch) in text.char_indices() {
            if at == source { return Some(span.formatted.start + offset); }
            at += encoding.scalar_source_width(ch);
            if at > source { return None; }
        }
        None
    }

    fn literal_span_for_source(&self, source: usize) -> Option<ProvenanceSpan> {
        self.literal_encoding?;
        let index = self.source_boundaries.partition_point_start(source);
        let boundary = self.source_boundaries.get(index).filter(|at| at.source.start == source)
            .or_else(|| index.checked_sub(1).and_then(|at| self.source_boundaries.get(at)))?;
        self.provenance.query_touching(&(boundary.formatted..boundary.formatted)).into_iter()
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
        let first = self.provenance.query_touching(&(range.start..range.start)).into_iter().next()?;
        let last = self.provenance.query_touching(&(range.end..range.end)).into_iter().last()?;
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
        for boundary in self.source_boundaries.query_touching(range) {
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
        let index = self.source_boundaries.partition_point_start(source);
        let following = self.source_boundaries.get(index);
        if following
            .as_ref()
            .is_some_and(|boundary| boundary.source.start == source)
        {
            let exact = self.source_boundaries.query_touching(&(source..source));
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
        let preceding = index
            .checked_sub(1)
            .and_then(|index| self.source_boundaries.get(index));
        let selected = if downstream {
            following.as_ref().or(preceding.as_ref())
        } else {
            preceding.as_ref().or(following.as_ref())
        }?;
        let at = selected.source.start;
        let adjacent = self.source_boundaries.query_touching(&(at..at));
        if at < source {
            adjacent.iter().map(|boundary| boundary.formatted).max()
        } else {
            adjacent.iter().map(|boundary| boundary.formatted).min()
        }
    }

    #[cfg(test)]
    pub(crate) fn source_boundary_query_work(&self, source: usize) -> (usize, usize) {
        let (_, stats) = self
            .source_boundaries
            .query_overlapping_with_stats(&(source.saturating_sub(1)..source.saturating_add(1)));
        (stats.nodes_visited, stats.items_examined)
    }

    /// Malformed source ranges represented by visible opaque replacement
    /// items in this exact formatted snapshot.
    pub fn decoding_diagnostics(&self) -> &[DecodingDiagnostic] {
        self.decoding_diagnostics.as_slice()
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

    /// Install an already validated generated-configuration sheet on an
    /// otherwise unchanged projection candidate. The caller binds the
    /// candidate to the new document revision before atomic publication.
    pub(crate) fn reassign_deleted_style(&mut self, id: &StyleId, block: bool) {
        if block {
            let mut blocks = self.blocks.to_vec();
            for block in &mut blocks {
                if &block.style == id {
                    block.style = self.style_sheet.base_paragraph.clone();
                }
            }
            self.blocks = OrderedRangeStore::new(blocks);
            if let Some(flow) = &mut self.flow_blocks {
                let mut blocks = flow.to_vec();
                for block in &mut blocks {
                    if &block.style == id { block.style = self.style_sheet.base_paragraph.clone(); }
                }
                *flow = OrderedRangeStore::new(blocks);
            }
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
        self.revision = revision;
        self.style_sheet = Arc::new(style_sheet);
        self.document_style = document_style;
    }

    pub(crate) fn install_code_styles(&mut self, sheet: Arc<StyleSheet>, runs: &[super::syntax::SyntaxRun]) {
        self.document_style = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        let names=sheet.character_styles().filter_map(|style|sheet.character_style_metadata(&style.id).map(|metadata|(metadata.display_name.as_str(),&style.id))).collect::<std::collections::BTreeMap<_,_>>();
        self.styles = IntervalRangeStore::new(runs.iter().filter_map(|run| {
            if run.range.start >= run.range.end || run.range.end > self.text.byte_len() { return None; }
            let ceil = |at| {
                if self.is_logical_grapheme_boundary(at).ok()? { Some(at) }
                else { self.next_logical_grapheme_boundary(at).ok()? }
            };
            // A cluster's first character owns its appearance. A later
            // capture beginning inside it starts at the following cluster.
            let range = ceil(run.range.start)?..ceil(run.range.end)?;
            if range.is_empty() { return None; }
            let id = names.get(run.name.0.as_str())?;
            Some(StyleSpan { range, application: StyleApplication::Automatic((*id).clone()) })
        }).collect());
        self.style_sheet = sheet;
    }

    pub(crate) fn has_block_style_assignment(&self, style: &StyleId) -> bool {
        self.document_style.style == *style || self.blocks.iter().any(|block| block.style == *style)
    }

    pub(crate) fn has_character_style_assignment(&self, style: &StyleId) -> bool {
        self.styles
            .iter()
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
        ranges.extend(
            self.blocks
                .iter()
                .filter(|block| block_styles.contains(&block.style))
                .map(|block| block.range.clone()),
        );
        if let Some(blocks) = &self.flow_blocks {
            ranges.extend(blocks.iter().filter(|block| block_styles.contains(&block.style))
                .map(|block| block.range.clone()));
        }
        ranges.extend(
            self.styles
                .iter()
                .filter_map(|span| match &span.application {
                    StyleApplication::Named(id) | StyleApplication::Automatic(id) => {
                        character_styles.contains(id).then(|| span.range.clone())
                    }
                    StyleApplication::SourceParagraph { style, .. } => {
                        block_styles.contains(style).then(|| span.range.clone())
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
        let spans = self.provenance();
        spans
            .iter()
            .all(|span| !span.source.is_empty() || span.formatted.is_empty())
            && spans.windows(2).all(|pair| {
                pair[0].source.end <= pair[1].source.start
                    && pair[0].formatted.end <= pair[1].formatted.start
            })
    }

    /// Batch counterpart of `source_range` for a complete reprojection diff.
    /// Adjacent graphemes share an ordered provenance frontier, avoiding four
    /// independent interval-tree queries and temporary vectors per item.
    pub(crate) fn source_grapheme_ranges(
        &self,
    ) -> impl Iterator<Item = (usize, &str, Option<Range<usize>>)> {
        let spans = self.provenance();
        let ordered_ends = spans
            .windows(2)
            .all(|pair| pair[0].formatted.end <= pair[1].formatted.end);
        let mut first_start = 0;
        let mut after_end = 0;
        let mut boundary = move |at: usize| {
            while first_start < spans.len() && spans[first_start].formatted.start < at {
                first_start += 1;
            }
            while after_end < spans.len() && spans[after_end].formatted.end <= at {
                after_end += 1;
            }
            let following = spans
                .get(first_start)
                .filter(|span| span.formatted.start == at)
                .map(|span| span.source.start);
            let preceding = after_end
                .checked_sub(1)
                .and_then(|index| spans.get(index))
                .filter(|span| span.formatted.end == at)
                .map(|span| span.source.end);
            (following, preceding)
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
            let source = if self.literal_encoding.is_none() && ordered_ends && !spans.is_empty() {
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

        let adjacent = self.provenance.query_touching(&(at..at));
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
            false,
        ),
        Format::MarkdownSource => project_markdown(
            normalized,
            revision,
            source_content_start,
            source_content_end,
            true,
        ),
        Format::Html => super::html::project(
            normalized,
            revision,
            source_content_start,
            source_content_end,
        ),
        Format::HtmlSource => super::html_source::project(
            normalized,
            revision,
            source_content_start,
            source_content_end,
        ),
        Format::Rtf => super::rtf::project(
            normalized,
            revision,
            source_content_start,
            source_content_end,
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
    pub(crate) fn include(&mut self, other: Self) {
        self.range_indexes.nodes_visited += other.range_indexes.nodes_visited;
        self.range_indexes.nodes_copied += other.range_indexes.nodes_copied;
        self.range_indexes.leaves_copied += other.range_indexes.leaves_copied;
        self.range_indexes.items_copied += other.range_indexes.items_copied;
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
    // HTML Source retains a pre's multi-line paragraph when it is quoted too.
    // A regional text edit must extend that same paragraph rather than demand
    // that the captured physical line cover its entire source extent.
    let partial_preserved_block = previous_region_blocks.len() == 1
        && matches!(
            previous_region_blocks[0].style.0.as_str(),
            "Code Block" | "Block quote"
        )
        && previous_region_blocks[0].range.start <= old_formatted.start
        && old_formatted.end <= previous_region_blocks[0].range.end
        && regional.blocks.len() == 1
        && regional.blocks.as_slice()[0].style == previous_region_blocks[0].style;
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
        let regional_lines = regional.hard_lines.as_slice();
        let mut used = std::collections::BTreeSet::new();
        let last = regional_blocks.len().saturating_sub(1);
        for (index, candidate) in regional_blocks.iter_mut().enumerate() {
            let line = regional_lines
                .partition_point(|line| line.range.start <= candidate.range.start)
                .saturating_sub(1);
            let old_line = previous
                .hard_lines
                .get(old_hard_lines.start + line)
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
            candidate.id = if used.insert(old.id) {
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
    if !literal_topology && regional_lines.len() != old_lines.len() {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let regional_hard_lines = if literal_topology {
        regional_lines.iter().enumerate().map(|(index, line)| {
            Ok(HardLine {
                id: regional_blocks[index].id,
                range: regional_blocks[index].range.clone(),
                separator_length: if index + 1 == regional_lines.len() { old_lines.last().unwrap().separator_length } else { line.separator_length },
            })
        }).collect::<Result<Vec<_>, BlockIdentityError>>()?
    } else { regional_lines
        .iter()
        .zip(&old_lines)
        .map(|(line, old)| {
            Ok(HardLine {
                id: old.id,
                range: shift_region_range(&line.range, old_formatted.start)?,
                separator_length: old.separator_length,
            })
        })
        .collect::<Result<Vec<_>, BlockIdentityError>>()? };
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

    let flow_lines = if let Some(old_flow) = &previous.flow_lines {
        let indices = old_flow.partition_point(|line| line.range.end < old_formatted.start)
            ..old_flow.partition_point(|line| line.range.start <= old_formatted.end);
        let old = old_flow
            .get_range(&indices)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let parsed = regional.flow_lines.as_ref().unwrap_or(&regional.hard_lines);
        let mut next = parsed
            .as_slice()
            .iter()
            .map(|line| {
                Ok(HardLine {
                    id: line.id,
                    range: shift_region_range(&line.range, old_formatted.start)?,
                    separator_length: line.separator_length,
                })
            })
            .collect::<Result<Vec<_>, BlockIdentityError>>()?;
        let (Some(first), Some(last)) = (old.first(), old.last()) else {
            return Err(BlockIdentityError::InvalidProjection);
        };
        // Source restart regions retain an unchanged neighboring physical line
        // (or validated inherited HTML prose context). Connections outside the
        // region therefore keep their old classification. Extend the edge
        // records without reading or rebuilding the untouched prose suffix.
        if first.range.start < old_formatted.start {
            next.first_mut()
                .ok_or(BlockIdentityError::InvalidProjection)?
                .range
                .start = first.range.start;
        }
        if last.range.end > old_formatted.end {
            let tail = next
                .last_mut()
                .ok_or(BlockIdentityError::InvalidProjection)?;
            tail.range.end = (last.range.end as i128 + new_formatted_end as i128
                - old_formatted.end as i128)
                .try_into()
                .map_err(|_| BlockIdentityError::InvalidProjection)?;
            tail.separator_length = last.separator_length;
        }
        Some(
            old_flow
                .splice(
                    indices,
                    next,
                    old_formatted.end,
                    new_formatted_end,
                    &mut range_stats,
                )
                .ok_or(BlockIdentityError::InvalidProjection)?,
        )
    } else {
        None
    };

    let flow_blocks = if let Some(old_flow) = &previous.flow_blocks {
        let indices = old_flow.partition_point(|block| {
            block.range.end <= old_formatted.start && !block.range.is_empty()
        })..old_flow.partition_point(|block| {
            block.range.start < old_formatted.end
                || block.range.is_empty() && block.range.start == old_formatted.end
        });
        let old = old_flow
            .get_range(&indices)
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let parsed = regional
            .flow_blocks
            .as_ref()
            .ok_or(BlockIdentityError::InvalidProjection)?;
        let mut next = parsed.to_vec();
        for block in &mut next {
            block.id = 0;
            block.range = shift_region_range(&block.range, old_formatted.start)?;
        }
        if let (Some(first), Some(old)) = (next.first_mut(), old.first()) {
            first.range.start = first.range.start.min(old.range.start);
        }
        if let (Some(last), Some(old)) = (next.last_mut(), old.last()) {
            if old.range.end > old_formatted.end {
                last.range.end = old.range.end - old_formatted.end + new_formatted_end;
            }
        }
        // Source tags can create, join, or remove several presentation
        // paragraphs inside an unchanged physical source line. Reconcile
        // only the touched metadata, using surviving bytes as identity
        // witnesses and the owning document's normal ID allocator.
        let mappings =
            build_edit_mappings(previous.text.byte_len(), target_text.byte_len(), edits)?;
        for old in &old {
            let at = if let Some(witness) = first_surviving_byte(&old.range, &mappings) {
                map_surviving_byte(witness, &mappings)?
            } else if old.range.is_empty() {
                match map_old_boundary_before(old.range.start, &mappings) {
                    Ok(at) => at,
                    Err(_) => continue,
                }
            } else {
                continue;
            };
            let index = next
                .partition_point(|block| block.range.start <= at)
                .saturating_sub(1);
            if let Some(block) = next.get_mut(index).filter(|block| {
                block.id == 0
                    && (block.range.contains(&at)
                        || block.range.is_empty() && block.range.start == at)
            }) {
                block.id = old.id;
            }
        }
        *next_projected_block_id =
            allocate_unassigned_block_ids(&mut next, *next_projected_block_id)?;
        Some(
            old_flow
                .splice(
                    indices,
                    next,
                    old_formatted.end,
                    new_formatted_end,
                    &mut range_stats,
                )
                .ok_or(BlockIdentityError::InvalidProjection)?,
        )
    } else {
        None
    };

    let literal_mapping = previous.literal_encoding.is_some() && regional.literal_encoding == previous.literal_encoding;
    let style_indices = if literal_mapping {
        let first = previous.styles.query_overlapping(&old_formatted).iter()
            .map(|span| span.range.start).min().unwrap_or(old_formatted.start);
        previous.styles.partition_point_start(first)..previous.styles.partition_point_start(old_formatted.end)
    } else { contained_interval_indices(&previous.styles, &old_formatted)? };
    let mut regional_styles = regional
        .styles
        .as_slice()
        .iter()
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
    let styles = previous
        .styles
        .splice(
            style_indices,
            regional_styles,
            old_formatted.end,
            new_formatted_end,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

    let provenance_indices = if literal_mapping {
        let mut first = previous.provenance.partition_point_start(old_formatted.start);
        if first > 0 && previous.provenance.get(first - 1).is_some_and(|span| span.formatted.end > old_formatted.start) {
            first -= 1;
        }
        first..previous.provenance.partition_point_start(old_formatted.end)
    } else { contained_interval_indices(&previous.provenance, &old_formatted)? };
    let old_provenance = previous
        .provenance
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
        &previous.provenance,
        &provenance_indices,
        &mapping_source,
        |span| &span.source,
    )?;
    let mut regional_provenance = regional
        .provenance
        .as_slice()
        .iter()
        .map(|span| {
            Ok(ProvenanceSpan {
                formatted: shift_region_range(&span.formatted, old_formatted.start)?,
                source: span.source.clone(),
            })
        })
        .collect::<Result<Vec<_>, BlockIdentityError>>()?;
    if literal_mapping {
        if let Some(first) = old_provenance.first().filter(|span| span.formatted.start < old_formatted.start) {
            let prefix = previous.clip_literal_span(first.clone(), &(first.formatted.start..old_formatted.start));
            if prefix.formatted.end != old_formatted.start { return Err(BlockIdentityError::InvalidProjection); }
            regional_provenance.insert(0, prefix);
        }
        if let Some(last) = old_provenance.last().filter(|span| span.formatted.end > old_formatted.end) {
            let suffix = previous.clip_literal_span(last.clone(), &(old_formatted.end..last.formatted.end));
            if suffix.formatted.start != old_formatted.end { return Err(BlockIdentityError::InvalidProjection); }
            let formatted = shift_range_i128(&suffix.formatted, new_formatted_end as i128 - old_formatted.end as i128)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            let source = shift_range_i128(&suffix.source, new_source.end as i128 - old_source.end as i128)
                .ok_or(BlockIdentityError::InvalidProjection)?;
            regional_provenance.push(ProvenanceSpan { formatted, source });
        }
    }
    // A source-reordered projection needs full reprojection when an edit can
    // shift its two orderings differently. Ordinary text/rich paragraphs
    // splice both persistent indexes without walking the untouched suffix.
    if !previous.source_ordered || !regional.source_ordered {
        return Err(BlockIdentityError::InvalidProjection);
    }
    let mut regional_boundaries = source_text_boundaries(regional_provenance.iter().cloned());
    regional_boundaries.extend(
        previous
            .source_boundaries
            .query_touching(&(mapping_source.start..mapping_source.start))
            .into_iter()
            .filter(|boundary| boundary.formatted <= old_formatted.start),
    );
    let formatted_delta = new_formatted_end as i128 - old_formatted.end as i128;
    regional_boundaries.extend(
        previous
            .source_boundaries
            .query_touching(&(mapping_source.end..mapping_source.end))
            .into_iter()
            .filter(|boundary| boundary.formatted >= old_formatted.end)
            .filter_map(|boundary| {
                let source = shift_range_i128(&boundary.source, new_source.end as i128 - old_source.end as i128)?;
                boundary.with_transform(source, formatted_delta, None)
            }),
    );
    normalize_source_boundaries(&mut regional_boundaries);
    let boundary_indices = previous
        .source_boundaries
        .partition_point_start(mapping_source.start)
        ..mapping_source
            .end
            .checked_add(1)
            .map_or(previous.source_boundaries.len(), |end| {
                previous.source_boundaries.partition_point_start(end)
            });
    let old_boundaries = previous
        .source_boundaries
        .get_range(&boundary_indices)
        .ok_or(BlockIdentityError::InvalidProjection)?;
    let (boundary_prefix, boundary_suffix) = shared_transformed_edges(
        &old_boundaries,
        &regional_boundaries,
        |old, new| old == new,
        |old, new| {
            transformed_source_boundary_eq(
                old,
                new,
                old_source.end,
                new_source.end,
                old_formatted.end,
                new_formatted_end,
            )
        },
    );
    let narrowed_boundary_indices = boundary_indices.start + boundary_prefix
        ..boundary_indices.end - boundary_suffix;
    let narrowed_regional_boundaries = regional_boundaries
        [boundary_prefix..regional_boundaries.len() - boundary_suffix]
        .to_vec();
    let source_boundaries = previous
        .source_boundaries
        .splice_transformed(
            narrowed_boundary_indices,
            narrowed_regional_boundaries,
            old_source.end,
            new_source.end,
            Some((old_formatted.end, new_formatted_end)),
            None,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;
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
    let provenance = previous
        .provenance
        .splice_transformed(
            narrowed_provenance_indices,
            narrowed_regional_provenance,
            old_formatted.end,
            new_formatted_end,
            Some((old_source.end, new_source.end)),
            None,
            &mut range_stats,
        )
        .ok_or(BlockIdentityError::InvalidProjection)?;

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
        .as_slice()
        .iter()
        .cloned()
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

    let candidate = FormattedDocument {
        revision,
        text: target_text,
        flat_text: Arc::new(OnceLock::new()),
        blocks,
        hard_lines,
        flow_lines,
        flow_blocks,
        styles,
        provenance,
        literal_encoding: regional.literal_encoding,
        source_boundaries,
        source_ordered: true,
        decoding_diagnostics,
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
    let indices =
        store.partition_point_start(region.start)..store.partition_point_start(region.end);
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
    old: &ProvenanceSpan,
    new: &ProvenanceSpan,
    old_formatted_end: usize,
    new_formatted_end: usize,
    old_source_end: usize,
    new_source_end: usize,
) -> bool {
    let formatted_shift = new_formatted_end as i128 - old_formatted_end as i128;
    let source_shift = new_source_end as i128 - old_source_end as i128;
    shift_range_i128(&old.formatted, formatted_shift).as_ref() == Some(&new.formatted)
        && shift_range_i128(&old.source, source_shift).as_ref() == Some(&new.source)
}

fn transformed_source_boundary_eq(
    old: &SourceTextBoundary,
    new: &SourceTextBoundary,
    old_source_end: usize,
    new_source_end: usize,
    old_formatted_end: usize,
    new_formatted_end: usize,
) -> bool {
    if old.side != new.side {
        return false;
    }
    let source_shift = new_source_end as i128 - old_source_end as i128;
    let formatted_shift = new_formatted_end as i128 - old_formatted_end as i128;
    shift_range_i128(&old.source, source_shift).as_ref() == Some(&new.source)
        && shift_range_i128(&(old.formatted..old.formatted), formatted_shift)
            .is_some_and(|range| range.start == new.formatted)
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
        .units
        .iter()
        .map(|unit| ProvenanceSpan {
            formatted: unit.normalized.clone(),
            source: unit.source.clone(),
        })
        .collect();
    let decoding_diagnostics = normalized
        .units
        .iter()
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
    let mut projection = FormattedDocument::from_parts(
        revision,
        normalized.text.clone(),
        blocks_for_hard_line_ranges(&hard_lines),
        Vec::new(),
        provenance,
        decoding_diagnostics,
        StyleSheet::default(),
        source_content_start,
        source_content_end,
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
        StyleSheet::default(),
        0,
        text_len,
    )
}

fn project_markdown(
    normalized: &NormalizedText,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    preserve_markers: bool,
) -> FormattedDocument {
    let quotes = super::markdown_quotes::classify(normalized);
    let quote_body = super::markdown_quotes::strip(normalized, &quotes);
    let quote_context = super::markdown_quotes::source_context(normalized);
    let list_context = super::markdown_blocks::source_context(&quote_body);
    if preserve_markers {
        let (cooked, explicit) = super::paragraph_flow::markdown_source(normalized);
        let soft = super::paragraph_flow::markdown_soft_breaks(normalized);
        let soft_sources = normalized
            .endings
            .iter()
            .filter(|ending| soft.contains(&ending.normalized.start))
            .map(|ending| (ending.source.start, ending.source.end))
            .collect::<std::collections::BTreeSet<_>>();
        let cooked_soft = cooked
            .endings
            .iter()
            .filter(|ending| soft_sources.contains(&(ending.source.start, ending.source.end)))
            .map(|ending| ending.normalized.start)
            .collect();
        let mut projected = project_markdown_lines(
            &cooked,
            revision,
            source_content_start,
            source_content_end,
            true,
            &list_context,
            &quote_context,
        );
        let flows = super::paragraph_flow::flow_ranges(&cooked, &cooked_soft);
        // Source keeps physical lines, while inline link labels/destinations
        // may cross a soft source break within the same paragraph. Recognize
        // those intervals without changing any source-visible text or blocks.
        let mut multiline_links = Vec::new();
        for flow in &flows {
            if !cooked.text[flow.clone()].contains('\n') { continue; }
            if projected.blocks_for_region(flow).iter().any(|block| block.style.0 == "Code Block") {
                continue;
            }
            for link in super::links::markdown_links_in(&cooked.text, flow.clone()) {
                if cooked.text[link.range.clone()].contains('\n') {
                    multiline_links.push(StyleSpan {
                        range: link.range,
                        application: StyleApplication::Automatic("Link".into()),
                    });
                }
            }
        }
        projected.append_link_styles(multiline_links);
        let mut paragraphs: Vec<Block> = Vec::new();
        let mut flow_index = 0;
        for block in projected.blocks() {
            while flow_index + 1 < flows.len() && flows[flow_index].end < block.range.start {
                flow_index += 1;
            }
            let join = paragraphs.last().is_some_and(|previous| {
                previous.style == block.style && previous.style.0 != "Code Block" && block.style.0 != "Code Block"
                    && (flows[flow_index].start <= previous.range.start && block.range.end <= flows[flow_index].end
                        || (previous.kind == BlockKind::Paragraph && block.kind == BlockKind::Paragraph
                            || matches!((&previous.kind, &block.kind),
                                (BlockKind::ListItem { ordered:a, ordinal:b, level:c, .. }, BlockKind::ListItem { ordered:d, ordinal:e, level:f, item_start:false, .. })
                                if a == d && b == e && c == f))
                            && projected.provenance_for_region(&(previous.range.end..block.range.start)).iter()
                                .any(|span| !span.formatted.is_empty() && explicit.contains(&span.source.end)))
            });
            if join {
                paragraphs.last_mut().unwrap().range.end = block.range.end;
            } else {
                paragraphs.push(block.clone());
            }
        }
        projected.install_paragraph_partition(paragraphs);
        projected.install_flow_ranges(flows);
        return projected;
    }
    let (cooked, explicit) = super::paragraph_flow::markdown(normalized);
    let mut projected = project_markdown_lines(
        &cooked,
        revision,
        source_content_start,
        source_content_end,
        false,
        &list_context,
        &quote_context,
    );
    if let Some(ending) = normalized
        .endings
        .last()
        .filter(|ending| ending.normalized.end == normalized.text.len())
    {
        if cooked
            .units
            .last()
            .map_or(true, |unit| unit.source.end <= ending.source.start)
        {
            projected.source_insertion_end = ending.source.start;
        }
    }
    if !explicit.is_empty() {
        let mut paragraphs: Vec<Block> = Vec::new();
        for block in projected.blocks() {
            let join = paragraphs.last().is_some_and(|previous| {
                (previous.kind == BlockKind::Paragraph && block.kind == BlockKind::Paragraph
                    || matches!((&previous.kind, &block.kind),
                        (BlockKind::ListItem { ordered:a, ordinal:b, level:c, .. }, BlockKind::ListItem { ordered:d, ordinal:e, level:f, item_start:false, .. })
                        if a == d && b == e && c == f))
                    && previous.style.0 != "Code Block"
                    && block.style.0 != "Code Block"
                    && previous.style == block.style
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
    projected
}

fn markdown_presented_kind(mut kind: BlockKind, preserve_markers: bool) -> BlockKind {
    if let BlockKind::ListItem {
        marker_is_decoration,
        ..
    } = &mut kind
    {
        *marker_is_decoration = !preserve_markers;
    }
    kind
}

fn project_markdown_lines(
    normalized: &NormalizedText,
    revision: Revision,
    source_content_start: usize,
    source_content_end: usize,
    preserve_markers: bool,
    list_context: &[(Range<usize>, super::markdown_blocks::ListLine)],
    quote_context: &[super::markdown_quotes::QuoteLine],
) -> FormattedDocument {
    let mut builder = MarkdownBuilder::new(
        &normalized.text,
        &normalized.units,
        normalized.encoding,
        revision,
    );
    builder.preserve_markers = preserve_markers;
    let input_lines = normalized_hard_line_ranges(normalized);
    let mut hard_breaks = Vec::new();

    let mut line_index = 0;
    while line_index < input_lines.len() {
        let line = &input_lines[line_index];
        let output_start = builder.output.len();
        let source_at = Some(builder.unit_at(line.start).map_or(source_content_end, |unit| unit.source.start));
        let quote = source_at.and_then(|at| quote_context
            .get(quote_context.partition_point(|line| line.range.start <= at).saturating_sub(1))
            .filter(|line| line.range.start <= at && at <= line.range.end));
        let quoted = quote.is_some_and(|line| line.depth > 0);
        let semantic_start = quote.filter(|_| preserve_markers).map_or(line.start, |quote| {
            builder.units.get(builder.units.partition_point(|unit| unit.source.start < quote.content_start))
                .map_or(line.end, |unit| unit.normalized.start).min(line.end)
        });
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
        if let Some((delimiter, length)) = markdown_fence(&normalized.text[semantic_start..line.end]) {
            let mut closing = line_index + 1;
            while closing < input_lines.len() {
                let raw = &normalized.text[input_lines[closing].clone()];
                let body = if preserve_markers && quoted {
                    raw[super::markdown_quotes::prefix(raw)..].trim()
                } else { raw.trim() };
                if body.len() >= length && body.bytes().all(|c| c == delimiter) {
                    break;
                }
                closing += 1;
            }
            let after = (closing + 1).min(input_lines.len());
            let body_start = if preserve_markers {
                line_index
            } else {
                line_index + 1
            };
            let body_end = if preserve_markers { after } else { closing };
            if body_start < body_end {
                for index in body_start..body_end {
                    let output_start = builder.output.len();
                    builder.emit_range(input_lines[index].start, input_lines[index].end);
                    if input_lines[index].is_empty() {
                        let at = builder
                            .unit_at(input_lines[index].start)
                            .map_or(source_content_end, |unit| unit.source.start);
                        builder.provenance.push(ProvenanceSpan {
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
                {
                    builder.blocks.push(Block::new(0, output_start..builder.output.len(), markdown_presented_kind(
                            context.map_or(BlockKind::Paragraph, |context| context.kind.clone()),
                            preserve_markers,
                        ), if quoted { "Block quote" } else { "Code Block" }.into(), None));
                }
            } else {
                let body_at = input_lines
                    .get(line_index + 1)
                    .map_or(line.end, |line| line.start);
                let source_at = builder
                    .unit_at(body_at)
                    .map_or(source_content_end, |unit| unit.source.start);
                builder.provenance.push(ProvenanceSpan {
                    formatted: output_start..output_start,
                    source: source_at..source_at,
                });
                builder.blocks.push(Block::new(0, output_start..output_start, markdown_presented_kind(
                        context.map_or(BlockKind::Paragraph, |context| context.kind.clone()),
                        preserve_markers,
                    ), if quoted { "Block quote" } else { "Code Block" }.into(), None));
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
            content_start = builder
                .units
                .get(
                    builder
                        .units
                        .partition_point(|unit| unit.source.start < context.content_start),
                )
                .map_or(line.end, |unit| unit.normalized.start)
                .min(line.end);
        } else if matches!(kind, BlockKind::ListItem { .. }) {
            kind = BlockKind::Paragraph;
            content_start = semantic_start;
        }
        if preserve_markers {
            builder.emit_range(line.start, content_start);
        }
        kind = markdown_presented_kind(kind, preserve_markers);
        builder.parse_inline(content_start, line.end);
        let output_end = builder.output.len();
        if output_start == output_end && (matches!(kind, BlockKind::ListItem { .. } | BlockKind::Heading(_)) || quoted) {
            // Structural prefixes are source syntax. Empty blocks retain the
            // boundary after their prefix, including an empty ATX heading.
            let content_at = builder.unit_at(content_start)
                .map_or(source_content_end, |unit| unit.source.start);
            let at = if matches!(kind, BlockKind::Heading(_)) { content_at } else {
                context.map_or_else(|| quote.map_or(content_at, |quote| quote.content_start), |context| context.content_start)
            };
            builder.provenance.push(ProvenanceSpan {
                formatted: output_start..output_start,
                source: at..at,
            });
        }
        let style = if quoted { "Block quote".into() } else { match kind {
            BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
            BlockKind::Paragraph => "Paragraph".into(),
            BlockKind::ListItem { ordered, level, .. } => {
                StyleId(format!("{}{}", if ordered { "NumberedList" } else { "BulletedList" }, u16::from(level).min(3) + 1))
            }
        }};
        builder.blocks.push(Block::new(0, output_start..output_end, kind, style, None));
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
    let mut projection = FormattedDocument::from_parts(
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
    let mut start = 0;
    let mut lines = Vec::with_capacity(hard_breaks.len() + 1);
    for at in hard_breaks {
        lines.push(start..at);
        start = at + 1;
    }
    lines.push(start..text_len);
    projection.install_hard_line_partition(lines);
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
    preserve_markers: bool,
    source_text: &'a str,
    units: &'a [LogicalUnit],
    output: String,
    inline_hard_breaks: Vec<usize>,
    blocks: Vec<Block>,
    styles: Vec<StyleSpan>,
    provenance: Vec<ProvenanceSpan>,
    decoding_diagnostics: Vec<DecodingDiagnostic>,
    encoding: super::Encoding,
    revision: Revision,
}

impl<'a> MarkdownBuilder<'a> {
    fn new(
        source_text: &'a str,
        units: &'a [LogicalUnit],
        encoding: super::Encoding,
        revision: Revision,
    ) -> Self {
        Self {
            preserve_markers: false,
            source_text,
            units,
            output: String::new(),
            inline_hard_breaks: Vec::new(),
            blocks: Vec::new(),
            styles: Vec::new(),
            provenance: Vec::new(),
            decoding_diagnostics: Vec::new(),
            encoding,
            revision,
        }
    }

    fn parse_inline(&mut self, start: usize, end: usize) {
        self.parse_inline_depth(start, end, 0);
    }

    fn parse_inline_depth(&mut self, start: usize, end: usize, depth: usize) {
        if depth >= 64 {
            self.emit_range_with_escapes(start, end);
            return;
        }
        let mut at = start;
        while at < end {
            if self.source_text[at..].starts_with('\\') {
                if let Some(next) = self.next_boundary(at) {
                    if next < end {
                        let escaped = self.source_text[next..].chars().next().unwrap();
                        if escaped.is_ascii_punctuation() {
                            if self.preserve_markers {
                                self.emit_range(at, next + escaped.len_utf8());
                            } else {
                                self.emit_escaped(at, next);
                            }
                            at = next + escaped.len_utf8();
                            continue;
                        }
                    }
                }
            }

            if !self.preserve_markers {
                if let Some((length, character)) = markdown_inline_character_reference(&self.source_text[at..end]) {
                    if let (Some(first), Some(last)) = (self.unit_at(at), self.unit_at(at + length - 1)) {
                        let source = first.source.start..last.source.end;
                        let output_start = self.output.len();
                        self.output.push(character);
                        self.provenance.push(ProvenanceSpan {
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
                        self.provenance.push(ProvenanceSpan {
                            formatted: output_start..self.output.len(), source,
                        });
                        at += length;
                        continue;
                    }
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
                    if self.preserve_markers {
                        self.emit_range(at, close + length);
                    } else {
                        let mut body = at + length..close;
                        let text = &self.source_text[body.clone()];
                        if text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty() {
                            body.start += 1;
                            body.end -= 1;
                        }
                        self.emit_range(body.start, body.end);
                    }
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

            if let Some(link) = super::links::markdown_inline_at(self.source_text, at, end) {
                let output_start = self.output.len();
                if self.preserve_markers {
                    self.emit_range(link.range.start, link.range.end);
                } else {
                    self.parse_inline_depth(link.label.start, link.label.end, depth + 1);
                }
                if output_start < self.output.len() {
                    self.styles.push(StyleSpan {
                        range: output_start..self.output.len(),
                        application: StyleApplication::Automatic("Link".into()),
                    });
                }
                at = link.range.end;
                continue;
            }

            // Canonical combined emphasis uses a triple delimiter. If its
            // closing run is split, the first inner closing run determines
            // whether the outer role is strong or emphasis.
            let triple = if self.source_text[at..].starts_with("***") {
                Some("***")
            } else if self.source_text[at..].starts_with("___") {
                Some("___")
            } else {
                None
            };
            let mut triple_outer_single = false;
            if let Some(marker) = triple {
                if let Some(close) = self.find_marker(at + 3, end, marker) {
                    let output_start = self.output.len();
                    if self.preserve_markers {
                        self.emit_range(at, at + 3);
                    }
                    self.parse_inline_depth(at + 3, close, depth + 1);
                    if self.preserve_markers {
                        self.emit_range(close, close + 3);
                    }
                    self.push_semantic_style(output_start, SemanticInlineStyle::Strong);
                    self.push_semantic_style(output_start, SemanticInlineStyle::Emphasis);
                    at = close + 3;
                    continue;
                }
                let byte = marker.as_bytes()[0];
                let mut probe = at + 3;
                while probe < end {
                    if self.source_text.as_bytes()[probe] == b'\\' {
                        probe = self
                            .next_boundary(probe)
                            .and_then(|p| self.next_boundary(p))
                            .unwrap_or(end);
                        continue;
                    }
                    if self.source_text.as_bytes()[probe] == byte {
                        triple_outer_single =
                            self.source_text.as_bytes().get(probe + 1) == Some(&byte);
                        break;
                    }
                    probe = self.next_boundary(probe).unwrap_or(end);
                }
            }
            let double = if triple_outer_single {
                None
            } else if self.source_text[at..].starts_with("**") {
                Some("**")
            } else if self.source_text[at..].starts_with("__") {
                Some("__")
            } else {
                None
            };
            if let Some(marker) = double {
                let inner = at + marker.len();
                if let Some(close) = self.find_marker(inner, end, marker) {
                    let output_start = self.output.len();
                    if self.preserve_markers {
                        self.emit_range(at, inner);
                    }
                    self.parse_inline_depth(inner, close, depth + 1);
                    if self.preserve_markers {
                        self.emit_range(close, close + marker.len());
                    }
                    self.push_semantic_style(output_start, SemanticInlineStyle::Strong);
                    at = close + marker.len();
                    continue;
                }
            }

            let marker = if self.source_text[at..].starts_with('*') {
                Some("*")
            } else if self.source_text[at..].starts_with('_') {
                Some("_")
            } else {
                None
            };
            if let Some(marker) = marker {
                let inner = at + 1;
                if let Some(close) = self.find_marker(inner, end, marker) {
                    let output_start = self.output.len();
                    if self.preserve_markers {
                        self.emit_range(at, inner);
                    }
                    self.parse_inline_depth(inner, close, depth + 1);
                    if self.preserve_markers {
                        self.emit_range(close, close + marker.len());
                    }
                    self.push_semantic_style(output_start, SemanticInlineStyle::Emphasis);
                    at = close + 1;
                    continue;
                }
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
        self.provenance.push(ProvenanceSpan {
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
        self.provenance.push(ProvenanceSpan {
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

    fn unit_at(&self, normalized: usize) -> Option<&LogicalUnit> {
        self.units
            .binary_search_by_key(&normalized, |unit| unit.normalized.start)
            .ok()
            .map(|index| &self.units[index])
    }

    fn next_boundary(&self, normalized: usize) -> Option<usize> {
        self.unit_at(normalized).map(|unit| unit.normalized.end)
    }

    fn find_marker(&self, mut at: usize, end: usize, marker: &str) -> Option<usize> {
        let delimiter = marker.as_bytes()[0];
        let styled = delimiter == b'*' || delimiter == b'_';
        let mut nested = false;
        while at + marker.len() <= end {
            if self.source_text[at..].starts_with('\\') {
                at = self.next_boundary(at)?;
                if at < end {
                    at = self.next_boundary(at)?;
                }
                continue;
            }
            if styled && self.source_text.as_bytes()[at] == delimiter {
                let mut run = 1;
                while at + run < end && self.source_text.as_bytes()[at + run] == delimiter {
                    run += 1;
                }
                match marker.len() {
                    3 if run >= 3 => return Some(at),
                    2 if run >= 2 => return Some(at + usize::from(nested && run >= 3)),
                    2 if run == 1 => nested = !nested,
                    1 if run == 1 && !nested => return Some(at),
                    1 if run >= 3 && nested => return Some(at + 2),
                    1 if run == 2 => {
                        if !nested {
                            let mut probe = at + run;
                            let mut closes_double = false;
                            while probe < end {
                                if self.source_text.as_bytes()[probe] == b'\\' {
                                    probe = self
                                        .next_boundary(probe)
                                        .and_then(|p| self.next_boundary(p))
                                        .unwrap_or(end);
                                    continue;
                                }
                                if self.source_text.as_bytes()[probe] == delimiter {
                                    closes_double = self.source_text.as_bytes().get(probe + 1)
                                        == Some(&delimiter);
                                    break;
                                }
                                probe = self.next_boundary(probe).unwrap_or(end);
                            }
                            if !closes_double {
                                return Some(at);
                            }
                        }
                        nested = !nested;
                    }
                    _ => {}
                }
                at += run;
                continue;
            }
            if !styled && self.source_text[at..].starts_with(marker) {
                return Some(at);
            }
            at = self.next_boundary(at)?;
        }
        None
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
    (bytes.get(at) == Some(&b'>')).then_some(at + 1)
}

/// Numeric character references preserve authored prose in its source encoding.
/// Controls other than TAB stay literal: a reference is not an implicit hard
/// break, source NUL replacement, or carriage-return normalization instruction.
/// Source views and code retain their literal spelling, like inline br syntax.
fn markdown_inline_character_reference(text: &str) -> Option<(usize, char)> {
    let body = text.strip_prefix("&#")?;
    let end = body.find(';').filter(|end| *end <= 8)?;
    let digits = &body[..end];
    let value = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
        u32::from_str_radix(hex, 16).ok()?
    } else {
        if !digits.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
        digits.parse::<u32>().ok()?
    };
    let character = char::from_u32(value)?;
    if character.is_control() && character != '\t' { return None; }
    Some((end + 3, character))
}

pub(crate) fn escape_markdown_insert(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '_' | '`' | '#' | '>') {
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
        assert!(projection.provenance.len() <= source.len().div_ceil(super::super::encoding::MAPPING_CHUNK_BYTES));
        assert!(projection.source_boundaries.len() <= projection.provenance.len() * 2);
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

    #[test]
    fn inherited_blocks_are_compact_and_direct_formatting_is_copy_on_write() {
        assert_eq!(std::mem::size_of::<Block>(), 32);
        let original = Block::paragraph(1, 0..1);
        let another = Block::paragraph(2, 2..3);
        assert!(Arc::ptr_eq(&original.attributes, &another.attributes));
        let mut styled = original.clone();
        assert!(Arc::ptr_eq(&original.attributes, &styled.attributes));
        styled.style = "Heading1".into();
        styled.kind = BlockKind::Heading(1);
        styled.direct_default_character.size = Some(24.0);
        styled.direct_paragraph.spacing_before = Some(2.0);
        assert!(original.direct_formatting.is_none());
        assert_eq!(original.style.0, "Paragraph");
        assert_eq!(original.kind, BlockKind::Paragraph);
        assert!(!Arc::ptr_eq(&original.attributes, &styled.attributes));
        assert_eq!(original.direct_default_character.size, None);
        let mut changed = styled.clone();
        assert!(Arc::ptr_eq(styled.direct_formatting.as_ref().unwrap(), changed.direct_formatting.as_ref().unwrap()));
        changed.direct_default_character.size = Some(30.0);
        assert_eq!(styled.direct_default_character.size, Some(24.0));
        assert_eq!(changed.direct_default_character.size, Some(30.0));
        assert_eq!(changed.direct_paragraph.spacing_before, Some(2.0));
        let mut explicit_empty = original.clone();
        explicit_empty.direct_default_character.size = None;
        assert_eq!(explicit_empty, original);
    }

    #[test]
    fn literal_blocks_share_attributes_across_open_edits_and_history() {
        use crate::document::Document;
        for format in [Format::PlainText, Format::Code] {
            let mut document = Document::from_bytes(b"a\n".repeat(10_000), Encoding::Utf8, format).unwrap();
            let initial = document.projection().blocks.get(0).unwrap();
            for index in [1, 5000, 10_000] {
                assert!(Arc::ptr_eq(&initial.attributes, &document.projection().blocks.get(index).unwrap().attributes));
            }
            document.insert(3, "x\ny").unwrap();
            assert_eq!(initial.kind, BlockKind::Paragraph);
            assert_eq!(initial.style.0, "Paragraph");
            assert!(document.undo());
            assert!(Arc::ptr_eq(&initial.attributes, &document.projection().blocks.get(0).unwrap().attributes));
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
    fn relocating_a_synthetic_separator_keeps_real_source_and_boundary_mappings() {
        let source = "<p>A<span style=\"white-space: pre-wrap\"> </span></p><p>B</p>";
        let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
        let normalized = normalize(&decoded, FileFormat::Unix);
        let document = project(&normalized, Format::Html, Revision(1), 0, source.len());
        let from = source.find("</span>").unwrap();
        let to = from + "</span>".len();
        let (relocated, _) = document
            .relocate_synthetic_hard_line_source_boundary(2, from, to)
            .unwrap()
            .unwrap();
        assert_eq!(relocated.text(), document.text());
        assert_eq!(relocated.blocks(), document.blocks());
        let old = document.provenance();
        let new = relocated.provenance();
        assert_eq!(old.len(), new.len());
        for (before, after) in old.iter().zip(new) {
            if before.formatted == (2..3) {
                assert_eq!(after.source, to..to);
            } else {
                assert_eq!(before, after);
            }
        }
        assert_eq!(
            relocated
                .map_source_boundary(Revision(1), from, BoundaryAffinity::Upstream)
                .unwrap()
                .formatted_offset,
            2
        );
        assert_eq!(
            relocated
                .map_source_boundary(Revision(1), to, BoundaryAffinity::Downstream)
                .unwrap()
                .formatted_offset,
            2
        );
        assert_eq!(
            relocated
                .map_source_boundary(Revision(1), to, BoundaryAffinity::Upstream)
                .unwrap()
                .formatted_offset,
            3
        );
        assert!(document
            .relocate_synthetic_hard_line_source_boundary(2, from + 1, to)
            .unwrap()
            .is_none());
        assert!(document
            .relocate_synthetic_hard_line_source_boundary(0, 3, to)
            .unwrap()
            .is_none());
        assert_eq!(
            document
                .relocate_synthetic_hard_line_source_boundary(
                    2,
                    from,
                    source.find('B').unwrap() + 1
                )
                .err(),
            Some(BlockIdentityError::InvalidProjection)
        );
        let source = "<pre>A\nB</pre>";
        let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
        let normalized = normalize(&decoded, FileFormat::Unix);
        let document = project(&normalized, Format::Html, Revision(1), 0, source.len());
        assert!(document
            .relocate_synthetic_hard_line_source_boundary(1, 6, 7)
            .unwrap()
            .is_none());
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
        document.document_style.direct_default_character.size = Some(16.0);
        let mut blocks = document.blocks.to_vec();
        blocks[0].direct_paragraph.spacing_after = Some(9.0);
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
            block.direct_paragraph.spacing_after = Some(9.0);
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
            .all(|block| block.direct_paragraph.spacing_after == Some(9.0)));
    }

    #[test]
    fn batched_grapheme_provenance_matches_checked_point_queries() {
        for (format, source) in [
            (
                Format::PlainText,
                "A\r\nB e\u{301} क्‍ष 👩🏽‍💻 🇺🇸🇨🇦\n\u{301}end",
            ),
            (
                Format::MarkdownSource,
                "# e\u{301} **bold**\n\n- item\n  continuation\n\n```\ncode\n\n```",
            ),
            (
                Format::Markdown,
                "# e\u{301} **bold**\n\n- item\n  continuation\n\n```\ncode\n\n```",
            ),
            (
                Format::Html,
                "<p>A&amp;B<i></i>é👩🏽‍💻</p><ul><li>item</li></ul>",
            ),
            (
                Format::Html,
                "<table>before<tr><td>cell</td></tr>after</table>",
            ),
            (Format::Rtf, "{\\rtf1 A{\\b }B\\par C}"),
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
            assert!(projection.blocks.iter().all(|block| {
                block.direct_paragraph == BlockProperties::default()
                    && block.direct_default_character == CharacterProperties::default()
            }));
        }
        assert_eq!(plain.blocks[0].style, plain.style_sheet.base_paragraph);
        assert_eq!(markdown.blocks[0].style, StyleId::from("Heading1"));
        assert_eq!(
            markdown.blocks[1].style,
            markdown.style_sheet.base_paragraph
        );
    }

    #[test]
    fn unchanged_projection_and_history_preserve_all_direct_assignments() {
        let mut previous = plain("first\nsecond".to_owned(), Revision(1));
        previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_root = previous.document_style.clone();
        let expected_block = previous.blocks[0].clone();

        let mut candidate = plain("first\nsecond".to_owned(), Revision(2));
        candidate.install_unchanged_block_ids(&previous).unwrap();
        assert_eq!(candidate.document_style, expected_root);
        assert_eq!(candidate.blocks[0], expected_block);

        let mut history = History::new(previous);
        history.commit(candidate, false);
        assert!(history.undo());
        assert_eq!(history.current().document_style, expected_root);
        assert_eq!(history.current().blocks[0], expected_block);
        assert!(history.redo());
        assert_eq!(history.current().document_style, expected_root);
        assert_eq!(history.current().blocks[0], expected_block);
    }

    #[test]
    fn text_reconciliation_carries_assignments_with_the_retained_block() {
        let old_text = "first\nsecond".to_owned();
        let new_text = "prefix first\nsecond".to_owned();
        let mut previous = plain(old_text.clone(), Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_root = previous.document_style.clone();
        let expected_id = previous.blocks[0].id;
        let expected_paragraph = previous.blocks[0].direct_paragraph.clone();
        let expected_character = previous.blocks[0].direct_default_character.clone();
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
        assert_eq!(candidate.blocks[0].id, expected_id);
        assert_eq!(candidate.blocks[0].direct_paragraph, expected_paragraph);
        assert_eq!(
            candidate.blocks[0].direct_default_character,
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
            let old_id = previous.blocks[0].id;
            let expected_paragraph = previous.blocks[0].direct_paragraph.clone();
            let expected_character = previous.blocks[0].direct_default_character.clone();
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
            assert!(candidate.blocks.iter().all(|block| {
                block.direct_paragraph == expected_paragraph
                    && block.direct_default_character == expected_character
            }));
            let retained_index = if new_text.starts_with('\n') { 1 } else { 0 };
            assert_eq!(candidate.blocks[retained_index].id, old_id);
        }
    }

    #[test]
    fn split_inheritance_does_not_replace_projected_markdown_block_styles() {
        let old_text = "title";
        let new_text = "title\nbody";
        let mut previous = markdown_at("# title", Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        add_direct_assignments(&mut previous);
        let expected_paragraph = previous.blocks[0].direct_paragraph.clone();
        let expected_character = previous.blocks[0].direct_default_character.clone();
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
        assert_eq!(candidate.blocks[0].style, StyleId::from("Heading1"));
        assert_eq!(candidate.blocks[1].style, StyleId::from("Paragraph"));
        assert!(candidate.blocks.iter().all(|block| {
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
        assert_eq!(candidate.blocks[0].id, previous.blocks[0].id);
        assert_eq!(candidate.blocks[0].kind, BlockKind::Heading(1));
    }

    #[test]
    fn large_block_reconciliation_preserves_all_shifted_ids_without_pairwise_diffing() {
        const LINES: usize = 100_000;
        let old_text = format!("{}tail", "x\n".repeat(LINES));
        let new_text = format!("new\n{old_text}");
        let mut previous = plain(old_text.clone(), Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        let previous_last = previous.blocks.last().unwrap().id;
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
        assert_eq!(candidate.blocks[0].id, next_id);
        assert_eq!(candidate.blocks[1].id, previous.blocks[0].id);
        assert_eq!(candidate.blocks.last().unwrap().id, previous_last);
        assert_eq!(after, next_id + 1);
    }

    #[test]
    fn markdown_projects_headings_and_inline_styles() {
        let projected = markdown("# A **bold** and *soft* with `code`\nplain");
        assert_eq!(projected.text(), "A bold and soft with code\nplain");
        assert_eq!(projected.text_tree().flatten(), projected.text());
        assert_eq!(projected.text_tree().hard_line_count(), 2);
        assert_eq!(projected.text_tree().hard_line_start(1).unwrap(), 26);
        assert_eq!(projected.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(projected.blocks[1].kind, BlockKind::Paragraph);
        assert_eq!(projected.styles.len(), 4);
        assert_eq!(projected.styles[0].range, 2..6);
        assert_eq!(
            projected.styles[0].application,
            StyleApplication::Semantic(SemanticInlineStyle::Strong)
        );
    }

    #[test]
    fn escaped_markdown_is_visible_but_escape_has_provenance() {
        let projected = markdown("\\*literal\\*");
        assert_eq!(projected.text(), "*literal*");
        assert_eq!(projected.provenance[0].source, 0..2);
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
    fn projection_clones_and_equal_reprojections_share_range_indexes() {
        let mut previous = markdown_at("# left\n**bold**", Revision(1));
        previous.assign_initial_block_ids(1).unwrap();
        let cloned = previous.clone();
        assert!(previous.blocks.shares_root_with(&cloned.blocks));
        assert!(previous.hard_lines.shares_root_with(&cloned.hard_lines));
        assert!(previous.styles.shares_root_with(&cloned.styles));

        let mut candidate = markdown_at("# left\n**bold**", Revision(2));
        candidate.install_unchanged_block_ids(&previous).unwrap();
        assert!(previous.blocks.shares_root_with(&candidate.blocks));
        assert!(previous.hard_lines.shares_root_with(&candidate.hard_lines));
        assert!(previous.styles.shares_root_with(&candidate.styles));
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
    fn same_length_local_edit_reuses_equal_block_and_style_indexes() {
        let old_text = "left\nbold";
        let new_text = "LEFT\nbold";
        let mut previous = markdown_at("left\n**bold**", Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        let mut candidate = markdown_at("LEFT\n**bold**", Revision(2));
        let edit = TextEdit::new(0..4, "LEFT");
        let map = PositionMap::for_text(
            DocumentId(1),
            Revision(1),
            Revision(2),
            old_text,
            new_text,
            vec![Splice::new(0..4, 4).unwrap()],
        )
        .unwrap();

        candidate
            .install_reconciled_block_ids(&previous, &[edit], &map, next_id)
            .unwrap();
        assert!(previous.blocks.shares_root_with(&candidate.blocks));
        assert!(previous.hard_lines.shares_root_with(&candidate.hard_lines));
        assert!(previous.styles.shares_root_with(&candidate.styles));
    }

    #[test]
    fn prefix_length_change_shares_shifted_suffix_block_leaves() {
        const LINES: usize = 256;
        let old_text = format!("{}tail", "x\n".repeat(LINES));
        let new_text = format!("prefix {old_text}");
        let mut previous = plain(old_text.clone(), Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        let previous_leaf_count = previous.blocks.leaf_count();
        let mut candidate = plain(new_text.clone(), Revision(2));
        let edit = TextEdit::new(0..0, "prefix ");
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
            .install_reconciled_block_ids(&previous, &[edit], &map, next_id)
            .unwrap();
        assert_eq!(candidate.blocks.leaf_count(), previous_leaf_count);
        assert_eq!(
            candidate.blocks.shared_leaf_count_with(&previous.blocks),
            previous_leaf_count - 1
        );
        assert_eq!(
            candidate
                .hard_lines
                .shared_leaf_count_with(&previous.hard_lines),
            previous_leaf_count - 1
        );
        assert_eq!(candidate.blocks()[1].id, previous.blocks()[1].id);
    }

    #[test]
    fn uniformly_shifted_style_index_reuses_its_complete_relative_root() {
        const SPANS: usize = 200;
        let marked = "**x** ".repeat(SPANS);
        let old_source = format!("a {marked}");
        let new_source = format!("prefix a {marked}");
        let old_text = format!("a {}", "x ".repeat(SPANS));
        let new_text = format!("prefix a {}", "x ".repeat(SPANS));
        let mut previous = markdown_at(&old_source, Revision(1));
        let next_id = previous.assign_initial_block_ids(1).unwrap();
        let mut candidate = markdown_at(&new_source, Revision(2));
        let edit = TextEdit::new(0..0, "prefix ");
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
            .install_reconciled_block_ids(&previous, &[edit], &map, next_id)
            .unwrap();
        assert_eq!(candidate.styles.len(), SPANS);
        assert!(candidate.styles.shares_root_with(&previous.styles));
        assert_eq!(
            candidate.styles.shared_leaf_count_with(&previous.styles),
            candidate.styles.leaf_count()
        );
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
            text_tree: FormattedTextTree::try_from_text(text).unwrap(),
            hard_lines: OrderedRangeStore::new(hard_lines),
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
        const ALL: [Format; 7] = [
            Format::PlainText,
            Format::Markdown,
            Format::MarkdownSource,
            Format::Html,
            Format::HtmlSource,
            Format::Rtf,
            Format::Code,
        ];

        fn members(predicate: fn(Format) -> bool) -> Vec<Format> {
            ALL.into_iter().filter(|format| predicate(*format)).collect()
        }

        assert_eq!(
            members(Format::is_markdown),
            [Format::Markdown, Format::MarkdownSource]
        );
        assert_eq!(members(Format::is_html), [Format::Html, Format::HtmlSource]);
        assert_eq!(
            members(Format::is_source_view),
            [Format::MarkdownSource, Format::HtmlSource]
        );
        assert_eq!(
            members(Format::is_wysiwyg),
            [Format::Markdown, Format::Html, Format::Rtf]
        );
        assert_eq!(members(Format::is_rich_text), [Format::Html, Format::Rtf]);
        assert_eq!(
            members(Format::has_rich_source),
            [Format::Html, Format::HtmlSource, Format::Rtf]
        );
        assert_eq!(
            members(Format::has_structural_lists),
            [
                Format::Markdown,
                Format::MarkdownSource,
                Format::Html,
                Format::Rtf
            ]
        );

        assert_eq!(members(Format::is_code), [Format::Code]);
        assert_eq!(members(Format::is_literal), [Format::PlainText, Format::Code]);
        // Every format is exactly one of literal, WYSIWYG, or source view.
        for format in ALL {
            let kinds = usize::from(format.is_wysiwyg())
                + usize::from(format.is_source_view())
                + usize::from(format.is_literal());
            assert_eq!(kinds, 1, "{format:?} is not exactly one view kind");
        }
    }
}
