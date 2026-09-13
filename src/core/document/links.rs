//! Link recognition and on-demand destinations. This is an original parser for
//! Markdown inline links; it shares Viem's passive HTML tokenizer and entities.
use super::html::TokenKind;
use super::line_endings::NormalizedText;
use super::*;

#[derive(Debug)]
pub(super) struct InlineLink {
    pub range: Range<usize>,
    pub label: Range<usize>,
    pub destination: String,
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
    let bytes = text.as_bytes();
    if end > bytes.len()
        || bytes.get(start) != Some(&b'[')
        || escaped(bytes, start)
        || (start > 0 && bytes[start - 1] == b'!' && !escaped(bytes, start - 1))
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
    if label_contains_link(text, start + 1, label_end) {
        return None;
    }
    Some(InlineLink {
        range: start..at + 1,
        label: start + 1..label_end,
        destination,
    })
}

pub(super) fn html_links(text: &str) -> Vec<InlineLink> {
    let mut result = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for token in super::html::tokenize(text) {
        if let TokenKind::Tag(tag) = token.kind {
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
        if start < text.len() {
            result.push(InlineLink {
                range: start..text.len(),
                label: start..text.len(),
                destination,
            });
        }
    }
    result
}

pub(super) fn style_html_links(document: &mut FormattedDocument, input: &NormalizedText) {
    let ranges = html_links(&input.text)
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

impl Document {
    /// Resolve a link from this exact snapshot's authored source on demand.
    /// Literal documents take the constant-time path and retain no link index.
    pub fn link_at(&self, point: TextPoint) -> Result<Option<String>, DocumentError> {
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
        if !(self.format().is_markdown() || self.format().is_html()) {
            return Ok(None);
        }
        let link_styles: Vec<_> = self
            .projection()
            .style_spans_for_region(&(point.offset()..point.offset().saturating_add(1)))
            .into_iter()
            .filter(|span| {
                span.range.contains(&point.offset())
                    && span.application == StyleApplication::Automatic("Link".into())
            })
            .collect();
        if link_styles.is_empty() {
            return Ok(None);
        }
        let Some(raw) = self
            .projection()
            .source_insertion_point(point.offset(), true)
        else {
            return Ok(None);
        };
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = super::line_endings::normalize(&decoded, self.file_format());
        let Some(unit) = input
            .units
            .get(input.units.partition_point(|u| u.source.end <= raw))
        else {
            return Ok(None);
        };
        let at = unit.normalized.start;
        if self.format().is_html() {
            return Ok(html_links(&input.text)
                .into_iter()
                .find(|link| link.range.contains(&at))
                .map(|link| link.destination));
        }
        let mut found: Option<InlineLink> = None;
        for (start, _) in input.text.match_indices('[') {
            if start > at {
                break;
            }
            let Some(link) = markdown_inline_at(&input.text, start, input.text.len()) else {
                continue;
            };
            if !link.range.contains(&at) {
                continue;
            }
            // Only the projector knows whether syntax occurs in code or a
            // different paragraph. Match the candidate's displayed extent to
            // a Link interval from that exact projection before using its URL.
            // WYSIWYG displays the label; Source displays the whole construct.
            let displayed = if self.format().is_source_view() {
                &link.range
            } else {
                &link.label
            };
            let begin = input
                .units
                .partition_point(|unit| unit.normalized.end <= displayed.start);
            let end = input
                .units
                .partition_point(|unit| unit.normalized.start < displayed.end);
            if begin >= end {
                continue;
            }
            let Some(first) = link_boundary(
                self,
                input.units[begin].source.start,
                BoundaryAffinity::Downstream,
            ) else {
                continue;
            };
            let Some(last) = link_boundary(
                self,
                input.units[end - 1].source.end,
                BoundaryAffinity::Upstream,
            ) else {
                continue;
            };
            if !link_styles.iter().any(|span| span.range == (first..last)) {
                continue;
            }
            // Prefer the narrowest authoritatively styled construct if more
            // than one candidate recovers the same displayed boundaries.
            if found
                .as_ref()
                .is_none_or(|previous| link.range.len() < previous.range.len())
            {
                found = Some(link);
            }
        }
        Ok(found.map(|link| link.destination))
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
