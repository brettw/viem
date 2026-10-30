//! GitHub alerts are top-level quotes whose first line is a hidden `[!TYPE]`
//! marker. Choosing a kind quotes any unquoted selected blocks first, then
//! inserts or rewrites one marker per selected quote. Neither step changes
//! visible text, so each verifies the candidate's alert kinds instead.
use super::{
    structural_style, Document, DocumentError, ModelTransactionError, PreparedModelTransaction, SourcePatch, TextEdit,
};
use crate::document::{AlertKind, Block, ContainerIdentity, ContainerKind, Format};
use std::ops::Range;

/// The physical source lines of one top-level quote's opening.
struct AlertSource {
    /// The quote's first source line, where a new marker is inserted.
    first_line: usize,
    /// The line holding the body's first visible source point.
    body_line: usize,
    /// The marker line and the source range of its `TYPE` token.
    marker: Option<(usize, Range<usize>, AlertKind)>,
}

use crate::document::markdown_quotes::alert_marker_kind as marker_kind;

impl Document {
    fn top_level_quote(block: &Block) -> Option<ContainerIdentity> {
        let first = block.containers.first()?;
        (first.container.kind == ContainerKind::Quote).then_some(first.container.id)
    }

    /// Alerts cannot nest, so every selected block must be top-level prose
    /// or the body of a top-level quote, outside lists, tables and metadata.
    pub fn can_set_alert(&self, range: Range<usize>) -> bool {
        if self.format() != Format::Markdown || !self.is_modifiable() || self.validate_range(&range).is_err()
            || self.projection().range_intersects_table(&range) {
            return false;
        }
        let selected = structural_style::selected_blocks(self, &range);
        !selected.is_empty() && selected.iter().all(|block| {
            !block.is_front_matter()
                && block.containers.iter().filter(|member| member.container.kind == ContainerKind::Quote).count() <= 1
                && !block.containers.iter().any(|member| matches!(member.container.kind,
                    ContainerKind::List { .. } | ContainerKind::ListItem))
                && !matches!(block.kind, crate::document::BlockKind::ListItem { .. })
                && (block.quote_depth == 0 || Self::top_level_quote(block).is_some())
        })
    }

    /// The first paragraph of each distinct top-level quote that owns a
    /// selected block, walking back only through that quote's paragraphs.
    fn alert_target_blocks(&self, range: &Range<usize>) -> Result<Vec<Block>, DocumentError> {
        let mut targets: Vec<Block> = Vec::new();
        for block in structural_style::selected_blocks(self, range) {
            let Some(id) = Self::top_level_quote(&block) else { continue; };
            if targets.iter().any(|target| Self::top_level_quote(target) == Some(id)) { continue; }
            let mut current = block;
            while !current.containers[0].starts_here {
                let start = current.range.start;
                let at = start.saturating_sub(1);
                current = self.projection().blocks_for_region(&(at..at)).into_iter()
                    .filter(|candidate| candidate.id != current.id && candidate.range.end <= start
                        && Self::top_level_quote(candidate) == Some(id))
                    .max_by_key(|candidate| (candidate.range.start, candidate.range.end))
                    .ok_or(DocumentError::AmbiguousProjection)?;
            }
            targets.push(current);
        }
        Ok(targets)
    }

