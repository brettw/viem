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
                block.range.start <= range.start && range.start <= block.range.end
                    && (range.start < block.range.end || !blocks.iter().any(|next| next.range.start == range.start && next.range.start != block.range.start))
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
        if !(self.format().is_wysiwyg() || self.format().is_source_view()) || self.validate_range(&range).is_err() {
            return (false, false);
        }
        let blocks = self.projection().list_indentation_blocks(self.format().is_source_view());
        let Some(mut selected) = blocks.index_touching_point(range.start) else { return (false, false); };
        // A nonempty half-open selection excludes the block ending at its
        // start. Empty carets retain the downstream boundary owner.
        if !range.is_empty() && blocks.get(selected).is_some_and(|block| block.range.end == range.start) {
            selected += 1;
        }
        let Some(current) = blocks.get(selected) else { return (false, false); };
        let Some(base) = level(&current).map(|level| u16::from(level) + 1) else { return (false, false); };
        let last = if range.is_empty() { selected + 1 } else { blocks.partition_point_start(range.end) };
        if selected >= last { return (false, false); }
        let lower = |summary: super::super::range_index::NavigationSummary| summary.minimum < base;
        let boundary = |summary: super::super::range_index::NavigationSummary| lower(summary) || summary.minimum_start <= base;
        let Some(first) = blocks.find_navigation(0..selected + 1, true, boundary) else { return (false, false); };
        if blocks.get(first).is_none_or(|block| level(&block).map(|level| u16::from(level) + 1) != Some(base))
            || blocks.find_navigation(first..last, false, lower).is_some() {
            return (false, false);
        }
        let end = blocks.find_navigation(last..blocks.len(), false, boundary).unwrap_or(blocks.len());
        let previous = blocks.find_navigation(0..first, true, boundary)
            .and_then(|index| blocks.get(index)).is_some_and(|block| level(&block).map(|level| u16::from(level) + 1) == Some(base));
        let parent = base > 1 && blocks.find_navigation(0..first, true, lower)
            .and_then(|index| blocks.get(index)).is_some_and(|block| level(&block).map(|level| u16::from(level) + 1) == Some(base - 1));
        let indent = previous && blocks.find_navigation(first..end, false, |summary| summary.maximum >= 4 || summary.flags & 1 != 0).is_none();
        let unindent = parent && blocks.find_navigation(first..end, false, |summary| summary.flags & 2 != 0).is_none();
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
                if self.format() == Format::HtmlSource {
                    Format::Html
                } else {
                    Format::Markdown
                },
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
            return self.prepare_html_source_patches(prepared.summary().source_patches().to_vec());
        }
        if !self.format().is_wysiwyg() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let blocks = self.projection().blocks();
        let target = targets(blocks, &range, unindent)?;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let mut table_indents = BTreeMap::new();
        let patches = match self.format() {
            Format::Html => html_patches(self, &input, blocks, &target, unindent)?,
            Format::Rtf => {
                rtf_patches(self, &input, blocks, &target, unindent, &mut table_indents)?
            }
            _ => markdown_patches(self, &input, blocks, &target, unindent)?,
        };
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
            let mut paragraph = before.direct_paragraph.clone();
            if let Some((old, new)) = table_indents.get(&index) {
                // Modern list tables own these two indentation defaults.
                // Authored paragraph overrides continue to win unchanged.
                if paragraph.leading_indent == old.leading_indent
                    && after.direct_paragraph.leading_indent == new.leading_indent
                {
                    paragraph.leading_indent = new.leading_indent;
                }
                if paragraph.first_line_indent == old.first_line_indent
                    && after.direct_paragraph.first_line_indent == new.first_line_indent
                {
                    paragraph.first_line_indent = new.first_line_indent;
                }
            }
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
        let last_root = *target.roots.last().ok_or(DocumentError::AmbiguousProjection)?;
        let root_line = line_for(&blocks[last_root])?;
        let root = &input.text[lines[root_line].clone()];
        let marker = super::super::markdown_blocks::marker_prefix_length(root)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let indentation = root.len() - root.trim_start_matches([' ', '\t']).len();
        let marker = &root[indentation..marker];
        let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
        let marker = canonical_ordinals.get(&root_line)
            .map_or_else(|| marker.to_owned(), |ordinal| format!("{ordinal}{}", &marker[digits..]));
        let required = marker.bytes().fold(desired_indent, |column, byte| {
            column + if byte == b'\t' { 4 - column % 4 } else { 1 }
        });
        let next = (end..lines.len()).find(|&index| !input.text[lines[index].clone()].trim().is_empty());
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

