//! Stable, ownership-safe C entry points for portable document and controller
//! state.
//!
//! The ABI deliberately exposes integer tokens rather than Rust pointers.
//! Tokens are process-local, nonzero, and never reused. Registry locks are
//! held only long enough to check a document or controller out for one serial
//! turn; model work, frontend callbacks, and caller-buffer copies happen after
//! the corresponding lock has been released. Concurrent access to the same
//! token receives a busy result instead of blocking. Destruction is likewise
//! nonblocking: destroying a checked-out token returns its typed busy status,
//! retains the token, and requires the caller to retry after the turn ends.
//!
//! Every content read is tagged with an expected immutable revision. Callers
//! first query the required byte length with a null output buffer and zero
//! capacity, then provide a sufficiently large buffer. Returned byte strings
//! are length-delimited and never NUL-terminated.

use crate::command::composition::{
    CompositionError, CompositionEvent, CompositionTarget, CompositionUpdate,
};
use crate::command::{CommandStatus, InputEvent, Key, Mode};
use crate::document::{
    BoundaryAffinity, Document, DocumentError, DocumentId, Encoding, FileFormat, FontSlant, Format,
    Revision,
};
use crate::layout::{
    ClusterCaretStop, LayoutError, LayoutExecutionContext, MeasurementEnvironmentId,
    MeasurementError, MetricsGeneration, ProviderThreading, RenderRunHandle, RenderRunOwner,
    RenderRunPolicy, RenderRunThreading, ResolvedTextStyle, ShapePurpose, ShapeRequest,
    ShapedBounds, ShapedCluster, ShapedFragment, ShapingDiagnostic, TextDirection,
    TextMeasurementProvider, TextMetrics,
};
use crate::{Core, CoreError, CoreEvent, CoreOutcome, ViewId};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{align_of, size_of};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::str;
use std::sync::{Mutex, OnceLock};

/// Version of the C ABI implemented by this library.
pub const EVIM_CORE_ABI_VERSION: u32 = 3;

/// First version of the injected text-measurement provider vtable.
pub const EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1: u32 = 1;
/// Adds paragraph base direction in the request's fixed-layout extension slot
/// and the context-owned cluster contract: shape the concatenated context and
/// interior, then return whole clusters whose logical start is in the stable
/// interior. Version 1 providers remain accepted with their exact-interior
/// response behavior.
pub const EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2: u32 = 2;
/// Current version of the injected text-measurement provider vtable.
pub const EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION: u32 =
    EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2;

/// Opaque process-local document token.
///
/// Zero is always invalid. Values have no relationship to a Rust or source
/// address and must not be inspected or synthesized by a caller.
pub type EvimDocumentHandle = u64;

/// Opaque process-local controller/core token. Zero is always invalid.
pub type EvimCoreHandle = u64;

/// Opaque view identity scoped to one core. Zero is always invalid, and a
/// removed value is never assigned to another view in that core.
pub type EvimViewId = u64;

pub const EVIM_ENCODING_UTF8: u32 = 1;
pub const EVIM_ENCODING_LATIN1: u32 = 2;
pub const EVIM_ENCODING_UTF16_LE: u32 = 3;
pub const EVIM_ENCODING_UTF16_BE: u32 = 4;

pub const EVIM_FORMAT_PLAIN_TEXT: u32 = 1;
pub const EVIM_FORMAT_MARKDOWN: u32 = 2;

/// Detect the line-ending interpretation through the core's shared open
/// policy.
pub const EVIM_FILE_FORMAT_DETECT: u32 = 0;
pub const EVIM_FILE_FORMAT_UNIX: u32 = 1;
pub const EVIM_FILE_FORMAT_DOS: u32 = 2;
pub const EVIM_FILE_FORMAT_MAC: u32 = 3;

/// Status returned by every fallible ABI operation.
///
/// The discriminants are part of ABI version 1 and therefore must not be
/// reordered or reused.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvimStatus {
    Ok = 0,
    InvalidArgument = 1,
    NullPointer = 2,
    InvalidHandle = 3,
    StaleRevision = 4,
    InvalidUtf8 = 5,
    InvalidEncoding = 6,
    InvalidFormat = 7,
    InvalidFileFormat = 8,
    BufferTooSmall = 9,
    InvalidRange = 10,
    NotGraphemeBoundary = 11,
    UnrepresentableCharacter = 12,
    AmbiguousProjection = 13,
    UnsupportedOperation = 14,
    PolicyRequired = 15,
    ResourceExhausted = 16,
    VerificationFailed = 17,
    LengthOverflow = 18,
    DocumentBusy = 19,
    CoreBusy = 20,
    InvalidView = 21,
    InvalidProvider = 22,
    ProviderFailure = 23,
    InvalidKey = 24,
    CoreFailure = 25,
    /// The shaping provider cannot guarantee a stable interior from the
    /// bounded context supplied by core. No partial layout is installed.
    UnstableShapingContext = 26,
    /// Absolute vertical viewport control is not yet available through the
    /// host ABI. No origin component changed.
    VerticalViewportOriginUnsupported = 27,
    InternalError = 254,
    Panic = 255,
}

/// Versioned options for [`evim_document_create`].
///
/// `struct_size` must be at least [`EVIM_DOCUMENT_OPTIONS_SIZE`]. A larger
/// value is accepted so future callers can append fields while retaining an
/// ABI-v1 prefix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimDocumentOptions {
    pub struct_size: u32,
    pub encoding: u32,
    pub format: u32,
    pub file_format: u32,
}

pub const EVIM_DOCUMENT_OPTIONS_SIZE: u32 = std::mem::size_of::<EvimDocumentOptions>() as u32;

impl Default for EvimDocumentOptions {
    fn default() -> Self {
        Self {
            struct_size: EVIM_DOCUMENT_OPTIONS_SIZE,
            encoding: EVIM_ENCODING_UTF8,
            format: EVIM_FORMAT_PLAIN_TEXT,
            file_format: EVIM_FILE_FORMAT_DETECT,
        }
    }
}

pub const EVIM_PROVIDER_THREADING_ANY_WORKER: u32 = 1;
pub const EVIM_PROVIDER_THREADING_DEDICATED_SERIAL: u32 = 2;
pub const EVIM_PROVIDER_THREADING_FRONTEND_MAIN: u32 = 3;

pub const EVIM_RENDER_THREADING_ANY: u32 = 1;
pub const EVIM_RENDER_THREADING_DEDICATED_SERIAL: u32 = 2;
pub const EVIM_RENDER_THREADING_FRONTEND_MAIN: u32 = 3;

pub const EVIM_LAYOUT_EXECUTION_WORKER_POOL: u32 = 1;
pub const EVIM_LAYOUT_EXECUTION_DEDICATED_SERIAL: u32 = 2;
pub const EVIM_LAYOUT_EXECUTION_FRONTEND_MAIN: u32 = 3;

pub const EVIM_TEXT_DIRECTION_AUTO: u32 = 0;
pub const EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT: u32 = 1;
pub const EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT: u32 = 2;

pub const EVIM_FONT_SLANT_UPRIGHT: u32 = 0;
pub const EVIM_FONT_SLANT_ITALIC: u32 = 1;
pub const EVIM_FONT_SLANT_OBLIQUE: u32 = 2;

pub const EVIM_SHAPE_PURPOSE_METRICS_ONLY: u32 = 1;
pub const EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA: u32 = 2;

pub const EVIM_BOUNDARY_AFFINITY_UPSTREAM: u32 = 1;
pub const EVIM_BOUNDARY_AFFINITY_DOWNSTREAM: u32 = 2;

pub const EVIM_KEY_CHARACTER: u32 = 1;
pub const EVIM_KEY_ESCAPE: u32 = 2;
pub const EVIM_KEY_ENTER: u32 = 3;
pub const EVIM_KEY_TAB: u32 = 4;
pub const EVIM_KEY_BACKSPACE: u32 = 5;
pub const EVIM_KEY_DELETE: u32 = 6;
pub const EVIM_KEY_LEFT: u32 = 7;
pub const EVIM_KEY_RIGHT: u32 = 8;
pub const EVIM_KEY_UP: u32 = 9;
pub const EVIM_KEY_DOWN: u32 = 10;
pub const EVIM_KEY_HOME: u32 = 11;
pub const EVIM_KEY_END: u32 = 12;
pub const EVIM_KEY_PAGE_UP: u32 = 13;
pub const EVIM_KEY_PAGE_DOWN: u32 = 14;
pub const EVIM_KEY_CONTROL_CHARACTER: u32 = 15;

pub const EVIM_COMMAND_STATUS_NONE: u32 = 0;
pub const EVIM_COMMAND_STATUS_COMPLETE: u32 = 1;
pub const EVIM_COMMAND_STATUS_PENDING: u32 = 2;
pub const EVIM_COMMAND_STATUS_CANCELLED: u32 = 3;
pub const EVIM_COMMAND_STATUS_NEEDS_MORE_LAYOUT: u32 = 4;
pub const EVIM_COMMAND_STATUS_SEARCH_NOT_FOUND: u32 = 5;
pub const EVIM_COMMAND_STATUS_UNSUPPORTED: u32 = 6;
pub const EVIM_COMMAND_STATUS_ERROR: u32 = 7;

pub const EVIM_MODE_NORMAL: u32 = 1;
pub const EVIM_MODE_INSERT: u32 = 2;
pub const EVIM_MODE_REPLACE: u32 = 3;
pub const EVIM_MODE_VISUAL_CHARACTER: u32 = 4;
pub const EVIM_MODE_VISUAL_LINE: u32 = 5;
pub const EVIM_MODE_VISUAL_BLOCK: u32 = 6;
pub const EVIM_MODE_COMMAND_LINE: u32 = 7;

pub const EVIM_OUTCOME_HAS_COMMAND: u32 = 1 << 0;
pub const EVIM_OUTCOME_CURSOR_MOVED: u32 = 1 << 1;
pub const EVIM_OUTCOME_DOCUMENT_CHANGED: u32 = 1 << 2;
pub const EVIM_OUTCOME_MODE_CHANGED: u32 = 1 << 3;
pub const EVIM_OUTCOME_LAYOUT_CHANGED: u32 = 1 << 4;
pub const EVIM_OUTCOME_HAS_POSITION_MAP: u32 = 1 << 5;
pub const EVIM_OUTCOME_HAS_LAYOUT: u32 = 1 << 6;
pub const EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS: u32 = 1 << 7;
pub const EVIM_OUTCOME_HAS_COMPOSITION_CHANGES: u32 = 1 << 8;

/// Length-delimited UTF-8. A null pointer is valid only when `length` is zero.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimUtf8Slice {
    pub data: *const u8,
    pub length: u64,
}

