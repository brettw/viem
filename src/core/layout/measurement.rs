//! Portable boundary between layout policy and platform text shaping.
//!
//! The macOS implementation will translate these values to and from Core Text.
//! Keeping the request batched and revision tagged lets a frontend schedule the
//! work without giving it ownership of document or view state.

pub use crate::document::{BoundaryAffinity, FontSlant};
use crate::document::{DocumentId, Revision};
use std::ops::Range;

/// Identity for all font-resolution and measurement inputs owned by a provider.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MetricsGeneration(pub u64);

/// Stable identity of one compatible font-resolution and shaping environment.
///
/// A generation is meaningful only within this identity. Separate provider
/// instances MAY report the same identity when they are interchangeable views
/// of the same underlying resolver and render-resource registry. Providers
/// whose measurements or render handles are not interchangeable MUST report
/// different identities, even when both currently use generation zero or one.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MeasurementEnvironmentId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextDirection {
    #[default]
    Auto,
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapePurpose {
    MetricsOnly,
    MetricsAndRenderData,
}

/// Thread rule for accessing or releasing one provider-owned render resource.
/// It is intentionally independent of [`ProviderThreading`]: a provider may
/// shape on a worker while its native draw object remains main-thread-bound.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderRunThreading {
    #[default]
    AnyThread,
    DedicatedSerialExecutor,
    FrontendMainThread,
}

/// Scheduling rule declared by a frontend provider. Core never infers thread
/// confinement from the platform or calls a provider while holding model state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProviderThreading {
    #[default]
    AnyWorker,
    DedicatedSerialExecutor,
    FrontendMainThread,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeFeature {
    pub tag: [u8; 4],
    pub value: u32,
}

/// The subset of resolved character style that can affect shaping or metrics.
/// Paint-only properties deliberately do not appear here.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTextStyle {
    pub font_families: Vec<String>,
    pub size: f32,
    pub weight: f32,
    pub relative_bold: bool,
    /// Preserve italic versus oblique so the platform shaper can select the
    /// document's requested face rather than reducing both to a boolean.
    pub slant: FontSlant,
    pub letter_spacing: f32,
    /// Baseline displacement in layout units. This participates in row ink
    /// metrics and consequently belongs in the shaping-cache key.
    pub baseline_shift: f32,
    pub language: Option<String>,
    pub script: Option<String>,
    pub direction: TextDirection,
    pub features: Vec<OpenTypeFeature>,
}

impl Default for ResolvedTextStyle {
    fn default() -> Self {
        Self {
            font_families: vec!["SF Pro".to_owned()],
            size: 14.0,
            weight: 400.0,
            relative_bold: false,
            slant: FontSlant::Upright,
            letter_spacing: 0.0,
            baseline_shift: 0.0,
            language: None,
            script: None,
            direction: TextDirection::Auto,
            features: Vec::new(),
        }
    }
}

impl ResolvedTextStyle {
    pub(crate) fn is_valid(&self) -> bool {
        self.size.is_finite()
            && self.size > 0.0
            && self.weight.is_finite()
            && self.letter_spacing.is_finite()
            && self.baseline_shift.is_finite()
            && !self.font_families.is_empty()
            && self.font_families.iter().all(|family| !family.is_empty())
            && self
                .language
                .as_ref()
                .map_or(true, |value| !value.is_empty())
            && self.script.as_ref().map_or(true, |value| !value.is_empty())
            && self.features.iter().enumerate().all(|(index, feature)| {
                feature.tag.iter().all(|byte| (0x20..=0x7e).contains(byte))
                    && !self.features[..index]
                        .iter()
                        .any(|previous| previous.tag == feature.tag)
            })
    }
}

/// A non-overlapping style run in global formatted UTF-8 byte coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeStyleRun {
    pub text_range: Range<usize>,
    pub style: ResolvedTextStyle,
}

