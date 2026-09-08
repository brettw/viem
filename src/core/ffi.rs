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

use crate::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget, ClipboardWriteRequest,
};
use crate::command::composition::{
    CompositionError, CompositionEvent, CompositionTarget, CompositionUpdate,
};
use crate::command::ex_execute::{
    ExFileRequest, ExFrontendRequest, ExInfoRequest, ExNavigation, ExOptionDisplay, ExOptionName,
    ExOptionValue, ExOutcome, HardLineRange,
};
use crate::command::visual_block::{
    resolve_block_selection, resolve_block_selection_to_line_end, VisualBlockError,
};
use crate::command::{
    CommandLineKind, CommandOutput, CommandStatus, InputEvent, Key, Mode, RegisterKind,
};
use crate::document::{
    BlockProperties, BlockRole, BoundaryAffinity, CharacterProperties, Color, Document,
    DocumentError, DocumentId, DocumentStyleAssignment, Encoding, FileFormat, FileFormatOrigin,
    FontSlant, Format, FormattedTextError, HardLineQueryError, HistorySemanticChangeKind,
    HistorySemanticSummary, LineSpacing, ModelTransactionError, ParagraphAlignment, Revision,
    SemanticInlineStyle, SourceArtifactDigest, StyleContribution, StyleContributionOrigin,
    StyleDefinitionFieldEdit, StyleDefinitionOrigin, StyleDependency, StyleError, StyleId,
    StyleNamespace, StyleProperty, StylePropertyValue, StyleSheetRevision, StyleTransactionError,
    TextRange, WritingDirection,
};
use crate::layout::{
    CaretPoint, ClusterCaretStop, LayoutError, LayoutExecutionContext, LayoutJobError, LayoutPoint,
    LayoutRect, LayoutSnapshot, MeasurementEnvironmentId, MeasurementError, MetricsGeneration,
    ProviderThreading, RenderRunHandle, RenderRunOwner, RenderRunPolicy, RenderRunThreading,
    ResolvedTextPaint, ResolvedTextStyle, ShapePurpose, ShapeRequest, ShapedBounds, ShapedCluster,
    ShapedFragment, ShapingDiagnostic, TextDirection, TextMeasurementProvider, TextMetrics,
};
use crate::{
    Core, CoreError, CoreEvent, CoreOutcome, LogicalSelectionIdentity, LogicalSelectionKind,
    SemanticStylePresentation, SemanticStyleState, StyleEditGroup, StyleEditGroupError,
    StyleEditGroupId, ViewId,
};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{align_of, size_of};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::str;
use std::sync::{Arc, Mutex, OnceLock};

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

/// Opaque immutable command-turn effect batch. Zero means that a successful
/// turn emitted no host effects and is never a valid owned handle.
pub type EvimEffectBatchHandle = u64;

/// Opaque view identity scoped to one core. Zero is always invalid, and a
/// removed value is never assigned to another view in that core.
pub type EvimViewId = u64;

/// Select encoding in core using supported BOMs, otherwise strict UTF-8, then
/// ISO-8859-1 fallback. Existing nonzero values remain explicit/forced.
pub const EVIM_ENCODING_DETECT: u32 = 0;
pub const EVIM_ENCODING_UTF8: u32 = 1;
pub const EVIM_ENCODING_LATIN1: u32 = 2;
pub const EVIM_ENCODING_UTF16_LE: u32 = 3;
pub const EVIM_ENCODING_UTF16_BE: u32 = 4;

pub const EVIM_FORMAT_PLAIN_TEXT: u32 = 1;
pub const EVIM_FORMAT_MARKDOWN: u32 = 2;
pub const EVIM_FORMAT_HTML: u32 = 3;
pub const EVIM_FORMAT_RTF: u32 = 4;
pub const EVIM_FORMAT_MARKDOWN_SOURCE: u32 = 5;
pub const EVIM_FORMAT_HTML_SOURCE: u32 = 6;

/// Detect the line-ending interpretation through the core's shared open
/// policy.
pub const EVIM_FILE_FORMAT_DETECT: u32 = 0;
pub const EVIM_FILE_FORMAT_UNIX: u32 = 1;
pub const EVIM_FILE_FORMAT_DOS: u32 = 2;
pub const EVIM_FILE_FORMAT_MAC: u32 = 3;

pub const EVIM_FILE_FORMAT_ORIGIN_DETECTED: u32 = 1;
pub const EVIM_FILE_FORMAT_ORIGIN_FORCED: u32 = 2;
pub const EVIM_FILE_FORMAT_ORIGIN_DEFAULTED: u32 = 3;

pub const EVIM_HISTORY_ACTION_CATEGORY_NONE: u32 = 0;
pub const EVIM_HISTORY_ACTION_CATEGORY_TEXT: u32 = 1;
pub const EVIM_HISTORY_ACTION_CATEGORY_STYLE: u32 = 2;
pub const EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT: u32 = 3;
pub const EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER: u32 = 4;
pub const EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_SOURCE_RESTORATION: u32 = 5;
pub const EVIM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA: u32 = 6;
pub const EVIM_HISTORY_ACTION_CATEGORY_MIXED: u32 = 7;

pub const EVIM_DOCUMENT_STATE_HAS_BOM: u32 = 1 << 0;
pub const EVIM_DOCUMENT_STATE_CAN_UNDO: u32 = 1 << 1;
pub const EVIM_DOCUMENT_STATE_CAN_REDO: u32 = 1 << 2;
pub const EVIM_DOCUMENT_STATE_IS_DIRTY: u32 = 1 << 3;
pub const EVIM_DOCUMENT_STATE_READ_ONLY: u32 = 1 << 4;
pub const EVIM_DOCUMENT_STATE_RECOVERED: u32 = 1 << 5;
pub const EVIM_DOCUMENT_STATE_INCLUDE_STYLE_DEFINITIONS: u32 = 1 << 6;

/// Immutable model/history metadata captured in one serial core query.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimDocumentStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
    pub encoding: u32,
    pub format: u32,
    pub file_format: u32,
    pub file_format_origin: u32,
    pub undo_action_category: u32,
    pub redo_action_category: u32,
    pub reserved: [u32; 2],
}

pub const EVIM_DOCUMENT_STATE_V1_SIZE: u32 = size_of::<EvimDocumentStateV1>() as u32;

impl Default for EvimDocumentStateV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_DOCUMENT_STATE_V1_SIZE,
            flags: 0,
            document_id: 0,
            document_revision: 0,
            style_sheet_revision: 0,
            encoding: 0,
            format: 0,
            file_format: 0,
            file_format_origin: 0,
            undo_action_category: EVIM_HISTORY_ACTION_CATEGORY_NONE,
            redo_action_category: EVIM_HISTORY_ACTION_CATEGORY_NONE,
            reserved: [0; 2],
        }
    }
}

/// Exact identity of one immutable formatted projection.
///
/// A caller copies this value from [`EvimFormattedSnapshotInfoV1`] into every
/// dependent range, mapping, and point request. Core rejects a request after
/// the document advances rather than applying its numeric positions to a new
/// projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimFormattedSnapshotIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE: u32 =
    size_of::<EvimFormattedSnapshotIdentityV1>() as u32;

/// Constant-time aggregate metadata for the current formatted projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimFormattedSnapshotInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimFormattedSnapshotIdentityV1,
    pub utf8_length: u64,
    pub utf16_length: u64,
    pub hard_line_count: u64,
}

pub const EVIM_FORMATTED_SNAPSHOT_INFO_V1_SIZE: u32 =
    size_of::<EvimFormattedSnapshotInfoV1>() as u32;

/// One scalar-aligned, half-open UTF-8 range in an exact formatted snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimFormattedUtf8RangeV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimFormattedSnapshotIdentityV1,
    pub utf8_start: u64,
    pub utf8_end: u64,
}

pub const EVIM_FORMATTED_UTF8_RANGE_V1_SIZE: u32 = size_of::<EvimFormattedUtf8RangeV1>() as u32;

/// Logical metadata for one grapheme-aligned formatted point.
///
/// Hard-line indices and grapheme columns are zero based. `hard_line_start`
/// and `hard_line_end` delimit content and exclude the following semantic hard
/// break. The point's UTF-16 offset uses the same document-wide origin as its
/// UTF-8 offset.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimFormattedPointInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimFormattedSnapshotIdentityV1,
    pub utf8_offset: u64,
    pub utf16_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
}

pub const EVIM_FORMATTED_POINT_INFO_V1_SIZE: u32 = size_of::<EvimFormattedPointInfoV1>() as u32;

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
    /// Reserved legacy value from ABI v3. Current vertical viewport requests
    /// use the identity-bound regional-layout protocol and do not return it.
    VerticalViewportOriginUnsupported = 27,
    /// No immutable layout snapshot matches the view's current document,
    /// configuration, measurement environment, and metrics generations.
    LayoutUnavailable = 28,
    /// The requested point or logical endpoint is outside the materialized
    /// coverage of the current partial layout snapshot.
    OutsideLayoutCoverage = 29,
    UnknownStyle = 30,
    StyleReadOnly = 31,
    InvalidStyleValue = 32,
    StyleInheritanceCycle = 33,
    IncompatibleStyleRole = 34,
    InvalidStyleRelationship = 35,
    /// A byte offset is in range but splits one UTF-8 scalar value.
    InvalidUtf8Boundary = 36,
    /// A UTF-16 offset is in range but splits one surrogate pair.
    InvalidUtf16Boundary = 37,
    /// Another explicit frontend-owned style edit group is already active in
    /// this core. End it or let an unrelated event consume it before retrying.
    StyleEditGroupActive = 38,
    /// The style edit group token is absent, consumed, forged, or does not
    /// match the immutable capability fields returned by begin.
    InvalidStyleEditGroup = 39,
    /// A live style edit group exists but belongs to another attached view.
    StyleEditGroupWrongOwner = 40,
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
pub const EVIM_KEY_BACK_TAB: u32 = 16;
pub const EVIM_KEY_DOCUMENT_START: u32 = 17;
pub const EVIM_KEY_DOCUMENT_END: u32 = 18;

pub const EVIM_COMMAND_STATUS_NONE: u32 = 0;
pub const EVIM_COMMAND_STATUS_COMPLETE: u32 = 1;
pub const EVIM_COMMAND_STATUS_PENDING: u32 = 2;
pub const EVIM_COMMAND_STATUS_CANCELLED: u32 = 3;
pub const EVIM_COMMAND_STATUS_NEEDS_MORE_LAYOUT: u32 = 4;
pub const EVIM_COMMAND_STATUS_SEARCH_NOT_FOUND: u32 = 5;
pub const EVIM_COMMAND_STATUS_UNSUPPORTED: u32 = 6;
pub const EVIM_COMMAND_STATUS_ERROR: u32 = 7;
pub const EVIM_COMMAND_STATUS_READ_ONLY: u32 = 8;

pub const EVIM_MODE_NORMAL: u32 = 1;
pub const EVIM_MODE_INSERT: u32 = 2;
pub const EVIM_MODE_REPLACE: u32 = 3;
pub const EVIM_MODE_VISUAL_CHARACTER: u32 = 4;
pub const EVIM_MODE_VISUAL_LINE: u32 = 5;
pub const EVIM_MODE_VISUAL_BLOCK: u32 = 6;
pub const EVIM_MODE_COMMAND_LINE: u32 = 7;

pub const EVIM_CLIPBOARD_TARGET_CLIPBOARD: u32 = 1;
pub const EVIM_CLIPBOARD_TARGET_PRIMARY: u32 = 2;

pub const EVIM_CLIPBOARD_TURN_HAS_READ: u32 = 1 << 0;
pub const EVIM_CLIPBOARD_TURN_WRITABLE: u32 = 1 << 1;

pub const EVIM_EFFECT_BATCH_HAS_EX_OUTCOME: u32 = 1 << 0;
pub const EVIM_EFFECT_BATCH_EX_DOCUMENT_CHANGED: u32 = 1 << 1;
pub const EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION: u32 = 1 << 2;
pub const EVIM_EFFECT_BATCH_EX_NAVIGATION_HISTORY: u32 = 1 << 3;

pub const EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER: u32 = 1 << 0;
pub const EVIM_REGISTER_KIND_NONE: u32 = 0;
pub const EVIM_REGISTER_KIND_CHARACTER: u32 = 1;
pub const EVIM_REGISTER_KIND_LINE: u32 = 2;
pub const EVIM_REGISTER_KIND_BLOCK: u32 = 3;

pub const EVIM_EX_FRONTEND_EDIT: u32 = 1;
pub const EVIM_EX_FRONTEND_NEW: u32 = 2;
pub const EVIM_EX_FRONTEND_WRITE: u32 = 3;
pub const EVIM_EX_FRONTEND_SAVE_AS: u32 = 4;
pub const EVIM_EX_FRONTEND_QUIT: u32 = 5;
pub const EVIM_EX_FRONTEND_QUIT_ALL: u32 = 6;
pub const EVIM_EX_FRONTEND_WRITE_QUIT: u32 = 7;
pub const EVIM_EX_FRONTEND_XIT: u32 = 8;
pub const EVIM_EX_FRONTEND_WRITE_ALL: u32 = 9;
pub const EVIM_EX_FRONTEND_MARKS: u32 = 10;
pub const EVIM_EX_FRONTEND_REGISTERS: u32 = 11;
pub const EVIM_EX_FRONTEND_JUMPS: u32 = 12;
pub const EVIM_EX_FRONTEND_OPTIONS: u32 = 13;
pub const EVIM_EX_FRONTEND_PRINT_LINES: u32 = 14;
pub const EVIM_EX_FRONTEND_NORMAL: u32 = 15;
pub const EVIM_EX_FRONTEND_SPLIT: u32 = 16;
pub const EVIM_EX_FRONTEND_MESSAGE: u32 = 17;
pub const EVIM_EX_FRONTEND_EDIT_NEW_WINDOW: u32 = 18;
pub const EVIM_EX_FRONTEND_PWD: u32 = 19;
pub const EVIM_EX_FRONTEND_CD: u32 = 20;
pub const EVIM_EX_FRONTEND_CHECKTIME: u32 = 21;

pub const EVIM_EX_FRONTEND_FORCE: u32 = 1 << 0;
pub const EVIM_EX_FRONTEND_HAS_PATH: u32 = 1 << 1;
pub const EVIM_EX_FRONTEND_HAS_RANGE: u32 = 1 << 2;
pub const EVIM_EX_FRONTEND_NUMBER: u32 = 1 << 3;
pub const EVIM_EX_FRONTEND_LIST: u32 = 1 << 4;
pub const EVIM_EX_FRONTEND_LITERAL: u32 = 1 << 5;

pub const EVIM_EX_OPTION_WRAP: u32 = 1;
pub const EVIM_EX_OPTION_LINEBREAK: u32 = 2;
pub const EVIM_EX_OPTION_FILE_FORMAT: u32 = 3;
pub const EVIM_EX_OPTION_FILE_FORMATS: u32 = 4;
pub const EVIM_EX_OPTION_IGNORECASE: u32 = 5;
pub const EVIM_EX_OPTION_SMARTCASE: u32 = 6;
pub const EVIM_EX_OPTION_WRAPSCAN: u32 = 7;

pub const EVIM_EX_OPTION_VALUE_BOOLEAN: u32 = 1;
pub const EVIM_EX_OPTION_VALUE_FILE_FORMAT: u32 = 2;
pub const EVIM_EX_OPTION_VALUE_FILE_FORMATS: u32 = 3;

pub const EVIM_EX_JUMP_CURRENT: u32 = 1 << 0;

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

/// One clipboard target captured immediately before a command turn.
///
/// `HAS_READ` supplies a generation-tagged immutable plain-text snapshot;
/// `WRITABLE` independently authorizes core to emit a write for the target.
/// A target may occur at most once in a turn context.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimClipboardTurnEntryV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub target: u32,
    pub reserved: u32,
    pub generation: u64,
    pub plain_text: EvimUtf8Slice,
}

pub const EVIM_CLIPBOARD_TURN_ENTRY_V1_SIZE: u32 = size_of::<EvimClipboardTurnEntryV1>() as u32;

impl Default for EvimClipboardTurnEntryV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_CLIPBOARD_TURN_ENTRY_V1_SIZE,
            flags: 0,
            target: 0,
            reserved: 0,
            generation: 0,
            plain_text: EvimUtf8Slice::default(),
        }
    }
}

/// Immutable host capabilities and snapshots for exactly one input turn.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimCommandTurnContextV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub clipboards: *const EvimClipboardTurnEntryV1,
    pub clipboard_count: u64,
}

pub const EVIM_COMMAND_TURN_CONTEXT_V1_SIZE: u32 = size_of::<EvimCommandTurnContextV1>() as u32;

impl Default for EvimCommandTurnContextV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_COMMAND_TURN_CONTEXT_V1_SIZE,
            reserved: 0,
            clipboards: std::ptr::null(),
            clipboard_count: 0,
        }
    }
}

/// V2 adds an optional validated eVim fragment JSON image. V1 remains unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimClipboardTurnEntryV2 {
    pub struct_size: u32,
    pub flags: u32,
    pub target: u32,
    pub reserved: u32,
    pub generation: u64,
    pub plain_text: EvimUtf8Slice,
    pub fragment_json: EvimUtf8Slice,
}
pub const EVIM_CLIPBOARD_TURN_ENTRY_V2_SIZE: u32 = size_of::<EvimClipboardTurnEntryV2>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimCommandTurnContextV2 {
    pub struct_size: u32,
    pub reserved: u32,
    pub clipboards: *const EvimClipboardTurnEntryV2,
    pub clipboard_count: u64,
}
pub const EVIM_COMMAND_TURN_CONTEXT_V2_SIZE: u32 = size_of::<EvimCommandTurnContextV2>() as u32;

/// Offset and length inside an effect batch's copied byte arena. Unless a field
/// explicitly documents otherwise, referenced bytes are valid UTF-8.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimEffectBytesRefV1 {
    pub offset: u64,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimClipboardWriteV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub target: u32,
    pub register_kind: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub plain_text: EvimEffectBytesRefV1,
    pub first_hard_break: u64,
    pub hard_break_count: u64,
}

pub const EVIM_CLIPBOARD_WRITE_V1_SIZE: u32 = size_of::<EvimClipboardWriteV1>() as u32;

/// One displayed option value embedded by an `OPTIONS` frontend request.
/// For `FILE_FORMATS`, `first_file_format..+file_format_count` indexes the
/// copied file-format value array in exact display order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExOptionDisplayV1 {
    pub struct_size: u32,
    pub name: u32,
    pub value_kind: u32,
    pub scalar_value: u32,
    pub first_file_format: u64,
    pub file_format_count: u64,
}

pub const EVIM_EX_OPTION_DISPLAY_V1_SIZE: u32 = size_of::<EvimExOptionDisplayV1>() as u32;

/// One resolved mark captured against the effect batch's exact formatted
/// revision. `line_text` contains the complete hard-line content without its
/// semantic break.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExMarkV1 {
    pub struct_size: u32,
    pub name: u32,
    pub utf8_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
    pub line_text: EvimEffectBytesRefV1,
}

pub const EVIM_EX_MARK_V1_SIZE: u32 = size_of::<EvimExMarkV1>() as u32;

/// One resolved Vim register captured for an Ex info request. Hard-break
/// indices reference the shared hard-break array and are UTF-8 byte offsets
/// inside `text`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExRegisterV1 {
    pub struct_size: u32,
    pub name: u32,
    pub register_kind: u32,
    pub reserved: u32,
    pub text: EvimEffectBytesRefV1,
    pub first_hard_break: u64,
    pub hard_break_count: u64,
}

pub const EVIM_EX_REGISTER_V1_SIZE: u32 = size_of::<EvimExRegisterV1>() as u32;

/// One jump captured in oldest-to-newest order. Exactly one record has
/// `CURRENT` when the list is nonempty. Line text excludes the semantic break.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExJumpV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub list_index: u64,
    pub utf8_offset: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub grapheme_column: u64,
    pub line_text: EvimEffectBytesRefV1,
}

pub const EVIM_EX_JUMP_V1_SIZE: u32 = size_of::<EvimExJumpV1>() as u32;

/// One exact formatted hard line captured for PRINT_LINES. Text excludes the
/// semantic hard break; request NUMBER/LIST flags describe its presentation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExTextLineV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub hard_line_index: u64,
    pub utf8_start: u64,
    pub utf8_end: u64,
    pub text: EvimEffectBytesRefV1,
}

pub const EVIM_EX_TEXT_LINE_V1_SIZE: u32 = size_of::<EvimExTextLineV1>() as u32;

/// One raw Ex host request in command execution order.
///
/// `text` is the optional path, `:normal` command string, or concatenated
/// Unicode mark/register-name scalar sequence according to `kind`. Hard-line
/// ranges are inclusive and zero based. Option indices apply only to OPTIONS.
/// `first_payload..+payload_count` selects the corresponding mark, register,
/// jump, or text-line array for those four info-request kinds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimExFrontendRequestV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub flags: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub text: EvimEffectBytesRefV1,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub first_option: u64,
    pub option_count: u64,
    pub first_payload: u64,
    pub payload_count: u64,
}

pub const EVIM_EX_FRONTEND_REQUEST_V1_SIZE: u32 = size_of::<EvimExFrontendRequestV1>() as u32;

/// Exact sizes and command identity for an immutable owned effect batch.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimEffectBatchInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub batch_handle: EvimEffectBatchHandle,
    pub document_id: u64,
    pub document_revision: u64,
    pub clipboard_write_count: u64,
    pub ex_request_count: u64,
    pub ex_option_count: u64,
    pub ex_mark_count: u64,
    pub ex_register_count: u64,
    pub ex_jump_count: u64,
    pub ex_text_line_count: u64,
    pub file_format_count: u64,
    pub hard_break_count: u64,
    pub string_bytes: u64,
    pub navigation_utf8_offset: u64,
    pub substitution_count: u64,
}

pub const EVIM_EFFECT_BATCH_INFO_V1_SIZE: u32 = size_of::<EvimEffectBatchInfoV1>() as u32;

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
/// requested. With `EVIM_VIEWPORT_ORIGIN_HAS_TOP`, the complete expected
/// identity must match the current immutable layout before core atomically
/// installs bounded regional layout and both requested coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvimViewportOriginV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    /// Exact state identity from `EvimViewportStateV1`. These fields are
    /// ignored for horizontal-only requests.
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
pub const EVIM_VIEWPORT_STATE_LINEBREAK: u32 = 1 << 4;

/// Current presentation origin and the exact dependency identity observed in
/// the same serial query. `maximum_left` describes visible rows only; without
/// its exact flag it is a provisional lower bound, not an authoritative clamp.
/// `scale` is always the exact positive view-local magnification. A
/// missing exact top flag means the installed snapshot uses an estimated
/// prefix; the value remains the view's current coordinate but must not be
/// treated as a durable absolute document position.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvimViewportStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub left: f32,
    pub top: f32,
    pub maximum_left: f32,
    pub scale: f32,
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
            scale: 1.0,
            document_id: 0,
            document_revision: 0,
            layout_revision: 0,
            configuration_generation: 0,
            measurement_environment_id: 0,
            metrics_generation: 0,
        }
    }
}

/// Exact dependency identity for one immutable layout snapshot. Callers copy
/// this value from [`EvimLayoutSnapshotInfoV1`] into subsequent geometry
/// requests; core never silently substitutes a newer snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimLayoutSnapshotIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub layout_revision: u64,
    pub configuration_generation: u64,
    pub measurement_environment_id: u64,
    pub metrics_generation: u64,
}

pub const EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE: u32 =
    size_of::<EvimLayoutSnapshotIdentityV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutInsetsV1 {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutRectV1 {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Normalized RGBA components copied from the core style model. Every
/// component is finite and in the inclusive range zero to one.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimRgbaV1 {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

pub const EVIM_STYLE_NAMESPACE_BLOCK: u32 = 1;
pub const EVIM_STYLE_NAMESPACE_CHARACTER: u32 = 2;

pub const EVIM_STYLE_ROLE_NONE: u32 = 0;
pub const EVIM_STYLE_ROLE_DOCUMENT: u32 = 1;
pub const EVIM_STYLE_ROLE_PARAGRAPH: u32 = 2;

pub const EVIM_STYLE_ORIGIN_SOURCE_BACKED: u32 = 1;
pub const EVIM_STYLE_ORIGIN_GENERATED_CONFIGURATION: u32 = 2;
pub const EVIM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY: u32 = 3;

pub const EVIM_STYLE_DEFINITION_HAS_PARENT: u32 = 1 << 0;
pub const EVIM_STYLE_DEFINITION_HAS_NEXT_STYLE: u32 = 1 << 1;
pub const EVIM_STYLE_DEFINITION_BASE_DOCUMENT: u32 = 1 << 2;
pub const EVIM_STYLE_DEFINITION_BASE_PARAGRAPH: u32 = 1 << 3;
pub const EVIM_STYLE_DEFINITION_BASE_CHARACTER: u32 = 1 << 4;
pub const EVIM_STYLE_DEFINITION_INTERNAL: u32 = 1 << 5;
pub const EVIM_STYLE_DEFINITION_INTERNAL_LIST: u32 = 1 << 6;

pub const EVIM_STYLE_CAPABILITY_EDIT_DECLARATIONS: u32 = 1 << 0;
pub const EVIM_STYLE_CAPABILITY_EDIT_PARENT: u32 = 1 << 1;
pub const EVIM_STYLE_CAPABILITY_EDIT_NEXT_STYLE: u32 = 1 << 2;
pub const EVIM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME: u32 = 1 << 3;
pub const EVIM_STYLE_CAPABILITY_ASSIGN: u32 = 1 << 4;
pub const EVIM_STYLE_CAPABILITY_DELETE: u32 = 1 << 5;

pub const EVIM_STYLE_PROPERTY_CANVAS_BACKGROUND: u32 = 1;
pub const EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP: u32 = 2;
pub const EVIM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT: u32 = 3;
pub const EVIM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM: u32 = 4;
pub const EVIM_STYLE_PROPERTY_CANVAS_PADDING_LEFT: u32 = 5;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE: u32 = 6;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_AFTER: u32 = 7;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING: u32 = 8;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT: u32 = 9;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT: u32 = 10;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT: u32 = 11;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT: u32 = 12;
pub const EVIM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION: u32 = 13;
pub const EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES: u32 = 14;
pub const EVIM_STYLE_PROPERTY_CHARACTER_SIZE: u32 = 15;
pub const EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT: u32 = 16;
pub const EVIM_STYLE_PROPERTY_CHARACTER_BOLD: u32 = 27;
pub const EVIM_STYLE_PROPERTY_CHARACTER_SLANT: u32 = 17;
pub const EVIM_STYLE_PROPERTY_CHARACTER_FOREGROUND: u32 = 18;
pub const EVIM_STYLE_PROPERTY_CHARACTER_BACKGROUND: u32 = 19;
pub const EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE: u32 = 20;
pub const EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH: u32 = 21;
pub const EVIM_STYLE_PROPERTY_CHARACTER_LANGUAGE: u32 = 22;
pub const EVIM_STYLE_PROPERTY_CHARACTER_DIRECTION: u32 = 23;
pub const EVIM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES: u32 = 24;
pub const EVIM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING: u32 = 25;
pub const EVIM_STYLE_PROPERTY_CHARACTER_BASELINE_SHIFT: u32 = 26;

pub const EVIM_STYLE_VALUE_NONE: u32 = 0;
pub const EVIM_STYLE_VALUE_FLOAT: u32 = 1;
pub const EVIM_STYLE_VALUE_UNSIGNED: u32 = 2;
pub const EVIM_STYLE_VALUE_BOOLEAN: u32 = 3;
pub const EVIM_STYLE_VALUE_COLOR: u32 = 4;
pub const EVIM_STYLE_VALUE_STRING: u32 = 5;
pub const EVIM_STYLE_VALUE_STRING_LIST: u32 = 6;
pub const EVIM_STYLE_VALUE_FONT_SLANT: u32 = 7;
pub const EVIM_STYLE_VALUE_WRITING_DIRECTION: u32 = 8;
pub const EVIM_STYLE_VALUE_OPEN_TYPE_FEATURES: u32 = 9;
pub const EVIM_STYLE_VALUE_LINE_SPACING: u32 = 10;
pub const EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT: u32 = 11;

pub const EVIM_STYLE_VALUE_ITEM_STRING: u32 = 1;
pub const EVIM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE: u32 = 2;

pub const EVIM_STYLE_LINE_SPACING_NORMAL: u32 = 1;
pub const EVIM_STYLE_LINE_SPACING_MULTIPLIER: u32 = 2;
pub const EVIM_STYLE_LINE_SPACING_AT_LEAST: u32 = 3;
pub const EVIM_STYLE_LINE_SPACING_EXACT: u32 = 4;

pub const EVIM_STYLE_PARAGRAPH_ALIGNMENT_START: u32 = 1;
pub const EVIM_STYLE_PARAGRAPH_ALIGNMENT_END: u32 = 2;
pub const EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER: u32 = 3;

pub const EVIM_STYLE_PROPERTY_DECLARED: u32 = 1 << 0;
pub const EVIM_STYLE_PROPERTY_EFFECTIVE_PRESENT: u32 = 1 << 1;
pub const EVIM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE: u32 = 1 << 2;

pub const EVIM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY: u32 = 1;
pub const EVIM_STYLE_CONTRIBUTOR_BLOCK_STYLE: u32 = 2;
pub const EVIM_STYLE_CONTRIBUTOR_CHARACTER_STYLE: u32 = 3;
pub const EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS: u32 = 4;
pub const EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER: u32 = 5;
pub const EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH: u32 = 6;
pub const EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER: u32 = 7;
pub const EVIM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER: u32 = 8;

pub const EVIM_STYLE_EDIT_SET_DECLARATION: u32 = 1;
pub const EVIM_STYLE_EDIT_CLEAR_DECLARATION: u32 = 2;
pub const EVIM_STYLE_EDIT_SET_PARENT: u32 = 3;
pub const EVIM_STYLE_EDIT_CLEAR_PARENT: u32 = 4;
pub const EVIM_STYLE_EDIT_SET_NEXT_STYLE: u32 = 5;
pub const EVIM_STYLE_EDIT_CLEAR_NEXT_STYLE: u32 = 6;
pub const EVIM_STYLE_EDIT_SET_DISPLAY_NAME: u32 = 7;

/// Exact identity of one immutable normalized style sheet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleSheetIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
}

pub const EVIM_STYLE_SHEET_IDENTITY_V1_SIZE: u32 = size_of::<EvimStyleSheetIdentityV1>() as u32;

/// Byte range in the UTF-8 arena returned with one style-sheet export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleStringRefV1 {
    pub offset: u64,
    pub length: u64,
}

/// Fixed summary and exact array/arena sizes for one immutable style sheet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleSheetInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimStyleSheetIdentityV1,
    pub definition_count: u64,
    pub property_count: u64,
    pub value_item_count: u64,
    pub dependency_count: u64,
    pub string_bytes: u64,
}

pub const EVIM_STYLE_SHEET_INFO_V1_SIZE: u32 = size_of::<EvimStyleSheetInfoV1>() as u32;

/// Tagged style value. String and array payloads index the arena and value-item
/// array copied by the same atomic export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimStyleValueV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub enum_value: u32,
    pub reserved: u32,
    pub number: f32,
    pub number_reserved: f32,
    pub color: EvimRgbaV1,
    pub string: EvimStyleStringRefV1,
    pub first_item: u64,
    pub item_count: u64,
}

pub const EVIM_STYLE_VALUE_V1_SIZE: u32 = size_of::<EvimStyleValueV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleValueItemV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub string: EvimStyleStringRefV1,
    pub unsigned_value: u32,
    pub reserved: u32,
}

pub const EVIM_STYLE_VALUE_ITEM_V1_SIZE: u32 = size_of::<EvimStyleValueItemV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleDependencyV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub style_id: EvimStyleStringRefV1,
}

pub const EVIM_STYLE_DEPENDENCY_V1_SIZE: u32 = size_of::<EvimStyleDependencyV1>() as u32;

/// One definition in namespace/stable-ID order. Property indices refer to the
/// property array from the same export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimStyleDefinitionV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub namespace: u32,
    pub role: u32,
    pub origin: u32,
    pub capabilities: u32,
    pub stable_id: EvimStyleStringRefV1,
    pub display_name: EvimStyleStringRefV1,
    pub parent_id: EvimStyleStringRefV1,
    pub next_style_id: EvimStyleStringRefV1,
    pub first_property: u64,
    pub property_count: u64,
}

pub const EVIM_STYLE_DEFINITION_V1_SIZE: u32 = size_of::<EvimStyleDefinitionV1>() as u32;

/// One applicable schema property for a definition. `declared` is meaningful
/// only with `DECLARED`; an absent optional effective value uses kind NONE.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimStylePropertyV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub property: u32,
    pub contributor_kind: u32,
    pub contributor_namespace: u32,
    pub reserved: u32,
    pub declared: EvimStyleValueV1,
    pub effective: EvimStyleValueV1,
    pub contributor_style_id: EvimStyleStringRefV1,
    pub first_dependency: u64,
    pub dependency_count: u64,
}

pub const EVIM_STYLE_PROPERTY_V1_SIZE: u32 = size_of::<EvimStylePropertyV1>() as u32;

/// Caller-owned item for an array-valued style edit. `text` contains a font
/// family or four-byte OpenType tag according to `kind`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimStyleEditValueItemV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub text: EvimUtf8Slice,
    pub unsigned_value: u32,
    pub reserved: u32,
}

pub const EVIM_STYLE_EDIT_VALUE_ITEM_V1_SIZE: u32 = size_of::<EvimStyleEditValueItemV1>() as u32;

impl Default for EvimStyleEditValueItemV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_STYLE_EDIT_VALUE_ITEM_V1_SIZE,
            kind: 0,
            text: EvimUtf8Slice::default(),
            unsigned_value: 0,
            reserved: 0,
        }
    }
}

/// Caller-owned typed value for one style field edit.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimStyleEditValueV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub enum_value: u32,
    pub reserved: u32,
    pub number: f32,
    pub number_reserved: f32,
    pub color: EvimRgbaV1,
    pub text: EvimUtf8Slice,
    pub items: *const EvimStyleEditValueItemV1,
    pub item_count: u64,
}

pub const EVIM_STYLE_EDIT_VALUE_V1_SIZE: u32 = size_of::<EvimStyleEditValueV1>() as u32;

impl Default for EvimStyleEditValueV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_STYLE_EDIT_VALUE_V1_SIZE,
            kind: EVIM_STYLE_VALUE_NONE,
            enum_value: 0,
            reserved: 0,
            number: 0.0,
            number_reserved: 0.0,
            color: EvimRgbaV1::default(),
            text: EvimUtf8Slice::default(),
            items: std::ptr::null(),
            item_count: 0,
        }
    }
}

/// Exact-revision request to edit one field of an existing editable style.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimStyleEditV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: EvimStyleSheetIdentityV1,
    pub namespace: u32,
    pub operation: u32,
    pub property: u32,
    pub reserved: u32,
    pub style_id: EvimUtf8Slice,
    pub value: EvimStyleEditValueV1,
}

pub const EVIM_STYLE_EDIT_V1_SIZE: u32 = size_of::<EvimStyleEditV1>() as u32;

impl Default for EvimStyleEditV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_STYLE_EDIT_V1_SIZE,
            flags: 0,
            identity: EvimStyleSheetIdentityV1::default(),
            namespace: 0,
            operation: 0,
            property: 0,
            reserved: 0,
            style_id: EvimUtf8Slice::default(),
            value: EvimStyleEditValueV1::default(),
        }
    }
}

/// Immutable capability for one explicit live style-edit group. `token` is a
/// process-wide non-reused identifier; all remaining identity fields are part
/// of the capability and must be passed back unchanged. The begin revisions
/// intentionally do not advance as grouped edits commit newer snapshots.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimStyleEditGroupV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub token: u64,
    pub view_id: u64,
    pub document_id: u64,
    pub begin_document_revision: u64,
    pub begin_style_sheet_revision: u64,
}

pub const EVIM_STYLE_EDIT_GROUP_V1_SIZE: u32 = size_of::<EvimStyleEditGroupV1>() as u32;

impl Default for EvimStyleEditGroupV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_STYLE_EDIT_GROUP_V1_SIZE,
            flags: 0,
            token: 0,
            view_id: 0,
            document_id: 0,
            begin_document_revision: 0,
            begin_style_sheet_revision: 0,
        }
    }
}

impl EvimStyleEditGroupV1 {
    fn from_core(group: StyleEditGroup) -> Self {
        Self {
            struct_size: EVIM_STYLE_EDIT_GROUP_V1_SIZE,
            flags: 0,
            token: group.id().0,
            view_id: group.view().0,
            document_id: group.document().0,
            begin_document_revision: group.begin_document_revision().0,
            begin_style_sheet_revision: group.begin_style_sheet_revision().0,
        }
    }

    fn into_core(self) -> StyleEditGroup {
        StyleEditGroup::from_parts(
            StyleEditGroupId(self.token),
            ViewId(self.view_id),
            DocumentId(self.document_id),
            Revision(self.begin_document_revision),
            StyleSheetRevision(self.begin_style_sheet_revision),
        )
    }
}

pub const EVIM_TEXT_PAINT_HAS_BACKGROUND: u32 = 1 << 0;
pub const EVIM_TEXT_PAINT_UNDERLINE: u32 = 1 << 1;
pub const EVIM_TEXT_PAINT_STRIKETHROUGH: u32 = 1 << 2;
pub const EVIM_TEXT_PAINT_DEFAULT_FOREGROUND: u32 = 1 << 3;
pub const EVIM_LAYOUT_PAINT_DEFAULT_CANVAS: u32 = 1 << 0;

/// Fully resolved paint-only text attributes. Foreground is always present;
/// background is meaningful only with `HAS_BACKGROUND`. Decoration flags mean
/// that the corresponding decoration is enabled.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimTextPaintV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub foreground: EvimRgbaV1,
    pub background: EvimRgbaV1,
}

pub const EVIM_TEXT_PAINT_V1_SIZE: u32 = size_of::<EvimTextPaintV1>() as u32;

/// Fixed canvas/default-paint state and required override-run count from one
/// exact immutable layout snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutPaintInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: EvimLayoutSnapshotIdentityV1,
    pub canvas_background: EvimRgbaV1,
    pub default_paint: EvimTextPaintV1,
    pub paint_run_count: u64,
}

pub const EVIM_LAYOUT_PAINT_INFO_V1_SIZE: u32 = size_of::<EvimLayoutPaintInfoV1>() as u32;

/// One logical half-open UTF-8 range whose resolved paint differs from the
/// default paint. Runs are ordered in document order and never overlap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimPaintStyleRunV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub paint: EvimTextPaintV1,
}

pub const EVIM_PAINT_STYLE_RUN_V1_SIZE: u32 = size_of::<EvimPaintStyleRunV1>() as u32;

pub const EVIM_LAYOUT_SNAPSHOT_FULL_DOCUMENT: u32 = 1 << 0;
pub const EVIM_LAYOUT_SNAPSHOT_PREFIX_EXACT: u32 = 1 << 1;
pub const EVIM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT: u32 = 1 << 2;
pub const EVIM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT: u32 = 1 << 3;

/// Fixed summary and required array lengths for a layout export. Geometry is
/// in document-layout coordinates; presentation applies the viewport origin
/// separately. Partial snapshots report their exact materialized coverage.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutSnapshotInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: EvimLayoutSnapshotIdentityV1,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub usable_width: f32,
    pub content_width: f32,
    pub total_height: f32,
    pub content_insets: EvimLayoutInsetsV1,
    pub coverage_hard_line_start: u64,
    pub coverage_hard_line_end: u64,
    pub document_hard_line_count: u64,
    pub coverage_y_start: f32,
    pub coverage_y_end: f32,
    pub row_count: u64,
    pub cluster_count: u64,
    pub caret_count: u64,
}

pub const EVIM_LAYOUT_SNAPSHOT_INFO_V1_SIZE: u32 = size_of::<EvimLayoutSnapshotInfoV1>() as u32;

pub const EVIM_VISUAL_ROW_HAS_PARAGRAPH: u32 = 1 << 0;
pub const EVIM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS: u32 = 1 << 1;
pub const EVIM_VISUAL_ROW_WRAPS_TO_NEXT: u32 = 1 << 2;

/// One positioned visual row. `first_*` and `*_count` index the arrays copied
/// by the same atomic snapshot export.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimVisualRowV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub row_index: u64,
    pub paragraph_id: u64,
    pub hard_line_index: u64,
    pub hard_line_start: u64,
    pub hard_line_end: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub y: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub line_advance: f32,
    pub width: f32,
    pub paragraph_content_x: f32,
    pub paragraph_content_width: f32,
    pub first_cluster: u64,
    pub cluster_count: u64,
    pub first_caret: u64,
    pub caret_count: u64,
}

pub const EVIM_VISUAL_ROW_V1_SIZE: u32 = size_of::<EvimVisualRowV1>() as u32;

pub const EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN: u32 = 1 << 0;

/// One shaped cluster in row visual order. The optional render-run token keeps
/// the provider ownership and generation lifetime declared on
/// [`EvimRenderRunHandleV1`]; core never transfers or retains it for callers.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimPositionedClusterV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub row_index: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub x: f32,
    pub advance: f32,
    pub typographic_bounds: EvimLayoutRectV1,
    pub ink_bounds: EvimLayoutRectV1,
    pub bidi_level: u32,
    pub reserved: u32,
    pub render_run: EvimRenderRunHandleV1,
}

pub const EVIM_POSITIONED_CLUSTER_V1_SIZE: u32 = size_of::<EvimPositionedClusterV1>() as u32;

/// Noneditable layout furniture, with label-local bytes in a separate export
/// blob. These offsets never address formatted document text.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutDecorationV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub row_index: u64,
    pub label_byte_start: u64,
    pub label_byte_length: u64,
    pub x: f32,
    pub advance: f32,
    pub font_size: f32,
    pub reserved: f32,
    pub typographic_bounds: EvimLayoutRectV1,
    pub ink_bounds: EvimLayoutRectV1,
    pub render_run: EvimRenderRunHandleV1,
    pub paint: EvimTextPaintV1,
}
pub const EVIM_LAYOUT_DECORATION_V1_SIZE: u32 = size_of::<EvimLayoutDecorationV1>() as u32;
pub const EVIM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER: u32 = 1 << 1;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutDecorationsInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimLayoutSnapshotIdentityV1,
    pub decoration_count: u64,
    pub label_bytes: u64,
}
pub const EVIM_LAYOUT_DECORATIONS_INFO_V1_SIZE: u32 =
    size_of::<EvimLayoutDecorationsInfoV1>() as u32;

/// One legal shaping caret stop positioned in document-layout coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimPositionedCaretV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub row_index: u64,
    pub text_offset: u64,
    pub x: f32,
    pub reserved: f32,
}

pub const EVIM_POSITIONED_CARET_V1_SIZE: u32 = size_of::<EvimPositionedCaretV1>() as u32;

/// Revision-bound request for logical endpoint geometry. The result may be a
/// containing-cluster fallback without changing the logical endpoint.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutCaretRequestV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub identity: EvimLayoutSnapshotIdentityV1,
    pub text_offset: u64,
}

pub const EVIM_LAYOUT_CARET_REQUEST_V1_SIZE: u32 = size_of::<EvimLayoutCaretRequestV1>() as u32;

/// Revision-bound request for hit testing a document-layout coordinate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutHitTestRequestV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimLayoutSnapshotIdentityV1,
    pub x: f32,
    pub y: f32,
}

pub const EVIM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE: u32 =
    size_of::<EvimLayoutHitTestRequestV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimLayoutCaretPointV1 {
    pub struct_size: u32,
    pub affinity: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub layout_revision: u64,
    pub text_offset: u64,
}

pub const EVIM_LAYOUT_CARET_POINT_V1_SIZE: u32 = size_of::<EvimLayoutCaretPointV1>() as u32;

pub const EVIM_CARET_GEOMETRY_CLUSTER_FALLBACK: u32 = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimLayoutCaretGeometryV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub point: EvimLayoutCaretPointV1,
    pub rect: EvimLayoutRectV1,
    pub row_index: u64,
}

pub const EVIM_LAYOUT_CARET_GEOMETRY_V1_SIZE: u32 = size_of::<EvimLayoutCaretGeometryV1>() as u32;

pub const EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR: u32 = 1 << 0;
pub const EVIM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT: u32 = 1 << 1;
pub const EVIM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK: u32 = 1 << 2;
pub const EVIM_VIEW_PRESENTATION_HAS_COMMAND_LINE: u32 = 1 << 3;
pub const EVIM_VIEW_PRESENTATION_HAS_DESIRED_X: u32 = 1 << 4;

/// Current controller presentation state. Linear Visual anchors do not retain
/// a visual affinity, so their affinity field is zero unless the exact flag is
/// present. Visual Block endpoints retain exact layout affinity and x edges.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimViewPresentationV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub mode: u32,
    pub cursor_affinity: u32,
    pub visual_anchor_affinity: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub cursor_utf8_offset: u64,
    pub visual_anchor_utf8_offset: u64,
    pub visual_block_left_x: f32,
    pub visual_block_right_x: f32,
    pub desired_x: f32,
    pub reserved_float: f32,
    pub command_line_utf8_length: u64,
    pub command_line_cursor_utf8_offset: u64,
}

pub const EVIM_VIEW_PRESENTATION_V1_SIZE: u32 = size_of::<EvimViewPresentationV1>() as u32;

pub const EVIM_COMMAND_LINE_KIND_NONE: u32 = 0;
pub const EVIM_COMMAND_LINE_KIND_EX: u32 = 1;
pub const EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD: u32 = 2;
pub const EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD: u32 = 3;

/// Exact identity of one command-line byte export. The opaque state identity
/// changes whenever the exported kind, cursor, or UTF-8 bytes change, even
/// when the document revision does not.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimCommandLineIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub state_identity: [u8; 32],
}

pub const EVIM_COMMAND_LINE_IDENTITY_V1_SIZE: u32 = size_of::<EvimCommandLineIdentityV1>() as u32;

/// Fixed summary and required byte length for one command-line export. The
/// cursor is a UTF-8 byte boundary in the copied bytes; the prompt prefix is
/// represented by `identity.kind` and is not part of those bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimCommandLineInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimCommandLineIdentityV1,
    pub utf8_length: u64,
    pub cursor_utf8_offset: u64,
}

pub const EVIM_COMMAND_LINE_INFO_V1_SIZE: u32 = size_of::<EvimCommandLineInfoV1>() as u32;

pub const EVIM_VISUAL_SELECTION_KIND_NONE: u32 = 0;
pub const EVIM_VISUAL_SELECTION_KIND_CHARACTER: u32 = 1;
pub const EVIM_VISUAL_SELECTION_KIND_LINE: u32 = 2;
pub const EVIM_VISUAL_SELECTION_KIND_BLOCK: u32 = 3;

pub const EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW: u32 = 1 << 0;
pub const EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE: u32 = 1 << 1;
pub const EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES: u32 = 1 << 2;

/// Exact identity of one Visual-selection export. Layout identity makes every
/// rectangle revision-bound; the opaque state identity additionally changes
/// when the selection payload changes without relayout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimVisualSelectionIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub layout: EvimLayoutSnapshotIdentityV1,
    pub state_identity: [u8; 32],
}

pub const EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE: u32 =
    size_of::<EvimVisualSelectionIdentityV1>() as u32;

/// Required array counts for the exact current Visual selection. Segments are
/// ordered by logical UTF-8 document order; rectangles are ordered by visual
/// row and x coordinate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimVisualSelectionInfoV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimVisualSelectionIdentityV1,
    pub segment_count: u64,
    pub rectangle_count: u64,
}

pub const EVIM_VISUAL_SELECTION_INFO_V1_SIZE: u32 = size_of::<EvimVisualSelectionInfoV1>() as u32;

/// One exact logical half-open UTF-8 selection segment. Character- and
/// Linewise Visual selections have one untagged segment. Blockwise segments
/// retain their originating row, hard line, and display-edge affinities.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimVisualSelectionSegmentV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub text_start: u64,
    pub text_end: u64,
    pub row_index: u64,
    pub hard_line_index: u64,
    pub left_affinity: u32,
    pub right_affinity: u32,
}

pub const EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE: u32 =
    size_of::<EvimVisualSelectionSegmentV1>() as u32;

/// One drawable rectangle for a logical selection segment. `segment_index`
/// indexes the segment array returned by the same atomic copy.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvimVisualSelectionRectangleV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub row_index: u64,
    pub segment_index: u64,
    pub rect: EvimLayoutRectV1,
}

pub const EVIM_VISUAL_SELECTION_RECTANGLE_V1_SIZE: u32 =
    size_of::<EvimVisualSelectionRectangleV1>() as u32;

pub const EVIM_LOGICAL_SELECTION_KIND_NONE: u32 = 0;
pub const EVIM_LOGICAL_SELECTION_KIND_CHARACTER: u32 = 1;
pub const EVIM_LOGICAL_SELECTION_KIND_LINE: u32 = 2;
pub const EVIM_LOGICAL_SELECTION_KIND_BLOCK: u32 = 3;

pub const EVIM_SEMANTIC_STYLE_STRONG: u32 = 1;
pub const EVIM_SEMANTIC_STYLE_EMPHASIS: u32 = 2;

pub const EVIM_SEMANTIC_STYLE_STATE_OFF: u32 = 0;
pub const EVIM_SEMANTIC_STYLE_STATE_ON: u32 = 1;
pub const EVIM_SEMANTIC_STYLE_STATE_MIXED: u32 = 2;

pub const EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE: u32 = 1 << 0;
/// Exact Insert/Replace caret target; toggles change pending typing policy only.
pub const EVIM_SEMANTIC_STYLE_TYPING_CONTEXT: u32 = 1 << 3;
pub const EVIM_SEMANTIC_STYLE_CAN_SET: u32 = 1 << 1;
pub const EVIM_SEMANTIC_STYLE_CAN_CLEAR: u32 = 1 << 2;

/// Exact current logical selection identity without layout geometry. Only
/// Character- and Linewise identities name an actionable contiguous range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimLogicalSelectionIdentityV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub state_identity: [u8; 32],
}

pub const EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE: u32 =
    size_of::<EvimLogicalSelectionIdentityV1>() as u32;

/// Check/mixed state and exact source-adapter capabilities for one semantic
/// style at the current core-owned selection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSemanticStylePresentationV1 {
    pub struct_size: u32,
    pub style: u32,
    pub state: u32,
    pub flags: u32,
    pub selection: EvimLogicalSelectionIdentityV1,
}

pub const EVIM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE: u32 =
    size_of::<EvimSemanticStylePresentationV1>() as u32;

/// Revision- and selection-bound request to set or clear one semantic style.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSetSemanticStyleV1 {
    pub struct_size: u32,
    pub style: u32,
    pub enabled: u32,
    pub reserved: u32,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
}

pub const EVIM_SET_SEMANTIC_STYLE_V1_SIZE: u32 = size_of::<EvimSetSemanticStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EvimDirectStyleEditV1 {
    pub struct_size: u32,
    pub operation: u32,
    pub property: u32,
    pub reserved: u32,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
    pub value: EvimStyleEditValueV1,
}
pub const EVIM_DIRECT_STYLE_EDIT_V1_SIZE: u32 = size_of::<EvimDirectStyleEditV1>() as u32;

pub const EVIM_PLACE_CURSOR_EXTEND_SELECTION: u32 = 1 << 0;

/// Revision-bound pointer-placement intention. `text_offset` is a formatted
/// UTF-8 boundary returned by exact hit testing; affinity preserves the visual
/// side at wrap or bidi split carets.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimPlaceCursorV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_revision: u64,
    pub text_offset: u64,
    pub affinity: u32,
    pub reserved: u32,
}

pub const EVIM_PLACE_CURSOR_V1_SIZE: u32 = size_of::<EvimPlaceCursorV1>() as u32;

/// Exact model identity for a native file-format change. Only the concrete
/// Unix, DOS, and Mac values are accepted; detection is an open-time policy.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimSetFileFormatV1 {
    pub struct_size: u32,
    pub file_format: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const EVIM_SET_FILE_FORMAT_V1_SIZE: u32 = size_of::<EvimSetFileFormatV1>() as u32;

/// Shared HTML style-serialization policy bound to one exact snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimSetIncludeStyleDefinitionsV1 {
    pub struct_size: u32,
    pub enabled: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const EVIM_SET_INCLUDE_STYLE_DEFINITIONS_V1_SIZE: u32 =
    size_of::<EvimSetIncludeStyleDefinitionsV1>() as u32;

impl Default for EvimSetIncludeStyleDefinitionsV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_SET_INCLUDE_STYLE_DEFINITIONS_V1_SIZE,
            enabled: 0,
            document_id: 0,
            document_revision: 0,
        }
    }
}

/// Source-format interpretation change bound to one exact document snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSetFormatV1 {
    pub struct_size: u32,
    pub format: u32,
    pub document_id: u64,
    pub document_revision: u64,
}
pub const EVIM_SET_FORMAT_V1_SIZE: u32 = size_of::<EvimSetFormatV1>() as u32;

/// Lossless source transcoding request; automatic detection is not a target.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSetEncodingV1 {
    pub struct_size: u32,
    pub encoding: u32,
    pub document_id: u64,
    pub document_revision: u64,
}
pub const EVIM_SET_ENCODING_V1_SIZE: u32 = size_of::<EvimSetEncodingV1>() as u32;

pub const EVIM_LIST_STYLE_NONE: u32 = 0;
pub const EVIM_LIST_STYLE_BULLET: u32 = 1;
pub const EVIM_LIST_STYLE_NUMBERED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSetListStyleV1 {
    pub struct_size: u32,
    pub style: u32,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
}
pub const EVIM_SET_LIST_STYLE_V1_SIZE: u32 = size_of::<EvimSetListStyleV1>() as u32;

pub const EVIM_LIST_CAN_INDENT: u32 = 1;
pub const EVIM_LIST_CAN_UNINDENT: u32 = 2;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimListIndentV1 {
    pub struct_size: u32,
    pub unindent: u32,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
}
pub const EVIM_LIST_INDENT_V1_SIZE: u32 = size_of::<EvimListIndentV1>() as u32;


#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimSetParagraphStyleV1 {
    pub struct_size: u32,
    /// Zero means Base Paragraph; one through six select a heading.
    pub level: u32,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
}
pub const EVIM_SET_PARAGRAPH_STYLE_V1_SIZE: u32 = size_of::<EvimSetParagraphStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimAssignStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: EvimStyleSheetIdentityV1,
    pub expected_selection: EvimLogicalSelectionIdentityV1,
    pub style_id: EvimUtf8Slice,
}
pub const EVIM_ASSIGN_STYLE_V1_SIZE: u32 = size_of::<EvimAssignStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimCreateStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: EvimStyleSheetIdentityV1,
    pub style_id: EvimUtf8Slice,
    pub display_name: EvimUtf8Slice,
    pub parent_id: EvimUtf8Slice,
    pub next_style_id: EvimUtf8Slice,
}
pub const EVIM_CREATE_STYLE_V1_SIZE: u32 = size_of::<EvimCreateStyleV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EvimDeleteStyleV1 {
    pub struct_size: u32,
    pub namespace: u32,
    pub identity: EvimStyleSheetIdentityV1,
    pub style_id: EvimUtf8Slice,
}
pub const EVIM_DELETE_STYLE_V1_SIZE: u32 = size_of::<EvimDeleteStyleV1>() as u32;

impl Default for EvimSetFileFormatV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_SET_FILE_FORMAT_V1_SIZE,
            file_format: EVIM_FILE_FORMAT_UNIX,
            document_id: 0,
            document_revision: 0,
        }
    }
}

/// Exact acknowledgement that the current authoritative source snapshot was
/// written successfully by a native document frontend.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvimMarkSavedV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub document_id: u64,
    pub document_revision: u64,
}

pub const EVIM_MARK_SAVED_V1_SIZE: u32 = size_of::<EvimMarkSavedV1>() as u32;

impl Default for EvimMarkSavedV1 {
    fn default() -> Self {
        Self {
            struct_size: EVIM_MARK_SAVED_V1_SIZE,
            reserved: 0,
            document_id: 0,
            document_revision: 0,
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

pub const EVIM_COMPOSITION_OVERLAY_ACTIVE: u32 = 1 << 0;

/// Exact identity of one disposable per-view composition projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimCompositionOverlayIdentityV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub view_id: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub generation: u64,
}

pub const EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE: u32 =
    size_of::<EvimCompositionOverlayIdentityV1>() as u32;

/// Metadata and exact ranges in the temporary composed UTF-8 projection.
/// An inactive view returns a zeroed value without `ACTIVE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimCompositionOverlayInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: EvimCompositionOverlayIdentityV1,
    pub utf8_length: u64,
    pub replacement_start: u64,
    pub replacement_end: u64,
    pub marked_start: u64,
    pub marked_end: u64,
    pub selected_start: u64,
    pub selected_end: u64,
}

pub const EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE: u32 =
    size_of::<EvimCompositionOverlayInfoV1>() as u32;

/// Scalar-aligned range read from one exact composition projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EvimCompositionOverlayUtf8RangeV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub identity: EvimCompositionOverlayIdentityV1,
    pub start: u64,
    pub end: u64,
}

pub const EVIM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE: u32 =
    size_of::<EvimCompositionOverlayUtf8RangeV1>() as u32;

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

fn parse_encoding(raw: u32) -> Result<Option<Encoding>, EvimStatus> {
    match raw {
        EVIM_ENCODING_DETECT => Ok(None),
        EVIM_ENCODING_UTF8 => Ok(Some(Encoding::Utf8)),
        EVIM_ENCODING_LATIN1 => Ok(Some(Encoding::Latin1)),
        EVIM_ENCODING_UTF16_LE => Ok(Some(Encoding::Utf16Le)),
        EVIM_ENCODING_UTF16_BE => Ok(Some(Encoding::Utf16Be)),
        _ => Err(EvimStatus::InvalidEncoding),
    }
}

fn parse_format(raw: u32) -> Result<Format, EvimStatus> {
    match raw {
        EVIM_FORMAT_PLAIN_TEXT => Ok(Format::PlainText),
        EVIM_FORMAT_MARKDOWN => Ok(Format::Markdown),
        EVIM_FORMAT_HTML => Ok(Format::Html),
        EVIM_FORMAT_RTF => Ok(Format::Rtf),
        EVIM_FORMAT_MARKDOWN_SOURCE => Ok(Format::MarkdownSource),
        EVIM_FORMAT_HTML_SOURCE => Ok(Format::HtmlSource),
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

fn parse_concrete_file_format(raw: u32) -> Result<FileFormat, EvimStatus> {
    parse_file_format(raw)?.ok_or(EvimStatus::InvalidFileFormat)
}

fn encoding_to_ffi(encoding: Encoding) -> u32 {
    match encoding {
        Encoding::Utf8 => EVIM_ENCODING_UTF8,
        Encoding::Latin1 => EVIM_ENCODING_LATIN1,
        Encoding::Utf16Le => EVIM_ENCODING_UTF16_LE,
        Encoding::Utf16Be => EVIM_ENCODING_UTF16_BE,
    }
}

fn format_to_ffi(format: Format) -> u32 {
    match format {
        Format::PlainText => EVIM_FORMAT_PLAIN_TEXT,
        Format::Markdown => EVIM_FORMAT_MARKDOWN,
        Format::Html => EVIM_FORMAT_HTML,
        Format::Rtf => EVIM_FORMAT_RTF,
        Format::MarkdownSource => EVIM_FORMAT_MARKDOWN_SOURCE,
        Format::HtmlSource => EVIM_FORMAT_HTML_SOURCE,
    }
}

fn file_format_to_ffi(file_format: FileFormat) -> u32 {
    match file_format {
        FileFormat::Unix => EVIM_FILE_FORMAT_UNIX,
        FileFormat::Dos => EVIM_FILE_FORMAT_DOS,
        FileFormat::Mac => EVIM_FILE_FORMAT_MAC,
    }
}

fn file_format_origin_to_ffi(origin: FileFormatOrigin) -> u32 {
    match origin {
        FileFormatOrigin::Detected => EVIM_FILE_FORMAT_ORIGIN_DETECTED,
        FileFormatOrigin::Forced => EVIM_FILE_FORMAT_ORIGIN_FORCED,
        FileFormatOrigin::Defaulted => EVIM_FILE_FORMAT_ORIGIN_DEFAULTED,
    }
}

fn history_change_to_ffi(change: HistorySemanticChangeKind) -> u32 {
    match change {
        HistorySemanticChangeKind::Text => EVIM_HISTORY_ACTION_CATEGORY_TEXT,
        HistorySemanticChangeKind::Style => EVIM_HISTORY_ACTION_CATEGORY_STYLE,
        HistorySemanticChangeKind::FileFormat => EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT,
        HistorySemanticChangeKind::HardLineTransfer => {
            EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER
        }
        HistorySemanticChangeKind::HardLineSourceRestoration => {
            EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_SOURCE_RESTORATION
        }
        HistorySemanticChangeKind::SourceMetadata => EVIM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA,
    }
}

fn history_action_category(summary: Option<&HistorySemanticSummary>) -> u32 {
    let Some(changes) = summary.map(HistorySemanticSummary::changes) else {
        return EVIM_HISTORY_ACTION_CATEGORY_NONE;
    };
    let Some(first) = changes.first().copied() else {
        return EVIM_HISTORY_ACTION_CATEGORY_NONE;
    };
    if changes.iter().all(|change| *change == first) {
        history_change_to_ffi(first)
    } else {
        EVIM_HISTORY_ACTION_CATEGORY_MIXED
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

fn formatted_snapshot_identity(document: &Document) -> EvimFormattedSnapshotIdentityV1 {
    EvimFormattedSnapshotIdentityV1 {
        struct_size: EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE,
        reserved: 0,
        document_id: document.id().0,
        document_revision: document.revision().0,
    }
}

fn validate_formatted_snapshot_identity(
    expected: EvimFormattedSnapshotIdentityV1,
    document: &Document,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let actual = formatted_snapshot_identity(document);
    (expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn formatted_text_status(error: FormattedTextError) -> EvimStatus {
    match error {
        FormattedTextError::InvalidByteOffset { .. }
        | FormattedTextError::InvalidRange { .. }
        | FormattedTextError::InvalidUtf16Offset { .. }
        | FormattedTextError::HardLineOutOfBounds { .. } => EvimStatus::InvalidRange,
        FormattedTextError::NotCharBoundary(_) => EvimStatus::InvalidUtf8Boundary,
        FormattedTextError::NotUtf16Boundary(_) => EvimStatus::InvalidUtf16Boundary,
        FormattedTextError::NotGraphemeBoundary(_) => EvimStatus::NotGraphemeBoundary,
        FormattedTextError::OverlappingSplices { .. } => EvimStatus::InvalidArgument,
        FormattedTextError::ArithmeticOverflow => EvimStatus::LengthOverflow,
        FormattedTextError::LeafIdentityExhausted => EvimStatus::ResourceExhausted,
        FormattedTextError::UnicodeBoundaryResolutionFailed => EvimStatus::CoreFailure,
        FormattedTextError::ResultTextMismatch => EvimStatus::VerificationFailed,
    }
}

fn hard_line_query_status(error: HardLineQueryError) -> EvimStatus {
    match error {
        HardLineQueryError::InvalidLineRange { .. }
        | HardLineQueryError::FormattedOffsetOutOfBounds { .. } => EvimStatus::InvalidRange,
        HardLineQueryError::NotCharacterBoundary { .. } => EvimStatus::InvalidUtf8Boundary,
    }
}

fn formatted_snapshot_info(document: &Document) -> Result<EvimFormattedSnapshotInfoV1, EvimStatus> {
    let snapshot = document.hard_line_snapshot();
    Ok(EvimFormattedSnapshotInfoV1 {
        struct_size: EVIM_FORMATTED_SNAPSHOT_INFO_V1_SIZE,
        reserved: 0,
        identity: formatted_snapshot_identity(document),
        utf8_length: checked_export_count(snapshot.text_length())?,
        utf16_length: checked_export_count(snapshot.utf16_length())?,
        hard_line_count: checked_export_count(snapshot.line_count())?,
    })
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
        DocumentError::LineEndingConversionWouldReinterpretContent
        | DocumentError::UnrepresentableFormattedCharacter { .. } => EvimStatus::PolicyRequired,
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
            reserved: u32::from(style.relative_bold),
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

#[derive(Clone, Debug)]
struct OwnedEffectBatch {
    document_id: DocumentId,
    document_revision: Revision,
    ex_outcome: Option<ExOutcome>,
    ex_info_payloads: Vec<Option<OwnedExInfoPayload>>,
    clipboard_writes: Vec<ClipboardWriteRequest>,
}

#[derive(Clone, Debug)]
struct OwnedResolvedPoint {
    utf8_offset: usize,
    hard_line_index: usize,
    hard_line_range: std::ops::Range<usize>,
    grapheme_column: usize,
    line_text: String,
}

#[derive(Clone, Debug)]
struct OwnedResolvedMark {
    name: char,
    point: OwnedResolvedPoint,
}

#[derive(Clone, Debug)]
struct OwnedResolvedRegister {
    name: char,
    value: crate::command::RegisterValue,
}

#[derive(Clone, Debug)]
struct OwnedResolvedJump {
    list_index: usize,
    current: bool,
    point: OwnedResolvedPoint,
}

#[derive(Clone, Debug)]
struct OwnedResolvedTextLine {
    hard_line_index: usize,
    utf8_range: std::ops::Range<usize>,
    text: String,
}

#[derive(Clone, Debug)]
enum OwnedExInfoPayload {
    Marks(Vec<OwnedResolvedMark>),
    Registers(Vec<OwnedResolvedRegister>),
    Jumps(Vec<OwnedResolvedJump>),
    TextLines(Vec<OwnedResolvedTextLine>),
}

fn capture_resolved_point(
    snapshot: &crate::document::HardLineSnapshot,
    utf8_offset: usize,
) -> OwnedResolvedPoint {
    let line = snapshot
        .line_at_offset(utf8_offset)
        .expect("a retained command position resolves in its exact snapshot");
    let range = line.content_range();
    let grapheme_column = snapshot
        .grapheme_count(range.start..utf8_offset)
        .expect("a retained command position is a grapheme boundary");
    let line_text = snapshot
        .slice_utf8(range.clone())
        .expect("a hard-line range is a valid UTF-8 range");
    OwnedResolvedPoint {
        utf8_offset,
        hard_line_index: line.index(),
        hard_line_range: range,
        grapheme_column,
        line_text,
    }
}

fn capture_ex_info_payloads(
    document: &Document,
    commands: &crate::command::CommandInterpreter,
    clipboard: &ClipboardCommandContext,
    ex: &ExOutcome,
) -> Vec<Option<OwnedExInfoPayload>> {
    let snapshot = document.hard_line_snapshot();
    ex.frontend_requests
        .iter()
        .map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Marks(requested)) => {
                let marks = if requested.is_empty() {
                    commands.mark_positions().collect::<Vec<_>>()
                } else {
                    requested
                        .iter()
                        .filter_map(|requested| {
                            commands
                                .mark_positions()
                                .find(|(name, _)| name == requested)
                        })
                        .collect()
                };
                Some(OwnedExInfoPayload::Marks(
                    marks
                        .into_iter()
                        .map(|(name, offset)| OwnedResolvedMark {
                            name,
                            point: capture_resolved_point(&snapshot, offset),
                        })
                        .collect(),
                ))
            }
            ExFrontendRequest::Info(ExInfoRequest::Registers(requested)) => {
                let names = if requested.is_empty() {
                    let mut names = commands.stored_register_names();
                    if document.artifact_binding().is_some() {
                        names.push('%');
                    }
                    for (name, target) in [
                        ('+', ClipboardTarget::Clipboard),
                        ('*', ClipboardTarget::Primary),
                    ] {
                        if clipboard.read(target).is_some() {
                            names.push(name);
                        }
                    }
                    names.sort_unstable();
                    names.dedup();
                    names
                } else {
                    requested.clone()
                };
                Some(OwnedExInfoPayload::Registers(
                    names
                        .into_iter()
                        .filter_map(|name| {
                            commands
                                .register_with_context(document, clipboard, name)
                                .ok()
                                .flatten()
                                .map(|value| OwnedResolvedRegister { name, value })
                        })
                        .collect(),
                ))
            }
            ExFrontendRequest::Info(ExInfoRequest::Jumps) => {
                let (positions, current) = commands.jump_positions();
                Some(OwnedExInfoPayload::Jumps(
                    positions
                        .iter()
                        .enumerate()
                        .map(|(list_index, offset)| OwnedResolvedJump {
                            list_index,
                            current: current == Some(list_index),
                            point: capture_resolved_point(&snapshot, *offset),
                        })
                        .collect(),
                ))
            }
            ExFrontendRequest::Info(ExInfoRequest::PrintLines { range, .. }) => {
                Some(OwnedExInfoPayload::TextLines(
                    (range.start..=range.end)
                        .map(|index| {
                            snapshot.line(index).expect(
                                "a committed Ex print range resolves in its post-turn snapshot",
                            )
                        })
                        .map(|line| {
                            let range = line.content_range();
                            OwnedResolvedTextLine {
                                hard_line_index: line.index(),
                                utf8_range: range.clone(),
                                text: snapshot
                                    .slice_utf8(range)
                                    .expect("a hard-line range is valid UTF-8"),
                            }
                        })
                        .collect(),
                ))
            }
            ExFrontendRequest::Info(ExInfoRequest::Options(_))
            | ExFrontendRequest::Info(ExInfoRequest::Message(_))
            | ExFrontendRequest::File(_)
            | ExFrontendRequest::Normal(_) => None,
        })
        .collect()
}

impl OwnedEffectBatch {
    fn from_command(
        document: &Document,
        commands: &crate::command::CommandInterpreter,
        clipboard: &ClipboardCommandContext,
        command: Option<CommandOutput>,
    ) -> Option<Self> {
        let mut command = command?;
        if let CommandStatus::ExError(error) = &command.status {
            let message = match error {
                crate::command::ExCommandError::Parse(error) => error.to_string(),
                crate::command::ExCommandError::Execute(error) => error.to_string(),
            };
            command
                .ex_outcome
                .get_or_insert_with(ExOutcome::default)
                .frontend_requests
                .push(ExFrontendRequest::Info(ExInfoRequest::Message(message)));
        }
        if command.ex_outcome.is_none() && command.clipboard_writes.is_empty() {
            return None;
        }
        let ex_info_payloads = command
            .ex_outcome
            .as_ref()
            .map(|ex| capture_ex_info_payloads(document, commands, clipboard, ex))
            .unwrap_or_default();
        Some(Self {
            document_id: document.id(),
            document_revision: document.revision(),
            ex_outcome: command.ex_outcome,
            ex_info_payloads,
            clipboard_writes: command.clipboard_writes,
        })
    }
}

#[derive(Clone, Debug)]
struct EffectBatchExport {
    info: EvimEffectBatchInfoV1,
    clipboard_writes: Vec<EvimClipboardWriteV1>,
    ex_requests: Vec<EvimExFrontendRequestV1>,
    ex_options: Vec<EvimExOptionDisplayV1>,
    ex_marks: Vec<EvimExMarkV1>,
    ex_registers: Vec<EvimExRegisterV1>,
    ex_jumps: Vec<EvimExJumpV1>,
    ex_text_lines: Vec<EvimExTextLineV1>,
    file_formats: Vec<u32>,
    hard_breaks: Vec<u64>,
    strings: Vec<u8>,
}

fn push_effect_text(strings: &mut Vec<u8>, text: &str) -> Result<EvimEffectBytesRefV1, EvimStatus> {
    let offset = checked_export_count(strings.len())?;
    let length = checked_export_count(text.len())?;
    strings.extend_from_slice(text.as_bytes());
    Ok(EvimEffectBytesRefV1 { offset, length })
}

fn clipboard_target_to_ffi(target: ClipboardTarget) -> u32 {
    match target {
        ClipboardTarget::Clipboard => EVIM_CLIPBOARD_TARGET_CLIPBOARD,
        ClipboardTarget::Primary => EVIM_CLIPBOARD_TARGET_PRIMARY,
    }
}

fn register_kind_to_ffi(kind: RegisterKind) -> u32 {
    match kind {
        RegisterKind::Characterwise => EVIM_REGISTER_KIND_CHARACTER,
        RegisterKind::Linewise => EVIM_REGISTER_KIND_LINE,
        RegisterKind::Blockwise => EVIM_REGISTER_KIND_BLOCK,
    }
}

fn ex_option_name_to_ffi(name: &ExOptionName) -> u32 {
    match name {
        ExOptionName::Wrap => EVIM_EX_OPTION_WRAP,
        ExOptionName::FileFormat => EVIM_EX_OPTION_FILE_FORMAT,
        ExOptionName::FileFormats => EVIM_EX_OPTION_FILE_FORMATS,
        ExOptionName::IgnoreCase => EVIM_EX_OPTION_IGNORECASE,
        ExOptionName::SmartCase => EVIM_EX_OPTION_SMARTCASE,
        ExOptionName::WrapScan => EVIM_EX_OPTION_WRAPSCAN,
    }
}

fn export_ex_option(
    option: &ExOptionDisplay,
    file_formats: &mut Vec<u32>,
) -> Result<EvimExOptionDisplayV1, EvimStatus> {
    let first_file_format = checked_export_count(file_formats.len())?;
    let (value_kind, scalar_value) = match &option.value {
        ExOptionValue::Boolean(value) => (EVIM_EX_OPTION_VALUE_BOOLEAN, u32::from(*value)),
        ExOptionValue::FileFormat(value) => {
            (EVIM_EX_OPTION_VALUE_FILE_FORMAT, file_format_to_ffi(*value))
        }
        ExOptionValue::FileFormats(values) => {
            file_formats.extend(values.iter().copied().map(file_format_to_ffi));
            (EVIM_EX_OPTION_VALUE_FILE_FORMATS, 0)
        }
    };
    Ok(EvimExOptionDisplayV1 {
        struct_size: EVIM_EX_OPTION_DISPLAY_V1_SIZE,
        name: ex_option_name_to_ffi(&option.name),
        value_kind,
        scalar_value,
        first_file_format,
        file_format_count: checked_export_count(file_formats.len())?
            .checked_sub(first_file_format)
            .ok_or(EvimStatus::LengthOverflow)?,
    })
}

fn set_ex_path(
    output: &mut EvimExFrontendRequestV1,
    strings: &mut Vec<u8>,
    path: Option<&str>,
) -> Result<(), EvimStatus> {
    if let Some(path) = path {
        output.flags |= EVIM_EX_FRONTEND_HAS_PATH;
        output.text = push_effect_text(strings, path)?;
    }
    Ok(())
}

fn set_ex_range(output: &mut EvimExFrontendRequestV1, range: Option<HardLineRange>) {
    if let Some(range) = range {
        output.flags |= EVIM_EX_FRONTEND_HAS_RANGE;
        output.hard_line_start = range.start as u64;
        output.hard_line_end = range.end as u64;
    }
}

#[allow(clippy::too_many_arguments)]
fn export_ex_frontend_request(
    request: &ExFrontendRequest,
    payload: Option<&OwnedExInfoPayload>,
    document_id: DocumentId,
    document_revision: Revision,
    strings: &mut Vec<u8>,
    options: &mut Vec<EvimExOptionDisplayV1>,
    marks: &mut Vec<EvimExMarkV1>,
    registers: &mut Vec<EvimExRegisterV1>,
    jumps: &mut Vec<EvimExJumpV1>,
    text_lines: &mut Vec<EvimExTextLineV1>,
    file_formats: &mut Vec<u32>,
    hard_breaks: &mut Vec<u64>,
) -> Result<EvimExFrontendRequestV1, EvimStatus> {
    let mut output = EvimExFrontendRequestV1 {
        struct_size: EVIM_EX_FRONTEND_REQUEST_V1_SIZE,
        document_id: document_id.0,
        document_revision: document_revision.0,
        ..EvimExFrontendRequestV1::default()
    };
    match request {
        ExFrontendRequest::File(request) => match request {
            ExFileRequest::EditNewWindow { path } => {
                output.kind = EVIM_EX_FRONTEND_EDIT_NEW_WINDOW;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::CheckTime => {
                output.kind = EVIM_EX_FRONTEND_CHECKTIME;
            }
            ExFileRequest::PrintWorkingDirectory => {
                output.kind = EVIM_EX_FRONTEND_PWD;
            }
            ExFileRequest::ChangeDirectory { path } => {
                output.kind = EVIM_EX_FRONTEND_CD;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::Split { path } => {
                output.kind = EVIM_EX_FRONTEND_SPLIT;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::Edit { path, force } => {
                output.kind = EVIM_EX_FRONTEND_EDIT;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::New { force } => {
                output.kind = EVIM_EX_FRONTEND_NEW;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::Write { path, force, range } => {
                output.kind = EVIM_EX_FRONTEND_WRITE;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
                set_ex_range(&mut output, *range);
            }
            ExFileRequest::SaveAs { path, force } => {
                output.kind = EVIM_EX_FRONTEND_SAVE_AS;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, Some(path))?;
            }
            ExFileRequest::Quit { force } => {
                output.kind = EVIM_EX_FRONTEND_QUIT;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::QuitAll { force } => {
                output.kind = EVIM_EX_FRONTEND_QUIT_ALL;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
            }
            ExFileRequest::WriteQuit { path, force, range } => {
                output.kind = EVIM_EX_FRONTEND_WRITE_QUIT;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
                set_ex_range(&mut output, *range);
            }
            ExFileRequest::Xit { path, force } => {
                output.kind = EVIM_EX_FRONTEND_XIT;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
                set_ex_path(&mut output, strings, path.as_deref())?;
            }
            ExFileRequest::WriteAll { force } => {
                output.kind = EVIM_EX_FRONTEND_WRITE_ALL;
                output.flags |= u32::from(*force) * EVIM_EX_FRONTEND_FORCE;
            }
        },
        ExFrontendRequest::Info(request) => match request {
            ExInfoRequest::Message(message) => {
                output.kind = EVIM_EX_FRONTEND_MESSAGE;
                output.text = push_effect_text(strings, message)?;
            }
            ExInfoRequest::Marks(names) => {
                output.kind = EVIM_EX_FRONTEND_MARKS;
                let names: String = names.iter().collect();
                output.text = push_effect_text(strings, &names)?;
                let Some(OwnedExInfoPayload::Marks(resolved)) = payload else {
                    return Err(EvimStatus::InternalError);
                };
                output.first_payload = checked_export_count(marks.len())?;
                for mark in resolved {
                    marks.push(EvimExMarkV1 {
                        struct_size: EVIM_EX_MARK_V1_SIZE,
                        name: u32::from(mark.name),
                        utf8_offset: checked_export_count(mark.point.utf8_offset)?,
                        hard_line_index: checked_export_count(mark.point.hard_line_index)?,
                        hard_line_start: checked_export_count(mark.point.hard_line_range.start)?,
                        hard_line_end: checked_export_count(mark.point.hard_line_range.end)?,
                        grapheme_column: checked_export_count(mark.point.grapheme_column)?,
                        line_text: push_effect_text(strings, &mark.point.line_text)?,
                    });
                }
                output.payload_count = checked_export_count(marks.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(EvimStatus::LengthOverflow)?;
            }
            ExInfoRequest::Registers(names) => {
                output.kind = EVIM_EX_FRONTEND_REGISTERS;
                let names: String = names.iter().collect();
                output.text = push_effect_text(strings, &names)?;
                let Some(OwnedExInfoPayload::Registers(resolved)) = payload else {
                    return Err(EvimStatus::InternalError);
                };
                output.first_payload = checked_export_count(registers.len())?;
                for register in resolved {
                    let first_hard_break = checked_export_count(hard_breaks.len())?;
                    hard_breaks.extend(
                        register
                            .value
                            .hard_break_offsets()
                            .iter()
                            .map(|offset| *offset as u64),
                    );
                    registers.push(EvimExRegisterV1 {
                        struct_size: EVIM_EX_REGISTER_V1_SIZE,
                        name: u32::from(register.name),
                        register_kind: register_kind_to_ffi(register.value.kind),
                        reserved: 0,
                        text: push_effect_text(strings, &register.value.text)?,
                        first_hard_break,
                        hard_break_count: checked_export_count(hard_breaks.len())?
                            .checked_sub(first_hard_break)
                            .ok_or(EvimStatus::LengthOverflow)?,
                    });
                }
                output.payload_count = checked_export_count(registers.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(EvimStatus::LengthOverflow)?;
            }
            ExInfoRequest::Jumps => {
                output.kind = EVIM_EX_FRONTEND_JUMPS;
                let Some(OwnedExInfoPayload::Jumps(resolved)) = payload else {
                    return Err(EvimStatus::InternalError);
                };
                output.first_payload = checked_export_count(jumps.len())?;
                for jump in resolved {
                    jumps.push(EvimExJumpV1 {
                        struct_size: EVIM_EX_JUMP_V1_SIZE,
                        flags: u32::from(jump.current) * EVIM_EX_JUMP_CURRENT,
                        list_index: checked_export_count(jump.list_index)?,
                        utf8_offset: checked_export_count(jump.point.utf8_offset)?,
                        hard_line_index: checked_export_count(jump.point.hard_line_index)?,
                        hard_line_start: checked_export_count(jump.point.hard_line_range.start)?,
                        hard_line_end: checked_export_count(jump.point.hard_line_range.end)?,
                        grapheme_column: checked_export_count(jump.point.grapheme_column)?,
                        line_text: push_effect_text(strings, &jump.point.line_text)?,
                    });
                }
                output.payload_count = checked_export_count(jumps.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(EvimStatus::LengthOverflow)?;
            }
            ExInfoRequest::Options(displays) => {
                output.kind = EVIM_EX_FRONTEND_OPTIONS;
                output.first_option = checked_export_count(options.len())?;
                for display in displays {
                    options.push(export_ex_option(display, file_formats)?);
                }
                output.option_count = checked_export_count(options.len())?
                    .checked_sub(output.first_option)
                    .ok_or(EvimStatus::LengthOverflow)?;
            }
            ExInfoRequest::PrintLines {
                range,
                number,
                list,
            } => {
                output.kind = EVIM_EX_FRONTEND_PRINT_LINES;
                output.flags |= u32::from(*number) * EVIM_EX_FRONTEND_NUMBER;
                output.flags |= u32::from(*list) * EVIM_EX_FRONTEND_LIST;
                set_ex_range(&mut output, Some(*range));
                let Some(OwnedExInfoPayload::TextLines(resolved)) = payload else {
                    return Err(EvimStatus::InternalError);
                };
                output.first_payload = checked_export_count(text_lines.len())?;
                for line in resolved {
                    text_lines.push(EvimExTextLineV1 {
                        struct_size: EVIM_EX_TEXT_LINE_V1_SIZE,
                        reserved: 0,
                        hard_line_index: checked_export_count(line.hard_line_index)?,
                        utf8_start: checked_export_count(line.utf8_range.start)?,
                        utf8_end: checked_export_count(line.utf8_range.end)?,
                        text: push_effect_text(strings, &line.text)?,
                    });
                }
                output.payload_count = checked_export_count(text_lines.len())?
                    .checked_sub(output.first_payload)
                    .ok_or(EvimStatus::LengthOverflow)?;
            }
        },
        ExFrontendRequest::Normal(request) => {
            output.kind = EVIM_EX_FRONTEND_NORMAL;
            output.flags |= u32::from(request.literal) * EVIM_EX_FRONTEND_LITERAL;
            output.text = push_effect_text(strings, &request.commands)?;
            set_ex_range(&mut output, Some(request.range));
        }
    }
    Ok(output)
}

fn export_effect_batch(
    handle: EvimEffectBatchHandle,
    batch: &OwnedEffectBatch,
) -> Result<EffectBatchExport, EvimStatus> {
    let mut flags = 0;
    let mut navigation_utf8_offset = 0;
    let mut substitution_count = 0;
    let mut clipboard_writes = Vec::with_capacity(batch.clipboard_writes.len());
    let mut ex_requests = Vec::new();
    let mut ex_options = Vec::new();
    let mut ex_marks = Vec::new();
    let mut ex_registers = Vec::new();
    let mut ex_jumps = Vec::new();
    let mut ex_text_lines = Vec::new();
    let mut file_formats = Vec::new();
    let mut hard_breaks = Vec::new();
    let mut strings = Vec::new();

    for request in &batch.clipboard_writes {
        let first_hard_break = checked_export_count(hard_breaks.len())?;
        let mut write_flags = 0;
        let mut register_kind = EVIM_REGISTER_KIND_NONE;
        if let Some(portable) = request.content().portable_register() {
            write_flags |= EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER;
            register_kind = register_kind_to_ffi(portable.kind);
            hard_breaks.extend(
                portable
                    .hard_break_offsets()
                    .iter()
                    .map(|offset| *offset as u64),
            );
        }
        clipboard_writes.push(EvimClipboardWriteV1 {
            struct_size: EVIM_CLIPBOARD_WRITE_V1_SIZE,
            flags: write_flags,
            target: clipboard_target_to_ffi(request.target()),
            register_kind,
            document_id: batch.document_id.0,
            document_revision: batch.document_revision.0,
            plain_text: push_effect_text(&mut strings, request.content().plain_text())?,
            first_hard_break,
            hard_break_count: checked_export_count(hard_breaks.len())?
                .checked_sub(first_hard_break)
                .ok_or(EvimStatus::LengthOverflow)?,
        });
    }

    if let Some(ex) = &batch.ex_outcome {
        flags |= EVIM_EFFECT_BATCH_HAS_EX_OUTCOME;
        if ex.document_changed {
            flags |= EVIM_EFFECT_BATCH_EX_DOCUMENT_CHANGED;
        }
        if let Some(navigation) = ex.navigation {
            flags |= EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION;
            match navigation {
                ExNavigation::TextOffset(offset) => navigation_utf8_offset = offset as u64,
                ExNavigation::HistoryRestoration => {
                    flags |= EVIM_EFFECT_BATCH_EX_NAVIGATION_HISTORY;
                }
            }
        }
        substitution_count = ex.substitutions as u64;
        ex_requests.reserve(ex.frontend_requests.len());
        for (index, request) in ex.frontend_requests.iter().enumerate() {
            ex_requests.push(export_ex_frontend_request(
                request,
                batch.ex_info_payloads.get(index).and_then(Option::as_ref),
                batch.document_id,
                batch.document_revision,
                &mut strings,
                &mut ex_options,
                &mut ex_marks,
                &mut ex_registers,
                &mut ex_jumps,
                &mut ex_text_lines,
                &mut file_formats,
                &mut hard_breaks,
            )?);
        }
    }

    Ok(EffectBatchExport {
        info: EvimEffectBatchInfoV1 {
            struct_size: EVIM_EFFECT_BATCH_INFO_V1_SIZE,
            flags,
            batch_handle: handle,
            document_id: batch.document_id.0,
            document_revision: batch.document_revision.0,
            clipboard_write_count: checked_export_count(clipboard_writes.len())?,
            ex_request_count: checked_export_count(ex_requests.len())?,
            ex_option_count: checked_export_count(ex_options.len())?,
            ex_mark_count: checked_export_count(ex_marks.len())?,
            ex_register_count: checked_export_count(ex_registers.len())?,
            ex_jump_count: checked_export_count(ex_jumps.len())?,
            ex_text_line_count: checked_export_count(ex_text_lines.len())?,
            file_format_count: checked_export_count(file_formats.len())?,
            hard_break_count: checked_export_count(hard_breaks.len())?,
            string_bytes: checked_export_count(strings.len())?,
            navigation_utf8_offset,
            substitution_count,
        },
        clipboard_writes,
        ex_requests,
        ex_options,
        ex_marks,
        ex_registers,
        ex_jumps,
        ex_text_lines,
        file_formats,
        hard_breaks,
        strings,
    })
}

enum EffectBatchRegistryEntry {
    Reserved,
    Ready(Arc<OwnedEffectBatch>),
}

struct EffectBatchRegistry {
    next_handle: EvimEffectBatchHandle,
    batches: HashMap<EvimEffectBatchHandle, EffectBatchRegistryEntry>,
}

impl EffectBatchRegistry {
    fn new() -> Self {
        Self {
            next_handle: 1,
            batches: HashMap::new(),
        }
    }
}

static EFFECT_BATCHES: OnceLock<Mutex<EffectBatchRegistry>> = OnceLock::new();

fn effect_batch_registry() -> &'static Mutex<EffectBatchRegistry> {
    EFFECT_BATCHES.get_or_init(|| Mutex::new(EffectBatchRegistry::new()))
}

struct EffectBatchReservation {
    handle: EvimEffectBatchHandle,
    committed: bool,
}

impl EffectBatchReservation {
    fn commit(mut self, batch: OwnedEffectBatch) -> Result<EvimEffectBatchHandle, EvimStatus> {
        let mut registry = effect_batch_registry()
            .lock()
            .map_err(|_| EvimStatus::InternalError)?;
        let entry = registry
            .batches
            .get_mut(&self.handle)
            .ok_or(EvimStatus::InternalError)?;
        if !matches!(entry, EffectBatchRegistryEntry::Reserved) {
            return Err(EvimStatus::InternalError);
        }
        *entry = EffectBatchRegistryEntry::Ready(Arc::new(batch));
        self.committed = true;
        Ok(self.handle)
    }
}

impl Drop for EffectBatchReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Ok(mut registry) = effect_batch_registry().lock() {
            if matches!(
                registry.batches.get(&self.handle),
                Some(EffectBatchRegistryEntry::Reserved)
            ) {
                registry.batches.remove(&self.handle);
            }
        }
    }
}

fn reserve_effect_batch() -> Result<EffectBatchReservation, EvimStatus> {
    let mut registry = effect_batch_registry()
        .lock()
        .map_err(|_| EvimStatus::InternalError)?;
    let handle = registry.next_handle;
    if handle == 0 || registry.batches.contains_key(&handle) {
        return Err(EvimStatus::ResourceExhausted);
    }
    registry.next_handle = handle.checked_add(1).unwrap_or(0);
    registry
        .batches
        .insert(handle, EffectBatchRegistryEntry::Reserved);
    Ok(EffectBatchReservation {
        handle,
        committed: false,
    })
}

fn owned_effect_batch(handle: EvimEffectBatchHandle) -> Result<Arc<OwnedEffectBatch>, EvimStatus> {
    if handle == 0 {
        return Err(EvimStatus::InvalidHandle);
    }
    let registry = effect_batch_registry()
        .lock()
        .map_err(|_| EvimStatus::InternalError)?;
    match registry.batches.get(&handle) {
        Some(EffectBatchRegistryEntry::Ready(batch)) => Ok(batch.clone()),
        Some(EffectBatchRegistryEntry::Reserved) | None => Err(EvimStatus::InvalidHandle),
    }
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
        // SAFETY: Forwarded directly from this function's pointer contracts.
        let bytes = unsafe { input_bytes(source, source_length)? }.to_vec();
        let document = create_document_from_source(bytes, options)?;
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
    match (encoding, file_format) {
        (Some(encoding), Some(file_format)) => {
            Document::from_bytes_with_file_format(bytes, encoding, format, file_format)
        }
        (Some(encoding), None) => Document::from_bytes(bytes, encoding, format),
        (None, Some(file_format)) => {
            Document::from_bytes_detect_encoding_with_file_format(bytes, format, file_format)
        }
        (None, None) => Document::from_bytes_detect_encoding(bytes, format),
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
        EVIM_KEY_BACK_TAB => special(Key::BackTab),
        EVIM_KEY_BACKSPACE => special(Key::Backspace),
        EVIM_KEY_DELETE => special(Key::Delete),
        EVIM_KEY_LEFT => special(Key::Left),
        EVIM_KEY_RIGHT => special(Key::Right),
        EVIM_KEY_UP => special(Key::Up),
        EVIM_KEY_DOWN => special(Key::Down),
        EVIM_KEY_HOME => special(Key::Home),
        EVIM_KEY_END => special(Key::End),
        EVIM_KEY_DOCUMENT_START => special(Key::DocumentStart),
        EVIM_KEY_DOCUMENT_END => special(Key::DocumentEnd),
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
        CommandStatus::ExError(crate::command::ExCommandError::Execute(
            crate::command::ex_execute::ExExecuteError::ReadOnly,
        )) => EVIM_COMMAND_STATUS_READ_ONLY,
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

fn style_error_status(error: StyleError) -> EvimStatus {
    match error {
        StyleError::UnknownStyle(_) => EvimStatus::UnknownStyle,
        StyleError::DefinitionNotGeneratedConfiguration { .. } => EvimStatus::StyleReadOnly,
        StyleError::InheritanceCycle(_) => EvimStatus::StyleInheritanceCycle,
        StyleError::IncompatibleBlockRole { .. }
        | StyleError::InapplicableBlockProperties { .. }
        | StyleError::InapplicableStyleProperty { .. } => EvimStatus::IncompatibleStyleRole,
        StyleError::MissingParent(_)
        | StyleError::InapplicableNextParagraphStyle { .. }
        | StyleError::InvalidNextParagraphStyle { .. }
        | StyleError::InapplicableStyleRelationship(_)
        | StyleError::CannotReplaceBaseStyle(_)
        | StyleError::CannotRemoveBaseStyle(_)
        | StyleError::InvalidBaseStyleDefinition(_)
        | StyleError::StyleInUse(_) => EvimStatus::InvalidStyleRelationship,
        StyleError::InvalidCharacterProperties(_)
        | StyleError::InvalidBlockProperties(_)
        | StyleError::InvalidStylePropertyValue { .. }
        | StyleError::InvalidDefinitionMetadata(_) => EvimStatus::InvalidStyleValue,
        StyleError::StyleSheetRevisionExhausted => EvimStatus::ResourceExhausted,
        StyleError::StyleAlreadyExists(_) => EvimStatus::InvalidArgument,
    }
}

fn style_transaction_status(error: StyleTransactionError) -> EvimStatus {
    match error {
        StyleTransactionError::Definition(error) => style_error_status(error),
        StyleTransactionError::Unsupported { .. }
        | StyleTransactionError::TranslationUnavailable => EvimStatus::UnsupportedOperation,
        StyleTransactionError::NeedsPolicy(_) => EvimStatus::PolicyRequired,
        StyleTransactionError::DefinitionReadOnly(_) => EvimStatus::StyleReadOnly,
        StyleTransactionError::ConfigurationIntentRequired(_)
        | StyleTransactionError::InvalidPropertyTarget { .. } => EvimStatus::InvalidStyleValue,
    }
}

fn model_transaction_status(error: ModelTransactionError) -> EvimStatus {
    match error {
        ModelTransactionError::Document(error) => document_status(error),
        ModelTransactionError::Style(error) => style_transaction_status(error),
        ModelTransactionError::WrongDocument { .. } => EvimStatus::InvalidArgument,
        ModelTransactionError::StaleRevision { .. } | ModelTransactionError::StaleDocumentState => {
            EvimStatus::StaleRevision
        }
        ModelTransactionError::RevisionExhausted => EvimStatus::ResourceExhausted,
        ModelTransactionError::Position(_) | ModelTransactionError::History(_) => {
            EvimStatus::CoreFailure
        }
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
        CoreError::ModelTransaction(error) => model_transaction_status(error),
        CoreError::StaleStyleSheet { .. } => EvimStatus::StaleRevision,
        CoreError::StyleEditGroup(error) => match error {
            StyleEditGroupError::AlreadyActive(_) => EvimStatus::StyleEditGroupActive,
            StyleEditGroupError::WrongOwner { .. } => EvimStatus::StyleEditGroupWrongOwner,
            StyleEditGroupError::WrongDocument { .. } => EvimStatus::InvalidArgument,
            StyleEditGroupError::NoActiveGroup
            | StyleEditGroupError::WrongGroup { .. }
            | StyleEditGroupError::IdentityMismatch => EvimStatus::InvalidStyleEditGroup,
        },
        CoreError::Composition(error) => composition_status(error),
        CoreError::Layout(error) => layout_status(error),
        CoreError::LayoutJob(LayoutJobError::Layout(error)) => layout_status(error),
        CoreError::LayoutJob(
            LayoutJobError::ProviderThreadingChanged { .. }
            | LayoutJobError::WrongMeasurementEnvironment { .. }
            | LayoutJobError::StaleMetrics { .. },
        ) => EvimStatus::LayoutUnavailable,
        CoreError::Persistence(crate::document::PersistenceError::ReadOnly) => {
            EvimStatus::PolicyRequired
        }
        CoreError::NoVisualSelection => EvimStatus::InvalidRange,
        CoreError::StaleLogicalSelection => EvimStatus::StaleRevision,
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
    let layout = core
        .presentation_layout(view_id)
        .ok_or(EvimStatus::InvalidView)?;
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
    // Reserved compatibility flag: wrapping always uses Unicode word boundaries.
    flags |= EVIM_VIEWPORT_STATE_LINEBREAK;
    let maximum_left = if let Some(maximum_left) = state.maximum_left() {
        flags |= EVIM_VIEWPORT_STATE_MAXIMUM_LEFT_EXACT;
        maximum_left
    } else {
        state.estimated_maximum_left()
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
        scale: state.scale(),
        document_id: state.document_id().0,
        document_revision: state.document_revision().0,
        layout_revision,
        configuration_generation: state.configuration_generation().0,
        measurement_environment_id: state.measurement_environment_id().0,
        metrics_generation: state.metrics_generation().0,
    })
}

fn summarize_document_state(document: &Document) -> EvimDocumentStateV1 {
    let history = document.history_status();
    let mut flags = 0;
    if document.has_bom() {
        flags |= EVIM_DOCUMENT_STATE_HAS_BOM;
    }
    if history.can_undo {
        flags |= EVIM_DOCUMENT_STATE_CAN_UNDO;
    }
    if history.can_redo {
        flags |= EVIM_DOCUMENT_STATE_CAN_REDO;
    }
    if history.is_dirty {
        flags |= EVIM_DOCUMENT_STATE_IS_DIRTY;
    }
    if document.is_read_only() {
        flags |= EVIM_DOCUMENT_STATE_READ_ONLY;
    }
    if document.is_recovered() {
        flags |= EVIM_DOCUMENT_STATE_RECOVERED;
    }
    if document.include_style_definitions_in_file() {
        flags |= EVIM_DOCUMENT_STATE_INCLUDE_STYLE_DEFINITIONS;
    }
    EvimDocumentStateV1 {
        struct_size: EVIM_DOCUMENT_STATE_V1_SIZE,
        flags,
        document_id: document.id().0,
        document_revision: document.revision().0,
        style_sheet_revision: document.projection().style_sheet().revision.0,
        encoding: encoding_to_ffi(document.encoding()),
        format: format_to_ffi(document.format()),
        file_format: file_format_to_ffi(document.file_format()),
        file_format_origin: file_format_origin_to_ffi(document.file_format_origin()),
        undo_action_category: history_action_category(history.undo_summary.as_ref()),
        redo_action_category: history_action_category(history.redo_summary.as_ref()),
        reserved: [0; 2],
    }
}

fn current_ffi_layout_snapshot(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<&LayoutSnapshot, EvimStatus> {
    let layout = core
        .presentation_layout(view_id)
        .ok_or(EvimStatus::InvalidView)?;
    let requirements = core
        .layout_provider_requirements(view_id)
        .map_err(core_status)?;
    layout
        .snapshot()
        .filter(|snapshot| {
            snapshot.document_id == core.document().id()
                && snapshot.document_revision == core.document().revision()
                && snapshot.configuration_generation == layout.configuration_generation()
                && snapshot.measurement_environment_id == requirements.measurement_environment_id
                && snapshot.metrics_generation == requirements.metrics_generation
        })
        .ok_or(EvimStatus::LayoutUnavailable)
}

fn composition_overlay_identity(
    overlay: &crate::command::composition::CompositionOverlay,
    view_id: ViewId,
) -> EvimCompositionOverlayIdentityV1 {
    EvimCompositionOverlayIdentityV1 {
        struct_size: EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE,
        reserved: 0,
        view_id: view_id.0,
        document_id: overlay.document_id().0,
        document_revision: overlay.revision().0,
        generation: overlay.generation(),
    }
}

fn composition_overlay_info(
    overlay: &crate::command::composition::CompositionOverlay,
    view_id: ViewId,
) -> Result<EvimCompositionOverlayInfoV1, EvimStatus> {
    let replacement = overlay.replacement_range();
    let marked = overlay.marked_range();
    let selected = overlay.selected_range_in_overlay();
    Ok(EvimCompositionOverlayInfoV1 {
        struct_size: EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE,
        flags: EVIM_COMPOSITION_OVERLAY_ACTIVE,
        identity: composition_overlay_identity(overlay, view_id),
        utf8_length: checked_export_count(overlay.utf8_len())?,
        replacement_start: checked_export_count(replacement.start)?,
        replacement_end: checked_export_count(replacement.end)?,
        marked_start: checked_export_count(marked.start)?,
        marked_end: checked_export_count(marked.end)?,
        selected_start: checked_export_count(selected.start)?,
        selected_end: checked_export_count(selected.end)?,
    })
}

fn validate_composition_overlay_identity(
    expected: EvimCompositionOverlayIdentityV1,
    overlay: &crate::command::composition::CompositionOverlay,
    view_id: ViewId,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let actual = composition_overlay_identity(overlay, view_id);
    (expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.generation == actual.generation)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn snapshot_identity(snapshot: &LayoutSnapshot, view_id: ViewId) -> EvimLayoutSnapshotIdentityV1 {
    EvimLayoutSnapshotIdentityV1 {
        struct_size: EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE,
        reserved: 0,
        view_id: view_id.0,
        document_id: snapshot.document_id.0,
        document_revision: snapshot.document_revision.0,
        layout_revision: snapshot.revision.0,
        configuration_generation: snapshot.configuration_generation.0,
        measurement_environment_id: snapshot.measurement_environment_id.0,
        metrics_generation: snapshot.metrics_generation.0,
    }
}

fn checked_export_count(value: usize) -> Result<u64, EvimStatus> {
    u64::try_from(value).map_err(|_| EvimStatus::LengthOverflow)
}

fn layout_snapshot_info(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<EvimLayoutSnapshotInfoV1, EvimStatus> {
    let mut flags = 0;
    if snapshot.coverage.is_full_document() {
        flags |= EVIM_LAYOUT_SNAPSHOT_FULL_DOCUMENT;
    }
    if snapshot.coverage.prefix_is_exact() {
        flags |= EVIM_LAYOUT_SNAPSHOT_PREFIX_EXACT;
    }
    if snapshot.content_width_is_exact {
        flags |= EVIM_LAYOUT_SNAPSHOT_CONTENT_WIDTH_EXACT;
    }
    if snapshot.total_height_is_exact {
        flags |= EVIM_LAYOUT_SNAPSHOT_TOTAL_HEIGHT_EXACT;
    }
    let hard_lines = snapshot.coverage.hard_lines();
    let vertical = snapshot
        .coverage
        .vertical_range()
        .unwrap_or(0.0..snapshot.total_height);
    let cluster_count = snapshot
        .rows
        .iter()
        .try_fold(0usize, |total, row| total.checked_add(row.clusters.len()));
    let caret_count = snapshot
        .rows
        .iter()
        .try_fold(0usize, |total, row| total.checked_add(row.carets.len()));
    Ok(EvimLayoutSnapshotInfoV1 {
        struct_size: EVIM_LAYOUT_SNAPSHOT_INFO_V1_SIZE,
        flags,
        identity: snapshot_identity(snapshot, view_id),
        viewport_width: snapshot.viewport_width,
        viewport_height: snapshot.viewport_height,
        usable_width: snapshot.usable_width,
        content_width: snapshot.content_width,
        total_height: snapshot.total_height,
        content_insets: EvimLayoutInsetsV1 {
            top: snapshot.content_insets.top,
            left: snapshot.content_insets.left,
            bottom: snapshot.content_insets.bottom,
            right: snapshot.content_insets.right,
        },
        coverage_hard_line_start: checked_export_count(hard_lines.start)?,
        coverage_hard_line_end: checked_export_count(hard_lines.end)?,
        document_hard_line_count: checked_export_count(
            snapshot.coverage.document_hard_line_count(),
        )?,
        coverage_y_start: vertical.start,
        coverage_y_end: vertical.end,
        row_count: checked_export_count(snapshot.rows.len())?,
        cluster_count: checked_export_count(cluster_count.ok_or(EvimStatus::LengthOverflow)?)?,
        caret_count: checked_export_count(caret_count.ok_or(EvimStatus::LengthOverflow)?)?,
    })
}

fn validate_snapshot_identity(
    expected: EvimLayoutSnapshotIdentityV1,
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let actual = snapshot_identity(snapshot, view_id);
    (expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.layout_revision == actual.layout_revision
        && expected.configuration_generation == actual.configuration_generation
        && expected.measurement_environment_id == actual.measurement_environment_id
        && expected.metrics_generation == actual.metrics_generation)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn validate_viewport_origin_identity(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
    request: EvimViewportOriginV1,
) -> Result<(), EvimStatus> {
    let snapshot = current_ffi_layout_snapshot(core, view_id)?;
    (request.expected_document_id == snapshot.document_id.0
        && request.expected_document_revision == snapshot.document_revision.0
        && request.expected_layout_revision == snapshot.revision.0
        && request.expected_configuration_generation == snapshot.configuration_generation.0
        && request.expected_measurement_environment_id == snapshot.measurement_environment_id.0
        && request.expected_metrics_generation == snapshot.metrics_generation.0)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn affinity_to_ffi(affinity: BoundaryAffinity) -> u32 {
    match affinity {
        BoundaryAffinity::Upstream => EVIM_BOUNDARY_AFFINITY_UPSTREAM,
        BoundaryAffinity::Downstream => EVIM_BOUNDARY_AFFINITY_DOWNSTREAM,
    }
}

fn parse_layout_affinity(raw: u32) -> Result<BoundaryAffinity, EvimStatus> {
    match raw {
        EVIM_BOUNDARY_AFFINITY_UPSTREAM => Ok(BoundaryAffinity::Upstream),
        EVIM_BOUNDARY_AFFINITY_DOWNSTREAM => Ok(BoundaryAffinity::Downstream),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

fn layout_rect_to_ffi(rect: LayoutRect) -> EvimLayoutRectV1 {
    EvimLayoutRectV1 {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn color_to_ffi(color: Color) -> EvimRgbaV1 {
    EvimRgbaV1 {
        red: color.red,
        green: color.green,
        blue: color.blue,
        alpha: color.alpha,
    }
}

fn text_paint_to_ffi(paint: &ResolvedTextPaint) -> EvimTextPaintV1 {
    let mut flags = if paint.foreground_is_default {
        EVIM_TEXT_PAINT_DEFAULT_FOREGROUND
    } else {
        0
    };
    let background = paint.background.map_or_else(EvimRgbaV1::default, |color| {
        flags |= EVIM_TEXT_PAINT_HAS_BACKGROUND;
        color_to_ffi(color)
    });
    if paint.underline {
        flags |= EVIM_TEXT_PAINT_UNDERLINE;
    }
    if paint.strikethrough {
        flags |= EVIM_TEXT_PAINT_STRIKETHROUGH;
    }
    EvimTextPaintV1 {
        struct_size: EVIM_TEXT_PAINT_V1_SIZE,
        flags,
        foreground: color_to_ffi(paint.foreground),
        background,
    }
}

fn render_threading_to_ffi(threading: RenderRunThreading) -> u32 {
    match threading {
        RenderRunThreading::AnyThread => EVIM_RENDER_THREADING_ANY,
        RenderRunThreading::DedicatedSerialExecutor => EVIM_RENDER_THREADING_DEDICATED_SERIAL,
        RenderRunThreading::FrontendMainThread => EVIM_RENDER_THREADING_FRONTEND_MAIN,
    }
}

fn render_run_to_ffi(render_run: RenderRunHandle) -> EvimRenderRunHandleV1 {
    EvimRenderRunHandleV1 {
        owner: render_run.owner.0,
        identifier: render_run.identifier,
        metrics_generation: render_run.metrics_generation.0,
        threading: render_threading_to_ffi(render_run.threading),
        reserved: 0,
    }
}

fn caret_point_to_ffi(point: CaretPoint) -> Result<EvimLayoutCaretPointV1, EvimStatus> {
    Ok(EvimLayoutCaretPointV1 {
        struct_size: EVIM_LAYOUT_CARET_POINT_V1_SIZE,
        affinity: affinity_to_ffi(point.affinity),
        document_id: point.document_id.0,
        document_revision: point.document_revision.0,
        layout_revision: point.layout_revision.0,
        text_offset: checked_export_count(point.text_offset)?,
    })
}

fn layout_query_status(error: LayoutError) -> EvimStatus {
    match error {
        LayoutError::InvalidGeometry => EvimStatus::InvalidArgument,
        LayoutError::InvalidTextOffset(_) => EvimStatus::InvalidRange,
        LayoutError::InvalidGraphemeBoundary { .. } => EvimStatus::NotGraphemeBoundary,
        LayoutError::WrongDocument
        | LayoutError::WrongDocumentRevision
        | LayoutError::StaleLayout { .. } => EvimStatus::StaleRevision,
        LayoutError::OutsideMaterializedCoverage
        | LayoutError::NoRows
        | LayoutError::NoCaretStops
        | LayoutError::NotACaretStop { .. }
        | LayoutError::LongLineSliceNeedsMoreText { .. } => EvimStatus::OutsideLayoutCoverage,
        other => layout_status(other),
    }
}

struct LayoutSnapshotExport {
    info: EvimLayoutSnapshotInfoV1,
    rows: Vec<EvimVisualRowV1>,
    clusters: Vec<EvimPositionedClusterV1>,
    carets: Vec<EvimPositionedCaretV1>,
}

struct LayoutPaintExport {
    info: EvimLayoutPaintInfoV1,
    runs: Vec<EvimPaintStyleRunV1>,
}

struct StyleSheetExport {
    info: EvimStyleSheetInfoV1,
    definitions: Vec<EvimStyleDefinitionV1>,
    properties: Vec<EvimStylePropertyV1>,
    value_items: Vec<EvimStyleValueItemV1>,
    dependencies: Vec<EvimStyleDependencyV1>,
    strings: Vec<u8>,
}

const FFI_CANVAS_PROPERTIES: [StyleProperty; 5] = [
    StyleProperty::CanvasBackground,
    StyleProperty::CanvasPaddingTop,
    StyleProperty::CanvasPaddingRight,
    StyleProperty::CanvasPaddingBottom,
    StyleProperty::CanvasPaddingLeft,
];

const FFI_PARAGRAPH_PROPERTIES: [StyleProperty; 8] = [
    StyleProperty::ParagraphSpacingBefore,
    StyleProperty::ParagraphSpacingAfter,
    StyleProperty::ParagraphLineSpacing,
    StyleProperty::ParagraphFirstLineIndent,
    StyleProperty::ParagraphLeadingIndent,
    StyleProperty::ParagraphTrailingIndent,
    StyleProperty::ParagraphAlignment,
    StyleProperty::ParagraphBaseDirection,
];

const FFI_CHARACTER_PROPERTIES: [StyleProperty; 14] = [
    StyleProperty::CharacterFontFamilies,
    StyleProperty::CharacterSize,
    StyleProperty::CharacterWeight,
    StyleProperty::CharacterBold,
    StyleProperty::CharacterSlant,
    StyleProperty::CharacterForeground,
    StyleProperty::CharacterBackground,
    StyleProperty::CharacterUnderline,
    StyleProperty::CharacterStrikethrough,
    StyleProperty::CharacterLanguage,
    StyleProperty::CharacterDirection,
    StyleProperty::CharacterOpenTypeFeatures,
    StyleProperty::CharacterLetterSpacing,
    StyleProperty::CharacterBaselineShift,
];

fn style_sheet_identity(document: &Document) -> EvimStyleSheetIdentityV1 {
    EvimStyleSheetIdentityV1 {
        struct_size: EVIM_STYLE_SHEET_IDENTITY_V1_SIZE,
        reserved: 0,
        document_id: document.id().0,
        document_revision: document.revision().0,
        style_sheet_revision: document.projection().style_sheet().revision.0,
    }
}

fn validate_style_sheet_identity(
    expected: EvimStyleSheetIdentityV1,
    document: &Document,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_STYLE_SHEET_IDENTITY_V1_SIZE || expected.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let actual = style_sheet_identity(document);
    (expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.style_sheet_revision == actual.style_sheet_revision)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn style_role_to_ffi(role: BlockRole) -> u32 {
    match role {
        BlockRole::Document => EVIM_STYLE_ROLE_DOCUMENT,
        BlockRole::Paragraph => EVIM_STYLE_ROLE_PARAGRAPH,
    }
}

fn style_origin_to_ffi(origin: StyleDefinitionOrigin) -> u32 {
    match origin {
        StyleDefinitionOrigin::SourceBacked => EVIM_STYLE_ORIGIN_SOURCE_BACKED,
        StyleDefinitionOrigin::GeneratedConfiguration => EVIM_STYLE_ORIGIN_GENERATED_CONFIGURATION,
        StyleDefinitionOrigin::SyntheticReadOnly => EVIM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY,
    }
}

fn style_property_to_ffi(property: StyleProperty) -> u32 {
    match property {
        StyleProperty::CanvasBackground => EVIM_STYLE_PROPERTY_CANVAS_BACKGROUND,
        StyleProperty::CanvasPaddingTop => EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
        StyleProperty::CanvasPaddingRight => EVIM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT,
        StyleProperty::CanvasPaddingBottom => EVIM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM,
        StyleProperty::CanvasPaddingLeft => EVIM_STYLE_PROPERTY_CANVAS_PADDING_LEFT,
        StyleProperty::ParagraphSpacingBefore => EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE,
        StyleProperty::ParagraphSpacingAfter => EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_AFTER,
        StyleProperty::ParagraphLineSpacing => EVIM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING,
        StyleProperty::ParagraphFirstLineIndent => EVIM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT,
        StyleProperty::ParagraphLeadingIndent => EVIM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT,
        StyleProperty::ParagraphTrailingIndent => EVIM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT,
        StyleProperty::ParagraphAlignment => EVIM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT,
        StyleProperty::ParagraphBaseDirection => EVIM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION,
        StyleProperty::CharacterFontFamilies => EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES,
        StyleProperty::CharacterSize => EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
        StyleProperty::CharacterWeight => EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT,
        StyleProperty::CharacterBold => EVIM_STYLE_PROPERTY_CHARACTER_BOLD,
        StyleProperty::CharacterSlant => EVIM_STYLE_PROPERTY_CHARACTER_SLANT,
        StyleProperty::CharacterForeground => EVIM_STYLE_PROPERTY_CHARACTER_FOREGROUND,
        StyleProperty::CharacterBackground => EVIM_STYLE_PROPERTY_CHARACTER_BACKGROUND,
        StyleProperty::CharacterUnderline => EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
        StyleProperty::CharacterStrikethrough => EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH,
        StyleProperty::CharacterLanguage => EVIM_STYLE_PROPERTY_CHARACTER_LANGUAGE,
        StyleProperty::CharacterDirection => EVIM_STYLE_PROPERTY_CHARACTER_DIRECTION,
        StyleProperty::CharacterOpenTypeFeatures => {
            EVIM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES
        }
        StyleProperty::CharacterLetterSpacing => EVIM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING,
        StyleProperty::CharacterBaselineShift => EVIM_STYLE_PROPERTY_CHARACTER_BASELINE_SHIFT,
    }
}

fn push_style_string(
    strings: &mut Vec<u8>,
    value: &str,
) -> Result<EvimStyleStringRefV1, EvimStatus> {
    let offset = checked_export_count(strings.len())?;
    let length = checked_export_count(value.len())?;
    strings
        .try_reserve(value.len())
        .map_err(|_| EvimStatus::ResourceExhausted)?;
    strings.extend_from_slice(value.as_bytes());
    Ok(EvimStyleStringRefV1 { offset, length })
}

fn empty_style_value() -> EvimStyleValueV1 {
    EvimStyleValueV1 {
        struct_size: EVIM_STYLE_VALUE_V1_SIZE,
        ..EvimStyleValueV1::default()
    }
}

fn style_value_to_ffi(
    value: &StylePropertyValue,
    items: &mut Vec<EvimStyleValueItemV1>,
    strings: &mut Vec<u8>,
) -> Result<EvimStyleValueV1, EvimStatus> {
    let mut output = empty_style_value();
    match value {
        StylePropertyValue::Float(value) => {
            output.kind = EVIM_STYLE_VALUE_FLOAT;
            output.number = *value;
        }
        StylePropertyValue::FontWeight(value) => {
            output.kind = EVIM_STYLE_VALUE_UNSIGNED;
            output.enum_value = u32::from(*value);
        }
        StylePropertyValue::Boolean(value) => {
            output.kind = EVIM_STYLE_VALUE_BOOLEAN;
            output.enum_value = u32::from(*value);
        }
        StylePropertyValue::Color(value) => {
            output.kind = EVIM_STYLE_VALUE_COLOR;
            output.color = color_to_ffi(*value);
        }
        StylePropertyValue::Text(value) => {
            output.kind = EVIM_STYLE_VALUE_STRING;
            output.string = push_style_string(strings, value)?;
        }
        StylePropertyValue::FontFamilies(values) => {
            output.kind = EVIM_STYLE_VALUE_STRING_LIST;
            output.first_item = checked_export_count(items.len())?;
            items
                .try_reserve(values.len())
                .map_err(|_| EvimStatus::ResourceExhausted)?;
            for value in values {
                items.push(EvimStyleValueItemV1 {
                    struct_size: EVIM_STYLE_VALUE_ITEM_V1_SIZE,
                    kind: EVIM_STYLE_VALUE_ITEM_STRING,
                    string: push_style_string(strings, value)?,
                    unsigned_value: 0,
                    reserved: 0,
                });
            }
            output.item_count = checked_export_count(values.len())?;
        }
        StylePropertyValue::FontSlant(value) => {
            output.kind = EVIM_STYLE_VALUE_FONT_SLANT;
            output.enum_value = match value {
                FontSlant::Upright => EVIM_FONT_SLANT_UPRIGHT,
                FontSlant::Italic => EVIM_FONT_SLANT_ITALIC,
                FontSlant::Oblique => EVIM_FONT_SLANT_OBLIQUE,
            };
        }
        StylePropertyValue::WritingDirection(value) => {
            output.kind = EVIM_STYLE_VALUE_WRITING_DIRECTION;
            output.enum_value = match value {
                WritingDirection::Natural => EVIM_TEXT_DIRECTION_AUTO,
                WritingDirection::LeftToRight => EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT,
                WritingDirection::RightToLeft => EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT,
            };
        }
        StylePropertyValue::OpenTypeFeatures(values) => {
            output.kind = EVIM_STYLE_VALUE_OPEN_TYPE_FEATURES;
            output.first_item = checked_export_count(items.len())?;
            items
                .try_reserve(values.len())
                .map_err(|_| EvimStatus::ResourceExhausted)?;
            for (tag, value) in values {
                items.push(EvimStyleValueItemV1 {
                    struct_size: EVIM_STYLE_VALUE_ITEM_V1_SIZE,
                    kind: EVIM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE,
                    string: push_style_string(strings, tag)?,
                    unsigned_value: *value,
                    reserved: 0,
                });
            }
            output.item_count = checked_export_count(values.len())?;
        }
        StylePropertyValue::LineSpacing(value) => {
            output.kind = EVIM_STYLE_VALUE_LINE_SPACING;
            match value {
                LineSpacing::Normal => output.enum_value = EVIM_STYLE_LINE_SPACING_NORMAL,
                LineSpacing::Multiplier(value) => {
                    output.enum_value = EVIM_STYLE_LINE_SPACING_MULTIPLIER;
                    output.number = *value;
                }
                LineSpacing::AtLeast(value) => {
                    output.enum_value = EVIM_STYLE_LINE_SPACING_AT_LEAST;
                    output.number = *value;
                }
                LineSpacing::Exact(value) => {
                    output.enum_value = EVIM_STYLE_LINE_SPACING_EXACT;
                    output.number = *value;
                }
            }
        }
        StylePropertyValue::ParagraphAlignment(value) => {
            output.kind = EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT;
            output.enum_value = match value {
                ParagraphAlignment::Start => EVIM_STYLE_PARAGRAPH_ALIGNMENT_START,
                ParagraphAlignment::End => EVIM_STYLE_PARAGRAPH_ALIGNMENT_END,
                ParagraphAlignment::Center => EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER,
            };
        }
    }
    Ok(output)
}

fn declared_character_property(
    properties: &CharacterProperties,
    property: StyleProperty,
) -> Option<StylePropertyValue> {
    match property {
        StyleProperty::CharacterFontFamilies => properties
            .font_families
            .clone()
            .map(StylePropertyValue::FontFamilies),
        StyleProperty::CharacterSize => properties.size.map(StylePropertyValue::Float),
        StyleProperty::CharacterWeight => properties.weight.map(StylePropertyValue::FontWeight),
        StyleProperty::CharacterBold => properties.bold.map(StylePropertyValue::Boolean),
        StyleProperty::CharacterSlant => properties.slant.map(StylePropertyValue::FontSlant),
        StyleProperty::CharacterForeground => properties.foreground.map(StylePropertyValue::Color),
        StyleProperty::CharacterBackground => properties.background.map(StylePropertyValue::Color),
        StyleProperty::CharacterUnderline => properties.underline.map(StylePropertyValue::Boolean),
        StyleProperty::CharacterStrikethrough => {
            properties.strikethrough.map(StylePropertyValue::Boolean)
        }
        StyleProperty::CharacterLanguage => {
            properties.language.clone().map(StylePropertyValue::Text)
        }
        StyleProperty::CharacterDirection => properties
            .direction
            .map(StylePropertyValue::WritingDirection),
        StyleProperty::CharacterOpenTypeFeatures => properties
            .open_type_features
            .clone()
            .map(StylePropertyValue::OpenTypeFeatures),
        StyleProperty::CharacterLetterSpacing => {
            properties.letter_spacing.map(StylePropertyValue::Float)
        }
        StyleProperty::CharacterBaselineShift => {
            properties.baseline_shift.map(StylePropertyValue::Float)
        }
        _ => None,
    }
}

fn declared_block_property(
    block: &BlockProperties,
    character: &CharacterProperties,
    property: StyleProperty,
) -> Option<StylePropertyValue> {
    declared_character_property(character, property).or_else(|| match property {
        StyleProperty::CanvasBackground => block.background.map(StylePropertyValue::Color),
        StyleProperty::CanvasPaddingTop => block.padding_top.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingRight => block.padding_right.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingBottom => block.padding_bottom.map(StylePropertyValue::Float),
        StyleProperty::CanvasPaddingLeft => block.padding_left.map(StylePropertyValue::Float),
        StyleProperty::ParagraphSpacingBefore => {
            block.spacing_before.map(StylePropertyValue::Float)
        }
        StyleProperty::ParagraphSpacingAfter => block.spacing_after.map(StylePropertyValue::Float),
        StyleProperty::ParagraphLineSpacing => {
            block.line_spacing.map(StylePropertyValue::LineSpacing)
        }
        StyleProperty::ParagraphFirstLineIndent => {
            block.first_line_indent.map(StylePropertyValue::Float)
        }
        StyleProperty::ParagraphLeadingIndent => {
            block.leading_indent.map(StylePropertyValue::Float)
        }
        StyleProperty::ParagraphTrailingIndent => {
            block.trailing_indent.map(StylePropertyValue::Float)
        }
        StyleProperty::ParagraphAlignment => {
            block.alignment.map(StylePropertyValue::ParagraphAlignment)
        }
        StyleProperty::ParagraphBaseDirection => block
            .base_direction
            .map(StylePropertyValue::WritingDirection),
        _ => None,
    })
}

fn effective_character_property(
    properties: &crate::document::ResolvedCharacterStyle,
    property: StyleProperty,
) -> Option<StylePropertyValue> {
    match property {
        StyleProperty::CharacterFontFamilies => Some(StylePropertyValue::FontFamilies(
            properties.font_families.clone(),
        )),
        StyleProperty::CharacterSize => Some(StylePropertyValue::Float(properties.size)),
        StyleProperty::CharacterWeight => {
            Some(StylePropertyValue::FontWeight(properties.base_weight))
        }
        StyleProperty::CharacterBold => Some(StylePropertyValue::Boolean(properties.bold)),
        StyleProperty::CharacterSlant => Some(StylePropertyValue::FontSlant(properties.slant)),
        StyleProperty::CharacterForeground => {
            Some(StylePropertyValue::Color(properties.foreground))
        }
        StyleProperty::CharacterBackground => properties.background.map(StylePropertyValue::Color),
        StyleProperty::CharacterUnderline => {
            Some(StylePropertyValue::Boolean(properties.underline))
        }
        StyleProperty::CharacterStrikethrough => {
            Some(StylePropertyValue::Boolean(properties.strikethrough))
        }
        StyleProperty::CharacterLanguage => {
            properties.language.clone().map(StylePropertyValue::Text)
        }
        StyleProperty::CharacterDirection => {
            Some(StylePropertyValue::WritingDirection(properties.direction))
        }
        StyleProperty::CharacterOpenTypeFeatures => Some(StylePropertyValue::OpenTypeFeatures(
            properties.open_type_features.clone(),
        )),
        StyleProperty::CharacterLetterSpacing => {
            Some(StylePropertyValue::Float(properties.letter_spacing))
        }
        StyleProperty::CharacterBaselineShift => {
            Some(StylePropertyValue::Float(properties.baseline_shift))
        }
        _ => None,
    }
}

fn effective_document_property(
    resolved: &crate::document::ResolvedDocumentStyle,
    property: StyleProperty,
) -> Option<StylePropertyValue> {
    effective_character_property(&resolved.character, property).or(match property {
        StyleProperty::CanvasBackground => Some(StylePropertyValue::Color(resolved.background)),
        StyleProperty::CanvasPaddingTop => Some(StylePropertyValue::Float(resolved.padding_top)),
        StyleProperty::CanvasPaddingRight => {
            Some(StylePropertyValue::Float(resolved.padding_right))
        }
        StyleProperty::CanvasPaddingBottom => {
            Some(StylePropertyValue::Float(resolved.padding_bottom))
        }
        StyleProperty::CanvasPaddingLeft => Some(StylePropertyValue::Float(resolved.padding_left)),
        _ => None,
    })
}

fn effective_paragraph_property(
    resolved: &crate::document::ResolvedParagraphStyle,
    property: StyleProperty,
) -> Option<StylePropertyValue> {
    effective_character_property(&resolved.character, property).or(match property {
        StyleProperty::ParagraphSpacingBefore => {
            Some(StylePropertyValue::Float(resolved.spacing_before))
        }
        StyleProperty::ParagraphSpacingAfter => {
            Some(StylePropertyValue::Float(resolved.spacing_after))
        }
        StyleProperty::ParagraphLineSpacing => {
            Some(StylePropertyValue::LineSpacing(resolved.line_spacing))
        }
        StyleProperty::ParagraphFirstLineIndent => {
            Some(StylePropertyValue::Float(resolved.first_line_indent))
        }
        StyleProperty::ParagraphLeadingIndent => {
            Some(StylePropertyValue::Float(resolved.leading_indent))
        }
        StyleProperty::ParagraphTrailingIndent => {
            Some(StylePropertyValue::Float(resolved.trailing_indent))
        }
        StyleProperty::ParagraphAlignment => {
            Some(StylePropertyValue::ParagraphAlignment(resolved.alignment))
        }
        StyleProperty::ParagraphBaseDirection => Some(StylePropertyValue::WritingDirection(
            resolved.base_direction,
        )),
        _ => None,
    })
}

fn style_contributor_to_ffi(
    origin: &StyleContributionOrigin,
    strings: &mut Vec<u8>,
) -> Result<(u32, u32, EvimStyleStringRefV1), EvimStatus> {
    let none = EvimStyleStringRefV1::default();
    Ok(match origin {
        StyleContributionOrigin::EngineEmergency => {
            (EVIM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY, 0, none)
        }
        StyleContributionOrigin::BlockStyle(id) => (
            EVIM_STYLE_CONTRIBUTOR_BLOCK_STYLE,
            EVIM_STYLE_NAMESPACE_BLOCK,
            push_style_string(strings, &id.0)?,
        ),
        StyleContributionOrigin::CharacterStyle(id) => (
            EVIM_STYLE_CONTRIBUTOR_CHARACTER_STYLE,
            EVIM_STYLE_NAMESPACE_CHARACTER,
            push_style_string(strings, &id.0)?,
        ),
        StyleContributionOrigin::DirectDocumentCanvas => {
            (EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CANVAS, 0, none)
        }
        StyleContributionOrigin::DirectDocumentCharacter => {
            (EVIM_STYLE_CONTRIBUTOR_DIRECT_DOCUMENT_CHARACTER, 0, none)
        }
        StyleContributionOrigin::DirectParagraph => {
            (EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH, 0, none)
        }
        StyleContributionOrigin::DirectParagraphCharacter => {
            (EVIM_STYLE_CONTRIBUTOR_DIRECT_PARAGRAPH_CHARACTER, 0, none)
        }
        StyleContributionOrigin::DirectCharacter => {
            (EVIM_STYLE_CONTRIBUTOR_DIRECT_CHARACTER, 0, none)
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn push_style_property(
    properties: &mut Vec<EvimStylePropertyV1>,
    value_items: &mut Vec<EvimStyleValueItemV1>,
    dependencies: &mut Vec<EvimStyleDependencyV1>,
    strings: &mut Vec<u8>,
    property: StyleProperty,
    declared: Option<StylePropertyValue>,
    effective: Option<StylePropertyValue>,
    contribution: &StyleContribution,
) -> Result<(), EvimStatus> {
    let mut flags = 0;
    let declared = if let Some(value) = declared.as_ref() {
        flags |= EVIM_STYLE_PROPERTY_DECLARED;
        style_value_to_ffi(value, value_items, strings)?
    } else {
        empty_style_value()
    };
    let effective = if let Some(value) = effective.as_ref() {
        flags |= EVIM_STYLE_PROPERTY_EFFECTIVE_PRESENT;
        style_value_to_ffi(value, value_items, strings)?
    } else {
        empty_style_value()
    };
    let (contributor_kind, contributor_namespace, contributor_style_id) =
        style_contributor_to_ffi(&contribution.winner, strings)?;
    if contributor_style_id.length != 0 {
        flags |= EVIM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE;
    }
    let first_dependency = checked_export_count(dependencies.len())?;
    dependencies
        .try_reserve(contribution.dependencies.len())
        .map_err(|_| EvimStatus::ResourceExhausted)?;
    for dependency in &contribution.dependencies {
        let (namespace, id) = match dependency {
            StyleDependency::Block(id) => (EVIM_STYLE_NAMESPACE_BLOCK, id),
            StyleDependency::Character(id) => (EVIM_STYLE_NAMESPACE_CHARACTER, id),
        };
        dependencies.push(EvimStyleDependencyV1 {
            struct_size: EVIM_STYLE_DEPENDENCY_V1_SIZE,
            namespace,
            style_id: push_style_string(strings, &id.0)?,
        });
    }
    properties.push(EvimStylePropertyV1 {
        struct_size: EVIM_STYLE_PROPERTY_V1_SIZE,
        flags,
        property: style_property_to_ffi(property),
        contributor_kind,
        contributor_namespace,
        reserved: 0,
        declared,
        effective,
        contributor_style_id,
        first_dependency,
        dependency_count: checked_export_count(contribution.dependencies.len())?,
    });
    Ok(())
}

fn generated_style_capabilities(
    origin: StyleDefinitionOrigin,
    is_base: bool,
    role: Option<BlockRole>,
    source_editable: bool,
) -> u32 {
    if origin != StyleDefinitionOrigin::GeneratedConfiguration
        && !(source_editable && origin == StyleDefinitionOrigin::SourceBacked)
    {
        return 0;
    }
    let mut capabilities =
        EVIM_STYLE_CAPABILITY_EDIT_DECLARATIONS | EVIM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME;
    if !is_base {
        capabilities |= EVIM_STYLE_CAPABILITY_EDIT_PARENT | EVIM_STYLE_CAPABILITY_DELETE;
    }
    if role == Some(BlockRole::Paragraph) {
        capabilities |= EVIM_STYLE_CAPABILITY_EDIT_NEXT_STYLE;
    }
    if source_editable
        && origin == StyleDefinitionOrigin::SourceBacked
        && role != Some(BlockRole::Document)
    {
        capabilities |= EVIM_STYLE_CAPABILITY_ASSIGN;
        if !is_base {
            capabilities |= EVIM_STYLE_CAPABILITY_DELETE;
        }
    }
    capabilities
}

fn export_style_sheet(document: &Document) -> Result<StyleSheetExport, EvimStatus> {
    let sheet = document.projection().style_sheet();
    let assignment = DocumentStyleAssignment::new(sheet.base_document.clone());
    let mut definitions = Vec::new();
    let mut properties = Vec::new();
    let mut value_items = Vec::new();
    let mut dependencies = Vec::new();
    let mut strings = Vec::new();
    definitions
        .try_reserve(sheet.block_style_count() + sheet.character_style_count())
        .map_err(|_| EvimStatus::ResourceExhausted)?;

    for style in sheet.block_styles() {
        let metadata = sheet
            .block_style_metadata(&style.id)
            .ok_or(EvimStatus::CoreFailure)?;
        let first_property = checked_export_count(properties.len())?;
        let property_keys = match style.role {
            BlockRole::Document => [
                FFI_CANVAS_PROPERTIES.as_slice(),
                FFI_CHARACTER_PROPERTIES.as_slice(),
            ]
            .concat(),
            BlockRole::Paragraph => [
                FFI_PARAGRAPH_PROPERTIES.as_slice(),
                FFI_CHARACTER_PROPERTIES.as_slice(),
            ]
            .concat(),
        };

        match style.role {
            BlockRole::Document => {
                let resolved = sheet
                    .resolve_document_style_with_contributions(
                        &style.id,
                        &BlockProperties::default(),
                        &CharacterProperties::default(),
                    )
                    .map_err(style_error_status)?;
                for property in property_keys.iter().copied() {
                    push_style_property(
                        &mut properties,
                        &mut value_items,
                        &mut dependencies,
                        &mut strings,
                        property,
                        declared_block_property(&style.block, &style.character, property),
                        effective_document_property(&resolved.value, property),
                        resolved
                            .contribution(property)
                            .ok_or(EvimStatus::CoreFailure)?,
                    )?;
                }
            }
            BlockRole::Paragraph => {
                let resolved = sheet
                    .resolve_assigned_paragraph_style_with_contributions(
                        &assignment,
                        &style.id,
                        &BlockProperties::default(),
                        &CharacterProperties::default(),
                        None,
                        &CharacterProperties::default(),
                    )
                    .map_err(style_error_status)?;
                for property in property_keys.iter().copied() {
                    push_style_property(
                        &mut properties,
                        &mut value_items,
                        &mut dependencies,
                        &mut strings,
                        property,
                        declared_block_property(&style.block, &style.character, property),
                        effective_paragraph_property(&resolved.value, property),
                        resolved
                            .contribution(property)
                            .ok_or(EvimStatus::CoreFailure)?,
                    )?;
                }
            }
        }

        let is_base_document = style.id == sheet.base_document;
        let is_base_paragraph = style.id == sheet.base_paragraph;
        let mut flags = if style.id.is_internal_list() || style.id.legacy_list_level().is_some() {
            EVIM_STYLE_DEFINITION_INTERNAL_LIST
        } else {
            0
        };
        let parent_id = if let Some(parent) = &style.based_on {
            flags |= EVIM_STYLE_DEFINITION_HAS_PARENT;
            push_style_string(&mut strings, &parent.0)?
        } else {
            EvimStyleStringRefV1::default()
        };
        let next_style_id = if let Some(next) = &style.next_paragraph_style {
            flags |= EVIM_STYLE_DEFINITION_HAS_NEXT_STYLE;
            push_style_string(&mut strings, &next.0)?
        } else {
            EvimStyleStringRefV1::default()
        };
        if is_base_document {
            flags |= EVIM_STYLE_DEFINITION_BASE_DOCUMENT;
        }
        if is_base_paragraph {
            flags |= EVIM_STYLE_DEFINITION_BASE_PARAGRAPH;
        }
        definitions.push(EvimStyleDefinitionV1 {
            struct_size: EVIM_STYLE_DEFINITION_V1_SIZE,
            flags,
            namespace: EVIM_STYLE_NAMESPACE_BLOCK,
            role: style_role_to_ffi(style.role),
            origin: style_origin_to_ffi(metadata.origin),
            capabilities: generated_style_capabilities(
                metadata.origin,
                is_base_document || is_base_paragraph,
                Some(style.role),
                matches!(
                    document.format(),
                    Format::Html | Format::HtmlSource | Format::Rtf
                ),
            ) | if sheet.has_user_default(&style.id, false)
                && style.role == BlockRole::Paragraph
                && (matches!(document.format(), Format::Html | Format::HtmlSource)
                    || (document.format() == Format::Rtf && style.id.0.starts_with("RtfP")))
            {
                EVIM_STYLE_CAPABILITY_ASSIGN
            } else {
                0
            } | if matches!(document.format(), Format::Markdown | Format::MarkdownSource)
                && style.id.0 == "Block quote"
            {
                EVIM_STYLE_CAPABILITY_ASSIGN
            } else {
                0
            } | if document.format() == Format::Rtf
                && style.id.0.starts_with("List")
                && crate::document::StyleSheet::builtin_block(&style.id)
            {
                EVIM_STYLE_CAPABILITY_ASSIGN
            } else {
                0
            },
            stable_id: push_style_string(&mut strings, &style.id.0)?,
            display_name: push_style_string(&mut strings, &metadata.display_name)?,
            parent_id,
            next_style_id,
            first_property,
            property_count: checked_export_count(property_keys.len())?,
        });
    }

    for style in sheet.character_styles() {
        let metadata = sheet
            .character_style_metadata(&style.id)
            .ok_or(EvimStatus::CoreFailure)?;
        let resolved = sheet
            .resolve_assigned_paragraph_style_with_contributions(
                &assignment,
                &sheet.base_paragraph,
                &BlockProperties::default(),
                &CharacterProperties::default(),
                Some(&style.id),
                &CharacterProperties::default(),
            )
            .map_err(style_error_status)?;
        let first_property = checked_export_count(properties.len())?;
        for property in FFI_CHARACTER_PROPERTIES {
            push_style_property(
                &mut properties,
                &mut value_items,
                &mut dependencies,
                &mut strings,
                property,
                declared_character_property(&style.properties, property),
                effective_character_property(&resolved.value.character, property),
                resolved
                    .contribution(property)
                    .ok_or(EvimStatus::CoreFailure)?,
            )?;
        }
        let is_base = style.id == sheet.base_character;
        let mut flags = u32::from(is_base) * EVIM_STYLE_DEFINITION_BASE_CHARACTER;
        if style.id.is_internal() {
            flags |= EVIM_STYLE_DEFINITION_INTERNAL;
        }
        let parent_id = if let Some(parent) = &style.based_on {
            flags |= EVIM_STYLE_DEFINITION_HAS_PARENT;
            push_style_string(&mut strings, &parent.0)?
        } else {
            EvimStyleStringRefV1::default()
        };
        definitions.push(EvimStyleDefinitionV1 {
            struct_size: EVIM_STYLE_DEFINITION_V1_SIZE,
            flags,
            namespace: EVIM_STYLE_NAMESPACE_CHARACTER,
            role: EVIM_STYLE_ROLE_NONE,
            origin: style_origin_to_ffi(metadata.origin),
            capabilities: if style.id.is_internal() {
                EVIM_STYLE_CAPABILITY_EDIT_DECLARATIONS
            } else {
                generated_style_capabilities(
                    metadata.origin,
                    is_base,
                    None,
                    matches!(
                        document.format(),
                        Format::Html | Format::HtmlSource | Format::Rtf
                    ),
                ) | if document.validate_typing_named_style(&style.id).is_ok()
                {
                    EVIM_STYLE_CAPABILITY_ASSIGN
                } else {
                    0
                }
            },
            stable_id: push_style_string(&mut strings, &style.id.0)?,
            display_name: push_style_string(&mut strings, &metadata.display_name)?,
            parent_id,
            next_style_id: EvimStyleStringRefV1::default(),
            first_property,
            property_count: checked_export_count(FFI_CHARACTER_PROPERTIES.len())?,
        });
    }

    let info = EvimStyleSheetInfoV1 {
        struct_size: EVIM_STYLE_SHEET_INFO_V1_SIZE,
        reserved: 0,
        identity: style_sheet_identity(document),
        definition_count: checked_export_count(definitions.len())?,
        property_count: checked_export_count(properties.len())?,
        value_item_count: checked_export_count(value_items.len())?,
        dependency_count: checked_export_count(dependencies.len())?,
        string_bytes: checked_export_count(strings.len())?,
    };
    Ok(StyleSheetExport {
        info,
        definitions,
        properties,
        value_items,
        dependencies,
        strings,
    })
}

fn layout_paint_info(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<EvimLayoutPaintInfoV1, EvimStatus> {
    Ok(EvimLayoutPaintInfoV1 {
        struct_size: EVIM_LAYOUT_PAINT_INFO_V1_SIZE,
        flags: if snapshot.canvas_background_is_default {
            EVIM_LAYOUT_PAINT_DEFAULT_CANVAS
        } else {
            0
        },
        identity: snapshot_identity(snapshot, view_id),
        canvas_background: color_to_ffi(snapshot.canvas_background),
        default_paint: text_paint_to_ffi(&snapshot.default_paint),
        paint_run_count: checked_export_count(snapshot.paint_runs.len())?,
    })
}

fn export_layout_paint(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<LayoutPaintExport, EvimStatus> {
    let info = layout_paint_info(snapshot, view_id)?;
    let mut previous_end = None;
    let runs = snapshot
        .paint_runs
        .iter()
        .map(|run| {
            if run.text_range.start > run.text_range.end
                || previous_end.is_some_and(|end| run.text_range.start < end)
            {
                return Err(EvimStatus::CoreFailure);
            }
            previous_end = Some(run.text_range.end);
            Ok(EvimPaintStyleRunV1 {
                struct_size: EVIM_PAINT_STYLE_RUN_V1_SIZE,
                reserved: 0,
                text_start: checked_export_count(run.text_range.start)?,
                text_end: checked_export_count(run.text_range.end)?,
                paint: text_paint_to_ffi(&run.paint),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    debug_assert_eq!(info.paint_run_count as usize, runs.len());
    Ok(LayoutPaintExport { info, runs })
}

struct CommandLineExport {
    info: EvimCommandLineInfoV1,
    bytes: Vec<u8>,
}

struct VisualSelectionExport {
    info: EvimVisualSelectionInfoV1,
    segments: Vec<EvimVisualSelectionSegmentV1>,
    rectangles: Vec<EvimVisualSelectionRectangleV1>,
}

fn command_line_kind_to_ffi(kind: Option<CommandLineKind>) -> u32 {
    match kind {
        None => EVIM_COMMAND_LINE_KIND_NONE,
        Some(CommandLineKind::Ex) => EVIM_COMMAND_LINE_KIND_EX,
        Some(CommandLineKind::SearchForward) => EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD,
        Some(CommandLineKind::SearchBackward) => EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD,
    }
}

fn command_line_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        EVIM_COMMAND_LINE_KIND_NONE
            | EVIM_COMMAND_LINE_KIND_EX
            | EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD
            | EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD
    )
}

fn visual_selection_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        EVIM_VISUAL_SELECTION_KIND_NONE
            | EVIM_VISUAL_SELECTION_KIND_CHARACTER
            | EVIM_VISUAL_SELECTION_KIND_LINE
            | EVIM_VISUAL_SELECTION_KIND_BLOCK
    )
}

fn opaque_state_identity(material: &[u8]) -> [u8; 32] {
    *SourceArtifactDigest::from_bytes(material).as_bytes()
}

fn export_command_line(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<CommandLineExport, EvimStatus> {
    let state = core.command_state(view_id).ok_or(EvimStatus::InvalidView)?;
    let (kind, bytes, cursor) = match (
        state.command_line_kind(),
        state.command_line(),
        state.command_line_cursor(),
    ) {
        (Some(kind), Some(text), Some(cursor))
            if cursor <= text.len() && text.is_char_boundary(cursor) =>
        {
            (
                command_line_kind_to_ffi(Some(kind)),
                text.as_bytes().to_vec(),
                cursor,
            )
        }
        (None, None, None) => (EVIM_COMMAND_LINE_KIND_NONE, Vec::new(), 0),
        _ => return Err(EvimStatus::CoreFailure),
    };
    let utf8_length = checked_export_count(bytes.len())?;
    let cursor_utf8_offset = checked_export_count(cursor)?;
    let mut material = Vec::with_capacity(32usize.saturating_add(bytes.len()));
    material.extend_from_slice(b"evim-command-line-v1\0");
    material.extend_from_slice(&kind.to_le_bytes());
    material.extend_from_slice(&utf8_length.to_le_bytes());
    material.extend_from_slice(&cursor_utf8_offset.to_le_bytes());
    let anchor = state
        .command_line_snapshot()
        .map_or(cursor, |snapshot| snapshot.anchor);
    material.extend_from_slice(&(anchor as u64).to_le_bytes());
    material.extend_from_slice(&bytes);
    let identity = EvimCommandLineIdentityV1 {
        struct_size: EVIM_COMMAND_LINE_IDENTITY_V1_SIZE,
        kind,
        view_id: view_id.0,
        document_id: core.document().id().0,
        document_revision: core.document().revision().0,
        state_identity: opaque_state_identity(&material),
    };
    Ok(CommandLineExport {
        info: EvimCommandLineInfoV1 {
            struct_size: EVIM_COMMAND_LINE_INFO_V1_SIZE,
            reserved: 0,
            identity,
            utf8_length,
            cursor_utf8_offset,
        },
        bytes,
    })
}

fn validate_command_line_identity(
    expected: EvimCommandLineIdentityV1,
    actual: EvimCommandLineIdentityV1,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_COMMAND_LINE_IDENTITY_V1_SIZE
        || !command_line_kind_is_valid(expected.kind)
    {
        return Err(EvimStatus::InvalidArgument);
    }
    (expected.kind == actual.kind
        && expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn visual_block_status(error: VisualBlockError) -> EvimStatus {
    match error {
        VisualBlockError::WrongDocument { .. }
        | VisualBlockError::WrongDocumentRevision { .. }
        | VisualBlockError::StaleLayout { .. } => EvimStatus::StaleRevision,
        VisualBlockError::EndpointNotInLayout(_) | VisualBlockError::EmptyLayout => {
            EvimStatus::OutsideLayoutCoverage
        }
        VisualBlockError::NonGraphemeBoundary(_) => EvimStatus::NotGraphemeBoundary,
        VisualBlockError::InvalidX => EvimStatus::InvalidArgument,
        VisualBlockError::TextDoesNotMatchLayout
        | VisualBlockError::OverlappingRanges
        | VisualBlockError::ReplacementTooLarge { .. } => EvimStatus::CoreFailure,
    }
}

fn checked_selection_range(
    document: &Document,
    start: usize,
    end: usize,
) -> Result<TextRange, EvimStatus> {
    let start = document.text_point(start).map_err(document_status)?;
    let end = document.text_point(end).map_err(document_status)?;
    TextRange::new(start, end).map_err(|_| EvimStatus::CoreFailure)
}

fn push_selection_rectangles(
    snapshot: &LayoutSnapshot,
    range: TextRange,
    empty_affinity: BoundaryAffinity,
    segment_index: usize,
    expected_row: Option<usize>,
    rectangles: &mut Vec<EvimVisualSelectionRectangleV1>,
) -> Result<(), EvimStatus> {
    let resolved = snapshot
        .selection_rectangles(range, empty_affinity)
        .map_err(layout_query_status)?;
    let mut matched = false;
    for rectangle in resolved {
        if expected_row.is_some_and(|row| row != rectangle.row_index) {
            continue;
        }
        matched = true;
        rectangles.push(EvimVisualSelectionRectangleV1 {
            struct_size: EVIM_VISUAL_SELECTION_RECTANGLE_V1_SIZE,
            reserved: 0,
            row_index: checked_export_count(rectangle.row_index)?,
            segment_index: checked_export_count(segment_index)?,
            rect: layout_rect_to_ffi(rectangle.rect),
        });
    }
    if expected_row.is_some() && !matched {
        return Err(EvimStatus::CoreFailure);
    }
    Ok(())
}

fn visual_selection_state_identity(
    kind: u32,
    segments: &[EvimVisualSelectionSegmentV1],
    rectangles: &[EvimVisualSelectionRectangleV1],
) -> [u8; 32] {
    let mut material = Vec::with_capacity(
        40usize
            .saturating_add(segments.len().saturating_mul(48))
            .saturating_add(rectangles.len().saturating_mul(40)),
    );
    material.extend_from_slice(b"evim-visual-selection-v1\0");
    material.extend_from_slice(&kind.to_le_bytes());
    material.extend_from_slice(&(segments.len() as u64).to_le_bytes());
    for segment in segments {
        material.extend_from_slice(&segment.flags.to_le_bytes());
        material.extend_from_slice(&segment.text_start.to_le_bytes());
        material.extend_from_slice(&segment.text_end.to_le_bytes());
        material.extend_from_slice(&segment.row_index.to_le_bytes());
        material.extend_from_slice(&segment.hard_line_index.to_le_bytes());
        material.extend_from_slice(&segment.left_affinity.to_le_bytes());
        material.extend_from_slice(&segment.right_affinity.to_le_bytes());
    }
    material.extend_from_slice(&(rectangles.len() as u64).to_le_bytes());
    for rectangle in rectangles {
        material.extend_from_slice(&rectangle.row_index.to_le_bytes());
        material.extend_from_slice(&rectangle.segment_index.to_le_bytes());
        for value in [
            rectangle.rect.x,
            rectangle.rect.y,
            rectangle.rect.width,
            rectangle.rect.height,
        ] {
            material.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    opaque_state_identity(&material)
}

fn export_visual_selection(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<VisualSelectionExport, EvimStatus> {
    let snapshot = current_ffi_layout_snapshot(core, view_id)?;
    let state = core.command_state(view_id).ok_or(EvimStatus::InvalidView)?;
    let document = core.document();
    let mut segments = Vec::new();
    let mut rectangles = Vec::new();
    let kind = match state.mode() {
        Mode::VisualCharacter | Mode::VisualLine => {
            let kind = if state.mode() == Mode::VisualCharacter {
                EVIM_VISUAL_SELECTION_KIND_CHARACTER
            } else {
                EVIM_VISUAL_SELECTION_KIND_LINE
            };
            let range = state
                .line_selection_range(document, Some(snapshot))
                .ok_or(EvimStatus::CoreFailure)?;
            let checked = checked_selection_range(document, range.start, range.end)?;
            segments.push(EvimVisualSelectionSegmentV1 {
                struct_size: EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE,
                flags: 0,
                text_start: checked_export_count(range.start)?,
                text_end: checked_export_count(range.end)?,
                row_index: 0,
                hard_line_index: 0,
                left_affinity: 0,
                right_affinity: 0,
            });
            push_selection_rectangles(
                snapshot,
                checked,
                state.boundary_affinity(),
                0,
                None,
                &mut rectangles,
            )?;
            kind
        }
        Mode::VisualBlock => {
            let selection = state.visual_block().ok_or(EvimStatus::CoreFailure)?;
            let resolved = if state.visual_block_to_line_end() {
                resolve_block_selection_to_line_end(selection, snapshot, document.text())
            } else {
                resolve_block_selection(selection, snapshot, document.text())
            }
            .map_err(visual_block_status)?;
            for (segment_index, segment) in resolved.range_set.segments.iter().enumerate() {
                let checked =
                    checked_selection_range(document, segment.range.start, segment.range.end)?;
                segments.push(EvimVisualSelectionSegmentV1 {
                    struct_size: EVIM_VISUAL_SELECTION_SEGMENT_V1_SIZE,
                    flags: EVIM_VISUAL_SELECTION_SEGMENT_HAS_VISUAL_ROW
                        | EVIM_VISUAL_SELECTION_SEGMENT_HAS_HARD_LINE
                        | EVIM_VISUAL_SELECTION_SEGMENT_HAS_EDGE_AFFINITIES,
                    text_start: checked_export_count(segment.range.start)?,
                    text_end: checked_export_count(segment.range.end)?,
                    row_index: checked_export_count(segment.row_index)?,
                    hard_line_index: checked_export_count(segment.hard_line_index)?,
                    left_affinity: affinity_to_ffi(segment.left_affinity),
                    right_affinity: affinity_to_ffi(segment.right_affinity),
                });
                push_selection_rectangles(
                    snapshot,
                    checked,
                    segment.left_affinity,
                    segment_index,
                    Some(segment.row_index),
                    &mut rectangles,
                )?;
            }
            rectangles.sort_by(|left, right| {
                left.row_index
                    .cmp(&right.row_index)
                    .then_with(|| left.rect.x.total_cmp(&right.rect.x))
                    .then_with(|| left.segment_index.cmp(&right.segment_index))
            });
            EVIM_VISUAL_SELECTION_KIND_BLOCK
        }
        Mode::Normal | Mode::Insert | Mode::Replace | Mode::CommandLine => {
            EVIM_VISUAL_SELECTION_KIND_NONE
        }
    };
    let identity = EvimVisualSelectionIdentityV1 {
        struct_size: EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE,
        kind,
        layout: snapshot_identity(snapshot, view_id),
        state_identity: visual_selection_state_identity(kind, &segments, &rectangles),
    };
    Ok(VisualSelectionExport {
        info: EvimVisualSelectionInfoV1 {
            struct_size: EVIM_VISUAL_SELECTION_INFO_V1_SIZE,
            reserved: 0,
            identity,
            segment_count: checked_export_count(segments.len())?,
            rectangle_count: checked_export_count(rectangles.len())?,
        },
        segments,
        rectangles,
    })
}

fn visual_selection_text(
    document: &Document,
    export: &VisualSelectionExport,
) -> Result<String, EvimStatus> {
    if export.info.identity.kind == EVIM_VISUAL_SELECTION_KIND_NONE || export.segments.is_empty() {
        return Err(EvimStatus::InvalidRange);
    }
    let block = export.info.identity.kind == EVIM_VISUAL_SELECTION_KIND_BLOCK;
    let mut text = String::new();
    for (index, segment) in export.segments.iter().enumerate() {
        let start = checked_length(segment.text_start)?;
        let end = checked_length(segment.text_end)?;
        let piece = document
            .text()
            .get(start..end)
            .ok_or(EvimStatus::InvalidRange)?;
        if block && index != 0 {
            text.push('\n');
        }
        text.push_str(piece);
    }
    if text.is_empty() {
        return Err(EvimStatus::InvalidRange);
    }
    Ok(text)
}

fn validate_visual_selection_identity(
    expected: EvimVisualSelectionIdentityV1,
    actual: EvimVisualSelectionIdentityV1,
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE
        || !visual_selection_kind_is_valid(expected.kind)
    {
        return Err(EvimStatus::InvalidArgument);
    }
    validate_snapshot_identity(expected.layout, snapshot, view_id)?;
    (expected.kind == actual.kind && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn logical_selection_kind_to_ffi(kind: LogicalSelectionKind) -> u32 {
    match kind {
        LogicalSelectionKind::None => EVIM_LOGICAL_SELECTION_KIND_NONE,
        LogicalSelectionKind::Character => EVIM_LOGICAL_SELECTION_KIND_CHARACTER,
        LogicalSelectionKind::Line => EVIM_LOGICAL_SELECTION_KIND_LINE,
        LogicalSelectionKind::Block => EVIM_LOGICAL_SELECTION_KIND_BLOCK,
    }
}

fn logical_selection_kind_is_valid(kind: u32) -> bool {
    matches!(
        kind,
        EVIM_LOGICAL_SELECTION_KIND_NONE
            | EVIM_LOGICAL_SELECTION_KIND_CHARACTER
            | EVIM_LOGICAL_SELECTION_KIND_LINE
            | EVIM_LOGICAL_SELECTION_KIND_BLOCK
    )
}

fn semantic_style_from_ffi(style: u32) -> Result<SemanticInlineStyle, EvimStatus> {
    match style {
        EVIM_SEMANTIC_STYLE_STRONG => Ok(SemanticInlineStyle::Strong),
        EVIM_SEMANTIC_STYLE_EMPHASIS => Ok(SemanticInlineStyle::Emphasis),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

fn semantic_style_to_ffi(style: SemanticInlineStyle) -> Result<u32, EvimStatus> {
    match style {
        SemanticInlineStyle::Strong => Ok(EVIM_SEMANTIC_STYLE_STRONG),
        SemanticInlineStyle::Emphasis => Ok(EVIM_SEMANTIC_STYLE_EMPHASIS),
        SemanticInlineStyle::Code => Err(EvimStatus::InvalidArgument),
    }
}

fn semantic_style_state_to_ffi(state: SemanticStyleState) -> u32 {
    match state {
        SemanticStyleState::Off => EVIM_SEMANTIC_STYLE_STATE_OFF,
        SemanticStyleState::On => EVIM_SEMANTIC_STYLE_STATE_ON,
        SemanticStyleState::Mixed => EVIM_SEMANTIC_STYLE_STATE_MIXED,
    }
}

fn logical_selection_state_identity(selection: &LogicalSelectionIdentity) -> [u8; 32] {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(b"evim-logical-selection-v1\0");
    material.extend_from_slice(&logical_selection_kind_to_ffi(selection.kind()).to_le_bytes());
    material.extend_from_slice(&selection.view().0.to_le_bytes());
    material.extend_from_slice(&selection.document().0.to_le_bytes());
    material.extend_from_slice(&selection.revision().0.to_le_bytes());
    material.extend_from_slice(&(selection.anchor() as u64).to_le_bytes());
    material.extend_from_slice(&(selection.active() as u64).to_le_bytes());
    material.extend_from_slice(&affinity_to_ffi(selection.active_affinity()).to_le_bytes());
    let range = selection.range();
    material.extend_from_slice(&(range.start as u64).to_le_bytes());
    material.extend_from_slice(&(range.end as u64).to_le_bytes());
    opaque_state_identity(&material)
}

fn logical_selection_identity_to_ffi(
    selection: &LogicalSelectionIdentity,
) -> Result<EvimLogicalSelectionIdentityV1, EvimStatus> {
    let range = selection.range();
    Ok(EvimLogicalSelectionIdentityV1 {
        struct_size: EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE,
        kind: logical_selection_kind_to_ffi(selection.kind()),
        view_id: selection.view().0,
        document_id: selection.document().0,
        document_revision: selection.revision().0,
        text_start: checked_export_count(range.start)?,
        text_end: checked_export_count(range.end)?,
        state_identity: logical_selection_state_identity(selection),
    })
}

fn export_semantic_style_presentation(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
    style: SemanticInlineStyle,
) -> Result<(EvimSemanticStylePresentationV1, SemanticStylePresentation), EvimStatus> {
    let presentation = core
        .selection_semantic_style_presentation(view_id, style)
        .map_err(core_status)?;
    let selection = match presentation.selection() {
        Some(selection) => logical_selection_identity_to_ffi(selection)?,
        None => EvimLogicalSelectionIdentityV1 {
            struct_size: EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE,
            kind: logical_selection_kind_to_ffi(presentation.selection_kind()),
            view_id: view_id.0,
            document_id: core.document().id().0,
            document_revision: core.document().revision().0,
            ..EvimLogicalSelectionIdentityV1::default()
        },
    };
    let mut flags = 0;
    if presentation
        .selection()
        .is_some_and(|selection| !selection.range().is_empty())
    {
        flags |= EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE;
    }
    if presentation
        .selection()
        .is_some_and(|selection| selection.kind() == LogicalSelectionKind::None)
    {
        flags |= EVIM_SEMANTIC_STYLE_TYPING_CONTEXT;
    }
    if presentation.can_set() {
        flags |= EVIM_SEMANTIC_STYLE_CAN_SET;
    }
    if presentation.can_clear() {
        flags |= EVIM_SEMANTIC_STYLE_CAN_CLEAR;
    }
    Ok((
        EvimSemanticStylePresentationV1 {
            struct_size: EVIM_SEMANTIC_STYLE_PRESENTATION_V1_SIZE,
            style: semantic_style_to_ffi(style)?,
            state: semantic_style_state_to_ffi(presentation.state()),
            flags,
            selection,
        },
        presentation,
    ))
}

fn validate_logical_selection_identity(
    expected: EvimLogicalSelectionIdentityV1,
    actual: EvimLogicalSelectionIdentityV1,
) -> Result<(), EvimStatus> {
    if expected.struct_size < EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE
        || !logical_selection_kind_is_valid(expected.kind)
    {
        return Err(EvimStatus::InvalidArgument);
    }
    (expected.kind == actual.kind
        && expected.view_id == actual.view_id
        && expected.document_id == actual.document_id
        && expected.document_revision == actual.document_revision
        && expected.text_start == actual.text_start
        && expected.text_end == actual.text_end
        && expected.state_identity == actual.state_identity)
        .then_some(())
        .ok_or(EvimStatus::StaleRevision)
}

fn export_layout_snapshot(
    snapshot: &LayoutSnapshot,
    view_id: ViewId,
) -> Result<LayoutSnapshotExport, EvimStatus> {
    let info = layout_snapshot_info(snapshot, view_id)?;
    let row_capacity = usize::try_from(info.row_count).map_err(|_| EvimStatus::LengthOverflow)?;
    let cluster_capacity =
        usize::try_from(info.cluster_count).map_err(|_| EvimStatus::LengthOverflow)?;
    let caret_capacity =
        usize::try_from(info.caret_count).map_err(|_| EvimStatus::LengthOverflow)?;
    let mut rows = Vec::with_capacity(row_capacity);
    let mut clusters = Vec::with_capacity(cluster_capacity);
    let mut carets = Vec::with_capacity(caret_capacity);
    for (row_index, row) in snapshot.rows.iter().enumerate() {
        let first_cluster = checked_export_count(clusters.len())?;
        let first_caret = checked_export_count(carets.len())?;
        for cluster in &row.clusters {
            let (flags, render_run) = cluster.render_run.map_or_else(
                || (0, EvimRenderRunHandleV1::default()),
                |render_run| {
                    (
                        EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN,
                        render_run_to_ffi(render_run),
                    )
                },
            );
            clusters.push(EvimPositionedClusterV1 {
                struct_size: EVIM_POSITIONED_CLUSTER_V1_SIZE,
                flags,
                row_index: checked_export_count(row_index)?,
                text_start: checked_export_count(cluster.text_range.start)?,
                text_end: checked_export_count(cluster.text_range.end)?,
                x: cluster.x,
                advance: cluster.advance,
                typographic_bounds: layout_rect_to_ffi(cluster.typographic_bounds),
                ink_bounds: layout_rect_to_ffi(cluster.ink_bounds),
                bidi_level: u32::from(cluster.bidi_level),
                reserved: 0,
                render_run,
            });
        }
        for caret in &row.carets {
            carets.push(EvimPositionedCaretV1 {
                struct_size: EVIM_POSITIONED_CARET_V1_SIZE,
                affinity: affinity_to_ffi(caret.point.affinity),
                row_index: checked_export_count(row_index)?,
                text_offset: checked_export_count(caret.point.text_offset)?,
                x: caret.x,
                reserved: 0.0,
            });
        }
        let mut flags = 0;
        let paragraph_id = row.paragraph_id.map_or(0, |paragraph_id| {
            flags |= EVIM_VISUAL_ROW_HAS_PARAGRAPH;
            paragraph_id
        });
        if row.wrapped_from_previous {
            flags |= EVIM_VISUAL_ROW_WRAPPED_FROM_PREVIOUS;
        }
        if row.wraps_to_next {
            flags |= EVIM_VISUAL_ROW_WRAPS_TO_NEXT;
        }
        rows.push(EvimVisualRowV1 {
            struct_size: EVIM_VISUAL_ROW_V1_SIZE,
            flags,
            row_index: checked_export_count(row_index)?,
            paragraph_id,
            hard_line_index: checked_export_count(row.hard_line_index)?,
            hard_line_start: checked_export_count(row.hard_line_range.start)?,
            hard_line_end: checked_export_count(row.hard_line_range.end)?,
            text_start: checked_export_count(row.text_range.start)?,
            text_end: checked_export_count(row.text_range.end)?,
            y: row.y,
            baseline: row.baseline,
            ascent: row.ascent,
            descent: row.descent,
            leading: row.leading,
            line_advance: row.line_advance,
            width: row.width,
            paragraph_content_x: row.paragraph_content_x,
            paragraph_content_width: row.paragraph_content_width,
            first_cluster,
            cluster_count: checked_export_count(row.clusters.len())?,
            first_caret,
            caret_count: checked_export_count(row.carets.len())?,
        });
    }
    debug_assert_eq!(rows.len(), row_capacity);
    debug_assert_eq!(clusters.len(), cluster_capacity);
    debug_assert_eq!(carets.len(), caret_capacity);
    Ok(LayoutSnapshotExport {
        info,
        rows,
        clusters,
        carets,
    })
}

fn summarize_view_presentation(
    core: &Core<CTextMeasurementProvider>,
    view_id: ViewId,
) -> Result<EvimViewPresentationV1, EvimStatus> {
    let state = core.command_state(view_id).ok_or(EvimStatus::InvalidView)?;
    let mut flags = 0;
    let mut cursor_offset = state.cursor();
    let mut cursor_affinity = state.boundary_affinity();
    let mut visual_anchor_offset = 0;
    let mut visual_anchor_affinity = 0;
    let mut block_left_x = 0.0;
    let mut block_right_x = 0.0;
    if let Some(block) = state.visual_block() {
        flags |= EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR
            | EVIM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT
            | EVIM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK;
        cursor_offset = block.active.text_offset;
        cursor_affinity = block.active.affinity;
        visual_anchor_offset = checked_export_count(block.anchor.text_offset)?;
        visual_anchor_affinity = affinity_to_ffi(block.anchor.affinity);
        block_left_x = block.left_x();
        block_right_x = block.right_x();
    } else if let Some(anchor) = state.visual_anchor() {
        flags |= EVIM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR;
        visual_anchor_offset = checked_export_count(anchor)?;
    }
    let desired_x = state.desired_x().map_or(0.0, |desired_x| {
        flags |= EVIM_VIEW_PRESENTATION_HAS_DESIRED_X;
        desired_x
    });
    let (command_line_length, command_line_cursor) =
        if let Some(command_line) = state.command_line() {
            flags |= EVIM_VIEW_PRESENTATION_HAS_COMMAND_LINE;
            (
                checked_export_count(command_line.len())?,
                checked_export_count(state.command_line_cursor().unwrap_or(0))?,
            )
        } else {
            (0, 0)
        };
    Ok(EvimViewPresentationV1 {
        struct_size: EVIM_VIEW_PRESENTATION_V1_SIZE,
        flags,
        mode: mode_to_ffi(state.mode()),
        cursor_affinity: affinity_to_ffi(cursor_affinity),
        visual_anchor_affinity,
        reserved: 0,
        document_id: core.document().id().0,
        document_revision: core.document().revision().0,
        cursor_utf8_offset: checked_export_count(cursor_offset)?,
        visual_anchor_utf8_offset: visual_anchor_offset,
        visual_block_left_x: block_left_x,
        visual_block_right_x: block_right_x,
        desired_x,
        reserved_float: 0.0,
        command_line_utf8_length: command_line_length,
        command_line_cursor_utf8_offset: command_line_cursor,
    })
}

fn typed_pointer_region<T>(pointer: *const T, count: u64) -> Result<(usize, usize), EvimStatus> {
    let count = checked_length(count)?;
    if count == 0 {
        return Ok((pointer as usize, 0));
    }
    if pointer.is_null() {
        return Err(EvimStatus::NullPointer);
    }
    if (pointer as usize) % align_of::<T>() != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    let bytes = count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or(EvimStatus::LengthOverflow)?;
    Ok((pointer as usize, bytes))
}

fn regions_overlap(left: (usize, usize), right: (usize, usize)) -> bool {
    pointer_ranges_overlap(left.0 as *const u8, left.1, right.0 as *const u8, right.1)
}

fn parse_clipboard_target(value: u32) -> Result<ClipboardTarget, EvimStatus> {
    match value {
        EVIM_CLIPBOARD_TARGET_CLIPBOARD => Ok(ClipboardTarget::Clipboard),
        EVIM_CLIPBOARD_TARGET_PRIMARY => Ok(ClipboardTarget::Primary),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

unsafe fn read_command_turn_context(
    pointer: *const EvimCommandTurnContextV1,
    forbidden_outputs: &[(usize, usize)],
) -> Result<ClipboardCommandContext, EvimStatus> {
    let context_region = typed_pointer_region(pointer, 1)?;
    if forbidden_outputs
        .iter()
        .any(|output| regions_overlap(context_region, *output))
    {
        return Err(EvimStatus::InvalidArgument);
    }
    let context = unsafe { pointer.read() };
    if context.struct_size < EVIM_COMMAND_TURN_CONTEXT_V1_SIZE || context.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    // There are exactly two distinct targets in ABI v3. Bound the caller-
    // supplied count before constructing a slice or allocating a copy; larger
    // values cannot be valid even if they would eventually fail duplicate-
    // target validation.
    if context.clipboard_count > 2 {
        return Err(EvimStatus::InvalidArgument);
    }
    let entries_region = typed_pointer_region(context.clipboards, context.clipboard_count)?;
    if forbidden_outputs
        .iter()
        .any(|output| regions_overlap(entries_region, *output))
    {
        return Err(EvimStatus::InvalidArgument);
    }
    let entry_count = checked_length(context.clipboard_count)?;
    let entries = if entry_count == 0 {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(context.clipboards, entry_count) }.to_vec()
    };
    let mut seen_clipboard = false;
    let mut seen_primary = false;
    let mut result = ClipboardCommandContext::new();
    for entry in entries {
        if entry.struct_size < EVIM_CLIPBOARD_TURN_ENTRY_V1_SIZE
            || entry.reserved != 0
            || entry.flags & !(EVIM_CLIPBOARD_TURN_HAS_READ | EVIM_CLIPBOARD_TURN_WRITABLE) != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let target = parse_clipboard_target(entry.target)?;
        let seen = match target {
            ClipboardTarget::Clipboard => &mut seen_clipboard,
            ClipboardTarget::Primary => &mut seen_primary,
        };
        if *seen {
            return Err(EvimStatus::InvalidArgument);
        }
        *seen = true;

        let text_region = typed_pointer_region(entry.plain_text.data, entry.plain_text.length)?;
        if forbidden_outputs
            .iter()
            .any(|output| regions_overlap(text_region, *output))
        {
            return Err(EvimStatus::InvalidArgument);
        }
        if entry.flags & EVIM_CLIPBOARD_TURN_HAS_READ != 0 {
            let bytes = unsafe { input_bytes(entry.plain_text.data, entry.plain_text.length)? };
            let text = str::from_utf8(bytes)
                .map_err(|_| EvimStatus::InvalidUtf8)?
                .to_owned();
            result = result.with_read(ClipboardSnapshot::new(
                target,
                ClipboardGeneration(entry.generation),
                ClipboardContent::from_plain_text(text),
            ));
        } else if entry.generation != 0 || entry.plain_text.length != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        if entry.flags & EVIM_CLIPBOARD_TURN_WRITABLE != 0 {
            result = result.with_write(target);
        }
    }
    Ok(result)
}

unsafe fn read_layout_identity(
    pointer: *const EvimLayoutSnapshotIdentityV1,
) -> Result<EvimLayoutSnapshotIdentityV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_formatted_snapshot_identity(
    pointer: *const EvimFormattedSnapshotIdentityV1,
) -> Result<EvimFormattedSnapshotIdentityV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_style_sheet_identity(
    pointer: *const EvimStyleSheetIdentityV1,
) -> Result<EvimStyleSheetIdentityV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    let identity = unsafe { pointer.read() };
    if identity.struct_size < EVIM_STYLE_SHEET_IDENTITY_V1_SIZE || identity.reserved != 0 {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_style_edit_group(
    pointer: *const EvimStyleEditGroupV1,
) -> Result<EvimStyleEditGroupV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    let group = unsafe { pointer.read() };
    if group.struct_size < EVIM_STYLE_EDIT_GROUP_V1_SIZE
        || group.flags != 0
        || group.token == 0
        || group.view_id == 0
        || group.document_id == 0
    {
        return Err(EvimStatus::InvalidStyleEditGroup);
    }
    Ok(group)
}

unsafe fn read_command_line_identity(
    pointer: *const EvimCommandLineIdentityV1,
) -> Result<EvimCommandLineIdentityV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < EVIM_COMMAND_LINE_IDENTITY_V1_SIZE
        || !command_line_kind_is_valid(identity.kind)
    {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(identity)
}

unsafe fn read_visual_selection_identity(
    pointer: *const EvimVisualSelectionIdentityV1,
) -> Result<EvimVisualSelectionIdentityV1, EvimStatus> {
    typed_pointer_region(pointer, 1)?;
    // SAFETY: The pointer is non-null, aligned, and caller-owned for this call.
    let identity = unsafe { pointer.read() };
    if identity.struct_size < EVIM_VISUAL_SELECTION_IDENTITY_V1_SIZE
        || !visual_selection_kind_is_valid(identity.kind)
        || identity.layout.struct_size < EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
        || identity.layout.reserved != 0
    {
        return Err(EvimStatus::InvalidArgument);
    }
    Ok(identity)
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

#[derive(Debug)]
struct ParsedStyleEdit {
    identity: EvimStyleSheetIdentityV1,
    namespace: StyleNamespace,
    style: StyleId,
    edit: StyleDefinitionFieldEdit,
}

fn parse_style_namespace(raw: u32) -> Result<StyleNamespace, EvimStatus> {
    match raw {
        EVIM_STYLE_NAMESPACE_BLOCK => Ok(StyleNamespace::Block),
        EVIM_STYLE_NAMESPACE_CHARACTER => Ok(StyleNamespace::Character),
        _ => Err(EvimStatus::InvalidArgument),
    }
}

fn parse_style_property(raw: u32) -> Result<StyleProperty, EvimStatus> {
    match raw {
        EVIM_STYLE_PROPERTY_CANVAS_BACKGROUND => Ok(StyleProperty::CanvasBackground),
        EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP => Ok(StyleProperty::CanvasPaddingTop),
        EVIM_STYLE_PROPERTY_CANVAS_PADDING_RIGHT => Ok(StyleProperty::CanvasPaddingRight),
        EVIM_STYLE_PROPERTY_CANVAS_PADDING_BOTTOM => Ok(StyleProperty::CanvasPaddingBottom),
        EVIM_STYLE_PROPERTY_CANVAS_PADDING_LEFT => Ok(StyleProperty::CanvasPaddingLeft),
        EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE => Ok(StyleProperty::ParagraphSpacingBefore),
        EVIM_STYLE_PROPERTY_PARAGRAPH_SPACING_AFTER => Ok(StyleProperty::ParagraphSpacingAfter),
        EVIM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING => Ok(StyleProperty::ParagraphLineSpacing),
        EVIM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT => {
            Ok(StyleProperty::ParagraphFirstLineIndent)
        }
        EVIM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT => Ok(StyleProperty::ParagraphLeadingIndent),
        EVIM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT => Ok(StyleProperty::ParagraphTrailingIndent),
        EVIM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT => Ok(StyleProperty::ParagraphAlignment),
        EVIM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION => Ok(StyleProperty::ParagraphBaseDirection),
        EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES => Ok(StyleProperty::CharacterFontFamilies),
        EVIM_STYLE_PROPERTY_CHARACTER_SIZE => Ok(StyleProperty::CharacterSize),
        EVIM_STYLE_PROPERTY_CHARACTER_WEIGHT => Ok(StyleProperty::CharacterWeight),
        EVIM_STYLE_PROPERTY_CHARACTER_BOLD => Ok(StyleProperty::CharacterBold),
        EVIM_STYLE_PROPERTY_CHARACTER_SLANT => Ok(StyleProperty::CharacterSlant),
        EVIM_STYLE_PROPERTY_CHARACTER_FOREGROUND => Ok(StyleProperty::CharacterForeground),
        EVIM_STYLE_PROPERTY_CHARACTER_BACKGROUND => Ok(StyleProperty::CharacterBackground),
        EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE => Ok(StyleProperty::CharacterUnderline),
        EVIM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH => Ok(StyleProperty::CharacterStrikethrough),
        EVIM_STYLE_PROPERTY_CHARACTER_LANGUAGE => Ok(StyleProperty::CharacterLanguage),
        EVIM_STYLE_PROPERTY_CHARACTER_DIRECTION => Ok(StyleProperty::CharacterDirection),
        EVIM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES => {
            Ok(StyleProperty::CharacterOpenTypeFeatures)
        }
        EVIM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING => Ok(StyleProperty::CharacterLetterSpacing),
        EVIM_STYLE_PROPERTY_CHARACTER_BASELINE_SHIFT => Ok(StyleProperty::CharacterBaselineShift),
        _ => Err(EvimStatus::InvalidStyleValue),
    }
}

fn style_edit_value_has_no_array(value: &EvimStyleEditValueV1) -> Result<(), EvimStatus> {
    if value.item_count == 0 {
        Ok(())
    } else {
        Err(EvimStatus::InvalidStyleValue)
    }
}

fn style_edit_value_has_no_text(value: &EvimStyleEditValueV1) -> Result<(), EvimStatus> {
    if value.text.length == 0 {
        Ok(())
    } else {
        Err(EvimStatus::InvalidStyleValue)
    }
}

unsafe fn parse_style_edit_items(
    value: &EvimStyleEditValueV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> Result<Vec<(u32, String, u32)>, EvimStatus> {
    let region = typed_pointer_region(value.items, value.item_count)?;
    let output_region = typed_pointer_region(out_outcome, 1)?;
    if regions_overlap(region, output_region) {
        return Err(EvimStatus::InvalidArgument);
    }
    let count = checked_length(value.item_count)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let raw_items = unsafe { slice::from_raw_parts(value.items, count) };
    let mut parsed = Vec::new();
    parsed
        .try_reserve(count)
        .map_err(|_| EvimStatus::ResourceExhausted)?;
    for item in raw_items {
        if item.struct_size < EVIM_STYLE_EDIT_VALUE_ITEM_V1_SIZE || item.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        parsed.push((
            item.kind,
            unsafe { composition_utf8(item.text, out_outcome)? },
            item.unsigned_value,
        ));
    }
    Ok(parsed)
}

unsafe fn parse_style_property_value(
    property: StyleProperty,
    value: &EvimStyleEditValueV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> Result<StylePropertyValue, EvimStatus> {
    if value.struct_size < EVIM_STYLE_EDIT_VALUE_V1_SIZE
        || value.reserved != 0
        || value.number_reserved != 0.0
    {
        return Err(EvimStatus::InvalidArgument);
    }
    let invalid = || EvimStatus::InvalidStyleValue;
    match property {
        StyleProperty::CanvasPaddingTop
        | StyleProperty::CanvasPaddingRight
        | StyleProperty::CanvasPaddingBottom
        | StyleProperty::CanvasPaddingLeft
        | StyleProperty::ParagraphSpacingBefore
        | StyleProperty::ParagraphSpacingAfter
        | StyleProperty::ParagraphFirstLineIndent
        | StyleProperty::ParagraphLeadingIndent
        | StyleProperty::ParagraphTrailingIndent
        | StyleProperty::CharacterSize
        | StyleProperty::CharacterLetterSpacing
        | StyleProperty::CharacterBaselineShift => {
            if value.kind != EVIM_STYLE_VALUE_FLOAT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::Float(value.number))
        }
        StyleProperty::CharacterWeight => {
            if value.kind != EVIM_STYLE_VALUE_UNSIGNED {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::FontWeight(
                u16::try_from(value.enum_value).map_err(|_| invalid())?,
            ))
        }
        StyleProperty::CharacterBold
        | StyleProperty::CharacterUnderline
        | StyleProperty::CharacterStrikethrough => {
            if value.kind != EVIM_STYLE_VALUE_BOOLEAN || value.enum_value > 1 {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::Boolean(value.enum_value != 0))
        }
        StyleProperty::CanvasBackground
        | StyleProperty::CharacterForeground
        | StyleProperty::CharacterBackground => {
            if value.kind != EVIM_STYLE_VALUE_COLOR {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            Ok(StylePropertyValue::Color(Color {
                red: value.color.red,
                green: value.color.green,
                blue: value.color.blue,
                alpha: value.color.alpha,
            }))
        }
        StyleProperty::CharacterLanguage => {
            if value.kind != EVIM_STYLE_VALUE_STRING {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            Ok(StylePropertyValue::Text(unsafe {
                composition_utf8(value.text, out_outcome)?
            }))
        }
        StyleProperty::CharacterFontFamilies => {
            if value.kind != EVIM_STYLE_VALUE_STRING_LIST {
                return Err(invalid());
            }
            style_edit_value_has_no_text(value)?;
            let items = unsafe { parse_style_edit_items(value, out_outcome)? };
            if items.iter().any(|(kind, text, unsigned)| {
                *kind != EVIM_STYLE_VALUE_ITEM_STRING || text.is_empty() || *unsigned != 0
            }) {
                return Err(invalid());
            }
            Ok(StylePropertyValue::FontFamilies(
                items.into_iter().map(|(_, text, _)| text).collect(),
            ))
        }
        StyleProperty::CharacterSlant => {
            if value.kind != EVIM_STYLE_VALUE_FONT_SLANT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let slant = match value.enum_value {
                EVIM_FONT_SLANT_UPRIGHT => FontSlant::Upright,
                EVIM_FONT_SLANT_ITALIC => FontSlant::Italic,
                EVIM_FONT_SLANT_OBLIQUE => FontSlant::Oblique,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::FontSlant(slant))
        }
        StyleProperty::CharacterDirection | StyleProperty::ParagraphBaseDirection => {
            if value.kind != EVIM_STYLE_VALUE_WRITING_DIRECTION {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let direction = match value.enum_value {
                EVIM_TEXT_DIRECTION_AUTO => WritingDirection::Natural,
                EVIM_TEXT_DIRECTION_LEFT_TO_RIGHT => WritingDirection::LeftToRight,
                EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT => WritingDirection::RightToLeft,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::WritingDirection(direction))
        }
        StyleProperty::CharacterOpenTypeFeatures => {
            if value.kind != EVIM_STYLE_VALUE_OPEN_TYPE_FEATURES {
                return Err(invalid());
            }
            style_edit_value_has_no_text(value)?;
            let items = unsafe { parse_style_edit_items(value, out_outcome)? };
            let mut features = std::collections::BTreeMap::new();
            for (kind, tag, setting) in items {
                if kind != EVIM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE
                    || tag.len() != 4
                    || !tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
                    || features.insert(tag, setting).is_some()
                {
                    return Err(invalid());
                }
            }
            Ok(StylePropertyValue::OpenTypeFeatures(features))
        }
        StyleProperty::ParagraphLineSpacing => {
            if value.kind != EVIM_STYLE_VALUE_LINE_SPACING {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let spacing = match value.enum_value {
                EVIM_STYLE_LINE_SPACING_NORMAL => LineSpacing::Normal,
                EVIM_STYLE_LINE_SPACING_MULTIPLIER => LineSpacing::Multiplier(value.number),
                EVIM_STYLE_LINE_SPACING_AT_LEAST => LineSpacing::AtLeast(value.number),
                EVIM_STYLE_LINE_SPACING_EXACT => LineSpacing::Exact(value.number),
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::LineSpacing(spacing))
        }
        StyleProperty::ParagraphAlignment => {
            if value.kind != EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT {
                return Err(invalid());
            }
            style_edit_value_has_no_array(value)?;
            style_edit_value_has_no_text(value)?;
            let alignment = match value.enum_value {
                EVIM_STYLE_PARAGRAPH_ALIGNMENT_START => ParagraphAlignment::Start,
                EVIM_STYLE_PARAGRAPH_ALIGNMENT_END => ParagraphAlignment::End,
                EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER => ParagraphAlignment::Center,
                _ => return Err(invalid()),
            };
            Ok(StylePropertyValue::ParagraphAlignment(alignment))
        }
    }
}

unsafe fn parse_style_edit_request(
    request: *const EvimStyleEditV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> Result<ParsedStyleEdit, EvimStatus> {
    let request = unsafe { read_core_request(request, out_outcome)? };
    if request.struct_size < EVIM_STYLE_EDIT_V1_SIZE
        || request.flags != 0
        || request.reserved != 0
        || request.identity.struct_size < EVIM_STYLE_SHEET_IDENTITY_V1_SIZE
        || request.identity.reserved != 0
    {
        return Err(EvimStatus::InvalidArgument);
    }
    let namespace = parse_style_namespace(request.namespace)?;
    let style = unsafe { composition_utf8(request.style_id, out_outcome)? };
    if style.is_empty() || style.contains('\0') {
        return Err(EvimStatus::InvalidArgument);
    }
    let relationship_value = || unsafe { composition_utf8(request.value.text, out_outcome) };
    let require_empty_value = || {
        if request.value.struct_size < EVIM_STYLE_EDIT_VALUE_V1_SIZE
            || request.value.kind != EVIM_STYLE_VALUE_NONE
            || request.value.reserved != 0
            || request.value.item_count != 0
            || request.value.text.length != 0
        {
            Err(EvimStatus::InvalidStyleValue)
        } else {
            Ok(())
        }
    };
    let edit = match request.operation {
        EVIM_STYLE_EDIT_SET_DECLARATION => {
            let property = parse_style_property(request.property)?;
            StyleDefinitionFieldEdit::SetDeclaration {
                property,
                value: unsafe {
                    parse_style_property_value(property, &request.value, out_outcome)?
                },
            }
        }
        EVIM_STYLE_EDIT_CLEAR_DECLARATION => {
            let property = parse_style_property(request.property)?;
            require_empty_value()?;
            StyleDefinitionFieldEdit::ClearDeclaration(property)
        }
        EVIM_STYLE_EDIT_SET_PARENT
        | EVIM_STYLE_EDIT_SET_NEXT_STYLE
        | EVIM_STYLE_EDIT_SET_DISPLAY_NAME => {
            if request.property != 0
                || request.value.struct_size < EVIM_STYLE_EDIT_VALUE_V1_SIZE
                || request.value.kind != EVIM_STYLE_VALUE_STRING
                || request.value.reserved != 0
                || request.value.item_count != 0
            {
                return Err(EvimStatus::InvalidStyleValue);
            }
            let target = relationship_value()?;
            if target.is_empty() || target.contains('\0') {
                return Err(EvimStatus::InvalidStyleRelationship);
            }
            match request.operation {
                EVIM_STYLE_EDIT_SET_PARENT => {
                    StyleDefinitionFieldEdit::SetParent(Some(StyleId(target)))
                }
                EVIM_STYLE_EDIT_SET_NEXT_STYLE => {
                    StyleDefinitionFieldEdit::SetNextParagraphStyle(Some(StyleId(target)))
                }
                EVIM_STYLE_EDIT_SET_DISPLAY_NAME => {
                    StyleDefinitionFieldEdit::SetDisplayName(target)
                }
                _ => unreachable!("the operation was matched above"),
            }
        }
        EVIM_STYLE_EDIT_CLEAR_PARENT | EVIM_STYLE_EDIT_CLEAR_NEXT_STYLE => {
            if request.property != 0 {
                return Err(EvimStatus::InvalidStyleRelationship);
            }
            require_empty_value()?;
            if request.operation == EVIM_STYLE_EDIT_CLEAR_PARENT {
                StyleDefinitionFieldEdit::SetParent(None)
            } else {
                StyleDefinitionFieldEdit::SetNextParagraphStyle(None)
            }
        }
        _ => return Err(EvimStatus::InvalidArgument),
    };
    Ok(ParsedStyleEdit {
        identity: request.identity,
        namespace,
        style: StyleId(style),
        edit,
    })
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
    let outcome = core.handle_with_layout(view, event).map_err(core_status)?;
    summarize_core_outcome(core, view, Some(&outcome))
}

fn dispatch_input_with_effects(
    core: &mut Core<CTextMeasurementProvider>,
    view: EvimViewId,
    input: InputEvent,
    clipboard: ClipboardCommandContext,
) -> Result<(EvimCoreOutcomeV1, Option<OwnedEffectBatch>), EvimStatus> {
    let view_id = ViewId(view);
    let effect_clipboard = clipboard.clone();
    let outcome = core
        .handle_with_layout(view_id, CoreEvent::InputWithClipboard { input, clipboard })
        .map_err(core_status)?;
    let mut summary = summarize_core_outcome(core, view_id, Some(&outcome))?;
    let effects = OwnedEffectBatch::from_command(
        core.document(),
        core.command_state(view_id).ok_or(EvimStatus::InvalidView)?,
        &effect_clipboard,
        outcome.command,
    );
    if effects.is_some() {
        summary.flags |= EVIM_OUTCOME_HAS_EXTERNAL_EFFECTS;
    }
    Ok((summary, effects))
}

fn publish_input_turn(
    handle: EvimCoreHandle,
    view: EvimViewId,
    input: InputEvent,
    clipboard: ClipboardCommandContext,
    reservation: EffectBatchReservation,
) -> Result<(EvimCoreOutcomeV1, EvimEffectBatchHandle), EvimStatus> {
    let (outcome, effects) = with_core_mut(handle, |core| {
        dispatch_input_with_effects(core, view, input, clipboard)
    })?;
    let effect_handle = match effects {
        Some(effects) => reservation.commit(effects)?,
        None => {
            drop(reservation);
            0
        }
    };
    Ok((outcome, effect_handle))
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

/// Atomically query immutable document, pipeline, and history metadata.
///
/// # Safety
///
/// `out_state` must identify one aligned writable state value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_document_state(
    handle: EvimCoreHandle,
    out_state: *mut EvimDocumentStateV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_state, 1)?;
        unsafe { out_state.write(EvimDocumentStateV1::default()) };
        let state = with_core(handle, |core| Ok(summarize_document_state(core.document())))?;
        unsafe { out_state.write(state) };
        Ok(())
    })
}

/// Query aggregate lengths and hard-line count for the current immutable
/// formatted projection without flattening its persistent UTF-8 tree.
///
/// # Safety
///
/// `out_info` must identify one aligned writable v1 record.
#[no_mangle]
pub unsafe extern "C" fn evim_core_formatted_snapshot_info(
    handle: EvimCoreHandle,
    out_info: *mut EvimFormattedSnapshotInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimFormattedSnapshotInfoV1::default()) };
        let info = with_core(handle, |core| formatted_snapshot_info(core.document()))?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Copy one exact scalar-aligned, half-open formatted UTF-8 range.
///
/// This is an all-or-none two-pass operation: `out_required` receives the
/// complete byte count after the identity and range validate. If `output` is
/// too small, no output byte is written and `BUFFER_TOO_SMALL` is returned.
/// The query visits only persistent text-tree leaves intersecting the range.
///
/// # Safety
///
/// `request` and `out_required` must identify readable/writable aligned
/// values. A nonzero capacity requires a writable output region. Writable
/// regions must not overlap each other or the request.
#[no_mangle]
pub unsafe extern "C" fn evim_core_copy_formatted_utf8_range(
    handle: EvimCoreHandle,
    request: *const EvimFormattedUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(output, output_capacity)?;
        let required_region = typed_pointer_region(out_required, 1)?;
        if regions_overlap(output_region, request_region)
            || regions_overlap(output_region, required_region)
            || regions_overlap(required_region, request_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_FORMATTED_UTF8_RANGE_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < EVIM_FORMATTED_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let start = usize::try_from(request.utf8_start).map_err(|_| EvimStatus::LengthOverflow)?;
        let end = usize::try_from(request.utf8_end).map_err(|_| EvimStatus::LengthOverflow)?;
        let bytes = with_core(handle, |core| {
            validate_formatted_snapshot_identity(request.identity, core.document())?;
            core.document()
                .hard_line_snapshot()
                .slice_utf8(start..end)
                .map(String::into_bytes)
                .map_err(formatted_text_status)
        })?;
        let required = checked_export_count(bytes.len())?;
        unsafe { out_required.write(required) };
        let capacity = checked_length(output_capacity)?;
        if capacity < bytes.len() {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            // SAFETY: Pointer shape, capacity, and all overlaps were validated
            // before the immutable snapshot was queried.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        }
        Ok(())
    })
}

#[derive(Clone, Copy)]
enum FormattedOffsetMapping {
    Utf8ToUtf16,
    Utf16ToUtf8,
}

#[derive(Clone, Copy)]
struct FormattedOffsetMappingBuffers {
    identity: *const EvimFormattedSnapshotIdentityV1,
    input_offsets: *const u64,
    input_count: u64,
    output_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
}

unsafe fn map_formatted_offsets(
    handle: EvimCoreHandle,
    buffers: FormattedOffsetMappingBuffers,
    mapping: FormattedOffsetMapping,
) -> EvimStatus {
    ffi_boundary(|| {
        let identity_region = typed_pointer_region(buffers.identity, 1)?;
        let input_region = typed_pointer_region(buffers.input_offsets, buffers.input_count)?;
        let output_region = typed_pointer_region(buffers.output_offsets, buffers.output_capacity)?;
        let required_region = typed_pointer_region(buffers.out_required, 1)?;
        if regions_overlap(output_region, identity_region)
            || regions_overlap(output_region, input_region)
            || regions_overlap(output_region, required_region)
            || regions_overlap(required_region, identity_region)
            || regions_overlap(required_region, input_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let identity = unsafe { read_formatted_snapshot_identity(buffers.identity)? };
        let count = checked_length(buffers.input_count)?;
        let inputs = if count == 0 {
            Vec::new()
        } else {
            // SAFETY: The caller supplies a readable region for the duration
            // of the call. Copying before any output write also makes aliased
            // caller mutations unable to produce a partial result.
            unsafe { slice::from_raw_parts(buffers.input_offsets, count) }.to_vec()
        };
        unsafe { buffers.out_required.write(0) };
        let outputs = with_core(handle, |core| {
            validate_formatted_snapshot_identity(identity, core.document())?;
            let snapshot = core.document().hard_line_snapshot();
            inputs
                .iter()
                .map(|offset| {
                    let offset =
                        usize::try_from(*offset).map_err(|_| EvimStatus::LengthOverflow)?;
                    let mapped = match mapping {
                        FormattedOffsetMapping::Utf8ToUtf16 => {
                            snapshot.utf16_offset_for_utf8(offset)
                        }
                        FormattedOffsetMapping::Utf16ToUtf8 => {
                            snapshot.utf8_offset_for_utf16(offset)
                        }
                    }
                    .map_err(formatted_text_status)?;
                    checked_export_count(mapped)
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        let required = checked_export_count(outputs.len())?;
        unsafe { buffers.out_required.write(required) };
        if checked_length(buffers.output_capacity)? < outputs.len() {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !outputs.is_empty() {
            // SAFETY: The complete batch was validated and materialized before
            // this single nonoverlapping copy.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    outputs.as_ptr(),
                    buffers.output_offsets,
                    outputs.len(),
                )
            };
        }
        Ok(())
    })
}

/// Batch-map exact UTF-8 scalar boundaries to UTF-16 code-unit offsets.
///
/// No output element is written unless the complete identity-bound input
/// batch validates and the output capacity is sufficient.
///
/// # Safety
///
/// The identity and nonempty input array must be readable and aligned;
/// `out_required` and a nonempty output array must be writable and aligned.
/// Writable regions must not overlap inputs or each other.
#[no_mangle]
pub unsafe extern "C" fn evim_core_map_formatted_utf8_to_utf16(
    handle: EvimCoreHandle,
    identity: *const EvimFormattedSnapshotIdentityV1,
    utf8_offsets: *const u64,
    offset_count: u64,
    utf16_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    unsafe {
        map_formatted_offsets(
            handle,
            FormattedOffsetMappingBuffers {
                identity,
                input_offsets: utf8_offsets,
                input_count: offset_count,
                output_offsets: utf16_offsets,
                output_capacity,
                out_required,
            },
            FormattedOffsetMapping::Utf8ToUtf16,
        )
    }
}

/// Batch-map exact UTF-16 code-unit boundaries to UTF-8 scalar boundaries.
/// A surrogate-pair interior returns `INVALID_UTF16_BOUNDARY` with no partial
/// output.
///
/// # Safety
///
/// Pointer requirements are identical to
/// [`evim_core_map_formatted_utf8_to_utf16`].
#[no_mangle]
pub unsafe extern "C" fn evim_core_map_formatted_utf16_to_utf8(
    handle: EvimCoreHandle,
    identity: *const EvimFormattedSnapshotIdentityV1,
    utf16_offsets: *const u64,
    offset_count: u64,
    utf8_offsets: *mut u64,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    unsafe {
        map_formatted_offsets(
            handle,
            FormattedOffsetMappingBuffers {
                identity,
                input_offsets: utf16_offsets,
                input_count: offset_count,
                output_offsets: utf8_offsets,
                output_capacity,
                out_required,
            },
            FormattedOffsetMapping::Utf16ToUtf8,
        )
    }
}

/// Resolve line and logical-grapheme status metadata for one exact formatted
/// point. The point must be a logical grapheme boundary, not merely a UTF-8
/// scalar boundary.
///
/// # Safety
///
/// `identity` and `out_info` must identify distinct aligned readable/writable
/// v1 records.
#[no_mangle]
pub unsafe extern "C" fn evim_core_formatted_point_info(
    handle: EvimCoreHandle,
    identity: *const EvimFormattedSnapshotIdentityV1,
    utf8_offset: u64,
    out_info: *mut EvimFormattedPointInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let identity_region = typed_pointer_region(identity, 1)?;
        let output_region = typed_pointer_region(out_info, 1)?;
        if regions_overlap(identity_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let identity = unsafe { read_formatted_snapshot_identity(identity)? };
        unsafe { out_info.write(EvimFormattedPointInfoV1::default()) };
        let utf8_offset = usize::try_from(utf8_offset).map_err(|_| EvimStatus::LengthOverflow)?;
        let info = with_core(handle, |core| {
            validate_formatted_snapshot_identity(identity, core.document())?;
            let snapshot = core.document().hard_line_snapshot();
            let line = snapshot
                .line_at_offset(utf8_offset)
                .map_err(hard_line_query_status)?;
            if !snapshot.is_grapheme_boundary(utf8_offset) {
                return Err(EvimStatus::NotGraphemeBoundary);
            }
            let line_range = line.content_range();
            let grapheme_column = snapshot
                .grapheme_count(line_range.start..utf8_offset)
                .ok_or(EvimStatus::CoreFailure)?;
            Ok(EvimFormattedPointInfoV1 {
                struct_size: EVIM_FORMATTED_POINT_INFO_V1_SIZE,
                reserved: 0,
                identity: formatted_snapshot_identity(core.document()),
                utf8_offset: checked_export_count(utf8_offset)?,
                utf16_offset: checked_export_count(
                    snapshot
                        .utf16_offset_for_utf8(utf8_offset)
                        .map_err(formatted_text_status)?,
                )?,
                hard_line_index: checked_export_count(line.index())?,
                hard_line_start: checked_export_count(line_range.start)?,
                hard_line_end: checked_export_count(line_range.end)?,
                grapheme_column: checked_export_count(grapheme_column)?,
            })
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Establish the exact current source snapshot as the native save point.
///
/// # Safety
///
/// `request` must identify one aligned readable v1 request.
#[no_mangle]
pub unsafe extern "C" fn evim_core_mark_saved(
    handle: EvimCoreHandle,
    request: *const EvimMarkSavedV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(request, 1)?;
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_MARK_SAVED_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            core.mark_saved(
                DocumentId(request.document_id),
                Revision(request.document_revision),
            )
            .map_err(core_status)
        })
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

/// Read exact sizes and identity for the current immutable normalized style
/// sheet. The companion copy call validates this identity before copying.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_style_sheet_info(
    handle: EvimCoreHandle,
    out_info: *mut EvimStyleSheetInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimStyleSheetInfoV1::default()) };
        let info = with_core(handle, |core| Ok(export_style_sheet(core.document())?.info))?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Atomically copy an exact immutable style sheet into typed arrays and one
/// UTF-8 arena. A zero-capacity/null-buffer call is the supported size query.
/// No definition, property, value item, dependency, or string byte is written
/// unless every capacity is sufficient.
///
/// # Safety
///
/// `expected` and `out_info` must identify aligned readable/writable values.
/// Each nonzero-capacity output must identify that many aligned writable
/// elements or bytes. All input/output regions must be pairwise disjoint.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn evim_core_copy_style_sheet(
    handle: EvimCoreHandle,
    expected: *const EvimStyleSheetIdentityV1,
    definitions: *mut EvimStyleDefinitionV1,
    definition_capacity: u64,
    properties: *mut EvimStylePropertyV1,
    property_capacity: u64,
    value_items: *mut EvimStyleValueItemV1,
    value_item_capacity: u64,
    dependencies: *mut EvimStyleDependencyV1,
    dependency_capacity: u64,
    string_bytes: *mut u8,
    string_capacity: u64,
    out_info: *mut EvimStyleSheetInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(definitions, definition_capacity)?,
            typed_pointer_region(properties, property_capacity)?,
            typed_pointer_region(value_items, value_item_capacity)?,
            typed_pointer_region(dependencies, dependency_capacity)?,
            typed_pointer_region(string_bytes, string_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        for left in 0..regions.len() {
            for right in left + 1..regions.len() {
                if regions_overlap(regions[left], regions[right]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let expected = unsafe { read_style_sheet_identity(expected)? };
        unsafe { out_info.write(EvimStyleSheetInfoV1::default()) };
        let export = with_core(handle, |core| {
            validate_style_sheet_identity(expected, core.document())?;
            export_style_sheet(core.document())
        })?;
        unsafe { out_info.write(export.info) };
        let fits = definition_capacity >= export.info.definition_count
            && property_capacity >= export.info.property_count
            && value_item_capacity >= export.info.value_item_count
            && dependency_capacity >= export.info.dependency_count
            && string_capacity >= export.info.string_bytes;
        if !fits {
            return Err(EvimStatus::BufferTooSmall);
        }
        unsafe {
            if !export.definitions.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.definitions.as_ptr(),
                    definitions,
                    export.definitions.len(),
                );
            }
            if !export.properties.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.properties.as_ptr(),
                    properties,
                    export.properties.len(),
                );
            }
            if !export.value_items.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.value_items.as_ptr(),
                    value_items,
                    export.value_items.len(),
                );
            }
            if !export.dependencies.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.dependencies.as_ptr(),
                    dependencies,
                    export.dependencies.len(),
                );
            }
            if !export.strings.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.strings.as_ptr(),
                    string_bytes,
                    export.strings.len(),
                );
            }
        }
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

/// Read a fixed-size summary of the view's current immutable layout.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_layout_snapshot_info(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_info: *mut EvimLayoutSnapshotInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimLayoutSnapshotInfoV1::default()) };
        let info = with_core(handle, |core| {
            let view_id = ViewId(view);
            layout_snapshot_info(current_ffi_layout_snapshot(core, view_id)?, view_id)
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Read canvas and default text paint plus the required override-run count
/// from the view's current immutable layout snapshot.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_layout_paint_info(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_info: *mut EvimLayoutPaintInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimLayoutPaintInfoV1::default()) };
        let info = with_core(handle, |core| {
            let view_id = ViewId(view);
            layout_paint_info(current_ffi_layout_snapshot(core, view_id)?, view_id)
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Atomically copy ordered paint override runs from one exact immutable layout
/// snapshot. A zero-capacity/null-buffer call is the supported count query. No
/// run is written when capacity is insufficient.
///
/// # Safety
///
/// `expected` and `out_info` must identify aligned readable/writable values. A
/// nonzero-capacity output must identify that many aligned writable runs. All
/// regions must be pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_layout_paint(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimLayoutSnapshotIdentityV1,
    runs: *mut EvimPaintStyleRunV1,
    run_capacity: u64,
    out_info: *mut EvimLayoutPaintInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let run_region = typed_pointer_region(runs, run_capacity)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        for (left, right) in [
            (expected_region, run_region),
            (expected_region, info_region),
            (run_region, info_region),
        ] {
            if regions_overlap(left, right) {
                return Err(EvimStatus::InvalidArgument);
            }
        }
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(EvimLayoutPaintInfoV1::default()) };
        let (info, export) = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(expected, snapshot, view_id)?;
            let info = layout_paint_info(snapshot, view_id)?;
            let export = (run_capacity >= info.paint_run_count)
                .then(|| export_layout_paint(snapshot, view_id))
                .transpose()?;
            Ok((info, export))
        })?;
        unsafe { out_info.write(info) };
        let Some(export) = export else {
            return Err(EvimStatus::BufferTooSmall);
        };
        debug_assert_eq!(export.info, info);
        unsafe {
            if !export.runs.is_empty() {
                std::ptr::copy_nonoverlapping(export.runs.as_ptr(), runs, export.runs.len());
            }
        }
        Ok(())
    })
}

/// Copy noneditable list-marker furniture from one exact layout. Null buffers
/// with zero capacity query counts; insufficient capacity writes only info.
///
/// # Safety
/// Pointer regions must be aligned, writable to their capacities and disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_layout_decorations(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimLayoutSnapshotIdentityV1,
    decorations: *mut EvimLayoutDecorationV1,
    decoration_capacity: u64,
    labels: *mut u8,
    label_capacity: u64,
    out_info: *mut EvimLayoutDecorationsInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(decorations, decoration_capacity)?,
            typed_pointer_region(labels, label_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        for (index, left) in regions.iter().enumerate() {
            for right in &regions[index + 1..] {
                if regions_overlap(*left, *right) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(EvimLayoutDecorationsInfoV1::default()) };
        let (info, values, bytes) = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(expected, snapshot, view_id)?;
            let count = snapshot.rows.iter().map(|row| row.decorations.len()).sum();
            let length = snapshot
                .rows
                .iter()
                .flat_map(|row| &row.decorations)
                .map(|item| item.text.len())
                .sum();
            let info = EvimLayoutDecorationsInfoV1 {
                struct_size: EVIM_LAYOUT_DECORATIONS_INFO_V1_SIZE,
                reserved: 0,
                identity: snapshot_identity(snapshot, view_id),
                decoration_count: checked_export_count(count)?,
                label_bytes: checked_export_count(length)?,
            };
            let mut values = Vec::new();
            let mut bytes = Vec::new();
            if decoration_capacity >= info.decoration_count && label_capacity >= info.label_bytes {
                values.reserve(count);
                bytes.reserve(length);
                for (row_index, row) in snapshot.rows.iter().enumerate() {
                    for item in &row.decorations {
                        values.push(EvimLayoutDecorationV1 {
                            struct_size: EVIM_LAYOUT_DECORATION_V1_SIZE,
                            flags: (if item.render_run.is_some() {
                                EVIM_POSITIONED_CLUSTER_HAS_RENDER_RUN
                            } else {
                                0
                            }) | if item.kind == crate::layout::DecorationKind::BlockQuoteBorder {
                                EVIM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER
                            } else { 0 },
                            row_index: checked_export_count(row_index)?,
                            label_byte_start: checked_export_count(bytes.len())?,
                            label_byte_length: checked_export_count(item.text.len())?,
                            x: item.x,
                            advance: item.advance,
                            font_size: item.font_size,
                            reserved: 0.0,
                            typographic_bounds: layout_rect_to_ffi(item.typographic_bounds),
                            ink_bounds: layout_rect_to_ffi(item.ink_bounds),
                            render_run: item.render_run.map(render_run_to_ffi).unwrap_or_default(),
                            paint: text_paint_to_ffi(&item.paint),
                        });
                        bytes.extend_from_slice(item.text.as_bytes());
                    }
                }
            }
            Ok((info, values, bytes))
        })?;
        unsafe { out_info.write(info) };
        if decoration_capacity < info.decoration_count || label_capacity < info.label_bytes {
            return Err(EvimStatus::BufferTooSmall);
        }
        unsafe {
            if !values.is_empty() {
                std::ptr::copy_nonoverlapping(values.as_ptr(), decorations, values.len());
            }
            if !bytes.is_empty() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), labels, bytes.len());
            }
        }
        Ok(())
    })
}

/// Atomically copy positioned rows, shaped clusters, and legal caret stops
/// from one exact immutable layout snapshot. A zero-capacity/null-buffer call
/// is the supported count query. No array element is written unless every
/// supplied capacity is sufficient.
///
/// # Safety
///
/// `expected` and `out_info` must identify aligned readable/writable values.
/// Each nonzero-capacity output must identify that many aligned writable
/// elements. All input and output regions must be pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_layout_snapshot(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimLayoutSnapshotIdentityV1,
    rows: *mut EvimVisualRowV1,
    row_capacity: u64,
    clusters: *mut EvimPositionedClusterV1,
    cluster_capacity: u64,
    carets: *mut EvimPositionedCaretV1,
    caret_capacity: u64,
    out_info: *mut EvimLayoutSnapshotInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        let row_region = typed_pointer_region(rows, row_capacity)?;
        let cluster_region = typed_pointer_region(clusters, cluster_capacity)?;
        let caret_region = typed_pointer_region(carets, caret_capacity)?;
        let regions = [
            expected_region,
            info_region,
            row_region,
            cluster_region,
            caret_region,
        ];
        for left in 0..regions.len() {
            for right in left + 1..regions.len() {
                if regions_overlap(regions[left], regions[right]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let expected = unsafe { read_layout_identity(expected)? };
        unsafe { out_info.write(EvimLayoutSnapshotInfoV1::default()) };
        let (info, export) = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(expected, snapshot, view_id)?;
            let info = layout_snapshot_info(snapshot, view_id)?;
            let fits = row_capacity >= info.row_count
                && cluster_capacity >= info.cluster_count
                && caret_capacity >= info.caret_count;
            let export = fits
                .then(|| export_layout_snapshot(snapshot, view_id))
                .transpose()?;
            Ok((info, export))
        })?;
        unsafe { out_info.write(info) };
        let Some(export) = export else {
            return Err(EvimStatus::BufferTooSmall);
        };
        debug_assert_eq!(export.info, info);
        unsafe {
            if !export.rows.is_empty() {
                std::ptr::copy_nonoverlapping(export.rows.as_ptr(), rows, export.rows.len());
            }
            if !export.clusters.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.clusters.as_ptr(),
                    clusters,
                    export.clusters.len(),
                );
            }
            if !export.carets.is_empty() {
                std::ptr::copy_nonoverlapping(export.carets.as_ptr(), carets, export.carets.len());
            }
        }
        Ok(())
    })
}

/// Resolve one logical endpoint against one exact immutable layout snapshot.
///
/// # Safety
///
/// `request` and `out_geometry` must identify aligned, disjoint readable and
/// writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_caret_geometry(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimLayoutCaretRequestV1,
    out_geometry: *mut EvimLayoutCaretGeometryV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(out_geometry, 1)?;
        if regions_overlap(request_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_LAYOUT_CARET_REQUEST_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        if request.identity.struct_size < EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let affinity = parse_layout_affinity(request.affinity)?;
        let text_offset =
            usize::try_from(request.text_offset).map_err(|_| EvimStatus::LengthOverflow)?;
        unsafe { out_geometry.write(EvimLayoutCaretGeometryV1::default()) };
        let geometry = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(request.identity, snapshot, view_id)?;
            snapshot
                .logical_endpoint_geometry(text_offset, affinity)
                .map_err(layout_query_status)
        })?;
        let mut flags = 0;
        if geometry.is_cluster_fallback {
            flags |= EVIM_CARET_GEOMETRY_CLUSTER_FALLBACK;
        }
        let geometry = EvimLayoutCaretGeometryV1 {
            struct_size: EVIM_LAYOUT_CARET_GEOMETRY_V1_SIZE,
            flags,
            point: caret_point_to_ffi(geometry.point)?,
            rect: layout_rect_to_ffi(geometry.rect),
            row_index: checked_export_count(geometry.row_index)?,
        };
        unsafe { out_geometry.write(geometry) };
        Ok(())
    })
}

/// Hit test one document-layout coordinate against one exact immutable layout
/// snapshot.
///
/// # Safety
///
/// `request` and `out_point` must identify aligned, disjoint readable and
/// writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_layout_hit_test(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimLayoutHitTestRequestV1,
    out_point: *mut EvimLayoutCaretPointV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(out_point, 1)?;
        if regions_overlap(request_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_LAYOUT_HIT_TEST_REQUEST_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < EVIM_LAYOUT_SNAPSHOT_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_point.write(EvimLayoutCaretPointV1::default()) };
        let point = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(request.identity, snapshot, view_id)?;
            snapshot
                .hit_test(LayoutPoint {
                    x: request.x,
                    y: request.y,
                })
                .map_err(layout_query_status)
        })?;
        unsafe { out_point.write(caret_point_to_ffi(point)?) };
        Ok(())
    })
}

/// Read current cursor, Visual-selection, desired-x, and command-line
/// presentation metadata without requesting layout.
///
/// # Safety
///
/// `out_presentation` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_presentation(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_presentation: *mut EvimViewPresentationV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_presentation, 1)?;
        unsafe { out_presentation.write(EvimViewPresentationV1::default()) };
        let presentation = with_core(handle, |core| {
            summarize_view_presentation(core, ViewId(view))
        })?;
        unsafe { out_presentation.write(presentation) };
        Ok(())
    })
}

/// Read a fixed-size summary of the current editable command-line contents.
/// The prompt prefix is represented by the returned kind and is not included
/// in the byte count.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_command_line_info(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_info: *mut EvimCommandLineInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimCommandLineInfoV1::default()) };
        let export = with_core(handle, |core| export_command_line(core, ViewId(view)))?;
        unsafe { out_info.write(export.info) };
        Ok(())
    })
}

/// Atomically copy the current command-line UTF-8 bytes for one exact state
/// identity. A zero-capacity/null-buffer call is the supported count query.
/// No byte is written when capacity is insufficient.
///
/// # Safety
///
/// `expected` and `out_info` must identify aligned readable/writable values.
/// A nonzero-capacity byte output must identify that many writable bytes. All
/// regions must be pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_command_line(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimCommandLineIdentityV1,
    utf8: *mut u8,
    utf8_capacity: u64,
    out_info: *mut EvimCommandLineInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let utf8_region = typed_pointer_region(utf8, utf8_capacity)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        for (left, right) in [
            (expected_region, utf8_region),
            (expected_region, info_region),
            (utf8_region, info_region),
        ] {
            if regions_overlap(left, right) {
                return Err(EvimStatus::InvalidArgument);
            }
        }
        let expected = unsafe { read_command_line_identity(expected)? };
        unsafe { out_info.write(EvimCommandLineInfoV1::default()) };
        let export = with_core(handle, |core| {
            let export = export_command_line(core, ViewId(view))?;
            validate_command_line_identity(expected, export.info.identity)?;
            Ok(export)
        })?;
        unsafe { out_info.write(export.info) };
        if utf8_capacity < export.info.utf8_length {
            return Err(EvimStatus::BufferTooSmall);
        }
        unsafe {
            if !export.bytes.is_empty() {
                std::ptr::copy_nonoverlapping(export.bytes.as_ptr(), utf8, export.bytes.len());
            }
        }
        Ok(())
    })
}

/// Read required logical-segment and drawable-rectangle counts for the exact
/// current Visual selection and layout. A non-Visual view returns kind NONE
/// with zero counts and the current layout identity.
///
/// # Safety
///
/// `out_info` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_visual_selection_info(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_info: *mut EvimVisualSelectionInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimVisualSelectionInfoV1::default()) };
        let export = with_core(handle, |core| export_visual_selection(core, ViewId(view)))?;
        unsafe { out_info.write(export.info) };
        Ok(())
    })
}

/// Atomically copy the ordered logical UTF-8 segments and exact-layout
/// rectangles for one Visual-selection identity. A zero-capacity/null-buffer
/// call is the supported count query. No array element is written unless both
/// capacities are sufficient.
///
/// # Safety
///
/// `expected` and `out_info` must identify aligned readable/writable values.
/// Each nonzero-capacity output must identify that many aligned writable
/// elements. All input and output regions must be pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_visual_selection(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimVisualSelectionIdentityV1,
    segments: *mut EvimVisualSelectionSegmentV1,
    segment_capacity: u64,
    rectangles: *mut EvimVisualSelectionRectangleV1,
    rectangle_capacity: u64,
    out_info: *mut EvimVisualSelectionInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let segment_region = typed_pointer_region(segments, segment_capacity)?;
        let rectangle_region = typed_pointer_region(rectangles, rectangle_capacity)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        let regions = [
            expected_region,
            segment_region,
            rectangle_region,
            info_region,
        ];
        for left in 0..regions.len() {
            for right in left + 1..regions.len() {
                if regions_overlap(regions[left], regions[right]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let expected = unsafe { read_visual_selection_identity(expected)? };
        unsafe { out_info.write(EvimVisualSelectionInfoV1::default()) };
        let export = with_core(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(expected.layout, snapshot, view_id)?;
            let export = export_visual_selection(core, view_id)?;
            validate_visual_selection_identity(expected, export.info.identity, snapshot, view_id)?;
            Ok(export)
        })?;
        unsafe { out_info.write(export.info) };
        if segment_capacity < export.info.segment_count
            || rectangle_capacity < export.info.rectangle_count
        {
            return Err(EvimStatus::BufferTooSmall);
        }
        unsafe {
            if !export.segments.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.segments.as_ptr(),
                    segments,
                    export.segments.len(),
                );
            }
            if !export.rectangles.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.rectangles.as_ptr(),
                    rectangles,
                    export.rectangles.len(),
                );
            }
        }
        Ok(())
    })
}

/// Query Bold/Italic check state and exact adapter capability from the current
/// core-owned logical selection. This call never requires a layout snapshot.
/// Unsupported adapters and selection shapes return a successful disabled
/// presentation rather than an operation error.
///
/// # Safety
///
/// `out_presentation` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_semantic_style_presentation(
    handle: EvimCoreHandle,
    view: EvimViewId,
    style: u32,
    out_presentation: *mut EvimSemanticStylePresentationV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_presentation, 1)?;
        unsafe { out_presentation.write(EvimSemanticStylePresentationV1::default()) };
        let style = semantic_style_from_ffi(style)?;
        let (presentation, _) = with_core(handle, |core| {
            export_semantic_style_presentation(core, ViewId(view), style)
        })?;
        unsafe { out_presentation.write(presentation) };
        Ok(())
    })
}

/// Set or clear Markdown Strong/Emphasis on the exact logical selection
/// returned by `evim_core_view_semantic_style_presentation`. Selection or
/// revision changes are rejected, and no layout identity is consulted.
///
/// # Safety
///
/// `request` and `out_outcome` must identify distinct aligned readable and
/// writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_semantic_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetSemanticStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_SEMANTIC_STYLE_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        if request.expected_selection.struct_size < EVIM_LOGICAL_SELECTION_IDENTITY_V1_SIZE
            || !matches!(
                request.expected_selection.kind,
                EVIM_LOGICAL_SELECTION_KIND_NONE
                    | EVIM_LOGICAL_SELECTION_KIND_CHARACTER
                    | EVIM_LOGICAL_SELECTION_KIND_LINE
            )
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let style = semantic_style_from_ffi(request.style)?;
        let enabled = parse_ffi_bool(request.enabled)?;
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let (actual, presentation) =
                export_semantic_style_presentation(core, ViewId(view), style)?;
            validate_logical_selection_identity(request.expected_selection, actual.selection)?;
            let expected = presentation
                .selection()
                .cloned()
                .ok_or(EvimStatus::InvalidRange)?;
            dispatch_event(
                core,
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style,
                    enabled,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Install the exact current Visual selection as a literal forward-search
/// pattern without moving the cursor. `expected` binds the request to both the
/// immutable layout and the opaque selection state, so a stale menu action
/// cannot silently capture different text.
///
/// # Safety
///
/// `expected` and `out_outcome` must identify distinct aligned readable and
/// writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_use_selection_for_find(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimVisualSelectionIdentityV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(expected_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let expected = unsafe { read_visual_selection_identity(expected)? };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let view_id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, view_id)?;
            validate_snapshot_identity(expected.layout, snapshot, view_id)?;
            let export = export_visual_selection(core, view_id)?;
            validate_visual_selection_identity(expected, export.info.identity, snapshot, view_id)?;
            let literal = visual_selection_text(core.document(), &export)?;
            dispatch_event(core, view, CoreEvent::SetFindPattern(literal))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Reveal the active endpoint of the current core-owned Visual selection.
/// Non-Visual views are rejected without changing their viewport.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_reveal_selection(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::RevealSelection)
        })?;
        unsafe { out_outcome.write(outcome) };
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

/// Deliver one normalized key with immutable clipboard snapshots and writable
/// capabilities captured by the host for this exact command turn.
///
/// On success `out_effect_batch` receives either zero (no host effects) or an
/// owned immutable batch which the caller must release. The legacy outcome is
/// still returned independently so existing presentation handling does not
/// depend on the effect export format.
///
/// # Safety
///
/// `input` and `context` (including every nested UTF-8 slice) must remain
/// readable for this call. The two aligned writable outputs must be distinct
/// from all inputs and from each other.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_key_with_host_context(
    handle: EvimCoreHandle,
    view: EvimViewId,
    input: *const EvimKeyInputV1,
    context: *const EvimCommandTurnContextV1,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effect_batch: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(input, 1)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(input_region, outcome_region)
            || regions_overlap(input_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }

        // Copy every caller-owned input before clearing either output. This
        // makes semantic validation failure deterministic without permitting
        // an aliased nested clipboard string to be corrupted by output setup.
        let raw_key = unsafe { input.read() };
        let parsed_context =
            unsafe { read_command_turn_context(context, &[outcome_region, effect_region]) };
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let clipboard = parsed_context?;
        let key = parse_key(raw_key)?;
        let reservation = reserve_effect_batch()?;
        let (outcome, effect_batch) =
            publish_input_turn(handle, view, InputEvent::Key(key), clipboard, reservation)?;
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_batch);
        }
        Ok(())
    })
}

/// Deliver one length-delimited UTF-8 text input event with host clipboard
/// state captured for this exact command turn.
///
/// # Safety
///
/// Nonempty text and all `context` inputs must remain readable for this call.
/// The two aligned writable outputs must be distinct from all inputs and from
/// each other. A successful nonzero effect handle is caller-owned.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_text_with_host_context(
    handle: EvimCoreHandle,
    view: EvimViewId,
    text: *const u8,
    text_length: u64,
    context: *const EvimCommandTurnContextV1,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effect_batch: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let text_region = typed_pointer_region(text, text_length)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(text_region, outcome_region)
            || regions_overlap(text_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }

        let text_bytes = if text_length == 0 {
            Vec::new()
        } else {
            let length = checked_length(text_length)?;
            unsafe { slice::from_raw_parts(text, length) }.to_vec()
        };
        let parsed_context =
            unsafe { read_command_turn_context(context, &[outcome_region, effect_region]) };
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let clipboard = parsed_context?;
        let text = str::from_utf8(&text_bytes)
            .map_err(|_| EvimStatus::InvalidUtf8)?
            .to_owned();
        let reservation = reserve_effect_batch()?;
        let (outcome, effect_batch) =
            publish_input_turn(handle, view, InputEvent::Text(text), clipboard, reservation)?;
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_batch);
        }
        Ok(())
    })
}

/// Deliver one normalized key with immutable clipboard snapshots and writable
/// capabilities captured by the host for this exact command turn.
///
/// On success `out_effect_batch` receives either zero (no host effects) or an
/// owned immutable batch which the caller must release. The legacy outcome is
/// still returned independently so existing presentation handling does not
/// depend on the effect export format.
///
/// # Safety
///
/// `input` and `context` (including every nested UTF-8 slice) must remain
/// readable for this call. The two aligned writable outputs must be distinct
/// from all inputs and from each other.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_key_with_host_context_v2(
    handle: EvimCoreHandle,
    view: EvimViewId,
    input: *const EvimKeyInputV1,
    context: *const EvimCommandTurnContextV2,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effect_batch: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(input, 1)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(input_region, outcome_region)
            || regions_overlap(input_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }

        // Copy every caller-owned input before clearing either output. This
        // makes semantic validation failure deterministic without permitting
        // an aliased nested clipboard string to be corrupted by output setup.
        let raw_key = unsafe { input.read() };
        let parsed_context =
            unsafe { read_command_turn_context_v2(context, &[outcome_region, effect_region]) };
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let clipboard = parsed_context?;
        let key = parse_key(raw_key)?;
        let reservation = reserve_effect_batch()?;
        let (outcome, effect_batch) =
            publish_input_turn(handle, view, InputEvent::Key(key), clipboard, reservation)?;
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_batch);
        }
        Ok(())
    })
}

/// Deliver one length-delimited UTF-8 text input event with host clipboard
/// state captured for this exact command turn.
///
/// # Safety
///
/// Nonempty text and all `context` inputs must remain readable for this call.
/// The two aligned writable outputs must be distinct from all inputs and from
/// each other. A successful nonzero effect handle is caller-owned.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_send_text_with_host_context_v2(
    handle: EvimCoreHandle,
    view: EvimViewId,
    text: *const u8,
    text_length: u64,
    context: *const EvimCommandTurnContextV2,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effect_batch: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let text_region = typed_pointer_region(text, text_length)?;
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        if regions_overlap(text_region, outcome_region)
            || regions_overlap(text_region, effect_region)
            || regions_overlap(outcome_region, effect_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }

        let text_bytes = if text_length == 0 {
            Vec::new()
        } else {
            let length = checked_length(text_length)?;
            unsafe { slice::from_raw_parts(text, length) }.to_vec()
        };
        let parsed_context =
            unsafe { read_command_turn_context_v2(context, &[outcome_region, effect_region]) };
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let clipboard = parsed_context?;
        let text = str::from_utf8(&text_bytes)
            .map_err(|_| EvimStatus::InvalidUtf8)?
            .to_owned();
        let reservation = reserve_effect_batch()?;
        let (outcome, effect_batch) =
            publish_input_turn(handle, view, InputEvent::Text(text), clipboard, reservation)?;
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_batch);
        }
        Ok(())
    })
}

/// Query exact sizes and fixed metadata for an immutable owned host-effect
/// batch. The batch remains valid independently of its originating core until
/// explicitly released.
///
/// # Safety
///
/// `out_info` must identify one aligned writable v1 record.
#[no_mangle]
pub unsafe extern "C" fn evim_effect_batch_info(
    handle: EvimEffectBatchHandle,
    out_info: *mut EvimEffectBatchInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(EvimEffectBatchInfoV1::default()) };
        let batch = owned_effect_batch(handle)?;
        let export = export_effect_batch(handle, &batch)?;
        unsafe { out_info.write(export.info) };
        Ok(())
    })
}

/// Copy every ordered host effect and subordinate value array from one owned
/// immutable batch.
///
/// This is an all-or-none two-pass operation. A short capacity returns
/// [`EvimStatus::BufferTooSmall`] with complete required counts in `out_info`
/// and does not write any array or arena element. Byte references in copied
/// records address `string_bytes`; all such bytes are valid UTF-8.
///
/// File requests are the exact raw Ex requests. They deliberately do not own
/// a prepared artifact write, perform I/O, or establish a save point. A host
/// must route the request and acknowledge a successful exact-revision native
/// save separately. A future binding/prepare/complete lifecycle can be added
/// without changing these raw records.
///
/// # Safety
///
/// Each nonzero capacity requires an aligned writable region of that many
/// elements. All output regions, including `out_info`, must be pairwise
/// disjoint and remain valid for this call.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn evim_effect_batch_copy(
    handle: EvimEffectBatchHandle,
    clipboard_writes: *mut EvimClipboardWriteV1,
    clipboard_write_capacity: u64,
    ex_requests: *mut EvimExFrontendRequestV1,
    ex_request_capacity: u64,
    ex_options: *mut EvimExOptionDisplayV1,
    ex_option_capacity: u64,
    ex_marks: *mut EvimExMarkV1,
    ex_mark_capacity: u64,
    ex_registers: *mut EvimExRegisterV1,
    ex_register_capacity: u64,
    ex_jumps: *mut EvimExJumpV1,
    ex_jump_capacity: u64,
    ex_text_lines: *mut EvimExTextLineV1,
    ex_text_line_capacity: u64,
    file_formats: *mut u32,
    file_format_capacity: u64,
    hard_breaks: *mut u64,
    hard_break_capacity: u64,
    string_bytes: *mut u8,
    string_capacity: u64,
    out_info: *mut EvimEffectBatchInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(clipboard_writes, clipboard_write_capacity)?,
            typed_pointer_region(ex_requests, ex_request_capacity)?,
            typed_pointer_region(ex_options, ex_option_capacity)?,
            typed_pointer_region(ex_marks, ex_mark_capacity)?,
            typed_pointer_region(ex_registers, ex_register_capacity)?,
            typed_pointer_region(ex_jumps, ex_jump_capacity)?,
            typed_pointer_region(ex_text_lines, ex_text_line_capacity)?,
            typed_pointer_region(file_formats, file_format_capacity)?,
            typed_pointer_region(hard_breaks, hard_break_capacity)?,
            typed_pointer_region(string_bytes, string_capacity)?,
            typed_pointer_region(out_info, 1)?,
        ];
        for left in 0..regions.len() {
            for right in left + 1..regions.len() {
                if regions_overlap(regions[left], regions[right]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        unsafe { out_info.write(EvimEffectBatchInfoV1::default()) };
        let batch = owned_effect_batch(handle)?;
        let export = export_effect_batch(handle, &batch)?;
        unsafe { out_info.write(export.info) };
        if clipboard_write_capacity < export.info.clipboard_write_count
            || ex_request_capacity < export.info.ex_request_count
            || ex_option_capacity < export.info.ex_option_count
            || ex_mark_capacity < export.info.ex_mark_count
            || ex_register_capacity < export.info.ex_register_count
            || ex_jump_capacity < export.info.ex_jump_count
            || ex_text_line_capacity < export.info.ex_text_line_count
            || file_format_capacity < export.info.file_format_count
            || hard_break_capacity < export.info.hard_break_count
            || string_capacity < export.info.string_bytes
        {
            return Err(EvimStatus::BufferTooSmall);
        }

        unsafe {
            if !export.clipboard_writes.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.clipboard_writes.as_ptr(),
                    clipboard_writes,
                    export.clipboard_writes.len(),
                );
            }
            if !export.ex_requests.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_requests.as_ptr(),
                    ex_requests,
                    export.ex_requests.len(),
                );
            }
            if !export.ex_options.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_options.as_ptr(),
                    ex_options,
                    export.ex_options.len(),
                );
            }
            if !export.ex_marks.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_marks.as_ptr(),
                    ex_marks,
                    export.ex_marks.len(),
                );
            }
            if !export.ex_registers.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_registers.as_ptr(),
                    ex_registers,
                    export.ex_registers.len(),
                );
            }
            if !export.ex_jumps.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_jumps.as_ptr(),
                    ex_jumps,
                    export.ex_jumps.len(),
                );
            }
            if !export.ex_text_lines.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.ex_text_lines.as_ptr(),
                    ex_text_lines,
                    export.ex_text_lines.len(),
                );
            }
            if !export.file_formats.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.file_formats.as_ptr(),
                    file_formats,
                    export.file_formats.len(),
                );
            }
            if !export.hard_breaks.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.hard_breaks.as_ptr(),
                    hard_breaks,
                    export.hard_breaks.len(),
                );
            }
            if !export.strings.is_empty() {
                std::ptr::copy_nonoverlapping(
                    export.strings.as_ptr(),
                    string_bytes,
                    export.strings.len(),
                );
            }
        }
        Ok(())
    })
}

/// Release one caller-owned immutable host-effect batch. Handles are never
/// reused; zero, unknown, and already released handles return INVALID_HANDLE.
#[no_mangle]
pub extern "C" fn evim_effect_batch_release(handle: EvimEffectBatchHandle) -> EvimStatus {
    ffi_boundary(|| {
        if handle == 0 {
            return Err(EvimStatus::InvalidHandle);
        }
        let batch = {
            let mut registry = effect_batch_registry()
                .lock()
                .map_err(|_| EvimStatus::InternalError)?;
            if !matches!(
                registry.batches.get(&handle),
                Some(EffectBatchRegistryEntry::Ready(_))
            ) {
                return Err(EvimStatus::InvalidHandle);
            }
            match registry
                .batches
                .remove(&handle)
                .ok_or(EvimStatus::InternalError)?
            {
                EffectBatchRegistryEntry::Ready(batch) => batch,
                EffectBatchRegistryEntry::Reserved => return Err(EvimStatus::InternalError),
            }
        };
        // A batch may own a large amount of text. Drop this registry's Arc
        // outside the global lock; a concurrent in-flight copy safely retains
        // its own Arc while future lookups fail immediately.
        drop(batch);
        Ok(())
    })
}

/// Place or extend a view cursor from a revision-bound frontend hit test.
///
/// # Safety
///
/// `request` must identify one aligned readable request and `out_outcome` one
/// distinct aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_place_cursor(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimPlaceCursorV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_PLACE_CURSOR_V1_SIZE
            || request.flags & !EVIM_PLACE_CURSOR_EXTEND_SELECTION != 0
            || request.reserved != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let text_offset =
            usize::try_from(request.text_offset).map_err(|_| EvimStatus::LengthOverflow)?;
        let affinity = parse_layout_affinity(request.affinity)?;
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::PlaceCursor {
                    document_revision: Revision(request.document_revision),
                    text_offset,
                    affinity,
                    extend_selection: request.flags & EVIM_PLACE_CURSOR_EXTEND_SELECTION != 0,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

unsafe fn core_view_navigate_history(
    handle: EvimCoreHandle,
    view: EvimViewId,
    navigation: crate::document::HistoryNavigationRequest,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::NavigateHistory(navigation))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Navigate one undo edge independently of the current Vim mode.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_undo(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    unsafe {
        core_view_navigate_history(
            handle,
            view,
            crate::document::HistoryNavigationRequest::Undo,
            out_outcome,
        )
    }
}

/// Navigate one preferred redo edge independently of the current Vim mode.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_redo(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    unsafe {
        core_view_navigate_history(
            handle,
            view,
            crate::document::HistoryNavigationRequest::Redo,
            out_outcome,
        )
    }
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

/// Query the exact temporary composition projection and its marked/selected
/// UTF-8 ranges. An attached view without active marked text returns a valid
/// record with `ACTIVE` clear.
///
/// # Safety
///
/// `out_info` must identify one aligned writable v1 record.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_composition_overlay_info(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_info: *mut EvimCompositionOverlayInfoV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        let inactive = EvimCompositionOverlayInfoV1 {
            struct_size: EVIM_COMPOSITION_OVERLAY_INFO_V1_SIZE,
            identity: EvimCompositionOverlayIdentityV1 {
                struct_size: EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE,
                ..EvimCompositionOverlayIdentityV1::default()
            },
            ..EvimCompositionOverlayInfoV1::default()
        };
        unsafe { out_info.write(inactive) };
        let info = with_core(handle, |core| {
            let view_id = ViewId(view);
            core.command_state(view_id).ok_or(EvimStatus::InvalidView)?;
            core.composition_overlay(view_id)
                .map_err(core_status)?
                .as_ref()
                .map(|overlay| composition_overlay_info(overlay, view_id))
                .transpose()
                .map(|value| value.unwrap_or(inactive))
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Copy one exact scalar-aligned range from the source-nonmutating composed
/// UTF-8 projection. The operation is all-or-none and uses the same two-pass
/// contract as formatted snapshot range reads.
///
/// # Safety
///
/// Pointer regions must be aligned, valid for their declared sizes, and
/// pairwise disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_copy_composition_utf8_range(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCompositionOverlayUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request, 1)?;
        let output_region = typed_pointer_region(output, output_capacity)?;
        let required_region = typed_pointer_region(out_required, 1)?;
        if regions_overlap(output_region, request_region)
            || regions_overlap(output_region, required_region)
            || regions_overlap(required_region, request_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_COMPOSITION_OVERLAY_UTF8_RANGE_V1_SIZE
            || request.reserved != 0
            || request.identity.struct_size < EVIM_COMPOSITION_OVERLAY_IDENTITY_V1_SIZE
            || request.identity.reserved != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_required.write(0) };
        let start = checked_length(request.start)?;
        let end = checked_length(request.end)?;
        let bytes = with_core(handle, |core| {
            let view_id = ViewId(view);
            let overlay = core
                .composition_overlay(view_id)
                .map_err(core_status)?
                .ok_or(EvimStatus::InvalidArgument)?;
            validate_composition_overlay_identity(request.identity, &overlay, view_id)?;
            if start > end || end > overlay.utf8_len() {
                return Err(EvimStatus::InvalidRange);
            }
            overlay
                .text_in_range(start..end)
                .map(String::into_bytes)
                .ok_or(EvimStatus::InvalidUtf8Boundary)
        })?;
        let required = checked_export_count(bytes.len())?;
        unsafe { out_required.write(required) };
        if checked_length(output_capacity)? < bytes.len() {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        }
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
/// presentation state and never run layout. A request carrying `HAS_TOP` must
/// identify the current immutable layout and atomically installs a bounded
/// exact viewport around the requested estimated-or-exact document y.
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
            if top.is_some() {
                validate_viewport_origin_identity(core, ViewId(view), request)?;
            }
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

/// Change application-owned canvas padding without a source transaction.
#[no_mangle]
pub extern "C" fn evim_core_view_set_padding(
    handle: EvimCoreHandle,
    view: EvimViewId,
    top: f32,
    left: f32,
    bottom: f32,
    right: f32,
) -> EvimStatus {
    ffi_boundary(|| {
        if [top, left, bottom, right]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            core.set_view_insets(
                ViewId(view),
                crate::layout::EdgeInsets {
                    top,
                    left,
                    bottom,
                    right,
                },
            )
            .map_err(core_status)
        })
    })
}

/// Return the next portable zoom stop without mutating document or view state.
/// `increasing` is 0 or 1; direct scales must lie within 25% through 500%.
///
/// # Safety
/// `out_scale` must identify one aligned writable float.
#[no_mangle]
pub unsafe extern "C" fn evim_core_adjacent_zoom_scale(
    scale: f32,
    increasing: u32,
    out_scale: *mut f32,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_scale.is_null() || (out_scale as usize) % std::mem::align_of::<f32>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { out_scale.write(0.0) };
        if increasing > 1 {
            return Err(EvimStatus::InvalidArgument);
        }
        let next = crate::layout::adjacent_zoom_scale(scale, increasing == 1)
            .map_err(|_| EvimStatus::InvalidArgument)?;
        unsafe { out_scale.write(next) };
        Ok(())
    })
}

/// Set one view's magnification and synchronously reflow its current viewport.
/// The value is presentation-only and never changes document state.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_scale(
    handle: EvimCoreHandle,
    view: EvimViewId,
    scale: f32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !crate::layout::valid_zoom_scale(scale) {
            return Err(EvimStatus::InvalidArgument);
        }
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetScale(scale))
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

/// Deprecated compatibility entry point. Word-boundary wrapping is mandatory:
/// true returns the current state unchanged; false is invalid.
///
/// # Safety
///
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_linebreak(
    handle: EvimCoreHandle,
    view: EvimViewId,
    linebreak: u32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if !parse_ffi_bool(linebreak)? {
            return Err(EvimStatus::InvalidArgument);
        }
        let outcome = with_core(handle, |core| {
            summarize_core_outcome(core, ViewId(view), None)
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Change shared line-ending spelling through one exact typed model
/// transaction. Detection is not a valid post-open target.
///
/// # Safety
///
/// `request` and `out_outcome` must identify distinct aligned readable and
/// writable v1 values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_file_format(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetFileFormatV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_FILE_FORMAT_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let target = parse_concrete_file_format(request.file_format)?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SetFileFormat {
                    document: DocumentId(request.document_id),
                    revision: Revision(request.document_revision),
                    target,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Change shared HTML style serialization through one exact model transaction.
/// The enabled value must be zero or one; unsupported formats are rejected.
///
/// # Safety
///
/// `request` and `out_outcome` must identify distinct aligned readable and
/// writable v1 values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_include_style_definitions(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetIncludeStyleDefinitionsV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_INCLUDE_STYLE_DEFINITIONS_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let enabled = parse_ffi_bool(request.enabled)?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SetIncludeStyleDefinitionsInFile {
                    document: DocumentId(request.document_id),
                    revision: Revision(request.document_revision),
                    enabled,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Change format using the document conversion policy. Native callers should
/// use the with-effects variant to present conversion-loss diagnostics.
///
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_format(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetFormatV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_FORMAT_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let target = parse_format(request.format)?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SetFormat {
                    document: DocumentId(request.document_id),
                    revision: Revision(request.document_revision),
                    target,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Explicitly transcode source syntax. Latin-1 conversion may substitute
/// unrepresentable scalars; the effects variant also returns the warning.
///
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_encoding(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetEncodingV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_ENCODING_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let target = parse_encoding(request.encoding)?.ok_or(EvimStatus::InvalidEncoding)?;
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::SetEncoding {
                    document: DocumentId(request.document_id),
                    revision: Revision(request.document_revision),
                    target,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Query an exact list-action target without requiring current layout.
///
/// # Safety
/// The output must identify an aligned writable selection identity.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_list_selection(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_selection: *mut EvimLogicalSelectionIdentityV1,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_selection, 1)?;
        unsafe { out_selection.write(EvimLogicalSelectionIdentityV1::default()) };
        let selection = with_core(handle, |core| {
            logical_selection_identity_to_ffi(
                &core
                    .list_selection_identity(ViewId(view))
                    .map_err(core_status)?,
            )
        })?;
        unsafe { out_selection.write(selection) };
        Ok(())
    })
}

/// Query verified list nesting actions for the exact current selection.
/// # Safety
/// Both pointers must identify aligned values; the output must be writable.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_list_indent_capabilities(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimLogicalSelectionIdentityV1,
    out_flags: *mut u32,
) -> EvimStatus {
    ffi_boundary(|| {
        let input_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_flags, 1)?;
        if regions_overlap(input_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let expected = unsafe { expected.read() };
        unsafe { out_flags.write(0) };
        let flags = with_core(handle, |core| {
            let current = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                expected,
                logical_selection_identity_to_ffi(&current)?,
            )?;
            let (indent, unindent) = core.document().list_indent_capabilities(current.range());
            Ok(u32::from(indent) * EVIM_LIST_CAN_INDENT
                | u32::from(unindent) * EVIM_LIST_CAN_UNINDENT)
        })?;
        unsafe { out_flags.write(flags) };
        Ok(())
    })
}

/// Indent/unindent complete selected list items by exactly one level.
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_indent_list(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimListIndentV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_LIST_INDENT_V1_SIZE || request.unindent > 1 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::IndentList {
                    expected,
                    unindent: request.unindent != 0,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Set/remove list markers on the selected paragraphs as one undo unit.
///
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_list_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetListStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_LIST_STYLE_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        let style = match request.style {
            EVIM_LIST_STYLE_NONE => None,
            EVIM_LIST_STYLE_BULLET => Some(crate::document::ListStyle::Bullet),
            EVIM_LIST_STYLE_NUMBERED => Some(crate::document::ListStyle::Numbered),
            _ => return Err(EvimStatus::InvalidArgument),
        };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(core, view, CoreEvent::SetListStyle { expected, style })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Set a paragraph/heading style using an exact native paragraph target.
///
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_paragraph_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetParagraphStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_SET_PARAGRAPH_STYLE_V1_SIZE || request.level > 6 {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            let style = if request.level == 0 {
                StyleId("Paragraph".into())
            } else {
                StyleId(format!("Heading{}", request.level))
            };
            dispatch_event(core, view, CoreEvent::SetParagraphStyle { expected, style })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Assign an existing paragraph or character style at exact source, style-sheet,
/// and logical-selection identities. With no selection, character assignment
/// updates the pending typing style; paragraph assignment targets the paragraph.
///
/// # Safety
/// Request, its UTF-8 slice, and outcome must be valid and not overlap output.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_assign_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimAssignStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_ASSIGN_STYLE_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        let namespace = parse_style_namespace(request.namespace)?;
        let style = unsafe { composition_utf8(request.style_id, out_outcome)? };
        if style.is_empty() || style.contains('\0') {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_style_sheet_identity(request.identity, core.document())?;
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::AssignNamedStyle {
                    expected,
                    style_sheet_revision: StyleSheetRevision(request.identity.style_sheet_revision),
                    namespace,
                    style: StyleId(style),
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Apply one direct declaration through exact source/selection verification.
/// # Safety
/// All request, nested value, and output pointers must be valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_edit_direct_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimDirectStyleEditV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_DIRECT_STYLE_EDIT_V1_SIZE || request.reserved != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let property = parse_style_property(request.property)?;
        if property < StyleProperty::ParagraphSpacingBefore {
            return Err(EvimStatus::UnsupportedOperation);
        }
        let value = match request.operation {
            EVIM_STYLE_EDIT_SET_DECLARATION => {
                Some(unsafe { parse_style_property_value(property, &request.value, out_outcome)? })
            }
            EVIM_STYLE_EDIT_CLEAR_DECLARATION
                if request.value.struct_size >= EVIM_STYLE_EDIT_VALUE_V1_SIZE
                    && request.value.kind == EVIM_STYLE_VALUE_NONE
                    && request.value.reserved == 0
                    && request.value.item_count == 0
                    && request.value.text.length == 0 =>
            {
                None
            }
            _ => return Err(EvimStatus::InvalidArgument),
        };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::EditDirectProperty {
                    expected,
                    property,
                    value,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Query decoration toggle state using the exact current logical selection.
/// # Safety
/// The request array, nested inputs, and output must be valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_edit_direct_character_batch(
    handle: EvimCoreHandle,
    view: EvimViewId,
    requests: *const EvimDirectStyleEditV1,
    count: u64,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if !(1..=32).contains(&count) {
            return Err(EvimStatus::InvalidArgument);
        }
        let input = typed_pointer_region(requests, count)?;
        let output = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(input, output) {
            return Err(EvimStatus::InvalidArgument);
        }
        let requests = unsafe { std::slice::from_raw_parts(requests, count as usize) };
        let expected_selection = requests[0].expected_selection;
        let mut values = Vec::with_capacity(count as usize);
        let mut seen = std::collections::BTreeSet::new();
        for request in requests {
            if request.struct_size < EVIM_DIRECT_STYLE_EDIT_V1_SIZE
                || request.reserved != 0
                || request.operation != EVIM_STYLE_EDIT_SET_DECLARATION
            {
                return Err(EvimStatus::InvalidArgument);
            }
            validate_logical_selection_identity(request.expected_selection, expected_selection)?;
            let property = parse_style_property(request.property)?;
            if !crate::document::is_character_property(property) || !seen.insert(property) {
                return Err(EvimStatus::InvalidArgument);
            }
            let value =
                unsafe { parse_style_property_value(property, &request.value, out_outcome)? };
            values.push((property, value));
        }
        unsafe {
            clear_outcome(out_outcome)?;
        }
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::SetDirectCharacterProperties { expected, values },
            )
        })?;
        unsafe {
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// Query decoration toggle state using the exact current logical selection.
/// # Safety
/// out_state must point to writable, aligned u32 storage for this call.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_decoration_state(
    handle: EvimCoreHandle,
    view: EvimViewId,
    property: u32,
    out_state: *mut u32,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_state, 1)?;
        let strike = match parse_style_property(property)? {
            StyleProperty::CharacterUnderline => false,
            StyleProperty::CharacterStrikethrough => true,
            _ => return Err(EvimStatus::InvalidArgument),
        };
        let state = with_core(handle, |core| {
            core.selection_decoration_state(ViewId(view), strike)
                .map(semantic_style_state_to_ffi)
                .map_err(core_status)
        })?;
        unsafe { out_state.write(state) };
        Ok(())
    })
}

/// Current selection/caret typography. Output strings and features are copied
/// in one batch and tied to the caller's exact document revision.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EvimTypographyInfoV1 {
    pub struct_size: u32,
    /// bit0 bold, bit1 mixed, bit2 default foreground.
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub font_family_bytes: u64,
    pub feature_count: u64,
    pub size: f32,
    pub weight: u32,
    pub base_weight: u32,
    pub slant: u32,
    pub foreground: EvimRgbaV1,
}

/// # Safety
/// Outputs must be valid for their capacities and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_typography_export(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected_revision: u64,
    out_info: *mut EvimTypographyInfoV1,
    out_family: *mut u8,
    family_capacity: u64,
    out_features: *mut EvimOpenTypeFeatureV1,
    feature_capacity: u64,
) -> EvimStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        let family_region = typed_pointer_region(out_family, family_capacity)?;
        let feature_region = typed_pointer_region(out_features, feature_capacity)?;
        let info_region = typed_pointer_region(out_info, 1)?;
        if regions_overlap(info_region, family_region)
            || regions_overlap(info_region, feature_region)
            || regions_overlap(family_region, feature_region)
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let (info, family, features) = with_core(handle, |core| {
            if core.document().revision().0 != expected_revision {
                return Err(EvimStatus::StaleRevision);
            }
            let (style, mixed) = core
                .selected_typography(ViewId(view))
                .map_err(core_status)?;
            let family = style
                .font_families
                .first()
                .cloned()
                .unwrap_or_default()
                .into_bytes();
            let features = style
                .open_type_features
                .iter()
                .map(|(tag, value)| EvimOpenTypeFeatureV1 {
                    tag: tag.as_bytes().try_into().unwrap_or(*b"    "),
                    value: *value,
                })
                .collect::<Vec<_>>();
            let info = EvimTypographyInfoV1 {
                struct_size: size_of::<EvimTypographyInfoV1>() as u32,
                flags: u32::from(style.bold)
                    | (u32::from(mixed) << 1)
                    | (u32::from(style.foreground_is_default) << 2),
                document_id: core.document().id().0,
                document_revision: expected_revision,
                font_family_bytes: family.len() as u64,
                feature_count: features.len() as u64,
                size: style.size,
                weight: u32::from(style.weight),
                base_weight: u32::from(style.base_weight),
                slant: match style.slant {
                    FontSlant::Upright => EVIM_FONT_SLANT_UPRIGHT,
                    FontSlant::Italic => EVIM_FONT_SLANT_ITALIC,
                    FontSlant::Oblique => EVIM_FONT_SLANT_OBLIQUE,
                },
                foreground: EvimRgbaV1 {
                    red: style.foreground.red,
                    green: style.foreground.green,
                    blue: style.foreground.blue,
                    alpha: style.foreground.alpha,
                },
            };
            Ok((info, family, features))
        })?;
        unsafe {
            out_info.write(info);
        }
        if family_capacity < info.font_family_bytes || feature_capacity < info.feature_count {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !family.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(family.as_ptr(), out_family, family.len());
            }
        }
        if !features.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(features.as_ptr(), out_features, features.len());
            }
        }
        Ok(())
    })
}

/// Create one sparse source-backed paragraph or character definition. Empty
/// parent selects its namespace base; empty next-style means no explicit next.
///
/// # Safety
/// Request, nested UTF-8 slices, and outcome must be valid and not overlap output.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_create_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimCreateStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_CREATE_STYLE_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        let namespace = parse_style_namespace(request.namespace)?;
        let id = unsafe { composition_utf8(request.style_id, out_outcome)? };
        let name = unsafe { composition_utf8(request.display_name, out_outcome)? };
        let parent = unsafe { composition_utf8(request.parent_id, out_outcome)? };
        let next = unsafe { composition_utf8(request.next_style_id, out_outcome)? };
        if id.is_empty()
            || name.is_empty()
            || [&id, &name, &parent, &next]
                .iter()
                .any(|value| value.contains('\0'))
        {
            return Err(EvimStatus::InvalidArgument);
        }
        if namespace == StyleNamespace::Character && !next.is_empty() {
            return Err(EvimStatus::InvalidStyleRelationship);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_style_sheet_identity(request.identity, core.document())?;
            let sheet = core.document().projection().style_sheet();
            let parent = if parent.is_empty() {
                match namespace {
                    StyleNamespace::Block => sheet.base_paragraph.clone(),
                    StyleNamespace::Character => sheet.base_character.clone(),
                }
            } else {
                StyleId(parent)
            };
            let metadata = crate::document::StyleDefinitionMetadata {
                display_name: name,
                origin: StyleDefinitionOrigin::SourceBacked,
            };
            let edit = match namespace {
                StyleNamespace::Block => crate::document::StyleDefinitionEdit::InsertBlock {
                    style: crate::document::BlockStyle {
                        id: StyleId(id),
                        based_on: Some(parent),
                        next_paragraph_style: (!next.is_empty()).then_some(StyleId(next)),
                        role: BlockRole::Paragraph,
                        character: CharacterProperties::default(),
                        block: BlockProperties::default(),
                    },
                    metadata,
                },
                StyleNamespace::Character => {
                    crate::document::StyleDefinitionEdit::InsertCharacter {
                        style: crate::document::CharacterStyle {
                            id: StyleId(id),
                            based_on: Some(parent),
                            properties: CharacterProperties::default(),
                        },
                        metadata,
                    }
                }
            };
            dispatch_event(
                core,
                view,
                CoreEvent::EditNamedStyleDefinition {
                    document: DocumentId(request.identity.document_id),
                    revision: Revision(request.identity.document_revision),
                    style_sheet_revision: StyleSheetRevision(request.identity.style_sheet_revision),
                    edit,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Delete a source-backed style and rebase its assignments and references in the
/// same undoable source transaction.
///
/// # Safety
/// Request, its UTF-8 slice, and outcome must be valid and not overlap output.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_delete_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimDeleteStyleV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < EVIM_DELETE_STYLE_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        let namespace = parse_style_namespace(request.namespace)?;
        let id = unsafe { composition_utf8(request.style_id, out_outcome)? };
        if id.is_empty() || id.contains('\0') {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_style_sheet_identity(request.identity, core.document())?;
            dispatch_event(
                core,
                view,
                CoreEvent::EditNamedStyleDefinition {
                    document: DocumentId(request.identity.document_id),
                    revision: Revision(request.identity.document_revision),
                    style_sheet_revision: StyleSheetRevision(request.identity.style_sheet_revision),
                    edit: match namespace {
                        StyleNamespace::Block => {
                            crate::document::StyleDefinitionEdit::DeleteBlock(StyleId(id))
                        }
                        StyleNamespace::Character => {
                            crate::document::StyleDefinitionEdit::DeleteCharacter(StyleId(id))
                        }
                    },
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Begin one explicit frontend-owned live style-edit group at an exact style
/// sheet identity. A core permits one active style group. A group containing
/// no successful edits creates no history entry.
///
/// Any ordinary coordinator event (key/text input, pointer placement,
/// composition, history, viewport/options, or a legacy standalone style edit)
/// closes the successful prefix and consumes this capability before handling
/// that event. Removing the owning view does the same. A frontend can therefore
/// safely recover from a missed end call by beginning a new group after the
/// unrelated event.
///
/// # Safety
///
/// `expected` and `out_group` must identify distinct aligned readable and
/// writable v1 values.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_begin_style_edit_group(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimStyleSheetIdentityV1,
    out_group: *mut EvimStyleEditGroupV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let expected_region = typed_pointer_region(expected, 1)?;
        let output_region = typed_pointer_region(out_group, 1)?;
        if regions_overlap(expected_region, output_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        let expected = unsafe { read_style_sheet_identity(expected)? };
        unsafe { out_group.write(EvimStyleEditGroupV1::default()) };
        let group = with_core_mut(handle, |core| {
            core.begin_style_edit_group(
                ViewId(view),
                DocumentId(expected.document_id),
                Revision(expected.document_revision),
                StyleSheetRevision(expected.style_sheet_revision),
            )
            .map_err(core_status)
        })?;
        unsafe { out_group.write(EvimStyleEditGroupV1::from_core(group)) };
        Ok(())
    })
}

/// Atomically edit one field of an existing style through its core-owned
/// authority: source-backed HTML/RTF definitions patch source, while generated
/// definitions update configuration. Synthetic definitions remain read-only.
/// Each successful non-no-op call is one standalone undo unit.
///
/// # Safety
///
/// `request`, its nonempty nested slices/items, and `out_outcome` must remain
/// valid for this call. The fixed request, item array, and output must be
/// aligned; input data must not overlap the writable output.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_edit_style(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimStyleEditV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let request = unsafe { parse_style_edit_request(request, out_outcome)? };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::EditGeneratedStyle {
                    document: DocumentId(request.identity.document_id),
                    revision: Revision(request.identity.document_revision),
                    style_sheet_revision: StyleSheetRevision(request.identity.style_sheet_revision),
                    namespace: request.namespace,
                    style: request.style,
                    edit: request.edit,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Commit one exact field edit into an explicit live style-edit group. Each
/// successful edit is immediately visible in all views, but all successful
/// edits before end share one document history node. A rejected or no-op edit
/// leaves both prior successful edits and the group usable.
///
/// `request.identity` must name the current document/style-sheet snapshot for
/// every call. The group's begin identities remain unchanged and are not a
/// substitute for that per-edit staleness check.
///
/// # Safety
///
/// `group`, `request`, their nonempty nested slices/items, and `out_outcome`
/// must remain valid for this call. Fixed records and output must be aligned
/// and pairwise disjoint; nested input data must not overlap the output.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_edit_style_in_group(
    handle: EvimCoreHandle,
    view: EvimViewId,
    group: *const EvimStyleEditGroupV1,
    request: *const EvimStyleEditV1,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(group, 1)?,
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
        ];
        for left in 0..regions.len() {
            for right in left + 1..regions.len() {
                if regions_overlap(regions[left], regions[right]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let group = unsafe { read_style_edit_group(group)? }.into_core();
        let request = unsafe { parse_style_edit_request(request, out_outcome)? };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let view_id = ViewId(view);
            let outcome = core
                .edit_generated_style_in_group(
                    view_id,
                    group,
                    DocumentId(request.identity.document_id),
                    Revision(request.identity.document_revision),
                    StyleSheetRevision(request.identity.style_sheet_revision),
                    request.namespace,
                    request.style,
                    request.edit,
                )
                .map_err(core_status)?;
            summarize_core_outcome(core, view_id, Some(&outcome))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Close and consume an explicit live style-edit group. All successful edits
/// remain committed and become one undo/redo unit; end never rolls back. A
/// second end or apply using the consumed capability returns
/// `EVIM_STATUS_INVALID_STYLE_EDIT_GROUP`.
///
/// # Safety
///
/// `group` must identify one aligned readable v1 value.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_end_style_edit_group(
    handle: EvimCoreHandle,
    view: EvimViewId,
    group: *const EvimStyleEditGroupV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let group = unsafe { read_style_edit_group(group)? }.into_core();
        with_core_mut(handle, |core| {
            core.end_style_edit_group(ViewId(view), group)
                .map_err(core_status)
        })?;
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
        checkout_core, checkout_document, evim_core_copy_formatted_utf8_range,
        evim_core_copy_style_sheet, evim_core_destroy, evim_core_formatted_point_info,
        evim_core_formatted_snapshot_info, evim_core_map_formatted_utf16_to_utf8,
        evim_core_map_formatted_utf8_to_utf16, evim_core_style_sheet_info,
        evim_core_view_copy_layout_paint, evim_core_view_layout_paint_info, evim_document_destroy,
        export_style_sheet, ffi_boundary, register_core, register_document,
        summarize_document_state, CTextMeasurementProvider, EvimClusterCaretStopV1,
        EvimCoreOutcomeV1, EvimFormattedPointInfoV1, EvimFormattedSnapshotInfoV1,
        EvimFormattedUtf8RangeV1, EvimLayoutPaintInfoV1, EvimPaintStyleRunV1,
        EvimRenderRunHandleV1, EvimShapeRequestV1, EvimShapeResponseV1, EvimShapedBoundsV1,
        EvimShapedClusterV1, EvimStatus, EvimStyleDefinitionV1, EvimStyleDependencyV1,
        EvimStyleEditV1, EvimStylePropertyV1, EvimStyleSheetInfoV1, EvimStyleStringRefV1,
        EvimStyleValueItemV1, EvimTextMetricsV1, EvimUtf8Slice, MarshalledRequest,
        EVIM_BOUNDARY_AFFINITY_DOWNSTREAM, EVIM_BOUNDARY_AFFINITY_UPSTREAM,
        EVIM_HISTORY_ACTION_CATEGORY_MIXED, EVIM_LAYOUT_PAINT_INFO_V1_SIZE,
        EVIM_PAINT_STYLE_RUN_V1_SIZE, EVIM_SHAPED_CLUSTER_V1_SIZE,
        EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA, EVIM_TEXT_DIRECTION_AUTO,
        EVIM_TEXT_DIRECTION_RIGHT_TO_LEFT, EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1,
        EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V2, EVIM_TEXT_PAINT_HAS_BACKGROUND,
        EVIM_TEXT_PAINT_STRIKETHROUGH, EVIM_TEXT_PAINT_UNDERLINE, EVIM_TEXT_PAINT_V1_SIZE,
    };
    use crate::command::{InputEvent, Key};
    use crate::document::{
        BlockProperties, CharacterProperties, Color, ConfigurationStyleIntent, Document, Encoding,
        FileFormat, Format, LineSpacing, ParagraphAlignment, StyleDefinitionEdit, StyleId,
        StyleModelIntent, StyleModelRequest,
    };
    use crate::layout::{
        MeasurementEnvironmentId, MetricsGeneration, ProviderThreading, RenderRunOwner,
        RenderRunPolicy, RenderRunThreading, ResolvedTextStyle, ShapePurpose, ShapeRequest,
        TextDirection,
    };
    use crate::{Core, CoreEvent};
    use std::ffi::c_void;
    use std::ptr;

    const PAINT_TEST_FONT: &[u8] = b"Paint Test Font";

    #[test]
    fn document_state_collapses_distinct_grouped_semantics_to_mixed() {
        let mut document = Document::from_bytes_with_file_format(
            b"a\nb".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        document.begin_edit_group();
        document.insert(0, "x").unwrap();
        document.set_file_format(FileFormat::Dos).unwrap();
        document.end_edit_group();

        assert_eq!(
            summarize_document_state(&document).undo_action_category,
            EVIM_HISTORY_ACTION_CATEGORY_MIXED
        );
        document.try_undo().unwrap();
        assert_eq!(
            summarize_document_state(&document).redo_action_category,
            EVIM_HISTORY_ACTION_CATEGORY_MIXED
        );
    }

    #[test]
    fn effect_batch_export_is_ordered_lossless_two_pass_and_owned() {
        use crate::command::clipboard::{ClipboardContent, ClipboardTarget, ClipboardWriteRequest};
        use crate::command::ex_execute::{
            ExFileRequest, ExFrontendRequest, ExInfoRequest, ExNavigation, ExNormalRequest,
            ExOptionDisplay, ExOptionName, ExOptionValue, ExOutcome, HardLineRange,
        };
        use crate::command::{RegisterKind, RegisterValue};
        use crate::document::{DocumentId, Revision};

        fn referenced_text(arena: &[u8], reference: super::EvimEffectBytesRefV1) -> &str {
            let start = usize::try_from(reference.offset).unwrap();
            let length = usize::try_from(reference.length).unwrap();
            std::str::from_utf8(&arena[start..start + length]).unwrap()
        }

        let file_range = HardLineRange::new(2, 4).unwrap();
        let mut ex = ExOutcome::default();
        ex.document_changed = true;
        ex.navigation = Some(ExNavigation::TextOffset(37));
        ex.substitutions = 9;
        ex.frontend_requests = vec![
            ExFrontendRequest::File(ExFileRequest::Edit {
                path: Some("文章/é.md".into()),
                force: true,
            }),
            ExFrontendRequest::File(ExFileRequest::New { force: false }),
            ExFrontendRequest::File(ExFileRequest::Write {
                path: Some("写す.txt".into()),
                force: true,
                range: Some(file_range),
            }),
            ExFrontendRequest::File(ExFileRequest::SaveAs {
                path: "別名.md".into(),
                force: false,
            }),
            ExFrontendRequest::File(ExFileRequest::Quit { force: true }),
            ExFrontendRequest::File(ExFileRequest::QuitAll { force: false }),
            ExFrontendRequest::File(ExFileRequest::WriteQuit {
                path: None,
                force: true,
                range: Some(HardLineRange::new(0, 1).unwrap()),
            }),
            ExFrontendRequest::File(ExFileRequest::Xit {
                path: Some("終了.txt".into()),
                force: false,
            }),
            ExFrontendRequest::File(ExFileRequest::WriteAll { force: true }),
            ExFrontendRequest::Info(ExInfoRequest::Marks(vec!['a', 'é', 'λ'])),
            ExFrontendRequest::Info(ExInfoRequest::Registers(vec!['"', '+', 'ß'])),
            ExFrontendRequest::Info(ExInfoRequest::Jumps),
            ExFrontendRequest::Info(ExInfoRequest::Options(vec![
                ExOptionDisplay {
                    name: ExOptionName::Wrap,
                    value: ExOptionValue::Boolean(true),
                },
                ExOptionDisplay {
                    name: ExOptionName::IgnoreCase,
                    value: ExOptionValue::Boolean(false),
                },
                ExOptionDisplay {
                    name: ExOptionName::FileFormat,
                    value: ExOptionValue::FileFormat(FileFormat::Dos),
                },
                ExOptionDisplay {
                    name: ExOptionName::FileFormats,
                    value: ExOptionValue::FileFormats(vec![
                        FileFormat::Unix,
                        FileFormat::Dos,
                        FileFormat::Mac,
                    ]),
                },
            ])),
            ExFrontendRequest::Info(ExInfoRequest::PrintLines {
                range: HardLineRange::new(5, 7).unwrap(),
                number: true,
                list: true,
            }),
            ExFrontendRequest::Normal(ExNormalRequest {
                range: HardLineRange::new(8, 9).unwrap(),
                commands: "gUiwé".into(),
                literal: true,
            }),
        ];
        let portable =
            RegisterValue::try_new("α\nβ", RegisterKind::Linewise, vec!["α".len()]).unwrap();
        let batch = super::OwnedEffectBatch {
            document_id: DocumentId(0xdecaf),
            document_revision: Revision(42),
            ex_outcome: Some(ex),
            ex_info_payloads: vec![
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(super::OwnedExInfoPayload::Marks(vec![
                    super::OwnedResolvedMark {
                        name: 'é',
                        point: super::OwnedResolvedPoint {
                            utf8_offset: 12,
                            hard_line_index: 2,
                            hard_line_range: 10..19,
                            grapheme_column: 1,
                            line_text: "héllo λ".into(),
                        },
                    },
                ])),
                Some(super::OwnedExInfoPayload::Registers(vec![
                    super::OwnedResolvedRegister {
                        name: '+',
                        value: RegisterValue::try_new(
                            "一\n二",
                            RegisterKind::Characterwise,
                            vec!["一".len()],
                        )
                        .unwrap(),
                    },
                ])),
                Some(super::OwnedExInfoPayload::Jumps(vec![
                    super::OwnedResolvedJump {
                        list_index: 3,
                        current: true,
                        point: super::OwnedResolvedPoint {
                            utf8_offset: 22,
                            hard_line_index: 4,
                            hard_line_range: 20..28,
                            grapheme_column: 2,
                            line_text: "jump 🦀".into(),
                        },
                    },
                ])),
                None,
                Some(super::OwnedExInfoPayload::TextLines(vec![
                    super::OwnedResolvedTextLine {
                        hard_line_index: 5,
                        utf8_range: 30..38,
                        text: "print é".into(),
                    },
                    super::OwnedResolvedTextLine {
                        hard_line_index: 6,
                        utf8_range: 39..42,
                        text: "終".into(),
                    },
                ])),
                None,
            ],
            clipboard_writes: vec![
                ClipboardWriteRequest::new(
                    ClipboardTarget::Clipboard,
                    ClipboardContent::from_plain_text("plain 👋"),
                ),
                ClipboardWriteRequest::from_register(ClipboardTarget::Primary, portable),
            ],
        };
        let handle = super::reserve_effect_batch()
            .unwrap()
            .commit(batch)
            .unwrap();

        let mut info = super::EvimEffectBatchInfoV1::default();
        assert_eq!(
            unsafe { super::evim_effect_batch_info(handle, &mut info) },
            EvimStatus::Ok
        );
        assert_eq!(info.batch_handle, handle);
        assert_eq!(info.document_id, 0xdecaf);
        assert_eq!(info.document_revision, 42);
        assert_eq!(info.clipboard_write_count, 2);
        assert_eq!(info.ex_request_count, 15);
        assert_eq!(info.ex_option_count, 4);
        assert_eq!(info.ex_mark_count, 1);
        assert_eq!(info.ex_register_count, 1);
        assert_eq!(info.ex_jump_count, 1);
        assert_eq!(info.ex_text_line_count, 2);
        assert_eq!(info.file_format_count, 3);
        assert_eq!(info.hard_break_count, 2);
        assert_eq!(info.navigation_utf8_offset, 37);
        assert_eq!(info.substitution_count, 9);
        assert_eq!(
            info.flags,
            super::EVIM_EFFECT_BATCH_HAS_EX_OUTCOME
                | super::EVIM_EFFECT_BATCH_EX_DOCUMENT_CHANGED
                | super::EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION
        );

        let mut writes = vec![
            super::EvimClipboardWriteV1 {
                target: u32::MAX,
                ..super::EvimClipboardWriteV1::default()
            };
            info.clipboard_write_count as usize
        ];
        let mut requests = vec![
            super::EvimExFrontendRequestV1 {
                kind: u32::MAX,
                ..super::EvimExFrontendRequestV1::default()
            };
            info.ex_request_count as usize
        ];
        let mut options = vec![
            super::EvimExOptionDisplayV1 {
                name: u32::MAX,
                ..super::EvimExOptionDisplayV1::default()
            };
            info.ex_option_count as usize
        ];
        let mut marks = vec![
            super::EvimExMarkV1 {
                name: u32::MAX,
                ..super::EvimExMarkV1::default()
            };
            info.ex_mark_count as usize
        ];
        let mut registers = vec![
            super::EvimExRegisterV1 {
                name: u32::MAX,
                ..super::EvimExRegisterV1::default()
            };
            info.ex_register_count as usize
        ];
        let mut jumps = vec![
            super::EvimExJumpV1 {
                flags: u32::MAX,
                ..super::EvimExJumpV1::default()
            };
            info.ex_jump_count as usize
        ];
        let mut text_lines = vec![
            super::EvimExTextLineV1 {
                hard_line_index: u64::MAX,
                ..super::EvimExTextLineV1::default()
            };
            info.ex_text_line_count as usize
        ];
        let mut file_formats = vec![u32::MAX; info.file_format_count as usize];
        let mut hard_breaks = vec![u64::MAX; info.hard_break_count as usize];
        let mut strings = vec![0xa5; info.string_bytes as usize];
        let mut copied_info = super::EvimEffectBatchInfoV1::default();
        assert_eq!(
            unsafe {
                super::evim_effect_batch_copy(
                    handle,
                    writes.as_mut_ptr(),
                    writes.len() as u64,
                    requests.as_mut_ptr(),
                    requests.len() as u64 - 1,
                    options.as_mut_ptr(),
                    options.len() as u64,
                    marks.as_mut_ptr(),
                    marks.len() as u64,
                    registers.as_mut_ptr(),
                    registers.len() as u64,
                    jumps.as_mut_ptr(),
                    jumps.len() as u64,
                    text_lines.as_mut_ptr(),
                    text_lines.len() as u64,
                    file_formats.as_mut_ptr(),
                    file_formats.len() as u64,
                    hard_breaks.as_mut_ptr(),
                    hard_breaks.len() as u64,
                    strings.as_mut_ptr(),
                    strings.len() as u64,
                    &mut copied_info,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(copied_info, info);
        assert!(writes.iter().all(|write| write.target == u32::MAX));
        assert!(requests.iter().all(|request| request.kind == u32::MAX));
        assert!(options.iter().all(|option| option.name == u32::MAX));
        assert!(marks.iter().all(|mark| mark.name == u32::MAX));
        assert!(registers.iter().all(|register| register.name == u32::MAX));
        assert!(jumps.iter().all(|jump| jump.flags == u32::MAX));
        assert!(text_lines
            .iter()
            .all(|line| line.hard_line_index == u64::MAX));
        assert!(file_formats.iter().all(|value| *value == u32::MAX));
        assert!(hard_breaks.iter().all(|value| *value == u64::MAX));
        assert!(strings.iter().all(|value| *value == 0xa5));

        assert_eq!(
            unsafe {
                super::evim_effect_batch_copy(
                    handle,
                    writes.as_mut_ptr(),
                    writes.len() as u64,
                    requests.as_mut_ptr(),
                    requests.len() as u64,
                    options.as_mut_ptr(),
                    options.len() as u64,
                    marks.as_mut_ptr(),
                    marks.len() as u64,
                    registers.as_mut_ptr(),
                    registers.len() as u64,
                    jumps.as_mut_ptr(),
                    jumps.len() as u64,
                    text_lines.as_mut_ptr(),
                    text_lines.len() as u64,
                    file_formats.as_mut_ptr(),
                    file_formats.len() as u64,
                    hard_breaks.as_mut_ptr(),
                    hard_breaks.len() as u64,
                    strings.as_mut_ptr(),
                    strings.len() as u64,
                    &mut copied_info,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(copied_info, info);
        assert_eq!(writes[0].target, super::EVIM_CLIPBOARD_TARGET_CLIPBOARD);
        assert_eq!(writes[0].flags, 0);
        assert_eq!(writes[0].register_kind, super::EVIM_REGISTER_KIND_NONE);
        assert_eq!(referenced_text(&strings, writes[0].plain_text), "plain 👋");
        assert_eq!(writes[1].target, super::EVIM_CLIPBOARD_TARGET_PRIMARY);
        assert_eq!(
            writes[1].flags,
            super::EVIM_CLIPBOARD_WRITE_HAS_PORTABLE_REGISTER
        );
        assert_eq!(writes[1].register_kind, super::EVIM_REGISTER_KIND_LINE);
        assert_eq!(referenced_text(&strings, writes[1].plain_text), "α\nβ");
        assert_eq!(hard_breaks, vec![2, 3]);

        assert_eq!(
            requests
                .iter()
                .map(|request| request.kind)
                .collect::<Vec<_>>(),
            (super::EVIM_EX_FRONTEND_EDIT..=super::EVIM_EX_FRONTEND_NORMAL).collect::<Vec<_>>()
        );
        assert!(requests
            .iter()
            .all(|request| { request.document_id == 0xdecaf && request.document_revision == 42 }));
        assert_eq!(
            requests[0].flags,
            super::EVIM_EX_FRONTEND_FORCE | super::EVIM_EX_FRONTEND_HAS_PATH
        );
        assert_eq!(referenced_text(&strings, requests[0].text), "文章/é.md");
        assert_eq!(
            requests[2].flags,
            super::EVIM_EX_FRONTEND_FORCE
                | super::EVIM_EX_FRONTEND_HAS_PATH
                | super::EVIM_EX_FRONTEND_HAS_RANGE
        );
        assert_eq!(referenced_text(&strings, requests[2].text), "写す.txt");
        assert_eq!(
            (requests[2].hard_line_start, requests[2].hard_line_end),
            (2, 4)
        );
        assert_eq!(referenced_text(&strings, requests[3].text), "別名.md");
        assert_eq!(referenced_text(&strings, requests[7].text), "終了.txt");
        assert_eq!(referenced_text(&strings, requests[9].text), "aéλ");
        assert_eq!(referenced_text(&strings, requests[10].text), "\"+ß");
        assert_eq!(
            (requests[12].first_option, requests[12].option_count),
            (0, 4)
        );
        assert_eq!(
            (requests[9].first_payload, requests[9].payload_count),
            (0, 1)
        );
        assert_eq!(
            (requests[10].first_payload, requests[10].payload_count),
            (0, 1)
        );
        assert_eq!(
            (requests[11].first_payload, requests[11].payload_count),
            (0, 1)
        );
        assert_eq!(
            options.iter().map(|option| option.name).collect::<Vec<_>>(),
            vec![
                super::EVIM_EX_OPTION_WRAP,
                super::EVIM_EX_OPTION_IGNORECASE,
                super::EVIM_EX_OPTION_FILE_FORMAT,
                super::EVIM_EX_OPTION_FILE_FORMATS,
            ]
        );
        assert_eq!(options[0].scalar_value, 1);
        assert_eq!(options[1].scalar_value, 0);
        assert_eq!(options[2].scalar_value, super::EVIM_FILE_FORMAT_DOS);
        assert_eq!(
            (options[3].first_file_format, options[3].file_format_count),
            (0, 3)
        );
        assert_eq!(
            file_formats,
            vec![
                super::EVIM_FILE_FORMAT_UNIX,
                super::EVIM_FILE_FORMAT_DOS,
                super::EVIM_FILE_FORMAT_MAC,
            ]
        );
        assert_eq!(
            requests[13].flags,
            super::EVIM_EX_FRONTEND_HAS_RANGE
                | super::EVIM_EX_FRONTEND_NUMBER
                | super::EVIM_EX_FRONTEND_LIST
        );
        assert_eq!(
            (requests[13].hard_line_start, requests[13].hard_line_end),
            (5, 7)
        );
        assert_eq!(
            (requests[13].first_payload, requests[13].payload_count),
            (0, 2)
        );
        assert_eq!(
            requests[14].flags,
            super::EVIM_EX_FRONTEND_HAS_RANGE | super::EVIM_EX_FRONTEND_LITERAL
        );
        assert_eq!(referenced_text(&strings, requests[14].text), "gUiwé");
        assert_eq!(
            (requests[14].hard_line_start, requests[14].hard_line_end),
            (8, 9)
        );
        assert_eq!(marks[0].name, u32::from('é'));
        assert_eq!(marks[0].utf8_offset, 12);
        assert_eq!(marks[0].grapheme_column, 1);
        assert_eq!(referenced_text(&strings, marks[0].line_text), "héllo λ");
        assert_eq!(registers[0].name, u32::from('+'));
        assert_eq!(
            registers[0].register_kind,
            super::EVIM_REGISTER_KIND_CHARACTER
        );
        assert_eq!(referenced_text(&strings, registers[0].text), "一\n二");
        assert_eq!(
            (registers[0].first_hard_break, registers[0].hard_break_count),
            (1, 1)
        );
        assert_eq!(jumps[0].flags, super::EVIM_EX_JUMP_CURRENT);
        assert_eq!(jumps[0].list_index, 3);
        assert_eq!(referenced_text(&strings, jumps[0].line_text), "jump 🦀");
        assert_eq!(text_lines[0].hard_line_index, 5);
        assert_eq!(referenced_text(&strings, text_lines[0].text), "print é");
        assert_eq!(referenced_text(&strings, text_lines[1].text), "終");

        let mut history_ex = ExOutcome::default();
        history_ex.navigation = Some(ExNavigation::HistoryRestoration);
        let history_batch = super::OwnedEffectBatch {
            document_id: DocumentId(1),
            document_revision: Revision(2),
            ex_outcome: Some(history_ex),
            ex_info_payloads: Vec::new(),
            clipboard_writes: Vec::new(),
        };
        let history = super::export_effect_batch(99, &history_batch).unwrap();
        assert_eq!(
            history.info.flags,
            super::EVIM_EFFECT_BATCH_HAS_EX_OUTCOME
                | super::EVIM_EFFECT_BATCH_EX_HAS_NAVIGATION
                | super::EVIM_EFFECT_BATCH_EX_NAVIGATION_HISTORY
        );

        assert_eq!(super::evim_effect_batch_release(handle), EvimStatus::Ok);
        info.batch_handle = u64::MAX;
        assert_eq!(
            unsafe { super::evim_effect_batch_info(handle, &mut info) },
            EvimStatus::InvalidHandle
        );
        assert_eq!(info, super::EvimEffectBatchInfoV1::default());
        assert_eq!(
            super::evim_effect_batch_release(handle),
            EvimStatus::InvalidHandle
        );
        assert_eq!(
            super::evim_effect_batch_release(0),
            EvimStatus::InvalidHandle
        );
    }

    #[test]
    fn formatted_projection_ffi_is_exact_unicode_safe_and_all_or_none() {
        let text = "Aé👩‍💻e\u{301}\nlast";
        let handle = register_core(Core::new(Document::new(text))).unwrap();

        let mut info = EvimFormattedSnapshotInfoV1::default();
        assert_eq!(
            unsafe { evim_core_formatted_snapshot_info(handle, &mut info) },
            EvimStatus::Ok
        );
        assert_eq!(info.utf8_length, text.len() as u64);
        assert_eq!(info.utf16_length, text.encode_utf16().count() as u64);
        assert_eq!(info.hard_line_count, 2);

        let range = EvimFormattedUtf8RangeV1 {
            struct_size: super::EVIM_FORMATTED_UTF8_RANGE_V1_SIZE,
            reserved: 0,
            identity: info.identity,
            utf8_start: 3,
            utf8_end: 17,
        };
        let mut required = u64::MAX;
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(required, 14);

        let mut short = vec![0xcc; required as usize - 1];
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    short.as_mut_ptr(),
                    short.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert!(short.iter().all(|byte| *byte == 0xcc));

        let mut copied = vec![0; required as usize];
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &range,
                    copied.as_mut_ptr(),
                    copied.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(std::str::from_utf8(&copied).unwrap(), "👩‍💻e\u{301}");

        let utf8 = [0, 1, 3, 7, 10, 14, 15, 17, 18, 22];
        let expected_utf16 = [0, 1, 2, 4, 5, 7, 8, 9, 10, 14];
        required = 0;
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(required, utf8.len() as u64);

        let mut short_offsets = vec![u64::MAX; utf8.len() - 1];
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    short_offsets.as_mut_ptr(),
                    short_offsets.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert!(short_offsets.iter().all(|offset| *offset == u64::MAX));

        let mut utf16 = vec![u64::MAX; utf8.len()];
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    utf8.as_ptr(),
                    utf8.len() as u64,
                    utf16.as_mut_ptr(),
                    utf16.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(utf16, expected_utf16);

        let mut round_trip = vec![u64::MAX; utf8.len()];
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf16_to_utf8(
                    handle,
                    &info.identity,
                    utf16.as_ptr(),
                    utf16.len() as u64,
                    round_trip.as_mut_ptr(),
                    round_trip.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(round_trip, utf8);

        let bad_utf8 = [0, 2];
        let mut untouched = [77, 88];
        required = 91;
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    bad_utf8.as_ptr(),
                    bad_utf8.len() as u64,
                    untouched.as_mut_ptr(),
                    untouched.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::InvalidUtf8Boundary
        );
        assert_eq!(untouched, [77, 88]);
        assert_eq!(required, 0);

        let bad_utf16 = [0, 3];
        required = 91;
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf16_to_utf8(
                    handle,
                    &info.identity,
                    bad_utf16.as_ptr(),
                    bad_utf16.len() as u64,
                    untouched.as_mut_ptr(),
                    untouched.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::InvalidUtf16Boundary
        );
        assert_eq!(untouched, [77, 88]);
        assert_eq!(required, 0);

        let mut point = EvimFormattedPointInfoV1::default();
        assert_eq!(
            unsafe { evim_core_formatted_point_info(handle, &info.identity, 17, &mut point) },
            EvimStatus::Ok
        );
        assert_eq!(point.identity, info.identity);
        assert_eq!(point.utf16_offset, 9);
        assert_eq!(point.hard_line_index, 0);
        assert_eq!((point.hard_line_start, point.hard_line_end), (0, 17));
        assert_eq!(point.grapheme_column, 4);

        assert_eq!(
            unsafe { evim_core_formatted_point_info(handle, &info.identity, 15, &mut point) },
            EvimStatus::NotGraphemeBoundary
        );
        assert_eq!(point, EvimFormattedPointInfoV1::default());

        let invalid_range = EvimFormattedUtf8RangeV1 {
            utf8_start: 2,
            utf8_end: 3,
            ..range
        };
        let mut range_output = [0xa5; 4];
        required = 91;
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &invalid_range,
                    range_output.as_mut_ptr(),
                    range_output.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::InvalidUtf8Boundary
        );
        assert_eq!(range_output, [0xa5; 4]);
        assert_eq!(required, 0);

        let mut stale = info.identity;
        stale.document_revision += 1;
        let stale_range = EvimFormattedUtf8RangeV1 {
            identity: stale,
            ..range
        };
        required = 91;
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &stale_range,
                    range_output.as_mut_ptr(),
                    range_output.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::StaleRevision
        );
        assert_eq!(range_output, [0xa5; 4]);
        assert_eq!(required, 0);
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

    #[test]
    fn bounded_projection_ffi_does_not_materialize_compatibility_text() {
        let mut document = Document::new("x\n".repeat(50_000));
        document.insert(50_000, "Z").unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
        let handle = register_core(Core::new(document)).unwrap();

        let mut info = EvimFormattedSnapshotInfoV1::default();
        assert_eq!(
            unsafe { evim_core_formatted_snapshot_info(handle, &mut info) },
            EvimStatus::Ok
        );
        let request = EvimFormattedUtf8RangeV1 {
            struct_size: super::EVIM_FORMATTED_UTF8_RANGE_V1_SIZE,
            identity: info.identity,
            utf8_start: 49_996,
            utf8_end: 50_008,
            ..EvimFormattedUtf8RangeV1::default()
        };
        let mut output = [0_u8; 12];
        let mut required = 0;
        assert_eq!(
            unsafe {
                evim_core_copy_formatted_utf8_range(
                    handle,
                    &request,
                    output.as_mut_ptr(),
                    output.len() as u64,
                    &mut required,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(required, 12);
        let input = [50_000];
        let mut mapped = [0];
        assert_eq!(
            unsafe {
                evim_core_map_formatted_utf8_to_utf16(
                    handle,
                    &info.identity,
                    input.as_ptr(),
                    1,
                    mapped.as_mut_ptr(),
                    1,
                    &mut required,
                )
            },
            EvimStatus::Ok
        );
        let mut point = EvimFormattedPointInfoV1::default();
        assert_eq!(
            unsafe { evim_core_formatted_point_info(handle, &info.identity, 50_000, &mut point) },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert!(!lease
                .core()
                .document()
                .projection()
                .compatibility_text_is_materialized());
        }
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

    fn style_arena_text(arena: &[u8], range: EvimStyleStringRefV1) -> &str {
        let start = usize::try_from(range.offset).unwrap();
        let length = usize::try_from(range.length).unwrap();
        std::str::from_utf8(&arena[start..start + length]).unwrap()
    }

    #[test]
    fn style_sheet_export_is_exact_typed_and_two_pass_without_partial_writes() {
        let handle = register_core(Core::new(Document::new("plain"))).unwrap();
        let mut info = EvimStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { evim_core_style_sheet_info(handle, &mut info) },
            EvimStatus::Ok
        );
        assert_eq!(info.definition_count, 20);
        assert_eq!(info.property_count, 421);
        assert_ne!(info.string_bytes, 0);

        let mut count_info = EvimStyleSheetInfoV1::default();
        assert_eq!(
            unsafe {
                evim_core_copy_style_sheet(
                    handle,
                    &info.identity,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut count_info,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(count_info, info);

        let mut definitions =
            vec![EvimStyleDefinitionV1::default(); info.definition_count as usize];
        let mut properties = vec![EvimStylePropertyV1::default(); info.property_count as usize];
        let mut value_items = vec![EvimStyleValueItemV1::default(); info.value_item_count as usize];
        let mut dependencies =
            vec![EvimStyleDependencyV1::default(); info.dependency_count as usize];
        let mut strings = vec![0_u8; info.string_bytes as usize];
        let mut copied = EvimStyleSheetInfoV1::default();
        assert_eq!(
            unsafe {
                evim_core_copy_style_sheet(
                    handle,
                    &info.identity,
                    definitions.as_mut_ptr(),
                    definitions.len() as u64,
                    properties.as_mut_ptr(),
                    properties.len() as u64,
                    value_items.as_mut_ptr(),
                    value_items.len() as u64,
                    dependencies.as_mut_ptr(),
                    dependencies.len() as u64,
                    strings.as_mut_ptr(),
                    strings.len() as u64,
                    &mut copied,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(copied, info);

        let document = definitions
            .iter()
            .find(|definition| style_arena_text(&strings, definition.stable_id) == "Document")
            .unwrap();
        assert_eq!(
            style_arena_text(&strings, document.display_name),
            "Base Document"
        );
        assert_ne!(
            style_arena_text(&strings, document.stable_id),
            style_arena_text(&strings, document.display_name)
        );
        assert_eq!(document.namespace, super::EVIM_STYLE_NAMESPACE_BLOCK);
        assert_eq!(document.role, super::EVIM_STYLE_ROLE_DOCUMENT);
        assert_eq!(
            document.origin,
            super::EVIM_STYLE_ORIGIN_GENERATED_CONFIGURATION
        );
        assert_ne!(
            document.capabilities & super::EVIM_STYLE_CAPABILITY_EDIT_DECLARATIONS,
            0
        );
        assert_eq!(
            document.capabilities & super::EVIM_STYLE_CAPABILITY_EDIT_PARENT,
            0
        );

        let document_properties = &properties[document.first_property as usize
            ..(document.first_property + document.property_count) as usize];
        let size = document_properties
            .iter()
            .find(|property| property.property == super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE)
            .unwrap();
        assert_ne!(size.flags & super::EVIM_STYLE_PROPERTY_DECLARED, 0);
        assert_eq!(size.declared.kind, super::EVIM_STYLE_VALUE_FLOAT);
        assert_eq!(size.declared.number, 14.0);
        assert_eq!(size.effective.number, 14.0);
        assert_eq!(
            size.contributor_kind,
            super::EVIM_STYLE_CONTRIBUTOR_BLOCK_STYLE
        );
        assert_eq!(
            style_arena_text(&strings, size.contributor_style_id),
            "Document"
        );

        let families = document_properties
            .iter()
            .find(|property| {
                property.property == super::EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES
            })
            .unwrap();
        assert_eq!(families.declared.kind, super::EVIM_STYLE_VALUE_STRING_LIST);
        let family = value_items[families.declared.first_item as usize];
        assert_eq!(family.kind, super::EVIM_STYLE_VALUE_ITEM_STRING);
        assert_eq!(style_arena_text(&strings, family.string), "SF Pro");

        let heading = definitions
            .iter()
            .find(|definition| style_arena_text(&strings, definition.stable_id) == "Heading1")
            .unwrap();
        assert_eq!(
            style_arena_text(&strings, heading.display_name),
            "Heading 1"
        );
        assert_eq!(style_arena_text(&strings, heading.parent_id), "Paragraph");
        assert_eq!(
            style_arena_text(&strings, heading.next_style_id),
            "Paragraph"
        );
        let heading_properties = &properties[heading.first_property as usize
            ..(heading.first_property + heading.property_count) as usize];
        let inherited_family = heading_properties
            .iter()
            .find(|property| {
                property.property == super::EVIM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES
            })
            .unwrap();
        assert_eq!(
            inherited_family.contributor_kind,
            super::EVIM_STYLE_CONTRIBUTOR_BLOCK_STYLE
        );
        assert_eq!(
            style_arena_text(&strings, inherited_family.contributor_style_id),
            "Document"
        );
        assert!(inherited_family.dependency_count >= 3);

        // Every output remains untouched when just one capacity is short.
        let sentinel_definition = EvimStyleDefinitionV1 {
            role: u32::MAX,
            ..EvimStyleDefinitionV1::default()
        };
        let sentinel_property = EvimStylePropertyV1 {
            property: u32::MAX,
            ..EvimStylePropertyV1::default()
        };
        definitions.fill(sentinel_definition);
        properties.fill(sentinel_property);
        value_items.iter_mut().for_each(|item| item.kind = u32::MAX);
        dependencies
            .iter_mut()
            .for_each(|dependency| dependency.namespace = u32::MAX);
        strings.fill(0xA5);
        assert_eq!(
            unsafe {
                evim_core_copy_style_sheet(
                    handle,
                    &info.identity,
                    definitions.as_mut_ptr(),
                    definitions.len() as u64,
                    properties.as_mut_ptr(),
                    properties.len() as u64 - 1,
                    value_items.as_mut_ptr(),
                    value_items.len() as u64,
                    dependencies.as_mut_ptr(),
                    dependencies.len() as u64,
                    strings.as_mut_ptr(),
                    strings.len() as u64,
                    &mut copied,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert!(definitions
            .iter()
            .all(|value| *value == sentinel_definition));
        assert!(properties.iter().all(|value| *value == sentinel_property));
        assert!(value_items.iter().all(|value| value.kind == u32::MAX));
        assert!(dependencies.iter().all(|value| value.namespace == u32::MAX));
        assert!(strings.iter().all(|byte| *byte == 0xA5));

        let mut stale = info.identity;
        stale.style_sheet_revision += 1;
        assert_eq!(
            unsafe {
                evim_core_copy_style_sheet(
                    handle,
                    &stale,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut copied,
                )
            },
            EvimStatus::StaleRevision
        );
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

    #[test]
    fn style_export_represents_every_value_kind() {
        fn configure(document: &mut Document, edit: StyleDefinitionEdit) {
            let request = StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit)),
            );
            document.apply_style_request(request).unwrap();
        }

        let mut document = Document::new("plain");
        let mut root = document
            .projection()
            .style_sheet()
            .block_style(&document.projection().style_sheet().base_document)
            .unwrap()
            .clone();
        root.character.background = Some(Color {
            red: 0.1,
            green: 0.2,
            blue: 0.3,
            alpha: 0.4,
        });
        root.character.language = Some("en-US".to_owned());
        root.character.open_type_features = Some(std::collections::BTreeMap::from([
            ("kern".to_owned(), 1),
            ("liga".to_owned(), 0),
        ]));
        configure(&mut document, StyleDefinitionEdit::UpdateBlock(root));
        let mut paragraph = document
            .projection()
            .style_sheet()
            .block_style(&document.projection().style_sheet().base_paragraph)
            .unwrap()
            .clone();
        paragraph.block.line_spacing = Some(LineSpacing::Exact(19.0));
        paragraph.block.alignment = Some(ParagraphAlignment::Center);
        configure(&mut document, StyleDefinitionEdit::UpdateBlock(paragraph));

        let export = export_style_sheet(&document).unwrap();
        let kinds = export
            .properties
            .iter()
            .filter(|property| property.flags & super::EVIM_STYLE_PROPERTY_DECLARED != 0)
            .map(|property| property.declared.kind)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_FLOAT));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_UNSIGNED));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_BOOLEAN));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_COLOR));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_STRING));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_STRING_LIST));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_FONT_SLANT));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_WRITING_DIRECTION));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_OPEN_TYPE_FEATURES));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_LINE_SPACING));
        assert!(kinds.contains(&super::EVIM_STYLE_VALUE_PARAGRAPH_ALIGNMENT));
        assert!(export.value_items.iter().any(|item| {
            item.kind == super::EVIM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE
                && style_arena_text(&export.strings, item.string) == "kern"
                && item.unsigned_value == 1
        }));
    }

    struct PaintTestResponse {
        _carets: Box<[EvimClusterCaretStopV1]>,
        clusters: Box<[EvimShapedClusterV1]>,
        visual_order: Box<[u64]>,
    }

    #[derive(Default)]
    struct PaintTestProviderStorage {
        responses: Vec<PaintTestResponse>,
    }

    unsafe extern "C" fn paint_test_metrics_generation(_context: *mut c_void) -> u64 {
        1
    }

    unsafe extern "C" fn paint_test_shape_batch(
        context: *mut c_void,
        requests: *const EvimShapeRequestV1,
        request_count: u64,
        responses: *mut EvimShapeResponseV1,
        response_capacity: u64,
    ) -> u32 {
        let Ok(count) = usize::try_from(request_count) else {
            return EvimStatus::LengthOverflow as u32;
        };
        if request_count > response_capacity
            || context.is_null()
            || (count != 0 && (requests.is_null() || responses.is_null()))
        {
            return EvimStatus::InvalidArgument as u32;
        }
        // SAFETY: This test callback receives the exact arrays advertised by
        // the in-process C-provider adapter for the synchronous call.
        let requests = unsafe { std::slice::from_raw_parts(requests, count) };
        let responses = unsafe { std::slice::from_raw_parts_mut(responses, count) };
        // SAFETY: The test retains this context until after core destruction.
        let storage = unsafe { &mut *(context as *mut PaintTestProviderStorage) };
        storage.responses.clear();
        storage.responses.reserve(count);
        for (index, (request, response)) in requests.iter().zip(responses).enumerate() {
            let metrics = EvimTextMetricsV1 {
                ascent: 10.0,
                descent: 3.0,
                leading: 1.0,
            };
            let owned = if request.text_start == request.text_end {
                PaintTestResponse {
                    _carets: Box::new([]),
                    clusters: Box::new([]),
                    visual_order: Box::new([]),
                }
            } else {
                let advance = 8.0;
                let carets = vec![
                    EvimClusterCaretStopV1 {
                        text_offset: request.text_start,
                        inline_offset: 0.0,
                        affinity: EVIM_BOUNDARY_AFFINITY_DOWNSTREAM,
                    },
                    EvimClusterCaretStopV1 {
                        text_offset: request.text_end,
                        inline_offset: advance,
                        affinity: EVIM_BOUNDARY_AFFINITY_UPSTREAM,
                    },
                ]
                .into_boxed_slice();
                let has_render_run = request.purpose == EVIM_SHAPE_PURPOSE_METRICS_AND_RENDER_DATA;
                let cluster = EvimShapedClusterV1 {
                    struct_size: EVIM_SHAPED_CLUSTER_V1_SIZE,
                    reserved: 0,
                    text_start: request.text_start,
                    text_end: request.text_end,
                    advance,
                    metrics,
                    typographic_bounds: EvimShapedBoundsV1 {
                        x: 0.0,
                        y: -10.0,
                        width: advance,
                        height: 13.0,
                    },
                    ink_bounds: EvimShapedBoundsV1 {
                        x: 0.0,
                        y: -10.0,
                        width: advance,
                        height: 13.0,
                    },
                    bidi_level: 0,
                    has_render_run: u32::from(has_render_run),
                    fallback_font: EvimUtf8Slice {
                        data: PAINT_TEST_FONT.as_ptr(),
                        length: PAINT_TEST_FONT.len() as u64,
                    },
                    caret_stops: carets.as_ptr(),
                    caret_stop_count: carets.len() as u64,
                    render_run: if has_render_run {
                        EvimRenderRunHandleV1 {
                            owner: request.render_run_owner,
                            identifier: index as u64 + 1,
                            metrics_generation: request.metrics_generation,
                            threading: request.render_run_threading,
                            reserved: 0,
                        }
                    } else {
                        EvimRenderRunHandleV1::default()
                    },
                };
                PaintTestResponse {
                    _carets: carets,
                    clusters: vec![cluster].into_boxed_slice(),
                    visual_order: vec![0].into_boxed_slice(),
                }
            };
            storage.responses.push(owned);
            let owned = storage.responses.last().expect("response was just pushed");
            *response = EvimShapeResponseV1 {
                document_id: request.document_id,
                document_revision: request.document_revision,
                measurement_environment_id: request.measurement_environment_id,
                metrics_generation: request.metrics_generation,
                text_start: request.text_start,
                text_end: request.text_end,
                clusters: if owned.clusters.is_empty() {
                    ptr::null()
                } else {
                    owned.clusters.as_ptr()
                },
                cluster_count: owned.clusters.len() as u64,
                visual_order: if owned.visual_order.is_empty() {
                    ptr::null()
                } else {
                    owned.visual_order.as_ptr()
                },
                visual_order_count: owned.visual_order.len() as u64,
                default_metrics: metrics,
                ..EvimShapeResponseV1::default()
            };
        }
        EvimStatus::Ok as u32
    }

    fn paint_test_provider(
        storage: &mut PaintTestProviderStorage,
        environment: u64,
        owner: u64,
    ) -> CTextMeasurementProvider {
        CTextMeasurementProvider {
            context: (storage as *mut PaintTestProviderStorage) as usize,
            abi_version: EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1,
            measurement_environment_id: MeasurementEnvironmentId(environment),
            threading: ProviderThreading::AnyWorker,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(owner),
                threading: RenderRunThreading::AnyThread,
            }),
            metrics_generation_callback: paint_test_metrics_generation,
            shape_batch_callback: paint_test_shape_batch,
        }
    }

    fn style_float_request(
        identity: super::EvimStyleSheetIdentityV1,
        style_id: &[u8],
        property: u32,
        value: f32,
    ) -> EvimStyleEditV1 {
        EvimStyleEditV1 {
            identity,
            namespace: super::EVIM_STYLE_NAMESPACE_BLOCK,
            operation: super::EVIM_STYLE_EDIT_SET_DECLARATION,
            property,
            style_id: EvimUtf8Slice {
                data: style_id.as_ptr(),
                length: style_id.len() as u64,
            },
            value: super::EvimStyleEditValueV1 {
                kind: super::EVIM_STYLE_VALUE_FLOAT,
                number: value,
                ..super::EvimStyleEditValueV1::default()
            },
            ..EvimStyleEditV1::default()
        }
    }

    fn current_style_identity(handle: super::EvimCoreHandle) -> super::EvimStyleSheetIdentityV1 {
        let mut info = EvimStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { evim_core_style_sheet_info(handle, &mut info) },
            EvimStatus::Ok
        );
        info.identity
    }

    #[test]
    fn ffi_semantic_style_is_selection_bound_layout_independent_and_undoable() {
        let document = Document::from_bytes_with_file_format(
            b"alpha beta".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut storage = PaintTestProviderStorage::default();
        let mut core = Core::new(document);
        let view = core.add_view(paint_test_provider(&mut storage, 501, 601), 240.0, 80.0);
        for input in [Key::Char('v'), Key::Char('4'), Key::Char('l')] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
        }
        let handle = register_core(core).unwrap();

        let mut presentation = super::EvimSemanticStylePresentationV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::EVIM_SEMANTIC_STYLE_STRONG,
                    &mut presentation,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(presentation.style, super::EVIM_SEMANTIC_STYLE_STRONG);
        assert_eq!(presentation.state, super::EVIM_SEMANTIC_STYLE_STATE_OFF);
        assert_eq!(
            presentation.flags,
            super::EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE | super::EVIM_SEMANTIC_STYLE_CAN_SET
        );
        assert_eq!(
            presentation.selection.kind,
            super::EVIM_LOGICAL_SELECTION_KIND_CHARACTER
        );
        assert_eq!(presentation.selection.text_start, 0);
        assert_eq!(presentation.selection.text_end, 5);

        let mut stale_selection = presentation.selection;
        stale_selection.state_identity[0] ^= 0xff;
        let stale_request = super::EvimSetSemanticStyleV1 {
            struct_size: super::EVIM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::EVIM_SEMANTIC_STYLE_STRONG,
            enabled: 1,
            reserved: 0,
            expected_selection: stale_selection,
        };
        let mut outcome = EvimCoreOutcomeV1 {
            document_revision: u64::MAX,
            ..EvimCoreOutcomeV1::default()
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &stale_request,
                    &mut outcome,
                )
            },
            EvimStatus::StaleRevision
        );
        assert_eq!(outcome, EvimCoreOutcomeV1::default());

        let set_strong = super::EvimSetSemanticStyleV1 {
            expected_selection: presentation.selection,
            ..stale_request
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_set_semantic_style(handle, view.0, &set_strong, &mut outcome)
            },
            EvimStatus::Ok
        );
        assert_ne!(outcome.flags & super::EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
        assert_ne!(outcome.flags & super::EVIM_OUTCOME_LAYOUT_CHANGED, 0);
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"**alpha** beta");
        }

        assert_eq!(
            unsafe {
                super::evim_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::EVIM_SEMANTIC_STYLE_STRONG,
                    &mut presentation,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(presentation.state, super::EVIM_SEMANTIC_STYLE_STATE_ON);
        assert_ne!(presentation.flags & super::EVIM_SEMANTIC_STYLE_CAN_CLEAR, 0);
        let clear_strong = super::EvimSetSemanticStyleV1 {
            struct_size: super::EVIM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::EVIM_SEMANTIC_STYLE_STRONG,
            enabled: 0,
            reserved: 0,
            expected_selection: presentation.selection,
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &clear_strong,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );

        assert_eq!(
            unsafe {
                super::evim_core_view_semantic_style_presentation(
                    handle,
                    view.0,
                    super::EVIM_SEMANTIC_STYLE_EMPHASIS,
                    &mut presentation,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(presentation.state, super::EVIM_SEMANTIC_STYLE_STATE_OFF);
        assert_ne!(presentation.flags & super::EVIM_SEMANTIC_STYLE_CAN_SET, 0);
        let set_emphasis = super::EvimSetSemanticStyleV1 {
            struct_size: super::EVIM_SET_SEMANTIC_STYLE_V1_SIZE,
            style: super::EVIM_SEMANTIC_STYLE_EMPHASIS,
            enabled: 1,
            reserved: 0,
            expected_selection: presentation.selection,
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_set_semantic_style(
                    handle,
                    view.0,
                    &set_emphasis,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"*alpha* beta");
        }
        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, view.0, &mut outcome) },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), b"alpha beta");
        }
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

    #[test]
    fn ffi_live_style_group_is_one_visible_undo_unit_and_survives_rejected_edits() {
        let document = Document::from_bytes_with_file_format(
            b"# heading\nbody".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut first_storage = PaintTestProviderStorage::default();
        let mut second_storage = PaintTestProviderStorage::default();
        let mut core = Core::new(document);
        let first = core.add_view(
            paint_test_provider(&mut first_storage, 101, 201),
            400.0,
            200.0,
        );
        let second = core.add_view(
            paint_test_provider(&mut second_storage, 102, 202),
            400.0,
            200.0,
        );
        let initial_history_nodes = core.document().history_status().node_count;
        let initial_style = core
            .document()
            .projection()
            .style_sheet()
            .block_style(&StyleId::from("Heading1"))
            .unwrap()
            .character
            .clone();
        let handle = register_core(core).unwrap();

        let initial_identity = current_style_identity(handle);
        let mut group = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &initial_identity,
                    &mut group,
                )
            },
            EvimStatus::Ok
        );
        assert_ne!(group.token, 0);
        assert_eq!(group.view_id, first.0);
        assert_eq!(group.document_id, initial_identity.document_id);
        assert_eq!(
            group.begin_document_revision,
            initial_identity.document_revision
        );
        assert_eq!(
            group.begin_style_sheet_revision,
            initial_identity.style_sheet_revision
        );

        let heading = b"Heading1";
        let size = style_float_request(
            initial_identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            31.0,
        );
        let mut outcome = EvimCoreOutcomeV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &size,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_ne!(outcome.flags & super::EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(
                lease.core().document().history_status().node_count,
                initial_history_nodes + 1
            );
            assert_eq!(
                lease
                    .core()
                    .document()
                    .projection()
                    .style_sheet()
                    .block_style(&StyleId::from("Heading1"))
                    .unwrap()
                    .character
                    .size,
                Some(31.0)
            );
        }

        // Every grouped edit is still exact-revision bound. A stale request
        // neither changes nor consumes the group.
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &size,
                    &mut outcome,
                )
            },
            EvimStatus::StaleRevision
        );
        let current_identity = current_style_identity(handle);
        let no_op = style_float_request(
            current_identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            31.0,
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &no_op,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(outcome.flags & super::EVIM_OUTCOME_DOCUMENT_CHANGED, 0);

        let invalid_role = style_float_request(
            current_identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
            7.0,
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &invalid_role,
                    &mut outcome,
                )
            },
            EvimStatus::IncompatibleStyleRole
        );

        let underline = EvimStyleEditV1 {
            identity: current_identity,
            namespace: super::EVIM_STYLE_NAMESPACE_BLOCK,
            operation: super::EVIM_STYLE_EDIT_SET_DECLARATION,
            property: super::EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            style_id: EvimUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::EvimStyleEditValueV1 {
                kind: super::EVIM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..super::EvimStyleEditValueV1::default()
            },
            ..EvimStyleEditV1::default()
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .history_status()
                .node_count,
            initial_history_nodes + 1
        );

        // Ownership and immutable token fields are checked without consuming
        // the valid group.
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    second.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            EvimStatus::StyleEditGroupWrongOwner
        );
        let mut forged = group;
        forged.token += 1;
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &forged,
                    &underline,
                    &mut outcome,
                )
            },
            EvimStatus::InvalidStyleEditGroup
        );
        forged = group;
        forged.document_id += 1;
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &forged) },
            EvimStatus::InvalidArgument
        );
        let mut second_group = super::EvimStyleEditGroupV1::default();
        let latest_identity = current_style_identity(handle);
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    second.0,
                    &latest_identity,
                    &mut second_group,
                )
            },
            EvimStatus::StyleEditGroupActive
        );
        assert_eq!(second_group.token, 0);

        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &group) },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &group) },
            EvimStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &group,
                    &underline,
                    &mut outcome,
                )
            },
            EvimStatus::InvalidStyleEditGroup
        );

        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            let style = lease
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap();
            assert_eq!(style.character.size, initial_style.size);
            assert_eq!(style.character.underline, initial_style.underline);
        }
        assert_eq!(
            unsafe { super::evim_core_view_redo(handle, first.0, &mut outcome) },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            let style = lease
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap();
            assert_eq!(style.character.size, Some(31.0));
            assert_eq!(style.character.underline, Some(true));
        }

        assert_eq!(
            super::evim_core_view_remove(handle, first.0),
            EvimStatus::Ok
        );
        assert_eq!(
            super::evim_core_view_remove(handle, second.0),
            EvimStatus::Ok
        );
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

    #[test]
    fn ffi_style_groups_are_empty_elided_separate_and_closed_by_unrelated_events() {
        let document = Document::from_bytes_with_file_format(
            b"# heading".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut first_storage = PaintTestProviderStorage::default();
        let mut second_storage = PaintTestProviderStorage::default();
        let mut core = Core::new(document);
        let first = core.add_view(
            paint_test_provider(&mut first_storage, 111, 211),
            400.0,
            200.0,
        );
        let second = core.add_view(
            paint_test_provider(&mut second_storage, 112, 212),
            400.0,
            200.0,
        );
        let initial_nodes = core.document().history_status().node_count;
        let handle = register_core(core).unwrap();
        let mut outcome = EvimCoreOutcomeV1::default();

        let identity = current_style_identity(handle);
        let mut empty = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(handle, first.0, &identity, &mut empty)
            },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &empty) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .history_status()
                .node_count,
            initial_nodes
        );

        let heading = b"Heading1";
        let identity = current_style_identity(handle);
        let mut first_group = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut first_group,
                )
            },
            EvimStatus::Ok
        );
        let size_32 = style_float_request(
            identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            32.0,
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &first_group,
                    &size_32,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &first_group) },
            EvimStatus::Ok
        );

        let identity = current_style_identity(handle);
        let mut second_group = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut second_group,
                )
            },
            EvimStatus::Ok
        );
        let size_33 = style_float_request(
            identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            33.0,
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &second_group,
                    &size_33,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );

        // A view-local option is still an unrelated coordinator event. It
        // closes the successful prefix before applying the option and makes
        // the old group capability invalid.
        assert_eq!(
            unsafe { super::evim_core_view_set_wrap(handle, first.0, 0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, first.0, &second_group) },
            EvimStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap()
                .character
                .size,
            Some(32.0)
        );
        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_ne!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap()
                .character
                .size,
            Some(32.0)
        );

        // Owner removal closes a committed group before dropping its
        // restoration state. The surviving view can undo the preserved unit.
        let identity = current_style_identity(handle);
        let mut removed_owner_group = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    first.0,
                    &identity,
                    &mut removed_owner_group,
                )
            },
            EvimStatus::Ok
        );
        let size_34 = style_float_request(
            identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            34.0,
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style_in_group(
                    handle,
                    first.0,
                    &removed_owner_group,
                    &size_34,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(
            super::evim_core_view_remove(handle, first.0),
            EvimStatus::Ok
        );
        assert_eq!(
            unsafe {
                super::evim_core_view_end_style_edit_group(handle, second.0, &removed_owner_group)
            },
            EvimStatus::InvalidStyleEditGroup
        );
        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_ne!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap()
                .character
                .size,
            Some(34.0)
        );

        // Core destruction owns final cleanup. The capability cannot be used
        // after the core handle is destroyed.
        let identity = current_style_identity(handle);
        let mut destroy_group = super::EvimStyleEditGroupV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_begin_style_edit_group(
                    handle,
                    second.0,
                    &identity,
                    &mut destroy_group,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
        assert_eq!(
            unsafe { super::evim_core_view_end_style_edit_group(handle, second.0, &destroy_group) },
            EvimStatus::InvalidHandle
        );
    }

    #[test]
    fn ffi_style_edits_are_exact_undoable_and_rebase_every_view() {
        let source = b"# heading\nbody".to_vec();
        let document = Document::from_bytes_with_file_format(
            source.clone(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let mut first_storage = PaintTestProviderStorage::default();
        let mut second_storage = PaintTestProviderStorage::default();
        let mut core = Core::new(document);
        let first = core.add_view(
            paint_test_provider(&mut first_storage, 71, 81),
            400.0,
            200.0,
        );
        let second = core.add_view(
            paint_test_provider(&mut second_storage, 72, 82),
            400.0,
            200.0,
        );
        let first_layout_before = core.layout(first).unwrap().snapshot().unwrap().revision;
        let second_layout_before = core.layout(second).unwrap().snapshot().unwrap().revision;
        let handle = register_core(core).unwrap();

        let mut info = EvimStyleSheetInfoV1::default();
        assert_eq!(
            unsafe { evim_core_style_sheet_info(handle, &mut info) },
            EvimStatus::Ok
        );
        let heading = b"Heading1";
        let first_request = style_float_request(
            info.identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CHARACTER_SIZE,
            30.0,
        );
        let mut outcome = EvimCoreOutcomeV1::default();
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style(handle, first.0, &first_request, &mut outcome)
            },
            EvimStatus::Ok
        );
        assert_ne!(outcome.flags & super::EVIM_OUTCOME_DOCUMENT_CHANGED, 0);
        assert_ne!(outcome.flags & super::EVIM_OUTCOME_LAYOUT_CHANGED, 0);
        let first_revision = outcome.document_revision;

        {
            let lease = checkout_core(handle).unwrap();
            assert_eq!(lease.core().document().source_bytes(), source);
            let style = lease
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap();
            assert_eq!(style.character.size, Some(30.0));
            let first_snapshot = lease.core().layout(first).unwrap().snapshot().unwrap();
            let second_snapshot = lease.core().layout(second).unwrap().snapshot().unwrap();
            assert_eq!(first_snapshot.document_revision.0, first_revision);
            assert_eq!(second_snapshot.document_revision.0, first_revision);
            assert_ne!(first_snapshot.revision, first_layout_before);
            assert_ne!(second_snapshot.revision, second_layout_before);
        }

        // The original exact identity is stale and a failed role edit is
        // atomic at the current identity.
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style(handle, second.0, &first_request, &mut outcome)
            },
            EvimStatus::StaleRevision
        );
        assert_eq!(outcome.document_revision, 0);
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .revision()
                .0,
            first_revision
        );

        assert_eq!(
            unsafe { evim_core_style_sheet_info(handle, &mut info) },
            EvimStatus::Ok
        );
        let bad_role = style_float_request(
            info.identity,
            heading,
            super::EVIM_STYLE_PROPERTY_CANVAS_PADDING_TOP,
            4.0,
        );
        assert_eq!(
            unsafe { super::evim_core_view_edit_style(handle, first.0, &bad_role, &mut outcome) },
            EvimStatus::IncompatibleStyleRole
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .revision()
                .0,
            first_revision
        );

        let self_parent = EvimStyleEditV1 {
            identity: info.identity,
            namespace: super::EVIM_STYLE_NAMESPACE_BLOCK,
            operation: super::EVIM_STYLE_EDIT_SET_PARENT,
            style_id: EvimUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::EvimStyleEditValueV1 {
                kind: super::EVIM_STYLE_VALUE_STRING,
                text: EvimUtf8Slice {
                    data: heading.as_ptr(),
                    length: heading.len() as u64,
                },
                ..super::EvimStyleEditValueV1::default()
            },
            ..EvimStyleEditV1::default()
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_edit_style(handle, second.0, &self_parent, &mut outcome)
            },
            EvimStatus::StyleInheritanceCycle
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .revision()
                .0,
            first_revision
        );

        // A second successful field is a distinct undo unit.
        let underline = EvimStyleEditV1 {
            identity: info.identity,
            namespace: super::EVIM_STYLE_NAMESPACE_BLOCK,
            operation: super::EVIM_STYLE_EDIT_SET_DECLARATION,
            property: super::EVIM_STYLE_PROPERTY_CHARACTER_UNDERLINE,
            style_id: EvimUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::EvimStyleEditValueV1 {
                kind: super::EVIM_STYLE_VALUE_BOOLEAN,
                enum_value: 1,
                ..super::EvimStyleEditValueV1::default()
            },
            ..EvimStyleEditV1::default()
        };
        assert_eq!(
            unsafe { super::evim_core_view_edit_style(handle, second.0, &underline, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap()
                .character
                .underline,
            Some(true)
        );

        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, first.0, &mut outcome) },
            EvimStatus::Ok
        );
        {
            let lease = checkout_core(handle).unwrap();
            let style = lease
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap();
            assert_eq!(style.character.size, Some(30.0));
            assert_eq!(style.character.underline, None);
            assert_eq!(lease.core().document().source_bytes(), source);
        }
        assert_eq!(
            unsafe { super::evim_core_view_redo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Heading1"))
                .unwrap()
                .character
                .underline,
            Some(true)
        );

        // Display names are editable metadata; the opaque stable ID does not
        // change and the rename participates in the same history protocol.
        assert_eq!(
            unsafe { evim_core_style_sheet_info(handle, &mut info) },
            EvimStatus::Ok
        );
        let begin_composition = super::EvimCompositionBeginV1 {
            struct_size: super::EVIM_COMPOSITION_BEGIN_V1_SIZE,
            reserved: 0,
            document_revision: info.identity.document_revision,
            replacement_start: 0,
            replacement_end: 0,
        };
        assert_eq!(
            unsafe {
                super::evim_core_view_composition_begin(
                    handle,
                    second.0,
                    &begin_composition,
                    &mut outcome,
                )
            },
            EvimStatus::Ok
        );
        let display_name = b"Chapter Heading";
        let rename = EvimStyleEditV1 {
            identity: info.identity,
            namespace: super::EVIM_STYLE_NAMESPACE_BLOCK,
            operation: super::EVIM_STYLE_EDIT_SET_DISPLAY_NAME,
            style_id: EvimUtf8Slice {
                data: heading.as_ptr(),
                length: heading.len() as u64,
            },
            value: super::EvimStyleEditValueV1 {
                kind: super::EVIM_STYLE_VALUE_STRING,
                text: EvimUtf8Slice {
                    data: display_name.as_ptr(),
                    length: display_name.len() as u64,
                },
                ..super::EvimStyleEditValueV1::default()
            },
            ..EvimStyleEditV1::default()
        };
        assert_eq!(
            unsafe { super::evim_core_view_edit_style(handle, first.0, &rename, &mut outcome) },
            EvimStatus::Ok
        );
        assert_ne!(
            outcome.flags & super::EVIM_OUTCOME_HAS_COMPOSITION_CHANGES,
            0
        );
        {
            let lease = checkout_core(handle).unwrap();
            let sheet = lease.core().document().projection().style_sheet();
            assert!(sheet.block_style(&StyleId::from("Heading1")).is_some());
            assert_eq!(
                sheet
                    .block_style_metadata(&StyleId::from("Heading1"))
                    .unwrap()
                    .display_name,
                "Chapter Heading"
            );
        }
        assert_eq!(
            unsafe { super::evim_core_view_undo(handle, second.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style_metadata(&StyleId::from("Heading1"))
                .unwrap()
                .display_name,
            "Heading 1"
        );
        assert_eq!(
            unsafe { super::evim_core_view_redo(handle, first.0, &mut outcome) },
            EvimStatus::Ok
        );
        assert_eq!(
            checkout_core(handle)
                .unwrap()
                .core()
                .document()
                .projection()
                .style_sheet()
                .block_style_metadata(&StyleId::from("Heading1"))
                .unwrap()
                .display_name,
            "Chapter Heading"
        );

        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
    }

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
    fn styled_layout_paint_copy_is_two_pass_and_never_partial() {
        fn apply_configuration(document: &mut Document, intent: ConfigurationStyleIntent) {
            let request = StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(intent),
            );
            document.apply_style_request(request).unwrap();
        }

        let mut document = Document::from_bytes_with_file_format(
            b"# painted".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();
        let canvas = Color {
            red: 0.1,
            green: 0.2,
            blue: 0.3,
            alpha: 0.9,
        };
        apply_configuration(
            &mut document,
            ConfigurationStyleIntent::SetDocumentCanvas(BlockProperties {
                background: Some(canvas),
                ..BlockProperties::default()
            }),
        );
        let default_foreground = Color {
            red: 0.4,
            green: 0.3,
            blue: 0.2,
            alpha: 1.0,
        };
        let default_background = Color {
            red: 0.9,
            green: 0.8,
            blue: 0.7,
            alpha: 0.6,
        };
        apply_configuration(
            &mut document,
            ConfigurationStyleIntent::SetDocumentDefaultCharacter(CharacterProperties {
                foreground: Some(default_foreground),
                background: Some(default_background),
                underline: Some(true),
                strikethrough: Some(false),
                ..CharacterProperties::default()
            }),
        );
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&"Heading1".into())
            .unwrap()
            .clone();
        let run_foreground = Color {
            red: 0.2,
            green: 0.4,
            blue: 0.8,
            alpha: 1.0,
        };
        let run_background = Color {
            red: 1.0,
            green: 0.95,
            blue: 0.4,
            alpha: 0.5,
        };
        heading.character.foreground = Some(run_foreground);
        heading.character.background = Some(run_background);
        heading.character.underline = Some(false);
        heading.character.strikethrough = Some(true);
        apply_configuration(
            &mut document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(heading)),
        );

        let mut provider_storage = PaintTestProviderStorage::default();
        let provider = CTextMeasurementProvider {
            context: (&mut provider_storage as *mut PaintTestProviderStorage) as usize,
            abi_version: EVIM_TEXT_MEASUREMENT_PROVIDER_ABI_VERSION_V1,
            measurement_environment_id: MeasurementEnvironmentId(41),
            threading: ProviderThreading::AnyWorker,
            render_run_policy: Some(RenderRunPolicy {
                owner: RenderRunOwner(42),
                threading: RenderRunThreading::AnyThread,
            }),
            metrics_generation_callback: paint_test_metrics_generation,
            shape_batch_callback: paint_test_shape_batch,
        };
        let mut core = Core::new(document);
        let view = core.add_view(provider, 800.0, 600.0);
        assert!(core.layout(view).unwrap().snapshot().is_some());
        let handle = register_core(core).unwrap();

        let mut info = EvimLayoutPaintInfoV1::default();
        assert_eq!(
            unsafe { evim_core_view_layout_paint_info(handle, view.0, &mut info) },
            EvimStatus::Ok
        );
        assert_eq!(info.struct_size, EVIM_LAYOUT_PAINT_INFO_V1_SIZE);
        assert_eq!(
            info.canvas_background,
            super::EvimRgbaV1 {
                red: canvas.red,
                green: canvas.green,
                blue: canvas.blue,
                alpha: canvas.alpha,
            }
        );
        assert_eq!(info.default_paint.struct_size, EVIM_TEXT_PAINT_V1_SIZE);
        assert_eq!(
            info.default_paint.flags,
            EVIM_TEXT_PAINT_HAS_BACKGROUND | EVIM_TEXT_PAINT_UNDERLINE
        );
        assert_eq!(info.default_paint.foreground.red, default_foreground.red);
        assert_eq!(
            info.default_paint.background.alpha,
            default_background.alpha
        );
        assert!(info.paint_run_count > 0);

        let mut copied = EvimLayoutPaintInfoV1::default();
        assert_eq!(
            unsafe {
                evim_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    ptr::null_mut(),
                    0,
                    &mut copied,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(copied, info);

        let mut runs = vec![EvimPaintStyleRunV1::default(); info.paint_run_count as usize];
        runs[0].text_start = u64::MAX;
        assert_eq!(
            unsafe {
                evim_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    runs.as_mut_ptr(),
                    runs.len().saturating_sub(1) as u64,
                    &mut copied,
                )
            },
            EvimStatus::BufferTooSmall
        );
        assert_eq!(copied, info);
        assert_eq!(runs[0].text_start, u64::MAX, "copy must not be partial");
        assert_eq!(
            unsafe {
                evim_core_view_copy_layout_paint(
                    handle,
                    view.0,
                    &info.identity,
                    runs.as_mut_ptr(),
                    runs.len() as u64,
                    &mut copied,
                )
            },
            EvimStatus::Ok
        );
        assert_eq!(copied, info);
        assert!(runs
            .windows(2)
            .all(|pair| pair[0].text_end <= pair[1].text_start));
        for run in &runs {
            assert_eq!(run.struct_size, EVIM_PAINT_STYLE_RUN_V1_SIZE);
            assert!(run.text_start < run.text_end);
            assert_eq!(run.paint.struct_size, EVIM_TEXT_PAINT_V1_SIZE);
            assert_eq!(
                run.paint.flags,
                EVIM_TEXT_PAINT_HAS_BACKGROUND | EVIM_TEXT_PAINT_STRIKETHROUGH
            );
            assert_eq!(run.paint.foreground.blue, run_foreground.blue);
            assert_eq!(run.paint.background.alpha, run_background.alpha);
        }

        assert_eq!(evim_core_destroy(handle), EvimStatus::Ok);
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

/// Read a view's line-command domain (0 visual, 1 physical source).
/// # Safety
/// out_mode must identify one aligned writable u32.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_line_mode(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_mode: *mut u32,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_mode.is_null() || (out_mode as usize) % std::mem::align_of::<u32>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let mode = with_core_mut(handle, |core| {
            core.command_state(crate::coordinator::ViewId(view))
                .map(|state| state.line_mode() as u32)
                .ok_or(EvimStatus::InvalidView)
        })?;
        unsafe {
            out_mode.write(mode);
        }
        Ok(())
    })
}
/// Set a view's line-command domain. RTF rejects physical-source mode.
/// # Safety
/// out_outcome must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_line_mode(
    handle: EvimCoreHandle,
    view: EvimViewId,
    mode: u32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe {
            clear_outcome(out_outcome)?;
        }
        let mode = match mode {
            0 => crate::command::LineMode::Visual,
            1 => crate::command::LineMode::PhysicalSource,
            _ => return Err(EvimStatus::InvalidArgument),
        };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetLineMode(mode))
        })?;
        unsafe {
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// Read effective structural paragraph flow. WYSIWYG Markdown/HTML always
/// flow; source modes have an independent per-view option, initially false.
/// # Safety
/// `out_enabled` must identify one aligned writable u32.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_paragraph_flow(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_enabled: *mut u32,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_enabled.is_null() || (out_enabled as usize) % std::mem::align_of::<u32>() != 0 {
            return Err(EvimStatus::InvalidArgument);
        }
        let enabled = with_core_mut(handle, |core| {
            core.paragraph_flow(ViewId(view)).map_err(core_status)
        })?;
        unsafe {
            out_enabled.write(u32::from(enabled));
        }
        Ok(())
    })
}

/// Change source-mode paragraph flow without changing source, undo, or other views.
/// # Safety
/// `out_outcome` must identify one aligned writable outcome.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_paragraph_flow(
    handle: EvimCoreHandle,
    view: EvimViewId,
    enabled: u32,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        unsafe {
            clear_outcome(out_outcome)?;
        }
        if enabled > 1 {
            return Err(EvimStatus::InvalidArgument);
        }
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetParagraphFlow(enabled != 0))
        })?;
        unsafe {
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// Set the application smart-quotes preference for one existing view. This is
/// presentation/input policy only and never changes source or undo state.
#[no_mangle]
pub extern "C" fn evim_core_view_set_smart_quotes(
    handle: EvimCoreHandle,
    view: EvimViewId,
    enabled: u32,
) -> EvimStatus {
    ffi_boundary(|| {
        if enabled > 1 {
            return Err(EvimStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            dispatch_event(core, view, CoreEvent::SetSmartQuotes(enabled != 0)).map(|_| ())
        })
    })
}

pub const EVIM_LINE_LOCATION_GLOBAL_LINE_EXACT: u32 = 1;
pub const EVIM_LINE_LOCATION_FRAGMENT_EXACT: u32 = 2;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvimViewLineLocationV1 {
    pub struct_size: u32,
    pub mode: u32,
    pub flags: u32,
    pub reserved: u32,
    pub line: u64,
    pub column: u64,
    pub hard_line: u64,
    pub fragment: u64,
}
pub const EVIM_VIEW_LINE_LOCATION_V1_SIZE: u32 =
    std::mem::size_of::<EvimViewLineLocationV1>() as u32;
/// Report one-based line/column and the exact hard-line/fragment fallback.
/// line is zero when the global visual row number is not yet materialized.
/// # Safety
/// out_location must identify one aligned writable V1 location record.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_line_location(
    handle: EvimCoreHandle,
    view: EvimViewId,
    out_location: *mut EvimViewLineLocationV1,
) -> EvimStatus {
    ffi_boundary(|| {
        if out_location.is_null()
            || (out_location as usize) % std::mem::align_of::<EvimViewLineLocationV1>() != 0
        {
            return Err(EvimStatus::InvalidArgument);
        }
        let location = with_core_mut(handle, |core| {
            core.line_location(ViewId(view)).map_err(core_status)
        })?;
        let result = EvimViewLineLocationV1 {
            struct_size: EVIM_VIEW_LINE_LOCATION_V1_SIZE,
            mode: location.mode as u32,
            flags: EVIM_LINE_LOCATION_FRAGMENT_EXACT
                | if location.line.is_some() {
                    EVIM_LINE_LOCATION_GLOBAL_LINE_EXACT
                } else {
                    0
                },
            reserved: 0,
            line: location.line.unwrap_or(0) as u64,
            column: location.column as u64,
            hard_line: location.hard_line as u64,
            fragment: location.fragment as u64,
        };
        unsafe {
            out_location.write(result);
        }
        Ok(())
    })
}

/// Change buffer-local readonly policy without editing source or undo state.
#[no_mangle]
pub extern "C" fn evim_core_set_read_only(
    handle: EvimCoreHandle,
    document: u64,
    revision: u64,
    read_only: u32,
) -> EvimStatus {
    ffi_boundary(|| {
        let value = match read_only {
            0 => false,
            1 => true,
            _ => return Err(EvimStatus::InvalidArgument),
        };
        with_core_mut(handle, |core| {
            core.set_read_only(
                crate::document::DocumentId(document),
                Revision(revision),
                value,
            )
            .map_err(core_status)
        })
    })
}
/// Mark recovered bytes as unsaved until a successful save acknowledgement.
#[no_mangle]
pub extern "C" fn evim_core_mark_recovered(
    handle: EvimCoreHandle,
    document: u64,
    revision: u64,
) -> EvimStatus {
    ffi_boundary(|| {
        with_core_mut(handle, |core| {
            core.mark_recovered(crate::document::DocumentId(document), Revision(revision))
                .map_err(core_status)
        })
    })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EvimSelectedStylesInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub style_sheet_revision: u64,
    pub paragraph_id_bytes: u64,
    pub character_id_bytes: u64,
}

/// Copy semantic assignment IDs, excluding automatic source syntax styles.
/// # Safety
/// Output regions must be aligned, writable, and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_selected_styles_export(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected_revision: u64,
    out_info: *mut EvimSelectedStylesInfoV1,
    out_utf8: *mut u8,
    capacity: u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let info_region = typed_pointer_region(out_info, 1)?;
        let bytes_region = typed_pointer_region(out_utf8, capacity)?;
        if regions_overlap(info_region, bytes_region) {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe {
            out_info.write(EvimSelectedStylesInfoV1::default());
        }
        let (info, bytes) = with_core(handle, |core| {
            if core.document().revision().0 != expected_revision {
                return Err(EvimStatus::StaleRevision);
            }
            let selected = core
                .selected_named_styles(ViewId(view))
                .map_err(core_status)?;
            let paragraph = selected.paragraph.map(|id| id.0).unwrap_or_default();
            let character = selected.character.map(|id| id.0).unwrap_or_default();
            let info = EvimSelectedStylesInfoV1 {
                struct_size: size_of::<EvimSelectedStylesInfoV1>() as u32,
                flags: u32::from(selected.paragraph_mixed)
                    | (u32::from(selected.character_mixed) << 1),
                document_id: core.document().id().0,
                document_revision: expected_revision,
                style_sheet_revision: core.document().projection().style_sheet().revision.0,
                paragraph_id_bytes: paragraph.len() as u64,
                character_id_bytes: character.len() as u64,
            };
            Ok((
                info,
                [paragraph.into_bytes(), character.into_bytes()].concat(),
            ))
        })?;
        unsafe {
            out_info.write(info);
        }
        if capacity < bytes.len() as u64 {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_utf8, bytes.len());
            }
        }
        Ok(())
    })
}

/// Load validated JSON defaults only before a core has views or edits. No source mutation.
#[no_mangle]
pub unsafe extern "C" fn evim_core_initialize_style_defaults(
    handle: EvimCoreHandle,
    expected_revision: u64,
    json: *const u8,
    length: u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let bytes = unsafe { input_bytes(json, length)? };
        with_core_mut(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            core.initialize_style_defaults(bytes)
                .map_err(|_| EvimStatus::InvalidArgument)
        })
    })
}

/// Two-pass JSON export of sparse defaults plus explicit document declarations.
#[no_mangle]
pub unsafe extern "C" fn evim_core_export_style_defaults(
    handle: EvimCoreHandle,
    expected_revision: u64,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        if required.is_null() {
            return Err(EvimStatus::NullPointer);
        }
        unsafe { required.write(0) };
        let bytes = with_core(handle, |core| {
            validate_revision(core.document(), expected_revision)?;
            core.document()
                .export_style_defaults()
                .map_err(|_| EvimStatus::InvalidArgument)
        })?;
        unsafe { required.write(bytes.len() as u64) };
        if capacity < bytes.len() as u64 {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            if output.is_null() {
                return Err(EvimStatus::NullPointer);
            }
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
        }
        Ok(())
    })
}

/// Directed, revision-checked command prompt selection (offsets exclude the prompt).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EvimCommandLineSelectionV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub anchor_utf8_offset: u64,
    pub active_utf8_offset: u64,
}

/// # Safety
/// `expected` and `out_selection` must be aligned and disjoint readable/writable records.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_command_line_selection(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimCommandLineIdentityV1,
    out_selection: *mut EvimCommandLineSelectionV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let a = typed_pointer_region(expected, 1)?;
        let b = typed_pointer_region(out_selection, 1)?;
        if regions_overlap(a, b) {
            return Err(EvimStatus::InvalidArgument);
        }
        let expected = unsafe { read_command_line_identity(expected)? };
        unsafe {
            out_selection.write(EvimCommandLineSelectionV1::default());
        }
        let selection = with_core(handle, |core| {
            validate_command_line_identity(
                expected,
                export_command_line(core, ViewId(view))?.info.identity,
            )?;
            let snapshot = core
                .command_state(ViewId(view))
                .and_then(|s| s.command_line_snapshot());
            Ok(EvimCommandLineSelectionV1 {
                struct_size: size_of::<EvimCommandLineSelectionV1>() as u32,
                reserved: 0,
                anchor_utf8_offset: snapshot.as_ref().map_or(0, |s| s.anchor as u64),
                active_utf8_offset: snapshot.as_ref().map_or(0, |s| s.active as u64),
            })
        })?;
        unsafe {
            out_selection.write(selection);
        }
        Ok(())
    })
}

/// Select (operation=0, text empty) or replace a range (operation=1) in one exact
/// command prompt snapshot. Selection endpoints are directed; replacement ranges
/// are ordered and half-open. Source/document history remains untouched.
/// # Safety
/// All records/buffers must be valid and mutually disjoint for their stated sizes.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_edit_command_line(
    handle: EvimCoreHandle,
    view: EvimViewId,
    expected: *const EvimCommandLineIdentityV1,
    operation: u32,
    start: u64,
    end: u64,
    text: *const u8,
    length: u64,
    out_outcome: *mut EvimCoreOutcomeV1,
) -> EvimStatus {
    ffi_boundary(|| {
        let a = typed_pointer_region(expected, 1)?;
        let b = typed_pointer_region(text, length)?;
        let c = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(a, b) || regions_overlap(a, c) || regions_overlap(b, c) {
            return Err(EvimStatus::InvalidArgument);
        }
        let expected = unsafe { read_command_line_identity(expected)? };
        let text = std::str::from_utf8(unsafe { input_bytes(text, length)? })
            .map_err(|_| EvimStatus::InvalidUtf8)?
            .to_owned();
        let start = usize::try_from(start).map_err(|_| EvimStatus::LengthOverflow)?;
        let end = usize::try_from(end).map_err(|_| EvimStatus::LengthOverflow)?;
        let action = match operation {
            0 if text.is_empty() => crate::command::CommandLineEditAction::Select {
                anchor: start,
                active: end,
            },
            1 => crate::command::CommandLineEditAction::Replace {
                range: start..end,
                text,
            },
            _ => return Err(EvimStatus::InvalidArgument),
        };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_command_line_identity(
                expected,
                export_command_line(core, ViewId(view))?.info.identity,
            )?;
            let snapshot = core
                .command_state(ViewId(view))
                .and_then(|s| s.command_line_snapshot())
                .ok_or(EvimStatus::InvalidArgument)?;
            dispatch_event(
                core,
                view,
                CoreEvent::EditCommandLine(crate::command::CommandLineEditRequest {
                    document: core.document().id(),
                    revision: core.document().revision(),
                    expected: snapshot,
                    action,
                }),
            )
        })?;
        unsafe {
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// Change format and return an owned immutable warning/effect batch.
/// # Safety
/// All request/output regions must be aligned, valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_format_with_effects(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetFormatV1,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effects: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ];
        for a in 0..regions.len() {
            for b in a + 1..regions.len() {
                if regions_overlap(regions[a], regions[b]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_SET_FORMAT_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe {
            clear_outcome(out_outcome)?;
            out_effects.write(0);
        }
        let target = parse_format(request.format)?;
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            let view = ViewId(view);
            let outcome = core
                .handle(
                    view,
                    CoreEvent::SetFormat {
                        document: DocumentId(request.document_id),
                        revision: Revision(request.document_revision),
                        target,
                    },
                )
                .map_err(core_status)?;
            let summary = summarize_core_outcome(core, view, Some(&outcome))?;
            let effects = OwnedEffectBatch::from_command(
                core.document(),
                core.command_state(view).ok_or(EvimStatus::InvalidView)?,
                &ClipboardCommandContext::default(),
                outcome.command,
            );
            Ok((summary, effects))
        })?;
        let effects = if let Some(effects) = effects {
            reservation.commit(effects)?
        } else {
            drop(reservation);
            0
        };
        unsafe {
            out_outcome.write(summary);
            out_effects.write(effects);
        }
        Ok(())
    })
}

/// Change encoding and return an owned immutable warning/effect batch.
/// # Safety
/// All request/output regions must be aligned, valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn evim_core_view_set_encoding_with_effects(
    handle: EvimCoreHandle,
    view: EvimViewId,
    request: *const EvimSetEncodingV1,
    out_outcome: *mut EvimCoreOutcomeV1,
    out_effects: *mut EvimEffectBatchHandle,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(request, 1)?,
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ];
        for a in 0..regions.len() {
            for b in a + 1..regions.len() {
                if regions_overlap(regions[a], regions[b]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_SET_ENCODING_V1_SIZE {
            return Err(EvimStatus::InvalidArgument);
        }
        unsafe {
            clear_outcome(out_outcome)?;
            out_effects.write(0);
        }
        let target = parse_encoding(request.encoding)?.ok_or(EvimStatus::InvalidEncoding)?;
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            let view = ViewId(view);
            let outcome = core
                .handle(
                    view,
                    CoreEvent::SetEncoding {
                        document: DocumentId(request.document_id),
                        revision: Revision(request.document_revision),
                        target,
                    },
                )
                .map_err(core_status)?;
            let summary = summarize_core_outcome(core, view, Some(&outcome))?;
            let effects = OwnedEffectBatch::from_command(
                core.document(),
                core.command_state(view).ok_or(EvimStatus::InvalidView)?,
                &ClipboardCommandContext::default(),
                outcome.command,
            );
            Ok((summary, effects))
        })?;
        let effects = if let Some(effects) = effects {
            reservation.commit(effects)?
        } else {
            drop(reservation);
            0
        };
        unsafe {
            out_outcome.write(summary);
            out_effects.write(effects);
        }
        Ok(())
    })
}

/// Copy exact source bytes for an explicit semantic hard-line range.
/// # Safety
/// Outputs must be aligned writable and disjoint; nonzero capacity needs bytes.
#[no_mangle]
pub unsafe extern "C" fn evim_core_copy_hard_line_source_bytes(
    handle: EvimCoreHandle,
    document: u64,
    revision: u64,
    first_line: u64,
    end_line: u64,
    output: *mut u8,
    capacity: u64,
    out_required: *mut u64,
    out_complete: *mut u32,
) -> EvimStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(out_required, 1)?,
            typed_pointer_region(out_complete, 1)?,
        ];
        for a in 0..regions.len() {
            for b in a + 1..regions.len() {
                if regions_overlap(regions[a], regions[b]) {
                    return Err(EvimStatus::InvalidArgument);
                }
            }
        }
        unsafe {
            out_required.write(0);
            out_complete.write(0);
        }
        let (bytes, complete) = with_core(handle, |core| {
            let doc = core.document();
            if doc.id() != DocumentId(document) {
                return Err(EvimStatus::InvalidArgument);
            }
            validate_revision(doc, revision)?;
            let range = doc
                .source_byte_range_for_hard_lines(
                    checked_length(first_line)?..checked_length(end_line)?,
                )
                .map_err(|_| EvimStatus::PolicyRequired)?;
            let complete = range.start == 0 && range.end == doc.source_byte_len();
            Ok((doc.source_bytes()[range].to_vec(), complete))
        })?;
        unsafe {
            out_required.write(bytes.len() as u64);
            out_complete.write(u32::from(complete));
        }
        if capacity < bytes.len() as u64 {
            return Err(EvimStatus::BufferTooSmall);
        }
        if !bytes.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
            }
        }
        Ok(())
    })
}

unsafe fn read_command_turn_context_v2(
    pointer: *const EvimCommandTurnContextV2,
    forbidden_outputs: &[(usize, usize)],
) -> Result<ClipboardCommandContext, EvimStatus> {
    let region = typed_pointer_region(pointer, 1)?;
    if forbidden_outputs.iter().any(|output| regions_overlap(region, *output)) { return Err(EvimStatus::InvalidArgument); }
    let context = unsafe { pointer.read() };
    if context.struct_size < EVIM_COMMAND_TURN_CONTEXT_V2_SIZE || context.reserved != 0 || context.clipboard_count > 2 { return Err(EvimStatus::InvalidArgument); }
    let entries_region = typed_pointer_region(context.clipboards, context.clipboard_count)?;
    if forbidden_outputs.iter().any(|output| regions_overlap(entries_region, *output)) { return Err(EvimStatus::InvalidArgument); }
    let entries = if context.clipboard_count == 0 { Vec::new() } else {
        unsafe { slice::from_raw_parts(context.clipboards, checked_length(context.clipboard_count)?) }.to_vec()
    };
    let mut result = ClipboardCommandContext::new();
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        if entry.struct_size < EVIM_CLIPBOARD_TURN_ENTRY_V2_SIZE || entry.reserved != 0
            || entry.flags & !(EVIM_CLIPBOARD_TURN_HAS_READ | EVIM_CLIPBOARD_TURN_WRITABLE) != 0 { return Err(EvimStatus::InvalidArgument); }
        let target = parse_clipboard_target(entry.target)?;
        if !seen.insert(target) { return Err(EvimStatus::InvalidArgument); }
        for region in [typed_pointer_region(entry.plain_text.data, entry.plain_text.length)?,typed_pointer_region(entry.fragment_json.data, entry.fragment_json.length)?] {
            if forbidden_outputs.iter().any(|output| regions_overlap(region,*output)) { return Err(EvimStatus::InvalidArgument); }
        }
        if entry.flags & EVIM_CLIPBOARD_TURN_HAS_READ != 0 {
            let text = str::from_utf8(unsafe { input_bytes(entry.plain_text.data,entry.plain_text.length)? }).map_err(|_| EvimStatus::InvalidUtf8)?.to_owned();
            let content = if entry.fragment_json.length == 0 { ClipboardContent::from_plain_text(text) } else {
                let bytes = unsafe { input_bytes(entry.fragment_json.data,entry.fragment_json.length)? };
                let register = str::from_utf8(bytes).ok()
                    .and_then(|json| crate::document::ClipboardFragment::from_json(json,&text).ok())
                    .and_then(|fragment| crate::command::RegisterValue::from_clipboard_fragment(fragment).ok());
                ClipboardContent::try_new(text,register).map_err(|_| EvimStatus::InvalidArgument)?
            };
            result = result.with_read(ClipboardSnapshot::new(target,ClipboardGeneration(entry.generation),content));
        } else if entry.generation != 0 || entry.plain_text.length != 0 || entry.fragment_json.length != 0 { return Err(EvimStatus::InvalidArgument); }
        if entry.flags & EVIM_CLIPBOARD_TURN_WRITABLE != 0 { result = result.with_write(target); }
    }
    Ok(result)
}

/// Copy versioned source/style clipboard JSON for an exact formatted range.
/// # Safety
/// Inputs/outputs must be valid, aligned where typed, and non-overlapping.
#[no_mangle]
pub unsafe extern "C" fn evim_core_copy_clipboard_json(
    handle: EvimCoreHandle,
    request: *const EvimFormattedUtf8RangeV1,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let request_region = typed_pointer_region(request,1)?;
        let output_region = typed_pointer_region(output,output_capacity)?;
        let required_region = typed_pointer_region(out_required,1)?;
        if regions_overlap(request_region,output_region) || regions_overlap(request_region,required_region) || regions_overlap(output_region,required_region) { return Err(EvimStatus::InvalidArgument); }
        let request = unsafe { request.read() };
        if request.struct_size < EVIM_FORMATTED_UTF8_RANGE_V1_SIZE || request.reserved != 0 { return Err(EvimStatus::InvalidArgument); }
        unsafe { out_required.write(0); }
        let fragment = with_core(handle,|core| {
            validate_formatted_snapshot_identity(request.identity,core.document())?;
            core.document().clipboard_fragment(checked_length(request.utf8_start)?..checked_length(request.utf8_end)?)
                .map_err(|_| EvimStatus::InvalidArgument)
        })?;
        unsafe { copy_clipboard_json_bytes(fragment.json().as_bytes(),output,output_capacity,out_required) }
    })
}

/// Copy the immutable fragment captured before an emitted clipboard write.
/// A plain-only write has a zero-byte result.
/// # Safety
/// Output storage must be valid and the output regions may not overlap.
#[no_mangle]
pub unsafe extern "C" fn evim_effect_batch_copy_clipboard_json(
    batch: EvimEffectBatchHandle,
    clipboard_index: u64,
    output: *mut u8,
    output_capacity: u64,
    out_required: *mut u64,
) -> EvimStatus {
    ffi_boundary(|| {
        let output_region = typed_pointer_region(output,output_capacity)?;
        let required_region = typed_pointer_region(out_required,1)?;
        if regions_overlap(output_region,required_region) { return Err(EvimStatus::InvalidArgument); }
        unsafe { out_required.write(0); }
        let batch = owned_effect_batch(batch)?;
        let write = batch.clipboard_writes.get(checked_length(clipboard_index)?).ok_or(EvimStatus::InvalidArgument)?;
        let bytes = write.content().portable_register().and_then(|value| value.clipboard_fragment()).map_or(&[][..], |fragment| fragment.json().as_bytes());
        unsafe { copy_clipboard_json_bytes(bytes,output,output_capacity,out_required) }
    })
}

unsafe fn copy_clipboard_json_bytes(bytes: &[u8], output: *mut u8, capacity: u64, required: *mut u64) -> Result<(),EvimStatus> {
    unsafe { required.write(checked_export_count(bytes.len())?); }
    if checked_length(capacity)? < bytes.len() { return Err(EvimStatus::BufferTooSmall); }
    if !bytes.is_empty() { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(),output,bytes.len()); } }
    Ok(())
}