impl Default for EvimUtf8Slice {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            length: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimTextMetricsV1 {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimShapedBoundsV1 {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimOpenTypeFeatureV1 {
    pub tag: [u8; 4],
    pub value: u32,
}

/// ABI-v1 resolved shaping style supplied to the frontend callback.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimResolvedTextStyleV1 {
    pub struct_size: u32,
    pub slant: u32,
    pub direction: u32,
    pub has_language: u32,
    pub has_script: u32,
    pub reserved: u32,
    pub size: f32,
    pub weight: f32,
    pub letter_spacing: f32,
    pub baseline_shift: f32,
    pub font_families: *const EvimUtf8Slice,
    pub font_family_count: u64,
    pub language: EvimUtf8Slice,
    pub script: EvimUtf8Slice,
    pub features: *const EvimOpenTypeFeatureV1,
    pub feature_count: u64,
}

pub const EVIM_RESOLVED_TEXT_STYLE_V1_SIZE: u32 = size_of::<EvimResolvedTextStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimShapeStyleRunV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub style: EvimResolvedTextStyleV1,
}

pub const EVIM_SHAPE_STYLE_RUN_V1_SIZE: u32 = size_of::<EvimShapeStyleRunV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Provider-owned opaque render resource.
///
/// `identifier` is an integer token, never a native pointer for core to
/// dereference. Core only compares, caches, and transports the value. The
/// provider MUST keep it valid for its declared owner and threading rule while
/// `metrics_generation` remains current and the owning view remains attached.
/// It MAY retire the token as soon as either condition stops being true. No
/// retain or release crosses ABI v1; callers MUST NOT retain a token after its
/// generation becomes stale or its view is removed/core is destroyed.
pub struct EvimRenderRunHandleV1 {
    pub owner: u64,
    pub identifier: u64,
    pub metrics_generation: u64,
    pub threading: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimClusterCaretStopV1 {
    pub text_offset: u64,
    pub inline_offset: f32,
    pub affinity: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimShapedClusterV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub advance: f32,
    pub metrics: EvimTextMetricsV1,
    pub typographic_bounds: EvimShapedBoundsV1,
    pub ink_bounds: EvimShapedBoundsV1,
    pub bidi_level: u32,
    pub has_render_run: u32,
    pub fallback_font: EvimUtf8Slice,
    pub caret_stops: *const EvimClusterCaretStopV1,
    pub caret_stop_count: u64,
    pub render_run: EvimRenderRunHandleV1,
}

pub const EVIM_SHAPED_CLUSTER_V1_SIZE: u32 = size_of::<EvimShapedClusterV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimShapingDiagnosticV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub message: EvimUtf8Slice,
}

pub const EVIM_SHAPING_DIAGNOSTIC_V1_SIZE: u32 = size_of::<EvimShapingDiagnosticV1>() as u32;

/// One immutable shaping request. Every pointer is borrowed only for the
/// synchronous `shape_batch` callback. For provider ABI v2, `text_start` and
/// `text_end` delimit the stable ownership interior represented by `text`.
/// Shape `context_before + text + context_after`, and return each whole cluster
/// whose logical start lies in that interior. Such a cluster may end in the
/// following context; a cluster beginning in preceding context is omitted.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimShapeRequestV1 {
    pub struct_size: u32,
    pub purpose: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub text: EvimUtf8Slice,
    pub context_before: EvimUtf8Slice,
    pub context_after: EvimUtf8Slice,
    pub style_runs: *const EvimShapeStyleRunV1,
    pub style_run_count: u64,
    pub default_style: EvimResolvedTextStyleV1,
    pub scale: f32,
    pub has_render_run_policy: u32,
    pub render_run_owner: u64,
    pub render_run_threading: u32,
    /// `EVIM_TEXT_DIRECTION_*` for provider ABI v2. Core writes zero (Auto)
    /// for a v1 provider, where this fixed-layout slot was reserved.
    pub paragraph_base_direction: u32,
}

pub const EVIM_SHAPE_REQUEST_V1_SIZE: u32 = size_of::<EvimShapeRequestV1>() as u32;

/// One callback-owned shaping response. All pointed-to arrays and UTF-8 bytes
/// must remain readable when the callback returns and until the next provider
/// callback for this view; core copies them immediately. Response `text_start`
/// and `text_end` echo the request's ownership interior, not the union of the
/// returned cluster ranges.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimShapeResponseV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub clusters: *const EvimShapedClusterV1,
    pub cluster_count: u64,
    pub visual_order: *const u64,
    pub visual_order_count: u64,
    pub default_metrics: EvimTextMetricsV1,
    pub diagnostics: *const EvimShapingDiagnosticV1,
    pub diagnostic_count: u64,
}

impl Default for EvimShapeResponseV1 {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            reserved: 0,
            document_id: 0,
            document_revision: 0,
            measurement_environment_id: 0,
            metrics_generation: 0,
            text_start: 0,
            text_end: 0,
            clusters: std::ptr::null(),
            cluster_count: 0,
            visual_order: std::ptr::null(),
            visual_order_count: 0,
            default_metrics: EvimTextMetricsV1::default(),
            diagnostics: std::ptr::null(),
            diagnostic_count: 0,
        }
    }
}

pub const EVIM_SHAPE_RESPONSE_V1_SIZE: u32 = size_of::<EvimShapeResponseV1>() as u32;

pub type EvimMetricsGenerationCallback = unsafe extern "C" fn(context: *mut c_void) -> u64;
pub type EvimShapeBatchCallback = unsafe extern "C" fn(
    context: *mut c_void,
    requests: *const EvimShapeRequestV1,
    request_count: u64,
    responses: *mut EvimShapeResponseV1,
    response_capacity: u64,
) -> u32;

/// Versioned frontend-owned provider table. Core copies this prefix while
/// adding a view; `context` and callback-owned response storage must remain
/// valid until that view is removed or its core is successfully destroyed. A
/// destroy call returning [`EvimStatus::CoreBusy`] has not destroyed the core
/// and does not end that lifetime. Render-run tokens have the separate
/// generation-scoped lifetime documented on [`EvimRenderRunHandleV1`]. A
/// successful ABI-v2 callback affirms stable ownership interiors. A provider
/// unable to make that bounded-context guarantee returns
/// [`EvimStatus::UnstableShapingContext`]; core caches and installs none of that
/// batch.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimTextMeasurementProviderV1 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub context: *mut c_void,
    pub measurement_environment_id: u64,
    pub threading: u32,
    pub has_render_run_policy: u32,
    pub render_run_owner: u64,
    pub render_run_threading: u32,
    pub reserved: u32,
    pub metrics_generation: Option<EvimMetricsGenerationCallback>,
    pub shape_batch: Option<EvimShapeBatchCallback>,
}

pub const EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE: u32 =
    size_of::<EvimTextMeasurementProviderV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvimViewOptionsV1 {
    pub struct_size: u32,
    pub execution_context: u32,
    pub width: f32,
    pub height: f32,
}

pub const EVIM_VIEW_OPTIONS_V1_SIZE: u32 = size_of::<EvimViewOptionsV1>() as u32;

impl Default for EvimViewOptionsV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_VIEW_OPTIONS_V1_SIZE,
            execution_context: EVIM_LAYOUT_EXECUTION_WORKER_POOL,
            width: 800.0,
            height: 600.0,
        }
    }
}

/// Request an absolute per-view presentation origin. `left` is always
/// requested. `EVIM_VIEWPORT_ORIGIN_HAS_TOP` is reserved for the future exact
/// vertical protocol and currently returns `VerticalViewportOriginUnsupported`
/// without changing either component.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvimViewportOriginV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    /// Exact state identity from `EvimViewportStateV1`. These fields are
    /// ignored for horizontal-only requests and reserve a safe revision-bound
    /// contract for a future vertical request version.
    pub expected_document_id: u64,
    pub expected_document_revision: u64,
    pub expected_layout_revision: u64,
    pub expected_configuration_generation: u64,
    pub expected_measurement_environment_id: u64,
    pub expected_metrics_generation: u64,
}

pub const EVIM_VIEWPORT_ORIGIN_V1_SIZE: u32 = size_of::<EvimViewportOriginV1>() as u32;
pub const EVIM_VIEWPORT_ORIGIN_HAS_TOP: u32 = 1 << 0;

impl Default for EvimViewportOriginV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_VIEWPORT_ORIGIN_V1_SIZE,
            flags: 0,
            left: 0.0,
            top: 0.0,
            expected_document_id: 0,
            expected_document_revision: 0,
            expected_layout_revision: 0,
            expected_configuration_generation: 0,
            expected_measurement_environment_id: 0,
            expected_metrics_generation: 0,
        }
    }
}

pub const EVIM_VIEWPORT_STATE_WRAP: u32 = 1 << 0;
pub const EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT: u32 = 1 << 1;
pub const EVIM_VIEWPORT_STATE_TOP_EXACT: u32 = 1 << 2;
pub const EVIM_VIEWPORT_STATE_HAS_LAYOUT: u32 = 1 << 3;

/// Current presentation origin and the exact dependency identity observed in
/// the same serial query. `maximum_left` is usable only when its exact flag is
/// set. A missing exact top flag means the installed snapshot uses an
/// estimated prefix; the value remains the view's current coordinate but must
/// not be treated as a durable absolute document position.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvimViewportStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    pub maximum_left: f32,
    pub reserved: f32,
    pub document_id: u64,
    pub document_revision: u64,
    pub layout_revision: u64,
    pub configuration_generation: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
}

pub const EVIM_VIEWPORT_STATE_V1_SIZE: u32 = size_of::<EvimViewportStateV1>() as u32;

impl Default for EvimViewportStateV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_VIEWPORT_STATE_V1_SIZE,
            flags: 0,
            left: 0.0,
            top: 0.0,
            maximum_left: 0.0,
            reserved: 0.0,
            document_id: 0,
            document_revision: 0,
            layout_revision: 0,
            configuration_generation: 0,
            measurement_environment_id: 0,
            metrics_generation: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimKeyInputV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub codepoint: u32,
    pub reserved: u32,
}

pub const EVIM_KEY_INPUT_V1_SIZE: u32 = size_of::<EvimKeyInputV1>() as u32;

/// Exact formatted-snapshot replacement target for beginning native marked
/// text. The frontend supplies its current selection, or an empty range at its
/// insertion caret, in UTF-8 byte offsets for `document_revision`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimCompositionBeginV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub replacement_start: u64,
    pub replacement_end: u64,
}

pub const EVIM_COMPOSITION_BEGIN_V1_SIZE: u32 = size_of::<EvimCompositionBeginV1>() as u32;

/// One replacement of the temporary marked-text overlay. Selection offsets
/// are relative to `marked_text` and are UTF-8 byte offsets.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimCompositionUpdateV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub marked_text: EvimUtf8Slice,
    pub selected_start: u64,
    pub selected_end: u64,
}

pub const EVIM_COMPOSITION_UPDATE_V1_SIZE: u32 = size_of::<EvimCompositionUpdateV1>() as u32;

/// Final UTF-8 payload to install as one authoritative document transaction
/// and one undo unit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimCompositionCommitV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
    pub committed_text: EvimUtf8Slice,
}

pub const EVIM_COMPOSITION_COMMIT_V1_SIZE: u32 = size_of::<EvimCompositionCommitV1>() as u32;

/// Revision precondition for explicitly cancelling marked text.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimCompositionCancelV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_revision: u64,
}

pub const EVIM_COMPOSITION_CANCEL_V1_SIZE: u32 = size_of::<EvimCompositionCancelV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimCoreOutcomeV1 {
    pub struct_size: u32,
    pub command_status: u32,
    pub mode: u32,
    pub flags: u32,
    pub document_revision: u64,
    pub view_id: u64,
    pub cursor_utf8_offset: u64,
    pub layout_revision: u64,
    pub configuration_generation: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
}

pub const EVIM_CORE_OUTCOME_V1_SIZE: u32 = size_of::<EvimCoreOutcomeV1>() as u32;

impl Default for EvimCoreOutcomeV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_CORE_OUTCOME_V1_SIZE,
            command_status: EVIM_COMMAND_STATUS_NONE,
            mode: 0,
            flags: 0,
            document_revision: 0,
            view_id: 0,
            cursor_utf8_offset: 0,
            layout_revision: 0,
            configuration_generation: 0,
            measurement_environment_id: 0,
            metrics_generation: 0,
        }
    }
}

struct Registry {
    next_handle: EvimDocumentHandle,
    documents: HashMap<EvimDocumentHandle, RegistryEntry>,
}

enum RegistryEntry {
    Ready(Document),
    Busy,
}

/// Exclusive, nonblocking coordinator-style turn for one document.
///
/// Dropping the lease returns the model to its handle. Destruction observes the
/// busy marker and fails without removing it. This also runs during unwinding,
/// before the outer ABI panic boundary returns, so a contained panic cannot
/// strand a live handle in the busy state.
struct DocumentLease {
    handle: EvimDocumentHandle,
    document: Option<Document>,
}

impl DocumentLease {
    fn document(&self) -> &Document {
        self.document
            .as_ref()
            .expect("a live document lease owns its model")
    }

    fn document_mut(&mut self) -> &mut Document {
        self.document
            .as_mut()
            .expect("a live document lease owns its model")
    }
}

impl Drop for DocumentLease {
    fn drop(&mut self) {
        let Some(document) = self.document.take() else {
            return;
        };
        let Ok(mut registry) = registry().lock() else {
            // A poisoned process-global registry cannot safely publish the
            // model again. The handle remains unusable rather than exposing a
            // potentially incoherent state across the ABI.
            return;
        };
        if let Some(entry @ RegistryEntry::Busy) = registry.documents.get_mut(&self.handle) {
            *entry = RegistryEntry::Ready(document);
        }
        // Absence is only possible after registry corruption: destroy retains
        // a Busy entry and callers cannot otherwise remove entries.
    }
}

