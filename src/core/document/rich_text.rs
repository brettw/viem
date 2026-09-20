//! Shared mechanics for passive rich-source adapters. Syntax always remains in
//! the authoritative source tree; the projection records only visible units.
use super::line_endings::NormalizedText;
use super::{
    Block, BlockKind, BlockProperties, CharacterProperties, DecodingDiagnostic, DocumentError,
    FormattedDocument, ProvenanceSpan, Revision, StyleApplication, StyleSheet, StyleSpan,
};
use std::ops::Range;

pub(super) struct Builder<'a> {
    pub input: &'a NormalizedText,
    pub text: String,
    pub blocks: Vec<Block>,
    pub spans: Vec<StyleSpan>,
    pub provenance: Vec<ProvenanceSpan>,
    pub kind: BlockKind,
    pub paragraph: BlockProperties,
    pub defaults: CharacterProperties,
    pub style_sheet: StyleSheet,
    pub paragraph_style: Option<super::StyleId>,
    pub named_character: Option<super::StyleId>,
    line_start: usize,
    paragraph_start: usize,
    paragraphs: Vec<Block>,
    revision: Revision,
    diagnostics: Vec<DecodingDiagnostic>,
    pending_empty_seed: Option<usize>,
}
impl<'a> Builder<'a> {
    pub fn new(input: &'a NormalizedText, revision: Revision) -> Self {
        Self {
            input,
            text: String::new(),
            blocks: Vec::new(),
            spans: Vec::new(),
            provenance: Vec::new(),
            kind: BlockKind::Paragraph,
            paragraph: BlockProperties::default(),
            defaults: CharacterProperties::default(),
            style_sheet: StyleSheet::default(),
            paragraph_style: None,
            named_character: None,
            line_start: 0,
            paragraph_start: 0,
            paragraphs: Vec::new(),
            revision,
            diagnostics: Vec::new(),
            pending_empty_seed: None,
        }
    }
    pub fn source_range(&self, range: Range<usize>) -> Range<usize> {
        let first = self
            .input
            .units
            .partition_point(|u| u.normalized.end <= range.start);
        let last = self
            .input
            .units
            .partition_point(|u| u.normalized.start < range.end);
        if first < last {
            self.input.units[first].source.start..self.input.units[last - 1].source.end
        } else {
            let at = self
                .input
                .units
                .get(first)
                .map(|u| u.source.start)
                .or_else(|| self.input.units.last().map(|u| u.source.end))
                .unwrap_or(0);
            at..at
        }
    }
    pub fn emit(&mut self, value: &str, input_range: Range<usize>, style: &CharacterProperties) {
        if value.is_empty() {
            return;
        }
        if let Some(index) = self.pending_empty_seed.take() {
            self.provenance.remove(index);
        }
        let start = self.text.len();
        self.text.push_str(value);
        let range = start..self.text.len();
        let source = self.source_range(input_range.clone());
        // Collapsible HTML whitespace may be emitted after an empty inline
        // element closes. Its source still precedes that element's typing
        // anchor, so the anchor follows the newly materialized space.
        let mut moved_empty_boundary = false;
        for anchor in self
            .provenance
            .iter_mut()
            .rev()
            .take_while(|span| span.formatted == (start..start))
        {
            if !source.is_empty() && source.end <= anchor.source.start {
                anchor.formatted = range.end..range.end;
                moved_empty_boundary = true;
            }
        }
        if moved_empty_boundary {
            for span in self.spans.iter_mut().rev().take_while(|span| span.range.start >= start) {
                if span.range == (start..start)
                    && span.application == StyleApplication::SourcePreservedWhitespace
                {
                    span.range = range.end..range.end;
                }
            }
        }
        self.provenance.push(ProvenanceSpan {
            formatted: range.clone(),
            source: source.clone(),
        });
        if let Some(id) = &self.named_character {
            let application = StyleApplication::Named(id.clone());
            if let Some(last) = self
                .spans
                .iter_mut()
                .rev()
                .take(3)
                .find(|span| span.range.end == start && span.application == application)
            {
                last.range.end = range.end;
            } else {
                self.spans.push(StyleSpan {
                    range: range.clone(),
                    application,
                });
            }
        }
        if *style != CharacterProperties::default() {
            if let Some(last) = self.spans.iter_mut().rev().take(3).find(|s| {
                s.range.end == start && s.application == StyleApplication::Direct(style.clone())
            }) {
                last.range.end = range.end;
            } else {
                self.spans.push(StyleSpan {
                    range: range.clone(),
                    application: StyleApplication::Direct(style.clone()),
                });
            }
        }
        let first = self
            .input
            .units
            .partition_point(|u| u.normalized.end <= input_range.start);
        for unit in self.input.units[first..]
            .iter()
            .take_while(|u| u.normalized.start < input_range.end)
        {
            if let Some(kind) = unit.decoding_diagnostic {
                self.diagnostics.push(DecodingDiagnostic {
                    revision: self.revision,
                    encoding: self.input.encoding,
                    kind,
                    source_range: unit.source.clone(),
                    formatted_range: range.clone(),
                });
            }
        }
    }
    pub fn line_is_empty(&self) -> bool {
        self.text.len() == self.line_start
    }
    pub fn empty_boundary_at(&mut self, input_at: usize) {
        if self.line_is_empty() {
            if let Some(index) = self.pending_empty_seed.take() {
                self.provenance.remove(index);
            }
            let at = self.text.len();
            self.pending_empty_seed = Some(self.provenance.len());
            self.provenance.push(ProvenanceSpan {
                formatted: at..at,
                source: self.source_range(input_at..input_at),
            });
        }
    }
    pub fn retain_empty_boundary(&mut self, source_at: usize, style: &CharacterProperties) {
        if let Some(index) = self.pending_empty_seed.take() {
            self.provenance.remove(index);
        }
        let at = self.text.len();
        // Innermost empty formatting context is the insertion context; an
        // enclosing empty container must not supersede it on its later close.
        if !self
            .provenance
            .last()
            .is_some_and(|span| span.formatted == (at..at))
        {
            if self.line_is_empty() {
                self.defaults = style.clone();
            }
            self.provenance.push(ProvenanceSpan {
                formatted: at..at,
                source: source_at..source_at,
            });
        }
    }
    pub fn emit_read_only(&mut self, value: &str) {
        if let Some(index) = self.pending_empty_seed.take() {
            self.provenance.remove(index);
        }
        self.text.push_str(value);
    }
    /// An opaque item has no editable interior, but both of its visible edges
    /// remain insertion locations. Retain only those boundaries, without
    /// declaring its source bytes to be ordinary replaceable text.
    pub fn emit_read_only_with_boundaries(
        &mut self,
        value: &str,
        input_range: Range<usize>,
        style: &CharacterProperties,
    ) {
        let source = self.source_range(input_range);
        self.retain_empty_boundary(source.start, style);
        self.emit_read_only(value);
        self.retain_empty_boundary(source.end, style);
    }
    fn current_block(&self, start: usize) -> Block {
        let mut style = self
            .paragraph_style
            .clone()
            .unwrap_or_else(|| match &self.kind {
                BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
                BlockKind::ListItem { ordered, level, .. } => {
                    self.style_sheet.list_style_id(*ordered, *level)
                }
                _ => "Paragraph".into(),
            });
        if self
            .style_sheet
            .deleted_source_blocks()
            .any(|id| id == &style)
        {
            style = self.style_sheet.base_paragraph.clone();
        }
        Block::new(0, start..self.text.len(), self.kind.clone(), style, super::BlockDirectFormatting::shared(self.paragraph.clone(), self.defaults.clone()))
    }
    fn finish_line(&mut self) {
        self.blocks.push(self.current_block(self.line_start));
    }
    pub fn hard_break(&mut self, input_range: Range<usize>) {
        self.pending_empty_seed = None;
        self.finish_line();
        let named = self.named_character.take();
        self.emit("\n", input_range, &CharacterProperties::default());
        self.named_character = named;
        self.line_start = self.text.len();
    }
    fn finish_paragraph(&mut self) {
        self.paragraphs.push(self.current_block(self.paragraph_start));
    }
    /// A paragraph boundary is a hard break plus a separate paragraph identity.
    /// Inline br/line breaks keep paragraph styles, first-indent and spacing
    /// associated with the containing paragraph.
    pub fn paragraph_break(&mut self, input_range: Range<usize>) {
        self.finish_paragraph();
        self.hard_break(input_range);
        self.paragraph_start = self.text.len();
    }
    pub fn finish(mut self, start: usize, end: usize) -> FormattedDocument {
        self.finish_paragraph();
        self.finish_line();
        let mut result = FormattedDocument::from_parts(
            self.revision,
            self.text,
            self.blocks,
            self.spans,
            self.provenance,
            self.diagnostics,
            self.style_sheet,
            start,
            end,
        );
        result.install_paragraph_partition(self.paragraphs);
        result
    }
}

