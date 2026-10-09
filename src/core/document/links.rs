//! Link recognition and on-demand destinations. This is an original parser for
//! Markdown inline links; it shares Viem's passive HTML tokenizer and entities.
use super::html::TokenKind;
use super::line_endings::NormalizedText;
use super::*;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LinkHeading {
    pub text: String,
    pub destination: String,
    pub level: u8,
    pub offset: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LinkHeadingList {
    pub headings: Vec<LinkHeading>,
    pub truncated: bool,
}

#[derive(Debug)]
pub(super) struct InlineLink {
    pub range: Range<usize>,
    pub label: Range<usize>,
    pub destination: String,
    pub destination_range: Option<Range<usize>>,
}

fn escaped(bytes: &[u8], at: usize) -> bool {
    bytes[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}
fn decode_destination(value: &str) -> String {
    let mut result = String::new();
    let mut at = 0;
    while at < value.len() {
        let bytes = value.as_bytes();
        if bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            result.push(bytes[at + 1] as char);
            at += 2;
        } else if bytes[at] == b'&' {
            if let Some((text, count)) = super::html::reference(&value[at..], false) {
                result.push_str(&text);
                at += count;
            } else {
                result.push('&');
                at += 1;
            }
        } else {
            let ch = value[at..].chars().next().unwrap();
            result.push(ch);
            at += ch.len_utf8();
        }
    }
    result
}

fn label_contains_link(text: &str, start: usize, end: usize) -> bool {
    let bytes = text.as_bytes();
    let mut at = start;
    while at < end {
        match bytes[at] {
            b'\\' if at + 1 < end && bytes[at + 1].is_ascii_punctuation() => at += 2,
            b'`' => {
                let count = bytes[at..end].iter().take_while(|&&b| b == b'`').count();
                let mut scan = at + count;
                let mut close = None;
                while scan < end {
                    if bytes[scan] == b'`' {
                        let run = bytes[scan..end].iter().take_while(|&&b| b == b'`').count();
                        if run == count {
                            close = Some(scan + run);
                            break;
                        }
                        scan += run;
                    } else {
                        scan += 1;
                    }
                }
                at = close.unwrap_or(at + count);
            }
            b'[' if markdown_inline_at(text, at, end).is_some() => return true,
            _ => at += 1,
        }
    }
    false
}

pub(super) fn markdown_inline_at(text: &str, start: usize, end: usize) -> Option<InlineLink> {
    markdown_bracket_at(text, start, end, false)
}
pub(super) fn markdown_image_at(text: &str, start: usize, end: usize) -> Option<InlineLink> {
    if text.as_bytes().get(start) != Some(&b'!') || escaped(text.as_bytes(), start) {
        return None;
    }
    let mut image = markdown_bracket_at(text, start + 1, end, true)?;
    image.range.start = start;
    Some(image)
}
fn markdown_bracket_at(text: &str, start: usize, end: usize, image: bool) -> Option<InlineLink> {
    let bytes = text.as_bytes();
    if end > bytes.len()
        || bytes.get(start) != Some(&b'[')
        || escaped(bytes, start)
        || (!image && start > 0 && bytes[start - 1] == b'!' && !escaped(bytes, start - 1))
    {
        return None;
    }
    let mut at = start + 1;
    let mut depth = 1;
    let label_end = loop {
        if at >= end {
            return None;
        }
        match bytes[at] {
            b'\\' if at + 1 < end && bytes[at + 1].is_ascii_punctuation() => {
                at += 2;
                continue;
            }
            b'`' => {
                let count = bytes[at..end].iter().take_while(|&&b| b == b'`').count();
                let mut scan = at + count;
                let mut close = None;
                while scan < end {
                    if bytes[scan] == b'`' {
                        let n = bytes[scan..end].iter().take_while(|&&b| b == b'`').count();
                        if n == count {
                            close = Some(scan + n);
                            break;
                        }
                        scan += n;
                    } else {
                        scan += 1;
                    }
                }
                at = close.unwrap_or(at + count);
                continue;
            }
            b'[' => {
                depth += 1;
                if depth > 64 {
                    return None;
                }
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break at;
                }
            }
            _ => {}
        }
        at += 1;
    };
    at = label_end + 1;
    if bytes.get(at) != Some(&b'(') || at >= end {
        return None;
    }
    at += 1;
    while at < end && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    let destination_begin = at;
    let destination;
    if bytes.get(at) == Some(&b'<') {
        at += 1;
        let begin = at;
        while at < end && bytes[at] != b'>' {
            if matches!(bytes[at], b'<' | b'\n' | b'\r') {
                return None;
            }
            if bytes[at] == b'\\' && at + 1 < end && bytes[at + 1].is_ascii_punctuation() {
                at += 1;
            }
            at += 1;
        }
        if at == end {
            return None;
        }
        destination = decode_destination(&text[begin..at]);
        at += 1;
    } else {
        let begin = at;
        let mut nesting = 0;
        while at < end {
            match bytes[at] {
                b'\\' if at + 1 < end && bytes[at + 1].is_ascii_punctuation() => {
                    at += 2;
                    continue;
                }
                b'(' => {
                    nesting += 1;
                    if nesting > 64 {
                        return None;
                    }
                }
                b')' if nesting == 0 => break,
                b')' => nesting -= 1,
                b if b.is_ascii_whitespace() || b.is_ascii_control() => break,
                _ => {}
            }
            at += 1;
        }
        if nesting != 0 {
            return None;
        }
        destination = decode_destination(&text[begin..at]);
    }
    let destination_end = at;
    let before_space = at;
    while at < end && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    if at > before_space && at < end && matches!(bytes[at], b'\'' | b'"' | b'(') {
        let close = if bytes[at] == b'(' { b')' } else { bytes[at] };
        at += 1;
        while at < end && bytes[at] != close {
            if bytes[at] == b'\\' && at + 1 < end && bytes[at + 1].is_ascii_punctuation() {
                at += 1;
            }
            at += 1;
        }
        if at == end {
            return None;
        }
        at += 1;
        while at < end && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
    }
    if at >= end || bytes[at] != b')' {
        return None;
    }
    // Inline constructs cannot cross a paragraph break. Projection calls are
    // paragraph-bounded, but an on-demand source lookup may inspect more text:
    // an unmatched '[' in an earlier paragraph must not capture this link.
    if text[start..at + 1]
        .split('\n')
        .skip(1)
        .any(|line| line.trim_matches([' ', '\t', '\r']).is_empty())
    {
        return None;
    }
    // A valid inner link makes the surrounding link label ineligible. The
    // bracket-depth limit above also bounds this recursive recognition.
    if !image && label_contains_link(text, start + 1, label_end) {
        return None;
    }
    Some(InlineLink {
        range: start..at + 1,
        label: start + 1..label_end,
        destination,
        destination_range: Some(destination_begin..destination_end),
    })
}