impl Registry {
    fn new() -> Self {
        Self {
            next_handle: 1,
            documents: HashMap::new(),
        }
    }
}

static DOCUMENTS: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    DOCUMENTS.get_or_init(|| Mutex::new(Registry::new()))
}

fn ffi_boundary(operation: impl FnOnce() -> Result<(), EvimStatus>) -> EvimStatus {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => EvimStatus::Ok,
        Ok(Err(status)) => status,
        Err(_) => EvimStatus::Panic,
    }
}

fn parse_encoding(raw: u32) -> Result<Encoding, EvimStatus> {
    match raw {
        EVIM_ENCODING_UTF8 => Ok(Encoding::Utf8),
        EVIM_ENCODING_LATIN1 => Ok(Encoding::Latin1),
        EVIM_ENCODING_UTF16_LE => Ok(Encoding::Utf16Le),
        EVIM_ENCODING_UTF16_BE => Ok(Encoding::Utf16Be),
        _ => Err(EvimStatus::InvalidEncoding),
    }
}

fn parse_format(raw: u32) -> Result<Format, EvimStatus> {
    match raw {
        EVIM_FORMAT_PLAIN_TEXT => Ok(Format::PlainText),
        EVIM_FORMAT_MARKDOWN => Ok(Format::Markdown),
        _ => Err(EvimStatus::InvalidFormat),
    }
}

fn parse_file_format(raw: u32) -> Result<Option<FileFormat>, EvimStatus> {
    match raw {
        EVIM_FILE_FORMAT_DETECT => Ok(None),
        EVIM_FILE_FORMAT_UNIX => Ok(Some(FileFormat::Unix)),
        EVIM_FILE_FORMAT_DOS => Ok(Some(FileFormat::Dos)),
        EVIM_FILE_FORMAT_MAC => Ok(Some(FileFormat::Mac)),
        _ => Err(EvimStatus::InvalidFileFormat),
    }
}

fn checked_length(length: u64) -> Result<usize, EvimStatus> {
    usize::try_from(length)
        .ok()
        .filter(|length| *length <= isize::MAX as usize)
        .ok_or(EvimStatus::LengthOverflow)
}

fn pointer_ranges_overlap(
    first: *const u8,
    first_length: usize,
    second: *const u8,
    second_length: usize,
) -> bool {
    if first_length == 0 || second_length == 0 {
        return false;
    }
    let first_start = first as usize;
    let second_start = second as usize;
    let Some(first_end) = first_start.checked_add(first_length) else {
        return true;
    };
    let Some(second_end) = second_start.checked_add(second_length) else {
        return true;
    };
    first_start < second_end && second_start < first_end
}

/// Borrow a length-delimited caller input for the duration of one ABI call.
///
/// # Safety
///
/// When `length` is nonzero, `pointer` must identify `length` readable bytes
/// which remain alive and are not mutated for the returned borrow's lifetime.
unsafe fn input_bytes<'a>(pointer: *const u8, length: u64) -> Result<&'a [u8], EvimStatus> {
    let length = checked_length(length)?;
    if length == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(EvimStatus::NullPointer);
    }
    // SAFETY: The caller contract above guarantees a live readable region;
    // null and target-size overflow were checked before constructing it.
    Ok(unsafe { slice::from_raw_parts(pointer, length) })
}

/// Read the ABI-v1 options prefix from caller memory.
///
/// # Safety
///
/// `pointer` must be null or point to a readable, properly aligned
/// `EvimDocumentOptions` value for the duration of this call.
unsafe fn read_options(
    pointer: *const EvimDocumentOptions,
) -> Result<EvimDocumentOptions, EvimStatus> {
    if pointer.is_null() {
        return Err(EvimStatus::NullPointer);
    }
    if (pointer as usize) % align_of::<EvimDocumentOptions>() != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    // SAFETY: The caller contract guarantees a readable, aligned value and
    // the null case was rejected above. The C representation makes this a
    // field-for-field copy with no Rust-owned resources.
    let options = unsafe { pointer.read() };
    if options.struct_size < EVIM_DOCUMENT_OPTIONS_SIZE {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(options)
}

fn register_document(document: Document) -> Result<EvimDocumentHandle, EvimStatus> {
    let mut registry = registry().lock().map_err(|_| EvimStatus::InternalError)?;
    let handle = registry.next_handle;
    if handle == 0 {
        return Err(EvimStatus::ResourceExhausted);
    }
    if registry.documents.contains_key(&handle) {
        return Err(EvimStatus::InternalError);
    }
    registry.next_handle = handle.checked_add(1).unwrap_or(0);
    registry
        .documents
        .insert(handle, RegistryEntry::Ready(document));
    Ok(handle)
}

fn checkout_document(handle: EvimDocumentHandle) -> Result<DocumentLease, EvimStatus> {
    if handle == 0 {
        return Err(EvimStatus::InvalidHandle);
    }
    // Move the model out while holding the registry lock. No document method,
    // parsing, projection, callback, allocation-heavy work, or output copy is
    // performed while this process-global lock is held.
    let mut registry = registry().lock().map_err(|_| EvimStatus::InternalError)?;
    let entry = registry
        .documents
        .get_mut(&handle)
        .ok_or(EvimStatus::InvalidHandle)?;
    let RegistryEntry::Ready(_) = entry else {
        return Err(EvimStatus::DocumentBusy);
    };
    let RegistryEntry::Ready(document) = std::mem::replace(entry, RegistryEntry::Busy) else {
        unreachable!("the ready entry was matched above")
    };
    Ok(DocumentLease {
        handle,
        document: Some(document),
    })
}

fn with_document<R>(
    handle: EvimDocumentHandle,
    operation: impl FnOnce(&Document) -> Result<R, EvimStatus>,
) -> Result<R, EvimStatus> {
    let document = checkout_document(handle)?;
    operation(document.document())
}

fn with_document_mut<R>(
    handle: EvimDocumentHandle,
    operation: impl FnOnce(&mut Document) -> Result<R, EvimStatus>,
) -> Result<R, EvimStatus> {
    let mut document = checkout_document(handle)?;
    operation(document.document_mut())
}

fn validate_revision(document: &Document, expected: u64) -> Result<(), EvimStatus> {
    if document.revision() == Revision(expected) {
        Ok(())
    } else {
        Err(EvimStatus::StaleRevision)
    }
}

fn document_status(error: DocumentError) -> EvimStatus {
    match error {
        DocumentError::InvalidRange { .. } => EvimStatus::InvalidRange,
        DocumentError::NotGraphemeBoundary(_) => EvimStatus::NotGraphemeBoundary,
        DocumentError::WrongSnapshot { .. } => EvimStatus::StaleRevision,
        DocumentError::WrongDocument => EvimStatus::InvalidArgument,
        DocumentError::InvalidEncoding { .. }
        | DocumentError::MismatchedBom
        | DocumentError::UnsupportedBom(_) => EvimStatus::InvalidEncoding,
        DocumentError::UnrepresentableCharacter { .. } => EvimStatus::UnrepresentableCharacter,
        DocumentError::AmbiguousProjection => EvimStatus::AmbiguousProjection,
        DocumentError::VerificationFailed
        | DocumentError::FormattedPayloadCannotReproject
        | DocumentError::HardLineTransferProjectionMismatch
        | DocumentError::HardLineSourceImageTopologyChanged { .. }
        | DocumentError::HardLineSourceImageTerminatorShapeChanged { .. }
        | DocumentError::HardLineSourceImageProjectionMismatch => EvimStatus::VerificationFailed,
        DocumentError::LineEndingConversionWouldReinterpretContent => EvimStatus::PolicyRequired,
        DocumentError::UnsupportedFormatting | DocumentError::OpaqueDecodingConflict { .. } => {
            EvimStatus::UnsupportedOperation
        }
        DocumentError::DocumentIdentityExhausted | DocumentError::BlockIdentityExhausted => {
            EvimStatus::ResourceExhausted
        }
        DocumentError::OverlappingEdits
        | DocumentError::OverlappingFormatting
        | DocumentError::InvalidHardLineTransferRange { .. }
        | DocumentError::InvalidHardLineTransferDestination { .. }
        | DocumentError::HardLineTransferDestinationInsideSource { .. }
        | DocumentError::InvalidHardLineSourceImageTarget { .. }
        | DocumentError::StaleHardLineSourceImage { .. }
        | DocumentError::IncompatibleHardLineSourceImage => EvimStatus::InvalidArgument,
        DocumentError::FormattedTextStorage(_) => EvimStatus::InternalError,
    }
}

fn snapshot_bytes(
    handle: EvimDocumentHandle,
    expected_revision: u64,
    source: bool,
) -> Result<Vec<u8>, EvimStatus> {
    with_document(handle, |document| {
        validate_revision(document, expected_revision)?;
        Ok(if source {
            document.source_bytes()
        } else {
            document.text().as_bytes().to_vec()
        })
    })
}

#[derive(Clone, Copy)]
struct CTextMeasurementProvider {
    context: usize,
    abi_version: u32,
    measurement_environment_id: MeasurementEnvironmentId,
    threading: ProviderThreading,
    render_run_policy: Option<RenderRunPolicy>,
    metrics_generation_callback: EvimMetricsGenerationCallback,
    shape_batch_callback: EvimShapeBatchCallback,
}

impl CTextMeasurementProvider {
    /// Copy and validate the v1 provider prefix. The frontend retains ownership
    /// of the context and every resource referenced by callbacks.
    unsafe fn from_ffi(provider: *const EvimTextMeasurementProviderV1) -> Result<Self, EvimStatus> {
        if provider.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if (provider as usize) % align_of::<EvimTextMeasurementProviderV1>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        // SAFETY: The API contract requires a readable aligned v1 prefix. Null
        // and alignment were checked before making the field-for-field copy.
        let provider = unsafe { provider.read() };
        if provider.struct_size < EVIM_TEXT_MEASUREMENT_PROVIDER_V1_SIZE
            || !(EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1
                ..=EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION)
                .contains(&provider.abi_version)
            || provider.reserved != 0
        {
            return Err(EvimStatus::InvalidProvider);
        }
        let threading = parse_provider_threading(provider.threading)?;
        let has_render_run_policy = parse_ffi_bool(provider.has_render_run_policy)?;
        let render_run_policy = if has_render_run_policy {
            Some(RenderRunPolicy {
                owner: RenderRunOwner(provider.render_run_owner),
                threading: parse_render_threading(provider.render_run_threading)?,
            })
        } else {
            None
        };
        let metrics_generation_callback = provider
            .metrics_generation
            .ok_or(EvimStatus::InvalidProvider)?;
        let shape_batch_callback = provider.shape_batch.ok_or(EvimStatus::InvalidProvider)?;
        Ok(Self {
            context: provider.context as usize,
            abi_version: provider.abi_version,
            measurement_environment_id: MeasurementEnvironmentId(
                provider.measurement_environment_id,
            ),
            threading,
            render_run_policy,
            metrics_generation_callback,
            shape_batch_callback,
        })
    }

    fn context(self) -> *mut c_void {
        self.context as *mut c_void
    }
}

struct MarshalledStyle {
    _font_families: Vec<EvimUtf8Slice>,
    _features: Vec<EvimOpenTypeFeatureV1>,
    ffi: EvimResolvedTextStyleV1,
}

impl MarshalledStyle {
    fn new(style: &ResolvedTextStyle) -> Self {
        let font_families: Vec<_> = style
            .font_families
            .iter()
            .map(|family| ffi_utf8_slice(family))
            .collect();
        let features: Vec<_> = style
            .features
            .iter()
            .map(|feature| EvimOpenTypeFeatureV1 {
                tag: feature.tag,
                value: feature.value,
            })
            .collect();
        let ffi = EvimResolvedTextStyleV1 {
            struct_size: EVIM_RESOLVED_TEXT_STYLE_V1_SIZE,
            slant: match style.slant {
                FontSlant::Upright => EVIM_FONT_SLANT_UPRIGHT,
                FontSlant::Italic => EVIM_FONT_SLANT_ITALIC,
                FontSlant::Oblique => EVIM_FONT_SLANT_OBLIQUE,
            },
            direction: match style.direction {
                TextDirection::Auto => EVIM_TEXT_DIRECTION_AUTO,
                TextDirection::LeftToRight => EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                TextDirection::RightToLeft => EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT,
            },
            has_language: u32::from(style.language.is_some()),
            has_script: u32::from(style.script.is_some()),
            reserved: 0,
            size: style.size,
            weight: style.weight,
            letter_spacing: style.letter_spacing,
            baseline_shift: style.baseline_shift,
            font_families: slice_pointer(&font_families),
            font_family_count: font_families.len() as u64,
            language: style
                .language
                .as_deref()
                .map_or_else(EvimUtf8Slice::default, ffi_utf8_slice),
            script: style
                .script
                .as_deref()
                .map_or_else(EvimUtf8Slice::default, ffi_utf8_slice),
            features: slice_pointer(&features),
            feature_count: features.len() as u64,
        };
        Self {
            _font_families: font_families,
            _features: features,
            ffi,
        }
    }
}

struct MarshalledRequest {
    default_style: MarshalledStyle,
    _run_styles: Vec<MarshalledStyle>,
    style_runs: Vec<EvimShapeStyleRunV1>,
}

impl MarshalledRequest {
    fn new(request: &ShapeRequest<'_>) -> Self {
        let default_style = MarshalledStyle::new(request.default_style);
        let run_styles: Vec<_> = request
            .style_runs
            .iter()
            .map(|run| MarshalledStyle::new(&run.style))
            .collect();
        let style_runs = request
            .style_runs
            .iter()
            .zip(&run_styles)
            .map(|(run, style)| EvimShapeStyleRunV1 {
                struct_size: EVIM_SHAPE_STYLE_RUN_V1_SIZE,
                reserved: 0,
                text_start: run.text_range.start as u64,
                text_end: run.text_range.end as u64,
                style: style.ffi,
            })
            .collect();
        Self {
            default_style,
            _run_styles: run_styles,
            style_runs,
        }
    }

    fn request(&self, request: &ShapeRequest<'_>, provider_abi_version: u32) -> EvimShapeRequestV1 {
        let (has_render_run_policy, render_run_owner, render_run_threading) =
            request.render_run_policy.map_or((0, 0, 0), |policy| {
                (1, policy.owner.0, ffi_render_threading(policy.threading))
            });
        EvimShapeRequestV1 {
            struct_size: EVIM_SHAPE_REQUEST_V1_SIZE,
            purpose: match request.purpose {
                ShapePurpose::MetricsOnly => EVIM_SHAPE_PURPOSE_METRICS_ONLY,
                ShapePurpose::MetricsAndRenderData => EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA,
            },
            document_id: request.document_id.0,
            document_revision: request.document_revision.0,
            measurement_environment_id: request.measurement_environment_id.0,
            metrics_generation: request.metrics_generation.0,
            text_start: request.text_range.start as u64,
            text_end: request.text_range.end as u64,
            text: ffi_utf8_slice(request.text),
            context_before: ffi_utf8_slice(request.context_before),
            context_after: ffi_utf8_slice(request.context_after),
            style_runs: slice_pointer(&self.style_runs),
            style_run_count: self.style_runs.len() as u64,
            default_style: self.default_style.ffi,
            scale: request.scale,
            has_render_run_policy,
            render_run_owner,
            render_run_threading,
            paragraph_base_direction: if provider_abi_version
                >= EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2
            {
                match request.paragraph_base_direction {
                    TextDirection::Auto => EVIM_TEXT_DIRECTION_AUTO,
                    TextDirection::LeftToRight => EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                    TextDirection::RightToLeft => EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT,
                }
            } else {
                EVIM_TEXT_DIRECTION_AUTO
            },
        }
    }
}

impl TextMeasurementProvider for CTextMeasurementProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.measurement_environment_id
    }

    fn metrics_generation(&self) -> MetricsGeneration {
        // SAFETY: The frontend guarantees that the copied function pointer and
        // context remain valid until view removal. Core owns no registry lock
        // while provider methods are called.
        MetricsGeneration(unsafe { (self.metrics_generation_callback)(self.context()) })
    }

    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.render_run_policy
    }

    fn threading(&self) -> ProviderThreading {
        self.threading
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        let storage: Vec<_> = requests.iter().map(MarshalledRequest::new).collect();
        let ffi_requests: Vec<_> = requests
            .iter()
            .zip(&storage)
            .map(|(request, storage)| storage.request(request, self.abi_version))
            .collect();
        let mut ffi_responses = vec![EvimShapeResponseV1::default(); requests.len()];
        // SAFETY: All request pointers refer to storage retained across this
        // synchronous callback. The response allocation has exactly the
        // advertised capacity. The frontend callback/context lifetime is the
        // view lifetime contract validated when the provider was installed.
        let callback_status = unsafe {
            (self.shape_batch_callback)(
                self.context(),
                slice_pointer(&ffi_requests),
                ffi_requests.len() as u64,
                slice_mut_pointer(&mut ffi_responses),
                ffi_responses.len() as u64,
            )
        };
        if callback_status == EvimStatus::UnstableShapingContext as u32 {
            return Err(MeasurementError::UnstableShapingContext(
                "C provider declined the bounded shaping context".to_owned(),
            ));
        }
        if callback_status != EvimStatus::Ok as u32 {
            return Err(MeasurementError::Provider(format!(
                "C provider callback returned status {callback_status}"
            )));
        }
        ffi_responses
            .iter()
            .map(|response| {
                // SAFETY: Successful callbacks guarantee that every returned
                // pointer/count pair remains readable while core copies it.
                unsafe { shaped_fragment_from_ffi(response) }
            })
            .collect()
    }
}