#[derive(Clone)]
struct Element {
    name: String,
    open: Range<usize>,
    close: Range<usize>,
    parent: Option<usize>,
    start_attributes: usize,
}

fn generated_list_open(name: &str, level: u8) -> Result<String, DocumentError> {
    let list_type = match (name, level.min(3)) {
        ("ol", 1) => "a",
        ("ol", 2) => "i",
        ("ol", _) => "1",
        ("ul", 1) => "circle",
        ("ul", 2) => "square",
        ("ul", _) => "disc",
        _ => return Err(DocumentError::UnsupportedFormatting),
    };
    Ok(format!("<{name} type=\"{list_type}\">"))
}

fn reparented_list_open(
    input: &str,
    container: &Element,
    level: u8,
) -> Result<String, DocumentError> {
    let mut open = input[container.open.clone()].to_owned();
    if container.name == "ol" {
        for _ in 0..container.start_attributes {
            let length = open.len();
            let (range, replacement) =
                super::super::html_styles::attribute_patch(&open, 0..length, "start", "");
            open.replace_range(range, &replacement);
        }
    }
    let list_type = match (container.name.as_str(), level.min(3)) {
        ("ol", 1) => "a",
        ("ol", 2) => "i",
        ("ol", _) => "1",
        ("ul", 1) => "circle",
        ("ul", 2) => "square",
        ("ul", _) => "disc",
        _ => return Err(DocumentError::UnsupportedFormatting),
    };
    let length = open.len();
    let (range, replacement) =
        super::super::html_styles::attribute_patch(&open, 0..length, "type", list_type);
    open.replace_range(range, &replacement);
    Ok(open)
}

