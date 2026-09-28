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
    pub list_indent_support: Option<(bool, bool)>,
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
            list_indent_support: None,
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
            // A new paragraph owns this empty line. A closed, transparent
            // sibling may have supplied an earlier seed at the same logical
            // boundary; retaining it would redirect typing into that sibling
            // and create an unintended paragraph before the actual owner.
            while self.provenance.last().is_some_and(|span| {
                span.formatted == (self.text.len()..self.text.len()) && span.source.is_empty()
            }) {
                self.provenance.pop();
            }
            // Point annotations belong to that same discarded empty context.
            // In particular, a preceding whitespace-preserving container must
            // not make the new paragraph's spaces behave as preformatted text.
            while self
                .spans
                .last()
                .is_some_and(|span| span.range == (self.text.len()..self.text.len()))
            {
                self.spans.pop();
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
        let style = self
            .paragraph_style
            .clone()
            .unwrap_or_else(|| match &self.kind {
                BlockKind::Heading(level) => format!("Heading{level}").as_str().into(),
                BlockKind::ListItem { ordered, level, .. } => {
                    self.style_sheet.list_style_id(*ordered, *level)
                }
                _ => "Paragraph".into(),
            });

        let mut block = Block::new(
            0,
            start..self.text.len(),
            self.kind.clone(),
            style,
            super::BlockDirectFormatting::shared(self.paragraph.clone(), self.defaults.clone()),
        );
        if let Some((indent, unindent)) = self
            .list_indent_support
            .filter(|_| matches!(self.kind, BlockKind::ListItem { .. }))
        {
            block.list_editing.indent = indent;
            block.list_editing.unindent = unindent;
        }
        block
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
        self.paragraphs
            .push(self.current_block(self.paragraph_start));
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
    Err(DocumentError::AmbiguousProjection)
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

pub(super) fn resolved_character_at(
    document: &FormattedDocument,
    at: usize,
) -> Option<super::ResolvedCharacterStyle> {
    resolved_character_at_with_style_context(
        document,
        at,
        document.style_sheet(),
        document.document_style(),
    )
}

pub(super) fn resolved_character_at_with_style_context(
    document: &FormattedDocument,
    at: usize,
    style_sheet: &super::StyleSheet,
    document_style: &super::DocumentStyleAssignment,
) -> Option<super::ResolvedCharacterStyle> {
    let blocks = document.blocks_for_region(&(at..at));
    let block = blocks.iter().find(|block| block.range.contains(&at))?;
    let paragraph_style = &block.style;
    let paragraph_defaults = &block.direct_default_character;
    let mut direct = CharacterProperties::default();
    let mut semantic = CharacterProperties::default();
    let mut named = None;
    let spans = document.style_spans_for_region(&(at..at + 1));
    for span in spans.iter().filter(|span| span.range.contains(&at)) {
        match &span.application {
            StyleApplication::Semantic(super::SemanticInlineStyle::Strong) => {
                semantic.bold = Some(true)
            }
            StyleApplication::Semantic(super::SemanticInlineStyle::Emphasis) => {
                semantic.slant = Some(super::FontSlant::Italic)
            }
            StyleApplication::Direct(properties) => overlay(&mut direct, properties),
            StyleApplication::Named(id) => named = Some(id),
            _ => {}
        }
    }
    overlay(&mut semantic, &direct);
    style_sheet
        .resolve_assigned_paragraph_style(
            document_style,
            paragraph_style,
            &block.direct_paragraph,
            paragraph_defaults,
            named,
            &semantic,
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
    let runs = visible
        .into_iter()
        .map(|run| run.source)
        .collect::<Vec<_>>();
    Ok(runs)
}
