//! Structural list gestures keep the affected Markdown markers sequential.
//! Loading or typing literal source does not canonicalize unrelated lists.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{Block, BlockKind, ListStyle};
use std::collections::{HashMap, HashSet};

pub(super) fn scope(request: &ModelRequest) -> Option<Range<usize>> {
    match request {
        ModelRequest::SetListStyle { range, .. }
        | ModelRequest::SetBlockQuote { range, .. }
        | ModelRequest::SetParagraphStyle { range, .. }
        | ModelRequest::AssignNamedStyle {
            range,
            namespace: super::super::StyleNamespace::Block,
            ..
        }
        | ModelRequest::IndentList { range, .. }
        | ModelRequest::DeleteLines { range, .. } => Some(range.clone()),
        ModelRequest::ContinueList { at, .. } | ModelRequest::OpenLine { at, .. } => Some(*at..*at),
        _ => None,
    }
}

fn selected(block: &Block, range: &Range<usize>) -> bool {
    if range.is_empty() {
        block.range.start <= range.start && range.start <= block.range.end
    } else {
        block.range.start < range.end && range.start <= block.range.end
    }
}

impl Document {
    pub(super) fn prepare_markdown_numbering(
        &self,
        prepared: PreparedModelTransaction,
        range: Range<usize>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if prepared.is_no_op() {
            return Ok(prepared);
        }
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Ok(prepared);
        };
        let map = |at, association, affinity| -> Result<usize, ModelTransactionError> {
            prepared
                .text_position_map()
                .map_text_point(
                    self.text_point(at)?,
                    association,
                    affinity,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )?
                .value()
                .map(|point| point.offset())
                .ok_or(DocumentError::AmbiguousProjection.into())
        };
        let mapped = map(
            range.start,
            Association::BeforeInsertion,
            BoundaryAffinity::Downstream,
        )?
            ..map(
                range.end,
                Association::AfterInsertion,
                BoundaryAffinity::Upstream,
            )?;
        let old_local = self
            .projection()
            .blocks_for_region(&range)
            .into_iter()
            .filter(|block| selected(block, &range))
            .collect::<Vec<_>>();
        let new_local = candidate
            .projection
            .blocks_for_region(&mapped)
            .into_iter()
            .filter(|block| selected(block, &mapped))
            .collect::<Vec<_>>();
        if !old_local
            .iter()
            .chain(&new_local)
            .any(|block| matches!(block.kind, BlockKind::ListItem { ordered: true, .. }))
        {
            return Ok(prepared);
        }
        let old_ids = old_local
            .iter()
            .map(|block| block.id)
            .collect::<HashSet<_>>();
        let new_ids = new_local
            .iter()
            .map(|block| block.id)
            .collect::<HashSet<_>>();
        let old_lists = self
            .projection()
            .list_structure()
            .lists
            .into_iter()
            .filter(|list| {
                list.style == ListStyle::Numbered
                    && list
                        .items
                        .iter()
                        .any(|item| item.paragraph_ids.iter().any(|id| old_ids.contains(id)))
            })
            .collect::<Vec<_>>();
        let affected = old_lists
            .iter()
            .flat_map(|list| &list.items)
            .flat_map(|item| item.paragraph_ids.iter().copied())
            .collect::<HashSet<_>>();
        let lists = candidate.projection.list_structure();
        let blocks = candidate
            .projection
            .blocks()
            .iter()
            .map(|block| (block.id, block))
            .collect::<HashMap<_, _>>();
        let mut patches = Vec::new();
        let mut indents = BTreeMap::new();
        for list in &lists.lists {
            if list.style != ListStyle::Numbered
                || !list.items.iter().any(|item| {
                    item.paragraph_ids
                        .iter()
                        .any(|id| affected.contains(id) || new_ids.contains(id))
                })
            {
                continue;
            }
            let mut start = list.start;
            if let Some(old) = old_lists.iter().find(|old| {
                old.items
                    .iter()
                    .any(|item| item.paragraph_id == list.items[0].paragraph_id)
            }) {
                // The first surviving run retains an authored non-one start.
                // A normal paragraph splitting off a later run restarts it.
                let first = blocks[&list.items[0].paragraph_id].range.start;
                let earlier_survivor = old.items.iter().any(|item| {
                    blocks.get(&item.paragraph_id).is_some_and(|block| {
                        block.range.start < first
                            && matches!(block.kind, BlockKind::ListItem { ordered: true, .. })
                    })
                });
                start = if earlier_survivor { 1 } else { old.start };
            }
            for (index, item) in list.items.iter().enumerate() {
                let number = start.saturating_add(index as u64);
                Self::markdown_number_patches(
                    candidate,
                    item,
                    &lists,
                    &blocks,
                    number,
                    &mut patches,
                    &mut indents,
                )?;
            }
        }
        for ((start, end), (old, delta)) in indents {
            if delta == 0 {
                continue;
            }
            let target = columns(&old)
                .checked_add_signed(delta)
                .ok_or(DocumentError::VerificationFailed)?;
            let prefix = resized_indent(&old, target);
            patches.push(SourcePatch::primary(
                start..end,
                candidate.encoding.encode_fragment(&prefix)?,
            ));
        }
        if patches.is_empty() {
            return Ok(prepared);
        }
        let kind = prepared.summary.kind;
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        for patch in prepared.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        formatted.record_formatted(&prepared)?;
        // Continue from the verified candidate in a private history. Its
        // original publication preconditions belong to the live document.
        scratch.next_revision = prepared
            .after_revision
            .0
            .checked_add(1)
            .ok_or(ModelTransactionError::RevisionExhausted)?;
        scratch.next_projected_block_id = prepared.next_projected_block_id_after;
        let PreparedPublication::State(state) = prepared.publication else {
            unreachable!()
        };
        scratch.history = super::super::history::History::transient(state);
        let edits = if self.format().is_source_view() {
            patches
                .iter()
                .map(|patch| {
                    let boundary = |at, affinity| {
                        scratch
                            .projection()
                            .map_source_boundary(scratch.revision(), at, affinity)
                            .map(|point| point.formatted_offset)
                            .map_err(|_| DocumentError::AmbiguousProjection)
                    };
                    let start = boundary(patch.range.start, BoundaryAffinity::Downstream)?;
                    let end = boundary(patch.range.end, BoundaryAffinity::Upstream)?;
                    let text = scratch
                        .encoding()
                        .decode_region(&patch.replacement, patch.range.start)?
                        .text;
                    Ok(TextEdit::new(start..end, text))
                })
                .collect::<Result<Vec<_>, DocumentError>>()?
        } else {
            Vec::new()
        };
        let renumbered = scratch.prepare_text_edits_with_patches(edits, Some(patches))?;
        let PreparedPublication::State(renumbered_state) = &renumbered.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        // Number changes may move source boundaries, but must not change the
        // ownership or styles of any paragraphs, continuation lines, or children.
        let shape = |block: &Block| {
            let mut kind = block.kind.clone();
            if let BlockKind::ListItem { ordinal, .. } = &mut kind {
                *ordinal = 0;
            }
            (kind, block.style.clone())
        };
        if scratch
            .projection()
            .blocks()
            .iter()
            .map(shape)
            .ne(renumbered_state.projection.blocks().iter().map(shape))
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        record(&mut scratch, renumbered, &mut sources, &mut formatted)?;
        let patches = sources.source_patches();
        let edits = formatted.formatted_edits();
        let mut result = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        let PreparedPublication::State(candidate) = &result.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        if candidate.projection.text() != scratch.text()
            || !candidate
                .projection
                .has_same_hard_line_structure(scratch.projection())
            || candidate
                .projection
                .blocks()
                .iter()
                .map(|block| (&block.range, &block.kind, &block.style))
                .ne(scratch
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (&block.range, &block.kind, &block.style)))
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        result.summary.kind = kind;
        Ok(result)
    }

    fn markdown_number_patches(
        state: &DocumentState,
        item: &crate::document::ListItemNode,
        lists: &crate::document::ListStructure,
        blocks: &HashMap<u64, &Block>,
        number: u64,
        patches: &mut Vec<SourcePatch>,
        indents: &mut BTreeMap<(usize, usize), (String, isize)>,
    ) -> Result<(), DocumentError> {
        let block = blocks[&item.paragraph_id];
        let at = state
            .projection
            .source_insertion_point(block.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = state
            .source_hard_lines
            .line_at_offset(at)
            .and_then(|index| state.source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = state
            .source
            .bytes_in(line.start..line.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let text = state.encoding.decode_region(&bytes, line.start)?.text;
        let quote = super::super::markdown_quotes::prefix(&text);
        let BlockKind::ListItem { level, .. } = block.kind else {
            return Err(DocumentError::AmbiguousProjection);
        };
        let (start, content) = ordered_marker_prefix(&text, level)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let digits = text[start..].bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !matches!(text.as_bytes().get(start + digits), Some(b'.' | b')')) {
            return Err(DocumentError::AmbiguousProjection);
        }
        let spelling = number.to_string();
        if text[start..start + digits] != spelling {
            let offset = line.start + state.encoding.encode_fragment(&text[..start])?.len();
            let length = state
                .encoding
                .encode_fragment(&text[start..start + digits])?
                .len();
            patches.push(SourcePatch::primary(
                offset..offset + length,
                state.encoding.encode_fragment(&spelling)?,
            ));
        }
        let content_columns = columns(&text[quote..content]);
        let new_prefix = format!(
            "{}{}{}",
            &text[quote..start],
            spelling,
            &text[start + digits..content]
        );
        let delta = columns(&new_prefix) as isize - content_columns as isize;
        if delta != 0 {
            let mut paragraphs = item.paragraph_ids.clone();
            let mut children = item.child_lists.clone();
            while let Some(id) = children.pop() {
                if let Some(list) = lists.lists.iter().find(|list| list.id == id) {
                    for child in &list.items {
                        paragraphs.extend_from_slice(&child.paragraph_ids);
                        children.extend_from_slice(&child.child_lists);
                    }
                }
            }
            let last = paragraphs
                .iter()
                .filter_map(|id| blocks.get(id))
                .max_by_key(|block| block.range.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let end = state
                .projection
                .source_range(last.range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?
                .end;
            let first_line = state
                .source_hard_lines
                .line_at_offset(line.start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let last_line = state
                .source_hard_lines
                .line_at_offset(end.saturating_sub(1).max(line.start))
                .ok_or(DocumentError::AmbiguousProjection)?;
            for index in first_line + 1..=last_line {
                let line = state
                    .source_hard_lines
                    .get(index)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = state
                    .source
                    .bytes_in(line.start..line.end)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let text = state.encoding.decode_region(&bytes, line.start)?.text;
                let quote = super::super::markdown_quotes::prefix(&text);
                let indent =
                    text[quote..].len() - text[quote..].trim_start_matches([' ', '\t']).len();
                let old = &text[quote..quote + indent];
                let width = columns(old);
                if width < content_columns || text[quote..].trim().is_empty() {
                    continue;
                }
                let start = line.start + state.encoding.encode_fragment(&text[..quote])?.len();
                let end = start + state.encoding.encode_fragment(old)?.len();
                indents
                    .entry((start, end))
                    .and_modify(|(_, change)| *change += delta)
                    .or_insert((old.to_owned(), delta));
            }
        }
        Ok(())
    }
}

fn ordered_marker_prefix(text: &str, level: u8) -> Option<(usize, usize)> {
    let mut at = 0;
    let mut marker = None;
    // One physical row can open several nested items, with quotes between
    // their markers. The parsed level bounds the prefixes that belong to
    // this item's marker rather than to its literal body.
    for depth in 0..=level {
        at += super::super::markdown_quotes::prefix(&text[at..]);
        let tail = &text[at..];
        let indent = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        if depth > 0 && columns(&tail[..indent]) > 3 {
            break;
        }
        let Some(content) = super::super::markdown_blocks::marker_prefix_length(tail) else {
            break;
        };
        let start = at + indent;
        let ordered = text.as_bytes()[start].is_ascii_digit();
        at += content;
        marker = Some((start, at, ordered));
    }
    marker.and_then(|(start, content, ordered)| ordered.then_some((start, content)))
}

fn columns(text: &str) -> usize {
    text.bytes().fold(0, |column, byte| {
        column + if byte == b'\t' { 4 - column % 4 } else { 1 }
    })
}

fn resized_indent(old: &str, target: usize) -> String {
    let mut prefix = String::new();
    let mut column = 0;
    for ch in old.chars() {
        let next = column + if ch == '\t' { 4 - column % 4 } else { 1 };
        if next > target {
            break;
        }
        prefix.push(ch);
        column = next;
    }
    prefix.extend(std::iter::repeat(' ').take(target - column));
    prefix
}

fn record(
    scratch: &mut Document,
    prepared: PreparedModelTransaction,
    sources: &mut PatchComposition,
    formatted: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    formatted.record_formatted(&prepared)?;
    scratch.commit_model_transaction(prepared)?;
    Ok(())
}
