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
    pub exit_source: Option<usize>,
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

    /// Emit the composed minimal source changes.
    pub(super) fn source_patches(self) -> Vec<SourcePatch> {
        self.patches()
            .into_iter()
            .map(|(range, bytes)| SourcePatch::primary(range, bytes))
            .collect()
    }

    pub(super) fn formatted_edits(self) -> Vec<TextEdit> {
        self.patches()
            .into_iter()
            .map(|(range, bytes)| {
                let text = String::from_utf8(bytes)
                    .expect("formatted composition retains valid UTF-8 boundaries");
                TextEdit::new(range, text)
            })
            .collect()
    }
}

impl Document {

    pub(crate) fn prepare_recorded_replacement_with_typing_style(
        &self,
        cursor: usize,
        target: usize,
        input: &str,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        affinity: BoundaryAffinity,
        link_disabled: bool,
        autodetect: bool,
        literal: bool,
        exit: Option<crate::document::SourcePoint>,
    ) -> Result<(PreparedModelTransaction, Vec<RecordedReplacement>), ModelTransactionError> {
        if let Some(style) = named {
            self.validate_typing_named_style(style)?;
        }
        self.text_point(cursor)?;
        self.text_point(target)?;
        let mut scratch = self.scratch_document();
        let mut source = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let mut records = Vec::new();
        let mut cursor = cursor;
        let mut target = target;
        let mut affinity = affinity;
        let mut exit_source = exit.map(|point| point.offset());
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
            let edit =
                FormattedPayloadEdit::new(target..end, payload).with_boundary_affinity(affinity);
            let (prepared, after_cursor, closed) = if link_disabled {
                let (prepared, caret, _) = scratch.prepare_insertion_without_link(edit, named, values, None)?;
                (prepared, caret, None)
            } else if self.format() == Format::Markdown && (autodetect || literal) {
                let (prepared, caret, _, closed) = scratch.prepare_markdown_typing_grapheme(edit, named, values, None, literal, exit_source.map(|at| scratch.source_point(at)).transpose()?)?;
                (prepared, caret, closed)
            } else if values.is_empty() && named.is_none() {
                let prepared = scratch.prepare_formatted_payload_edits(vec![edit])?;
                let caret = map_after(&scratch, &prepared, target)?;
                (prepared, caret, None)
            } else {
                let (prepared, caret) = scratch.prepare_insertion_with_typing_style(edit, named, values)?;
                (prepared, caret, None)
            };
            // Source-visible formatting inserts closing delimiters beyond the
            // presentation caret. The next Replace consumes the next original
            // item after those owned delimiters, preserving the generated pair.
            let next_target = if self.format().is_source_view() {
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
                let old = scratch
                    .state()
                    .source
                    .bytes_in(range)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                inverse_source.push(SourcePatch::primary(
                    start..start + patch.replacement().len(),
                    old,
                ));
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
                exit_source: closed,
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
            exit_source = closed;
            affinity = if exit_source.is_some() { BoundaryAffinity::Downstream } else { BoundaryAffinity::Upstream };
        }
        let source_patches = source.source_patches();
        let text_edits = formatted.formatted_edits();
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
