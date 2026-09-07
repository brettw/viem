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
        for anchor in self
            .provenance
            .iter_mut()
            .rev()
            .take_while(|span| span.formatted == (start..start))
        {
            if !source.is_empty() && source.end <= anchor.source.start {
                anchor.formatted = range.end..range.end;
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
    fn finish_line(&mut self) {
        let mut style = self
            .paragraph_style
            .clone()
            .unwrap_or_else(|| match &self.kind {
                BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
                BlockKind::ListItem { level, .. } => {
                    format!("List{}", u16::from(*level) + 1).as_str().into()
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
        self.blocks.push(Block {
            id: 0,
            range: self.line_start..self.text.len(),
            kind: self.kind.clone(),
            style,
            direct_paragraph: self.paragraph.clone(),
            direct_default_character: self.defaults.clone(),
        });
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
        let mut style = self
            .paragraph_style
            .clone()
            .unwrap_or_else(|| match &self.kind {
                BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
                BlockKind::ListItem { level, .. } => {
                    format!("List{}", u16::from(*level) + 1).as_str().into()
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
        self.paragraphs.push(Block {
            id: 0,
            range: self.paragraph_start..self.text.len(),
            kind: self.kind.clone(),
            style,
            direct_paragraph: self.paragraph.clone(),
            direct_default_character: self.defaults.clone(),
        });
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
    if block.range.is_empty() {
        let spans = projection.provenance();
        let end = spans.partition_point(|span| span.formatted.start <= block.range.start);
        return spans[..end]
            .iter()
            .rev()
            .take_while(|span| span.formatted.start == block.range.start)
            .find(|span| span.formatted.is_empty() && span.source.is_empty())
            .map(|span| span.source.start)
            .ok_or(DocumentError::AmbiguousProjection);
    }
    let spans = projection.provenance_for_region(&block.range);
    spans
        .iter()
        .find(|span| !span.formatted.is_empty() && !span.source.is_empty())
        .map(|span| span.source.start)
        .or_else(|| {
            spans
                .iter()
                .rev()
                .find(|span| span.formatted == block.range && span.source.is_empty())
                .map(|span| span.source.start)
        })
        .ok_or(DocumentError::AmbiguousProjection)
}

/// A visible selection is editable only when all its source bytes are visible,
/// contiguous and individually mapped. In particular, hidden scripts, controls,
/// comments and destinations can never be consumed by the contiguous hull.
pub(super) fn editable_source_range(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> Result<Range<usize>, DocumentError> {
    if range.is_empty() {
        // An empty rich document requires a grammar-specific body insertion
        // anchor. Refuse to insert outside its wrapper until one is available.
        if projection.text_tree().byte_len() == 0 {
            return Err(DocumentError::AmbiguousProjection);
        }
        let downstream = range.start != projection.text_tree().byte_len()
            && !projection
                .hard_line_at_offset(range.start)
                .and_then(|line| projection.hard_line_range(line))
                .is_some_and(|line| range.start == line.end && !line.is_empty());
        let at = projection
            .source_insertion_point(range.start, downstream)
            .ok_or(DocumentError::AmbiguousProjection)?;
        return Ok(at..at);
    }
    let mut formatted = range.start;
    let mut result: Option<Range<usize>> = None;
    for span in projection.provenance_for_region(range) {
        if span.formatted.start != formatted
            || span.formatted.end > range.end
            || span.source.is_empty()
        {
            return Err(DocumentError::AmbiguousProjection);
        }
        formatted = span.formatted.end;
        if let Some(source) = result.as_mut() {
            if source.end != span.source.start {
                return Err(DocumentError::AmbiguousProjection);
            }
            source.end = span.source.end;
        } else {
            result = Some(span.source.clone());
        }
    }
    if formatted != range.end {
        return Err(DocumentError::AmbiguousProjection);
    }
    result.ok_or(DocumentError::AmbiguousProjection)
}

pub(super) fn overlay(target: &mut CharacterProperties, source: &CharacterProperties) {
    if source.weight.is_some() {
        target.bold = None;
    }
    macro_rules! copy { ($($field:ident),*) => { $(if source.$field.is_some() { target.$field = source.$field.clone(); })* }; }
    copy!(
        font_families,
        size,
        weight,
        bold,
        slant,
        foreground,
        background,
        underline,
        strikethrough,
        language,
        direction,
        open_type_features,
        letter_spacing,
        baseline_shift
    );
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
    let mut boundaries = vec![0, before.text().len(), selected.start, selected.end];
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.extend([span.range.start, span.range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let at = |document: &FormattedDocument,
              offset,
              override_properties: Option<&CharacterProperties>| {
        let block = document
            .blocks()
            .iter()
            .find(|block| block.range.contains(&offset))?;
        let mut direct = CharacterProperties::default();
        let mut named = None;
        for span in document
            .style_spans()
            .iter()
            .filter(|s| s.range.contains(&offset))
        {
            match &span.application {
                StyleApplication::Direct(layer) => overlay(&mut direct, layer),
                StyleApplication::Named(style) => named = Some(style),
                StyleApplication::SourcePreservedWhitespace => {}
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
        document
            .style_sheet()
            .resolve_assigned_paragraph_style(
                document.document_style(),
                &block.style,
                &block.direct_paragraph,
                &block.direct_default_character,
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
    macro_rules! copy {($($field:ident),*)=>{$(if source.$field.is_some(){target.$field=source.$field.clone();})*};}
    copy!(
        spacing_before,
        spacing_after,
        line_spacing,
        first_line_indent,
        leading_indent,
        trailing_indent,
        padding_top,
        padding_right,
        padding_bottom,
        padding_left,
        background,
        alignment,
        base_direction
    );
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
    escape_html_text_in_context(text, encoding, false)
}

fn escape_html_text_in_context(text: &str, encoding: super::Encoding, preserve: bool) -> String {
    let syntax = if preserve {
        super::html::escape_preserving_whitespace(text)
    } else {
        super::html::escape(text)
    };
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
    let preserve =
        text.contains([' ', '\t']) && html_preserves_whitespace_at_source(document, source_start)?;
    let syntax = escape_html_text_in_context(text, document.encoding(), preserve);
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
            escape_html_text_in_context(&text[first.len_utf8()..], document.encoding(), preserve)
        ),
        None => "<!---->".to_owned(),
    })
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
        if span.source.start == source_start && preserved(span.formatted.start)
            || span.source.end == source_start && preserved(span.formatted.end - 1)
        {
            return Ok(true);
        }
    }
    // Empty elements have a source anchor but no character to annotate. Read
    // only the local gap after the preceding visible character, never the
    // complete code paragraph. The annotation seeds inherited pre behavior.
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
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut position = range.start;
    for span in document.projection().provenance_for_region(range) {
        if span.formatted.start != position
            || span.formatted.end > range.end
            || span.source.is_empty()
        {
            return Err(DocumentError::AmbiguousProjection);
        }
        position = span.formatted.end;
        if let Some(last) = runs.last_mut().filter(|last| last.end == span.source.start) {
            last.end = span.source.end;
        } else {
            runs.push(span.source);
        }
    }
    if position != range.end {
        return Err(DocumentError::AmbiguousProjection);
    }
    Ok(runs)
}