fn ffi_utf8_slice(value: &str) -> EvimUtf8Slice {
    EvimUtf8Slice {
        data: value.as_ptr(),
        length: value.len() as u64,
    }
}

fn slice_pointer<T>(values: &[T]) -> *const T {
    if values.is_empty() {
        std::ptr::null()
    } else {
        values.as_ptr()
    }
}

fn slice_mut_pointer<T>(values: &mut [T]) -> *mut T {
    if values.is_empty() {
        std::ptr::null_mut()
    } else {
        values.as_mut_ptr()
    }
}

fn ffi_render_threading(threading: RenderRunThreading) -> u32 {
    match threading {
        RenderRunThreading::AnyThread => EVIM_RENDER_THREADING_ANY,
        RenderRunThreading::DedicatedSerialExecutor => EVIM_RENDER_THREADING_DEDICATED_SERIAL,
        RenderRunThreading::FrontendMainThread => EVIM_RENDER_THREADING_FRONTEND_MAIN,
    }
}

fn parse_provider_threading(raw: u32) -> Result<ProviderThreading, EvimStatus> {
    match raw {
        EVIM_PROVIDER_THREADING_ANY_WORKER => Ok(ProviderThreading::AnyWorker),
        EVIM_PROVIDER_THREADING_DEDICATED_SERIAL => Ok(ProviderThreading::DedicatedSerialExecutor),
        EVIM_PROVIDER_THREADING_FRONTEND_MAIN => Ok(ProviderThreading::FrontendMainThread),
        _ => Err(EvimStatus::InvalidProvider),
    }
}

fn parse_render_threading(raw: u32) -> Result<RenderRunThreading, EvimStatus> {
    match raw {
        EVIM_RENDER_THREADING_ANY => Ok(RenderRunThreading::AnyThread),
        EVIM_RENDER_THREADING_DEDICATED_SERIAL => Ok(RenderRunThreading::DedicatedSerialExecutor),
        EVIM_RENDER_THREADING_FRONTEND_MAIN => Ok(RenderRunThreading::FrontendMainThread),
        _ => Err(EvimStatus::InvalidProvider),
    }
}

fn parse_ffi_bool(raw: u32) -> Result<bool, EvimStatus> {
    match raw {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

fn measurement_failure(message: impl Into<String>) -> MeasurementError {
    MeasurementError::Provider(message.into())
}

unsafe fn provider_slice<'a, T>(
    pointer: *const T,
    count: u64,
    field: &'static str,
) -> Result<&'a [T], MeasurementError> {
    let count = usize::try_from(count)
        .map_err(|_| measurement_failure(format!("{field} count overflows this platform")))?;
    let byte_count = count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or_else(|| measurement_failure(format!("{field} byte count is too large")))?;
    if count == 0 {
        return Ok(&[]);
    }
    if pointer.is_null() {
        return Err(measurement_failure(format!(
            "{field} pointer is null for a nonempty array"
        )));
    }
    if (pointer as usize) % align_of::<T>() != 0 {
        return Err(measurement_failure(format!(
            "{field} pointer is misaligned"
        )));
    }
    debug_assert!(byte_count > 0 || size_of::<T>() == 0);
    // SAFETY: The provider contract supplies a readable array; null,
    // alignment, multiplication overflow, and Rust's maximum slice size were
    // checked above.
    Ok(unsafe { slice::from_raw_parts(pointer, count) })
}

unsafe fn provider_utf8(
    value: EvimUtf8Slice,
    field: &'static str,
) -> Result<String, MeasurementError> {
    let bytes = unsafe { provider_slice(value.data, value.length, field)? };
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| measurement_failure(format!("{field} is not valid UTF-8")))
}

fn text_metrics_from_ffi(metrics: EvimTextMetricsV1) -> TextMetrics {
    TextMetrics {
        ascent: metrics.ascent,
        descent: metrics.descent,
        leading: metrics.leading,
    }
}

