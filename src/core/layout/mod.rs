//! Width-independent shaping and per-view unpaginated layout.

mod composition;
mod engine;
mod height_index;
mod jobs;
mod long_line_cache;
mod measurement;
mod mock;
mod style;

pub(crate) use composition::capture_range as capture_composition_range;
pub(crate) use engine::HardLineLayoutSlice;

pub(crate) use jobs::{flow_paragraph_styles, resolve_flow_paragraph_styles};
pub(crate) use long_line_cache::LongLineCheckpointCache;

pub use engine::{
    CaretGeometry, CaretPoint, EdgeInsets, LayoutCancellationProbe, LayoutComputationError,
    LayoutCoverage, LayoutEngine, LayoutError, LayoutPoint, LayoutRect, LayoutRevision,
    LayoutSnapshot, LayoutWorkStatistics, LongLineLayoutCheckpoint, PositionedCaret,
    PositionedCluster, RegionalHardLineLayout, RegionalLayoutCacheLimits,
    RegionalLayoutCacheStatistics, RegionalLayoutSnapshot, SelectionRectangle,
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
pub use measurement::{
    BoundaryAffinity, ClusterCaretStop, FontSlant, MeasurementEnvironmentId, MeasurementError,
    MetricsGeneration, OpenTypeFeature, ProviderThreading, RenderRunHandle, RenderRunOwner,
    RenderRunPolicy, RenderRunThreading, ResolvedTextStyle, ShapePurpose, ShapeRequest,
    ShapeStyleRun, ShapedBounds, ShapedCluster, ShapedFragment, ShapingDiagnostic, TextDirection,
    TextMeasurementProvider, TextMetrics,
};
pub use mock::MockTextMeasurementProvider;
pub use style::{
    DocumentLayoutStyles, DocumentStyleError, DocumentStyleInput, PaintStyleRun,
    ParagraphLayoutStyle, ResolvedTextPaint,
};