pub(super) fn text_source_range(
    document: &super::Document,
    range: &Range<usize>,
) -> Result<Range<usize>, DocumentError> {
    if document.projection().text_tree().byte_len() != 0 || !range.is_empty() {
        return editable_source_range(document.projection(), range);
    }
    if let Some(anchor) = document
        .projection()
        .provenance()
        .iter()
        .rev()
        .find(|span| span.formatted.is_empty() && span.source.is_empty())
    {
        return Ok(anchor.source.clone());
    }
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let normalized = super::line_endings::normalize(&decoded, document.file_format());
    let at = match document.format() {
        super::Format::Html => super::html::empty_insertion_point(&normalized),
        super::Format::Rtf => super::rtf::empty_insertion_point(&normalized)
            .or_else(|| document.source_bytes().is_empty().then_some(0)),
        _ => None,
    }
    .ok_or(DocumentError::AmbiguousProjection)?;
    Ok(at..at)
}

/// A structural list action identifies its source container from actual body
/// provenance. Empty items use their innermost editable anchor; decoration
/// never contributes a competing source boundary to the formatted model.
pub(super) fn list_item_source_point(
    projection: &FormattedDocument,
    item: &super::ListItemNode,
) -> Result<usize, DocumentError> {
    let block = projection
        .blocks()
        .iter()
        .find(|block| block.id == item.paragraph_id)
        .ok_or(DocumentError::AmbiguousProjection)?;
    block_source_point(projection, block)
}

