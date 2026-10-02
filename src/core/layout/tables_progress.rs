//! Bounded intrinsic discovery and glyph materialization for very long cells.
use super::bidi::BidiBandWidths;
use super::*;
const PROGRESS_FRAGMENT: usize = 4096;
const PROGRESS_CACHE_CELLS: usize = 16;
const CHECKPOINT_BLOCK: usize = 64;
const CHECKPOINT_BLOCKS: usize = 256;
#[derive(Clone, Debug)]
struct CellCheckpoint {
    at: usize,
    x: f32,
    y: f32,
}
#[derive(Clone, Debug)]
pub(super) struct CellProgress {
    key: WidthKey,
    range: Range<usize>,
    next: usize,
    width: f32,
    height: f32,
    line_width: f32,
    completed_height: f32,
    line_height: f32,
    checkpoints: VecDeque<Arc<Vec<CellCheckpoint>>>,
    retained_checkpoints: VecDeque<Arc<Vec<CellCheckpoint>>>,
    stride: usize,
    fragments: usize,
    last_layout: Option<LayoutRevision>,
    direction: WritingDirection,
    isolates: usize,
    band: Range<usize>,
    band_origin: CellCheckpoint,
    band_signature: (Option<usize>, u32, u32),
    band_first_line: usize,
    band_last_line: usize,
    bidi: BidiBandWidths,
    prefix_bidi: BidiBandWidths,
    suffix_bidi: BidiBandWidths,
}
impl CellProgress {
    fn checkpoint(&self, at: usize) -> CellCheckpoint {
        [&self.checkpoints, &self.retained_checkpoints]
            .into_iter()
            .filter_map(|checkpoints| {
                let block = checkpoints
                    .partition_point(|block| block.first().is_some_and(|point| point.at <= at))
                    .saturating_sub(1);
                checkpoints
                    .get(block)?
                    .iter()
                    .rev()
                    .find(|point| point.at <= at)
                    .cloned()
            })
            .max_by_key(|point| point.at)
            .unwrap_or(CellCheckpoint {
                at: self.range.start,
                x: 0.,
                y: 0.,
            })
    }
    fn checkpoint_x(&self, x: f32) -> CellCheckpoint {
        [&self.checkpoints, &self.retained_checkpoints]
            .into_iter()
            .filter_map(|checkpoints| {
                let block = checkpoints
                    .partition_point(|block| block.first().is_some_and(|point| point.x <= x))
                    .saturating_sub(1);
                checkpoints
                    .get(block)?
                    .iter()
                    .rev()
                    .find(|point| point.x <= x)
                    .cloned()
            })
            .max_by(|left, right| left.x.total_cmp(&right.x))
            .unwrap_or(CellCheckpoint {
                at: self.range.start,
                x: 0.,
                y: 0.,
            })
    }
    fn checkpoint_y(&self, y: f32) -> CellCheckpoint {
        [&self.checkpoints, &self.retained_checkpoints]
            .into_iter()
            .filter_map(|checkpoints| {
                let block = checkpoints
                    .partition_point(|block| block.first().is_some_and(|point| point.y <= y))
                    .saturating_sub(1);
                checkpoints
                    .get(block)?
                    .iter()
                    .rev()
                    .find(|point| point.y <= y)
                    .cloned()
            })
            .max_by(|left, right| left.y.total_cmp(&right.y).then(left.at.cmp(&right.at)))
            .unwrap_or(CellCheckpoint {
                at: self.range.start,
                x: 0.,
                y: 0.,
            })
    }
    fn remember(&mut self) {
        if self.fragments % self.stride != 0 {
            return;
        }
        if self
            .checkpoints
            .back()
            .is_none_or(|block| block.len() == CHECKPOINT_BLOCK)
        {
            self.checkpoints.push_back(Arc::new(Vec::new()));
        }
        Arc::make_mut(self.checkpoints.back_mut().unwrap()).push(CellCheckpoint {
            at: self.next,
            x: self.line_width,
            y: self.completed_height,
        });
        if self.checkpoints.len() > CHECKPOINT_BLOCKS {
            let points = self
                .checkpoints
                .iter()
                .flat_map(|block| block.iter())
                .step_by(2)
                .cloned()
                .collect::<Vec<_>>();
            self.checkpoints = points
                .chunks(CHECKPOINT_BLOCK)
                .map(|chunk| Arc::new(chunk.to_vec()))
                .collect();
            self.stride *= 2;
        }
    }
}
fn boundary(
    tree: &crate::document::FormattedTextTree,
    at: usize,
    limit: usize,
) -> Result<usize, LayoutError> {
    if at >= limit {
        return Ok(limit);
    }
    let mut at = at;
    while !tree
        .is_char_boundary(at)
        .map_err(|_| LayoutError::InvalidTextOffset(at))?
    {
        at += 1;
    }
    if tree
        .is_grapheme_boundary(at)
        .map_err(|_| LayoutError::InvalidTextOffset(at))?
    {
        Ok(at.min(limit))
    } else {
        tree.next_grapheme_boundary(at)
            .map_err(|_| LayoutError::InvalidTextOffset(at))?
            .map(|at| at.min(limit))
            .ok_or(LayoutError::InvalidTextOffset(at))
    }
}
impl<P: TextMeasurementProvider> LayoutEngine<P> {
    pub(super) fn table_cell_complete(&self, range: &Range<usize>, revision: Revision) -> bool {
        range.len() <= PROGRESS_FRAGMENT
            || self
                .table_cache
                .progress
                .iter()
                .find(|cell| cell.range == *range && cell.key.revision == revision)
                .is_some_and(|cell| cell.next == range.end)
    }
    pub(super) fn table_cell_width(&self, range: &Range<usize>, revision: Revision) -> Option<f32> {
        self.table_cache
            .progress
            .iter()
            .find(|cell| cell.range == *range && cell.key.revision == revision)
            .map(|cell| cell.width)
    }
    pub(super) fn table_cell_height(
        &self,
        range: &Range<usize>,
        revision: Revision,
    ) -> Option<f32> {
        self.table_cache
            .progress
            .iter()
            .find(|cell| cell.range == *range && cell.key.revision == revision)
            .map(|cell| cell.height)
    }
    /// Render only owned glyphs; shaping context on both sides protects
    /// ligatures and joining scripts at the discovery fragment boundary.
    fn progress_fragment(
        &mut self,
        context: &TableLayoutContext,
        range: Range<usize>,
        whole: &Range<usize>,
        document: DocumentId,
        revision: Revision,
        view: &LayoutJobViewConfiguration,
        control: &LayoutRunControl<'_>,
        direction: WritingDirection,
    ) -> Result<Vec<VisualRow>, LayoutComputationError> {
        let tree = context.projection.text_tree();
        // Expand at grapheme boundaries. A fixed byte subtraction can omit the
        // beginning of a joining/combining cluster from shaping context.
        let mut start = range.start;
        while start > whole.start && range.start - start < SHAPING_CONTEXT_BYTES {
            start = tree
                .previous_grapheme_boundary(start)
                .map_err(|_| LayoutError::InvalidTextOffset(start))?
                .unwrap_or(whole.start)
                .max(whole.start);
        }
        let mut end = range.end;
        while end < whole.end && end - range.end < SHAPING_CONTEXT_BYTES {
            end = tree
                .next_grapheme_boundary(end)
                .map_err(|_| LayoutError::InvalidTextOffset(end))?
                .unwrap_or(whole.end)
                .min(whole.end);
        }
        let rows = self.shape_table_cell_piece(
            context,
            start..end,
            document,
            revision,
            view,
            control,
            Some(direction),
        )?;
        let mut owned = Vec::new();
        for mut row in rows {
            let line = row.text_range.clone();
            let owns_break = line.end >= range.start
                && line.end < range.end
                && tree.byte_chunk_at(line.end).first() == Some(&b'\n');
            let owns_empty = line.is_empty()
                && range.start <= line.start
                && (line.start < range.end || range.end == whole.end && line.start == range.end);
            if !(line.start < range.end && range.start < line.end || owns_break || owns_empty) {
                continue;
            }
            row.clusters.retain(|cluster| {
                range.start <= cluster.text_range.start && cluster.text_range.start < range.end
            });
            row.carets.retain(|caret| {
                range.start <= caret.point.text_offset && caret.point.text_offset <= range.end
            });
            let x = row
                .clusters
                .iter()
                .map(|cluster| cluster.x)
                .reduce(f32::min)
                .or_else(|| row.carets.first().map(|caret| caret.x))
                .unwrap_or(0.);
            translate_x(&mut row, -x);
            row.width = row.clusters.iter().map(|cluster| cluster.advance).sum();
            row.paragraph_content_width = row.width;
            // Keep line boundaries so discovery can reset its horizontal
            // accumulator at a real in-cell hard break, including empty lines.
            row.text_range = line.start.max(range.start)..line.end.min(range.end);
            owned.push(row);
        }
        Ok(owned)
    }
    pub(super) fn shape_table_cell(
        &mut self,
        context: &TableLayoutContext,
        range: Range<usize>,
        document: DocumentId,
        revision: Revision,
        view: &LayoutJobViewConfiguration,
        control: &LayoutRunControl<'_>,
    ) -> Result<Vec<VisualRow>, LayoutComputationError> {
        if range.len() <= PROGRESS_FRAGMENT {
            return self
                .shape_table_cell_piece(context, range, document, revision, view, control, None);
        }
        let key = WidthKey {
            document,
            revision,
            table: 0,
            source: context.source,
            metrics: self.provider.metrics_generation(),
            environment: self.provider.measurement_environment_id(),
            scale: view.scale,
            style_revision: context.projection.style_sheet().revision,
            override_style: view
                .default_style_is_override
                .then(|| view.default_style.clone()),
        };
        let mut progress = self
            .table_cache
            .progress
            .iter()
            .position(|entry| entry.key == key && entry.range == range)
            .and_then(|index| self.table_cache.progress.remove(index))
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| CellProgress {
                key,
                range: range.clone(),
                next: range.start,
                width: 0.,
                height: 0.,
                line_width: 0.,
                completed_height: 0.,
                line_height: 0.,
                checkpoints: VecDeque::new(),
                retained_checkpoints: VecDeque::new(),
                stride: 1,
                fragments: 0,
                last_layout: None,
                direction: WritingDirection::Natural,
                isolates: 0,
                band: range.start..range.start,
                band_origin: CellCheckpoint {
                    at: range.start,
                    x: 0.,
                    y: 0.,
                },
                band_signature: (None, u32::MAX, u32::MAX),
                band_first_line: 0,
                band_last_line: 0,
                bidi: Default::default(),
                prefix_bidi: Default::default(),
                suffix_bidi: Default::default(),
            });
        if progress.direction == WritingDirection::Natural {
            let first_end = boundary(
                context.projection.text_tree(),
                range.start.saturating_add(PROGRESS_FRAGMENT).min(range.end),
                range.end,
            )?;
            let first = context
                .projection
                .text_tree()
                .slice(range.start..first_end)
                .map_err(|_| LayoutError::InvalidTextOffset(range.start))?;
            if let Some(direction) = detect_direction(&first, &mut 0) {
                progress.direction = direction;
            }
        }
        let focus = view.horizontal_focus_offset.and_then(|at| {
            if range.start <= at && at <= range.end {
                return Some(at);
            }
            let table = context.projection.table_at(range.start)?;
            if at < table.range.start || at > table.range.end {
                return None;
            }
            let column = |offset: usize| -> Option<usize> {
                if context.source {
                    let row = table
                        .source_rows
                        .partition_point(|row| row.range.end < offset);
                    let cells = &table.source_rows.get(row)?.cells;
                    Some(
                        cells
                            .partition_point(|cell| cell.end < offset)
                            .min(table.columns.len() - 1),
                    )
                } else {
                    let row = table.rows.partition_point(|row| row.range.end < offset);
                    let cells = &table.rows.get(row)?.cells;
                    Some(
                        cells
                            .partition_point(|cell| cell.range.end < offset)
                            .min(table.columns.len() - 1),
                    )
                }
            };
            let current = column(range.start)?;
            let active = column(at)?;
            // A caret in another table cell can move with the shared column
            // width. Keep intrinsic discovery on one logical band while that
            // viewport moves, instead of restarting it at every width update.
            Some(if active > current {
                range.end
            } else if active < current {
                range.start
            } else {
                match table.columns[current] {
                    TableAlignment::Right => range.end,
                    TableAlignment::Center => range.start + range.len() / 2,
                    _ => range.start,
                }
            })
        });
        let multiline = !context.source
            && context.projection.hard_line_at_offset(range.start)
                != context.projection.hard_line_at_offset(range.end);
        let signature = (
            focus,
            if focus.is_some() {
                0
            } else {
                view.viewport_left.to_bits()
            },
            if multiline && focus.is_none() {
                view.regional_viewport_top.to_bits()
            } else {
                0
            },
        );
        if progress.band_signature != signature {
            let estimate = if progress.next > range.start {
                progress.width / (progress.next - range.start) as f32
            } else {
                view.default_style.size * view.scale * 0.6
            };
            let start = if let Some(focus) = focus {
                let checkpoint = progress.checkpoint(focus);
                if focus - checkpoint.at < PROGRESS_FRAGMENT {
                    checkpoint.at
                } else {
                    focus.saturating_sub(PROGRESS_FRAGMENT / 2).max(range.start)
                }
            } else if multiline {
                progress.checkpoint_y(view.regional_viewport_top).at
            } else if progress.direction == WritingDirection::RightToLeft {
                range
                    .end
                    .saturating_sub(
                        (view.viewport_left / estimate.max(0.01)) as usize + PROGRESS_FRAGMENT,
                    )
                    .max(range.start)
            } else {
                progress.checkpoint_x(view.viewport_left).at
            };
            let start = boundary(context.projection.text_tree(), start, range.end)?;
            let end = boundary(
                context.projection.text_tree(),
                start.saturating_add(PROGRESS_FRAGMENT).min(range.end),
                range.end,
            )?;
            progress.band_origin = progress.checkpoint(start);
            progress.band = start..end;
            progress.band_signature = signature;
            progress.band_first_line = context.projection.hard_line_at_offset(start).unwrap_or(0);
            let last = context
                .projection
                .text_tree()
                .previous_grapheme_boundary(end)
                .map_err(|_| LayoutError::InvalidTextOffset(end))?
                .unwrap_or(start)
                .max(start);
            progress.band_last_line = context.projection.hard_line_at_offset(last).unwrap_or(0);
            // Band changes need new bidi context, but cannot discard already
            // known coordinate checkpoints used by immediate manual scrolling.
            let latest = |points: &VecDeque<Arc<Vec<CellCheckpoint>>>| {
                points
                    .back()
                    .and_then(|block| block.last())
                    .map_or(0, |point| point.at)
            };
            if latest(&progress.checkpoints) > latest(&progress.retained_checkpoints) {
                progress.retained_checkpoints = progress.checkpoints.clone();
            }
            progress.checkpoints.clear();
            progress.fragments = 0;
            progress.stride = 1;
            progress.next = range.start;
            progress.line_width = 0.;
            progress.completed_height = 0.;
            progress.line_height = 0.;
            progress.last_layout = None;
            progress.bidi = Default::default();
            progress.prefix_bidi = Default::default();
            progress.suffix_bidi = Default::default();
        }
        if progress.last_layout != self.table_cache.current_layout {
            progress.last_layout = self.table_cache.current_layout;
            while progress.next < range.end && self.table_cache.remaining_bytes > 0 {
                control.checkpoint()?;
                progress.remember();
                let end = boundary(
                    context.projection.text_tree(),
                    progress
                        .next
                        .saturating_add(PROGRESS_FRAGMENT)
                        .min(range.end),
                    range.end,
                )?;
                if progress.direction == WritingDirection::Natural {
                    let text = context
                        .projection
                        .text_tree()
                        .slice(progress.next..end)
                        .map_err(|_| LayoutError::InvalidTextOffset(progress.next))?;
                    if let Some(direction) = detect_direction(&text, &mut progress.isolates) {
                        progress.direction = direction;
                        if progress.next > range.start {
                            progress.band_signature = (None, u32::MAX, u32::MAX);
                            progress.next = range.start;
                            break;
                        }
                    }
                }
                let rows = self.progress_fragment(
                    context,
                    progress.next..end,
                    &range,
                    document,
                    revision,
                    view,
                    control,
                    progress.direction,
                )?;
                for row in rows {
                    let mut logical = row.clusters.iter().collect::<Vec<_>>();
                    logical.sort_by_key(|cluster| cluster.text_range.start);
                    for cluster in logical {
                        let line = context
                            .projection
                            .hard_line_at_offset(cluster.text_range.start)
                            .unwrap_or(0);
                        if cluster.text_range.start < progress.band.start
                            && line == progress.band_first_line
                        {
                            progress
                                .bidi
                                .push_prefix(cluster.bidi_level, cluster.advance);
                            progress
                                .prefix_bidi
                                .push_prefix(cluster.bidi_level, cluster.advance);
                        } else if cluster.text_range.start >= progress.band.end
                            && line == progress.band_last_line
                        {
                            progress
                                .bidi
                                .push_suffix(cluster.bidi_level, cluster.advance);
                            progress
                                .suffix_bidi
                                .push_suffix(cluster.bidi_level, cluster.advance);
                        }
                    }
                    progress.line_width += row.width;
                    progress.line_height = progress.line_height.max(row.natural_height());
                    progress.width = progress.width.max(progress.line_width);
                    let at = row.text_range.end;
                    if !context.source
                        && at < end
                        && context
                            .projection
                            .text_tree()
                            .slice(at..at + 1)
                            .is_ok_and(|text| text == "\n")
                    {
                        progress.completed_height += progress.line_height;
                        progress.line_width = 0.;
                        progress.line_height = 0.;
                    }
                }
                progress.height = progress.completed_height + progress.line_height;
                self.table_cache.remaining_bytes = self
                    .table_cache
                    .remaining_bytes
                    .saturating_sub(end - progress.next);
                progress.next = end;
                progress.fragments += 1;
            }
        }
        if !context.source && progress.next < range.end {
            let remaining = context
                .projection
                .hard_line_at_offset(range.end)
                .unwrap_or(0)
                .saturating_sub(
                    context
                        .projection
                        .hard_line_at_offset(progress.next)
                        .unwrap_or(0),
                );
            progress.height = progress.completed_height
                + progress.line_height
                + remaining as f32 * view.default_style.size * view.scale * 1.12;
        }
        let mut point = progress.checkpoint(progress.band.start);
        if progress.band_origin.at > point.at {
            point = progress.band_origin.clone();
        }
        // A cold caret demand can begin between discovery checkpoints. Carry
        // the exact explicit-line heights across that bounded prefix before
        // placing its band; horizontal checkpoint x alone cannot locate it.
        if multiline
            && point.at < progress.band.start
            && progress.band.start - point.at <= PROGRESS_FRAGMENT
        {
            let prefix = self.progress_fragment(
                context,
                point.at..progress.band.start,
                &range,
                document,
                revision,
                view,
                control,
                progress.direction,
            )?;
            for row in prefix {
                if row.text_range.end < progress.band.start
                    && context
                        .projection
                        .text_tree()
                        .byte_chunk_at(row.text_range.end)
                        .first()
                        == Some(&b'\n')
                {
                    point.y += row.height();
                    point.x = 0.;
                } else {
                    point.x += row.width;
                }
            }
        }
        if multiline && progress.band.start - point.at > PROGRESS_FRAGMENT {
            let skipped = context
                .projection
                .hard_line_at_offset(progress.band.start)
                .unwrap_or(0)
                .saturating_sub(
                    context
                        .projection
                        .hard_line_at_offset(point.at)
                        .unwrap_or(0),
                );
            point.y += skipped as f32 * view.default_style.size * view.scale * 1.12;
            point.x = 0.;
        }
        let mut rows = self.progress_fragment(
            context,
            progress.band.clone(),
            &range,
            document,
            revision,
            view,
            control,
            progress.direction,
        )?;
        for (index, row) in rows.iter_mut().enumerate() {
            let x = if index == 0 { point.x } else { 0. };
            translate_x(row, x);
            translate_row_vertically(row, point.y).map_err(LayoutError::from)?;
            if progress.next == range.end {
                let line = context
                    .projection
                    .hard_line_at_offset(row.text_range.start)
                    .unwrap_or(0);
                let empty = BidiBandWidths::default();
                let bidi = if progress.band_first_line == progress.band_last_line {
                    &progress.bidi
                } else if line == progress.band_first_line {
                    &progress.prefix_bidi
                } else if line == progress.band_last_line {
                    &progress.suffix_bidi
                } else {
                    &empty
                };
                place_bidi_band(row, bidi);
                if multiline {
                    row.width = bidi.total_width(
                        row.clusters
                            .iter()
                            .map(|cluster| f64::from(cluster.advance))
                            .sum(),
                    );
                }
            }
            if context.source
                || context.projection.hard_line_at_offset(range.start)
                    == context.projection.hard_line_at_offset(range.end)
            {
                row.text_range = range.clone();
                row.hard_line_range = range.clone();
            }
            if !multiline {
                row.width = progress.width.max(x + row.width);
            }
            row.paragraph_content_width = row.width;
        }
        self.table_cache.progress.push_back(Arc::new(progress));
        while self.table_cache.progress.len() > PROGRESS_CACHE_CELLS {
            self.table_cache.progress.pop_front();
        }
        Ok(rows)
    }
}