fn html_patches(
    document: &Document,
    input: &super::super::line_endings::NormalizedText,
    blocks: &[Block],
    target: &Targets,
    unindent: bool,
) -> Result<Vec<SourcePatch>, DocumentError> {
    use super::super::html::TokenKind;
    let tokens = super::super::html::tokenize(&input.text);
    let mut elements: Vec<Element> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for token in &tokens {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if !super::super::html::list_element(&tag.name) {
            continue;
        }
        if tag.end {
            if let Some(index) = stack.pop() {
                if elements[index].name != tag.name {
                    return Err(DocumentError::UnsupportedFormatting);
                }
                elements[index].close = token.range.clone();
            }
        } else {
            let index = elements.len();
            elements.push(Element {
                name: tag.name.clone(),
                open: token.range.clone(),
                close: 0..0,
                parent: stack.last().copied(),
                start_attributes: tag
                    .attributes
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("start"))
                    .count(),
            });
            stack.push(index);
        }
    }
    let converter = super::super::rich_text::Builder::new(input, Revision(0));
    let item_for = |block: &Block| -> Result<usize, DocumentError> {
        let source_at = block_source_at(document, block)?;
        elements
            .iter()
            .enumerate()
            .filter(|(_, element)| {
                element.name == "li"
                    && !element.close.is_empty()
                    && converter.source_range(element.open.clone()).end <= source_at
                    && source_at <= converter.source_range(element.close.clone()).start
            })
            .max_by_key(|(_, element)| element.open.start)
            .map(|(index, _)| index)
            .ok_or(DocumentError::AmbiguousProjection)
    };
    let first_index = item_for(&blocks[target.first])?;
    let last_index = item_for(&blocks[*target.roots.last().unwrap()])?;
    let first = &elements[first_index];
    let last = &elements[last_index];
    let container_index = first.parent.ok_or(DocumentError::UnsupportedFormatting)?;
    let container = &elements[container_index];
    if container.close.is_empty()
        || target.roots.iter().any(|index| {
            item_for(&blocks[*index]).ok().map_or(true, |index| {
                elements[index].parent != Some(container_index)
            })
        })
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    let mut raw = Vec::new();
    if !unindent {
        let previous_index = item_for(&blocks[target.previous.unwrap()])?;
        let previous = &elements[previous_index];
        let same_container = previous.parent == first.parent;
        let previous_container = previous
            .parent
            .and_then(|index| elements.get(index))
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if !same_container
            && (previous_container.parent != container.parent
                || previous_container.close.is_empty()
                || elements.iter().any(|element| {
                    element.name == "li"
                        && element.parent == previous.parent
                        && element.open.start >= previous.close.end
                }))
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        // Indenting starts a new child container, whose counter begins at one.
        // Spell the generated level style in HTML as well as presenting it in
        // Viem, so another HTML renderer sees the same marker family.
        let nested_level = level(&blocks[target.first])
            .unwrap()
            .saturating_add(1)
            .min(3);
        let open = generated_list_open(&container.name, nested_level)?;
        if same_container {
            raw.push((previous.close.clone(), open));
            raw.push((
                last.close.end..last.close.end,
                format!(
                    "</{}>{}",
                    container.name,
                    &input.text[previous.close.clone()]
                ),
            ));
        } else {
            raw.push((
                container.open.clone(),
                reparented_list_open(&input.text, container, nested_level)?,
            ));
            raw.push((previous.close.clone(), String::new()));
            raw.push((previous_container.close.clone(), String::new()));
            let following = elements.iter().any(|element| {
                element.name == "li"
                    && element.parent == first.parent
                    && element.open.start >= last.close.end
            });
            if following {
                raw.push((
                    last.close.end..last.close.end,
                    format!(
                        "</{}>{}{}{}",
                        container.name,
                        &input.text[previous.close.clone()],
                        &input.text[previous_container.close.clone()],
                        &input.text[container.open.clone()]
                    ),
                ));
            } else {
                raw.push((
                    container.close.end..container.close.end,
                    format!(
                        "{}{}",
                        &input.text[previous.close.clone()],
                        &input.text[previous_container.close.clone()]
                    ),
                ));
            }
        }
    } else {
        let parent_index = container
            .parent
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let parent = &elements[parent_index];
        if parent.name != "li" || parent.close.is_empty() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let parent_container = parent
            .parent
            .and_then(|index| elements.get(index))
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let mixed = parent_container.name != container.name;
        let following = elements.iter().any(|element| {
            element.name == "li"
                && element.parent == Some(container_index)
                && element.open.start >= last.close.end
        });
        let preceding = elements.iter().any(|element| {
            element.name == "li"
                && element.parent == Some(container_index)
                && element.close.end <= first.open.start
        });
        let remove_original_container = !mixed && !preceding;
        let mut close_parent = if remove_original_container {
            input.text[parent.close.clone()].to_owned()
        } else {
            format!(
                "{}{}",
                &input.text[container.close.clone()],
                &input.text[parent.close.clone()]
            )
        };
        if mixed {
            if parent_container.close.is_empty() {
                return Err(DocumentError::UnsupportedFormatting);
            }
            close_parent.push_str(&input.text[parent_container.close.clone()]);
            close_parent.push_str(&input.text[container.open.clone()]);
            raw.push((
                parent.close.end..parent.close.end,
                format!(
                    "{}{}",
                    &input.text[container.close.clone()],
                    &input.text[parent_container.open.clone()]
                ),
            ));
        }
        raw.push((first.open.start..first.open.start, close_parent));
        if following {
            raw.push((
                last.close.clone(),
                input.text[container.open.clone()].to_owned(),
            ));
        } else {
            // With no preceding child, this same-family container no longer
            // belongs under the old parent. A following run is reopened under
            // the last moved item above; otherwise the container disappears.
            if remove_original_container {
                raw.push((container.open.clone(), String::new()));
            }
            raw.push((container.close.clone(), String::new()));
            raw.push((parent.close.clone(), String::new()));
        }
        if following && remove_original_container {
            raw.push((container.open.clone(), String::new()));
        }
    }
    raw.into_iter()
        .map(|(range, text)| {
            Ok(SourcePatch::primary(
                converter.source_range(range),
                document.encoding().encode_fragment(&text)?,
            ))
        })
        .collect()
}

