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
    block.list_level()
}
fn navigation_level(block: &Block) -> Option<u8> {
    block.list_navigation_level()
}
fn item_start(block: &Block) -> bool {
    block.list_editing.item_start
}

fn same_list_parent(left: &Block, right: &Block) -> bool {
    let parent = |block: &Block| block.containers.iter().rposition(|member|
        matches!(member.container.kind, super::super::ContainerKind::List { .. })).unwrap_or(0);
    let left_parent = &left.containers[..parent(left)];
    let right_parent = &right.containers[..parent(right)];
    left_parent.len() == right_parent.len() && left_parent.iter().zip(right_parent)
        .all(|(left, right)| left.container.id == right.container.id)
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
                block.range.start < range.end
                    && (range.start < block.range.end || block.range.is_empty() && range.start == block.range.start)
                    // An unassigned list separator can lie inside a selection,
                    // but is never itself an item selected for indentation.
                    && !(level(block).is_none() && navigation_level(block).is_some())
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
        .any(|block| navigation_level(block).map_or(true, |level| level < base))
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    let roots = (first..=last)
        .filter(|index| level(&blocks[*index]) == Some(base) && item_start(&blocks[*index]))
        .collect::<Vec<_>>();
    let end = (last + 1..blocks.len())
        .find(|index| {
            navigation_level(&blocks[*index]).map_or(true, |level| {
                level < base || level == base && item_start(&blocks[*index])
            })
        })
        .unwrap_or(blocks.len());
    let previous = (0..first)
        .rev()
        .find(|index| {
            navigation_level(&blocks[*index]).map_or(true, |level| {
                level < base || level == base && item_start(&blocks[*index])
            })
        })
        .filter(|index| level(&blocks[*index]) == Some(base)
            && same_list_parent(&blocks[*index], &blocks[first]));
    let parent = (0..first)
        .rev()
        .find(|index| navigation_level(&blocks[*index]).map_or(true, |level| level < base))
        .filter(|index| level(&blocks[*index]) == base.checked_sub(1))
        .and_then(|index| {
            // The nearest parent leaf may be a continuation or an editable
            // empty paragraph. Its physical indentation is body whitespace;
            // lifting a child uses the owning item's marker and ordinal.
            if item_start(&blocks[index]) { return Some(index); }
            let owner = blocks[index].list_item_owner()?;
            (0..=index).rev().find(|candidate|
                level(&blocks[*candidate]) == base.checked_sub(1)
                    && item_start(&blocks[*candidate])
                    && blocks[*candidate].list_item_owner() == Some(owner))
        });
    if unindent {
        // Without a parent paragraph the item may still share its line with
        // an empty parent's marker (`- - b`); the source patch decides.
        if base == 0 {
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
        if !self.format().is_markdown()
            || self.validate_range(&range).is_err()
        {
            return (false, false);
        }
        let blocks = self
            .projection()
            .list_indentation_blocks();
        let Some(mut selected) = blocks.index_touching_point(range.start) else {
            return (false, false);
        };
        // A nonempty half-open selection excludes the block ending at its
        // start. Empty carets retain the downstream boundary owner.
        if !range.is_empty()
            && blocks
                .get(selected)
                .is_some_and(|block| !block.range.is_empty() && block.range.end == range.start)
        {
            selected += 1;
        }
        let last = if range.is_empty() {
            selected + 1
        } else {
            blocks.partition_point_start(range.end)
        };
        let Some(mut current) = blocks.get(selected) else {
            return (false, false);
        };
        if !range.is_empty() && level(&current).is_none() && navigation_level(&current).is_some() {
            // Skip a leading unassigned separator only when the selection
            // reaches another item. A caret or separator-only selection is
            // still ineligible, and a non-list boundary still stops the scan.
            let Some(next) = blocks.find_navigation(selected + 1..last, false, |summary|
                summary.minimum == 0 || summary.minimum_start != u16::MAX)
            else { return (false, false); };
            selected = next;
            let Some(next) = blocks.get(selected) else { return (false, false); };
            current = next;
        }
        let Some(base) = level(&current).map(|level| u16::from(level) + 1) else {
            return (false, false);
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
            .is_some_and(|block| level(&block).map(|level| u16::from(level) + 1) == Some(base)
                && same_list_parent(&block, &current));
        // A nested item without a parent paragraph shares its line with an
        // empty parent's marker (`- - b`) and can still be lifted.
        let parent = base > 1
            && blocks
                .find_navigation(0..first, true, lower)
                .and_then(|index| blocks.get(index))
                .is_none_or(|block| {
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
        self.require_modifiable()?;
        let parentless = unindent && self.format().is_markdown()
            && targets(self.projection().blocks(), &range, unindent).is_ok_and(|target| target.parent.is_none());
        let prepared = self.prepare_list_indent_in_place(range, unindent);
        // Lifting out of an empty parent is only an alternative spelling; when
        // it cannot be verified the item simply has no unindent available.
        if parentless && prepared.is_err() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        prepared
    }

    fn prepare_list_indent_in_place(
        &self,
        range: Range<usize>,
        unindent: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let blocks = self.projection().blocks();
        let target = targets(blocks, &range, unindent)?;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let patches = markdown_patches(self, &input, blocks, &target, unindent)?;
        let separator = patches.iter().find_map(|patch| {
            let decoded = self.encoding().decode_region(&patch.replacement, patch.range.start).ok()?;
            decoded.text.starts_with(self.file_format().spelling()).then(|| {
                patch.range.start..patch.range.start + self.encoding().encode_fragment(self.file_format().spelling()).unwrap().len()
            })
        });
        let mut prepared = self.prepare_source_only_patches(patches)?;
        if let PreparedPublication::State(candidate) = &mut prepared.publication {
            if separator.is_some() { candidate.transient_list_separator = separator; }
        }
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
            // Supporting syntax can bring a retained list-only separator
            // inside an empty parent whose new child crosses that separator.
            // Its source-derived depth, text and paragraph boundary must stay
            // correct, independently of whether it now has an item owner.
            let expected = navigation_level(before).map(|level| {
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
            let retained_parent_continuation = !unindent
                && before.kind == BlockKind::Paragraph && before.range.is_empty()
                && before.list_item_owner().is_none()
                && matches!(after.kind, BlockKind::ListItem { item_start: false, .. })
                && target.previous.is_some_and(|parent| parent < index && index < target.first
                    && blocks[parent].range.is_empty()
                    && after.list_item_owner().is_some()
                    && after.list_item_owner() == candidate.projection.blocks()[parent].list_item_owner());

            if before.range != after.range
                || expected != navigation_level(after)
                || ordered(before) != ordered(after) && !retained_parent_continuation
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
    let original = input;
    let syntax = super::super::markdown_syntax::Blocks::parse(
        &super::super::markdown_syntax::grammar_text(input),
        super::super::markdown_syntax::ParseContext::source_region(input.source_at(0), document.projection().source_content_start()),
    ).to_source(input);
    let mut quotes = super::super::markdown_quotes::classify_with_syntax(input, &syntax);
    // Move the item's inner quotes with it. Only quotes enclosing the list
    // remain outside the indentation patch; stripping every quote would add
    // whitespace to the quote body instead of moving its marker.
    let memberships = &blocks[target.first].containers;
    let owner = memberships.iter().rposition(|member|
        member.container.kind == super::super::ContainerKind::ListItem).unwrap_or(0);
    let outer_quotes = memberships[..owner].iter().filter(|member|
        member.container.kind == super::super::ContainerKind::Quote).count();
    for quote in &mut quotes {
        if quote.depth > outer_quotes {
            quote.content_start = quote.range.start + super::super::markdown_quotes::prefix_with_limit(
                &input.text[quote.range.clone()], outer_quotes).0;
        }
    }
    let stripped = super::super::markdown_quotes::strip(input, &quotes);
    let input = &stripped;
    let lines = super::super::paragraph_flow::source_lines(input);
    // Reuse original ownership: removing quote markers cannot turn a quoted
    // leading thematic rule into a document-leading metadata delimiter.
    let contexts = super::super::markdown_blocks::classify_with_syntax(input, &syntax);
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
        target.parent
    } else {
        target.previous
    };
    let reference_line = reference.map(|reference| line_for(&blocks[reference])).transpose()?;
    // An item can share its first line with an empty parent's marker
    // (`- - b`). Lifting it removes that outer marker rather than whitespace,
    // so its indentation runs through every column before its own marker.
    let own_marker = contexts[start].as_ref().and_then(|context| context.marker.clone())
        .filter(|marker| lines[start].start <= marker.start && marker.start <= lines[start].end)
        .map(|marker| marker.start - lines[start].start);
    let start_text = &input.text[lines[start].clone()];
    let start_leading = start_text.len() - start_text.trim_start_matches([' ', '\t']).len();
    let shared_marker_prefix = own_marker.filter(|at| unindent && *at > start_leading
        && super::super::markdown_blocks::marker_prefix_length(&start_text[start_leading..]).is_some());
    let original_indent = match shared_marker_prefix {
        Some(at) => start_text[..at].bytes().fold(0usize, |column, byte| {
            column + if byte == b'\t' { 4 - column % 4 } else { 1 }
        }),
        None => columns(start_text),
    };
    let desired_indent = if unindent {
        match (reference_line, shared_marker_prefix) {
            (Some(line), _) => columns(&input.text[lines[line].clone()]),
            // Lifting out of an empty parent with no paragraph puts the item
            // at that parent's marker column.
            (None, Some(_)) => columns(start_text),
            (None, None) => return Err(DocumentError::UnsupportedFormatting),
        }
    } else {
        let reference_line = reference_line.ok_or(DocumentError::UnsupportedFormatting)?;
        let context = contexts[reference_line]
            .as_ref()
            .ok_or(DocumentError::AmbiguousProjection)?;
        // Tabs advance from the physical source column, including quote
        // markers. Removing those prefixes before counting changes the list
        // padding and can make the intended child a lazy prose continuation.
        let source = converter
            .source_range(context.content_start..context.content_start)
            .start;
        let at = original.normalized_at(source);
        let physical_columns = |range: Range<usize>| {
            original.text[range].bytes().fold(0usize, |column, byte| {
                column + if byte == b'\t' { 4 - column % 4 } else { 1 }
            })
        };
        physical_columns(quotes[reference_line].range.start..at)
            - physical_columns(quotes[reference_line].range.start..quotes[reference_line].content_start)
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
        match target.parent.map(|parent| &blocks[parent].kind) {
            Some(BlockKind::ListItem {
                ordered: true,
                ordinal,
                ..
            }) => Some(ordinal.saturating_add(1)),
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
    let empty_reference = |reference: usize| blocks[reference].range.is_empty()
        && blocks[reference + 1..target.first].iter().any(|block|
            level(block).is_none() && navigation_level(block).is_some());
    if let (false, Some(reference), Some(reference_line)) = (unindent, reference, reference_line) {
        if empty_reference(reference) {
            // A bare empty marker ends before surplus blank lines, so indentation
            // alone would leave the moved item at the top level. Give that empty
            // parent a passive body to retain its new child across the separator.
            let row = &lines[reference_line];
            let text = &input.text[row.clone()];
            let marker = super::super::markdown_blocks::marker_prefix_length(text)
                .ok_or(DocumentError::AmbiguousProjection)?;
            if text[marker..].trim().is_empty() {
                let body = if text.ends_with([' ', '\t']) { "<span></span>" } else { " <span></span>" };
                // Quote stripping removes source contributors at the next row's
                // boundary. Address this physical row's ending directly rather
                // than giving an empty cooked range that neighboring ownership.
                let at = original.endings.get(reference_line).map(|ending| ending.source.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                patches.push(SourcePatch::primary(
                    at..at,
                    document.encoding().encode_fragment(body)?,
                ));
            }
        }
    }
    for (line_index, line) in lines.iter().enumerate().take(end).skip(start) {
        let text = &input.text[line.clone()];
        if text.trim().is_empty() {
            continue;
        }
        let (prefix, current) = match shared_marker_prefix.filter(|_| line_index == start) {
            Some(at) => (at, original_indent),
            None => (text.len() - text.trim_start_matches([' ', '\t']).len(), columns(text)),
        };
        let next = if unindent {
            current.saturating_sub(delta)
        } else {
            current + delta
        };
        let mut indentation = " ".repeat(next);
        if !unindent && line_index == start && blocks[target.first].range.is_empty()
            && line_index > 0 && !input.text[lines[line_index - 1].clone()].trim().is_empty()
            && reference.is_some_and(|reference| !blocks[reference].range.is_empty())
        {
            // An empty nested bullet cannot interrupt its parent's paragraph
            // in GFM (and '-' may instead become a setext underline). A blank
            // source line makes the new empty child an actual list item.
            let quote = &original.text[quotes[line_index].range.start..quotes[line_index].content_start];
            indentation = format!("{}{quote}{indentation}", document.file_format().spelling());
        }
        patches.push(SourcePatch::primary(
            converter.source_range(line.start..line.start + prefix),
            document.encoding().encode_fragment(&indentation)?,
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
        if let Some(index) = next.filter(|&index| {
            contexts[index].as_ref().is_some_and(|context| matches!(context.kind,
                BlockKind::ListItem { level, ordered: true, ordinal, .. }
                    if level > new_level && ordinal != 1))
                && index > 0 && !input.text[lines[index - 1].clone()].trim().is_empty()
        }) {
            // A retained numbered sibling becomes a child of the lifted item.
            // Starting above one needs a separator to remain a list in GFM.
            let at = converter.source_range(lines[index].start..lines[index].start).start;
            let physical = document.state().source_hard_lines.line_at_offset(at)
                .and_then(|line| document.state().source_hard_lines.get(line))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = document.state().source.bytes_in(physical.start..at)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut separator = prefix;
            separator.extend(document.encoding().encode_fragment(document.file_format().spelling())?);
            patches.push(SourcePatch::primary(physical.start..physical.start, separator));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn front_matter_recognition_does_not_hide_quote_stripped_list_indentation() {
        let source = "> ---\n> - parent\n> - child\n> ---";
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let before = document.text().to_owned();
        assert!(document.projection().blocks().iter().all(|block| !block.is_front_matter()));
        let at = before.find("child").unwrap();
        let transaction = document.prepare_list_indent(at..at, false).unwrap();
        document.commit_model_transaction(transaction).unwrap();
        assert_eq!(document.text(), before);
        assert_eq!(document.source_bytes(), b"> ---\n> - parent\n>   - child\n> ---");
        assert!(document.projection().blocks().iter().all(|block| !block.is_front_matter()));
        let levels: Vec<_> = document.projection().blocks().iter().filter_map(Block::list_level).collect();
        assert_eq!(levels, [0, 1]);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
