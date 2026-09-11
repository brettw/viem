use super::{BoundaryAffinity, FormattedDocument, Range, SelectedNamedStyles, StyleApplication};
use std::collections::{BTreeMap, BTreeSet};

impl FormattedDocument {
    pub(crate) fn selected_code_named_styles(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
    ) -> SelectedNamedStyles {
        let sheet = self.style_sheet();
        let mut result = SelectedNamedStyles {
            paragraph: Some(sheet.base_paragraph.clone()),
            character: None,
            paragraph_mixed: false,
            character_mixed: false,
        };
        let tree = self.text_tree();
        if range.is_empty() {
            let empty_line = tree
                .hard_line_at_byte(range.start)
                .ok()
                .is_some_and(|line| {
                    tree.hard_line_start(line).ok() == tree.hard_line_end(line).ok()
                });
            let at = if !empty_line
                && (affinity == BoundaryAffinity::Upstream || range.start == tree.byte_len())
            {
                range.start.saturating_sub(1)
            } else {
                range.start
            };
            result.character = if at == tree.byte_len() {
                None
            } else {
                self.style_spans_for_region(&(at..(at + 1).min(tree.byte_len())))
                    .into_iter()
                    .filter_map(|span| match span.application {
                        StyleApplication::Automatic(id) => Some(id),
                        _ => None,
                    })
                    .next_back()
            }
            .or_else(|| Some(sheet.base_character.clone()));
            return result;
        }

        // Code has one paragraph assignment. Sweep only cached automatic
        // assignments: never collect its potentially millions of hard lines
        // or blocks, and never request fresh syntax work from a menu query.
        let spans = self.style_spans_for_region(&range);
        let mut boundaries = BTreeSet::from([range.start, range.end]);
        let mut starts = BTreeMap::<usize, Vec<usize>>::new();
        let mut ends = BTreeMap::<usize, Vec<usize>>::new();
        for (index, span) in spans.iter().enumerate() {
            if !matches!(span.application, StyleApplication::Automatic(_)) {
                continue;
            }
            let lower = span.range.start.max(range.start);
            let upper = span.range.end.min(range.end);
            if lower < upper {
                boundaries.extend([lower, upper]);
                starts.entry(lower).or_default().push(index);
                ends.entry(upper).or_default().push(index);
            }
        }
        let boundaries = boundaries.into_iter().collect::<Vec<_>>();
        let mut active = BTreeMap::new();
        for interval in boundaries.windows(2) {
            let (start, end) = (interval[0], interval[1]);
            if let Some(indices) = ends.get(&start) {
                for index in indices {
                    active.remove(index);
                }
            }
            if let Some(indices) = starts.get(&start) {
                for index in indices {
                    if let StyleApplication::Automatic(id) = &spans[*index].application {
                        active.insert(*index, id);
                    }
                }
            }
            // Normalized hard breaks are one byte. Tree aggregates classify
            // newline-only gaps in O(log n), regardless of their line count.
            let newline_only = tree
                .hard_line_at_byte(start)
                .ok()
                .zip(tree.hard_line_at_byte(end).ok())
                .is_some_and(|(first, last)| last - first == end - start);
            if newline_only {
                continue;
            }
            let character = active
                .last_key_value()
                .map(|(_, id)| *id)
                .unwrap_or(&sheet.base_character);
            if result.character.as_ref().is_some_and(|id| id != character) {
                result.character = None;
                result.character_mixed = true;
                return result;
            }
            result.character = Some(character.clone());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        code_style,
        syntax::{SyntaxRun, SyntaxStyleName},
        Document, Encoding, Format,
    };
    use std::sync::Arc;

    fn document(text: &str, runs: &[(Range<usize>, &str)]) -> Document {
        let mut doc =
            Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let runs = runs
            .iter()
            .map(|(range, name)| SyntaxRun {
                range: range.clone(),
                name: SyntaxStyleName((*name).into()),
                origin: "test".into(),
                priority: 0,
            })
            .collect::<Vec<_>>();
        doc.install_code_presentation(Arc::new(code_style::default_sheet()), &runs);
        doc
    }

    #[test]
    fn code_carets_report_displayed_leaf_affinity_and_default_fallbacks() {
        let doc = document(
            "fn //doc\nx\n",
            &[
                (0..2, "@keyword.function"),
                (3..8, "@comment.documentation"),
                (9..10, "@undefined.capture"),
            ],
        );
        let projection = doc.projection();
        let query = |at, affinity| projection.selected_code_named_styles(at..at, affinity);
        assert_eq!(
            query(0, BoundaryAffinity::Downstream).character,
            Some("syntax:@keyword.function".into())
        );
        assert_eq!(
            query(2, BoundaryAffinity::Upstream).character,
            Some("syntax:@keyword.function".into())
        );
        assert_eq!(
            query(2, BoundaryAffinity::Downstream).character,
            Some(projection.style_sheet().base_character.clone())
        );
        assert_eq!(
            query(4, BoundaryAffinity::Downstream).character,
            Some("syntax:@comment.documentation".into())
        );
        for (at, affinity) in [
            (9, BoundaryAffinity::Downstream),
            (11, BoundaryAffinity::Upstream),
        ] {
            let selected = query(at, affinity);
            assert_eq!(
                selected.character,
                Some(projection.style_sheet().base_character.clone())
            );
            assert_eq!(
                selected.paragraph,
                Some(projection.style_sheet().base_paragraph.clone())
            );
            assert!(!selected.character_mixed && !selected.paragraph_mixed);
        }
        let multiline = document("/*\n\n*/", &[(0..6, "@comment")]);
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            assert_eq!(
                multiline
                    .projection()
                    .selected_code_named_styles(3..3, affinity)
                    .character,
                Some("syntax:@comment".into()),
                "An empty line inside a displayed multiline capture keeps that style"
            );
        }
    }

    #[test]
    fn code_ranges_distinguish_named_leaves_and_unstyled_text() {
        let doc = document(
            "aa\nbb cc",
            &[
                (0..2, "@comment"),
                (3..5, "@comment"),
                (6..8, "@comment.documentation"),
            ],
        );
        let query = |range| {
            doc.projection()
                .selected_code_named_styles(range, BoundaryAffinity::Downstream)
        };
        assert_eq!(query(0..5).character, Some("syntax:@comment".into()));
        for range in [0..6, 3..8] {
            assert!(query(range).character_mixed);
        }
        assert_eq!(query(2..3).character, None);
        assert!(!query(2..3).character_mixed);
        let empty = document("", &[]);
        assert_eq!(
            empty
                .projection()
                .selected_code_named_styles(0..0, BoundaryAffinity::Downstream)
                .character,
            Some(empty.projection().style_sheet().base_character.clone())
        );
    }

    #[test]
    fn code_large_selection_classifies_long_separator_gaps_with_tree_aggregates() {
        let text = format!("a{}b", "\n".repeat(20_000));
        let last = text.len() - 1;
        let doc = document(&text, &[(0..1, "@comment"), (last..last + 1, "@comment")]);
        let selected = doc
            .projection()
            .selected_code_named_styles(0..text.len(), BoundaryAffinity::Downstream);
        assert_eq!(selected.character, Some("syntax:@comment".into()));
        assert!(!selected.character_mixed && !selected.paragraph_mixed);
        let unstyled = document(&text, &[(0..1, "@comment")]);
        assert!(
            unstyled
                .projection()
                .selected_code_named_styles(0..text.len(), BoundaryAffinity::Downstream)
                .character_mixed
        );
    }
}
