//! Streaming geometry for enormous unwrapped rows. Width and bidi summaries
//! are compact; glyphs and carets are retained only around demanded geometry.
use super::*;
use crate::document::FormattedTextTree;

#[derive(Clone, Debug)]
pub(super) struct HorizontalMaterialization {
    pub text: FormattedTextTree,
    pub rows: Vec<(usize, usize, Vec<Range<f32>>)>,
}

impl PartialEq for HorizontalMaterialization {
    fn eq(&self, other: &Self) -> bool {
        self.text.shares_root_with(&other.text) && self.rows == other.rows
    }
}

#[derive(Clone)]
struct DirectionRun {
    text: Range<usize>,
    level: u8,
    width: f64,
    left: f64,
}

struct FragmentSummary {
    text: Range<usize>,
    runs: Range<usize>,
    whitespace_leading: bool,
    whitespace_advance: f64,
}

#[derive(PartialEq)]
struct SummaryKey {
    code_wrap: bool,
    whitespace_style: ResolvedTextStyle,
    whitespace_basis: super::super::WhitespaceBasis,
    whitespace_tabstop: u32,
    whitespace_leading: bool,
    whitespace_advance: u64,
    text: crate::document::FormattedTextSnapshotIdentity,
    line: Range<usize>,
    context_line: Range<usize>,
    default_style: ResolvedTextStyle,
    style_runs: Vec<ShapeStyleRun>,
    direction: TextDirection,
    marker: Option<Range<usize>>,
    scale: u32,
    environment: MeasurementEnvironmentId,
    metrics: MetricsGeneration,
    render_policy: Option<RenderRunPolicy>,
}

struct LineSummary {
    whitespace_unit: f32,
    leading_indent_width: f64,
    fragments: Vec<FragmentSummary>,
    runs: Vec<DirectionRun>,
    metrics: TextMetrics,
    width: f64,
    marker_width: f32,
    marker_end: usize,
}

#[derive(Default)]
pub(super) struct UnwrappedSummaryCache {
    entries: VecDeque<(SummaryKey, Arc<LineSummary>, usize)>,
    bytes: usize,
}

impl UnwrappedSummaryCache {
    pub fn clear(&mut self) {
        self.entries = VecDeque::new();
        self.bytes = 0;
    }

    fn get(&mut self, key: &SummaryKey) -> Option<Arc<LineSummary>> {
        let index = self
            .entries
            .iter()
            .position(|(cached, _, _)| cached == key)?;
        let entry = self.entries.remove(index)?;
        let summary = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(summary)
    }

    fn insert(&mut self, key: SummaryKey, summary: Arc<LineSummary>) {
        const MAX_BYTES: usize = 8 * 1024 * 1024;
        const MAX_ENTRIES: usize = 32;
        fn style_bytes(style: &ResolvedTextStyle) -> usize {
            style.font_families.capacity() * std::mem::size_of::<String>()
                + style
                    .font_families
                    .iter()
                    .map(String::capacity)
                    .sum::<usize>()
                + style.language.as_ref().map_or(0, String::capacity)
                + style.script.as_ref().map_or(0, String::capacity)
                + style.features.capacity() * std::mem::size_of::<OpenTypeFeature>()
                + style.font_face.capacity()
                + style.font_axes.len() * 96
        }
        let bytes = std::mem::size_of::<(SummaryKey, Arc<LineSummary>, usize)>()
            + std::mem::size_of::<LineSummary>()
            + 2 * std::mem::size_of::<usize>()
            + key.text.retained_identity_bytes()
            + summary.fragments.capacity() * std::mem::size_of::<FragmentSummary>()
            + summary.runs.capacity() * std::mem::size_of::<DirectionRun>()
            + key.style_runs.capacity() * std::mem::size_of::<ShapeStyleRun>()
            + key
                .style_runs
                .iter()
                .map(|run| style_bytes(&run.style))
                .sum::<usize>()
            + style_bytes(&key.default_style)
            + style_bytes(&key.whitespace_style);
        if bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes > MAX_BYTES - bytes {
            if let Some((_, _, removed)) = self.entries.pop_front() {
                self.bytes -= removed;
            }
        }
        self.bytes += bytes;
        self.entries.push_back((key, summary, bytes));
    }
}

fn text_error(error: crate::document::FormattedTextError) -> LayoutComputationError {
    LayoutError::DocumentStyle(DocumentStyleError::TextStorage(error)).into()
}

fn next_break(
    tree: &FormattedTextTree,
    range: Range<usize>,
    state: &mut WrapBreakState,
    cancellation: &dyn LayoutCancellationProbe,
) -> Result<usize, LayoutComputationError> {
    super::super::line_breaks::first_line_break(tree, range, false, state, cancellation).map_err(|error| {
        match error {
            super::super::jobs::LayoutJobError::Cancelled => LayoutComputationError::Cancelled,
            super::super::jobs::LayoutJobError::FormattedText(error) => text_error(error),
            _ => LayoutError::MalformedMeasurement("streamed line-break range is invalid").into(),
        }
    })
}

fn add_statistics(total: &mut LayoutWorkStatistics, work: LayoutWorkStatistics) {
    total.segmented_text_bytes += work.segmented_text_bytes;
    total.shaping_fragment_count += work.shaping_fragment_count;
    total.maximum_shaping_fragment_bytes = total
        .maximum_shaping_fragment_bytes
        .max(work.maximum_shaping_fragment_bytes);
    total.wrapped_cluster_count += work.wrapped_cluster_count;
    total.maximum_wrap_checkpoint_clusters = total
        .maximum_wrap_checkpoint_clusters
        .max(work.maximum_wrap_checkpoint_clusters);
    total.positioned_cluster_count += work.positioned_cluster_count;
    total.maximum_position_checkpoint_clusters = total
        .maximum_position_checkpoint_clusters
        .max(work.maximum_position_checkpoint_clusters);
}

fn chunk_end(
    tree: &FormattedTextTree,
    start: usize,
    end: usize,
) -> Result<usize, LayoutComputationError> {
    bounded_end(tree, start, end, MAX_SHAPE_FRAGMENT_BYTES)
}

fn bounded_end(
    tree: &FormattedTextTree,
    start: usize,
    end: usize,
    limit: usize,
) -> Result<usize, LayoutComputationError> {
    let mut target = start.saturating_add(limit).min(end);
    if target == end {
        return Ok(end);
    }
    while target > start && !tree.is_char_boundary(target).map_err(text_error)? {
        target -= 1;
    }
    if tree.is_grapheme_boundary(target).map_err(text_error)? {
        return Ok(target);
    }
    let previous = tree
        .previous_grapheme_boundary(target)
        .map_err(text_error)?
        .unwrap_or(start);
    if previous > start {
        return Ok(previous);
    }
    tree.next_grapheme_boundary(start)
        .map_err(text_error)?
        .ok_or_else(|| LayoutError::MalformedMeasurement("unwrapped chunk cannot advance").into())
}

