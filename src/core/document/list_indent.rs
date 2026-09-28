//! Structural one-level list indentation. Presentation reads parsed structure;
//! execution additionally prepares and verifies the complete source transaction.
use super::super::{Block, BlockKind};
use super::*;

struct Targets {
    first: usize,
    end: usize,
    roots: Vec<usize>,
    previous: Option<usize>,
    parent: Option<usize>,
}

fn level(block: &Block) -> Option<u8> {
    match block.kind {
        BlockKind::ListItem { level, .. } => Some(level),
        _ => None,
    }
}
fn item_start(block: &Block) -> bool {
    block.list_editing.item_start
}

fn targets(
    blocks: &[Block],
    range: &Range<usize>,
    unindent: bool,
) -> Result<Targets, DocumentError> {
    let selected = blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            if range.is_empty() {
                block.range.start <= range.start
                    && range.start <= block.range.end
                    && (range.start < block.range.end
                        || !blocks.iter().any(|next| {
                            next.range.start == range.start && next.range.start != block.range.start
                        }))
            } else {
                block.range.start < range.end && range.start < block.range.end
            }
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let first_selected = *selected
        .first()
        .ok_or(DocumentError::UnsupportedFormatting)?;
    if selected
        .iter()
        .any(|index| level(&blocks[*index]).is_none())
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    let base = level(&blocks[first_selected]).unwrap();
    let first = (0..=first_selected)
        .rev()
        .find(|index| level(&blocks[*index]) == Some(base) && item_start(&blocks[*index]))
        .ok_or(DocumentError::UnsupportedFormatting)?;
    let last = *selected.last().unwrap();
    if blocks[first..=last]
        .iter()
        .any(|block| level(block).map_or(true, |level| level < base))
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    let roots = (first..=last)
        .filter(|index| level(&blocks[*index]) == Some(base) && item_start(&blocks[*index]))
        .collect::<Vec<_>>();
    let end = (last + 1..blocks.len())
        .find(|index| {
            level(&blocks[*index]).map_or(true, |level| {
                level < base || level == base && item_start(&blocks[*index])
            })
        })
        .unwrap_or(blocks.len());
    let previous = (0..first)
        .rev()
        .find(|index| {
            level(&blocks[*index]).map_or(true, |level| {
                level < base || level == base && item_start(&blocks[*index])
            })
        })
        .filter(|index| level(&blocks[*index]) == Some(base));
    let parent = (0..first)
        .rev()
        .find(|index| level(&blocks[*index]).map_or(true, |level| level < base))
        .filter(|index| level(&blocks[*index]) == base.checked_sub(1));
    if unindent {
        if base == 0 || parent.is_none() {
            return Err(DocumentError::UnsupportedFormatting);
        }
    } else if previous.is_none()
        || blocks[first..end]
            .iter()
            .any(|block| level(block).is_some_and(|level| level >= 3))
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    Ok(Targets {
        first,
        end,
        roots,
        previous,
        parent,
    })
}