fn shaped_bounds_from_ffi(bounds: EvimShapedBoundsV1) -> ShapedBounds {
    ShapedBounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn affinity_from_ffi(raw: u32) -> Result<BoundaryAffinity, MeasurementError> {
    match raw {
        EVIM_BOUNDARY_AFFINITY_UPSTREAM => Ok(BoundaryAffinity::Upstream),
        EVIM_BOUNDARY_AFFINITY_DOWNSTREAM => Ok(BoundaryAffinity::Downstream),
        _ => Err(measurement_failure("invalid caret affinity")),
    }
}

fn render_threading_from_response(raw: u32) -> Result<RenderRunThreading, MeasurementError> {
    match raw {
        EVIM_RENDER_THREADING_ANY => Ok(RenderRunThreading::AnyThread),
        EVIM_RENDER_THREADING_DEDICATED_SERIAL => Ok(RenderRunThreading::DedicatedSerialExecutor),
        EVIM_RENDER_THREADING_FRONTEND_MAIN => Ok(RenderRunThreading::FrontendMainThread),
        _ => Err(measurement_failure("invalid render-run threading value")),
    }
}

unsafe fn shaped_fragment_from_ffi(
    response: &EvimShapeResponseV1,
) -> Result<ShapedFragment, MeasurementError> {
    if response.struct_size < EVIM_SHAPE_RESPONSE_V1_SIZE || response.reserved != 0 {
        return Err(measurement_failure("shaping response prefix is too small"));
    }
    let clusters = unsafe {
        provider_slice(
            response.clusters,
            response.cluster_count,
            "response clusters",
        )?
    }
    .iter()
    .map(|cluster| unsafe { shaped_cluster_from_ffi(cluster) })
    .collect::<Result<Vec<_>, _>>()?;
    let visual_order = unsafe {
        provider_slice(
            response.visual_order,
            response.visual_order_count,
            "response visual order",
        )?
    }
    .iter()
    .map(|index| {
        usize::try_from(*index)
            .map_err(|_| measurement_failure("visual-order index overflows this platform"))
    })
    .collect::<Result<Vec<_>, _>>()?;
    let diagnostics = unsafe {
        provider_slice(
            response.diagnostics,
            response.diagnostic_count,
            "response diagnostics",
        )?
    }
    .iter()
    .map(|diagnostic| unsafe { shaping_diagnostic_from_ffi(diagnostic) })
    .collect::<Result<Vec<_>, _>>()?;
    Ok(ShapedFragment {
        document_id: DocumentId(response.document_id),
        document_revision: Revision(response.document_revision),
        measurement_environment_id: MeasurementEnvironmentId(response.measurement_environment_id),
        metrics_generation: MetricsGeneration(response.metrics_generation),
        text_range: checked_response_offset(response.text_start)?
            ..checked_response_offset(response.text_end)?,
        clusters,
        visual_order,
        default_metrics: text_metrics_from_ffi(response.default_metrics),
        diagnostics,
    })
}

unsafe fn shaped_cluster_from_ffi(
    cluster: &EvimShapedClusterV1,
) -> Result<ShapedCluster, MeasurementError> {
    if cluster.struct_size < EVIM_SHAPED_CLUSTER_V1_SIZE || cluster.reserved != 0 {
        return Err(measurement_failure("shaped-cluster prefix is too small"));
    }
    let caret_stops = unsafe {
        provider_slice(
            cluster.caret_stops,
            cluster.caret_stop_count,
            "cluster caret stops",
        )?
    }
    .iter()
    .map(|caret| {
        Ok(ClusterCaretStop {
            text_offset: checked_response_offset(caret.text_offset)?,
            inline_offset: caret.inline_offset,
            affinity: affinity_from_ffi(caret.affinity)?,
        })
    })
    .collect::<Result<Vec<_>, MeasurementError>>()?;
    let has_render_run = match cluster.has_render_run {
        0 => false,
        1 => true,
        _ => return Err(measurement_failure("invalid render-run presence flag")),
    };
    let render_run = has_render_run
        .then(|| {
            if cluster.render_run.reserved != 0 {
                return Err(measurement_failure(
                    "render-run response reserved field is nonzero",
                ));
            }
            Ok(RenderRunHandle {
                owner: RenderRunOwner(cluster.render_run.owner),
                identifier: cluster.render_run.identifier,
                metrics_generation: MetricsGeneration(cluster.render_run.metrics_generation),
                threading: render_threading_from_response(cluster.render_run.threading)?,
            })
        })
        .transpose()?;
    let bidi_level = u8::try_from(cluster.bidi_level)
        .map_err(|_| measurement_failure("bidi level exceeds u8"))?;
    Ok(ShapedCluster {
        text_range: checked_response_offset(cluster.text_start)?
            ..checked_response_offset(cluster.text_end)?,
        advance: cluster.advance,
        metrics: text_metrics_from_ffi(cluster.metrics),
        typographic_bounds: shaped_bounds_from_ffi(cluster.typographic_bounds),
        ink_bounds: shaped_bounds_from_ffi(cluster.ink_bounds),
        bidi_level,
        fallback_font: unsafe { provider_utf8(cluster.fallback_font, "fallback font")? },
        caret_stops,
        render_run,
    })
}

unsafe fn shaping_diagnostic_from_ffi(
    diagnostic: &EvimShapingDiagnosticV1,
) -> Result<ShapingDiagnostic, MeasurementError> {
    if diagnostic.struct_size < EVIM_SHAPING_DIAGNOSTIC_V1_SIZE || diagnostic.reserved != 0 {
        return Err(measurement_failure(
            "shaping-diagnostic prefix is too small",
        ));
    }
    Ok(ShapingDiagnostic {
        text_range: checked_response_offset(diagnostic.text_start)?
            ..checked_response_offset(diagnostic.text_end)?,
        message: unsafe { provider_utf8(diagnostic.message, "shaping diagnostic")? },
    })
}

fn checked_response_offset(value: u64) -> Result<usize, MeasurementError> {
    usize::try_from(value).map_err(|_| measurement_failure("text offset overflows this platform"))
}

enum CoreRegistryEntry {
    Ready(Core<CTextMeasurementProvider>),
    Busy,
}

struct CoreRegistry {
    next_handle: EvimCoreHandle,
    cores: HashMap<EvimCoreHandle, CoreRegistryEntry>,
}

impl CoreRegistry {
    fn new() -> Self {
        Self {
            next_handle: 1,
            cores: HashMap::new(),
        }
    }
}

static CORES: OnceLock<Mutex<CoreRegistry>> = OnceLock::new();

fn core_registry() -> &'static Mutex<CoreRegistry> {
    CORES.get_or_init(|| Mutex::new(CoreRegistry::new()))
}

/// Exclusive, nonblocking coordinator turn for one core.
///
/// Destruction observes the Busy marker without removing it, so dropping this
/// lease restores the same handle before any caller can successfully destroy
/// the core or release provider-owned callback state. Restoration also happens
/// during unwinding inside the outer ABI panic boundary.
struct CoreLease {
    handle: EvimCoreHandle,
    core: Option<Core<CTextMeasurementProvider>>,
}

impl CoreLease {
    fn core(&self) -> &Core<CTextMeasurementProvider> {
        self.core
            .as_ref()
            .expect("a live core lease owns its model")
    }

    fn core_mut(&mut self) -> &mut Core<CTextMeasurementProvider> {
        self.core
            .as_mut()
            .expect("a live core lease owns its model")
    }
}

impl Drop for CoreLease {
    fn drop(&mut self) {
        let Some(core) = self.core.take() else {
            return;
        };
        let Ok(mut registry) = core_registry().lock() else {
            return;
        };
        if let Some(entry @ CoreRegistryEntry::Busy) = registry.cores.get_mut(&self.handle) {
            *entry = CoreRegistryEntry::Ready(core);
        }
    }
}

fn register_core(core: Core<CTextMeasurementProvider>) -> Result<EvimCoreHandle, EvimStatus> {
    let mut registry = core_registry()
        .lock()
        .map_err(|_| EvimStatus::InternalError)?;
    let handle = registry.next_handle;
    if handle == 0 {
        return Err(EvimStatus::ResourceExhausted);
    }
    if registry.cores.contains_key(&handle) {
        return Err(EvimStatus::InternalError);
    }
    registry.next_handle = handle.checked_add(1).unwrap_or(0);
    registry
        .cores
        .insert(handle, CoreRegistryEntry::Ready(core));
    Ok(handle)
}

fn checkout_core(handle: EvimCoreHandle) -> Result<CoreLease, EvimStatus> {
    if handle == 0 {
        return Err(EvimStatus::InvalidHandle);
    }
    let mut registry = core_registry()
        .lock()
        .map_err(|_| EvimStatus::InternalError)?;
    let entry = registry
        .cores
        .get_mut(&handle)
        .ok_or(EvimStatus::InvalidHandle)?;
    let CoreRegistryEntry::Ready(_) = entry else {
        return Err(EvimStatus::CoreBusy);
    };
    let CoreRegistryEntry::Ready(core) = std::mem::replace(entry, CoreRegistryEntry::Busy) else {
        unreachable!("the ready entry was matched above")
    };
    Ok(CoreLease {
        handle,
        core: Some(core),
    })
}

fn with_core<R>(
    handle: EvimCoreHandle,
    operation: impl FnOnce(&Core<CTextMeasurementProvider>) -> Result<R, EvimStatus>,
) -> Result<R, EvimStatus> {
    let core = checkout_core(handle)?;
    operation(core.core())
}

fn with_core_mut<R>(
    handle: EvimCoreHandle,
    operation: impl FnOnce(&mut Core<CTextMeasurementProvider>) -> Result<R, EvimStatus>,
) -> Result<R, EvimStatus> {
    let mut core = checkout_core(handle)?;
    operation(core.core_mut())
}

/// Return the ABI version without consulting any document state.
#[no_mangle]
pub extern "C" fn evim_core_abi_version() -> u32 {
    catch_unwind(|| EVIM_CORE_ABI_VERSION).unwrap_or(0)
}

/// Create a document from an exact length-delimited source byte sequence.
///
/// On success, `out_document` receives a new nonzero token and `out_revision`
/// receives its initial revision (currently zero). Both outputs are cleared
/// before validation so a failed call cannot leave a plausible handle behind.
///
/// # Safety
///
/// - `options` must point to a readable, aligned options prefix.
/// - When `source_length` is nonzero, `source` must point to that many readable
///   bytes.
/// - `out_document` and `out_revision` must point to distinct, properly
///   aligned writable values.
/// - Readable inputs must not overlap the writable outputs, and all pointed-to
///   storage must remain valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn evim_document_create(
    source: *const u8,
    source_length: u64,
    options: *const EvimDocumentOptions,
    out_document: *mut EvimDocumentHandle,
    out_revision: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_document.is_null() || out_revision.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        // SAFETY: The function contract requires writable outputs and both
        // pointers were checked for null above.
        unsafe {
            out_document.write(0);
            out_revision.write(0);
        }

        // SAFETY: Forwarded directly from this function's pointer contracts.
        let options = unsafe { read_options(options)? };
        let encoding = parse_encoding(options.encoding)?;
        let format = parse_format(options.format)?;
        let file_format = parse_file_format(options.file_format)?;
        // SAFETY: Forwarded directly from this function's pointer contracts.
        let bytes = unsafe { input_bytes(source, source_length)? }.to_vec();

        let document = match file_format {
            Some(file_format) => {
                Document::from_bytes_with_file_format(bytes, encoding, format, file_format)
            }
            None => Document::from_bytes(bytes, encoding, format),
        }
        .map_err(document_status)?;
        let revision = document.revision().0;
        let handle = register_document(document)?;

        // SAFETY: The output pointers satisfy the function contract and stay
        // valid until this call returns.
        unsafe {
            out_document.write(handle);
            out_revision.write(revision);
        }
        Ok(())
    })
}

/// Destroy one document token.
///
/// If an operation has checked out the document, this returns
/// [`EvimStatus::DocumentBusy`] without removing the token. The caller may retry
/// after that operation returns. A successful call makes the token invalid;
/// tokens are never reused, so a stale token can never address a later
/// document.
#[no_mangle]
pub extern "C" fn evim_document_destroy(handle: EvimDocumentHandle) -> EvimStatus {
    ffi_boundary(|| {
        if handle == 0 {
            return Err(EvimStatus::InvalidHandle);
        }
        let document = {
            let mut registry = registry().lock().map_err(|_| EvimStatus::InternalError)?;
            match registry.documents.get(&handle) {
                None => return Err(EvimStatus::InvalidHandle),
                Some(RegistryEntry::Busy) => return Err(EvimStatus::DocumentBusy),
                Some(RegistryEntry::Ready(_)) => registry
                    .documents
                    .remove(&handle)
                    .ok_or(EvimStatus::InternalError)?,
            }
        };
        // Keep potentially allocation-heavy model destruction outside the
        // process-global registry lock.
        drop(document);
        Ok(())
    })
}

/// Query the current document revision.
///
/// # Safety
///
/// `out_revision` must point to a properly aligned writable `u64` which
/// remains valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn evim_document_revision(
    handle: EvimDocumentHandle,
    out_revision: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_revision.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        // SAFETY: The function contract requires a writable output and the
        // pointer was checked for null above.
        unsafe { out_revision.write(0) };
        let revision = with_document(handle, |document| Ok(document.revision().0))?;
        // SAFETY: The output remains valid until this call returns.
        unsafe { out_revision.write(revision) };
        Ok(())
    })
}

/// Copy the exact authoritative source bytes for `expected_revision`.
///
/// Pass a null `output` and zero `output_capacity` to query the required byte
/// count. A non-empty result then returns [`EvimStatus::BufferTooSmall`], with
/// `out_required` populated. No NUL terminator is written.
///
/// # Safety
///
/// - `out_required` must point to a properly aligned writable `u64`.
/// - When `output_capacity` is nonzero, `output` must point to that many
///   writable bytes.
/// - The two writable regions must not overlap and must remain valid for the
///   duration of the call.
#[no_mangle]
pub unsafe extern "C" fn evim_document_copy_source_bytes(
    handle: EvimDocumentHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    // SAFETY: This function exposes the same pointer contract as the shared
    // implementation and simply selects authoritative source bytes.
    unsafe {
        copy_snapshot_bytes(
            handle,
            expected_revision,
            true,
            output,
            output_capacity,
            out_required,
        )
    }
}

