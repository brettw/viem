//! Passive Markdown images and verified source-local authoring. Native frontends
//! alone may decode local resources; this module performs no external I/O.
use super::*;

/// Snapshot-local image context. Source shows its whole authored construct;
/// WYSIWYG exposes one atomic U+FFFC object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageSnapshot {
    pub range: Range<usize>,
    pub text: String,
    pub destination: String,
    pub editable: bool,
    source: Range<usize>,
    inline: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageEditIntent {
    Insert {
        range: Range<usize>,
        text: String,
        destination: String,
    },
    Edit {
        range: Range<usize>,
        text: String,
        destination: String,
    },
    Remove {
        range: Range<usize>,
    },
}
const MAX_IMAGE_BYTES: usize = 32 * 1024;
impl Document {
    pub fn image_snapshot_at(
        &self,
        point: TextPoint,
    ) -> Result<Option<ImageSnapshot>, DocumentError> {
        if point.document() != self.id() {
            return Err(DocumentError::WrongDocument);
        }
        if point.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: point.revision(),
            });
        }
        self.text_point(point.offset())?;
        Ok(self
            .projection()
            .inline_images_for_region(&(point.offset()..point.offset().saturating_add(1)))
            .into_iter()
            .find(|image| image.range.contains(&point.offset()))
            .map(|image| ImageSnapshot {
                range: image.range,
                text: image.text,
                destination: image.destination,
                editable: image.source.len() <= MAX_IMAGE_BYTES && !self.is_read_only(),
                source: image.source,
                inline: image.inline,
            }))
    }
    pub fn can_insert_image(&self, range: Range<usize>) -> bool {
        self.can_insert_link(range.clone())
            && self
                .projection()
                .inline_images_for_region(&range)
                .iter()
                .all(|image| {
                    range.is_empty()
                        && (range.start == image.range.start || range.start == image.range.end)
                })
    }
    pub fn prepare_image_edit(
        &self,
        document: DocumentId,
        revision: Revision,
        intent: ImageEditIntent,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if document != self.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if revision != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: revision,
            }
            .into());
        }
        if self.is_read_only() || !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let (range, text, destination, remove, existing) = match intent {
            ImageEditIntent::Insert {
                range,
                text,
                destination,
            } => {
                if !self.can_insert_image(range.clone()) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                (range, text, destination, false, None)
            }
            ImageEditIntent::Edit {
                range,
                text,
                destination,
            } => {
                let image = self
                    .image_snapshot_at(self.text_point(range.start)?)?
                    .filter(|image| image.editable && image.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, text, destination, false, Some(image))
            }
            ImageEditIntent::Remove { range } => {
                let image = self
                    .image_snapshot_at(self.text_point(range.start)?)?
                    .filter(|image| image.editable && image.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, String::new(), String::new(), true, Some(image))
            }
        };
        if text.len() + destination.len() > MAX_IMAGE_BYTES
            || text.contains(['\n', '\r'])
            || destination.chars().any(char::is_control)
            || (!remove && destination.is_empty())
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if !remove
            && existing
                .as_ref()
                .is_some_and(|image| image.text == text && image.destination == destination)
        {
            return Ok((
                self.prepare_reprojected_source_patches(Vec::new())?,
                range.start,
            ));
        }
        let source = existing
            .as_ref()
            .map(|image| image.source.clone())
            .or_else(|| self.projection().source_range(range.clone()))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let spelling = if remove {
            String::new()
        } else if let Some(image) = existing.as_ref().filter(|image| image.inline) {
            let bytes = self
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            let decoded = self.encoding().decode_region(&bytes, 0)?;
            // Raw decoded offsets preserve existing title, whitespace and label syntax.
            let parsed = super::links::markdown_image_at(&decoded.text, 0, decoded.text.len())
                .ok_or(DocumentError::VerificationFailed)?;
            let destination_range = parsed
                .destination_range
                .ok_or(DocumentError::VerificationFailed)?;
            let mut value = decoded.text.clone();
            if image.destination != destination {
                value.replace_range(
                    destination_range,
                    &format!("<{}>", super::links::escape_destination(&destination)),
                );
            }
            if image.text != text {
                value.replace_range(parsed.label, &super::links::escape_label(&text));
            }
            value
        } else {
            format!(
                "![{}](<{}>)",
                super::links::escape_label(&text),
                super::links::escape_destination(&destination)
            )
        };
        let replacement = self.encoding().encode_fragment(&spelling)?;
        let expected = if self.format().is_source_view() {
            super::line_endings::normalize(
                &self.encoding().decode_region(&replacement, source.start)?,
                self.file_format(),
            )
            .text
        } else if remove {
            String::new()
        } else {
            "\u{fffc}".into()
        };
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.clone(), expected.clone())],
            Some(vec![SourcePatch::primary(source.clone(), replacement)]),
            true,
        )?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        if !remove {
            let image = candidate
                .image_snapshot_at(candidate.text_point(range.start)?)?
                .ok_or(DocumentError::VerificationFailed)?;
            if image.text != text
                || image.destination != destination
                || image.source.start != source.start
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        // A newly authored image remains the active object for the popup.
        let caret = range.start;
        self.prepared_text_point(&prepared, caret)?;
        Ok((prepared, caret))
    }
}

impl Document {
    /// Interoperable object representation for system clipboard plain text.
    /// Private clipboard payloads retain the atomic image and authored source.
    pub(crate) fn image_plain_text_for_range(
        &self,
        range: Range<usize>,
    ) -> Result<Option<String>, DocumentError> {
        if !self.format().is_wysiwyg() {
            return Ok(None);
        }
        let images = self.projection().inline_images_for_region(&range);
        if images.is_empty() {
            return Ok(None);
        }
        let mut text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        for image in images.iter().rev() {
            if range.start > image.range.start || image.range.end > range.end {
                return Err(DocumentError::AmbiguousProjection);
            }
            text.replace_range(
                image.range.start - range.start..image.range.end - range.start,
                &format!(
                    "![{}](<{}>)",
                    super::links::escape_label(&image.text),
                    super::links::escape_destination(&image.destination)
                ),
            );
        }
        Ok(Some(text))
    }
}
