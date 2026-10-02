//! Content-sized table geometry. Source alignment changes advances only; rich
//! tables retain one presentation band per row and independent cell caret stops.
#[path = "tables_bidi.rs"]
mod bidi;
#[path = "tables_progress.rs"]
mod progress;
use super::*;
use crate::document::{FormattedDocument, MarkdownTable, TableAlignment};
use crate::layout::persistent_map::PersistentMap;
use progress::CellProgress;

const DISCOVERY_CELLS: usize = 128;
const CACHE_TABLES: usize = 16;
const CACHE_CELL_CONTRIBUTIONS: usize = 65_536;
const CACHE_MEASURED_INTERVALS: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub struct TableCellGeometry {
    pub table_id: u64,
    pub row: usize,
    pub column: usize,
    pub cell_id: u64,
    pub text_range: Range<usize>,
    pub rect: LayoutRect,
    pub(crate) columns: usize,
    pub(crate) table_x: f32,
    pub(crate) table_width: f32,
    pub(crate) rows: usize,
    pub(crate) exact: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TableGeometry {
    pub table_id: u64,
    pub rect: LayoutRect,
    pub column_widths: Vec<(usize, f32)>,
    pub row_heights: Vec<(usize, f32)>,
    /// False during bounded cold discovery or when only part is materialized.
    pub exact: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TableLayoutContext {
    projection: Arc<FormattedDocument>,
    source: bool,
}
impl PartialEq for TableLayoutContext {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.projection.revision() == other.projection.revision()
            && Arc::ptr_eq(&self.projection, &other.projection)
    }
}
impl TableLayoutContext {
    pub(crate) fn new(document: &FormattedDocument, source: bool) -> Self {
        Self {
            projection: Arc::new(document.clone()),
            source,
        }
    }
    fn row(&self, range: &Range<usize>) -> Option<(&MarkdownTable, usize)> {
        let table = self.projection.table_at(range.start)?;
        if self.source {
            let index = table
                .source_rows
                .partition_point(|row| row.range.start < range.start);
            table
                .source_rows
                .get(index)
                .filter(|row| row.range == *range)
                .map(|_| (table, index))
        } else {
            let index = table
                .rows
                .partition_point(|row| row.range.start < range.start);
            table
                .rows
                .get(index)
                .filter(|row| row.range == *range)
                .map(|_| (table, index))
        }
    }
    pub(super) fn contains_line(&self, range: &Range<usize>) -> bool {
        self.row(range).is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WidthKey {
    document: DocumentId,
    revision: Revision,
    table: u64,
    source: bool,
    metrics: MetricsGeneration,
    environment: MeasurementEnvironmentId,
    scale: f32,
    style_revision: StyleSheetRevision,
    override_style: Option<ResolvedTextStyle>,
}
#[derive(Clone, Debug)]
struct Widths {
    key: WidthKey,
    widths: Vec<f32>,
    measured: MeasuredCells,
    next: usize,
    total: usize,
    projection: Arc<FormattedDocument>,
    source_range_start: usize,
    contributions: PersistentMap<(usize, usize), u32>,
    maxima: Vec<PersistentMap<u32, usize>>,
    dirty: VecDeque<(usize, usize)>,
    discovery_layout: Option<LayoutRevision>,
    aggregate_only: bool,
    row_heights: VecDeque<Arc<RowHeights>>,
}

/// Width refinement may discard shaped offscreen glyphs, but their measured
/// heights still determine the active row's vertical geometry. Keep a bounded
/// cache independent of the all-table width contribution budget.
#[derive(Clone, Debug, Default)]
struct RowHeights {
    row: usize,
    contributions: PersistentMap<usize, u32>,
    maxima: PersistentMap<u32, usize>,
    measured: MeasuredCells,
    maximum: f32,
    aggregate_only: bool,
    next: usize,
}
impl RowHeights {
    fn height(&self) -> f32 {
        if self.aggregate_only {
            self.maximum
        } else {
            self.maxima
                .last_key_value()
                .map_or(0., |(bits, _)| f32::from_bits(*bits))
        }
    }
    fn remove(&mut self, column: usize) {
        if self.aggregate_only {
            let row = self.row;
            *self = Self {
                row,
                ..Default::default()
            };
            return;
        }
        if let Some(bits) = self.contributions.remove(&column) {
            let count = *self
                .maxima
                .get(&bits)
                .expect("height contribution is indexed");
            if count == 1 {
                self.maxima.remove(&bits);
            } else {
                self.maxima.insert(bits, count - 1);
            }
        }
        self.measured.remove(column);
        self.next = self.next.min(column);
    }
    fn record(&mut self, column: usize, height: f32, complete: bool) {
        if !self.aggregate_only
            && self.contributions.len() >= 256
            && !self.contributions.contains_key(&column)
        {
            self.maximum = self.height();
            self.aggregate_only = true;
            self.contributions.clear();
            self.maxima.clear();
        }
        if self.aggregate_only {
            self.maximum = self.maximum.max(height);
        } else {
            self.remove(column);
            let bits = height.max(0.).to_bits();
            self.contributions.insert(column, bits);
            let count = self.maxima.get(&bits).copied().unwrap_or(0);
            self.maxima.insert(bits, count + 1);
        }
        if complete {
            self.measured.insert(column);
        } else {
            self.measured.remove(column);
        }
    }
}
/// Consecutive discovery costs one interval, even for millions of cells.
#[derive(Clone, Debug, Default)]
struct MeasuredCells {
    intervals: BTreeMap<usize, usize>,
    count: usize,
}
impl MeasuredCells {
    fn contains(&self, at: usize) -> bool {
        self.intervals
            .range(..=at)
            .next_back()
            .is_some_and(|(_, end)| at < *end)
    }
    fn insert(&mut self, at: usize) {
        if self.contains(at) {
            return;
        }
        let mut start = at;
        let mut end = at + 1;
        if let Some((&before, &before_end)) = self.intervals.range(..at).next_back() {
            if before_end == at {
                start = before;
                self.intervals.remove(&before);
            }
        }
        if let Some(after_end) = self.intervals.remove(&(at + 1)) {
            end = after_end;
        }
        self.intervals.insert(start, end);
        self.count += 1;
        // Eviction changes completeness only; measured maxima remain a safe
        // provisional upper bound until discovery revisits the evicted cells.
        while self.intervals.len() > CACHE_MEASURED_INTERVALS {
            if let Some((start, end)) = self.intervals.pop_last() {
                self.count -= end - start;
            }
        }
    }
    fn remove(&mut self, at: usize) {
        if let Some((&start, &end)) = self.intervals.range(..=at).next_back() {
            if at < end {
                self.intervals.remove(&start);
                if start < at {
                    self.intervals.insert(start, at);
                }
                if at + 1 < end {
                    self.intervals.insert(at + 1, end);
                }
                self.count -= 1;
            }
        }
    }
}
impl Widths {
    fn record_height(&mut self, row: usize, column: usize, height: f32, complete: bool) {
        let index = self.row_heights.iter().position(|entry| entry.row == row);
        let mut entry = index
            .and_then(|index| self.row_heights.remove(index))
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| RowHeights {
                row,
                ..Default::default()
            });
        entry.record(column, height, complete);
        self.row_heights.push_back(Arc::new(entry));
        while self.row_heights.len() > 256 {
            self.row_heights.pop_front();
        }
    }
    fn seen(&self, row: usize, column: usize) -> bool {
        self.measured.contains(row * self.widths.len() + column)
    }
    fn record(&mut self, row: usize, column: usize, width: f32) {
        if !self.aggregate_only
            && self.contributions.len() >= CACHE_CELL_CONTRIBUTIONS
            && !self.contributions.contains_key(&(row, column))
        {
            self.refresh_widths(0.);
            self.aggregate_only = true;
            self.contributions.clear();
            for maxima in &mut self.maxima {
                maxima.clear();
            }
        }
        if self.aggregate_only {
            self.widths[column] = self.widths[column].max(width);
        } else {
            self.remove(row, column);
            let bits = width.max(0.).to_bits();
            self.contributions.insert((row, column), bits);
            let count = self.maxima[column].get(&bits).copied().unwrap_or(0);
            self.maxima[column].insert(bits, count + 1);
        }
        self.measured.insert(row * self.widths.len() + column);
    }
    fn remove(&mut self, row: usize, column: usize) {
        if let Some(bits) = self.contributions.remove(&(row, column)) {
            let count = *self.maxima[column]
                .get(&bits)
                .expect("width contribution is indexed");
            if count == 1 {
                self.maxima[column].remove(&bits);
            } else {
                self.maxima[column].insert(bits, count - 1);
            }
        }
        self.measured.remove(row * self.widths.len() + column);
        if let Some(entry) = self.row_heights.iter_mut().find(|entry| entry.row == row) {
            Arc::make_mut(entry).remove(column);
        }
    }
    fn refresh_widths(&mut self, minimum: f32) {
        if self.aggregate_only {
            for width in &mut self.widths {
                *width = width.max(minimum);
            }
            return;
        }
        for (width, maxima) in self.widths.iter_mut().zip(&self.maxima) {
            *width = maxima
                .last_key_value()
                .map_or(minimum, |(bits, _)| f32::from_bits(*bits).max(minimum));
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct TableMeasurementCache {
    entries: VecDeque<Arc<Widths>>,
    progress: VecDeque<Arc<CellProgress>>,
    current_layout: Option<LayoutRevision>,
    remaining_bytes: usize,
    pub(super) measured_cells: usize,
    pub(super) measured_text_bytes: usize,
}
impl TableMeasurementCache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.progress.clear();
    }
}

pub(super) fn collect_cells(rows: &[VisualRow]) -> Vec<TableCellGeometry> {
    let mut seen = BTreeSet::new();
    let mut cells = Vec::new();
    for cell in rows.iter().filter_map(|row| row.table_cell.as_ref()) {
        if seen.insert((cell.table_id, cell.row, cell.column)) {
            cells.push(cell.clone());
        }
    }
    cells.sort_by_key(|cell| (cell.table_id, cell.row, cell.column));
    cells
}
pub(super) fn collect_tables(rows: &[VisualRow]) -> Vec<TableGeometry> {
    let cells = collect_cells(rows);
    let mut tables: Vec<TableGeometry> = Vec::new();
    for cell in cells {
        let index = match tables
            .iter()
            .position(|table| table.table_id == cell.table_id)
        {
            Some(index) => index,
            None => {
                tables.push(TableGeometry {
                    table_id: cell.table_id,
                    rect: cell.rect,
                    column_widths: Vec::new(),
                    row_heights: Vec::new(),
                    exact: cell.exact,
                });
                tables.len() - 1
            }
        };
        let table = &mut tables[index];
        let right = (table.rect.x + table.rect.width).max(cell.rect.x + cell.rect.width);
        let bottom = (table.rect.y + table.rect.height).max(cell.rect.y + cell.rect.height);
        table.rect.x = table.rect.x.min(cell.rect.x);
        table.rect.y = table.rect.y.min(cell.rect.y);
        table.rect.width = right - table.rect.x;
        table.rect.height = bottom - table.rect.y;
        table.rect.x = cell.table_x;
        table.rect.width = cell.table_width;
        if !table
            .column_widths
            .iter()
            .any(|(index, _)| *index == cell.column)
        {
            table.column_widths.push((cell.column, cell.rect.width));
        }
        if table
            .row_heights
            .last()
            .is_none_or(|(index, _)| *index != cell.row)
        {
            table.row_heights.push((cell.row, cell.rect.height));
        }
        table.exact &= cell.exact;
    }

    tables
}

fn translate_x(row: &mut VisualRow, delta: f32) {
    row.paragraph_content_x += delta;
    for caret in &mut row.carets {
        caret.x += delta;
    }
    for cluster in &mut row.clusters {
        cluster.x += delta;
        cluster.typographic_bounds.x += delta;
        cluster.ink_bounds.x += delta;
    }
    for decoration in &mut row.decorations {
        decoration.x += delta;
        decoration.typographic_bounds.x += delta;
        decoration.ink_bounds.x += delta;
    }
}
fn span_width(rows: &[VisualRow]) -> f32 {
    rows.iter()
        .map(|row| {
            row.ink_bounds().map_or(row.width, |ink| {
                row.width
                    .max(ink.x + ink.width)
                    .max(row.width - ink.x.min(0.))
            })
        })
        .fold(0., f32::max)
}
fn complete_boundary_affinities(rows: &mut [VisualRow], range: &Range<usize>) {
    for (offset, affinity) in [
        (range.start, BoundaryAffinity::Upstream),
        (range.end, BoundaryAffinity::Downstream),
    ] {
        if rows
            .iter()
            .flat_map(|row| &row.carets)
            .any(|caret| caret.point.text_offset == offset && caret.point.affinity == affinity)
        {
            continue;
        }
        if let Some(row) = rows.iter_mut().find(|row| {
            row.carets
                .iter()
                .any(|caret| caret.point.text_offset == offset)
        }) {
            let mut caret = row
                .carets
                .iter()
                .find(|caret| caret.point.text_offset == offset)
                .unwrap()
                .clone();
            caret.point.affinity = affinity;
            row.carets.push(caret);
            row.carets.sort_by(|left, right| left.x.total_cmp(&right.x));
        }
    }
}

fn alignment_extra(alignment: TableAlignment, available: f32) -> f32 {
    match alignment {
        TableAlignment::Center => available / 2.,
        TableAlignment::Right => available,
        _ => 0.,
    }
}

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    fn shape_table_cell_piece(
        &mut self,
        context: &TableLayoutContext,
        range: Range<usize>,
        document_id: DocumentId,
        revision: Revision,
        view: &LayoutJobViewConfiguration,
        control: &LayoutRunControl<'_>,
        direction: Option<WritingDirection>,
    ) -> Result<Vec<VisualRow>, LayoutComputationError> {
        control.checkpoint()?;
        self.table_cache.measured_cells = self.table_cache.measured_cells.saturating_add(1);
        self.table_cache.measured_text_bytes = self
            .table_cache
            .measured_text_bytes
            .saturating_add(range.len());
        let text = context
            .projection
            .text_tree()
            .slice(range.clone())
            .map_err(|_| LayoutError::InvalidTextOffset(range.start))?;
        let mut styles = DocumentLayoutStyles::resolve_region_for_presentation(
            &context.projection,
            range.clone(),
            false,
            &[],
            if context.source {
                crate::document::Format::MarkdownSource
            } else {
                crate::document::Format::Markdown
            },
        )
        .map_err(LayoutError::from)?;
        styles.table_context = None;
        styles.document_insets = EdgeInsets::default();
        for paragraph in &mut styles.paragraphs {
            paragraph.block_box = super::super::BlockBoxStyle::default();
            paragraph.containers = Arc::from([]);
            paragraph.margin_top = 0.;
            paragraph.margin_bottom = 0.;
            paragraph.leading_indent = 0.;
            paragraph.trailing_indent = 0.;
            paragraph.first_line_indent = 0.;
            paragraph.alignment = ParagraphAlignment::Start;
            if paragraph.base_direction == WritingDirection::Natural {
                if let Some(direction) = direction {
                    paragraph.base_direction = direction;
                }
            }
            paragraph.list_marker_decoration = None;
            paragraph.list_marker_range = None;
            paragraph.quote_border = false;
            paragraph.thematic_break = false;
        }
        let lines = if context.source {
            vec![range.clone()]
        } else {
            hard_line_ranges(&text, range.start)
        };
        let mut cell_view = view.clone();
        cell_view.wrap = false;
        cell_view.insets = EdgeInsets::default();
        cell_view.width = 1.;
        cell_view.cached_lines.clear();
        let snapshot = self.layout_hard_line_region_cancellable(
            document_id,
            revision,
            &text,
            range.start,
            &lines,
            0,
            lines.len(),
            context.projection.text_tree().byte_len(),
            None,
            &styles,
            &cell_view,
            control.cancellation,
        )?;
        let mut rows = Vec::new();
        let mut y = 0.;
        for line in snapshot.lines {
            for mut row in line.rows {
                translate_row_vertically(&mut row, y).map_err(LayoutError::from)?;
                rows.push(row);
            }
            y += line.height as f32;
        }
        Ok(rows)
    }

    pub(super) fn table_line(
        &mut self,
        context: &TableLayoutContext,
        range: &Range<usize>,
        document_id: DocumentId,
        revision: Revision,
        view: &LayoutJobViewConfiguration,
        line_index: usize,
        layout_revision: LayoutRevision,
        x: f32,
        y: f32,
        complete_columns: bool,
        control: &LayoutRunControl<'_>,
    ) -> Result<Option<(Vec<VisualRow>, f32)>, LayoutComputationError> {
        let Some((table, row_index)) = context.row(range) else {
            return Ok(None);
        };
        if self.table_cache.current_layout != Some(layout_revision) {
            self.table_cache.current_layout = Some(layout_revision);
            self.table_cache.remaining_bytes = 64 * 1024;
        }
        let column_count = table.columns.len();
        if column_count == 0 {
            return Ok(None);
        }
        let resolve_style = |name: &str| {
            context
                .projection
                .style_sheet()
                .resolve_assigned_paragraph_style(
                    context.projection.document_style(),
                    &name.into(),
                    &Default::default(),
                    &Default::default(),
                    None,
                    &Default::default(),
                )
                .or_else(|_| {
                    crate::document::StyleSheet::for_format(crate::document::Format::Markdown)
                        .resolve_paragraph_style(
                            &"Paragraph".into(),
                            &name.into(),
                            None,
                            &Default::default(),
                            &Default::default(),
                        )
                })
                .map_err(DocumentStyleError::from)
                .map_err(LayoutError::from)
        };
        let cell_style = resolve_style("Table cell")?;
        let header_style = resolve_style("Table header")?;
        let table_style = resolve_style("Table")?;
        let default_style = if view.default_style_is_override {
            view.default_style.clone()
        } else {
            super::super::shaping_style(&cell_style.character).map_err(LayoutError::from)?
        };
        let em = default_style.size * view.scale;
        let empty = self.shape_ranges(
            document_id,
            revision,
            "",
            &[0..0],
            &[0..0],
            &[],
            &[default_style],
            &[TextDirection::LeftToRight],
            view.scale,
            self.provider.measurement_environment_id(),
            self.provider.metrics_generation(),
            control,
        )?;
        let minimum_height = empty
            .first()
            .map_or(em, |fragment| fragment.default_metrics.height());
        let row_style = if row_index == 0 {
            &header_style
        } else {
            &cell_style
        };
        let padding_left = cell_style.padding_left.max(header_style.padding_left) * view.scale;
        let padding_right = cell_style.padding_right.max(header_style.padding_right) * view.scale;
        let padding_top = row_style.padding_top * view.scale;
        let padding_bottom = row_style.padding_bottom * view.scale;
        let border = cell_style
            .border_left_width
            .max(cell_style.border_right_width)
            .max(header_style.border_left_width)
            .max(header_style.border_right_width)
            * view.scale;
        let min_width = if context.source { 0. } else { 4. * em };
        let key = WidthKey {
            document: document_id,
            revision,
            table: table.id,
            source: context.source,
            metrics: self.provider.metrics_generation(),
            environment: self.provider.measurement_environment_id(),
            scale: view.scale,
            style_revision: context.projection.style_sheet().revision,
            override_style: view
                .default_style_is_override
                .then(|| view.default_style.clone()),
        };
        let row_count = if context.source {
            table.source_rows.len()
        } else {
            table.rows.len()
        };
        let compatible = |entry: &Widths| {
            let mut previous = entry.key.clone();
            previous.revision = revision;
            previous == key
                && (!entry.aggregate_only || entry.key.revision == revision)
                && entry.total == row_count * column_count
                && entry.widths.len() == column_count
        };
        let mut widths = self
            .table_cache
            .entries
            .iter()
            .position(|entry| compatible(entry))
            .and_then(|index| self.table_cache.entries.remove(index))
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| Widths {
                key: key.clone(),
                widths: vec![min_width; column_count],
                measured: MeasuredCells::default(),
                next: 0,
                total: row_count * column_count,
                projection: Arc::clone(&context.projection),
                source_range_start: table.range.start,
                contributions: PersistentMap::new(),
                maxima: vec![PersistentMap::new(); column_count],
                dirty: VecDeque::new(),
                discovery_layout: None,
                aggregate_only: false,
                row_heights: VecDeque::new(),
            });
        if widths.key.revision != revision {
            if let Some((old, _)) = widths
                .projection
                .text_tree()
                .changed_extent(context.projection.text_tree())
            {
                if let Some(previous) = widths.projection.table_at(widths.source_range_start) {
                    let mut changed = Vec::new();
                    if context.source {
                        let first = previous
                            .source_rows
                            .partition_point(|row| row.range.end < old.start);
                        for (row_index, row) in previous
                            .source_rows
                            .iter()
                            .enumerate()
                            .skip(first)
                            .take_while(|(_, row)| row.range.start <= old.end)
                        {
                            for (column, cell) in row.cells.iter().enumerate().take(column_count) {
                                if cell.start <= old.end && old.start <= cell.end {
                                    changed.push((row_index, column));
                                }
                            }
                        }
                    } else {
                        let first = previous
                            .rows
                            .partition_point(|row| row.range.end < old.start);
                        for (row_index, row) in previous
                            .rows
                            .iter()
                            .enumerate()
                            .skip(first)
                            .take_while(|(_, row)| row.range.start <= old.end)
                        {
                            for (column, cell) in row.cells.iter().enumerate() {
                                if cell.range.start <= old.end && old.start <= cell.range.end {
                                    changed.push((row_index, column));
                                }
                            }
                        }
                    }
                    for (row, column) in changed {
                        widths.remove(row, column);
                        widths.dirty.push_back((row, column));
                    }
                }
            }
            widths.key = key;
            widths.projection = Arc::clone(&context.projection);
            widths.source_range_start = table.range.start;
            widths.discovery_layout = None;
        }
        let height_index = widths
            .row_heights
            .iter()
            .position(|entry| entry.row == row_index);
        let mut row_heights = height_index
            .and_then(|index| widths.row_heights.remove(index))
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| RowHeights {
                row: row_index,
                ..Default::default()
            });
        let ranges = |row: usize, column: usize| -> Option<Range<usize>> {
            if context.source {
                table.source_rows.get(row)?.cells.get(column).cloned()
            } else {
                table
                    .rows
                    .get(row)?
                    .cells
                    .get(column)
                    .map(|cell| cell.range.clone())
            }
        };
        // Exact active-row geometry is always measured first. Additional cold
        // work is limited independently of the number of rows in the table.
        let mut measured_cells = Vec::new();
        let mut column_left = x;
        for column in 0..column_count {
            control.checkpoint()?;
            let estimate = if context.source && !widths.measured.contains(column) {
                4. * em
            } else {
                widths.widths[column]
            };
            let column_right = column_left
                + estimate
                + if context.source {
                    em * 0.45
                } else {
                    padding_left + padding_right + border
                };
            let owns_focus = view.horizontal_focus_offset.is_some_and(|at| {
                ranges(row_index, column).is_some_and(|range| {
                    range.start <= at && at <= range.end
                        || context.source
                            && (column == 0
                                && table.source_rows[row_index].range.start <= at
                                && at < range.start
                                || column + 1 == column_count
                                    && range.end < at
                                    && at <= table.source_rows[row_index].range.end)
                })
            });
            let visible = column_right >= view.viewport_left
                && column_left <= view.viewport_left + view.width;
            column_left = column_right;
            if !complete_columns && !visible && !owns_focus {
                measured_cells.push(Vec::new());
                continue;
            }
            let cell = if let Some(range) = ranges(row_index, column) {
                self.shape_table_cell(context, range, document_id, revision, view, control)?
            } else {
                Vec::new()
            };
            let width = ranges(row_index, column)
                .and_then(|range| self.table_cell_width(&range, revision))
                .unwrap_or_else(|| span_width(&cell));
            widths.record(row_index, column, width);
            let complete = ranges(row_index, column)
                .is_none_or(|range| self.table_cell_complete(&range, revision));
            if !context.source {
                let height = ranges(row_index, column)
                    .and_then(|range| self.table_cell_height(&range, revision))
                    .unwrap_or_else(|| {
                        cell.last()
                            .map_or(minimum_height, |row| row.y + row.natural_height())
                    });
                row_heights.record(column, height, complete);
            }
            if !complete {
                widths.measured.remove(row_index * column_count + column);
            }
            measured_cells.push(cell);
        }
        let mut budget = if widths.discovery_layout == Some(layout_revision) {
            0
        } else {
            DISCOVERY_CELLS
        };
        widths.discovery_layout = Some(layout_revision);
        // Rediscover evicted active-row heights before unrelated width work.
        while !context.source
            && row_heights.measured.count < column_count
            && row_heights.next < column_count
            && budget > 0
        {
            let column = row_heights.next;
            row_heights.next += 1;
            if row_heights.measured.contains(column) {
                continue;
            }
            control.checkpoint()?;
            budget -= 1;
            let cell = if let Some(range) = ranges(row_index, column) {
                self.shape_table_cell(context, range, document_id, revision, view, control)?
            } else {
                Vec::new()
            };
            let complete = ranges(row_index, column)
                .is_none_or(|range| self.table_cell_complete(&range, revision));
            let height = ranges(row_index, column)
                .and_then(|range| self.table_cell_height(&range, revision))
                .unwrap_or_else(|| {
                    cell.last()
                        .map_or(minimum_height, |row| row.y + row.natural_height())
                });
            let width = ranges(row_index, column)
                .and_then(|range| self.table_cell_width(&range, revision))
                .unwrap_or_else(|| span_width(&cell));
            widths.record(row_index, column, width);
            row_heights.record(column, height, complete);
            if !complete {
                widths.measured.remove(row_index * column_count + column);
                row_heights.next = column;
                break;
            }
        }
        while (!widths.dirty.is_empty() || widths.next < widths.total) && budget > 0 {
            control.checkpoint()?;
            let (row, column) = if let Some(cell) = widths.dirty.pop_front() {
                cell
            } else {
                let cell = (widths.next / column_count, widths.next % column_count);
                widths.next += 1;
                cell
            };
            if widths.seen(row, column) {
                continue;
            }
            budget -= 1;
            let cell = if let Some(range) = ranges(row, column) {
                self.shape_table_cell(context, range, document_id, revision, view, control)?
            } else {
                Vec::new()
            };
            let width = ranges(row, column)
                .and_then(|range| self.table_cell_width(&range, revision))
                .unwrap_or_else(|| span_width(&cell));
            widths.record(row, column, width);
            let complete =
                ranges(row, column).is_none_or(|range| self.table_cell_complete(&range, revision));
            if !context.source {
                let height = ranges(row, column)
                    .and_then(|range| self.table_cell_height(&range, revision))
                    .unwrap_or_else(|| {
                        cell.last()
                            .map_or(minimum_height, |row| row.y + row.natural_height())
                    });
                if row == row_index {
                    row_heights.record(column, height, complete);
                } else {
                    widths.record_height(row, column, height, complete);
                }
            }
            if !complete {
                widths.measured.remove(row * column_count + column);
                widths.dirty.push_back((row, column));
                if self.table_cache.remaining_bytes == 0 {
                    break;
                }
            }
        }
        widths.refresh_widths(min_width);
        let exact = widths.measured.count == widths.total
            && (context.source || row_heights.measured.count == column_count);
        let retained_row_height = row_heights.height();
        if !context.source {
            widths.row_heights.push_back(Arc::new(row_heights));
            while widths.row_heights.len() > 256 {
                widths.row_heights.pop_front();
            }
        }
        let column_widths = widths.widths.clone();
        let total_width = column_widths.iter().sum::<f32>()
            + (padding_left + padding_right + border) * column_count as f32;
        let available = (view.width
            - x
            - view.insets.right
            - table_style.margin_left * view.scale
            - table_style.margin_right * view.scale)
            .max(0.);
        let x = if context.source {
            x
        } else {
            x + table_style.margin_left * view.scale
                + match table_style.alignment {
                    ParagraphAlignment::Center => ((available - total_width) / 2.).max(0.),
                    ParagraphAlignment::End => (available - total_width).max(0.),
                    _ => 0.,
                }
        };
        self.table_cache.entries.push_back(Arc::new(widths));
        while self.table_cache.entries.len() > CACHE_TABLES {
            self.table_cache.entries.pop_front();
        }
        let mut rows = if context.source {
            self.align_source_table_row(
                context,
                table,
                row_index,
                measured_cells,
                &column_widths,
                document_id,
                revision,
                view,
                control,
            )?
        } else {
            let height = measured_cells
                .iter()
                .enumerate()
                .map(|(column, rows)| {
                    self.table_cell_height(&table.rows[row_index].cells[column].range, revision)
                        .unwrap_or_else(|| {
                            rows.last()
                                .map_or(minimum_height, |row| row.y + row.natural_height())
                        })
                })
                .fold(minimum_height.max(retained_row_height), f32::max)
                + padding_top
                + padding_bottom
                + row_style
                    .border_top_width
                    .max(row_style.border_bottom_width)
                    * view.scale;
            let mut result = Vec::new();
            let mut left = 0.;
            for (column, cell_rows) in measured_cells.iter_mut().enumerate() {
                let width = column_widths[column] + padding_left + padding_right + border;
                if cell_rows.is_empty() {
                    left += width;
                    continue;
                }
                let cell = &table.rows[row_index].cells[column];
                complete_boundary_affinities(cell_rows, &cell.range);
                let cell_line = context
                    .projection
                    .hard_line_at_offset(cell.range.start)
                    .unwrap_or(0);
                let geometry = TableCellGeometry {
                    table_id: table.id,
                    row: row_index,
                    column,
                    cell_id: cell.id,
                    text_range: cell.range.clone(),
                    rect: LayoutRect {
                        x: x + left,
                        y,
                        width,
                        height,
                    },
                    columns: column_count,
                    rows: row_count,
                    table_x: x,
                    table_width: total_width,
                    exact,
                };
                for row in cell_rows.iter_mut() {
                    row.fragment_index = context
                        .projection
                        .hard_line_at_offset(row.text_range.start)
                        .unwrap_or(cell_line)
                        .saturating_sub(cell_line);
                    let extra = alignment_extra(
                        table.columns[column],
                        (column_widths[column] - row.width).max(0.),
                    );
                    translate_x(row, left + padding_left + extra);
                    translate_row_vertically(row, padding_top).map_err(LayoutError::from)?;
                    row.paragraph_content_x = left + padding_left;
                    row.paragraph_content_width = column_widths[column];
                    row.table_cell = Some(geometry.clone());
                }
                if let Some(row) = cell_rows.first_mut() {
                    decorate_cell(
                        row,
                        &geometry,
                        -x,
                        -y,
                        &cell_style,
                        &header_style,
                        &table_style,
                        view.scale,
                    );
                }
                result.append(cell_rows);
                left += width;
            }
            result
        };
        if context.source {
            complete_boundary_affinities(&mut rows, &table.source_rows[row_index].range);
        }
        let height = if context.source {
            rows.iter()
                .map(|row| row.y + row.height())
                .fold(em * 1.12, f32::max)
        } else {
            rows.first()
                .and_then(|row| row.table_cell.as_ref())
                .map_or(em * 1.12, |cell| cell.rect.height)
        };
        for row in &mut rows {
            translate_x(row, x);
            // Cell geometry already uses global row coordinates.
            let cell = row.table_cell.take();
            translate_row_vertically(row, y).map_err(LayoutError::from)?;
            row.table_cell = cell;
            row.table_widths_are_exact = Some(exact);
            row.hard_line_index = line_index;
            row.hard_line_range = range.clone();
            row.wrapped_from_previous = false;
            row.wraps_to_next = false;
            for caret in &mut row.carets {
                caret.point.layout_revision = layout_revision;
            }
        }
        rows.sort_by(|a, b| {
            a.y.total_cmp(&b.y)
                .then(a.paragraph_content_x.total_cmp(&b.paragraph_content_x))
        });
        Ok(Some((rows, height)))
    }

    fn align_source_table_row(
        &mut self,
        context: &TableLayoutContext,
        table: &MarkdownTable,
        row_index: usize,
        mut cells: Vec<Vec<VisualRow>>,
        widths: &[f32],
        document_id: DocumentId,
        revision: Revision,
        view: &LayoutJobViewConfiguration,
        control: &LayoutRunControl<'_>,
    ) -> Result<Vec<VisualRow>, LayoutComputationError> {
        let source = &table.source_rows[row_index];
        // Every physical row uses the same structural gutter, even when the
        // header and body styles give the actual pipe glyph different advances.
        let mut pipe_width = 0f32;
        for row in [table.source_rows.first(), table.source_rows.get(2)]
            .into_iter()
            .flatten()
        {
            if let Some(&pipe) = row.pipes.first() {
                pipe_width = pipe_width.max(span_width(&self.shape_table_cell_piece(
                    context,
                    pipe..pipe + 1,
                    document_id,
                    revision,
                    view,
                    control,
                    None,
                )?));
            }
        }
        if source.range.len() > 4096 || source.cells.len() > 16 {
            let mut result: Option<VisualRow> = None;
            let mut boundary = 0.;
            let mut previous = source.range.start;
            let mut append = |mut piece: VisualRow, x: f32| {
                translate_x(&mut piece, x);
                if let Some(row) = &mut result {
                    row.ascent = row.ascent.max(piece.ascent);
                    row.descent = row.descent.max(piece.descent);
                    row.leading = row.leading.max(piece.leading);
                    row.clusters.append(&mut piece.clusters);
                    row.carets.append(&mut piece.carets);
                    row.decorations.append(&mut piece.decorations);
                    row.width = row.width.max(x + piece.width);
                } else {
                    result = Some(piece);
                }
            };
            for (column, cell) in source.cells.iter().take(widths.len()).enumerate() {
                let visible = !cells[column].is_empty();
                if visible && previous < cell.start {
                    let gap = self.shape_table_cell(
                        context,
                        previous..cell.start,
                        document_id,
                        revision,
                        view,
                        control,
                    )?;
                    for row in gap {
                        append(row, boundary);
                    }
                }
                let content = span_width(&cells[column]);
                let x = boundary
                    + pipe_width
                    + alignment_extra(table.columns[column], (widths[column] - content).max(0.));
                for row in cells[column].drain(..) {
                    append(row, x);
                }
                boundary += pipe_width + widths[column];
                previous = cell.end;
            }
            if previous < source.range.end
                && (boundary <= view.viewport_left + view.width && boundary >= view.viewport_left
                    || view
                        .horizontal_focus_offset
                        .is_some_and(|at| previous <= at && at <= source.range.end))
            {
                for row in self.shape_table_cell(
                    context,
                    previous..source.range.end,
                    document_id,
                    revision,
                    view,
                    control,
                )? {
                    append(row, boundary);
                }
            }
            if let Some(mut row) = result {
                row.width = row.width.max(boundary + pipe_width);
                row.text_range = source.range.clone();
                row.hard_line_range = source.range.clone();
                row.paragraph_content_width = row.width;
                row.line_advance = row.natural_height();
                row.carets.sort_by(|a, b| a.x.total_cmp(&b.x));
                return Ok(vec![row]);
            }
            return Ok(Vec::new());
        }
        let mut rows = self.shape_table_cell(
            context,
            source.range.clone(),
            document_id,
            revision,
            view,
            control,
        )?;
        let Some(row) = rows.first_mut() else {
            return Ok(rows);
        };
        // Structural pipes keep their real glyphs. Optional outer pipes use
        // the same column origin, without drawing or adding an absent pipe.
        let first_cell = source
            .cells
            .first()
            .map_or(source.range.start, |cell| cell.start);
        let first_pipe = source
            .pipes
            .first()
            .copied()
            .filter(|pipe| *pipe < first_cell);
        let prefix_end = first_pipe.unwrap_or(first_cell);
        let prefix = row
            .clusters
            .iter()
            .filter(|cluster| cluster.text_range.end <= prefix_end)
            .map(|cluster| cluster.advance)
            .sum::<f32>();
        let original = row.clusters.clone();
        let mut placements: Vec<(Range<usize>, f32)> = Vec::new();
        let mut boundary = prefix;
        for (column, cell) in source.cells.iter().take(widths.len()).enumerate() {
            let start = original
                .iter()
                .find(|cluster| cluster.text_range.start >= cell.start)
                .map_or(row.width, |cluster| cluster.x);
            let content = original
                .iter()
                .filter(|cluster| {
                    cell.start <= cluster.text_range.start && cluster.text_range.end <= cell.end
                })
                .map(|cluster| cluster.advance)
                .sum::<f32>();
            let desired = boundary
                + pipe_width
                + alignment_extra(table.columns[column], (widths[column] - content).max(0.));
            placements.push((cell.clone(), desired - start));
            if let Some(pipe) = source.pipes.iter().copied().find(|pipe| {
                *pipe < cell.start
                    && *pipe
                        >= if column == 0 {
                            source.range.start
                        } else {
                            source.cells[column - 1].end
                        }
            }) {
                if let Some(cluster) = original
                    .iter()
                    .find(|cluster| cluster.text_range.start == pipe)
                {
                    placements.push((pipe..pipe + 1, boundary - cluster.x));
                }
            }
            boundary += pipe_width + widths[column];
        }
        if let Some(last) = source.cells.get(widths.len().saturating_sub(1)) {
            if let Some(cluster) = original
                .iter()
                .find(|cluster| cluster.text_range.start >= last.end)
            {
                placements.push((last.end..source.range.end, boundary - cluster.x));
            }
        }
        for cluster in &mut row.clusters {
            if let Some((_, delta)) = placements.iter().find(|(range, _)| {
                range.start <= cluster.text_range.start && cluster.text_range.end <= range.end
            }) {
                cluster.x += delta;
                cluster.typographic_bounds.x += delta;
                cluster.ink_bounds.x += delta;
            }
        }
        for caret in &mut row.carets {
            let at = caret.point.text_offset;
            let adjacent = placements
                .iter()
                .filter(|(range, _)| range.start <= at && at <= range.end)
                .min_by_key(|(range, _)| {
                    if caret.point.affinity == BoundaryAffinity::Downstream {
                        usize::from(at == range.end)
                    } else {
                        usize::from(at == range.start)
                    }
                });
            if let Some((_, delta)) = adjacent {
                caret.x += delta;
            }
        }
        row.width = row
            .clusters
            .iter()
            .map(|cluster| cluster.x + cluster.advance)
            .fold(0., f32::max);
        row.paragraph_content_width = row.width;
        row.carets.sort_by(|left, right| left.x.total_cmp(&right.x));
        Ok(rows)
    }
}