pub(super) fn block_source_point(
    projection: &FormattedDocument,
    block: &Block,
) -> Result<usize, DocumentError> {
    super::source_edit::insertion_point(projection, block.range.start, None)
        .ok_or(DocumentError::AmbiguousProjection)
}

/// A visible selection is editable only when all its source bytes are visible,
/// contiguous and individually mapped. In particular, hidden scripts, controls,
/// comments and destinations can never be consumed by the contiguous hull.
pub(super) fn editable_source_range(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> Result<Range<usize>, DocumentError> {
    super::source_edit::contiguous_range(projection, range)
}

pub(super) fn overlay(target: &mut CharacterProperties, source: &CharacterProperties) {
    target.overlay(source);
}

/// Validate every affected and unaffected style interval, independent of span
/// splitting/coalescing performed by the parser. Source syntax changes may
/// alter span segmentation, but must not alter unrelated effective properties.
pub(super) fn character_edit_verified(
    before: &FormattedDocument,
    after: &FormattedDocument,
    selected: &Range<usize>,
    properties: &CharacterProperties,
) -> bool {
    character_edit_verified_with_queries(before, after, selected, properties, |_, _| {})
}

fn character_edit_verified_with_queries(
    before: &FormattedDocument,
    after: &FormattedDocument,
    selected: &Range<usize>,
    properties: &CharacterProperties,
    mut observe_queries: impl FnMut(usize, usize),
) -> bool {
    let mut boundaries = vec![0, before.text().len(), selected.start, selected.end];
    for at in before.hard_breaks_for_region(&(0..before.text().len())) {
        boundaries.extend([at, at + 1]);
    }
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.extend([span.range.start, span.range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut at = |document: &FormattedDocument,
              offset,
              override_properties: Option<&CharacterProperties>| {
        let blocks = document.blocks_for_region(&(offset..offset));
        let spans = document.style_spans_touching(&(offset..offset));
        observe_queries(blocks.len(), spans.len());
        let block = blocks.iter().find(|block| block.range.contains(&offset))?;
        let mut direct = CharacterProperties::default();
        let mut link_defaults = CharacterProperties::default();
        let mut named = None;
        for span in spans.iter().filter(|span| span.range.contains(&offset))
        {
            match &span.application {
                StyleApplication::Direct(layer) => overlay(&mut direct, layer),
                StyleApplication::Named(style) => named = Some(style),
                StyleApplication::SourcePreservedWhitespace => {}
                StyleApplication::Automatic(id) if id.0 == "Link" => {
                    overlay(&mut link_defaults,
                        &document.style_sheet().automatic_character_properties(id).ok()?);
                }
                StyleApplication::Semantic(_)
                | StyleApplication::Automatic(_)
                | StyleApplication::SourceSyntax
                | StyleApplication::SourceRawText
                | StyleApplication::SourceParagraph { .. } => return None,
            }
        }
        if let Some(properties) = override_properties {
            let mut properties = properties.clone();
            // A face selection changes its base weight while retaining the
            // independently authored emphasis at each affected run.
            if properties.font_families.is_some()
                && properties.weight.is_some()
                && properties.bold.is_none()
            {
                properties.bold = Some(resolved_character_at(document, offset)?.bold);
            }
            overlay(&mut direct, &properties);
        }
        let mut defaults = block.direct_default_character.clone();
        defaults.merge_declarations(&link_defaults);
        document
            .style_sheet()
            .resolve_assigned_paragraph_style(
                document.document_style(),
                &block.style,
                &block.direct_paragraph,
                &defaults,
                named,
                &direct,
            )
            .ok()
            .map(|resolved| resolved.character)
    };
    boundaries.windows(2).all(|pair| {
        if &before.text()[pair[0]..pair[1]] == "\n" {
            return true;
        }
        let override_properties =
            (selected.start <= pair[0] && pair[1] <= selected.end).then_some(properties);
        let expected = at(before, pair[0], override_properties);
        expected.is_some() && expected == at(after, pair[0], None)
    })
}

pub(super) fn overlay_block(target: &mut BlockProperties, source: &BlockProperties) {
    target.merge_declarations(source);
}

/// HTML direction on a paragraph supplies both the paragraph base and an
/// inherited character direction. Permit that one native side effect while
/// verifying every other effective character property and all outside runs.
pub(super) fn paragraph_direction_edit_verified(
    before: &FormattedDocument,
    after: &FormattedDocument,
    ranges: &[Range<usize>],
) -> bool {
    let mut boundaries = vec![0, before.text().len()];
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.extend([span.range.start, span.range.end]);
    }
    for block in before.blocks() { boundaries.extend([block.range.start, block.range.end]); }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries.windows(2).all(|pair| {
        if before.text().get(pair[0]..pair[1]) == Some("\n") { return true; }
        let Some(mut expected) = resolved_character_at(before, pair[0]) else { return false; };
        let Some(actual) = resolved_character_at(after, pair[0]) else { return false; };
        if ranges.iter().any(|range| range.start <= pair[0] && pair[1] <= range.end) {
            expected.direction = actual.direction;
        }
        expected == actual
    })
}

pub(super) fn character_clear_verified(
    before: &FormattedDocument,
    after: &FormattedDocument,
    range: &Range<usize>,
    clear: &std::collections::BTreeSet<super::StyleProperty>,
) -> bool {
    let mut boundaries = vec![0, before.text().len(), range.start, range.end];
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.extend([span.range.start, span.range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let at = |document: &FormattedDocument, offset| {
        let mut direct = CharacterProperties::default();
        let mut named = None;
        for span in document
            .style_spans()
            .iter()
            .filter(|span| span.range.contains(&offset))
        {
            match &span.application {
                StyleApplication::Direct(properties) => overlay(&mut direct, properties),
                StyleApplication::Named(id) => named = Some(id.clone()),
                _ => {}
            }
        }
        (direct, named)
    };
    boundaries.windows(2).all(|pair| {
        if before.text().get(pair[0]..pair[1]) == Some("\n") {
            return true;
        }
        let (mut expected, named) = at(before, pair[0]);
        if range.start <= pair[0] && pair[1] <= range.end {
            for property in clear {
                if super::style::clear_character_property(
                    &"Direct".into(),
                    &mut expected,
                    *property,
                )
                .is_err()
                {
                    return false;
                }
            }
        }
        (expected, named) == at(after, pair[0])
    })
}

/// Escape new text exactly without changing the source encoding. Characters
/// unavailable in the original converter use HTML numeric references.
pub(super) fn escape_html_text(text: &str, encoding: super::Encoding) -> String {
    escape_html_text_in_context(text, encoding, false, false, false)
}

fn escape_html_text_in_context(
    text: &str,
    encoding: super::Encoding,
    preserve: bool,
    before: bool,
    after: bool,
) -> String {
    escape_html_text_with_spaces(text, encoding, preserve, before, after, &[])
}

fn escape_html_text_with_spaces(
    text: &str,
    encoding: super::Encoding,
    preserve: bool,
    before: bool,
    after: bool,
    protective_spaces: &[usize],
) -> String {
    let syntax = if preserve {
        super::html::escape_preserving_whitespace(text)
    } else {
        super::html::escape_with_context(text, before, after)
    };
    let mut nbsp_offsets = text.char_indices().filter_map(|(at, ch)| (ch == '\u{a0}').then_some(at));
    let syntax = syntax.chars().map(|ch| {
        if ch == '\u{a0}' {
            if nbsp_offsets.next().is_some_and(|at| protective_spaces.binary_search(&at).is_ok()) {
                "&nbsp;".to_owned()
            } else {
                "&#160;".to_owned()
            }
        } else {
            ch.to_string()
        }
    }).collect::<String>();
    if encoding.encode_fragment(&syntax).is_ok() {
        return syntax;
    }
    syntax
        .chars()
        .map(|c| {
            if encoding.encode_fragment(&c.to_string()).is_ok() {
                c.to_string()
            } else {
                format!("&#x{:X};", c as u32)
            }
        })
        .collect()
}

/// Prevent a local splice from completing a character reference across its
/// boundary. The untouched prefix may itself be a valid semicolonless
/// reference. Escaping the first new character, or inserting an empty comment
/// for a deletion, preserves that prefix's existing interpretation.
pub(super) fn escape_html_source_edit(
    document: &super::Document,
    source_start: usize,
    text: &str,
) -> Result<String, DocumentError> {
    let neighbors = html_text_neighbors_at_source(document, source_start)?;
    escape_html_source_edit_with_context(document, source_start, text, neighbors, &[], false)
}

pub(super) fn escape_html_text_edit(
    document: &super::Document,
    source_start: usize,
    edit: &super::TextEdit,
) -> Result<String, DocumentError> {
    let (before, _) = html_text_neighbors_at_text(document, edit.range.start)?;
    let (_, after) = html_text_neighbors_at_text(document, edit.range.end)?;
    escape_html_source_edit_with_context(
        document,
        source_start,
        &edit.replacement,
        (before, after),
        &edit.html_protective_spaces,
        edit.html_normalized,
    )
}

fn escape_html_source_edit_with_context(
    document: &super::Document,
    source_start: usize,
    text: &str,
    (before, after): (bool, bool),
    protective_spaces: &[usize],
    normalized: bool,
) -> Result<String, DocumentError> {
    // A normalized edit already accounts for the whole intended batch. Its
    // spaces must not be reinterpreted against the old snapshot's neighbors.
    let preserve = normalized
        || text.contains([' ', '\t'])
            && html_preserves_whitespace_at_source(document, source_start)?;
    let syntax = escape_html_text_with_spaces(text, document.encoding(), preserve, before, after, protective_spaces);
    if source_start == 0
        || syntax
            .chars()
            .next()
            .is_some_and(|c| !c.is_ascii_alphanumeric() && c != ';')
    {
        return Ok(syntax);
    }
    // Named references have bounded length. Longer uninterrupted suffixes
    // can still be numeric references, so conservatively isolate them too.
    // 128 is even, retaining UTF-16 source-code-unit alignment.
    let start = source_start.saturating_sub(128);
    let bytes = document
        .state()
        .source
        .bytes_in(start..source_start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let prefix = document.encoding().decode_region(&bytes, start)?;
    let mut incomplete_reference = start != 0;
    for c in prefix.text.chars().rev() {
        if c == '&' {
            incomplete_reference = true;
            break;
        }
        if !c.is_ascii_alphanumeric() && c != '#' {
            incomplete_reference = false;
            break;
        }
    }
    if !incomplete_reference {
        return Ok(syntax);
    }
    Ok(match text.chars().next() {
        Some(first) => format!(
            "&#x{:X};{}",
            first as u32,
            escape_html_text_with_spaces(
                &text[first.len_utf8()..],
                document.encoding(),
                preserve,
                !super::html_whitespace::collapsible(first),
                after
                , &protective_spaces.iter().filter_map(|at| at.checked_sub(first.len_utf8())).collect::<Vec<_>>()
            )
        ),
        None => "<!---->".to_owned(),
    })
}

fn html_text_neighbors_at_source(
    document: &super::Document,
    source_at: usize,
) -> Result<(bool, bool), DocumentError> {
    let Ok(mapped) = document.projection().map_source_boundary(
        document.revision(),
        source_at,
        super::BoundaryAffinity::Downstream,
    ) else {
        return Ok((false, false));
    };
    html_text_neighbors_at_text(document, mapped.formatted_offset)
}

pub(super) fn html_text_neighbors_at_text(
    document: &super::Document,
    at: usize,
) -> Result<(bool, bool), DocumentError> {
    let tree = document.projection().text_tree();
    let previous = tree
        .previous_grapheme_boundary(at)
        .map_err(DocumentError::FormattedTextStorage)?;
    let next = tree
        .next_grapheme_boundary(at)
        .map_err(DocumentError::FormattedTextStorage)?;
    let is_content = |character: Option<char>, sample: usize| {
        character.is_some_and(|character| {
            if character == '\n' {
                return false;
            }
            !super::html_whitespace::collapsible(character)
                || document
                    .projection()
                    .style_spans_for_region(&(sample..sample + 1))
                    .iter()
                    .any(|span| {
                        span.range.contains(&sample)
                            && span.application == StyleApplication::SourcePreservedWhitespace
                    })
        })
    };
    let before = if let Some(start) = previous {
        let character = tree
            .slice(start..at)
            .map_err(DocumentError::FormattedTextStorage)?
            .chars()
            .next_back();
        is_content(character, at - 1)
    } else {
        false
    };
    let after = if let Some(end) = next {
        let character = tree
            .slice(at..end)
            .map_err(DocumentError::FormattedTextStorage)?
            .chars()
            .next();
        is_content(character, at)
    } else {
        false
    };
    Ok((before, after))
}

/// Compact only syntax emitted by our text encoder in this document. Authored
/// and reopened whitespace wrappers retain their exact source spelling.
pub(super) fn generated_html_space_before(
    document: &super::Document,
    at: usize,
) -> Result<Option<Range<usize>>, DocumentError> {
    let tree = document.projection().text_tree();
    if at == 0 || tree.slice(at - 1..at).as_deref() != Ok(" ") {
        return Ok(None);
    }
    let spans = document.projection().provenance_for_region(&(at - 1..at));
    let Some(space) = spans.iter().find(|span| span.formatted == (at - 1..at)) else {
        return Ok(None);
    };
    let opening = document
        .encoding()
        .encode_fragment("<span style=\"white-space: pre-wrap\">")?;
    let closing = document.encoding().encode_fragment("</span>")?;
    let Some(start) = space.source.start.checked_sub(opening.len()) else {
        return Ok(None);
    };
    let end = space.source.end + closing.len();
    let range = start..end;
    if !document
        .state()
        .source
        .range_is_generated_text(range.clone())
    {
        return Ok(None);
    }
    let Some(bytes) = document.state().source.bytes_in(range.clone()) else {
        return Ok(None);
    };
    if !bytes.starts_with(&opening) || !bytes.ends_with(&closing) {
        return Ok(None);
    }
    let content = document.encoding().decode_region(
        &bytes[opening.len()..bytes.len() - closing.len()],
        space.source.start,
    )?;
    if !matches!(content.text.as_str(), " " | "&#32;") {
        return Ok(None);
    }
    Ok(Some(range))
}

pub(super) fn compact_generated_html_space(
    document: &super::Document,
    edit: &super::TextEdit,
    source_at: usize,
) -> Result<Option<(Range<usize>, String)>, DocumentError> {
    if !edit.range.is_empty()
        || !edit
            .replacement
            .chars()
            .next()
            .is_some_and(|c| !super::html_whitespace::collapsible(c))
    {
        return Ok(None);
    }
    let at = edit.range.start;
    let Some(wrapper) = generated_html_space_before(document, at)? else {
        return Ok(None);
    };
    let closing = document.encoding().encode_fragment("</span>")?;
    if source_at != wrapper.end - closing.len() && source_at != wrapper.end {
        return Ok(None);
    }
    let tree = document.projection().text_tree();
    let Some(previous) = tree
        .previous_grapheme_boundary(at - 1)
        .map_err(DocumentError::FormattedTextStorage)?
    else {
        return Ok(None);
    };
    if !tree
        .slice(previous..at - 1)
        .map_err(DocumentError::FormattedTextStorage)?
        .chars()
        .next_back()
        .is_some_and(|c| !super::html_whitespace::collapsible(c))
    {
        return Ok(None);
    }
    let (_, after) = html_text_neighbors_at_source(document, source_at)?;
    let syntax = escape_html_text_in_context(
        &format!(" {}", edit.replacement),
        document.encoding(),
        false,
        true,
        after,
    );
    Ok(Some((wrapper, syntax)))
}

pub(super) fn html_preserves_whitespace_at_source(
    document: &super::Document,
    source_start: usize,
) -> Result<bool, DocumentError> {
    let projection = document.projection();
    let Ok(mapped) = projection.map_source_boundary(
        document.revision(),
        source_start,
        super::BoundaryAffinity::Downstream,
    ) else {
        return Ok(false);
    };
    let at = mapped.formatted_offset;
    let point = at..at;
    let boundary_provenance = projection.provenance_touching(&point);
    if boundary_provenance.iter().any(|span| {
        span.formatted == point && span.source == (source_start..source_start)
    }) {
        return Ok(projection.style_spans_touching(&point).iter().any(|span| {
            span.range == point && span.application == StyleApplication::SourcePreservedWhitespace
        }));
    }
    let range = at.saturating_sub(1)..(at + 1).min(projection.text_tree().byte_len());
    let styles = projection.style_spans_for_region(&range);
    let preserved = |sample| {
        styles.iter().any(|span| {
            span.range.contains(&sample)
                && span.application == StyleApplication::SourcePreservedWhitespace
        })
    };
    let provenance = projection.provenance_for_region(&range);
    for span in &provenance {
        if span.formatted.is_empty() {
            continue;
        }
        if span.source.start == source_start {
            return Ok(preserved(span.formatted.start));
        }
        if span.source.end == source_start {
            return Ok(preserved(span.formatted.end - 1));
        }
    }
    // For other source gaps, inspect only the local syntax after the previous
    // visible character, never the complete code paragraph. Exact empty
    // element anchors already carry their context above.
    let previous = provenance
        .iter()
        .filter(|span| !span.formatted.is_empty() && span.source.end <= source_start)
        .max_by_key(|span| span.source.end);
    let start = previous.map_or(0, |span| span.source.end);
    if source_start.saturating_sub(start) > 1024 {
        return Ok(false);
    }
    let bytes = document
        .state()
        .source
        .bytes_in(start..source_start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let gap = document.encoding().decode_region(&bytes, start)?;
    Ok(super::html::whitespace_after_source_gap(
        &gap.text,
        previous.is_some_and(|span| preserved(span.formatted.end - 1)),
    ))
}

pub(super) fn resolved_character_at(
    document: &FormattedDocument,
    at: usize,
) -> Option<super::ResolvedCharacterStyle> {
    let blocks = document.blocks_for_region(&(at..at));
    let block = blocks.iter().find(|block| block.range.contains(&at))?;
    let mut paragraph_style = &block.style;
    let mut paragraph_defaults = &block.direct_default_character;
    let mut direct = CharacterProperties::default();
    let mut named = None;
    let spans = document.style_spans_for_region(&(at..at + 1));
    for span in spans.iter().filter(|span| span.range.contains(&at)) {
        match &span.application {
            StyleApplication::Direct(properties) => overlay(&mut direct, properties),
            StyleApplication::Named(id) => named = Some(id),
            StyleApplication::SourceParagraph { style, defaults } => {
                paragraph_style = style;
                paragraph_defaults = defaults;
            }
            _ => {}
        }
    }
    document
        .style_sheet()
        .resolve_assigned_paragraph_style(
            document.document_style(),
            paragraph_style,
            &block.direct_paragraph,
            paragraph_defaults,
            named,
            &direct,
        )
        .ok()
        .map(|resolved| resolved.character)
}

/// Minimal discontiguous text runs. Hidden syntax between visible runs is
/// preserved and never consumed by a hull; synthetic/opaque visible items lack
/// a complete relation and therefore cannot participate in a text patch.
pub(super) fn text_source_runs(
    document: &super::Document,
    range: &Range<usize>,
) -> Result<Vec<Range<usize>>, DocumentError> {
    if range.is_empty() {
        return Ok(vec![text_source_range(document, range)?]);
    }
    let visible = super::source_edit::visible_runs(document.projection(), range)?;
    let mut runs = visible.into_iter().map(|run| run.source).collect::<Vec<_>>();
    // HTML whitespace may have additional collapsed contributors separated by
    // hidden syntax. Preserve that grammar-specific relation alongside the
    // shared minimal visible runs.
    if document.format() == super::Format::Html {
        let end = document.hard_line_snapshot().next_grapheme_boundary(range.end)
            .unwrap_or(range.end);
        let spans = document.projection().provenance_for_region(&(range.start..end));
        for (index, span) in spans.iter().enumerate().filter(|(_, span)| {
            !span.formatted.is_empty() && span.formatted.end <= range.end
        }) {
            if let Some(following) = spans[index + 1..].iter().find(|s| !s.formatted.is_empty()) {
                runs.extend(super::html_whitespace::collapsed_space_tail(
                    document, span, following.source.start,
                )?);
            }
        }
        runs.sort_by_key(|run| (run.start, run.end));
    }
    Ok(runs)
}

#[cfg(test)]
mod character_verification_tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    #[test]
    fn complete_character_verification_uses_bounded_context_queries_in_large_documents() {
        let paragraphs = 10_000;
        let source = "<p><span style='color:#123456'>A</span>B</p>".repeat(paragraphs);
        let before = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let mut block_records = 0;
        let mut style_records = 0;
        assert!(character_edit_verified_with_queries(
            before.projection(), before.projection(), &(0..0), &CharacterProperties::default(),
            |blocks, styles| { block_records += blocks; style_records += styles; },
        ));
        // Each complete-document interval needs only its adjacent paragraph
        // and style runs, regardless of all the other paragraphs in the file.
        assert!(block_records <= paragraphs * 8, "{block_records} block records");
        assert!(style_records <= paragraphs * 8, "{style_records} style records");
        assert!(block_records >= paragraphs * 2);
        assert!(style_records >= paragraphs);

        let mut changed = source;
        let at = changed.rfind("#123456").unwrap();
        changed.replace_range(at..at + 7, "#654321");
        let after = Document::from_bytes(changed.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        // A late, unselected style change must still be rejected: indexing
        // accelerates complete verification rather than narrowing its scope.
        assert!(!character_edit_verified(
            before.projection(), after.projection(), &(0..1), &CharacterProperties::default(),
        ));
    }
}
