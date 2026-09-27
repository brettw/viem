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
    pub style: super::StyleId,
    pub quote_depth: usize,
    pub direct: super::BlockProperties,
    pub defaults: CharacterProperties,
}

impl ReplacementParagraphStyle {
    pub(crate) fn matches(&self, document: &Document, at: usize) -> bool {
        document
            .projection()
            .blocks_for_region(&(at..at))
            .iter()
            .find(|block| block.range.contains(&at) || block.range.start == at)
            .is_some_and(|block| {
                block.style == self.style
                    && block.direct_paragraph == self.direct
                    && block.direct_default_character == self.defaults
                    && block.quote_depth == self.quote_depth
            })
    }
}

impl Document {
    pub(crate) fn replacement_typing_context(
        &self,
        range: Range<usize>,
    ) -> Result<Option<ReplacementTypingContext>, DocumentError> {
        if !self.format().is_wysiwyg() || range.is_empty() {
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
        let paragraph = ReplacementParagraphStyle {
            style: owner.style.clone(),
            quote_depth: owner.quote_depth,
            direct: owner.direct_paragraph.clone(),
            defaults: owner.direct_default_character.clone(),
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
            self.link_at(self.text_point(sample)?)?
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
    pub source_caret: usize,
}