pub(super) fn html_links(text: &str) -> Vec<InlineLink> {
    html_links_from_tokens(text.len(), &super::html::tokenize(text))
}

fn html_links_from_tokens(text_len: usize, tokens: &[super::html::Token]) -> Vec<InlineLink> {
    let mut result = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for token in tokens {
        if let TokenKind::Tag(tag) = &token.kind {
            if tag.name != "a" {
                continue;
            }
            // A nested anchor implicitly closes the preceding anchor, matching
            // HTML recovery without applying the outer destination to the inner.
            if let Some((start, destination)) = open.take() {
                if start < token.range.start {
                    result.push(InlineLink {
                        range: start..token.range.start,
                        label: start..token.range.start,
                        destination,
                        destination_range: None,
                    });
                }
            }
            if !tag.end {
                open = tag
                    .attribute("href")
                    .map(|href| (token.range.end, href.to_owned()));
            }
        }
    }
    if let Some((start, destination)) = open {
        if start < text_len {
            result.push(InlineLink {
                range: start..text_len,
                label: start..text_len,
                destination,
                destination_range: None,
            });
        }
    }
    result
}

pub(super) fn style_html_links(
    document: &mut FormattedDocument,
    input: &NormalizedText,
    tokens: &[super::html::Token],
) {
    let ranges = html_links_from_tokens(input.text.len(), tokens)
        .into_iter()
        .filter_map(|link| {
            let begin = input
                .units
                .partition_point(|u| u.normalized.end <= link.range.start);
            let end = input
                .units
                .partition_point(|u| u.normalized.start < link.range.end);
            (begin < end).then(|| input.units[begin].source.start..input.units[end - 1].source.end)
        })
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        return;
    }
    let mut spans: Vec<StyleSpan> = Vec::new();
    for provenance in document.provenance() {
        let i = ranges.partition_point(|r| r.end <= provenance.source.start);
        if ranges
            .get(i)
            .is_some_and(|r| r.start <= provenance.source.start && provenance.source.end <= r.end)
            && !provenance.formatted.is_empty()
            && !provenance.source.is_empty()
        {
            if let Some(previous) = spans
                .last_mut()
                .filter(|s| s.range.end == provenance.formatted.start)
            {
                previous.range.end = provenance.formatted.end;
            } else {
                spans.push(StyleSpan {
                    range: provenance.formatted.clone(),
                    application: StyleApplication::Automatic("Link".into()),
                });
            }
        }
    }
    document.append_link_styles(spans);
}

/// Hidden inline delimiters have no visible interior. A destination query can
/// use their single projected boundary, but never choose between two different
/// neighbors. The resulting extent is checked against an authored Link span.
fn link_boundary(document: &Document, source: usize, affinity: BoundaryAffinity) -> Option<usize> {
    match document
        .projection()
        .map_source_boundary(document.revision(), source, affinity)
    {
        Ok(point) => Some(point.formatted_offset),
        Err(SourceToTextError::InteriorHiddenSyntax {
            upstream_formatted,
            downstream_formatted,
            ..
        }) => match (upstream_formatted, downstream_formatted) {
            (Some(left), Some(right)) if left == right => Some(left),
            (Some(point), None) | (None, Some(point)) => Some(point),
            _ => None,
        },
        _ => None,
    }
}

/// A link resolved from one exact document projection. `range` covers the
/// complete inline construct in Source and its visible label in WYSIWYG.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkSnapshot {
    pub range: Range<usize>,
    pub text: String,
    pub destination: String,
    pub editable: bool,
    source: Range<usize>,
    label_source: Range<usize>,
    pub(super) destination_source: Option<Range<usize>>,
}

const MAX_LINK_QUERY_BYTES: usize = 64 * 1024;

fn plain_label(source: &str) -> String {
    use pulldown_cmark::Event;
    let mut result = String::new();
    for event in
        pulldown_cmark::Parser::new_ext(source, pulldown_cmark::Options::ENABLE_STRIKETHROUGH)
    {
        match event {
            Event::Text(text) | Event::Code(text) => result.push_str(&text),
            Event::SoftBreak | Event::HardBreak => result.push(' '),
            _ => {}
        }
    }
    result
}

