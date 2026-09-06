use super::height_index::{
    HardLineHeightHit, HeightMeasurement, ViewHeightIndex, ViewHeightIndexError,
    ViewHeightIndexStatistics,
};
use super::jobs::LayoutJobId;
use super::measurement::*;
use super::style::{
    DocumentLayoutStyles, DocumentStyleError, PaintStyleRun, ParagraphLayoutStyle,
    ResolvedTextPaint,
};
use crate::document::{
    Color, Document, DocumentId, LineSpacing, ParagraphAlignment, Revision, StyleSheetRevision,
    TextRange, WritingDirection,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use unicode_linebreak::{linebreaks, split_at_safe};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const MAX_SHAPE_FRAGMENT_BYTES: usize = 4096;
pub(super) const SHAPING_CONTEXT_BYTES: usize = 32;
const DEFAULT_CACHE_ENTRIES: usize = 2048;
const DEFAULT_ESTIMATED_HARD_LINE_HEIGHT: f64 = 16.0;
const DEFAULT_REGIONAL_CACHE_HARD_LINES: usize = 2048;
const DEFAULT_REGIONAL_CACHE_VISUAL_ROWS: usize = 8192;
const DEFAULT_REGIONAL_CACHE_ESTIMATED_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS: usize = 32;
pub(super) const CANCELLATION_CLUSTER_BATCH: usize = 256;
const CANCELLATION_TEXT_SCAN_BYTES: usize = 64 * 1024;

/// Resume state for a wrapped hard line whose exact rows are materialized in
/// bounded text slices. Checkpoints are emitted only at visual-row boundaries,
/// so the pending row advance is zero; the cumulative advance and most recent
/// candidate break make the consumed prefix explicit for diagnostics and
/// future cache admission.
#[derive(Clone, Debug, PartialEq)]
pub struct LongLineLayoutCheckpoint {
    pub(super) document_id: DocumentId,
    pub(super) document_revision: Revision,
    pub(super) configuration_generation: ViewConfigurationGeneration,
    pub(super) measurement_environment_id: MeasurementEnvironmentId,
    pub(super) metrics_generation: MetricsGeneration,
    pub(super) hard_line_index: usize,
    pub(super) hard_line_range: Range<usize>,
    pub(super) next_text_offset: usize,
    pub(super) completed_visual_rows: usize,
    pub(super) completed_height: f32,
    pub(super) cumulative_advance: f64,
    pub(super) last_candidate_break: Option<usize>,
    pub(super) continuation_width: f32,
    pub(super) right_to_left: bool,
}

impl LongLineLayoutCheckpoint {
    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn configuration_generation(&self) -> ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    pub fn metrics_generation(&self) -> MetricsGeneration {
        self.metrics_generation
    }

    pub fn hard_line_index(&self) -> usize {
        self.hard_line_index
    }

    pub fn hard_line_range(&self) -> Range<usize> {
        self.hard_line_range.clone()
    }

    pub fn next_text_offset(&self) -> usize {
        self.next_text_offset
    }

    pub fn completed_visual_rows(&self) -> usize {
        self.completed_visual_rows
    }

    pub fn completed_height(&self) -> f32 {
        self.completed_height
    }

    pub fn cumulative_advance(&self) -> f64 {
        self.cumulative_advance
    }

    pub fn last_candidate_break(&self) -> Option<usize> {
        self.last_candidate_break
    }

    pub fn continuation_width(&self) -> f32 {
        self.continuation_width
    }

    pub fn right_to_left(&self) -> bool {
        self.right_to_left
    }
}

/// Deterministic structural work accounting for one regional computation.
/// These counters let tests enforce bounded units without timing assertions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutWorkStatistics {
    segmented_text_bytes: usize,
    shaping_fragment_count: usize,
    maximum_shaping_fragment_bytes: usize,
    wrapped_cluster_count: usize,
    maximum_wrap_checkpoint_clusters: usize,
    positioned_cluster_count: usize,
    maximum_position_checkpoint_clusters: usize,
}

impl LayoutWorkStatistics {
    pub fn segmented_text_bytes(self) -> usize {
        self.segmented_text_bytes
    }

    pub fn shaping_fragment_count(self) -> usize {
        self.shaping_fragment_count
    }

    pub fn maximum_shaping_fragment_bytes(self) -> usize {
        self.maximum_shaping_fragment_bytes
    }

    pub fn wrapped_cluster_count(self) -> usize {
        self.wrapped_cluster_count
    }

    pub fn maximum_wrap_checkpoint_clusters(self) -> usize {
        self.maximum_wrap_checkpoint_clusters
    }

    pub fn positioned_cluster_count(self) -> usize {
        self.positioned_cluster_count
    }

    pub fn maximum_position_checkpoint_clusters(self) -> usize {
        self.maximum_position_checkpoint_clusters
    }
}

/// A cheap cooperative cancellation query used by interruptible snapshot
/// layout. Implementations must not block; background jobs normally use one
/// atomic load per checkpoint.
pub trait LayoutCancellationProbe {
    fn is_cancelled(&self) -> bool;
}

/// Failure from interruptible layout keeps cancellation distinct from an
/// invalid document, style, or measurement result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutComputationError {
    Cancelled,
    Layout(LayoutError),
}

