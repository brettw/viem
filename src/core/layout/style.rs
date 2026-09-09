//! Adapter from the normalized document style model to layout and paint runs.
//!
//! Source adapters own source-language cascade rules. This module only
//! resolves the already-normalized block/character cascade and separates
//! shape-affecting properties from paint-only properties.

use super::{EdgeInsets, OpenTypeFeature, ResolvedTextStyle, ShapeStyleRun, TextDirection};
use crate::document::{
    Block, CharacterProperties, Color, DocumentStyleAssignment, FontSlant, FormattedDocument,
    FormattedTextError, FormattedTextTree, LineSpacing, ParagraphAlignment, ResolvedCharacterStyle,
    SemanticInlineStyle, StyleApplication, StyleError, StyleId, StyleSheet, StyleSheetRevision,
    StyleSpan, WritingDirection,
};
use std::collections::BTreeSet;
use std::ops::Range;

/// Paint-only character values deliberately kept out of shaping cache keys.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTextPaint {
    pub foreground: Color,
    pub foreground_is_default: bool,
    pub background: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for ResolvedTextPaint {
    fn default() -> Self {
        paint_style(&ResolvedCharacterStyle::default())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaintStyleRun {
    pub text_range: Range<usize>,
    pub paint: ResolvedTextPaint,
}

/// Resolved geometry and default text appearance for one paragraph-bearing
/// block. It is intentionally independent of shaping output so spacing,
/// indentation, and alignment changes can reuse width-independent fragments.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLayoutStyle {
    pub block_id: u64,
    pub text_range: Range<usize>,
    /// Logical label prefix; layout hangs this generated/source-visible furniture
    /// before the item body without changing its text or edit coordinates.
    pub list_marker_range: Option<Range<usize>>,
    /// Noneditable WYSIWYG label, measured separately from formatted text.
    pub list_marker_decoration: Option<String>,
    /// Noneditable left border for the shared HTML/Markdown quote treatment.
    pub quote_border: bool,
    pub marker_paint: ResolvedTextPaint,
    pub spacing_before: f32,
    pub spacing_after: f32,
    pub line_spacing: LineSpacing,
    pub first_line_indent: f32,
    pub leading_indent: f32,
    pub trailing_indent: f32,
    pub alignment: ParagraphAlignment,
    pub base_direction: WritingDirection,
    pub default_shaping_style: ResolvedTextStyle,
}

/// Layout-facing result of resolving one immutable formatted snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentLayoutStyles {
    pub style_sheet_revision: StyleSheetRevision,
    pub document_insets: EdgeInsets,
    pub canvas_background: Color,
    pub canvas_background_is_default: bool,
    pub default_shaping_style: ResolvedTextStyle,
    pub shaping_runs: Vec<ShapeStyleRun>,
    pub default_paint: ResolvedTextPaint,
    pub paint_runs: Vec<PaintStyleRun>,
    pub paragraphs: Vec<ParagraphLayoutStyle>,
}

/// Borrowed normalized style inputs. Projection implementations can use this
/// directly without constructing a `Document`, and tests/configuration layers
/// can exercise explicit style sheets independently of source syntax.
#[derive(Clone, Copy, Debug)]
pub struct DocumentStyleInput<'a> {
    pub text: &'a str,
    pub blocks: &'a [Block],
    pub style_spans: &'a [StyleSpan],
    pub style_sheet: &'a StyleSheet,
    pub document_style: &'a DocumentStyleAssignment,
}

impl<'a> From<&'a FormattedDocument> for DocumentStyleInput<'a> {
    fn from(document: &'a FormattedDocument) -> Self {
        Self {
            text: document.text(),
            blocks: document.blocks(),
            style_spans: document.style_spans(),
            style_sheet: document.style_sheet(),
            document_style: document.document_style(),
        }
    }
}

/// The cascade itself never needs to read document text. Keep its inputs
/// separate from [`DocumentStyleInput`] so a regional resolve can validate
/// global byte boundaries against the persistent text tree without first
/// materializing the complete compatibility string.
#[derive(Clone, Copy, Debug)]
struct StyleCascadeInput<'a> {
    blocks: &'a [Block],
    style_spans: &'a [StyleSpan],
    style_sheet: &'a StyleSheet,
    document_style: &'a DocumentStyleAssignment,
}