    fn decoded_source_line(&self, index: usize) -> Result<(Range<usize>, crate::document::encoding::DecodedText), DocumentError> {
        let line = self.state().source_hard_lines.get(index).ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(line.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        Ok((line, decoded))
    }

    /// Locate a top-level quote's opening from its first paragraph. Only the
    /// marker, blank quote rows and a fence opener can precede that body.
    fn alert_source(&self, first: &Block) -> Result<AlertSource, DocumentError> {
        let at = self.projection().provenance_for_region(&first.range).first().map(|span| span.source.start)
            .or_else(|| self.projection().provenance_touching(&first.range).iter()
                .find(|span| span.formatted == first.range).map(|span| span.source.start))
            .or_else(|| self.projection().source_insertion_point(first.range.start, true))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let body_line = self.state().source_hard_lines.line_at_offset(at).ok_or(DocumentError::AmbiguousProjection)?;
        let mut first_line = body_line;
        let mut marker = None;
        let (_, decoded) = self.decoded_source_line(body_line)?;
        if let Some((kind, token)) = marker_kind(&decoded.text[crate::document::markdown_quotes::prefix(&decoded.text)..]) {
            // An alert without a body row presents its empty body on the marker.
            let offset = crate::document::markdown_quotes::prefix(&decoded.text);
            let token = decoded.source_boundary(offset + token.start).ok_or(DocumentError::AmbiguousProjection)?
                ..decoded.source_boundary(offset + token.end).ok_or(DocumentError::AmbiguousProjection)?;
            return Ok(AlertSource { first_line, body_line, marker: Some((body_line, token, kind)) });
        }
        while first_line > 0 {
            let (_, decoded) = self.decoded_source_line(first_line - 1)?;
            let text = decoded.text.trim_end_matches(['\r', '\n']);
            if !text.trim_start_matches([' ', '\t']).starts_with('>') { break; }
            let prefix = crate::document::markdown_quotes::prefix(text);
            let body = &text[prefix..];
            if let Some((kind, token)) = marker_kind(body) {
                let token = decoded.source_boundary(prefix + token.start).ok_or(DocumentError::AmbiguousProjection)?
                    ..decoded.source_boundary(prefix + token.end).ok_or(DocumentError::AmbiguousProjection)?;
                marker = Some((first_line - 1, token, kind));
                first_line -= 1;
                break;
            }
            if !body.trim().is_empty() && crate::document::projection::markdown_fence(body).is_none() { break; }
            first_line -= 1;
        }
        Ok(AlertSource { first_line, body_line, marker })
    }

    /// The source line opening the alert whose first paragraph is `first`;
    /// a regional parse starting there sees the hidden marker itself.
    pub(super) fn alert_opening_line(&self, first: &Block) -> Option<usize> {
        let member = first.containers.first()?;
        (member.starts_here && member.container.alert.is_some()).then(|| self.alert_source(first).ok()).flatten()
            .map(|source| source.first_line)
    }

    /// Patches that change each target quote to `kind`, or to a plain quote.
    fn alert_marker_patches(&self, targets: &[Block], kind: Option<AlertKind>) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let mut patches = Vec::new();
        for first in targets {
            let source = self.alert_source(first)?;
            match (source.marker, kind) {
                (Some((_, token, current)), Some(kind)) if current != kind => patches.push(SourcePatch::primary(
                    token, self.encoding().encode_fragment(kind.marker_name())?)),
                (Some(_), Some(_)) | (None, None) => {}
                (None, Some(kind)) => {
                    let line = self.state().source_hard_lines.get(source.first_line).ok_or(DocumentError::AmbiguousProjection)?;
                    let marker = format!("> [!{}]{}", kind.marker_name(), self.file_format().spelling());
                    patches.push(SourcePatch::primary(line.start..line.start, self.encoding().encode_fragment(&marker)?));
                }
                (Some((line, _, _)), None) => patches.push(self.alert_marker_removal(line, source.body_line)?),
            }
        }
        Ok(patches)
    }

    /// Delete a marker line with its ending. When the empty body is presented
    /// on the marker line itself, keep that row as an empty quote row instead.
    fn alert_marker_removal(&self, line: usize, body_line: usize) -> Result<SourcePatch, ModelTransactionError> {
        let (range, decoded) = self.decoded_source_line(line)?;
        if body_line > line {
            let next = self.state().source_hard_lines.get(line + 1).ok_or(DocumentError::AmbiguousProjection)?;
            return Ok(SourcePatch::primary(range.start..next.start, Vec::new()));
        }
        let text = decoded.text.trim_end_matches(['\r', '\n']);
        let prefix = crate::document::markdown_quotes::prefix(text);
        let start = decoded.source_boundary(prefix).ok_or(DocumentError::AmbiguousProjection)?;
        let end = decoded.source_boundary(text.len()).ok_or(DocumentError::AmbiguousProjection)?;
        Ok(SourcePatch::primary(start..end, Vec::new()))
    }