impl Document {
    pub(crate) fn link_typing_end(&self, link: &LinkSnapshot) -> Result<usize, DocumentError> {
        if self.format().is_source_view() {
            link_boundary(self, link.label_source.end, BoundaryAffinity::Upstream)
                .ok_or(DocumentError::AmbiguousProjection)
        } else {
            Ok(link.range.end)
        }
    }
    pub(crate) fn prepared_link_typing_caret(
        &self,
        prepared: &PreparedModelTransaction,
        caret: usize,
    ) -> Result<Option<usize>, DocumentError> {
        let candidate = self.prepared_candidate_document(prepared)?;
        candidate
            .link_at_boundary(caret, BoundaryAffinity::Upstream)?
            .map(|link| candidate.link_typing_end(&link))
            .transpose()
    }
    /// Bounded, source-local lookup. The projection's indexed Link interval is
    /// authoritative; unrelated source is neither copied nor decoded.
    pub fn link_snapshot_at(
        &self,
        point: TextPoint,
    ) -> Result<Option<LinkSnapshot>, DocumentError> {
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
        if !self.format().is_markdown() {
            return Ok(None);
        }
        let spans = self
            .projection()
            .style_spans_for_region(&(point.offset()..point.offset().saturating_add(1)));
        let Some(span) = spans.iter().find(|span| {
            span.range.contains(&point.offset())
                && span.application == StyleApplication::Automatic("Link".into())
        }) else {
            return Ok(None);
        };
        let Some(first) = self
            .projection()
            .source_insertion_point(span.range.start, false)
        else {
            return Ok(None);
        };
        let Some(last) = self
            .projection()
            .source_insertion_point(span.range.end, true)
        else {
            return Ok(None);
        };
        if last.saturating_sub(first) > MAX_LINK_QUERY_BYTES - 2048 {
            return Ok(None);
        }
        let mut start = first.saturating_sub(1024);
        let mut end = last.saturating_add(1024).min(self.source_byte_len());
        if matches!(self.encoding(), Encoding::Utf16Le | Encoding::Utf16Be) {
            start -= start % 2;
            end -= end % 2;
        }
        let bytes = self
            .state()
            .source
            .bytes_in(start..end)
            .ok_or(DocumentError::VerificationFailed)?;
        let decoded = self.encoding().decode_region(&bytes, start)?;
        let input = super::line_endings::normalize(&decoded, self.file_format());
        let source_range = |range: &Range<usize>| -> Option<Range<usize>> {
            let begin = input
                .units
                .partition_point(|unit| unit.normalized.end <= range.start);
            let end = input
                .units
                .partition_point(|unit| unit.normalized.start < range.end);
            if begin < end {
                Some(input.units[begin].source.start..input.units[end - 1].source.end)
            } else if range.is_empty() {
                input
                    .units
                    .get(begin)
                    .map(|unit| unit.source.start..unit.source.start)
                    .or_else(|| {
                        input
                            .units
                            .last()
                            .map(|unit| unit.source.end..unit.source.end)
                    })
            } else {
                None
            }
        };
        for (at, _) in input.text.match_indices('[') {
            let Some(link) = markdown_inline_at(&input.text, at, input.text.len()) else {
                continue;
            };
            let displayed = if self.format().is_source_view() {
                &link.range
            } else {
                &link.label
            };
            let Some(physical) = source_range(displayed) else {
                continue;
            };
            if link_boundary(self, physical.start, BoundaryAffinity::Downstream)
                != Some(span.range.start)
                || link_boundary(self, physical.end, BoundaryAffinity::Upstream)
                    != Some(span.range.end)
            {
                continue;
            }
            let Some(source) = source_range(&link.range) else {
                continue;
            };
            let Some(label_source) = source_range(&link.label) else {
                continue;
            };
            let text = if self.format().is_source_view() {
                plain_label(&input.text[link.range.clone()])
            } else {
                self.projection()
                    .text_tree()
                    .slice(span.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?
            };
            return Ok(Some(LinkSnapshot {
                range: span.range.clone(),
                text,
                destination: link.destination,
                editable: true,
                source,
                label_source,
                destination_source: link.destination_range.as_ref().and_then(source_range),
            }));
        }
        // Passive HTML and autolinks remain navigable. Inline Markdown editing
        // requires explicit delimiters, so these retain a read-only popup.
        let Some(raw) = self
            .projection()
            .source_insertion_point(point.offset(), true)
        else {
            return Ok(None);
        };
        let Some(unit) = input
            .units
            .get(input.units.partition_point(|unit| unit.source.end <= raw))
        else {
            return Ok(None);
        };
        let at = unit.normalized.start;
        let mut destination = html_links(&input.text)
            .into_iter()
            .find(|link| link.range.contains(&at))
            .map(|link| link.destination);
        let mut autolink_range = None;
        if destination.is_none() {
            for (event, range) in pulldown_cmark::Parser::new(&input.text).into_offset_iter() {
                if !range.contains(&at) {
                    continue;
                }
                if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                    link_type,
                    dest_url,
                    ..
                }) = event
                {
                    destination = match link_type {
                        pulldown_cmark::LinkType::Email => Some(format!("mailto:{dest_url}")),
                        pulldown_cmark::LinkType::Autolink => Some(dest_url.into_string()),
                        _ => None,
                    };
                    if destination.is_some() {
                        autolink_range = source_range(&range);
                        break;
                    }
                }
            }
        }
        if destination.is_none() {
            for (start, _) in input
                .text
                .char_indices()
                .take_while(|(start, _)| *start <= at)
            {
                if let Some((end, value)) =
                    super::markdown_syntax::autolink(&input.text, start, input.text.len())
                {
                    if start <= at && at < end {
                        destination = Some(value);
                        autolink_range = source_range(&(start..end));
                        break;
                    }
                }
            }
        }
        Ok(destination.map(|destination| LinkSnapshot {
            range: span.range.clone(),
            text: if autolink_range.is_some() {
                plain_label(
                    &self
                        .projection()
                        .text_tree()
                        .slice(span.range.clone())
                        .unwrap_or_default(),
                )
            } else {
                self.projection()
                    .text_tree()
                    .slice(span.range.clone())
                    .unwrap_or_default()
            },
            destination,
            editable: autolink_range.is_some(),
            source: autolink_range.clone().unwrap_or(first..last),
            label_source: autolink_range.unwrap_or(first..last),
            destination_source: None,
        }))
    }

    pub fn link_at(&self, point: TextPoint) -> Result<Option<String>, DocumentError> {
        Ok(self.link_snapshot_at(point)?.map(|link| link.destination))
    }
}