fn rtf_patches(
    document: &Document,
    input: &super::super::line_endings::NormalizedText,
    blocks: &[Block],
    target: &Targets,
    unindent: bool,
    table_indents: &mut BTreeMap<usize, (BlockProperties, BlockProperties)>,
) -> Result<Vec<SourcePatch>, DocumentError> {
    use super::super::rtf::{self, Kind};
    let tokens = rtf::tokenize(input);
    let tables = super::super::rtf_lists::ListTables::read(&tokens);
    let converter = super::super::rich_text::Builder::new(input, Revision(0));
    let mut state = (None, 0u8, false, true);
    let mut stack = Vec::new();
    let mut selectors = vec![(0, None)];
    for token in &tokens {
        match &token.kind {
            Kind::Open => {
                stack.push(state);
                state.3 = true;
            }
            Kind::Close => state = stack.pop().unwrap_or((None, 0, false, true)),
            Kind::Symbol('*') if state.3 => state.2 = true,
            Kind::Control(name, number) => {
                if state.3 && rtf::non_body(name) {
                    state.2 = true;
                }
                state.3 = false;
                if !state.2 {
                    match name.as_str() {
                        "ls" => state.0 = number.filter(|id| (1..=2000).contains(id)),
                        "ilvl" => {
                            state.1 =
                                number.filter(|level| (0..9).contains(level)).unwrap_or(0) as u8
                        }
                        "pard" => {
                            state.0 = None;
                            state.1 = 0;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let selector = (!state.2)
            .then_some(state.0.map(|id| (id, state.1)))
            .flatten();
        if selectors.last().unwrap().1 != selector {
            selectors.push((converter.source_range(token.range.clone()).end, selector));
        }
    }
    let selector_at = |at| {
        selectors[selectors
            .partition_point(|(position, _)| *position <= at)
            .saturating_sub(1)]
        .1
    };
    let mut inserts: BTreeMap<usize, String> = BTreeMap::new();
    for index in target.first..target.end {
        let block = &blocks[index];
        let start = super::super::rich_text::block_source_point(document.projection(), block)?;
        let (id, old_level) = selector_at(start).ok_or(DocumentError::UnsupportedFormatting)?;
        let new_level = if unindent {
            old_level.checked_sub(1)
        } else {
            old_level.checked_add(1)
        }
        .ok_or(DocumentError::UnsupportedFormatting)?;
        let old = tables
            .level(id, old_level)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let new = tables
            .level(id, new_level)
            .filter(|new| new.ordered == old.ordered)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if Some(old_level) != level(block) {
            return Err(DocumentError::AmbiguousProjection);
        }
        let mut end = document
            .projection()
            .provenance_for_region(&block.range)
            .iter()
            .filter(|span| !span.source.is_empty())
            .map(|span| span.source.end)
            .max()
            .unwrap_or(start);
        // The existing paragraph delimiter belongs to the selected paragraph;
        // its selector must resume only after that delimiter in external RTF.
        if let Some(boundary) = document
            .projection()
            .provenance_for_region(&(block.range.end..block.range.end.saturating_add(1)))
            .iter()
            .find(|span| {
                span.formatted.start == block.range.end
                    && document.text().get(span.formatted.clone()) == Some("\n")
            })
        {
            end = end.max(boundary.source.end);
        }
        let restore = selector_at(end).map_or_else(
            || "\\ls0 ".to_owned(),
            |(id, level)| format!("\\ls{id}\\ilvl{level} "),
        );
        inserts
            .entry(start)
            .or_default()
            .push_str(&format!("\\ilvl{new_level} "));
        if end != start || index + 1 < blocks.len() {
            inserts.entry(end).or_default().push_str(&restore);
        }
        table_indents.insert(index, (old.paragraph.clone(), new.paragraph.clone()));
    }
    inserts
        .into_iter()
        .map(|(at, text)| {
            Ok(SourcePatch::primary(
                at..at,
                document.encoding().encode_fragment(&text)?,
            ))
        })
        .collect()
}
