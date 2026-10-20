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
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub editable: bool,
    title: String,
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
        /// Positive pixel sizes; absent axes use the natural image ratio.
        width: Option<u32>,
        height: Option<u32>,
    },
    Edit {
        range: Range<usize>,
        text: String,
        destination: String,
        width: Option<u32>,
        height: Option<u32>,
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
                title: image.title,
                width: image.width,
                height: image.height,
                editable: image.source.len() <= MAX_IMAGE_BYTES && (!self.is_read_only() && self.is_modifiable()),
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
        self.require_modifiable()?;
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
        if !self.is_modifiable() || self.is_read_only() || !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let (range, text, destination, width, height, remove, existing) = match intent {
            ImageEditIntent::Insert {
                range,
                text,
                destination,
                width,
                height,
            } => {
                if !self.can_insert_image(range.clone()) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                (range, text, destination, width, height, false, None)
            }
            ImageEditIntent::Edit {
                range,
                text,
                destination,
                width,
                height,
            } => {
                let image = self
                    .image_snapshot_at(self.text_point(range.start)?)?
                    .filter(|image| image.editable && image.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, text, destination, width, height, false, Some(image))
            }
            ImageEditIntent::Remove { range } => {
                let image = self
                    .image_snapshot_at(self.text_point(range.start)?)?
                    .filter(|image| image.editable && image.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, String::new(), String::new(), None, None, true, Some(image))
            }
        };
        if width == Some(0) || height == Some(0)
            || text.len() + destination.len() > MAX_IMAGE_BYTES
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
                .is_some_and(|image| image.text == text && image.destination == destination
                    && image.width == width && image.height == height)
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
                width,
                height,
            )?
        } else {
            let spelling = if remove {
                String::new()
            } else if width.is_some() || height.is_some() {
                let mut value = format!("<img src={} alt={}",
                    quoted_html_attribute(&destination), quoted_html_attribute(&text));
                if let Some(image) = existing.as_ref().filter(|image| !image.title.is_empty()) {
                    value.push_str(&format!(" title={}", quoted_html_attribute(&image.title)));
                }
                for (name, value_px) in [("width", width), ("height", height)] {
                    if let Some(value_px) = value_px { value.push_str(&format!(" {name}=\"{value_px}\"")); }
                }
                value.push('>');
                value
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
        let expected = if remove {
            String::new()
        } else {
            "\u{fffc}".into()
        };
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.clone(), expected.clone())],
            if remove
                && self.format().is_markdown()
                && existing.as_ref().is_some_and(|image| image.html)
            {
                // HTML whitespace may need local protection after removing an
                // object. Use the ordinary verified deletion repair path.
                None
            } else {
                Some(vec![SourcePatch::primary(source.clone(), replacement.clone())])
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
                || image.width != width
                || image.height != height
                || ((width.is_some() || height.is_some())
                    && existing.as_ref().is_some_and(|original| image.title != original.title))
                || image.source.start != prepared.summary().authored_source_start(&source, &replacement)
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

fn quoted_html_attribute(value: &str) -> String {
    format!("\"{}\"", value.replace('&', "&amp;")
        .replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('|', "&#124;").replace('\n', "&#10;").replace('\r', "&#13;"))
}

/// Change only the requested HTML attribute values. Unchanged dimensions,
/// titles, unknown attributes and surrounding whitespace survive.
fn edit_html_image(
    bytes: &[u8],
    encoding: Encoding,
    file_format: FileFormat,
    image: &ImageSnapshot,
    text: &str,
    destination: &str,
    width: Option<u32>,
    height: Option<u32>,
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
        let quoted = quoted_html_attribute(new);
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
    for (name, old, new) in [("width", image.width, width), ("height", image.height, height)] {
        if old == new { continue; }
        if let Some(value) = new {
            if let Some(range) = tag.attribute_range(name) {
                let (_, equals) = html_attribute_prefix(source, &range, name, &excluded)?;
                let value = format!("\"{value}\"");
                edits.push((range, if equals.is_some() { value } else { format!("={value}") }));
            } else {
                edits.push((4..4, format!(" {name}=\"{value}\"")));
            }
        } else {
            // Remove every duplicate so deleting the effective dimension cannot
            // expose a previously ignored declaration. Preserve line endings,
            // quote-owner prefixes, and spacing between the attribute tokens.
            for (_, range) in tag.attribute_ranges.iter().filter(|(key, _)| key == name) {
                let (name_range, equals) = html_attribute_prefix(source, range, name, &excluded)?;
                edits.push((name_range, String::new()));
                if let Some(at) = equals { edits.push((at..at + 1, String::new())); }
                if !range.is_empty() { edits.push((range.clone(), String::new())); }
            }
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

/// Attribute values retain their exact source coordinates. Walk only the
/// adjacent syntax to locate the name/equals without consuming container trivia.
fn html_attribute_prefix(
    source: &str,
    value: &Range<usize>,
    name: &str,
    excluded: &[Range<usize>],
) -> Result<(Range<usize>, Option<usize>), DocumentError> {
    let mut before = source[..value.start].char_indices().rev()
        .filter(|(at, ch)| !ch.is_ascii_whitespace() && !excluded.iter().any(|range| range.contains(at)));
    let last = before.next().ok_or(DocumentError::VerificationFailed)?;
    let (name_end, equals) = if last.1 == '=' {
        let (at, ch) = before.next().ok_or(DocumentError::VerificationFailed)?;
        (at + ch.len_utf8(), Some(last.0))
    } else { (last.0 + last.1.len_utf8(), None) };
    let start = name_end.checked_sub(name.len()).ok_or(DocumentError::VerificationFailed)?;
    if !source.get(start..name_end).is_some_and(|value| value.eq_ignore_ascii_case(name)) {
        return Err(DocumentError::VerificationFailed);
    }
    Ok((start..name_end, equals))
}

impl Document {
    /// Interoperable object representation for system clipboard plain text.
    /// Private clipboard payloads retain the atomic image and authored source.
    pub(crate) fn image_plain_text_for_range(
        &self,
        range: Range<usize>,
    ) -> Result<Option<String>, DocumentError> {
        if !self.format().is_markdown() {
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
