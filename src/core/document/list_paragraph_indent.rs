//! Moving one paragraph between a list and the level around it. Tab at the
//! start of a paragraph that follows a list continues the list's last item,
//! exactly as Option-Enter at that item's end would have; Shift-Tab at the
//! start of an item's continuation paragraph lifts it out of every list
//! when no deeper content of that list follows it. Only the paragraph's own
//! source rows change, by the item's indentation.
use super::*;
use crate::document::{edit_boundary, Block, BlockKind, ContainerIdentity, ContainerKind};

struct Target {
    paragraph: Block,
    /// The block before the paragraph. Indenting joins the item owning it.
    previous: Block,
    /// Quotes enclosing the list, which the paragraph retains.
    quotes: usize,
    /// A top-level item after a lifted paragraph, which starts a new list.
    following: Option<Block>,
}

/// The outermost list and the number of quotes around it. A quote between
/// two list levels has no continuation spelled by indentation alone.
fn list_context(block: &Block) -> Option<(ContainerIdentity, usize)> {
    block.list_item_owner()?;
    let containers = &block.containers;
    let first = containers.iter().position(|member| matches!(member.container.kind, ContainerKind::List { .. }))?;
    let item = containers.iter().rposition(|member| member.container.kind == ContainerKind::ListItem)?;
    (containers[..first].iter().all(|member| member.container.kind == ContainerKind::Quote)
        && containers[first..item].iter().all(|member|
            matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem)))
        .then(|| (containers[first].container.id, first))
}

fn line_content(text: &str, file_format: FileFormat) -> &str {
    match file_format {
        FileFormat::Dos => text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')),
        _ => text.strip_suffix(file_format.spelling()),
    }
    .unwrap_or(text)
}

impl Document {
    /// Tab or Shift-Tab at the first caret position of a paragraph moves it
    /// into the list just before it, or out of every list it continues.
    pub(crate) fn list_paragraph_indent_key_request(
        &self,
        at: usize,
        unindent: bool,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        Ok(self.list_paragraph_target(at, unindent)?.map(|_| ModelRequest::IndentListParagraph {
            document: self.id(),
            revision: self.revision(),
            at,
            unindent,
        }))
    }

