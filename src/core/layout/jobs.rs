//! Revision-checked background layout job boundary.
//!
//! Preparation runs on the buffer coordinator and captures owned immutable
//! inputs. Computation receives no `Document` or live `ViewLayout`, so provider
//! calls cannot observe or mutate editor state. Installation is one checked
//! mutation of the target view. Both hard-line and viewport requests capture,
//! shape, and wrap only their requested hard-line regions.

use super::engine::{
    HardLineLayoutSlice, LayoutCancellationProbe, LayoutComputationError, LayoutEngine,
    LayoutError, LayoutJobViewConfiguration, LayoutRevision, LongLineLayoutCheckpoint,
    RegionalLayoutSnapshot, ViewLayout, SHAPING_CONTEXT_BYTES,
};
#[cfg(test)]
use super::engine::{
    CANCELLATION_CLUSTER_BATCH, MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS, MAX_SHAPE_FRAGMENT_BYTES,
};
use super::measurement::{
    MeasurementEnvironmentId, MetricsGeneration, ProviderThreading, TextMeasurementProvider,
};
use super::style::{DocumentLayoutStyles, DocumentStyleError};
use crate::document::{Document, DocumentId, FormattedTextError, FormattedTextTree, Revision};
use std::ops::Range;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// Maximum owned UTF-8 text in one resumable long-hard-line computation. A
/// capture may additionally retain bounded context on both sides. One
/// indivisible word or grapheme may exceed this limit and is kept whole.
pub const MAX_LONG_LINE_LAYOUT_SLICE_BYTES: usize = 64 * 1024;
const LONG_LINE_CAPTURE_CONTEXT_BYTES: usize = SHAPING_CONTEXT_BYTES * 4;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutJobId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LayoutJobPriority {
    ChangedVisibleRows,
    NewlyExposedRows,
    ViewportOverscan,
    Background,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardLineLayoutRegion {
    range: Range<usize>,
}

impl HardLineLayoutRegion {
    pub fn new(range: Range<usize>) -> Result<Self, LayoutJobError> {
        if range.start >= range.end {
            return Err(LayoutJobError::InvalidRegion(
                "hard-line range must be non-empty and ordered",
            ));
        }
        Ok(Self { range })
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewportLayoutRegion {
    hard_lines: Range<usize>,
    viewport_top: f32,
    viewport_height: f32,
    long_line_checkpoint: Option<LongLineLayoutCheckpoint>,
    horizontal_focus_offset: Option<usize>,
    horizontal_desired_x: Option<f32>,
    complete_horizontal_geometry: bool,
}

impl ViewportLayoutRegion {
    pub fn new(
        hard_lines: Range<usize>,
        viewport_top: f32,
        viewport_height: f32,
    ) -> Result<Self, LayoutJobError> {
        if hard_lines.start >= hard_lines.end {
            return Err(LayoutJobError::InvalidRegion(
                "viewport hard-line range must be non-empty and ordered",
            ));
        }
        if !viewport_top.is_finite()
            || viewport_top < 0.0
            || !viewport_height.is_finite()
            || viewport_height <= 0.0
        {
            return Err(LayoutJobError::InvalidRegion(
                "viewport geometry must be finite and non-negative",
            ));
        }
        Ok(Self {
            hard_lines,
            viewport_top,
            viewport_height,
            long_line_checkpoint: None,
            horizontal_focus_offset: None,
            horizontal_desired_x: None,
            complete_horizontal_geometry: false,
        })
    }

    /// Continue a previously returned long-line prefix at its exact visual-row
    /// boundary. Dependency identities are validated again during job capture.
    pub fn resume_long_line(
        checkpoint: LongLineLayoutCheckpoint,
        viewport_top: f32,
        viewport_height: f32,
    ) -> Result<Self, LayoutJobError> {
        let hard_line = checkpoint.hard_line_index();
        let end = hard_line
            .checked_add(1)
            .ok_or(LayoutJobError::InvalidRegion("hard-line index overflow"))?;
        let mut region = Self::new(hard_line..end, viewport_top, viewport_height)?;
        region.long_line_checkpoint = Some(checkpoint);
        Ok(region)
    }

    pub fn hard_lines(&self) -> Range<usize> {
        self.hard_lines.clone()
    }

    pub fn viewport_top(&self) -> f32 {
        self.viewport_top
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn long_line_checkpoint(&self) -> Option<&LongLineLayoutCheckpoint> {
        self.long_line_checkpoint.as_ref()
    }

    pub fn with_horizontal_focus(mut self, offset: usize, desired_x: Option<f32>) -> Self {
        self.horizontal_focus_offset = Some(offset);
        self.horizontal_desired_x = desired_x.filter(|value| value.is_finite());
        self
    }

    pub(crate) fn has_horizontal_focus(&self) -> bool { self.horizontal_focus_offset.is_some() }

    /// Rectangular edits currently inspect every intervening shaping cluster.
    /// Request their complete row geometry before resolving a source change.
    pub fn with_complete_horizontal_geometry(mut self) -> Self {
        self.complete_horizontal_geometry = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutJobRegion {
    HardLines(HardLineLayoutRegion),
    Viewport(ViewportLayoutRegion),
}

impl LayoutJobRegion {
    fn hard_lines(&self) -> Range<usize> {
        match self {
            Self::HardLines(region) => region.range(),
            Self::Viewport(region) => region.hard_lines(),
        }
    }
}

const CANCELLATION_FRESH: u8 = 0;
const CANCELLATION_REGISTERED: u8 = 1;
const CANCELLATION_CANCELLED: u8 = 2;

/// Cooperative state shared by the coordinator, scheduler, and one worker.
/// Registration races cancellation through one atomic modification order:
/// cancellation that wins first prevents the request from becoming current;
/// cancellation after registration applies to that accepted request. Release/
/// acquire ordering makes both transitions visible without a mutex.
#[derive(Clone, Debug, Default)]
pub struct LayoutCancellationToken(Arc<AtomicU8>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationRegistrationError {
    Cancelled,
    AlreadyRegistered,
}

impl LayoutCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(CANCELLATION_CANCELLED, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCELLATION_CANCELLED
    }

    pub(crate) fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn try_register(&self) -> Result<(), CancellationRegistrationError> {
        match self.0.compare_exchange(
            CANCELLATION_FRESH,
            CANCELLATION_REGISTERED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(CANCELLATION_CANCELLED) => Err(CancellationRegistrationError::Cancelled),
            Err(CANCELLATION_REGISTERED) => Err(CancellationRegistrationError::AlreadyRegistered),
            Err(_) => unreachable!("layout cancellation state is valid"),
        }
    }
}

impl LayoutCancellationProbe for LayoutCancellationToken {
    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutProviderRequirements {
    pub measurement_environment_id: MeasurementEnvironmentId,
    pub metrics_generation: MetricsGeneration,
    pub threading: ProviderThreading,
}

/// Inspect provider scheduling requirements without borrowing document or view
/// state. Callers can do this before entering a coordinator turn.
pub fn inspect_layout_provider<P: TextMeasurementProvider>(
    engine: &LayoutEngine<P>,
) -> LayoutProviderRequirements {
    LayoutProviderRequirements {
        measurement_environment_id: engine.provider().measurement_environment_id(),
        metrics_generation: engine.provider().metrics_generation(),
        threading: engine.provider().threading(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutExecutionContext {
    WorkerPool,
    DedicatedSerialExecutor,
    FrontendMainThread,
}

impl LayoutExecutionContext {
    fn permits(self, threading: ProviderThreading) -> bool {
        match threading {
            ProviderThreading::AnyWorker => true,
            ProviderThreading::DedicatedSerialExecutor => self == Self::DedicatedSerialExecutor,
            ProviderThreading::FrontendMainThread => self == Self::FrontendMainThread,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutComputationScope {
    /// Only the requested hard lines were shaped and wrapped.
    RegionalHardLines,
    /// Only the requested viewport hard lines were shaped and wrapped; install
    /// assembles them into a partial visible snapshot with explicit coverage.
    PartialViewport,
}

#[derive(Clone, Debug)]
enum CapturedLayoutInput {
    UnwrappedViewport {
        text: FormattedTextTree,
        line_ranges: Vec<Range<usize>>,
        following_line_range: Option<Range<usize>>,
        document_hard_line_count: usize,
        styles: Arc<DocumentLayoutStyles>,
    },
    StreamingOverflowSlice {
        tree: FormattedTextTree,
        line_slice: HardLineLayoutSlice,
        document_hard_line_count: usize,
        following_line_range: Option<Range<usize>>,
        styles: Arc<DocumentLayoutStyles>,
    },
    Regional {
        text: Arc<str>,
        text_origin: usize,
        line_ranges: Vec<Range<usize>>,
        following_line_range: Option<Range<usize>>,
        document_hard_line_count: usize,
        styles: Arc<DocumentLayoutStyles>,
    },
    LongHardLineSlice {
        text: Arc<str>,
        text_origin: usize,
        line_slice: HardLineLayoutSlice,
        document_hard_line_count: usize,
        following_line_range: Option<Range<usize>>,
        styles: Arc<DocumentLayoutStyles>,
    },
}

/// Owned input package safe to move to a provider-compatible executor.
#[derive(Clone, Debug)]
pub struct LayoutJobRequest {
    job_id: LayoutJobId,
    priority: LayoutJobPriority,
    document_id: DocumentId,
    document_revision: Revision,
    configuration_generation: super::engine::ViewConfigurationGeneration,
    provider_requirements: LayoutProviderRequirements,
    region: LayoutJobRegion,
    cancellation: LayoutCancellationToken,
    projection_text_len: usize,
    input: CapturedLayoutInput,
    captured_view: LayoutJobViewConfiguration,
}

/// Structural accounting for one worker request. Counts are exposed instead of
/// allocator-specific byte telemetry so tests can assert that capture remains
/// regional. Positioned rows, height-index nodes, and regional-cache entries
/// are always zero because the worker view input cannot contain them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutJobCaptureStatistics {
    regional_text_bytes: usize,
    document_shaping_style_runs: usize,
    document_paint_style_runs: usize,
    document_paragraph_styles: usize,
    view_override_style_runs: usize,
}

impl LayoutJobCaptureStatistics {
    pub fn regional_text_bytes(self) -> usize {
        self.regional_text_bytes
    }

    pub fn document_shaping_style_runs(self) -> usize {
        self.document_shaping_style_runs
    }

    pub fn document_paint_style_runs(self) -> usize {
        self.document_paint_style_runs
    }

    pub fn document_paragraph_styles(self) -> usize {
        self.document_paragraph_styles
    }

    pub fn view_override_style_runs(self) -> usize {
        self.view_override_style_runs
    }

    pub fn retained_positioned_rows(self) -> usize {
        0
    }

    pub fn retained_height_index_nodes(self) -> usize {
        0
    }

    pub fn retained_regional_cache_lines(self) -> usize {
        0
    }
}

impl LayoutJobRequest {
    pub fn job_id(&self) -> LayoutJobId {
        self.job_id
    }

    pub fn priority(&self) -> LayoutJobPriority {
        self.priority
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn configuration_generation(&self) -> super::engine::ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn metrics_generation(&self) -> MetricsGeneration {
        self.provider_requirements.metrics_generation
    }

    pub fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.provider_requirements.measurement_environment_id
    }

    pub fn provider_threading(&self) -> ProviderThreading {
        self.provider_requirements.threading
    }

    pub fn region(&self) -> &LayoutJobRegion {
        &self.region
    }

    pub fn cancellation_token(&self) -> LayoutCancellationToken {
        self.cancellation.clone()
    }

    pub fn projection_text_len(&self) -> usize {
        self.projection_text_len
    }

    /// Bytes copied into this request's owned text buffer. Streaming requests
    /// share the immutable formatted tree instead and therefore report zero.
    pub fn captured_text_len(&self) -> usize {
        match &self.input {
            CapturedLayoutInput::UnwrappedViewport { .. }
            | CapturedLayoutInput::StreamingOverflowSlice { .. } => 0,
            CapturedLayoutInput::Regional { text, .. }
            | CapturedLayoutInput::LongHardLineSlice { text, .. } => text.len(),
        }
    }

    pub fn capture_statistics(&self) -> LayoutJobCaptureStatistics {
        match &self.input {
            CapturedLayoutInput::UnwrappedViewport { styles, .. }
            | CapturedLayoutInput::StreamingOverflowSlice { styles, .. } => LayoutJobCaptureStatistics {
                regional_text_bytes: 0,
                document_shaping_style_runs: styles.shaping_runs.len(),
                document_paint_style_runs: styles.paint_runs.len(),
                document_paragraph_styles: styles.paragraphs.len(),
                view_override_style_runs: self.captured_view.retained_override_style_run_count(),
            },
            CapturedLayoutInput::Regional { text, styles, .. }
            | CapturedLayoutInput::LongHardLineSlice { text, styles, .. } => {
                LayoutJobCaptureStatistics {
                    regional_text_bytes: text.len(),
                    document_shaping_style_runs: styles.shaping_runs.len(),
                    document_paint_style_runs: styles.paint_runs.len(),
                    document_paragraph_styles: styles.paragraphs.len(),
                    view_override_style_runs: self
                        .captured_view
                        .retained_override_style_run_count(),
                }
            }
        }
    }
}

/// Source-flow shaping substitutes only classified internal source breaks.
/// UTF-8 byte positions remain identical to the editable source projection.
pub(super) fn flow_text(text: String, origin: usize, lines: &[Range<usize>]) -> String {
    let mut bytes = text.into_bytes();
    for line in lines {
        let start = line.start.max(origin) - origin;
        let end = line.end.saturating_sub(origin).min(bytes.len());
        for byte in &mut bytes[start.min(end)..end] {
            if *byte == b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("ASCII whitespace replacement preserves UTF-8")
}

pub(crate) fn flow_paragraph_styles(styles: &mut DocumentLayoutStyles, lines: &[Range<usize>]) {
    let mut paragraphs: Vec<super::ParagraphLayoutStyle> = Vec::new();
    for line in lines {
        if let Some(paragraph) = styles.paragraphs.iter().find(|paragraph| {
            paragraph.text_range.start <= line.start && line.start < paragraph.text_range.end
                || paragraph.text_range.is_empty() && paragraph.text_range.start == line.start
        }) {
            // A Source paragraph can retain explicit hard breaks while its soft
            // breaks flow. Keep its original start/identity so continuation
            // lines neither repeat paragraph spacing nor first-line indent.
            // Older source projections may use several physical blocks for one
            // flow line; extend their first block only as far as that line.
            if let Some(previous) = paragraphs
                .last_mut()
                .filter(|previous| previous.block_id == paragraph.block_id)
            {
                previous.text_range.end = previous.text_range.end.max(line.end);
            } else {
                let mut paragraph = paragraph.clone();
                paragraph.text_range.start = paragraph.text_range.start.min(line.start);
                paragraph.text_range.end = paragraph.text_range.end.max(line.end);
                paragraphs.push(paragraph);
            }
        }
    }
    styles.paragraphs = paragraphs;
}

/// A bounded long-line capture may start after the first physical block in a
/// flowed source paragraph. Fetch only that block's cascade before extending
/// its paragraph geometry over the complete presentation line.
pub(crate) fn resolve_flow_paragraph_styles(
    projection: &crate::document::FormattedDocument,
    styles: &mut DocumentLayoutStyles,
    lines: &[Range<usize>],
) -> Result<(), DocumentStyleError> {
    if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
        if let Some(paragraphs) =
            DocumentLayoutStyles::source_flow_paragraphs(projection, first.start..last.end)?
        {
            styles.paragraphs = paragraphs;
            return Ok(());
        }
    }
    for line in lines {
        if !styles.paragraphs.iter().any(|paragraph| {
            paragraph.text_range.start <= line.start && line.start <= paragraph.text_range.end
        }) {
            let origin = DocumentLayoutStyles::resolve_region(projection, line.start..line.start)?;
            if let Some(paragraph) = origin.paragraphs.into_iter().find(|paragraph| {
                paragraph.text_range.start <= line.start && line.start <= paragraph.text_range.end
            }) {
                styles.paragraphs.push(paragraph);
            }
        }
    }
    flow_paragraph_styles(styles, lines);
    Ok(())
}

fn retain_regional_styles(
    mut styles: DocumentLayoutStyles,
    line_ranges: &[Range<usize>],
    following_line_range: Option<&Range<usize>>,
) -> DocumentLayoutStyles {
    let text_start = line_ranges.first().map_or(0, |line| line.start);
    let text_end = line_ranges.last().map_or(text_start, |line| line.end);
    styles
        .shaping_runs
        .retain(|run| run.text_range.start < text_end && text_start < run.text_range.end);
    styles
        .paint_runs
        .retain(|run| run.text_range.start < text_end && text_start < run.text_range.end);
    styles.paragraphs.retain(|paragraph| {
        line_ranges
            .iter()
            .chain(following_line_range)
            .any(|line| paragraph_matches_line(paragraph.text_range.clone(), line))
    });
    styles
}

fn following_style_end(
    document: &Document,
    following: Option<&Range<usize>>,
    default_end: usize,
) -> Result<usize, FormattedTextError> {
    let Some(following) = following else {
        return Ok(default_end);
    };
    Ok(document
        .projection()
        .text_tree()
        .next_grapheme_boundary(following.start)?
        .unwrap_or(following.start)
        .min(following.end))
}

fn paragraph_matches_line(paragraph: Range<usize>, line: &Range<usize>) -> bool {
    if line.is_empty() {
        paragraph.start == line.start
            || (paragraph.start < line.start && line.start < paragraph.end)
    } else {
        paragraph.start <= line.start && line.end <= paragraph.end
    }
}

fn document_line_range(
    document: &Document,
    hard_line: usize,
    flow: bool,
) -> Result<Range<usize>, LayoutJobError> {
    document
        .projection()
        .presentation_line_range(hard_line, flow)
        .ok_or(LayoutJobError::InvalidDocumentLineIndex { hard_line })
}

fn validate_long_line_checkpoint(
    document: &Document,
    view: &ViewLayout,
    provider: LayoutProviderRequirements,
    hard_line: &Range<usize>,
    checkpoint: &LongLineLayoutCheckpoint,
) -> Result<(), LayoutJobError> {
    let valid = checkpoint.document_id() == document.id()
        && checkpoint.document_revision() == document.revision()
        && checkpoint.configuration_generation() == view.configuration_generation()
        && checkpoint.measurement_environment_id() == provider.measurement_environment_id
        && checkpoint.metrics_generation() == provider.metrics_generation
        && checkpoint.hard_line_range() == *hard_line
        && hard_line.start <= checkpoint.next_text_offset()
        && checkpoint.next_text_offset() < hard_line.end;
    if valid {
        Ok(())
    } else {
        Err(LayoutJobError::InvalidLongLineCheckpoint(
            "checkpoint identities or hard-line extent are stale",
        ))
    }
}

fn bounded_long_line_work_end(
    document: &Document,
    start: usize,
    hard_line_end: usize,
    paragraph_flow: bool,
    cancellation: &LayoutCancellationToken,
) -> Result<usize, LayoutJobError> {
    let bounded_end = grapheme_bounded_long_line_work_end(document, start, hard_line_end)?;
    if bounded_end == hard_line_end {
        return Ok(bounded_end);
    }
    let first_break = super::line_breaks::first_line_break(
        document.projection().text_tree(), start..hard_line_end, paragraph_flow, cancellation,
    )?;
    // Ordinary text keeps a bounded work slice. An oversized first word must
    // reach its actual break so the worker can publish a complete overflow row.
    Ok(bounded_end.max(first_break))
}

fn grapheme_bounded_long_line_work_end(
    document: &Document,
    start: usize,
    hard_line_end: usize,
) -> Result<usize, LayoutJobError> {
    let remaining = hard_line_end.saturating_sub(start);
    if remaining <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES {
        return Ok(hard_line_end);
    }
    let target = start + MAX_LONG_LINE_LAYOUT_SLICE_BYTES;
    let mut probe_end = target.saturating_add(4).min(hard_line_end);
    let tree = document.projection().text_tree();
    while probe_end > target && !tree.is_char_boundary(probe_end)? {
        probe_end -= 1;
    }
    let probe = tree.slice(start..probe_end)?;
    for (offset, _) in probe.grapheme_indices(true).rev() {
        let candidate = start + offset;
        if candidate > start && candidate <= target && document.text_point(candidate).is_ok() {
            return Ok(candidate);
        }
    }
    // A single legal grapheme may be larger than the configured byte budget.
    // It is indivisible, so retaining it whole is the only correct progress.
    document
        .next_grapheme_boundary(start)
        .filter(|next| *next <= hard_line_end)
        .ok_or(LayoutJobError::InvalidLongLineCheckpoint(
            "no legal grapheme boundary advances the long-line slice",
        ))
}

fn bounded_context_start(document: &Document, work_start: usize, hard_line_start: usize) -> usize {
    let mut start = work_start;
    while work_start.saturating_sub(start) < LONG_LINE_CAPTURE_CONTEXT_BYTES {
        let Some(previous) = document.previous_grapheme_boundary(start) else {
            break;
        };
        if previous < hard_line_start {
            break;
        }
        start = previous;
        if start == hard_line_start {
            break;
        }
    }
    start
}

fn bounded_context_end(document: &Document, work_end: usize, hard_line_end: usize) -> usize {
    let mut end = work_end;
    while end.saturating_sub(work_end) < LONG_LINE_CAPTURE_CONTEXT_BYTES {
        let Some(next) = document.next_grapheme_boundary(end) else {
            break;
        };
        if next > hard_line_end {
            break;
        }
        end = next;
        if end == hard_line_end {
            break;
        }
    }
    end
}

/// Find the end of an ASCII space/tab prefix without copying its text. Large
/// immutable leaves still check cancellation after at most 4096 consumed bytes.
pub(crate) fn ascii_indentation_end(
    tree: &FormattedTextTree,
    range: Range<usize>,
    cancellation: &dyn LayoutCancellationProbe,
) -> Result<usize, LayoutJobError> {
    if range.start > range.end || range.end > tree.byte_len() {
        return Err(LayoutJobError::InvalidRegion(
            "indentation scan requires an ordered text range",
        ));
    }
    let mut at = range.start;
    while at < range.end {
        let chunk = tree.byte_chunk_at(at);
        let chunk = &chunk[..chunk.len().min(range.end - at)];
        for batch in chunk.chunks(4096) {
            if cancellation.is_cancelled() {
                return Err(LayoutJobError::Cancelled);
            }
            if let Some(offset) = batch.iter().position(|byte| !matches!(byte, b' ' | b'\t')) {
                return Ok(at + offset);
            }
            at += batch.len();
        }
    }
    if cancellation.is_cancelled() {
        return Err(LayoutJobError::Cancelled);
    }
    Ok(at)
}

/// Capture a revision-bound request and make it the only current layout job
/// for this view. Replacing an older request is O(1); its scheduler-owned token
/// should also be cancelled so retained inputs are released promptly.
#[allow(clippy::too_many_arguments)]
pub fn prepare_layout_job(
    document: &Document,
    view: &mut ViewLayout,
    provider_requirements: LayoutProviderRequirements,
    job_id: LayoutJobId,
    priority: LayoutJobPriority,
    region: LayoutJobRegion,
    cancellation: LayoutCancellationToken,
) -> Result<LayoutJobRequest, LayoutJobError> {
    prepare_layout_job_with_registration_hooks(
        document,
        view,
        provider_requirements,
        job_id,
        priority,
        region,
        cancellation,
        || {},
        || {},
    )
}

/// The callbacks are deterministic test seams around registration. Production
/// callers always use the no-op wrapper above.
#[allow(clippy::too_many_arguments)]
fn prepare_layout_job_with_registration_hooks<F, G>(
    document: &Document,
    view: &mut ViewLayout,
    provider_requirements: LayoutProviderRequirements,
    job_id: LayoutJobId,
    priority: LayoutJobPriority,
    region: LayoutJobRegion,
    cancellation: LayoutCancellationToken,
    before_registration: F,
    after_registration: G,
) -> Result<LayoutJobRequest, LayoutJobError>
where
    F: FnOnce(),
    G: FnOnce(),
{
    if cancellation.is_cancelled() {
        return Err(LayoutJobError::Cancelled);
    }
    let newest_job = view
        .active_layout_job()
        .into_iter()
        .chain(view.last_installed_layout_job())
        .max();
    if newest_job.is_some_and(|newest| job_id <= newest) {
        return Err(LayoutJobError::NonMonotonicJobId {
            requested: job_id,
            newest: newest_job,
        });
    }
    let hard_line_count = document
        .projection()
        .presentation_line_count(view.paragraph_flow());
    let requested_lines = region.hard_lines();
    if requested_lines.end > hard_line_count {
        return Err(LayoutJobError::RegionOutsideDocument { hard_line_count });
    }

    let range = requested_lines;
    let line_ranges = (range.clone())
        .map(|hard_line| {
            if cancellation.is_cancelled() {
                return Err(LayoutJobError::Cancelled);
            }
            document_line_range(document, hard_line, view.paragraph_flow())
        })
        .collect::<Result<Vec<_>, LayoutJobError>>()?;
    let checkpoint = match &region {
        LayoutJobRegion::Viewport(viewport) => viewport.long_line_checkpoint().cloned(),
        LayoutJobRegion::HardLines(_) => None,
    };
    if checkpoint.is_some() && (!view.wrap() || line_ranges.len() != 1) {
        return Err(LayoutJobError::InvalidLongLineCheckpoint(
            "a continuation requires one wrapped viewport hard line",
        ));
    }
    let use_long_line_slice = line_ranges.len() == 1
        && view.wrap()
        && (checkpoint.is_some() || line_ranges[0].len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES);

    // Multi-line viewport capture shares the tree whenever one line is
    // enormous; the worker wraps its ordinary lines and streams oversized rows.
    // Single wrapped-line continuations retain the explicit resumable protocol.
    let giant_line = line_ranges.iter().any(|line| line.len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
    let stream_wrapped_region = view.wrap() && checkpoint.is_none()
        && (line_ranges.len() > 1 || (giant_line
            && super::line_breaks::first_line_break(document.projection().text_tree(), line_ranges[0].clone(), false, &cancellation)? == line_ranges[0].end));
    let use_unwrapped_viewport = (!view.wrap() || stream_wrapped_region) && !view.paragraph_flow()
        && matches!(&region, LayoutJobRegion::Viewport(viewport) if !viewport.complete_horizontal_geometry)
        && giant_line;
    let (input, mut captured_view) = if use_unwrapped_viewport {
        let text_origin = line_ranges.first().unwrap().start;
        let text_end = line_ranges.last().unwrap().end;
        let following_line_range = if range.end < hard_line_count {
            Some(document_line_range(document, range.end, false)?)
        } else { None };
        let style_end = following_style_end(document, following_line_range.as_ref(), text_end)?;
        let mut styles = DocumentLayoutStyles::resolve_region(document.projection(), text_origin..style_end)?;
        styles.apply_source_quote_policy(document.format(), false);
        let styles = retain_regional_styles(styles, &line_ranges, following_line_range.as_ref());
        (
            CapturedLayoutInput::UnwrappedViewport { text: document.projection().text_tree().clone(), line_ranges,
                following_line_range, document_hard_line_count: hard_line_count, styles: Arc::new(styles) },
            view.capture_for_regional_layout_job(text_origin..text_end),
        )
    } else if use_long_line_slice {
        let full_range = line_ranges[0].clone();
        if let Some(checkpoint) = &checkpoint {
            validate_long_line_checkpoint(
                document,
                view,
                provider_requirements,
                &full_range,
                checkpoint,
            )?;
        }
        let work_start = checkpoint
            .as_ref()
            .map_or(full_range.start, LongLineLayoutCheckpoint::next_text_offset);
        if work_start >= full_range.end {
            return Err(LayoutJobError::InvalidLongLineCheckpoint(
                "the continuation is already at the hard-line end",
            ));
        }
        let work_end = bounded_long_line_work_end(
            document, work_start, full_range.end, view.paragraph_flow(), &cancellation,
        )?;
        let context_start = bounded_context_start(document, work_start, full_range.start);
        let context_end = bounded_context_end(document, work_end, full_range.end);
        let capture_range = context_start..context_end;
        let stream_overflow = !view.paragraph_flow()
            && matches!(&region, LayoutJobRegion::Viewport(viewport) if !viewport.complete_horizontal_geometry)
            && work_end - work_start > MAX_LONG_LINE_LAYOUT_SLICE_BYTES
            && super::line_breaks::first_line_break(document.projection().text_tree(), work_start..full_range.end, false, &cancellation)? == work_end;
        let text = if stream_overflow { None } else {
            Some(document.projection().text_tree().slice(capture_range.clone())?)
        };
        let following_line_range = if (work_end == full_range.end || stream_overflow) && range.end < hard_line_count {
            Some(document_line_range(
                document,
                range.end,
                view.paragraph_flow(),
            )?)
        } else {
            None
        };
        // The rare width-fit fallback can complete the remaining paragraph.
        // Retain its immutable style metadata, while the text stays in the tree.
        let indentation_tree = (checkpoint.is_none()
            && document.format().is_code()
            && view.wrap()
            && work_end < full_range.end)
            .then(|| document.projection().text_tree().clone());
        let mut style_capture_range = if stream_overflow { context_start..full_range.end } else { capture_range.clone() };
        if let Some(tree) = &indentation_tree {
            // Initial slices entirely inside a giant indentation prefix need
            // all of its styles for measurement, but retain only bounded text.
            let indentation_end = ascii_indentation_end(tree, full_range.clone(), &cancellation)?;
            style_capture_range.end = style_capture_range.end.max(indentation_end);
        }
        let style_end = following_style_end(document, following_line_range.as_ref(), style_capture_range.end)?;
        if cancellation.is_cancelled() {
            return Err(LayoutJobError::Cancelled);
        }
        let mut styles =
            DocumentLayoutStyles::resolve_region_with_flow(document.projection(), context_start..style_end, view.paragraph_flow())?;
        styles.apply_source_quote_policy(document.format(), view.paragraph_flow());
        if cancellation.is_cancelled() {
            return Err(LayoutJobError::Cancelled);
        }
        let (text, styles) = if view.paragraph_flow() {
            let mut styles = styles;
            let mut style_ranges = vec![full_range.clone()];
            style_ranges.extend(following_line_range.clone());
            resolve_flow_paragraph_styles(document.projection(), &mut styles, &style_ranges)?;
            (
                Some(flow_text(text.expect("flow uses captured text"), context_start, std::slice::from_ref(&full_range))),
                styles,
            )
        } else {
            (text, styles)
        };
        let styles = retain_regional_styles(
            styles,
            std::slice::from_ref(&style_capture_range),
            following_line_range.as_ref(),
        );
        let captured_view = view.capture_for_regional_layout_job(style_capture_range);
        let line_slice = HardLineLayoutSlice {
            indentation_tree,
            full_range,
            work_range: work_start..work_end,
            shaping_context_range: capture_range,
            hard_line_index: range.start,
            checkpoint,
        };
        let input = if stream_overflow {
            CapturedLayoutInput::StreamingOverflowSlice {
                tree: document.projection().text_tree().clone(),
                line_slice,
                document_hard_line_count: hard_line_count,
                following_line_range,
                styles: Arc::new(styles),
            }
        } else {
            CapturedLayoutInput::LongHardLineSlice {
                text: Arc::from(text.expect("ordinary slice captured its text")),
                text_origin: context_start,
                line_slice,
                document_hard_line_count: hard_line_count,
                following_line_range,
                styles: Arc::new(styles),
            }
        };
        (input, captured_view)
    } else {
        let text_origin = line_ranges
            .first()
            .expect("a validated job region is nonempty")
            .start;
        let text_end = line_ranges
            .last()
            .expect("a validated job region is nonempty")
            .end;
        let text = document
            .projection()
            .text_tree()
            .slice(text_origin..text_end)?;
        let following_line_range = if range.end < hard_line_count {
            Some(document_line_range(
                document,
                range.end,
                view.paragraph_flow(),
            )?)
        } else {
            None
        };
        // Resolve document-root values once, but visit only the requested
        // blocks, intersecting spans, and one following paragraph needed to
        // determine the requested region's trailing spacing.
        let style_end = following_style_end(document, following_line_range.as_ref(), text_end)?;
        if cancellation.is_cancelled() {
            return Err(LayoutJobError::Cancelled);
        }
        let mut styles =
            DocumentLayoutStyles::resolve_region_with_flow(document.projection(), text_origin..style_end, view.paragraph_flow())?;
        styles.apply_source_quote_policy(document.format(), view.paragraph_flow());
        if cancellation.is_cancelled() {
            return Err(LayoutJobError::Cancelled);
        }
        let (text, styles) = if view.paragraph_flow() {
            let mut styles = styles;
            let mut ranges = line_ranges.clone();
            ranges.extend(following_line_range.clone());
            resolve_flow_paragraph_styles(document.projection(), &mut styles, &ranges)?;
            (flow_text(text, text_origin, &line_ranges), styles)
        } else {
            (text, styles)
        };
        let styles = retain_regional_styles(styles, &line_ranges, following_line_range.as_ref());
        let captured_view = view.capture_for_regional_layout_job(text_origin..text_end);
        (
            CapturedLayoutInput::Regional {
                text: Arc::from(text),
                text_origin,
                line_ranges,
                following_line_range,
                document_hard_line_count: hard_line_count,
                styles: Arc::new(styles),
            },
            captured_view,
        )
    };
    if let LayoutJobRegion::Viewport(viewport) = &region {
        let prefix = view.hard_line_prefix_height(range.start).map_or(0.0, |height| height.height() as f32);
        captured_view.regional_viewport_top = (viewport.viewport_top - prefix).max(0.0);
        captured_view.horizontal_focus_offset = viewport.horizontal_focus_offset;
        captured_view.horizontal_desired_x = viewport.horizontal_desired_x;
    }
    before_registration();
    // This CAS is the preparation acceptance point. If cancellation happened
    // while capture was running, it wins the token's atomic modification order
    // and the existing view job remains current. Once registration wins, a
    // later cancellation legitimately applies to this accepted request.
    cancellation.try_register().map_err(|error| match error {
        CancellationRegistrationError::Cancelled => LayoutJobError::Cancelled,
        CancellationRegistrationError::AlreadyRegistered => {
            LayoutJobError::CancellationTokenAlreadyRegistered
        }
    })?;
    let registered = view.begin_layout_job(job_id);
    debug_assert!(
        registered,
        "the serial view job precondition was validated before capture"
    );
    if !registered {
        return Err(LayoutJobError::NonMonotonicJobId {
            requested: job_id,
            newest: newest_job,
        });
    }
    after_registration();

    Ok(LayoutJobRequest {
        job_id,
        priority,
        document_id: document.id(),
        document_revision: document.revision(),
        configuration_generation: view.configuration_generation(),
        provider_requirements,
        region,
        cancellation,
        projection_text_len: document.projection().text_tree().byte_len(),
        input,
        captured_view,
    })
}

/// The worker products are intentionally distinct so a hard-line cache fill
/// cannot accidentally replace the visible viewport snapshot.
#[derive(Debug)]
pub enum LayoutJobProduct {
    /// Exact rows and heights for only the requested hard-line range.
    RegionalHardLines(RegionalLayoutSnapshot),
    /// Exact rows and heights used to assemble a coverage-bounded visible
    /// snapshot at installation time.
    PartialViewport(RegionalLayoutSnapshot),
}

/// Immutable worker result. Only `install_layout_job` can consume its private
/// product and publish it to a view.
#[derive(Debug)]
pub struct LayoutJobCandidate {
    job_id: LayoutJobId,
    priority: LayoutJobPriority,
    document_id: DocumentId,
    document_revision: Revision,
    configuration_generation: super::engine::ViewConfigurationGeneration,
    measurement_environment_id: MeasurementEnvironmentId,
    metrics_generation: MetricsGeneration,
    requested_region: LayoutJobRegion,
    computation_scope: LayoutComputationScope,
    cancellation: LayoutCancellationToken,
    product: LayoutJobProduct,
}

impl LayoutJobCandidate {
    pub fn job_id(&self) -> LayoutJobId {
        self.job_id
    }

    pub fn priority(&self) -> LayoutJobPriority {
        self.priority
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn configuration_generation(&self) -> super::engine::ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn metrics_generation(&self) -> MetricsGeneration {
        self.metrics_generation
    }

    pub fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    pub fn requested_region(&self) -> &LayoutJobRegion {
        &self.requested_region
    }

    pub fn computation_scope(&self) -> LayoutComputationScope {
        self.computation_scope
    }

    pub fn product(&self) -> &LayoutJobProduct {
        &self.product
    }

    /// The bounded worker result for either computation kind.
    pub fn regional_snapshot(&self) -> &RegionalLayoutSnapshot {
        match &self.product {
            LayoutJobProduct::RegionalHardLines(snapshot)
            | LayoutJobProduct::PartialViewport(snapshot) => snapshot,
        }
    }

    pub fn next_long_line_checkpoint(&self) -> Option<&LongLineLayoutCheckpoint> {
        self.regional_snapshot().next_long_line_checkpoint()
    }

    pub(crate) fn retain_viewport_tail(&mut self, previous: Option<&RegionalLayoutSnapshot>) {
        match &mut self.product {
            LayoutJobProduct::RegionalHardLines(region)
            | LayoutJobProduct::PartialViewport(region) => {
                region.retain_viewport_tail(previous);
            }
        }
    }

    pub(crate) fn append_adjacent_region(&mut self, following: &RegionalLayoutSnapshot) {
        match &mut self.product {
            LayoutJobProduct::RegionalHardLines(region)
            | LayoutJobProduct::PartialViewport(region) => region.append_adjacent_region(following),
        }
    }

    pub(crate) fn prepend_adjacent_region(&mut self, preceding: &RegionalLayoutSnapshot) {
        match &mut self.product {
            LayoutJobProduct::RegionalHardLines(region)
            | LayoutJobProduct::PartialViewport(region) => {
                region.prepend_adjacent_region(preceding);
            }
        }
    }

    pub(crate) fn prepend_long_line_slice(&mut self, preceding: &RegionalLayoutSnapshot) {
        match &mut self.product {
            LayoutJobProduct::RegionalHardLines(region)
            | LayoutJobProduct::PartialViewport(region) => region.prepend_long_line_slice(preceding),
        }
    }

    pub(crate) fn append_following_viewport_tail(
        &mut self,
        following: Option<&RegionalLayoutSnapshot>,
    ) {
        match &mut self.product {
            LayoutJobProduct::RegionalHardLines(region)
            | LayoutJobProduct::PartialViewport(region) => {
                region.append_following_viewport_tail(following);
            }
        }
    }
}

/// Compute using only captured values. No mutable document or live view is
/// reachable here, and provider callbacks occur outside any core lock. Both
/// request kinds are strict shaping/wrapping work bounds. Cooperative
/// checkpoints bound cancellation latency in either path.
pub fn compute_layout_job<P: TextMeasurementProvider>(
    engine: &mut LayoutEngine<P>,
    request: &LayoutJobRequest,
    execution_context: LayoutExecutionContext,
) -> Result<LayoutJobCandidate, LayoutJobError> {
    if request.cancellation.is_cancelled() {
        return Err(LayoutJobError::Cancelled);
    }
    if !execution_context.permits(request.provider_requirements.threading) {
        return Err(LayoutJobError::WrongExecutionContext {
            required: request.provider_requirements.threading,
            actual: execution_context,
        });
    }
    let actual_requirements = inspect_layout_provider(engine);
    if actual_requirements.threading != request.provider_requirements.threading {
        return Err(LayoutJobError::ProviderThreadingChanged {
            expected: request.provider_requirements.threading,
            actual: actual_requirements.threading,
        });
    }
    if actual_requirements.measurement_environment_id
        != request.provider_requirements.measurement_environment_id
    {
        return Err(LayoutJobError::WrongMeasurementEnvironment {
            expected: request.provider_requirements.measurement_environment_id,
            actual: actual_requirements.measurement_environment_id,
        });
    }
    if actual_requirements.metrics_generation != request.provider_requirements.metrics_generation {
        return Err(LayoutJobError::StaleMetrics {
            expected: request.provider_requirements.metrics_generation,
            actual: actual_requirements.metrics_generation,
        });
    }

    let snapshot = match &request.input {
        CapturedLayoutInput::UnwrappedViewport { text, line_ranges, following_line_range, document_hard_line_count, styles } => {
            match engine.layout_unwrapped_viewport_cancellable(request.document_id, request.document_revision, text, line_ranges,
                request.region.hard_lines().start, *document_hard_line_count, following_line_range.clone(), styles,
                &request.captured_view, &request.cancellation) {
                Ok(snapshot) => snapshot,
                Err(LayoutComputationError::Cancelled) => return Err(LayoutJobError::Cancelled),
                Err(LayoutComputationError::Layout(error)) => return Err(LayoutJobError::Layout(error)),
            }
        }
        CapturedLayoutInput::StreamingOverflowSlice { tree, line_slice, document_hard_line_count, following_line_range, styles } => {
            match engine.layout_overflow_slice_cancellable(request.document_id, request.document_revision, tree, line_slice,
                *document_hard_line_count, following_line_range.clone(), styles, &request.captured_view, &request.cancellation) {
                Ok(snapshot) => snapshot,
                Err(LayoutComputationError::Cancelled) => return Err(LayoutJobError::Cancelled),
                Err(LayoutComputationError::Layout(error)) => return Err(LayoutJobError::Layout(error)),
            }
        }
        CapturedLayoutInput::Regional {
            text,
            text_origin,
            line_ranges,
            following_line_range,
            document_hard_line_count,
            styles,
        } => {
            match engine.layout_hard_line_region_cancellable(
                request.document_id,
                request.document_revision,
                text,
                *text_origin,
                line_ranges,
                request.region.hard_lines().start,
                *document_hard_line_count,
                request.projection_text_len,
                following_line_range.clone(),
                styles,
                &request.captured_view,
                &request.cancellation,
            ) {
                Ok(snapshot) => snapshot,
                Err(LayoutComputationError::Cancelled) => return Err(LayoutJobError::Cancelled),
                Err(LayoutComputationError::Layout(error)) => {
                    return Err(LayoutJobError::Layout(error));
                }
            }
        }
        CapturedLayoutInput::LongHardLineSlice {
            text,
            text_origin,
            line_slice,
            document_hard_line_count,
            following_line_range,
            styles,
        } => {
            match engine.layout_hard_line_slices_cancellable(
                request.document_id,
                request.document_revision,
                text,
                *text_origin,
                std::slice::from_ref(line_slice),
                *document_hard_line_count,
                request.projection_text_len,
                following_line_range.clone(),
                styles,
                &request.captured_view,
                &request.cancellation,
            ) {
                Ok(snapshot) => snapshot,
                Err(LayoutComputationError::Cancelled) => return Err(LayoutJobError::Cancelled),
                Err(LayoutComputationError::Layout(error)) => {
                    return Err(LayoutJobError::Layout(error));
                }
            }
        }
    };
    if snapshot.measurement_environment_id()
        != request.provider_requirements.measurement_environment_id
    {
        return Err(LayoutJobError::WrongMeasurementEnvironment {
            expected: request.provider_requirements.measurement_environment_id,
            actual: snapshot.measurement_environment_id(),
        });
    }
    if snapshot.metrics_generation() != request.provider_requirements.metrics_generation {
        return Err(LayoutJobError::StaleMetrics {
            expected: request.provider_requirements.metrics_generation,
            actual: snapshot.metrics_generation(),
        });
    }
    let (computation_scope, product) = match request.region {
        LayoutJobRegion::HardLines(_) => (
            LayoutComputationScope::RegionalHardLines,
            LayoutJobProduct::RegionalHardLines(snapshot),
        ),
        LayoutJobRegion::Viewport(_) => (
            LayoutComputationScope::PartialViewport,
            LayoutJobProduct::PartialViewport(snapshot),
        ),
    };
    if request.cancellation.is_cancelled() {
        return Err(LayoutJobError::Cancelled);
    }

    Ok(LayoutJobCandidate {
        job_id: request.job_id,
        priority: request.priority,
        document_id: request.document_id,
        document_revision: request.document_revision,
        configuration_generation: request.configuration_generation,
        measurement_environment_id: request.provider_requirements.measurement_environment_id,
        metrics_generation: request.provider_requirements.metrics_generation,
        requested_region: request.region.clone(),
        computation_scope,
        cancellation: request.cancellation.clone(),
        product,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutInstallTarget {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub measurement_environment_id: MeasurementEnvironmentId,
    pub metrics_generation: MetricsGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledLayoutJob {
    pub job_id: LayoutJobId,
    pub layout_revision: LayoutRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutJobInstallRejection {
    Cancelled,
    OutsideHorizontalCoverage,
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    StaleDocumentRevision {
        expected: Revision,
        actual: Revision,
    },
    StaleConfiguration {
        expected: super::engine::ViewConfigurationGeneration,
        actual: super::engine::ViewConfigurationGeneration,
    },
    WrongMeasurementEnvironment {
        expected: MeasurementEnvironmentId,
        actual: MeasurementEnvironmentId,
    },
    StaleMetrics {
        expected: MetricsGeneration,
        actual: MetricsGeneration,
    },
    Superseded {
        installed: LayoutJobId,
        candidate: LayoutJobId,
    },
    NotCurrentJob {
        expected: Option<LayoutJobId>,
        actual: LayoutJobId,
    },
    HeightIndex(super::height_index::ViewHeightIndexError),
}

/// Validate every dependency before performing the sole view mutation. Any
/// rejection leaves the installed snapshot and presentation state unchanged.
/// The start of this coordinator call is the cancellation linearization point:
/// cancellation observed later races after an already accepted result.
pub fn install_layout_job(
    view: &mut ViewLayout,
    target: LayoutInstallTarget,
    candidate: LayoutJobCandidate,
) -> Result<InstalledLayoutJob, LayoutJobInstallRejection> {
    if candidate.cancellation.is_cancelled() {
        return Err(LayoutJobInstallRejection::Cancelled);
    }
    if candidate.document_id != target.document_id {
        return Err(LayoutJobInstallRejection::WrongDocument {
            expected: target.document_id,
            actual: candidate.document_id,
        });
    }
    if candidate.document_revision != target.document_revision {
        return Err(LayoutJobInstallRejection::StaleDocumentRevision {
            expected: target.document_revision,
            actual: candidate.document_revision,
        });
    }
    let current_configuration = view.configuration_generation();
    if candidate.configuration_generation != current_configuration {
        return Err(LayoutJobInstallRejection::StaleConfiguration {
            expected: current_configuration,
            actual: candidate.configuration_generation,
        });
    }
    if candidate.measurement_environment_id != target.measurement_environment_id {
        return Err(LayoutJobInstallRejection::WrongMeasurementEnvironment {
            expected: target.measurement_environment_id,
            actual: candidate.measurement_environment_id,
        });
    }
    if candidate.metrics_generation != target.metrics_generation {
        return Err(LayoutJobInstallRejection::StaleMetrics {
            expected: target.metrics_generation,
            actual: candidate.metrics_generation,
        });
    }
    if let Some(installed) = view.last_installed_layout_job() {
        if candidate.job_id <= installed {
            return Err(LayoutJobInstallRejection::Superseded {
                installed,
                candidate: candidate.job_id,
            });
        }
    }
    if view.active_layout_job() != Some(candidate.job_id) {
        return Err(LayoutJobInstallRejection::NotCurrentJob {
            expected: view.active_layout_job(),
            actual: candidate.job_id,
        });
    }
    let region = match &candidate.product {
        LayoutJobProduct::RegionalHardLines(region) | LayoutJobProduct::PartialViewport(region) => region,
    };
    if !region.covers_horizontal_viewport(view.viewport_left(), view.width()) {
        return Err(LayoutJobInstallRejection::OutsideHorizontalCoverage);
    }

    let job_id = candidate.job_id;
    let requested_viewport_top = match &candidate.requested_region {
        LayoutJobRegion::Viewport(region) => Some(region.viewport_top()),
        LayoutJobRegion::HardLines(_) => None,
    };
    let layout_revision = match candidate.product {
        LayoutJobProduct::RegionalHardLines(region) => view
            .publish_layout_job_region(job_id, region)
            .map_err(LayoutJobInstallRejection::HeightIndex)?,
        LayoutJobProduct::PartialViewport(region) => view
            .publish_layout_job_viewport(
                job_id,
                region,
                requested_viewport_top.expect("a viewport product has a viewport request"),
            )
            .map_err(LayoutJobInstallRejection::HeightIndex)?,
    };
    Ok(InstalledLayoutJob {
        job_id,
        layout_revision,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutJobError {
    Cancelled,
    CancellationTokenAlreadyRegistered,
    CancellationTokenAlreadyActive {
        job_id: LayoutJobId,
    },
    InvalidRegion(&'static str),
    InvalidLongLineCheckpoint(&'static str),
    RegionOutsideDocument {
        hard_line_count: usize,
    },
    InvalidDocumentLineIndex {
        hard_line: usize,
    },
    NonMonotonicJobId {
        requested: LayoutJobId,
        newest: Option<LayoutJobId>,
    },
    WrongExecutionContext {
        required: ProviderThreading,
        actual: LayoutExecutionContext,
    },
    ProviderThreadingChanged {
        expected: ProviderThreading,
        actual: ProviderThreading,
    },
    WrongMeasurementEnvironment {
        expected: MeasurementEnvironmentId,
        actual: MeasurementEnvironmentId,
    },
    StaleMetrics {
        expected: MetricsGeneration,
        actual: MetricsGeneration,
    },
    DocumentStyle(DocumentStyleError),
    FormattedText(FormattedTextError),
    Layout(LayoutError),
}

impl From<DocumentStyleError> for LayoutJobError {
    fn from(value: DocumentStyleError) -> Self {
        Self::DocumentStyle(value)
    }
}

impl From<FormattedTextError> for LayoutJobError {
    fn from(value: FormattedTextError) -> Self {
        Self::FormattedText(value)
    }
}

impl From<LayoutError> for LayoutJobError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{TextEdit, TextRange};
    use crate::layout::{
        ClusterCaretStop, MeasurementError, MockTextMeasurementProvider,
        RegionalLayoutCacheStatistics, RenderRunHandle, RenderRunPolicy, ShapeRequest,
        ShapedBounds, ShapedCluster, ShapedFragment, TextMetrics,
    };
    use std::sync::atomic::AtomicBool;

    #[derive(Clone, Debug)]
    struct InstrumentedProvider {
        inner: MockTextMeasurementProvider,
        threading: ProviderThreading,
        cancel_during_shape: Option<LayoutCancellationToken>,
        coordinator_lock_held: Arc<AtomicBool>,
        shape_calls: usize,
        shape_requests: usize,
        shaped_text_bytes: usize,
        maximum_request_bytes: usize,
        request_ranges: Vec<Range<usize>>,
        request_contexts: Vec<(String, String)>,
        request_default_sizes: Vec<f32>,
    }

    impl InstrumentedProvider {
        fn new(threading: ProviderThreading) -> Self {
            Self {
                inner: MockTextMeasurementProvider::new(),
                threading,
                cancel_during_shape: None,
                coordinator_lock_held: Arc::new(AtomicBool::new(false)),
                shape_calls: 0,
                shape_requests: 0,
                shaped_text_bytes: 0,
                maximum_request_bytes: 0,
                request_ranges: Vec::new(),
                request_contexts: Vec::new(),
                request_default_sizes: Vec::new(),
            }
        }
    }

    impl TextMeasurementProvider for InstrumentedProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.inner.measurement_environment_id()
        }

        fn metrics_generation(&self) -> MetricsGeneration {
            self.inner.metrics_generation()
        }

        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.inner.render_run_policy()
        }

        fn threading(&self) -> ProviderThreading {
            self.threading
        }

        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            assert!(
                !self.coordinator_lock_held.load(Ordering::Acquire),
                "provider was invoked while the fake coordinator lock was held"
            );
            self.shape_calls += 1;
            self.shape_requests += requests.len();
            self.shaped_text_bytes += requests
                .iter()
                .map(|request| request.text.len())
                .sum::<usize>();
            self.maximum_request_bytes = self.maximum_request_bytes.max(
                requests
                    .iter()
                    .map(|request| request.text.len())
                    .max()
                    .unwrap_or(0),
            );
            self.request_ranges
                .extend(requests.iter().map(|request| request.text_range.clone()));
            self.request_contexts.extend(requests.iter().map(|request| {
                (
                    request.context_before.to_owned(),
                    request.context_after.to_owned(),
                )
            }));
            self.request_default_sizes
                .extend(requests.iter().map(|request| request.default_style.size));
            if let Some(token) = &self.cancel_during_shape {
                token.cancel();
            }
            self.inner.shape_batch(requests)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct FragmentAggregateProvider {
        requests: usize,
        shaped_bytes: usize,
        maximum_request_bytes: usize,
    }

    impl TextMeasurementProvider for FragmentAggregateProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            MeasurementEnvironmentId::default()
        }

        fn metrics_generation(&self) -> MetricsGeneration {
            MetricsGeneration(1)
        }

        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            MockTextMeasurementProvider::new().render_run_policy()
        }

        fn shape_batch(
            &mut self,
            requests: &[ShapeRequest<'_>],
        ) -> Result<Vec<ShapedFragment>, MeasurementError> {
            self.requests += requests.len();
            self.shaped_bytes += requests
                .iter()
                .map(|request| request.text.len())
                .sum::<usize>();
            self.maximum_request_bytes = self.maximum_request_bytes.max(
                requests
                    .iter()
                    .map(|request| request.text.len())
                    .max()
                    .unwrap_or(0),
            );
            Ok(requests
                .iter()
                .enumerate()
                .map(|(index, request)| {
                    let metrics = TextMetrics {
                        ascent: 10.0,
                        descent: 3.0,
                        leading: 1.0,
                    };
                    let advance = request.text.chars().count() as f32;
                    let bounds = ShapedBounds {
                        x: 0.0,
                        y: -metrics.ascent,
                        width: advance,
                        height: metrics.ascent + metrics.descent,
                    };
                    let clusters = (!request.text.is_empty())
                        .then(|| ShapedCluster {
                            text_range: request.text_range.clone(),
                            advance,
                            metrics: metrics.clone(),
                            typographic_bounds: bounds,
                            ink_bounds: bounds,
                            bidi_level: 0,
                            fallback_font: "Aggregate LTR".into(),
                            caret_stops: vec![
                                ClusterCaretStop {
                                    text_offset: request.text_range.start,
                                    inline_offset: 0.0,
                                    affinity: super::super::BoundaryAffinity::Downstream,
                                },
                                ClusterCaretStop {
                                    text_offset: request.text_range.end,
                                    inline_offset: advance,
                                    affinity: super::super::BoundaryAffinity::Upstream,
                                },
                            ],
                            render_run: request.render_run_policy.map(|policy| RenderRunHandle::new(
                                policy.owner, index as u64 + 1,
                                request.metrics_generation, policy.threading)),
                        })
                        .into_iter()
                        .collect::<Vec<_>>();
                    ShapedFragment {
                        document_id: request.document_id,
                        document_revision: request.document_revision,
                        measurement_environment_id: request.measurement_environment_id,
                        metrics_generation: request.metrics_generation,
                        text_range: request.text_range.clone(),
                        visual_order: (0..clusters.len()).collect(),
                        clusters,
                        default_metrics: metrics,
                        diagnostics: Vec::new(),
                    }
                })
                .collect())
        }
    }

    fn hard_lines(range: Range<usize>) -> LayoutJobRegion {
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(range).unwrap())
    }

    fn viewport(range: Range<usize>, top: f32, height: f32) -> LayoutJobRegion {
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(range, top, height).unwrap())
    }

    fn assert_send_and_sync<T: Send + Sync>() {}

    fn target(document: &Document, metrics_generation: MetricsGeneration) -> LayoutInstallTarget {
        LayoutInstallTarget {
            document_id: document.id(),
            document_revision: document.revision(),
            measurement_environment_id: MeasurementEnvironmentId::default(),
            metrics_generation,
        }
    }

    #[test]
    fn region_validation_precedes_view_job_registration() {
        assert_send_and_sync::<LayoutJobRequest>();
        assert_send_and_sync::<LayoutJobCandidate>();
        assert_send_and_sync::<LayoutCancellationToken>();

        assert_eq!(
            HardLineLayoutRegion::new(2..2),
            Err(LayoutJobError::InvalidRegion(
                "hard-line range must be non-empty and ordered"
            ))
        );
        assert!(ViewportLayoutRegion::new(0..1, f32::NAN, 100.0).is_err());

        let document = Document::new("one\ntwo");
        let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let outside = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(1..3),
            LayoutCancellationToken::new(),
        );
        assert_eq!(
            outside.unwrap_err(),
            LayoutJobError::RegionOutsideDocument { hard_line_count: 2 }
        );

        // The failed preparation did not consume the job identity.
        assert!(prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .is_ok());
    }

    #[test]
    fn cancellation_during_capture_cannot_replace_the_current_job() {
        let document = Document::new("zero\none\ntwo");
        let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let current_token = LayoutCancellationToken::new();
        prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            viewport(0..1, 0.0, 100.0),
            current_token.clone(),
        )
        .unwrap();

        let replacement_token = LayoutCancellationToken::new();
        let token_to_cancel = replacement_token.clone();
        let replacement = prepare_layout_job_with_registration_hooks(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(1..2, 16.0, 100.0),
            replacement_token,
            move || token_to_cancel.cancel(),
            || {},
        );

        assert_eq!(replacement.unwrap_err(), LayoutJobError::Cancelled);
        assert_eq!(view.active_layout_job(), Some(LayoutJobId(1)));
        assert!(!current_token.is_cancelled());

        // The opposite atomic ordering is also explicit: once registration
        // wins, cancellation belongs to the newly accepted request rather than
        // rolling view identity backward.
        let accepted_token = LayoutCancellationToken::new();
        let token_to_cancel = accepted_token.clone();
        let accepted = prepare_layout_job_with_registration_hooks(
            &document,
            &mut view,
            requirements,
            LayoutJobId(3),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(1..2, 16.0, 100.0),
            accepted_token,
            || {},
            move || token_to_cancel.cancel(),
        )
        .unwrap();
        assert_eq!(accepted.job_id(), LayoutJobId(3));
        assert!(accepted.cancellation_token().is_cancelled());
        assert_eq!(view.active_layout_job(), Some(LayoutJobId(3)));

        assert_eq!(
            prepare_layout_job(
                &document,
                &mut view,
                requirements,
                LayoutJobId(4),
                LayoutJobPriority::Background,
                viewport(0..1, 0.0, 100.0),
                current_token,
            )
            .unwrap_err(),
            LayoutJobError::CancellationTokenAlreadyRegistered
        );
        assert_eq!(view.active_layout_job(), Some(LayoutJobId(3)));
    }

    #[test]
    fn hard_line_job_is_regional_and_preserves_an_installed_full_snapshot() {
        let document = Document::from_bytes(
            b"plain\n# Heading words\ntail".to_vec(),
            crate::document::Encoding::Utf8,
            crate::document::Format::Markdown,
        )
        .unwrap();
        let mut full_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(90.0, 100.0);
        full_engine.relayout(&document, &mut view).unwrap();
        let full_snapshot = view.snapshot().unwrap().clone();

        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(1..2),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let expected = document.line_start(1).unwrap()..document.line_end(1).unwrap();
        assert_eq!(request.captured_text_len(), expected.len());
        assert!(request.captured_text_len() < request.projection_text_len());

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(
            candidate.computation_scope(),
            LayoutComputationScope::RegionalHardLines
        );
        assert!(matches!(
            candidate.product(),
            LayoutJobProduct::RegionalHardLines(_)
        ));
        let regional = candidate.regional_snapshot();
        assert_eq!(regional.hard_lines(), 1..2);
        assert_eq!(regional.lines()[0].hard_line_range(), expected.clone());
        assert!(regional.lines()[0]
            .rows()
            .iter()
            .flat_map(|row| &row.clusters)
            .all(|cluster| expected.start <= cluster.text_range.start
                && cluster.text_range.end <= expected.end));
        let full_height = view.hard_line_range_height(1..2).unwrap().height();
        assert!((regional.lines()[0].height() - full_height).abs() < 1.0e-5);
        assert_eq!(engine.provider().request_ranges, vec![expected]);
        assert!(engine
            .provider()
            .request_default_sizes
            .iter()
            .all(|size| *size == 24.0));

        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        assert_eq!(view.snapshot(), Some(&full_snapshot));
        assert_eq!(view.regional_cached_ranges(), vec![1..2]);
        assert_eq!(view.regional_cache_statistics().hard_line_count(), 1);
        assert!(view.content_height().is_exact());
    }

    #[test]
    fn regional_unicode_line_breaks_match_full_layout() {
        let document = Document::new(
            "prefix\na\u{00a0}b ab\u{200b}cd \u{4e2d}\u{ff08}\u{6587} word,word\ntail",
        );
        let mut full_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut full_view = ViewLayout::new(75.0, 200.0);
        full_engine.relayout(&document, &mut full_view).unwrap();
        let full_ranges: Vec<_> = full_view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .filter(|row| row.hard_line_index == 1)
            .map(|row| row.text_range.clone())
            .collect();

        let mut regional_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&regional_engine);
        let mut regional_view = ViewLayout::new(75.0, 200.0);
        let request = prepare_layout_job(
            &document,
            &mut regional_view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(1..2),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate = compute_layout_job(
            &mut regional_engine,
            &request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        install_layout_job(
            &mut regional_view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        let regional_ranges: Vec<_> = regional_view
            .regional_hard_line_layout(1)
            .unwrap()
            .rows()
            .iter()
            .map(|row| row.text_range.clone())
            .collect();

        assert_eq!(regional_ranges, full_ranges);
    }

    #[test]
    fn mac_regional_job_keeps_literal_lf_inside_line_and_shaping_context() {
        let mut first_line = "a".repeat(MAX_SHAPE_FRAGMENT_BYTES - 8);
        first_line.push('\n');
        first_line.push_str(&"b".repeat(15));
        first_line.push('\n');
        first_line.push_str(&"c".repeat(64));
        let source = format!("{first_line}\rtail");
        let document = Document::from_bytes_with_file_format(
            source.into_bytes(),
            crate::document::Encoding::Utf8,
            crate::document::Format::PlainText,
            crate::document::FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.line_start(0), Some(0));
        assert_eq!(document.line_end(0), Some(first_line.len()));

        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(20_000.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request.captured_text_len(), first_line.len());

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let regional = candidate.regional_snapshot();
        assert_eq!(regional.hard_lines(), 0..1);
        assert_eq!(regional.lines()[0].hard_line_range(), 0..first_line.len());
        assert_eq!(engine.provider().request_ranges.len(), 2);
        assert_eq!(
            engine.provider().request_ranges,
            vec![
                0..MAX_SHAPE_FRAGMENT_BYTES,
                MAX_SHAPE_FRAGMENT_BYTES..first_line.len()
            ]
        );

        let first_context_after = &engine.provider().request_contexts[0].1;
        let second_context_before = &engine.provider().request_contexts[1].0;
        assert_eq!(first_context_after.len(), 32);
        assert_eq!(second_context_before.len(), 32);
        assert!(first_context_after.contains('\n'));
        assert!(second_context_before.contains('\n'));
        assert!(regional.lines()[0]
            .rows()
            .iter()
            .flat_map(|row| &row.clusters)
            .any(|cluster| {
                cluster.text_range == (MAX_SHAPE_FRAGMENT_BYTES - 8..MAX_SHAPE_FRAGMENT_BYTES - 7)
            }));
    }

    #[test]
    fn regional_work_is_independent_of_a_million_line_document() {
        const DOCUMENT_LINES: usize = 1_000_000;
        const FIRST_LINE: usize = 750_000;
        let document = Document::new("fixture");
        let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        styles.shaping_runs.clear();
        styles.paint_runs.clear();
        styles.paragraphs.clear();
        let origin = 1_500_000usize;
        let mut view = ViewLayout::new(100.0, 100.0);
        assert!(view.begin_layout_job(LayoutJobId(1)));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let request = LayoutJobRequest {
            job_id: LayoutJobId(1),
            priority: LayoutJobPriority::Background,
            document_id: document.id(),
            document_revision: document.revision(),
            configuration_generation: view.configuration_generation(),
            provider_requirements: requirements,
            region: hard_lines(FIRST_LINE..FIRST_LINE + 3),
            cancellation: LayoutCancellationToken::new(),
            projection_text_len: DOCUMENT_LINES * 2 - 1,
            input: CapturedLayoutInput::Regional {
                text: Arc::from("x\nx\nx"),
                text_origin: origin,
                line_ranges: vec![
                    origin..origin + 1,
                    origin + 2..origin + 3,
                    origin + 4..origin + 5,
                ],
                following_line_range: Some(origin + 6..origin + 7),
                document_hard_line_count: DOCUMENT_LINES,
                styles: Arc::new(styles),
            },
            captured_view: view.capture_for_layout_job(),
        };

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(request.captured_text_len(), 5);
        assert_eq!(engine.provider().shape_requests, 3);
        assert_eq!(engine.provider().shaped_text_bytes, 3);
        assert_eq!(candidate.regional_snapshot().lines().len(), 3);
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        assert_eq!(
            view.height_index_statistics().hard_line_count(),
            DOCUMENT_LINES
        );
        assert!(view.height_index_statistics().run_count() <= 3);
        assert_eq!(
            view.regional_cached_ranges(),
            vec![FIRST_LINE..FIRST_LINE + 3]
        );
        assert!(!view.content_height().is_exact());
    }

    #[test]
    fn million_line_viewport_retains_and_shapes_only_materialized_lines() {
        const DOCUMENT_LINES: usize = 1_000_000;
        const FIRST_LINE: usize = 750_000;
        let document = Document::new("fixture");
        let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        styles.shaping_runs.clear();
        styles.paint_runs.clear();
        styles.paragraphs.clear();
        let origin = 1_500_000usize;
        let mut view = ViewLayout::new(100.0, 48.0);
        assert!(view.begin_layout_job(LayoutJobId(1)));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let request = LayoutJobRequest {
            job_id: LayoutJobId(1),
            priority: LayoutJobPriority::ViewportOverscan,
            document_id: document.id(),
            document_revision: document.revision(),
            configuration_generation: view.configuration_generation(),
            provider_requirements: requirements,
            region: viewport(FIRST_LINE..FIRST_LINE + 3, FIRST_LINE as f32 * 16.0, 48.0),
            cancellation: LayoutCancellationToken::new(),
            projection_text_len: DOCUMENT_LINES * 2 - 1,
            input: CapturedLayoutInput::Regional {
                text: Arc::from("x\nx\nx"),
                text_origin: origin,
                line_ranges: vec![
                    origin..origin + 1,
                    origin + 2..origin + 3,
                    origin + 4..origin + 5,
                ],
                following_line_range: Some(origin + 6..origin + 7),
                document_hard_line_count: DOCUMENT_LINES,
                styles: Arc::new(styles),
            },
            captured_view: view.capture_for_regional_layout_job(origin..origin + 5),
        };

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(
            candidate.computation_scope(),
            LayoutComputationScope::PartialViewport
        );
        assert_eq!(request.captured_text_len(), 5);
        assert_eq!(engine.provider().shape_requests, 3);
        assert_eq!(engine.provider().shaped_text_bytes, 3);
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 3);
        assert_eq!(snapshot.coverage.hard_lines(), FIRST_LINE..FIRST_LINE + 3);
        assert_eq!(snapshot.coverage.document_hard_line_count(), DOCUMENT_LINES);
        assert_eq!(snapshot.rows[0].hard_line_index, FIRST_LINE);
        assert_eq!(snapshot.rows[0].text_range, origin..origin + 1);
        assert_eq!(snapshot.rows[0].y, FIRST_LINE as f32 * 16.0);
        assert_eq!(
            view.height_index_statistics().hard_line_count(),
            DOCUMENT_LINES
        );
        assert!(view.height_index_statistics().run_count() <= 3);
        assert!(!view.content_height().is_exact());
    }

    #[test]
    fn partial_viewport_uses_global_offsets_and_rejects_uncovered_geometry() {
        let document = Document::new("zero\none two\nthree");
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(200.0, 20.0);
        let expected = document.line_start(1).unwrap()..document.line_end(1).unwrap();
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(1..2, 16.0, 20.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        let snapshot = view.snapshot().unwrap();
        assert_eq!(engine.provider().request_ranges, vec![expected.clone()]);
        assert_eq!(snapshot.coverage.hard_lines(), 1..2);
        assert!(!snapshot.coverage.prefix_is_exact());
        assert_eq!(snapshot.rows[0].hard_line_index, 1);
        assert_eq!(snapshot.rows[0].hard_line_range, expected);
        assert_eq!(
            snapshot.rows[0].clusters[0].text_range.start,
            expected.start
        );
        let point = snapshot
            .caret_point(expected.start, super::super::BoundaryAffinity::Downstream)
            .unwrap();
        let hit = snapshot
            .hit_test(super::super::LayoutPoint {
                x: snapshot.rows[0].carets[0].x,
                y: snapshot.rows[0].y + 1.0,
            })
            .unwrap();
        assert_eq!(hit.text_offset, point.text_offset);
        assert_eq!(
            snapshot.caret_point(0, super::super::BoundaryAffinity::Downstream),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
        let covered = TextRange::new(
            document.text_point(expected.start).unwrap(),
            document.text_point(expected.end).unwrap(),
        )
        .unwrap();
        assert!(!snapshot
            .selection_rectangles(covered, super::super::BoundaryAffinity::Downstream)
            .unwrap()
            .is_empty());
        let uncovered = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(1).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot.selection_rectangles(uncovered, super::super::BoundaryAffinity::Downstream),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
        let crossing = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(expected.start + 1).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot.selection_rectangles(crossing, super::super::BoundaryAffinity::Downstream),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
        let coverage = snapshot.coverage.vertical_range().unwrap();
        assert_eq!(
            snapshot.hit_test(super::super::LayoutPoint {
                x: 0.0,
                y: coverage.start - 1.0,
            }),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
    }

    #[test]
    fn partial_viewport_materializes_an_empty_final_hard_line() {
        let document = Document::new("alpha\n");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 20.0);
        let end = document.text().len();
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(1..2, 16.0, 20.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request.captured_text_len(), 0);
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.coverage.hard_lines(), 1..2);
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.rows[0].hard_line_index, 1);
        assert_eq!(snapshot.rows[0].hard_line_range, end..end);
        assert_eq!(snapshot.rows[0].text_range, end..end);
        assert!(snapshot.rows[0].clusters.is_empty());
        assert_eq!(snapshot.rows[0].carets.len(), 2);
        assert!(snapshot.rows[0]
            .carets
            .iter()
            .all(|caret| caret.point.text_offset == end));
        assert_eq!(snapshot.rows[0].y, 16.0);
        assert!(snapshot
            .caret_point(end, super::super::BoundaryAffinity::Downstream)
            .is_ok());
    }

    #[test]
    fn viewport_can_begin_midway_through_a_long_wrapped_hard_line() {
        let long_line = "word ".repeat(500);
        let text = format!("prefix\n{long_line}\ntail");
        let document = Document::new(text);
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(80.0, 40.0);
        let expected = document.line_start(1).unwrap()..document.line_end(1).unwrap();
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(1..2, 116.0, 40.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request.captured_text_len(), long_line.len());
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert!(engine.provider().maximum_request_bytes <= MAX_SHAPE_FRAGMENT_BYTES);
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        let snapshot = view.snapshot().unwrap();
        assert!(snapshot.rows.len() > 10);
        assert!(snapshot
            .rows
            .iter()
            .all(|row| row.hard_line_index == 1 && row.hard_line_range == expected));
        assert!(snapshot
            .rows
            .iter()
            .flat_map(|row| &row.clusters)
            .all(|cluster| expected.start <= cluster.text_range.start
                && cluster.text_range.end <= expected.end));
        assert_eq!(snapshot.rows[0].y, 16.0);
        assert_eq!(view.viewport_top(), 116.0);
        let hit = snapshot
            .hit_test(super::super::LayoutPoint {
                x: 10.0,
                y: view.viewport_top() + 1.0,
            })
            .unwrap();
        assert!(expected.start <= hit.text_offset && hit.text_offset <= expected.end);
    }

    #[test]
    fn horizontal_scroll_during_a_job_is_not_a_layout_dependency() {
        let document = Document::new(format!("{}\nshort\nshort\nshort", "W".repeat(40)));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 40.0);
        view.set_wrap(false);
        view.set_viewport_left(40.0).unwrap();
        assert_eq!(view.maximum_viewport_left(), None);

        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::NewlyExposedRows,
            viewport(1..4, 16.0, 40.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let configuration = request.configuration_generation();

        view.set_viewport_left(60.0).unwrap();
        assert_eq!(view.viewport_left(), 60.0);
        assert_eq!(view.configuration_generation(), configuration);
        assert!(view.snapshot().is_none());

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        assert_eq!(view.viewport_left(), 0.0);
        assert_eq!(view.maximum_viewport_left(), Some(0.0));
        assert!(!view.snapshot().unwrap().content_width_is_exact);
    }

    #[test]
    fn partial_viewport_ignores_a_matching_exact_offscreen_document_width() {
        let document = Document::new(format!("{}\nshort\nshort\nshort", "W".repeat(40)));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 40.0);
        view.set_wrap(false);
        engine.relayout(&document, &mut view).unwrap();
        let exact_width = view.snapshot().unwrap().content_width;
        assert!(view.maximum_viewport_left().unwrap() > 60.0);
        view.set_viewport_left(60.0).unwrap();

        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::NewlyExposedRows,
            viewport(1..4, 16.0, 40.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        assert!(view.snapshot().unwrap().content_width_is_exact);
        assert_eq!(view.snapshot().unwrap().content_width, exact_width);
        assert_eq!(view.maximum_viewport_left(), Some(0.0));
        assert_eq!(view.viewport_left(), 0.0);
    }

    #[test]
    fn partial_viewport_preserves_document_insets_and_paragraph_spacing() {
        let document = Document::new("a\nb");
        let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs.len(), 2);
        styles.document_insets = super::super::EdgeInsets {
            top: 7.0,
            left: 2.0,
            bottom: 11.0,
            right: 3.0,
        };
        styles.paragraphs[0].spacing_before = 3.0;
        styles.paragraphs[0].spacing_after = 5.0;
        styles.paragraphs[1].spacing_before = 2.0;
        styles.paragraphs[1].spacing_after = 4.0;

        let mut view = ViewLayout::new(200.0, 100.0);
        view.set_insets(super::super::EdgeInsets {
            top: 13.0,
            left: 17.0,
            bottom: 19.0,
            right: 23.0,
        });
        assert!(view.begin_layout_job(LayoutJobId(1)));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let request = LayoutJobRequest {
            job_id: LayoutJobId(1),
            priority: LayoutJobPriority::ChangedVisibleRows,
            document_id: document.id(),
            document_revision: document.revision(),
            configuration_generation: view.configuration_generation(),
            provider_requirements: requirements,
            region: viewport(0..2, 0.0, 100.0),
            cancellation: LayoutCancellationToken::new(),
            projection_text_len: 3,
            input: CapturedLayoutInput::Regional {
                text: Arc::from("a\nb"),
                text_origin: 0,
                line_ranges: vec![0..1, 2..3],
                following_line_range: None,
                document_hard_line_count: 2,
                styles: Arc::new(styles),
            },
            captured_view: view.capture_for_regional_layout_job(0..3),
        };
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        let snapshot = view.snapshot().unwrap();
        assert_eq!(
            snapshot.content_insets,
            super::super::EdgeInsets {
                top: 20.0,
                left: 19.0,
                bottom: 30.0,
                right: 26.0,
            }
        );
        assert!((snapshot.rows[0].y - 23.0).abs() < 1.0e-5);
        let first_band = view.hard_line_range_height(0..1).unwrap().height() as f32;
        assert!((first_band - (snapshot.rows[0].line_advance + 30.0)).abs() < 1.0e-5);
        assert!((snapshot.rows[1].y - first_band).abs() < 1.0e-5);
        assert!(
            (snapshot.total_height - (snapshot.rows[1].y + snapshot.rows[1].line_advance + 34.0))
                .abs()
                < 1.0e-5
        );
        assert!(snapshot.total_height_is_exact);
        assert_eq!(
            snapshot.coverage.vertical_range().unwrap(),
            0.0..snapshot.total_height
        );
    }

    #[test]
    fn exact_line_spacing_preserves_final_row_extent_in_every_layout_path() {
        let document = Document::from_bytes(
            b"<p style=\"font-size:20pt;line-height:8pt;margin-block-start:0pt;margin-block-end:0pt\">first<br>second<br>last</p>".to_vec(),
            crate::document::Encoding::Utf8, crate::document::Format::Html,
        ).unwrap();
        assert_eq!(document.line_count(), 3);
        for regional in [false, true] {
            for wrap in [false, true] {
                let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
                let mut view = ViewLayout::new(400., 40.);
                view.set_wrap(wrap);
                for (job, margin) in [(1, 12.), (2, 32.)] {
                    let old_configuration = view.configuration_generation();
                    view.set_insets(super::super::EdgeInsets { bottom: margin, ..Default::default() });
                    assert_ne!(view.configuration_generation(), old_configuration);
                    if regional {
                        let requirements = inspect_layout_provider(&engine);
                        let request = prepare_layout_job(
                            &document, &mut view, requirements, LayoutJobId(job),
                            LayoutJobPriority::ChangedVisibleRows, viewport(0..3, 0., 40.),
                            LayoutCancellationToken::new(),
                        ).unwrap();
                        let candidate = compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
                        install_layout_job(&mut view, target(&document, requirements.metrics_generation), candidate).unwrap();
                    } else {
                        engine.relayout(&document, &mut view).unwrap();
                    }
                    let snapshot = view.snapshot().unwrap();
                    let rows = &snapshot.rows;
                    let last = rows.last().unwrap();
                    assert_eq!(rows.len(), 3);
                    assert_eq!(last.line_advance, 8.);
                    assert!(last.natural_height() > last.line_advance);
                    assert_eq!(rows[1].y - rows[0].y, 8., "interline spacing remains unchanged");
                    assert_eq!(last.y - rows[1].y, 8.);
                    assert!((snapshot.total_height - row_bottom(last) - margin).abs() < 0.001,
                        "regional={regional}, wrap={wrap}: final-row bottom {} plus margin {margin} differs from extent {}",
                        row_bottom(last), snapshot.total_height);
                    assert!(snapshot.total_height_is_exact);
                    assert!((view.content_height().height() as f32 - snapshot.total_height).abs() < 0.001);
                    let max_top = snapshot.total_height - view.height();
                    view.set_viewport_top(f32::MAX).unwrap();
                    assert!((view.viewport_top() - max_top).abs() < 0.001);
                }
            }
        }

        fn row_bottom(row: &super::super::VisualRow) -> f32 {
            let natural = row.y + row.natural_height();
            row.ink_bounds().map_or(natural, |ink| natural.max(ink.y + ink.height))
        }
    }

    #[test]
    fn viewport_long_line_uses_bounded_shape_fragments() {
        let document = Document::new("x".repeat(10_000));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        view.set_wrap(false);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            viewport(0..1, 0.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(
            candidate.computation_scope(),
            LayoutComputationScope::PartialViewport
        );
        assert_eq!(engine.provider().shaped_text_bytes, 10_000);
        assert!(engine.provider().maximum_request_bytes <= MAX_SHAPE_FRAGMENT_BYTES);
        assert_eq!(engine.provider().shape_requests, 3);
        assert_eq!(candidate.regional_snapshot().lines()[0].rows().len(), 1);
    }

    #[test]
    fn multi_megabyte_wrapped_viewport_captures_and_computes_one_bounded_slice() {
        const LONG_LINE_BYTES: usize = 2 * 1024 * 1024;
        let text = "word ".repeat(LONG_LINE_BYTES / 5 + 1);
        let document = Document::new(text[..LONG_LINE_BYTES].to_owned());
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(96.0, 80.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 80.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();

        assert!(
            request.captured_text_len()
                <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + LONG_LINE_CAPTURE_CONTEXT_BYTES * 2,
            "bounded viewport capture unexpectedly retained {} bytes",
            request.captured_text_len()
        );
        assert!(request.captured_text_len() < LONG_LINE_BYTES / 16);

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let regional = candidate.regional_snapshot();
        let line = &regional.lines()[0];
        let checkpoint = line.next_checkpoint().unwrap();
        let work = regional.work_statistics();
        assert!(work.segmented_text_bytes() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        assert!(work.shaping_fragment_count() > 1);
        assert!(work.maximum_shaping_fragment_bytes() <= MAX_SHAPE_FRAGMENT_BYTES);
        assert_eq!(
            work.wrapped_cluster_count(),
            engine.provider().shaped_text_bytes
        );
        assert!(work.maximum_wrap_checkpoint_clusters() <= CANCELLATION_CLUSTER_BATCH);
        assert!(work.maximum_position_checkpoint_clusters() <= CANCELLATION_CLUSTER_BATCH);
        assert!(line.text_coverage().end < LONG_LINE_BYTES);
        assert_eq!(checkpoint.next_text_offset(), line.text_coverage().end);
        assert_eq!(checkpoint.completed_visual_rows(), line.rows().len());
        assert!(checkpoint.cumulative_advance() > 0.0);
        assert_eq!(
            checkpoint.last_candidate_break(),
            Some(checkpoint.next_text_offset())
        );
        assert!(checkpoint.continuation_width() > 0.0);
        assert!(line.rows().last().unwrap().wraps_to_next);
        assert!(!line.height_is_exact());
        let coverage_end = line.text_coverage().end;
        let next_offset = checkpoint.next_text_offset();

        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        let snapshot = view.snapshot().unwrap();
        assert!(!snapshot.total_height_is_exact);
        assert_eq!(view.maximum_viewport_left(), Some(0.0));
        assert!(view.regional_cached_ranges().is_empty());
        assert!(snapshot
            .caret_point(coverage_end, super::super::BoundaryAffinity::Upstream,)
            .is_ok());
        assert_eq!(
            snapshot.caret_point(next_offset + 1, super::super::BoundaryAffinity::Downstream,),
            Err(LayoutError::OutsideMaterializedCoverage)
        );
    }

    #[test]
    fn multi_megabyte_unwrapped_line_streams_and_retains_only_horizontal_geometry() {
        const LONG_LINE_BYTES: usize = 2 * 1024 * 1024;
        let document = Document::new("x".repeat(LONG_LINE_BYTES));
        let mut engine = LayoutEngine::new(FragmentAggregateProvider::default());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(96.0, 80.0);
        view.set_wrap(false);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 80.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(request.captured_text_len(), 0, "capture must share the immutable tree");

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let regional = candidate.regional_snapshot();
        let work = regional.work_statistics();
        assert!(work.segmented_text_bytes() >= LONG_LINE_BYTES);
        assert!(work.segmented_text_bytes() <= LONG_LINE_BYTES + MAX_SHAPE_FRAGMENT_BYTES * 4);
        assert!(work.shaping_fragment_count() >= LONG_LINE_BYTES / MAX_SHAPE_FRAGMENT_BYTES);
        assert_eq!(
            work.maximum_shaping_fragment_bytes(),
            MAX_SHAPE_FRAGMENT_BYTES
        );
        assert_eq!(
            engine.provider().maximum_request_bytes,
            MAX_SHAPE_FRAGMENT_BYTES
        );
        assert!(engine.provider().shaped_bytes <= LONG_LINE_BYTES + MAX_SHAPE_FRAGMENT_BYTES * 4);
        assert!(regional.lines()[0].rows()[0].clusters.len() <= 4);
        assert!(work.maximum_wrap_checkpoint_clusters() <= CANCELLATION_CLUSTER_BATCH);
        assert!(work.maximum_position_checkpoint_clusters() <= CANCELLATION_CLUSTER_BATCH);
        assert_eq!(regional.lines()[0].rows().len(), 1);
        assert_eq!(regional.lines()[0].text_coverage(), 0..LONG_LINE_BYTES);
        assert!(regional.lines()[0].height_is_exact());
        assert!(regional.lines()[0].next_checkpoint().is_none());
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        assert!(view.maximum_viewport_left().unwrap() > LONG_LINE_BYTES as f32 / 2.0);
        assert!(view.snapshot().unwrap().has_horizontal_materialization());
        assert!(view.regional_cached_ranges().is_empty());
        assert_eq!(view.snapshot().unwrap().caret_point(LONG_LINE_BYTES / 2, super::super::BoundaryAffinity::Downstream),
            Err(LayoutError::OutsideMaterializedCoverage));
    }

    #[test]
    fn mixed_short_and_giant_viewport_lines_share_text_but_explicit_complete_geometry_does_not() {
        for giant in ["a".repeat(80_000), "word ".repeat(20_000)] {
            let document = Document::new(format!("short\n{giant}\ntail"));
            let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            let requirements = inspect_layout_provider(&engine);
            let mut view = ViewLayout::new(120.0, 80.0);
            let request = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(1), LayoutJobPriority::ChangedVisibleRows,
                viewport(0..3, 0.0, 80.0), LayoutCancellationToken::new()).unwrap();
            assert_eq!(request.captured_text_len(), 0);
            assert!(matches!(request.input, CapturedLayoutInput::UnwrappedViewport { .. }));
            let complete = LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..3, 0.0, 80.0).unwrap().with_complete_horizontal_geometry());
            let request = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(2), LayoutJobPriority::ChangedVisibleRows,
                complete, LayoutCancellationToken::new()).unwrap();
            assert_eq!(request.captured_text_len(), document.projection().text_tree().byte_len());
            assert!(!document.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn giant_word_inside_paragraph_captures_tree_for_overflow_continuation() {
        let word_bytes = 2 * 1024 * 1024;
        let document = Document::new(format!("prefix {} tail\nfollowing", "a".repeat(word_bytes)));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(120.0, 80.0);
        let first = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(1), LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 80.0), LayoutCancellationToken::new()).unwrap();
        assert!(first.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + LONG_LINE_CAPTURE_CONTEXT_BYTES * 2);
        let first = compute_layout_job(&mut engine, &first, LayoutExecutionContext::WorkerPool).unwrap();
        let checkpoint = first.next_long_line_checkpoint().unwrap().clone();
        assert_eq!(checkpoint.next_text_offset(), 7);
        let resumed = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(2), LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::resume_long_line(checkpoint.clone(), 0.0, 80.0).unwrap()),
            LayoutCancellationToken::new()).unwrap();
        assert_eq!(resumed.captured_text_len(), 0);
        assert_eq!(resumed.capture_statistics().regional_text_bytes(), 0);
        match &resumed.input {
            CapturedLayoutInput::StreamingOverflowSlice { line_slice, following_line_range, .. } => {
                assert_eq!(line_slice.work_range, 7..word_bytes + 8);
                assert_eq!(line_slice.full_range, 0..word_bytes + 12);
                assert!(following_line_range.is_some(), "width-fit fallback retains the following paragraph");
            }
            _ => panic!("giant word continuation must share the text tree"),
        }
        assert!(!document.projection().compatibility_text_is_materialized());
        view.resize(90.0, 80.0);
        assert!(matches!(prepare_layout_job(&document, &mut view, requirements, LayoutJobId(3), LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 80.0).unwrap()), LayoutCancellationToken::new()),
            Err(LayoutJobError::InvalidLongLineCheckpoint(_))));
    }

    #[test]
    fn streamed_overflow_between_prefix_and_tail_matches_complete_row_geometry() {
        let text = format!("prefix {} tail", "AVfi".repeat(24_000));
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(120.0, 80.0);
        let mut checkpoint = None;
        let mut rows = Vec::new();
        for job in 1..=5 {
            let region = checkpoint.take().map_or_else(|| viewport(0..1, 0.0, 80.0), |checkpoint| {
                LayoutJobRegion::Viewport(ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 80.0).unwrap())
            });
            let request = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(job), LayoutJobPriority::ChangedVisibleRows,
                region, LayoutCancellationToken::new()).unwrap();
            let candidate = compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
            let line = &candidate.regional_snapshot().lines()[0];
            rows.extend(line.rows().iter().cloned());
            checkpoint = line.next_checkpoint().cloned();
            if checkpoint.is_none() { assert!(line.height_is_exact()); break; }
        }
        assert!(checkpoint.is_none());
        assert_eq!(rows.len(), 3);
        assert!(rows[1].clusters.len() < 5000);
        let mut full_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut full_view = ViewLayout::new(120.0, 80.0);
        full_engine.relayout(&document, &mut full_view).unwrap();
        let complete = &full_view.snapshot().unwrap().rows;
        assert_eq!(rows.len(), complete.len());
        for (row, expected) in rows.iter().zip(complete) {
            assert_eq!(row.text_range, expected.text_range);
            assert_eq!(row.hard_line_range, expected.hard_line_range);
            assert_eq!(row.fragment_index, expected.fragment_index);
            assert_eq!(row.wrapped_from_previous, expected.wrapped_from_previous);
            assert_eq!(row.wraps_to_next, expected.wraps_to_next);
            assert_eq!(row.y, expected.y);
            assert_eq!(row.baseline, expected.baseline);
            assert_eq!(row.width, expected.width);
            for cluster in &row.clusters {
                let actual = expected.clusters.iter().find(|candidate| candidate.text_range == cluster.text_range).unwrap();
                assert_eq!(cluster.x, actual.x);
                assert_eq!(cluster.advance, actual.advance);
            }
        }
    }

    #[test]
    fn resumed_wrapped_slices_are_geometry_equivalent_to_complete_layout() {
        let text = "word ".repeat(32_000);
        assert!(text.len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES * 2);
        let document = Document::new(text.clone());
        let mut regional_engine =
            LayoutEngine::new(InstrumentedProvider::new(ProviderThreading::AnyWorker));
        let requirements = inspect_layout_provider(&regional_engine);
        let mut regional_view = ViewLayout::new(93.0, 64.0);
        let mut checkpoint = None;
        let mut rows = Vec::new();
        let mut job = 1_u64;

        loop {
            let region = checkpoint.take().map_or_else(
                || viewport(0..1, 0.0, 64.0),
                |checkpoint| {
                    LayoutJobRegion::Viewport(
                        ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 64.0).unwrap(),
                    )
                },
            );
            let request = prepare_layout_job(
                &document,
                &mut regional_view,
                requirements,
                LayoutJobId(job),
                LayoutJobPriority::ChangedVisibleRows,
                region,
                LayoutCancellationToken::new(),
            )
            .unwrap();
            let candidate = compute_layout_job(
                &mut regional_engine,
                &request,
                LayoutExecutionContext::WorkerPool,
            )
            .unwrap();
            let line = &candidate.regional_snapshot().lines()[0];
            rows.extend(line.rows().iter().cloned());
            checkpoint = line.next_checkpoint().cloned();
            job += 1;
            if checkpoint.is_none() {
                assert!(line.height_is_exact());
                break;
            }
        }

        let mut full_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut full_view = ViewLayout::new(93.0, 64.0);
        full_engine.relayout(&document, &mut full_view).unwrap();
        let full_rows = &full_view.snapshot().unwrap().rows;
        assert_eq!(rows.len(), full_rows.len());
        for (resumed, full) in rows.iter().zip(full_rows) {
            assert_eq!(resumed.text_range, full.text_range);
            assert_eq!(resumed.wrapped_from_previous, full.wrapped_from_previous);
            assert_eq!(resumed.wraps_to_next, full.wraps_to_next);
            assert_eq!(resumed.width, full.width);
            assert_eq!(resumed.y, full.y);
            assert_eq!(resumed.baseline, full.baseline);
            assert_eq!(resumed.clusters, full.clusters);
        }
    }

    #[test]
    fn resize_invalidates_a_long_line_continuation_before_capture() {
        let document = Document::new("word ".repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES / 2));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(96.0, 64.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 64.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let checkpoint = candidate.next_long_line_checkpoint().unwrap().clone();

        view.resize(80.0, 64.0);
        let region = LayoutJobRegion::Viewport(
            ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 64.0).unwrap(),
        );
        assert_eq!(
            prepare_layout_job(
                &document,
                &mut view,
                requirements,
                LayoutJobId(2),
                LayoutJobPriority::ChangedVisibleRows,
                region,
                LayoutCancellationToken::new(),
            )
            .unwrap_err(),
            LayoutJobError::InvalidLongLineCheckpoint(
                "checkpoint identities or hard-line extent are stale"
            )
        );
        assert_eq!(view.active_layout_job(), Some(LayoutJobId(1)));
    }

    #[test]
    fn new_document_revision_replaces_viewport_cache_and_reconciles_line_count() {
        let mut document = Document::new("zero\none\ntwo\nthree");
        let mut engine = LayoutEngine::new(InstrumentedProvider::new(ProviderThreading::AnyWorker));
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let first = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            viewport(1..3, 16.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let first =
            compute_layout_job(&mut engine, &first, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            first,
        )
        .unwrap();
        assert_eq!(view.regional_cached_ranges(), vec![1..3]);
        assert_eq!(view.height_index_statistics().hard_line_count(), 4);

        document.insert(0, "new\n").unwrap();
        let second = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let second =
            compute_layout_job(&mut engine, &second, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            second,
        )
        .unwrap();
        assert_eq!(view.regional_cached_ranges(), vec![0..1]);
        assert_eq!(view.height_index_statistics().hard_line_count(), 5);
        assert!(!view.content_height().is_exact());
        assert_eq!(view.snapshot().unwrap().coverage.hard_lines(), 0..1);

        view.resize(60.0, 100.0);
        assert_eq!(
            view.regional_cache_statistics(),
            RegionalLayoutCacheStatistics::default()
        );
    }

    #[test]
    fn regional_completion_permutation_installs_only_the_current_job() {
        let document = Document::new("zero\none\ntwo");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let first = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let second = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(2..3),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let first =
            compute_layout_job(&mut engine, &first, LayoutExecutionContext::WorkerPool).unwrap();
        let second =
            compute_layout_job(&mut engine, &second, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            second,
        )
        .unwrap();
        assert_eq!(view.regional_cached_ranges(), vec![2..3]);
        assert_eq!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                first,
            ),
            Err(LayoutJobInstallRejection::Superseded {
                installed: LayoutJobId(2),
                candidate: LayoutJobId(1),
            })
        );
        assert_eq!(view.regional_cached_ranges(), vec![2..3]);
    }

    #[test]
    fn hard_line_then_viewport_merges_cache_heights_and_reuses_shapes() {
        let document = Document::new("same\nmiddle\nsame");
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(120.0, 40.0);

        for (job, range) in [(LayoutJobId(1), 0..1), (LayoutJobId(2), 2..3)] {
            let request = prepare_layout_job(
                &document,
                &mut view,
                requirements,
                job,
                LayoutJobPriority::Background,
                hard_lines(range),
                LayoutCancellationToken::new(),
            )
            .unwrap();
            let candidate =
                compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                candidate,
            )
            .unwrap();
        }
        assert_eq!(engine.provider().shape_requests, 1);
        assert_eq!(view.regional_cached_ranges(), vec![0..1, 2..3]);
        assert!(view.snapshot().is_none());

        let viewport_top = view.hard_line_prefix_height(2).unwrap().height() as f32;
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(3),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(2..3, viewport_top, 40.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(engine.provider().shape_requests, 1);
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();

        assert_eq!(view.regional_cached_ranges(), vec![0..1, 2..3]);
        assert!(view.hard_line_range_height(0..1).unwrap().is_exact());
        assert!(!view.hard_line_range_height(1..2).unwrap().is_exact());
        assert!(view.hard_line_range_height(2..3).unwrap().is_exact());
        assert_eq!(view.snapshot().unwrap().coverage.hard_lines(), 2..3);
        assert_eq!(view.snapshot().unwrap().rows[0].hard_line_index, 2);
        assert!(!view.content_height().is_exact());
    }

    #[test]
    fn hard_line_refinement_before_partial_viewport_repositions_it_atomically() {
        let document = Document::new("zero\none\ntwo\nthree");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(120.0, 20.0);
        let viewport_request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(2..3, 32.0, 20.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let viewport_candidate = compute_layout_job(
            &mut engine,
            &viewport_request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            viewport_candidate,
        )
        .unwrap();
        assert_eq!(view.snapshot().unwrap().rows[0].y, 32.0);

        let refinement = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::Background,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let refinement =
            compute_layout_job(&mut engine, &refinement, LayoutExecutionContext::WorkerPool)
                .unwrap();
        let installed = install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            refinement,
        )
        .unwrap();

        let prefix = view.hard_line_prefix_height(2).unwrap();
        assert!(!prefix.is_exact());
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.revision, installed.layout_revision);
        assert_eq!(snapshot.rows[0].y, prefix.height() as f32);
        assert_eq!(
            snapshot.coverage.vertical_range().unwrap().start,
            prefix.height() as f32
        );
        assert_eq!(view.viewport_top(), prefix.height() as f32);
        assert_eq!(snapshot.total_height, view.content_height().height() as f32);
        assert_eq!(view.regional_cached_ranges(), vec![0..1, 2..3]);
    }

    #[test]
    fn deterministic_completion_permutation_installs_only_newest_job() {
        let document = Document::new("alpha\nbeta");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(120.0, 100.0);

        let first = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ViewportOverscan,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let viewport_height = view.height();
        let second = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(
                ViewportLayoutRegion::new(0..2, 0.0, viewport_height).unwrap(),
            ),
            LayoutCancellationToken::new(),
        )
        .unwrap();

        // Deliberately complete the second request first.
        let second_candidate =
            compute_layout_job(&mut engine, &second, LayoutExecutionContext::WorkerPool).unwrap();
        let first_candidate =
            compute_layout_job(&mut engine, &first, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(
            second_candidate.computation_scope(),
            LayoutComputationScope::PartialViewport
        );

        let installed = install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            second_candidate,
        )
        .unwrap();
        assert_eq!(installed.job_id, LayoutJobId(2));
        assert!(view.content_height().is_exact());
        assert_eq!(view.height_index_statistics().hard_line_count(), 2);
        let snapshot = view.snapshot().unwrap().clone();
        let installed_height = view.content_height();

        assert_eq!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                first_candidate,
            ),
            Err(LayoutJobInstallRejection::Superseded {
                installed: LayoutJobId(2),
                candidate: LayoutJobId(1),
            })
        );
        assert_eq!(view.snapshot(), Some(&snapshot));
        assert_eq!(view.content_height(), installed_height);
    }

    #[test]
    fn resize_rejects_partial_viewport_candidate_without_replacing_snapshot() {
        let document = Document::new("some words that wrap");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(200.0, 100.0);
        engine.relayout(&document, &mut view).unwrap();
        let original = view.snapshot().unwrap().clone();
        let requirements = inspect_layout_provider(&engine);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(10),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();

        view.resize(60.0, 100.0);
        let invalidated_height = view.content_height();
        assert!(!invalidated_height.is_exact());
        assert!(matches!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                candidate,
            ),
            Err(LayoutJobInstallRejection::StaleConfiguration { .. })
        ));
        assert_eq!(view.snapshot(), Some(&original));
        assert_eq!(view.content_height(), invalidated_height);
    }

    #[test]
    fn document_edit_rejects_stale_viewport_candidate_atomically() {
        let mut document = Document::new("before");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let initial_height = view.content_height();
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::NewlyExposedRows,
            viewport(0..1, 0.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        document
            .apply_edits(vec![TextEdit::new(6..6, " after")])
            .unwrap();

        assert!(matches!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                candidate,
            ),
            Err(LayoutJobInstallRejection::StaleDocumentRevision { .. })
        ));
        assert!(view.snapshot().is_none());
        assert_eq!(view.content_height(), initial_height);
    }

    #[test]
    fn stale_top_level_result_is_rejected_but_exact_content_cache_hit_is_revalidated() {
        let mut document = Document::new("same\nother");
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);

        let stale_request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let stale_candidate = compute_layout_job(
            &mut engine,
            &stale_request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        assert_eq!(engine.provider().shape_requests, 1);

        // Change an unrelated hard line. The old top-level result is stale,
        // while its content-keyed shaping fragment remains safe only after the
        // new request has revalidated text, bounded context, style, direction,
        // environment, and metrics through the complete cache key.
        let old_revision = document.revision();
        document.insert(document.text().len(), "!").unwrap();
        let current_request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let current_candidate = compute_layout_job(
            &mut engine,
            &current_request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        assert_eq!(engine.provider().shape_requests, 1);
        assert_eq!(current_candidate.document_revision(), document.revision());
        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            current_candidate,
        )
        .unwrap();

        assert_eq!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                stale_candidate,
            ),
            Err(LayoutJobInstallRejection::StaleDocumentRevision {
                expected: document.revision(),
                actual: old_revision,
            })
        );
        assert_eq!(view.regional_cached_ranges(), vec![0..1]);
    }

    #[test]
    fn cancellation_before_during_and_after_compute_never_publishes() {
        let document = Document::new("cancel me");
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let initial_height = view.content_height();

        let before_token = LayoutCancellationToken::new();
        let before = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            viewport(0..1, 0.0, 100.0),
            before_token.clone(),
        )
        .unwrap();
        before_token.cancel();
        assert_eq!(
            compute_layout_job(&mut engine, &before, LayoutExecutionContext::WorkerPool)
                .unwrap_err(),
            LayoutJobError::Cancelled
        );
        assert_eq!(engine.provider().shape_calls, 0);

        let during_token = LayoutCancellationToken::new();
        let during = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(2),
            LayoutJobPriority::ViewportOverscan,
            viewport(0..1, 0.0, 100.0),
            during_token.clone(),
        )
        .unwrap();
        engine.provider_mut().cancel_during_shape = Some(during_token);
        assert_eq!(
            compute_layout_job(&mut engine, &during, LayoutExecutionContext::WorkerPool)
                .unwrap_err(),
            LayoutJobError::Cancelled
        );
        assert_eq!(engine.provider().shape_calls, 1);
        assert!(view.snapshot().is_none());

        engine.provider_mut().cancel_during_shape = None;
        let after_token = LayoutCancellationToken::new();
        let after = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(3),
            LayoutJobPriority::ChangedVisibleRows,
            viewport(0..1, 0.0, 100.0),
            after_token.clone(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &after, LayoutExecutionContext::WorkerPool).unwrap();
        after_token.cancel();
        assert_eq!(
            install_layout_job(
                &mut view,
                target(&document, requirements.metrics_generation),
                candidate,
            ),
            Err(LayoutJobInstallRejection::Cancelled)
        );
        assert!(view.snapshot().is_none());
        assert_eq!(view.content_height(), initial_height);
    }

    #[test]
    fn cancellation_during_large_shape_stops_at_a_bounded_batch() {
        let line_count = 5_000;
        let document = Document::new(vec!["x"; line_count].join("\n"));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let token = LayoutCancellationToken::new();
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::Background,
            hard_lines(0..1),
            token.clone(),
        )
        .unwrap();
        engine.provider_mut().cancel_during_shape = Some(token);

        assert_eq!(
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                .unwrap_err(),
            LayoutJobError::Cancelled
        );
        assert_eq!(engine.provider().shape_calls, 1);
        assert!(engine.provider().shape_requests <= MAX_CANCELLABLE_SHAPE_BATCH_FRAGMENTS);
        assert!(engine.provider().shape_requests < line_count);
        assert!(
            view.snapshot().is_none(),
            "worker output must remain private"
        );
    }

    #[test]
    fn provider_threading_declaration_selects_execution_context() {
        let document = Document::new("main-thread provider");
        let provider = InstrumentedProvider::new(ProviderThreading::FrontendMainThread);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(200.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();

        assert_eq!(
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                .unwrap_err(),
            LayoutJobError::WrongExecutionContext {
                required: ProviderThreading::FrontendMainThread,
                actual: LayoutExecutionContext::WorkerPool,
            }
        );
        assert_eq!(engine.provider().shape_calls, 0);
        assert!(compute_layout_job(
            &mut engine,
            &request,
            LayoutExecutionContext::FrontendMainThread,
        )
        .is_ok());
        assert_eq!(engine.provider().shape_calls, 1);
    }

    #[test]
    fn metrics_change_rejects_request_before_shaping() {
        let document = Document::new("metrics");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));

        assert_eq!(
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                .unwrap_err(),
            LayoutJobError::StaleMetrics {
                expected: MetricsGeneration(1),
                actual: MetricsGeneration(2),
            }
        );
        assert_eq!(engine.provider().request_calls(), 0);
    }

    #[test]
    fn different_measurement_environment_rejects_request_before_shaping() {
        let document = Document::new("environment");
        let mut preparation_provider = MockTextMeasurementProvider::new();
        preparation_provider.set_measurement_environment_id(MeasurementEnvironmentId(11));
        let preparation_engine = LayoutEngine::new(preparation_provider);
        let requirements = inspect_layout_provider(&preparation_engine);
        assert_eq!(requirements.metrics_generation, MetricsGeneration(1));

        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();

        let mut worker_provider = MockTextMeasurementProvider::new();
        worker_provider.set_measurement_environment_id(MeasurementEnvironmentId(22));
        let mut worker_engine = LayoutEngine::new(worker_provider);
        assert_eq!(
            worker_engine.provider().metrics_generation(),
            MetricsGeneration(1)
        );

        assert_eq!(
            compute_layout_job(
                &mut worker_engine,
                &request,
                LayoutExecutionContext::WorkerPool,
            )
            .unwrap_err(),
            LayoutJobError::WrongMeasurementEnvironment {
                expected: MeasurementEnvironmentId(11),
                actual: MeasurementEnvironmentId(22),
            }
        );
        assert_eq!(worker_engine.provider().request_calls(), 0);
    }

    #[test]
    fn metrics_change_after_compute_is_rejected_at_install() {
        let document = Document::new("metrics");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();

        assert_eq!(
            install_layout_job(
                &mut view,
                target(&document, MetricsGeneration(2)),
                candidate,
            ),
            Err(LayoutJobInstallRejection::StaleMetrics {
                expected: MetricsGeneration(2),
                actual: MetricsGeneration(1),
            })
        );
        assert!(view.snapshot().is_none());
    }

    #[test]
    fn candidate_from_different_generation_one_environment_is_rejected_at_install() {
        let document = Document::new("environment");
        let mut provider = MockTextMeasurementProvider::new();
        provider.set_measurement_environment_id(MeasurementEnvironmentId(11));
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        assert_eq!(requirements.metrics_generation, MetricsGeneration(1));

        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(candidate.metrics_generation(), MetricsGeneration(1));

        assert_eq!(
            install_layout_job(
                &mut view,
                LayoutInstallTarget {
                    document_id: document.id(),
                    document_revision: document.revision(),
                    measurement_environment_id: MeasurementEnvironmentId(22),
                    metrics_generation: MetricsGeneration(1),
                },
                candidate,
            ),
            Err(LayoutJobInstallRejection::WrongMeasurementEnvironment {
                expected: MeasurementEnvironmentId(22),
                actual: MeasurementEnvironmentId(11),
            })
        );
        assert!(view.snapshot().is_none());
    }

    #[test]
    fn provider_callback_observes_no_coordinator_lock() {
        let document = Document::new("unlocked");
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let lock_flag = provider.coordinator_lock_held.clone();
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            hard_lines(0..1),
            LayoutCancellationToken::new(),
        )
        .unwrap();

        lock_flag.store(false, Ordering::Release);
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(engine.provider().shape_calls, 1);
    }

    #[test]
    fn large_viewport_request_is_bounded_and_installs_partial_coverage() {
        let document = Document::new(vec!["x"; 5_000].join("\n"));
        let provider = InstrumentedProvider::new(ProviderThreading::AnyWorker);
        let mut engine = LayoutEngine::new(provider);
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(100.0, 100.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ViewportOverscan,
            viewport(2_500..2_505, 40_000.0, 100.0),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            request.region(),
            &viewport(2_500..2_505, 40_000.0, 100.0),
            "the scheduler request itself must never silently widen"
        );
        assert_eq!(request.captured_text_len(), 9);

        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert_eq!(
            candidate.computation_scope(),
            LayoutComputationScope::PartialViewport
        );
        assert_eq!(candidate.requested_region(), request.region());
        assert!(matches!(
            candidate.product(),
            LayoutJobProduct::PartialViewport(_)
        ));
        assert_eq!(engine.provider().shape_requests, 5);
        assert_eq!(engine.provider().shaped_text_bytes, 5);

        install_layout_job(
            &mut view,
            target(&document, requirements.metrics_generation),
            candidate,
        )
        .unwrap();
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.coverage.hard_lines(), 2_500..2_505);
        assert_eq!(snapshot.coverage.document_hard_line_count(), 5_000);
        assert!(!snapshot.coverage.is_full_document());
        assert_eq!(snapshot.rows.len(), 5);
        assert_eq!(view.height_index_statistics().hard_line_count(), 5_000);
        assert!(view.height_index_statistics().run_count() <= 3);
        assert!(!view.content_height().is_exact());
    }
}
