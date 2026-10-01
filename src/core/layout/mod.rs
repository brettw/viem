//! Width-independent shaping and per-view unpaginated layout.

// Leave room for the native insertion indicator at a zero-margin line end.
pub(crate) const CARET_REVEAL_WIDTH: f32 = 2.0;

pub mod panes;
mod composition;
mod block_box;
mod direction;
mod engine;
mod height_index;
mod jobs;
mod long_line_cache;
mod line_breaks;
mod unicode_breaks;
mod unicode_break_data;
mod measurement;
mod mock;
mod search_overlay;
mod scroll;
mod row_intervals;
mod style;
mod zoom;
mod whitespace;
pub use whitespace::{ListChars, ListCharsError, VisibleWhitespaceOptions, WhitespaceBasis,
    WhitespaceMarker, WhitespaceMarkerKind, WhitespacePresentationOptions};

pub(crate) use composition::capture_range as capture_composition_range;
pub(crate) use engine::{affinity_rank, hard_line_ranges, nearest_caret, DocumentLayoutChange, HardLineLayoutSlice};

pub(crate) use jobs::{ascii_indentation_end, flow_paragraph_styles, resolve_flow_paragraph_styles};
pub(crate) use jobs::{install_layout_job_cache, prepare_cache_layout_job};
pub(crate) use long_line_cache::LongLineCheckpointCache;

pub use zoom::{adjacent_zoom_scale, valid_zoom_scale, ZOOM_STOPS};

pub use engine::{
    CaretGeometry, CaretPoint, EdgeInsets, LayoutCancellationProbe, LayoutComputationError,
    LayoutCoverage, LayoutEngine, LayoutError, LayoutPoint, LayoutRect, LayoutRevision,
    LayoutSnapshot, LayoutWorkStatistics, LongLineLayoutCheckpoint, PositionedCaret,
    DecorationKind, DecorationOwner, PositionedCluster, PositionedDecoration, RegionalHardLineLayout, RegionalLayoutCacheLimits,
    RegionalLayoutCacheStatistics, RegionalLayoutSnapshot, SelectionRectangle,
    ShapingCacheStatistics,
    ViewConfigurationGeneration, ViewLayout, ViewLayoutState, VisualRow,
};
pub use height_index::{
    HardLineHeightHit, HeightMeasurement, ViewHeightIndex, ViewHeightIndexError,
    ViewHeightIndexStatistics,
};
pub use jobs::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    HardLineLayoutRegion, InstalledLayoutJob, LayoutCancellationToken, LayoutComputationScope,
    LayoutExecutionContext, LayoutInstallTarget, LayoutJobCandidate, LayoutJobCaptureStatistics,
    LayoutJobError, LayoutJobId, LayoutJobInstallRejection, LayoutJobPriority, LayoutJobProduct,
    LayoutJobRegion, LayoutJobRequest, LayoutProviderRequirements, ViewportLayoutRegion,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
pub(crate) use measurement::default_column_width;
pub use measurement::{
    BoundaryAffinity, ClusterCaretStop, FontSlant, MeasurementEnvironmentId, MeasurementError,
    MetricsGeneration, OpenTypeFeature, ProviderThreading, RenderRunHandle, RenderRunOwner,
    RenderRunPolicy, RenderRunThreading, ResolvedTextStyle, ShapePurpose, ShapeRequest,
    ShapeStyleRun, ShapedBounds, ShapedCluster, ShapedFragment, ShapingDiagnostic, TextDirection,
    TextMeasurementProvider, TextMetrics,
};
pub use mock::MockTextMeasurementProvider;
pub(crate) use style::shaping_style;
pub use style::{
    DocumentLayoutStyles, DocumentStyleError, DocumentStyleInput, PaintStyleRun,
    ParagraphLayoutStyle, ResolvedTextPaint,
};
pub use block_box::{BlockBoxStyle, ContainerLayoutStyle};