/// A single item in a batch sent across the platform boundary.
///
/// `text_range` is the requested stable ownership interior represented by
/// `text`. `context_before` and `context_after` are bounded, immediately
/// adjacent complete grapheme sequences. A provider shapes their concatenation
/// with `text`, but returns only whole clusters whose logical start is inside
/// `text_range`. Consequently, a returned cluster may extend beyond
/// `text_range.end` into `context_after`, while a cluster beginning in
/// `context_before` is omitted even when it overlaps the interior. Adjacent
/// requests then partition clusters by logical start without cutting a
/// ligature at a cache-fragment boundary.
///
/// A successful response explicitly affirms that text outside the supplied
/// context cannot change the owned clusters. A provider that cannot establish
/// that guarantee for its shaper, fonts, and features MUST return
/// [`MeasurementError::UnstableShapingContext`] instead. Core never widens the
/// bounded context without a separate contract and never caches or installs a
/// failed batch. `style_runs` uses global formatted coordinates and covers
/// every non-default style intersecting context or the ownership interior;
/// this makes a context-only style change part of the shaping dependency.
#[derive(Clone, Debug)]
pub struct ShapeRequest<'a> {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub measurement_environment_id: MeasurementEnvironmentId,
    pub text_range: Range<usize>,
    pub text: &'a str,
    pub context_before: &'a str,
    pub context_after: &'a str,
    pub style_runs: &'a [ShapeStyleRun],
    pub default_style: &'a ResolvedTextStyle,
    /// Base direction of the containing paragraph. This is distinct from the
    /// character-level direction override in [`ResolvedTextStyle`]: a natural
    /// character run still needs the paragraph base level for correct bidi
    /// embedding, visual order, and caret affinity.
    pub paragraph_base_direction: TextDirection,
    pub scale: f32,
    pub metrics_generation: MetricsGeneration,
    pub purpose: ShapePurpose,
    /// Required owner/access policy for opaque render data. Metrics-only
    /// requests use `None`; a render-data request without a policy is invalid.
    pub render_run_policy: Option<RenderRunPolicy>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

/// Bounds in a cluster-local coordinate system whose origin is the visual left
/// edge on the row baseline. X grows toward the visual right and Y grows down.
/// Negative origins are valid for glyph overhang. Width and height are
/// non-negative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ShapedBounds {
    pub(crate) fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.width >= 0.0
            && self.height.is_finite()
            && self.height >= 0.0
    }
}

/// Provider registry identity for an opaque native render resource. Core only
/// compares and transports this value; it never dereferences it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderRunOwner(pub u64);

/// Opaque render data with an explicit owner, access rule, and generation
/// lifetime. A frontend must reject or retire the handle when its owner or
/// metrics generation is no longer current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderRunHandle {
    pub owner: RenderRunOwner,
    pub identifier: u64,
    pub metrics_generation: MetricsGeneration,
    pub threading: RenderRunThreading,
}

/// Resource policy declared by the shaping provider and copied into every
/// render-data request. Returned handles must match it exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderRunPolicy {
    pub owner: RenderRunOwner,
    pub threading: RenderRunThreading,
}

