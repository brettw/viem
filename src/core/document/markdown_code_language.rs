//! Source-local language annotations and disposable code-block presentation.
use super::*;
use crate::document::Block;

impl Document {
    /// Set a code language against the exact document snapshot. Only the
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
        if block.markdown_html {
            let patches = self.passive_pre_language_patches(&block, language)?;
            if patches.is_empty() {
                return Ok(self.no_op_prepared());
            }
            return self.prepare_code_language_patches(&block, at, expected, patches);
        }
        if block.code_language == expected {
            return Ok(self.no_op_prepared());
        }
        let Some(fence) = super::super::markdown_code::fenced_source(self, &block)? else {
            if super::super::markdown_indented_code::source_block(self, &block)?.is_none() {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let mut prepared = self
                .prepare_with_fenced_indented_code(std::slice::from_ref(&block), |doc| {
                    doc.prepare_code_block_language(doc.id(), doc.revision(), at, language)
                })?;
            prepared.layout_invalidation = Some(block.range);
            return Ok(prepared);
        };
        let bytes = self
            .state()
            .source
            .bytes_in(fence.opening.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, fence.opening.start)?;
        let normalized = normalize(&decoded, self.file_format());
        let line = normalized.text.trim_end_matches('\n');
        let prefix = super::super::markdown_quotes::container_prefix(line);
        let info = prefix + fence.width;
        let start = if language.is_none() { info } else {
            info + line[info..].len() - line[info..].trim_start_matches([' ', '\t']).len()
        };
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
        self.prepare_code_language_patches(&block, at, expected, vec![patch])
    }

    fn prepare_code_language_patches(
        &self,
        block: &Block,
        at: usize,
        expected: Option<String>,
        patches: Vec<SourcePatch>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut prepared = self.prepare_text_edits_with_patches(Vec::new(), Some(patches))?;
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
            || matched.is_none_or(|matched| {
                matched.code_language != expected
                    || matched.range != block.range
                    || matched.kind != block.kind
                    || matched.quote_depth != block.quote_depth
            })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        prepared.summary.kind = ModelChangeKind::SemanticStyle;
        // Only the opening annotation changed; complete source verification
        // retained the visible body and topology. Retire this owner's geometry
        // while keeping unrelated measured heights and regional cache entries.
        prepared.layout_invalidation = Some(block.range.clone());
        Ok(prepared)
    }