impl From<LayoutError> for LayoutComputationError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutRevision(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ViewConfigurationGeneration(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeInsets {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Materialized geometry represented by one immutable layout snapshot.
/// Partial snapshots retain document-global hard-line, text, and vertical
/// coordinates. Their vertical origin may be based on estimated preceding
/// heights, as reported by `prefix_is_exact`; geometry within the represented
/// hard-line bands is exact for the snapshot's dependencies.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutCoverage {
    FullDocument {
        hard_line_count: usize,
    },
    PartialHardLines {
        hard_lines: Range<usize>,
        document_hard_line_count: usize,
        text_ranges: Vec<Range<usize>>,
        vertical_range: Range<f32>,
        prefix_is_exact: bool,
    },
}

impl LayoutCoverage {
    pub fn is_full_document(&self) -> bool {
        matches!(self, Self::FullDocument { .. })
    }

    pub fn hard_lines(&self) -> Range<usize> {
        match self {
            Self::FullDocument { hard_line_count } => 0..*hard_line_count,
            Self::PartialHardLines { hard_lines, .. } => hard_lines.clone(),
        }
    }

    pub fn document_hard_line_count(&self) -> usize {
        match self {
            Self::FullDocument { hard_line_count } => *hard_line_count,
            Self::PartialHardLines {
                document_hard_line_count,
                ..
            } => *document_hard_line_count,
        }
    }

    pub fn vertical_range(&self) -> Option<Range<f32>> {
        match self {
            Self::FullDocument { .. } => None,
            Self::PartialHardLines { vertical_range, .. } => Some(vertical_range.clone()),
        }
    }

    pub fn prefix_is_exact(&self) -> bool {
        match self {
            Self::FullDocument { .. } => true,
            Self::PartialHardLines {
                prefix_is_exact, ..
            } => *prefix_is_exact,
        }
    }

    pub fn contains_text_offset(&self, text_offset: usize) -> bool {
        match self {
            Self::FullDocument { .. } => true,
            Self::PartialHardLines { text_ranges, .. } => text_ranges
                .iter()
                .any(|range| range.start <= text_offset && text_offset <= range.end),
        }
    }

    pub fn contains_y(&self, y: f32) -> bool {
        match self {
            Self::FullDocument { .. } => true,
            Self::PartialHardLines { vertical_range, .. } => {
                vertical_range.start <= y && y < vertical_range.end
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretPoint {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub layout_revision: LayoutRevision,
    pub text_offset: usize,
    pub affinity: BoundaryAffinity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretGeometry {
    pub point: CaretPoint,
    pub rect: LayoutRect,
    pub row_index: usize,
    /// True when this represents the containing indivisible shaping cluster
    /// rather than an independently addressable visual caret stop.
    pub is_cluster_fallback: bool,
}

/// One drawable portion of a logical formatted-text selection. Rectangles are
/// returned in visual row order and then left-to-right order within each row.
/// A logical range may produce several rectangles on one row when bidi visual
/// ordering places unselected content between selected clusters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRectangle {
    pub rect: LayoutRect,
    pub row_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionedCluster {
    pub text_range: Range<usize>,
    pub x: f32,
    pub advance: f32,
    /// Absolute typographic bounds in layout coordinates.
    pub typographic_bounds: LayoutRect,
    /// Absolute ink bounds, which may extend outside the row's line advance.
    pub ink_bounds: LayoutRect,
    pub bidi_level: u8,
    pub fallback_font: String,
    pub render_run: Option<RenderRunHandle>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionedCaret {
    pub point: CaretPoint,
    pub x: f32,
    pub row_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualRow {
    pub paragraph_id: Option<u64>,
    pub hard_line_index: usize,
    /// Zero-based visual fragment within this formatted hard line.
    pub fragment_index: usize,
    pub hard_line_range: Range<usize>,
    pub text_range: Range<usize>,
    pub y: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    /// Vertical distance to the next row. It may be smaller than the ink
    /// bounds for exact line spacing.
    pub line_advance: f32,
    pub width: f32,
    pub paragraph_content_x: f32,
    pub paragraph_content_width: f32,
    pub wrapped_from_previous: bool,
    pub wraps_to_next: bool,
    pub clusters: Vec<PositionedCluster>,
    pub carets: Vec<PositionedCaret>,
}

impl VisualRow {
    pub fn height(&self) -> f32 {
        self.line_advance
    }

    pub fn natural_height(&self) -> f32 {
        self.ascent + self.descent + self.leading
    }

    pub fn bounds(&self, left: f32) -> LayoutRect {
        LayoutRect {
            x: left,
            y: self.y,
            width: self.width,
            height: self.height(),
        }
    }

    /// Union of provider-reported glyph ink in absolute layout coordinates.
    /// Empty hard lines have caret/line geometry but no ink bounds.
    pub fn ink_bounds(&self) -> Option<LayoutRect> {
        let mut bounds = self.clusters.first()?.ink_bounds;
        for cluster in &self.clusters[1..] {
            let right = bounds.x + bounds.width;
            let bottom = bounds.y + bounds.height;
            let cluster_right = cluster.ink_bounds.x + cluster.ink_bounds.width;
            let cluster_bottom = cluster.ink_bounds.y + cluster.ink_bounds.height;
            let x = bounds.x.min(cluster.ink_bounds.x);
            let y = bounds.y.min(cluster.ink_bounds.y);
            bounds = LayoutRect {
                x,
                y,
                width: right.max(cluster_right) - x,
                height: bottom.max(cluster_bottom) - y,
            };
        }
        Some(bounds)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSnapshot {
    pub revision: LayoutRevision,
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub configuration_generation: ViewConfigurationGeneration,
    pub measurement_environment_id: MeasurementEnvironmentId,
    pub metrics_generation: MetricsGeneration,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub usable_width: f32,
    /// Sum of view-owned chrome insets and resolved document canvas padding.
    pub content_insets: EdgeInsets,
    pub document_insets: EdgeInsets,
    pub document_style_revision: Option<StyleSheetRevision>,
    pub canvas_background: Color,
    pub canvas_background_is_default: bool,
    pub default_paint: ResolvedTextPaint,
    pub paint_runs: Vec<PaintStyleRun>,
    pub coverage: LayoutCoverage,
    pub rows: Vec<VisualRow>,
    /// Horizontal extent of the materialized canvas in document layout
    /// coordinates. This is the exact full-document extent when
    /// `content_width_is_exact` is true and otherwise only a lower bound from
    /// the materialized rows. Presentation scroll is deliberately not baked
    /// into this geometry.
    pub content_width: f32,
    pub content_width_is_exact: bool,
    pub total_height: f32,
    pub total_height_is_exact: bool,
    pub diagnostics: Vec<ShapingDiagnostic>,
    text_len: usize,
    /// Sorted logical extended-grapheme boundaries covered by this snapshot.
    /// This remains distinct from provider caret stops because an indivisible
    /// shaping cluster may contain several legal logical edit boundaries.
    grapheme_boundaries: Vec<usize>,
}

/// Exact layout for one hard line. Row `y` values are relative to the start of
/// this hard-line band; global vertical placement is obtained from the view's
/// height-index prefix. Text ranges and caret offsets remain document-global.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionalHardLineLayout {
    layout_revision: LayoutRevision,
    hard_line_index: usize,
    hard_line_range: Range<usize>,
    text_coverage: Range<usize>,
    rows: Vec<VisualRow>,
    height: f64,
    height_is_exact: bool,
    next_checkpoint: Option<LongLineLayoutCheckpoint>,
}

impl RegionalHardLineLayout {
    pub fn layout_revision(&self) -> LayoutRevision {
        self.layout_revision
    }

    pub fn hard_line_index(&self) -> usize {
        self.hard_line_index
    }

    pub fn hard_line_range(&self) -> Range<usize> {
        self.hard_line_range.clone()
    }

    /// Exact logical text represented by `rows`. This equals the complete hard
    /// line for ordinary regional results and is a bounded subrange for a
    /// resumable long-line viewport result.
    pub fn text_coverage(&self) -> Range<usize> {
        self.text_coverage.clone()
    }

    pub fn rows(&self) -> &[VisualRow] {
        &self.rows
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    pub fn height_is_exact(&self) -> bool {
        self.height_is_exact
    }

    pub fn next_checkpoint(&self) -> Option<&LongLineLayoutCheckpoint> {
        self.next_checkpoint.as_ref()
    }
}

/// Immutable worker result for a bounded sequence of hard lines.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionalLayoutSnapshot {
    revision: LayoutRevision,
    document_id: DocumentId,
    document_revision: Revision,
    configuration_generation: ViewConfigurationGeneration,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
    hard_lines: Range<usize>,
    document_hard_line_count: usize,
    document_text_len: usize,
    viewport_width: f32,
    viewport_height: f32,
    usable_width: f32,
    content_insets: EdgeInsets,
    document_insets: EdgeInsets,
    document_style_revision: Option<StyleSheetRevision>,
    canvas_background: Color,
    canvas_background_is_default: bool,
    default_paint: ResolvedTextPaint,
    paint_runs: Vec<PaintStyleRun>,
    lines: Vec<RegionalHardLineLayout>,
    diagnostics: Vec<ShapingDiagnostic>,
    grapheme_boundaries: Vec<usize>,
    work_statistics: LayoutWorkStatistics,
}

impl RegionalLayoutSnapshot {
    pub fn revision(&self) -> LayoutRevision {
        self.revision
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn configuration_generation(&self) -> ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    pub fn metrics_generation(&self) -> MetricsGeneration {
        self.metrics_generation
    }

    pub fn hard_lines(&self) -> Range<usize> {
        self.hard_lines.clone()
    }

    pub fn document_hard_line_count(&self) -> usize {
        self.document_hard_line_count
    }

    pub fn document_text_len(&self) -> usize {
        self.document_text_len
    }

    pub fn viewport_width(&self) -> f32 {
        self.viewport_width
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn usable_width(&self) -> f32 {
        self.usable_width
    }

    pub fn content_insets(&self) -> EdgeInsets {
        self.content_insets
    }

    pub fn document_insets(&self) -> EdgeInsets {
        self.document_insets
    }

    pub fn document_style_revision(&self) -> Option<StyleSheetRevision> {
        self.document_style_revision
    }

    pub fn canvas_background(&self) -> Color {
        self.canvas_background
    }

    pub fn default_paint(&self) -> &ResolvedTextPaint {
        &self.default_paint
    }

    pub fn paint_runs(&self) -> &[PaintStyleRun] {
        &self.paint_runs
    }

    pub fn lines(&self) -> &[RegionalHardLineLayout] {
        &self.lines
    }

    pub fn diagnostics(&self) -> &[ShapingDiagnostic] {
        &self.diagnostics
    }

    pub fn work_statistics(&self) -> LayoutWorkStatistics {
        self.work_statistics
    }

    pub fn next_long_line_checkpoint(&self) -> Option<&LongLineLayoutCheckpoint> {
        self.lines
            .iter()
            .find_map(RegionalHardLineLayout::next_checkpoint)
    }

    fn rebind_revision(&mut self, revision: LayoutRevision) {
        self.revision = revision;
        for line in &mut self.lines {
            line.layout_revision = revision;
            for row in &mut line.rows {
                for caret in &mut row.carets {
                    caret.point.layout_revision = revision;
                }
            }
        }
    }
}

/// Coordinator-owned limits for exact off-screen hard-line layouts.
///
/// A zero limit disables regional retention for that dimension. The currently
/// installed immutable viewport snapshot is independent of this cache and is
/// never evicted by these limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionalLayoutCacheLimits {
    pub max_hard_lines: usize,
    pub max_visual_rows: usize,
    pub max_estimated_bytes: usize,
}

impl Default for RegionalLayoutCacheLimits {
    fn default() -> Self {
        Self {
            max_hard_lines: DEFAULT_REGIONAL_CACHE_HARD_LINES,
            max_visual_rows: DEFAULT_REGIONAL_CACHE_VISUAL_ROWS,
            max_estimated_bytes: DEFAULT_REGIONAL_CACHE_ESTIMATED_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegionalLayoutCacheStatistics {
    hard_line_count: usize,
    contiguous_region_count: usize,
    visual_row_count: usize,
    estimated_bytes: usize,
    eviction_count: u64,
}

impl RegionalLayoutCacheStatistics {
    pub fn hard_line_count(self) -> usize {
        self.hard_line_count
    }

    pub fn contiguous_region_count(self) -> usize {
        self.contiguous_region_count
    }

    pub fn visual_row_count(self) -> usize {
        self.visual_row_count
    }

    /// Deterministic accounting estimate used by the cache budget. This is not
    /// allocator telemetry, but includes retained rows, clusters, carets, and
    /// owned fallback-font strings.
    pub fn estimated_bytes(self) -> usize {
        self.estimated_bytes
    }

    pub fn eviction_count(self) -> u64 {
        self.eviction_count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegionalCacheIdentity {
    document_id: DocumentId,
    document_revision: Revision,
    configuration_generation: ViewConfigurationGeneration,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
    document_hard_line_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct RegionalLayoutCache {
    identity: Option<RegionalCacheIdentity>,
    latest_revision: Option<LayoutRevision>,
    lines: BTreeMap<usize, Arc<RegionalHardLineLayout>>,
    /// Installation-recency order. Regional reads are immutable, so installing
    /// or refreshing a line is the cache's LRU-equivalent touch operation.
    recency: VecDeque<usize>,
    visual_row_count: usize,
    estimated_bytes: usize,
    eviction_count: u64,
}

impl RegionalLayoutCache {
    fn with_identity(identity: RegionalCacheIdentity) -> Self {
        Self {
            identity: Some(identity),
            ..Self::default()
        }
    }

    fn insert(&mut self, line: RegionalHardLineLayout) {
        let hard_line = line.hard_line_index;
        if let Some(previous) = self.lines.remove(&hard_line) {
            self.remove_accounting(&previous);
        }
        self.recency.retain(|cached| *cached != hard_line);
        self.add_accounting(&line);
        self.lines.insert(hard_line, Arc::new(line));
        self.recency.push_back(hard_line);
    }

    fn enforce_limits(&mut self, limits: RegionalLayoutCacheLimits) {
        while self.lines.len() > limits.max_hard_lines
            || self.visual_row_count > limits.max_visual_rows
            || self.estimated_bytes > limits.max_estimated_bytes
        {
            let Some(oldest) = self.recency.pop_front() else {
                break;
            };
            if let Some(evicted) = self.lines.remove(&oldest) {
                self.remove_accounting(&evicted);
                self.eviction_count = self.eviction_count.saturating_add(1);
            }
        }
    }

    fn add_accounting(&mut self, line: &RegionalHardLineLayout) {
        self.visual_row_count = self.visual_row_count.saturating_add(line.rows.len());
        self.estimated_bytes = self
            .estimated_bytes
            .saturating_add(estimated_regional_line_bytes(line));
    }

    fn remove_accounting(&mut self, line: &RegionalHardLineLayout) {
        self.visual_row_count = self.visual_row_count.saturating_sub(line.rows.len());
        self.estimated_bytes = self
            .estimated_bytes
            .saturating_sub(estimated_regional_line_bytes(line));
    }
}

fn estimated_regional_line_bytes(line: &RegionalHardLineLayout) -> usize {
    let mut bytes = std::mem::size_of::<RegionalHardLineLayout>();
    for row in &line.rows {
        bytes = bytes
            .saturating_add(std::mem::size_of::<VisualRow>())
            .saturating_add(
                row.clusters
                    .len()
                    .saturating_mul(std::mem::size_of::<PositionedCluster>()),
            )
            .saturating_add(
                row.carets
                    .len()
                    .saturating_mul(std::mem::size_of::<PositionedCaret>()),
            );
        for cluster in &row.clusters {
            bytes = bytes.saturating_add(cluster.fallback_font.len());
        }
    }
    bytes
}

struct PreparedRegionalInstall {
    installed_revision: LayoutRevision,
    region: RegionalLayoutSnapshot,
    height_index: ViewHeightIndex,
    cache: RegionalLayoutCache,
}

impl LayoutSnapshot {
    pub fn hit_test(&self, point: LayoutPoint) -> Result<CaretPoint, LayoutError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        if !self.coverage.contains_y(point.y) {
            // The blank canvas before/after the document belongs to its
            // first/last row. Partial viewport snapshots often contain an
            // entire short document but retain PartialHardLines coverage.
            // Only extend hit testing when the actual text endpoint is exact;
            // missing off-screen text must still request materialization.
            let before_start = self
                .rows
                .first()
                .is_some_and(|row| row.text_range.start == 0 && point.y < row.y);
            let after_end = self.rows.last().is_some_and(|row| {
                row.text_range.end == self.text_len && point.y >= row.y + row.height()
            });
            if !before_start && !after_end {
                return Err(LayoutError::OutsideMaterializedCoverage);
            }
        }
        let row = self.closest_row(point.y).ok_or(LayoutError::NoRows)?;
        row.carets
            .iter()
            .min_by(|left, right| {
                let left_distance = (left.x - point.x).abs();
                let right_distance = (right.x - point.x).abs();
                left_distance
                    .partial_cmp(&right_distance)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| {
                        affinity_rank(left.point.affinity).cmp(&affinity_rank(right.point.affinity))
                    })
            })
            .map(|caret| caret.point)
            .ok_or(LayoutError::NoCaretStops)
    }

    pub fn caret_geometry(&self, point: CaretPoint) -> Result<CaretGeometry, LayoutError> {
        self.validate_caret_point(point)?;
        for row in &self.rows {
            if let Some(caret) = row.carets.iter().find(|caret| caret.point == point) {
                return Ok(CaretGeometry {
                    point,
                    rect: LayoutRect {
                        x: caret.x,
                        y: row.y,
                        width: 0.0,
                        height: row.ascent + row.descent,
                    },
                    row_index: caret.row_index,
                    is_cluster_fallback: false,
                });
            }
        }
        Err(LayoutError::NotACaretStop {
            text_offset: point.text_offset,
        })
    }

    /// Returns geometry for a logical grapheme endpoint even when the shaper
    /// made its containing cluster visually indivisible. This does not promote
    /// the endpoint to a legal visual caret stop.
    pub fn logical_endpoint_geometry(
        &self,
        text_offset: usize,
        affinity: BoundaryAffinity,
    ) -> Result<CaretGeometry, LayoutError> {
        if text_offset > self.text_len {
            return Err(LayoutError::InvalidTextOffset(text_offset));
        }
        if !self.coverage.contains_text_offset(text_offset) {
            return Err(LayoutError::OutsideMaterializedCoverage);
        }
        let point = CaretPoint {
            document_id: self.document_id,
            document_revision: self.document_revision,
            layout_revision: self.revision,
            text_offset,
            affinity,
        };
        if let Ok(geometry) = self.caret_geometry(point) {
            return Ok(geometry);
        }

        for (row_index, row) in self.rows.iter().enumerate() {
            if let Some(cluster) = row.clusters.iter().find(|cluster| {
                cluster.text_range.start < text_offset && text_offset < cluster.text_range.end
            }) {
                return Ok(CaretGeometry {
                    point,
                    rect: LayoutRect {
                        x: cluster.x,
                        y: row.y,
                        width: cluster.advance,
                        height: row.ascent + row.descent,
                    },
                    row_index,
                    is_cluster_fallback: true,
                });
            }
        }
        Err(LayoutError::NotACaretStop { text_offset })
    }

    /// Maps one checked logical half-open text range to drawable geometry in
    /// this immutable snapshot. The range remains logical: a boundary inside
    /// an indivisible shaping cluster highlights that complete cluster without
    /// widening the text selection. Empty ranges return caret geometry, using
    /// `empty_affinity` to choose a split visual side.
    pub fn selection_rectangles(
        &self,
        range: TextRange,
        empty_affinity: BoundaryAffinity,
    ) -> Result<Vec<SelectionRectangle>, LayoutError> {
        self.validate_selection_range(range)?;
        let start = range.start().offset();
        let end = range.end().offset();
        if start == end {
            let geometry = self.logical_endpoint_geometry(start, empty_affinity)?;
            return Ok(vec![SelectionRectangle {
                rect: geometry.rect,
                row_index: geometry.row_index,
            }]);
        }

        let mut rectangles = Vec::new();
        for (row_index, row) in self.rows.iter().enumerate() {
            let mut current = None;
            for cluster in &row.clusters {
                let selected = cluster.text_range.start < end && start < cluster.text_range.end;
                if !selected {
                    flush_selection_rectangle(&mut rectangles, row_index, &mut current);
                    continue;
                }
                append_selection_cluster(&mut rectangles, row_index, &mut current, cluster);
            }
            flush_selection_rectangle(&mut rectangles, row_index, &mut current);

            // A hard-line boundary is an atomic logical item but has no glyph
            // cluster. Preserve geometry for a selection containing only that
            // item by returning its upstream end-of-line caret rectangle.
            let break_offset = row.hard_line_range.end;
            if !row.wraps_to_next
                && break_offset < self.text_len
                && start <= break_offset
                && break_offset < end
            {
                if let Some(caret) = row
                    .carets
                    .iter()
                    .find(|caret| {
                        caret.point.text_offset == break_offset
                            && caret.point.affinity == BoundaryAffinity::Upstream
                    })
                    .or_else(|| {
                        row.carets
                            .iter()
                            .find(|caret| caret.point.text_offset == break_offset)
                    })
                {
                    rectangles.push(SelectionRectangle {
                        rect: LayoutRect {
                            x: caret.x,
                            y: row.y,
                            width: 0.0,
                            height: row.ascent + row.descent,
                        },
                        row_index,
                    });
                }
            }
        }
        rectangles.sort_by(|left, right| {
            left.row_index
                .cmp(&right.row_index)
                .then_with(|| left.rect.x.total_cmp(&right.rect.x))
        });
        Ok(rectangles)
    }

    pub fn caret_point(
        &self,
        text_offset: usize,
        affinity: BoundaryAffinity,
    ) -> Result<CaretPoint, LayoutError> {
        let point = CaretPoint {
            document_id: self.document_id,
            document_revision: self.document_revision,
            layout_revision: self.revision,
            text_offset,
            affinity,
        };
        self.caret_geometry(point).map(|_| point)
    }

    fn closest_row(&self, y: f32) -> Option<&VisualRow> {
        if self.rows.is_empty() {
            return None;
        }
        let insertion = self.rows.partition_point(|row| row.y + row.height() <= y);
        if insertion == self.rows.len() {
            return self.rows.last();
        }
        if insertion == 0 || y >= self.rows[insertion].y {
            return self.rows.get(insertion);
        }
        let before = &self.rows[insertion - 1];
        let after = &self.rows[insertion];
        let before_distance = (y - (before.y + before.height())).abs();
        let after_distance = (after.y - y).abs();
        if before_distance <= after_distance {
            Some(before)
        } else {
            Some(after)
        }
    }

    fn validate_caret_point(&self, point: CaretPoint) -> Result<(), LayoutError> {
        if point.document_id != self.document_id {
            return Err(LayoutError::WrongDocument);
        }
        if point.document_revision != self.document_revision {
            return Err(LayoutError::WrongDocumentRevision);
        }
        if point.layout_revision != self.revision {
            return Err(LayoutError::StaleLayout {
                expected: self.revision,
                actual: point.layout_revision,
            });
        }
        if point.text_offset > self.text_len {
            return Err(LayoutError::InvalidTextOffset(point.text_offset));
        }
        if !self.coverage.contains_text_offset(point.text_offset) {
            return Err(LayoutError::OutsideMaterializedCoverage);
        }
        Ok(())
    }

    fn validate_selection_range(&self, range: TextRange) -> Result<(), LayoutError> {
        let start = range.start();
        let end = range.end();
        if start.document() != self.document_id || end.document() != self.document_id {
            return Err(LayoutError::WrongDocument);
        }
        if start.revision() != self.document_revision || end.revision() != self.document_revision {
            return Err(LayoutError::WrongDocumentRevision);
        }
        for offset in [start.offset(), end.offset()] {
            if offset > self.text_len {
                return Err(LayoutError::InvalidTextOffset(offset));
            }
        }
        if !self.selection_range_is_covered(start.offset()..end.offset()) {
            return Err(LayoutError::OutsideMaterializedCoverage);
        }
        for offset in [start.offset(), end.offset()] {
            if self.grapheme_boundaries.binary_search(&offset).is_err() {
                return Err(LayoutError::InvalidGraphemeBoundary {
                    text_offset: offset,
                });
            }
        }
        Ok(())
    }

    fn selection_range_is_covered(&self, range: Range<usize>) -> bool {
        match &self.coverage {
            LayoutCoverage::FullDocument { .. } => true,
            LayoutCoverage::PartialHardLines { text_ranges, .. } => {
                let Some(first) = text_ranges.first() else {
                    return false;
                };
                let Some(last) = text_ranges.last() else {
                    return false;
                };
                first.start <= range.start && range.end <= last.end
            }
        }
    }
}

fn append_selection_cluster(
    rectangles: &mut Vec<SelectionRectangle>,
    row_index: usize,
    current: &mut Option<LayoutRect>,
    cluster: &PositionedCluster,
) {
    let candidate = cluster.typographic_bounds;
    if let Some(existing) = current.as_mut() {
        let existing_right = existing.x + existing.width;
        let candidate_right = candidate.x + candidate.width;
        let same_vertical_metrics = layout_units_equal(existing.y, candidate.y)
            && layout_units_equal(existing.height, candidate.height);
        let horizontally_contiguous = candidate.x
            <= existing_right + layout_epsilon(existing_right)
            && existing.x <= candidate_right + layout_epsilon(candidate_right);
        if same_vertical_metrics && horizontally_contiguous {
            let left = existing.x.min(candidate.x);
            let right = existing_right.max(candidate_right);
            existing.x = left;
            existing.width = right - left;
            return;
        }
        flush_selection_rectangle(rectangles, row_index, current);
    }
    *current = Some(candidate);
}

fn flush_selection_rectangle(
    rectangles: &mut Vec<SelectionRectangle>,
    row_index: usize,
    current: &mut Option<LayoutRect>,
) {
    if let Some(rect) = current.take() {
        rectangles.push(SelectionRectangle { rect, row_index });
    }
}

fn layout_units_equal(left: f32, right: f32) -> bool {
    (left - right).abs() <= layout_epsilon(left.abs().max(right.abs()))
}

fn layout_epsilon(value: f32) -> f32 {
    value.max(1.0) * f32::EPSILON * 8.0
}

/// Minimal immutable view input captured for regional worker layout. It
/// deliberately contains no positioned snapshot, height index, regional cache,
/// presentation-only horizontal/vertical scroll, errors, or scheduling state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutJobViewConfiguration {
    width: f32,
    height: f32,
    insets: EdgeInsets,
    wrap: bool,
    linebreak: bool,
    scale: f32,
    configuration_generation: ViewConfigurationGeneration,
    default_style: ResolvedTextStyle,
    default_style_is_override: bool,
    style_runs: Vec<ShapeStyleRun>,
    style_runs_are_override: bool,
    latest_layout_revision: Option<LayoutRevision>,
}

impl LayoutJobViewConfiguration {
    pub(super) fn retained_override_style_run_count(&self) -> usize {
        if self.style_runs_are_override {
            self.style_runs.len()
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewLayout {
    width: f32,
    height: f32,
    viewport_left: f32,
    viewport_top: f32,
    insets: EdgeInsets,
    wrap: bool,
    linebreak: bool,
    scale: f32,
    configuration_generation: ViewConfigurationGeneration,
    default_style: ResolvedTextStyle,
    default_style_is_override: bool,
    style_runs: Vec<ShapeStyleRun>,
    style_runs_are_override: bool,
    snapshot: Option<LayoutSnapshot>,
    height_index: ViewHeightIndex,
    regional_cache: RegionalLayoutCache,
    regional_cache_limits: RegionalLayoutCacheLimits,
    last_error: Option<LayoutError>,
    active_layout_job: Option<LayoutJobId>,
    last_installed_layout_job: Option<LayoutJobId>,
}

/// Name used by the architecture specification. `ViewLayout` remains the short
/// facade name used by the coordinator.
pub type ViewLayoutState = ViewLayout;

impl ViewLayout {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_nonnegative(width),
            height: finite_nonnegative(height),
            viewport_left: 0.0,
            viewport_top: 0.0,
            insets: EdgeInsets::default(),
            wrap: true,
            linebreak: true,
            scale: 1.0,
            configuration_generation: ViewConfigurationGeneration(1),
            default_style: ResolvedTextStyle::default(),
            default_style_is_override: false,
            style_runs: Vec::new(),
            style_runs_are_override: false,
            snapshot: None,
            height_index: ViewHeightIndex::new_estimated(1, DEFAULT_ESTIMATED_HARD_LINE_HEIGHT)
                .expect("the built-in hard-line height estimate is valid"),
            regional_cache: RegionalLayoutCache::default(),
            regional_cache_limits: RegionalLayoutCacheLimits::default(),
            last_error: None,
            active_layout_job: None,
            last_installed_layout_job: None,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        let width = finite_nonnegative(width);
        let height = finite_nonnegative(height);
        if self.width != width || self.height != height {
            let width_changed = self.width != width;
            self.width = width;
            self.height = height;
            self.bump_configuration(width_changed);
        }
    }

    pub fn set_insets(&mut self, insets: EdgeInsets) {
        let insets = EdgeInsets {
            top: finite_nonnegative(insets.top),
            left: finite_nonnegative(insets.left),
            bottom: finite_nonnegative(insets.bottom),
            right: finite_nonnegative(insets.right),
        };
        if self.insets != insets {
            self.insets = insets;
            self.bump_configuration(true);
        }
    }

    pub fn set_wrap(&mut self, wrap: bool) {
        if wrap {
            self.viewport_left = 0.0;
        }
        if self.wrap != wrap {
            self.wrap = wrap;
            self.bump_configuration(true);
        }
    }

    pub fn set_linebreak(&mut self, linebreak: bool) {
        if self.linebreak != linebreak {
            self.linebreak = linebreak;
            self.bump_configuration(true);
        }
    }

    pub fn set_scale(&mut self, scale: f32) -> Result<(), LayoutError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(LayoutError::InvalidScale);
        }
        if self.scale != scale {
            self.scale = scale;
            self.bump_configuration(true);
        }
        Ok(())
    }

    pub fn set_default_style(&mut self, style: ResolvedTextStyle) -> Result<(), LayoutError> {
        if !style.is_valid() {
            return Err(LayoutError::InvalidStyle);
        }
        if !self.default_style_is_override || self.default_style != style {
            self.default_style = style;
            self.default_style_is_override = true;
            self.bump_configuration(true);
        }
        Ok(())
    }

    /// Resume using the formatted document's resolved default character style.
    pub fn clear_default_style_override(&mut self) {
        if self.default_style_is_override {
            self.default_style_is_override = false;
            self.bump_configuration(true);
        }
    }

    pub fn set_style_runs(&mut self, style_runs: Vec<ShapeStyleRun>) -> Result<(), LayoutError> {
        validate_style_runs_structure(&style_runs)?;
        if !self.style_runs_are_override || self.style_runs != style_runs {
            self.style_runs = style_runs;
            self.style_runs_are_override = true;
            self.bump_configuration(true);
        }
        Ok(())
    }

    /// Resume using style runs derived from the formatted projection.
    pub fn clear_style_runs_override(&mut self) {
        if self.style_runs_are_override {
            self.style_runs_are_override = false;
            self.bump_configuration(true);
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Horizontal presentation offset in document layout coordinates.
    /// Immutable row, caret, selection, and hit-test geometry never includes
    /// this offset. Frontends translate between viewport and document x by
    /// applying it at presentation time.
    pub fn viewport_left(&self) -> f32 {
        self.viewport_left
    }

    /// Set the horizontal presentation offset. Non-finite values are rejected
    /// without changing state, negative values clamp to zero, wrapping forces
    /// zero, and an exact current content width supplies the upper clamp.
    ///
    /// When only partial or stale geometry is installed there is no honest
    /// upper bound, so the non-negative request is retained until exact layout
    /// becomes available.
    pub fn set_viewport_left(&mut self, left: f32) -> Result<(), LayoutError> {
        if !left.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        self.viewport_left = self.clamp_viewport_left(left);
        Ok(())
    }

    /// Exact maximum horizontal offset for the current view dependencies.
    /// `None` means the installed geometry is partial or stale and must not be
    /// used to clamp presentation state prematurely. Wrapped views always
    /// return zero.
    pub fn maximum_viewport_left(&self) -> Option<f32> {
        if self.wrap {
            return Some(0.0);
        }
        let snapshot = self.snapshot.as_ref()?;
        if snapshot.configuration_generation != self.configuration_generation
            || !snapshot.content_width_is_exact
            || !self.height_index.total_height().is_exact()
        {
            return None;
        }
        Some((snapshot.content_width - self.width).max(0.0))
    }

    /// Vertical scroll position in the unpaginated layout coordinate space.
    /// Scrolling does not change the immutable layout snapshot identity.
    pub fn viewport_top(&self) -> f32 {
        self.viewport_top
    }

    pub fn set_viewport_top(&mut self, top: f32) -> Result<(), LayoutError> {
        if !top.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        self.viewport_top = self.clamp_viewport_top(top);
        Ok(())
    }

    pub fn usable_width(&self) -> f32 {
        (self.width - self.insets.left - self.insets.right).max(0.0)
    }

    pub fn wrap(&self) -> bool {
        self.wrap
    }

    pub fn linebreak(&self) -> bool {
        self.linebreak
    }

    /// View-local magnification applied during shaping and layout. Document
    /// style distances remain expressed in unscaled layout units; the scale is
    /// an independent presentation input and therefore part of the immutable
    /// layout configuration identity.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn configuration_generation(&self) -> ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn snapshot(&self) -> Option<&LayoutSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn last_error(&self) -> Option<&LayoutError> {
        self.last_error.as_ref()
    }

    /// Retain a synchronous presentation failure without disturbing the last
    /// immutable snapshot. Coordinator/model publication has already
    /// completed when this is used after an edit.
    pub(crate) fn record_error(&mut self, error: LayoutError) {
        self.last_error = Some(error);
    }

    /// The current exact or estimated height of the unpaginated content.
    /// Certainty is false whenever any hard line still needs layout for this
    /// view configuration.
    pub fn content_height(&self) -> HeightMeasurement {
        self.height_index.total_height()
    }

    /// Height of hard lines `[0, hard_line_end)` in the current view.
    pub fn hard_line_prefix_height(
        &self,
        hard_line_end: usize,
    ) -> Result<HeightMeasurement, ViewHeightIndexError> {
        self.height_index.prefix_height(hard_line_end)
    }

    /// Height and certainty of an ordered half-open hard-line range.
    pub fn hard_line_range_height(
        &self,
        range: Range<usize>,
    ) -> Result<HeightMeasurement, ViewHeightIndexError> {
        self.height_index.range_height(range)
    }

    /// Locate the hard line whose indexed vertical band contains `y`.
    pub fn hard_line_at_y(
        &self,
        y: f64,
    ) -> Result<Option<HardLineHeightHit>, ViewHeightIndexError> {
        self.height_index.hard_line_at_y(y)
    }

    /// Constant-time structural diagnostics, primarily useful for schedulers
    /// and tests that must verify the estimate remains compact.
    pub fn height_index_statistics(&self) -> ViewHeightIndexStatistics {
        self.height_index.statistics()
    }

    /// Compact summary of bounded hard-line results installed independently
    /// from the current full or partial visible snapshot.
    pub fn regional_cache_statistics(&self) -> RegionalLayoutCacheStatistics {
        let mut contiguous_region_count = 0usize;
        let mut previous = None;
        let mut visual_row_count = 0usize;
        for (hard_line, layout) in &self.regional_cache.lines {
            if previous.map_or(true, |value| *hard_line != value + 1) {
                contiguous_region_count += 1;
            }
            previous = Some(*hard_line);
            visual_row_count += layout.rows.len();
        }
        RegionalLayoutCacheStatistics {
            hard_line_count: self.regional_cache.lines.len(),
            contiguous_region_count,
            visual_row_count,
            estimated_bytes: self.regional_cache.estimated_bytes,
            eviction_count: self.regional_cache.eviction_count,
        }
    }

    pub fn regional_cache_limits(&self) -> RegionalLayoutCacheLimits {
        self.regional_cache_limits
    }

    /// Update regional-cache budgets and synchronously evict the oldest
    /// installed entries until all three bounds are satisfied. The immutable
    /// viewport snapshot and height index remain installed and exact.
    pub fn set_regional_cache_limits(&mut self, limits: RegionalLayoutCacheLimits) {
        self.regional_cache_limits = limits;
        self.regional_cache.enforce_limits(limits);
    }

    /// Inspect one exact cached hard-line layout without materializing the
    /// rest of the document.
    pub fn regional_hard_line_layout(&self, hard_line: usize) -> Option<&RegionalHardLineLayout> {
        self.regional_cache.lines.get(&hard_line).map(Arc::as_ref)
    }

    /// Ordered compact ranges currently represented by regional cache entries.
    pub fn regional_cached_ranges(&self) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for hard_line in self.regional_cache.lines.keys().copied() {
            if let Some(last) = ranges.last_mut() {
                if last.end == hard_line {
                    last.end += 1;
                    continue;
                }
            }
            ranges.push(hard_line..hard_line + 1);
        }
        ranges
    }

    /// Invalidate heights when the frontend's font resolver or measurement
    /// environment advances to a new generation. The installed immutable
    /// snapshot remains available while its replacement is computed, but its
    /// heights are no longer advertised as exact for the new environment.
    pub fn invalidate_text_metrics(&mut self) {
        self.invalidate_all_heights();
        self.regional_cache = RegionalLayoutCache::default();
    }

    /// Prepare the compact height index for a newly observed document shape
    /// before a regional request chooses its y-derived hard-line interval.
    /// Existing estimates are retained; exact entries are invalidated when
    /// they belong to an older document revision.
    pub(crate) fn synchronize_document_hard_line_count(
        &mut self,
        hard_line_count: usize,
        invalidate_exact: bool,
    ) -> Result<(), ViewHeightIndexError> {
        reconcile_height_index_count(&mut self.height_index, hard_line_count)?;
        if invalidate_exact {
            self.invalidate_all_heights();
            self.regional_cache = RegionalLayoutCache::default();
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn capture_for_layout_job(&self) -> LayoutJobViewConfiguration {
        let style_runs = if self.style_runs_are_override {
            self.style_runs.clone()
        } else {
            Vec::new()
        };
        self.capture_layout_job_with_style_runs(style_runs)
    }

    fn capture_layout_job_with_style_runs(
        &self,
        style_runs: Vec<ShapeStyleRun>,
    ) -> LayoutJobViewConfiguration {
        LayoutJobViewConfiguration {
            width: self.width,
            height: self.height,
            insets: self.insets,
            wrap: self.wrap,
            linebreak: self.linebreak,
            scale: self.scale,
            configuration_generation: self.configuration_generation,
            default_style: self.default_style.clone(),
            default_style_is_override: self.default_style_is_override,
            style_runs,
            style_runs_are_override: self.style_runs_are_override,
            latest_layout_revision: self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.revision)
                .into_iter()
                .chain(self.regional_cache.latest_revision)
                .max(),
        }
    }

    pub(crate) fn capture_for_regional_layout_job(
        &self,
        text: Range<usize>,
    ) -> LayoutJobViewConfiguration {
        let style_runs = if self.style_runs_are_override {
            self.style_runs
                .iter()
                .filter(|run| run.text_range.start < text.end && text.start < run.text_range.end)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.capture_layout_job_with_style_runs(style_runs)
    }

    pub(crate) fn begin_layout_job(&mut self, job_id: LayoutJobId) -> bool {
        let newest = self
            .active_layout_job
            .into_iter()
            .chain(self.last_installed_layout_job)
            .max();
        if newest.is_some_and(|current| job_id <= current) {
            return false;
        }
        self.active_layout_job = Some(job_id);
        true
    }

    pub(crate) fn active_layout_job(&self) -> Option<LayoutJobId> {
        self.active_layout_job
    }

    pub(crate) fn last_installed_layout_job(&self) -> Option<LayoutJobId> {
        self.last_installed_layout_job
    }

    pub(crate) fn publish_layout_job_region(
        &mut self,
        job_id: LayoutJobId,
        region: RegionalLayoutSnapshot,
    ) -> Result<LayoutRevision, ViewHeightIndexError> {
        let changed_range = region.hard_lines.clone();
        let prepared = self.prepare_layout_job_region(region)?;
        let previous_coverage_start = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.coverage.vertical_range())
            .map(|range| range.start);
        let next_snapshot = refresh_partial_snapshot_after_height_change(
            self.snapshot.as_ref(),
            &prepared.region,
            &prepared.height_index,
            changed_range,
            prepared.installed_revision,
        )?;
        let next_coverage_start = next_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.coverage.vertical_range())
            .map(|range| range.start);
        let adjusted_viewport_top = match (previous_coverage_start, next_coverage_start) {
            (Some(previous), Some(next)) => self.viewport_top + (next - previous),
            _ => self.viewport_top,
        };

        self.snapshot = next_snapshot;
        self.height_index = prepared.height_index;
        self.regional_cache = prepared.cache;
        self.viewport_top = self.clamp_viewport_top(adjusted_viewport_top);
        self.viewport_left = self.clamp_viewport_left(self.viewport_left);
        self.last_error = None;
        self.active_layout_job = None;
        self.last_installed_layout_job = Some(job_id);
        Ok(prepared.installed_revision)
    }

    pub(crate) fn publish_layout_job_viewport(
        &mut self,
        job_id: LayoutJobId,
        region: RegionalLayoutSnapshot,
        requested_viewport_top: f32,
    ) -> Result<LayoutRevision, ViewHeightIndexError> {
        let prepared = self.prepare_layout_job_region(region)?;
        let mut snapshot = partial_snapshot_from_region(&prepared.region, &prepared.height_index)?;
        if let Some(previous) = self.snapshot.as_ref().filter(|previous| {
            previous.content_width_is_exact
                && previous.document_id == snapshot.document_id
                && previous.document_revision == snapshot.document_revision
                && previous.configuration_generation == snapshot.configuration_generation
                && previous.measurement_environment_id == snapshot.measurement_environment_id
                && previous.metrics_generation == snapshot.metrics_generation
        }) {
            // A partial viewport does not invalidate an exact extent already
            // known for the same dependencies. Its own materialized width is a
            // lower bound and deterministic layout must not exceed that exact
            // prior result.
            if snapshot.content_width <= previous.content_width {
                snapshot.content_width = previous.content_width;
                snapshot.content_width_is_exact = true;
            }
        }

        self.snapshot = Some(snapshot);
        self.height_index = prepared.height_index;
        self.regional_cache = prepared.cache;
        self.viewport_top = self.clamp_viewport_top(requested_viewport_top);
        self.viewport_left = self.clamp_viewport_left(self.viewport_left);
        self.last_error = None;
        self.active_layout_job = None;
        self.last_installed_layout_job = Some(job_id);
        Ok(prepared.installed_revision)
    }

    fn prepare_layout_job_region(
        &self,
        mut region: RegionalLayoutSnapshot,
    ) -> Result<PreparedRegionalInstall, ViewHeightIndexError> {
        let newest_revision = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.revision)
            .into_iter()
            .chain(self.regional_cache.latest_revision)
            .max();
        let installed_revision = newest_revision.map_or(region.revision, |current| {
            if region.revision > current {
                region.revision
            } else {
                LayoutRevision(current.0.wrapping_add(1).max(1))
            }
        });
        region.rebind_revision(installed_revision);

        let identity = RegionalCacheIdentity {
            document_id: region.document_id,
            document_revision: region.document_revision,
            configuration_generation: region.configuration_generation,
            measurement_environment_id: region.measurement_environment_id,
            metrics_generation: region.metrics_generation,
            document_hard_line_count: region.document_hard_line_count,
        };
        let mut next_index = self.height_index.clone();
        let mut next_cache = self.regional_cache.clone();
        if next_cache.identity != Some(identity) {
            next_cache = RegionalLayoutCache::with_identity(identity);
            let installed_snapshot_matches = self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.document_id == identity.document_id
                    && snapshot.document_revision == identity.document_revision
                    && snapshot.configuration_generation == identity.configuration_generation
                    && snapshot.measurement_environment_id == identity.measurement_environment_id
                    && snapshot.metrics_generation == identity.metrics_generation
                    && snapshot_hard_line_count(snapshot) == identity.document_hard_line_count
            });
            if !installed_snapshot_matches {
                let old_count = next_index.hard_line_count();
                next_index.invalidate(0..old_count)?;
            }
            reconcile_height_index_count(&mut next_index, region.document_hard_line_count)?;
        }

        if region
            .lines
            .iter()
            .enumerate()
            .any(|(offset, line)| line.hard_line_index != region.hard_lines.start + offset)
        {
            return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                "regional hard-line records are not contiguous and ordered",
            ));
        }
        if region.lines.len() != region.hard_lines.end - region.hard_lines.start {
            return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                "regional result count does not match its hard-line range",
            ));
        }
        for line in &region.lines {
            if line.height_is_exact {
                next_index.set_exact_heights(line.hard_line_index, &[line.height])?;
            }
            if line.text_coverage == line.hard_line_range {
                next_cache.insert(line.clone());
            }
        }
        next_cache.enforce_limits(self.regional_cache_limits);
        next_cache.latest_revision = Some(installed_revision);
        Ok(PreparedRegionalInstall {
            installed_revision,
            region,
            height_index: next_index,
            cache: next_cache,
        })
    }

    fn bump_configuration(&mut self, invalidates_heights: bool) {
        self.configuration_generation.0 = self.configuration_generation.0.wrapping_add(1).max(1);
        self.regional_cache = RegionalLayoutCache::default();
        if invalidates_heights {
            self.invalidate_all_heights();
        }
    }

    fn invalidate_all_heights(&mut self) {
        let count = self.height_index.hard_line_count();
        self.height_index
            .invalidate(0..count)
            .expect("an index can always be reset to its validated estimate");
    }

    fn height_index_for_snapshot(
        &self,
        snapshot: &LayoutSnapshot,
    ) -> Result<ViewHeightIndex, ViewHeightIndexError> {
        let heights = snapshot_hard_line_heights(snapshot)?;
        let mut next = self.height_index.clone();
        reconcile_height_index_count(&mut next, heights.len())?;
        next.set_exact_heights(0, &heights)?;
        Ok(next)
    }

    fn clamp_viewport_top(&self, requested: f32) -> f32 {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return requested.max(0.0);
        };
        if let Some(coverage) = snapshot.coverage.vertical_range() {
            return requested.clamp(
                coverage.start,
                (coverage.end - self.height).max(coverage.start),
            );
        }
        let first_y = snapshot.rows.first().map_or(0.0, |row| row.y);
        requested.clamp(first_y, (snapshot.total_height - self.height).max(first_y))
    }

    fn clamp_viewport_left(&self, requested: f32) -> f32 {
        if self.wrap {
            return 0.0;
        }
        let requested = requested.max(0.0);
        self.maximum_viewport_left()
            .map_or(requested, |maximum| requested.min(maximum))
    }
}

impl LayoutSnapshot {
    fn rebind_revision(&mut self, revision: LayoutRevision) {
        self.revision = revision;
        for row in &mut self.rows {
            for caret in &mut row.carets {
                caret.point.layout_revision = revision;
            }
        }
    }
}

fn partial_snapshot_from_region(
    region: &RegionalLayoutSnapshot,
    height_index: &ViewHeightIndex,
) -> Result<LayoutSnapshot, ViewHeightIndexError> {
    let prefix = height_index.prefix_height(region.hard_lines.start)?;
    let mut rows = Vec::new();
    for line in &region.lines {
        let line_top =
            height_as_layout_unit(height_index.prefix_height(line.hard_line_index)?.height())?;
        for relative_row in &line.rows {
            let mut row = relative_row.clone();
            translate_row_vertically(&mut row, line_top)?;
            let row_index = rows.len();
            for caret in &mut row.carets {
                caret.row_index = row_index;
            }
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
            "a materialized viewport must contain visual rows",
        ));
    }

    let fully_materialized = region
        .lines
        .iter()
        .all(|line| line.height_is_exact && line.text_coverage == line.hard_line_range);
    let vertical_range = if fully_materialized {
        let materialized_height = height_index.range_height(region.hard_lines.clone())?;
        let vertical_end = prefix.height() + materialized_height.height();
        if !vertical_end.is_finite() {
            return Err(ViewHeightIndexError::HeightOverflow);
        }
        height_as_layout_unit(prefix.height())?..height_as_layout_unit(vertical_end)?
    } else {
        let start = rows.first().map(|row| row.y).ok_or(
            ViewHeightIndexError::InconsistentLayoutSnapshot(
                "a materialized viewport must contain visual rows",
            ),
        )?;
        let end = rows.last().map(|row| row.y + row.height()).ok_or(
            ViewHeightIndexError::InconsistentLayoutSnapshot(
                "a materialized viewport must contain visual rows",
            ),
        )?;
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                "partial long-line rows have invalid vertical coverage",
            ));
        }
        start..end
    };

    let content_width = content_width_from_rows(&rows, region.viewport_width);
    let content_width_is_exact = fully_materialized
        && region.hard_lines.start == 0
        && region.hard_lines.end == region.document_hard_line_count;
    let total = height_index.total_height();
    Ok(LayoutSnapshot {
        revision: region.revision,
        document_id: region.document_id,
        document_revision: region.document_revision,
        configuration_generation: region.configuration_generation,
        measurement_environment_id: region.measurement_environment_id,
        metrics_generation: region.metrics_generation,
        viewport_width: region.viewport_width,
        viewport_height: region.viewport_height,
        usable_width: region.usable_width,
        content_insets: region.content_insets,
        document_insets: region.document_insets,
        document_style_revision: region.document_style_revision,
        canvas_background: region.canvas_background,
        canvas_background_is_default: region.canvas_background_is_default,
        default_paint: region.default_paint.clone(),
        paint_runs: region.paint_runs.clone(),
        coverage: LayoutCoverage::PartialHardLines {
            hard_lines: region.hard_lines.clone(),
            document_hard_line_count: region.document_hard_line_count,
            text_ranges: region
                .lines
                .iter()
                .map(|line| line.text_coverage.clone())
                .collect(),
            vertical_range,
            prefix_is_exact: prefix.is_exact(),
        },
        rows,
        content_width,
        content_width_is_exact,
        total_height: height_as_layout_unit(total.height())?,
        total_height_is_exact: total.is_exact(),
        diagnostics: region.diagnostics.clone(),
        text_len: region.document_text_len,
        grapheme_boundaries: region.grapheme_boundaries.clone(),
    })
}

fn refresh_partial_snapshot_after_height_change(
    snapshot: Option<&LayoutSnapshot>,
    region: &RegionalLayoutSnapshot,
    height_index: &ViewHeightIndex,
    changed_hard_lines: Range<usize>,
    revision: LayoutRevision,
) -> Result<Option<LayoutSnapshot>, ViewHeightIndexError> {
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let LayoutCoverage::PartialHardLines {
        hard_lines,
        text_ranges,
        ..
    } = &snapshot.coverage
    else {
        return Ok(Some(snapshot.clone()));
    };
    if snapshot.document_id != region.document_id
        || snapshot.document_revision != region.document_revision
        || snapshot.configuration_generation != region.configuration_generation
        || snapshot.measurement_environment_id != region.measurement_environment_id
        || snapshot.metrics_generation != region.metrics_generation
        || snapshot.coverage.document_hard_line_count() != region.document_hard_line_count
    {
        return Ok(None);
    }
    if hard_lines.start < changed_hard_lines.end && changed_hard_lines.start < hard_lines.end {
        // The regional result may contain different wrapping from the visible
        // snapshot. Removing it is preferable to publishing internally
        // inconsistent rows; a viewport request will rematerialize it.
        return Ok(None);
    }
    if snapshot
        .rows
        .first()
        .is_some_and(|row| text_ranges.first() != Some(&row.hard_line_range))
    {
        // A long-line continuation snapshot starts inside its hard-line band.
        // Without retaining the old estimated prefix separately, an unrelated
        // height refinement cannot shift it exactly; discard it and request a
        // fresh revision-bound viewport slice instead.
        return Ok(None);
    }

    let prefix = height_index.prefix_height(hard_lines.start)?;
    let new_start = height_as_layout_unit(prefix.height())?;
    let old_range = snapshot
        .coverage
        .vertical_range()
        .expect("partial coverage always has a vertical range");
    let old_start = old_range.start;
    let delta = new_start - old_start;
    let new_end = checked_layout_sum(old_range.end, delta)?;

    let mut refreshed = snapshot.clone();
    for row in &mut refreshed.rows {
        translate_row_vertically(row, delta)?;
    }
    let LayoutCoverage::PartialHardLines {
        vertical_range,
        prefix_is_exact,
        ..
    } = &mut refreshed.coverage
    else {
        unreachable!("a cloned partial snapshot remains partial")
    };
    *vertical_range = new_start..new_end;
    *prefix_is_exact = prefix.is_exact();
    let total = height_index.total_height();
    refreshed.total_height = height_as_layout_unit(total.height())?;
    refreshed.total_height_is_exact = total.is_exact();
    refreshed.rebind_revision(revision);
    Ok(Some(refreshed))
}

fn height_as_layout_unit(height: f64) -> Result<f32, ViewHeightIndexError> {
    if !height.is_finite() || height < 0.0 || height > f64::from(f32::MAX) {
        return Err(ViewHeightIndexError::HeightOverflow);
    }
    Ok(height as f32)
}

fn checked_layout_sum(left: f32, right: f32) -> Result<f32, ViewHeightIndexError> {
    let sum = left + right;
    if !sum.is_finite() || sum < 0.0 {
        Err(ViewHeightIndexError::HeightOverflow)
    } else {
        Ok(sum)
    }
}

/// Move every absolute vertical coordinate owned by a positioned row while
/// preserving cluster-local geometry. Regional layout initially positions
/// rows relative to their hard-line band; globalization and later height-index
/// refinements must therefore translate provider bounds along with the row and
/// baseline used to draw them.
fn translate_row_vertically(row: &mut VisualRow, delta: f32) -> Result<(), ViewHeightIndexError> {
    row.y = checked_layout_sum(row.y, delta)?;
    row.baseline = checked_layout_sum(row.baseline, delta)?;
    for cluster in &mut row.clusters {
        cluster.typographic_bounds.y =
            checked_layout_coordinate_sum(cluster.typographic_bounds.y, delta)?;
        cluster.ink_bounds.y = checked_layout_coordinate_sum(cluster.ink_bounds.y, delta)?;
    }
    Ok(())
}

fn checked_layout_coordinate_sum(left: f32, right: f32) -> Result<f32, ViewHeightIndexError> {
    let sum = left + right;
    if sum.is_finite() {
        Ok(sum)
    } else {
        Err(ViewHeightIndexError::HeightOverflow)
    }
}

/// Measure the rightward document-coordinate extent represented by positioned
/// rows. The viewport width is the minimum canvas extent, so ordinary content
/// that fits produces no horizontal scroll range. Provider ink overhangs are
/// included to keep the complete drawn result reachable.
fn content_width_from_rows(rows: &[VisualRow], viewport_width: f32) -> f32 {
    rows.iter().fold(viewport_width, |content_width, row| {
        let positioned_right = row
            .clusters
            .iter()
            .flat_map(|cluster| {
                [
                    cluster.x + cluster.advance,
                    cluster.typographic_bounds.x + cluster.typographic_bounds.width,
                    cluster.ink_bounds.x + cluster.ink_bounds.width,
                ]
            })
            .chain(row.carets.iter().map(|caret| caret.x))
            .fold(row.paragraph_content_x, f32::max);
        let paragraph_box_right = row.paragraph_content_x + row.paragraph_content_width;
        let trailing_canvas = (viewport_width - paragraph_box_right).max(0.0);
        content_width.max(positioned_right + trailing_canvas)
    })
}

fn snapshot_hard_line_count(snapshot: &LayoutSnapshot) -> usize {
    snapshot.coverage.document_hard_line_count()
}

fn reconcile_height_index_count(
    index: &mut ViewHeightIndex,
    new_count: usize,
) -> Result<(), ViewHeightIndexError> {
    let old_count = index.hard_line_count();
    match old_count.cmp(&new_count) {
        Ordering::Greater => index.splice(new_count..old_count, 0),
        Ordering::Less => index.splice(old_count..old_count, new_count - old_count),
        Ordering::Equal => Ok(()),
    }
}

/// Convert full-document row geometry into exact hard-line bands. The first
/// band owns the document's top contribution, gaps between hard lines belong
/// to the preceding band, and the last band owns the bottom contribution.
/// Consequently prefixes line up with the first visual row of every hard line
/// and all bands sum to `LayoutSnapshot::total_height`.
fn snapshot_hard_line_heights(snapshot: &LayoutSnapshot) -> Result<Vec<f64>, ViewHeightIndexError> {
    if !snapshot.coverage.is_full_document() {
        return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
            "partial snapshots cannot replace a complete height index",
        ));
    }
    if !snapshot.total_height_is_exact {
        return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
            "a partial layout cannot publish every hard-line height as exact",
        ));
    }
    let total_height = f64::from(snapshot.total_height);
    if !total_height.is_finite() || total_height <= 0.0 {
        return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
            "total layout height must be finite and positive",
        ));
    }

    let mut starts = Vec::new();
    let mut current_hard_line = None;
    for row in &snapshot.rows {
        if !row.y.is_finite() || row.y < 0.0 || !row.height().is_finite() || row.height() <= 0.0 {
            return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                "visual-row geometry must be finite, non-negative, and advancing",
            ));
        }
        match current_hard_line {
            None => {
                if row.hard_line_index != 0 {
                    return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                        "hard-line row groups must begin at zero",
                    ));
                }
                starts.push(f64::from(row.y));
                current_hard_line = Some(0usize);
            }
            Some(current) if row.hard_line_index == current => {}
            Some(current) if row.hard_line_index == current.saturating_add(1) => {
                starts.push(f64::from(row.y));
                current_hard_line = Some(row.hard_line_index);
            }
            Some(_) => {
                return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                    "hard-line row groups must be contiguous and ordered",
                ));
            }
        }
    }
    if starts.is_empty() {
        return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
            "a formatted document must have at least one hard-line row",
        ));
    }

    let mut heights = Vec::with_capacity(starts.len());
    for hard_line in 0..starts.len() {
        let band_start = if hard_line == 0 {
            0.0
        } else {
            starts[hard_line]
        };
        let band_end = starts.get(hard_line + 1).copied().unwrap_or(total_height);
        let height = band_end - band_start;
        if !height.is_finite() || height <= 0.0 {
            return Err(ViewHeightIndexError::InconsistentLayoutSnapshot(
                "hard-line vertical bands must be finite, ordered, and non-empty",
            ));
        }
        heights.push(height);
    }
    Ok(heights)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    Measurement(MeasurementError),
    DocumentStyle(DocumentStyleError),
    HeightIndex(ViewHeightIndexError),
    InvalidScale,
    InvalidStyle,
    InvalidStyleRun {
        index: usize,
        reason: &'static str,
    },
    InvalidGeometry,
    OutsideMaterializedCoverage,
    NoRows,
    NoCaretStops,
    InvalidTextOffset(usize),
    InvalidGraphemeBoundary {
        text_offset: usize,
    },
    LongLineSliceNeedsMoreText {
        text_offset: usize,
    },
    UnstableLineBreakContext {
        text_offset: usize,
    },
    NotACaretStop {
        text_offset: usize,
    },
    WrongDocumentRevision,
    WrongDocument,
    StaleLayout {
        expected: LayoutRevision,
        actual: LayoutRevision,
    },
    StaleMeasurementResponse,
    MeasurementEnvironmentChangedDuringShape,
    MetricsChangedDuringShape,
    MalformedMeasurement(&'static str),
}

