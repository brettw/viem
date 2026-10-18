//! Adapter from the normalized document style model to layout and paint runs.
//!
//! Source adapters own source-language cascade rules. This module only
//! resolves the already-normalized block/character cascade and separates
//! shape-affecting properties from paint-only properties.

use super::{EdgeInsets, OpenTypeFeature, ResolvedTextStyle, ShapeStyleRun, TextDirection};
use super::search_overlay::SearchPaintOverlay;
use crate::document::{
    Block, CharacterProperties, Color, DocumentStyleAssignment, FontSlant, FormattedDocument,
    FormattedTextError, FormattedTextTree, LineSpacing, ParagraphAlignment, ResolvedCharacterStyle,
    SemanticInlineStyle, StyleApplication, StyleError, StyleId, StyleSheet, StyleSheetRevision,
    StyleSpan, WritingDirection, InlineImage,
};
use std::collections::BTreeSet;
use std::ops::Range;

mod recovery;

fn lower_alpha(mut ordinal: u64) -> Option<String> {
    if ordinal == 0 {
        return None;
    }
    let mut reversed = Vec::new();
    while ordinal > 0 {
        ordinal -= 1;
        reversed.push((b'a' + (ordinal % 26) as u8) as char);
        ordinal /= 26;
    }
    reversed.reverse();
    Some(reversed.into_iter().collect())
}

fn lower_roman(ordinal: u64) -> Option<String> {
    if !(1..=3_999).contains(&ordinal) {
        return None;
    }
    const DIGITS: &[(u64, &str)] = &[
        (1_000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut remaining = ordinal;
    let mut result = String::new();
    for &(value, spelling) in DIGITS {
        while remaining >= value {
            result.push_str(spelling);
            remaining -= value;
        }
    }
    Some(result)
}

/// Generated list furniture follows the four shipped structural style levels.
/// Deeper authored lists reuse the fourth style rather than cycling again.
fn list_marker_decoration(ordered: bool, ordinal: u64, level: u8) -> String {
    let marker = ParagraphLayoutStyle::list_marker_type(ordered, level);
    if !ordered {
        return match marker {
            "disc" => "•",
            "circle" => "◦",
            "square" => "▪",
            _ => unreachable!(),
        }
        .into();
    }
    let number = match marker {
        "lower-alpha" => lower_alpha(ordinal),
        "lower-roman" => lower_roman(ordinal),
        "decimal" => None,
        _ => unreachable!(),
    }
    .unwrap_or_else(|| ordinal.to_string());
    format!("{number}.")
}

/// Paint-only character values deliberately kept out of shaping cache keys.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTextPaint {
    pub foreground: Color,
    pub foreground_is_default: bool,
    pub background: Option<Color>,
    pub underline: bool,
    pub strikethrough: bool,
    /// Unscaled baseline displacement; positive values raise the text.
    pub baseline_offset: f32,
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
    pub quote_depth: usize,
    pub thematic_break: bool,
    pub marker_paint: ResolvedTextPaint,
    pub block_box: super::BlockBoxStyle,
    pub containers: std::sync::Arc<[super::ContainerLayoutStyle]>,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub line_spacing: LineSpacing,
    pub first_line_indent: f32,
    pub leading_indent: f32,
    pub trailing_indent: f32,
    pub alignment: ParagraphAlignment,
    pub base_direction: WritingDirection,
    pub default_shaping_style: ResolvedTextStyle,
}

impl ParagraphLayoutStyle {
    /// Shared native furniture / HTML export marker vocabulary. Deeper lists
    /// retain the fourth shipped style rather than cycling.
    pub(crate) fn list_marker_type(ordered: bool, level: u8) -> &'static str {
        match (ordered, level.min(3)) {
            (true, 1) => "lower-alpha", (true, 2) => "lower-roman", (true, _) => "decimal",
            (false, 1) => "circle", (false, 2) => "square", (false, _) => "disc",
        }
    }
}

/// Layout-facing result of resolving one immutable formatted snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentLayoutStyles {
    pub(crate) table_context: Option<super::engine::tables::TableLayoutContext>,
    pub(crate) recovery_diagnostic: Option<super::ShapingDiagnostic>,
    /// Default Paragraph font, excluding authored character spans and the
    /// current paragraph's named style. Whitespace geometry uses this basis.
    pub whitespace_shaping_style: ResolvedTextStyle,
    /// Ordinary paragraph spacing, independent of the paragraphs currently
    /// materialized. Screen paging uses it with the default font's metrics.
    pub default_line_spacing: LineSpacing,
    pub style_sheet_revision: StyleSheetRevision,
    pub document_insets: EdgeInsets,
    pub canvas_background: Color,
    pub canvas_background_is_default: bool,
    pub default_shaping_style: ResolvedTextStyle,
    pub shaping_runs: Vec<ShapeStyleRun>,
    pub inline_images: Vec<super::ShapeInlineImage>,
    pub default_paint: ResolvedTextPaint,
    pub paint_runs: Vec<PaintStyleRun>,
    pub paragraphs: Vec<ParagraphLayoutStyle>,
    /// Sparse presentation declarations retained until cluster geometry is
    /// available. They must not absorb surrounding authored paint properties.
    pub(crate) search_paint_overlay: Option<SearchPaintOverlay>,
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
    inline_images: &'a [InlineImage],
    style_sheet: &'a StyleSheet,
    document_style: &'a DocumentStyleAssignment,
    search_matches: &'a [Range<usize>],
}