/// Copy the formatted snapshot's valid UTF-8 bytes for `expected_revision`.
///
/// This has the same two-pass and no-NUL semantics as
/// [`evim_document_copy_source_bytes`].
///
/// # Safety
///
/// - `out_required` must point to a properly aligned writable `u64`.
/// - When `output_capacity` is nonzero, `output` must point to that many
///   writable bytes.
/// - The two writable regions must not overlap and must remain valid for the
///   duration of the call.
#[no_mangle]
pub unsafe extern "C" fn evim_document_copy_formatted_utf8(
    handle: EvimDocumentHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    // SAFETY: This function exposes the same pointer contract as the shared
    // implementation and simply selects formatted UTF-8 bytes.
    unsafe {
        copy_snapshot_bytes(
            handle,
            expected_revision,
            false,
            output,
            output_capacity,
            out_required,
        )
    }
}

/// Shared two-pass snapshot copy implementation.
///
/// # Safety
///
/// `out_required` must be properly aligned and writable. `output` may be null
/// only when capacity is zero; otherwise it must identify `output_capacity`
/// writable bytes. Writable regions must not overlap.
unsafe fn copy_snapshot_bytes(
    handle: EvimDocumentHandle,
    expected_revision: u64,
    source: bool,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_required.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        // SAFETY: The caller contract requires a writable result pointer and
        // the null case was rejected above.
        unsafe { out_required.write(0) };
        let capacity = checked_length(output_capacity)?;
        if output.is_null() && capacity != 0 {
            return Err(EvimStatus::NullPointer);
        }

        let bytes = snapshot_bytes(handle, expected_revision, source)?;
        let required = u64::try_from(bytes.len()).map_err(|_| EvimStatus::LengthOverflow)?;
        // SAFETY: `out_required` remains writable for the complete call.
        unsafe { out_required.write(required) };

        if capacity < bytes.len() {
            return Err(EvimStatus::BufferTooSmall);
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if output.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        // SAFETY: Capacity was checked against `bytes.len()`; the caller owns
        // the writable destination and its non-overlap with `out_required` is
        // part of the function contract.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        Ok(())
    })
}

/// Replace one grapheme-aligned half-open range in an exact formatted
/// snapshot.
///
/// `replacement` is length-delimited UTF-8. On success, `out_revision`
/// receives the newly current revision. No range is clamped and no stale
/// ordinal is interpreted in a newer snapshot.
///
/// # Safety
///
/// - When `replacement_length` is nonzero, `replacement` must point to that
///   many readable bytes which remain immutable for the duration of the call.
/// - `out_revision` must point to a properly aligned writable `u64` which does
///   not overlap the replacement bytes.
#[no_mangle]
pub unsafe extern "C" fn evim_document_replace_formatted_utf8(
    handle: EvimDocumentHandle,
    expected_revision: u64,
    start: u64,
    end: u64,
    replacement: *const u8,
    replacement_length: u64,
    out_revision: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_revision.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        // SAFETY: The function contract requires a writable output and the
        // null case was rejected above.
        unsafe { out_revision.write(0) };
        let start = checked_length(start)?;
        let end = checked_length(end)?;
        // SAFETY: Forwarded directly from this function's pointer contract.
        let replacement = unsafe { input_bytes(replacement, replacement_length)? };
        let replacement = str::from_utf8(replacement).map_err(|_| EvimStatus::InvalidUtf8)?;

        let revision = with_document_mut(handle, |document| {
            validate_revision(document, expected_revision)?;
            let start = document.text_point(start).map_err(document_status)?;
            let end = document.text_point(end).map_err(document_status)?;
            document
                .replace_points(start, end, replacement)
                .map_err(document_status)?;
            Ok(document.revision().0)
        })?;

        // SAFETY: The caller-provided output remains valid until return.
        unsafe { out_revision.write(revision) };
        Ok(())
    })
}

fn create_document_from_source(
    bytes: Vec<u8>,
    options: EvimDocumentOptions,
) -> Result<Document, EvimStatus> {
    let encoding = parse_encoding(options.encoding)?;
    let format = parse_format(options.format)?;
    let file_format = parse_file_format(options.file_format)?;
    match file_format {
        Some(file_format) => {
            Document::from_bytes_with_file_format(bytes, encoding, format, file_format)
        }
        None => Document::from_bytes(bytes, encoding, format),
    }
    .map_err(document_status)
}