    fn passive_pre_language_patches(
        &self,
        block: &Block,
        language: Option<&str>,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        use super::super::{
            html, markdown_blocks, markdown_quotes, markdown_syntax, projection, rich_text,
        };

        let source_at = self
            .projection()
            .source_insertion_point(block.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let normalized_at = input
            .unit(input.unit_partition_point(|unit| unit.source.end <= source_at))
            .map_or(input.text.len(), |unit| unit.normalized.start);
        let syntax = markdown_syntax::Blocks::parse(&markdown_syntax::grammar_text(&input));
        let region = syntax
            .blocks
            .iter()
            .find(|candidate| {
                matches!(candidate.role, markdown_syntax::BlockRole::Html)
                    && candidate.range.start <= normalized_at
                    && normalized_at <= candidate.range.end
            })
            .map(|candidate| candidate.range.clone())
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let quotes = markdown_quotes::classify(&input);
        let lists = markdown_blocks::classify_with_syntax(&input, &syntax.to_source(&input));
        let mut excluded = Vec::new();
        for (index, row) in super::super::paragraph_flow::source_lines(&input)
            .into_iter()
            .enumerate()
        {
            if row.end < region.start {
                continue;
            }
            if row.start >= region.end {
                break;
            }
            let content = quotes[index].content_start.max(
                lists[index]
                    .as_ref()
                    .map_or(row.start, |list| list.content_start),
            );
            let start = row.start.max(region.start);
            let end = content.min(region.end);
            if start < end {
                excluded.push(start - region.start..end - region.start);
            }
        }
        let owners = projection::markdown_html_pre_owners(&input.text[region.clone()], &excluded);
        let mut matching = owners.iter().filter(|owner| {
            region.start + owner.range.start <= normalized_at
                && normalized_at <= region.start + owner.range.end
        });
        let owner = matching
            .next()
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if matching.next().is_some() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let opening = owner.opening.clone();
        let excluded: Vec<_> = excluded
            .into_iter()
            .filter_map(|range| {
                let start = range.start.max(opening.start);
                let end = range.end.min(opening.end);
                (start < end).then(|| start - opening.start..end - opening.start)
            })
            .collect();
        let raw = &input.text[region.start + opening.start..region.start + opening.end];
        let removed = html::without_ranges(raw, &excluded);
        let tokens = html::tokenize(&removed.text);
        let [html::Token {
            range,
            kind: html::TokenKind::Tag(tag),
        }] = tokens.as_slice()
        else {
            return Err(DocumentError::UnsupportedFormatting.into());
        };
        if range != &(0..removed.text.len()) || tag.end || tag.name != "pre" {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let attributes: Vec<_> = tag
            .attribute_ranges
            .iter()
            .filter(|(name, _)| name == "lang")
            .collect();
        // None removes authored empty/boolean annotations too. A same-language
        // choice preserves an already meaningful alternate spelling.
        if language.is_some()
            && block.code_language
                == language.map(super::super::syntax::detection::canonical_language)
        {
            return Ok(Vec::new());
        }
        let mut changes = Vec::new();
        if attributes.is_empty() {
            if let Some(language) = language {
                changes.push((4..4, format!(" lang=\"{language}\"")));
            }
        } else {
            for (_, value) in attributes {
                let bytes = removed.text.as_bytes();
                let mut key_end = value.start;
                while key_end > 4 && bytes[key_end - 1].is_ascii_whitespace() {
                    key_end -= 1;
                }
                let assigned = key_end > 4 && bytes[key_end - 1] == b'=';
                if assigned {
                    key_end -= 1;
                    while key_end > 4 && bytes[key_end - 1].is_ascii_whitespace() {
                        key_end -= 1;
                    }
                } else if !value.is_empty() {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                let key_start = key_end
                    .checked_sub(4)
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                if !removed.text[key_start..key_end].eq_ignore_ascii_case("lang")
                    || key_start <= 4
                    || !bytes[key_start - 1].is_ascii_whitespace()
                {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                if let Some(language) = language {
                    let mut content = value.clone();
                    let replacement = if value.is_empty() {
                        format!("{}\"{language}\"", if assigned { "" } else { "=" })
                    } else if matches!(bytes[value.start], b'\'' | b'"') {
                        if value.len() < 2 || bytes[value.end - 1] != bytes[value.start] {
                            return Err(DocumentError::UnsupportedFormatting.into());
                        }
                        content = value.start + 1..value.end - 1;
                        language.to_owned()
                    } else {
                        if bytes[value.clone()]
                            .iter()
                            .any(|byte| b"\"'`=<>".contains(byte))
                        {
                            return Err(DocumentError::UnsupportedFormatting.into());
                        }
                        language.to_owned()
                    };
                    changes.push((content, replacement));
                    break; // HTML uses the first duplicate attribute.
                } else {
                    let mut start = key_start;
                    while start > 4 && matches!(bytes[start - 1], b' ' | b'\t') {
                        start -= 1;
                    }
                    changes.push((start..value.end, String::new()));
                }
            }
        }
        let mapper = rich_text::Builder::new(&input, self.revision());
        changes
            .into_iter()
            .map(|(range, replacement)| {
                let original = removed.original_range(range);
                if excluded
                    .iter()
                    .any(|prefix| prefix.start < original.end && original.start < prefix.end)
                {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                let source = mapper.source_range(
                    region.start + opening.start + original.start
                        ..region.start + opening.start + original.end,
                );
                Ok(SourcePatch::primary(
                    source,
                    self.encoding().encode_fragment(&replacement)?,
                ))
            })
            .collect()
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