    /// Markers of the alerts whose every paragraph a line deletion removes.
    /// A remaining marker would become an empty alert and absorb lazy text.
    pub(super) fn deleted_alert_marker_patches(&self, range: &Range<usize>) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        if self.format() != Format::Markdown { return Ok(Vec::new()); }
        let mut firsts = Vec::new();
        for first in self.projection().blocks_for_region(range) {
            let Some(member) = first.containers.first().filter(|member|
                member.starts_here && member.container.alert.is_some()) else { continue; };
            if first.range.start < range.start { continue; }
            let id = member.container.id;
            let mut last = first.clone();
            while !last.containers[0].ends_here && last.range.end <= range.end {
                let at = last.range.end + 1;
                let Some(next) = self.projection().blocks_for_region(&(at..at)).into_iter()
                    .find(|block| block.range.start >= last.range.end && block.id != last.id
                        && block.containers.first().is_some_and(|top| top.container.id == id)) else { break; };
                last = next;
            }
            if last.containers[0].ends_here && last.range.end <= range.end { firsts.push(first); }
        }
        self.alert_marker_patches(&firsts, None)
    }

    /// Markers of the alerts whose first paragraph loses its quote treatment.
    pub(super) fn unquoted_alert_marker_patches(&self, range: &Range<usize>) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        if self.format() != Format::Markdown { return Ok(Vec::new()); }
        let firsts = structural_style::selected_blocks(self, range).into_iter()
            .filter(|block| block.containers.first().is_some_and(|member|
                member.starts_here && member.container.alert.is_some()))
            .collect::<Vec<_>>();
        self.alert_marker_patches(&firsts, None)
    }

    /// Set every selected block's top-level quote to `kind`, quoting unquoted
    /// blocks first; `None` makes them plain quotes, removing alert markers.
    pub(super) fn prepare_markdown_alert(&self, range: Range<usize>, kind: Option<AlertKind>)
        -> Result<PreparedModelTransaction, ModelTransactionError>
    {
        self.validate_range(&range)?;
        if kind.is_some() && !self.can_set_alert(range.clone()) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let selected = structural_style::selected_blocks(self, &range);
        if selected.iter().any(|block| block.quote_depth == 0) {
            let quoted = self.prepare_markdown_quote_style_raw(range.clone(), true)?;
            let patches = quoted.summary.source_patches.clone();
            return self.prepare_markdown_supporting_patches(patches, |document|
                document.prepare_markdown_alert(range.clone(), kind));
        }
        let targets = self.alert_target_blocks(&range)?;
        let patches = self.alert_marker_patches(&targets, kind)?;
        if patches.is_empty() { return Ok(self.no_op_prepared()); }
        // Hidden marker syntax has no visible position of its own. The kind
        // reaches every paragraph of its quote, so each paragraph start
        // supplies part of the local verification window for unchanged text.
        let mut windows = Vec::new();
        for first in &targets {
            let id = first.containers[0].container.id;
            let mut block = first.clone();
            loop {
                windows.push(TextEdit::new(block.range.start..block.range.start, ""));
                if block.containers[0].ends_here { break; }
                let at = block.range.end + 1;
                let Some(next) = self.projection().blocks_for_region(&(at..at)).into_iter()
                    .find(|next| next.range.start >= block.range.end && next.id != block.id
                        && next.containers.first().is_some_and(|top| top.container.id == id)) else { break; };
                block = next;
            }
        }
        let prepared = self.prepare_text_edits_with_patch_policy(windows, Some(patches), true)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        let verified = structural_style::selected_blocks(&candidate, &range).iter().all(|block| {
            let quote = block.containers.first().filter(|member| member.container.kind == ContainerKind::Quote);
            match kind {
                Some(_) => quote.is_some_and(|member| member.container.alert == kind),
                None => block.quote_depth > 0 && quote.is_none_or(|member| member.container.alert.is_none()),
            }
        });
        if !verified { return Err(DocumentError::VerificationFailed.into()); }
        Ok(prepared)
    }
}


#[cfg(test)]
mod tests {
    use crate::document::{Document, Encoding, Format};

    #[test]
    fn formatted_serialization_writes_the_marker_before_the_first_paragraph() {
        let source = "> [!WARNING]\n> one\n>\n> two";
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let markdown = crate::document::markdown_serialization::markdown_document(document.projection());
        // The serializer spells separator rows with its own `> ` prefix.
        assert_eq!(markdown, "> [!WARNING]\n> one\n> \n> two");
    }
}