fn decorate_cell(
    row: &mut VisualRow,
    cell: &TableCellGeometry,
    dx: f32,
    dy: f32,
    body: &crate::document::ResolvedParagraphStyle,
    header: &crate::document::ResolvedParagraphStyle,
    table: &crate::document::ResolvedParagraphStyle,
    scale: f32,
) {
    let rect = LayoutRect {
        x: cell.rect.x + dx,
        y: cell.rect.y + dy,
        ..cell.rect
    };
    let style = if cell.row == 0 { header } else { body };
    let last_row = cell.row + 1 == cell.rows;
    let last_column = cell.column + 1 == cell.columns;
    let default_color = Color {
        red: 0.65,
        green: 0.67,
        blue: 0.7,
        alpha: 0.65,
    };
    let mut add = |kind, bounds: LayoutRect, color: Color| {
        row.decorations.push(PositionedDecoration {
            kind,
            owner: Some(DecorationOwner::Paragraph(cell.cell_id)),
            text: String::new(),
            x: bounds.x,
            advance: bounds.width,
            typographic_bounds: bounds,
            ink_bounds: bounds,
            render_run: None,
            paint: ResolvedTextPaint {
                foreground: color,
                foreground_is_default: false,
                ..Default::default()
            },
            font_size: 0.,
        });
    };
    if let Some(color) = table.background {
        add(DecorationKind::BlockBackground, rect, color);
    }
    if let Some(color) = style.background {
        add(DecorationKind::BlockBackground, rect, color);
    }
    let top = if cell.row == 0 && table.border_top_width > 0. {
        table.border_top_width
    } else {
        style.border_top_width
    } * scale;
    let bottom = if last_row && table.border_bottom_width > 0. {
        table.border_bottom_width
    } else {
        style
            .border_bottom_width
            .max(if last_row { 0. } else { body.border_top_width })
    } * scale;
    let left = if cell.column == 0 && table.border_left_width > 0. {
        table.border_left_width
    } else {
        style.border_left_width
    } * scale;
    let right = if last_column && table.border_right_width > 0. {
        table.border_right_width
    } else {
        style.border_right_width.max(style.border_left_width)
    } * scale;
    for (show, width, bounds, color) in [
        (
            cell.row == 0,
            top,
            LayoutRect {
                height: top,
                ..rect
            },
            style.border_top_color,
        ),
        (
            cell.column == 0,
            left,
            LayoutRect {
                width: left,
                ..rect
            },
            style.border_left_color,
        ),
        (
            true,
            bottom,
            LayoutRect {
                y: rect.y + rect.height - bottom,
                height: bottom,
                ..rect
            },
            style.border_bottom_color,
        ),
        (
            true,
            right,
            LayoutRect {
                x: rect.x + rect.width - right,
                width: right,
                ..rect
            },
            style.border_right_color,
        ),
    ] {
        if show && width > 0. {
            add(
                DecorationKind::BlockBorder,
                bounds,
                color.unwrap_or(default_color),
            );
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TableNavigationIndex {
    cells: BTreeMap<(u64, usize, usize), Vec<usize>>,
}
impl TableNavigationIndex {
    pub(super) fn new(rows: &[VisualRow]) -> Self {
        let mut cells: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some(cell) = &row.table_cell {
                cells
                    .entry((cell.table_id, cell.row, cell.column))
                    .or_default()
                    .push(index);
            }
        }
        Self { cells }
    }
}
impl LayoutSnapshot {
    /// Geometric vertical movement visits explicit lines inside a cell, then
    /// the same column in the adjacent table row. Ordinary prose enters the
    /// cell beneath its desired x. Missing adjacent geometry returns None.
    pub fn adjacent_visual_row(&self, index: usize, down: bool, x: f32) -> Option<usize> {
        let current = self.rows.get(index)?;
        if let Some(cell) = &current.table_cell {
            let lines = self
                .table_navigation
                .cells
                .get(&(cell.table_id, cell.row, cell.column))?;
            let local = lines.binary_search(&index).ok()?;
            if down {
                if let Some(next) = lines.get(local + 1) {
                    return Some(*next);
                }
            } else if let Some(previous) = local.checked_sub(1).and_then(|at| lines.get(at)) {
                return Some(*previous);
            }
            let adjacent = if down {
                cell.row.checked_add(1).filter(|row| *row < cell.rows)
            } else {
                cell.row.checked_sub(1)
            };
            if let Some(row) = adjacent {
                let lines = self
                    .table_navigation
                    .cells
                    .get(&(cell.table_id, row, cell.column))?;
                return if down {
                    lines.first().copied()
                } else {
                    lines.last().copied()
                };
            }
        }
        // Crossing the table perimeter skips every other cell at this y.
        let candidate = if current.table_cell.is_some() {
            if down {
                self.rows
                    .partition_point(|row| row.hard_line_index <= current.hard_line_index)
            } else {
                self.rows
                    .partition_point(|row| row.hard_line_index < current.hard_line_index)
                    .checked_sub(1)?
            }
        } else if down {
            index.checked_add(1)?
        } else {
            index.checked_sub(1)?
        };
        let row = self.rows.get(candidate)?;
        let Some(cell) = &row.table_cell else {
            return Some(candidate);
        };
        let first = self.table_cells.partition_point(|candidate| {
            (candidate.table_id, candidate.row) < (cell.table_id, cell.row)
        });
        let end = self.table_cells.partition_point(|candidate| {
            (candidate.table_id, candidate.row) <= (cell.table_id, cell.row)
        });
        let choices = &self.table_cells[first..end];
        let column = choices
            .partition_point(|candidate| candidate.rect.x + candidate.rect.width <= x)
            .min(choices.len().saturating_sub(1));
        let target = choices.get(column)?;
        let lines =
            self.table_navigation
                .cells
                .get(&(target.table_id, target.row, target.column))?;
        if down {
            lines.first().copied()
        } else {
            lines.last().copied()
        }
    }
}

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    /// A large table row shares its immutable rope with a worker. Capture never
    /// allocates the full row or sends cell separators through prose wrapping.
    pub(super) fn layout_table_region_from_tree(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        tree: &crate::document::FormattedTextTree,
        ranges: &[Range<usize>],
        first: usize,
        total: usize,
        following: Option<Range<usize>>,
        styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        let control = LayoutRunControl::cancellable(cancellation);
        let context = styles
            .table_context
            .as_ref()
            .ok_or(LayoutError::InvalidGeometry)?;
        let revision = LayoutRevision(
            self.next_layout_revision.max(
                view.latest_layout_revision
                    .map_or(1, |revision| revision.0 + 1),
            ),
        );
        let content_insets = add_insets(view.insets, styles.document_insets);
        let before = (
            self.table_cache.measured_cells,
            self.table_cache.measured_text_bytes,
        );
        for (index, range) in ranges.iter().enumerate() {
            control.checkpoint()?;
            if context.contains_line(range) {
                self.table_line(
                    context,
                    range,
                    document_id,
                    document_revision,
                    view,
                    first + index,
                    revision,
                    content_insets.left,
                    0.,
                    false,
                    &control,
                )?;
            }
        }
        let mut lines = Vec::new();
        for (index, range) in ranges.iter().enumerate() {
            control.checkpoint()?;
            let line_index = first + index;
            let next = ranges.get(index + 1).cloned().or_else(|| following.clone());
            if let Some((rows, height)) = self.table_line(
                context,
                range,
                document_id,
                document_revision,
                view,
                line_index,
                revision,
                content_insets.left,
                if line_index == 0 {
                    content_insets.top
                } else {
                    0.
                },
                false,
                &control,
            )? {
                let paragraph = resolve_line_paragraph(
                    range,
                    &styles.paragraphs,
                    &styles.default_shaping_style,
                );
                let next_paragraph = next.as_ref().map(|range| {
                    resolve_line_paragraph(range, &styles.paragraphs, &styles.default_shaping_style)
                });
                lines.push(RegionalHardLineLayout {
                    layout_revision: revision,
                    hard_line_index: line_index,
                    hard_line_range: range.clone(),
                    text_coverage: range.clone(),
                    rows,
                    height: f64::from(
                        height
                            + if line_index == 0 {
                                content_insets.top
                            } else {
                                0.
                            }
                            + if line_index + 1 == total {
                                content_insets.bottom
                            } else {
                                0.
                            },
                    ),
                    height_is_exact: true,
                    next_checkpoint: None,
                    diagnostics: Vec::new(),
                    render_run_policy: self.provider.render_run_policy(),
                    inputs: line_layout_inputs(
                        range,
                        &paragraph,
                        next_paragraph.as_ref(),
                        &styles.shaping_runs,
                        styles,
                    ),
                });
            } else {
                let text = tree
                    .slice(range.clone())
                    .map_err(|_| LayoutError::InvalidTextOffset(range.start))?;
                let text =
                    super::super::jobs::flow_text(text, range.start, std::slice::from_ref(range));
                let mut part = self.layout_hard_line_region_cancellable(
                    document_id,
                    document_revision,
                    &text,
                    range.start,
                    std::slice::from_ref(range),
                    line_index,
                    total,
                    tree.byte_len(),
                    next,
                    styles,
                    view,
                    cancellation,
                )?;
                part.rebind_revision(revision);
                lines.extend(part.lines);
            }
        }
        self.next_layout_revision = self.next_layout_revision.max(revision.0.saturating_add(1));
        Ok(RegionalLayoutSnapshot {
            whitespace_unit: styles.whitespace_shaping_style.size * view.scale * 0.5,
            revision,
            document_id,
            document_revision,
            configuration_generation: view.configuration_generation,
            measurement_environment_id: self.provider.measurement_environment_id(),
            metrics_generation: self.provider.metrics_generation(),
            hard_lines: first..first + ranges.len(),
            document_hard_line_count: total,
            document_text_len: tree.byte_len(),
            viewport_width: view.width,
            viewport_height: view.height,
            usable_width: usable_width(view.width, content_insets),
            content_insets,
            document_insets: styles.document_insets,
            document_style_revision: Some(styles.style_sheet_revision),
            canvas_background: styles.canvas_background,
            canvas_background_is_default: styles.canvas_background_is_default,
            default_paint: styles.default_paint.clone(),
            paint_runs: styles.paint_runs.clone(),
            lines,
            diagnostics: styles.recovery_diagnostic.iter().cloned().collect(),
            grapheme_boundaries: Vec::new(),
            work_statistics: LayoutWorkStatistics {
                table_measured_cells: self.table_cache.measured_cells.saturating_sub(before.0),
                table_measured_text_bytes: self
                    .table_cache
                    .measured_text_bytes
                    .saturating_sub(before.1),
                ..Default::default()
            },
            horizontal_materialization: Some(HorizontalMaterialization {
                text: tree.clone(),
                rows: Vec::new(),
            }),
        })
    }
}