impl TextMetrics {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent + self.leading
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.ascent.is_finite()
            && self.ascent >= 0.0
            && self.descent.is_finite()
            && self.descent >= 0.0
            && self.leading.is_finite()
            && self.leading >= 0.0
            && self.height() > 0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClusterCaretStop {
    /// Global formatted UTF-8 byte boundary.
    pub text_offset: usize,
    /// Distance from the visual left edge of this cluster.
    pub inline_offset: f32,
    pub affinity: BoundaryAffinity,
}

/// One indivisible visual shaping cluster. It may contain several extended
/// grapheme clusters (for example when a platform shaper exposes a ligature as
/// one caret unit).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedCluster {
    pub text_range: Range<usize>,
    pub advance: f32,
    pub metrics: TextMetrics,
    /// Typographic bounds relative to the cluster's visual-left baseline.
    pub typographic_bounds: ShapedBounds,
    /// Ink bounds relative to the cluster's visual-left baseline.
    pub ink_bounds: ShapedBounds,
    pub bidi_level: u8,
    pub fallback_font: String,
    pub caret_stops: Vec<ClusterCaretStop>,
    /// Present when render data was requested. Metrics-only shaping must not
    /// retain a provider resource.
    pub render_run: Option<RenderRunHandle>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapingDiagnostic {
    pub text_range: Range<usize>,
    pub message: String,
}

/// Response corresponding to exactly one request at the same batch index.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedFragment {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub measurement_environment_id: MeasurementEnvironmentId,
    pub metrics_generation: MetricsGeneration,
    /// Echo of the request's ownership interior. Returned clusters are owned
    /// by this interval by logical start, but their ends may lie in the
    /// request's following context.
    pub text_range: Range<usize>,
    pub clusters: Vec<ShapedCluster>,
    /// Indices into `clusters`, in provider-resolved visual order. Core checks
    /// this permutation against the returned embedding levels, then resolves
    /// row-local order again after portable wrapping splits the fragment.
    pub visual_order: Vec<usize>,
    /// Supplies line geometry even when `text_range` is empty.
    pub default_metrics: TextMetrics,
    pub diagnostics: Vec<ShapingDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeasurementError {
    Provider(String),
    /// The provider cannot affirm that the requested ownership interiors are
    /// stable with the bounded context supplied in this batch. The diagnostic
    /// is provider-authored and must explain the unsupported shaping case.
    UnstableShapingContext(String),
    ResponseCount {
        expected: usize,
        actual: usize,
    },
}

/// Authoritative text measurement supplied by a platform frontend.
///
/// Implementations must return responses in request order. A provider may be
/// thread-confined, but that rule is declared by its frontend adapter rather
/// than inferred by core. No callback receives mutable document or view state.
/// Each successful response follows the context/ownership rule on
/// [`ShapeRequest`] and affirms that its echoed ownership interval is stable;
/// core validates the individual intervals and the gap-free cluster partition
/// assembled from adjacent responses. Providers decline with
/// [`MeasurementError::UnstableShapingContext`] rather than returning a result
/// whose dependence exceeds the bounded request context.
pub trait TextMeasurementProvider {
    /// Identifies the compatibility domain for measurements and render handles.
    /// This value MUST remain stable for the provider's lifetime.
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId;

    fn metrics_generation(&self) -> MetricsGeneration;

    /// Declares the registry and thread rule for opaque render resources.
    /// Providers used only for metrics may retain the default `None`.
    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        None
    }

    fn threading(&self) -> ProviderThreading {
        ProviderThreading::AnyWorker
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_shaping_style_rejects_ambiguous_or_empty_provider_inputs() {
        assert!(ResolvedTextStyle::default().is_valid());

        let missing_font = ResolvedTextStyle {
            font_families: Vec::new(),
            ..ResolvedTextStyle::default()
        };
        assert!(!missing_font.is_valid());

        let empty_language = ResolvedTextStyle {
            language: Some(String::new()),
            ..ResolvedTextStyle::default()
        };
        assert!(!empty_language.is_valid());

        let duplicate_feature = ResolvedTextStyle {
            features: vec![
                OpenTypeFeature {
                    tag: *b"liga",
                    value: 1,
                },
                OpenTypeFeature {
                    tag: *b"liga",
                    value: 0,
                },
            ],
            ..ResolvedTextStyle::default()
        };
        assert!(!duplicate_feature.is_valid());

        let nonprintable_feature = ResolvedTextStyle {
            features: vec![OpenTypeFeature {
                tag: [b'l', b'i', 0, b'a'],
                value: 1,
            }],
            ..ResolvedTextStyle::default()
        };
        assert!(!nonprintable_feature.is_valid());
    }
}