impl<'a> From<DocumentStyleInput<'a>> for StyleCascadeInput<'a> {
    fn from(input: DocumentStyleInput<'a>) -> Self {
        Self {
            blocks: input.blocks,
            style_spans: input.style_spans,
            style_sheet: input.style_sheet,
            document_style: input.document_style,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentStyleError {
    Cascade(StyleError),
    TextStorage(FormattedTextError),
    InvalidBlockRange { index: usize },
    InvalidStyleSpan { index: usize },
    MultipleNamedCharacterStyles { range_start: usize },
    InvalidOpenTypeFeatureTag(String),
}

impl From<StyleError> for DocumentStyleError {
    fn from(value: StyleError) -> Self {
        Self::Cascade(value)
    }
}

impl From<FormattedTextError> for DocumentStyleError {
    fn from(value: FormattedTextError) -> Self {
        Self::TextStorage(value)
    }
}

impl DocumentLayoutStyles {
    pub(crate) fn apply_source_quote_policy(&mut self, format: crate::document::Format, flow: bool) {
        if format == crate::document::Format::MarkdownSource && !flow {
            for paragraph in &mut self.paragraphs {
                if paragraph.quote_border {
                    paragraph.quote_border = false;
                    paragraph.leading_indent = 0.0;
                    paragraph.trailing_indent = 0.0;
                    paragraph.first_line_indent = 0.0;
                }
            }
        }
    }

    /// Resolve the content associated with one current logical boundary in
    /// logarithmic plus local style-run work; empty paragraphs need no shaping.
    pub fn character_at(
        document: &FormattedDocument,
        offset: usize,
        upstream: bool,
    ) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
        Self::character_at_with_automatic(document, offset, upstream, true)
    }

    /// The authored cascade, excluding automatically applied source syntax.
    /// Style pickers and typing inheritance use this semantic view.
    pub fn semantic_character_at(
        document: &FormattedDocument,
        offset: usize,
        upstream: bool,
    ) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
        Self::character_at_with_automatic(document, offset, upstream, false)
    }

    fn character_at_with_automatic(
        document: &FormattedDocument,
        offset: usize,
        upstream: bool,
        automatic: bool,
    ) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
        let text = document.text_tree();
        if offset > text.byte_len() || !text.is_char_boundary(offset)? {
            return Err(DocumentStyleError::InvalidBlockRange { index: 0 });
        }
        let empty_paragraph = document
            .blocks_for_region(&(offset..offset))
            .iter()
            .any(|block| block.range == (offset..offset));
        let at = if upstream && offset > 0 && !empty_paragraph {
            text.previous_grapheme_boundary(offset)?.unwrap_or(offset)
        } else {
            offset
        };
        let end = text.next_grapheme_boundary(at)?.unwrap_or(at);
        let blocks = document.blocks_for_region(&(at..at));
        let block = blocks
            .iter()
            .find(|block| block.range.contains(&at))
            .or_else(|| blocks.iter().find(|block| block.range.start == at))
            .or_else(|| blocks.last());
        let Some(block) = block else {
            return Ok(document
                .style_sheet()
                .resolve_document_assignment(document.document_style())?
                .character);
        };
        let range = at..end.min(block.range.end);
        let mut spans = document.style_spans_for_region(&range);
        if !automatic {
            spans.retain(|span| {
                !matches!(
                    span.application,
                    StyleApplication::Automatic(_)
                        | StyleApplication::SourceSyntax
                        | StyleApplication::SourceRawText
                        | StyleApplication::SourcePreservedWhitespace
                )
            });
        }
        resolve_character_at(
            StyleCascadeInput {
                blocks: &blocks,
                style_spans: &spans,
                style_sheet: document.style_sheet(),
                document_style: document.document_style(),
            },
            block,
            range,
        )
    }

    /// Resolve document, paragraph, named-character, semantic, and direct
    /// layers without consulting a platform or mutating the projection.
    pub fn resolve(document: &FormattedDocument) -> Result<Self, DocumentStyleError> {
        Self::resolve_region(document, 0..document.text_tree().byte_len())
    }

    pub(crate) fn source_flow_paragraphs(
        document: &FormattedDocument,
        range: Range<usize>,
    ) -> Result<Option<Vec<ParagraphLayoutStyle>>, DocumentStyleError> {
        let Some(blocks) = document.flow_blocks_for_region(&range) else {
            return Ok(None);
        };
        let resolved = Self::resolve_validated(StyleCascadeInput {
            blocks: &blocks,
            style_spans: &[],
            style_sheet: document.style_sheet(),
            document_style: document.document_style(),
        })?;
        Ok(Some(resolved.paragraphs))
    }

    /// Resolve only blocks and character spans that can affect a contiguous
    /// formatted-text region. Document-root values are still resolved from the
    /// immutable style sheet, while paragraph/run work is proportional to the
    /// materialized region rather than the complete document.
    pub fn resolve_region(
        document: &FormattedDocument,
        text_range: Range<usize>,
    ) -> Result<Self, DocumentStyleError> {
        Self::resolve_region_with_flow(document, text_range, false)
    }

    pub(crate) fn resolve_region_with_flow(
        document: &FormattedDocument,
        text_range: Range<usize>,
        flow: bool,
    ) -> Result<Self, DocumentStyleError> {
        let text = document.text_tree();
        if text_range.start > text_range.end
            || text_range.end > text.byte_len()
            || !text.is_char_boundary(text_range.start)?
            || !text.is_char_boundary(text_range.end)?
        {
            return Err(DocumentStyleError::InvalidBlockRange { index: 0 });
        }
        let flow_blocks = flow
            .then(|| document.flow_blocks_for_region(&text_range))
            .flatten();
        let structural_flow = flow_blocks.is_some();
        let regional_blocks =
            flow_blocks.unwrap_or_else(|| document.blocks_for_region(&text_range));
        let mut regional_spans = document.style_spans_for_region(&text_range);
        if structural_flow {
            // The structural block now supplies the complete paragraph
            // cascade, including empty elements and their surrounding tags.
            // Physical-line character overlays must not reapply a neighbor's
            // paragraph defaults on top of that assignment.
            regional_spans.retain(|span| {
                !matches!(span.application, StyleApplication::SourceParagraph { .. })
            });
        }
        validate_blocks_in_tree(text, &regional_blocks)?;
        validate_spans_in_tree(text, &regional_spans)?;
        let mut styles = Self::resolve_validated(StyleCascadeInput {
            blocks: &regional_blocks,
            style_spans: &regional_spans,
            style_sheet: document.style_sheet(),
            document_style: document.document_style(),
        })?;
        for (paragraph, block) in styles.paragraphs.iter_mut().zip(&regional_blocks) {
            if !structural_flow {
                paragraph.list_marker_range = document
                    .list_marker_range_for_block(block)
                    .filter(|range| !range.is_empty());
            }
        }
        Ok(styles)
    }

    pub fn resolve_input(input: DocumentStyleInput<'_>) -> Result<Self, DocumentStyleError> {
        validate_blocks(input.text, input.blocks)?;
        validate_spans(input.text, input.style_spans)?;
        Self::resolve_validated(input.into())
    }

    fn resolve_validated(input: StyleCascadeInput<'_>) -> Result<Self, DocumentStyleError> {
        let sheet = input.style_sheet;
        let resolved_document = sheet.resolve_document_assignment(input.document_style)?;
        let default_shaping_style = shaping_style(&resolved_document.character)?;
        let default_paint = paint_style(&resolved_document.character);

        let mut shaping_runs = Vec::new();
        let mut paint_runs = Vec::new();
        let mut paragraphs = Vec::with_capacity(input.blocks.len());
        let mut source_quotes: Vec<Range<usize>> = Vec::new();
        for span in input.style_spans {
            if matches!(&span.application, StyleApplication::SourceParagraph { style, .. }
                if style.0 == "Block quote")
            {
                if let Some(previous) = source_quotes
                    .last_mut()
                    .filter(|previous| span.range.start <= previous.end)
                {
                    previous.end = previous.end.max(span.range.end);
                } else {
                    source_quotes.push(span.range.clone());
                }
            }
        }
        for block in input.blocks {
            let mut assigned = Some(&block.style);
            let mut quote_border = false;
            while let Some(id) = assigned {
                if id.0 == "Block quote" {
                    quote_border = true;
                    break;
                }
                assigned = sheet
                    .block_style(id)
                    .and_then(|style| style.based_on.as_ref());
            }
            // HTML Source retains its physical text, including block tags.
            // Its semantic context supplies the quote treatment for that line.
            let source_quote = source_quotes
                .get(source_quotes.partition_point(|range| range.end <= block.range.start))
                .is_some_and(|range| range.start < block.range.end);
            quote_border |= source_quote;
            let quote_id = StyleId::from("Block quote");
            let paragraph = sheet.resolve_assigned_paragraph_style(
                input.document_style,
                if source_quote {
                    &quote_id
                } else {
                    &block.style
                },
                &block.direct_paragraph,
                &block.direct_default_character,
                None,
                &CharacterProperties::default(),
            )?;
            // A heading or code paragraph inside an item keeps its own style
            // plus the containing list's inset. List-role paragraphs already
            // declare that inset themselves.
            let list_inset = if let crate::document::BlockKind::ListItem { ordered, level, .. } =
                block.kind
            {
                let id = sheet.list_style_id(ordered, level);
                let extra = if id.is_internal_list() {
                    32.0 * f32::from(level.saturating_sub(3))
                } else {
                    0.0
                };
                let mut assigned = Some(&block.style);
                let mut inherits_list = false;
                while let Some(ancestor) = assigned {
                    if ancestor == &id {
                        inherits_list = true;
                        break;
                    }
                    assigned = sheet
                        .block_style(ancestor)
                        .and_then(|style| style.based_on.as_ref());
                }
                if inherits_list || sheet.block_style(&id).is_none() {
                    extra
                } else {
                    sheet
                        .resolve_assigned_paragraph_style(
                            input.document_style,
                            &id,
                            &Default::default(),
                            &Default::default(),
                            None,
                            &Default::default(),
                        )?
                        .leading_indent
                        + extra
                }
            } else {
                0.0
            };
            let list_marker_range = match block.kind {
                crate::document::BlockKind::ListItem {
                    ordered,
                    ordinal,
                    item_start: true,
                    marker_is_decoration: false,
                    ..
                } => {
                    let length = if ordered {
                        format!("{ordinal}. ").len()
                    } else {
                        "• ".len()
                    };
                    Some(block.range.start..(block.range.start + length).min(block.range.end))
                }
                _ => None,
            };
            paragraphs.push(ParagraphLayoutStyle {
                block_id: block.id,
                text_range: block.range.clone(),
                list_marker_range,
                quote_border,
                list_marker_decoration: match block.kind {
                    crate::document::BlockKind::ListItem {
                        ordered,
                        ordinal,
                        item_start: true,
                        marker_is_decoration: true,
                        ..
                    } => Some(if ordered {
                        format!("{ordinal}.")
                    } else {
                        "•".into()
                    }),
                    _ => None,
                },
                marker_paint: paint_style(&paragraph.character),
                spacing_before: paragraph.spacing_before,
                spacing_after: paragraph.spacing_after,
                line_spacing: paragraph.line_spacing,
                first_line_indent: if matches!(
                    block.kind,
                    crate::document::BlockKind::ListItem {
                        item_start: false,
                        ..
                    }
                ) && block.direct_paragraph.first_line_indent.is_none()
                {
                    0.0
                } else {
                    paragraph.first_line_indent
                },
                leading_indent: paragraph.leading_indent + list_inset,
                trailing_indent: paragraph.trailing_indent,
                alignment: paragraph.alignment,
                base_direction: paragraph.base_direction,
                default_shaping_style: shaping_style(&paragraph.character)?,
            });
            resolve_block_runs(
                input,
                block,
                &default_shaping_style,
                &default_paint,
                &mut shaping_runs,
                &mut paint_runs,
            )?;
        }

        Ok(Self {
            style_sheet_revision: sheet.revision,
            document_insets: EdgeInsets {
                top: resolved_document.padding_top,
                left: resolved_document.padding_left,
                bottom: resolved_document.padding_bottom,
                right: resolved_document.padding_right,
            },
            canvas_background: resolved_document.background,
            canvas_background_is_default: resolved_document.background_is_default,
            default_shaping_style,
            shaping_runs,
            default_paint,
            paint_runs,
            paragraphs,
        })
    }
}

