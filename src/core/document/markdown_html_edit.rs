//! Reuse the HTML adapter's structural edit rules for passive HTML blocks.
//! Only the containing source block is prepared; its patches are translated
//! back into the Markdown transaction and verified by the Markdown projector.
use super::*;

impl Document {
    pub(super) fn prepare_markdown_html_text_edits(&self, edits: &[TextEdit]) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.format() != Format::Markdown || !edits.iter().any(|edit| self.projection().blocks_for_region(&edit.range).iter().any(|block| block.markdown_html)) { return Ok(None); }
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let syntax = super::super::markdown_syntax::Blocks::parse(&input.text).to_source(&input);
        let Some(first) = edits.first() else { return Ok(None); };
        let at = super::super::source_edit::insertion_point(self.projection(), first.range.start, None).ok_or(DocumentError::AmbiguousProjection)?;
        let Some(block) = syntax.blocks.iter().find(|block| matches!(block.role, super::super::markdown_syntax::BlockRole::Html) && block.range.start <= at && at <= block.range.end) else { return Ok(None); };
        let contributors = self.projection().provenance_contained_in_source(&block.range);
        let Some(start) = contributors.iter().map(|span| span.formatted.start).min() else { return Ok(None); };
        let Some(end) = contributors.iter().filter(|span| !input.endings.iter().any(|ending| ending.source == span.source && ending.source.end == block.range.end)).map(|span| span.formatted.end).max() else { return Ok(None); };
        let crosses = edits.iter().any(|edit| edit.range.start < start || edit.range.end > end);
        let source = self.state().source.bytes_in(block.range.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let mut scratch = Document::from_bytes_with_file_format(source, self.encoding(), Format::Html, self.file_format())?;
        // Sanitized HTML and visible comments can deliberately differ from the
        // standalone HTML adapter. Such text retains the Markdown path.
        if scratch.text() != &self.text()[start..end] { return Ok(None); }
        if crosses {
            scratch.apply_model_request(ModelRequest::SetFormat { document: scratch.id(), revision: scratch.revision(), target: Format::Markdown, operation: super::super::FormatOperation::Convert })?;
            if scratch.text() != &self.text()[start..end] { return Ok(None); }
            let mut replacement = scratch.source_bytes();
            if input.endings.iter().any(|ending| ending.source.end == block.range.end) && !replacement.ends_with(&self.encoding().encode_fragment(self.file_format().spelling())?) {
                replacement.extend(self.encoding().encode_fragment(self.file_format().spelling())?);
            }
            return self.prepare_markdown_supporting_patches(vec![SourcePatch::primary(block.range.clone(), replacement)], |doc| doc.prepare_text_edits(edits.to_vec())).map(Some);
        }
        let local = edits.iter().map(|edit| TextEdit::new(edit.range.start - start..edit.range.end - start, edit.replacement.clone())).collect();
        let prepared = scratch.prepare_text_edits(local)?;
        let mut composition = replacement::PatchComposition::new(scratch.source_byte_len());
        for patch in prepared.summary.source_patches.iter().rev() { composition.splice(patch.range.clone(), &patch.replacement); }
        let mut expected = scratch.text().to_owned();
        for edit in edits.iter().rev() { expected.replace_range(edit.range.start - start..edit.range.end - start, &edit.replacement); }
        if let PreparedPublication::State(candidate) = &prepared.publication {
            let mut support = Vec::new();
            for ((at, actual), wanted) in candidate.projection.text().char_indices().zip(expected.chars()) {
                if actual == '\u{a0}' && wanted == ' ' {
                    if let Some(range) = candidate.projection.source_range(at..at + actual.len_utf8()) {
                        support.push((range, self.encoding().encode_fragment("&#32;")?));
                    }
                }
            }
            // An emptied HTML line must not prematurely terminate its GFM HTML block.
            let decoded = self.encoding().decode(&candidate.source.bytes())?;
            let normalized = normalize(&decoded, self.file_format());
            for pair in normalized.endings.windows(2) {
                if normalized.text[pair[0].normalized.end..pair[1].normalized.start].trim().is_empty() {
                    support.push((pair[0].source.clone(), Vec::new()));
                }
            }
            support.sort_by_key(|(range, _)| range.start);
            for (range, bytes) in support.into_iter().rev() { composition.splice(range, &bytes); }
        }
        let patches = composition.patches().into_iter().map(|(range, bytes)| SourcePatch::primary(
            range.start + block.range.start..range.end + block.range.start, bytes,
        )).collect();
        self.prepare_text_edits_with_patches(edits.to_vec(), Some(patches)).map(Some)
    }
}
