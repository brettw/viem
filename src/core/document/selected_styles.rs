use super::{BoundaryAffinity, FormattedDocument, StyleApplication, StyleId};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Named assignments surrounding a logical selection. Automatic syntax paint
/// and direct formatting are intentionally separate from assignment identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedNamedStyles {
    pub paragraph: Option<StyleId>,
    pub character: Option<StyleId>,
    pub paragraph_mixed: bool,
    pub character_mixed: bool,
}

impl FormattedDocument {
    pub(crate) fn selected_named_styles(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
    ) -> SelectedNamedStyles {
        let length = self.text_tree().byte_len();
        let start = if range.is_empty()
            && (affinity == BoundaryAffinity::Upstream || range.start == length)
        {
            range.start.saturating_sub(1)
        } else {
            range.start
        };
        let end = if range.is_empty() {
            (start + 1).min(length)
        } else {
            range.end
        };
        let blocks = self.blocks_for_region(&(start..end));
        let spans = self.style_spans_for_region(&(start..end));
        // Sweep only intersecting assignment spans. A caret query visits its
        // local interval frontier; a range query is O((blocks + spans) log n).
        let mut boundaries = BTreeSet::from([start]);
        for separator in self.hard_breaks_for_region(&(start..end)) {
            if separator + 1 < end {
                boundaries.insert(separator + 1);
            }
        }
        let mut starts = BTreeMap::<usize, Vec<usize>>::new();
        let mut ends = BTreeMap::<usize, Vec<usize>>::new();
        for block in &blocks {
            if start < block.range.start && block.range.start < end {
                boundaries.insert(block.range.start);
            }
        }
        for (index, span) in spans.iter().enumerate() {
            if !matches!(
                span.application,
                StyleApplication::Named(_)
                    | StyleApplication::SourceParagraph { .. }
                    | StyleApplication::Semantic(super::SemanticInlineStyle::Code)
            ) {
                continue;
            }
            let lower = span.range.start.max(start);
            let upper = span.range.end.min(end);
            if lower >= upper {
                continue;
            }
            boundaries.insert(lower);
            boundaries.insert(upper);
            starts.entry(lower).or_default().push(index);
            ends.entry(upper).or_default().push(index);
        }
        let mut paragraphs = BTreeSet::new();
        let mut characters = BTreeSet::new();
        let mut active_paragraphs = BTreeMap::new();
        let mut active_characters = BTreeMap::new();
        let mut active_code = BTreeSet::new();
        for point in boundaries {
            if point == end && start != end {
                break;
            }
            if let Some(indices) = ends.get(&point) {
                for index in indices {
                    active_paragraphs.remove(index);
                    active_characters.remove(index);
                    active_code.remove(index);
                }
            }
            if let Some(indices) = starts.get(&point) {
                for index in indices {
                    match &spans[*index].application {
                        StyleApplication::Named(id) if !id.is_internal() => {
                            active_characters.insert(*index, id.clone());
                        }
                        StyleApplication::SourceParagraph { style, .. } => {
                            active_paragraphs.insert(*index, style.clone());
                        }
                        StyleApplication::Semantic(super::SemanticInlineStyle::Code) => {
                            active_code.insert(*index);
                        }
                        _ => {}
                    }
                }
            }
            let block = blocks
                .partition_point(|block| block.range.start <= point)
                .checked_sub(1)
                .and_then(|index| blocks.get(index))
                .or_else(|| blocks.first());
            let paragraph = active_paragraphs
                .last_key_value()
                .map(|(_, id)| id.clone())
                .or_else(|| block.map(|block| block.style.clone()))
                .unwrap_or_else(|| self.style_sheet().base_paragraph.clone());
            let character = active_characters
                .last_key_value()
                .map(|(_, id)| id.clone())
                .or_else(|| (!active_code.is_empty()).then(|| StyleId::from("Code")))
                .unwrap_or_else(|| self.style_sheet().base_character.clone());
            paragraphs.insert(paragraph);
            // Character assignments attach to text within each containing
            // block. Separators between those wrappers carry no competing
            // base-character assignment for a nonempty selection.
            if range.is_empty()
                || !self
                    .hard_breaks_for_region(&(point..point + 1))
                    .contains(&point)
            {
                characters.insert(character);
            }
        }
        SelectedNamedStyles {
            paragraph_mixed: paragraphs.len() > 1,
            character_mixed: characters.len() > 1,
            paragraph: (paragraphs.len() == 1).then(|| paragraphs.into_iter().next().unwrap()),
            character: (characters.len() == 1).then(|| characters.into_iter().next().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    #[test]
    fn uniform_character_assignment_across_paragraphs_ignores_separator_gaps() {
        for (format, source) in [
            (
                Format::Html,
                "<p><code>one</code></p><p><code>two</code></p><p><code>three</code></p>",
            ),
            (Format::Markdown, "`one`\n\n`two`\n\n`three`"),
        ] {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let selected = document
                .projection()
                .selected_named_styles(1..9, BoundaryAffinity::Downstream);
            assert_eq!(selected.character, Some("Code".into()));
            assert!(!selected.character_mixed);
        }
        let document =
            Document::from_bytes(b"```\ncode\n```".to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(
            document
                .projection()
                .selected_named_styles(0..1, BoundaryAffinity::Downstream)
                .character,
            Some("Code".into())
        );
        for (format, source) in [
            (Format::Html, "<p><code>one</code><br>two</p>"),
            (Format::Markdown, "`one`  \ntwo"),
        ] {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let selected = document
                .projection()
                .selected_named_styles(0..document.text().len(), BoundaryAffinity::Downstream);
            assert!(selected.character_mixed, "{format:?}");
            assert_eq!(selected.character, None);
        }
    }

    #[test]
    fn source_syntax_preserves_surrounding_paragraph_assignment() {
        let source = "<h2 title='value'>Heading &amp; text</h2><p>Body</p>";
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap();
        for needle in ["h2", "title", "value", "Heading", "amp"] {
            let at = source.find(needle).unwrap();
            let selected = document
                .projection()
                .selected_named_styles(at..at, BoundaryAffinity::Downstream);
            assert_eq!(selected.paragraph, Some("Heading2".into()), "{needle}");
            assert_eq!(selected.character, Some("Character".into()));
        }
        let selected = document
            .projection()
            .selected_named_styles(0..source.len(), BoundaryAffinity::Downstream);
        assert!(selected.paragraph_mixed);
        assert_eq!(selected.paragraph, None);
        assert!(!selected.character_mixed);
    }

    #[test]
    fn named_character_assignment_ignores_automatic_paint_and_honors_affinity() {
        let document = Document::new("abcdef");
        let base = document.projection();
        let projected = FormattedDocument::from_parts(
            base.revision(),
            "abcdef".into(),
            base.blocks().to_vec(),
            vec![
                crate::document::StyleSpan {
                    range: 1..3,
                    application: StyleApplication::Named("Accent".into()),
                },
                crate::document::StyleSpan {
                    range: 0..6,
                    application: StyleApplication::Automatic("* HTML Tag name".into()),
                },
                crate::document::StyleSpan {
                    range: 0..6,
                    application: StyleApplication::SourceSyntax,
                },
            ],
            base.provenance().to_vec(),
            vec![],
            base.style_sheet().clone(),
            0,
            6,
        );
        assert_eq!(
            projected
                .selected_named_styles(1..1, BoundaryAffinity::Downstream)
                .character,
            Some("Accent".into())
        );
        assert_eq!(
            projected
                .selected_named_styles(1..1, BoundaryAffinity::Upstream)
                .character,
            Some("Character".into())
        );
        assert_eq!(
            projected
                .selected_named_styles(3..3, BoundaryAffinity::Upstream)
                .character,
            Some("Accent".into())
        );
        assert_eq!(
            projected
                .selected_named_styles(3..3, BoundaryAffinity::Downstream)
                .character,
            Some("Character".into())
        );
        assert!(
            projected
                .selected_named_styles(0..6, BoundaryAffinity::Downstream)
                .character_mixed
        );
    }

    #[test]
    fn empty_document_has_base_assignments() {
        let document = Document::new("");
        let selected = document
            .projection()
            .selected_named_styles(0..0, BoundaryAffinity::Downstream);
        assert_eq!(selected.paragraph, Some("Paragraph".into()));
        assert_eq!(selected.character, Some("Character".into()));
    }
}