fn validate_blocks_in_tree(
    text: &FormattedTextTree,
    blocks: &[Block],
) -> Result<(), DocumentStyleError> {
    for (index, block) in blocks.iter().enumerate() {
        if block.range.start > block.range.end
            || block.range.end > text.byte_len()
            || !text.is_char_boundary(block.range.start)?
            || !text.is_char_boundary(block.range.end)?
        {
            return Err(DocumentStyleError::InvalidBlockRange { index });
        }
    }
    Ok(())
}

fn validate_spans_in_tree(
    text: &FormattedTextTree,
    style_spans: &[StyleSpan],
) -> Result<(), DocumentStyleError> {
    for (index, span) in style_spans.iter().enumerate() {
        if span.range.start > span.range.end
            || (span.range.is_empty()
                && span.application != StyleApplication::SourcePreservedWhitespace)
            || span.range.end > text.byte_len()
            || !text.is_char_boundary(span.range.start)?
            || !text.is_char_boundary(span.range.end)?
        {
            return Err(DocumentStyleError::InvalidStyleSpan { index });
        }
    }
    Ok(())
}

fn validate_blocks(text: &str, blocks: &[Block]) -> Result<(), DocumentStyleError> {
    for (index, block) in blocks.iter().enumerate() {
        if block.range.start > block.range.end
            || block.range.end > text.len()
            || !text.is_char_boundary(block.range.start)
            || !text.is_char_boundary(block.range.end)
        {
            return Err(DocumentStyleError::InvalidBlockRange { index });
        }
    }
    Ok(())
}

