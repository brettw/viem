//! HTML whitespace policy for semantic formatted edits.
//!
//! CSS Text whitespace processing ignores inline tag boundaries. The formatted
//! neighbors therefore decide which authored spaces need nonbreaking spelling;
//! source provenance decides the local patch and whether old generated spacing
//! can be simplified without changing imported or explicitly nonbreaking text.
use super::{
    BoundaryAffinity, Document, DocumentError, Format, FormattedPayloadEdit, FormattedTextPayload,
};
use std::ops::Range;

pub(super) fn collapsible(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

/// Inserting into an empty inline element can divide one collapsed whitespace
/// run. Remove only the formerly invisible whitespace that would become a new
/// visible space on the other side of the insertion; retain every tag/comment.
pub(super) fn exposed_whitespace(
    document: &Document,
    edit: &super::TextEdit,
    source_at: usize,
) -> Result<Vec<Range<usize>>, DocumentError> {
    if !edit.html_normalized || !edit.range.is_empty() || edit.replacement.is_empty() {
        return Ok(Vec::new());
    }
    let tree = document.projection().text_tree();
    let start = tree
        .previous_grapheme_boundary(edit.range.start)
        .map_err(DocumentError::FormattedTextStorage)?
        .unwrap_or(edit.range.start);
    let end = tree
        .next_grapheme_boundary(edit.range.end)
        .map_err(DocumentError::FormattedTextStorage)?
        .unwrap_or(edit.range.end);
    let spans = document.projection().provenance_for_region(&(start..end));
    let mut gaps = Vec::new();
    if start < edit.range.start
        && edit
            .replacement
            .as_str()
            .chars()
            .next()
            .is_some_and(|ch| !collapsible(ch))
    {
        let previous = tree
            .slice(start..edit.range.start)
            .map_err(DocumentError::FormattedTextStorage)?;
        if previous
            .chars()
            .next_back()
            .is_some_and(|ch| !collapsible(ch))
        {
            if let Some(span) = spans
                .iter()
                .find(|span| span.formatted.end == edit.range.start && !span.formatted.is_empty())
            {
                gaps.push(span.source.end..source_at);
            }
        }
    }
    if edit.range.end < end
        && edit
            .replacement
            .as_str()
            .chars()
            .next_back()
            .is_some_and(|ch| !collapsible(ch))
    {
        let next = tree
            .slice(edit.range.end..end)
            .map_err(DocumentError::FormattedTextStorage)?;
        if next.chars().next().is_some_and(|ch| !collapsible(ch)) {
            if let Some(span) = spans
                .iter()
                .find(|span| span.formatted.start == edit.range.end && !span.formatted.is_empty())
            {
                gaps.push(source_at..span.source.start);
            }
        }
    }
    let mut result = Vec::new();
    for gap in gaps {
        if gap.start >= gap.end {
            continue;
        }
        let bytes = document
            .state()
            .source
            .bytes_in(gap.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, gap.start)?;
        let input = super::line_endings::normalize(&decoded, document.file_format());
        let mapper = super::rich_text::Builder::new(&input, document.revision());
        let mut whitespace = Vec::new();
        let mut safe = true;
        for token in super::html::tokenize(&input.text) {
            match token.kind {
                super::html::TokenKind::Text => {
                    let mut at = token.range.start;
                    while at < token.range.end {
                        let (value, length) =
                            super::html::reference(&input.text[at..token.range.end], false)
                                .unwrap_or_else(|| {
                                    let ch = input.text[at..].chars().next().unwrap();
                                    (ch.to_string(), ch.len_utf8())
                                });
                        if !value.chars().all(collapsible) {
                            safe = false;
                            break;
                        }
                        at += length;
                    }
                    whitespace.push(mapper.source_range(token.range));
                }
                super::html::TokenKind::Tag(tag) => {
                    if super::html::block(&tag.name)
                        || super::html::hidden(&tag.name)
                        || super::html::atomic(&tag.name)
                        || tag.name == "br"
                    {
                        safe = false;
                        break;
                    }
                }
                super::html::TokenKind::Opaque => {
                    if !input.text[token.range].starts_with("<!--") {
                        safe = false;
                        break;
                    }
                }
                super::html::TokenKind::MappedText { .. } => {
                    unreachable!("lossless tokenizer emits raw text")
                }
            }
        }
        if safe {
            result.extend(whitespace);
        }
    }
    Ok(result)
}

impl Document {
    pub(super) fn normalize_html_payload_edits(
        &self,
        edits: &mut Vec<FormattedPayloadEdit>,
    ) -> Result<(), DocumentError> {
        if self.format() == Format::Html {
            *edits = std::mem::take(edits)
                .into_iter()
                .map(|edit| {
                    if edit.payload.text().contains('\r') && !edit.typing_normalized {
                        Ok(edit)
                    } else {
                        self.normalize_typing_payload(edit)
                    }
                })
                .collect::<Result<Vec<_>, DocumentError>>()?;
        }
        if self.format() == Format::Html {
            let mut text_edits = edits
                .iter()
                .map(FormattedPayloadEdit::text_edit)
                .collect::<Vec<_>>();
            let supporting_edits = self.html_boundary_space_edits(&mut text_edits)?;
            for (edit, normalized) in edits.iter_mut().zip(text_edits) {
                if edit.payload.text() != normalized.replacement {
                    // Batch context can protect additional spaces inside a
                    // payload. Scalar count stays fixed, but UTF-8 offsets of
                    // later hard-break markers must follow the new spelling.
                    let breaks = edit
                        .payload
                        .text()
                        .char_indices()
                        .zip(normalized.replacement.char_indices())
                        .filter_map(|((old, _), (new, _))| {
                            edit.payload
                                .break_offsets()
                                .binary_search(&old)
                                .is_ok()
                                .then_some(new)
                        })
                        .collect();
                    edit.payload = FormattedTextPayload::new(
                        &self.hard_line_snapshot(),
                        normalized.replacement,
                        breaks,
                    )
                    .expect("space protection preserves scalar and hard-break identities");
                }
                edit.html_protective_spaces = normalized.html_protective_spaces;
            }
            for support in supporting_edits {
                let payload = FormattedTextPayload::new(
                    &self.hard_line_snapshot(),
                    support.replacement,
                    Vec::new(),
                )
                .expect("a protective space has no hard breaks");
                let mut edit = FormattedPayloadEdit::new(support.range, payload);
                edit.html_protective_spaces = support.html_protective_spaces;
                edit.typing_normalized = true;
                edits.push(edit);
            }
        }
        Ok(())
    }

    pub(super) fn html_boundary_space_edits(
        &self,
        edits: &mut [super::TextEdit],
    ) -> Result<Vec<super::TextEdit>, DocumentError> {
        if self.format() != Format::Html || edits.is_empty() {
            return Ok(Vec::new());
        }
        // Resolve neighbors in the complete intended batch. Looking at the
        // old neighbors of each individual deletion misses adjacent deletions
        // and spaces whose other neighbor is replaced in the same transaction.
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        enum Origin {
            Original(usize),
            Inserted(usize, usize),
        }
        let tree = self.projection().text_tree();
        let mut order = (0..edits.len()).collect::<Vec<_>>();
        order.sort_by_key(|&index| (edits[index].range.start, edits[index].range.end));
        for pair in order.windows(2) {
            let first = &edits[pair[0]].range;
            let second = &edits[pair[1]].range;
            if first.end > second.start
                || (first.is_empty() && second.is_empty() && first.start == second.start)
            {
                return Err(DocumentError::OverlappingEdits);
            }
        }
        let mut shifts = vec![0isize];
        let mut ranks = vec![0; edits.len()];
        for (rank, &index) in order.iter().enumerate() {
            ranks[index] = rank;
            let inserted = isize::try_from(edits[index].replacement.len())
                .map_err(|_| DocumentError::AmbiguousProjection)?;
            let removed = isize::try_from(edits[index].range.len())
                .map_err(|_| DocumentError::AmbiguousProjection)?;
            shifts.push(
                shifts[rank]
                    .checked_add(inserted)
                    .and_then(|value| value.checked_sub(removed))
                    .ok_or(DocumentError::AmbiguousProjection)?,
            );
        }
        let original_before = |at: usize| -> Result<Option<(char, Origin)>, DocumentError> {
            let Some(start) = tree
                .previous_grapheme_boundary(at)
                .map_err(DocumentError::FormattedTextStorage)?
            else {
                return Ok(None);
            };
            let text = tree
                .slice(start..at)
                .map_err(DocumentError::FormattedTextStorage)?;
            Ok(text
                .chars()
                .next_back()
                .map(|ch| (ch, Origin::Original(at - ch.len_utf8()))))
        };
        let original_after = |at: usize| -> Result<Option<(char, Origin)>, DocumentError> {
            let Some(end) = tree
                .next_grapheme_boundary(at)
                .map_err(DocumentError::FormattedTextStorage)?
            else {
                return Ok(None);
            };
            let text = tree
                .slice(at..end)
                .map_err(DocumentError::FormattedTextStorage)?;
            Ok(text.chars().next().map(|ch| (ch, Origin::Original(at))))
        };
        let before =
            |mut at: usize, mut count: usize| -> Result<Option<(char, Origin)>, DocumentError> {
                while count > 0 {
                    let index = order[count - 1];
                    let edit = &edits[index];
                    if edit.range.end != at {
                        break;
                    }
                    if let Some((offset, ch)) = edit.replacement.char_indices().next_back() {
                        return Ok(Some((ch, Origin::Inserted(index, offset))));
                    }
                    at = edit.range.start;
                    count -= 1;
                }
                original_before(at)
            };
        let after =
            |mut at: usize, mut rank: usize| -> Result<Option<(char, Origin)>, DocumentError> {
                while rank < order.len() {
                    let index = order[rank];
                    let edit = &edits[index];
                    if edit.range.start != at {
                        break;
                    }
                    if let Some(ch) = edit.replacement.chars().next() {
                        return Ok(Some((ch, Origin::Inserted(index, 0))));
                    }
                    at = edit.range.end;
                    rank += 1;
                }
                original_after(at)
            };
        let original_preserved = |at: usize| {
            self.projection()
                .style_spans_for_region(&(at..at + 1))
                .iter()
                .any(|span| {
                    span.range.contains(&at)
                        && span.application == super::StyleApplication::SourcePreservedWhitespace
                })
        };
        let mut inserted_preserved = vec![false; edits.len()];
        for (index, edit) in edits.iter().enumerate() {
            if !edit.replacement.contains(' ') {
                continue;
            }
            let runs = match super::rich_text::text_source_runs(self, &edit.range) {
                Err(DocumentError::AmbiguousProjection) if !edit.range.is_empty() => {
                    super::rich_text::text_source_runs(self, &(edit.range.start..edit.range.start))?
                }
                result => result?,
            };
            inserted_preserved[index] =
                super::rich_text::html_preserves_whitespace_at_source(self, runs[0].start)?;
        }
        let preserved = |origin: Origin| match origin {
            Origin::Original(at) => original_preserved(at),
            Origin::Inserted(index, _) => inserted_preserved[index],
        };
        let mut candidates = Vec::new();
        for (index, edit) in edits.iter().enumerate() {
            for at in [edit.range.start.checked_sub(1), Some(edit.range.end)]
                .into_iter()
                .flatten()
            {
                if at >= tree.byte_len()
                    || tree.slice(at..at + 1).as_deref() != Ok(" ")
                    || original_preserved(at)
                {
                    continue;
                }
                let count = order.partition_point(|&i| edits[i].range.end <= at);
                if order
                    .get(count)
                    .is_some_and(|&i| edits[i].range.start <= at && at < edits[i].range.end)
                {
                    continue;
                }
                let position = at
                    .checked_add_signed(shifts[count])
                    .ok_or(DocumentError::AmbiguousProjection)?;
                candidates.push((position, Origin::Original(at)));
            }
            if !inserted_preserved[index] {
                let position = edit
                    .range
                    .start
                    .checked_add_signed(shifts[ranks[index]])
                    .ok_or(DocumentError::AmbiguousProjection)?;
                candidates.extend(
                    edit.replacement
                        .char_indices()
                        .filter(|(_, ch)| *ch == ' ')
                        .map(|(at, _)| (position + at, Origin::Inserted(index, at))),
                );
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let mut protect = std::collections::BTreeSet::new();
        for (_, origin) in candidates {
            let (left, right) = match origin {
                Origin::Original(at) => {
                    let count = order.partition_point(|&i| edits[i].range.end <= at);
                    let rank = order.partition_point(|&i| edits[i].range.start < at + 1);
                    (before(at, count)?, after(at + 1, rank)?)
                }
                Origin::Inserted(index, at) => {
                    let edit = &edits[index];
                    let left = if at > 0 {
                        edit.replacement[..at]
                            .char_indices()
                            .next_back()
                            .map(|(offset, ch)| (ch, Origin::Inserted(index, offset)))
                    } else {
                        before(edit.range.start, ranks[index])?
                    };
                    let right = if at + 1 < edit.replacement.len() {
                        edit.replacement[at + 1..]
                            .chars()
                            .next()
                            .map(|ch| (ch, Origin::Inserted(index, at + 1)))
                    } else {
                        after(edit.range.end, ranks[index] + 1)?
                    };
                    (left, right)
                }
            };
            let content = |(ch, origin)| {
                ch != '\n' && (!collapsible(ch) || preserved(origin) || protect.contains(&origin))
            };
            let left_content = left.is_some_and(content);
            // In a run, the following ordinary space can be protected on its
            // own turn. This retains the minimum nonbreaking spelling.
            let following = right.is_some_and(|item| content(item) || item.0 == ' ');
            if !left_content || !following {
                protect.insert(origin);
            }
        }
        let mut result = Vec::new();
        for origin in &protect {
            if let Origin::Original(at) = *origin {
                let end = tree
                    .next_grapheme_boundary(at)
                    .map_err(DocumentError::FormattedTextStorage)?
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let suffix = tree
                    .slice(at + 1..end)
                    .map_err(DocumentError::FormattedTextStorage)?;
                let mut edit = super::TextEdit::new(at..end, format!("\u{a0}{suffix}"));
                edit.html_protective_spaces.push(0);
                edit.html_normalized = true;
                result.push(edit);
            }
        }
        // Inserting text immediately before one of our protective NBSPs can
        // make it an ordinary interior word space. Source origin and exact
        // named spelling distinguish that supporting edit from an authored
        // or explicitly inserted nonbreaking character, even across tags.
        let named_nbsp = self.encoding().encode_fragment("&nbsp;")?;
        let mut simplified = std::collections::BTreeSet::new();
        for edit in edits
            .iter()
            .filter(|edit| edit.range.is_empty() && !edit.replacement.is_empty())
        {
            let at = edit.range.start;
            if original_after(at)?.map(|item| item.0) != Some('\u{a0}')
                || original_preserved(at)
                || !simplified.insert(at)
            {
                continue;
            }
            let end = at + '\u{a0}'.len_utf8();
            let rank = order.partition_point(|&index| edits[index].range.start < end);
            if rank
                .checked_sub(1)
                .and_then(|rank| order.get(rank))
                .is_some_and(|&index| edits[index].range.end > at)
            {
                continue;
            }
            let count = order.partition_point(|&index| edits[index].range.end <= at);
            let content = |(ch, origin)| {
                ch != '\n' && (!collapsible(ch) || preserved(origin) || protect.contains(&origin))
            };
            if !before(at, count)?.is_some_and(content) || !after(end, rank)?.is_some_and(content) {
                continue;
            }
            let provenance = self.projection().provenance_for_region(&(at..end));
            if !provenance.iter().any(|span| {
                span.formatted == (at..end)
                    && self
                        .state()
                        .source
                        .range_is_generated_text(span.source.clone())
                    && self.state().source.bytes_in(span.source.clone()).as_deref()
                        == Some(named_nbsp.as_slice())
            }) {
                continue;
            }
            let cluster_end = tree
                .next_grapheme_boundary(at)
                .map_err(DocumentError::FormattedTextStorage)?
                .ok_or(DocumentError::AmbiguousProjection)?;
            let suffix = tree
                .slice(end..cluster_end)
                .map_err(DocumentError::FormattedTextStorage)?;
            let mut support = super::TextEdit::new(at..cluster_end, format!(" {suffix}"));
            support.html_normalized = true;
            result.push(support);
        }
        for (index, edit) in edits.iter_mut().enumerate() {
            if protect
                .range(Origin::Inserted(index, 0)..=Origin::Inserted(index, usize::MAX))
                .next()
                .is_none()
            {
                continue;
            }
            let mut text = String::new();
            let mut offsets = Vec::new();
            for (at, ch) in edit.replacement.char_indices() {
                let protected = protect.contains(&Origin::Inserted(index, at));
                if protected || edit.html_protective_spaces.binary_search(&at).is_ok() {
                    offsets.push(text.len());
                }
                text.push(if protected { '\u{a0}' } else { ch });
            }
            edit.replacement = text;
            edit.html_protective_spaces = offsets;
            edit.html_normalized = true;
        }
        Ok(result)
    }

    pub(super) fn normalize_html_text_edit(
        &self,
        edit: super::TextEdit,
    ) -> Result<super::TextEdit, DocumentError> {
        if self.format() != Format::Html || edit.html_normalized || edit.replacement.contains('\r')
        {
            // Exact CR is unreprojectable in HTML. Keep that intention intact
            // so verification rejects it instead of silently changing it.
            return Ok(edit);
        }
        let breaks = edit
            .replacement
            .match_indices('\n')
            .map(|(at, _)| at)
            .collect();
        let payload =
            FormattedTextPayload::new(&self.hard_line_snapshot(), edit.replacement, breaks)
                .expect("every marked newline is a validated scalar boundary");
        let mut payload_edit = FormattedPayloadEdit::new(edit.range, payload);
        payload_edit.html_protective_spaces = edit.html_protective_spaces;
        Ok(self.normalize_typing_payload(payload_edit)?.text_edit())
    }

    pub(crate) fn normalize_typing_payload(
        &self,
        mut edit: FormattedPayloadEdit,
    ) -> Result<FormattedPayloadEdit, DocumentError> {
        if edit.payload.document() != self.id() {
            return Err(DocumentError::WrongDocument);
        }
        if edit.payload.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: edit.payload.revision(),
                actual: self.revision(),
            });
        }
        if self.format() != Format::Html || edit.typing_normalized || edit.payload.text().is_empty()
        {
            return Ok(edit);
        }
        self.validate_range(&edit.range)?;
        let previous_is_nbsp = edit.range.start >= 2
            && self
                .projection()
                .text_tree()
                .slice(edit.range.start - 2..edit.range.start)
                .as_deref()
                == Ok("\u{a0}");
        if !edit.payload.text().contains([' ', '\t', '\r']) && !previous_is_nbsp {
            edit.typing_normalized = true;
            return Ok(edit);
        }
        // A structural selection (for example J's paragraph separator) need
        // not have a contiguous visible source run. Only its starting context
        // is needed here; the transaction still translates and verifies the
        // complete structural edit independently.
        let runs = match super::rich_text::text_source_runs(self, &edit.range) {
            Err(DocumentError::AmbiguousProjection) if !edit.range.is_empty() => {
                super::rich_text::text_source_runs(self, &(edit.range.start..edit.range.start))?
            }
            result => result?,
        };
        let source_at = if edit.range.is_empty()
            && edit.boundary_affinity == Some(BoundaryAffinity::Upstream)
            && self
                .projection()
                .hard_line_at_offset(edit.range.start)
                .and_then(|line| self.projection().hard_line_range(line))
                .is_some_and(|line| !line.is_empty())
        {
            self.projection()
                .source_insertion_point(edit.range.start, false)
                .ok_or(DocumentError::AmbiguousProjection)?
        } else {
            runs[0].start
        };
        edit.typing_normalized = true;
        if super::rich_text::html_preserves_whitespace_at_source(self, source_at)? {
            if edit.payload.text().contains('\r') {
                edit.payload = FormattedTextPayload::new(
                    &self.hard_line_snapshot(),
                    edit.payload.text().replace('\r', " "),
                    edit.payload.break_offsets().to_vec(),
                )
                .expect("CR-to-space preserves byte positions and break markers");
            }
            return Ok(edit);
        }

        let (mut previous_is_content, _) =
            super::rich_text::html_text_neighbors_at_text(self, edit.range.start)?;
        let (_, following_is_content) =
            super::rich_text::html_text_neighbors_at_text(self, edit.range.end)?;
        let mut text = String::new();
        let mut breaks = Vec::new();

        // Completing a word can make the preceding protected space ordinary.
        // Its exact named spelling and source origin distinguish it from an
        // authored NBSP, including one explicitly typed in this same session.
        if edit.range.is_empty()
            && previous_is_nbsp
            && edit
                .payload
                .text()
                .chars()
                .next()
                .is_some_and(|ch| ch != '\n')
            && edit.range.start >= 2
        {
            let start = edit.range.start - 2;
            if super::rich_text::html_text_neighbors_at_text(self, start)?.0 {
                let spans = self
                    .projection()
                    .provenance_for_region(&(start..edit.range.start));
                if let Some(span) = spans
                    .iter()
                    .find(|span| span.formatted == (start..edit.range.start))
                {
                    let named = self.encoding().encode_fragment("&nbsp;")?;
                    if (source_at == span.source.end)
                        && self
                            .state()
                            .source
                            .range_is_generated_text(span.source.clone())
                        && self.state().source.bytes_in(span.source.clone()).as_deref()
                            == Some(named.as_slice())
                    {
                        edit.range.start = start;
                        text.push(' ');
                        previous_is_content = false;
                    }
                }
            }
        }

        let mut characters = edit.payload.text().char_indices().peekable();
        while let Some((at, ch)) = characters.next() {
            if ch == '\n' {
                if edit.payload.break_offsets().binary_search(&at).is_ok() {
                    breaks.push(text.len());
                }
                text.push(ch);
                previous_is_content = false;
            } else if matches!(ch, ' ' | '\t' | '\r') {
                let following = characters
                    .peek()
                    .map(|(_, ch)| !collapsible(*ch))
                    .unwrap_or(following_is_content);
                // Within an inserted run the next space can be protected, so
                // alternating ordinary/NBSP uses the fewest required escapes.
                let interior_of_input_run = characters
                    .peek()
                    .is_some_and(|(_, ch)| matches!(ch, ' ' | '\t' | '\r'));
                let ordinary = previous_is_content && (interior_of_input_run || following);
                let output = if ordinary { ' ' } else { '\u{a0}' };
                if !ordinary {
                    edit.html_protective_spaces.push(text.len());
                }
                text.push(output);
                previous_is_content = !collapsible(output);
            } else {
                text.push(ch);
                previous_is_content = true;
            }
        }
        edit.payload = FormattedTextPayload::new(&self.hard_line_snapshot(), text, breaks)
            .expect("typing normalization preserves validated hard-break markers");
        Ok(edit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn typing_normalization_does_not_rebind_foreign_or_stale_payloads() {
        let mut document =
            Document::from_bytes(b"<p>A</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let payload =
            FormattedTextPayload::new(&document.hard_line_snapshot(), " ", vec![]).unwrap();
        let foreign =
            Document::from_bytes(b"<p>A</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(
            foreign.normalize_typing_payload(FormattedPayloadEdit::new(0..0, payload.clone())),
            Err(DocumentError::WrongDocument)
        );
        let before = document.revision();
        document.insert(1, "B").unwrap();
        assert_eq!(
            document.normalize_typing_payload(FormattedPayloadEdit::new(0..0, payload)),
            Err(DocumentError::WrongSnapshot {
                expected: before,
                actual: document.revision(),
            })
        );
        assert_eq!(document.source_bytes(), b"<p>AB</p>");
    }
}
