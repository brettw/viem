//! Mixed source/display line deletion keeps visible editing semantics while
//! admitting explicitly addressed hidden source trivia at a physical edge.
use super::*;

impl Document {
    /// A mixed source/display transfer may reuse original source bytes only
    /// when they own complete formatted paragraphs and reproduce the captured
    /// fragment independently. A partial owner needs semantic reverse repairs,
    /// which raw source transfer/reordering cannot provide.
    pub fn addressed_source_fragment_is_self_contained(
        &self,
        text: Range<usize>,
        source: Range<usize>,
    ) -> Result<bool, DocumentError> {
        self.validate_range(&text)?;
        self.source_point(source.start)
            .map_err(|_| DocumentError::AmbiguousProjection)?;
        self.source_point(source.end)
            .map_err(|_| DocumentError::AmbiguousProjection)?;
        if source.start > source.end {
            return Err(DocumentError::AmbiguousProjection);
        }
        if self
            .projection()
            .blocks_for_region(&text)
            .iter()
            .any(|block| {
                block.range.start < text.start
                    || text.end < block.range.end
                    || !block.containers.is_empty()
                    || block.markdown_html
            })
            || self
                .projection()
                .style_spans_for_region(&text)
                .iter()
                .any(|span| span.range.start < text.start || text.end < span.range.end)
        {
            return Ok(false);
        }
        let bytes = self.source_bytes_in(source)?;
        let fragment = Document::from_bytes_with_file_format(
            bytes.clone(),
            self.encoding(),
            self.format(),
            self.file_format(),
        )?;
        let snapshot = self.hard_line_snapshot();
        let mut body = text.clone();
        if body.start < body.end {
            // A captured following paragraph boundary has no corresponding
            // final boundary in an independently parsed Markdown fragment.
            if snapshot
                .line_at_offset(snapshot.previous_grapheme_boundary(body.end)
                    .ok_or(DocumentError::AmbiguousProjection)?)
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .separator_range()
                .is_some_and(|separator| separator.end == body.end)
            {
                body.end -= 1;
            }
        }
        if snapshot
            .slice_utf8(body.clone())
            .map_err(DocumentError::FormattedTextStorage)?
            != fragment
                .hard_line_snapshot()
                .slice_utf8(0..fragment.projection().text_tree().byte_len())
                .map_err(DocumentError::FormattedTextStorage)?
        {
            return Ok(false);
        }
        let styles = |projection: &super::super::FormattedDocument, range: Range<usize>| {
            projection
                .style_spans_for_region(&range)
                .into_iter()
                .filter_map(|span| {
                    let start = span.range.start.max(range.start);
                    let end = span.range.end.min(range.end);
                    (start < end)
                        .then_some((start - range.start..end - range.start, span.application))
                })
                .collect::<Vec<_>>()
        };
        let fragment_length = fragment.projection().text_tree().byte_len();
        if styles(self.projection(), body.clone())
            != styles(fragment.projection(), 0..fragment_length)
        {
            return Ok(false);
        }
        let blocks = |projection: &super::super::FormattedDocument, range: Range<usize>| {
            projection
                .blocks_for_region(&range)
                .into_iter()
                .map(|block| {
                    (
                        block.range.start.max(range.start) - range.start
                            ..block.range.end.min(range.end) - range.start,
                        (
                            block.kind.clone(),
                            block.style.clone(),
                            block.quote_depth,
                            block.direct_formatting.clone(),
                            block.thematic_break,
                            block.code_language.clone(),
                        ),
                    )
                })
                .collect::<Vec<_>>()
        };
        if blocks(self.projection(), body) != blocks(fragment.projection(), 0..fragment_length) {
            return Ok(false);
        }
        // An unclosed passive inline scope can look correct at fragment EOF
        // yet consume later destination content. A plain following paragraph
        // must remain plain when parsed with the selected original bytes.
        const GUARD: &str = "ViemAddressBoundary";
        let mut guarded = bytes;
        guarded.extend(
            self.encoding()
                .encode_fragment(&format!("{0}{0}{GUARD}", self.file_format().spelling()))?,
        );
        let guarded = Document::from_bytes_with_file_format(
            guarded,
            self.encoding(),
            self.format(),
            self.file_format(),
        )?;
        let guard_end = guarded.projection().text_tree().byte_len();
        let Some(guard_start) = guard_end.checked_sub(GUARD.len()) else {
            return Ok(false);
        };
        let guard_range = guard_start..guard_end;
        if guarded
            .hard_line_snapshot()
            .slice_utf8(guard_range.clone())
            .map_err(DocumentError::FormattedTextStorage)?
            != GUARD
            || !guarded
                .projection()
                .style_spans_for_region(&guard_range)
                .is_empty()
        {
            return Ok(false);
        }
        Ok(guarded
            .projection()
            .blocks_for_region(&guard_range)
            .iter()
            .all(|block| {
                block.kind == super::super::BlockKind::Paragraph
                    && block.style.0 == "Paragraph"
                    && block.containers.is_empty()
                    && block.quote_depth == 0
                    && !block.markdown_html
            }))
    }