impl Document {
    pub fn list_indent_capabilities(&self, range: Range<usize>) -> (bool, bool) {
        if !(self.format().is_wysiwyg() || self.format().is_source_view())
            || self.validate_range(&range).is_err()
        {
            return (false, false);
        }
        let blocks = self
            .projection()
            .list_indentation_blocks(self.format().is_source_view());
        let Some(mut selected) = blocks.index_touching_point(range.start) else {
            return (false, false);
        };
        // A nonempty half-open selection excludes the block ending at its
        // start. Empty carets retain the downstream boundary owner.
        if !range.is_empty()
            && blocks
                .get(selected)
                .is_some_and(|block| block.range.end == range.start)
        {
            selected += 1;
        }
        let Some(current) = blocks.get(selected) else {
            return (false, false);
        };
        let Some(base) = level(&current).map(|level| u16::from(level) + 1) else {
            return (false, false);
        };
        let last = if range.is_empty() {
            selected + 1
        } else {
            blocks.partition_point_start(range.end)
        };
        if selected >= last {
            return (false, false);
        }
        let lower = |summary: super::super::range_index::NavigationSummary| summary.minimum < base;
        let boundary = |summary: super::super::range_index::NavigationSummary| {
            lower(summary) || summary.minimum_start <= base
        };
        let Some(first) = blocks.find_navigation(0..selected + 1, true, boundary) else {
            return (false, false);
        };
        if blocks
            .get(first)
            .is_none_or(|block| level(&block).map(|level| u16::from(level) + 1) != Some(base))
            || blocks.find_navigation(first..last, false, lower).is_some()
        {
            return (false, false);
        }
        let end = blocks
            .find_navigation(last..blocks.len(), false, boundary)
            .unwrap_or(blocks.len());
        let previous = blocks
            .find_navigation(0..first, true, boundary)
            .and_then(|index| blocks.get(index))
            .is_some_and(|block| level(&block).map(|level| u16::from(level) + 1) == Some(base));
        let parent = base > 1
            && blocks
                .find_navigation(0..first, true, lower)
                .and_then(|index| blocks.get(index))
                .is_some_and(|block| {
                    level(&block).map(|level| u16::from(level) + 1) == Some(base - 1)
                });
        let indent = previous
            && blocks
                .find_navigation(first..end, false, |summary| {
                    summary.maximum >= 4 || summary.flags & 1 != 0
                })
                .is_none();
        let unindent = parent
            && blocks
                .find_navigation(first..end, false, |summary| summary.flags & 2 != 0)
                .is_none();
        (indent, unindent)
    }

    pub fn prepare_list_indent(
        &self,
        range: Range<usize>,
        unindent: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if self.format().is_source_view() {
            let visible = Document::from_bytes_with_file_format(
                self.source_bytes(),
                self.encoding(),
                Format::Markdown,
                self.file_format(),
            )?;
            let raw = self
                .projection()
                .source_range(range.clone())
                .or_else(|| {
                    self.projection()
                        .source_insertion_point(range.start, true)
                        .map(|at| at..at)
                })
                .ok_or(DocumentError::AmbiguousProjection)?;
            let start = visible.visible_point_for_source(raw.start, true)?;
            let end = if range.is_empty() {
                start
            } else {
                visible.visible_point_for_source(raw.end, false)?
            };
            let prepared = visible.prepare_list_indent(start..end, unindent)?;
            return self
                .prepare_visible_source_patches(prepared.summary().source_patches().to_vec());
        }
        if !self.format().is_wysiwyg() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let blocks = self.projection().blocks();
        let target = targets(blocks, &range, unindent)?;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let patches = markdown_patches(self, &input, blocks, &target, unindent)?;
        let prepared = self.prepare_source_only_patches(patches)?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let after = &candidate.projection;
        if after.text() != self.text()
            || !after.has_same_hard_line_structure(self.projection())
            || after.blocks().len() != blocks.len()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        verify_character_styles(self.projection(), after)?;
        for (index, (before, after)) in blocks.iter().zip(after.blocks()).enumerate() {
            let expected = level(before).map(|level| {
                if (target.first..target.end).contains(&index) {
                    if unindent {
                        level - 1
                    } else {
                        level + 1
                    }
                } else {
                    level
                }
            });
            let ordered = |block: &Block| match block.kind {
                BlockKind::ListItem { ordered, .. } => Some(ordered),
                _ => None,
            };
            let paragraph = before.direct_paragraph.clone();

            if before.range != after.range
                || expected != level(after)
                || ordered(before) != ordered(after)
                || paragraph != after.direct_paragraph
                || before.direct_default_character != after.direct_default_character
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        Ok(prepared)
    }
}

fn verify_character_styles(
    before: &FormattedDocument,
    after: &FormattedDocument,
) -> Result<(), DocumentError> {
    let mut boundaries = BTreeSet::from([0]);
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.insert(span.range.start);
        boundaries.insert(span.range.end);
    }
    let state = |projection: &FormattedDocument, at: usize| {
        let mut direct = CharacterProperties::default();
        let mut named = None;
        let mut semantic = [false; 3];
        for span in projection.style_spans_for_region(&(at..at + 1)) {
            if !span.range.contains(&at) {
                continue;
            }
            match span.application {
                StyleApplication::Direct(properties) => {
                    super::super::rich_text::overlay(&mut direct, &properties)
                }
                StyleApplication::Named(id) => named = Some(id),
                StyleApplication::Semantic(style) => {
                    semantic[match style {
                        SemanticInlineStyle::Strong => 0,
                        SemanticInlineStyle::Emphasis => 1,
                        SemanticInlineStyle::Code => 2,
                    }] = true
                }
                _ => {}
            }
        }
        (direct, named, semantic)
    };
    for at in boundaries
        .into_iter()
        .filter(|at| *at < before.text().len())
    {
        if state(before, at) != state(after, at) {
            return Err(DocumentError::VerificationFailed);
        }
    }
    Ok(())
}