fn parse_execution_context(raw: u32) -> Result<LayoutExecutionContext, EvimStatus> {
    match raw {
        EVIM_LAYOUT_EXECUTION_WORKER_POOL => Ok(LayoutExecutionContext::WorkerPool),
        EVIM_LAYOUT_EXECUTION_DEDICATED_SERIAL => {
            Ok(LayoutExecutionContext::DedicatedSerialExecutor)
        }
        EVIM_LAYOUT_EXECUTION_FRONTEND_MAIN => Ok(LayoutExecutionContext::FrontendMainThread),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

fn execution_context_permits(
    execution_context: LayoutExecutionContext,
    threading: ProviderThreading,
) -> bool {
    match threading {
        ProviderThreading::AnyWorker => true,
        ProviderThreading::DedicatedSerialExecutor => {
            execution_context == LayoutExecutionContext::DedicatedSerialExecutor
        }
        ProviderThreading::FrontendMainThread => {
            execution_context == LayoutExecutionContext::FrontendMainThread
        }
    }
}

fn parse_key(input: EvimKeyInputV1) -> Result<Key, EvimStatus> {
    if input.struct_size < EVIM_KEY_INPUT_V1_SIZE || input.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let scalar = || char::from_u32(input.codepoint).ok_or(EvimStatus::InvalidKey);
    let special = |key| {
        if input.codepoint == 0 {
            Ok(key)
        } else {
            Err(EvimStatus::InvalidKey)
        }
    };
    match input.kind {
        EVIM_KEY_CHARACTER => Ok(Key::Char(scalar()?)),
        EVIM_KEY_ESCAPE => special(Key::Escape),
        EVIM_KEY_ENTER => special(Key::Enter),
        EVIM_KEY_TAB => special(Key::Tab),
        EVIM_KEY_BACKSPACE => special(Key::Backspace),
        EVIM_KEY_DELETE => special(Key::Delete),
        EVIM_KEY_LEFT => special(Key::Left),
        EVIM_KEY_RIGHT => special(Key::Right),
        EVIM_KEY_UP => special(Key::Up),
        EVIM_KEY_DOWN => special(Key::Down),
        EVIM_KEY_HOME => special(Key::Home),
        EVIM_KEY_END => special(Key::End),
        EVIM_KEY_PAGE_UP => special(Key::PageUp),
        EVIM_KEY_PAGE_DOWN => special(Key::PageDown),
        EVIM_KEY_CONTROL_CHARACTER => Ok(Key::Ctrl(scalar()?)),
        _ => Err(EvimStatus::InvalidKey),
    }
}

fn mode_to_ffi(mode: Mode) -> u32 {
    match mode {
        Mode::Normal => EVIM_MODE_NORMAL,
        Mode::Insert => EVIM_MODE_INSERT,
        Mode::Replace => EVIM_MODE_REPLACE,
        Mode::VisualCharacter => EVIM_MODE_VISUAL_CHARACTER,
        Mode::VisualLine => EVIM_MODE_VISUAL_LINE,
        Mode::VisualBlock => EVIM_MODE_VISUAL_BLOCK,
        Mode::CommandLine => EVIM_MODE_COMMAND_LINE,
    }
}

fn command_status_to_ffi(status: &CommandStatus) -> u32 {
    match status {
        CommandStatus::Complete => EVIM_COMMAND_STATUS_COMPLETE,
        CommandStatus::Pending => EVIM_COMMAND_STATUS_PENDING,
        CommandStatus::Cancelled => EVIM_COMMAND_STATUS_CANCELLED,
        CommandStatus::NeedsMoreLayout(_) => EVIM_COMMAND_STATUS_NEEDS_MORE_LAYOUT,
        CommandStatus::SearchNotFound => EVIM_COMMAND_STATUS_SEARCH_NOT_FOUND,
        CommandStatus::Unsupported(_) => EVIM_COMMAND_STATUS_UNSUPPORTED,
        CommandStatus::Error(_)
        | CommandStatus::CountError(_)
        | CommandStatus::RegisterReadError(_)
        | CommandStatus::RegisterWriteError(_)
        | CommandStatus::ExError(_)
        | CommandStatus::VisualBlockError(_) => EVIM_COMMAND_STATUS_ERROR,
    }
}

fn layout_status(error: LayoutError) -> EvimStatus {
    match error {
        LayoutError::Measurement(MeasurementError::UnstableShapingContext(_)) => {
            EvimStatus::UnstableShapingContext
        }
        LayoutError::Measurement(_)
        | LayoutError::StaleMeasurementResponse
        | LayoutError::MeasurementEnvironmentChangedDuringShape
        | LayoutError::MetricsChangedDuringShape
        | LayoutError::MalformedMeasurement(_) => EvimStatus::ProviderFailure,
        LayoutError::InvalidScale
        | LayoutError::InvalidStyle
        | LayoutError::InvalidStyleRun { .. }
        | LayoutError::InvalidGeometry => EvimStatus::InvalidArgument,
        _ => EvimStatus::CoreFailure,
    }
}

fn core_status(error: CoreError) -> EvimStatus {
    match error {
        CoreError::UnknownView(_) => EvimStatus::InvalidView,
        CoreError::IdentifierExhausted(_) => EvimStatus::ResourceExhausted,
        CoreError::VerticalViewportOriginUnsupported => {
            EvimStatus::VerticalViewportOriginUnsupported
        }
        CoreError::Document(error) => document_status(error),
        CoreError::Composition(error) => composition_status(error),
        CoreError::Layout(error) => layout_status(error),
        _ => EvimStatus::CoreFailure,
    }
}

fn composition_status(error: CompositionError) -> EvimStatus {
    match error {
        CompositionError::AlreadyActive
        | CompositionError::NoActiveSession
        | CompositionError::WrongDocument { .. } => EvimStatus::InvalidArgument,
        CompositionError::StaleRevision { .. } => EvimStatus::StaleRevision,
        CompositionError::InvalidRange { .. } => EvimStatus::InvalidRange,
        CompositionError::InvalidGraphemeBoundary { .. } => EvimStatus::NotGraphemeBoundary,
        CompositionError::Document(error) => document_status(error),
        CompositionError::UnresolvableCommitCaret
        | CompositionError::Position(_)
        | CompositionError::Transaction(_) => EvimStatus::CoreFailure,
    }
}

fn summarize_core_outcome(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
    outcome: Option<&CoreOutcome>,
) -> Result<EvimCoreOutcomeV1, EvimStatus> {
    let command_state = core.command_state(view_id).ok_or(EvimStatus::InvalidView)?;
    let layout = core.layout(view_id).ok_or(EvimStatus::InvalidView)?;
    let mut flags = 0;
    let command_status = if let Some(command) = outcome.and_then(|value| value.command.as_ref()) {
        flags |= EVIM_OUTCOME_HAS_COMMAND;
        if command.cursor_moved {
            flags |= EVIM_OUTCOME_CURSOR_MOVED;
        }
        if command.mode_changed {
            flags |= EVIM_OUTCOME_MODE_CHANGED;
        }
        if command.ex_outcome.is_some() || !command.clipboard_writes.is_empty() {
            flags |= EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS;
        }
        command_status_to_ffi(&command.status)
    } else {
        EVIM_COMMAND_STATUS_NONE
    };
    if outcome.is_some_and(|value| value.document_changed) {
        flags |= EVIM_OUTCOME_DOCUMENT_CHANGED;
    }
    if outcome.is_some_and(|value| value.layout_changed) {
        flags |= EVIM_OUTCOME_LAYOUT_CHANGED;
    }
    if outcome.is_some_and(|value| value.position_map.is_some()) {
        flags |= EVIM_OUTCOME_HAS_POSITION_MAP;
    }
    if outcome.is_some_and(|value| !value.composition_changes.is_empty()) {
        flags |= EVIM_OUTCOME_HAS_COMPOSITION_CHANGES;
    }
    let requirements = core
        .layout_provider_requirements(view_id)
        .map_err(core_status)?;
    let current_snapshot = layout.snapshot().filter(|snapshot| {
        snapshot.document_revision == core.document().revision()
            && snapshot.configuration_generation == layout.configuration_generation()
            && snapshot.measurement_environment_id == requirements.measurement_environment_id
            && snapshot.metrics_generation == requirements.metrics_generation
    });
    let (layout_revision, measurement_environment_id, metrics_generation) =
        if let Some(snapshot) = current_snapshot {
            flags |= EVIM_OUTCOME_HAS_LAYOUT;
            (
                snapshot.revision.0,
                snapshot.measurement_environment_id.0,
                snapshot.metrics_generation.0,
            )
        } else {
            (
                0,
                requirements.measurement_environment_id.0,
                requirements.metrics_generation.0,
            )
        };
    Ok(EvimCoreOutcomeV1 {
        struct_size: EVIM_CORE_OUTCOME_V1_SIZE,
        command_status,
        mode: mode_to_ffi(command_state.mode()),
        flags,
        document_revision: core.document().revision().0,
        view_id: view_id.0,
        cursor_utf8_offset: command_state.cursor() as u64,
        layout_revision,
        configuration_generation: layout.configuration_generation().0,
        measurement_environment_id,
        metrics_generation,
    })
}

fn summarize_viewport_state(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<EvimViewportStateV1, EvimStatus> {
    let state = core.viewport_state(view_id).map_err(core_status)?;
    let mut flags = 0;
    if state.wrap() {
        flags |= EVIM_VIEWPORT_STATE_WRAP;
    }
    let maximum_left = if let Some(maximum_left) = state.maximum_left() {
        flags |= EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT;
        maximum_left
    } else {
        0.0
    };
    if state.top_is_exact() {
        flags |= EVIM_VIEWPORT_STATE_TOP_EXACT;
    }
    let layout_revision = if let Some(layout_revision) = state.layout_revision() {
        flags |= EVIM_VIEWPORT_STATE_HAS_LAYOUT;
        layout_revision.0
    } else {
        0
    };
    Ok(EvimViewportStateV1 {
        struct_size: EVIM_VIEWPORT_STATE_V1_SIZE,
        flags,
        left: state.left(),
        top: state.top(),
        maximum_left,
        reserved: 0.0,
        document_id: state.document_id().0,
        document_revision: state.document_revision().0,
        layout_revision,
        configuration_generation: state.configuration_generation().0,
        measurement_environment_id: state.measurement_environment_id().0,
        metrics_generation: state.metrics_generation().0,
    })
}

unsafe fn clear_outcome(output: *mut EvimCoreOutcomeV1) -> Result<(), EvimStatus> {
    if output.is_null() {
        return Err(EvimStatus::NullPointer);
    }
    if (output as usize) % align_of::<EvimCoreOutcomeV1>() != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    // SAFETY: Null and alignment were checked; the public function contract
    // requires one writable output value.
    unsafe { output.write(EvimCoreOutcomeV1::default()) };
    Ok(())
}

/// Copy one fixed-layout request only after validating it cannot be corrupted
/// when the outcome is cleared.
///
/// # Safety
///
/// `request` and `out_outcome` must satisfy the public function's readable and
/// writable pointer contracts respectively.
unsafe fn read_core_request<T: Copy>(
    request: *const T,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> Result<T, EvimStatus> {
    if request.is_null() || out_outcome.is_null() {
        return Err(EvimStatus::NullPointer);
    }
    if (request as usize) % align_of::<T>() != 0
        || (out_outcome as usize) % align_of::<EvimCoreOutcomeV1>() != 0
        || pointer_ranges_overlap(
            request.cast(),
            size_of::<T>(),
            out_outcome.cast(),
            size_of::<EvimCoreOutcomeV1>(),
        )
    {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(unsafe { request.read() })
}

/// Copy a nested length-delimited UTF-8 request field before any caller output
/// is changed.
///
/// # Safety
///
/// Nonempty `value` must identify immutable readable bytes. `out_outcome` must
/// satisfy the public function's writable pointer contract.
unsafe fn composition_utf8(
    value: EvimUtf8Slice,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> Result<String, EvimStatus> {
    let length = checked_length(value.length)?;
    if value.data.is_null() && length != 0 {
        return Err(EvimStatus::NullPointer);
    }
    if pointer_ranges_overlap(
        value.data,
        length,
        out_outcome.cast(),
        size_of::<EvimCoreOutcomeV1>(),
    ) {
        return Err(EvimStatus::InvalidArgument);
    }
    let bytes = unsafe { input_bytes(value.data, value.length)? };
    str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| EvimStatus::InvalidUtf8)
}

fn validate_composition_turn(
    core: &Core<CTextMeasurementProvider>,
    view: EvimViewId,
    expected_revision: u64,
) -> Result<ViewId, EvimStatus> {
    let view = ViewId(view);
    if core.command_state(view).is_none() {
        return Err(EvimStatus::InvalidView);
    }
    validate_revision(core.document(), expected_revision)?;
    Ok(view)
}

fn dispatch_event(
    core: &mut Core<CTextMeasurementProvider>,
    view: EvimViewId,
    event: CoreEvent,
) -> Result<EvimCoreOutcomeV1, EvimStatus> {
    let view = ViewId(view);
    let outcome = core.handle(view, event).map_err(core_status)?;
    summarize_core_outcome(core, view, Some(&outcome))
}

/// Create a serial controller/core from exact source bytes and pipeline
/// options. The returned opaque token is process-local and never reused.
///
/// # Safety
///
/// Inputs must identify their declared readable ranges; options and the two
/// distinct outputs must be aligned and remain valid for this call.
#[no_mangle]
pub unsafe extern "C" fn evim_core_create(
    source: *const u8,
    source_length: u64,
    options: *const EvimDocumentOptions,
    out_core: *mut EvimCoreHandle,
    out_revision: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_core.is_null() || out_revision.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if options.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        let source_length = checked_length(source_length)?;
        if source.is_null() && source_length != 0 {
            return Err(EvimStatus::NullPointer);
        }
        if (out_core as usize) % align_of::<EvimCoreHandle>() != 0
            || (out_revision as usize) % align_of::<u64>() != 0
            || (options as usize) % align_of::<EvimDocumentOptions>() != 0
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<EvimCoreHandle>(),
                out_revision.cast(),
                size_of::<u64>(),
            )
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<EvimCoreHandle>(),
                options.cast(),
                size_of::<EvimDocumentOptions>(),
            )
            || pointer_ranges_overlap(
                out_revision.cast(),
                size_of::<u64>(),
                options.cast(),
                size_of::<EvimDocumentOptions>(),
            )
            || pointer_ranges_overlap(
                out_core.cast(),
                size_of::<EvimCoreHandle>(),
                source,
                source_length,
            )
            || pointer_ranges_overlap(out_revision.cast(), size_of::<u64>(), source, source_length)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        // SAFETY: Both outputs were validated and are required writable.
        unsafe {
            out_core.write(0);
            out_revision.write(0);
        }
        let options = unsafe { read_options(options)? };
        let bytes = unsafe { input_bytes(source, source_length as u64)? }.to_vec();
        let document = create_document_from_source(bytes, options)?;
        let revision = document.revision().0;
        let handle = register_core(Core::new(document))?;
        // SAFETY: The caller-owned outputs remain valid through return.
        unsafe {
            out_core.write(handle);
            out_revision.write(revision);
        }
        Ok(())
    })
}

/// Destroy one core token.
///
/// If an operation has checked out the core, this returns
/// [`EvimStatus::CoreBusy`] without removing the token. Provider contexts and
/// callbacks must remain valid, and the caller may retry after that operation
/// returns. Only a successful call ends the core and provider lifetimes.
#[no_mangle]
pub extern "C" fn evim_core_destroy(handle: EvimCoreHandle) -> EvimStatus {
    ffi_boundary(|| {
        if handle == 0 {
            return Err(EvimStatus::InvalidHandle);
        }
        let core = {
            let mut registry = core_registry()
                .lock()
                .map_err(|_| EvimStatus::InternalError)?;
            match registry.cores.get(&handle) {
                None => return Err(EvimStatus::InvalidHandle),
                Some(CoreRegistryEntry::Busy) => return Err(EvimStatus::CoreBusy),
                Some(CoreRegistryEntry::Ready(_)) => registry
                    .cores
                    .remove(&handle)
                    .ok_or(EvimStatus::InternalError)?,
            }
        };
        // Core teardown can release a large document and layout cache; keep it
        // outside the process-global registry lock just like all model work.
        drop(core);
        Ok(())
    })
}

/// Query the current formatted/source document revision owned by a core.
///
/// # Safety
///
/// `out_revision` must identify one aligned writable `u64`.
#[no_mangle]
pub unsafe extern "C" fn evim_core_revision(
    handle: EvimCoreHandle,
    out_revision: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_revision.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if (out_revision as usize) % align_of::<u64>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_revision.write(0) };
        let revision = with_core(handle, |core| Ok(core.document().revision().0))?;
        unsafe { out_revision.write(revision) };
        Ok(())
    })
}

/// Attach a view and copy a versioned frontend measurement provider table.
///
/// # Safety
///
/// Options and provider must identify readable aligned v1 prefixes. Outputs
/// must be distinct aligned writable values. Provider context/callbacks must
/// remain valid until the returned view is removed.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_add(
    handle: EvimCoreHandle,
    options: *const EvimViewOptionsV1,
    provider: *const EvimTextMeasurementProviderV1,
    out_view: *mut EvimViewId,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_view.is_null() || out_outcome.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if (out_view as usize) % align_of::<EvimViewId>() != 0
            || (out_outcome as usize) % align_of::<EvimCoreOutcomeV1>() != 0
            || pointer_ranges_overlap(
                out_view.cast(),
                size_of::<EvimViewId>(),
                out_outcome.cast(),
                size_of::<EvimCoreOutcomeV1>(),
            )
            || (!options.is_null()
                && ((options as usize) % align_of::<EvimViewOptionsV1>() != 0
                    || pointer_ranges_overlap(
                        out_view.cast(),
                        size_of::<EvimViewId>(),
                        options.cast(),
                        size_of::<EvimViewOptionsV1>(),
                    )
                    || pointer_ranges_overlap(
                        out_outcome.cast(),
                        size_of::<EvimCoreOutcomeV1>(),
                        options.cast(),
                        size_of::<EvimViewOptionsV1>(),
                    )))
            || (!provider.is_null()
                && ((provider as usize) % align_of::<EvimTextMeasurementProviderV1>() != 0
                    || pointer_ranges_overlap(
                        out_view.cast(),
                        size_of::<EvimViewId>(),
                        provider.cast(),
                        size_of::<EvimTextMeasurementProviderV1>(),
                    )
                    || pointer_ranges_overlap(
                        out_outcome.cast(),
                        size_of::<EvimCoreOutcomeV1>(),
                        provider.cast(),
                        size_of::<EvimTextMeasurementProviderV1>(),
                    )))
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe {
            out_view.write(0);
            clear_outcome(out_outcome)?;
        }
        if options.is_null() || provider.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        let options = unsafe { options.read() };
        if options.struct_size < EVIM_VIEW_OPTIONS_V1_SIZE
            || !options.width.is_finite()
            || options.width < 0.0
            || !options.height.is_finite()
            || options.height < 0.0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let execution_context = parse_execution_context(options.execution_context)?;
        let provider = unsafe { CTextMeasurementProvider::from_ffi(provider)? };
        if provider.render_run_policy.is_none()
            || !execution_context_permits(execution_context, provider.threading)
        {
            return Err(EvimStatus::InvalidProvider);
        }
        let (view, outcome) = with_core_mut(handle, move |core| {
            let view = core
                .try_add_view_with_layout_execution_context(
                    provider,
                    options.width,
                    options.height,
                    execution_context,
                )
                .map_err(core_status)?;
            if let Some(error) = core
                .layout(view)
                .and_then(|layout| layout.last_error())
                .cloned()
            {
                core.remove_view(view).map_err(core_status)?;
                return Err(layout_status(error));
            }
            let outcome = summarize_core_outcome(core, view, None)?;
            Ok((view.0, outcome))
        })?;
        unsafe {
            out_view.write(view);
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// Detach a view. Its opaque ID remains permanently retired.
#[no_mangle]
pub extern "C" fn evim_core_view_remove(handle: EvimCoreHandle, view: EvimViewId) -> EvimStatus {
    ffi_boundary(|| {
        if view == 0 {
            return Err(EvimStatus::InvalidView);
        }
        with_core_mut(handle, |core| {
            core.remove_view(ViewId(view)).map_err(core_status)?;
            Ok(())
        })
    })
}

/// Read the current fixed-size state summary for a view.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_state(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core(handle, |core| {
            summarize_core_outcome(core, ViewId(view), None)
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Read the per-view presentation origin without requesting layout.
///
/// # Safety
///
/// `out_state` must identify one aligned writable state value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_viewport_state(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_state: *mut EvimViewportStateV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_state.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if (out_state as usize) % align_of::<EvimViewportStateV1>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_state.write(EvimViewportStateV1::default()) };
        let state = with_core(handle, |core| summarize_viewport_state(core, ViewId(view)))?;
        unsafe { out_state.write(state) };
        Ok(())
    })
}