/// Scan one paragraph in source order, respecting the tighter binding of code.
/// Used when a source view's physical-line parser cannot see a whole link.
pub(super) fn markdown_links_in(text: &str, range: Range<usize>) -> Vec<InlineLink> {
    let bytes = text.as_bytes();
    let mut at = range.start;
    let mut links = Vec::new();
    while at < range.end {
        match bytes[at] {
            b'\\' if at + 1 < range.end && bytes[at + 1].is_ascii_punctuation() => at += 2,
            b'`' => {
                let count = bytes[at..range.end]
                    .iter()
                    .take_while(|&&b| b == b'`')
                    .count();
                let mut scan = at + count;
                let mut closing = None;
                while scan < range.end {
                    if bytes[scan] == b'`' {
                        let run = bytes[scan..range.end]
                            .iter()
                            .take_while(|&&b| b == b'`')
                            .count();
                        if run == count {
                            closing = Some(scan + run);
                            break;
                        }
                        scan += run;
                    } else {
                        scan += 1;
                    }
                }
                at = closing.unwrap_or(at + count);
            }
            b'[' => {
                if let Some(link) = markdown_inline_at(text, at, range.end) {
                    at = link.range.end;
                    links.push(link);
                } else {
                    at += 1;
                }
            }
            _ => at += 1,
        }
    }
    links
}

/// Native link authoring is a document intention, independent of Vim grammar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkEditIntent {
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
    RemoveSelection {
        range: Range<usize>,
    },
}

pub(super) fn escape_label(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
pub(super) fn escape_destination(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "&#92;")
}

impl Document {
    pub(crate) fn link_at_boundary(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
    ) -> Result<Option<LinkSnapshot>, DocumentError> {
        let sample = if affinity == BoundaryAffinity::Upstream && at > 0 {
            self.hard_line_snapshot()
                .previous_grapheme_boundary(at)
                .unwrap_or(at)
        } else {
            at
        };
        self.link_snapshot_at(self.text_point(sample)?)
    }

    pub(crate) fn can_exit_link_typing(&self, at: usize, affinity: BoundaryAffinity) -> bool {
        if self.is_read_only() || !self.format().is_markdown() {
            return false;
        }
        let Ok(Some(link)) = self.link_at_boundary(at, affinity) else {
            return false;
        };
        link.editable
            && (link.destination_source.is_none()
                || !self.format().is_source_view()
                || self
                    .projection()
                    .source_insertion_point(at, true)
                    .is_some_and(|source| {
                        link.label_source.start <= source && source <= link.label_source.end
                    }))
    }

    pub(crate) fn can_remove_link_selection(&self, range: Range<usize>) -> bool {
        if range.is_empty() || range.len() > MAX_LINK_QUERY_BYTES / 2 {
            return false;
        }
        if self.is_read_only() || !self.format().is_markdown() {
            return false;
        }
        let Ok(Some(link)) = self.link_snapshot_at(match self.text_point(range.start) {
            Ok(point) => point,
            Err(_) => return false,
        }) else {
            return false;
        };
        if !link.editable || range.end > link.range.end {
            return false;
        }
        range == link.range
            || link.destination_source.is_none()
            || (!self.format().is_source_view()
                || self.projection().source_range(range).is_some_and(|source| {
                    link.label_source.start <= source.start && source.end <= link.label_source.end
                }))
    }

    /// Convert only an editable automatic-link construct to explicit Markdown
    /// before splitting its label. This is a speculative mutation, never a
    /// toolbar capability query; passive HTML anchors remain unavailable.
    pub(crate) fn prepare_canonical_autolink(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
    ) -> Result<(PreparedModelTransaction, Range<usize>), ModelTransactionError> {
        let link = self
            .link_at_boundary(range.start, affinity)?
            .filter(|link| {
                link.editable && link.destination_source.is_none() && range.end <= link.range.end
            })
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let source = self
            .state()
            .source
            .bytes_in(link.source.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let original = Document::from_bytes_with_file_format(
            source,
            self.encoding(),
            Format::Markdown,
            self.file_format(),
        )?;
        let replacement = self.encoding().encode_fragment(&format!(
            "[{}](<{}>)",
            escape_label(&link.text),
            escape_destination(&link.destination)
        ))?;
        let explicit = Document::from_bytes_with_file_format(
            replacement.clone(),
            self.encoding(),
            Format::Markdown,
            self.file_format(),
        )?;
        if original.text() != explicit.text() {
            return Err(DocumentError::VerificationFailed.into());
        }
        let relative = |at: usize, boundary: BoundaryAffinity| -> Result<usize, DocumentError> {
            let source = self
                .projection()
                .source_insertion_point(at, boundary == BoundaryAffinity::Downstream)
                .ok_or(DocumentError::AmbiguousProjection)?;
            link_boundary(
                &original,
                source
                    .checked_sub(link.source.start)
                    .ok_or(DocumentError::AmbiguousProjection)?,
                boundary,
            )
            .ok_or(DocumentError::AmbiguousProjection)
        };
        let logical = if self.format().is_source_view() {
            relative(range.start, affinity)?
                ..relative(
                    range.end,
                    if range.is_empty() {
                        affinity
                    } else {
                        BoundaryAffinity::Upstream
                    },
                )?
        } else {
            range.start - link.range.start..range.end - link.range.start
        };
        let patch = SourcePatch::primary(link.source.clone(), replacement);
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(vec![patch])?
        } else {
            self.prepare_text_edits_with_patch_policy(Vec::new(), Some(vec![patch]), true)?
        };
        let projected = if self.format().is_source_view() {
            let candidate = self.prepared_candidate_document(&prepared)?;
            let map = |at: usize, downstream: bool| -> Result<usize, DocumentError> {
                let source = explicit
                    .projection()
                    .source_insertion_point(at, downstream)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                link_boundary(
                    &candidate,
                    link.source.start + source,
                    if downstream {
                        BoundaryAffinity::Downstream
                    } else {
                        BoundaryAffinity::Upstream
                    },
                )
                .ok_or(DocumentError::AmbiguousProjection)
            };
            map(logical.start, true)?..map(logical.end, false)?
        } else {
            range
        };
        Ok((prepared, projected))
    }

