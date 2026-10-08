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
    html: bool,
    quote_depth: usize,
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
                quote_depth: self
                    .projection()
                    .blocks_for_region(&(image.range.start..image.range.start.saturating_add(1)))
                    .first()
                    .map_or(0, |block| block.quote_depth),
                range: image.range,
                text: image.text,
                destination: image.destination,
                editable: image.source.len() <= MAX_IMAGE_BYTES && !self.is_read_only(),
                source: image.source,
                inline: image.inline,
                html: image.html,
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
            || (text.contains(['\n', '\r'])
                && !existing.as_ref().is_some_and(|image| image.html && image.text == text))
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
        let replacement = if let Some(image) =
            existing.as_ref().filter(|image| image.html && !remove)
        {
            let bytes = self
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::VerificationFailed)?;
            edit_html_image(
                &bytes,
                self.encoding(),
                self.file_format(),
                image,
                &text,
                &destination,
            )?
        } else {
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
            self.encoding().encode_fragment(&spelling)?
        };
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
            if remove
                && self.format().is_wysiwyg()
                && existing.as_ref().is_some_and(|image| image.html)
            {
                // HTML whitespace may need local protection after removing an
                // object. Use the ordinary verified deletion repair path.
                None
            } else {
                Some(vec![SourcePatch::primary(source.clone(), replacement)])
            },
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

/// Change only the requested HTML attribute values. In particular, authored
/// dimensions, titles, unknown attributes and surrounding whitespace survive.
fn edit_html_image(
    bytes: &[u8],
    encoding: Encoding,
    file_format: FileFormat,
    image: &ImageSnapshot,
    text: &str,
    destination: &str,
) -> Result<Vec<u8>, DocumentError> {
    let decoded = encoding.decode_region(bytes, 0)?;
    let source = &decoded.text;
    let mut excluded = Vec::new();
    let separator = if file_format == FileFormat::Mac {
        '\r'
    } else {
        '\n'
    };
    let mut at = 0;
    for line in source.split_inclusive(separator) {
        if at > 0 && image.quote_depth > 0 {
            let mut prefix = 0;
            for _ in 0..image.quote_depth {
                let tail = &line[prefix..];
                if super::markdown_quotes::prefix(tail) == 0 {
                    break;
                }
                prefix += tail.find('>').ok_or(DocumentError::VerificationFailed)? + 1;
                if line
                    .as_bytes()
                    .get(prefix)
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
                {
                    prefix += 1;
                }
            }
            if prefix > 0 {
                excluded.push(at..at + prefix);
            }
        }
        at += line.len();
    }
    let tokens = super::html::tokenize_without_ranges(source, &excluded);
    let Some(super::html::Token {
        kind: super::html::TokenKind::Tag(tag),
        range,
    }) = tokens.first()
    else {
        return Err(DocumentError::VerificationFailed);
    };
    if tag.name != "img" || tag.end || range != &(0..source.len()) {
        return Err(DocumentError::VerificationFailed);
    }
    let mut edits = Vec::new();
    for (name, old, new) in [
        ("src", image.destination.as_str(), destination),
        ("alt", image.text.as_str(), text),
    ] {
        if old == new {
            continue;
        }
        let quoted = format!(
            "\"{}\"",
            new.replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        );
        if let Some(range) = tag.attribute_range(name) {
            let has_equals = !range.is_empty()
                || source[..range.start]
                    .char_indices()
                    .rev()
                    .find(|(at, ch)| {
                        !ch.is_whitespace() && !excluded.iter().any(|range| range.contains(at))
                    })
                    .is_some_and(|(_, ch)| ch == '=');
            edits.push((
                range,
                if has_equals {
                    quoted
                } else {
                    format!("={quoted}")
                },
            ));
        } else {
            // Inserting after the tag name avoids changing self-closing syntax
            // or unquoted values at the end of a tag.
            edits.push((4..4, format!(" {name}={quoted}")));
        }
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = bytes.to_owned();
    for (range, value) in edits {
        let start = decoded
            .source_boundary(range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let end = decoded
            .source_boundary(range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        result.splice(start..end, encoding.encode_fragment(&value)?);
    }
    Ok(result)
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