impl<'a> From<DocumentStyleInput<'a>> for StyleCascadeInput<'a> {
    fn from(input: DocumentStyleInput<'a>) -> Self {
        Self {
            blocks: input.blocks,
            style_spans: input.style_spans,
            inline_images: &[],
            style_sheet: input.style_sheet,
            document_style: input.document_style,
            search_matches: &[],
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
        let images = if automatic { document.inline_images_for_region(&range) } else { Vec::new() };
        let mut block = block.clone();
        if !automatic && block.style.0 == "Image" {
            block.style = document.style_sheet().base_paragraph.clone();
        }
        let mut spans = if automatic { document.layout_style_spans_for_region(&range) } else { document.style_spans_for_region(&range) };
        if !automatic {
            spans.retain(|span| {
                !matches!(
                    &span.application,
                    StyleApplication::Automatic(id) if id.0 != "Strikethrough"
                )
            });
        }
        resolve_character_at(
            StyleCascadeInput {
                blocks: &blocks,
                style_spans: &spans,
                inline_images: &images,
                style_sheet: if automatic { document.layout_style_sheet() } else { document.style_sheet() },
                document_style: document.document_style(),
                search_matches: &[],
            },
            &block,
            range,
        )
    }

    /// Resolve document, paragraph, named-character, semantic, and direct
    /// layers without consulting a platform or mutating the projection.
    pub fn resolve(document: &FormattedDocument) -> Result<Self, DocumentStyleError> {
        Self::resolve_region(document, 0..document.text_tree().byte_len())
    }

    /// Resolve only blocks and character spans that can affect a contiguous
    /// formatted-text region. Document-root values are still resolved from the
    /// immutable style sheet, while paragraph/run work is proportional to the
    /// materialized region rather than the complete document.
    pub fn resolve_region(
        document: &FormattedDocument,
        text_range: Range<usize>,
    ) -> Result<Self, DocumentStyleError> {
        Self::resolve_region_with_search(document, text_range, &[])
    }

    pub(crate) fn resolve_region_with_search(
        document: &FormattedDocument,
        text_range: Range<usize>,
        search_matches: &[Range<usize>],
    ) -> Result<Self, DocumentStyleError> {
        Self::resolve_region_impl(document, text_range, search_matches, None, None)
    }

    pub(crate) fn resolve_region_for_presentation(
        document: &FormattedDocument,
        text_range: Range<usize>,
        search_matches: &[Range<usize>],
        format: crate::document::Format,
    ) -> Result<Self, DocumentStyleError> {
        Self::resolve_region_impl(document, text_range, search_matches, Some(format), None)
    }

    /// As [`Self::resolve_region_for_presentation`], reusing the appearance of
    /// paragraphs whose inputs are unchanged since the view's previous
    /// resolution.
    pub(crate) fn resolve_region_for_presentation_cached(
        document: &FormattedDocument,
        text_range: Range<usize>,
        search_matches: &[Range<usize>],
        format: crate::document::Format,
        cache: &mut ParagraphStyleCache,
    ) -> Result<Self, DocumentStyleError> {
        let styles = Self::resolve_region_impl(document, text_range.clone(), search_matches, Some(format), Some(cache))?;
        // Debug builds verify every reuse against a complete resolution.
        #[cfg(debug_assertions)]
        {
            let mut fresh = Self::resolve_region_impl(document, text_range, search_matches, Some(format), None)?;
            // Each resolution captures its own table context for one projection.
            assert_eq!(styles.table_context.is_some(), fresh.table_context.is_some());
            fresh.table_context = styles.table_context.clone();
            assert!(styles == fresh, "reused paragraph styles differ from a complete resolution");
        }
        Ok(styles)
    }

    fn resolve_region_impl(
        document: &FormattedDocument,
        text_range: Range<usize>,
        search_matches: &[Range<usize>],
        recovery_format: Option<crate::document::Format>,
        mut cache: Option<&mut ParagraphStyleCache>,
    ) -> Result<Self, DocumentStyleError> {
        let text = document.text_tree();
        if text_range.start > text_range.end
            || text_range.end > text.byte_len()
            || !text.is_char_boundary(text_range.start)?
            || !text.is_char_boundary(text_range.end)?
        {
            return Err(DocumentStyleError::InvalidBlockRange { index: 0 });
        }
        let regional_blocks = document.blocks_for_region(&text_range);
        let regional_spans = document.layout_style_spans_for_region(&text_range);
        let regional_images = document.inline_images_for_region(&text_range);
        validate_blocks_in_tree(text, &regional_blocks)?;
        validate_spans_in_tree(text, &regional_spans)?;
        let input = StyleCascadeInput {
            blocks: &regional_blocks,
            style_spans: &regional_spans,
            inline_images: &regional_images,
            style_sheet: document.layout_style_sheet(),
            document_style: document.document_style(),
            search_matches,
        };
        if let Some(cache) = cache.as_deref_mut() {
            cache.prepare(document.layout_style_sheet_arc(), document.document_style());
        }
        let mut styles = match recovery_format {
            Some(format) => Self::resolve_for_presentation(input, format, cache),
            None => Self::resolve_validated(input, cache),
        }?;
        for (paragraph, block) in styles.paragraphs.iter_mut().zip(&regional_blocks) {
            paragraph.list_marker_range = document
                .list_marker_range_for_block(block)
                .filter(|range| !range.is_empty());
        }
        if !document.tables().is_empty() {
            styles.table_context = Some(super::engine::tables::TableLayoutContext::new(document));
        }
        styles.inline_images = regional_images.into_iter()
            .map(|image| super::ShapeInlineImage {
                text_range: image.range, destination: image.destination,
                width: image.width, height: image.height,
            })
            .collect();
        Ok(styles)
    }

    pub fn resolve_input(input: DocumentStyleInput<'_>) -> Result<Self, DocumentStyleError> {
        validate_blocks(input.text, input.blocks)?;
        validate_spans(input.text, input.style_spans)?;
        Self::resolve_validated(input.into(), None)
    }

    fn resolve_validated(input: StyleCascadeInput<'_>, cache: Option<&mut ParagraphStyleCache>)
        -> Result<Self, DocumentStyleError> {
        let sheet = input.style_sheet;
        let resolved_document = sheet.resolve_document_assignment(input.document_style)?;
        let default_shaping_style = shaping_style(&resolved_document.character)?;
        let whitespace_paragraph = sheet.resolve_assigned_paragraph_style(
            input.document_style, &StyleId("Paragraph".into()),
            &crate::document::BlockProperties::default(), &CharacterProperties::default(),
            None, &CharacterProperties::default(),
        )?;
        let whitespace_shaping_style = shaping_style(&whitespace_paragraph.character)?;
        let default_paint = paint_style(&resolved_document.character);
        let search_paint_overlay = if input.search_matches.is_empty() {
            None
        } else {
            let start = input.blocks.first().map_or(0, |block| block.range.start);
            let end = input.blocks.last().map_or(start, |block| block.range.end);
            SearchPaintOverlay::new(
                input.search_matches,
                start..end,
                &sheet.incremental_match_properties()?,
            )
        };

        let mut shaping_runs = Vec::new();
        let mut paint_runs = Vec::new();
        let mut paragraphs = Vec::with_capacity(input.blocks.len());
        let mut cascades = CharacterCascadeMemo::new(resolved_document.character.size);
        let mut retained = cache.as_ref().map(|_| std::collections::HashMap::with_capacity(input.blocks.len()));
        for block in input.blocks {
            if let Some(cached) = cache.as_ref().and_then(|cache| cache.paragraphs.get(&block.id))
                .filter(|cached| cached.matches(&input, block))
            {
                let start = block.range.start;
                let mut paragraph = cached.paragraph.clone();
                paragraph.text_range = start..start + paragraph.text_range.end;
                paragraph.list_marker_range = paragraph.list_marker_range.map(|range| start + range.start..start + range.end);
                paragraphs.push(paragraph);
                for run in &cached.shaping_runs {
                    push_shape_run(&mut shaping_runs, start + run.text_range.start..start + run.text_range.end, run.style.clone());
                }
                for run in &cached.paint_runs {
                    push_paint_run(&mut paint_runs, start + run.text_range.start..start + run.text_range.end, run.paint.clone());
                }
                if let Some(retained) = retained.as_mut() { retained.insert(block.id, std::sync::Arc::clone(cached)); }
                continue;
            }
            let mut container_character = CharacterProperties::default();
            let mut container_font_size = resolved_document.character.size;
            let mut container_paragraph = crate::document::BlockProperties::default();
            let mut containers = Vec::with_capacity(block.containers.len());
            for membership in block.containers.iter() {
                let container = &membership.container;
                let declarations = cascades.container(sheet, input.document_style, container)?;
                let mut resolved = declarations.resolved.clone();
                merge_container_character_properties(&mut container_character, &mut container_font_size,
                    declarations.character.clone());
                if let Some(color) = container_character.foreground {
                    resolved.character.foreground = color;
                    resolved.character.foreground_is_default = false;
                }
                let defaults = &declarations.paragraph;
                if defaults.line_spacing.is_some() { container_paragraph.line_spacing = defaults.line_spacing; }
                if defaults.alignment.is_some() { container_paragraph.alignment = defaults.alignment; }
                if defaults.base_direction.is_some() { container_paragraph.base_direction = defaults.base_direction; }
                if defaults.first_line_indent.is_some() { container_paragraph.first_line_indent = defaults.first_line_indent; }
                let mut container_box = super::BlockBoxStyle::from_resolved(&resolved);
                if block.code_language_label && container.kind == crate::document::ContainerKind::CodeBlock {
                    container_box.language_label = Some(if block.front_matter { "front matter".into() }
                        else { block.code_language.as_deref().map(crate::document::syntax::languages::display_name).unwrap_or_else(|| "None".into()) });
                    container_box.language_picker_disabled = block.front_matter;
                }
                container_box.inline_start = resolved.leading_indent;
                container_box.inline_end = resolved.trailing_indent;
                containers.push(super::ContainerLayoutStyle { id: container.id, kind: container.kind,
                    style: container_box,
                    starts_here: membership.starts_here, ends_here: membership.ends_here });
            }
            let mut assigned = Some(&block.style);
            let mut quote_border = block.quote_depth > 0;
            while let Some(id) = assigned {
                if id.0 == "Block quote" {
                    quote_border = true;
                    break;
                }
                assigned = sheet
                    .block_style(id)
                    .and_then(|style| style.based_on.as_ref());
            }
            quote_border &= block.containers.is_empty();
            let container_body = block.containers.iter().any(|membership| membership.container.style == block.style)
                && sheet.block_style(&block.style).is_some_and(|style| style.role.is_container());
            let paragraph = cascades.paragraph(
                sheet,
                input.document_style,
                if container_body {
                    &sheet.base_paragraph
                } else {
                    &block.style
                },
                &block.direct_paragraph,
                &container_character,
                &block.direct_default_character,
                Some(&container_paragraph),
            )?;
            // A heading or code paragraph inside an item keeps its own style
            // plus the containing list's inset. List-role paragraphs already
            // declare that inset themselves.
            let list_inset = if block.containers.iter().any(|membership| matches!(membership.container.kind, crate::document::ContainerKind::List { .. })) { 0.0 } else if let crate::document::BlockKind::ListItem { ordered, level, .. } =
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
                    cascades
                        .paragraph(
                            sheet,
                            input.document_style,
                            &id,
                            &Default::default(),
                            &Default::default(),
                            &Default::default(),
                            None,
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
                quote_depth: block.quote_depth.max(usize::from(quote_border)),
                thematic_break: block.thematic_break,
                list_marker_decoration: match block.kind {
                    crate::document::BlockKind::ListItem {
                        ordered,
                        ordinal,
                        level,
                        item_start: true,
                        marker_is_decoration: true,
                        ..
                    } => Some(list_marker_decoration(ordered, ordinal, level)),
                    _ => None,
                },
                marker_paint: paint_style(&paragraph.character),
                block_box: {
                    let mut value = if container_body { super::BlockBoxStyle::default() } else { super::BlockBoxStyle::from_resolved(&paragraph) };
                    if block.code_language_label && !block.containers.iter().any(|member| member.container.kind == crate::document::ContainerKind::CodeBlock) {
                        value.language_label = Some(if block.front_matter { "front matter".into() }
                            else { block.code_language.as_deref().map(crate::document::syntax::languages::display_name).unwrap_or_else(|| "None".into()) });
                        value.language_picker_disabled = block.front_matter;
                    }
                    value
                },
                containers: containers.into(),
                margin_top: if container_body { 0.0 } else if block.list_loose { paragraph.margin_top.max(7.0) } else { paragraph.margin_top },
                margin_bottom: if container_body { 0.0 } else if block.list_loose { paragraph.margin_bottom.max(7.0) } else { paragraph.margin_bottom },
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
            let mut block_shaping = Vec::new();
            let mut block_paint = Vec::new();
            resolve_block_runs(
                input,
                block,
                &default_shaping_style,
                &default_paint,
                &mut block_shaping,
                &mut block_paint,
                &mut cascades,
            )?;
            if let Some(retained) = retained.as_mut() {
                retained.insert(block.id, std::sync::Arc::new(CachedParagraphStyle::new(&input, block,
                    paragraphs.last().expect("pushed above"), &block_shaping, &block_paint)));
            }
            // Appending through the same merge reproduces runs pushed directly.
            for run in block_shaping { push_shape_run(&mut shaping_runs, run.text_range, run.style); }
            for run in block_paint { push_paint_run(&mut paint_runs, run.text_range, run.paint); }
        }
        if let (Some(cache), Some(retained)) = (cache, retained) {
            cache.paragraphs = retained;
        }

        Ok(Self {
            table_context: None,
            recovery_diagnostic: None,
            whitespace_shaping_style,
            default_line_spacing: whitespace_paragraph.line_spacing,
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
            inline_images: Vec::new(),
            default_paint,
            paint_runs,
            paragraphs,
            search_paint_overlay,
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
        if span.range.start >= span.range.end
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
        if span.range.start >= span.range.end
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
    memo: &mut CharacterCascadeMemo,
) -> Result<(), DocumentStyleError> {
    if block.range.is_empty() {
        return Ok(());
    }
    // Resolved on the block's first styled segment, as each segment would.
    let mut context = None;

    let mut boundaries = BTreeSet::from([block.range.start, block.range.end]);
    let first_image = input.inline_images.partition_point(|image| image.range.end <= block.range.start);
    for image in input.inline_images[first_image..].iter().take_while(|image| image.range.start < block.range.end) {
        boundaries.insert(image.range.start.max(block.range.start));
        boundaries.insert(image.range.end.min(block.range.end));
    }
    let mut events = Vec::new();
    for (index, span) in input.style_spans.iter().enumerate() {
        let start = span.range.start.max(block.range.start);
        let end = span.range.end.min(block.range.end);
        if start < end {
            boundaries.insert(start);
            boundaries.insert(end);
            events.push((start, true, index));
            events.push((end, false, index));
        }
    }
    events.sort_unstable();
    // Search presentation supplies sorted, disjoint coverage. Restrict the
    // sweep to this block so dense matches do not make each style interval
    // rescan the entire viewport's match list.
    let first_match = input
        .search_matches
        .partition_point(|matched| matched.end <= block.range.start);
    let last_match = input
        .search_matches
        .partition_point(|matched| matched.start < block.range.end);
    let search_matches = &input.search_matches[first_match..last_match];
    for matched in search_matches {
        let start = matched.start.max(block.range.start);
        let end = matched.end.min(block.range.end);
        if start < end {
            boundaries.insert(start);
            boundaries.insert(end);
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().collect();

    let mut match_index = 0;
    let mut event_index = 0;
    let mut active = BTreeSet::new();
    // Cascade state after each active span, in span order, and the lowest
    // span index changed since that state was last brought up to date.
    let mut cascades: Vec<(usize, SpanCascade<'_>)> = Vec::new();
    let mut changed_from = 0;
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        while event_index < events.len() && events[event_index].0 <= range.start {
            let (_, opening, index) = events[event_index];
            if opening { active.insert(index); } else { active.remove(&index); }
            changed_from = changed_from.min(index);
            event_index += 1;
        }
        // Original span order is the cascade order. A sweep avoids rescanning
        // every unrelated decoration for each segment of a long source line,
        // and resumes the fold below the first span that changed.
        let mut character = if let Some(image) = inline_image_character(input, &range)? {
            image
        } else {
            cascades.truncate(cascades.partition_point(|(index, _)| *index < changed_from));
            let mut cascade = cascades.last().map(|(_, cascade)| cascade.clone()).unwrap_or_default();
            for &index in active.range(changed_from..) {
                cascade.apply(input.style_sheet, &input.style_spans[index], &range)?;
                cascades.push((index, cascade.clone()));
            }
            changed_from = usize::MAX;
            let context = match &mut context {
                Some(context) => context,
                None => context.insert(BlockCharacterContext::new(input, block, memo)?),
            };
            cascade.resolve(input, block, context, memo)?
        };
        while match_index < search_matches.len() && search_matches[match_index].end <= range.start {
            match_index += 1;
        }
        if search_matches
            .get(match_index)
            .is_some_and(|matched| matched.start <= range.start && range.end <= matched.end)
        {
            character = input.style_sheet.overlay_incremental_match(&character)?;
        }
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
    let active = input.style_spans.iter()
        .filter(|span| span.range.start <= range.start && range.end <= span.range.end);
    resolve_character_spans_at(input, block, range.clone(), active)
}

fn resolve_character_spans_at<'a>(
    input: StyleCascadeInput<'a>,
    block: &Block,
    range: Range<usize>,
    active: impl Iterator<Item = &'a StyleSpan>,
) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
    if let Some(image) = inline_image_character(input, &range)? {
        return Ok(image);
    }
    let mut cascade = SpanCascade::default();
    for span in active {
        cascade.apply(input.style_sheet, span, &range)?;
    }
    let document_font_size = input.style_sheet.resolve_document_assignment(input.document_style)?.character.size;
    let mut memo = CharacterCascadeMemo::new(document_font_size);
    let context = BlockCharacterContext::new(input, block, &mut memo)?;
    cascade.resolve(input, block, &context, &mut memo)
}

/// The appearance of each paragraph of a view's previous resolution. A
/// paragraph's style and runs are reused while all of their inputs are
/// unchanged: its shared block attributes and length, and the spans, inline
/// objects and search matches within it relative to its start, under the same
/// layout style sheet and document style. Only the latest resolution's
/// paragraphs are retained, so the cache is bounded by one region.
#[derive(Clone, Default)]
pub(crate) struct ParagraphStyleCache {
    identity: Option<(std::sync::Arc<StyleSheet>, DocumentStyleAssignment)>,
    paragraphs: std::collections::HashMap<u64, std::sync::Arc<CachedParagraphStyle>>,
}

impl ParagraphStyleCache {
    fn prepare(&mut self, sheet: &std::sync::Arc<StyleSheet>, document_style: &DocumentStyleAssignment) {
        let current = self.identity.as_ref()
            .is_some_and(|(cached, style)| std::sync::Arc::ptr_eq(cached, sheet) && style == document_style);
        if !current {
            self.identity = Some((std::sync::Arc::clone(sheet), document_style.clone()));
            self.paragraphs.clear();
        }
    }
}

/// A disposable cache never distinguishes otherwise equal views.
impl PartialEq for ParagraphStyleCache {
    fn eq(&self, _: &Self) -> bool { true }
}

impl std::fmt::Debug for ParagraphStyleCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ParagraphStyleCache").field("paragraphs", &self.paragraphs.len()).finish()
    }
}

/// One paragraph's resolved appearance in coordinates relative to its start,
/// with the inputs that produced it.
struct CachedParagraphStyle {
    block: Block,
    spans: Vec<(Range<usize>, StyleApplication)>,
    images: Vec<(isize, isize)>,
    search: Vec<Range<usize>>,
    paragraph: ParagraphLayoutStyle,
    shaping_runs: Vec<ShapeStyleRun>,
    paint_runs: Vec<PaintStyleRun>,
}

/// The spans, inline objects and search matches intersecting `block`,
/// relative to its start, in the order the cascade consumes them.
fn paragraph_inputs<'a>(input: &'a StyleCascadeInput<'_>, block: &'a Block) -> (
    impl Iterator<Item = (Range<usize>, &'a StyleApplication)> + 'a,
    impl Iterator<Item = (isize, isize)> + 'a,
    impl Iterator<Item = Range<usize>> + 'a,
) {
    let range = block.range.clone();
    let start = range.start;
    let spans = input.style_spans.iter().filter_map(move |span| {
        let clipped = span.range.start.max(range.start)..span.range.end.min(range.end);
        (clipped.start < clipped.end).then(|| (clipped.start - start..clipped.end - start, &span.application))
    });
    let first_image = input.inline_images.partition_point(|image| image.range.end <= block.range.start);
    let images = input.inline_images[first_image..].iter().take_while(move |image| image.range.start < block.range.end)
        .map(move |image| (image.range.start as isize - start as isize, image.range.end as isize - start as isize));
    let first_match = input.search_matches.partition_point(|matched| matched.end <= block.range.start);
    let search = input.search_matches[first_match..].iter().take_while(move |matched| matched.start < block.range.end)
        .map(move |matched| matched.start.max(block.range.start) - start..matched.end.min(block.range.end) - start);
    (spans, images, search)
}

impl CachedParagraphStyle {
    fn new(input: &StyleCascadeInput<'_>, block: &Block, paragraph: &ParagraphLayoutStyle,
        shaping_runs: &[ShapeStyleRun], paint_runs: &[PaintStyleRun]) -> Self {
        let start = block.range.start;
        let (spans, images, search) = paragraph_inputs(input, block);
        let mut paragraph = paragraph.clone();
        paragraph.text_range = 0..block.range.len();
        paragraph.list_marker_range = paragraph.list_marker_range.map(|range| range.start - start..range.end - start);
        Self {
            block: block.clone(),
            spans: spans.map(|(range, application)| (range, application.clone())).collect(),
            images: images.collect(),
            search: search.collect(),
            paragraph,
            shaping_runs: shaping_runs.iter().map(|run| ShapeStyleRun {
                text_range: run.text_range.start - start..run.text_range.end - start, style: run.style.clone() }).collect(),
            paint_runs: paint_runs.iter().map(|run| PaintStyleRun {
                text_range: run.text_range.start - start..run.text_range.end - start, paint: run.paint.clone() }).collect(),
        }
    }

    fn matches(&self, input: &StyleCascadeInput<'_>, block: &Block) -> bool {
        if !std::sync::Arc::ptr_eq(&self.block.attributes, &block.attributes)
            || self.block.range.len() != block.range.len()
        {
            return false;
        }
        let (spans, images, search) = paragraph_inputs(input, block);
        spans.eq(self.spans.iter().map(|(range, application)| (range.clone(), application)))
            && images.eq(self.images.iter().copied())
            && search.eq(self.search.iter().cloned())
    }
}

/// Block-level inputs of every character cascade in one paragraph, resolved
/// once rather than for each styled segment.
struct BlockCharacterContext<'a> {
    paragraph_style: &'a StyleId,
    container_character: CharacterProperties,
}

impl<'a> BlockCharacterContext<'a> {
    fn new(input: StyleCascadeInput<'a>, block: &'a Block, memo: &mut CharacterCascadeMemo)
        -> Result<Self, DocumentStyleError> {
        let sheet = input.style_sheet;
        let mut container_character = CharacterProperties::default();
        let mut container_font_size = memo.document_font_size;
        for member in block.containers.iter() {
            let character = memo.container_character(sheet, &member.container)?;
            merge_container_character_properties(&mut container_character, &mut container_font_size, character);
        }
        let container_body = block.containers.iter().any(|member| member.container.style == block.style)
            && sheet.block_style(&block.style).is_some_and(|style| style.role.is_container());
        Ok(Self {
            paragraph_style: if container_body { &sheet.base_paragraph } else { &block.style },
            container_character,
        })
    }
}

/// Cascades already resolved during one style resolution. Styled segments
/// repeat a few combinations within and across paragraphs, paragraphs repeat
/// a few styles, and blocks share their container allocations. Every cascade
/// is pure in its inputs.
struct CharacterCascadeMemo {
    document_font_size: f32,
    entries: Vec<(CharacterCascadeKey, ResolvedCharacterStyle)>,
    last_hit: usize,
    paragraphs: Vec<(ParagraphCascadeKey, crate::document::ResolvedParagraphStyle)>,
    /// Keyed by the shared container allocation, alive for the resolution.
    containers: Vec<(*const crate::document::ContainerAttributes, ContainerDeclarations)>,
    container_characters: Vec<(*const crate::document::ContainerAttributes, CharacterProperties)>,
}

/// Sheet lookups for one container, independent of its ancestors.
struct ContainerDeclarations {
    resolved: crate::document::ResolvedParagraphStyle,
    character: CharacterProperties,
    paragraph: crate::document::BlockProperties,
}

#[derive(PartialEq)]
struct ParagraphCascadeKey {
    paragraph_style: StyleId,
    direct_paragraph: crate::document::BlockProperties,
    container_character: CharacterProperties,
    default_character: CharacterProperties,
    /// Container paragraph defaults applied after the cascade, if any.
    container_paragraph: Option<crate::document::BlockProperties>,
}

struct CharacterCascadeKey {
    paragraph_style: StyleId,
    direct_paragraph: crate::document::BlockProperties,
    container_character: CharacterProperties,
    direct_default_character: CharacterProperties,
    named: Option<StyleId>,
    semantic: CharacterProperties,
}

impl CharacterCascadeKey {
    fn matches(&self, paragraph_style: &StyleId, direct_paragraph: &crate::document::BlockProperties,
        container_character: &CharacterProperties, direct_default_character: &CharacterProperties,
        named: Option<&StyleId>, semantic: &CharacterProperties) -> bool {
        self.paragraph_style == *paragraph_style && self.named.as_ref() == named && self.semantic == *semantic
            && self.direct_default_character == *direct_default_character
            && self.container_character == *container_character && self.direct_paragraph == *direct_paragraph
    }
}

impl CharacterCascadeMemo {
    /// Bounds each lookup's linear scan; a region with more combinations
    /// simply starts over.
    const CAPACITY: usize = 64;

    fn new(document_font_size: f32) -> Self {
        Self { document_font_size, entries: Vec::new(), last_hit: 0, paragraphs: Vec::new(),
            containers: Vec::new(), container_characters: Vec::new() }
    }

    /// The paragraph cascade, followed by container paragraph defaults when
    /// they are supplied.
    #[allow(clippy::too_many_arguments)]
    fn paragraph(&mut self, sheet: &StyleSheet, document: &DocumentStyleAssignment, paragraph_style: &StyleId,
        direct_paragraph: &crate::document::BlockProperties, container_character: &CharacterProperties,
        default_character: &CharacterProperties, container_paragraph: Option<&crate::document::BlockProperties>)
        -> Result<crate::document::ResolvedParagraphStyle, DocumentStyleError> {
        if let Some((_, resolved)) = self.paragraphs.iter().find(|(key, _)| key.paragraph_style == *paragraph_style
            && key.direct_paragraph == *direct_paragraph && key.container_character == *container_character
            && key.default_character == *default_character && key.container_paragraph.as_ref() == container_paragraph)
        {
            return Ok(resolved.clone());
        }
        let mut resolved = sheet.resolve_assigned_paragraph_style_in_container(document, paragraph_style,
            direct_paragraph, container_character, default_character, None, &CharacterProperties::default())?;
        if let Some(defaults) = container_paragraph {
            sheet.apply_container_paragraph_defaults(paragraph_style, direct_paragraph, defaults, &mut resolved)?;
        }
        if self.paragraphs.len() == Self::CAPACITY { self.paragraphs.clear(); }
        self.paragraphs.push((ParagraphCascadeKey {
            paragraph_style: paragraph_style.clone(),
            direct_paragraph: direct_paragraph.clone(),
            container_character: container_character.clone(),
            default_character: default_character.clone(),
            container_paragraph: container_paragraph.cloned(),
        }, resolved.clone()));
        Ok(resolved)
    }

    fn container(&mut self, sheet: &StyleSheet, document: &DocumentStyleAssignment,
        container: &std::sync::Arc<crate::document::ContainerAttributes>)
        -> Result<&ContainerDeclarations, DocumentStyleError> {
        let key = std::sync::Arc::as_ptr(container);
        if let Some(index) = self.containers.iter().position(|(entry, _)| *entry == key) {
            return Ok(&self.containers[index].1);
        }
        let direct = container.direct_formatting.as_deref().cloned().unwrap_or_default();
        let resolved = sheet.resolve_assigned_container_style(document,
            &container.style, &direct.direct_paragraph, &direct.direct_default_character)?;
        let character = sheet.container_character_declarations(&container.style, &direct.direct_default_character)?;
        let paragraph = sheet.container_paragraph_declarations(&container.style, &direct.direct_paragraph)?;
        if self.containers.len() == Self::CAPACITY { self.containers.clear(); }
        self.containers.push((key, ContainerDeclarations { resolved, character, paragraph }));
        Ok(&self.containers.last().expect("pushed above").1)
    }

    /// Only the container's character declarations, as a styled segment needs.
    fn container_character(&mut self, sheet: &StyleSheet,
        container: &std::sync::Arc<crate::document::ContainerAttributes>)
        -> Result<CharacterProperties, DocumentStyleError> {
        let key = std::sync::Arc::as_ptr(container);
        if let Some((_, declarations)) = self.containers.iter().find(|(entry, _)| *entry == key) {
            return Ok(declarations.character.clone());
        }
        if let Some((_, character)) = self.container_characters.iter().find(|(entry, _)| *entry == key) {
            return Ok(character.clone());
        }
        let direct = container.direct_formatting.as_deref().map(|direct| &direct.direct_default_character)
            .cloned().unwrap_or_default();
        let character = sheet.container_character_declarations(&container.style, &direct)?;
        if self.container_characters.len() == Self::CAPACITY { self.container_characters.clear(); }
        self.container_characters.push((key, character.clone()));
        Ok(character)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve(&mut self, sheet: &StyleSheet, document: &DocumentStyleAssignment, paragraph_style: &StyleId,
        direct_paragraph: &crate::document::BlockProperties, container_character: &CharacterProperties,
        direct_default_character: &CharacterProperties, named: Option<&StyleId>, semantic: &CharacterProperties)
        -> Result<ResolvedCharacterStyle, DocumentStyleError> {
        let matches = |key: &CharacterCascadeKey| key.matches(paragraph_style, direct_paragraph,
            container_character, direct_default_character, named, semantic);
        let hit = self.entries.get(self.last_hit).filter(|(key, _)| matches(key)).map(|_| self.last_hit)
            .or_else(|| self.entries.iter().position(|(key, _)| matches(key)));
        if let Some(index) = hit {
            self.last_hit = index;
            return Ok(self.entries[index].1.clone());
        }
        let resolved = sheet.resolve_assigned_paragraph_style_in_container(document, paragraph_style,
            direct_paragraph, container_character, direct_default_character, named, semantic)?.character;
        if self.entries.len() == Self::CAPACITY { self.entries.clear(); }
        self.entries.push((CharacterCascadeKey {
            paragraph_style: paragraph_style.clone(),
            direct_paragraph: direct_paragraph.clone(),
            container_character: container_character.clone(),
            direct_default_character: direct_default_character.clone(),
            named: named.cloned(),
            semantic: semantic.clone(),
        }, resolved.clone()));
        self.last_hit = self.entries.len() - 1;
        Ok(resolved)
    }
}

/// The Image paragraph definition also owns labels on inline image objects
/// and their complete Source notation. It is presentation, not a character
/// assignment inherited by surrounding typed prose.
fn inline_image_character(
    input: StyleCascadeInput<'_>,
    range: &Range<usize>,
) -> Result<Option<ResolvedCharacterStyle>, DocumentStyleError> {
    let sheet = input.style_sheet;
    let image_index = input.inline_images.partition_point(|image| image.range.end <= range.start);
    if !input.inline_images.get(image_index).is_some_and(|image|
        image.range.start <= range.start && range.end <= image.range.end) {
        return Ok(None);
    }
    let id = StyleId::from("Image");
    let style = if sheet.block_style(&id).is_some() { &id } else { &sheet.base_paragraph };
    sheet.resolve_assigned_paragraph_style(input.document_style, style,
        &Default::default(), &Default::default(), None, &Default::default())
        .map(|paragraph| Some(paragraph.character)).map_err(Into::into)
}

/// Character spans folded in their original (cascade) order into sparse
/// buckets. Folding is pure, so a sweep can resume from the state before the
/// first span that changed instead of refolding every span that is active
/// over each segment; nested spans would otherwise make a long source line
/// quadratic.
#[derive(Clone, Default)]
struct SpanCascade<'a> {
    named: Option<&'a StyleId>,
    semantic: CharacterProperties,
    direct: CharacterProperties,
    automatic: CharacterProperties,
    link_defaults: CharacterProperties,
}

impl<'a> SpanCascade<'a> {
    fn apply(&mut self, sheet: &StyleSheet, span: &'a StyleSpan, range: &Range<usize>)
        -> Result<(), DocumentStyleError> {
        match &span.application {
            StyleApplication::Automatic(id) => {
                if id.0 == "Link" {
                    merge_character_properties(&mut self.link_defaults, &sheet.automatic_character_properties(id)?);
                } else {
                    let mut chain = Vec::new();
                    let mut current = Some(id);
                    while let Some(id) = current {
                        let style = sheet.character_style(id)
                            .ok_or_else(|| StyleError::UnknownStyle(id.clone()))?;
                        chain.push(&style.properties);
                        current = style.based_on.as_ref();
                    }
                    for properties in chain.into_iter().rev() {
                        merge_character_properties(&mut self.automatic, properties);
                    }
                }
            }
            StyleApplication::Named(id) => {
                if self.named.is_some_and(|existing| existing != id) {
                    return Err(DocumentStyleError::MultipleNamedCharacterStyles {
                        range_start: range.start,
                    });
                }
                self.named = Some(id);
            }
            StyleApplication::Semantic(value) => {
                merge_character_properties(&mut self.semantic, &semantic_properties(*value));
            }
            StyleApplication::Direct(properties) => {
                merge_character_properties(&mut self.direct, properties);
            }
        }
        Ok(())
    }

    fn resolve(self, input: StyleCascadeInput<'_>, block: &Block, context: &BlockCharacterContext<'_>,
        memo: &mut CharacterCascadeMemo) -> Result<ResolvedCharacterStyle, DocumentStyleError> {
        let Self { named, mut semantic, direct, automatic, link_defaults } = self;
        // Semantic markup is a sparse convenience layer. Explicit direct
        // formatting wins property-by-property, independent of source-span order.
        merge_character_properties(&mut semantic, &direct);
        merge_character_properties(&mut semantic, &automatic);
        // Link is a content-derived default. Authored character styles and direct
        // formatting still win; source/code syntax overlays retain their existing
        // priority. Keep it out of the authored typing-style cascade.
        let linked;
        let direct_default_character = if link_defaults != CharacterProperties::default() {
            let mut value = block.direct_default_character.clone();
            merge_character_properties(&mut value, &link_defaults);
            linked = value;
            &linked
        } else {
            &block.direct_default_character
        };
        memo.resolve(input.style_sheet, input.document_style, context.paragraph_style, &block.direct_paragraph,
            &context.container_character, direct_default_character, named, &semantic)
    }
}

fn merge_container_character_properties(target: &mut CharacterProperties, inherited_size: &mut f32,
    mut declarations: CharacterProperties) {
    // CSS percentages resolve at each document ancestor, not once after the
    // sparse declarations from every level have been combined.
    if let Some(size) = declarations.size {
        *inherited_size = size.resolve(*inherited_size);
        declarations.size = Some((*inherited_size).into());
    }
    merge_character_properties(target, &declarations);
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
    if source.font_families.is_some() {
        destination.font_face = None;
        destination.font_axes = None;
        destination.weight = None;
    }
    replace_some!(font_families);
    replace_some!(font_face);
    replace_some!(font_axes);
    replace_some!(size);
    replace_some!(weight);
    replace_some!(bold);
    replace_some!(slant);
    replace_some!(foreground);
    replace_some!(background);
    replace_some!(underline);
    replace_some!(strikethrough);
    replace_some!(superscript);
    replace_some!(subscript);
    replace_some!(language);
    replace_some!(direction);
    replace_some!(open_type_features);
    replace_some!(letter_spacing);
}

pub(crate) fn shaping_style(
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
        font_face: character.font_face.clone(),
        font_axes: character.font_axes.clone(),
        size: character.size * if character.superscript || character.subscript { 0.75 } else { 1.0 },
        baseline_offset: character.size * if character.superscript { 0.35 } else if character.subscript { -0.2 } else { 0.0 },
        weight: f32::from(character.weight),
        relative_bold: character.bold,
        slant: character.slant,
        letter_spacing: character.letter_spacing,
        language: character.language.clone(),
        script: None,
        direction: match character.direction {
            WritingDirection::Natural => TextDirection::Auto,
            WritingDirection::LeftToRight => TextDirection::LeftToRight,
            WritingDirection::RightToLeft => TextDirection::RightToLeft,
        },
        features,
    }
    .with_implicit_kerning())
}

fn paint_style(character: &ResolvedCharacterStyle) -> ResolvedTextPaint {
    ResolvedTextPaint {
        foreground: character.foreground,
        foreground_is_default: character.foreground_is_default,
        background: character.background,
        underline: character.underline,
        strikethrough: character.strikethrough,
        baseline_offset: character.size * if character.superscript { 0.35 } else if character.subscript { -0.2 } else { 0.0 },
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
    fn span_sweep_preserves_overlapping_cascade_order() {
        let document = Document::new("x".repeat(257));
        let projection = document.projection();
        // Deliberately unordered starts, crossing extents, and coincident
        // endpoints: source order still decides direct-property precedence.
        let spans = (0..96).rev().map(|index| {
            let start = index * 31 % 220;
            StyleSpan { range: start..start + 1 + index % 30,
                application: StyleApplication::direct(CharacterProperties {
                    bold: Some(index % 2 == 0),
                    underline: (index % 3 == 0).then_some(true),
                    size: Some((12.0 + (index % 5) as f32).into()),
                    ..Default::default()
                }) }
        }).collect::<Vec<_>>();
        let input = StyleCascadeInput { blocks: projection.blocks(), style_spans: &spans, inline_images: &[],
            style_sheet: projection.style_sheet(), document_style: projection.document_style(), search_matches: &[] };
        let actual = DocumentLayoutStyles::resolve_validated(input, None).unwrap();
        let mut shapes = Vec::new();
        let mut paints = Vec::new();
        for at in 0..257 {
            let range = at..at + 1;
            let character = resolve_character_at(input, &projection.blocks()[0], range.clone()).unwrap();
            let shape = shaping_style(&character).unwrap();
            let paint = paint_style(&character);
            if shape != actual.default_shaping_style { push_shape_run(&mut shapes, range.clone(), shape); }
            if paint != actual.default_paint { push_paint_run(&mut paints, range, paint); }
        }
        assert_eq!(actual.shaping_runs, shapes);
        assert_eq!(actual.paint_runs, paints);
    }

    #[test]
    fn span_sweep_resumes_deeply_nested_and_crossing_cascades() {
        let document = Document::new("x".repeat(400));
        let projection = document.projection();
        // Nested spans close in reverse order; crossing spans remove entries
        // from the middle of the active cascade, so resumption must refold.
        let mut spans = (0..150).map(|index| StyleSpan { range: index..400 - index,
            application: if index % 3 == 0 {
                StyleApplication::Semantic(if index % 2 == 0 { SemanticInlineStyle::Strong } else { SemanticInlineStyle::Emphasis })
            } else {
                StyleApplication::direct(CharacterProperties {
                    bold: Some(index % 4 == 1),
                    size: Some((10.0 + (index % 7) as f32).into()),
                    underline: (index % 5 == 0).then_some(index % 10 == 0),
                    ..Default::default()
                })
            } }).collect::<Vec<_>>();
        spans.extend((0..40).map(|index| StyleSpan { range: index * 7..index * 7 + 90,
            application: StyleApplication::direct(CharacterProperties {
                strikethrough: Some(index % 2 == 0), ..Default::default() }) }));
        let input = StyleCascadeInput { blocks: projection.blocks(), style_spans: &spans, inline_images: &[],
            style_sheet: projection.style_sheet(), document_style: projection.document_style(), search_matches: &[] };
        let actual = DocumentLayoutStyles::resolve_validated(input, None).unwrap();
        let mut shapes = Vec::new();
        let mut paints = Vec::new();
        for at in 0..400 {
            let range = at..at + 1;
            let character = resolve_character_at(input, &projection.blocks()[0], range.clone()).unwrap();
            let shape = shaping_style(&character).unwrap();
            let paint = paint_style(&character);
            if shape != actual.default_shaping_style { push_shape_run(&mut shapes, range.clone(), shape); }
            if paint != actual.default_paint { push_paint_run(&mut paints, range, paint); }
        }
        assert_eq!(actual.shaping_runs, shapes);
        assert_eq!(actual.paint_runs, paints);
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
                    based_on: Some(sheet.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Document,
                    character: CharacterProperties {
                        font_families: Some(vec!["Theme Sans".to_owned()]),
                        size: Some(11.0.into()),
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
        root.direct_default_character.weight = Some(550);
        root.direct_default_character.size = Some(13.0.into());
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
        root.direct_default_character.size = Some(12.0.into());
        let mut blocks = projection.blocks().to_vec();
        blocks[0].direct_paragraph = BlockProperties {
            margin_top: Some(7.0),
            margin_bottom: Some(5.0),
            first_line_indent: Some(11.0),
            leading_indent: Some(3.0),
            trailing_indent: Some(4.0),
            ..BlockProperties::default()
        };
        blocks[0].direct_default_character = CharacterProperties {
            font_families: Some(vec!["Paragraph Serif".to_owned()]),
            size: Some(17.0.into()),
            ..CharacterProperties::default()
        };

        let styles =
            resolve_custom(projection, &sheet, &root, &blocks, projection.style_spans()).unwrap();
        let first = &styles.paragraphs[0];
        assert_eq!(first.margin_top, 7.0);
        assert_eq!(first.margin_bottom, 5.0);
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
                    based_on: None,
                    properties: CharacterProperties {
                        font_families: Some(vec!["Named Serif".to_owned()]),
                        size: Some(20.0.into()),
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
            size: Some(16.0.into()),
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
                application: StyleApplication::direct(CharacterProperties {
                    size: Some(24.0.into()),
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
        let document = Document::from_bytes(
            b"plain".to_vec(), crate::document::Encoding::Utf8, crate::document::Format::Markdown,
        ).unwrap();
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
        root.style = "Heading1".into();
        assert!(matches!(
            resolve_custom(projection, &sheet, &root, &blocks, &[]),
            Err(DocumentStyleError::Cascade(
                StyleError::IncompatibleBlockRole { .. }
            ))
        ));

        root = projection.document_style().clone();
        root.direct_canvas.margin_top = Some(2.0);
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
        root.direct_default_character.size = Some(0.0.into());
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
        invalid_blocks[0].style = sheet.base_paragraph.clone();
        assert!(resolve_custom(projection, &sheet, &root, &invalid_blocks, &[]).is_ok());
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
