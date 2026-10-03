use super::{BoundaryAffinity, FormattedDocument, StyleApplication, StyleId};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

mod code;

/// Named styles surrounding a logical selection. Code reports automatic
/// character-style assignments; other formats report authored assignments.
/// Direct formatting never creates a separate named-style identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedNamedStyles {
    pub paragraph: Option<StyleId>,
    pub character: Option<StyleId>,
    pub paragraph_mixed: bool,
    pub character_mixed: bool,
    /// Structural list membership is independent of named paragraph styles
    /// and nesting depth. A selection may contain more than one kind.
    pub has_bullets: bool,
    pub has_numbering: bool,
    pub has_non_list: bool,
    pub has_quotes: bool,
    pub has_non_quote: bool,
    pub has_table: bool,
    /// Structural code ownership, including quoted fences; excludes inline Code.
    pub has_code_block: bool,
}

impl FormattedDocument {
    pub(crate) fn selected_named_styles(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
    ) -> SelectedNamedStyles {
        let length = self.text_tree().byte_len();
        // An empty paragraph has its own editable boundary even at EOF. It
        // must not inherit the previous paragraph's menu assignment merely
        // because there is no following character to sample.
        let empty_paragraph = range.is_empty()
            && self
                .blocks_for_region(&range)
                .iter()
                .any(|block| block.range.is_empty() && block.range.start == range.start);
        let start = if range.is_empty()
            && !empty_paragraph
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
        let mut active_characters = BTreeMap::new();
        let mut active_code = BTreeSet::new();
        let mut has_bullets = false;
        let mut has_numbering = false;
        let mut has_non_list = false;
        let mut has_code_block = false;
        let mut has_quotes = false;
        let mut has_non_quote = false;
        for point in boundaries {
            if point == end && start != end {
                break;
            }
            if let Some(indices) = ends.get(&point) {
                for index in indices {
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
            let paragraph = block.map(|block| {
                    if block.style == self.style_sheet().base_paragraph || block.style.0 == "Code Block" {
                        block.containers.iter().rev().find(|member| matches!(member.container.kind, super::ContainerKind::Quote | super::ContainerKind::CodeBlock))
                            .map(|member| member.container.style.clone()).unwrap_or_else(|| block.style.clone())
                    } else { block.style.clone() }
                })
                .unwrap_or_else(|| self.style_sheet().base_paragraph.clone());
            has_code_block |= block.is_some_and(|block| block.style.0 == "Code Block"
                || block.containers.iter().any(|member| member.container.kind == super::ContainerKind::CodeBlock));
            if block.is_some_and(|block| block.quote_depth > 0) { has_quotes = true; }
            else { has_non_quote = true; }
            match block.map(|block| &block.kind) {
                Some(super::BlockKind::ListItem { ordered: false, .. }) => has_bullets = true,
                Some(super::BlockKind::ListItem { ordered: true, .. }) => has_numbering = true,
                _ => has_non_list = true,
            }
            let character = active_characters
                .last_key_value()
                .map(|(_, id)| id.clone())
                .or_else(|| (!active_code.is_empty() && paragraph.0 != "Code Block").then(|| StyleId::from("Code")));
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
            has_quotes,
            has_non_quote,
            has_code_block,
            has_table: self.range_intersects_table(&range),
            has_bullets,
            has_numbering,
            has_non_list,
            paragraph_mixed: paragraphs.len() > 1,
            character_mixed: characters.len() > 1,
            paragraph: (paragraphs.len() == 1).then(|| paragraphs.into_iter().next().unwrap()),
            character: (characters.len() == 1).then(|| characters.into_iter().next().unwrap()).flatten(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    #[test]
    fn code_block_membership_survives_quotes_and_mixed_selections() {
        for format in [Format::Markdown, Format::MarkdownSource] {
            for code in ["```mermaid\ngraph LR\n```", "    graph LR", "> ```\n> graph LR\n> ```"] {
                let source = format!("Before `inline`\n\n{code}\n\nAfter");
                let doc = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
                let projection = doc.projection();
                let at = doc.text().find("graph").unwrap();
                assert!(projection.selected_named_styles(at..at, BoundaryAffinity::Downstream).has_code_block,
                    "{format:?}: {code}");
                assert!(projection.selected_named_styles(0..doc.text().len(), BoundaryAffinity::Downstream).has_code_block);
                assert!(!projection.selected_named_styles(0..6, BoundaryAffinity::Downstream).has_code_block);
                let inline = doc.text().find("inline").unwrap();
                assert!(!projection.selected_named_styles(inline..inline, BoundaryAffinity::Downstream).has_code_block);
                let after = doc.text().find("After").unwrap();
                assert!(!projection.selected_named_styles(after..after, BoundaryAffinity::Downstream).has_code_block);
            }
        }
    }

    #[test]
    fn list_membership_ignores_depth_and_retains_mixed_non_list_content() {
        for (format, source) in [
            (Format::Markdown, "- One\n  - Two\n\nPlain"),

        ] {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let projection = document.projection();
            let lists = projection.selected_named_styles(0..7, BoundaryAffinity::Downstream);
            assert!(lists.has_bullets);
            assert!(!lists.has_numbering);
            assert!(!lists.has_non_list);
            assert!(lists.paragraph_mixed);
            let mixed = projection.selected_named_styles(
                0..document.text().len(),
                BoundaryAffinity::Downstream,
            );
            assert!(mixed.has_bullets);
            assert!(mixed.has_non_list);
            assert!(!mixed.has_numbering);
            let edge = projection.selected_named_styles(0..8, BoundaryAffinity::Downstream);
            assert!(
                !edge.has_non_list,
                "The paragraph at a half-open upper edge is unselected"
            );
        }
    }

    #[test]
    fn empty_terminal_paragraph_reports_its_own_style_for_either_affinity() {
        for (format, source) in [

            (Format::Markdown, "prose\n\n> "),
        ] {
            let doc =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let at = doc.text().len();
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                let selected = doc.projection().selected_named_styles(at..at, affinity);
                assert_eq!(selected.paragraph, Some("Block quote".into()));
                assert!(!selected.paragraph_mixed);
            }
        }
    }

    #[test]
    fn uniform_character_assignment_across_paragraphs_ignores_separator_gaps() {
        for (format, source) in [

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
            None
        );
        for (format, source) in [

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
                    application: StyleApplication::Automatic("Syntax keyword".into()),
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
            None
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
            None
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
        assert_eq!(selected.character, None);
    }
}
