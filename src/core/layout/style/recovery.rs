//! A damaged appearance must not make otherwise valid text undisplayable.
use super::*;
use crate::document::{BlockKind, ContainerKind, Format};
use std::sync::Arc;

impl DocumentLayoutStyles {
    pub(super) fn resolve_for_presentation(
        input: StyleCascadeInput<'_>,
        format: Format,
    ) -> Result<Self, DocumentStyleError> {
        let error = match Self::resolve_validated(input) {
            Ok(styles) => return Ok(styles),
            Err(
                error @ (DocumentStyleError::Cascade(
                    StyleError::UnknownStyle(_)
                    | StyleError::MissingParent(_)
                    | StyleError::InheritanceCycle(_)
                    | StyleError::InvalidBaseStyleDefinition(_)
                    | StyleError::IncompatibleBlockRole { .. }
                    | StyleError::InapplicableBlockProperties { .. }
                    | StyleError::InvalidCharacterProperties(_)
                    | StyleError::InvalidBlockProperties(_),
                )
                | DocumentStyleError::MultipleNamedCharacterStyles { .. }
                | DocumentStyleError::InvalidOpenTypeFeatureTag(_)),
            ) => error,
            Err(error) => return Err(error),
        };
        // Ranges were validated by the capture boundary. Keep their exact
        // identities and structural owners; discard only presentation layers.
        let sheet = StyleSheet::for_format(format);
        let assignment = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
        let blocks: Vec<_> = input
            .blocks
            .iter()
            .cloned()
            .map(|mut block| {
                let attributes = Arc::make_mut(&mut block.attributes);
                attributes.direct_formatting = None;
                attributes.style = if sheet.block_style(&attributes.style).is_some() {
                    attributes.style.clone()
                } else {
                    match attributes.kind {
                        BlockKind::Heading(level) => {
                            StyleId(format!("Heading{}", level.clamp(1, 6)))
                        }
                        BlockKind::ListItem { ordered, level, .. } => {
                            sheet.list_style_id(ordered, level)
                        }
                        BlockKind::Paragraph => sheet.base_paragraph.clone(),
                    }
                };
                attributes.containers = attributes
                    .containers
                    .iter()
                    .cloned()
                    .map(|mut member| {
                        let container = Arc::make_mut(&mut member.container);
                        container.direct_formatting = None;
                        container.style = match container.kind {
                            ContainerKind::Quote => "Block quote",
                            ContainerKind::CodeBlock => "Code Block",
                            ContainerKind::List { ordered: false } => "Bulleted List",
                            ContainerKind::List { ordered: true } => "Numbered List",
                            ContainerKind::ListItem => "List item",
                        }
                        .into();
                        member
                    })
                    .collect();
                block
            })
            .collect();
        let spans: Vec<_> = input
            .style_spans
            .iter()
            .filter(|span| match &span.application {
                StyleApplication::Semantic(_) => true,
                StyleApplication::Automatic(id) => sheet.character_style(id).is_some(),
                StyleApplication::Named(_) | StyleApplication::Direct(_) => false,
            })
            .cloned()
            .collect();
        let mut styles = Self::resolve_validated(StyleCascadeInput {
            blocks: &blocks,
            style_spans: &spans,
            inline_images: input.inline_images,
            style_sheet: &sheet,
            document_style: &assignment,
            search_matches: &[],
        })?;
        // Recovery is still an exact layout of the current source/style
        // snapshot, not an installed replacement stylesheet.
        styles.style_sheet_revision = input.style_sheet.revision;
        styles.recovery_diagnostic = Some(super::super::ShapingDiagnostic {
            text_range: input.blocks.first().map_or(0, |block| block.range.start)
                ..input.blocks.last().map_or(0, |block| block.range.end),
            message: format!(
                "Could not resolve text appearance ({error:?}); using default styles."
            )
            .chars()
            .take(1024)
            .collect(),
        });
        Ok(styles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding};

    fn damaged_projection(document: &Document) -> FormattedDocument {
        let mut projection = document.projection().clone();
        let mut sheet = projection.style_sheet().clone();
        // Model the theme regression: source syntax still references Link,
        // while a damaged appearance sheet no longer defines it.
        sheet.remove_character_style(&"Link".into(), false).unwrap();
        let assignment = projection.document_style().clone();
        projection.install_configuration_styles(projection.revision(), sheet, assignment);
        projection
    }

    #[test]
    fn missing_automatic_style_recovers_structure_without_changing_source_or_styles() {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let source =
                b"# Heading\n\n> - [link](https://example.com) with **bold**\n\n```rs\ncode\n```\n"
                    .to_vec();
            let document = Document::from_bytes(source.clone(), Encoding::Utf8, format).unwrap();
            let projection = damaged_projection(&document);
            assert!(DocumentLayoutStyles::resolve(&projection).is_err());
            let region = 0..projection.text_tree().byte_len();
            let fallback = DocumentLayoutStyles::resolve_region_for_presentation(
                &projection,
                region.clone(),
                false,
                &[],
                format,
            )
            .unwrap();
            let expected = DocumentLayoutStyles::resolve_region_with_search(
                document.projection(),
                region,
                false,
                &[],
            )
            .unwrap();
            assert!(fallback
                .recovery_diagnostic
                .as_ref()
                .unwrap()
                .message
                .contains("UnknownStyle"));
            assert_eq!(fallback.paragraphs, expected.paragraphs);
            assert_eq!(fallback.shaping_runs, expected.shaping_runs);
            assert_eq!(fallback.paint_runs, expected.paint_runs);
            assert_eq!(
                fallback.style_sheet_revision,
                projection.style_sheet().revision
            );
            assert!(projection
                .style_sheet()
                .character_style(&"Link".into())
                .is_none());
            assert_eq!(document.source_bytes(), source);
            assert!(!document.is_dirty());
        }
    }

    #[test]
    fn regional_recovery_does_not_resolve_unrelated_large_document_content() {
        let source = "[link](https://example.com)\n\n".repeat(20_000);
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let projection = damaged_projection(&document);
        let fallback = DocumentLayoutStyles::resolve_region_for_presentation(
            &projection,
            0..4,
            false,
            &[],
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(fallback.paragraphs.len(), 1);
        assert_eq!(fallback.recovery_diagnostic.unwrap().text_range, 0..4);
        let corrected = DocumentLayoutStyles::resolve_region_for_presentation(
            document.projection(),
            0..4,
            false,
            &[],
            Format::Markdown,
        )
        .unwrap();
        assert!(corrected.recovery_diagnostic.is_none());
    }

    #[test]
    fn presentation_recovery_rejects_invalid_text_coordinates() {
        let document = Document::from_bytes(
            "[é](url)".as_bytes().to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let projection = damaged_projection(&document);
        for region in [1..2, 0..99] {
            assert!(DocumentLayoutStyles::resolve_region_for_presentation(
                &projection,
                region,
                false,
                &[],
                Format::Markdown
            )
            .is_err());
        }
    }
}
