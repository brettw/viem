//! Capture semantic style inheritance before replacement removes its source.
use super::*;

/// The first replaced paragraph and character contexts, captured before their
/// source is deleted. These are source syntax values, not stale source offsets.
/// Paragraph separators do not supply character scopes; retaining the selected
/// text's scopes also retains semantic identity such as a link destination.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplacementTypingContext {
    pub character: super::ResolvedCharacterStyle,
    pub named: Option<super::StyleId>,
    pub link: Option<String>,
    pub paragraph: ReplacementParagraphStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplacementParagraphStyle {
    pub code_language: Option<String>,
    pub style: super::StyleId,
    /// The replacement removes the table that owns this paragraph. Table cell
    /// styles belong to table syntax and are never assigned to a paragraph,
    /// so the replacement keeps the style of the paragraph left in its place.
    pub removes_table: bool,
    pub quote_depth: usize,
    pub markdown_prefix: Option<String>,
}

impl Document {
    fn markdown_replacement_prefix(&self, owner: &super::Block) -> Result<Option<String>, DocumentError> {
        if self.format() != Format::Markdown || owner.style.0 == "Code Block"
            || owner.markdown_html || owner.thematic_break
            || owner.quote_depth == 0 && !matches!(owner.kind, super::BlockKind::ListItem { .. } | super::BlockKind::Heading(_))
        { return Ok(None); }
        let at = self.projection().source_insertion_point(owner.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = self.state().source_hard_lines.line_at_offset(at)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(line.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let mut end = 0;
        loop {
            let previous = end;
            end += super::markdown_quotes::prefix(&decoded.text[end..]);
            if let Some(marker) = super::markdown_blocks::marker_prefix_length(&decoded.text[end..]) { end += marker; }
            if end == previous { break; }
        }
        let mut prefix = decoded.text[..end].to_owned();
        if let Some(level) = owner.style.0.strip_prefix("Heading").and_then(|level| level.parse::<usize>().ok()).filter(|level| (1..=6).contains(level)) {
            prefix.push_str(&"#".repeat(level)); prefix.push(' ');
        }
        Ok((!prefix.is_empty()).then_some(prefix))
    }

    pub(crate) fn replacement_typing_context(
        &self,
        range: Range<usize>,
    ) -> Result<Option<ReplacementTypingContext>, DocumentError> {
        if !self.format().is_markdown() || range.is_empty() {
            return Ok(None);
        }
        self.validate_range(&range)?;
        let blocks = self.projection().blocks_for_region(&range);
        let Some(owner) = blocks
            .iter()
            .find(|block| block.range.start <= range.start && range.start <= block.range.end)
        else {
            return Ok(None);
        };
        let removes_table = self.projection().table_at(owner.range.start)
            .is_some_and(|table| range.start <= table.range.start && table.range.end <= range.end);
        let paragraph = ReplacementParagraphStyle {
            // Image appearance is derived from the paragraph's contents. Once
            // replacement turns it into prose there is no image-only owner to
            // restore in source.
            style: if owner.style.0 == "Image" { self.projection().style_sheet().base_paragraph.clone() } else { owner.style.clone() },
            removes_table,
            quote_depth: owner.quote_depth,
            markdown_prefix: self.markdown_replacement_prefix(owner)? ,
            code_language: owner.code_language.clone(),
        };
        // Paragraph separators have no character style of their own. Skip
        // only structural separators, retaining authored hard breaks/spaces.
        let mut sample = range.start;
        while sample < range.end
            && blocks.iter().any(|block| block.range.end == sample)
            && self
                .projection()
                .text_tree()
                .slice(sample..sample + 1)
                .as_deref()
                == Ok("\n")
        {
            sample += 1;
        }
        let separators_only = sample == range.end;
        let character = if separators_only {
            crate::layout::DocumentLayoutStyles::semantic_character_at(
                self.projection(),
                range.start,
                true,
            )
            .map_err(|_| DocumentError::AmbiguousProjection)?
        } else if removes_table {
            // The cell's appearance (a bold header) leaves with its table;
            // only the text's own formatting is inherited.
            super::rich_text::resolved_character_at_in_paragraph(self.projection(), sample,
                Some(&self.projection().style_sheet().base_paragraph))
                .ok_or(DocumentError::AmbiguousProjection)?
        } else {
            super::rich_text::resolved_character_at(self.projection(), sample)
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let named = self
            .projection()
            .selected_named_styles(
                if separators_only {
                    range.start..range.start
                } else {
                    sample..sample
                },
                if separators_only {
                    BoundaryAffinity::Upstream
                } else {
                    BoundaryAffinity::Downstream
                },
            )
            .character;
        let link = if separators_only {
            None
        } else {
            self.link_snapshot_at(self.text_point(sample)?)?
                .filter(|link| link.destination_source.is_some()
                    || range.start > link.range.start || range.end < link.range.end)
                .map(|link| link.destination)
        };
        Ok(Some(ReplacementTypingContext {
            character,
            named,
            link,
            paragraph,
        }))
    }
}

pub(super) struct Insertion {
    pub source: Range<usize>,
    pub syntax: String,
}