fn validate_spans(text: &str, style_spans: &[StyleSpan]) -> Result<(), DocumentStyleError> {
    for (index, span) in style_spans.iter().enumerate() {
        if span.range.start > span.range.end
            || (span.range.is_empty()
                && span.application != StyleApplication::SourcePreservedWhitespace)
            || span.range.end > text.len()
            || !text.is_char_boundary(span.range.start)
            || !text.is_char_boundary(span.range.end)
        {
            return Err(DocumentStyleError::InvalidStyleSpan { index });
        }
    }
    Ok(())
}

fn resolve_block_runs(
    input: StyleCascadeInput<'_>,
    block: &Block,
    default_shaping_style: &ResolvedTextStyle,
    default_paint: &ResolvedTextPaint,
    shaping_runs: &mut Vec<ShapeStyleRun>,
    paint_runs: &mut Vec<PaintStyleRun>,
) -> Result<(), DocumentStyleError> {
    if block.range.is_empty() {
        return Ok(());
    }

    let mut boundaries = BTreeSet::from([block.range.start, block.range.end]);
    for span in input.style_spans {
        let start = span.range.start.max(block.range.start);
        let end = span.range.end.min(block.range.end);
        if start < end {
            boundaries.insert(start);
            boundaries.insert(end);
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().collect();

    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        let character = resolve_character_at(input, block, range.clone())?;
        let shape = shaping_style(&character)?;
        let paint = paint_style(&character);
        if shape != *default_shaping_style {
            push_shape_run(shaping_runs, range.clone(), shape);
        }
        if paint != *default_paint {
            push_paint_run(paint_runs, range, paint);
        }
    }
    Ok(())
}

fn resolve_character_at(
    input: StyleCascadeInput<'_>,
    block: &Block,
    range: Range<usize>,
) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
    let sheet = input.style_sheet;
    let active = input
        .style_spans
        .iter()
        .filter(|span| span.range.start <= range.start && range.end <= span.range.end);

    let mut named: Option<&StyleId> = None;
    let mut semantic = CharacterProperties::default();
    let mut direct = CharacterProperties::default();
    let mut automatic = CharacterProperties::default();
    let mut source_block = block.clone();
    for span in active {
        match &span.application {
            StyleApplication::SourceSyntax
            | StyleApplication::SourceRawText
            | StyleApplication::SourcePreservedWhitespace => {}
            StyleApplication::Automatic(id) => {
                let mut chain = Vec::new();
                let mut current = Some(id);
                while let Some(id) = current {
                    let style = sheet
                        .character_style(id)
                        .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                    chain.push(&style.properties);
                    current = style.based_on.as_ref();
                }
                for properties in chain.into_iter().rev() {
                    merge_character_properties(&mut automatic, properties);
                }
            }
            StyleApplication::SourceParagraph { style, defaults } => {
                source_block.style = style.clone();
                source_block.direct_default_character = defaults.clone();
            }
            StyleApplication::Named(id) => {
                if named.is_some_and(|existing| existing != id) {
                    return Err(DocumentStyleError::MultipleNamedCharacterStyles {
                        range_start: range.start,
                    });
                }
                named = Some(id);
            }
            StyleApplication::Semantic(value) => {
                merge_character_properties(&mut semantic, &semantic_properties(*value));
            }
            StyleApplication::Direct(properties) => {
                merge_character_properties(&mut direct, properties);
            }
        }
    }
    // Semantic markup is a sparse convenience layer. Explicit direct
    // formatting wins property-by-property, independent of source-span order.
    merge_character_properties(&mut semantic, &direct);
    merge_character_properties(&mut semantic, &automatic);

    sheet
        .resolve_assigned_paragraph_style(
            input.document_style,
            &source_block.style,
            &source_block.direct_paragraph,
            &source_block.direct_default_character,
            named,
            &semantic,
        )
        .map(|paragraph| paragraph.character)
        .map_err(Into::into)
}