fn place_bidi_band(row: &mut VisualRow, bidi: &BidiBandWidths) {
    let mut order = (0..row.clusters.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| row.clusters[index].text_range.start);
    let levels = order
        .iter()
        .map(|&index| row.clusters[index].bidi_level)
        .collect::<Vec<_>>();
    let widths = order
        .iter()
        .map(|&index| row.clusters[index].advance)
        .collect::<Vec<_>>();
    let positions = bidi.positions(&levels, &widths);
    let mut changes = Vec::new();
    for (&index, &x) in order.iter().zip(&positions) {
        let cluster = &mut row.clusters[index];
        let old_x = cluster.x;
        let delta = x - old_x;
        changes.push((
            cluster.text_range.clone(),
            old_x,
            old_x + cluster.advance,
            delta,
        ));
        cluster.x = x;
        cluster.typographic_bounds.x += delta;
        cluster.ink_bounds.x += delta;
    }
    for caret in &mut row.carets {
        // A logical boundary belongs to at most the two adjacent clusters.
        // Search by source order instead of rescanning every visible glyph for
        // every caret when a large band's bidi coordinates become exact.
        let after =
            changes.partition_point(|(range, _, _, _)| range.start <= caret.point.text_offset);
        if let Some((_, _, _, delta)) = changes[after.saturating_sub(2)..after]
            .iter()
            .filter(|(range, _, _, _)| {
                range.start <= caret.point.text_offset && caret.point.text_offset <= range.end
            })
            .min_by(|(_, left, right, _), (_, other_left, other_right, _)| {
                ((caret.x - *left).abs().min((caret.x - *right).abs())).total_cmp(
                    &((caret.x - *other_left)
                        .abs()
                        .min((caret.x - *other_right).abs())),
                )
            })
        {
            caret.x += delta;
        }
    }
}

fn detect_direction(text: &str, isolates: &mut usize) -> Option<WritingDirection> {
    use unicode_bidi::BidiClass;
    for character in text.chars() {
        match unicode_bidi::bidi_class(character) {
            BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => *isolates += 1,
            BidiClass::PDI => *isolates = isolates.saturating_sub(1),
            BidiClass::L if *isolates == 0 => return Some(WritingDirection::LeftToRight),
            BidiClass::R | BidiClass::AL if *isolates == 0 => {
                return Some(WritingDirection::RightToLeft)
            }
            _ => {}
        }
    }
    None
}