impl From<MeasurementError> for LayoutError {
    fn from(value: MeasurementError) -> Self {
        Self::Measurement(value)
    }
}

impl From<DocumentStyleError> for LayoutError {
    fn from(value: DocumentStyleError) -> Self {
        Self::DocumentStyle(value)
    }
}

impl From<ViewHeightIndexError> for LayoutError {
    fn from(value: ViewHeightIndexError) -> Self {
        Self::HeightIndex(value)
    }
}

#[derive(Clone)]
struct RelativeCluster {
    text_range: Range<usize>,
    advance: f32,
    metrics: TextMetrics,
    typographic_bounds: ShapedBounds,
    ink_bounds: ShapedBounds,
    bidi_level: u8,
    fallback_font: String,
    caret_stops: Vec<ClusterCaretStop>,
    render_run: Option<RenderRunHandle>,
}

#[derive(Clone)]
struct RelativeDiagnostic {
    text_range: Range<usize>,
    message: String,
}

#[derive(Clone)]
struct RelativeFragment {
    /// Length of the stable ownership interior. Cluster ranges are relative to
    /// its start and may extend past this length into following context.
    ownership_len: usize,
    clusters: Vec<RelativeCluster>,
    visual_order: Vec<usize>,
    default_metrics: TextMetrics,
    diagnostics: Vec<RelativeDiagnostic>,
}

#[derive(Clone)]
struct ShapeCacheEntry {
    text: String,
    context_before: String,
    context_after: String,
    style_runs: Vec<ShapeStyleRun>,
    default_style: ResolvedTextStyle,
    paragraph_base_direction: TextDirection,
    scale_bits: u32,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
    purpose: ShapePurpose,
    render_run_policy: Option<RenderRunPolicy>,
    fragment: RelativeFragment,
}

struct PendingShape<'a> {
    output_index: usize,
    text_range: Range<usize>,
    text: &'a str,
    context_before: &'a str,
    context_after: &'a str,
    global_style_runs: Vec<ShapeStyleRun>,
    relative_style_runs: Vec<ShapeStyleRun>,
    default_style: ResolvedTextStyle,
    paragraph_base_direction: TextDirection,
}

#[derive(Clone)]
struct LineParagraphLayout {
    paragraph_index: Option<usize>,
    paragraph_id: Option<u64>,
    is_first_hard_line: bool,
    style: ParagraphLayoutStyle,
}

#[derive(Clone, Debug)]
pub(super) struct HardLineLayoutSlice {
    pub full_range: Range<usize>,
    pub work_range: Range<usize>,
    pub shaping_context_range: Range<usize>,
    pub hard_line_index: usize,
    pub checkpoint: Option<LongLineLayoutCheckpoint>,
}

#[derive(Clone, Copy)]
struct ParagraphRowBox {
    x: f32,
    width: f32,
}

struct NeverCancelled;

impl LayoutCancellationProbe for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct LayoutRunControl<'a> {
    cancellation: &'a dyn LayoutCancellationProbe,
    max_shape_batch_fragments: usize,
}

impl<'a> LayoutRunControl<'a> {
    fn synchronous(cancellation: &'a dyn LayoutCancellationProbe) -> Self {
        Self {
            cancellation,
            max_shape_batch_fragments: usize::MAX,
        }
    }

    fn cancellable(cancellation: &'a dyn LayoutCancellationProbe) -> Self {
        Self {
            cancellation,
            max_shape_batch_fragments: MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS,
        }
    }