/// Deliver one normalized key to a view's command interpreter.
///
/// # Safety
///
/// `input` must identify an aligned readable key prefix and `out_outcome` one
/// aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_key(
    handle: EvimCoreHandle,
    view: EvimViewId,
    input: *const EvimKeyInputV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if input.is_null() || out_outcome.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        if (input as usize) % align_of::<EvimKeyInputV1>() != 0
            || (out_outcome as usize) % align_of::<EvimCoreOutcomeV1>() != 0
            || pointer_ranges_overlap(
                input.cast(),
                size_of::<EvimKeyInputV1>(),
                out_outcome.cast(),
                size_of::<EvimCoreOutcomeV1>(),
            )
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let key = parse_key(unsafe { input.read() })?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::Input(InputEvent::Key(key)))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Deliver one length-delimited valid UTF-8 text input event.
///
/// # Safety
///
/// Nonempty text must identify its declared readable bytes; `out_outcome`
/// must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_text(
    handle: EvimCoreHandle,
    view: EvimViewId,
    text: *const u8,
    text_length: u64,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_outcome.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        let text_length = checked_length(text_length)?;
        if text.is_null() && text_length != 0 {
            return Err(EvimStatus::NullPointer);
        }
        if (out_outcome as usize) % align_of::<EvimCoreOutcomeV1>() != 0
            || pointer_ranges_overlap(
                text,
                text_length,
                out_outcome.cast(),
                size_of::<EvimCoreOutcomeV1>(),
            )
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let text = unsafe { input_bytes(text, text_length as u64)? };
        let text = str::from_utf8(text)
            .map_err(|_| EvimStatus::InvalidUtf8)?
            .to_owned();
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::Input(InputEvent::Text(text)))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Begin a native marked-text session over one exact formatted-snapshot range.
/// The range is the frontend's current selection, or empty at its caret.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request and `out_outcome`
/// one distinct aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_composition_begin(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCompositionBeginV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_COMPOSITION_BEGIN_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let start = checked_length(request.replacement_start)?;
        let end = checked_length(request.replacement_end)?;
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_composition_turn(core, view, request.document_revision)?;
            let target = CompositionTarget::at_offsets(core.document(), start..end)
                .map_err(composition_status)?;
            dispatch_event(
                core,
                view,
                CoreEvent::Composition(CompositionEvent::Begin(target)),
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Replace the temporary native marked text and its relative selection.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request. Its nonempty text
/// slice must remain readable for the call. `out_outcome` must identify a
/// distinct aligned writable outcome and must not overlap the text bytes.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_composition_update(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCompositionUpdateV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_COMPOSITION_UPDATE_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let marked_text = unsafe { composition_utf8(request.marked_text, out_outcome)? };
        let selected_start = checked_length(request.selected_start)?;
        let selected_end = checked_length(request.selected_end)?;
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_composition_turn(core, view, request.document_revision)?;
            dispatch_event(
                core,
                view,
                CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                    marked_text,
                    selected_start..selected_end,
                ))),
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Commit a final native composition payload as one document transaction and
/// one undo unit. A model-policy failure leaves the authoritative source
/// unchanged and keeps the final text as the active marked overlay so the
/// frontend can cancel it or request another policy.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request. Its nonempty text
/// slice must remain readable for the call. `out_outcome` must identify a
/// distinct aligned writable outcome and must not overlap the text bytes.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_composition_commit(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCompositionCommitV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_COMPOSITION_COMMIT_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let committed_text = unsafe { composition_utf8(request.committed_text, out_outcome)? };
        let caret = committed_text.len();
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let view_id = validate_composition_turn(core, view, request.document_revision)?;
            core.handle(
                view_id,
                CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                    committed_text,
                    caret..caret,
                ))),
            )
            .map_err(core_status)?;
            let committed = core
                .handle(view_id, CoreEvent::Composition(CompositionEvent::Commit))
                .map_err(core_status)?;
            summarize_core_outcome(core, view_id, Some(&committed))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Cancel native marked text and restore its exact base snapshot without a
/// source edit.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request and `out_outcome`
/// one distinct aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_composition_cancel(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCompositionCancelV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_COMPOSITION_CANCEL_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_composition_turn(core, view, request.document_revision)?;
            dispatch_event(core, view, CoreEvent::Composition(CompositionEvent::Cancel))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Set an absolute per-view presentation origin. Horizontal-only requests are
/// presentation state and never run layout. A request carrying `HAS_TOP`
/// currently returns `VerticalViewportOriginUnsupported` atomically.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request and `out_outcome`
/// one distinct aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_viewport_origin(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimViewportOriginV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_VIEWPORT_ORIGIN_V1_SIZE
            || request.flags & !EVIM_VIEWPORT_ORIGIN_HAS_TOP != 0
            || !request.left.is_finite()
            || (request.flags & EVIM_VIEWPORT_ORIGIN_HAS_TOP != 0 && !request.top.is_finite())
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let top = (request.flags & EVIM_VIEWPORT_ORIGIN_HAS_TOP != 0).then_some(request.top);
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SetViewportOrigin {
                    left: request.left,
                    top,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Resize a view and synchronously refresh its current materialized layout.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_resize(
    handle: EvimCoreHandle,
    view: EvimViewId,
    width: f32,
    height: f32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !width.is_finite() || width < 0.0 || !height.is_finite() || height < 0.0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::Resize { width, height })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Change a view's wrapping option and return its new revision-tagged state.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_wrap(
    handle: EvimCoreHandle,
    view: EvimViewId,
    wrap: u32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let wrap = parse_ffi_bool(wrap)?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetWrap(wrap))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

fn core_snapshot_bytes(
    handle: EvimCoreHandle,
    expected_revision: u64,
    source: bool,
) -> Result<Vec<u8>, EvimStatus> {
    with_core(handle, |core| {
        validate_revision(core.document(), expected_revision)?;
        Ok(if source {
            core.document().source_bytes()
        } else {
            core.document().text().as_bytes().to_vec()
        })
    })
}

unsafe fn copy_core_snapshot_bytes(
    handle: EvimCoreHandle,
    expected_revision: u64,
    source: bool,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_required.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        let capacity = checked_length(output_capacity)?;
        if output.is_null() && capacity != 0 {
            return Err(EvimStatus::NullPointer);
        }
        if (out_required as usize) % align_of::<u64>() != 0
            || pointer_ranges_overlap(output, capacity, out_required.cast(), size_of::<u64>())
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let bytes = core_snapshot_bytes(handle, expected_revision, source)?;
        let required = u64::try_from(bytes.len()).map_err(|_| EvimStatus::LengthOverflow)?;
        unsafe { out_required.write(required) };
        if capacity < bytes.len() {
            return Err(EvimStatus::BufferTooSmall);
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if output.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        Ok(())
    })
}

#[no_mangle]
/// Copy exact source bytes from one immutable core-owned revision.
///
/// # Safety
///
/// `out_required` must be aligned and writable. A nonzero output capacity
/// requires a writable output region of that size.
pub unsafe extern "C" fn evim_core_copy_source_bytes(
    handle: EvimCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    unsafe {
        copy_core_snapshot_bytes(
            handle,
            expected_revision,
            true,
            output,
            output_capacity,
            out_required,
        )
    }
}

#[no_mangle]
/// Copy formatted valid UTF-8 from one immutable core-owned revision.
///
/// # Safety
///
/// `out_required` must be aligned and writable. A nonzero output capacity
/// requires a writable output region of that size.
pub unsafe extern "C" fn evim_core_copy_formatted_utf8(
    handle: EvimCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    unsafe {
        copy_core_snapshot_bytes(
            handle,
            expected_revision,
            false,
            output,
            output_capacity,
            out_required,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        checkout_core, checkout_document, evim_core_destroy, evim_document_destroy, ffi_boundary,
        register_core, register_document, EvimStatus, MarshalledRequest, EVIM_TEXT_DIRECTION_AUTO,
        EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT, EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1,
        EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2,
    };
    use crate::document::Document;
    use crate::layout::{
        MeasurementEnvironmentId, MetricsGeneration, ResolvedTextStyle, ShapePurpose, ShapeRequest,
        TextDirection,
    };
    use crate::Core;

    #[test]
    fn ffi_boundary_contains_rust_panics() {
        let status = ffi_boundary(|| -> Result<(), EvimStatus> {
            panic!("deliberate ABI-boundary test panic")
        });
        assert_eq!(status, EvimStatus::Panic);
    }

    #[test]
    fn document_turns_never_hold_the_registry_lock_during_model_work() {
        let handle = register_document(Document::new("text")).unwrap();
        let lease = checkout_document(handle).unwrap();
        assert!(matches!(
            checkout_document(handle),
            Err(EvimStatus::DocumentBusy)
        ));
        drop(lease);
        drop(checkout_document(handle).unwrap());
        assert_eq!(evim_document_destroy(handle), EvimStatus::Ok);
    }

    #[test]
    fn panic_returns_checked_out_handles_and_busy_destroy_is_retryable() {
        let handle = register_document(Document::new("text")).unwrap();
        let status = ffi_boundary(|| -> Result<(), EvimStatus> {
            let _lease = checkout_document(handle)?;
            assert_eq!(
                evim_document_destroy(handle),
                EvimStatus::DocumentBusy,
                "destroy must retain a checked-out document"
            );
            panic!("panic while a document turn is checked out")
        });
        assert_eq!(status, EvimStatus::Panic);
        drop(checkout_document(handle).unwrap());
        assert_eq!(evim_document_destroy(handle), EvimStatus::Ok);
        assert!(matches!(
            checkout_document(handle),
            Err(EvimStatus::InvalidHandle)
        ));

        let core_handle = register_core(Core::new(Document::new("text"))).unwrap();
        let status = ffi_boundary(|| -> Result<(), EvimStatus> {
            let _lease = checkout_core(core_handle)?;
            assert_eq!(
                evim_core_destroy(core_handle),
                EvimStatus::CoreBusy,
                "destroy must retain a checked-out core"
            );
            panic!("panic while a core turn is checked out")
        });
        assert_eq!(status, EvimStatus::Panic);
        drop(checkout_core(core_handle).unwrap());
        assert_eq!(evim_core_destroy(core_handle), EvimStatus::Ok);
        assert!(matches!(
            checkout_core(core_handle),
            Err(EvimStatus::InvalidHandle)
        ));
    }

    #[test]
    fn provider_request_versions_preserve_v1_and_supply_v2_paragraph_direction() {
        let style = ResolvedTextStyle::default();
        let request = ShapeRequest {
            document_id: crate::document::DocumentId(1),
            document_revision: crate::document::Revision(2),
            measurement_environment_id: MeasurementEnvironmentId(3),
            text_range: 0..1,
            text: "x",
            context_before: "",
            context_after: "",
            style_runs: &[],
            default_style: &style,
            paragraph_base_direction: TextDirection::RightToLeft,
            scale: 1.0,
            metrics_generation: MetricsGeneration(4),
            purpose: ShapePurpose::MetricsOnly,
            render_run_policy: None,
        };
        let storage = MarshalledRequest::new(&request);

        assert_eq!(
            storage
                .request(&request, EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1)
                .paragraph_base_direction,
            EVIM_TEXT_DIRECTION_AUTO
        );
        assert_eq!(
            storage
                .request(&request, EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2)
                .paragraph_base_direction,
            EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT
        );
    }
}