fn context(
    tree: &FormattedTextTree,
    text: &Range<usize>,
    line: &Range<usize>,
) -> Result<Range<usize>, LayoutComputationError> {
    let mut start = text.start;
    while start > line.start && text.start - start < SHAPING_CONTEXT_BYTES {
        start = tree
            .previous_grapheme_boundary(start)
            .map_err(text_error)?
            .unwrap_or(line.start)
            .max(line.start);
    }
    let mut end = text.end;
    while end < line.end && end - text.end < SHAPING_CONTEXT_BYTES {
        end = tree
            .next_grapheme_boundary(end)
            .map_err(text_error)?
            .unwrap_or(line.end)
            .min(line.end);
    }
    Ok(start..end)
}

fn base_direction(
    tree: &FormattedTextTree,
    range: Range<usize>,
    style: &ParagraphLayoutStyle,
    control: &LayoutRunControl<'_>,
) -> Result<bool, LayoutComputationError> {
    use unicode_bidi::BidiClass;
    match style.base_direction {
        WritingDirection::LeftToRight => return Ok(false),
        WritingDirection::RightToLeft => return Ok(true),
        WritingDirection::Natural => {}
    }
    let mut at = style
        .list_marker_range
        .as_ref()
        .map_or(range.start, |marker| marker.end.max(range.start))
        .min(range.end);
    let mut isolates = 0usize;
    while at < range.end {
        control.checkpoint()?;
        let bytes = tree.byte_chunk_at(at);
        let bytes = &bytes[..bytes.len().min(range.end - at)];
        let text = std::str::from_utf8(bytes)
            .map_err(|_| LayoutError::MalformedMeasurement("text leaf is not UTF-8"))?;
        for character in text.chars() {
            match unicode_bidi::bidi_class(character) {
                BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => isolates += 1,
                BidiClass::PDI => isolates = isolates.saturating_sub(1),
                BidiClass::L if isolates == 0 => return Ok(false),
                BidiClass::R | BidiClass::AL if isolates == 0 => return Ok(true),
                _ => {}
            }
        }
        at += bytes.len();
    }
    Ok(false)
}

fn run_order(runs: &[DirectionRun]) -> Vec<usize> {
    let mut order: Vec<_> = (0..runs.len()).collect();
    let Some(lowest) = runs
        .iter()
        .map(|run| run.level)
        .filter(|level| level % 2 != 0)
        .min()
    else {
        return order;
    };
    let highest = runs.iter().map(|run| run.level).max().unwrap_or(0);
    for level in (lowest..=highest).rev() {
        let mut at = 0;
        while at < order.len() {
            while at < order.len() && runs[order[at]].level < level {
                at += 1;
            }
            let start = at;
            while at < order.len() && runs[order[at]].level >= level {
                at += 1;
            }
            order[start..at].reverse();
        }
    }
    order
}

fn intersects(bands: &[Range<f32>], left: f64, right: f64) -> bool {
    bands
        .iter()
        .any(|band| left <= f64::from(band.end) && f64::from(band.start) <= right)
}

