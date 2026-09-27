//! Paragraph style commands replace the current structural treatment. Imported
//! nested containers remain readable; only selected paragraphs are normalized.
use super::replacement::PatchComposition;
use super::*;

#[derive(Clone)]
pub(super) enum Assignment {
    Paragraph(StyleId),
    List(Option<super::super::ListStyle>),
}

impl Document {
    pub(super) fn prepare_markdown_heading_patches(
        &self,
        patches: Vec<SourcePatch>,
        range: &Range<usize>,
        style: &StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        // A Markdown heading is one native source line. Styling a paragraph
        // with internal hard breaks therefore creates several heading blocks.
        // Verify that structural intention explicitly; text-only preparation
        // correctly requires the original paragraph partition to survive.
        let mut prepared = self.prepare_reprojected_source_patches(patches)?;
        if prepared.is_no_op() {
            return Ok(prepared);
        }
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let projection = &candidate.projection;
        if projection.text() != self.text()
            || !projection.has_same_hard_line_structure(self.projection())
            || projection.style_spans() != self.projection().style_spans()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let selected = |block: &super::super::Block| {
            if range.is_empty() {
                block.range.start == range.start && block.range.end == range.end
            } else {
                block.range.start < range.end && range.start < block.range.end
            }
        };
        for block in projection.blocks() {
            if selected(block) {
                if block.style != *style
                    || block.range.start < range.start
                    || block.range.end > range.end
                {
                    return Err(DocumentError::VerificationFailed.into());
                }
            } else {
                let old = self.projection().blocks();
                let matching = old
                    .binary_search_by_key(&block.range.start, |old| old.range.start)
                    .ok()
                    .and_then(|index| old.get(index));
                if !matching.is_some_and(|old| {
                    old.range == block.range
                        && old.kind == block.kind
                        && old.style == block.style
                        && old.direct_paragraph == block.direct_paragraph
                        && old.direct_default_character == block.direct_default_character
                }) {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        prepared.summary.kind = ModelChangeKind::SemanticStyle;
        Ok(prepared)
    }

    pub(super) fn prepare_exclusive_structural_style(
        &self,
        range: Range<usize>,
        assignment: Assignment,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !matches!(
            self.format(),
            Format::Html | Format::Markdown | Format::MarkdownSource
        ) {
            return Ok(None);
        }
        self.validate_range(&range)?;
        let wants_quote =
            matches!(&assignment, Assignment::Paragraph(style) if style.0 == "Block quote");
        let wants_plain = matches!(&assignment, Assignment::Paragraph(style)
            if style == &self.projection().style_sheet().base_paragraph);
        let wants_code = matches!(&assignment, Assignment::Paragraph(style) if style.0 == "Code Block");
        let selected = selected_blocks(self, &range);
        let needs_normalization = selected.iter().any(|block| {
            if wants_quote {
                structural_body(block)
            } else {
                block.style.0 == "Block quote"
                    || (wants_plain || wants_code && block.style.0 != "Code Block")
                        && structural_body(block)
            }
        });
        if !needs_normalization {
            return Ok(None);
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let mut selected_range = range;
        // Removing an outer quote may expose an imported heading or list.
        // Normalize those exposed styles before assigning the requested one.
        let mut seen_sources = std::collections::BTreeSet::new();
        loop {
            let blocks = selected_blocks(&scratch, &selected_range);
            let incompatible = |block: &&super::super::Block| {
                structural_body(block) && !(wants_code && block.style.0 == "Code Block")
                    || !wants_quote && block.style.0 == "Block quote"
            };
            let Some(first) = blocks.iter().position(|block| incompatible(&block)) else {
                break;
            };
            // Preserve compatible selected quotes, including their original
            // cite/class spelling, while normalizing each adjoining run that
            // needs a different native body structure.
            let last = blocks[first..]
                .iter()
                .take_while(incompatible)
                .last()
                .unwrap();
            let normalize = blocks[first].range.start..last.range.end;
            // Each plain-style adapter removes a structural layer. Reject a
            // repeated source state instead of assuming an imported document
            // has a small, fixed nesting depth.
            if !seen_sources.insert(scratch.source_bytes()) {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let plain = scratch.projection().style_sheet().base_paragraph.clone();
            let prepared = scratch
                .prepare_structural_assignment_raw(normalize, Assignment::Paragraph(plain))?;
            if prepared.is_no_op() {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            publish(
                &mut scratch,
                prepared,
                &mut selected_range,
                &mut sources,
                &mut formatted,
            )?;
        }
        let prepared =
            scratch.prepare_structural_assignment_raw(selected_range.clone(), assignment)?;
        publish(
            &mut scratch,
            prepared,
            &mut selected_range,
            &mut sources,
            &mut formatted,
        )?;
        let source_patches = sources.source_patches(&scratch.state().source)?;
        let edits = formatted.formatted_edits(&scratch)?;
        let mut prepared = if scratch.text() == self.text() {
            self.prepare_reprojected_source_patches(source_patches)?
        } else {
            self.prepare_text_edits_with_patches(edits, Some(source_patches))?
        };
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let actual = &candidate.projection;
        let expected = scratch.projection();
        if actual.text() != expected.text()
            || !actual.has_same_hard_line_structure(expected)
            || actual.style_spans() != expected.style_spans()
            || actual.blocks().len() != expected.blocks().len()
            || actual
                .blocks()
                .iter()
                .zip(expected.blocks())
                .any(|(actual, expected)| {
                    actual.range != expected.range
                        || actual.kind != expected.kind
                        || actual.style != expected.style
                        || actual.direct_paragraph != expected.direct_paragraph
                        || actual.direct_default_character != expected.direct_default_character
                })
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        prepared.summary.kind = ModelChangeKind::SemanticStyle;
        Ok(Some(prepared))
    }

    fn prepare_structural_assignment_raw(
        &self,
        range: Range<usize>,
        assignment: Assignment,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        match assignment {
            Assignment::List(style) => self.prepare_list_style_raw(range, style),
            Assignment::Paragraph(style) if self.format() == Format::Html => self
                .prepare_html_named_style_raw(PersistedStyleIntent::AssignBlockStyle {
                    target: StyleBlockTarget::Paragraphs(TextRange::new(
                        self.text_point(range.start)?,
                        self.text_point(range.end)?,
                    )?),
                    style,
                }, false),
            Assignment::Paragraph(style) => {
                if style == self.projection().style_sheet().base_paragraph {
                    if let Some(prepared) = self.prepare_markdown_code_as_prose(&range)? {
                        return Ok(prepared);
                    }
                }
                self.prepare_markdown_paragraph_style_raw(range, style)
            }
        }
    }

    fn prepare_markdown_code_as_prose(
        &self,
        range: &Range<usize>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !self.format().is_markdown()
            || !selected_blocks(self, range).iter().any(|block| block.style.0 == "Code Block")
        {
            return Ok(None);
        }
        Ok(Some(self.prepare_markdown_code_style(range, false)?))
    }
}

pub(super) fn selected_blocks(document: &Document, range: &Range<usize>) -> Vec<super::super::Block> {
    document
        .projection()
        .blocks_for_region(range)
        .into_iter()
        .filter(|block| {
            if range.is_empty() {
                block.range.start <= range.start && range.start <= block.range.end
            } else {
                block.range.start < range.end && range.start < block.range.end
            }
        })
        .collect()
}

fn structural_body(block: &super::super::Block) -> bool {
    matches!(
        block.kind,
        super::super::BlockKind::ListItem { .. } | super::super::BlockKind::Heading(_)
    ) || block.style.0 == "Code Block"
}

fn publish(
    scratch: &mut Document,
    prepared: PreparedModelTransaction,
    selection: &mut Range<usize>,
    sources: &mut PatchComposition,
    formatted: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    if prepared.is_no_op() {
        return Ok(());
    }
    let map = prepared.text_position_map();
    let map_point = |at, association, affinity| -> Result<usize, ModelTransactionError> {
        Ok(map
            .map_text_point(
                scratch.text_point(at)?,
                association,
                affinity,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset())
    };
    let start = map_point(
        selection.start,
        Association::BeforeInsertion,
        BoundaryAffinity::Downstream,
    )?;
    let end = map_point(
        selection.end,
        Association::AfterInsertion,
        BoundaryAffinity::Upstream,
    )?;
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    formatted.record_formatted(&prepared)?;
    scratch.commit_model_transaction(prepared)?;
    *selection = start..end;
    Ok(())
}
