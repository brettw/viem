//! Source-local language annotations and disposable code-block presentation.
use super::*;

impl Document {
    /// Set a fenced code language against the exact document snapshot. Only the
    /// opening annotation changes; body text, delimiters and endings survive.
    pub fn prepare_code_block_language(
        &self,
        document: DocumentId,
        revision: Revision,
        at: usize,
        language: Option<&str>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if document != self.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: revision,
                actual: self.revision(),
            });
        }
        if self.format() != Format::Markdown || self.is_read_only() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        self.text_point(at)?;
        let language = language.filter(|value| !value.is_empty());
        if language.is_some_and(|value| {
            value.len() > 128
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_+.#-".contains(&byte))
        }) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let block = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| {
                block.style.0 == "Code Block" && block.range.start <= at && at <= block.range.end
            })
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let expected = language.map(super::super::syntax::detection::canonical_language);
        if block.code_language == expected {
            return Ok(self.no_op_prepared());
        }
        let Some(fence) = super::super::markdown_code::fenced_source(self, &block)? else {
            return self.prepare_with_fenced_indented_code(&[block], |doc| {
                doc.prepare_code_block_language(doc.id(), doc.revision(), at, language)
            });
        };
        let bytes = self
            .state()
            .source
            .bytes_in(fence.opening.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, fence.opening.start)?;
        let normalized = normalize(&decoded, self.file_format());
        let line = normalized.text.trim_end_matches('\n');
        let prefix = super::super::markdown_quotes::prefix(line);
        let prefix = prefix
            + super::super::markdown_blocks::marker_prefix_length(&line[prefix..]).unwrap_or(0);
        let prefix =
            prefix + line[prefix..].len() - line[prefix..].trim_start_matches([' ', '\t']).len();
        let info = prefix + fence.width;
        let start = info + line[info..].len() - line[info..].trim_start_matches([' ', '\t']).len();
        let end = if language.is_none() {
            line.len()
        } else {
            start + line[start..].split_whitespace().next().map_or(0, str::len)
        };
        let offset = |index| {
            self.encoding()
                .encode_fragment(&line[..index])
                .map(|bytes| fence.opening.start + bytes.len())
        };
        let patch = SourcePatch::primary(
            offset(start)?..offset(end)?,
            self.encoding().encode_fragment(language.unwrap_or(""))?,
        );
        let mut prepared = self.prepare_text_edits_with_patches(Vec::new(), Some(vec![patch]))?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let matched = candidate
            .projection
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| {
                block.style.0 == "Code Block" && block.range.start <= at && at <= block.range.end
            });
        if candidate.projection.text_tree().byte_len() != self.projection().text_tree().byte_len()
            || matched.is_none_or(|block| block.code_language != expected)
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        prepared.summary.kind = ModelChangeKind::SemanticStyle;
        Ok(prepared)
    }

    pub(crate) fn install_markdown_code_presentation(
        &mut self,
        sheet: std::sync::Arc<StyleSheet>,
        runs: &[super::super::syntax::SyntaxRun],
    ) {
        if self.format() != Format::Markdown {
            return;
        }
        let mut projection = self.state().projection.clone();
        projection.install_markdown_code_styles(sheet, runs);
        self.publish_code_presentation(projection);
    }
}
