//! Exact, local restoration for styled Replace input. A native text batch is
//! prepared on persistent scratch snapshots and published once. Its individual
//! grapheme frontiers retain inverse source patches, including generated syntax
//! and supporting table changes, rather than reconstructing original styling
//! from plain text during Backspace.
use super::*;
use crate::document::StylePropertyValue;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceEditRestoration {
    document: DocumentId,
    expected_revision: Revision,
    source_patches: Vec<SourcePatch>,
    text_edits: Vec<TextEdit>,
}

impl SourceEditRestoration {
    /// The immediately newer recorded frontier has just been restored exactly.
    /// Its predecessor now names these same bytes in the new revision. This is
    /// an explicit journal rebind, never a general stale-snapshot fallback.
    pub(crate) fn after_newer_frontier_restored(&mut self, revision: Revision) {
        self.expected_revision = revision;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedReplacement {
    pub before_cursor: usize,
    pub after_cursor: usize,
    pub next_target: usize,
    pub inserted: String,
    pub original: Option<String>,
    pub restoration: SourceEditRestoration,
}
#[derive(Clone)]
enum Fragment {
    Original(Range<usize>),
    Added(Vec<u8>),
}
impl Fragment {
    fn len(&self) -> usize {
        match self {
            Self::Original(r) => r.len(),
            Self::Added(v) => v.len(),
        }
    }
    fn slice(&self, r: Range<usize>) -> Self {
        match self {
            Self::Original(old) => Self::Original(old.start + r.start..old.start + r.end),
            Self::Added(v) => Self::Added(v[r].to_vec()),
        }
    }
}
pub(super) struct PatchComposition {
    fragments: Vec<Fragment>,
    original_len: usize,
}
impl PatchComposition {
    pub(super) fn new(len: usize) -> Self {
        Self {
            fragments: vec![Fragment::Original(0..len)],
            original_len: len,
        }
    }
    pub(super) fn splice(&mut self, range: Range<usize>, bytes: &[u8]) {
        let mut next = Vec::new();
        let mut offset = 0;
        let mut inserted = false;
        for fragment in &self.fragments {
            let end = offset + fragment.len();
            if end <= range.start {
                next.push(fragment.clone());
            } else if offset >= range.end {
                if !inserted {
                    next.push(Fragment::Added(bytes.to_vec()));
                    inserted = true;
                }
                next.push(fragment.clone());
            } else {
                if offset < range.start {
                    next.push(fragment.slice(0..range.start - offset));
                }
                if !inserted {
                    next.push(Fragment::Added(bytes.to_vec()));
                    inserted = true;
                }
                if end > range.end {
                    next.push(fragment.slice(range.end - offset..fragment.len()));
                }
            }
            offset = end;
        }
        if !inserted {
            next.push(Fragment::Added(bytes.to_vec()));
        }
        self.fragments = next;
    }
    pub(super) fn patches(self) -> Vec<(Range<usize>, Vec<u8>)> {
        let mut result = Vec::new();
        let mut original = 0;
        let mut added = Vec::new();
        for fragment in self.fragments {
            match fragment {
                Fragment::Added(bytes) => added.extend(bytes),
                Fragment::Original(range) => {
                    if original != range.start || !added.is_empty() {
                        result.push((original..range.start, std::mem::take(&mut added)));
                    }
                    original = range.end;
                }
            }
        }
        if original != self.original_len || !added.is_empty() {
            result.push((original..self.original_len, added));
        }
        result
    }

    /// Compose the verified formatted changes, including normalization and
    /// supporting edits that were not part of the caller's original payload.
    pub(super) fn record_formatted(
        &mut self,
        prepared: &PreparedModelTransaction,
    ) -> Result<(), DocumentError> {
        let PreparedPublication::State(state) = &prepared.publication else {
            return Ok(());
        };
        let mut delta = 0isize;
        let mut replacements = Vec::new();
        for splice in &prepared.summary.formatted_splices {
            let range = splice.old_range();
            let start = range
                .start
                .checked_add_signed(delta)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let length = splice.inserted_len();
            delta += length as isize - range.len() as isize;
            let text = state
                .projection
                .text_tree()
                .slice(start..start + length)
                .map_err(DocumentError::FormattedTextStorage)?;
            replacements.push((range, text));
        }
        for (range, text) in replacements.into_iter().rev() {
            self.splice(range, text.as_bytes());
        }
        Ok(())
    }

    /// Recover per-byte origin from the verified speculative source. A single
    /// composed patch may contain both new text and copied authored markup.
    pub(super) fn source_patches(
        self,
        candidate: &crate::document::source::SourceSnapshot,
    ) -> Result<Vec<SourcePatch>, DocumentError> {
        let mut delta = 0isize;
        self.patches()
            .into_iter()
            .map(|(range, bytes)| {
                let start = range
                    .start
                    .checked_add_signed(delta)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let end = start
                    .checked_add(bytes.len())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                delta += bytes.len() as isize - range.len() as isize;
                let generated = candidate
                    .generated_text_ranges(start..end)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                Ok(SourcePatch::primary(range, bytes).with_generated_text_ranges(generated))
            })
            .collect()
    }

    pub(super) fn formatted_edits(
        self,
        candidate: &Document,
    ) -> Result<Vec<TextEdit>, DocumentError> {
        let mut delta = 0isize;
        let named = candidate.encoding().encode_fragment("&nbsp;")?;
        self.patches()
            .into_iter()
            .map(|(range, bytes)| {
                let start = range
                    .start
                    .checked_add_signed(delta)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                delta += bytes.len() as isize - range.len() as isize;
                let text = String::from_utf8(bytes)
                    .expect("formatted composition retains valid UTF-8 boundaries");
                let mut edit = TextEdit::new(range, text);
                if candidate.format() == Format::Html {
                    edit.html_normalized = true;
                    for (at, ch) in edit
                        .replacement
                        .char_indices()
                        .filter(|(_, ch)| *ch == '\u{a0}')
                    {
                        let formatted = start + at..start + at + ch.len_utf8();
                        let spans = candidate.projection().provenance_for_region(&formatted);
                        if spans.iter().any(|span| {
                            span.formatted == formatted
                                && candidate
                                    .state()
                                    .source
                                    .range_is_generated_text(span.source.clone())
                                && candidate
                                    .state()
                                    .source
                                    .bytes_in(span.source.clone())
                                    .as_deref()
                                    == Some(named.as_slice())
                        }) {
                            edit.html_protective_spaces.push(at);
                        }
                    }
                }
                Ok(edit)
            })
            .collect()
    }
}

impl Document {
    #[cfg(test)]
    pub(crate) fn prepare_recorded_replacement(
        &self,
        cursor: usize,
        target: usize,
        input: &str,
        values: &[(StyleProperty, StylePropertyValue)],
        affinity: BoundaryAffinity,
    ) -> Result<(PreparedModelTransaction, Vec<RecordedReplacement>), ModelTransactionError> {
        self.prepare_recorded_replacement_with_typing_style(cursor, target, input, None, values, affinity)
    }

    pub(crate) fn prepare_recorded_replacement_with_typing_style(
        &self, cursor: usize, target: usize, input: &str, named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)], affinity: BoundaryAffinity,
    ) -> Result<(PreparedModelTransaction, Vec<RecordedReplacement>), ModelTransactionError> {
        if let Some(style) = named { self.validate_typing_named_style(style)?; }
        self.text_point(cursor)?;
        self.text_point(target)?;
        let mut scratch = self.scratch_document();
        let mut source = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let mut records = Vec::new();
        let mut cursor = cursor;
        let mut target = target;
        for inserted in input.graphemes(true) {
            let lines = scratch.hard_line_snapshot();
            let end = lines
                .grapheme_range_at(target)
                .filter(|range| {
                    lines
                        .line_at_offset(range.start)
                        .ok()
                        .and_then(|line| line.separator_range())
                        .as_ref()
                        != Some(range)
                })
                .map_or(target, |range| range.end);
            let original = if end > target {
                Some(
                    lines
                        .slice_utf8(target..end)
                        .map_err(DocumentError::FormattedTextStorage)?,
                )
            } else {
                None
            };
            let payload = FormattedTextPayload::new(&lines, inserted, vec![])
                .expect("journaled Replace excludes semantic hard breaks");
            let edit = scratch.normalize_typing_payload(
                FormattedPayloadEdit::new(target..end, payload).with_boundary_affinity(affinity),
            )?;
            let (prepared, after_cursor) = if values.is_empty() && named.is_none() {
                let prepared = scratch.prepare_formatted_payload_edits(vec![edit])?;
                let caret = map_after(&scratch, &prepared, target)?;
                (prepared, caret)
            } else {
                scratch.prepare_insertion_with_typing_style(edit, named, values)?
            };
            // Source-visible formatting inserts closing delimiters beyond the
            // presentation caret. The next Replace consumes the next original
            // item after those owned delimiters, preserving the generated pair.
            let next_target =
                if self.format().is_source_view() {
                    map_after(&scratch, &prepared, end)?
                } else {
                    after_cursor
                };
            let mut source_delta = 0isize;
            let mut inverse_source = Vec::new();
            for patch in prepared.summary.source_patches.iter() {
                let range = patch.range();
                let start = (range.start as isize + source_delta) as usize;
                source_delta += patch.replacement().len() as isize - range.len() as isize;
                let origins = scratch
                    .state()
                    .source
                    .generated_text_ranges(range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let old = scratch
                    .state()
                    .source
                    .bytes_in(range)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                inverse_source.push(
                    SourcePatch::primary(start..start + patch.replacement().len(), old)
                        .with_generated_text_ranges(origins),
                );
            }
            let mut text_delta = 0isize;
            let mut inverse_text = Vec::new();
            let mut replacements = Vec::new();
            if let PreparedPublication::State(state) = &prepared.publication {
                for splice in prepared.summary.formatted_splices.iter() {
                    let range = splice.old_range();
                    let start = (range.start as isize + text_delta) as usize;
                    text_delta += splice.inserted_len() as isize - range.len() as isize;
                    let old = scratch
                        .projection()
                        .text_tree()
                        .slice(range.clone())
                        .map_err(DocumentError::FormattedTextStorage)?;
                    inverse_text.push(TextEdit::new(start..start + splice.inserted_len(), old));
                    let new = state
                        .projection
                        .text_tree()
                        .slice(start..start + splice.inserted_len())
                        .map_err(DocumentError::FormattedTextStorage)?;
                    replacements.push((range, new.into_bytes()));
                }
            }
            for patch in prepared.summary.source_patches.iter().rev() {
                source.splice(patch.range(), patch.replacement());
            }
            for (range, bytes) in replacements.into_iter().rev() {
                formatted.splice(range, &bytes);
            }
            let expected_revision = prepared.after_revision();
            records.push(RecordedReplacement {
                before_cursor: cursor,
                after_cursor,
                next_target,
                inserted: inserted.to_owned(),
                original,
                restoration: SourceEditRestoration {
                    document: self.id(),
                    expected_revision,
                    source_patches: inverse_source,
                    text_edits: inverse_text,
                },
            });
            scratch.commit_model_transaction(prepared)?;
            cursor = after_cursor;
            target = next_target;
        }
        let source_patches = source.source_patches(&scratch.state().source)?;
        let text_edits = formatted.formatted_edits(&scratch)?;
        let prepared = self.prepare_text_edits_with_patches(text_edits, Some(source_patches))?;
        if let Some(last) = records.last_mut() {
            last.restoration.expected_revision = prepared.after_revision();
        }
        Ok((prepared, records))
    }

    pub(crate) fn restore_recorded_replacement(
        &mut self,
        restoration: &SourceEditRestoration,
    ) -> Result<(), ModelTransactionError> {
        if restoration.document != self.id() {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id(),
                actual: restoration.document,
            });
        }
        if restoration.expected_revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: restoration.expected_revision,
                actual: self.revision(),
            });
        }
        let prepared = self.prepare_text_edits_with_patches(
            restoration.text_edits.clone(),
            Some(restoration.source_patches.clone()),
        )?;
        self.commit_model_transaction(prepared)?;
        Ok(())
    }
}