    #[inline]
    fn checkpoint(&self) -> Result<(), LayoutComputationError> {
        if self.cancellation.is_cancelled() {
            Err(LayoutComputationError::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub struct LayoutEngine<P: TextMeasurementProvider> {
    provider: P,
    next_layout_revision: u64,
    shape_cache: VecDeque<ShapeCacheEntry>,
    cache_capacity: usize,
}

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            next_layout_revision: 1,
            shape_cache: VecDeque::new(),
            cache_capacity: DEFAULT_CACHE_ENTRIES,
        }
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    pub fn clear_caches(&mut self) {
        self.shape_cache.clear();
    }

    pub fn set_cache_capacity(&mut self, capacity: usize) {
        self.cache_capacity = capacity;
        while self.shape_cache.len() > capacity {
            self.shape_cache.pop_front();
        }
    }

    /// Coordinator convenience entry point. Errors remain observable on the
    /// view while its previous valid immutable snapshot stays installed.
    pub fn layout(&mut self, document: &Document, view: &mut ViewLayout) {
        if let Err(error) = self.relayout(document, view) {
            view.last_error = Some(error);
        }
    }

    pub fn relayout(
        &mut self,
        document: &Document,
        view: &mut ViewLayout,
    ) -> Result<(), LayoutError> {
        let document_styles = DocumentLayoutStyles::resolve(document.projection())?;
        let text = document.text();
        let hard_lines = document
            .projection()
            .hard_lines_for_region(&(0..text.len()));
        self.relayout_text_with_document_styles_and_hard_lines(
            document.id(),
            document.revision(),
            text,
            &hard_lines,
            view,
            Some(document_styles),
        )
    }

    /// Snapshot-oriented entry point useful to background jobs and unit tests.
    /// Lacking an owning projection, this compatibility API interprets each
    /// U+000A in `text` as an explicit hard-line boundary. Document-backed
    /// layout uses the projection's authoritative semantic hard-line index.
    pub fn relayout_text(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        view: &mut ViewLayout,
    ) -> Result<(), LayoutError> {
        self.relayout_text_with_document_styles(document_id, document_revision, text, view, None)
    }

    /// Snapshot-oriented entry point for a normalized projection whose style
    /// sheet was resolved independently (for example by a background pipeline
    /// job). Lacking the owning projection's semantic index, this compatibility
    /// API treats U+000A as a hard-line boundary. Geometry-only differences do
    /// not enter the shaping cache key.
    pub fn relayout_styled_text(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        document_styles: DocumentLayoutStyles,
        view: &mut ViewLayout,
    ) -> Result<(), LayoutError> {
        self.relayout_text_with_document_styles(
            document_id,
            document_revision,
            text,
            view,
            Some(document_styles),
        )
    }

    /// Build and install one immutable snapshot using cooperative cancellation.
    /// The view is changed only after all shaping and positioning succeeds and
    /// the final cancellation checkpoint passes. Completed shaping fragments
    /// may remain in the engine cache so a later revision-compatible request
    /// can reuse them.
    pub fn relayout_styled_text_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        document_styles: DocumentLayoutStyles,
        view: &mut ViewLayout,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<(), LayoutComputationError> {
        let control = LayoutRunControl::cancellable(cancellation);
        let hard_lines = hard_line_ranges_cancellable(text, &control)?;
        self.relayout_text_with_document_styles_controlled(
            document_id,
            document_revision,
            text,
            &hard_lines,
            view,
            Some(document_styles),
            &control,
        )
    }

    /// Compute exact layout for a bounded, contiguous hard-line region. The
    /// supplied text contains only those hard lines (and their intervening
    /// U+000A separators), while every externally visible text coordinate
    /// remains relative to the complete formatted document.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_hard_line_region_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        region_text: &str,
        text_origin: usize,
        line_ranges: &[Range<usize>],
        first_hard_line: usize,
        document_hard_line_count: usize,
        document_text_len: usize,
        following_line_range: Option<Range<usize>>,
        document_styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        let line_slices = line_ranges
            .iter()
            .enumerate()
            .map(|(offset, range)| HardLineLayoutSlice {
                full_range: range.clone(),
                work_range: range.clone(),
                shaping_context_range: range.clone(),
                hard_line_index: first_hard_line + offset,
                checkpoint: None,
            })
            .collect::<Vec<_>>();
        self.layout_hard_line_slices_cancellable(
            document_id,
            document_revision,
            region_text,
            text_origin,
            &line_slices,
            document_hard_line_count,
            document_text_len,
            following_line_range,
            document_styles,
            view,
            cancellation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn layout_hard_line_slices_cancellable(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        region_text: &str,
        text_origin: usize,
        line_slices: &[HardLineLayoutSlice],
        document_hard_line_count: usize,
        document_text_len: usize,
        following_line_range: Option<Range<usize>>,
        document_styles: &DocumentLayoutStyles,
        view: &LayoutJobViewConfiguration,
        cancellation: &dyn LayoutCancellationProbe,
    ) -> Result<RegionalLayoutSnapshot, LayoutComputationError> {
        let control = LayoutRunControl::cancellable(cancellation);
        control.checkpoint()?;
        let Some(first_hard_line) = line_slices.first().map(|line| line.hard_line_index) else {
            return Err(LayoutError::MalformedMeasurement(
                "regional hard-line bounds are inconsistent",
            )
            .into());
        };
        if first_hard_line
            .checked_add(line_slices.len())
            .map_or(true, |end| end > document_hard_line_count)
            || line_slices
                .iter()
                .enumerate()
                .any(|(offset, line)| line.hard_line_index != first_hard_line + offset)
        {
            return Err(LayoutError::MalformedMeasurement(
                "regional hard-line bounds are inconsistent",
            )
            .into());
        }
        let regional_end = text_origin
            .checked_add(region_text.len())
            .ok_or(LayoutError::InvalidTextOffset(text_origin))?;
        if regional_end > document_text_len {
            return Err(LayoutError::InvalidTextOffset(regional_end).into());
        }
        for line in line_slices {
            control.checkpoint()?;
            let valid_range = |range: &Range<usize>| {
                range.start >= text_origin
                    && range.end <= regional_end
                    && range.start <= range.end
                    && region_text.is_char_boundary(range.start - text_origin)
                    && region_text.is_char_boundary(range.end - text_origin)
            };
            if line.full_range.start > line.work_range.start
                || line.work_range.end > line.full_range.end
                || line.work_range.start > line.work_range.end
                || line.shaping_context_range.start > line.work_range.start
                || line.work_range.end > line.shaping_context_range.end
                || !valid_range(&line.work_range)
                || !valid_range(&line.shaping_context_range)
            {
                return Err(LayoutError::MalformedMeasurement(
                    "regional hard-line slice is invalid",
                )
                .into());
            }
            if let Some(checkpoint) = &line.checkpoint {
                if checkpoint.document_id != document_id
                    || checkpoint.document_revision != document_revision
                    || checkpoint.configuration_generation != view.configuration_generation
                    || checkpoint.measurement_environment_id
                        != self.provider.measurement_environment_id()
                    || checkpoint.metrics_generation != self.provider.metrics_generation()
                    || checkpoint.hard_line_index != line.hard_line_index
                    || checkpoint.hard_line_range != line.full_range
                    || checkpoint.next_text_offset != line.work_range.start
                    || !checkpoint.completed_height.is_finite()
                    || checkpoint.completed_height < 0.0
                    || !checkpoint.cumulative_advance.is_finite()
                    || checkpoint.cumulative_advance < 0.0
                {
                    return Err(LayoutError::MalformedMeasurement(
                        "long-line checkpoint does not match captured dependencies",
                    )
                    .into());
                }
            }
        }
        let requested_end = first_hard_line + line_slices.len();
        let reaches_last_line_end = line_slices
            .last()
            .is_some_and(|line| line.work_range.end == line.full_range.end);
        if ((requested_end < document_hard_line_count) && reaches_last_line_end)
            != following_line_range.is_some()
        {
            return Err(LayoutError::MalformedMeasurement(
                "regional layout requires exactly one following-line style context",
            )
            .into());
        }

        let default_style = if view.default_style_is_override {
            view.default_style.clone()
        } else {
            document_styles.default_shaping_style.clone()
        };
        if !default_style.is_valid() {
            return Err(LayoutError::InvalidStyle.into());
        }
        let style_runs = if view.style_runs_are_override {
            view.style_runs.as_slice()
        } else {
            document_styles.shaping_runs.as_slice()
        };
        validate_style_runs_structure(style_runs)?;

        let document_insets = document_styles.document_insets;
        let content_insets = add_insets(view.insets, document_insets);
        let usable_width = usable_width(view.width, content_insets);
        let paragraph_styles = &document_styles.paragraphs;
        let mut line_paragraphs = Vec::with_capacity(line_slices.len());
        let mut local_line_ranges = Vec::with_capacity(line_slices.len());
        let mut local_context_ranges = Vec::with_capacity(line_slices.len());
        for line in line_slices {
            control.checkpoint()?;
            line_paragraphs.push(resolve_line_paragraph(
                &line.full_range,
                paragraph_styles,
                &default_style,
            ));
            local_line_ranges
                .push((line.work_range.start - text_origin)..(line.work_range.end - text_origin));
            local_context_ranges.push(
                (line.shaping_context_range.start - text_origin)
                    ..(line.shaping_context_range.end - text_origin),
            );
        }
        control.checkpoint()?;
        let following_paragraph = following_line_range
            .as_ref()
            .map(|line| resolve_line_paragraph(line, paragraph_styles, &default_style));

        let fragment_ranges: Vec<Vec<Range<usize>>> = local_line_ranges
            .iter()
            .map(|line| fragment_range_at_graphemes(region_text, line.clone(), &control))
            .collect::<Result<_, _>>()?;
        let mut flat_fragment_ranges = Vec::new();
        let mut fragment_default_styles = Vec::new();
        let mut fragment_base_directions = Vec::new();
        let mut fragment_line_bounds = Vec::new();
        for ((ranges, paragraph), line) in fragment_ranges
            .iter()
            .zip(&line_paragraphs)
            .zip(&local_context_ranges)
        {
            let style = if view.default_style_is_override {
                &default_style
            } else {
                &paragraph.style.default_shaping_style
            };
            let direction = shaping_base_direction(paragraph.style.base_direction);
            for (index, range) in ranges.iter().enumerate() {
                if index % MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS == 0 {
                    control.checkpoint()?;
                }
                flat_fragment_ranges.push(range.clone());
                fragment_default_styles.push(style.clone());
                fragment_base_directions.push(direction);
                fragment_line_bounds.push(line.clone());
            }
        }

        let mut work_statistics = LayoutWorkStatistics {
            segmented_text_bytes: line_slices.iter().map(|line| line.work_range.len()).sum(),
            shaping_fragment_count: flat_fragment_ranges.len(),
            maximum_shaping_fragment_bytes: flat_fragment_ranges
                .iter()
                .map(Range::len)
                .max()
                .unwrap_or(0),
            ..LayoutWorkStatistics::default()
        };

        let measurement_environment_id = self.provider.measurement_environment_id();
        let metrics_generation = self.provider.metrics_generation();
        let shaped = self.shape_ranges_with_origin(
            document_id,
            document_revision,
            region_text,
            &flat_fragment_ranges,
            &fragment_line_bounds,
            text_origin,
            style_runs,
            &fragment_default_styles,
            &fragment_base_directions,
            view.scale,
            measurement_environment_id,
            metrics_generation,
            &control,
        )?;
        control.checkpoint()?;
        if self.provider.measurement_environment_id() != measurement_environment_id {
            return Err(LayoutError::MeasurementEnvironmentChangedDuringShape.into());
        }
        if self.provider.metrics_generation() != metrics_generation {
            return Err(LayoutError::MetricsChangedDuringShape.into());
        }

        let layout_revision = LayoutRevision(view.latest_layout_revision.map_or(
            self.next_layout_revision,
            |revision| {
                self.next_layout_revision
                    .max(revision.0.wrapping_add(1).max(1))
            },
        ));
        let mut diagnostics: Vec<_> = shaped
            .iter()
            .flat_map(|fragment| fragment.diagnostics.iter().cloned())
            .collect();
        let mut fragment_cursor = 0usize;
        let mut lines = Vec::with_capacity(line_slices.len());

        for (line_offset, line_slice) in line_slices.iter().enumerate() {
            control.checkpoint()?;
            let hard_line_index = line_slice.hard_line_index;
            let line_range = &line_slice.work_range;
            let paragraph = &line_paragraphs[line_offset];
            let count = fragment_ranges[line_offset].len();
            let line_fragments = &shaped[fragment_cursor..fragment_cursor + count];
            fragment_cursor += count;
            let extends_past_work = line_slice.work_range.end < line_slice.full_range.end;
            let (clusters, empty_metrics) = flatten_line_fragments_for_slice(
                line_range,
                line_fragments,
                extends_past_work,
                &control,
            )?;
            let right_to_left = line_slice.checkpoint.as_ref().map_or_else(
                || paragraph_is_right_to_left(&paragraph.style, &clusters),
                |checkpoint| checkpoint.right_to_left,
            );
            let (paragraph_first_box, continuation_box) = paragraph_row_boxes(
                content_insets.left,
                usable_width,
                &paragraph.style,
                right_to_left,
            );
            let first_row_box = if line_slice.checkpoint.is_none() && paragraph.is_first_hard_line {
                paragraph_first_box
            } else {
                continuation_box
            };
            let breaks = if view.wrap && view.linebreak {
                unicode_line_break_opportunities_for_slice(
                    &region_text[local_context_ranges[line_offset].clone()],
                    line_slice.shaping_context_range.start,
                    line_slice.work_range.start,
                    line_slice.shaping_context_range.start == line_slice.full_range.start,
                    &control,
                )?
            } else {
                BTreeSet::new()
            };
            let mut row_cluster_ranges = wrap_cluster_ranges(
                &clusters,
                first_row_box.width,
                continuation_box.width,
                view.wrap,
                view.linebreak,
                &breaks,
                &control,
            )?;
            work_statistics.wrapped_cluster_count = work_statistics
                .wrapped_cluster_count
                .saturating_add(clusters.len());
            work_statistics.maximum_wrap_checkpoint_clusters = work_statistics
                .maximum_wrap_checkpoint_clusters
                .max(clusters.len().min(CANCELLATION_CLUSTER_BATCH));
            if extends_past_work {
                row_cluster_ranges.pop();
                if row_cluster_ranges.is_empty() {
                    return Err(LayoutError::LongLineSliceNeedsMoreText {
                        text_offset: line_slice.work_range.start,
                    }
                    .into());
                }
            }
            let mut rows = Vec::new();
            let starting_row = line_slice
                .checkpoint
                .as_ref()
                .map_or(0, |checkpoint| checkpoint.completed_visual_rows);
            let mut y = line_slice.checkpoint.as_ref().map_or_else(
                || {
                    if hard_line_index == 0 {
                        content_insets.top + paragraph.style.spacing_before
                    } else {
                        0.0
                    }
                },
                |checkpoint| checkpoint.completed_height,
            );

            if clusters.is_empty() {
                let line_advance =
                    line_advance(empty_metrics.height(), paragraph.style.line_spacing);
                let x = aligned_row_x(first_row_box, 0.0, paragraph.style.alignment, right_to_left);
                let point_downstream = CaretPoint {
                    document_id,
                    document_revision,
                    layout_revision,
                    text_offset: line_range.start,
                    affinity: BoundaryAffinity::Downstream,
                };
                let point_upstream = CaretPoint {
                    affinity: BoundaryAffinity::Upstream,
                    ..point_downstream
                };
                rows.push(VisualRow {
                    paragraph_id: paragraph.paragraph_id,
                    hard_line_index,
                    fragment_index: starting_row,
                    hard_line_range: line_slice.full_range.clone(),
                    text_range: line_range.clone(),
                    y,
                    baseline: y + empty_metrics.ascent,
                    ascent: empty_metrics.ascent,
                    descent: empty_metrics.descent,
                    leading: empty_metrics.leading,
                    line_advance,
                    width: 0.0,
                    paragraph_content_x: first_row_box.x,
                    paragraph_content_width: first_row_box.width,
                    wrapped_from_previous: starting_row > 0,
                    wraps_to_next: false,
                    clusters: Vec::new(),
                    carets: vec![
                        PositionedCaret {
                            point: point_downstream,
                            x,
                            row_index: 0,
                        },
                        PositionedCaret {
                            point: point_upstream,
                            x,
                            row_index: 0,
                        },
                    ],
                });
                y += line_advance;
            } else {
                for (relative_row, cluster_range) in row_cluster_ranges.iter().enumerate() {
                    control.checkpoint()?;
                    let row_in_line = starting_row + relative_row;
                    let rows_in_line =
                        starting_row + row_cluster_ranges.len() + usize::from(extends_past_work);
                    let row = position_row(
                        &clusters[cluster_range.clone()],
                        paragraph.paragraph_id,
                        if relative_row == 0 {
                            first_row_box
                        } else {
                            continuation_box
                        },
                        paragraph.style.alignment,
                        right_to_left,
                        paragraph.style.line_spacing,
                        hard_line_index,
                        line_slice.full_range.clone(),
                        relative_row,
                        row_in_line,
                        rows_in_line,
                        y,
                        document_id,
                        document_revision,
                        layout_revision,
                        &control,
                    )?;
                    y += row.height();
                    work_statistics.positioned_cluster_count = work_statistics
                        .positioned_cluster_count
                        .saturating_add(cluster_range.len());
                    work_statistics.maximum_position_checkpoint_clusters = work_statistics
                        .maximum_position_checkpoint_clusters
                        .max(cluster_range.len().min(CANCELLATION_CLUSTER_BATCH));
                    rows.push(row);
                }
            }

            if !extends_past_work {
                let next_paragraph = line_paragraphs
                    .get(line_offset + 1)
                    .or(following_paragraph.as_ref());
                if let Some(next) = next_paragraph {
                    if starts_new_paragraph(paragraph, next) {
                        y += paragraph.style.spacing_after + next.style.spacing_before;
                    }
                } else {
                    y += paragraph.style.spacing_after + content_insets.bottom;
                }
            }
            let height = f64::from(y);
            if !height.is_finite() || height <= 0.0 {
                return Err(LayoutError::HeightIndex(
                    ViewHeightIndexError::InconsistentLayoutSnapshot(
                        "regional hard-line height must be finite and positive",
                    ),
                )
                .into());
            }
            let text_coverage_end = rows.last().map_or(line_range.end, |row| row.text_range.end);
            let consumed_advance = row_cluster_ranges
                .iter()
                .flat_map(|range| &clusters[range.clone()])
                .map(|cluster| f64::from(cluster.advance))
                .sum::<f64>();
            let next_checkpoint = extends_past_work.then(|| LongLineLayoutCheckpoint {
                document_id,
                document_revision,
                configuration_generation: view.configuration_generation,
                measurement_environment_id,
                metrics_generation,
                hard_line_index,
                hard_line_range: line_slice.full_range.clone(),
                next_text_offset: text_coverage_end,
                completed_visual_rows: starting_row + rows.len(),
                completed_height: y,
                cumulative_advance: line_slice
                    .checkpoint
                    .as_ref()
                    .map_or(0.0, |checkpoint| checkpoint.cumulative_advance)
                    + consumed_advance,
                last_candidate_break: breaks
                    .contains(&text_coverage_end)
                    .then_some(text_coverage_end),
                continuation_width: continuation_box.width,
                right_to_left,
            });
            lines.push(RegionalHardLineLayout {
                layout_revision,
                hard_line_index,
                hard_line_range: line_slice.full_range.clone(),
                text_coverage: line_range.start..text_coverage_end,
                rows,
                height,
                height_is_exact: !extends_past_work,
                next_checkpoint,
            });
        }

        let paint_start = lines.first().expect("nonempty range").text_coverage.start;
        let paint_end = lines.last().expect("nonempty range").text_coverage.end;
        let paint_runs = document_styles
            .paint_runs
            .iter()
            .filter(|run| run.text_range.start < paint_end && paint_start < run.text_range.end)
            .cloned()
            .collect();
        let hard_lines = first_hard_line..requested_end;
        let coverage_ranges = lines
            .iter()
            .map(|line| line.text_coverage.clone())
            .collect::<Vec<_>>();
        let grapheme_boundaries = boundaries_in_ordered_ranges(
            logical_grapheme_boundaries(region_text, text_origin, &control)?,
            &coverage_ranges,
            &control,
        )?;
        diagnostics.retain(|diagnostic| {
            coverage_ranges.iter().any(|range| {
                diagnostic.text_range.start < range.end && range.start < diagnostic.text_range.end
            })
        });
        let result = RegionalLayoutSnapshot {
            revision: layout_revision,
            document_id,
            document_revision,
            configuration_generation: view.configuration_generation,
            measurement_environment_id,
            metrics_generation,
            hard_lines,
            document_hard_line_count,
            document_text_len,
            viewport_width: view.width,
            viewport_height: view.height,
            usable_width,
            content_insets,
            document_insets,
            document_style_revision: Some(document_styles.style_sheet_revision),
            canvas_background: document_styles.canvas_background,
            canvas_background_is_default: document_styles.canvas_background_is_default,
            default_paint: document_styles.default_paint.clone(),
            paint_runs,
            lines,
            diagnostics,
            grapheme_boundaries,
            work_statistics,
        };
        control.checkpoint()?;
        self.next_layout_revision = layout_revision.0.wrapping_add(1).max(1);
        Ok(result)
    }

    fn relayout_text_with_document_styles(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        view: &mut ViewLayout,
        document_styles: Option<DocumentLayoutStyles>,
    ) -> Result<(), LayoutError> {
        let hard_lines = hard_line_ranges(text);
        self.relayout_text_with_document_styles_and_hard_lines(
            document_id,
            document_revision,
            text,
            &hard_lines,
            view,
            document_styles,
        )
    }

    fn relayout_text_with_document_styles_and_hard_lines(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        hard_lines: &[Range<usize>],
        view: &mut ViewLayout,
        document_styles: Option<DocumentLayoutStyles>,
    ) -> Result<(), LayoutError> {
        let cancellation = NeverCancelled;
        let control = LayoutRunControl::synchronous(&cancellation);
        match self.relayout_text_with_document_styles_controlled(
            document_id,
            document_revision,
            text,
            hard_lines,
            view,
            document_styles,
            &control,
        ) {
            Ok(()) => Ok(()),
            Err(LayoutComputationError::Layout(error)) => Err(error),
            Err(LayoutComputationError::Cancelled) => {
                unreachable!("the synchronous layout probe never cancels")
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn relayout_text_with_document_styles_controlled(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        hard_lines: &[Range<usize>],
        view: &mut ViewLayout,
        document_styles: Option<DocumentLayoutStyles>,
        control: &LayoutRunControl<'_>,
    ) -> Result<(), LayoutComputationError> {
        control.checkpoint()?;
        let default_style = if view.default_style_is_override {
            view.default_style.clone()
        } else {
            document_styles.as_ref().map_or_else(
                || view.default_style.clone(),
                |styles| styles.default_shaping_style.clone(),
            )
        };
        let style_runs = if view.style_runs_are_override {
            view.style_runs.clone()
        } else {
            document_styles.as_ref().map_or_else(
                || view.style_runs.clone(),
                |styles| styles.shaping_runs.clone(),
            )
        };
        let document_insets = document_styles
            .as_ref()
            .map_or_else(EdgeInsets::default, |styles| styles.document_insets);
        let content_insets = add_insets(view.insets, document_insets);
        let usable_width = usable_width(view.width, content_insets);
        let document_style_revision = document_styles
            .as_ref()
            .map(|styles| styles.style_sheet_revision);
        let canvas_background = document_styles.as_ref().map_or_else(
            || Color {
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
            },
            |styles| styles.canvas_background,
        );
        let canvas_background_is_default = document_styles
            .as_ref()
            .map_or(true, |styles| styles.canvas_background_is_default);
        let default_paint = document_styles
            .as_ref()
            .map_or_else(ResolvedTextPaint::default, |styles| {
                styles.default_paint.clone()
            });
        let paint_runs = document_styles
            .as_ref()
            .map_or_else(Vec::new, |styles| styles.paint_runs.clone());
        let paragraph_styles = document_styles
            .as_ref()
            .map_or_else(Vec::new, |styles| styles.paragraphs.clone());

        validate_style_runs_for_text(&style_runs, text)?;
        if !default_style.is_valid() {
            return Err(LayoutError::InvalidStyle.into());
        }

        validate_hard_line_ranges(text, hard_lines, control)?;
        let measurement_environment_id = self.provider.measurement_environment_id();
        let metrics_generation = self.provider.metrics_generation();
        let mut line_paragraphs = Vec::with_capacity(hard_lines.len());
        for line in hard_lines {
            control.checkpoint()?;
            line_paragraphs.push(resolve_line_paragraph(
                line,
                &paragraph_styles,
                &default_style,
            ));
        }
        let fragment_ranges: Vec<Vec<Range<usize>>> = hard_lines
            .iter()
            .map(|line| fragment_range_at_graphemes(text, line.clone(), control))
            .collect::<Result<_, _>>()?;
        let mut flat_fragment_ranges = Vec::new();
        let mut fragment_default_styles = Vec::new();
        let mut fragment_base_directions = Vec::new();
        let mut fragment_line_bounds = Vec::new();
        for ((ranges, paragraph), line) in
            fragment_ranges.iter().zip(&line_paragraphs).zip(hard_lines)
        {
            let style = if view.default_style_is_override {
                &default_style
            } else {
                &paragraph.style.default_shaping_style
            };
            let direction = shaping_base_direction(paragraph.style.base_direction);
            for (index, range) in ranges.iter().enumerate() {
                if index % MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS == 0 {
                    control.checkpoint()?;
                }
                flat_fragment_ranges.push(range.clone());
                fragment_default_styles.push(style.clone());
                fragment_base_directions.push(direction);
                fragment_line_bounds.push(line.clone());
            }
        }
        let shaped = self.shape_ranges(
            document_id,
            document_revision,
            text,
            &flat_fragment_ranges,
            &fragment_line_bounds,
            &style_runs,
            &fragment_default_styles,
            &fragment_base_directions,
            view.scale,
            measurement_environment_id,
            metrics_generation,
            control,
        )?;

        control.checkpoint()?;
        if self.provider.measurement_environment_id() != measurement_environment_id {
            return Err(LayoutError::MeasurementEnvironmentChangedDuringShape.into());
        }
        if self.provider.metrics_generation() != metrics_generation {
            return Err(LayoutError::MetricsChangedDuringShape.into());
        }

        let layout_revision = LayoutRevision(view.snapshot.as_ref().map_or(
            self.next_layout_revision,
            |snapshot| {
                self.next_layout_revision
                    .max(snapshot.revision.0.wrapping_add(1).max(1))
            },
        ));
        let diagnostics = shaped
            .iter()
            .flat_map(|fragment| fragment.diagnostics.iter().cloned())
            .collect();
        let mut fragment_cursor = 0;
        let mut rows = Vec::new();
        let mut y = content_insets.top;
        let mut previous_paragraph_index = None;
        let mut previous_spacing_after = 0.0;
        let mut has_previous_paragraph = false;

        for (hard_line_index, line_range) in hard_lines.iter().enumerate() {
            control.checkpoint()?;
            let paragraph = &line_paragraphs[hard_line_index];
            let begins_paragraph = !has_previous_paragraph
                || paragraph.paragraph_index.is_none()
                || paragraph.paragraph_index != previous_paragraph_index;
            if begins_paragraph {
                if has_previous_paragraph {
                    y += previous_spacing_after;
                }
                y += paragraph.style.spacing_before;
            }
            previous_paragraph_index = paragraph.paragraph_index;
            previous_spacing_after = paragraph.style.spacing_after;
            has_previous_paragraph = true;

            let count = fragment_ranges[hard_line_index].len();
            let line_fragments = &shaped[fragment_cursor..fragment_cursor + count];
            fragment_cursor += count;
            let (clusters, empty_metrics) =
                flatten_line_fragments(line_range, line_fragments, control)?;
            let right_to_left = paragraph_is_right_to_left(&paragraph.style, &clusters);
            let (paragraph_first_box, continuation_box) = paragraph_row_boxes(
                content_insets.left,
                usable_width,
                &paragraph.style,
                right_to_left,
            );
            let first_row_box = if paragraph.is_first_hard_line {
                paragraph_first_box
            } else {
                continuation_box
            };
            let breaks = if view.wrap && view.linebreak {
                unicode_line_break_opportunities(
                    &text[line_range.clone()],
                    line_range.start,
                    control,
                )?
            } else {
                BTreeSet::new()
            };
            let row_cluster_ranges = wrap_cluster_ranges(
                &clusters,
                first_row_box.width,
                continuation_box.width,
                view.wrap,
                view.linebreak,
                &breaks,
                control,
            )?;

            if clusters.is_empty() {
                let row_index = rows.len();
                let line_advance =
                    line_advance(empty_metrics.height(), paragraph.style.line_spacing);
                let x = aligned_row_x(first_row_box, 0.0, paragraph.style.alignment, right_to_left);
                let point_downstream = CaretPoint {
                    document_id,
                    document_revision,
                    layout_revision,
                    text_offset: line_range.start,
                    affinity: BoundaryAffinity::Downstream,
                };
                let point_upstream = CaretPoint {
                    affinity: BoundaryAffinity::Upstream,
                    ..point_downstream
                };
                rows.push(VisualRow {
                    paragraph_id: paragraph.paragraph_id,
                    hard_line_index,
                    fragment_index: 0,
                    hard_line_range: line_range.clone(),
                    text_range: line_range.clone(),
                    y,
                    baseline: y + empty_metrics.ascent,
                    ascent: empty_metrics.ascent,
                    descent: empty_metrics.descent,
                    leading: empty_metrics.leading,
                    line_advance,
                    width: 0.0,
                    paragraph_content_x: first_row_box.x,
                    paragraph_content_width: first_row_box.width,
                    wrapped_from_previous: false,
                    wraps_to_next: false,
                    clusters: Vec::new(),
                    carets: vec![
                        PositionedCaret {
                            point: point_downstream,
                            x,
                            row_index,
                        },
                        PositionedCaret {
                            point: point_upstream,
                            x,
                            row_index,
                        },
                    ],
                });
                y += line_advance;
                continue;
            }

            for (row_in_line, cluster_range) in row_cluster_ranges.iter().enumerate() {
                control.checkpoint()?;
                let logical_clusters = &clusters[cluster_range.clone()];
                let row_box = if row_in_line == 0 {
                    first_row_box
                } else {
                    continuation_box
                };
                let row = position_row(
                    logical_clusters,
                    paragraph.paragraph_id,
                    row_box,
                    paragraph.style.alignment,
                    right_to_left,
                    paragraph.style.line_spacing,
                    hard_line_index,
                    line_range.clone(),
                    rows.len(),
                    row_in_line,
                    row_cluster_ranges.len(),
                    y,
                    document_id,
                    document_revision,
                    layout_revision,
                    control,
                )?;
                y += row.height();
                rows.push(row);
            }
        }

        if has_previous_paragraph {
            y += previous_spacing_after;
        }
        y += content_insets.bottom;
        let content_width = content_width_from_rows(&rows, view.width);
        let snapshot = LayoutSnapshot {
            revision: layout_revision,
            document_id,
            document_revision,
            configuration_generation: view.configuration_generation,
            measurement_environment_id,
            metrics_generation,
            viewport_width: view.width,
            viewport_height: view.height,
            usable_width,
            content_insets,
            document_insets,
            document_style_revision,
            canvas_background,
            canvas_background_is_default,
            default_paint,
            paint_runs,
            coverage: LayoutCoverage::FullDocument {
                hard_line_count: hard_lines.len(),
            },
            rows,
            content_width,
            content_width_is_exact: true,
            total_height: y,
            total_height_is_exact: true,
            diagnostics,
            text_len: text.len(),
            grapheme_boundaries: logical_grapheme_boundaries(text, 0, control)?,
        };
        let height_index = view
            .height_index_for_snapshot(&snapshot)
            .map_err(LayoutError::from)?;
        control.checkpoint()?;
        self.next_layout_revision = layout_revision.0.wrapping_add(1).max(1);
        view.snapshot = Some(snapshot);
        view.height_index = height_index;
        view.regional_cache = RegionalLayoutCache::default();
        view.viewport_top = view.clamp_viewport_top(view.viewport_top);
        view.viewport_left = view.clamp_viewport_left(view.viewport_left);
        view.last_error = None;
        view.active_layout_job = None;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_ranges(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        ranges: &[Range<usize>],
        hard_line_bounds: &[Range<usize>],
        style_runs: &[ShapeStyleRun],
        default_styles: &[ResolvedTextStyle],
        paragraph_base_directions: &[TextDirection],
        scale: f32,
        measurement_environment_id: MeasurementEnvironmentId,
        metrics_generation: MetricsGeneration,
        control: &LayoutRunControl<'_>,
    ) -> Result<Vec<ShapedFragment>, LayoutComputationError> {
        self.shape_ranges_with_origin(
            document_id,
            document_revision,
            text,
            ranges,
            hard_line_bounds,
            0,
            style_runs,
            default_styles,
            paragraph_base_directions,
            scale,
            measurement_environment_id,
            metrics_generation,
            control,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_ranges_with_origin(
        &mut self,
        document_id: DocumentId,
        document_revision: Revision,
        text: &str,
        ranges: &[Range<usize>],
        hard_line_bounds: &[Range<usize>],
        coordinate_origin: usize,
        style_runs: &[ShapeStyleRun],
        default_styles: &[ResolvedTextStyle],
        paragraph_base_directions: &[TextDirection],
        scale: f32,
        measurement_environment_id: MeasurementEnvironmentId,
        metrics_generation: MetricsGeneration,
        control: &LayoutRunControl<'_>,
    ) -> Result<Vec<ShapedFragment>, LayoutComputationError> {
        let mut output: Vec<Option<ShapedFragment>> = vec![None; ranges.len()];
        let mut pending = Vec::new();
        let purpose = ShapePurpose::MetricsAndRenderData;
        let render_run_policy =
            self.provider
                .render_run_policy()
                .ok_or(LayoutError::MalformedMeasurement(
                    "render-data provider did not declare a render-run policy",
                ))?;

        if default_styles.len() != ranges.len()
            || paragraph_base_directions.len() != ranges.len()
            || hard_line_bounds.len() != ranges.len()
        {
            return Err(LayoutError::MalformedMeasurement(
                "shape-range metadata counts do not match",
            )
            .into());
        }

        for (output_index, (((local_text_range, hard_line), default_style), base_direction)) in
            ranges
                .iter()
                .zip(hard_line_bounds)
                .zip(default_styles)
                .zip(paragraph_base_directions)
                .enumerate()
        {
            if output_index % MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS == 0 {
                control.checkpoint()?;
            }
            let text_range = local_text_range
                .start
                .checked_add(coordinate_origin)
                .zip(local_text_range.end.checked_add(coordinate_origin))
                .map(|(start, end)| start..end)
                .ok_or(LayoutError::InvalidTextOffset(coordinate_origin))?;
            if local_text_range.start < hard_line.start
                || local_text_range.end > hard_line.end
                || hard_line.start > hard_line.end
                || hard_line.end > text.len()
            {
                return Err(LayoutError::MalformedMeasurement(
                    "shape fragment lies outside its hard line",
                )
                .into());
            }
            let context_before_range =
                context_before_range(text, local_text_range.start, hard_line);
            let context_after_range = context_after_range(text, local_text_range.end, hard_line);
            let shaping_range = context_before_range
                .start
                .checked_add(coordinate_origin)
                .zip(context_after_range.end.checked_add(coordinate_origin))
                .map(|(start, end)| start..end)
                .ok_or(LayoutError::InvalidTextOffset(coordinate_origin))?;
            let text_slice = &text[local_text_range.clone()];
            let context_before = &text[context_before_range];
            let context_after = &text[context_after_range];
            let (global_style_runs, relative_style_runs) =
                styles_for_shaping_span(style_runs, &shaping_range);

            if let Some(relative) = self.cache_lookup(
                text_slice,
                context_before,
                context_after,
                &relative_style_runs,
                default_style,
                *base_direction,
                scale,
                measurement_environment_id,
                metrics_generation,
                purpose,
                Some(render_run_policy),
            ) {
                output[output_index] = Some(relative.to_absolute(
                    document_id,
                    document_revision,
                    measurement_environment_id,
                    metrics_generation,
                    text_range.start,
                ));
            } else {
                pending.push(PendingShape {
                    output_index,
                    text_range,
                    text: text_slice,
                    context_before,
                    context_after,
                    global_style_runs,
                    relative_style_runs,
                    default_style: default_style.clone(),
                    paragraph_base_direction: *base_direction,
                });
            }
        }

        if !pending.is_empty() {
            let batch_size = control.max_shape_batch_fragments.min(pending.len()).max(1);
            for batch in pending.chunks(batch_size) {
                control.checkpoint()?;
                let requests: Vec<ShapeRequest<'_>> = batch
                    .iter()
                    .map(|item| ShapeRequest {
                        document_id,
                        document_revision,
                        measurement_environment_id,
                        text_range: item.text_range.clone(),
                        text: item.text,
                        context_before: item.context_before,
                        context_after: item.context_after,
                        style_runs: &item.global_style_runs,
                        default_style: &item.default_style,
                        paragraph_base_direction: item.paragraph_base_direction,
                        scale,
                        metrics_generation,
                        purpose,
                        render_run_policy: Some(render_run_policy),
                    })
                    .collect();
                let responses = self
                    .provider
                    .shape_batch(&requests)
                    .map_err(LayoutError::from)?;
                control.checkpoint()?;
                if responses.len() != batch.len() {
                    return Err(LayoutError::Measurement(MeasurementError::ResponseCount {
                        expected: batch.len(),
                        actual: responses.len(),
                    })
                    .into());
                }

                for (request_index, (item, response)) in batch.iter().zip(responses).enumerate() {
                    if request_index % CANCELLATION_CLUSTER_BATCH == 0 {
                        control.checkpoint()?;
                    }
                    validate_response(&response, &requests[request_index], item.text)?;
                    let relative =
                        RelativeFragment::from_absolute(&response, item.text_range.start);
                    self.cache_insert(ShapeCacheEntry {
                        text: item.text.to_owned(),
                        context_before: item.context_before.to_owned(),
                        context_after: item.context_after.to_owned(),
                        style_runs: item.relative_style_runs.clone(),
                        default_style: item.default_style.clone(),
                        paragraph_base_direction: item.paragraph_base_direction,
                        scale_bits: scale.to_bits(),
                        measurement_environment_id,
                        metrics_generation,
                        purpose,
                        render_run_policy: Some(render_run_policy),
                        fragment: relative,
                    });
                    output[item.output_index] = Some(response);
                }
            }
        }

        control.checkpoint()?;
        let completed: Result<Vec<_>, LayoutError> = output
            .into_iter()
            .map(|fragment| fragment.ok_or(LayoutError::MalformedMeasurement("missing response")))
            .collect();
        completed.map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cache_lookup(
        &mut self,
        text: &str,
        context_before: &str,
        context_after: &str,
        style_runs: &[ShapeStyleRun],
        default_style: &ResolvedTextStyle,
        paragraph_base_direction: TextDirection,
        scale: f32,
        measurement_environment_id: MeasurementEnvironmentId,
        metrics_generation: MetricsGeneration,
        purpose: ShapePurpose,
        render_run_policy: Option<RenderRunPolicy>,
    ) -> Option<RelativeFragment> {
        let position = self.shape_cache.iter().position(|entry| {
            entry.text == text
                && entry.context_before == context_before
                && entry.context_after == context_after
                && entry.style_runs == style_runs
                && entry.default_style == *default_style
                && entry.paragraph_base_direction == paragraph_base_direction
                && entry.scale_bits == scale.to_bits()
                && entry.measurement_environment_id == measurement_environment_id
                && entry.metrics_generation == metrics_generation
                && entry.purpose == purpose
                && entry.render_run_policy == render_run_policy
        })?;
        let entry = self.shape_cache.remove(position)?;
        let result = entry.fragment.clone();
        self.shape_cache.push_back(entry);
        Some(result)
    }

    fn cache_insert(&mut self, entry: ShapeCacheEntry) {
        if self.cache_capacity == 0 {
            return;
        }
        while self.shape_cache.len() >= self.cache_capacity {
            self.shape_cache.pop_front();
        }
        self.shape_cache.push_back(entry);
    }
}

impl RelativeFragment {
    fn from_absolute(fragment: &ShapedFragment, origin: usize) -> Self {
        debug_assert_eq!(fragment.text_range.start, origin);
        Self {
            ownership_len: fragment.text_range.end - origin,
            clusters: fragment
                .clusters
                .iter()
                .map(|cluster| RelativeCluster {
                    text_range: (cluster.text_range.start - origin)
                        ..(cluster.text_range.end - origin),
                    advance: cluster.advance,
                    metrics: cluster.metrics.clone(),
                    typographic_bounds: cluster.typographic_bounds,
                    ink_bounds: cluster.ink_bounds,
                    bidi_level: cluster.bidi_level,
                    fallback_font: cluster.fallback_font.clone(),
                    caret_stops: cluster
                        .caret_stops
                        .iter()
                        .map(|caret| ClusterCaretStop {
                            text_offset: caret.text_offset - origin,
                            inline_offset: caret.inline_offset,
                            affinity: caret.affinity,
                        })
                        .collect(),
                    render_run: cluster.render_run,
                })
                .collect(),
            visual_order: fragment.visual_order.clone(),
            default_metrics: fragment.default_metrics.clone(),
            diagnostics: fragment
                .diagnostics
                .iter()
                .map(|diagnostic| RelativeDiagnostic {
                    text_range: (diagnostic.text_range.start - origin)
                        ..(diagnostic.text_range.end - origin),
                    message: diagnostic.message.clone(),
                })
                .collect(),
        }
    }

    fn to_absolute(
        &self,
        document_id: DocumentId,
        document_revision: Revision,
        measurement_environment_id: MeasurementEnvironmentId,
        metrics_generation: MetricsGeneration,
        origin: usize,
    ) -> ShapedFragment {
        let clusters = self
            .clusters
            .iter()
            .map(|cluster| ShapedCluster {
                text_range: (cluster.text_range.start + origin)..(cluster.text_range.end + origin),
                advance: cluster.advance,
                metrics: cluster.metrics.clone(),
                typographic_bounds: cluster.typographic_bounds,
                ink_bounds: cluster.ink_bounds,
                bidi_level: cluster.bidi_level,
                fallback_font: cluster.fallback_font.clone(),
                caret_stops: cluster
                    .caret_stops
                    .iter()
                    .map(|caret| ClusterCaretStop {
                        text_offset: caret.text_offset + origin,
                        inline_offset: caret.inline_offset,
                        affinity: caret.affinity,
                    })
                    .collect(),
                render_run: cluster.render_run,
            })
            .collect::<Vec<_>>();
        let text_range = origin..origin + self.ownership_len;
        ShapedFragment {
            document_id,
            document_revision,
            measurement_environment_id,
            metrics_generation,
            text_range,
            clusters,
            visual_order: self.visual_order.clone(),
            default_metrics: self.default_metrics.clone(),
            diagnostics: self
                .diagnostics
                .iter()
                .map(|diagnostic| ShapingDiagnostic {
                    text_range: (diagnostic.text_range.start + origin)
                        ..(diagnostic.text_range.end + origin),
                    message: diagnostic.message.clone(),
                })
                .collect(),
        }
    }
}

fn validate_response(
    response: &ShapedFragment,
    request: &ShapeRequest<'_>,
    request_text: &str,
) -> Result<(), LayoutError> {
    if request.text_range.start > request.text_range.end
        || request.text_range.end - request.text_range.start != request_text.len()
        || request.text != request_text
    {
        return Err(LayoutError::MalformedMeasurement(
            "invalid shaping ownership request",
        ));
    }
    let context_start = request
        .text_range
        .start
        .checked_sub(request.context_before.len())
        .ok_or(LayoutError::MalformedMeasurement(
            "shaping context coordinates underflow",
        ))?;
    let context_end = request
        .text_range
        .end
        .checked_add(request.context_after.len())
        .ok_or(LayoutError::MalformedMeasurement(
            "shaping context coordinates overflow",
        ))?;
    if response.document_id != request.document_id
        || response.document_revision != request.document_revision
        || response.measurement_environment_id != request.measurement_environment_id
        || response.text_range != request.text_range
    {
        return Err(LayoutError::StaleMeasurementResponse);
    }
    if response.metrics_generation != request.metrics_generation {
        return Err(LayoutError::StaleMeasurementResponse);
    }
    if !response.default_metrics.is_valid() {
        return Err(LayoutError::MalformedMeasurement("invalid default metrics"));
    }
    if response.visual_order.len() != response.clusters.len() {
        return Err(LayoutError::MalformedMeasurement(
            "invalid visual order length",
        ));
    }
    let mut visual_seen = vec![false; response.clusters.len()];
    for &index in &response.visual_order {
        let Some(seen) = visual_seen.get_mut(index) else {
            return Err(LayoutError::MalformedMeasurement(
                "visual order index out of bounds",
            ));
        };
        if *seen {
            return Err(LayoutError::MalformedMeasurement(
                "duplicate visual order index",
            ));
        }
        *seen = true;
    }
    if response.visual_order != fragment_visual_order(&response.clusters) {
        return Err(LayoutError::MalformedMeasurement(
            "visual order conflicts with bidi levels",
        ));
    }

    let shaping_capacity = request
        .context_before
        .len()
        .checked_add(request_text.len())
        .and_then(|length| length.checked_add(request.context_after.len()))
        .ok_or(LayoutError::MalformedMeasurement(
            "shaping context length overflow",
        ))?;
    let mut shaping_text = String::with_capacity(shaping_capacity);
    shaping_text.push_str(request.context_before);
    shaping_text.push_str(request_text);
    shaping_text.push_str(request.context_after);
    let grapheme_boundaries: BTreeSet<usize> = std::iter::once(context_start)
        .chain(
            shaping_text
                .grapheme_indices(true)
                .map(|(offset, _)| context_start + offset),
        )
        .chain(std::iter::once(context_end))
        .collect();
    let mut previous_owned_end = None;
    for cluster in &response.clusters {
        if cluster.text_range.start < request.text_range.start
            || cluster.text_range.start >= request.text_range.end
            || cluster.text_range.end <= cluster.text_range.start
            || cluster.text_range.end > context_end
            || previous_owned_end.is_some_and(|end| cluster.text_range.start != end)
            || !cluster.advance.is_finite()
            || cluster.advance < 0.0
            || !cluster.metrics.is_valid()
            || !cluster.typographic_bounds.is_valid()
            || !cluster.ink_bounds.is_valid()
            || cluster.bidi_level > 125
            || cluster.fallback_font.is_empty()
        {
            return Err(LayoutError::MalformedMeasurement("invalid shaping cluster"));
        }
        match (
            request.purpose,
            request.render_run_policy,
            cluster.render_run,
        ) {
            (ShapePurpose::MetricsOnly, _, Some(_)) => {
                return Err(LayoutError::MalformedMeasurement(
                    "metrics-only response retained render data",
                ));
            }
            (ShapePurpose::MetricsAndRenderData, _, None) => {
                return Err(LayoutError::MalformedMeasurement(
                    "render response omitted render data",
                ));
            }
            (_, _, Some(handle)) if handle.metrics_generation != request.metrics_generation => {
                return Err(LayoutError::StaleMeasurementResponse);
            }
            (ShapePurpose::MetricsAndRenderData, Some(policy), Some(handle))
                if handle.owner != policy.owner || handle.threading != policy.threading =>
            {
                return Err(LayoutError::MalformedMeasurement(
                    "render handle violates requested owner policy",
                ));
            }
            (ShapePurpose::MetricsAndRenderData, None, Some(_)) => {
                return Err(LayoutError::MalformedMeasurement(
                    "render request omitted render-run policy",
                ));
            }
            _ => {}
        }
        if !grapheme_boundaries.contains(&cluster.text_range.start)
            || !grapheme_boundaries.contains(&cluster.text_range.end)
        {
            return Err(LayoutError::MalformedMeasurement("cluster splits grapheme"));
        }
        let mut has_start = false;
        let mut has_end = false;
        for (caret_index, caret) in cluster.caret_stops.iter().enumerate() {
            if caret.text_offset < cluster.text_range.start
                || caret.text_offset > cluster.text_range.end
                || !caret.inline_offset.is_finite()
                || caret.inline_offset < 0.0
                || caret.inline_offset > cluster.advance
                || cluster.caret_stops[..caret_index].iter().any(|previous| {
                    previous.text_offset == caret.text_offset && previous.affinity == caret.affinity
                })
            {
                return Err(LayoutError::MalformedMeasurement("invalid caret stop"));
            }
            if !grapheme_boundaries.contains(&caret.text_offset) {
                return Err(LayoutError::MalformedMeasurement("caret splits grapheme"));
            }
            has_start |= caret.text_offset == cluster.text_range.start;
            has_end |= caret.text_offset == cluster.text_range.end;
        }
        if !has_start || !has_end {
            return Err(LayoutError::MalformedMeasurement(
                "cluster lacks edge caret",
            ));
        }
        previous_owned_end = Some(cluster.text_range.end);
    }
    if request.context_before.is_empty()
        && response
            .clusters
            .first()
            .is_some_and(|cluster| cluster.text_range.start != request.text_range.start)
    {
        return Err(LayoutError::MalformedMeasurement(
            "owned clusters do not cover the interior prefix",
        ));
    }
    if previous_owned_end.is_some_and(|end| end < request.text_range.end)
        || (previous_owned_end.is_none()
            && !request_text.is_empty()
            && request.context_before.is_empty())
    {
        return Err(LayoutError::MalformedMeasurement(
            "owned clusters do not cover the interior suffix",
        ));
    }
    if request.text.is_empty() && !response.clusters.is_empty() {
        return Err(LayoutError::MalformedMeasurement(
            "empty request has clusters",
        ));
    }
    for diagnostic in &response.diagnostics {
        if diagnostic.text_range.start > diagnostic.text_range.end
            || diagnostic.text_range.start < request.text_range.start
            || diagnostic.text_range.end > request.text_range.end
        {
            return Err(LayoutError::MalformedMeasurement(
                "diagnostic range lies outside request",
            ));
        }
    }
    Ok(())
}

fn fragment_visual_order(clusters: &[ShapedCluster]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..clusters.len()).collect();
    let max_level = clusters
        .iter()
        .map(|cluster| cluster.bidi_level)
        .max()
        .unwrap_or(0);
    let Some(min_odd_level) = clusters
        .iter()
        .map(|cluster| cluster.bidi_level)
        .filter(|level| level % 2 == 1)
        .min()
    else {
        return order;
    };
    for level in (min_odd_level..=max_level).rev() {
        let mut start = 0;
        while start < order.len() {
            while start < order.len() && clusters[order[start]].bidi_level < level {
                start += 1;
            }
            let mut end = start;
            while end < order.len() && clusters[order[end]].bidi_level >= level {
                end += 1;
            }
            order[start..end].reverse();
            start = end;
        }
    }
    order
}

fn flatten_line_fragments(
    line_range: &Range<usize>,
    fragments: &[ShapedFragment],
    control: &LayoutRunControl<'_>,
) -> Result<(Vec<ShapedCluster>, TextMetrics), LayoutComputationError> {
    flatten_line_fragments_for_slice(line_range, fragments, false, control)
}

fn flatten_line_fragments_for_slice(
    line_range: &Range<usize>,
    fragments: &[ShapedFragment],
    allow_context_suffix: bool,
    control: &LayoutRunControl<'_>,
) -> Result<(Vec<ShapedCluster>, TextMetrics), LayoutComputationError> {
    let mut clusters = Vec::new();
    let mut empty_metrics = TextMetrics {
        ascent: 0.0,
        descent: 0.0,
        leading: 0.0,
    };
    for fragment in fragments {
        control.checkpoint()?;
        empty_metrics.ascent = empty_metrics.ascent.max(fragment.default_metrics.ascent);
        empty_metrics.descent = empty_metrics.descent.max(fragment.default_metrics.descent);
        empty_metrics.leading = empty_metrics.leading.max(fragment.default_metrics.leading);
        clusters.extend(fragment.clusters.iter().cloned());
    }
    if clusters.is_empty() {
        if !empty_metrics.is_valid() {
            return Err(LayoutError::MalformedMeasurement("empty line lacks metrics").into());
        }
    } else {
        let mut expected_start = line_range.start;
        for cluster in &clusters {
            if cluster.text_range.start != expected_start
                || (!allow_context_suffix && cluster.text_range.end > line_range.end)
            {
                return Err(
                    LayoutError::MalformedMeasurement("line fragments do not cover line").into(),
                );
            }
            expected_start = cluster.text_range.end;
        }
        if (!allow_context_suffix && expected_start != line_range.end)
            || (allow_context_suffix && expected_start < line_range.end)
        {
            return Err(
                LayoutError::MalformedMeasurement("line fragments do not cover line").into(),
            );
        }
    }
    Ok((clusters, empty_metrics))
}

#[allow(clippy::too_many_arguments)]
fn position_row(
    logical_clusters: &[ShapedCluster],
    paragraph_id: Option<u64>,
    paragraph_box: ParagraphRowBox,
    alignment: ParagraphAlignment,
    right_to_left: bool,
    line_spacing: LineSpacing,
    hard_line_index: usize,
    hard_line_range: Range<usize>,
    row_index: usize,
    row_in_line: usize,
    rows_in_line: usize,
    y: f32,
    document_id: DocumentId,
    document_revision: Revision,
    layout_revision: LayoutRevision,
    control: &LayoutRunControl<'_>,
) -> Result<VisualRow, LayoutComputationError> {
    control.checkpoint()?;
    let mut ascent = 0.0_f32;
    let mut descent = 0.0_f32;
    let mut leading = 0.0_f32;
    let mut row_width = 0.0_f32;
    for (index, cluster) in logical_clusters.iter().enumerate() {
        if index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        ascent = ascent.max(cluster.metrics.ascent);
        descent = descent.max(cluster.metrics.descent);
        leading = leading.max(cluster.metrics.leading);
        row_width += cluster.advance;
    }
    let natural_height = ascent + descent + leading;
    let line_advance = line_advance(natural_height, line_spacing);
    let text_range = logical_clusters.first().unwrap().text_range.start
        ..logical_clusters.last().unwrap().text_range.end;
    let order = reorder_for_bidi(logical_clusters, control)?;
    let left = aligned_row_x(paragraph_box, row_width, alignment, right_to_left);
    let mut x = left;
    let mut clusters = Vec::with_capacity(logical_clusters.len());
    let mut carets = Vec::new();

    for (visual_index, logical_index) in order.into_iter().enumerate() {
        if visual_index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        let cluster = &logical_clusters[logical_index];
        let cluster_x = x;
        clusters.push(PositionedCluster {
            text_range: cluster.text_range.clone(),
            x: cluster_x,
            advance: cluster.advance,
            typographic_bounds: position_shaped_bounds(
                cluster.typographic_bounds,
                cluster_x,
                y + ascent,
            ),
            ink_bounds: position_shaped_bounds(cluster.ink_bounds, cluster_x, y + ascent),
            bidi_level: cluster.bidi_level,
            fallback_font: cluster.fallback_font.clone(),
            render_run: cluster.render_run,
        });
        for caret in &cluster.caret_stops {
            let point = CaretPoint {
                document_id,
                document_revision,
                layout_revision,
                text_offset: caret.text_offset,
                affinity: caret.affinity,
            };
            carets.push(PositionedCaret {
                point,
                x: cluster_x + caret.inline_offset,
                row_index,
            });
        }
        x += cluster.advance;
    }
    control.checkpoint()?;
    carets.sort_by(|left, right| {
        left.x
            .partial_cmp(&right.x)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.point.text_offset.cmp(&right.point.text_offset))
            .then_with(|| {
                affinity_rank(left.point.affinity).cmp(&affinity_rank(right.point.affinity))
            })
    });
    carets.dedup_by(|right, left| right.point == left.point && right.x == left.x);

    Ok(VisualRow {
        paragraph_id,
        hard_line_index,
        fragment_index: row_in_line,
        hard_line_range,
        text_range,
        y,
        baseline: y + ascent,
        ascent,
        descent,
        leading,
        line_advance,
        width: row_width,
        paragraph_content_x: paragraph_box.x,
        paragraph_content_width: paragraph_box.width,
        wrapped_from_previous: row_in_line > 0,
        wraps_to_next: row_in_line + 1 < rows_in_line,
        clusters,
        carets,
    })
}

fn position_shaped_bounds(bounds: ShapedBounds, x: f32, baseline: f32) -> LayoutRect {
    LayoutRect {
        x: x + bounds.x,
        y: baseline + bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn reorder_for_bidi(
    clusters: &[ShapedCluster],
    control: &LayoutRunControl<'_>,
) -> Result<Vec<usize>, LayoutComputationError> {
    let mut order = Vec::with_capacity(clusters.len());
    let mut max_level = 0;
    let mut min_odd = None;
    for (index, cluster) in clusters.iter().enumerate() {
        if index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        order.push(index);
        max_level = max_level.max(cluster.bidi_level);
        if cluster.bidi_level % 2 == 1 {
            min_odd = Some(min_odd.map_or(cluster.bidi_level, |level: u8| {
                level.min(cluster.bidi_level)
            }));
        }
    }
    let Some(min_odd) = min_odd else {
        return Ok(order);
    };
    for level in (min_odd..=max_level).rev() {
        let mut start = 0;
        let mut scanned = 0;
        while start < order.len() {
            while start < order.len() && clusters[order[start]].bidi_level < level {
                start += 1;
                scanned += 1;
                if scanned % CANCELLATION_CLUSTER_BATCH == 0 {
                    control.checkpoint()?;
                }
            }
            let mut end = start;
            while end < order.len() && clusters[order[end]].bidi_level >= level {
                end += 1;
                scanned += 1;
                if scanned % CANCELLATION_CLUSTER_BATCH == 0 {
                    control.checkpoint()?;
                }
            }
            let mut left = start;
            let mut right = end.saturating_sub(1);
            let mut swaps = 0;
            while left < right {
                order.swap(left, right);
                left += 1;
                right -= 1;
                swaps += 1;
                if swaps % CANCELLATION_CLUSTER_BATCH == 0 {
                    control.checkpoint()?;
                }
            }
            start = end;
        }
    }
    Ok(order)
}

fn resolve_line_paragraph(
    line: &Range<usize>,
    paragraphs: &[ParagraphLayoutStyle],
    fallback_style: &ResolvedTextStyle,
) -> LineParagraphLayout {
    let paragraph_index = paragraphs.iter().position(|paragraph| {
        if line.is_empty() {
            paragraph.text_range.start == line.start
                || (paragraph.text_range.start < line.start
                    && line.start < paragraph.text_range.end)
        } else {
            paragraph.text_range.start <= line.start && line.end <= paragraph.text_range.end
        }
    });
    if let Some(index) = paragraph_index {
        let style = paragraphs[index].clone();
        return LineParagraphLayout {
            paragraph_index: Some(index),
            paragraph_id: Some(style.block_id),
            is_first_hard_line: line.start == style.text_range.start,
            style,
        };
    }

    LineParagraphLayout {
        paragraph_index: None,
        paragraph_id: None,
        is_first_hard_line: true,
        style: ParagraphLayoutStyle {
            block_id: 0,
            text_range: line.clone(),
            spacing_before: 0.0,
            spacing_after: 0.0,
            line_spacing: LineSpacing::Normal,
            first_line_indent: 0.0,
            leading_indent: 0.0,
            trailing_indent: 0.0,
            alignment: ParagraphAlignment::Start,
            base_direction: WritingDirection::Natural,
            default_shaping_style: fallback_style.clone(),
        },
    }
}

fn starts_new_paragraph(current: &LineParagraphLayout, next: &LineParagraphLayout) -> bool {
    next.paragraph_index.is_none() || next.paragraph_index != current.paragraph_index
}

fn shaping_base_direction(direction: WritingDirection) -> TextDirection {
    match direction {
        WritingDirection::Natural => TextDirection::Auto,
        WritingDirection::LeftToRight => TextDirection::LeftToRight,
        WritingDirection::RightToLeft => TextDirection::RightToLeft,
    }
}

fn paragraph_is_right_to_left(
    paragraph: &ParagraphLayoutStyle,
    clusters: &[ShapedCluster],
) -> bool {
    match paragraph.base_direction {
        WritingDirection::LeftToRight => false,
        WritingDirection::RightToLeft => true,
        WritingDirection::Natural => clusters
            .iter()
            .find(|cluster| cluster.advance > 0.0)
            .is_some_and(|cluster| cluster.bidi_level % 2 == 1),
    }
}

fn paragraph_row_boxes(
    canvas_left: f32,
    canvas_width: f32,
    paragraph: &ParagraphLayoutStyle,
    right_to_left: bool,
) -> (ParagraphRowBox, ParagraphRowBox) {
    let canvas_right = canvas_left + canvas_width;
    let (continuation_left, continuation_right, first_left, first_right) = if right_to_left {
        let left = canvas_left + paragraph.trailing_indent;
        let right = canvas_right - paragraph.leading_indent;
        (left, right, left, right - paragraph.first_line_indent)
    } else {
        let left = canvas_left + paragraph.leading_indent;
        let right = canvas_right - paragraph.trailing_indent;
        (left, right, left + paragraph.first_line_indent, right)
    };
    (
        row_box(first_left, first_right),
        row_box(continuation_left, continuation_right),
    )
}

fn row_box(left: f32, right: f32) -> ParagraphRowBox {
    ParagraphRowBox {
        x: left,
        width: (right - left).max(0.0),
    }
}

fn aligned_row_x(
    paragraph_box: ParagraphRowBox,
    row_width: f32,
    alignment: ParagraphAlignment,
    right_to_left: bool,
) -> f32 {
    let remaining = paragraph_box.width - row_width;
    match (alignment, right_to_left) {
        (ParagraphAlignment::Start, true) | (ParagraphAlignment::End, false) => {
            paragraph_box.x + remaining
        }
        (ParagraphAlignment::Center, _) => paragraph_box.x + remaining / 2.0,
        (ParagraphAlignment::Start, false) | (ParagraphAlignment::End, true) => paragraph_box.x,
    }
}

fn line_advance(natural_height: f32, spacing: LineSpacing) -> f32 {
    match spacing {
        LineSpacing::Normal => natural_height,
        LineSpacing::Multiplier(multiplier) => natural_height * multiplier,
        LineSpacing::AtLeast(minimum) => natural_height.max(minimum),
        LineSpacing::Exact(exact) => exact,
    }
}

fn wrap_cluster_ranges(
    clusters: &[ShapedCluster],
    first_row_width: f32,
    continuation_width: f32,
    wrap: bool,
    linebreak: bool,
    word_breaks: &BTreeSet<usize>,
    control: &LayoutRunControl<'_>,
) -> Result<Vec<Range<usize>>, LayoutComputationError> {
    control.checkpoint()?;
    if clusters.is_empty() {
        return Ok(Vec::new());
    }
    if !wrap {
        return Ok(std::iter::once(0..clusters.len()).collect());
    }

    let mut rows = Vec::new();
    let mut start = 0;
    while start < clusters.len() {
        control.checkpoint()?;
        let usable_width = if rows.is_empty() {
            first_row_width
        } else {
            continuation_width
        };
        let mut width = 0.0;
        let mut next = start;
        let mut last_word_break = None;
        while next < clusters.len() {
            if (next - start) % CANCELLATION_CLUSTER_BATCH == 0 {
                control.checkpoint()?;
            }
            let candidate = width + clusters[next].advance;
            if next > start && candidate > usable_width {
                break;
            }
            width = candidate;
            next += 1;
            if linebreak && word_breaks.contains(&clusters[next - 1].text_range.end) {
                last_word_break = Some(next);
            }
            if width > usable_width {
                break;
            }
        }
        if next == start {
            next += 1;
        } else if next < clusters.len() && linebreak {
            if let Some(word_break) = last_word_break.filter(|break_at| *break_at > start) {
                next = word_break;
            }
        }
        rows.push(start..next);
        start = next;
    }
    Ok(rows)
}

fn unicode_line_break_opportunities(
    line: &str,
    origin: usize,
    control: &LayoutRunControl<'_>,
) -> Result<BTreeSet<usize>, LayoutComputationError> {
    control.checkpoint()?;
    let mut result = BTreeSet::new();
    // `unicode_linebreak` implements the default UAX #14 algorithm and emits
    // UTF-8 byte boundaries. Keep them in the formatted document's absolute
    // coordinate space; wrapping later accepts an opportunity only when it is
    // also the end of a legal shaping cluster.
    for (index, (offset, _opportunity)) in linebreaks(line).enumerate() {
        if index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        let absolute = origin
            .checked_add(offset)
            .ok_or(LayoutError::InvalidTextOffset(origin))?;
        result.insert(absolute);
    }
    control.checkpoint()?;
    Ok(result)
}

fn unicode_line_break_opportunities_for_slice(
    captured: &str,
    capture_origin: usize,
    work_start: usize,
    capture_starts_hard_line: bool,
    control: &LayoutRunControl<'_>,
) -> Result<BTreeSet<usize>, LayoutComputationError> {
    let local_work_start = work_start
        .checked_sub(capture_origin)
        .filter(|offset| *offset <= captured.len() && captured.is_char_boundary(*offset))
        .ok_or(LayoutError::InvalidTextOffset(work_start))?;
    let safe_start = if local_work_start == 0 {
        0
    } else {
        let (discardable_prefix, _) = split_at_safe(&captured[..local_work_start]);
        if discardable_prefix.is_empty() && !capture_starts_hard_line {
            return Err(LayoutError::UnstableLineBreakContext {
                text_offset: work_start,
            }
            .into());
        }
        discardable_prefix.len()
    };
    unicode_line_break_opportunities(
        &captured[safe_start..],
        capture_origin + safe_start,
        control,
    )
}

fn hard_line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (offset, character) in text.char_indices() {
        if character == '\n' {
            lines.push(start..offset);
            start = offset + 1;
        }
    }
    lines.push(start..text.len());
    lines
}

fn hard_line_ranges_cancellable(
    text: &str,
    control: &LayoutRunControl<'_>,
) -> Result<Vec<Range<usize>>, LayoutComputationError> {
    control.checkpoint()?;
    let mut lines = Vec::new();
    let mut start = 0;
    let mut next_checkpoint = CANCELLATION_TEXT_SCAN_BYTES;
    for (offset, character) in text.char_indices() {
        if offset >= next_checkpoint {
            control.checkpoint()?;
            next_checkpoint = offset.saturating_add(CANCELLATION_TEXT_SCAN_BYTES);
        }
        if character == '\n' {
            lines.push(start..offset);
            start = offset + 1;
        }
    }
    lines.push(start..text.len());
    control.checkpoint()?;
    Ok(lines)
}

fn logical_grapheme_boundaries(
    text: &str,
    origin: usize,
    control: &LayoutRunControl<'_>,
) -> Result<Vec<usize>, LayoutComputationError> {
    let mut boundaries = Vec::new();
    for (index, (offset, _)) in text.grapheme_indices(true).enumerate() {
        if index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        boundaries.push(
            origin
                .checked_add(offset)
                .ok_or(LayoutError::InvalidTextOffset(origin))?,
        );
    }
    let end = origin
        .checked_add(text.len())
        .ok_or(LayoutError::InvalidTextOffset(origin))?;
    if boundaries.last().copied() != Some(end) {
        boundaries.push(end);
    }
    control.checkpoint()?;
    Ok(boundaries)
}

fn boundaries_in_ordered_ranges(
    boundaries: Vec<usize>,
    ranges: &[Range<usize>],
    control: &LayoutRunControl<'_>,
) -> Result<Vec<usize>, LayoutComputationError> {
    let mut selected = Vec::with_capacity(boundaries.len());
    let mut range_index = 0;
    for (index, boundary) in boundaries.into_iter().enumerate() {
        if index % CANCELLATION_CLUSTER_BATCH == 0 {
            control.checkpoint()?;
        }
        while ranges
            .get(range_index)
            .is_some_and(|range| range.end < boundary)
        {
            range_index += 1;
        }
        if ranges
            .get(range_index)
            .is_some_and(|range| range.start <= boundary && boundary <= range.end)
        {
            selected.push(boundary);
        }
    }
    control.checkpoint()?;
    Ok(selected)
}

fn validate_hard_line_ranges(
    text: &str,
    lines: &[Range<usize>],
    control: &LayoutRunControl<'_>,
) -> Result<(), LayoutComputationError> {
    control.checkpoint()?;
    if lines.is_empty()
        || lines.first().map(|line| line.start) != Some(0)
        || lines.last().map(|line| line.end) != Some(text.len())
    {
        return Err(
            LayoutError::MalformedMeasurement("hard lines do not cover the complete text").into(),
        );
    }
    for line in lines {
        control.checkpoint()?;
        if line.start > line.end
            || line.end > text.len()
            || !text.is_char_boundary(line.start)
            || !text.is_char_boundary(line.end)
        {
            return Err(LayoutError::MalformedMeasurement(
                "hard-line range is not a valid UTF-8 text range",
            )
            .into());
        }
    }
    for pair in lines.windows(2) {
        control.checkpoint()?;
        let separator = pair[0].end..pair[1].start;
        if separator.start > separator.end
            || separator.end > text.len()
            || text.get(separator) != Some("\n")
        {
            return Err(LayoutError::MalformedMeasurement(
                "adjacent hard lines lack one explicit U+000A break item",
            )
            .into());
        }
    }
    Ok(())
}

fn fragment_range_at_graphemes(
    text: &str,
    range: Range<usize>,
    control: &LayoutRunControl<'_>,
) -> Result<Vec<Range<usize>>, LayoutComputationError> {
    control.checkpoint()?;
    if range.is_empty() {
        return Ok(vec![range]);
    }
    let mut result = Vec::new();
    let mut start = range.start;
    while start < range.end {
        // Each ordinary interval owns at most MAX_SHAPE_FRAGMENT_BYTES, so
        // checking once per interval bounds cancellation latency by one
        // shaping fragment. A single extended grapheme may exceed that bound
        // and remains necessarily indivisible.
        control.checkpoint()?;
        let target = (start + MAX_SHAPE_FRAGMENT_BYTES).min(range.end);
        let end = if target == range.end {
            range.end
        } else {
            let slice = &text[start..range.end];
            slice
                .grapheme_indices(true)
                .map(|(offset, _)| start + offset)
                .take_while(|boundary| *boundary <= target)
                .filter(|boundary| *boundary > start)
                .last()
                .unwrap_or_else(|| {
                    start
                        + slice
                            .grapheme_indices(true)
                            .nth(1)
                            .map_or(slice.len(), |(offset, _)| offset)
                })
        };
        result.push(start..end);
        start = end;
    }
    Ok(result)
}

fn context_before_range(text: &str, offset: usize, hard_line: &Range<usize>) -> Range<usize> {
    let target = offset
        .saturating_sub(SHAPING_CONTEXT_BYTES)
        .max(hard_line.start);
    let mut start = offset;
    for (relative_start, _) in text[hard_line.start..offset].grapheme_indices(true).rev() {
        start = hard_line.start + relative_start;
        if start <= target {
            break;
        }
    }
    start..offset
}

fn context_after_range(text: &str, offset: usize, hard_line: &Range<usize>) -> Range<usize> {
    let mut end = offset;
    for (relative_start, grapheme) in text[offset..hard_line.end].grapheme_indices(true) {
        end = offset + relative_start + grapheme.len();
        if end - offset >= SHAPING_CONTEXT_BYTES {
            break;
        }
    }
    offset..end
}

fn styles_for_shaping_span(
    style_runs: &[ShapeStyleRun],
    shaping_span: &Range<usize>,
) -> (Vec<ShapeStyleRun>, Vec<ShapeStyleRun>) {
    let global: Vec<_> = style_runs
        .iter()
        .filter_map(|run| {
            let start = run.text_range.start.max(shaping_span.start);
            let end = run.text_range.end.min(shaping_span.end);
            (start < end).then(|| ShapeStyleRun {
                text_range: start..end,
                style: run.style.clone(),
            })
        })
        .collect();
    let relative = global
        .iter()
        .map(|run| ShapeStyleRun {
            text_range: (run.text_range.start - shaping_span.start)
                ..(run.text_range.end - shaping_span.start),
            style: run.style.clone(),
        })
        .collect();
    (global, relative)
}

fn validate_style_runs_structure(style_runs: &[ShapeStyleRun]) -> Result<(), LayoutError> {
    let mut previous_end = 0;
    for (index, run) in style_runs.iter().enumerate() {
        if run.text_range.start >= run.text_range.end {
            return Err(LayoutError::InvalidStyleRun {
                index,
                reason: "style run must be non-empty",
            });
        }
        if index > 0 && run.text_range.start < previous_end {
            return Err(LayoutError::InvalidStyleRun {
                index,
                reason: "style runs must be sorted and non-overlapping",
            });
        }
        if !run.style.is_valid() {
            return Err(LayoutError::InvalidStyleRun {
                index,
                reason: "style metrics are invalid",
            });
        }
        previous_end = run.text_range.end;
    }
    Ok(())
}

fn validate_style_runs_for_text(
    style_runs: &[ShapeStyleRun],
    text: &str,
) -> Result<(), LayoutError> {
    validate_style_runs_structure(style_runs)?;
    let grapheme_boundaries: BTreeSet<usize> = std::iter::once(0)
        .chain(text.grapheme_indices(true).map(|(offset, _)| offset))
        .chain(std::iter::once(text.len()))
        .collect();
    for (index, run) in style_runs.iter().enumerate() {
        if run.text_range.end > text.len()
            || !grapheme_boundaries.contains(&run.text_range.start)
            || !grapheme_boundaries.contains(&run.text_range.end)
        {
            return Err(LayoutError::InvalidStyleRun {
                index,
                reason: "style run is outside text or splits UTF-8",
            });
        }
    }
    Ok(())
}

fn affinity_rank(affinity: BoundaryAffinity) -> u8 {
    match affinity {
        BoundaryAffinity::Downstream => 0,
        BoundaryAffinity::Upstream => 1,
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn add_insets(view: EdgeInsets, document: EdgeInsets) -> EdgeInsets {
    EdgeInsets {
        top: view.top + document.top,
        left: view.left + document.left,
        bottom: view.bottom + document.bottom,
        right: view.right + document.right,
    }
}

fn usable_width(width: f32, insets: EdgeInsets) -> f32 {
    (width - insets.left - insets.right).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        Block, BlockKind, BlockProperties, BlockRole, BlockStyle, CharacterProperties, Document,
        DocumentStyleAssignment, Encoding, FileFormat, Format, StyleDefinitionMetadata, StyleId,
        StyleSheet, TextRange,
    };
    use crate::layout::DocumentStyleInput;
    use std::cell::Cell;

    fn lay_out(
        text: &str,
        width: f32,
    ) -> (
        Document,
        LayoutEngine<crate::layout::MockTextMeasurementProvider>,
        ViewLayout,
    ) {
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(width, 400.0);
        engine.relayout(&document, &mut view).unwrap();
        (document, engine, view)
    }

    fn lay_out_regional_line(
        engine: &mut LayoutEngine<crate::layout::MockTextMeasurementProvider>,
        document: &Document,
        view: &ViewLayout,
        hard_line: usize,
    ) -> RegionalLayoutSnapshot {
        let line_range =
            document.line_start(hard_line).unwrap()..document.line_end(hard_line).unwrap();
        let following_line_range = (hard_line + 1 < document.line_count()).then(|| {
            document.line_start(hard_line + 1).unwrap()..document.line_end(hard_line + 1).unwrap()
        });
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        let captured_view = view.capture_for_regional_layout_job(line_range.clone());

        engine
            .layout_hard_line_region_cancellable(
                document.id(),
                document.revision(),
                &document.text()[line_range.clone()],
                line_range.start,
                std::slice::from_ref(&line_range),
                hard_line,
                document.line_count(),
                document.text().len(),
                following_line_range,
                &styles,
                &captured_view,
                &NeverCancelled,
            )
            .unwrap()
    }

    fn insert_paragraph_style(
        sheet: &mut StyleSheet,
        name: &str,
        block: BlockProperties,
        character: CharacterProperties,
    ) -> StyleId {
        let id: StyleId = name.into();
        sheet
            .insert_block_style(
                BlockStyle {
                    id: id.clone(),
                    based_on: Some(sheet.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character,
                    block,
                },
                StyleDefinitionMetadata::generated(name),
            )
            .unwrap();
        id
    }

    fn resolve_fixture_styles(
        text: &str,
        blocks: &[Block],
        sheet: &StyleSheet,
    ) -> DocumentLayoutStyles {
        let document_style = DocumentStyleAssignment::new(sheet.base_document.clone());
        DocumentLayoutStyles::resolve_input(DocumentStyleInput {
            text,
            blocks,
            style_spans: &[],
            style_sheet: sheet,
            document_style: &document_style,
        })
        .unwrap()
    }

    fn assert_height_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= expected.abs().max(1.0) * 1.0e-6,
            "expected height {expected}, got {actual}"
        );
    }

    #[test]
    fn unattached_view_starts_with_one_compact_estimated_hard_line() {
        let view = ViewLayout::new(100.0, 100.0);
        assert_eq!(view.viewport_left(), 0.0);
        assert_eq!(view.maximum_viewport_left(), Some(0.0));
        let height = view.content_height();
        assert_eq!(height.height(), DEFAULT_ESTIMATED_HARD_LINE_HEIGHT);
        assert!(!height.is_exact());
        let statistics = view.height_index_statistics();
        assert_eq!(statistics.hard_line_count(), 1);
        assert_eq!(statistics.run_count(), 1);
        assert_eq!(statistics.tree_depth(), 1);
        let hit = view.hard_line_at_y(0.0).unwrap().unwrap();
        assert_eq!(hit.hard_line(), 0);
        assert!(!hit.line_is_exact());
    }

    #[test]
    fn partial_snapshot_globalizes_cluster_bounds_with_their_visual_row() {
        let document = Document::new("zero\none\ntwo");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let view = ViewLayout::new(200.0, 100.0);
        let region = lay_out_regional_line(&mut engine, &document, &view, 2);
        let relative_row = region.lines()[0].rows()[0].clone();
        assert!(!relative_row.clusters.is_empty());

        let mut height_index = ViewHeightIndex::new_estimated(document.line_count(), 40.0).unwrap();
        height_index
            .set_exact_height(2, region.lines()[0].height())
            .unwrap();
        let line_top = height_index.prefix_height(2).unwrap().height() as f32;
        let snapshot = partial_snapshot_from_region(&region, &height_index).unwrap();
        let positioned_row = &snapshot.rows[0];

        assert!((positioned_row.y - (relative_row.y + line_top)).abs() < 1.0e-5);
        assert!((positioned_row.baseline - (relative_row.baseline + line_top)).abs() < 1.0e-5);
        for (positioned, relative) in positioned_row.clusters.iter().zip(&relative_row.clusters) {
            assert!(
                (positioned.typographic_bounds.y - (relative.typographic_bounds.y + line_top))
                    .abs()
                    < 1.0e-5
            );
            assert!((positioned.ink_bounds.y - (relative.ink_bounds.y + line_top)).abs() < 1.0e-5);
        }
    }

    #[test]
    fn partial_snapshot_height_refresh_moves_cluster_bounds_with_their_visual_row() {
        let document = Document::new("zero\none\ntwo");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let view = ViewLayout::new(200.0, 100.0);
        let visible_region = lay_out_regional_line(&mut engine, &document, &view, 2);

        let mut old_height_index =
            ViewHeightIndex::new_estimated(document.line_count(), 40.0).unwrap();
        old_height_index
            .set_exact_height(2, visible_region.lines()[0].height())
            .unwrap();
        let old_snapshot =
            partial_snapshot_from_region(&visible_region, &old_height_index).unwrap();

        let changed_region = lay_out_regional_line(&mut engine, &document, &view, 0);
        let mut new_height_index = old_height_index.clone();
        new_height_index
            .set_exact_height(0, changed_region.lines()[0].height())
            .unwrap();
        let old_start = old_snapshot.coverage.vertical_range().unwrap().start;
        let new_start = new_height_index.prefix_height(2).unwrap().height() as f32;
        let delta = new_start - old_start;
        assert!(delta.abs() > 1.0);

        let refreshed = refresh_partial_snapshot_after_height_change(
            Some(&old_snapshot),
            &changed_region,
            &new_height_index,
            0..1,
            LayoutRevision(99),
        )
        .unwrap()
        .unwrap();

        for (new_row, old_row) in refreshed.rows.iter().zip(&old_snapshot.rows) {
            assert!((new_row.y - (old_row.y + delta)).abs() < 1.0e-5);
            assert!((new_row.baseline - (old_row.baseline + delta)).abs() < 1.0e-5);
            for (new_cluster, old_cluster) in new_row.clusters.iter().zip(&old_row.clusters) {
                assert!(
                    (new_cluster.typographic_bounds.y - (old_cluster.typographic_bounds.y + delta))
                        .abs()
                        < 1.0e-5
                );
                assert!(
                    (new_cluster.ink_bounds.y - (old_cluster.ink_bounds.y + delta)).abs() < 1.0e-5
                );
            }
        }
    }

    #[test]
    fn wide_proportional_text_scrolls_without_mutating_document_geometry() {
        let document = Document::new("Wi".repeat(16));
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(80.0, 100.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();

        let snapshot = view.snapshot().unwrap();
        assert!(snapshot.rows[0].clusters[0].advance > snapshot.rows[0].clusters[1].advance);
        assert!(snapshot.content_width_is_exact);
        assert!(snapshot.content_width > snapshot.viewport_width);
        let maximum = view.maximum_viewport_left().unwrap();
        assert!(maximum > 0.0);
        let revision = snapshot.revision;
        let configuration = view.configuration_generation();
        let rows = snapshot.rows.clone();
        let provider_requests = engine.provider().request_calls();

        view.set_viewport_left(maximum / 2.0).unwrap();

        assert_eq!(view.viewport_left(), maximum / 2.0);
        assert_eq!(view.configuration_generation(), configuration);
        assert_eq!(view.snapshot().unwrap().revision, revision);
        assert_eq!(view.snapshot().unwrap().rows, rows);
        assert_eq!(engine.provider().request_calls(), provider_requests);

        view.set_viewport_left(f32::MAX).unwrap();
        assert_eq!(view.viewport_left(), maximum);
        view.set_viewport_left(-50.0).unwrap();
        assert_eq!(view.viewport_left(), 0.0);
    }

    #[test]
    fn unwrapped_resize_preserves_then_clamps_scroll_after_exact_relayout() {
        let document = Document::new("W".repeat(40));
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(120.0, 100.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        let provider_requests = engine.provider().request_calls();
        let old_maximum = view.maximum_viewport_left().unwrap();
        assert!(old_maximum > 300.0);
        view.set_viewport_left(300.0).unwrap();

        view.resize(400.0, 100.0);
        assert_eq!(view.viewport_left(), 300.0);
        assert_eq!(view.maximum_viewport_left(), None);

        engine.relayout(&document, &mut view).unwrap();
        let resized_maximum = view.maximum_viewport_left().unwrap();
        assert!(resized_maximum < 300.0);
        assert_eq!(view.viewport_left(), resized_maximum);
        assert_eq!(engine.provider().request_calls(), provider_requests);

        let preserved = resized_maximum / 2.0;
        view.set_viewport_left(preserved).unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(view.viewport_left(), preserved);
        assert_eq!(engine.provider().request_calls(), provider_requests);
    }

    #[test]
    fn wrap_normalizes_horizontal_scroll_and_invalid_values_are_atomic() {
        let document = Document::new("W".repeat(24));
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(80.0, 100.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        view.set_viewport_left(20.0).unwrap();

        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                view.set_viewport_left(invalid),
                Err(LayoutError::InvalidGeometry)
            );
            assert_eq!(view.viewport_left(), 20.0);
        }

        view.set_wrap(true);
        assert_eq!(view.viewport_left(), 0.0);
        assert_eq!(view.maximum_viewport_left(), Some(0.0));
        view.set_viewport_left(30.0).unwrap();
        assert_eq!(view.viewport_left(), 0.0);

        view.set_wrap(false);
        assert_eq!(view.viewport_left(), 0.0);
    }

    #[test]
    fn horizontal_scroll_state_is_independent_between_views() {
        let document = Document::new("Wi".repeat(20));
        let mut first_engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut second_engine =
            LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut first = ViewLayout::new(100.0, 100.0);
        let mut second = ViewLayout::new(100.0, 100.0);
        first.set_wrap(false);
        second.set_wrap(false);
        first_engine.relayout(&document, &mut first).unwrap();
        second_engine.relayout(&document, &mut second).unwrap();
        assert_eq!(
            first.snapshot().unwrap().rows,
            second.snapshot().unwrap().rows
        );

        first.set_viewport_left(40.0).unwrap();

        assert_eq!(first.viewport_left(), 40.0);
        assert_eq!(second.viewport_left(), 0.0);
        assert_eq!(
            first.snapshot().unwrap().rows,
            second.snapshot().unwrap().rows
        );
    }

    #[test]
    fn mixed_widths_and_sizes_control_row_metrics() {
        let document = Document::new("iiWW");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 200.0);
        let large = ResolvedTextStyle {
            size: 28.0,
            ..ResolvedTextStyle::default()
        };
        view.set_style_runs(vec![ShapeStyleRun {
            text_range: 2..4,
            style: large,
        }])
        .unwrap();

        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        let row = &snapshot.rows[0];
        assert_eq!(row.clusters.len(), 4);
        assert!(row.clusters[2].advance > row.clusters[0].advance * 3.5);
        assert!((row.ascent - 28.0 * 0.78).abs() < 0.001);
        assert!((row.descent - 28.0 * 0.22).abs() < 0.001);
        assert_eq!(row.baseline, row.y + row.ascent);
        assert!(
            row.clusters[2].typographic_bounds.height > row.clusters[0].typographic_bounds.height
        );
        assert!(row.clusters.iter().all(|cluster| {
            cluster
                .render_run
                .is_some_and(|run| run.metrics_generation == MetricsGeneration(1))
        }));
        let ink = row.ink_bounds().unwrap();
        assert!(ink.width >= row.width);
        assert!(ink.y <= row.baseline);
    }

    #[test]
    fn markdown_block_styles_feed_shaping_without_view_configuration() {
        let document = Document::from_bytes(
            b"# Heading\nparagraph".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 200.0);
        engine.relayout(&document, &mut view).unwrap();

        let snapshot = view.snapshot().unwrap();
        assert_eq!(
            snapshot.document_style_revision,
            Some(StyleSheetRevision(1))
        );
        assert!((snapshot.rows[0].ascent - 24.0 * 0.78).abs() < 0.001);
        assert!((snapshot.rows[1].ascent - 14.0 * 0.78).abs() < 0.001);
        assert_eq!(snapshot.rows[0].clusters[0].fallback_font, "SF Pro");
    }

    #[test]
    fn markdown_source_marker_edits_invalidate_heading_and_list_layout() {
        let mut document = Document::from_bytes(
            "body text\n".repeat(10_000).into_bytes(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 200.0);
        engine.relayout(&document, &mut view).unwrap();
        assert!((view.snapshot().unwrap().rows[0].ascent - 14.0 * 0.78).abs() < 0.001);
        document.insert(0, "# ").unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert!((view.snapshot().unwrap().rows[0].ascent - 24.0 * 0.78).abs() < 0.001);
        assert!((view.snapshot().unwrap().rows[1].ascent - 14.0 * 0.78).abs() < 0.001);
        document
            .set_list_style(0..0, Some(crate::document::ListStyle::Bullet))
            .unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert!((view.snapshot().unwrap().rows[0].ascent - 14.0 * 0.78).abs() < 0.001);
        assert_eq!(
            document.projection().blocks()[0].style,
            crate::document::StyleId::from("List1")
        );
    }

    #[test]
    fn document_layout_uses_semantic_mac_hard_lines_while_raw_text_defaults_to_u000a() {
        let document = Document::from_bytes_with_file_format(
            b"alpha\nliteral\romega\ninside".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.text(), "alpha\nliteral\nomega\ninside");
        assert_eq!(document.line_count(), 2);

        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(1_000.0, 200.0);
        engine.relayout(&document, &mut view).unwrap();

        let snapshot = view.snapshot().unwrap();
        let first = document.line_start(0).unwrap()..document.line_end(0).unwrap();
        let second = document.line_start(1).unwrap()..document.line_end(1).unwrap();
        assert_eq!(snapshot.coverage.hard_lines(), 0..2);
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.rows[0].hard_line_range, first.clone());
        assert_eq!(snapshot.rows[1].hard_line_range, second.clone());
        assert!(snapshot.rows[0]
            .clusters
            .iter()
            .any(|cluster| cluster.text_range == (5..6)));
        let second_literal_lf = second.start + "omega".len();
        assert!(snapshot.rows[1]
            .clusters
            .iter()
            .any(|cluster| { cluster.text_range == (second_literal_lf..second_literal_lf + 1) }));

        let mut raw_view = ViewLayout::new(1_000.0, 200.0);
        engine
            .relayout_text(
                document.id(),
                document.revision(),
                document.text(),
                &mut raw_view,
            )
            .unwrap();
        assert_eq!(raw_view.snapshot().unwrap().rows.len(), 4);
    }

    #[test]
    fn explicit_view_shaping_overrides_can_be_cleared() {
        let document = Document::new("text");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 200.0);
        view.set_default_style(ResolvedTextStyle {
            font_families: vec!["View Override".to_owned()],
            size: 10.0,
            ..ResolvedTextStyle::default()
        })
        .unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert!((view.snapshot().unwrap().rows[0].ascent - 10.0 * 0.78).abs() < 0.001);
        assert_eq!(
            view.snapshot().unwrap().rows[0].clusters[0].fallback_font,
            "View Override"
        );

        view.clear_default_style_override();
        engine.relayout(&document, &mut view).unwrap();
        assert!((view.snapshot().unwrap().rows[0].ascent - 14.0 * 0.78).abs() < 0.001);
        assert_eq!(
            view.snapshot().unwrap().rows[0].clusters[0].fallback_font,
            "SF Pro"
        );
    }

    #[test]
    fn paragraph_indents_wrap_alignment_and_resize_reuse_shaping() {
        let document = Document::new("abcdefghij");
        let mut sheet = StyleSheet::default();
        let paragraph_style = insert_paragraph_style(
            &mut sheet,
            "NarrowFirstLine",
            BlockProperties {
                first_line_indent: Some(60.0),
                leading_indent: Some(10.0),
                trailing_indent: Some(10.0),
                alignment: Some(ParagraphAlignment::Center),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let blocks = [Block {
            id: 41,
            range: 0..document.text().len(),
            kind: BlockKind::Paragraph,
            style: paragraph_style,
            direct_paragraph: BlockProperties::default(),
            direct_default_character: CharacterProperties::default(),
        }];
        let styles = resolve_fixture_styles(document.text(), &blocks, &sheet);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(120.0, 300.0);
        view.set_linebreak(false);
        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                document.text(),
                styles.clone(),
                &mut view,
            )
            .unwrap();

        let rows = &view.snapshot().unwrap().rows;
        assert!(rows.len() >= 2);
        assert_eq!(rows[0].paragraph_id, Some(41));
        assert_eq!(rows[0].paragraph_content_x, 70.0);
        assert_eq!(rows[0].paragraph_content_width, 40.0);
        assert_eq!(rows[1].paragraph_content_x, 10.0);
        assert_eq!(rows[1].paragraph_content_width, 100.0);
        assert!(rows[0].clusters[0].x > rows[0].paragraph_content_x);
        let centered_x = rows[0].clusters[0].x;
        let shape_calls = engine.provider().request_calls();

        let mut end_aligned = styles.clone();
        end_aligned.paragraphs[0].alignment = ParagraphAlignment::End;
        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                document.text(),
                end_aligned.clone(),
                &mut view,
            )
            .unwrap();
        assert_eq!(engine.provider().request_calls(), shape_calls);
        assert!(view.snapshot().unwrap().rows[0].clusters[0].x > centered_x);

        view.resize(150.0, 300.0);
        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                document.text(),
                end_aligned,
                &mut view,
            )
            .unwrap();
        assert_eq!(engine.provider().request_calls(), shape_calls);
        assert_eq!(view.snapshot().unwrap().usable_width, 150.0);
    }

    #[test]
    fn mixed_paragraph_spacing_line_rules_and_empty_style_are_exact() {
        let text = "a\nb\n";
        let document = Document::new(text);
        let mut sheet = StyleSheet::default();
        let doubled = insert_paragraph_style(
            &mut sheet,
            "Doubled",
            BlockProperties {
                spacing_before: Some(3.0),
                spacing_after: Some(4.0),
                line_spacing: Some(LineSpacing::Multiplier(2.0)),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let minimum = insert_paragraph_style(
            &mut sheet,
            "Minimum",
            BlockProperties {
                spacing_before: Some(5.0),
                spacing_after: Some(6.0),
                line_spacing: Some(LineSpacing::AtLeast(40.0)),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let empty = insert_paragraph_style(
            &mut sheet,
            "EmptyExact",
            BlockProperties {
                spacing_before: Some(7.0),
                spacing_after: Some(8.0),
                line_spacing: Some(LineSpacing::Exact(9.0)),
                first_line_indent: Some(11.0),
                ..BlockProperties::default()
            },
            CharacterProperties {
                size: Some(20.0),
                ..CharacterProperties::default()
            },
        );
        let blocks = [
            Block {
                id: 1,
                range: 0..1,
                kind: BlockKind::Paragraph,
                style: doubled,
                direct_paragraph: BlockProperties::default(),
                direct_default_character: CharacterProperties::default(),
            },
            Block {
                id: 2,
                range: 2..3,
                kind: BlockKind::Paragraph,
                style: minimum,
                direct_paragraph: BlockProperties::default(),
                direct_default_character: CharacterProperties::default(),
            },
            Block {
                id: 3,
                range: 4..4,
                kind: BlockKind::Paragraph,
                style: empty,
                direct_paragraph: BlockProperties::default(),
                direct_default_character: CharacterProperties::default(),
            },
        ];
        let styles = resolve_fixture_styles(text, &blocks, &sheet);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(200.0, 300.0);
        engine
            .relayout_styled_text(document.id(), document.revision(), text, styles, &mut view)
            .unwrap();

        let snapshot = view.snapshot().unwrap();
        let rows = &snapshot.rows;
        assert_eq!(rows.len(), 3);
        let natural_14 = 14.0 * 1.12;
        assert!((rows[0].y - 3.0).abs() < 0.001);
        assert!((rows[0].line_advance - natural_14 * 2.0).abs() < 0.001);
        assert!((rows[1].y - (3.0 + natural_14 * 2.0 + 4.0 + 5.0)).abs() < 0.001);
        assert_eq!(rows[1].line_advance, 40.0);
        assert!((rows[2].y - (rows[1].y + 40.0 + 6.0 + 7.0)).abs() < 0.001);
        assert_eq!(rows[2].line_advance, 9.0);
        assert!((rows[2].ascent - 20.0 * 0.78).abs() < 0.001);
        assert_eq!(rows[2].paragraph_content_x, 11.0);
        assert_eq!(rows[2].carets[0].x, 11.0);
        assert!((snapshot.total_height - (rows[2].y + 9.0 + 8.0)).abs() < 0.001);
        let second_line_y = f64::from(rows[1].y);
        let third_line_y = f64::from(rows[2].y);
        let total_height = f64::from(snapshot.total_height);
        let indexed_total = view.content_height();
        assert!(indexed_total.is_exact());
        assert_height_close(indexed_total.height(), total_height);
        let first_prefix = view.hard_line_prefix_height(1).unwrap();
        assert!(first_prefix.is_exact());
        assert_height_close(first_prefix.height(), second_line_y);
        let tail = view.hard_line_range_height(1..3).unwrap();
        assert!(tail.is_exact());
        assert_height_close(tail.height(), total_height - second_line_y);
        assert_eq!(
            view.hard_line_at_y(third_line_y)
                .unwrap()
                .unwrap()
                .hard_line(),
            2
        );
    }

    #[test]
    fn logical_start_and_end_alignment_follow_paragraph_direction() {
        let text = "ab\nab";
        let document = Document::new(text);
        let mut sheet = StyleSheet::default();
        let rtl_start = insert_paragraph_style(
            &mut sheet,
            "RtlStart",
            BlockProperties {
                alignment: Some(ParagraphAlignment::Start),
                base_direction: Some(WritingDirection::RightToLeft),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let rtl_end = insert_paragraph_style(
            &mut sheet,
            "RtlEnd",
            BlockProperties {
                alignment: Some(ParagraphAlignment::End),
                base_direction: Some(WritingDirection::RightToLeft),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let blocks = [
            Block {
                id: 10,
                range: 0..2,
                kind: BlockKind::Paragraph,
                style: rtl_start,
                direct_paragraph: BlockProperties::default(),
                direct_default_character: CharacterProperties::default(),
            },
            Block {
                id: 11,
                range: 3..5,
                kind: BlockKind::Paragraph,
                style: rtl_end,
                direct_paragraph: BlockProperties::default(),
                direct_default_character: CharacterProperties::default(),
            },
        ];
        let styles = resolve_fixture_styles(text, &blocks, &sheet);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100.0, 100.0);
        engine
            .relayout_styled_text(document.id(), document.revision(), text, styles, &mut view)
            .unwrap();
        let rows = &view.snapshot().unwrap().rows;
        assert!(rows.iter().flat_map(|row| &row.clusters).all(|cluster| {
            cluster.bidi_level == 2
        }), "natural Latin runs retain left-to-right order at embedding level 2 inside an RTL paragraph");
        assert!(rows[0].clusters[0].x > rows[1].clusters[0].x);
        assert_eq!(rows[1].clusters[0].x, 0.0);
    }

    #[test]
    fn paragraph_base_direction_participates_in_the_shaping_cache_key() {
        let text = "ab";
        let document = Document::new(text);
        let mut sheet = StyleSheet::default();
        let left_to_right = insert_paragraph_style(
            &mut sheet,
            "LeftToRight",
            BlockProperties {
                base_direction: Some(WritingDirection::LeftToRight),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let right_to_left = insert_paragraph_style(
            &mut sheet,
            "RightToLeft",
            BlockProperties {
                base_direction: Some(WritingDirection::RightToLeft),
                ..BlockProperties::default()
            },
            CharacterProperties::default(),
        );
        let block = |style| Block {
            id: 1,
            range: 0..2,
            kind: BlockKind::Paragraph,
            style,
            direct_paragraph: BlockProperties::default(),
            direct_default_character: CharacterProperties::default(),
        };
        let left_styles = resolve_fixture_styles(text, &[block(left_to_right)], &sheet);
        let right_styles = resolve_fixture_styles(text, &[block(right_to_left)], &sheet);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100.0, 100.0);

        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                text,
                left_styles,
                &mut view,
            )
            .unwrap();
        assert_eq!(engine.provider().request_calls(), 1);
        assert!(view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .all(|cluster| cluster.bidi_level == 0));

        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                text,
                right_styles,
                &mut view,
            )
            .unwrap();
        assert_eq!(engine.provider().request_calls(), 2);
        assert!(view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .all(|cluster| cluster.bidi_level == 2));
    }

    #[test]
    fn word_wrap_and_cluster_wrap_choose_different_boundaries() {
        let document = Document::new("one two");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut word_view = ViewLayout::new(55.0, 200.0);
        engine.relayout(&document, &mut word_view).unwrap();
        let word_rows = &word_view.snapshot().unwrap().rows;
        assert_eq!(word_rows.len(), 2);
        assert_eq!(word_rows[0].text_range, 0..4);

        let mut cluster_view = ViewLayout::new(55.0, 200.0);
        cluster_view.set_linebreak(false);
        engine.relayout(&document, &mut cluster_view).unwrap();
        let cluster_rows = &cluster_view.snapshot().unwrap().rows;
        assert_eq!(cluster_rows.len(), 2);
        assert_eq!(cluster_rows[0].text_range, 0..5);
    }

    #[test]
    fn unicode_line_break_opportunities_follow_uax14() {
        let control = LayoutRunControl::synchronous(&NeverCancelled);

        let nbsp = unicode_line_break_opportunities("a\u{00a0}b c", 10, &control).unwrap();
        assert_eq!(nbsp, BTreeSet::from([15, 16]));

        let word_joiner = unicode_line_break_opportunities("a\u{2060}b c", 20, &control).unwrap();
        assert_eq!(word_joiner, BTreeSet::from([26, 27]));

        let zero_width_space =
            unicode_line_break_opportunities("ab\u{200b}cd", 30, &control).unwrap();
        assert_eq!(zero_width_space, BTreeSet::from([35, 37]));

        let cjk =
            unicode_line_break_opportunities("\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}", 40, &control)
                .unwrap();
        assert_eq!(cjk, BTreeSet::from([43, 46, 49, 52]));

        let closing_punctuation =
            unicode_line_break_opportunities("\u{4e2d}\u{ff0c}\u{6587}", 50, &control).unwrap();
        assert_eq!(closing_punctuation, BTreeSet::from([56, 59]));

        let opening_punctuation =
            unicode_line_break_opportunities("\u{4e2d}\u{ff08}\u{6587}", 60, &control).unwrap();
        assert_eq!(opening_punctuation, BTreeSet::from([63, 69]));

        let ascii_punctuation =
            unicode_line_break_opportunities("word,word word-word", 70, &control).unwrap();
        assert!(!ascii_punctuation.contains(&75));
        assert!(ascii_punctuation.contains(&80));
        assert!(ascii_punctuation.contains(&85));
    }

    #[test]
    fn long_unicode_line_break_scan_observes_cancellation_batches() {
        struct CancelAfterTwoChecks(std::cell::Cell<usize>);

        impl LayoutCancellationProbe for CancelAfterTwoChecks {
            fn is_cancelled(&self) -> bool {
                let checks = self.0.get();
                self.0.set(checks + 1);
                checks >= 2
            }
        }

        let cancellation = CancelAfterTwoChecks(std::cell::Cell::new(0));
        let control = LayoutRunControl::cancellable(&cancellation);
        let line = "\u{4e2d}".repeat(CANCELLATION_CLUSTER_BATCH * 4);

        assert_eq!(
            unicode_line_break_opportunities(&line, 0, &control),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(cancellation.0.get(), 3);
    }

    #[test]
    fn unicode_word_wrap_honors_glue_joiners_and_punctuation() {
        let (_document, _engine, nbsp_view) = lay_out("a\u{00a0}b c", 29.0);
        assert_eq!(nbsp_view.snapshot().unwrap().rows[0].text_range, 0..4);

        let (_document, _engine, word_joiner_view) = lay_out("a\u{2060}b c", 22.0);
        assert_eq!(
            word_joiner_view.snapshot().unwrap().rows[0].text_range,
            0..5
        );

        let (_document, _engine, zero_width_space_view) = lay_out("ab\u{200b}cd", 32.0);
        assert_eq!(
            zero_width_space_view.snapshot().unwrap().rows[0].text_range,
            0..5
        );

        let (_document, _engine, cjk_view) = lay_out("\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}", 51.0);
        assert_eq!(cjk_view.snapshot().unwrap().rows[0].text_range, 0..6);

        let (_document, _engine, closing_view) = lay_out("\u{4e2d}\u{ff0c}\u{6587}", 40.0);
        assert_eq!(closing_view.snapshot().unwrap().rows[0].text_range, 0..6);

        let (_document, _engine, opening_view) = lay_out("\u{4e2d}\u{ff08}\u{6587}", 40.0);
        assert_eq!(opening_view.snapshot().unwrap().rows[0].text_range, 0..3);

        let (_document, _engine, comma_view) = lay_out("word,word", 75.0);
        assert_eq!(comma_view.snapshot().unwrap().rows[0].text_range, 0..7);
    }

    #[test]
    fn linebreak_off_ignores_uax14_opportunities() {
        let document = Document::new("ab\u{200b}cd");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(32.0, 200.0);
        view.set_linebreak(false);
        engine.relayout(&document, &mut view).unwrap();

        assert_eq!(view.snapshot().unwrap().rows[0].text_range, 0..6);
    }

    #[test]
    fn cluster_fallback_never_splits_a_shaper_ligature() {
        let (_document, _engine, view) = lay_out("fix", 1.0);
        let first_row = &view.snapshot().unwrap().rows[0];

        assert_eq!(first_row.text_range, 0..2);
        assert_eq!(first_row.clusters.len(), 1);
    }

    #[test]
    fn unicode_rewrap_reuses_shaping_and_matches_fresh_layout() {
        let text = "prefix \u{4e2d}\u{ff08}\u{6587} ab\u{200b}cd a\u{00a0}b suffix";
        let (document, mut engine, mut view) = lay_out(text, 500.0);
        let shape_calls = engine.provider().request_calls();

        view.resize(75.0, 400.0);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), shape_calls);
        let resized_ranges: Vec<_> = view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| row.text_range.clone())
            .collect();

        let (_fresh_document, _fresh_engine, fresh_view) = lay_out(text, 75.0);
        let fresh_ranges: Vec<_> = fresh_view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| row.text_range.clone())
            .collect();
        assert_eq!(resized_ranges, fresh_ranges);
    }

    #[test]
    fn resize_rewraps_without_reshaping() {
        let (document, mut engine, mut view) = lay_out("alpha beta gamma delta", 300.0);
        let calls = engine.provider().request_calls();
        let original_rows = view.snapshot().unwrap().rows.len();
        assert_eq!(original_rows, 1);
        assert!(view.content_height().is_exact());
        let hard_line_count = view.height_index_statistics().hard_line_count();

        view.resize(55.0, 200.0);
        assert_eq!(
            view.height_index_statistics().hard_line_count(),
            hard_line_count
        );
        assert!(!view.content_height().is_exact());
        engine.relayout(&document, &mut view).unwrap();
        assert!(view.snapshot().unwrap().rows.len() > original_rows);
        assert_eq!(engine.provider().request_calls(), calls);
        assert!(view.content_height().is_exact());
        let exact_height = view.content_height();

        view.resize(55.0, 800.0);
        assert_eq!(view.content_height(), exact_height);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), calls);
    }

    #[test]
    fn style_and_metrics_changes_invalidate_certainty_without_losing_line_count() {
        let (document, mut engine, mut view) = lay_out("one\ntwo\nthree", 200.0);
        assert!(view.content_height().is_exact());
        assert_eq!(view.height_index_statistics().hard_line_count(), 3);

        view.set_default_style(ResolvedTextStyle {
            size: 20.0,
            ..ResolvedTextStyle::default()
        })
        .unwrap();
        assert_eq!(view.height_index_statistics().hard_line_count(), 3);
        assert!(!view.content_height().is_exact());
        engine.relayout(&document, &mut view).unwrap();
        assert!(view.content_height().is_exact());

        view.invalidate_text_metrics();
        assert_eq!(view.height_index_statistics().hard_line_count(), 3);
        assert!(!view.content_height().is_exact());
        engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        engine.relayout(&document, &mut view).unwrap();
        assert!(view.content_height().is_exact());
    }

    #[test]
    fn hard_line_count_changes_reconcile_by_splice_before_exact_publication() {
        let (mut document, mut engine, mut view) = lay_out("one\ntwo\nthree", 200.0);
        assert_eq!(view.height_index_statistics().hard_line_count(), 3);

        document.insert(4, "inserted\nlines\n").unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(view.height_index_statistics().hard_line_count(), 5);
        assert!(view.content_height().is_exact());
        assert_height_close(
            view.content_height().height(),
            f64::from(view.snapshot().unwrap().total_height),
        );

        document.delete(3..19).unwrap();
        engine.relayout(&document, &mut view).unwrap();
        let expected_count = document
            .text()
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        assert_eq!(
            view.height_index_statistics().hard_line_count(),
            expected_count
        );
        assert!(view.content_height().is_exact());
    }

    #[test]
    fn unchanged_hard_line_reuses_shape_after_document_edit() {
        let (mut document, mut engine, mut view) = lay_out("alpha\nbeta", 400.0);
        assert_eq!(engine.provider().request_calls(), 2);
        document.insert(document.text().len(), "!").unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), 3);
        assert_eq!(view.snapshot().unwrap().rows.len(), 2);
    }

    #[test]
    fn grapheme_clusters_never_split_during_narrow_wrap() {
        let family = "👩‍👩‍👧‍👦";
        let text = format!("a{family}b");
        let (_document, _engine, view) = lay_out(&text, 1.0);
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 3);
        assert_eq!(snapshot.rows[1].text_range, 1..1 + family.len());
        assert_eq!(snapshot.rows[1].clusters.len(), 1);
    }

    #[test]
    fn caret_hit_testing_round_trips_and_ligature_has_fallback_geometry() {
        let (_document, _engine, view) = lay_out("office אב", 500.0);
        let snapshot = view.snapshot().unwrap();
        let start = snapshot
            .caret_point(0, BoundaryAffinity::Downstream)
            .unwrap();
        let geometry = snapshot.caret_geometry(start).unwrap();
        let hit = snapshot
            .hit_test(LayoutPoint {
                x: geometry.rect.x,
                y: geometry.rect.y + 1.0,
            })
            .unwrap();
        assert_eq!(hit.text_offset, 0);
        assert_eq!(
            snapshot.caret_geometry(hit).unwrap().rect.x,
            geometry.rect.x
        );

        let rtl_run_start = "office ".len();
        let rtl = snapshot
            .caret_point(rtl_run_start, BoundaryAffinity::Downstream)
            .unwrap();
        let rtl_geometry = snapshot.caret_geometry(rtl).unwrap();
        let rtl_hit = snapshot
            .hit_test(LayoutPoint {
                x: rtl_geometry.rect.x,
                y: rtl_geometry.rect.y + 1.0,
            })
            .unwrap();
        assert_eq!(rtl_hit, rtl);

        // Mock shaping presents "ffi" as one visual cluster. Offset two is a
        // valid logical grapheme boundary but not a visual caret stop.
        assert_eq!(
            snapshot.caret_point(2, BoundaryAffinity::Downstream),
            Err(LayoutError::NotACaretStop { text_offset: 2 })
        );
        let fallback = snapshot
            .logical_endpoint_geometry(2, BoundaryAffinity::Downstream)
            .unwrap();
        assert!(fallback.is_cluster_fallback);
        assert!(fallback.rect.width > 0.0);
    }

    #[test]
    fn provider_bidi_levels_produce_visual_order() {
        let (_document, _engine, view) = lay_out("aאבb", 500.0);
        let clusters = &view.snapshot().unwrap().rows[0].clusters;
        assert_eq!(clusters.len(), 4);
        assert_eq!(clusters[0].text_range, 0..1);
        assert_eq!(clusters[1].text_range, 3..5);
        assert_eq!(clusters[2].text_range, 1..3);
        assert_eq!(clusters[3].text_range, 5..6);
    }

    #[test]
    fn selection_geometry_preserves_bidi_gaps_in_visual_order() {
        let (document, _engine, view) = lay_out("aאבb", 500.0);
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(3).unwrap(),
        )
        .unwrap();
        let rectangles = view
            .snapshot()
            .unwrap()
            .selection_rectangles(range, BoundaryAffinity::Downstream)
            .unwrap();

        assert_eq!(rectangles.len(), 2);
        assert_eq!(rectangles[0].row_index, 0);
        assert_eq!(rectangles[1].row_index, 0);
        assert!(rectangles[0].rect.x + rectangles[0].rect.width < rectangles[1].rect.x);
    }

    #[test]
    fn selection_geometry_spans_wrapped_rows_and_respects_mixed_metrics() {
        let (document, _engine, view) = lay_out("abcdefgh", 20.0);
        assert!(view.snapshot().unwrap().rows.len() > 1);
        let range = TextRange::new(
            document.text_point(1).unwrap(),
            document.text_point(7).unwrap(),
        )
        .unwrap();
        let rectangles = view
            .snapshot()
            .unwrap()
            .selection_rectangles(range, BoundaryAffinity::Downstream)
            .unwrap();
        assert!(rectangles.len() > 1);
        assert!(rectangles.windows(2).all(|pair| {
            pair[0].row_index < pair[1].row_index
                || (pair[0].row_index == pair[1].row_index && pair[0].rect.x <= pair[1].rect.x)
        }));
        assert!(rectangles
            .iter()
            .all(|rectangle| rectangle.rect.width > 0.0));

        let document = Document::new("iiWW");
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 200.0);
        view.set_style_runs(vec![ShapeStyleRun {
            text_range: 2..4,
            style: ResolvedTextStyle {
                size: 28.0,
                ..ResolvedTextStyle::default()
            },
        }])
        .unwrap();
        engine.relayout(&document, &mut view).unwrap();
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(document.text().len()).unwrap(),
        )
        .unwrap();
        let rectangles = view
            .snapshot()
            .unwrap()
            .selection_rectangles(range, BoundaryAffinity::Downstream)
            .unwrap();
        assert_eq!(rectangles.len(), 2);
        assert!(rectangles[0].rect.height < rectangles[1].rect.height);
        assert!(rectangles[0].rect.y > rectangles[1].rect.y);
    }

    #[test]
    fn empty_selection_uses_caret_or_indivisible_cluster_geometry() {
        let (document, _engine, view) = lay_out("office", 500.0);
        let snapshot = view.snapshot().unwrap();

        let ordinary = document.text_point(0).unwrap();
        let ordinary = TextRange::new(ordinary, ordinary).unwrap();
        let caret = snapshot
            .selection_rectangles(ordinary, BoundaryAffinity::Downstream)
            .unwrap();
        assert_eq!(caret.len(), 1);
        assert_eq!(caret[0].rect.width, 0.0);

        let inside_ligature = document.text_point(2).unwrap();
        let inside_ligature = TextRange::new(inside_ligature, inside_ligature).unwrap();
        let fallback = snapshot
            .selection_rectangles(inside_ligature, BoundaryAffinity::Downstream)
            .unwrap();
        assert_eq!(fallback.len(), 1);
        assert!(fallback[0].rect.width > 0.0);
    }

    #[test]
    fn selected_hard_line_boundary_has_explicit_geometry() {
        let (document, _engine, view) = lay_out("a\nb", 500.0);
        let hard_break = TextRange::new(
            document.text_point(1).unwrap(),
            document.text_point(2).unwrap(),
        )
        .unwrap();
        let rectangles = view
            .snapshot()
            .unwrap()
            .selection_rectangles(hard_break, BoundaryAffinity::Downstream)
            .unwrap();

        assert_eq!(rectangles.len(), 1);
        assert_eq!(rectangles[0].row_index, 0);
        assert_eq!(rectangles[0].rect.width, 0.0);
        assert!(rectangles[0].rect.height > 0.0);
    }

    #[test]
    fn selection_geometry_strictly_checks_document_revision_and_graphemes() {
        let (mut document, _engine, view) = lay_out("a\u{301}b", 500.0);
        let snapshot = view.snapshot().unwrap().clone();
        let other = Document::new("a\u{301}b");
        let other_range = TextRange::new(
            other.text_point(0).unwrap(),
            other.text_point(other.text().len()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot.selection_rectangles(other_range, BoundaryAffinity::Downstream),
            Err(LayoutError::WrongDocument)
        );

        let boundary = document.text_point("a\u{301}".len()).unwrap();
        let range = TextRange::new(boundary, boundary).unwrap();
        document.insert(document.text().len(), "c").unwrap();
        let stale_range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(document.text().len()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot.selection_rectangles(stale_range, BoundaryAffinity::Downstream),
            Err(LayoutError::WrongDocumentRevision)
        );

        let mut missing_boundary = snapshot.clone();
        missing_boundary
            .grapheme_boundaries
            .retain(|offset| *offset != boundary.offset());
        assert_eq!(
            missing_boundary.selection_rectangles(range, BoundaryAffinity::Downstream),
            Err(LayoutError::InvalidGraphemeBoundary {
                text_offset: boundary.offset(),
            })
        );
        assert!(document.text_point(1).is_err());
    }

    #[test]
    fn empty_and_trailing_hard_lines_have_real_geometry() {
        let (_document, _engine, view) = lay_out("\n\n", 100.0);
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 3);
        assert_eq!(snapshot.rows[0].text_range, 0..0);
        assert_eq!(snapshot.rows[1].text_range, 1..1);
        assert_eq!(snapshot.rows[2].text_range, 2..2);
        assert!(snapshot.rows.iter().all(|row| row.height() > 0.0));
        assert!(snapshot.rows.iter().all(|row| row.carets.len() == 2));
        assert!(snapshot.total_height > 0.0);
        for row in &snapshot.rows {
            let caret = row.carets[0].point;
            let geometry = snapshot.caret_geometry(caret).unwrap();
            let hit = snapshot
                .hit_test(LayoutPoint {
                    x: geometry.rect.x,
                    y: geometry.rect.y + geometry.rect.height * 0.5,
                })
                .unwrap();
            assert_eq!(hit.text_offset, caret.text_offset);
            assert_eq!(
                snapshot.caret_geometry(hit).unwrap().rect.x,
                geometry.rect.x
            );
        }
    }

    #[test]
    fn extremely_long_line_is_shaped_in_bounded_fragments() {
        let document = Document::new("x".repeat(10_000));
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100.0, 200.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().batch_calls(), 1);
        assert_eq!(engine.provider().request_calls(), 3);
        assert_eq!(view.snapshot().unwrap().rows.len(), 1);
        assert_eq!(view.snapshot().unwrap().rows[0].clusters.len(), 10_000);

        view.resize(50.0, 200.0);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), 3);
    }

    #[test]
    fn ligature_crossing_a_nominal_fragment_boundary_matches_regional_and_cache_hits() {
        struct Cancelled;

        impl LayoutCancellationProbe for Cancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let mut text = "x".repeat(MAX_SHAPE_FRAGMENT_BYTES - 1);
        text.push_str("fitail");
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100_000.0, 200.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();

        assert_eq!(engine.provider().request_calls(), 2);
        let full_row = view.snapshot().unwrap().rows[0].clone();
        let ligature_range = MAX_SHAPE_FRAGMENT_BYTES - 1..MAX_SHAPE_FRAGMENT_BYTES + 1;
        assert_eq!(
            full_row
                .clusters
                .iter()
                .filter(|cluster| cluster.text_range == ligature_range)
                .count(),
            1,
            "one whole ligature, owned by the preceding fragment, crosses the nominal boundary"
        );

        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), 2);
        assert_eq!(view.snapshot().unwrap().rows[0].clusters, full_row.clusters);

        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        let hard_line_range = 0..document.text().len();
        let mut regional_engine =
            LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let regional = regional_engine
            .layout_hard_line_region_cancellable(
                document.id(),
                document.revision(),
                document.text(),
                0,
                std::slice::from_ref(&hard_line_range),
                0,
                1,
                document.text().len(),
                None,
                &styles,
                &view.capture_for_layout_job(),
                &NeverCancelled,
            )
            .unwrap();
        assert_eq!(regional_engine.provider().request_calls(), 2);
        let regional_row = &regional.lines()[0].rows()[0];
        assert_eq!(regional_row.text_range, full_row.text_range);
        assert_eq!(regional_row.width, full_row.width);
        assert_eq!(regional_row.clusters, full_row.clusters);

        let cached_regional = regional_engine
            .layout_hard_line_region_cancellable(
                document.id(),
                document.revision(),
                document.text(),
                0,
                std::slice::from_ref(&hard_line_range),
                0,
                1,
                document.text().len(),
                None,
                &styles,
                &view.capture_for_layout_job(),
                &NeverCancelled,
            )
            .unwrap();
        assert_eq!(regional_engine.provider().request_calls(), 2);
        let cached_regional_row = &cached_regional.lines()[0].rows()[0];
        assert_eq!(cached_regional_row.text_range, regional_row.text_range);
        assert_eq!(cached_regional_row.width, regional_row.width);
        assert_eq!(cached_regional_row.clusters, regional_row.clusters);

        assert_eq!(
            regional_engine.layout_hard_line_region_cancellable(
                document.id(),
                document.revision(),
                document.text(),
                0,
                std::slice::from_ref(&hard_line_range),
                0,
                1,
                document.text().len(),
                None,
                &styles,
                &view.capture_for_layout_job(),
                &Cancelled,
            ),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(regional_engine.provider().request_calls(), 2);
    }

    #[test]
    fn shaping_context_bounds_never_cut_an_extended_grapheme() {
        let long_grapheme = format!("a{}", "\u{301}".repeat(SHAPING_CONTEXT_BYTES));
        assert!(long_grapheme.len() > SHAPING_CONTEXT_BYTES);
        let text = format!("{long_grapheme}x{long_grapheme}");
        let middle = long_grapheme.len();
        let line = 0..text.len();

        let before = context_before_range(&text, middle, &line);
        let after = context_after_range(&text, middle + 1, &line);
        assert_eq!(&text[before.clone()], long_grapheme);
        assert_eq!(&text[after.clone()], long_grapheme);

        let boundaries: BTreeSet<_> = text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
            .collect();
        assert!(boundaries.contains(&before.start));
        assert!(boundaries.contains(&before.end));
        assert!(boundaries.contains(&after.start));
        assert!(boundaries.contains(&after.end));

        let ascii = "x".repeat(SHAPING_CONTEXT_BYTES * 3);
        let ascii_line = 0..ascii.len();
        assert_eq!(
            context_before_range(&ascii, SHAPING_CONTEXT_BYTES * 2, &ascii_line).len(),
            SHAPING_CONTEXT_BYTES
        );
        assert_eq!(
            context_after_range(&ascii, SHAPING_CONTEXT_BYTES, &ascii_line).len(),
            SHAPING_CONTEXT_BYTES
        );

        let mut boundary_text = "x".repeat(MAX_SHAPE_FRAGMENT_BYTES - 1);
        boundary_text.push_str("a\u{301}tail");
        let never_cancelled = NeverCancelled;
        let control = LayoutRunControl::cancellable(&never_cancelled);
        let fragments =
            fragment_range_at_graphemes(&boundary_text, 0..boundary_text.len(), &control).unwrap();
        assert_eq!(fragments[0], 0..MAX_SHAPE_FRAGMENT_BYTES - 1);
        assert_eq!(fragments[1].start, MAX_SHAPE_FRAGMENT_BYTES - 1);
        assert_eq!(
            &boundary_text[fragments[1].start..fragments[1].start + 3],
            "a\u{301}"
        );

        let document = Document::new(boundary_text);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100_000.0, 100.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        assert!(view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .any(|cluster| {
                cluster.text_range == (MAX_SHAPE_FRAGMENT_BYTES - 1..MAX_SHAPE_FRAGMENT_BYTES + 2)
            }));
    }

    #[test]
    fn grapheme_fragmentation_observes_cancellation_at_a_fixed_byte_cadence() {
        struct CancelAfterCheckpoints {
            checks: Cell<usize>,
            cancel_at: usize,
        }

        impl LayoutCancellationProbe for CancelAfterCheckpoints {
            fn is_cancelled(&self) -> bool {
                let next = self.checks.get() + 1;
                self.checks.set(next);
                next >= self.cancel_at
            }
        }

        let text = "x".repeat(MAX_SHAPE_FRAGMENT_BYTES * 100);
        let cancellation = CancelAfterCheckpoints {
            checks: Cell::new(0),
            cancel_at: 4,
        };
        let control = LayoutRunControl::cancellable(&cancellation);

        assert_eq!(
            fragment_range_at_graphemes(&text, 0..text.len(), &control),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(cancellation.checks.get(), 4);
    }

    #[test]
    fn text_prepasses_observe_cancellation_at_bounded_scan_and_cluster_cadences() {
        struct CancelAfterCheckpoints {
            checks: Cell<usize>,
            cancel_at: usize,
        }

        impl LayoutCancellationProbe for CancelAfterCheckpoints {
            fn is_cancelled(&self) -> bool {
                let next = self.checks.get() + 1;
                self.checks.set(next);
                next >= self.cancel_at
            }
        }

        let text = "x".repeat(CANCELLATION_TEXT_SCAN_BYTES * 4);
        let line_cancellation = CancelAfterCheckpoints {
            checks: Cell::new(0),
            cancel_at: 2,
        };
        let line_control = LayoutRunControl::cancellable(&line_cancellation);
        assert_eq!(
            hard_line_ranges_cancellable(&text, &line_control),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(line_cancellation.checks.get(), 2);

        let boundary_cancellation = CancelAfterCheckpoints {
            checks: Cell::new(0),
            cancel_at: 2,
        };
        let boundary_control = LayoutRunControl::cancellable(&boundary_cancellation);
        assert_eq!(
            logical_grapheme_boundaries(&text, 0, &boundary_control),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(boundary_cancellation.checks.get(), 2);
    }

    #[test]
    fn context_style_changes_invalidate_both_sides_of_a_fragment_boundary() {
        let mut text = "x".repeat(MAX_SHAPE_FRAGMENT_BYTES - 1);
        text.push_str("AVtail");
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100_000.0, 200.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), 2);
        let a_range = MAX_SHAPE_FRAGMENT_BYTES - 1..MAX_SHAPE_FRAGMENT_BYTES;
        let kerned = view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .find(|cluster| cluster.text_range == a_range)
            .unwrap()
            .advance;

        let unkerned_style = ResolvedTextStyle {
            features: vec![OpenTypeFeature {
                tag: *b"kern",
                value: 0,
            }],
            ..ResolvedTextStyle::default()
        };
        view.set_style_runs(vec![ShapeStyleRun {
            text_range: MAX_SHAPE_FRAGMENT_BYTES..MAX_SHAPE_FRAGMENT_BYTES + 1,
            style: unkerned_style.clone(),
        }])
        .unwrap();
        engine.relayout(&document, &mut view).unwrap();

        // The second fragment's interior style changed, and the first
        // fragment's context dependency changed. Reusing either entry would
        // retain stale kerning at the boundary.
        assert_eq!(engine.provider().request_calls(), 4);
        let unkerned = view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .find(|cluster| cluster.text_range == a_range)
            .unwrap()
            .advance;
        assert!(unkerned > kerned);

        view.set_style_runs(vec![
            ShapeStyleRun {
                text_range: a_range,
                style: ResolvedTextStyle {
                    weight: 500.0,
                    ..ResolvedTextStyle::default()
                },
            },
            ShapeStyleRun {
                text_range: MAX_SHAPE_FRAGMENT_BYTES..MAX_SHAPE_FRAGMENT_BYTES + 1,
                style: unkerned_style,
            },
        ])
        .unwrap();
        engine.relayout(&document, &mut view).unwrap();

        // Changing only the preceding context of the second fragment must
        // invalidate that fragment as well as the first fragment's interior.
        assert_eq!(engine.provider().request_calls(), 6);
    }

    #[test]
    fn stale_metrics_result_is_rejected_without_replacing_snapshot() {
        let (document, mut engine, mut view) = lay_out("text", 100.0);
        let installed = view.snapshot().unwrap().revision;
        let installed_height = view.content_height();
        engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        engine
            .provider_mut()
            .set_response_generation_override(Some(MetricsGeneration(1)));
        engine.layout(&document, &mut view);
        assert_eq!(
            view.last_error(),
            Some(&LayoutError::StaleMeasurementResponse)
        );
        assert_eq!(view.snapshot().unwrap().revision, installed);
        assert_eq!(view.content_height(), installed_height);
    }

    #[test]
    fn caret_points_reject_other_documents_and_stale_layouts() {
        let (document, mut engine, mut view) = lay_out("first", 100.0);
        let first_snapshot = view.snapshot().unwrap().clone();
        let old_point = first_snapshot
            .caret_point(0, BoundaryAffinity::Downstream)
            .unwrap();

        view.resize(90.0, 200.0);
        engine.relayout(&document, &mut view).unwrap();
        assert!(matches!(
            view.snapshot().unwrap().caret_geometry(old_point),
            Err(LayoutError::StaleLayout { .. })
        ));

        let other = Document::new("other");
        let mut other_engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        let mut other_view = ViewLayout::new(100.0, 100.0);
        other_engine.relayout(&other, &mut other_view).unwrap();
        assert_eq!(
            other_view.snapshot().unwrap().caret_geometry(old_point),
            Err(LayoutError::WrongDocument)
        );
    }

    #[test]
    fn metrics_generation_invalidates_width_independent_shape_cache() {
        let (document, mut engine, mut view) = lay_out("abc", 100.0);
        assert_eq!(engine.provider().request_calls(), 1);
        engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), 2);
        assert_eq!(
            view.snapshot().unwrap().metrics_generation,
            MetricsGeneration(2)
        );
        assert!(view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .all(|cluster| {
                cluster
                    .render_run
                    .is_some_and(|run| run.metrics_generation == MetricsGeneration(2))
            }));
    }

    #[test]
    fn measurement_environment_invalidates_width_independent_shape_cache() {
        let (document, mut engine, mut view) = lay_out("abc", 100.0);
        assert_eq!(engine.provider().request_calls(), 1);
        engine
            .provider_mut()
            .set_measurement_environment_id(MeasurementEnvironmentId(2));

        engine.relayout(&document, &mut view).unwrap();

        assert_eq!(engine.provider().request_calls(), 2);
        assert_eq!(
            view.snapshot().unwrap().measurement_environment_id,
            MeasurementEnvironmentId(2)
        );
        assert_eq!(
            view.snapshot().unwrap().metrics_generation,
            MetricsGeneration(1)
        );
    }

    #[test]
    fn render_owner_policy_invalidates_shape_cache_without_a_metrics_change() {
        let (document, mut engine, mut view) = lay_out("abc", 100.0);
        assert_eq!(engine.provider().request_calls(), 1);
        let policy = RenderRunPolicy {
            owner: RenderRunOwner(77),
            threading: RenderRunThreading::DedicatedSerialExecutor,
        };
        engine.provider_mut().set_render_run_policy(policy);

        engine.relayout(&document, &mut view).unwrap();

        assert_eq!(engine.provider().request_calls(), 2);
        assert!(view.snapshot().unwrap().rows[0]
            .clusters
            .iter()
            .all(|cluster| cluster.render_run.is_some_and(|handle| {
                handle.owner == policy.owner && handle.threading == policy.threading
            })));
    }

    #[test]
    fn malformed_bounds_caret_and_render_lifetimes_are_rejected() {
        let text = "ab";
        let style = ResolvedTextStyle::default();
        let request = ShapeRequest {
            document_id: DocumentId(9),
            document_revision: Revision(4),
            measurement_environment_id: MeasurementEnvironmentId::default(),
            text_range: 10..12,
            text,
            context_before: "",
            context_after: "",
            style_runs: &[],
            default_style: &style,
            paragraph_base_direction: TextDirection::Auto,
            scale: 1.0,
            metrics_generation: MetricsGeneration(3),
            purpose: ShapePurpose::MetricsAndRenderData,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(0x4d4f_434b),
                threading: RenderRunThreading::AnyThread,
            }),
        };
        let mut provider = crate::layout::MockTextMeasurementProvider::new();
        provider.set_metrics_generation(MetricsGeneration(3));
        let valid = provider.shape_batch(&[request.clone()]).unwrap().remove(0);
        validate_response(&valid, &request, text).unwrap();

        let mut wrong_environment = valid.clone();
        wrong_environment.measurement_environment_id = MeasurementEnvironmentId(44);
        assert_eq!(
            validate_response(&wrong_environment, &request, text),
            Err(LayoutError::StaleMeasurementResponse)
        );

        let mut escaped_response = valid.clone();
        escaped_response.text_range = 9..12;
        assert_eq!(
            validate_response(&escaped_response, &request, text),
            Err(LayoutError::StaleMeasurementResponse)
        );

        let mut escaped_cluster = valid.clone();
        escaped_cluster.clusters[0].text_range.start = 9;
        assert!(matches!(
            validate_response(&escaped_cluster, &request, text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let mut inconsistent_visual_order = valid.clone();
        inconsistent_visual_order.visual_order.swap(0, 1);
        assert!(matches!(
            validate_response(&inconsistent_visual_order, &request, text),
            Err(LayoutError::MalformedMeasurement(
                "visual order conflicts with bidi levels"
            ))
        ));

        let mut invalid_bidi_level = valid.clone();
        invalid_bidi_level.clusters[0].bidi_level = 126;
        assert!(matches!(
            validate_response(&invalid_bidi_level, &request, text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let mut invalid_bounds = valid.clone();
        invalid_bounds.clusters[0].ink_bounds.width = f32::NAN;
        assert!(matches!(
            validate_response(&invalid_bounds, &request, text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let mut duplicate_caret = valid.clone();
        let duplicate = duplicate_caret.clusters[0].caret_stops[0].clone();
        duplicate_caret.clusters[0].caret_stops.push(duplicate);
        assert!(matches!(
            validate_response(&duplicate_caret, &request, text),
            Err(LayoutError::MalformedMeasurement("invalid caret stop"))
        ));

        let mut missing_render_data = valid.clone();
        missing_render_data.clusters[0].render_run = None;
        assert!(matches!(
            validate_response(&missing_render_data, &request, text),
            Err(LayoutError::MalformedMeasurement(
                "render response omitted render data"
            ))
        ));

        let mut wrong_owner = valid.clone();
        wrong_owner.clusters[0].render_run.as_mut().unwrap().owner = RenderRunOwner(99);
        assert!(matches!(
            validate_response(&wrong_owner, &request, text),
            Err(LayoutError::MalformedMeasurement(
                "render handle violates requested owner policy"
            ))
        ));

        let mut missing_policy_request = request.clone();
        missing_policy_request.render_run_policy = None;
        assert!(matches!(
            validate_response(&valid, &missing_policy_request, text),
            Err(LayoutError::MalformedMeasurement(
                "render request omitted render-run policy"
            ))
        ));

        let mut stale_render_data = valid;
        stale_render_data.clusters[0]
            .render_run
            .as_mut()
            .unwrap()
            .metrics_generation = MetricsGeneration(2);
        assert_eq!(
            validate_response(&stale_render_data, &request, text),
            Err(LayoutError::StaleMeasurementResponse)
        );
    }

    #[test]
    fn crossing_cluster_validation_enforces_ownership_context_and_partition_bounds() {
        let style = ResolvedTextStyle::default();
        let request = ShapeRequest {
            document_id: DocumentId(9),
            document_revision: Revision(4),
            measurement_environment_id: MeasurementEnvironmentId::default(),
            text_range: 10..11,
            text: "f",
            context_before: "",
            context_after: "i",
            style_runs: &[],
            default_style: &style,
            paragraph_base_direction: TextDirection::Auto,
            scale: 1.0,
            metrics_generation: MetricsGeneration(1),
            purpose: ShapePurpose::MetricsAndRenderData,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(0x4d4f_434b),
                threading: RenderRunThreading::AnyThread,
            }),
        };
        let mut provider = crate::layout::MockTextMeasurementProvider::new();
        let valid = provider.shape_batch(&[request.clone()]).unwrap().remove(0);
        assert_eq!(valid.clusters.len(), 1);
        assert_eq!(valid.clusters[0].text_range, 10..12);
        validate_response(&valid, &request, request.text).unwrap();

        let cached = RelativeFragment::from_absolute(&valid, request.text_range.start);
        let rebased = cached.to_absolute(
            DocumentId(10),
            Revision(5),
            MeasurementEnvironmentId(6),
            MetricsGeneration(1),
            30,
        );
        assert_eq!(rebased.text_range, 30..31);
        assert_eq!(rebased.clusters[0].text_range, 30..32);

        // ABI-v1 providers shaped only the nominal text. Their exact-interior
        // fragments remain accepted and form a contiguous (if less capable)
        // partition when used through the compatibility path.
        let legacy_head_request = ShapeRequest {
            context_after: "",
            ..request.clone()
        };
        let legacy_head = provider
            .shape_batch(&[legacy_head_request])
            .unwrap()
            .remove(0);
        validate_response(&legacy_head, &request, request.text).unwrap();

        let legacy_tail_shape_request = ShapeRequest {
            text_range: 11..12,
            text: "i",
            context_after: "",
            ..request.clone()
        };
        let legacy_tail = provider
            .shape_batch(&[legacy_tail_shape_request.clone()])
            .unwrap()
            .remove(0);
        let legacy_tail_contract_request = ShapeRequest {
            context_before: "f",
            ..legacy_tail_shape_request
        };
        validate_response(
            &legacy_tail,
            &legacy_tail_contract_request,
            legacy_tail_contract_request.text,
        )
        .unwrap();
        flatten_line_fragments(
            &(10..12),
            &[legacy_head, legacy_tail],
            &LayoutRunControl::synchronous(&NeverCancelled),
        )
        .unwrap();

        let mut starts_in_prior_context = valid.clone();
        starts_in_prior_context.clusters[0].text_range.start = 9;
        assert!(matches!(
            validate_response(&starts_in_prior_context, &request, request.text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let mut exceeds_context = valid.clone();
        exceeds_context.clusters[0].text_range.end = 13;
        assert!(matches!(
            validate_response(&exceeds_context, &request, request.text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let underflowing_context = ShapeRequest {
            text_range: 0..1,
            text: "f",
            context_before: "x",
            ..request.clone()
        };
        assert!(matches!(
            validate_response(&valid, &underflowing_context, underflowing_context.text),
            Err(LayoutError::MalformedMeasurement(
                "shaping context coordinates underflow"
            ))
        ));

        let overflowing_context = ShapeRequest {
            text_range: usize::MAX - 1..usize::MAX,
            text: "f",
            context_before: "",
            context_after: "i",
            ..request.clone()
        };
        assert!(matches!(
            validate_response(&valid, &overflowing_context, overflowing_context.text),
            Err(LayoutError::MalformedMeasurement(
                "shaping context coordinates overflow"
            ))
        ));

        let next_request = ShapeRequest {
            text_range: 11..12,
            text: "i",
            context_after: "",
            ..request.clone()
        };
        let duplicate_next = provider.shape_batch(&[next_request]).unwrap().remove(0);
        assert!(matches!(
            flatten_line_fragments(
                &(10..12),
                &[valid.clone(), duplicate_next],
                &LayoutRunControl::synchronous(&NeverCancelled),
            ),
            Err(LayoutComputationError::Layout(
                LayoutError::MalformedMeasurement("line fragments do not cover line")
            ))
        ));

        let request = ShapeRequest {
            text_range: 10..13,
            text: "abc",
            context_after: "",
            ..request
        };
        let valid = provider.shape_batch(&[request.clone()]).unwrap().remove(0);
        let mut gap = valid.clone();
        gap.clusters.remove(1);
        gap.visual_order = vec![0, 1];
        assert!(matches!(
            validate_response(&gap, &request, request.text),
            Err(LayoutError::MalformedMeasurement("invalid shaping cluster"))
        ));

        let mut prefix_gap = valid;
        prefix_gap.clusters.remove(0);
        prefix_gap.visual_order = vec![0, 1];
        assert!(matches!(
            validate_response(&prefix_gap, &request, request.text),
            Err(LayoutError::MalformedMeasurement(
                "owned clusters do not cover the interior prefix"
            ))
        ));
    }

    #[test]
    fn cancellation_during_positioning_keeps_the_previous_snapshot_atomic() {
        struct CancelAfterPolls {
            cancel_at: usize,
            polls: std::cell::Cell<usize>,
        }

        impl LayoutCancellationProbe for CancelAfterPolls {
            fn is_cancelled(&self) -> bool {
                let polls = self.polls.get() + 1;
                self.polls.set(polls);
                polls >= self.cancel_at
            }
        }

        let line_count = 5_000;
        let document = Document::new(vec!["x"; line_count].join("\n"));
        let mut engine = LayoutEngine::new(crate::layout::MockTextMeasurementProvider::new());
        engine.set_cache_capacity(line_count + 1);
        let mut view = ViewLayout::new(100.0, 100.0);
        engine.relayout(&document, &mut view).unwrap();
        let original = view.snapshot().unwrap().clone();
        let original_height = view.content_height();
        let shape_requests = engine.provider().request_calls();

        // With every fragment cached, shaping performs at most one poll per
        // cancellable batch plus its boundary polls. This threshold therefore
        // lets positioning begin, but is far below the work needed for every
        // hard line.
        let shape_poll_upper_bound = (line_count + MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS - 1)
            / MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS
            + 3;
        let probe = CancelAfterPolls {
            cancel_at: 500,
            polls: std::cell::Cell::new(0),
        };
        assert!(probe.cancel_at > shape_poll_upper_bound);
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();

        assert_eq!(
            engine.relayout_styled_text_cancellable(
                document.id(),
                document.revision(),
                document.text(),
                styles,
                &mut view,
                &probe,
            ),
            Err(LayoutComputationError::Cancelled)
        );
        assert_eq!(probe.polls.get(), probe.cancel_at);
        assert_eq!(engine.provider().request_calls(), shape_requests);
        assert_eq!(view.snapshot(), Some(&original));
        assert_eq!(view.content_height(), original_height);
    }
}