fn semantic_properties(style: SemanticInlineStyle) -> CharacterProperties {
    match style {
        SemanticInlineStyle::Strong => CharacterProperties {
            bold: Some(true),
            ..CharacterProperties::default()
        },
        SemanticInlineStyle::Emphasis => CharacterProperties {
            slant: Some(FontSlant::Italic),
            ..CharacterProperties::default()
        },
        // Code is assigned a named style by the format adapter. Keeping the
        // semantic marker sparse lets users edit that named font and color.
        SemanticInlineStyle::Code => CharacterProperties::default(),
    }
}

fn merge_character_properties(destination: &mut CharacterProperties, source: &CharacterProperties) {
    macro_rules! replace_some {
        ($field:ident) => {
            if source.$field.is_some() {
                destination.$field.clone_from(&source.$field);
            }
        };
    }
    replace_some!(font_families);
    replace_some!(size);
    replace_some!(weight);
    replace_some!(bold);
    replace_some!(slant);
    replace_some!(foreground);
    replace_some!(background);
    replace_some!(underline);
    replace_some!(strikethrough);
    replace_some!(language);
    replace_some!(direction);
    replace_some!(open_type_features);
    replace_some!(letter_spacing);
    replace_some!(baseline_shift);
}

fn shaping_style(
    character: &ResolvedCharacterStyle,
) -> Result<ResolvedTextStyle, DocumentStyleError> {
    let features = character
        .open_type_features
        .iter()
        .map(|(tag, value)| {
            let bytes = tag.as_bytes();
            if bytes.len() != 4 || !bytes.is_ascii() {
                return Err(DocumentStyleError::InvalidOpenTypeFeatureTag(tag.clone()));
            }
            Ok(OpenTypeFeature {
                tag: [bytes[0], bytes[1], bytes[2], bytes[3]],
                value: *value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ResolvedTextStyle {
        font_families: character.font_families.clone(),
        size: character.size,
        weight: f32::from(character.weight),
        relative_bold: character.bold,
        slant: character.slant,
        letter_spacing: character.letter_spacing,
        baseline_shift: character.baseline_shift,
        language: character.language.clone(),
        script: None,
        direction: match character.direction {
            WritingDirection::Natural => TextDirection::Auto,
            WritingDirection::LeftToRight => TextDirection::LeftToRight,
            WritingDirection::RightToLeft => TextDirection::RightToLeft,
        },
        features,
    })
}

fn paint_style(character: &ResolvedCharacterStyle) -> ResolvedTextPaint {
    ResolvedTextPaint {
        foreground: character.foreground,
        foreground_is_default: character.foreground_is_default,
        background: character.background,
        underline: character.underline,
        strikethrough: character.strikethrough,
    }
}

fn push_shape_run(runs: &mut Vec<ShapeStyleRun>, range: Range<usize>, style: ResolvedTextStyle) {
    if let Some(previous) = runs.last_mut() {
        if previous.text_range.end == range.start && previous.style == style {
            previous.text_range.end = range.end;
            return;
        }
    }
    runs.push(ShapeStyleRun {
        text_range: range,
        style,
    });
}

fn push_paint_run(runs: &mut Vec<PaintStyleRun>, range: Range<usize>, paint: ResolvedTextPaint) {
    if let Some(previous) = runs.last_mut() {
        if previous.text_range.end == range.start && previous.paint == paint {
            previous.text_range.end = range.end;
            return;
        }
    }
    runs.push(PaintStyleRun {
        text_range: range,
        paint,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        BlockProperties, BlockRole, BlockStyle, CharacterStyle, Document, Encoding, Format,
        StyleDefinitionMetadata,
    };

    fn resolve_custom(
        projection: &FormattedDocument,
        sheet: &StyleSheet,
        document_style: &DocumentStyleAssignment,
        blocks: &[Block],
        style_spans: &[StyleSpan],
    ) -> Result<DocumentLayoutStyles, DocumentStyleError> {
        DocumentLayoutStyles::resolve_input(DocumentStyleInput {
            text: projection.text(),
            blocks,
            style_spans,
            style_sheet: sheet,
            document_style,
        })
    }

    #[test]
    fn document_assignment_supplies_canvas_and_inherited_font() {
        let document = Document::new("plain");
        let projection = document.projection();
        let mut sheet = projection.style_sheet().clone();
        let root_style: StyleId = "Writing Canvas".into();
        sheet
            .insert_block_style(
                BlockStyle {
                    id: root_style.clone(),
                    based_on: Some(sheet.base_document.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Document,
                    character: CharacterProperties {
                        font_families: Some(vec!["Theme Sans".to_owned()]),
                        size: Some(11.0),
                        weight: Some(550),
                        ..CharacterProperties::default()
                    },
                    block: BlockProperties {
                        padding_top: Some(4.0),
                        padding_right: Some(8.0),
                        padding_bottom: Some(4.0),
                        padding_left: Some(8.0),
                        background: Some(Color {
                            red: 0.8,
                            green: 0.8,
                            blue: 0.8,
                            alpha: 1.0,
                        }),
                        ..BlockProperties::default()
                    },
                },
                StyleDefinitionMetadata::generated("Writing Canvas"),
            )
            .unwrap();
        let mut root = projection.document_style().clone();
        root.style = root_style;
        root.direct_canvas.padding_top = Some(12.0);
        root.direct_canvas.padding_right = Some(16.0);
        root.direct_canvas.padding_bottom = Some(12.0);
        root.direct_canvas.padding_left = Some(16.0);
        root.direct_canvas.background = Some(Color {
            red: 0.1,
            green: 0.2,
            blue: 0.3,
            alpha: 1.0,
        });
        root.direct_default_character.font_families = Some(vec!["Writer Serif".to_owned()]);
        root.direct_default_character.size = Some(13.0);
        let styles = resolve_custom(
            projection,
            &sheet,
            &root,
            projection.blocks(),
            projection.style_spans(),
        )
        .unwrap();
        assert_eq!(
            styles.document_insets,
            EdgeInsets {
                top: 12.0,
                left: 16.0,
                bottom: 12.0,
                right: 16.0,
            }
        );
        assert_eq!(
            styles.canvas_background,
            root.direct_canvas.background.unwrap()
        );
        assert_eq!(styles.default_shaping_style.font_families, ["Writer Serif"]);
        assert_eq!(styles.default_shaping_style.size, 13.0);
        assert_eq!(styles.default_shaping_style.weight, 550.0);
        assert_eq!(
            styles.paragraphs[0].default_shaping_style.font_families,
            ["Writer Serif"]
        );
        assert_eq!(styles.paragraphs[0].default_shaping_style.weight, 550.0);
    }

    #[test]
    fn paragraph_direct_layout_and_default_character_override_root() {
        let document = Document::new("first\nsecond");
        let projection = document.projection();
        let sheet = projection.style_sheet().clone();
        let mut root = projection.document_style().clone();
        root.direct_default_character.font_families = Some(vec!["Root Sans".to_owned()]);
        root.direct_default_character.size = Some(12.0);
        let mut blocks = projection.blocks().to_vec();
        blocks[0].direct_paragraph = BlockProperties {
            spacing_before: Some(7.0),
            spacing_after: Some(5.0),
            first_line_indent: Some(11.0),
            leading_indent: Some(3.0),
            trailing_indent: Some(4.0),
            ..BlockProperties::default()
        };
        blocks[0].direct_default_character = CharacterProperties {
            font_families: Some(vec!["Paragraph Serif".to_owned()]),
            size: Some(17.0),
            ..CharacterProperties::default()
        };

        let styles =
            resolve_custom(projection, &sheet, &root, &blocks, projection.style_spans()).unwrap();
        let first = &styles.paragraphs[0];
        assert_eq!(first.spacing_before, 7.0);
        assert_eq!(first.spacing_after, 5.0);
        assert_eq!(first.first_line_indent, 11.0);
        assert_eq!(first.leading_indent, 3.0);
        assert_eq!(first.trailing_indent, 4.0);
        assert_eq!(
            first.default_shaping_style.font_families,
            ["Paragraph Serif"]
        );
        assert_eq!(first.default_shaping_style.size, 17.0);
        assert_eq!(
            styles.paragraphs[1].default_shaping_style.font_families,
            ["Root Sans"]
        );
        assert!(styles.shaping_runs.iter().any(|run| {
            run.text_range == blocks[0].range
                && run.style.font_families == ["Paragraph Serif"]
                && run.style.size == 17.0
        }));
    }

    #[test]
    fn named_character_then_direct_span_override_paragraph_defaults() {
        let document = Document::new("styled");
        let projection = document.projection();
        let mut sheet = projection.style_sheet().clone();
        let named_id: StyleId = "Emphatic".into();
        sheet
            .insert_character_style(
                CharacterStyle {
                    id: named_id.clone(),
                    based_on: Some(sheet.base_character.clone()),
                    properties: CharacterProperties {
                        font_families: Some(vec!["Named Serif".to_owned()]),
                        size: Some(20.0),
                        weight: Some(650),
                        ..CharacterProperties::default()
                    },
                },
                StyleDefinitionMetadata::generated("Emphatic"),
            )
            .unwrap();
        let root = projection.document_style().clone();
        let mut blocks = projection.blocks().to_vec();
        blocks[0].direct_default_character = CharacterProperties {
            font_families: Some(vec!["Paragraph Sans".to_owned()]),
            size: Some(16.0),
            weight: Some(500),
            ..CharacterProperties::default()
        };
        let spans = vec![
            StyleSpan {
                range: 0..6,
                application: StyleApplication::Named(named_id),
            },
            StyleSpan {
                range: 0..6,
                application: StyleApplication::Direct(CharacterProperties {
                    size: Some(24.0),
                    slant: Some(FontSlant::Italic),
                    ..CharacterProperties::default()
                }),
            },
        ];

        let styles = resolve_custom(projection, &sheet, &root, &blocks, &spans).unwrap();
        let run = styles
            .shaping_runs
            .iter()
            .find(|run| run.text_range == (0..6))
            .unwrap();
        assert_eq!(run.style.font_families, ["Named Serif"]);
        assert_eq!(run.style.weight, 650.0);
        assert_eq!(run.style.size, 24.0);
        assert_eq!(run.style.slant, FontSlant::Italic);
    }

    #[test]
    fn invalid_root_and_paragraph_assignments_are_typed_errors() {
        let document = Document::new("plain");
        let projection = document.projection();
        let sheet = projection.style_sheet().clone();
        let blocks = projection.blocks().to_vec();

        let mut root = projection.document_style().clone();
        root.style = "Missing Document".into();
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(StyleError::UnknownStyle(_)))
        ));

        root = projection.document_style().clone();
        root.style = sheet.base_paragraph.clone();
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::IncompatibleBlockRole { .. }
            ))
        ));

        root = projection.document_style().clone();
        root.direct_canvas.spacing_before = Some(2.0);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InapplicableBlockProperties { .. }
            ))
        ));

        root = projection.document_style().clone();
        root.direct_canvas.padding_left = Some(f32::NAN);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InvalidBlockProperties(_)
            ))
        ));

        root = projection.document_style().clone();
        root.direct_default_character.size = Some(0.0);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InvalidCharacterProperties(_)
            ))
        ));

        let root = projection.document_style().clone();
        let mut invalid_blocks = blocks.clone();
        invalid_blocks[0].style = "Missing Paragraph".into();
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]),
            Err(DocumentStyleError::Cascade(StyleError::UnknownStyle(_)))
        ));

        invalid_blocks = blocks.clone();
        invalid_blocks[0].style = sheet.base_document.clone();
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::IncompatibleBlockRole { .. }
            ))
        ));

        invalid_blocks = blocks.clone();
        invalid_blocks[0].direct_paragraph.padding_left = Some(2.0);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InapplicableBlockProperties { .. }
            ))
        ));

        invalid_blocks = blocks;
        invalid_blocks[0].direct_paragraph.leading_indent = Some(f32::INFINITY);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InvalidBlockProperties(_)
            ))
        ));

        let mut invalid_blocks = projection.blocks().to_vec();
        invalid_blocks[0].direct_default_character.size = Some(-1.0);
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::InvalidCharacterProperties(_)
            ))
        ));

        let unknown_named = [StyleSpan {
            range: 0..5,
            application: StyleApplication::Named("Missing Character".into()),
        }];
        assert!(matches!(
            resolve_custom(
                projection,
                &sheet,
                &root,
                projection.blocks(),
                &unknown_named,
            ),
            Err(DocumentStyleError::Cascade(StyleError::UnknownStyle(_)))
        ));
    }

    #[test]
    fn markdown_blocks_and_semantic_spans_resolve_to_shaping_runs() {
        let document = Document::from_bytes(
            b"# Heading\n**bold** *italic* `code`".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        let heading = styles
            .shaping_runs
            .iter()
            .find(|run| run.text_range == (0..7))
            .unwrap();
        assert_eq!(heading.style.size, 24.0);
        assert_eq!(heading.style.weight, 700.0);

        let text = document.text();
        let bold_start = text.find("bold").unwrap();
        let italic_start = text.find("italic").unwrap();
        let code_start = text.find("code").unwrap();
        let bold = styles
            .shaping_runs
            .iter()
            .find(|run| run.text_range == (bold_start..bold_start + 4))
            .unwrap();
        let italic = styles
            .shaping_runs
            .iter()
            .find(|run| run.text_range == (italic_start..italic_start + 6))
            .unwrap();
        let code = styles
            .shaping_runs
            .iter()
            .find(|run| run.text_range == (code_start..code_start + 4))
            .unwrap();
        assert_eq!(bold.style.weight, 700.0);
        assert_eq!(italic.style.slant, FontSlant::Italic);
        assert_eq!(code.style.font_families, ["monospace"]);
    }

    #[test]
    fn indexed_region_resolution_matches_the_flat_filter_oracle() {
        let document = Document::from_bytes(
            b"# Heading\nplain **bold** text\nlast *soft* line".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let projection = document.projection();
        let start = projection.text().find("bold").unwrap();
        let end = projection.text().find("last").unwrap() + "last".len();
        let range = start..end;

        let blocks = projection.blocks();
        let first = blocks.partition_point(|block| block.range.end < range.start);
        let block_end = blocks.partition_point(|block| block.range.start <= range.end);
        let oracle_blocks = blocks[first..block_end].to_vec();
        let oracle_spans = projection
            .style_spans()
            .iter()
            .filter(|span| span.range.start < range.end && range.start < span.range.end)
            .cloned()
            .collect::<Vec<_>>();
        let oracle = DocumentLayoutStyles::resolve_input(DocumentStyleInput {
            text: projection.text(),
            blocks: &oracle_blocks,
            style_spans: &oracle_spans,
            style_sheet: projection.style_sheet(),
            document_style: projection.document_style(),
        })
        .unwrap();

        assert_eq!(
            DocumentLayoutStyles::resolve_region(projection, range).unwrap(),
            oracle
        );
    }

    #[test]
    fn regional_resolution_does_not_materialize_the_complete_text_tree() {
        let mut document = Document::new("line\n".repeat(20_000));
        document
            .insert(2, "X")
            .expect("local edit creates a persistent regional projection");
        assert!(
            !document.projection().compatibility_text_is_materialized(),
            "the regional projector deliberately leaves the flat compatibility cache empty"
        );

        let first_line = document
            .projection()
            .hard_line_range(0)
            .expect("the first hard line is indexed");
        let styles = DocumentLayoutStyles::resolve_region(document.projection(), first_line)
            .expect("style resolution can validate ranges against the rope");

        assert_eq!(styles.paragraphs.len(), 1);
        assert!(
            !document.projection().compatibility_text_is_materialized(),
            "regional style resolution must not flatten unrelated text"
        );
    }

    #[test]
    fn empty_source_whitespace_context_is_metadata_with_validated_boundaries() {
        let text = "é";
        let tree = FormattedTextTree::try_from_text(text).unwrap();
        for at in [0, text.len()] {
            let spans = [StyleSpan {
                range: at..at,
                application: StyleApplication::SourcePreservedWhitespace,
            }];
            assert!(validate_spans(text, &spans).is_ok());
            assert!(validate_spans_in_tree(&tree, &spans).is_ok());
        }
        for span in [
            StyleSpan {
                range: 0..0,
                application: StyleApplication::Direct(CharacterProperties::default()),
            },
            StyleSpan {
                range: 0..0,
                application: StyleApplication::Named("Code".into()),
            },
            StyleSpan {
                range: 1..1,
                application: StyleApplication::SourcePreservedWhitespace,
            },
            StyleSpan {
                range: 3..3,
                application: StyleApplication::SourcePreservedWhitespace,
            },
            StyleSpan {
                range: 2..0,
                application: StyleApplication::SourcePreservedWhitespace,
            },
        ] {
            assert!(validate_spans(text, std::slice::from_ref(&span)).is_err());
            assert!(validate_spans_in_tree(&tree, &[span]).is_err());
        }
        let document = Document::new(text);
        let projection = document.projection();
        let base = DocumentLayoutStyles::resolve(projection).unwrap();
        let with_context = resolve_custom(
            projection,
            projection.style_sheet(),
            projection.document_style(),
            projection.blocks(),
            &[StyleSpan {
                range: 0..0,
                application: StyleApplication::SourcePreservedWhitespace,
            }],
        )
        .unwrap();
        assert_eq!(
            with_context, base,
            "point context has no layout or paint effect"
        );
    }

    #[test]
    fn color_changes_are_absent_from_the_shaping_value() {
        let base = ResolvedCharacterStyle::default();
        let mut recolored = base.clone();
        recolored.foreground = Color {
            red: 1.0,
            green: 0.2,
            blue: 0.1,
            alpha: 1.0,
        };
        assert_eq!(
            shaping_style(&base).unwrap(),
            shaping_style(&recolored).unwrap()
        );
        assert_ne!(paint_style(&base), paint_style(&recolored));
    }
}