fn block_source_at(document: &Document, block: &Block) -> Result<usize, DocumentError> {
    document
        .projection()
        .source_insertion_point(block.range.start, true)
        .ok_or(DocumentError::AmbiguousProjection)
}

fn markdown_patches(
    document: &Document,
    input: &super::super::line_endings::NormalizedText,
    blocks: &[Block],
    target: &Targets,
    unindent: bool,
) -> Result<Vec<SourcePatch>, DocumentError> {
    let quotes = super::super::markdown_quotes::classify(input);
    let stripped = super::super::markdown_quotes::strip(input, &quotes);
    let input = &stripped;
    let lines = super::super::paragraph_flow::source_lines(input);
    let contexts = super::super::markdown_blocks::classify(input);
    let converter = super::super::rich_text::Builder::new(input, Revision(0));
    let line_for = |block: &Block| -> Result<usize, DocumentError> {
        let at = block_source_at(document, block)?;
        lines
            .iter()
            .position(|line| {
                let source = converter.source_range(line.clone());
                source.start <= at && at <= source.end
            })
            .ok_or(DocumentError::AmbiguousProjection)
    };
    let start = line_for(&blocks[target.first])?;
    let end = if target.end < blocks.len() {
        line_for(&blocks[target.end])?
    } else {
        lines.len()
    };
    let columns = |text: &str| {
        text.bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .fold(0usize, |column, byte| {
                column + if byte == b'\t' { 4 - column % 4 } else { 1 }
            })
    };
    let reference = if unindent {
        target.parent.unwrap()
    } else {
        target.previous.unwrap()
    };
    let reference_line = line_for(&blocks[reference])?;
    let original_indent = columns(&input.text[lines[start].clone()]);
    let desired_indent = if unindent {
        columns(&input.text[lines[reference_line].clone()])
    } else {
        let context = contexts[reference_line]
            .as_ref()
            .ok_or(DocumentError::AmbiguousProjection)?;
        input.text[lines[reference_line].start..context.content_start]
            .bytes()
            .fold(0usize, |column, byte| {
                column + if byte == b'\t' { 4 - column % 4 } else { 1 }
            })
    };
    let delta = if unindent {
        original_indent.checked_sub(desired_indent)
    } else {
        desired_indent.checked_sub(original_indent)
    }
    .filter(|delta| *delta > 0)
    .ok_or(DocumentError::UnsupportedFormatting)?;
    // A moved ordered run becomes a new list container when indented, so its
    // first root starts at one.  When it is moved back beside an ordered
    // parent, continue after that parent's ordinal.  Normalize every moved
    // root marker in the run: these lines already belong to the structural
    // edit, while unrelated source marker spelling remains untouched.
    let mut ordered_next = if unindent {
        match blocks[target.parent.unwrap()].kind {
            BlockKind::ListItem {
                ordered: true,
                ordinal,
                ..
            } => Some(ordinal.saturating_add(1)),
            _ => None,
        }
    } else {
        None
    };
    let mut canonical_ordinals = BTreeMap::new();
    for (root_number, index) in target.roots.iter().copied().enumerate() {
        let BlockKind::ListItem {
            ordered,
            container_start,
            ..
        } = blocks[index].kind
        else {
            continue;
        };
        if !ordered {
            ordered_next = None;
            continue;
        }
        if root_number == 0 {
            ordered_next = Some(ordered_next.unwrap_or(1));
        } else if container_start || ordered_next.is_none() {
            ordered_next = Some(1);
        }
        let ordinal = ordered_next.unwrap();
        canonical_ordinals.insert(line_for(&blocks[index])?, ordinal);
        ordered_next = Some(ordinal.saturating_add(1));
    }
    let mut patches = Vec::new();
    for (line_index, line) in lines.iter().enumerate().take(end).skip(start) {
        let text = &input.text[line.clone()];
        if text.trim().is_empty() {
            continue;
        }
        let prefix = text.len() - text.trim_start_matches([' ', '\t']).len();
        let current = columns(text);
        let next = if unindent {
            current.saturating_sub(delta)
        } else {
            current + delta
        };
        patches.push(SourcePatch::primary(
            converter.source_range(line.start..line.start + prefix),
            document.encoding().encode_fragment(&" ".repeat(next))?,
        ));
        if let Some(ordinal) = canonical_ordinals.get(&line_index) {
            let context = contexts[line_index]
                .as_ref()
                .ok_or(DocumentError::AmbiguousProjection)?;
            let marker = context
                .marker
                .clone()
                .ok_or(DocumentError::AmbiguousProjection)?;
            let digits = input.text[marker.clone()]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if digits == 0 {
                return Err(DocumentError::AmbiguousProjection);
            }
            patches.push(SourcePatch::primary(
                converter.source_range(marker.start..marker.start + digits),
                document.encoding().encode_fragment(&ordinal.to_string())?,
            ));
        }
    }
    if unindent && end < lines.len() {
        let last_root = *target
            .roots
            .last()
            .ok_or(DocumentError::AmbiguousProjection)?;
        let root_line = line_for(&blocks[last_root])?;
        let root = &input.text[lines[root_line].clone()];
        let marker = super::super::markdown_blocks::marker_prefix_length(root)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let indentation = root.len() - root.trim_start_matches([' ', '\t']).len();
        let marker = &root[indentation..marker];
        let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
        let marker = canonical_ordinals.get(&root_line).map_or_else(
            || marker.to_owned(),
            |ordinal| format!("{ordinal}{}", &marker[digits..]),
        );
        let required = marker.bytes().fold(desired_indent, |column, byte| {
            column + if byte == b'\t' { 4 - column % 4 } else { 1 }
        });
        let next =
            (end..lines.len()).find(|&index| !input.text[lines[index].clone()].trim().is_empty());
        let next_indent = next.map_or(required, |index| columns(&input.text[lines[index].clone()]));
        let new_level = level(&blocks[target.first]).unwrap() - 1;
        if next.and_then(|index| contexts[index].as_ref()).is_some_and(|context|
            matches!(context.kind, BlockKind::ListItem { level, .. } if level > new_level))
            && next_indent < required
        {
            // Following siblings retain their depth beneath the newly lifted
            // item. Its marker can be wider than their former parent's label.
            let extra = required - next_indent;
            for (index, row) in lines.iter().enumerate().skip(end) {
                let text = &input.text[row.clone()];
                if text.trim().is_empty() { continue; }
                if !contexts[index].as_ref().is_some_and(|context| matches!(context.kind, BlockKind::ListItem { level, .. } if level > new_level)) { break; }
                let prefix = text.len() - text.trim_start_matches([' ', '\t']).len();
                patches.push(SourcePatch::primary(converter.source_range(row.start..row.start + prefix),
                    document.encoding().encode_fragment(&" ".repeat(columns(text) + extra))?));
            }
        }
    }
    Ok(patches)
}