    fn list_paragraph_target(&self, at: usize, unindent: bool) -> Result<Option<Target>, DocumentError> {
        if !self.format().is_markdown() || at == 0 || self.projection().table_cell_at(at).is_some() {
            return Ok(None);
        }
        let Some(paragraph) = edit_boundary::paragraph_at(self, at)? else { return Ok(None) };
        if paragraph.range.start != at
            || paragraph.markdown_html
            || paragraph.thematic_break
            || edit_boundary::is_code_paragraph(self, &paragraph)?
        {
            return Ok(None);
        }
        let Some(previous) = edit_boundary::paragraph_at(self, at - 1)?.filter(|block| block.range.end + 1 == at) else {
            return Ok(None);
        };
        let target = if unindent {
            if matches!(paragraph.kind, BlockKind::ListItem { item_start: true, .. })
                || self.is_trailing_blank_list_paragraph(&paragraph)?
            {
                return Ok(None);
            }
            let Some((list, quotes)) = list_context(&paragraph) else { return Ok(None) };
            // Lifting splits the list. Later content of the paragraph's own
            // item or a nested list would be left without an owner, so Shift-Tab
            // keeps moving the owning item there instead.
            let mut following = self.block_after(&paragraph).filter(|next|
                next.containers.get(quotes).is_some_and(|member| member.container.id == list));
            if let Some(next) = &following {
                let top_level_item = matches!(next.kind, BlockKind::ListItem { item_start: true, .. })
                    && next.containers.iter().filter(|member|
                        matches!(member.container.kind, ContainerKind::List { .. })).count() == 1;
                if !top_level_item {
                    if !self.is_trailing_blank_list_paragraph(next)? {
                        return Ok(None);
                    }
                    following = None;
                }
            }
            Some(Target { paragraph, previous, quotes, following })
        } else {
            let in_list = |block: &Block| block.containers.iter().any(|member|
                matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem));
            // Trailing blank rows still parse inside the last item, but they
            // spell an independent paragraph after the list.
            if in_list(&paragraph) && !self.is_trailing_blank_list_paragraph(&paragraph)? {
                return Ok(None);
            }
            // The paragraph keeps exactly the quotes around the list.
            list_context(&previous)
                .filter(|(_, quotes)| {
                    paragraph.containers.len() >= *quotes
                        && paragraph.containers[..*quotes].iter().zip(&previous.containers[..*quotes])
                            .all(|(left, right)| left.container.id == right.container.id)
                })
                .map(|(_, quotes)| Target { paragraph, previous, quotes, following: None })
        };
        Ok(target)
    }

    pub(super) fn prepare_list_paragraph_indent(
        &self,
        at: usize,
        unindent: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let target = self.list_paragraph_target(at, unindent)?.ok_or(DocumentError::UnsupportedFormatting)?;
        let paragraph = &target.paragraph;
        let item = if unindent { paragraph } else { &target.previous };
        let (_, owner_line, continuation) =
            self.markdown_list_continuation_at_depth(item.range.start, item, target.quotes)?;
        // Measure the item's body from the end of its own quote prefix. The
        // continuation reuses that spelling verbatim and replaces markers with
        // spaces, so the remainder is the indentation inside those quotes. A
        // tab in that prefix would make the body column depend on the row.
        let (owner_quotes, depth) = super::super::markdown_quotes::prefix_with_limit(&owner_line.text, target.quotes);
        let quote_prefix = &owner_line.text[..owner_quotes];
        let indentation = continuation.strip_prefix(quote_prefix)
            .filter(|indentation| depth == target.quotes && !quote_prefix.contains('\t')
                && !indentation.is_empty() && indentation.bytes().all(|byte| byte == b' '))
            .ok_or(DocumentError::UnsupportedFormatting)?
            .len();
        let mut patches = self.list_paragraph_patches(&target, indentation, unindent)?;
        if patches.is_empty() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if let Some(next) = &target.following {
            patches.extend(self.list_paragraph_separator(&target, next)?);
        }
        // Row prefixes are hidden syntax without an unambiguous formatted
        // boundary, so anchor the local verification at the visible start.
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(at..at, "")], Some(patches), true,
        )?;
        self.verify_list_paragraph_indent(&prepared, &target, unindent)?;
        Ok(prepared)
    }

    fn block_after(&self, block: &Block) -> Option<Block> {
        let at = block.range.end + 1;
        (at <= self.projection().text_tree().byte_len()).then(|| self.projection().blocks_for_region(&(at..at))
            .into_iter().find(|next| next.range.start == at)).flatten()
    }

    /// Indent or unindent every physical row of the paragraph after its
    /// retained quote prefix. Lazy rows without that prefix stay lazy.
    fn list_paragraph_patches(
        &self,
        target: &Target,
        indentation: usize,
        unindent: bool,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let paragraph = &target.paragraph;
        let lines = &self.state().source_hard_lines;
        let (first, last) = if paragraph.range.is_empty() {
            let at = self.projection().source_insertion_point(paragraph.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let line = lines.line_at_offset(at).ok_or(DocumentError::AmbiguousProjection)?;
            (line, line)
        } else {
            let source = self.projection().source_range(paragraph.range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            (
                lines.line_at_offset(source.start).ok_or(DocumentError::AmbiguousProjection)?,
                lines.line_at_offset(source.end - 1).ok_or(DocumentError::AmbiguousProjection)?,
            )
        };
        let mut patches = Vec::new();
        for index in first..=last {
            let line = lines.get(index).ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = self.state().source.bytes_in(line.clone()).ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, line.start)?;
            let text = line_content(&decoded.text, self.file_format());
            if !unindent && paragraph.range.is_empty() && target.quotes == 0 && text.trim_matches([' ', '\t']).is_empty() {
                // Blank indented rows alone are trivia. Give the empty body the
                // passive spelling Option-Enter writes, retaining whether the
                // document ends with a line terminator.
                let mut body = format!("{}<span></span>", " ".repeat(indentation));
                if line.is_empty() {
                    body.push_str(self.file_format().spelling());
                }
                let end = decoded.source_boundary(text.len()).ok_or(DocumentError::AmbiguousProjection)?;
                patches.push(SourcePatch::primary(line.start..end, self.encoding().encode_fragment(&body)?));
                continue;
            }
            let (quote, depth) = super::super::markdown_quotes::prefix_with_limit(text, target.quotes);
            if text[quote..].trim_start_matches([' ', '\t']).is_empty() || depth < target.quotes {
                // The first row names the paragraph; it cannot be blank or lazy.
                if index == first {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                continue;
            }
            if target.quotes > 0 && text[..quote].contains('\t') {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let start = decoded.source_boundary(quote).ok_or(DocumentError::AmbiguousProjection)?;
            if !unindent {
                // A quote marker without its optional space would claim the
                // first inserted space instead.
                let width = indentation + usize::from(target.quotes > 0 && !text[..quote].ends_with(' '));
                patches.push(SourcePatch::primary(start..start, self.encoding().encode_fragment(&" ".repeat(width))?));
                continue;
            }
            let rest = &text[quote..];
            let mut column = 0;
            let removed = rest.bytes().take_while(|byte| {
                if column >= indentation || !matches!(byte, b' ' | b'\t') {
                    return false;
                }
                column += if *byte == b'\t' { 4 - column % 4 } else { 1 };
                true
            }).count();
            if removed == 0 {
                continue;
            }
            if target.quotes > 0 && rest[..removed].contains('\t') {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let end = decoded.source_boundary(quote + removed).ok_or(DocumentError::AmbiguousProjection)?;
            // A tab can reach past the item's body column; keep its excess.
            let excess = " ".repeat(column.saturating_sub(indentation));
            patches.push(SourcePatch::primary(start..end, self.encoding().encode_fragment(&excess)?));
        }
        Ok(patches)
    }

    /// A lifted paragraph is ordinary prose, which an ordered item numbered
    /// other than one cannot interrupt. Separate it from the following item
    /// with a blank row inside the retained quotes.
    fn list_paragraph_separator(&self, target: &Target, next: &Block) -> Result<Option<SourcePatch>, ModelTransactionError> {
        let lines = &self.state().source_hard_lines;
        let paragraph_end = if target.paragraph.range.is_empty() {
            self.projection().source_insertion_point(target.paragraph.range.start, true)
        } else {
            self.projection().source_range(target.paragraph.range.clone()).map(|source| source.end - 1)
        }.and_then(|at| lines.line_at_offset(at)).ok_or(DocumentError::AmbiguousProjection)?;
        let next_at = self.projection().source_insertion_point(next.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let next_line = lines.line_at_offset(next_at).ok_or(DocumentError::AmbiguousProjection)?;
        if next_line != paragraph_end + 1 {
            return Ok(None);
        }
        let line = lines.get(next_line).ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(line.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let (quote, _) = super::super::markdown_quotes::prefix_with_limit(&decoded.text, target.quotes);
        let separator = format!("{}{}", decoded.text[..quote].trim_end_matches([' ', '\t']), self.file_format().spelling());
        Ok(Some(SourcePatch::primary(line.start..line.start, self.encoding().encode_fragment(&separator)?)))
    }

    fn verify_list_paragraph_indent(
        &self,
        prepared: &PreparedModelTransaction,
        target: &Target,
        unindent: bool,
    ) -> Result<(), ModelTransactionError> {
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let after = |range: &Range<usize>| candidate.projection.blocks_for_region(range).into_iter()
            .find(|block| block.range == *range).ok_or(DocumentError::VerificationFailed);
        let before = &target.paragraph;
        let paragraph = after(&before.range)?;
        let previous = after(&target.previous.range)?;
        let heading = |block: &Block| block.style.0.starts_with("Heading").then(|| block.style.clone());
        let moved = if unindent {
            paragraph.list_navigation_level().is_none()
        } else {
            paragraph.list_item_owner().is_some()
                && paragraph.list_item_owner() == target.previous.list_item_owner()
                && previous.list_item_owner() == target.previous.list_item_owner()
                && !matches!(paragraph.kind, BlockKind::ListItem { item_start: true, .. })
        };
        // The neighbors keep their own structure; only a following item may
        // start or join a list.
        let unchanged = |left: &Block, right: &Block, ordinal: bool| {
            let shape = |block: &Block| match block.kind {
                BlockKind::ListItem { ordered, ordinal: number, level, item_start, .. } =>
                    Some((ordered, level, item_start, ordinal.then_some(number))),
                _ => None,
            };
            left.style == right.style && left.quote_depth == right.quote_depth && shape(left) == shape(right)
        };
        let following_unchanged = self.block_after(before)
            .is_none_or(|old| after(&old.range).is_ok_and(|new| unchanged(&old, &new, false)));
        if !moved
            || paragraph.quote_depth != before.quote_depth
            || heading(&paragraph) != heading(before)
            || paragraph.style.0 == "Code Block"
            || !unchanged(&target.previous, &previous, true)
            || !following_unchanged
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        super::list_indent::verify_character_styles_in(self.projection(), &candidate.projection, &before.range)?;
        Ok(())
    }
}