fn map_after(
    document: &Document,
    prepared: &PreparedModelTransaction,
    offset: usize,
) -> Result<usize, ModelTransactionError> {
    prepared
        .text_position_map()
        .map_text_point(
            document.text_point(offset)?,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )?
        .value()
        .map(|point| point.offset())
        .ok_or(DocumentError::AmbiguousProjection.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, FontSlant};
    #[test]
    fn composed_patch_preserves_mixed_source_origin_when_authored_bytes_are_copied() {
        let original = crate::document::source::SourceSnapshot::new(b"&nbsp;".to_vec());
        let candidate = original
            .replace_generated_text(0, 0, b"&nbsp;".to_vec())
            .unwrap();
        let mut composition = PatchComposition::new(original.len());
        composition.splice(0..original.len(), &candidate.bytes());
        let patches = composition.source_patches(&candidate).unwrap();
        let replayed = apply_source_patches(&original, &patches).unwrap();
        assert_eq!(replayed.bytes(), b"&nbsp;&nbsp;");
        assert_eq!(replayed.generated_text_ranges(0..12), Some(vec![0..6]));
        assert!(!replayed.range_is_generated_text(6..12));
    }

    #[test]
    fn recorded_replacement_rejects_stale_restore_without_touching_source() {
        let mut document =
            Document::from_bytes(b"<p>word</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let (prepared, records) = document
            .prepare_recorded_replacement(
                0,
                0,
                "a",
                &[(
                    StyleProperty::CharacterSlant,
                    StylePropertyValue::FontSlant(FontSlant::Italic),
                )],
                BoundaryAffinity::Downstream,
            )
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        document.insert(4, "x").unwrap();
        let bytes = document.source_bytes();
        assert!(matches!(
            document.restore_recorded_replacement(&records[0].restoration),
            Err(ModelTransactionError::StaleRevision { .. })
        ));
        assert_eq!(document.source_bytes(), bytes);
    }
    #[test]
    fn recorded_replacement_keeps_projection_work_local_in_large_document() {
        let mut source = "<p>line</p>".repeat(10_000);
        source.push_str("<p><i>word</i></p>");
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        let at = document.projection().text_tree().byte_len() - 4;
        let (prepared, _) = document
            .prepare_recorded_replacement(
                at,
                at,
                "a",
                &[(
                    StyleProperty::CharacterSlant,
                    StylePropertyValue::FontSlant(FontSlant::Italic),
                )],
                BoundaryAffinity::Downstream,
            )
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert!(prepared.summary().source_patches()[0].replacement().len() <= 32);
        assert!(
            prepared.summary().projection_work().projected_hard_lines() <= 2,
            "{:?}",
            prepared.summary().projection_work()
        );
        assert_eq!(
            prepared
                .summary()
                .projection_work()
                .full_text_bytes_materialized(),
            0
        );
    }

    #[test]
    fn trailing_space_replacement_keeps_protection_and_projection_work_local() {
        let source = format!("{}<p>AB</p>", "<p>line</p>".repeat(10_000));
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        let at = document.projection().text_tree().byte_len() - 1;
        let (prepared, _) = document
            .prepare_recorded_replacement(at, at, " ", &[], BoundaryAffinity::Downstream)
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(
            prepared.summary().source_patches()[0].replacement(),
            b"&nbsp;"
        );
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert!(work.decoded_source_bytes() < 256, "{work:?}");
        assert_eq!(work.full_text_bytes_materialized(), 0);
    }
}