    pub(super) fn prepare_addressed_line_deletion(
        &self,
        range: Range<usize>,
        source: Range<usize>,
        physical_start: bool,
        physical_end: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        self.source_point(source.start)?;
        self.source_point(source.end)?;
        if source.start > source.end || physical_start == physical_end {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        // This is the same structural intention as Visual Line deletion. Its
        // reverse rules repair code/emphasis scopes, tables and paragraph edges
        // without allowing a physical endpoint to erase retained appearance.
        let mut prepared = self.prepare_line_deletion(range.clone())?;
        let visible = self.addressed_source_range(range.clone(), None, None)?;
        let hidden = if physical_start {
            source.start..visible.start.min(source.end)
        } else {
            visible.end.max(source.start)..source.end
        };
        if hidden.is_empty() {
            return Ok(prepared);
        }

        // The semantic patches may already consume the physical edge, or may
        // recreate syntax required by retained content. Remove only original
        // trivia still present there. Never remove supporting replacement bytes.
        let gaps = untouched_source_ranges(hidden, prepared.summary().source_patches());
        if gaps.is_empty() {
            return Ok(prepared);
        }
        let mut patches = prepared.summary().source_patches().to_vec();
        patches.extend(
            gaps.into_iter()
                .map(|range| SourcePatch::primary(range, Vec::new())),
        );
        patches.sort_by_key(|patch| (patch.range().start, patch.range().end));
        if let Ok(candidate) = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.clone(), "")],
            Some(patches),
            false,
        ) {
            // Removing an opener/divider can preserve text but change retained
            // formatting. Such bytes are necessary supporting syntax, rather
            // than disposable trivia; keep the semantic reverse-rule result.
            if addressed_candidates_match(self, &prepared, &candidate, range.start)? {
                prepared = candidate;
            }
        }
        Ok(prepared)
    }
}

fn untouched_source_ranges(range: Range<usize>, patches: &[SourcePatch]) -> Vec<Range<usize>> {
    let mut at = range.start;
    let mut result = Vec::new();
    for patch in patches {
        let selected = patch.range();
        if selected.end <= at || range.end <= selected.start {
            continue;
        }
        if at < selected.start {
            result.push(at..selected.start.min(range.end));
        }
        at = at.max(selected.end).min(range.end);
    }
    if at < range.end {
        result.push(at..range.end);
    }
    result
}

fn addressed_candidates_match(
    document: &Document,
    expected: &PreparedModelTransaction,
    candidate: &PreparedModelTransaction,
    at: usize,
) -> Result<bool, DocumentError> {
    let expected = match &expected.publication {
        PreparedPublication::State(state) => &state.projection,
        PreparedPublication::NoOp => document.projection(),
        PreparedPublication::History { .. } => return Err(DocumentError::VerificationFailed),
    };
    let candidate = match &candidate.publication {
        PreparedPublication::State(state) => &state.projection,
        PreparedPublication::NoOp => document.projection(),
        PreparedPublication::History { .. } => return Err(DocumentError::VerificationFailed),
    };
    if expected.text_tree().byte_len() != candidate.text_tree().byte_len()
        || expected.hard_line_count() != candidate.hard_line_count()
    {
        return Ok(false);
    }
    let blocks = expected.blocks_for_region(&(at..at));
    let start = blocks.first().map_or(at, |block| block.range.start);
    let end = blocks.last().map_or(at, |block| block.range.end);
    let region = start..end;
    if expected
        .text_tree()
        .slice(region.clone())
        .map_err(DocumentError::FormattedTextStorage)?
        != candidate
            .text_tree()
            .slice(region.clone())
            .map_err(DocumentError::FormattedTextStorage)?
    {
        return Ok(false);
    }
    let expected_blocks = expected.blocks_for_region(&region);
    let candidate_blocks = candidate.blocks_for_region(&region);
    if expected_blocks.len() != candidate_blocks.len()
        || !expected_blocks
            .iter()
            .zip(candidate_blocks.iter())
            .all(|(a, b)| a.range == b.range && a.attributes == b.attributes)
    {
        return Ok(false);
    }
    let clip = |projection: &super::super::FormattedDocument| {
        projection
            .style_spans_for_region(&region)
            .into_iter()
            .map(|mut span| {
                span.range = span.range.start.max(start)..span.range.end.min(end);
                span
            })
            .collect::<Vec<_>>()
    };
    Ok(clip(expected) == clip(candidate))
}