fn merge_bands(mut bands: Vec<Range<f32>>) -> Vec<Range<f32>> {
    bands.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut result: Vec<Range<f32>> = Vec::new();
    for band in bands {
        if let Some(last) = result.last_mut().filter(|last| band.start <= last.end) {
            last.end = last.end.max(band.end);
        } else {
            result.push(band);
        }
    }
    result
}

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    #[allow(clippy::too_many_arguments)]
    fn layout_wrapped_tree_line(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        tree: &FormattedTextTree,
        full_range: &Range<usize>,
        hard_line_index: usize,
        document_hard_line_count: usize,
        following: Option<Range<usize>>,
        styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        desired_top: f32,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        let mut checkpoint: Option<LongLineLayoutCheckpoint> = None;
        let mut retained_rows = Vec::new();
        let mut horizontal_rows = Vec::new();
        let mut statistics = LayoutWorkStatistics::default();
        let mut diagnostics: Vec<_> = styles.recovery_diagnostic.iter().cloned().collect();
        let mut wanted_top = (desired_top - view.height).max(0.0);
        let mut wanted_bottom = desired_top + view.height * 2.0;
        let focus = view
            .horizontal_focus_offset
            .filter(|offset| full_range.start <= *offset && *offset <= full_range.end);
        loop {
            if cancellation.is_cancelled() {
                return Err(LayoutComputationError::Cancelled);
            }
            let start = checkpoint
                .as_ref()
                .map_or(full_range.start, |checkpoint| checkpoint.next_text_offset);
            let mut break_state = checkpoint.as_ref().map_or_else(
                || WrapBreakState::new(view.whitespace.format.is_code()),
                |checkpoint| checkpoint.wrap_break_state,
            );
            let first_break = next_break(tree, start..full_range.end, &mut break_state, cancellation)?;
            let end = bounded_end(
                tree,
                start,
                full_range.end,
                super::super::jobs::MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
            )?
            .max(first_break);
            let slice = HardLineLayoutSlice {
                full_range: full_range.clone(),
                work_range: start..end,
                shaping_context_range: context(tree, &(start..end), full_range)?,
                hard_line_index,
                indentation_tree: (checkpoint.is_none()
                    && view.wrap
                    && view.whitespace.format.is_code())
                .then(|| tree.clone()),
                checkpoint,
            };
            let mut part = if end - start > super::super::jobs::MAX_LONG_LINE_LAYOUT_SLICE_BYTES
                && first_break == end
            {
                self.layout_overflow_slice_cancellable(
                    document_id,
                    document_revision,
                    tree,
                    &slice,
                    document_hard_line_count,
                    following.clone(),
                    styles,
                    view,
                    cancellation,
                )?
            } else {
                let text = tree
                    .slice(slice.shaping_context_range.clone())
                    .map_err(text_error)?;
                self.layout_hard_line_slices_cancellable(
                    document_id,
                    document_revision,
                    &text,
                    slice.shaping_context_range.start,
                    std::slice::from_ref(&slice),
                    document_hard_line_count,
                    tree.byte_len(),
                    (end == full_range.end).then(|| following.clone()).flatten(),
                    styles,
                    view,
                    cancellation,
                )?
            };
            add_statistics(&mut statistics, part.work_statistics);
            diagnostics.extend(part.diagnostics.iter().cloned());
            let line = &mut part.lines[0];
            if let Some(focus_row) = focus.and_then(|focus| {
                line.rows
                    .iter()
                    .find(|row| row.text_range.start <= focus && focus <= row.text_range.end)
            }) {
                if focus_row.y >= wanted_bottom {
                    wanted_top = (focus_row.y - view.height).max(0.0);
                    wanted_bottom = focus_row.y + view.height * 2.0;
                    retained_rows.clear();
                    horizontal_rows.clear();
                }
            }
            let last = line.rows.last().cloned();
            retained_rows.extend(
                line.rows
                    .drain(..)
                    .filter(|row| row.y + row.height() > wanted_top && row.y < wanted_bottom),
            );
            if let Some(sparse) = &part.horizontal_materialization {
                horizontal_rows.extend(sparse.rows.iter().cloned());
            }
            checkpoint = line.next_checkpoint.clone();
            let focus_reached =
                focus.is_none_or(|focus| focus < line.text_coverage.end || checkpoint.is_none());
            if checkpoint.is_none() || (line.height >= f64::from(wanted_bottom) && focus_reached) {
                if retained_rows.is_empty() {
                    retained_rows.extend(last);
                }
                line.rows = retained_rows;
                line.text_coverage = line.rows.first().unwrap().text_range.start
                    ..line.rows.last().unwrap().text_range.end;
                horizontal_rows.retain(|(line_index, fragment, _)| {
                    line.rows.iter().any(|row| {
                        row.hard_line_index == *line_index && row.fragment_index == *fragment
                    })
                });
                part.horizontal_materialization = Some(HorizontalMaterialization {
                    text: tree.clone(),
                    rows: horizontal_rows,
                });
                part.grapheme_boundaries.clear();
                part.work_statistics = statistics;
                part.diagnostics = diagnostics;
                return Ok(part);
            }
        }
    }

    pub(super) fn shape_unwrapped_fragment(
        &mut self,
        document_id: DocumentId,
        revision: Revision,
        tree: &FormattedTextTree,
        fragment: Range<usize>,
        line: &Range<usize>,
        default_style: &ResolvedTextStyle,
        style_runs: &[ShapeStyleRun],
        direction: TextDirection,
        view: &LayoutJobViewConfiguration,
        control: &LayoutRunControl<'_>,
    ) -> Result<ShapedFragment, LayoutComputationError> {
        let context = context(tree, &fragment, line)?;
        let text = tree.slice(context.clone()).map_err(text_error)?;
        let mut result = self.shape_ranges_with_origin(
            document_id,
            revision,
            &text,
            &[fragment.start - context.start..fragment.end - context.start],
            &[0..text.len()],
            context.start,
            style_runs,
            std::slice::from_ref(default_style),
            &[direction],
            view.scale,
            self.provider.measurement_environment_id(),
            self.provider.metrics_generation(),
            control,
        )?;
        Ok(result.remove(0))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_unwrapped_viewport_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        tree: &FormattedTextTree,
        line_ranges: &[Range<usize>],
        first_hard_line: usize,
        document_hard_line_count: usize,
        following_line_range: Option<Range<usize>>,
        styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        if styles.table_context.as_ref().is_some_and(|context| line_ranges.iter().any(|range| context.contains_line(range))) {
            return self.layout_table_region_from_tree(document_id, document_revision, tree, line_ranges,
                first_hard_line, document_hard_line_count, following_line_range, styles, view, cancellation);
        }
        self.layout_streamed_rows_cancellable(
            document_id,
            document_revision,
            tree,
            line_ranges,
            first_hard_line,
            document_hard_line_count,
            following_line_range,
            styles,
            view,
            cancellation,
            None,
        )
        .map(|(region, _)| region)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_overflow_slice_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        tree: &FormattedTextTree,
        slice: &HardLineLayoutSlice,
        document_hard_line_count: usize,
        following_line_range: Option<Range<usize>>,
        styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        let (mut region, code_wrap_indent) = self.layout_streamed_rows_cancellable(
            document_id,
            document_revision,
            tree,
            std::slice::from_ref(&slice.work_range),
            slice.hard_line_index,
            document_hard_line_count,
            following_line_range.clone(),
            styles,
            view,
            cancellation,
            Some(slice),
        )?;
        let extends = slice.work_range.end < slice.full_range.end;
        let row = &region.lines[0].rows[0];
        if extends && row.width < row.paragraph_content_width {
            // Byte length alone does not establish overflow. An unusually wide
            // viewport or a nearly zero-advance word can fit with following
            // content, so retain the existing exact complete-row computation.
            let mut tail = slice.clone();
            tail.work_range.end = tail.full_range.end;
            tail.shaping_context_range.end = tail.full_range.end;
            let text = tree
                .slice(tail.shaping_context_range.clone())
                .map_err(text_error)?;
            return self.layout_hard_line_slices_cancellable(
                document_id,
                document_revision,
                &text,
                tail.shaping_context_range.start,
                std::slice::from_ref(&tail),
                document_hard_line_count,
                tree.byte_len(),
                following_line_range,
                styles,
                view,
                cancellation,
            );
        }
        let starting_row = slice
            .checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.completed_visual_rows);
        let delta = slice
            .checkpoint
            .as_ref()
            .map_or(0.0, |checkpoint| checkpoint.completed_height - row.y);
        let default_style = if view.default_style_is_override {
            &view.default_style
        } else {
            &styles.default_shaping_style
        };
        let paragraph =
            resolve_line_paragraph(&slice.full_range, &styles.paragraphs, default_style);
        let right_to_left = if let Some(checkpoint) = &slice.checkpoint {
            checkpoint.right_to_left
        } else {
            base_direction(
                tree,
                slice.full_range.clone(),
                &paragraph.style,
                &LayoutRunControl::cancellable(cancellation),
            )?
        };
        let (_, continuation) = paragraph_row_boxes(
            region.content_insets.left,
            region.usable_width,
            &paragraph.style,
            right_to_left,
            view.scale,
        );
        let code_wrap_indent = code_wrap_indent.unwrap_or(0.0);
        let continuation = if view.wrap && view.whitespace.format.is_code() {
            code_continuation_box(continuation, code_wrap_indent, right_to_left)
        } else {
            continuation
        };
        let line = &mut region.lines[0];
        let row = &mut line.rows[0];
        translate_row_vertically(row, delta).map_err(LayoutError::from)?;
        row.hard_line_range = slice.full_range.clone();
        row.fragment_index = starting_row;
        row.wrapped_from_previous = starting_row > 0;
        row.wraps_to_next = extends;
        line.hard_line_range = slice.full_range.clone();
        line.height = if extends {
            f64::from(row.y + row.height())
        } else {
            line.height + f64::from(delta)
        };
        line.height_is_exact = !extends;
        let mut wrap_break_state = slice.checkpoint.as_ref().map_or_else(
            || WrapBreakState::new(view.whitespace.format.is_code()),
            |checkpoint| checkpoint.wrap_break_state,
        );
        if extends {
            // This slice is one indivisible overflow segment. Advance directly
            // over tree leaves, retaining no flat copy of a potentially huge word.
            let end = next_break(tree, slice.work_range.clone(), &mut wrap_break_state, cancellation)?;
            debug_assert_eq!(end, slice.work_range.end);
        }
        line.next_checkpoint = extends.then(|| LongLineLayoutCheckpoint {
            wrap_break_state,
            // A soft wrap can no longer occur inside the initial whitespace.
            whitespace_leading: false,
            code_wrap_indent,
            document_id,
            document_revision,
            configuration_generation: view.configuration_generation,
            measurement_environment_id: region.measurement_environment_id,
            metrics_generation: region.metrics_generation,
            hard_line_index: slice.hard_line_index,
            hard_line_range: slice.full_range.clone(),
            next_text_offset: slice.work_range.end,
            completed_visual_rows: starting_row + 1,
            completed_height: row.y + row.height(),
            cumulative_advance: slice
                .checkpoint
                .as_ref()
                .map_or(0.0, |checkpoint| checkpoint.cumulative_advance)
                + f64::from(row.width),
            last_candidate_break: Some(slice.work_range.end),
            continuation_width: continuation.width,
            right_to_left,
        });
        if let Some(sparse) = &mut region.horizontal_materialization {
            for (_, fragment, _) in &mut sparse.rows {
                *fragment = starting_row;
            }
        }
        Ok(region)
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_streamed_rows_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        tree: &FormattedTextTree,
        line_ranges: &[Range<usize>],
        first_hard_line: usize,
        document_hard_line_count: usize,
        following_line_range: Option<Range<usize>>,
        styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
        overflow_slice: Option<&HardLineLayoutSlice>,
    ) -> Result<(RegionalLayoutSnapshot, Option<f32>), LayoutComputationError> {
        let control = LayoutRunControl::cancellable(cancellation);
        let measurement_environment_id = self.provider.measurement_environment_id();
        let metrics_generation = self.provider.metrics_generation();
        let mut whitespace_unit = styles.whitespace_shaping_style.size * view.scale * 0.5;
        let layout_revision = LayoutRevision(
            view.latest_layout_revision
                .map_or(self.next_layout_revision, |old| {
                    self.next_layout_revision.max(old.0.wrapping_add(1).max(1))
                }),
        );
        let default_style = if view.default_style_is_override {
            &view.default_style
        } else {
            &styles.default_shaping_style
        };
        let style_runs = if view.style_runs_are_override {
            &view.style_runs
        } else {
            &styles.shaping_runs
        };
        validate_style_runs_structure(style_runs)?;
        let content_insets = add_insets(view.insets, styles.document_insets);
        let usable_width = usable_width(view.width, content_insets);
        let mut lines: Vec<RegionalHardLineLayout> = Vec::with_capacity(line_ranges.len());
        let mut horizontal_rows = Vec::new();
        let mut diagnostics = Vec::new();
        let mut statistics = LayoutWorkStatistics::default();
        let mut overflow_code_wrap_indent = None;
        for (line_offset, line_range) in line_ranges.iter().enumerate() {
            control.checkpoint()?;
            let hard_line_index = first_hard_line + line_offset;
            let following = line_ranges
                .get(line_offset + 1)
                .cloned()
                .or_else(|| following_line_range.clone());
            if line_range.len() <= super::super::jobs::MAX_LONG_LINE_LAYOUT_SLICE_BYTES {
                let text = tree.slice(line_range.clone()).map_err(text_error)?;
                let region = self.layout_hard_line_region_cancellable(
                    document_id,
                    document_revision,
                    &text,
                    line_range.start,
                    std::slice::from_ref(line_range),
                    hard_line_index,
                    document_hard_line_count,
                    tree.byte_len(),
                    following,
                    styles,
                    view,
                    cancellation,
                )?;
                add_statistics(&mut statistics, region.work_statistics);
                lines.extend(region.lines);
                diagnostics.extend(region.diagnostics);
                continue;
            }
            if view.wrap
                && overflow_slice.is_none()
                && next_break(tree, line_range.clone(), &mut WrapBreakState::new(view.whitespace.format.is_code()), cancellation)? < line_range.end
            {
                let preceding_height: f64 = lines.iter().map(|line| line.height).sum();
                let desired_top = (view.regional_viewport_top - preceding_height as f32).max(0.0);
                let region = self.layout_wrapped_tree_line(
                    document_id,
                    document_revision,
                    tree,
                    line_range,
                    hard_line_index,
                    document_hard_line_count,
                    following,
                    styles,
                    view,
                    desired_top,
                    cancellation,
                )?;
                add_statistics(&mut statistics, region.work_statistics);
                if let Some(sparse) = &region.horizontal_materialization {
                    horizontal_rows.extend(sparse.rows.iter().cloned());
                }
                let partial = region
                    .lines
                    .last()
                    .is_some_and(|line| !line.height_is_exact);
                lines.extend(region.lines);
                diagnostics.extend(region.diagnostics);
                if partial {
                    break;
                }
                continue;
            }
            let full_range = overflow_slice.map_or(line_range, |slice| &slice.full_range);
            let mut paragraph =
                resolve_line_paragraph(full_range, &styles.paragraphs, default_style);
            if overflow_slice.is_some_and(|slice| slice.checkpoint.is_some()) {
                paragraph.is_first_hard_line = false;
            }
            let shaping_style = if view.default_style_is_override {
                default_style
            } else {
                &paragraph.style.default_shaping_style
            };
            let right_to_left = if let Some(checkpoint) =
                overflow_slice.and_then(|slice| slice.checkpoint.as_ref())
            {
                checkpoint.right_to_left
            } else {
                base_direction(tree, full_range.clone(), &paragraph.style, &control)?
            };
            let direction = if right_to_left { TextDirection::RightToLeft } else { TextDirection::LeftToRight };
            let summary_key = SummaryKey {
                code_wrap: view.wrap && view.whitespace.format.is_code(),
                whitespace_style: styles.whitespace_shaping_style.clone(),
                whitespace_basis: view.whitespace.basis(),
                whitespace_tabstop: view.whitespace.tabstop,
                whitespace_leading: overflow_slice
                    .and_then(|s| s.checkpoint.as_ref())
                    .map_or(true, |c| c.whitespace_leading),
                whitespace_advance: overflow_slice
                    .and_then(|s| s.checkpoint.as_ref())
                    .map_or(0.0, |c| c.cumulative_advance)
                    .to_bits(),
                text: tree.snapshot_identity(),
                line: line_range.clone(),
                context_line: full_range.clone(),
                default_style: shaping_style.clone(),
                style_runs: style_runs.to_vec(),
                direction,
                marker: paragraph.style.list_marker_range.clone(),
                scale: view.scale.to_bits(),
                environment: measurement_environment_id,
                metrics: metrics_generation,
                render_policy: self.provider.render_run_policy(),
            };
            let summary = if let Some(summary) = self.unwrapped_summary_cache.get(&summary_key) {
                summary
            } else {
                let mut fragments = Vec::new();
                let mut runs: Vec<DirectionRun> = Vec::new();
                let mut metrics = TextMetrics {
                    ascent: 0.0,
                    descent: 0.0,
                    leading: 0.0,
                };
                let mut at = line_range.start;
                let mut expected = at;
                let mut width = 0.0f64;
                let mut marker_width = 0.0f32;
                let mut marker_end = line_range.start;
                let mut whitespace_leading = summary_key.whitespace_leading;
                let mut whitespace_advance = f64::from_bits(summary_key.whitespace_advance);
                let mut leading_indent_width = 0.0;
                let mut measuring_leading_indent = whitespace_leading;
                while at < line_range.end {
                    control.checkpoint()?;
                    let end = chunk_end(tree, at, line_range.end)?;
                    let mut fragment = self.shape_unwrapped_fragment(
                        document_id,
                        document_revision,
                        tree,
                        at..end,
                        full_range,
                        shaping_style,
                        style_runs,
                        direction,
                        view,
                        &control,
                    )?;
                    let initial_whitespace = (whitespace_leading, whitespace_advance);
                    let actual_end = fragment.clusters.last().map_or(end, |c| c.text_range.end);
                    let whitespace_text = tree.slice(at..actual_end).map_err(text_error)?;
                    (whitespace_leading, whitespace_advance, _) = self.layout_whitespace_clusters(
                        &mut fragment.clusters,
                        &whitespace_text,
                        at,
                        &view.whitespace,
                        whitespace_unit,
                        shaping_style,
                        style_runs,
                        view.scale,
                        document_id,
                        document_revision,
                        &control,
                        whitespace_leading,
                        whitespace_advance,
                    )?;
                    let run_start = runs.len();
                    for cluster in &fragment.clusters {
                        if cluster.text_range.start != expected
                            || cluster.text_range.end > line_range.end
                        {
                            return Err(LayoutError::MalformedMeasurement(
                                "streamed fragments do not cover line",
                            )
                            .into());
                        }
                        expected = cluster.text_range.end;
                        if measuring_leading_indent {
                            let spelling = &whitespace_text
                                [cluster.text_range.start - at..cluster.text_range.end - at];
                            measuring_leading_indent =
                                spelling.bytes().all(|byte| byte == b' ' || byte == b'\t');
                            if measuring_leading_indent {
                                leading_indent_width += f64::from(cluster.advance);
                            }
                        }
                        metrics.ascent = metrics.ascent.max(cluster.metrics.ascent);
                        metrics.descent = metrics.descent.max(cluster.metrics.descent);
                        metrics.leading = metrics.leading.max(cluster.metrics.leading);
                        width += f64::from(cluster.advance);
                        if paragraph
                            .style
                            .list_marker_range
                            .as_ref()
                            .is_some_and(|marker| {
                                marker.start == line_range.start
                                    && cluster.text_range.end <= marker.end
                            })
                        {
                            marker_width += cluster.advance;
                            marker_end = cluster.text_range.end;
                        }
                        let extends_run = runs.len() > run_start
                            && runs.last().is_some_and(|run| {
                                run.level == cluster.bidi_level
                                    && run.text.end == cluster.text_range.start
                            });
                        if extends_run {
                            let run = runs.last_mut().unwrap();
                            run.text.end = cluster.text_range.end;
                            run.width += f64::from(cluster.advance);
                        } else {
                            runs.push(DirectionRun {
                                text: cluster.text_range.clone(),
                                level: cluster.bidi_level,
                                width: f64::from(cluster.advance),
                                left: 0.0,
                            });
                        }
                    }
                    // Keep run ownership within each shaping fragment. Even a
                    // uniform multi-gigabyte line therefore has a sparse index.
                    fragments.push(FragmentSummary {
                        whitespace_leading: initial_whitespace.0,
                        whitespace_advance: initial_whitespace.1,
                        text: at..end,
                        runs: run_start..runs.len(),
                    });
                    statistics.segmented_text_bytes += end - at;
                    statistics.shaping_fragment_count += 1;
                    statistics.maximum_shaping_fragment_bytes =
                        statistics.maximum_shaping_fragment_bytes.max(end - at);
                    at = end;
                }
                if expected != line_range.end {
                    return Err(
                        LayoutError::MalformedMeasurement("streamed line is incomplete").into(),
                    );
                }
                if summary_key.code_wrap
                    && measuring_leading_indent
                    && line_range.start == full_range.start
                    && line_range.end < full_range.end
                {
                    // The first overflowing segment can end within an enormous
                    // indentation prefix. Cache the complete prefix measurement
                    // so every later row still uses the original line's margin.
                    let (complete_indent_width, prefix_statistics) = self
                        .code_leading_indent_from_tree(
                            tree,
                            full_range,
                            view,
                            whitespace_unit,
                            shaping_style,
                            style_runs,
                            document_id,
                            document_revision,
                            &control,
                        )?;
                    leading_indent_width = complete_indent_width;
                    add_statistics(&mut statistics, prefix_statistics);
                }
                let summary = Arc::new(LineSummary {
                    whitespace_unit,
                    leading_indent_width,
                    fragments,
                    runs,
                    metrics,
                    width,
                    marker_width,
                    marker_end,
                });
                self.unwrapped_summary_cache
                    .insert(summary_key, Arc::clone(&summary));
                summary
            };
            whitespace_unit = summary.whitespace_unit;
            let fragments = &summary.fragments;
            let mut runs = summary.runs.clone();
            let metrics = summary.metrics.clone();
            let width = summary.width;
            let marker_width = summary.marker_width;
            let marker_end = summary.marker_end;
            let (first_box, continuation_box) = paragraph_row_boxes(
                content_insets.left,
                usable_width,
                &paragraph.style,
                right_to_left,
                view.scale,
            );
            let code_wrap_indent = if view.wrap && view.whitespace.format.is_code() {
                if let Some(checkpoint) = overflow_slice.and_then(|slice| slice.checkpoint.as_ref())
                {
                    checkpoint.code_wrap_indent
                } else {
                    let extra_indent = self.code_wrap_extra_indent(
                        full_range.start,
                        view,
                        &styles.whitespace_shaping_style,
                        shaping_style,
                        style_runs,
                        document_id,
                        document_revision,
                        &control,
                    )?;
                    (summary.leading_indent_width + f64::from(extra_indent)) as f32
                }
            } else {
                0.0
            };
            if overflow_slice.is_some() {
                overflow_code_wrap_indent = Some(code_wrap_indent);
            }
            let first_box = if paragraph
                .style
                .list_marker_range
                .as_ref()
                .is_some_and(|marker| {
                    !marker.is_empty()
                        && marker.start == line_range.start
                        && marker.end == marker_end
                }) {
                if right_to_left {
                    row_box(
                        continuation_box.x,
                        continuation_box.x + continuation_box.width
                            - paragraph.style.first_line_indent
                            + marker_width,
                    )
                } else {
                    row_box(
                        continuation_box.x + paragraph.style.first_line_indent - marker_width,
                        continuation_box.x + continuation_box.width,
                    )
                }
            } else {
                first_box
            };
            let paragraph_box = if view.wrap
                && view.whitespace.format.is_code()
                && overflow_slice.is_some_and(|slice| slice.checkpoint.is_some())
            {
                code_continuation_box(continuation_box, code_wrap_indent, right_to_left)
            } else if paragraph.is_first_hard_line {
                first_box
            } else {
                continuation_box
            };
            let left = aligned_row_x(
                paragraph_box,
                width as f32,
                paragraph.style.alignment,
                right_to_left,
            );
            let mut x = f64::from(left);
            for index in run_order(&runs) {
                runs[index].left = x;
                x += runs[index].width;
            }
            let overscan = view.width.max(64.0);
            let mut bands =
                vec![(view.viewport_left - overscan)..(view.viewport_left + view.width + overscan)];
            if let Some(desired) = view.horizontal_desired_x {
                bands.push(desired - overscan..desired + overscan);
            }
            if let Some(focus) = view.horizontal_focus_offset {
                for run in runs
                    .iter()
                    .filter(|run| run.text.start <= focus && focus <= run.text.end)
                {
                    bands
                        .push(run.left as f32 - overscan..(run.left + run.width) as f32 + overscan);
                }
            }
            let bands = merge_bands(bands);
            let (y, limited_origin) = block_box::editable_flow_position(None, if hard_line_index == 0 {
                content_insets.top + block_box::before(&paragraph.style) * view.scale
            } else {
                0.0
            }, view.scale);
            if limited_origin { diagnostics.push(ShapingDiagnostic { text_range: line_range.clone(), message: block_box::REVERSE_FLOW_DIAGNOSTIC.into() }); }
            let baseline = y + metrics.ascent;
            let mut row = VisualRow {
                table_cell: None,
                table_widths_are_exact: None,
                paragraph_id: paragraph.paragraph_id,
                hard_line_index,
                fragment_index: 0,
                hard_line_range: line_range.clone(),
                text_range: line_range.clone(),
                y,
                baseline,
                ascent: metrics.ascent,
                descent: metrics.descent,
                leading: metrics.leading,
                line_advance: line_advance(metrics.height(), paragraph.style.line_spacing),
                width: width as f32,
                paragraph_content_x: paragraph_box.x,
                paragraph_content_width: paragraph_box.width,
                wrapped_from_previous: false,
                wraps_to_next: false,
                clusters: Vec::new(),
                carets: Vec::new(),
                decorations: Vec::new(),
            };
            for (fragment_index, summary) in fragments.iter().enumerate() {
                control.checkpoint()?;
                let endpoints = fragment_index == 0
                    || fragment_index + 1 == fragments.len()
                    || runs[summary.runs.clone()].iter().any(|run| {
                        run.left == f64::from(left)
                            || run.left + run.width == f64::from(left) + width
                    });
                if !endpoints
                    && !runs[summary.runs.clone()]
                        .iter()
                        .any(|run| intersects(&bands, run.left, run.left + run.width))
                {
                    continue;
                }
                let mut fragment = self.shape_unwrapped_fragment(
                    document_id,
                    document_revision,
                    tree,
                    summary.text.clone(),
                    full_range,
                    shaping_style,
                    style_runs,
                    direction,
                    view,
                    &control,
                )?;
                let actual_end = fragment
                    .clusters
                    .last()
                    .map_or(summary.text.end, |c| c.text_range.end);
                let whitespace_text = tree
                    .slice(summary.text.start..actual_end)
                    .map_err(text_error)?;
                let (_, _, tab_units) = self.layout_whitespace_clusters(
                    &mut fragment.clusters,
                    &whitespace_text,
                    summary.text.start,
                    &view.whitespace,
                    whitespace_unit,
                    shaping_style,
                    style_runs,
                    view.scale,
                    document_id,
                    document_revision,
                    &control,
                    summary.whitespace_leading,
                    summary.whitespace_advance,
                )?;
                statistics.segmented_text_bytes += summary.text.len();
                statistics.shaping_fragment_count += 1;
                for run in &runs[summary.runs.clone()] {
                    let start = fragment
                        .clusters
                        .partition_point(|cluster| cluster.text_range.start < run.text.start);
                    let end = fragment
                        .clusters
                        .partition_point(|cluster| cluster.text_range.start < run.text.end);
                    let clusters = &fragment.clusters[start..end];
                    let mut x = run.left;
                    for offset in 0..clusters.len() {
                        let cluster = &clusters[if run.level % 2 == 0 {
                            offset
                        } else {
                            clusters.len() - offset - 1
                        }];
                        let right = x + f64::from(cluster.advance);
                        let is_endpoint = cluster.text_range.start == line_range.start
                            || cluster.text_range.end == line_range.end
                            || x == f64::from(left)
                            || right == f64::from(left) + width;
                        let ink_left = x + f64::from(cluster.ink_bounds.x);
                        let ink_right = ink_left + f64::from(cluster.ink_bounds.width);
                        if is_endpoint || intersects(&bands, x.min(ink_left), right.max(ink_right))
                        {
                            let cluster_x = x as f32;
                            row.clusters.push(PositionedCluster {
                                whitespace_unit: tab_units.get(&cluster.text_range.start).copied(),
                                text_range: cluster.text_range.clone(),
                                x: cluster_x,
                                advance: cluster.advance,
                                typographic_bounds: position_shaped_bounds(
                                    cluster.typographic_bounds,
                                    cluster_x,
                                    baseline,
                                ),
                                ink_bounds: position_shaped_bounds(
                                    cluster.ink_bounds,
                                    cluster_x,
                                    baseline,
                                ),
                                bidi_level: cluster.bidi_level,
                                fallback_font: Arc::clone(&cluster.fallback_font),
                                render_run: cluster.render_run.clone(),
                            });
                            for caret in &cluster.caret_stops {
                                row.carets.push(PositionedCaret {
                                    point: CaretPoint {
                                        document_id,
                                        document_revision,
                                        layout_revision,
                                        text_offset: caret.text_offset,
                                        affinity: caret.affinity,
                                    },
                                    x: cluster_x + caret.inline_offset,
                                    row_index: 0,
                                });
                            }
                        }
                        x = right;
                    }
                }
                diagnostics.extend(fragment.diagnostics);
            }
            row.clusters.sort_by(|a, b| {
                a.x.total_cmp(&b.x)
                    .then(a.text_range.start.cmp(&b.text_range.start))
            });
            row.carets.sort_by(|a, b| {
                a.x.total_cmp(&b.x)
                    .then(a.point.text_offset.cmp(&b.point.text_offset))
                    .then(affinity_rank(a.point.affinity).cmp(&affinity_rank(b.point.affinity)))
            });
            row.carets.dedup_by(|a, b| a.point == b.point && a.x == b.x);
            if paragraph.is_first_hard_line {
                self.decorate_list_row(
                    &mut row,
                    &paragraph.style,
                    right_to_left,
                    document_id,
                    document_revision,
                    view.default_style_is_override.then_some(default_style),
                    view.scale,
                    measurement_environment_id,
                    metrics_generation,
                    &control,
                )?;
            }
            decorate_quote_row(&mut row, &paragraph.style, view.scale);
            let mut height = row.y + row.height();
            let next = following
                .as_ref()
                .map(|range| {
                    resolve_flow_line_paragraph(
                        range,
                        &styles.paragraphs,
                        default_style,
                        styles.table_context.as_ref(),
                    )
                })
                .transpose()?;
            let inputs = super::line_layout_inputs(
                &line_range, &paragraph, next.as_ref(), style_runs, styles,
            );
            let mut resolved_gap = 0.;
            if let Some(next) = &next {
                if starts_new_paragraph(&paragraph, &next) {
                    let (position, limited) = block_box::editable_flow_position(Some(y),
                        height + block_box::between(&paragraph.style, &next.style) * view.scale, view.scale);
                    resolved_gap = position - height;
                    height = position;
                    if limited { diagnostics.push(ShapingDiagnostic { text_range: line_range.clone(), message: block_box::REVERSE_FLOW_DIAGNOSTIC.into() }); }
                }
            }
            decorate_block_row(&mut row, &paragraph.style, resolved_gap,
                content_insets.left, usable_width, view.scale, right_to_left);
            if next.is_none() {
                height = document_end_extent(
                    height, Some(&row), block_box::after(&paragraph.style) * view.scale, content_insets.bottom,
                );
            }
            statistics.positioned_cluster_count += row.clusters.len();
            statistics.maximum_position_checkpoint_clusters = statistics
                .maximum_position_checkpoint_clusters
                .max(row.clusters.len().min(CANCELLATION_CLUSTER_BATCH));
            horizontal_rows.push((hard_line_index, 0, bands));
            lines.push(RegionalHardLineLayout {
                render_run_policy: self.provider.render_run_policy(),
                diagnostics: Vec::new(), // Sparse horizontal results are not cached.
                layout_revision,
                hard_line_index,
                hard_line_range: line_range.clone(),
                text_coverage: line_range.clone(),
                rows: vec![row],
                height: f64::from(height),
                height_is_exact: true,
                next_checkpoint: None,
                inputs,
            });
        }
        control.checkpoint()?;
        if self.provider.measurement_environment_id() != measurement_environment_id {
            return Err(LayoutError::MeasurementEnvironmentChangedDuringShape.into());
        }
        if self.provider.metrics_generation() != metrics_generation {
            return Err(LayoutError::MetricsChangedDuringShape.into());
        }
        self.next_layout_revision = self
            .next_layout_revision
            .max(layout_revision.0.wrapping_add(1).max(1));
        let mut paint_runs = styles.paint_runs.clone();
        normalize_search_paint(
            &mut paint_runs,
            &styles.default_paint,
            lines.iter().flat_map(|line| &line.rows)
                .flat_map(|row| &row.clusters).map(|cluster| &cluster.text_range),
            styles.search_paint_overlay.as_ref(),
        );
        Ok((
            RegionalLayoutSnapshot {
                whitespace_unit,
                revision: layout_revision,
                document_id,
                document_revision,
                configuration_generation: view.configuration_generation,
                measurement_environment_id,
                metrics_generation,
                hard_lines: first_hard_line..first_hard_line + lines.len(),
                document_hard_line_count,
                document_text_len: tree.byte_len(),
                viewport_width: view.width,
                viewport_height: view.height,
                usable_width,
                content_insets,
                document_insets: styles.document_insets,
                document_style_revision: Some(styles.style_sheet_revision),
                canvas_background: styles.canvas_background,
                canvas_background_is_default: styles.canvas_background_is_default,
                default_paint: styles.default_paint.clone(),
                paint_runs,
                lines,
                diagnostics,
                grapheme_boundaries: Vec::new(),
                work_statistics: statistics,
                horizontal_materialization: Some(HorizontalMaterialization {
                    text: tree.clone(),
                    rows: horizontal_rows,
                }),
            },
            overflow_code_wrap_indent,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{
        compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
        LayoutCancellationToken, LayoutExecutionContext, LayoutInstallTarget, LayoutJobId,
        LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider, ViewportLayoutRegion,
    };

    fn materialize(
        document: &Document,
        engine: &mut LayoutEngine<MockTextMeasurementProvider>,
        view: &mut ViewLayout,
        job: u64,
        focus: usize,
    ) -> LayoutWorkStatistics {
        let requirements = inspect_layout_provider(engine);
        let region = ViewportLayoutRegion::new(0..document.line_count(), 0.0, view.height())
            .unwrap()
            .with_horizontal_focus(focus, None);
        let request = prepare_layout_job(
            document,
            view,
            requirements,
            LayoutJobId(job),
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request.captured_text_len(), 0);
        let candidate =
            compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let work = candidate.regional_snapshot().work_statistics();
        install_layout_job(
            view,
            LayoutInstallTarget {
                document_id: document.id(),
                document_revision: document.revision(),
                measurement_environment_id: requirements.measurement_environment_id,
                metrics_generation: requirements.metrics_generation,
            },
            candidate,
        )
        .unwrap();
        work
    }

    #[test]
    fn horizontal_windows_match_complete_bidi_ligature_and_grapheme_geometry() {
        let text = format!(
            "{}fi AV e\u{301} שלום مرحبا 👩‍🚀 {}",
            "a".repeat(4095),
            "fi AV e\u{301} שלום مرحبا 👩‍🚀 ".repeat(2500)
        );
        assert!(text.len() > super::super::super::jobs::MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        let document = Document::new(text);
        let mut reference_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut reference_view = ViewLayout::new(320.0, 80.0);
        reference_view.set_wrap(false);
        reference_engine
            .relayout(&document, &mut reference_view)
            .unwrap();
        let reference = reference_view.snapshot().unwrap();
        let reference_row = &reference.rows[0];
        let expected: BTreeMap<_, _> = reference_row
            .clusters
            .iter()
            .map(|cluster| (cluster.text_range.start, cluster))
            .collect();
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(320.0, 80.0);
        view.set_wrap(false);
        for (job, left) in [
            0.0,
            reference_row.width / 2.0,
            (reference_row.width - 320.0).max(0.0),
        ]
        .into_iter()
        .enumerate()
        {
            view.set_viewport_left(left).unwrap();
            materialize(&document, &mut engine, &mut view, job as u64 + 1, 4096);
            let snapshot = view.snapshot().unwrap();
            let row = &snapshot.rows[0];
            assert_eq!(row.width, reference_row.width);
            assert_eq!(row.baseline, reference_row.baseline);
            assert_eq!(row.line_advance, reference_row.line_advance);
            assert!(row.clusters.len() < reference_row.clusters.len() / 4);
            for cluster in &row.clusters {
                let expected = expected[&cluster.text_range.start];
                assert_eq!(cluster.text_range, expected.text_range);
                assert!(
                    (cluster.x - expected.x).abs() <= 0.02,
                    "cluster {:?}: streamed {} reference {}",
                    cluster.text_range,
                    cluster.x,
                    expected.x
                );
                assert_eq!(cluster.advance, expected.advance);
                assert_eq!(cluster.bidi_level, expected.bidi_level);
            }
            for x in [left, left + 160.0, left + 319.0] {
                let point = LayoutPoint { x, y: row.y + 1.0 };
                let actual = snapshot.hit_test(point).unwrap();
                let expected = reference.hit_test(point).unwrap();
                assert_eq!(
                    (actual.text_offset, actual.affinity),
                    (expected.text_offset, expected.affinity)
                );
            }
            let full = TextRange::new(
                document.text_point(0).unwrap(),
                document.text_point(document.text().len()).unwrap(),
            )
            .unwrap();
            assert!(!snapshot
                .selection_rectangles(full, BoundaryAffinity::Downstream)
                .unwrap()
                .is_empty());
            // A logical endpoint within the cross-fragment fi ligature keeps
            // its original selection extent and uses containing-cluster ink.
            let geometry = snapshot
                .logical_endpoint_geometry(4096, BoundaryAffinity::Downstream)
                .unwrap();
            assert!(geometry.is_cluster_fallback);
        }
        let before = engine.provider().request_calls();
        engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        view.resize(240.0, 100.0);
        materialize(&document, &mut engine, &mut view, 4, 4096);
        assert_eq!(
            view.snapshot().unwrap().metrics_generation,
            MetricsGeneration(2)
        );
        assert!(engine.provider().request_calls() > before);
    }

    #[test]
    fn multi_megabyte_ascii_line_retains_bounded_real_cluster_and_caret_arrays() {
        let document = Document::new("AV fi word ".repeat(200_000));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(400.0, 100.0);
        view.set_wrap(false);
        materialize(&document, &mut engine, &mut view, 1, 1_000_000);
        let snapshot = view.snapshot().unwrap();
        assert!(snapshot.rows[0].clusters.len() < 10_000);
        assert!(snapshot.rows[0].carets.len() < 20_000);
        assert!(snapshot.grapheme_boundaries.is_empty());
        assert!(snapshot
            .logical_endpoint_geometry(1_000_000, BoundaryAffinity::Downstream)
            .is_ok());
        assert_eq!(
            snapshot.logical_endpoint_geometry(500_000, BoundaryAffinity::Downstream),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
        assert!(engine.shaping_cache_statistics().estimated_bytes <= DEFAULT_SHAPE_CACHE_BYTES);
        assert!(view.regional_cached_ranges().is_empty());
    }

    #[test]
    fn wrapped_multi_megabyte_word_streams_overflow_and_reuses_only_compact_summaries() {
        let mut document = Document::new("a".repeat(2 * 1024 * 1024));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(400.0, 100.0);
        assert!(view.wrap());
        materialize(&document, &mut engine, &mut view, 1, 0);
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 1);
        assert!(snapshot.rows[0].clusters.len() < 5000);
        assert!(snapshot.grapheme_boundaries.is_empty());
        let width = snapshot.rows[0].width;
        assert!(engine.unwrapped_summary_cache.bytes < 128 * 1024);
        view.set_viewport_left(width / 2.0).unwrap();
        let scrolled_work = materialize(&document, &mut engine, &mut view, 2, 0);
        assert_eq!(view.snapshot().unwrap().rows[0].width, width);
        assert!(
            scrolled_work.segmented_text_bytes < 64 * 1024,
            "horizontal scrolling must not rescan every glyph of the giant word"
        );
        document.insert(100_000, "W").unwrap();
        let edited_work = materialize(&document, &mut engine, &mut view, 3, 100_000);
        assert!(view.snapshot().unwrap().rows[0].width > width);
        assert!(
            edited_work.segmented_text_bytes >= 2 * 1024 * 1024,
            "an edited tree cannot reuse an old width summary"
        );
        engine.clear_caches();
        assert!(engine.unwrapped_summary_cache.entries.is_empty());
        assert!(view
            .snapshot()
            .unwrap()
            .logical_endpoint_geometry(100_000, BoundaryAffinity::Downstream)
            .is_ok());
    }

    #[test]
    fn wrapped_code_sparse_overflow_retains_the_original_line_margin() {
        use crate::document::{Encoding, Format};
        let text = format!(
            " \tfirst {} {} end\nfollowing",
            "a".repeat(80_000),
            "b".repeat(80_000)
        );
        let document =
            Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(240.0, 240.0);
        for (job, indent) in [(1, 4), (2, 8)] {
            let options = WhitespacePresentationOptions {
                code_wrapped_line_indent: indent,
                ..Default::default()
            };
            view.set_whitespace_presentation(options, Format::Code, 2)
                .unwrap();
            let mut complete = view.clone();
            let mut reference_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            reference_engine.relayout(&document, &mut complete).unwrap();
            materialize(&document, &mut engine, &mut view, job, 0);
            let actual = view.snapshot().unwrap();
            let expected = complete.snapshot().unwrap();
            assert_eq!(actual.rows.len(), expected.rows.len());
            assert!(actual.rows.len() >= 3);
            assert!(actual.rows[1].paragraph_content_x > actual.rows[0].paragraph_content_x);
            for (actual, expected) in actual.rows.iter().zip(expected.rows.iter()) {
                assert_eq!(actual.text_range, expected.text_range);
                assert_eq!(actual.paragraph_content_x, expected.paragraph_content_x);
                assert_eq!(
                    actual.paragraph_content_width,
                    expected.paragraph_content_width
                );
                for cluster in &actual.clusters {
                    let index = expected
                        .clusters
                        .binary_search_by_key(&cluster.text_range.start, |candidate| {
                            candidate.text_range.start
                        })
                        .unwrap();
                    assert_eq!(cluster.x, expected.clusters[index].x);
                }
            }
            assert!(
                actual
                    .rows
                    .iter()
                    .map(|row| row.clusters.len())
                    .sum::<usize>()
                    < 10_000
            );
        }
    }

    #[test]
    fn sparse_worker_rejects_scroll_outside_its_horizontal_coverage_atomically() {
        let document = Document::new("word ".repeat(20_000));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(320.0, 100.0);
        view.set_wrap(false);
        materialize(&document, &mut engine, &mut view, 1, 0);
        let requirements = inspect_layout_provider(&engine);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::Background,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 100.0).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        view.set_viewport_left(100_000.0).unwrap();
        let before = view.snapshot().unwrap().revision;
        assert_eq!(
            install_layout_job(
                &mut view,
                LayoutInstallTarget {
                    document_id: document.id(),
                    document_revision: document.revision(),
                    measurement_environment_id: requirements.measurement_environment_id,
                    metrics_generation: requirements.metrics_generation
                },
                candidate
            ),
            Err(crate::layout::LayoutJobInstallRejection::OutsideHorizontalCoverage)
        );
        assert_eq!(view.snapshot().unwrap().revision, before);
        assert_eq!(view.viewport_left(), 100_000.0);
    }

    #[test]
    fn giant_source_lists_quotes_and_styled_paragraphs_match_complete_layout() {
        use crate::document::{Encoding, Format};
        for (format, source) in [
            (
                Format::MarkdownSource,
                format!("12. {}", "AV fi word ".repeat(7_000)),
            ),
            (
                Format::MarkdownSource,
                format!("> {}", "AV fi word ".repeat(7_000)),
            ),

        ] {
            let document =
                Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            let mut full = ViewLayout::new(320.0, 100.0);
            full.set_wrap(false);
            engine.relayout(&document, &mut full).unwrap();
            let expected = &full.snapshot().unwrap().rows[0];
            let mut sparse = ViewLayout::new(320.0, 100.0);
            sparse.set_wrap(false);
            materialize(&document, &mut engine, &mut sparse, 1, 0);
            let actual = &sparse.snapshot().unwrap().rows[0];
            assert_eq!(actual.width, expected.width, "{format:?}");
            assert_eq!(actual.baseline, expected.baseline, "{format:?}");
            assert_eq!(
                actual.paragraph_content_x, expected.paragraph_content_x,
                "{format:?}"
            );
            assert_eq!(
                actual.paragraph_content_width, expected.paragraph_content_width,
                "{format:?}"
            );
            assert_eq!(actual.decorations, expected.decorations, "{format:?}");
            for cluster in &actual.clusters {
                let reference = expected
                    .clusters
                    .iter()
                    .find(|entry| entry.text_range == cluster.text_range)
                    .unwrap();
                assert_eq!(cluster.x, reference.x, "{format:?}");
                assert_eq!(cluster.advance, reference.advance, "{format:?}");
            }
        }
    }
}