    fn link_label_document(&self, link: &LinkSnapshot) -> Result<Document, DocumentError> {
        let mut bytes = self.encoding().encode_fragment("[")?;
        bytes.extend(
            self.state()
                .source
                .bytes_in(link.label_source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?,
        );
        bytes.extend(self.encoding().encode_fragment("](viem-label)")?);
        Document::from_bytes_with_file_format(
            bytes,
            self.encoding(),
            Format::Markdown,
            self.file_format(),
        )
    }

    fn link_label_logical_range(
        &self,
        link: &LinkSnapshot,
        range: Range<usize>,
    ) -> Result<Range<usize>, DocumentError> {
        if range.start < link.range.start || range.end > link.range.end {
            return Err(DocumentError::UnsupportedFormatting);
        }
        if !self.format().is_source_view() {
            return Ok(range.start - link.range.start..range.end - link.range.start);
        }
        let lower = self
            .projection()
            .source_insertion_point(range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let upper = self
            .projection()
            .source_insertion_point(range.end, false)
            .ok_or(DocumentError::AmbiguousProjection)?;
        if lower < link.label_source.start || upper > link.label_source.end || upper < lower {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let fragment = self.link_label_document(link)?;
        let prefix = self.encoding().encode_fragment("[")?.len();
        let start = link_boundary(
            &fragment,
            prefix + lower - link.label_source.start,
            BoundaryAffinity::Downstream,
        )
        .ok_or(DocumentError::UnsupportedFormatting)?;
        let end = link_boundary(
            &fragment,
            prefix + upper - link.label_source.start,
            BoundaryAffinity::Upstream,
        )
        .ok_or(DocumentError::UnsupportedFormatting)?;
        fragment.text_point(start)?;
        fragment.text_point(end)?;
        Ok(start..end)
    }

    /// Slice one bounded link label through ordinary verified deletion, so
    /// retained emphasis/code scopes receive the same local delimiter repairs.
    fn link_label_fragment(
        &self,
        link: &LinkSnapshot,
        range: Range<usize>,
        unlink: bool,
    ) -> Result<Vec<u8>, ModelTransactionError> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let mut fragment = self.link_label_document(link)?;
        let length = fragment.projection().text_tree().byte_len();
        fragment.text_point(range.start)?;
        fragment.text_point(range.end)?;
        let expected = fragment
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let mut edits = Vec::new();
        if range.start > 0 {
            edits.push(TextEdit::new(0..range.start, ""));
        }
        if range.end < length {
            edits.push(TextEdit::new(range.end..length, ""));
        }
        if !edits.is_empty() {
            let prepared = fragment.prepare_model_request(ModelRequest::ApplyTextEdits {
                document: fragment.id(),
                revision: fragment.revision(),
                edits,
            })?;
            fragment.commit_model_transaction(prepared)?;
        }
        if fragment.text() != expected {
            return Err(DocumentError::VerificationFailed.into());
        }
        let item = fragment
            .link_snapshot_at(fragment.text_point(0)?)?
            .ok_or(DocumentError::VerificationFailed)?;
        if unlink {
            let (prepared, _) = fragment.prepare_link_edit(
                fragment.id(),
                fragment.revision(),
                LinkEditIntent::Remove { range: item.range },
            )?;
            fragment.commit_model_transaction(prepared)?;
            Ok(fragment.source_bytes())
        } else {
            fragment
                .state()
                .source
                .bytes_in(item.label_source)
                .ok_or(DocumentError::VerificationFailed.into())
        }
    }

    fn split_link_replacement(
        &self,
        link: &LinkSnapshot,
        selected: Range<usize>,
        middle: Vec<u8>,
    ) -> Result<(Vec<u8>, usize), ModelTransactionError> {
        if link.destination_source.is_none() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let logical = self.link_label_logical_range(link, selected)?;
        let length = self
            .link_label_document(link)?
            .projection()
            .text_tree()
            .byte_len();
        let prefix = self
            .state()
            .source
            .bytes_in(link.source.start..link.label_source.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let suffix = self
            .state()
            .source
            .bytes_in(link.label_source.end..link.source.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut replacement = Vec::new();
        if logical.start > 0 {
            replacement.extend_from_slice(&prefix);
            replacement.extend(self.link_label_fragment(link, 0..logical.start, false)?);
            replacement.extend_from_slice(&suffix);
        }
        replacement.extend_from_slice(&middle);
        let caret_source = link.source.start + replacement.len();
        if logical.end < length {
            replacement.extend(prefix);
            replacement.extend(self.link_label_fragment(link, logical.end..length, false)?);
            replacement.extend(suffix);
        }
        Ok((replacement, caret_source))
    }

    fn prepare_split_link(
        &self,
        link: &LinkSnapshot,
        range: Range<usize>,
        middle: Vec<u8>,
        text: &str,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let (replacement, caret_source) =
            self.split_link_replacement(link, range.clone(), middle)?;
        let original = self
            .state()
            .source
            .bytes_in(link.source.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = original
            .iter()
            .zip(&replacement)
            .take_while(|(old, new)| old == new)
            .count();
        let suffix = original[prefix..]
            .iter()
            .rev()
            .zip(replacement[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        let decoded = self
            .encoding()
            .decode_region(&original, link.source.start)?;
        // Keep the patch at complete encoded scalar boundaries, while retaining
        // every unchanged label byte and its source-position identity.
        let prefix = decoded
            .scalar_spans()
            .take_while(|unit| unit.source.end <= link.source.start + prefix)
            .last()
            .map_or(0, |unit| unit.source.end - link.source.start);
        let suffix_start = decoded
            .scalar_spans()
            .find(|unit| unit.source.start >= link.source.end - suffix)
            .map_or(link.source.end, |unit| unit.source.start);
        let suffix = link.source.end - suffix_start;
        let patch = SourcePatch::primary(
            link.source.start + prefix..suffix_start,
            replacement[prefix..replacement.len() - suffix].to_vec(),
        );
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(vec![patch])?
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![TextEdit::new(range.clone(), text)],
                Some(vec![patch]),
                true,
            )?
        };
        let candidate = self.prepared_candidate_document(&prepared)?;
        let caret = if self.format().is_source_view() {
            link_boundary(&candidate, caret_source, BoundaryAffinity::Upstream)
                .ok_or(DocumentError::VerificationFailed)?
        } else {
            range.start + text.len()
        };
        candidate.text_point(caret)?;
        Ok((prepared, caret))
    }

    pub(crate) fn prepare_new_link_at_unlinked_caret(
        &self,
        range: Range<usize>,
        text: String,
        destination: String,
        affinity: BoundaryAffinity,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if !range.is_empty() || !self.can_exit_link_typing(range.start, affinity) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let payload = FormattedTextPayload::new(&self.hard_line_snapshot(), text, Vec::new())
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        self.prepare_link_after_unlinked_typing(
            FormattedPayloadEdit::new(range, payload).with_boundary_affinity(affinity),
            named,
            values,
            destination,
        )
    }

    fn prepare_link_selection_removal(
        &self,
        range: Range<usize>,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if self.is_read_only()
            || !self.format().is_markdown()
            || range.is_empty()
            || range.len() > MAX_LINK_QUERY_BYTES / 2
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let link = self
            .link_snapshot_at(self.text_point(range.start)?)?
            .filter(|link| link.editable && range.end <= link.range.end)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if range == link.range {
            return self.prepare_link_edit(
                self.id(),
                self.revision(),
                LinkEditIntent::Remove { range },
            );
        }
        if link.destination_source.is_none() {
            return self.prepare_autolink_selection_removal(range);
        }
        let logical = self.link_label_logical_range(&link, range.clone())?;
        let middle = self.link_label_fragment(&link, logical, true)?;
        let text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let (prepared, caret) = self.prepare_split_link(&link, range.clone(), middle, &text)?;
        if !self.format().is_source_view() {
            let candidate = self.prepared_candidate_document(&prepared)?;
            if candidate
                .projection()
                .style_spans_for_region(&range)
                .iter()
                .any(|span| {
                    span.application == StyleApplication::Automatic("Link".into())
                        && span.range.start < range.end
                        && range.start < span.range.end
                })
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        Ok((prepared, caret))
    }

    pub(crate) fn prepare_unlinked_typing_boundary(
        &self,
        range: Range<usize>,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let link = self
            .link_snapshot_at(self.text_point(range.start)?)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let logical = self.link_label_logical_range(&link, range.clone())?;
        let middle = self.link_label_fragment(&link, logical, true)?;
        let length = middle.len();
        let fragment = Document::from_bytes_with_file_format(
            middle.clone(),
            self.encoding(),
            Format::Markdown,
            self.file_format(),
        )?;
        let inner = fragment
            .projection()
            .source_insertion_point(0, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let (prepared, end) = self.prepare_split_link(&link, range.clone(), middle, &text)?;
        let at = if self.format().is_source_view() {
            let candidate = self.prepared_candidate_document(&prepared)?;
            let end_source = candidate
                .projection()
                .source_insertion_point(end, false)
                .ok_or(DocumentError::VerificationFailed)?;
            link_boundary(
                &candidate,
                end_source
                    .checked_sub(length)
                    .ok_or(DocumentError::VerificationFailed)?
                    + inner,
                BoundaryAffinity::Downstream,
            )
            .ok_or(DocumentError::VerificationFailed)?
        } else {
            range.start
        };
        Ok((prepared, at))
    }

    pub(crate) fn prepare_insertion_without_link(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        inherited: Option<&super::ReplacementTypingContext>,
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        self.validate_typing_payload(&edit)?;
        // Replacement consumes the first selected character regardless of the
        // previous insertion side, just like ordinary replacement inheritance.
        let affinity = if edit.range.is_empty() {
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream)
        } else {
            BoundaryAffinity::Downstream
        };
        let edit = edit.with_boundary_affinity(affinity);
        let Some(link) = self.link_at_boundary(edit.range.start, affinity)? else {
            return self.prepare_typing_without_automatic_links(edit, named, values, inherited);
        };
        if !self.can_exit_link_typing(edit.range.start, affinity) || edit.range.end > link.range.end
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if link.destination_source.is_none() {
            return self.prepare_typing_after_canonical_link(edit, named, values);
        }
        let (prepared, caret, start) = self.prepare_typing_after_link_split(edit, named, values)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        if candidate.link_at(candidate.text_point(start)?)?.is_some() {
            return Err(DocumentError::VerificationFailed.into());
        }
        let actual =
            crate::layout::DocumentLayoutStyles::character_at(candidate.projection(), start, false)
                .map_err(|_| DocumentError::VerificationFailed)?;
        let requested = self.validate_typing_properties(values)?;
        if requested.bold.is_some_and(|value| value != actual.bold)
            || requested.slant.is_some_and(|value| value != actual.slant)
            || requested
                .strikethrough
                .is_some_and(|value| value != actual.strikethrough)
            || named.is_some_and(|style| style.0 == "Code")
                && !candidate.is_code_at(start, BoundaryAffinity::Downstream)?
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok((prepared, caret, start))
    }

    pub fn can_insert_link(&self, range: Range<usize>) -> bool {
        if !self.format().is_markdown()
            || self.is_read_only()
            || range.start > range.end
            || range.len() > MAX_LINK_QUERY_BYTES / 2
            || self.text_point(range.start).is_err()
            || self.text_point(range.end).is_err()
            || self
                .projection()
                .markdown_replacement_begins_in_code(&range)
            || self
                .projection()
                .has_decoding_diagnostic_overlapping(&range)
        {
            return false;
        }
        let Ok(text) = self.projection().text_tree().slice(range.clone()) else {
            return false;
        };
        if text.contains(['\n', '\r']) {
            return false;
        }
        if self
            .projection()
            .blocks_for_region(&range)
            .iter()
            .any(|block| block.markdown_html || block.thematic_break)
        {
            return false;
        }
        if self
            .projection()
            .style_spans_for_region(&range)
            .iter()
            .any(|span| {
                span.application == StyleApplication::Automatic("Link".into())
                    && if range.is_empty() {
                        span.range.start < range.start && range.start < span.range.end
                    } else {
                        span.range.start < range.end && range.start < span.range.end
                    }
            })
        {
            return false;
        }
        self.projection().source_range(range).is_some()
    }

    /// Prepare exact local source patches, then verify the requested label,
    /// destination and surrounding visible text before any history publication.
    pub fn prepare_link_edit(
        &self,
        document: DocumentId,
        revision: Revision,
        intent: LinkEditIntent,
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
        if let LinkEditIntent::RemoveSelection { range } = intent {
            return self.prepare_link_selection_removal(range);
        }
        let (range, text, destination, remove, existing) = match intent {
            LinkEditIntent::Insert {
                range,
                text,
                destination,
            } => {
                if !self.can_insert_link(range.clone()) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                (range, text, destination, false, None)
            }
            LinkEditIntent::Edit {
                range,
                text,
                destination,
            } => {
                let link = self
                    .link_snapshot_at(self.text_point(range.start)?)?
                    .filter(|link| link.editable && link.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, text, destination, false, Some(link))
            }
            LinkEditIntent::Remove { range } => {
                let link = self
                    .link_snapshot_at(self.text_point(range.start)?)?
                    .filter(|link| link.editable && link.range == range)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (range, link.text.clone(), String::new(), true, Some(link))
            }
            LinkEditIntent::RemoveSelection { .. } => unreachable!("handled above"),
        };
        if (!remove && text.is_empty())
            || text.len() + destination.len() > MAX_LINK_QUERY_BYTES / 2
            || text.contains(['\n', '\r'])
            || destination.chars().any(char::is_control)
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if !remove
            && existing
                .as_ref()
                .is_some_and(|link| link.text == text && link.destination == destination)
        {
            return Ok((
                self.prepare_reprojected_source_patches(Vec::new())?,
                range.end,
            ));
        }
        let old_text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let source_range = existing
            .as_ref()
            .map(|link| link.source.clone())
            .or_else(|| self.projection().source_range(range.clone()))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let preserved_label = if let Some(link) = &existing {
            (text == link.text).then(|| link.label_source.clone())
        } else if !self.format().is_source_view() && text == old_text {
            Some(source_range.clone())
        } else {
            None
        };
        let label_bytes = if let Some(label) = preserved_label {
            self.state()
                .source
                .bytes_in(label)
                .ok_or(DocumentError::VerificationFailed)?
        } else if let Some(link) = existing
            .as_ref()
            .filter(|link| link.destination_source.is_some() && !link.text.is_empty())
        {
            self.replacement_link_label(link, &text)?
        } else {
            self.encoding().encode_fragment(&escape_label(&text))?
        };
        let mut replacement = Vec::new();
        if let Some(link) = &existing {
            if link.destination_source.is_none() {
                let label = self.encoding().encode_fragment(&escape_label(&text))?;
                if !remove {
                    replacement.extend(self.encoding().encode_fragment("[")?);
                }
                replacement.extend(label);
                if !remove {
                    replacement.extend(
                        self.encoding().encode_fragment(&format!(
                            "](<{}>)",
                            escape_destination(&destination)
                        ))?,
                    );
                }
            } else if !remove {
                let dest = link
                    .destination_source
                    .clone()
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                replacement.extend(
                    self.state()
                        .source
                        .bytes_in(source_range.start..link.label_source.start)
                        .ok_or(DocumentError::VerificationFailed)?,
                );
                replacement.extend(label_bytes);
                replacement.extend(
                    self.state()
                        .source
                        .bytes_in(link.label_source.end..dest.start)
                        .ok_or(DocumentError::VerificationFailed)?,
                );
                if destination == link.destination {
                    replacement.extend(
                        self.state()
                            .source
                            .bytes_in(dest.clone())
                            .ok_or(DocumentError::VerificationFailed)?,
                    );
                } else {
                    replacement.extend(
                        self.encoding()
                            .encode_fragment(&format!("<{}>", escape_destination(&destination)))?,
                    );
                }
                replacement.extend(
                    self.state()
                        .source
                        .bytes_in(dest.end..source_range.end)
                        .ok_or(DocumentError::VerificationFailed)?,
                );
            } else {
                let label = self.encoding().decode_region(&label_bytes, 0)?.text;
                let mut protected = label.clone();
                let code: Vec<_> = pulldown_cmark::Parser::new(&label)
                    .into_offset_iter()
                    .filter_map(|(event, range)| {
                        matches!(event, pulldown_cmark::Event::Code(_)).then_some(range)
                    })
                    .collect();
                let mut links = Vec::new();
                let mut consumed = 0;
                for (at, _) in label.char_indices() {
                    if at < consumed || code.iter().any(|range| range.contains(&at)) {
                        continue;
                    }
                    if let Some((end, _)) =
                        super::markdown_syntax::autolink(&label, at, label.len())
                    {
                        links.push(at..end);
                        consumed = end;
                    }
                }
                for range in links.iter().rev() {
                    protected.replace_range(range.clone(), &escape_label(&label[range.clone()]));
                }
                replacement.extend(self.encoding().encode_fragment(&protected)?);
            }
        } else {
            replacement.extend(self.encoding().encode_fragment("[")?);
            replacement.extend(label_bytes);
            replacement.extend(
                self.encoding()
                    .encode_fragment(&format!("](<{}>)", escape_destination(&destination)))?,
            );
        }
        let replacement_text = super::line_endings::normalize(
            &self
                .encoding()
                .decode_region(&replacement, source_range.start)?,
            self.file_format(),
        )
        .text;
        let expected_replacement = if self.format().is_source_view() {
            replacement_text
        } else {
            text.clone()
        };
        // The shared authored-syntax path verifies a persistent text splice and
        // reparses the affected region, preserving unrelated document storage.
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.clone(), expected_replacement.clone())],
            Some(vec![SourcePatch::primary(
                source_range.clone(),
                replacement,
            )]),
            true,
        )?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        if candidate
            .projection()
            .text_tree()
            .slice(range.start..range.start + expected_replacement.len())
            .map_err(DocumentError::FormattedTextStorage)?
            != expected_replacement
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        if !remove {
            let at = range.start;
            let link = candidate
                .link_snapshot_at(candidate.text_point(at)?)?
                .ok_or(DocumentError::VerificationFailed)?;
            if link.destination != destination
                || link.text != text
                || link.source.start != source_range.start
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        let caret = range.start + expected_replacement.len();
        self.prepared_text_point(&prepared, caret)?;
        Ok((prepared, caret))
    }

    /// Run the existing semantic replacement policy on a bounded inline
    /// fragment. The wrapper keeps label punctuation in inline context and
    /// supplies a stable owner while the first character's styles are retained.
    fn replacement_link_label(
        &self,
        link: &LinkSnapshot,
        text: &str,
    ) -> Result<Vec<u8>, ModelTransactionError> {
        let mut bytes = self.encoding().encode_fragment("[")?;
        bytes.extend(
            self.state()
                .source
                .bytes_in(link.label_source.clone())
                .ok_or(DocumentError::VerificationFailed)?,
        );
        bytes.extend(self.encoding().encode_fragment("](viem-label)")?);
        let mut fragment = Document::from_bytes_with_file_format(
            bytes,
            self.encoding(),
            Format::Markdown,
            self.file_format(),
        )?;
        let range = 0..fragment.projection().text_tree().byte_len();
        let inherited = fragment.replacement_typing_context(range.clone())?;
        let payload = FormattedTextPayload::new(&fragment.hard_line_snapshot(), text, Vec::new())
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let (prepared, _, _) = fragment.prepare_insertion_with_typing_context(
            FormattedPayloadEdit::new(range, payload),
            None,
            &[],
            inherited.as_ref(),
        )?;
        fragment.commit_model_transaction(prepared)?;
        let label = fragment
            .link_snapshot_at(fragment.text_point(0)?)?
            .ok_or(DocumentError::VerificationFailed)?;
        if label.text != text {
            return Err(DocumentError::VerificationFailed.into());
        }
        fragment
            .state()
            .source
            .bytes_in(label.label_source)
            .ok_or(DocumentError::VerificationFailed.into())
    }

    /// Explicit navigation and destination-picker queries share fragment names.
    /// Passive caret refresh never enumerates headings.
    fn visit_link_headings(
        &self,
        byte_budget: usize,
        mut visit: impl FnMut(LinkHeading) -> bool,
    ) -> Result<bool, DocumentError> {
        if !self.format().is_markdown() {
            return Ok(false);
        }
        let mut slugs = std::collections::HashMap::<String, usize>::new();
        let mut index = 0;
        let mut decoded_bytes = 0usize;
        while let Some((found, block)) = self
            .projection()
            .heading_after(index, self.format().is_source_view())
        {
            index = found + 1;
            let level = match block.kind {
                BlockKind::Heading(level) => level,
                _ => block
                    .style
                    .0
                    .strip_prefix("Heading")
                    .and_then(|level| level.parse::<u8>().ok())
                    .filter(|level| (1..=6).contains(level))
                    .ok_or(DocumentError::AmbiguousProjection)?,
            };
            let source_start = self
                .projection()
                .source_insertion_point(block.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_end = self
                .projection()
                .source_insertion_point(block.range.end, false)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let lines = &self.state().source_hard_lines;
            let first = lines
                .line_at_offset(source_start)
                .and_then(|line| lines.get(line))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let last = lines
                .line_at_offset(source_end.saturating_sub(1).max(source_start))
                .and_then(|line| lines.get(line))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source = first.start..last.end;
            decoded_bytes = decoded_bytes.saturating_add(source.len());
            if decoded_bytes > byte_budget {
                return Ok(true);
            }
            let bytes = self
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, source.start)?;
            let input = super::line_endings::normalize(&decoded, self.file_format());
            // Each owner is already classified as a heading. Remove its local
            // list/quote prefix so a nested heading parses independently of its
            // unvisited ancestors. Authored body whitespace remains escaped.
            let body = input
                .text
                .lines()
                .map(|line| {
                    let line = &line[super::markdown_quotes::prefix(line)..];
                    let line = line.trim_start_matches([' ', '\t']);
                    let prefix = super::markdown_blocks::marker_prefix_length(line).unwrap_or(0);
                    &line[prefix..]
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut label = String::new();
            use pulldown_cmark::{Event, Options};
            for event in pulldown_cmark::Parser::new_ext(&body, Options::ENABLE_STRIKETHROUGH) {
                match event {
                    Event::Text(text) | Event::Code(text) => label.push_str(&text),
                    Event::SoftBreak | Event::HardBreak => label.push(' '),
                    _ => {}
                }
            }
            let base: String = label
                .trim()
                .to_lowercase()
                .chars()
                .filter_map(|ch| {
                    if ch == ' ' {
                        Some('-')
                    } else if ch == '-' || ch == '_' || ch.is_alphanumeric() {
                        Some(ch)
                    } else {
                        None
                    }
                })
                .collect();
            let mut slug = base.clone();
            while slugs.contains_key(&slug) {
                let suffix = slugs.entry(base.clone()).or_default();
                *suffix += 1;
                slug = format!("{base}-{suffix}");
            }
            slugs.insert(slug.clone(), 0);
            if !visit(LinkHeading {
                text: label,
                destination: format!("#{slug}"),
                level,
                offset: block.range.start,
            }) {
                break;
            }
        }
        Ok(false)
    }

    pub fn find_link_fragment(&self, fragment: &str) -> Result<Option<usize>, DocumentError> {
        if !self.format().is_markdown() {
            return Ok(None);
        }
        if fragment.is_empty() {
            return Ok(Some(0));
        }
        let mut result = None;
        self.visit_link_headings(usize::MAX, |heading| {
            if heading.destination.strip_prefix('#') == Some(fragment) {
                result = Some(heading.offset);
                false
            } else {
                true
            }
        })?;
        Ok(result)
    }

    /// An explicit picker request, with finite native menu/label retention.
    pub fn link_headings(&self) -> Result<LinkHeadingList, DocumentError> {
        let mut result = LinkHeadingList {
            headings: Vec::new(),
            truncated: false,
        };
        let mut retained = 0usize;
        let input_truncated = self.visit_link_headings(256 * 1024, |heading| {
            retained = retained
                .saturating_add(heading.text.len())
                .saturating_add(heading.destination.len());
            if result.headings.len() == 1024 || retained > 256 * 1024 {
                result.truncated = true;
                return false;
            }
            result.headings.push(heading);
            true
        })?;
        result.truncated |= input_truncated;
        Ok(result)
    }
}
